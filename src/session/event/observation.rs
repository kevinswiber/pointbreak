use serde::{Deserialize, Serialize};

use super::kind::EventType;
use super::payload::EventPayload;
use crate::model::{ObservationId, ReviewTargetRef, RevisionId, TrackId};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewObservationRecordedPayload {
    pub observation_id: ObservationId,
    pub target: ReviewTargetRef,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_artifact_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_byte_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supersedes_observation_ids: Vec<ObservationId>,
}

impl ReviewObservationRecordedPayload {
    pub fn idempotency_key(
        review_unit_id: &RevisionId,
        track_id: &TrackId,
        source_key: &str,
    ) -> String {
        format!(
            "review_observation_recorded:{}:{}:{}",
            review_unit_id.as_str(),
            track_id.as_str(),
            source_key
        )
    }
}

impl EventPayload for ReviewObservationRecordedPayload {
    fn event_type(&self) -> EventType {
        EventType::ReviewObservationRecorded
    }
}
