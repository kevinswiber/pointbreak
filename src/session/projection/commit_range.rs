//! Git-free projection of a Revision's commit-range lifecycle.
//!
//! Folds `WorkObjectProposed` plus the four association/withdrawal events into a
//! per-unit view of which commits and refs the unit currently claims. Every
//! status is derived: `current = capture_target_seed ∪ (associated − withdrawn)`.
//! A commit-range capture seeds one anchored commit that is never withdrawable;
//! only association-backed edges are subtracted by their association id.
//!
//! This fold touches only `event_type` and `payload` — no git, no clock, no
//! store reads. Reachability (merged/live/unreachable/missing) is a separate
//! read-time enrichment and never enters this view.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::error::Result;
use crate::model::{
    CommitAssociationId, CommitWithdrawalId, RefAssociationId, RefWithdrawalId, ReviewEndpoint,
    ReviewTargetRef, RevisionId,
};
use crate::session::event::{
    EventType, GitProvenance, RevisionCommitAssociatedPayload, RevisionCommitWithdrawnPayload,
    RevisionRefAssociatedPayload, RevisionRefWithdrawnPayload, ShoreEvent, WorkObjectProposal,
    WorkObjectProposedPayload,
};
use crate::session::state::ProjectionDiagnostic;

/// Two or more current landing claims compete: incomparable under ancestry
/// (neither reaches the other), each live or merged, with distinct trees.
/// Realizes the condition ADR-0008 reserved as `ambiguous_supersession` for the
/// commit axis, re-scoped by the 2026-07-09 ADR-0014 amendment: only
/// association-source edges compete (the capture target is provenance, not a
/// landing claim), a chain of successive landings is history, and unreachable
/// claims never compete. Ancestry needs git, so this is emitted by the
/// read-time liveness enrichment, never by this pure fold — the const lives
/// here as the code's single definition beside its sibling.
pub const DIVERGENT_COMMIT_ASSOCIATION_CODE: &str = "divergent_commit_association";
/// Two or more current association edges carry the same tree under different
/// commit OIDs — a content-equivalent rewrite (rebase, cherry-pick, amend).
/// Informational: nothing competes, but the stale edge is worth withdrawing.
/// Decidable in this git-free fold because the payload stores `tree_oid`.
pub const REWRITTEN_COMMIT_ASSOCIATION_CODE: &str = "rewritten_commit_association";
/// A withdrawal names an association id absent from the unit's associated set.
/// The withdrawal has no effect yet; the diagnostic clears when the association
/// backfills. Recorded, never rejected.
pub const RETRACTION_TARGET_MISSING_CODE: &str = "retraction_target_missing";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionCommitRangeProjection {
    pub units: BTreeMap<RevisionId, RevisionCommitRangeView>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionCommitRangeView {
    pub revision_id: RevisionId,
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

impl RevisionCommitRangeProjection {
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
                EventType::RevisionCommitAssociated => {
                    let payload: RevisionCommitAssociatedPayload =
                        serde_json::from_value(event.payload.clone())?;
                    if let (
                        Some(revision_id),
                        ReviewEndpoint::GitCommit {
                            commit_oid,
                            tree_oid,
                        },
                    ) = (revision_of(&payload.target), payload.commit)
                    {
                        builders
                            .entry(revision_id)
                            .or_default()
                            .associated_commits
                            .insert(payload.commit_association_id, (commit_oid, tree_oid));
                    }
                }
                EventType::RevisionCommitWithdrawn => {
                    let payload: RevisionCommitWithdrawnPayload =
                        serde_json::from_value(event.payload.clone())?;
                    if let Some(revision_id) = revision_of(&payload.target) {
                        builders
                            .entry(revision_id)
                            .or_default()
                            .withdrawn_commits
                            .insert(payload.commit_association_id, payload.commit_withdrawal_id);
                    }
                }
                EventType::RevisionRefAssociated => {
                    let payload: RevisionRefAssociatedPayload =
                        serde_json::from_value(event.payload.clone())?;
                    if let Some(revision_id) = revision_of(&payload.target) {
                        builders
                            .entry(revision_id)
                            .or_default()
                            .associated_refs
                            .insert(
                                payload.ref_association_id,
                                (payload.ref_name, payload.head_oid),
                            );
                    }
                }
                EventType::RevisionRefWithdrawn => {
                    let payload: RevisionRefWithdrawnPayload =
                        serde_json::from_value(event.payload.clone())?;
                    if let Some(revision_id) = revision_of(&payload.target) {
                        builders
                            .entry(revision_id)
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
            .map(|(revision_id, builder)| {
                let view = builder.finish(revision_id.clone());
                (revision_id, view)
            })
            .collect();

        Ok(Self { units })
    }

    pub fn unit(&self, revision_id: &RevisionId) -> Option<&RevisionCommitRangeView> {
        self.units.get(revision_id)
    }

    /// Units whose current refs include an exact `ref_name` label match. Used by
    /// the offline `--ref` (Label) read filter.
    pub fn units_for_ref(&self, ref_name: &str) -> Vec<&RevisionCommitRangeView> {
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
/// `Revision` target shape.
pub(crate) fn revision_of(target: &ReviewTargetRef) -> Option<RevisionId> {
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
    fn finish(self, revision_id: RevisionId) -> RevisionCommitRangeView {
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
                    "revision {} withdraws association {missing}, which has no matching association",
                    revision_id.as_str()
                ),
            ));
        }

        // rewritten_commit_association: same tree under different commit OIDs among
        // the association edges — a content-equivalent rewrite, decidable without
        // git. Divergence (competing landing claims) needs ancestry and is decided
        // by the read-time liveness enrichment, never here: multiple distinct OIDs
        // accreting on one revision over successive landings are history, not a
        // conflict, and the capture-target edge is provenance rather than a claim.
        let mut oids_by_tree = BTreeMap::<&str, std::collections::BTreeSet<&str>>::new();
        for commit in current_commits
            .iter()
            .filter(|commit| commit.source == CommitEdgeSource::Association)
        {
            oids_by_tree
                .entry(commit.tree_oid.as_str())
                .or_default()
                .insert(commit.commit_oid.as_str());
        }
        for (tree_oid, oids) in oids_by_tree {
            if oids.len() > 1 {
                diagnostics.push(diagnostic(
                    REWRITTEN_COMMIT_ASSOCIATION_CODE,
                    format!(
                        "revision {} has {} content-equivalent commit associations \
                         for tree {tree_oid} — a rewritten landing; withdraw the stale edge",
                        revision_id.as_str(),
                        oids.len(),
                    ),
                ));
            }
        }

        RevisionCommitRangeView {
            revision_id,
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
        EngagementId, JournalId, ObjectId, ReviewEndpoint, ReviewTargetRef, RevisionId,
        RevisionSource, RootCommitCaptureMode, WorktreeCaptureMode,
    };
    use crate::session::event::{
        EventTarget, EventType, GitProvenance, Revision, RevisionCommitAssociatedPayload,
        RevisionCommitWithdrawnPayload, RevisionRefAssociatedPayload, ShoreEvent,
        WorkObjectProposal, WorkObjectProposedPayload, Writer, build_commit_association_id,
        build_commit_withdrawal_id, build_ref_association_id,
    };

    fn revision_id() -> RevisionId {
        RevisionId::new("rev:git:sha256:def")
    }

    fn target() -> ReviewTargetRef {
        ReviewTargetRef::Revision {
            revision_id: revision_id(),
        }
    }

    fn envelope() -> EventTarget {
        EventTarget::for_revision(JournalId::new("journal:default"), revision_id(), None).unwrap()
    }

    fn capture(target_endpoint: ReviewEndpoint) -> ShoreEvent {
        ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("work_object_proposed:{}", revision_id().as_str()),
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
                        id: revision_id(),
                        object_id: ObjectId::new("snap:git:sha256:ghi"),
                        git_provenance: Some(GitProvenance {
                            source: RevisionSource::GitWorktree {
                                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                                include_untracked: true,
                                pathspecs: Vec::new(),
                            },
                            base: ReviewEndpoint::GitCommit {
                                commit_oid: "base".to_owned(),
                                tree_oid: "base-tree".to_owned(),
                            },
                            target: target_endpoint,
                        }),
                    },
                    summary: None,
                    object_artifact_content_hash: "sha256:artifact".to_owned(),
                    supersedes: vec![],
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

    fn root_capture() -> ShoreEvent {
        ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("work_object_proposed:{}", revision_id().as_str()),
            envelope(),
            Writer::shore_local("test"),
            WorkObjectProposedPayload {
                engagement_id: EngagementId::new(format!(
                    "engagement:sha256:{}",
                    crate::canonical_hash::sha256_bytes_hex(revision_id().as_str().as_bytes())
                )),
                work_object: WorkObjectProposal::Revision {
                    revision: Revision {
                        id: revision_id(),
                        object_id: ObjectId::new("snap:git:sha256:root"),
                        git_provenance: Some(GitProvenance {
                            source: RevisionSource::GitRootCommit {
                                mode: RootCommitCaptureMode::EmptyTreeToTargetTree,
                                pathspecs: Vec::new(),
                            },
                            base: ReviewEndpoint::GitTree {
                                tree_oid: "empty-tree".to_owned(),
                            },
                            target: ReviewEndpoint::GitCommit {
                                commit_oid: "target".to_owned(),
                                tree_oid: "target-tree".to_owned(),
                            },
                        }),
                    },
                    summary: None,
                    object_artifact_content_hash: "sha256:artifact".to_owned(),
                    supersedes: vec![],
                },
            },
            "2026-06-19T00:00:00Z",
        )
        .unwrap()
    }

    fn commit_associated(commit_oid: &str) -> ShoreEvent {
        commit_associated_with_tree(commit_oid, &format!("{commit_oid}-tree"))
    }

    fn commit_associated_with_tree(commit_oid: &str, tree_oid: &str) -> ShoreEvent {
        let ru = revision_id();
        let cid = build_commit_association_id(&ru, commit_oid).unwrap();
        ShoreEvent::new(
            EventType::RevisionCommitAssociated,
            RevisionCommitAssociatedPayload::idempotency_key(&ru, commit_oid),
            envelope(),
            Writer::shore_local("test"),
            RevisionCommitAssociatedPayload {
                commit_association_id: cid,
                target: target(),
                commit: ReviewEndpoint::GitCommit {
                    commit_oid: commit_oid.to_owned(),
                    tree_oid: tree_oid.to_owned(),
                },
            },
            "2026-06-19T00:00:01Z",
        )
        .unwrap()
    }

    fn commit_withdrawn(commit_oid: &str) -> ShoreEvent {
        let ru = revision_id();
        let cid = build_commit_association_id(&ru, commit_oid).unwrap();
        let wid = build_commit_withdrawal_id(&ru, &cid).unwrap();
        ShoreEvent::new(
            EventType::RevisionCommitWithdrawn,
            RevisionCommitWithdrawnPayload::idempotency_key(&cid),
            envelope(),
            Writer::shore_local("test"),
            RevisionCommitWithdrawnPayload {
                commit_withdrawal_id: wid,
                target: target(),
                commit_association_id: cid,
            },
            "2026-06-19T00:00:02Z",
        )
        .unwrap()
    }

    fn ref_associated(ref_name: &str, head_oid: &str) -> ShoreEvent {
        let ru = revision_id();
        let rid = build_ref_association_id(&ru, ref_name, head_oid).unwrap();
        ShoreEvent::new(
            EventType::RevisionRefAssociated,
            RevisionRefAssociatedPayload::idempotency_key(&ru, ref_name, head_oid),
            envelope(),
            Writer::shore_local("test"),
            RevisionRefAssociatedPayload {
                ref_association_id: rid,
                target: target(),
                ref_name: ref_name.to_owned(),
                head_oid: head_oid.to_owned(),
            },
            "2026-06-19T00:00:03Z",
        )
        .unwrap()
    }

    fn view_of(events: &[ShoreEvent]) -> RevisionCommitRangeView {
        RevisionCommitRangeProjection::from_events(events)
            .unwrap()
            .unit(&revision_id())
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
    fn revision_source_root_capture_is_anchored_by_target_commit() {
        let view = view_of(&[root_capture()]);

        assert!(view.anchored);
        assert_eq!(view.current_commits.len(), 1);
        assert_eq!(view.current_commits[0].commit_oid, "target");
        assert_eq!(
            view.current_commits[0].source,
            CommitEdgeSource::CaptureTarget
        );
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
    fn accreted_commit_associations_are_history_not_divergence() {
        // Successive landings on one revision (different trees) surface both
        // edges with NO fold diagnostic: divergence needs ancestry and belongs
        // to the read-time liveness enrichment, never this git-free fold.
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
            view.diagnostics.is_empty(),
            "accretion is history, not a conflict: {:?}",
            view.diagnostics
        );
    }

    #[test]
    fn rewritten_commit_association_flags_content_equivalent_edges() {
        let view = view_of(&[
            worktree_capture(),
            commit_associated_with_tree("oidA", "sharedtree"),
            commit_associated_with_tree("oidB", "sharedtree"),
        ]);

        assert_eq!(view.current_commits.len(), 2);
        let rewritten = view
            .diagnostics
            .iter()
            .find(|d| d.code == REWRITTEN_COMMIT_ASSOCIATION_CODE)
            .expect("rewritten diagnostic present");
        assert!(
            rewritten.message.contains("revision"),
            "message names the revision work object: {}",
            rewritten.message
        );
        assert!(
            !view
                .diagnostics
                .iter()
                .any(|d| d.code == DIVERGENT_COMMIT_ASSOCIATION_CODE),
            "a content-equivalent rewrite never reads as divergence"
        );
    }

    #[test]
    fn capture_target_never_joins_the_rewrite_check() {
        // A commit-range capture target sharing its tree with one landed
        // association (a message-amended landing) is provenance beside a claim,
        // not a rewrite pair.
        let view = view_of(&[
            capture(ReviewEndpoint::GitCommit {
                commit_oid: "target".to_owned(),
                tree_oid: "sharedtree".to_owned(),
            }),
            commit_associated_with_tree("oidA", "sharedtree"),
        ]);

        assert_eq!(view.current_commits.len(), 2);
        assert!(
            view.diagnostics.is_empty(),
            "capture target is provenance, never a claim: {:?}",
            view.diagnostics
        );
    }

    #[test]
    fn retraction_target_missing_self_heals() {
        let missing = view_of(&[worktree_capture(), commit_withdrawn("oidA")]);
        let retraction = missing
            .diagnostics
            .iter()
            .find(|d| d.code == RETRACTION_TARGET_MISSING_CODE)
            .expect("retraction diagnostic present");
        assert!(
            retraction.message.contains("revision"),
            "message names the revision work object: {}",
            retraction.message
        );
        assert!(
            !retraction.message.contains("review unit"),
            "retired vocabulary swept: {}",
            retraction.message
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

        let projection = RevisionCommitRangeProjection::from_events(&[
            worktree_capture(),
            ref_associated("refs/heads/feat/x", "oidH"),
        ])
        .unwrap();
        assert_eq!(projection.units_for_ref("refs/heads/feat/x").len(), 1);
        assert_eq!(projection.units_for_ref("refs/heads/other").len(), 0);
    }
}
