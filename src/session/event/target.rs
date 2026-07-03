use serde::{Deserialize, Serialize};

use super::subject_id::subject_id;
use crate::error::{Result, ShoreError};
use crate::model::{
    EngagementType, JournalId, ReviewTargetRef, RevisionId, TargetRef, TrackId,
    engagement_type_of_subject,
};

/// The addressed triple every event envelope carries: the journal it files into,
/// the opaque `subjectId` it addresses, and an optional review track.
///
/// `subjectId` is a sha256 over the subject's identity-bearing fields only (seam
/// **S2**), so a future display rename of a subject's kind tag is projection-only:
/// the signed envelope binds the rename-proof id, not the structural subject. The
/// structural subject lives in the event payload and is reconstructed for display
/// by the projection ([`ShoreEvent::reconstruct_subject`](super::ShoreEvent::reconstruct_subject)).
///
/// `subjectId` is `None` only for the fieldless `TargetRef::Journal` carrier:
/// genuinely subject-less events (the detached co-signature carrier and content
/// removal) address their real target by payload content and ride the journal
/// carrier.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventTarget {
    pub journal_id: JournalId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_id: Option<TrackId>,
}

impl EventTarget {
    /// Address an explicit subject, deriving and storing only its opaque
    /// `subjectId`. Every non-journal [`TargetRef`] is self-identifying, so this
    /// serves all domains; a `TargetRef::Journal` yields no `subjectId` (use
    /// [`Self::for_journal`] for the bare carrier).
    pub fn for_subject(
        journal_id: JournalId,
        subject: TargetRef,
        track_id: Option<TrackId>,
    ) -> Result<Self> {
        Ok(Self {
            journal_id,
            subject_id: subject_id(&subject)?,
            track_id,
        })
    }

    /// Carrier for a genuinely subject-less event: the detached co-signature
    /// carrier (addresses its target by the payload `target_event_id` /
    /// `target_event_record_hash`) and content removal (addresses its blob by
    /// the payload `content_hash`). The envelope files the fact into its journal
    /// by `journal_id`; the target stays addressed by payload content and is
    /// never duplicated onto the envelope. `subjectId` is absent.
    pub fn for_journal(journal_id: JournalId) -> Self {
        Self {
            journal_id,
            subject_id: None,
            track_id: None,
        }
    }

    /// Checked constructor for a generative move: the engagement's activity
    /// (`EngagementType`) must match the subject's derived domain. A `Review`
    /// engagement cannot mint a `Task` subject and vice versa — the single
    /// domain axis enforced at the write boundary rather than asserted as a
    /// free wire field. The domain is checked against the structural `subject`
    /// before it is reduced to its opaque `subjectId`.
    pub fn for_generative_move(
        journal_id: JournalId,
        engagement_type: EngagementType,
        subject: TargetRef,
        track_id: Option<TrackId>,
    ) -> Result<Self> {
        match engagement_type_of_subject(&subject) {
            Some(subject_domain) if subject_domain == engagement_type => {
                Self::for_subject(journal_id, subject, track_id)
            }
            other => Err(ShoreError::Message(format!(
                "generative move domain mismatch: a {engagement_type:?} engagement cannot address a {other:?} subject"
            ))),
        }
    }

