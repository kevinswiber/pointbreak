//! Reconstructing an event's structural subject from its payload.
//!
//! The signed envelope binds only an opaque `subjectId` (seam **S2**, DD2); the
//! structural subject — which `revision` a review event addresses, whether an
//! event is task-domain, the review sub-anchor for display — lives in the event
//! payload (bound by `payloadHash`) and is reconstructed here. Every family
//! already payload-carries its subject: the review families carry a `target`
//! (`ReviewTargetRef`) or a `revision` id, the task families carry their
//! `checkpointId` / work-object id, and the journal carriers address their real
//! target by payload content.
//!
//! This replaces the pre-break reads of `event.target.subject`, which no longer
//! exists on the envelope.

use super::input_request::decode_input_request_opened_payload;
use super::{
    EventType, InputRequestRespondedPayload, ReviewAssessmentRecordedPayload,
    ReviewObservationRecordedPayload, RevisionCommitAssociatedPayload,
    RevisionCommitWithdrawnPayload, RevisionRefAssociatedPayload, RevisionRefWithdrawnPayload,
    ShoreEvent, TaskCheckpointCapturedPayload, TaskObservationRecordedPayload,
    ValidationCheckRecordedPayload, WorkObjectProposal, WorkObjectProposedPayload,
};
use crate::error::{Result, ShoreError};
use crate::model::{ReviewTargetRef, RevisionId, TargetRef, TaskTargetRef, ValidationTarget};

impl ShoreEvent {
    /// The structural subject this event addresses, reconstructed from its
    /// payload. The signed envelope carries only the opaque `subjectId`, so the
    /// payload is the authoritative structural source. A genuinely subject-less
    /// carrier yields [`TargetRef::Journal`].
    pub(crate) fn reconstruct_subject(&self) -> Result<TargetRef> {
        let subject = match self.event_type {
            // Journal carriers: pre-revision journal facts, imported notes (whose
            // target is not a review sub-anchor), and the content-addressed
            // co-signature / content-removal carriers.
            EventType::ReviewInitialized
            | EventType::ReviewNoteImported
            | EventType::EventSignatureRecorded
            | EventType::ArtifactRemoved => TargetRef::Journal,

            // The generative move proposes either a revision (review) or a task
            // attempt (task); the arm discriminates the domain.
            EventType::WorkObjectProposed => {
                let payload: WorkObjectProposedPayload =
                    serde_json::from_value(self.payload.clone())?;
                match payload.work_object {
                    WorkObjectProposal::Revision { revision, .. } => {
                        TargetRef::Review(ReviewTargetRef::Revision {
                            revision_id: revision.id,
                        })
                    }
                    WorkObjectProposal::TaskAttempt {
                        task_attempt_id, ..
                    } => TargetRef::Task(TaskTargetRef::TaskAttempt { task_attempt_id }),
                }
            }

            // Review families whose payload carries the full sub-anchor verbatim.
            EventType::ReviewObservationRecorded => {
                let payload: ReviewObservationRecordedPayload =
                    serde_json::from_value(self.payload.clone())?;
                TargetRef::Review(payload.target)
            }
            EventType::ReviewAssessmentRecorded => {
                let payload: ReviewAssessmentRecordedPayload =
                    serde_json::from_value(self.payload.clone())?;
                TargetRef::Review(payload.target)
            }
            EventType::RevisionRefAssociated => {
                let payload: RevisionRefAssociatedPayload =
                    serde_json::from_value(self.payload.clone())?;
                TargetRef::Review(payload.target)
            }
            EventType::RevisionRefWithdrawn => {
                let payload: RevisionRefWithdrawnPayload =
                    serde_json::from_value(self.payload.clone())?;
                TargetRef::Review(payload.target)
            }
            EventType::RevisionCommitAssociated => {
                let payload: RevisionCommitAssociatedPayload =
                    serde_json::from_value(self.payload.clone())?;
                TargetRef::Review(payload.target)
            }
            EventType::RevisionCommitWithdrawn => {
                let payload: RevisionCommitWithdrawnPayload =
                    serde_json::from_value(self.payload.clone())?;
                TargetRef::Review(payload.target)
            }

            // Input requests: review-domain carries the sub-anchor in `target`; a
            // task-domain request carries its full task subject in `task_target`
            // (attempt or checkpoint — the distinction drives resumption freshness).
            EventType::InputRequestOpened => {
                let payload = decode_input_request_opened_payload(self.payload.clone())?;
                match payload.task_target {
                    Some(task_target) => TargetRef::Task(task_target),
                    None => TargetRef::Review(payload.target),
                }
            }
            EventType::InputRequestResponded => {
                let payload: InputRequestRespondedPayload =
                    serde_json::from_value(self.payload.clone())?;
                match (payload.revision_id, payload.task_target) {
                    (Some(revision_id), _) => TargetRef::Review(ReviewTargetRef::InputRequest {
                        revision_id,
                        input_request_id: payload.input_request_id,
                    }),
                    (None, Some(task_target)) => TargetRef::Task(task_target),
                    (None, None) => {
                        return Err(ShoreError::InvalidEvent {
                            message:
                                "input_request_responded payload carries neither a revision_id \
                                      (review) nor a task_target (task)"
                                    .to_owned(),
                        });
                    }
                }
            }

            // Validation carries its own single-variant target type.
            EventType::ValidationCheckRecorded => {
                let payload: ValidationCheckRecordedPayload =
                    serde_json::from_value(self.payload.clone())?;
                match payload.target {
                    ValidationTarget::Revision { revision_id } => {
                        TargetRef::Review(ReviewTargetRef::Revision { revision_id })
                    }
                }
            }

            // Task families carry their checkpoint id (or its absence).
            EventType::TaskCheckpointCaptured => {
                let payload: TaskCheckpointCapturedPayload =
                    serde_json::from_value(self.payload.clone())?;
                TargetRef::Task(TaskTargetRef::Checkpoint {
                    checkpoint_id: payload.checkpoint_id,
                })
            }
            EventType::TaskObservationRecorded => {
                let payload: TaskObservationRecordedPayload =
                    serde_json::from_value(self.payload.clone())?;
                match payload.checkpoint_id {
                    Some(checkpoint_id) => {
                        TargetRef::Task(TaskTargetRef::Checkpoint { checkpoint_id })
                    }
                    // The translator always records task observations under a
                    // checkpoint; an attempt-level task observation is not
                    // produced and its payload carries no attempt id to
                    // reconstruct the subject from.
                    None => {
                        return Err(ShoreError::InvalidEvent {
                            message: "task observation without a checkpoint cannot reconstruct \
                                      its subject (attempt-level task observations are unsupported)"
                                .to_owned(),
                        });
                    }
                }
            }
        };
        Ok(subject)
    }

