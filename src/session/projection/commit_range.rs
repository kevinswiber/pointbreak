//! Git-free projection of a ReviewUnit's commit-range lifecycle.
//!
//! Folds `WorkObjectProposed` plus the four association/withdrawal events into a
//! per-unit view of which commits and refs the unit currently claims. Every
//! status is derived: `current = capture_target_seed ∪ (associated − withdrawn)`.
//! A commit-range capture seeds one anchored commit that is never withdrawable;
//! only association-backed edges are subtracted by their association id.
//!
//! This fold touches only `event_type` and `payload` — no git, no clock, no
//! store reads. Reachability (merged/live/orphaned) is a separate read-time
//! enrichment and never enters this view.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::error::Result;
use crate::model::{
    CommitAssociationId, CommitWithdrawalId, RefAssociationId, RefWithdrawalId, ReviewEndpoint,
    ReviewTargetRef, RevisionId,
};
use crate::session::event::{
    EventType, GitProvenance, ReviewUnitCommitAssociatedPayload, ReviewUnitCommitWithdrawnPayload,
    ReviewUnitRefAssociatedPayload, ReviewUnitRefWithdrawnPayload, ShoreEvent, WorkObjectProposal,
    WorkObjectProposedPayload,
};
use crate::session::state::ProjectionDiagnostic;

