// Shared view-document mappers used by review unit show and the leaf read commands.
use shore::model::ReviewTargetRef;
use shore::session::event::{
    InterventionMode, InterventionReasonCode, InterventionResolutionOutcome, ReviewDisposition,
    Writer,
};
use shore::session::{
    CurrentDispositionStatus, DispositionView, InterventionView, ObservationView,
};

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
    mode: InterventionMode,
    reason_code: InterventionReasonCode,
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
    outcome: InterventionResolutionOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason_content_hash: Option<String>,
    created_at: String,
    writer: Writer,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CurrentDispositionDocument {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    disposition_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    disposition: Option<ReviewDisposition>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    candidates: Vec<DispositionViewDocument>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DispositionViewDocument {
    id: String,
    event_id: String,
    track_id: String,
    target: ReviewTargetRef,
    disposition: ReviewDisposition,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary_content_hash: Option<String>,
    status: &'static str,
    replaces: Vec<String>,
    related_observations: Vec<String>,
    related_interventions: Vec<String>,
    overrides: Vec<ReviewTargetRef>,
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

impl From<InterventionView> for InterventionViewDocument {
    fn from(view: InterventionView) -> Self {
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
                .resolutions
                .into_iter()
                .map(InterventionResolutionViewDocument::from)
                .collect(),
            created_at: view.created_at,
            writer: view.writer,
        }
    }
}

impl From<shore::session::InterventionResolutionView> for InterventionResolutionViewDocument {
    fn from(view: shore::session::InterventionResolutionView) -> Self {
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

impl From<shore::session::CurrentDispositionView> for CurrentDispositionDocument {
    fn from(current: shore::session::CurrentDispositionView) -> Self {
        let status = current.status;
        let mut dispositions = current.dispositions.into_iter();
        match status {
            CurrentDispositionStatus::None => Self {
                status: status.as_str(),
                disposition_id: None,
                disposition: None,
                candidates: Vec::new(),
            },
            CurrentDispositionStatus::Resolved => {
                let disposition = dispositions
                    .next()
                    .expect("resolved current disposition has one record");
                Self {
                    status: status.as_str(),
                    disposition_id: Some(disposition.id.as_str().to_owned()),
                    disposition: Some(disposition.disposition),
                    candidates: Vec::new(),
                }
            }
            CurrentDispositionStatus::Ambiguous => Self {
                status: status.as_str(),
                disposition_id: None,
                disposition: None,
                candidates: dispositions.map(DispositionViewDocument::from).collect(),
            },
        }
    }
}

impl From<DispositionView> for DispositionViewDocument {
    fn from(view: DispositionView) -> Self {
        Self {
            id: view.id.as_str().to_owned(),
            event_id: view.event_id.as_str().to_owned(),
            track_id: view.track_id.as_str().to_owned(),
            target: view.target,
            disposition: view.disposition,
            summary: view.summary,
            summary_content_hash: view.summary_content_hash,
            status: view.status.as_str(),
            replaces: view
                .replaces
                .into_iter()
                .map(|disposition_id| disposition_id.as_str().to_owned())
                .collect(),
            related_observations: view
                .related_observations
                .into_iter()
                .map(|observation_id| observation_id.as_str().to_owned())
                .collect(),
            related_interventions: view
                .related_interventions
                .into_iter()
                .map(|intervention_id| intervention_id.as_str().to_owned())
                .collect(),
            overrides: view.overrides,
            created_at: view.created_at,
            writer: view.writer,
        }
    }
}
