// Shared view-document mappers used by review unit show and the leaf read commands.
use std::collections::{BTreeMap, BTreeSet};

use crate::model::{
    EventId, ReviewTargetRef, RevisionId, ValidationStatus, ValidationTarget, ValidationTrigger,
};
use crate::session::event::{
    AssertionMode, BodyContentType, InputRequestReasonCode, InputRequestResponseOutcome,
    ReviewAssessment, Writer,
};
use crate::session::{
    AssessmentView, CurrentAssessmentStatus, DelegationMap, EndorsementReadback,
    EventVerificationStatus, InputRequestView, MemberReadback, ObservationView, PrincipalView,
    ValidationCheckView, principal_view_for,
};

/// Look up the reader-relative readback for a document's event id. The side table
/// is keyed by `EventId`; the documents hold the id as a `String`, so compare by
/// `as_str()`. Shared by every `with_readback` builder.
fn readback_for<'a>(
    table: &'a BTreeMap<EventId, MemberReadback>,
    event_id: &str,
) -> Option<&'a MemberReadback> {
    table
        .iter()
        .find(|(key, _)| key.as_str() == event_id)
        .map(|(_, readback)| readback)
}

/// Resolve the principal object for a document built from `writer` at
/// `created_at`. `None` for non-agent writers; the mirror posture (`status:
/// none`) for agent writers with no map. Shared by every leaf view document.
fn resolve_document_principal(
    writer: &Writer,
    created_at: &str,
    map: Option<&DelegationMap>,
) -> Option<PrincipalView> {
    principal_view_for(&writer.actor_id, map, created_at)
}

/// Documented per-item shape for one observation.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservationViewDocument {
    id: String,
    event_id: String,
    track_id: String,
    target: ReviewTargetRef,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(skip_serializing_if = "BodyContentType::is_text_plain")]
    body_content_type: BodyContentType,
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    confidence: Option<String>,
    status: crate::session::ObservationStatus,
    supersedes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_content_hash: Option<String>,
    created_at: String,
    writer: Writer,
    #[serde(skip_serializing_if = "Option::is_none")]
    principal: Option<PrincipalView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verification_status: Option<EventVerificationStatus>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    endorsements: Vec<EndorsementReadback>,
}

/// Documented per-item shape for one input request and its responses.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputRequestViewDocument {
    id: String,
    event_id: String,
    track_id: String,
    target: ReviewTargetRef,
    mode: InputRequestAssertionModeDocument,
    reason_code: InputRequestReasonCode,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(skip_serializing_if = "BodyContentType::is_text_plain")]
    body_content_type: BodyContentType,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_content_hash: Option<String>,
    status: &'static str,
    responses: Vec<InputRequestResponseViewDocument>,
    created_at: String,
    writer: Writer,
    #[serde(skip_serializing_if = "Option::is_none")]
    principal: Option<PrincipalView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verification_status: Option<EventVerificationStatus>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    endorsements: Vec<EndorsementReadback>,
}

/// Documented per-item shape for one input-request response.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputRequestResponseViewDocument {
    id: String,
    event_id: String,
    outcome: InputRequestResponseOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "BodyContentType::is_text_plain")]
    reason_content_type: BodyContentType,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason_content_hash: Option<String>,
    created_at: String,
    writer: Writer,
    #[serde(skip_serializing_if = "Option::is_none")]
    principal: Option<PrincipalView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verification_status: Option<EventVerificationStatus>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    endorsements: Vec<EndorsementReadback>,
}

/// Documented snake_case assertion mode for input requests, shared by the
/// view documents and the list filter.
#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputRequestAssertionModeDocument {
    Operative,
    Advisory,
}

/// Documented current-assessment summary for a Revision.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentAssessmentDocument {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    assessment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    assessment: Option<ReviewAssessment>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    candidates: Vec<AssessmentViewDocument>,
}

/// Documented per-item shape for one assessment.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssessmentViewDocument {
    id: String,
    event_id: String,
    track_id: String,
    target: ReviewTargetRef,
    assessment: ReviewAssessment,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(skip_serializing_if = "BodyContentType::is_text_plain")]
    summary_content_type: BodyContentType,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary_content_hash: Option<String>,
    status: &'static str,
    replaces: Vec<String>,
    related_observations: Vec<String>,
    related_input_requests: Vec<String>,
    created_at: String,
    writer: Writer,
    #[serde(skip_serializing_if = "Option::is_none")]
    principal: Option<PrincipalView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verification_status: Option<EventVerificationStatus>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    endorsements: Vec<EndorsementReadback>,
}

