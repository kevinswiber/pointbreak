use serde::Serialize;

use crate::model::{
    AssessmentId, EventId, InputRequestId, InputRequestResponseId, ObservationId, ReviewEndpoint,
    ReviewTargetRef, ReviewUnitId, ReviewUnitLineageBasisV1, ReviewUnitLineageId,
    ReviewUnitLineageRoundId, ReviewUnitSource, RevisionId, SessionId, SnapshotId, TrackId,
    ValidationCheckId, ValidationStatus, ValidationTarget, ValidationTrigger,
};
use crate::session::event::{
    AssertionMode, EventType, ImportedNoteTarget, InputRequestReasonCode,
    InputRequestResponseOutcome, ReviewAssessment, SidecarSource, Writer,
};
use crate::session::{EventVerificationStatus, PrincipalView};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_status: Option<EventVerificationStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal: Option<PrincipalView>,
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
    InputRequestOpened {
        input_request_id: InputRequestId,
        target: ReviewTargetRef,
        #[serde(serialize_with = "serialize_input_request_mode")]
        mode: AssertionMode,
        reason_code: InputRequestReasonCode,
        title: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        body: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        body_byte_size: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        body_content_hash: Option<String>,
    },
    InputRequestResponded {
        input_request_response_id: InputRequestResponseId,
        input_request_id: InputRequestId,
        outcome: InputRequestResponseOutcome,
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
        related_input_requests: Vec<InputRequestId>,
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
    ReviewUnitLineageDeclared {
        lineage_id: ReviewUnitLineageId,
        basis: ReviewUnitLineageBasisV1,
    },
    ReviewUnitLineageRoundRecorded {
        lineage_id: ReviewUnitLineageId,
        round_id: ReviewUnitLineageRoundId,
        review_unit_id: ReviewUnitId,
        #[serde(skip_serializing_if = "Option::is_none")]
        predecessor_review_unit_id: Option<ReviewUnitId>,
        #[serde(skip_serializing_if = "Option::is_none")]
        change_id: Option<String>,
    },
    ValidationCheckRecorded {
        validation_check_id: ValidationCheckId,
        target: ValidationTarget,
        check_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        status: ValidationStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        exit_code: Option<i64>,
        trigger: ValidationTrigger,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_fingerprint: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary_content_hash: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        started_at: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        completed_at: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        log_artifact_content_hashes: Vec<String>,
    },
}

fn serialize_input_request_mode<S>(mode: &AssertionMode, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(match mode {
        AssertionMode::Operative => "operative",
        AssertionMode::Advisory => "advisory",
    })
}