/// Two un-withdrawn commit edges with different OIDs claim one unit at once. The
/// projection surfaces both and withholds any headline. Realizes the condition
/// ADR-0008 reserved as `ambiguous_supersession` for the commit axis.
pub const DIVERGENT_COMMIT_ASSOCIATION_CODE: &str = "divergent_commit_association";
/// A withdrawal names an association id absent from the unit's associated set.
/// The withdrawal has no effect yet; the diagnostic clears when the association
/// backfills. Recorded, never rejected.
pub const RETRACTION_TARGET_MISSING_CODE: &str = "retraction_target_missing";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUnitCommitRangeProjection {
    pub units: BTreeMap<RevisionId, ReviewUnitCommitRangeView>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUnitCommitRangeView {
    pub review_unit_id: RevisionId,
    pub anchored: bool,
    pub current_commits: Vec<CurrentCommitAssociation>,
    pub current_refs: Vec<CurrentRefAssociation>,
    pub withdrawn_commits: Vec<WithdrawnCommitAssociation>,
    pub withdrawn_refs: Vec<WithdrawnRefAssociation>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

/// How a current commit edge entered the view.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitEdgeSource {
    /// A commit-range capture's seed target: born anchored, not withdrawable.
    CaptureTarget,
    /// An association event: subtractable by its association id.
    Association,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentCommitAssociation {
    pub commit_oid: String,
    pub tree_oid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_association_id: Option<CommitAssociationId>,
    pub source: CommitEdgeSource,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentRefAssociation {
    pub ref_association_id: RefAssociationId,
    pub ref_name: String,
    pub head_oid: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WithdrawnCommitAssociation {
    pub commit_oid: String,
    pub tree_oid: String,
    pub commit_association_id: CommitAssociationId,
    pub commit_withdrawal_id: CommitWithdrawalId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WithdrawnRefAssociation {
    pub ref_association_id: RefAssociationId,
    pub ref_name: String,
    pub head_oid: String,
    pub ref_withdrawal_id: RefWithdrawalId,
}

impl ReviewUnitCommitRangeProjection {
    pub fn from_events(events: &[ShoreEvent]) -> Result<Self> {
        let mut builders = BTreeMap::<RevisionId, CommitRangeBuilder>::new();

        for event in events {
            match event.event_type {
                EventType::WorkObjectProposed => {
                    let payload: WorkObjectProposedPayload =
                        serde_json::from_value(event.payload.clone())?;
                    if let WorkObjectProposal::Revision { revision, .. } = payload.work_object {
                        let builder = builders.entry(revision.id.clone()).or_default();
                        if let Some(GitProvenance {
                            target:
                                ReviewEndpoint::GitCommit {
                                    commit_oid,
                                    tree_oid,
                                },
                            ..
                        }) = revision.git_provenance
                        {
                            builder.capture_target = Some((commit_oid, tree_oid));
                        }
                    }
                }
                EventType::ReviewUnitCommitAssociated => {
                    let payload: ReviewUnitCommitAssociatedPayload =
                        serde_json::from_value(event.payload.clone())?;
                    if let (
                        Some(review_unit_id),
                        ReviewEndpoint::GitCommit {
                            commit_oid,
                            tree_oid,
                        },
                    ) = (review_unit_of(&payload.target), payload.commit)
                    {
                        builders
                            .entry(review_unit_id)
                            .or_default()
                            .associated_commits
                            .insert(payload.commit_association_id, (commit_oid, tree_oid));
                    }
                }
                EventType::ReviewUnitCommitWithdrawn => {
                    let payload: ReviewUnitCommitWithdrawnPayload =
                        serde_json::from_value(event.payload.clone())?;
                    if let Some(review_unit_id) = review_unit_of(&payload.target) {
                        builders
                            .entry(review_unit_id)
                            .or_default()
                            .withdrawn_commits
                            .insert(payload.commit_association_id, payload.commit_withdrawal_id);
                    }
                }
                EventType::ReviewUnitRefAssociated => {
                    let payload: ReviewUnitRefAssociatedPayload =
                        serde_json::from_value(event.payload.clone())?;
                    if let Some(review_unit_id) = review_unit_of(&payload.target) {
                        builders
                            .entry(review_unit_id)
                            .or_default()
                            .associated_refs
                            .insert(
                                payload.ref_association_id,
                                (payload.ref_name, payload.head_oid),
                            );
                    }
                }
                EventType::ReviewUnitRefWithdrawn => {
                    let payload: ReviewUnitRefWithdrawnPayload =
                        serde_json::from_value(event.payload.clone())?;
                    if let Some(review_unit_id) = review_unit_of(&payload.target) {
                        builders
                            .entry(review_unit_id)
                            .or_default()
                            .withdrawn_refs
                            .insert(payload.ref_association_id, payload.ref_withdrawal_id);
                    }
                }
                _ => {}
            }
        }

        let units = builders
            .into_iter()
            .map(|(review_unit_id, builder)| {
                let view = builder.finish(review_unit_id.clone());
                (review_unit_id, view)
            })
            .collect();

        Ok(Self { units })
    }

    pub fn unit(&self, review_unit_id: &RevisionId) -> Option<&ReviewUnitCommitRangeView> {
        self.units.get(review_unit_id)
    }

    /// Units whose current refs include an exact `ref_name` label match. Used by
    /// the offline `--ref` (Label) read filter.
    pub fn units_for_ref(&self, ref_name: &str) -> Vec<&ReviewUnitCommitRangeView> {
        self.units
            .values()
            .filter(|view| {
                view.current_refs
                    .iter()
                    .any(|current| current.ref_name == ref_name)
            })
            .collect()
    }
}

/// The review-unit subject of an association payload, if it is the expected
/// `ReviewUnit` target shape.
pub(crate) fn review_unit_of(target: &ReviewTargetRef) -> Option<RevisionId> {
    match target {
        ReviewTargetRef::Revision { revision_id } => Some(revision_id.clone()),
        _ => None,
    }
}

#[derive(Debug, Default)]
struct CommitRangeBuilder {
    capture_target: Option<(String, String)>,
    associated_commits: BTreeMap<CommitAssociationId, (String, String)>,
    withdrawn_commits: BTreeMap<CommitAssociationId, CommitWithdrawalId>,
    associated_refs: BTreeMap<RefAssociationId, (String, String)>,
    withdrawn_refs: BTreeMap<RefAssociationId, RefWithdrawalId>,
}

impl CommitRangeBuilder {
    fn finish(self, review_unit_id: RevisionId) -> ReviewUnitCommitRangeView {
        let mut diagnostics = Vec::new();

        let commit_axis = partition_axis(self.associated_commits, self.withdrawn_commits);
        let ref_axis = partition_axis(self.associated_refs, self.withdrawn_refs);

        // current_commits: capture-target seed (never withdrawable) ∪ association-backed survivors.
        let mut current_commits = Vec::new();
        if let Some((commit_oid, tree_oid)) = self.capture_target {
            current_commits.push(CurrentCommitAssociation {
                commit_oid,
                tree_oid,
                commit_association_id: None,
                source: CommitEdgeSource::CaptureTarget,
            });
        }
        for (commit_association_id, (commit_oid, tree_oid)) in commit_axis.current {
            current_commits.push(CurrentCommitAssociation {
                commit_oid,
                tree_oid,
                commit_association_id: Some(commit_association_id),
                source: CommitEdgeSource::Association,
            });
        }
        current_commits.sort_by(|left, right| left.commit_oid.cmp(&right.commit_oid));

        let mut withdrawn_commits = commit_axis
            .withdrawn
            .into_iter()
            .map(
                |(commit_association_id, (commit_oid, tree_oid), commit_withdrawal_id)| {
                    WithdrawnCommitAssociation {
                        commit_oid,
                        tree_oid,
                        commit_association_id,
                        commit_withdrawal_id,
                    }
                },
            )
            .collect::<Vec<_>>();
        withdrawn_commits.sort_by(|left, right| left.commit_oid.cmp(&right.commit_oid));

        let mut current_refs = ref_axis
            .current
            .into_iter()
            .map(
                |(ref_association_id, (ref_name, head_oid))| CurrentRefAssociation {
                    ref_association_id,
                    ref_name,
                    head_oid,
                },
            )
            .collect::<Vec<_>>();
        current_refs.sort_by(|left, right| {
            left.ref_name
                .cmp(&right.ref_name)
                .then_with(|| left.head_oid.cmp(&right.head_oid))
        });

        let mut withdrawn_refs = ref_axis
            .withdrawn
            .into_iter()
            .map(
                |(ref_association_id, (ref_name, head_oid), ref_withdrawal_id)| {
                    WithdrawnRefAssociation {
                        ref_association_id,
                        ref_name,
                        head_oid,
                        ref_withdrawal_id,
                    }
                },
            )
            .collect::<Vec<_>>();
        withdrawn_refs.sort_by(|left, right| left.ref_name.cmp(&right.ref_name));

        // retraction_target_missing: a withdrawal whose association id never appeared.
        for missing in commit_axis
            .missing_targets
            .into_iter()
            .map(|id| id.as_str().to_owned())
            .chain(
                ref_axis
                    .missing_targets
                    .into_iter()
                    .map(|id| id.as_str().to_owned()),
            )
        {
            diagnostics.push(diagnostic(
                RETRACTION_TARGET_MISSING_CODE,
                format!(
                    "review unit {} withdraws association {missing}, which has no matching association",
                    review_unit_id.as_str()
                ),
            ));
        }

        // divergent_commit_association: two or more distinct current OIDs claim the unit.
        let distinct_oids = current_commits
            .iter()
            .map(|commit| commit.commit_oid.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        if distinct_oids.len() > 1 {
            diagnostics.push(diagnostic(
                DIVERGENT_COMMIT_ASSOCIATION_CODE,
                format!(
                    "review unit {} has {} distinct current commit associations",
                    review_unit_id.as_str(),
                    distinct_oids.len()
                ),
            ));
        }

        ReviewUnitCommitRangeView {
            review_unit_id,
            anchored: !current_commits.is_empty(),
            current_commits,
            current_refs,
            withdrawn_commits,
            withdrawn_refs,
            diagnostics,
        }
    }
}

struct AxisPartition<I, V, W> {
    current: Vec<(I, V)>,
    withdrawn: Vec<(I, V, W)>,
    missing_targets: Vec<I>,
}

/// Splits an associated map against a withdrawn map by id: survivors are
/// `associated − withdrawn`, withdrawn-history pairs an association with its
/// withdrawal, and a withdrawal with no matching association is a missing
/// target (the `retraction_target_missing` signal). Shared by both axes.
fn partition_axis<I, V, W>(
    associated: BTreeMap<I, V>,
    mut withdrawn: BTreeMap<I, W>,
) -> AxisPartition<I, V, W>
where
    I: Ord + Clone,
{
    let mut current = Vec::new();
    let mut withdrawn_history = Vec::new();
    for (id, value) in associated {
        match withdrawn.remove(&id) {
            Some(withdrawal) => withdrawn_history.push((id, value, withdrawal)),
            None => current.push((id, value)),
        }
    }
    let missing_targets = withdrawn.into_keys().collect();
    AxisPartition {
        current,
        withdrawn: withdrawn_history,
        missing_targets,
    }
}

fn diagnostic(code: &str, message: String) -> ProjectionDiagnostic {
    ProjectionDiagnostic {
        code: code.to_owned(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        EngagementId, LedgerId, ObjectId, ReviewEndpoint, ReviewTargetRef, ReviewUnitSource,
        RevisionId, WorktreeCaptureMode,
    };
    use crate::session::event::{
        EventTarget, EventType, GitProvenance, ReviewUnitCommitAssociatedPayload,
        ReviewUnitCommitWithdrawnPayload, ReviewUnitRefAssociatedPayload, Revision, ShoreEvent,
        WorkObjectProposal, WorkObjectProposedPayload, Writer, build_commit_association_id,
        build_commit_withdrawal_id, build_ref_association_id,
    };

    fn review_unit_id() -> RevisionId {
        RevisionId::new("rev:git:sha256:def")
    }

    fn target() -> ReviewTargetRef {
        ReviewTargetRef::Revision {
            revision_id: review_unit_id(),
        }
    }

    fn envelope() -> EventTarget {
        EventTarget::for_revision(LedgerId::new("session:default"), review_unit_id(), None)
    }

    fn capture(target_endpoint: ReviewEndpoint) -> ShoreEvent {
        ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("work_object_proposed:{}", review_unit_id().as_str()),
            envelope(),
            Writer::shore_local("test"),
            WorkObjectProposedPayload {
                engagement_id: EngagementId::new(format!(
                    "engagement:sha256:{}",
                    crate::canonical_hash::sha256_bytes_hex(
                        (RevisionId::new("rev:git:sha256:def")).as_str().as_bytes()
                    )
                )),
                work_object: WorkObjectProposal::Revision {
                    revision: Revision {
                        id: review_unit_id(),
                        object_id: ObjectId::new("snap:git:sha256:ghi"),
                        git_provenance: Some(GitProvenance {
                            source: ReviewUnitSource::GitWorktree {
                                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                                include_untracked: true,
                            },
                            base: ReviewEndpoint::GitCommit {
                                commit_oid: "base".to_owned(),
                                tree_oid: "base-tree".to_owned(),
                            },
                            target: target_endpoint,
                        }),
                    },
                    snapshot_artifact_content_hash: "sha256:artifact".to_owned(),
                },
            },
            "2026-06-19T00:00:00Z",
        )
        .unwrap()
    }

    fn worktree_capture() -> ShoreEvent {
        capture(ReviewEndpoint::GitWorkingTree {
            worktree_root: "/repo".to_owned(),
        })
    }

    fn commit_associated(commit_oid: &str) -> ShoreEvent {
        let ru = review_unit_id();
        let cid = build_commit_association_id(&ru, commit_oid).unwrap();
        ShoreEvent::new(
            EventType::ReviewUnitCommitAssociated,
            ReviewUnitCommitAssociatedPayload::idempotency_key(&ru, commit_oid),
            envelope(),
            Writer::shore_local("test"),
            ReviewUnitCommitAssociatedPayload {
                commit_association_id: cid,
                target: target(),
                commit: ReviewEndpoint::GitCommit {
                    commit_oid: commit_oid.to_owned(),
                    tree_oid: format!("{commit_oid}-tree"),
                },
            },
            "2026-06-19T00:00:01Z",
        )
        .unwrap()
    }

    fn commit_withdrawn(commit_oid: &str) -> ShoreEvent {
        let ru = review_unit_id();
        let cid = build_commit_association_id(&ru, commit_oid).unwrap();
        let wid = build_commit_withdrawal_id(&ru, &cid).unwrap();
        ShoreEvent::new(
            EventType::ReviewUnitCommitWithdrawn,
            ReviewUnitCommitWithdrawnPayload::idempotency_key(&cid),
            envelope(),
            Writer::shore_local("test"),
            ReviewUnitCommitWithdrawnPayload {
                commit_withdrawal_id: wid,
                target: target(),
                commit_association_id: cid,
            },
            "2026-06-19T00:00:02Z",
        )
        .unwrap()
    }

    fn ref_associated(ref_name: &str, head_oid: &str) -> ShoreEvent {
        let ru = review_unit_id();
        let rid = build_ref_association_id(&ru, ref_name, head_oid).unwrap();
        ShoreEvent::new(
            EventType::ReviewUnitRefAssociated,
            ReviewUnitRefAssociatedPayload::idempotency_key(&ru, ref_name, head_oid),
            envelope(),
            Writer::shore_local("test"),
            ReviewUnitRefAssociatedPayload {
                ref_association_id: rid,
                target: target(),
                ref_name: ref_name.to_owned(),
                head_oid: head_oid.to_owned(),
            },
            "2026-06-19T00:00:03Z",
        )
        .unwrap()
    }

    fn view_of(events: &[ShoreEvent]) -> ReviewUnitCommitRangeView {
        ReviewUnitCommitRangeProjection::from_events(events)
            .unwrap()
            .unit(&review_unit_id())
            .unwrap()
            .clone()
    }

    #[test]
    fn commit_range_capture_is_born_anchored() {
        let view = view_of(&[capture(ReviewEndpoint::GitCommit {
            commit_oid: "target".to_owned(),
            tree_oid: "target-tree".to_owned(),
        })]);

        assert!(view.anchored);
        assert_eq!(view.current_commits.len(), 1);
        assert_eq!(view.current_commits[0].commit_oid, "target");
        assert_eq!(
            view.current_commits[0].source,
            CommitEdgeSource::CaptureTarget
        );
        assert!(view.current_commits[0].commit_association_id.is_none());
    }

    #[test]
    fn worktree_capture_is_born_floating() {
        let view = view_of(&[worktree_capture()]);

        assert!(!view.anchored);
        assert!(view.current_commits.is_empty());
    }

    #[test]
    fn current_set_is_associated_minus_withdrawn() {
        let anchored = view_of(&[worktree_capture(), commit_associated("oidA")]);
        assert!(anchored.anchored);
        assert_eq!(anchored.current_commits.len(), 1);
        assert_eq!(anchored.current_commits[0].commit_oid, "oidA");
        assert_eq!(
            anchored.current_commits[0].source,
            CommitEdgeSource::Association
        );

        let floating = view_of(&[
            worktree_capture(),
            commit_associated("oidA"),
            commit_withdrawn("oidA"),
        ]);
        assert!(!floating.anchored);
        assert!(floating.current_commits.is_empty());
        assert_eq!(floating.withdrawn_commits.len(), 1);
        assert_eq!(floating.withdrawn_commits[0].commit_oid, "oidA");
    }

    #[test]
    fn capture_target_seed_is_not_withdrawable() {
        // A withdrawal cannot name the capture-target seed (it has no association id),
        // so an anchored commit-range capture stays anchored.
        let view = view_of(&[
            capture(ReviewEndpoint::GitCommit {
                commit_oid: "target".to_owned(),
                tree_oid: "target-tree".to_owned(),
            }),
            commit_associated("oidA"),
            commit_withdrawn("oidA"),
        ]);

        assert!(view.anchored);
        assert_eq!(view.current_commits.len(), 1);
        assert_eq!(
            view.current_commits[0].source,
            CommitEdgeSource::CaptureTarget
        );
    }

    #[test]
    fn withdrawal_is_terminal_no_revival() {
        let view = view_of(&[
            worktree_capture(),
            commit_associated("oidA"),
            commit_withdrawn("oidA"),
            commit_associated("oidA"),
        ]);

        assert!(!view.anchored);
        assert!(view.current_commits.is_empty());
        assert_eq!(view.withdrawn_commits.len(), 1);
    }

    #[test]
    fn divergent_commit_association_withholds_and_surfaces_both() {
        let view = view_of(&[
            worktree_capture(),
            commit_associated("oidA"),
            commit_associated("oidB"),
        ]);

        assert_eq!(view.current_commits.len(), 2);
        let oids: Vec<&str> = view
            .current_commits
            .iter()
            .map(|commit| commit.commit_oid.as_str())
            .collect();
        assert_eq!(oids, vec!["oidA", "oidB"]);
        assert!(
            view.diagnostics
                .iter()
                .any(|d| d.code == DIVERGENT_COMMIT_ASSOCIATION_CODE)
        );
    }

    #[test]
    fn retraction_target_missing_self_heals() {
        let missing = view_of(&[worktree_capture(), commit_withdrawn("oidA")]);
        assert!(
            missing
                .diagnostics
                .iter()
                .any(|d| d.code == RETRACTION_TARGET_MISSING_CODE)
        );
        assert!(!missing.anchored);

        // When the association backfills, the diagnostic clears and the edge is subtracted.
        let healed = view_of(&[
            worktree_capture(),
            commit_withdrawn("oidA"),
            commit_associated("oidA"),
        ]);
        assert!(
            !healed
                .diagnostics
                .iter()
                .any(|d| d.code == RETRACTION_TARGET_MISSING_CODE)
        );
        assert!(!healed.anchored);
        assert_eq!(healed.withdrawn_commits.len(), 1);
    }

    #[test]
    fn ref_axis_tracks_current_and_units_for_ref() {
        let view = view_of(&[
            worktree_capture(),
            ref_associated("refs/heads/feat/x", "oidH"),
        ]);
        assert_eq!(view.current_refs.len(), 1);
        assert_eq!(view.current_refs[0].ref_name, "refs/heads/feat/x");

        let projection = ReviewUnitCommitRangeProjection::from_events(&[
            worktree_capture(),
            ref_associated("refs/heads/feat/x", "oidH"),
        ])
        .unwrap();
        assert_eq!(projection.units_for_ref("refs/heads/feat/x").len(), 1);
        assert_eq!(projection.units_for_ref("refs/heads/other").len(), 0);
    }
}