/// Documented per-item shape for one advisory validation check.
///
/// Validation evidence is informational review context and does not encode
/// merge, gate, or acceptance authority.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationCheckViewDocument {
    id: String,
    event_id: String,
    track_id: String,
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
    #[serde(skip_serializing_if = "BodyContentType::is_text_plain")]
    summary_content_type: BodyContentType,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary_content_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_at: Option<String>,
    log_artifact_content_hashes: Vec<String>,
    created_at: String,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    superseded_by_revisions: BTreeSet<RevisionId>,
    writer: Writer,
    #[serde(skip_serializing_if = "Option::is_none")]
    principal: Option<PrincipalView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verification_status: Option<EventVerificationStatus>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    endorsements: Vec<EndorsementReadback>,
}

impl From<ObservationView> for ObservationViewDocument {
    fn from(view: ObservationView) -> Self {
        Self {
            id: view.id.as_str().to_owned(),
            event_id: view.event_id.as_str().to_owned(),
            track_id: view.track_id.as_str().to_owned(),
            target: view.target,
            title: view.title,
            body: view.body,
            body_content_type: view.body_content_type,
            tags: view.tags,
            confidence: view.confidence,
            status: view.status,
            supersedes: view
                .supersedes
                .into_iter()
                .map(|observation_id| observation_id.as_str().to_owned())
                .collect(),
            body_content_hash: view.body_content_hash,
            created_at: view.created_at,
            writer: view.writer,
            principal: None,
            verification_status: None,
            endorsements: Vec::new(),
        }
    }
}

impl From<InputRequestView> for InputRequestViewDocument {
    fn from(view: InputRequestView) -> Self {
        Self {
            id: view.id.as_str().to_owned(),
            event_id: view.event_id.as_str().to_owned(),
            track_id: view.track_id.as_str().to_owned(),
            target: view.target,
            mode: view.mode.into(),
            reason_code: view.reason_code,
            title: view.title,
            body: view.body,
            body_content_type: view.body_content_type,
            body_content_hash: view.body_content_hash,
            status: view.status.as_str(),
            responses: view
                .responses
                .into_iter()
                .map(InputRequestResponseViewDocument::from)
                .collect(),
            created_at: view.created_at,
            writer: view.writer,
            principal: None,
            verification_status: None,
            endorsements: Vec::new(),
        }
    }
}

impl From<AssertionMode> for InputRequestAssertionModeDocument {
    fn from(mode: AssertionMode) -> Self {
        match mode {
            AssertionMode::Operative => Self::Operative,
            AssertionMode::Advisory => Self::Advisory,
        }
    }
}

impl From<crate::session::InputRequestResponseView> for InputRequestResponseViewDocument {
    fn from(view: crate::session::InputRequestResponseView) -> Self {
        Self {
            id: view.id.as_str().to_owned(),
            event_id: view.event_id.as_str().to_owned(),
            outcome: view.outcome,
            reason: view.reason,
            reason_content_type: view.reason_content_type,
            reason_content_hash: view.reason_content_hash,
            created_at: view.created_at,
            writer: view.writer,
            principal: None,
            verification_status: None,
            endorsements: Vec::new(),
        }
    }
}

impl From<crate::session::CurrentAssessmentView> for CurrentAssessmentDocument {
    fn from(current: crate::session::CurrentAssessmentView) -> Self {
        let status = current.status;
        let mut records = current.records.into_iter();
        match status {
            CurrentAssessmentStatus::Unassessed => Self {
                status: status.as_str(),
                assessment_id: None,
                assessment: None,
                candidates: Vec::new(),
            },
            CurrentAssessmentStatus::Resolved(assessment) => {
                let record = records
                    .next()
                    .expect("resolved current assessment has one record");
                Self {
                    status: status.as_str(),
                    assessment_id: Some(record.id.as_str().to_owned()),
                    assessment: Some(assessment),
                    candidates: Vec::new(),
                }
            }
            CurrentAssessmentStatus::Ambiguous(_) => Self {
                status: status.as_str(),
                assessment_id: None,
                assessment: None,
                candidates: records.map(AssessmentViewDocument::from).collect(),
            },
        }
    }
}

impl From<AssessmentView> for AssessmentViewDocument {
    fn from(view: AssessmentView) -> Self {
        Self {
            id: view.id.as_str().to_owned(),
            event_id: view.event_id.as_str().to_owned(),
            track_id: view.track_id.as_str().to_owned(),
            target: view.target,
            assessment: view.assessment,
            summary: view.summary,
            summary_content_type: view.summary_content_type,
            summary_content_hash: view.summary_content_hash,
            status: view.status.as_str(),
            replaces: view
                .replaces
                .into_iter()
                .map(|assessment_id| assessment_id.as_str().to_owned())
                .collect(),
            related_observations: view
                .related_observations
                .into_iter()
                .map(|observation_id| observation_id.as_str().to_owned())
                .collect(),
            related_input_requests: view
                .related_input_requests
                .into_iter()
                .map(|input_request_id| input_request_id.as_str().to_owned())
                .collect(),
            created_at: view.created_at,
            writer: view.writer,
            principal: None,
            verification_status: None,
            endorsements: Vec::new(),
        }
    }
}

