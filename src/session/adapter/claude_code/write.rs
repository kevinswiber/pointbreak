use std::path::Path;

use super::translate::AdapterIntent;
use crate::canonical_hash::sha256_bytes_hex;
use crate::error::{Result, ShoreError};
use crate::model::{EngagementId, ObservationId, TargetRef, TaskTargetRef, WorkObjectType};
use crate::session::event::{
    EventTarget, EventType, ShoreEvent, TaskCheckpointCapturedPayload,
    TaskObservationRecordedPayload, WorkObjectProposal, WorkObjectProposedPayload,
};
use crate::session::{EventStore, EventWriteOutcome};

#[allow(dead_code)]
pub(crate) fn intent_to_event(intent: &AdapterIntent) -> Result<ShoreEvent> {
    match intent {
        AdapterIntent::TaskAttemptCaptured {
            task_attempt_id,
            session_id,
            source_ref,
            assertion_mode,
            writer,
            occurred_at,
            project_path,
            claude_session_uuid,
            initial_prompt_hash,
            predecessor,
            source_speaker,
        } => {
            let target = EventTarget::for_subject(
                session_id.clone(),
                TargetRef::Task(TaskTargetRef::TaskAttempt),
                None,
            );
            // The engagement grouping hint seeds deterministically from the
            // task attempt's own id, mirroring how the review path seeds an
            // engagement from its revision id: two captures of the same attempt
            // converge to the same engagement.
            let engagement_id = EngagementId::new(format!(
                "engagement:sha256:{}",
                sha256_bytes_hex(task_attempt_id.as_str().as_bytes())
            ));
            let payload = WorkObjectProposedPayload {
                engagement_id,
                work_object: WorkObjectProposal::TaskAttempt {
                    task_attempt_id: task_attempt_id.clone(),
                    project_path: project_path.clone(),
                    claude_session_uuid: claude_session_uuid.clone(),
                    initial_prompt_hash: initial_prompt_hash.clone(),
                    predecessor: predecessor.clone(),
                    base_snapshot_fingerprint: None,
                    source_speaker: Some(*source_speaker),
                },
            };
            // The generative move is keyed on the proposed work object's id, the
            // same scheme the review path uses for its revision proposal.
            let idempotency_key = format!("work_object_proposed:{}", task_attempt_id.as_str());
            let mut event = ShoreEvent::new(
                EventType::WorkObjectProposed,
                idempotency_key,
                target,
                writer.clone(),
                payload,
                occurred_at.clone(),
            )?;
            event.source_ref = source_ref.clone();
            event.assertion_mode = *assertion_mode;
            Ok(event)
        }
        AdapterIntent::CheckpointCaptured {
            checkpoint_id,
            parent_task_attempt_id,
            // intent.target is redundant; the envelope subject is derived from
            // checkpoint_id so a malformed intent cannot persist an event whose
            // payload names checkpoint A while the envelope subject points at
            // checkpoint B (or a non-task target).
            target: _,
            session_id,
            source_ref,
            assertion_mode,
            writer,
            occurred_at,
            assistant_message_id,
            tool_use_ids,
            source_speaker,
        } => {
            let target = EventTarget::for_subject(
                session_id.clone(),
                TargetRef::Task(TaskTargetRef::Checkpoint {
                    checkpoint_id: checkpoint_id.clone(),
                }),
                None,
            );
            let payload = TaskCheckpointCapturedPayload {
                checkpoint_id: checkpoint_id.clone(),
                parent_task_attempt_id: parent_task_attempt_id.clone(),
                assistant_message_id: assistant_message_id.clone(),
                tool_use_ids: tool_use_ids.clone(),
                checkpoint_fingerprint: None,
                source_speaker: Some(*source_speaker),
            };
            let idempotency_key = TaskCheckpointCapturedPayload::idempotency_key_for_work_object(
                parent_task_attempt_id,
                WorkObjectType::TaskAttempt,
                checkpoint_id.as_str(),
            );
            let mut event = ShoreEvent::new(
                EventType::TaskCheckpointCaptured,
                idempotency_key,
                target,
                writer.clone(),
                payload,
                occurred_at.clone(),
            )?;
            event.source_ref = source_ref.clone();
            event.assertion_mode = *assertion_mode;
            Ok(event)
        }
        AdapterIntent::ObservationRecorded {
            parent_task_attempt_id,
            target: target_ref,
            session_id,
            source_ref,
            assertion_mode,
            writer,
            occurred_at,
            title,
            source_speaker,
        } => {
            // source_ref is required: observation_id is derived from source_id
            // so replays match. Without a source_ref, two no-source observations
            // for the same task would collapse onto the same id and idempotency
            // key.
            let source_id = source_ref
                .as_ref()
                .ok_or_else(|| {
                    ShoreError::Message(
                        "ObservationRecorded intent requires source_ref for deterministic observation_id"
                            .to_owned(),
                    )
                })?
                .source_id
                .clone();
            let observation_id = ObservationId::new(format!(
                "obs:sha256:{}",
                sha256_bytes_hex(source_id.as_bytes())
            ));
            let checkpoint_id = match target_ref {
                TargetRef::Task(TaskTargetRef::Checkpoint { checkpoint_id }) => {
                    Some(checkpoint_id.clone())
                }
                TargetRef::Task(TaskTargetRef::TaskAttempt) => None,
                _ => {
                    return Err(ShoreError::Message(
                        "ObservationRecorded intent target must be TargetRef::Task(...)".to_owned(),
                    ));
                }
            };
            let target = EventTarget::for_subject(session_id.clone(), target_ref.clone(), None);
            let payload = TaskObservationRecordedPayload {
                observation_id: observation_id.clone(),
                checkpoint_id,
                title: title.clone(),
                body: None,
                body_artifact_path: None,
                body_byte_size: None,
                body_content_hash: None,
                source_speaker: Some(*source_speaker),
            };
            let idempotency_key = TaskObservationRecordedPayload::idempotency_key_for_work_object(
                parent_task_attempt_id,
                WorkObjectType::TaskAttempt,
                observation_id.as_str(),
            );
            let mut event = ShoreEvent::new(
                EventType::TaskObservationRecorded,
                idempotency_key,
                target,
                writer.clone(),
                payload,
                occurred_at.clone(),
            )?;
            event.source_ref = source_ref.clone();
            event.assertion_mode = *assertion_mode;
            Ok(event)
        }
        AdapterIntent::InputRequestRequested => Err(ShoreError::Message(
            "AdapterIntent::InputRequestRequested has no task-event write mapping".to_owned(),
        )),
    }
}

