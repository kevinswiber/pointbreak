use serde::{Deserialize, Serialize};

use super::kind::EventType;
use super::payload::EventPayload;
use crate::error::{Result, ShoreError};
use crate::model::{
    InputRequestId, InputRequestResponseId, ReviewTargetRef, ReviewUnitId, TrackId, WorkObjectId,
    WorkObjectType,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputRequestReasonCode {
    AmbiguousState,
    UnsafeAction,
    StaleRevision,
    FailedGate,
    ExternalSideEffect,
    ConflictingEvent,
    MissingPermission,
    ManualDecisionRequired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputRequestResponseOutcome {
    Approved,
    Rejected,
    Dismissed,
    Superseded,
    Abandoned,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputRequestOpenedPayload {
    pub input_request_id: InputRequestId,
    pub target: ReviewTargetRef,
    pub reason_code: InputRequestReasonCode,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_artifact_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_byte_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_content_hash: Option<String>,
    /// Opaque fingerprint of the code state the requester observed when
    /// opening this input request. Compared as a string by downstream
    /// freshness rules; carries no semantics beyond `==` equality.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_fingerprint: Option<String>,
}

pub(crate) fn decode_input_request_opened_payload(
    value: serde_json::Value,
) -> Result<InputRequestOpenedPayload> {
    if value.get("mode").is_some() {
        return Err(ShoreError::InvalidEvent {
            message: "input_request_opened payload mode is no longer supported; use envelope assertionMode"
                .to_owned(),
        });
    }

    Ok(serde_json::from_value(value)?)
}

impl InputRequestOpenedPayload {
    // The two idempotency-key constructors materialize one shared pattern:
    // `<event-kind>:<work-object-identity-in-domain-appropriate-form>:<source_key>`.
    // Review-domain identity is `(review_unit_id, track_id)`; task-domain is
    // `(work_object_id, work_object_type)`. Two serializations of one pattern --
    // callers pick the constructor that matches their work-object kind.

    pub fn idempotency_key(
        review_unit_id: &ReviewUnitId,
        track_id: &TrackId,
        source_key: &str,
    ) -> String {
        format!(
            "input_request_opened:{}:{}:{}",
            review_unit_id.as_str(),
            track_id.as_str(),
            source_key
        )
    }

    pub fn idempotency_key_for_work_object(
        work_object_id: &WorkObjectId,
        work_object_type: WorkObjectType,
        source_key: &str,
    ) -> String {
        let kind = match work_object_type {
            WorkObjectType::ReviewUnit => "review_unit",
            WorkObjectType::TaskAttempt => "task_attempt",
        };
        format!(
            "input_request_opened:{}:{}:{}",
            work_object_id.as_str(),
            kind,
            source_key
        )
    }
}

impl EventPayload for InputRequestOpenedPayload {
    fn event_type(&self) -> EventType {
        EventType::InputRequestOpened
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputRequestRespondedPayload {
    pub input_request_response_id: InputRequestResponseId,
    pub input_request_id: InputRequestId,
    pub outcome: InputRequestResponseOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_artifact_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_byte_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_content_hash: Option<String>,
    /// Opaque fingerprint of the code state the responder acted on. Compared
    /// as a string against the latest checkpoint's `checkpoint_fingerprint`
    /// by the agent-resumption projection; mismatch marks the resolution
    /// stale even when its target identity matches the latest checkpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_fingerprint: Option<String>,
}

impl InputRequestRespondedPayload {
    pub fn idempotency_key(input_request_id: &InputRequestId, source_key: &str) -> String {
        format!(
            "input_request_responded:{}:{}",
            input_request_id.as_str(),
            source_key
        )
    }
}

impl EventPayload for InputRequestRespondedPayload {
    fn event_type(&self) -> EventType {
        EventType::InputRequestResponded
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
    use crate::model::{SessionId, WorkObjectId, WorkObjectType, WorkUnitId};
    use crate::session::event::{EventTarget, ShoreEvent, Writer};

    #[test]
    fn input_request_opened_idempotency_key_uses_new_review_domain_prefix() {
        let key = InputRequestOpenedPayload::idempotency_key(
            &ReviewUnitId::new("ru-1"),
            &TrackId::new("human:kevin"),
            "source-1",
        );
        assert_eq!(key, "input_request_opened:ru-1:human:kevin:source-1");
    }

    #[test]
    fn input_request_opened_idempotency_key_for_work_object_uses_new_prefix() {
        let key = InputRequestOpenedPayload::idempotency_key_for_work_object(
            &WorkObjectId::new("task-attempt:sha256:abc"),
            WorkObjectType::TaskAttempt,
            "source-1",
        );
        assert_eq!(
            key,
            "input_request_opened:task-attempt:sha256:abc:task_attempt:source-1"
        );
    }

    #[test]
    fn input_request_response_idempotency_key_uses_new_prefix() {
        let key = InputRequestRespondedPayload::idempotency_key(
            &InputRequestId::new("input-request:sha256:abc"),
            "response-source",
        );
        assert_eq!(
            key,
            "input_request_responded:input-request:sha256:abc:response-source"
        );
    }

    #[test]
    fn idempotency_key_constructors_do_not_collide_on_shared_source_key() {
        let review = InputRequestOpenedPayload::idempotency_key(
            &ReviewUnitId::new("shared"),
            &TrackId::new("track-a"),
            "source-1",
        );
        let task = InputRequestOpenedPayload::idempotency_key_for_work_object(
            &WorkObjectId::new("shared"),
            WorkObjectType::TaskAttempt,
            "source-1",
        );
        assert_ne!(review, task);
    }

    #[test]
    fn input_request_opened_payload_no_longer_serializes_mode() {
        let payload = opened_input_request_payload();
        let json = serde_json::to_value(&payload).unwrap();

        assert!(json.get("mode").is_none(), "{json}");
    }

    #[test]
    fn legacy_input_request_payload_mode_is_rejected() {
        let legacy = serde_json::json!({
            "inputRequestId": "input-request:sha256:abc",
            "target": {
                "kind": "review_unit",
                "reviewUnitId": "review-unit:sha256:ru"
            },
            "mode": "blocking",
            "reasonCode": "manual_decision_required",
            "title": "legacy"
        });

        let error = decode_input_request_opened_payload(legacy).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("payload mode is no longer supported")
        );
    }

    #[test]
    fn input_request_opened_payload_skips_target_fingerprint_when_none() {
        let payload = opened_input_request_payload();
        let json = serde_json::to_value(&payload).unwrap();
        assert!(json.get("targetFingerprint").is_none());
    }

    #[test]
    fn input_request_opened_payload_round_trips_target_fingerprint() {
        let fp =
            "sha256:000000000000000000000000000000000000000000000000000000000000000b".to_owned();
        let payload = InputRequestOpenedPayload {
            target_fingerprint: Some(fp.clone()),
            ..opened_input_request_payload()
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["targetFingerprint"], fp);
        let round: InputRequestOpenedPayload = serde_json::from_value(json).unwrap();
        assert_eq!(round, payload);
    }

    #[test]
    fn input_request_responded_payload_skips_target_fingerprint_when_none() {
        let payload = InputRequestRespondedPayload {
            input_request_response_id: InputRequestResponseId::new(
                "input-request-response:sha256:r",
            ),
            input_request_id: InputRequestId::new("input-request:sha256:abc"),
            outcome: InputRequestResponseOutcome::Approved,
            reason: None,
            reason_artifact_path: None,
            reason_byte_size: None,
            reason_content_hash: None,
            target_fingerprint: None,
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert!(json.get("targetFingerprint").is_none());
    }

    #[test]
    fn input_request_responded_payload_round_trips_target_fingerprint() {
        let fp =
            "sha256:000000000000000000000000000000000000000000000000000000000000000c".to_owned();
        let payload = InputRequestRespondedPayload {
            input_request_response_id: InputRequestResponseId::new(
                "input-request-response:sha256:r",
            ),
            input_request_id: InputRequestId::new("input-request:sha256:abc"),
            outcome: InputRequestResponseOutcome::Approved,
            reason: None,
            reason_artifact_path: None,
            reason_byte_size: None,
            reason_content_hash: None,
            target_fingerprint: Some(fp.clone()),
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["targetFingerprint"], fp);
        let round: InputRequestRespondedPayload = serde_json::from_value(json).unwrap();
        assert_eq!(round, payload);
    }

    #[test]
    fn input_request_opened_event_hashes_pin_new_wire_shape() {
        let review_unit_id = ReviewUnitId::new("review-unit:sha256:unit");
        let track_id = TrackId::new("human:kevin");
        let target = ReviewTargetRef::ReviewUnit {
            review_unit_id: review_unit_id.clone(),
        };
        let payload = InputRequestOpenedPayload {
            input_request_id: InputRequestId::new("input-request:sha256:abc"),
            target: target.clone(),
            reason_code: InputRequestReasonCode::ManualDecisionRequired,
            title: "Need a decision".to_owned(),
            body: Some("Which path should win?".to_owned()),
            body_artifact_path: None,
            body_byte_size: Some(22),
            body_content_hash: Some("sha256:body".to_owned()),
            target_fingerprint: None,
        };
        let idempotency_key =
            InputRequestOpenedPayload::idempotency_key(&review_unit_id, &track_id, "source-1");

        let event = ShoreEvent::new(
            EventType::InputRequestOpened,
            idempotency_key.clone(),
            EventTarget::new(
                SessionId::new("session:default"),
                WorkUnitId::new("work:default"),
            ),
            Writer::shore_local("test"),
            payload,
            "2026-05-20T00:00:00Z",
        )
        .unwrap();

        assert_eq!(
            event.event_id.as_str(),
            format!(
                "evt:sha256:{}",
                sha256_bytes_hex(idempotency_key.as_bytes())
            )
        );
        assert_eq!(
            event.payload_hash,
            sha256_json_prefixed(&serde_json::json!({
                "inputRequestId": "input-request:sha256:abc",
                "target": target,
                "reasonCode": "manual_decision_required",
                "title": "Need a decision",
                "body": "Which path should win?",
                "bodyByteSize": 22,
                "bodyContentHash": "sha256:body"
            }))
            .unwrap()
        );
        assert!(event.payload.get("interventionId").is_none());
        assert_eq!(event.payload["inputRequestId"], "input-request:sha256:abc");

        let legacy_payload_hash = sha256_json_prefixed(&serde_json::json!({
            "interventionId": "intervention:sha256:abc",
            "target": target,
            "mode": "blocking",
            "reasonCode": "manual_decision_required",
            "title": "Need a decision",
            "body": "Which path should win?",
            "bodyByteSize": 22,
            "bodyContentHash": "sha256:body"
        }))
        .unwrap();
        assert_ne!(event.payload_hash, legacy_payload_hash);
    }

    #[test]
    fn input_request_responded_event_hashes_pin_new_wire_shape() {
        let payload = InputRequestRespondedPayload {
            input_request_response_id: InputRequestResponseId::new(
                "input-request-response:sha256:def",
            ),
            input_request_id: InputRequestId::new("input-request:sha256:abc"),
            outcome: InputRequestResponseOutcome::Approved,
            reason: Some("Approved locally".to_owned()),
            reason_artifact_path: None,
            reason_byte_size: Some(16),
            reason_content_hash: Some("sha256:reason".to_owned()),
            target_fingerprint: None,
        };
        let idempotency_key = InputRequestRespondedPayload::idempotency_key(
            &InputRequestId::new("input-request:sha256:abc"),
            "response-source",
        );

        let event = ShoreEvent::new(
            EventType::InputRequestResponded,
            idempotency_key.clone(),
            EventTarget::new(
                SessionId::new("session:default"),
                WorkUnitId::new("work:default"),
            ),
            Writer::shore_local("test"),
            payload,
            "2026-05-20T00:00:01Z",
        )
        .unwrap();

        assert_eq!(
            event.event_id.as_str(),
            format!(
                "evt:sha256:{}",
                sha256_bytes_hex(idempotency_key.as_bytes())
            )
        );
        assert_eq!(
            event.payload_hash,
            sha256_json_prefixed(&serde_json::json!({
                "inputRequestResponseId": "input-request-response:sha256:def",
                "inputRequestId": "input-request:sha256:abc",
                "outcome": "approved",
                "reason": "Approved locally",
                "reasonByteSize": 16,
                "reasonContentHash": "sha256:reason"
            }))
            .unwrap()
        );
        assert!(event.payload.get("interventionResolutionId").is_none());
        assert_eq!(
            event.payload["inputRequestResponseId"],
            "input-request-response:sha256:def"
        );
    }

    fn opened_input_request_payload() -> InputRequestOpenedPayload {
        InputRequestOpenedPayload {
            input_request_id: InputRequestId::new("input-request:sha256:abc"),
            target: ReviewTargetRef::ReviewUnit {
                review_unit_id: ReviewUnitId::new("ru-1"),
            },
            reason_code: InputRequestReasonCode::ManualDecisionRequired,
            title: "t".to_owned(),
            body: None,
            body_artifact_path: None,
            body_byte_size: None,
            body_content_hash: None,
            target_fingerprint: None,
        }
    }
}