impl From<ValidationCheckView> for ValidationCheckViewDocument {
    fn from(view: ValidationCheckView) -> Self {
        Self {
            id: view.id.as_str().to_owned(),
            event_id: view.event_id.as_str().to_owned(),
            track_id: view.track_id.as_str().to_owned(),
            target: view.target,
            check_name: view.check_name,
            command: view.command,
            status: view.status,
            exit_code: view.exit_code,
            trigger: view.trigger,
            source_fingerprint: view.source_fingerprint,
            summary: view.summary,
            summary_content_type: view.summary_content_type,
            summary_content_hash: view.summary_content_hash,
            started_at: view.started_at,
            completed_at: view.completed_at,
            log_artifact_content_hashes: view.log_artifact_content_hashes,
            created_at: view.created_at,
            superseded_by_revisions: view.superseded_by_revisions,
            writer: view.writer,
            principal: None,
            verification_status: None,
            endorsements: Vec::new(),
        }
    }
}

impl ObservationViewDocument {
    /// Attach the resolved principal object using this document's own writer and
    /// `createdAt`. Used by the leaf list/show builders; the unit document and
    /// add paths keep the plain `From` (no principal).
    pub fn with_resolved_principal(mut self, map: Option<&DelegationMap>) -> Self {
        self.principal = resolve_document_principal(&self.writer, &self.created_at, map);
        self
    }

    /// Attach the reader-relative readback (verification status) for this
    /// document's event id, looked up in the unit-show side table.
    pub fn with_readback(mut self, table: &BTreeMap<EventId, MemberReadback>) -> Self {
        if let Some(readback) = readback_for(table, &self.event_id) {
            self.verification_status = readback.verification_status;
            self.endorsements = readback.endorsements.clone();
        }
        self
    }
}

impl InputRequestViewDocument {
    /// Attach the principal to the request and to each of its responses (every
    /// response is a separate writer at its own `createdAt`).
    pub fn with_resolved_principal(mut self, map: Option<&DelegationMap>) -> Self {
        self.principal = resolve_document_principal(&self.writer, &self.created_at, map);
        self.responses = self
            .responses
            .into_iter()
            .map(|response| response.with_resolved_principal(map))
            .collect();
        self
    }

    /// Attach the readback to the request and to each of its responses (every
    /// response is a separate event with its own status).
    pub fn with_readback(mut self, table: &BTreeMap<EventId, MemberReadback>) -> Self {
        if let Some(readback) = readback_for(table, &self.event_id) {
            self.verification_status = readback.verification_status;
            self.endorsements = readback.endorsements.clone();
        }
        self.responses = self
            .responses
            .into_iter()
            .map(|response| response.with_readback(table))
            .collect();
        self
    }
}

impl InputRequestResponseViewDocument {
    pub fn with_resolved_principal(mut self, map: Option<&DelegationMap>) -> Self {
        self.principal = resolve_document_principal(&self.writer, &self.created_at, map);
        self
    }

    pub fn with_readback(mut self, table: &BTreeMap<EventId, MemberReadback>) -> Self {
        if let Some(readback) = readback_for(table, &self.event_id) {
            self.verification_status = readback.verification_status;
            self.endorsements = readback.endorsements.clone();
        }
        self
    }
}

impl AssessmentViewDocument {
    pub fn with_resolved_principal(mut self, map: Option<&DelegationMap>) -> Self {
        self.principal = resolve_document_principal(&self.writer, &self.created_at, map);
        self
    }

    pub fn with_readback(mut self, table: &BTreeMap<EventId, MemberReadback>) -> Self {
        if let Some(readback) = readback_for(table, &self.event_id) {
            self.verification_status = readback.verification_status;
            self.endorsements = readback.endorsements.clone();
        }
        self
    }
}

impl CurrentAssessmentDocument {
    /// Enrich the ambiguous-case candidate assessments with their principals.
    /// The resolved/unassessed cases carry no candidate documents.
    pub fn with_resolved_principal(mut self, map: Option<&DelegationMap>) -> Self {
        self.candidates = self
            .candidates
            .into_iter()
            .map(|candidate| candidate.with_resolved_principal(map))
            .collect();
        self
    }
}

impl ValidationCheckViewDocument {
    pub fn with_resolved_principal(mut self, map: Option<&DelegationMap>) -> Self {
        self.principal = resolve_document_principal(&self.writer, &self.created_at, map);
        self
    }

    pub fn with_readback(mut self, table: &BTreeMap<EventId, MemberReadback>) -> Self {
        if let Some(readback) = readback_for(table, &self.event_id) {
            self.verification_status = readback.verification_status;
            self.endorsements = readback.endorsements.clone();
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_request_assertion_mode_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(InputRequestAssertionModeDocument::Operative).unwrap(),
            serde_json::json!("operative")
        );
        assert_eq!(
            serde_json::to_value(InputRequestAssertionModeDocument::from(
                AssertionMode::Advisory
            ))
            .unwrap(),
            serde_json::json!("advisory")
        );
    }
}