#[allow(dead_code)]
pub(crate) fn write_session_intents(
    intents: &[AdapterIntent],
    store_dir: &Path,
) -> Result<Vec<EventWriteOutcome>> {
    let store = EventStore::open(store_dir);
    intents
        .iter()
        .map(|intent| {
            let event = intent_to_event(intent)?;
            store.record_event_once(&event)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::{AdapterIntent, parse_session, translate_session};
    use super::*;
    use crate::canonical_hash::sha256_bytes_hex;
    use crate::model::{
        ActorId, CheckpointId, LedgerId, ReviewTargetRef, RevisionId, TargetRef, TaskTargetRef,
        WorkObjectId,
    };
    use crate::session::EventStore;
    use crate::session::event::{
        AssertionMode, EventType, ShoreEvent, SourceRef, SourceSpeaker, WorkObjectProposal,
        WorkObjectProposedPayload, Writer, WriterProducer,
    };

    fn writer_user_for_test() -> Writer {
        Writer {
            actor_id: ActorId::new("actor:claude_code:user"),
            producer: WriterProducer {
                name: "claude_code".to_owned(),
                version: String::new(),
            },
        }
    }

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/claude_code_session/a0ce57f0-485d-45b7-98fc-f0f13f467d72.jsonl")
    }

    fn task_attempt_intent_basic() -> AdapterIntent {
        AdapterIntent::TaskAttemptCaptured {
            task_attempt_id: WorkObjectId::new("task-attempt:sha256:ta"),
            session_id: LedgerId::new("session:claude:uuid-1"),
            source_ref: Some(SourceRef::new("claude_code", "uuid-1")),
            assertion_mode: AssertionMode::Advisory,
            writer: writer_user_for_test(),
            occurred_at: "2026-05-18T00:00:00Z".to_owned(),
            project_path: "/repo".to_owned(),
            claude_session_uuid: "uuid-1".to_owned(),
            initial_prompt_hash: "sha256:prompt".to_owned(),
            predecessor: None,
            source_speaker: SourceSpeaker::User,
        }
    }

    fn checkpoint_intent_basic() -> AdapterIntent {
        let checkpoint_id = CheckpointId::new("checkpoint:sha256:cp");
        AdapterIntent::CheckpointCaptured {
            checkpoint_id: checkpoint_id.clone(),
            parent_task_attempt_id: WorkObjectId::new("task-attempt:sha256:ta"),
            target: TargetRef::Task(TaskTargetRef::Checkpoint { checkpoint_id }),
            session_id: LedgerId::new("session:claude:uuid-1"),
            source_ref: Some(SourceRef::new("claude_code", "uuid-1#assistant:msg_1")),
            assertion_mode: AssertionMode::Advisory,
            writer: Writer::shore_local("test"),
            occurred_at: "2026-05-18T00:00:01Z".to_owned(),
            assistant_message_id: "msg_1".to_owned(),
            tool_use_ids: vec!["tu_1".to_owned()],
            source_speaker: SourceSpeaker::Agent,
        }
    }

    fn observation_intent_basic() -> AdapterIntent {
        let checkpoint_id = CheckpointId::new("checkpoint:sha256:cp");
        AdapterIntent::ObservationRecorded {
            parent_task_attempt_id: WorkObjectId::new("task-attempt:sha256:ta"),
            target: TargetRef::Task(TaskTargetRef::Checkpoint { checkpoint_id }),
            session_id: LedgerId::new("session:claude:uuid-1"),
            source_ref: Some(SourceRef::new("claude_code", "uuid-1#tool_result:tu_1")),
            assertion_mode: AssertionMode::Advisory,
            writer: Writer::shore_local("test"),
            occurred_at: "2026-05-18T00:00:02Z".to_owned(),
            title: "tool_result: Bash".to_owned(),
            source_speaker: SourceSpeaker::Agent,
        }
    }

    #[test]
    fn intent_to_event_maps_task_attempt_captured_intent_to_shore_event() {
        let intent = task_attempt_intent_basic();
        let event = intent_to_event(&intent).unwrap();

        assert_eq!(event.event_type, EventType::WorkObjectProposed);
        assert_eq!(
            event.idempotency_key,
            "work_object_proposed:task-attempt:sha256:ta"
        );
        assert_eq!(
            event.target.subject,
            TargetRef::Task(TaskTargetRef::TaskAttempt)
        );
        assert_eq!(
            event.target.ledger_id,
            LedgerId::new("session:claude:uuid-1")
        );
        assert_eq!(event.assertion_mode, AssertionMode::Advisory);
        assert_eq!(
            event.source_ref,
            Some(SourceRef::new("claude_code", "uuid-1"))
        );
        assert_eq!(event.writer.actor_id.as_str(), "actor:claude_code:user");
    }

    #[test]
    fn intent_to_event_maps_checkpoint_captured_intent_to_envelope_target() {
        let intent = checkpoint_intent_basic();
        let event = intent_to_event(&intent).unwrap();

        assert_eq!(event.event_type, EventType::TaskCheckpointCaptured);
        assert_eq!(
            event.target.subject,
            TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: CheckpointId::new("checkpoint:sha256:cp"),
            })
        );
        // The parent task-attempt id is carried by the payload, not the envelope.
        assert_eq!(
            event.target.ledger_id,
            LedgerId::new("session:claude:uuid-1")
        );

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["payload"]["checkpointId"], "checkpoint:sha256:cp");
        assert_eq!(
            json["payload"]["parentTaskAttemptId"],
            "task-attempt:sha256:ta"
        );
    }

    #[test]
    fn intent_to_event_maps_observation_recorded_intent() {
        let intent = observation_intent_basic();
        let event = intent_to_event(&intent).unwrap();

        assert_eq!(event.event_type, EventType::TaskObservationRecorded);
        assert_eq!(
            event.target.subject,
            TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: CheckpointId::new("checkpoint:sha256:cp"),
            })
        );

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["payload"]["title"], "tool_result: Bash");

        let expected_observation_id = format!(
            "obs:sha256:{}",
            sha256_bytes_hex(b"uuid-1#tool_result:tu_1")
        );
        assert_eq!(json["payload"]["observationId"], expected_observation_id);
    }

    #[test]
    fn intent_to_event_maps_source_speaker_into_payloads() {
        let event = intent_to_event(&task_attempt_intent_basic()).unwrap();
        let payload: WorkObjectProposedPayload =
            serde_json::from_value(serde_json::to_value(&event).unwrap()["payload"].clone())
                .unwrap();
        let WorkObjectProposal::TaskAttempt { source_speaker, .. } = payload.work_object else {
            panic!("task attempt capture proposes a TaskAttempt work object");
        };
        assert_eq!(source_speaker, Some(SourceSpeaker::User));

        let event = intent_to_event(&checkpoint_intent_basic()).unwrap();
        let payload: TaskCheckpointCapturedPayload =
            serde_json::from_value(serde_json::to_value(&event).unwrap()["payload"].clone())
                .unwrap();
        assert_eq!(payload.source_speaker, Some(SourceSpeaker::Agent));

        let event = intent_to_event(&observation_intent_basic()).unwrap();
        let payload: TaskObservationRecordedPayload =
            serde_json::from_value(serde_json::to_value(&event).unwrap()["payload"].clone())
                .unwrap();
        assert_eq!(payload.source_speaker, Some(SourceSpeaker::Agent));
    }

    #[test]
    fn intent_to_event_propagates_operative_assertion_mode() {
        let intent = AdapterIntent::TaskAttemptCaptured {
            task_attempt_id: WorkObjectId::new("task-attempt:sha256:ta"),
            session_id: LedgerId::new("session:claude:uuid-1"),
            source_ref: Some(SourceRef::new("claude_code", "uuid-1")),
            assertion_mode: AssertionMode::Operative,
            writer: writer_user_for_test(),
            occurred_at: "2026-05-18T00:00:00Z".to_owned(),
            project_path: "/repo".to_owned(),
            claude_session_uuid: "uuid-1".to_owned(),
            initial_prompt_hash: "sha256:prompt".to_owned(),
            predecessor: None,
            source_speaker: SourceSpeaker::User,
        };

        let event = intent_to_event(&intent).unwrap();

        assert_eq!(event.assertion_mode, AssertionMode::Operative);
    }

    #[test]
    fn intent_to_event_rejects_unhandled_input_request_requested_variant() {
        let result = intent_to_event(&AdapterIntent::InputRequestRequested);
        assert!(result.is_err(), "InputRequestRequested must not map");
    }

    #[test]
    fn intent_to_event_derives_checkpoint_subject_from_payload_id_not_intent_target() {
        // Pin: a malformed intent whose `target` names a *different* checkpoint
        // than `checkpoint_id` must not persist an event whose envelope subject
        // contradicts its payload. The mapper derives the subject from
        // checkpoint_id, ignoring intent.target.
        let payload_checkpoint = CheckpointId::new("checkpoint:sha256:from-payload");
        let stale_checkpoint = CheckpointId::new("checkpoint:sha256:stale-stub");
        let intent = AdapterIntent::CheckpointCaptured {
            checkpoint_id: payload_checkpoint.clone(),
            parent_task_attempt_id: WorkObjectId::new("task-attempt:sha256:ta"),
            target: TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: stale_checkpoint,
            }),
            session_id: LedgerId::new("session:claude:uuid-1"),
            source_ref: Some(SourceRef::new("claude_code", "uuid-1#assistant:msg_x")),
            assertion_mode: AssertionMode::Advisory,
            writer: Writer::shore_local("test"),
            occurred_at: "2026-05-18T00:00:01Z".to_owned(),
            assistant_message_id: "msg_x".to_owned(),
            tool_use_ids: vec![],
            source_speaker: SourceSpeaker::Agent,
        };

        let event = intent_to_event(&intent).unwrap();

        assert_eq!(
            event.target.subject,
            TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: payload_checkpoint,
            })
        );
    }

    #[test]
    fn intent_to_event_requires_source_ref_for_observation_recorded() {
        // Pin: observation_id is derived from source_id; without a source_ref the
        // empty-string hash would collapse two no-source observations onto one
        // observation_id / idempotency key.
        let intent = AdapterIntent::ObservationRecorded {
            parent_task_attempt_id: WorkObjectId::new("task-attempt:sha256:ta"),
            target: TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: CheckpointId::new("checkpoint:sha256:cp"),
            }),
            session_id: LedgerId::new("session:claude:uuid-1"),
            source_ref: None,
            assertion_mode: AssertionMode::Advisory,
            writer: Writer::shore_local("test"),
            occurred_at: "2026-05-18T00:00:02Z".to_owned(),
            title: "tool_result: Bash".to_owned(),
            source_speaker: SourceSpeaker::Agent,
        };

        let error = intent_to_event(&intent).expect_err("missing source_ref rejected");
        assert!(
            error.to_string().contains("source_ref"),
            "error must mention source_ref; got: {error}"
        );
    }

    #[test]
    fn write_session_intents_writes_phase_3_fixture_round_trip() {
        let parsed = parse_session(&fixture_path()).unwrap();
        let intents = translate_session(&parsed);
        let dir = tempfile::tempdir().unwrap();

        let outcomes = write_session_intents(&intents, dir.path()).unwrap();

        for outcome in &outcomes {
            assert_eq!(*outcome, crate::session::EventWriteOutcome::Created);
        }

        let events = EventStore::open(dir.path()).list_events().unwrap();
        assert_eq!(events.len(), intents.len());
        assert_eq!(
            events
                .iter()
                .filter(|e| e.event_type == EventType::WorkObjectProposed)
                .count(),
            1
        );
        let intent_checkpoint_count = intents
            .iter()
            .filter(|i| matches!(i, AdapterIntent::CheckpointCaptured { .. }))
            .count();
        assert_eq!(
            events
                .iter()
                .filter(|e| e.event_type == EventType::TaskCheckpointCaptured)
                .count(),
            intent_checkpoint_count
        );
        for event in &events {
            assert_eq!(event.assertion_mode, AssertionMode::Advisory);
            assert!(event.source_ref.is_some());
        }
    }

    #[test]
    fn write_session_intents_is_idempotent_on_replay() {
        let parsed = parse_session(&fixture_path()).unwrap();
        let intents = translate_session(&parsed);
        let dir = tempfile::tempdir().unwrap();

        let _ = write_session_intents(&intents, dir.path()).unwrap();
        let second = write_session_intents(&intents, dir.path()).unwrap();

        for outcome in &second {
            assert_eq!(*outcome, crate::session::EventWriteOutcome::Existing);
        }
        let events_after = EventStore::open(dir.path()).list_events().unwrap();
        assert_eq!(events_after.len(), intents.len());
    }

    #[test]
    fn task_event_target_subject_serializes_as_externally_tagged_target_ref() {
        let intent = checkpoint_intent_basic();
        let event: ShoreEvent = intent_to_event(&intent).unwrap();

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["target"]["subject"]["task"]["kind"], "checkpoint");
        assert_eq!(
            json["target"]["subject"]["task"]["checkpointId"],
            "checkpoint:sha256:cp"
        );
    }

    fn _suppress_unused_review_target_ref() {
        let _ = ReviewTargetRef::Revision {
            revision_id: RevisionId::new("u"),
        };
    }
}
