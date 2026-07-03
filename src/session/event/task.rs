use serde::{Deserialize, Serialize};

use super::kind::EventType;
use super::payload::EventPayload;
use crate::model::{CheckpointId, ObservationId, WorkObjectId, WorkObjectType};

/// Which conversation participant produced the source message this event was
/// translated from. A fact about the source conversation, recorded by the
/// adapter that owns the payload — not a fact about the durable-event writer.
/// See docs/adr/adr-0007-writer-act-vocabulary.md.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceSpeaker {
    User,
    Agent,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskCheckpointCapturedPayload {
    pub checkpoint_id: CheckpointId,
    pub parent_task_attempt_id: WorkObjectId,
    pub assistant_message_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_use_ids: Vec<String>,
    /// Opaque fingerprint of the code state at this checkpoint. Compared as a
    /// string by the resumption projection's freshness rule; carries no
    /// semantics beyond `==` equality.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_speaker: Option<SourceSpeaker>,
}

impl TaskCheckpointCapturedPayload {
    pub fn idempotency_key_for_work_object(
        work_object_id: &WorkObjectId,
        work_object_type: WorkObjectType,
        source_key: &str,
    ) -> String {
        let kind = match work_object_type {
            WorkObjectType::Revision => "revision",
            WorkObjectType::TaskAttempt => "task_attempt",
        };
        format!(
            "task_checkpoint_captured:{}:{}:{}",
            work_object_id.as_str(),
            kind,
            source_key
        )
    }
}

