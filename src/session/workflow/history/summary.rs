use serde::Serialize;

use crate::model::{
    AssessmentId, EventId, InterventionId, InterventionResolutionId, ObservationId, ReviewEndpoint,
    ReviewTargetRef, ReviewUnitId, ReviewUnitSource, RevisionId, SessionId, SnapshotId, TrackId,
};
use crate::session::event::{
    EventType, ImportedNoteTarget, InterventionMode, InterventionReasonCode,
    InterventionResolutionOutcome, ReviewAssessment, SidecarSource, Writer,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewHistoryEntry {
    pub event_id: EventId,
    pub event_type: EventType,
    pub occurred_at: String,
    pub payload_hash: String,
    pub session_id: SessionId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_unit_id: Option<ReviewUnitId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<RevisionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<SnapshotId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_id: Option<TrackId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<ReviewTargetRef>,
    pub writer: Writer,
    pub summary: ReviewHistorySummary,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ReviewHistorySummary {
    ReviewInitialized {},
    ReviewUnitCaptured {
        review_unit_id: ReviewUnitId,
        source: ReviewUnitSource,
        base: ReviewEndpoint,
        target: ReviewEndpoint,
        revision_id: RevisionId,
        snapshot_id: SnapshotId,
        snapshot_artifact_content_hash: String,
    },
    ReviewObservationRecorded {
        observation_id: ObservationId,
        target: ReviewTargetRef,
        title: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        body: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        body_byte_size: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        body_content_hash: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        tags: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        confidence: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        supersedes: Vec<ObservationId>,
    },
    InterventionRequested {
        intervention_id: InterventionId,
        target: ReviewTargetRef,
        mode: InterventionMode,
        reason_code: InterventionReasonCode,
        title: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        body: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        body_byte_size: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        body_content_hash: Option<String>,
    },
    InterventionResolved {
        intervention_resolution_id: InterventionResolutionId,
        intervention_id: InterventionId,
        outcome: InterventionResolutionOutcome,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason_byte_size: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason_content_hash: Option<String>,
    },
    ReviewAssessmentRecorded {
        assessment_id: AssessmentId,
        target: ReviewTargetRef,
        assessment: ReviewAssessment,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary_byte_size: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary_content_hash: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        replaces: Vec<AssessmentId>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        related_observations: Vec<ObservationId>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        related_interventions: Vec<InterventionId>,
    },
    ReviewNoteImported {
        sidecar_source: SidecarSource,
        note_id: String,
        file_path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        file_old_path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        target: Option<ImportedNoteTarget>,
        title: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        body: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        body_byte_size: Option<u64>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        tags: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        confidence: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        external_source: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        author: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        created_at: Option<String>,
        sidecar_content_hash: String,
    },
}
