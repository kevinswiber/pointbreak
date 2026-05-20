// Shared view-document mappers used by review unit show and the leaf read commands.
use shore::model::ReviewTargetRef;
use shore::session::event::{
    InputRequestMode, InputRequestReasonCode, InputRequestResponseOutcome, ReviewAssessment, Writer,
};
use shore::session::{AssessmentView, CurrentAssessmentStatus, InputRequestView, ObservationView};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ObservationViewDocument {
    id: String,
    event_id: String,
    track_id: String,
    target: ReviewTargetRef,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    confidence: Option<String>,
    status: shore::session::ObservationStatus,
    supersedes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_content_hash: Option<String>,
    created_at: String,
    writer: Writer,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InterventionViewDocument {
    id: String,
    event_id: String,
    track_id: String,
    target: ReviewTargetRef,
    mode: InputRequestMode,
    reason_code: InputRequestReasonCode,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_content_hash: Option<String>,
    status: &'static str,
    resolutions: Vec<InterventionResolutionViewDocument>,
    created_at: String,
    writer: Writer,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InterventionResolutionViewDocument {
    id: String,
    event_id: String,
    outcome: InputRequestResponseOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason_content_hash: Option<String>,
    created_at: String,
    writer: Writer,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CurrentAssessmentDocument {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    assessment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    assessment: Option<ReviewAssessment>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    candidates: Vec<AssessmentViewDocument>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AssessmentViewDocument {
    id: String,
    event_id: String,
    track_id: String,
    target: ReviewTargetRef,
    assessment: ReviewAssessment,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary_content_hash: Option<String>,
    status: &'static str,
    replaces: Vec<String>,
    related_observations: Vec<String>,
    related_input_requests: Vec<String>,
    created_at: String,
    writer: Writer,
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
        }
    }
}

impl From<InputRequestView> for InterventionViewDocument {
    fn from(view: InputRequestView) -> Self {
        Self {
            id: view.id.as_str().to_owned(),
            event_id: view.event_id.as_str().to_owned(),
            track_id: view.track_id.as_str().to_owned(),
            target: view.target,
            mode: view.mode,
            reason_code: view.reason_code,
            title: view.title,
            body: view.body,
            body_content_hash: view.body_content_hash,
            status: view.status.as_str(),
            resolutions: view
                .responses
                .into_iter()
                .map(InterventionResolutionViewDocument::from)
                .collect(),
            created_at: view.created_at,
            writer: view.writer,
        }
    }
}

impl From<shore::session::InputRequestResponseView> for InterventionResolutionViewDocument {
    fn from(view: shore::session::InputRequestResponseView) -> Self {
        Self {
            id: view.id.as_str().to_owned(),
            event_id: view.event_id.as_str().to_owned(),
            outcome: view.outcome,
            reason: view.reason,
            reason_content_hash: view.reason_content_hash,
            created_at: view.created_at,
            writer: view.writer,
        }
    }
}

impl From<shore::session::CurrentAssessmentView> for CurrentAssessmentDocument {
    fn from(current: shore::session::CurrentAssessmentView) -> Self {
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
        }
    }
}
