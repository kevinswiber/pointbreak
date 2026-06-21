use serde::{Deserialize, Serialize};

use super::kind::EventType;
use super::payload::EventPayload;
use crate::model::{
    RevisionId, TrackId, ValidationCheckId, ValidationStatus, ValidationTarget, ValidationTrigger,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationCheckRecordedPayload {
    pub validation_check_id: ValidationCheckId,
    pub target: ValidationTarget,
    pub check_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub status: ValidationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    pub trigger: ValidationTrigger,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_artifact_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_byte_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_content_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log_artifact_content_hashes: Vec<String>,
}

impl ValidationCheckRecordedPayload {
    pub fn idempotency_key(
        review_unit_id: &RevisionId,
        track_id: &TrackId,
        source_key: &str,
    ) -> String {
        format!(
            "validation_check_recorded:{}:{}:{}",
            review_unit_id.as_str(),
            track_id.as_str(),
            source_key
        )
    }
}

impl EventPayload for ValidationCheckRecordedPayload {
    fn event_type(&self) -> EventType {
        EventType::ValidationCheckRecorded
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        RevisionId, TrackId, ValidationCheckId, ValidationStatus, ValidationTarget,
        ValidationTrigger,
    };

    fn fixture_payload() -> ValidationCheckRecordedPayload {
        ValidationCheckRecordedPayload {
            validation_check_id: ValidationCheckId::new("validation:sha256:ghi"),
            target: ValidationTarget::ReviewUnit {
                review_unit_id: RevisionId::new("review-unit:sha256:def"),
            },
            check_name: "cargo test".to_owned(),
            command: None,
            status: ValidationStatus::Passed,
            exit_code: Some(0),
            trigger: ValidationTrigger::Manual,
            source_fingerprint: None,
            summary: None,
            summary_artifact_path: None,
            summary_byte_size: None,
            summary_content_hash: None,
            started_at: None,
            completed_at: None,
            log_artifact_content_hashes: vec![],
        }
    }

    #[test]
    fn validation_check_recorded_payload_round_trips_and_uses_expected_wire_keys() {
        let payload = fixture_payload();

        let value = serde_json::to_value(&payload).unwrap();
        assert!(value.get("validationCheckId").is_some());
        assert!(value.get("checkName").is_some());
        assert_eq!(value["status"], "passed");
        assert_eq!(value["trigger"], "manual");
        assert!(value.get("logArtifactContentHashes").is_none());
        assert!(value.get("summary").is_none());

        let back: ValidationCheckRecordedPayload = serde_json::from_value(value).unwrap();
        assert_eq!(back, payload);
    }

    #[test]
    fn validation_idempotency_key_is_review_unit_track_and_source_scoped() {
        let key = ValidationCheckRecordedPayload::idempotency_key(
            &RevisionId::new("review-unit:sha256:def"),
            &TrackId::new("agent:codex"),
            "validation:sha256:ghi",
        );

        assert_eq!(
            key,
            "validation_check_recorded:review-unit:sha256:def:agent:codex:validation:sha256:ghi"
        );
    }

    #[test]
    fn validation_event_payload_and_target_are_path_free() {
        let payload = fixture_payload();
        let key = ValidationCheckRecordedPayload::idempotency_key(
            &RevisionId::new("review-unit:sha256:def"),
            &TrackId::new("agent:codex"),
            payload.validation_check_id.as_str(),
        );
        let serialized = serde_json::to_string(&payload).unwrap();

        for needle in ["/Users/", "worktreeRoot", ".git", ".shore/data"] {
            assert!(
                !serialized.contains(needle),
                "payload leaked path token {needle}"
            );
            assert!(
                !key.contains(needle),
                "idempotency key leaked path token {needle}"
            );
        }
    }
}
