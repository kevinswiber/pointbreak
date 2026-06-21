use serde::{Deserialize, Serialize};

use super::kind::EventType;
use super::payload::EventPayload;
use crate::model::{
    AssessmentId, InputRequestId, ObservationId, ReviewTargetRef, RevisionId, TrackId,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewAssessment {
    Accepted,
    AcceptedWithFollowUp,
    NeedsChanges,
    NeedsClarification,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewAssessmentRecordedPayload {
    pub assessment_id: AssessmentId,
    pub target: ReviewTargetRef,
    pub assessment: ReviewAssessment,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_artifact_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_byte_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replaces_assessment_ids: Vec<AssessmentId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_observation_ids: Vec<ObservationId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_input_request_ids: Vec<InputRequestId>,
}

impl ReviewAssessmentRecordedPayload {
    pub fn idempotency_key(
        review_unit_id: &RevisionId,
        track_id: &TrackId,
        source_key: &str,
    ) -> String {
        format!(
            "review_assessment_recorded:{}:{}:{}",
            review_unit_id.as_str(),
            track_id.as_str(),
            source_key
        )
    }
}

impl EventPayload for ReviewAssessmentRecordedPayload {
    fn event_type(&self) -> EventType {
        EventType::ReviewAssessmentRecorded
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ReviewTargetRef, RevisionId};
    use crate::session::event::EventType;

    #[test]
    fn review_assessment_event_type_wire_string_is_review_assessment_recorded() {
        assert_eq!(
            serde_json::to_string(&EventType::ReviewAssessmentRecorded).unwrap(),
            "\"review_assessment_recorded\""
        );
    }

    #[test]
    fn review_assessment_serializes_four_variants_in_snake_case() {
        for (variant, wire) in [
            (ReviewAssessment::Accepted, "accepted"),
            (
                ReviewAssessment::AcceptedWithFollowUp,
                "accepted_with_follow_up",
            ),
            (ReviewAssessment::NeedsChanges, "needs_changes"),
            (ReviewAssessment::NeedsClarification, "needs_clarification"),
        ] {
            assert_eq!(
                serde_json::to_string(&variant).unwrap(),
                format!("\"{wire}\"")
            );
        }
    }

    #[test]
    fn review_assessment_recorded_payload_serializes_with_expected_wire_keys() {
        let payload = ReviewAssessmentRecordedPayload {
            assessment_id: AssessmentId::new("assess:sha256:one"),
            target: ReviewTargetRef::Revision {
                revision_id: RevisionId::new("review-unit:sha256:one"),
            },
            assessment: ReviewAssessment::Accepted,
            summary: Some("ship it".to_owned()),
            summary_artifact_path: None,
            summary_byte_size: None,
            summary_content_hash: Some("sha256:00".to_owned()),
            replaces_assessment_ids: vec![],
            related_observation_ids: vec![],
            related_input_request_ids: vec![],
        };

        let json = serde_json::to_value(&payload).unwrap();

        assert_eq!(json["assessmentId"], "assess:sha256:one");
        assert_eq!(json["assessment"], "accepted");
        assert_eq!(json["target"]["kind"], "revision");
        assert!(
            json.get("replacesAssessmentIds").is_none(),
            "empty Vec must be omitted"
        );
        assert!(
            json.get("overrides").is_none(),
            "no overrides field on assessment payload"
        );
    }

    #[test]
    fn review_assessment_payload_serializes_related_input_request_ids() {
        let payload = ReviewAssessmentRecordedPayload {
            assessment_id: AssessmentId::new("assess:sha256:one"),
            target: ReviewTargetRef::InputRequest {
                revision_id: RevisionId::new("review-unit:sha256:one"),
                input_request_id: InputRequestId::new("input-request:sha256:one"),
            },
            assessment: ReviewAssessment::NeedsClarification,
            summary: None,
            summary_artifact_path: None,
            summary_byte_size: None,
            summary_content_hash: None,
            replaces_assessment_ids: vec![],
            related_observation_ids: vec![],
            related_input_request_ids: vec![InputRequestId::new("input-request:sha256:one")],
        };

        let json = serde_json::to_value(&payload).unwrap();

        assert!(json.get("relatedInterventionIds").is_none());
        assert_eq!(
            json["relatedInputRequestIds"][0],
            "input-request:sha256:one"
        );
    }

    #[test]
    fn review_assessment_recorded_payload_idempotency_key_prefix() {
        let key = ReviewAssessmentRecordedPayload::idempotency_key(
            &RevisionId::new("review-unit:sha256:one"),
            &TrackId::new("human:kevin"),
            "source-key",
        );

        assert!(key.starts_with("review_assessment_recorded:"));
    }
}
