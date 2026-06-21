use serde::Serialize;

use crate::model::{
    AssessmentId, CommitAssociationId, CommitWithdrawalId, EngagementId, EventId, InputRequestId,
    InputRequestResponseId, LedgerId, ObjectId, ObservationId, RefAssociationId, RefWithdrawalId,
    ReviewEndpoint, ReviewTargetRef, ReviewUnitSource, RevisionId, TrackId, ValidationCheckId,
    ValidationStatus, ValidationTarget, ValidationTrigger,
};
use crate::session::event::{
    AssertionMode, EventType, ImportedNoteTarget, InputRequestReasonCode,
    InputRequestResponseOutcome, ReviewAssessment, SidecarSource, Writer,
};
use crate::session::{EndorsementReadback, EventVerificationStatus, PrincipalView};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewHistoryEntry {
    pub event_id: EventId,
    pub event_type: EventType,
    pub occurred_at: String,
    pub payload_hash: String,
    pub ledger_id: LedgerId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_id: Option<TrackId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<ReviewTargetRef>,
    pub writer: Writer,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_status: Option<EventVerificationStatus>,
    /// Reader-relative endorsement readback. Plural and member-level: one entry per
    /// endorsement attestation (co-signature member), so a target co-signed by several
    /// signers — or by one actor via multiple enrolled keys — surfaces each; collapsing
    /// to one would be a silent cap.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub endorsements: Vec<EndorsementReadback>,
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
    RevisionCaptured {
        revision_id: RevisionId,
        object_id: ObjectId,
        engagement_id: EngagementId,
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<ReviewUnitSource>,
        #[serde(skip_serializing_if = "Option::is_none")]
        base: Option<ReviewEndpoint>,
        #[serde(skip_serializing_if = "Option::is_none")]
        target: Option<ReviewEndpoint>,
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
    RevisionRefAssociated {
        ref_association_id: RefAssociationId,
        ref_name: String,
        head_oid: String,
    },
    RevisionRefWithdrawn {
        ref_withdrawal_id: RefWithdrawalId,
        ref_association_id: RefAssociationId,
    },
    RevisionCommitAssociated {
        commit_association_id: CommitAssociationId,
        commit_oid: String,
        tree_oid: String,
    },
    RevisionCommitWithdrawn {
        commit_withdrawal_id: CommitWithdrawalId,
        commit_association_id: CommitAssociationId,
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
