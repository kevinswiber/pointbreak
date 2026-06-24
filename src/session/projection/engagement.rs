use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::error::Result;
use crate::model::{EngagementId, JournalId, ObjectId, RevisionId};
use crate::session::event::{
    EventType, ReviewAssessment, ShoreEvent, WorkObjectProposal, WorkObjectProposedPayload,
};
use crate::session::projection::supersession::SupersessionView;
use crate::session::state::ProjectionDiagnostic;
use crate::session::workflow::assessment::{
    AssessmentProjectionOptions, CurrentAssessmentStatus, project_assessments,
};
use crate::session::workflow::observation::ResolvedRevision;

/// A capture bridges two or more previously-separate engagements (its supersession
/// targets carried different engagement hints). The grouping unifies the connected
/// component; the stored events are never re-stamped.
pub const ENGAGEMENTS_MERGED_CODE: &str = "engagements_merged";

/// The DAG-authoritative engagement grouping over the event log: an engagement is
/// one connected component of the supersession graph (the authoritative
/// membership), the stored `engagement_id` is only a write-time hint. Grouping is
/// purely read-time — a dangling target self-heals on backfill and a cross-engagement
/// bridge merges in the projection; nothing gates a write on membership.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngagementGrouping {
    pub engagements: Vec<EngagementView>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngagementView {
    /// The canonical engagement id for the component (the smallest member
    /// revision's stored hint; a merged component collapses to one).
    pub engagement_id: EngagementId,
    /// The component's revisions (its authoritative membership).
    pub revisions: BTreeSet<RevisionId>,
    /// The component-scoped current heads (`>= 2` is a fork; the engagement stays
    /// in progress).
    pub heads: BTreeSet<RevisionId>,
    pub lifecycle: EngagementLifecycle,
}

/// The derived engagement lifecycle. No `Opened`/`Closed` events exist — start is
/// the root revision, and the terminal is derived from the current-assessment
/// projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngagementLifecycle {
    /// Un-superseded heads remain or no single un-replaced acceptance has resolved
    /// (competing heads keep an engagement here).
    InProgress,
    /// The single current head resolved to one un-replaced `Accepted` assessment.
    Accepted,
}

impl EngagementGrouping {
    pub fn from_events(events: &[ShoreEvent]) -> Result<Self> {
        let view = SupersessionView::from_events(events)?;
        let captures = revision_captures(events)?;

        // Echo the supersession diagnostics (a dangling target / a cycle); they
        // describe the same DAG this grouping reads.
        let mut diagnostics = view.diagnostics.clone();
        let mut engagements = Vec::new();

        for component in &view.components {
            // Every component revision is a known capture, so the smallest member
            // always yields a hint; skip defensively rather than panic if not.
            let Some(canonical) = component
                .iter()
                .find_map(|revision| captures.get(revision).map(|c| c.engagement_hint.clone()))
            else {
                continue;
            };

            let distinct_hints: BTreeSet<EngagementId> = component
                .iter()
                .filter_map(|revision| captures.get(revision).map(|c| c.engagement_hint.clone()))
                .collect();
            if distinct_hints.len() > 1 {
                let merged = distinct_hints
                    .iter()
                    .map(EngagementId::as_str)
                    .collect::<Vec<_>>()
                    .join(", ");
                diagnostics.push(ProjectionDiagnostic {
                    code: ENGAGEMENTS_MERGED_CODE.to_owned(),
                    message: format!(
                        "a capture bridged separate engagements, now merged: {merged}"
                    ),
                });
            }

            let heads: BTreeSet<RevisionId> =
                component.intersection(&view.heads).cloned().collect();
            let lifecycle = engagement_lifecycle(events, &heads, &captures)?;

            engagements.push(EngagementView {
                engagement_id: canonical,
                revisions: component.clone(),
                heads,
                lifecycle,
            });
        }

        Ok(Self {
            engagements,
            diagnostics,
        })
    }
}

struct RevisionCapture {
    object_id: ObjectId,
    journal_id: JournalId,
    engagement_hint: EngagementId,
}

/// Map each captured revision to its content object, journal, and stored
/// engagement hint, discriminating the generative arm: only a review-domain
/// revision is grouped; a task-attempt proposal in a mixed log is skipped.
fn revision_captures(events: &[ShoreEvent]) -> Result<BTreeMap<RevisionId, RevisionCapture>> {
    let mut captures = BTreeMap::new();
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::WorkObjectProposed)
    {
        let payload: WorkObjectProposedPayload = serde_json::from_value(event.payload.clone())?;
        if let WorkObjectProposal::Revision { revision, .. } = payload.work_object {
            captures.insert(
                revision.id.clone(),
                RevisionCapture {
                    object_id: revision.object_id,
                    journal_id: event.target.journal_id.clone(),
                    engagement_hint: payload.engagement_id,
                },
            );
        }
    }
    Ok(captures)
}