    /// The revision this event's subject addresses, if any — the payload-borne
    /// replacement for `subject_revision_id(&event.target.subject)`.
    pub(crate) fn subject_revision_id(&self) -> Result<Option<RevisionId>> {
        Ok(crate::model::subject_revision_id(&self.reconstruct_subject()?).cloned())
    }

    /// Whether this event addresses a task-domain subject, reconstructed from its
    /// payload.
    pub(crate) fn addresses_task_subject(&self) -> Result<bool> {
        Ok(matches!(self.reconstruct_subject()?, TargetRef::Task(_)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        CheckpointId, EngagementId, InputRequestId, InputRequestResponseId, JournalId, ObjectId,
        ReviewTargetRef, RevisionId, WorkObjectId,
    };
    use crate::session::event::{
        ArtifactRemovedPayload, EventPayload, EventTarget, EventType, InputRequestOpenedPayload,
        InputRequestReasonCode, InputRequestRespondedPayload, InputRequestResponseOutcome,
        Revision, ShoreEvent, TaskCheckpointCapturedPayload, WorkObjectProposal,
        WorkObjectProposedPayload, Writer,
    };

    // reconstruct_subject reads only event_type + payload; the envelope subject is
    // gone, so a bare journal envelope is a fine stand-in for every family here.
    fn event_with<P: EventPayload>(event_type: EventType, payload: P) -> ShoreEvent {
        ShoreEvent::new(
            event_type,
            "idempotency:reconstruct-test",
            EventTarget::for_journal(JournalId::new("journal:test")),
            Writer::shore_local("test"),
            payload,
            "2026-01-01T00:00:00Z",
        )
        .expect("event builds")
    }

    fn opened_payload(task_target: Option<TaskTargetRef>) -> InputRequestOpenedPayload {
        InputRequestOpenedPayload {
            input_request_id: InputRequestId::new("input-request:sha256:ir"),
            target: ReviewTargetRef::Revision {
                revision_id: RevisionId::new("rev:sha256:r"),
            },
            task_target,
            reason_code: InputRequestReasonCode::ManualDecisionRequired,
            title: "t".to_owned(),
            body: None,
            body_content_type: Default::default(),
            body_artifact_path: None,
            body_byte_size: None,
            body_content_hash: None,
            target_fingerprint: None,
        }
    }

    fn responded_payload(
        revision_id: Option<RevisionId>,
        task_target: Option<TaskTargetRef>,
    ) -> InputRequestRespondedPayload {
        InputRequestRespondedPayload {
            input_request_response_id: InputRequestResponseId::new(
                "input-request-response:sha256:x",
            ),
            input_request_id: InputRequestId::new("input-request:sha256:ir"),
            revision_id,
            task_target,
            outcome: InputRequestResponseOutcome::Approved,
            reason: None,
            reason_content_type: Default::default(),
            reason_artifact_path: None,
            reason_byte_size: None,
            reason_content_hash: None,
            target_fingerprint: None,
        }
    }

    #[test]
    fn work_object_proposed_reconstructs_by_domain_arm() {
        let revision = event_with(
            EventType::WorkObjectProposed,
            WorkObjectProposedPayload {
                engagement_id: EngagementId::new("engagement:sha256:e"),
                work_object: WorkObjectProposal::Revision {
                    revision: Revision {
                        id: RevisionId::new("rev:sha256:r"),
                        object_id: ObjectId::new("obj:sha256:o"),
                        git_provenance: None,
                    },
                    summary: None,
                    object_artifact_content_hash: "sha256:a".to_owned(),
                    supersedes: vec![],
                },
            },
        );
        assert_eq!(
            revision.reconstruct_subject().unwrap(),
            TargetRef::Review(ReviewTargetRef::Revision {
                revision_id: RevisionId::new("rev:sha256:r"),
            })
        );

        let task = event_with(
            EventType::WorkObjectProposed,
            WorkObjectProposedPayload {
                engagement_id: EngagementId::new("engagement:sha256:e"),
                work_object: WorkObjectProposal::TaskAttempt {
                    task_attempt_id: WorkObjectId::new("task-attempt:sha256:ta"),
                    project_path: "/repo".to_owned(),
                    claude_session_uuid: "uuid".to_owned(),
                    initial_prompt_hash: "sha256:p".to_owned(),
                    predecessor: None,
                    base_state_fingerprint: None,
                    source_speaker: None,
                },
            },
        );
        assert_eq!(
            task.reconstruct_subject().unwrap(),
            TargetRef::Task(TaskTargetRef::TaskAttempt {
                task_attempt_id: WorkObjectId::new("task-attempt:sha256:ta"),
            })
        );
    }

    #[test]
    fn input_request_opened_domain_is_the_task_target() {
        let review = event_with(EventType::InputRequestOpened, opened_payload(None));
        assert_eq!(
            review.reconstruct_subject().unwrap(),
            TargetRef::Review(ReviewTargetRef::Revision {
                revision_id: RevisionId::new("rev:sha256:r"),
            })
        );

        let task = event_with(
            EventType::InputRequestOpened,
            opened_payload(Some(TaskTargetRef::TaskAttempt {
                task_attempt_id: WorkObjectId::new("task-attempt:sha256:ta"),
            })),
        );
        assert_eq!(
            task.reconstruct_subject().unwrap(),
            TargetRef::Task(TaskTargetRef::TaskAttempt {
                task_attempt_id: WorkObjectId::new("task-attempt:sha256:ta"),
            })
        );

        // A checkpoint-targeted task request round-trips as a checkpoint subject —
        // the attempt-vs-checkpoint distinction is load-bearing for freshness.
        let checkpoint = event_with(
            EventType::InputRequestOpened,
            opened_payload(Some(TaskTargetRef::Checkpoint {
                checkpoint_id: CheckpointId::new("checkpoint:sha256:c"),
            })),
        );
        assert_eq!(
            checkpoint.reconstruct_subject().unwrap(),
            TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: CheckpointId::new("checkpoint:sha256:c"),
            })
        );
    }