    /// Convenience for addressing a review-domain revision subject, optionally on
    /// a track. Sugar over [`Self::for_subject`] with the `Review(Revision)`
    /// subject — the common review-event target.
    pub fn for_revision(
        journal_id: JournalId,
        revision_id: RevisionId,
        track_id: Option<TrackId>,
    ) -> Result<Self> {
        Self::for_subject(
            journal_id,
            TargetRef::Review(ReviewTargetRef::Revision { revision_id }),
            track_id,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{TaskTargetRef, WorkObjectId};

    fn journal_id() -> JournalId {
        JournalId::new("journal:default")
    }

    fn revision_ref() -> ReviewTargetRef {
        ReviewTargetRef::Revision {
            revision_id: RevisionId::new("rev:sha256:abc"),
        }
    }

    #[test]
    fn for_subject_binds_an_opaque_subject_id_not_the_structure() {
        let target =
            EventTarget::for_subject(journal_id(), TargetRef::Review(revision_ref()), None)
                .unwrap();

        assert_eq!(target.journal_id, journal_id());
        let id = target
            .subject_id
            .as_deref()
            .expect("review subject has an id");
        assert!(id.starts_with("subject:sha256:"), "got {id}");
        assert!(target.track_id.is_none());
    }

    #[test]
    fn journal_carrier_has_no_subject_id() {
        let target = EventTarget::for_journal(journal_id());

        assert!(target.subject_id.is_none());
        assert!(target.track_id.is_none());
    }

    #[test]
    fn event_target_names_the_container_journal() {
        // The store-level container is the append-only Journal. The wire key is
        // `journalId` with the `journal:` prefix; the carrier binds no subjectId.
        let target = EventTarget::for_journal(JournalId::new("journal:claude:uuid"));

        let json = serde_json::to_value(&target).unwrap();
        assert!(json.get("journalId").is_some(), "wire key is journalId");
        assert!(
            json.get("ledgerId").is_none(),
            "legacy ledgerId key is gone"
        );
        assert!(json["journalId"].as_str().unwrap().starts_with("journal:"));
        assert!(
            json.get("subjectId").is_none(),
            "the journal carrier binds no subjectId"
        );
    }

    #[test]
    fn for_journal_binds_no_subject_id_and_round_trips() {
        let target = EventTarget::for_journal(JournalId::new("journal:fixture"));

        let json = serde_json::to_value(&target).unwrap();
        assert_eq!(json["journalId"], "journal:fixture");
        assert!(json.get("subjectId").is_none());
        assert!(
            json.get("subject").is_none(),
            "no structural subject on the wire"
        );
        assert!(json.get("trackId").is_none());

        // Path-free: the carrier files into the journal by identity, not path.
        let text = json.to_string();
        assert!(!text.contains("/Users/"));
        assert!(!text.contains("worktreeRoot"));

        let parsed: EventTarget = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, target);
    }

    #[test]
    fn signed_target_binds_subject_id_not_structure() {
        // The reshaped envelope carries the opaque subjectId (DD2) and none of the
        // structural subject tags — a future kind rename is projection-only.
        let target =
            EventTarget::for_subject(journal_id(), TargetRef::Review(revision_ref()), None)
                .unwrap();

        let json = serde_json::to_value(&target).unwrap();
        assert!(
            json["subjectId"]
                .as_str()
                .unwrap()
                .starts_with("subject:sha256:"),
            "got {}",
            json
        );
        assert!(json.get("subject").is_none(), "no structural subject");
        assert!(json.get("review").is_none());
        assert!(json.get("workObjectId").is_none());
        assert!(json.get("workObjectType").is_none());
        assert!(json.get("domain").is_none());

        let parsed: EventTarget = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, target);
    }

    #[test]
    fn for_subject_derives_a_task_subject_id_from_the_self_identifying_ref() {
        let target = EventTarget::for_subject(
            journal_id(),
            TargetRef::Task(TaskTargetRef::TaskAttempt {
                task_attempt_id: WorkObjectId::new("task-attempt:sha256:abc"),
            }),
            None,
        )
        .unwrap();

        let id = target
            .subject_id
            .as_deref()
            .expect("task subject has an id");
        assert!(id.starts_with("subject:sha256:"), "got {id}");

        // Distinct attempts derive distinct subject ids.
        let other = EventTarget::for_subject(
            journal_id(),
            TargetRef::Task(TaskTargetRef::TaskAttempt {
                task_attempt_id: WorkObjectId::new("task-attempt:sha256:def"),
            }),
            None,
        )
        .unwrap();
        assert_ne!(target.subject_id, other.subject_id);
    }

    #[test]
    fn a_review_engagement_refuses_a_task_subject() {
        let err = EventTarget::for_generative_move(
            journal_id(),
            EngagementType::Review,
            TargetRef::Task(TaskTargetRef::TaskAttempt {
                task_attempt_id: WorkObjectId::new("task-attempt:sha256:ta"),
            }),
            None,
        )
        .unwrap_err();

        assert!(matches!(err, ShoreError::Message(_)));
    }

    #[test]
    fn for_generative_move_accepts_a_matching_domain() {
        let target = EventTarget::for_generative_move(
            journal_id(),
            EngagementType::Review,
            TargetRef::Review(revision_ref()),
            None,
        )
        .unwrap();

        assert!(target.subject_id.is_some());
    }

    #[test]
    fn for_generative_move_refuses_a_journal_subject() {
        // A `Journal` carrier has no domain, so it cannot be a generative move.
        let err = EventTarget::for_generative_move(
            journal_id(),
            EngagementType::Review,
            TargetRef::Journal,
            None,
        )
        .unwrap_err();
        assert!(matches!(err, ShoreError::Message(_)));
    }

    #[test]
    fn rejects_legacy_envelope_with_no_journal_id() {
        // The old envelope shape (a sessionId/workUnitId pair) must fail to
        // deserialize: journalId is required.
        let legacy = r#"{"sessionId":"session:default","workUnitId":"work:default"}"#;
        let result: Result<EventTarget> = serde_json::from_str(legacy).map_err(Into::into);
        assert!(
            result.is_err(),
            "legacy subject-less envelope must not deserialize, got {:?}",
            result.ok()
        );
    }

    #[test]
    fn rejects_legacy_envelope_with_review_id() {
        let legacy = r#"{"reviewId":"review:default","subject":"ledger"}"#;
        let result: Result<EventTarget> = serde_json::from_str(legacy).map_err(Into::into);
        assert!(
            result.is_err(),
            "legacy reviewId envelope must not deserialize, got {:?}",
            result.ok()
        );
    }
}