impl EventPayload for TaskCheckpointCapturedPayload {
    fn event_type(&self) -> EventType {
        EventType::TaskCheckpointCaptured
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskObservationRecordedPayload {
    pub observation_id: ObservationId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<CheckpointId>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_artifact_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_byte_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_content_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_speaker: Option<SourceSpeaker>,
}

impl TaskObservationRecordedPayload {
    pub fn idempotency_key_for_work_object(
        work_object_id: &WorkObjectId,
        work_object_type: WorkObjectType,
        source_key: &str,
    ) -> String {
        let kind = match work_object_type {
            WorkObjectType::Revision => "revision",
            WorkObjectType::TaskAttempt => "task_attempt",
        };
        format!(
            "task_observation_recorded:{}:{}:{}",
            work_object_id.as_str(),
            kind,
            source_key
        )
    }
}

impl EventPayload for TaskObservationRecordedPayload {
    fn event_type(&self) -> EventType {
        EventType::TaskObservationRecorded
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        CheckpointId, JournalId, ObservationId, RevisionId, TargetRef, TaskTargetRef, TrackId,
        WorkObjectId, WorkObjectType,
    };
    use crate::session::event::{
        EventPayload, EventTarget, EventType, ReviewObservationRecordedPayload, ShoreEvent, Writer,
    };

    fn task_attempt_subject() -> TargetRef {
        TargetRef::Task(TaskTargetRef::TaskAttempt {
            task_attempt_id: WorkObjectId::new("task-attempt:sha256:ta"),
        })
    }

    fn sample_checkpoint_payload() -> TaskCheckpointCapturedPayload {
        TaskCheckpointCapturedPayload {
            checkpoint_id: CheckpointId::new("checkpoint:sha256:cp"),
            parent_task_attempt_id: WorkObjectId::new("task-attempt:sha256:ta"),
            assistant_message_id: "msg_1".to_owned(),
            tool_use_ids: vec!["tu_1".to_owned(), "tu_2".to_owned()],
            checkpoint_fingerprint: None,
            source_speaker: None,
        }
    }

    #[test]
    fn task_checkpoint_captured_payload_round_trips_through_serde() {
        let payload = sample_checkpoint_payload();
        let json = serde_json::to_string(&payload).unwrap();
        let round: TaskCheckpointCapturedPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(round, payload);
    }

    #[test]
    fn task_checkpoint_captured_payload_serializes_camel_case_fields() {
        let json = serde_json::to_value(sample_checkpoint_payload()).unwrap();

        assert_eq!(json["checkpointId"], "checkpoint:sha256:cp");
        assert_eq!(json["parentTaskAttemptId"], "task-attempt:sha256:ta");
        assert_eq!(json["assistantMessageId"], "msg_1");
        assert_eq!(json["toolUseIds"], serde_json::json!(["tu_1", "tu_2"]));
        assert!(json.get("checkpointFingerprint").is_none());
        assert!(json.get("target").is_none());
        assert!(json.get("assertionMode").is_none());
        assert!(json.get("sourceRef").is_none());
        assert!(json.get("toolIntent").is_none());
        assert!(json.get("toolName").is_none());
        assert!(json.get("submissionId").is_none());
        assert!(json.get("relationType").is_none());
    }

    #[test]
    fn task_checkpoint_captured_payload_round_trips_checkpoint_fingerprint() {
        let payload = TaskCheckpointCapturedPayload {
            checkpoint_fingerprint: Some(
                "sha256:000000000000000000000000000000000000000000000000000000000000000b"
                    .to_owned(),
            ),
            ..sample_checkpoint_payload()
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(
            json["checkpointFingerprint"],
            "sha256:000000000000000000000000000000000000000000000000000000000000000b"
        );
        let round: TaskCheckpointCapturedPayload = serde_json::from_value(json).unwrap();
        assert_eq!(round, payload);
    }

    #[test]
    fn task_checkpoint_captured_payload_skips_empty_tool_use_ids() {
        let payload = TaskCheckpointCapturedPayload {
            tool_use_ids: Vec::new(),
            ..sample_checkpoint_payload()
        };

        let json = serde_json::to_value(&payload).unwrap();
        assert!(json.get("toolUseIds").is_none());
    }

    #[test]
    fn task_checkpoint_captured_idempotency_key_for_work_object_uses_substrate_form() {
        let key = TaskCheckpointCapturedPayload::idempotency_key_for_work_object(
            &WorkObjectId::new("task-attempt:sha256:ta"),
            WorkObjectType::TaskAttempt,
            "checkpoint:sha256:cp",
        );
        assert_eq!(
            key,
            "task_checkpoint_captured:task-attempt:sha256:ta:task_attempt:checkpoint:sha256:cp"
        );
    }

    #[test]
    fn task_checkpoint_captured_idempotency_key_does_not_collide_with_observation() {
        let checkpoint_key = TaskCheckpointCapturedPayload::idempotency_key_for_work_object(
            &WorkObjectId::new("shared"),
            WorkObjectType::TaskAttempt,
            "source-1",
        );
        let observation_key = TaskObservationRecordedPayload::idempotency_key_for_work_object(
            &WorkObjectId::new("shared"),
            WorkObjectType::TaskAttempt,
            "source-1",
        );
        assert_ne!(checkpoint_key, observation_key);
    }

    #[test]
    fn task_checkpoint_captured_payload_reports_matching_event_type() {
        assert_eq!(
            sample_checkpoint_payload().event_type(),
            EventType::TaskCheckpointCaptured
        );
    }

    #[test]
    fn task_checkpoint_captured_event_builds_with_envelope_checkpoint_target() {
        let target = EventTarget::for_subject(
            JournalId::new("journal:claude:uuid-1"),
            task_attempt_subject(),
            None,
        )
        .unwrap();
        let idempotency_key = TaskCheckpointCapturedPayload::idempotency_key_for_work_object(
            &WorkObjectId::new("task-attempt:sha256:ta"),
            WorkObjectType::TaskAttempt,
            "checkpoint:sha256:cp",
        );

        let event = ShoreEvent::new(
            EventType::TaskCheckpointCaptured,
            idempotency_key,
            target,
            Writer::shore_local("test"),
            sample_checkpoint_payload(),
            "2026-05-18T00:00:00Z",
        )
        .unwrap();

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["target"]["subject"]["task"]["kind"], "task_attempt");
        assert_eq!(json["payload"]["checkpointId"], "checkpoint:sha256:cp");
        assert_eq!(
            json["payload"]["parentTaskAttemptId"],
            "task-attempt:sha256:ta"
        );
    }

    #[test]
    fn task_checkpoint_captured_payload_has_no_tool_intent_field() {
        let json = serde_json::to_value(sample_checkpoint_payload()).unwrap();
        let keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert!(
            !keys.iter().any(|k| k.contains("oolIntent")),
            "no toolIntent field; got keys {keys:?}"
        );
        assert!(
            !keys.iter().any(|k| k.eq_ignore_ascii_case("toolName")),
            "no toolName field; got keys {keys:?}"
        );
    }

    fn sample_observation_payload() -> TaskObservationRecordedPayload {
        TaskObservationRecordedPayload {
            observation_id: ObservationId::new("obs:sha256:o1"),
            checkpoint_id: Some(CheckpointId::new("checkpoint:sha256:cp")),
            title: "tool_result: Bash".to_owned(),
            body: None,
            body_artifact_path: None,
            body_byte_size: None,
            body_content_hash: None,
            source_speaker: None,
        }
    }

    #[test]
    fn task_observation_recorded_payload_round_trips_through_serde() {
        let payload = sample_observation_payload();
        let json = serde_json::to_string(&payload).unwrap();
        let round: TaskObservationRecordedPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(round, payload);
    }

    #[test]
    fn task_observation_recorded_payload_serializes_camel_case_fields() {
        let json = serde_json::to_value(sample_observation_payload()).unwrap();

        assert_eq!(json["observationId"], "obs:sha256:o1");
        assert_eq!(json["checkpointId"], "checkpoint:sha256:cp");
        assert_eq!(json["title"], "tool_result: Bash");
        assert!(json.get("body").is_none());
        assert!(json.get("bodyArtifactPath").is_none());
        assert!(json.get("bodyByteSize").is_none());
        assert!(json.get("bodyContentHash").is_none());
        assert!(json.get("error").is_none());
        assert!(json.get("severity").is_none());
        assert!(json.get("toolIntent").is_none());
        assert!(json.get("toolName").is_none());
        assert!(json.get("assertionMode").is_none());
        assert!(json.get("sourceRef").is_none());
        assert!(json.get("submissionId").is_none());
        assert!(json.get("relationType").is_none());
    }

    #[test]
    fn task_observation_recorded_payload_omits_checkpoint_id_for_task_attempt_targeted_observation()
    {
        let payload = TaskObservationRecordedPayload {
            checkpoint_id: None,
            ..sample_observation_payload()
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert!(json.get("checkpointId").is_none());
    }

    #[test]
    fn task_observation_recorded_idempotency_key_for_work_object_uses_substrate_form() {
        let key = TaskObservationRecordedPayload::idempotency_key_for_work_object(
            &WorkObjectId::new("task-attempt:sha256:ta"),
            WorkObjectType::TaskAttempt,
            "obs:sha256:o1",
        );
        assert_eq!(
            key,
            "task_observation_recorded:task-attempt:sha256:ta:task_attempt:obs:sha256:o1"
        );
    }

    #[test]
    fn task_observation_recorded_idempotency_key_does_not_collide_with_review_observation_recorded()
    {
        let task_key = TaskObservationRecordedPayload::idempotency_key_for_work_object(
            &WorkObjectId::new("shared"),
            WorkObjectType::TaskAttempt,
            "shared-source",
        );
        let review_key = ReviewObservationRecordedPayload::idempotency_key(
            &RevisionId::new("shared"),
            &TrackId::new("agent:codex"),
            "shared-source",
        );
        assert_ne!(task_key, review_key);
    }

    #[test]
    fn task_observation_recorded_payload_reports_matching_event_type() {
        assert_eq!(
            sample_observation_payload().event_type(),
            EventType::TaskObservationRecorded
        );
    }

    fn assert_source_speaker_round_trip<P>(payload: &P, expected: &str)
    where
        P: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_value(payload).unwrap();
        assert_eq!(json["sourceSpeaker"], expected);
        let back: P = serde_json::from_value(json).unwrap();
        assert_eq!(&back, payload);
    }

    #[test]
    fn task_payloads_round_trip_source_speaker() {
        let checkpoint = TaskCheckpointCapturedPayload {
            source_speaker: Some(SourceSpeaker::Agent),
            ..sample_checkpoint_payload()
        };
        assert_source_speaker_round_trip(&checkpoint, "agent");

        let observation = TaskObservationRecordedPayload {
            source_speaker: Some(SourceSpeaker::Agent),
            ..sample_observation_payload()
        };
        assert_source_speaker_round_trip(&observation, "agent");
    }

    #[test]
    fn task_payloads_omit_source_speaker_when_absent() {
        let payload = TaskCheckpointCapturedPayload {
            source_speaker: None,
            ..sample_checkpoint_payload()
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert!(json.get("sourceSpeaker").is_none());

        // Pre-relocation JSON shape (no sourceSpeaker) still deserializes.
        let legacy = serde_json::json!({
            "checkpointId": "checkpoint:sha256:cp",
            "parentTaskAttemptId": "task-attempt:sha256:ta",
            "assistantMessageId": "msg_1",
        });
        let back: TaskCheckpointCapturedPayload = serde_json::from_value(legacy).unwrap();
        assert_eq!(back.source_speaker, None);
    }

    #[test]
    fn task_observation_recorded_event_builds_through_shore_event_new() {
        let target = EventTarget::for_subject(
            JournalId::new("journal:claude:uuid-1"),
            task_attempt_subject(),
            None,
        )
        .unwrap();
        let idempotency_key = TaskObservationRecordedPayload::idempotency_key_for_work_object(
            &WorkObjectId::new("task-attempt:sha256:ta"),
            WorkObjectType::TaskAttempt,
            "obs:sha256:o1",
        );

        let event = ShoreEvent::new(
            EventType::TaskObservationRecorded,
            idempotency_key,
            target,
            Writer::shore_local("test"),
            sample_observation_payload(),
            "2026-05-18T00:00:00Z",
        )
        .unwrap();

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["eventType"], "task_observation_recorded");
        assert_eq!(json["target"]["subject"]["task"]["kind"], "task_attempt");
        assert_eq!(json["payload"]["observationId"], "obs:sha256:o1");
    }
}