    #[test]
    fn input_request_responded_reconstructs_review_and_task_and_rejects_neither() {
        let review = event_with(
            EventType::InputRequestResponded,
            responded_payload(Some(RevisionId::new("rev:sha256:r")), None),
        );
        assert_eq!(
            review.reconstruct_subject().unwrap(),
            TargetRef::Review(ReviewTargetRef::InputRequest {
                revision_id: RevisionId::new("rev:sha256:r"),
                input_request_id: InputRequestId::new("input-request:sha256:ir"),
            })
        );

        let task = event_with(
            EventType::InputRequestResponded,
            responded_payload(
                None,
                Some(TaskTargetRef::Checkpoint {
                    checkpoint_id: CheckpointId::new("checkpoint:sha256:c"),
                }),
            ),
        );
        assert_eq!(
            task.reconstruct_subject().unwrap(),
            TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: CheckpointId::new("checkpoint:sha256:c"),
            })
        );

        let neither = event_with(
            EventType::InputRequestResponded,
            responded_payload(None, None),
        );
        assert!(neither.reconstruct_subject().is_err());
    }

    #[test]
    fn task_checkpoint_reconstructs_the_checkpoint_subject() {
        let event = event_with(
            EventType::TaskCheckpointCaptured,
            TaskCheckpointCapturedPayload {
                checkpoint_id: CheckpointId::new("checkpoint:sha256:c"),
                parent_task_attempt_id: WorkObjectId::new("task-attempt:sha256:ta"),
                assistant_message_id: "msg".to_owned(),
                tool_use_ids: vec![],
                checkpoint_fingerprint: None,
                source_speaker: None,
            },
        );
        assert_eq!(
            event.reconstruct_subject().unwrap(),
            TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: CheckpointId::new("checkpoint:sha256:c"),
            })
        );
    }

    #[test]
    fn journal_carrier_reconstructs_as_journal() {
        let event = event_with(
            EventType::ArtifactRemoved,
            ArtifactRemovedPayload {
                content_hash: "sha256:c".to_owned(),
            },
        );
        assert_eq!(event.reconstruct_subject().unwrap(), TargetRef::Journal);
        assert_eq!(event.subject_revision_id().unwrap(), None);
        assert!(!event.addresses_task_subject().unwrap());
    }
}