/// The Review-domain terminal: a single current head whose current-assessment
/// projection resolves to one un-replaced `Accepted`. Competing heads, an
/// unassessed head, or an ambiguous/non-accepted resolution keep the engagement
/// in progress. Task engagements have no V1 terminal, so a component with no
/// review head resolves in progress.
fn engagement_lifecycle(
    events: &[ShoreEvent],
    heads: &BTreeSet<RevisionId>,
    captures: &BTreeMap<RevisionId, RevisionCapture>,
) -> Result<EngagementLifecycle> {
    if heads.len() != 1 {
        return Ok(EngagementLifecycle::InProgress);
    }
    let head = heads.iter().next().expect("one head");
    let Some(capture) = captures.get(head) else {
        return Ok(EngagementLifecycle::InProgress);
    };

    let resolved = ResolvedRevision {
        journal_id: capture.journal_id.clone(),
        revision_id: head.clone(),
        object_id: capture.object_id.clone(),
    };
    let (current, _) = project_assessments(AssessmentProjectionOptions {
        // A status-only projection: `include_summary: false` means no summary
        // artifact is read, so no backend handle is needed.
        backend: None,
        events,
        resolved: &resolved,
        track_filter: None,
        include_summary: false,
        include_all: false,
    })?;

    Ok(match current.status {
        CurrentAssessmentStatus::Resolved(ReviewAssessment::Accepted) => {
            EngagementLifecycle::Accepted
        }
        _ => EngagementLifecycle::InProgress,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        AssessmentId, EngagementId, JournalId, ObjectId, ReviewEndpoint, ReviewTargetRef,
        RevisionId, RevisionSource, TargetRef, TrackId, WorktreeCaptureMode,
    };
    use crate::session::event::{
        EventTarget, EventType, GitProvenance, ReviewAssessment, ReviewAssessmentRecordedPayload,
        Revision, ShoreEvent, WorkObjectProposal, WorkObjectProposedPayload, Writer,
    };

    fn rev(suffix: &str) -> RevisionId {
        RevisionId::new(format!("rev:sha256:{suffix}"))
    }

    fn engagement(suffix: &str) -> EngagementId {
        EngagementId::new(format!("engagement:sha256:{suffix}"))
    }

    fn capture_event(
        suffix: &str,
        engagement_suffix: &str,
        supersedes: Vec<RevisionId>,
    ) -> ShoreEvent {
        let revision_id = rev(suffix);
        ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("work_object_proposed:{}", revision_id.as_str()),
            EventTarget::for_revision(JournalId::new("journal:default"), revision_id.clone(), None),
            Writer::shore_local("test"),
            WorkObjectProposedPayload {
                engagement_id: engagement(engagement_suffix),
                work_object: WorkObjectProposal::Revision {
                    revision: Revision {
                        id: revision_id,
                        object_id: ObjectId::new(format!("obj:sha256:{suffix}")),
                        git_provenance: Some(GitProvenance {
                            source: RevisionSource::GitWorktree {
                                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                                include_untracked: true,
                            },
                            base: ReviewEndpoint::GitCommit {
                                commit_oid: "base".to_owned(),
                                tree_oid: "base-tree".to_owned(),
                            },
                            target: ReviewEndpoint::GitWorkingTree {
                                worktree_root: "/repo".to_owned(),
                            },
                        }),
                    },
                    object_artifact_content_hash: format!("sha256:artifact:{suffix}"),
                    supersedes,
                },
            },
            "2026-06-04T00:00:00Z",
        )
        .unwrap()
    }

    fn accepted_event(revision_suffix: &str, assessment_suffix: &str) -> ShoreEvent {
        let revision_id = rev(revision_suffix);
        let track_id = TrackId::new("agent:tester");
        let assessment_id = AssessmentId::new(format!("assess:sha256:{assessment_suffix}"));
        let target = ReviewTargetRef::Revision {
            revision_id: revision_id.clone(),
        };
        ShoreEvent::new(
            EventType::ReviewAssessmentRecorded,
            ReviewAssessmentRecordedPayload::idempotency_key(
                &revision_id,
                &track_id,
                assessment_id.as_str(),
            ),
            EventTarget::for_subject(
                JournalId::new("journal:default"),
                TargetRef::Review(target.clone()),
                Some(track_id),
            ),
            Writer::shore_local("test"),
            ReviewAssessmentRecordedPayload {
                assessment_id,
                target,
                assessment: ReviewAssessment::Accepted,
                summary: None,
                summary_artifact_path: None,
                summary_byte_size: None,
                summary_content_hash: None,
                replaces_assessment_ids: vec![],
                related_observation_ids: vec![],
                related_input_request_ids: vec![],
            },
            "2026-06-04T00:00:01Z",
        )
        .unwrap()
    }

    #[test]
    fn groups_revisions_by_supersession_component() {
        // Thread 1: A <- {B, C}; Thread 2 (unrelated): Z.
        let events = vec![
            capture_event("a", "a", vec![]),
            capture_event("b", "a", vec![rev("a")]),
            capture_event("c", "a", vec![rev("a")]),
            capture_event("z", "z", vec![]),
        ];
        let grouping = EngagementGrouping::from_events(&events).unwrap();

        assert_eq!(grouping.engagements.len(), 2);
        let thread = grouping
            .engagements
            .iter()
            .find(|e| e.revisions.contains(&rev("a")))
            .unwrap();
        assert_eq!(
            thread.revisions,
            [rev("a"), rev("b"), rev("c")].into_iter().collect()
        );
        assert_eq!(thread.heads, [rev("b"), rev("c")].into_iter().collect());
        // Two competing heads keep the engagement in progress.
        assert_eq!(thread.lifecycle, EngagementLifecycle::InProgress);
        // Unrelated thread Z is its own engagement.
        assert!(
            grouping
                .engagements
                .iter()
                .any(|e| e.revisions == [rev("z")].into_iter().collect())
        );
    }

    #[test]
    fn a_bridge_across_engagements_merges_them() {
        // Two roots in separate engagements, then a bridge superseding both.
        let events = vec![
            capture_event("a", "a", vec![]),
            capture_event("b", "b", vec![]),
            capture_event("m", "a", vec![rev("a"), rev("b")]),
        ];
        let grouping = EngagementGrouping::from_events(&events).unwrap();

        // The connected component unifies all three.
        assert_eq!(grouping.engagements.len(), 1);
        assert_eq!(
            grouping.engagements[0].revisions,
            [rev("a"), rev("b"), rev("m")].into_iter().collect()
        );
        assert!(
            grouping
                .diagnostics
                .iter()
                .any(|d| d.code == ENGAGEMENTS_MERGED_CODE)
        );
    }

    #[test]
    fn a_dangling_supersedes_target_surfaces_a_self_healing_diagnostic() {
        let events = vec![capture_event("b", "b", vec![rev("missing")])];
        let grouping = EngagementGrouping::from_events(&events).unwrap();

        assert!(grouping.diagnostics.iter().any(|d| d.code
            == crate::session::projection::supersession::SUPERSESSION_TARGET_MISSING_CODE));
        // The dangling capture is still a valid head in its own engagement.
        assert_eq!(grouping.engagements.len(), 1);
        assert_eq!(
            grouping.engagements[0].heads,
            [rev("b")].into_iter().collect()
        );
    }

    #[test]
    fn a_single_accepted_head_resolves_the_engagement_as_accepted() {
        let events = vec![capture_event("a", "a", vec![]), accepted_event("a", "one")];
        let grouping = EngagementGrouping::from_events(&events).unwrap();

        assert_eq!(grouping.engagements.len(), 1);
        assert_eq!(
            grouping.engagements[0].lifecycle,
            EngagementLifecycle::Accepted
        );
    }

    #[test]
    fn an_unassessed_single_head_stays_in_progress() {
        let events = vec![capture_event("a", "a", vec![])];
        let grouping = EngagementGrouping::from_events(&events).unwrap();

        assert_eq!(
            grouping.engagements[0].lifecycle,
            EngagementLifecycle::InProgress
        );
    }

    #[test]
    fn an_accepted_but_superseded_revision_does_not_make_the_engagement_terminal() {
        // A accepted, then superseded by B (B unassessed): the head is B, not the
        // accepted A, so the engagement stays in progress.
        let events = vec![
            capture_event("a", "a", vec![]),
            accepted_event("a", "one"),
            capture_event("b", "a", vec![rev("a")]),
        ];
        let grouping = EngagementGrouping::from_events(&events).unwrap();

        assert_eq!(
            grouping.engagements[0].heads,
            [rev("b")].into_iter().collect()
        );
        assert_eq!(
            grouping.engagements[0].lifecycle,
            EngagementLifecycle::InProgress
        );
    }
}
