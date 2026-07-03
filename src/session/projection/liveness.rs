use super::freshness::event_set_hash_for_events;
use super::state::SessionState;
use crate::error::Result;
use crate::model::RevisionId;
use crate::session::event::ShoreEvent;

/// The reach of a [`LivenessToken`]: the whole store, or a single captured
/// work object's facts.
///
/// `WorkObject` is keyed on the captured unit's identity. Narrower reaches
/// (for example one activity's connected revisions) are added as the identity
/// model grows; the token shape stays the same.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LivenessScope {
    /// Every event in the store.
    Journal,
    /// Only the events that target this captured work object.
    WorkObject(RevisionId),
}

/// A read-only attention signal over a scoped event set.
///
/// It reports *what* the scoped set currently is (`event_set_hash`), *how
/// much* it holds (`event_count`), and *how much needs attention*
/// (`diagnostic_count`). It is purely observed: it carries no instruction, no
/// selected head, and no gate, so reading it can never be a precondition for a
/// write. A reader compares a fresh token against the last one it saw and
/// decides for itself whether to look again.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LivenessToken {
    pub scope: LivenessScope,
    pub event_set_hash: String,
    pub event_count: usize,
    pub diagnostic_count: usize,
}

impl LivenessToken {
    /// Fingerprints the whole store's event set.
    pub fn for_journal(events: &[ShoreEvent]) -> Result<Self> {
        Self::over(LivenessScope::Journal, events)
    }

    /// Fingerprints only the events that target `work_object`.
    pub fn for_work_object(events: &[ShoreEvent], work_object: &RevisionId) -> Result<Self> {
        let scoped: Vec<ShoreEvent> = events
            .iter()
            .filter(|event| {
                event.subject_revision_id().ok().flatten().as_ref() == Some(work_object)
            })
            .cloned()
            .collect();
        Self::over(LivenessScope::WorkObject(work_object.clone()), &scoped)
    }

    /// Builds a token over an already-scoped event set: the content fingerprint
    /// (reusing the shared event-set hash so it stays order-independent and
    /// envelope-stable), the count, and the projection diagnostics the set
    /// raises (derived from the rebuilt state — there is no stored count).
    fn over(scope: LivenessScope, events: &[ShoreEvent]) -> Result<Self> {
        let event_set_hash = event_set_hash_for_events(events)?;
        let diagnostic_count = SessionState::from_events(events)?.diagnostics.len();
        Ok(Self {
            scope,
            event_set_hash,
            event_count: events.len(),
            diagnostic_count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        EngagementId, JournalId, ObjectId, ReviewEndpoint, RevisionId, RevisionSource,
        WorktreeCaptureMode,
    };
    use crate::session::event::{
        EventTarget, EventType, GitProvenance, Revision, WorkObjectProposal,
        WorkObjectProposedPayload, Writer,
    };

    #[test]
    fn liveness_token_is_order_independent_and_envelope_stable() {
        let events = sample_events();
        let forward = LivenessToken::for_journal(&events).unwrap();

        let mut shuffled = events.clone();
        shuffled.reverse();
        let reversed = LivenessToken::for_journal(&shuffled).unwrap();

        // Order-independent: the underlying freshness sort makes the hash a
        // property of the set, not the input order.
        assert_eq!(forward.event_set_hash, reversed.event_set_hash);
        assert_eq!(forward.event_count, events.len());
        assert_eq!(forward.diagnostic_count, 0);

        // Envelope-stable: an envelope-only change (the timestamp) leaves the
        // fingerprint untouched.
        let mut restamped = events.clone();
        restamped[0].occurred_at = "2026-05-13T15:00:00Z".to_owned();
        let restamped_token = LivenessToken::for_journal(&restamped).unwrap();
        assert_eq!(forward.event_set_hash, restamped_token.event_set_hash);
    }

    #[test]
    fn liveness_token_scopes_to_a_work_object() {
        let events = events_across_two_work_objects();
        let scoped = LivenessToken::for_work_object(&events, &work_object_a()).unwrap();
        let whole = LivenessToken::for_journal(&events).unwrap();

        assert_ne!(scoped.event_set_hash, whole.event_set_hash);
        assert!(scoped.event_count < whole.event_count);
        assert!(matches!(scoped.scope, LivenessScope::WorkObject(_)));
    }

    fn work_object_a() -> RevisionId {
        RevisionId::new("review-unit:sha256:a")
    }

    fn sample_events() -> Vec<ShoreEvent> {
        vec![
            captured_event("review-unit:sha256:a", "2026-05-10T00:00:00Z"),
            captured_event("review-unit:sha256:b", "2026-05-10T00:00:01Z"),
        ]
    }

    fn events_across_two_work_objects() -> Vec<ShoreEvent> {
        vec![
            captured_event("review-unit:sha256:a", "2026-05-10T00:00:00Z"),
            captured_event("review-unit:sha256:b", "2026-05-10T00:00:01Z"),
            captured_event("review-unit:sha256:b", "2026-05-10T00:00:02Z"),
        ]
    }

    fn captured_event(revision_id: &str, occurred_at: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("work_object_proposed:{revision_id}:{occurred_at}"),
            EventTarget::for_revision(
                JournalId::new("journal:default"),
                RevisionId::new(revision_id),
                None,
            )
            .unwrap(),
            Writer::shore_local("0.1.0"),
            WorkObjectProposedPayload {
                engagement_id: EngagementId::new(format!(
                    "engagement:sha256:{}",
                    crate::canonical_hash::sha256_bytes_hex(
                        (RevisionId::new(format!("rev:{revision_id}")))
                            .as_str()
                            .as_bytes()
                    )
                )),
                work_object: WorkObjectProposal::Revision {
                    revision: Revision {
                        id: RevisionId::new(format!("rev:{revision_id}")),
                        object_id: ObjectId::new(format!("snap:{revision_id}")),
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
                            target: ReviewEndpoint::GitWorkingTree {
                                worktree_root: "/tmp/repo".to_owned(),
                            },
                        }),
                    },
                    object_artifact_content_hash: "sha256:artifact".to_owned(),
                    supersedes: vec![],
                },
            },
            occurred_at,
        )
        .unwrap()
    }
}
