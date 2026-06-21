use std::path::Path;

use crate::error::{Result, ShoreError};
use crate::model::{
    AssessmentId, InputRequestId, ObservationId, ReviewTargetRef, RevisionId, Side,
};
use crate::session::event::{
    EventType, ReviewAssessmentRecordedPayload, ReviewObservationRecordedPayload, ShoreEvent,
    decode_input_request_opened_payload,
};
use crate::session::observation::{
    ObservationTargetSelector, ResolvedReviewUnit, resolve_observation_target,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssessmentTargetSelector {
    ReviewUnit,
    File {
        path: String,
    },
    Range {
        path: String,
        side: Side,
        start_line: u32,
        end_line: Option<u32>,
    },
    Observation {
        observation_id: ObservationId,
    },
    InputRequest {
        input_request_id: InputRequestId,
    },
    Assessment {
        assessment_id: AssessmentId,
    },
    Direct(ReviewTargetRef),
}

impl AssessmentTargetSelector {
    pub fn review_unit() -> Self {
        Self::ReviewUnit
    }

    pub fn file(path: impl Into<String>) -> Self {
        Self::File { path: path.into() }
    }

    pub fn range(
        path: impl Into<String>,
        side: Side,
        start_line: u32,
        end_line: Option<u32>,
    ) -> Self {
        Self::Range {
            path: path.into(),
            side,
            start_line,
            end_line,
        }
    }

    pub fn observation(observation_id: ObservationId) -> Self {
        Self::Observation { observation_id }
    }

    pub fn input_request(input_request_id: InputRequestId) -> Self {
        Self::InputRequest { input_request_id }
    }

    pub fn assessment(assessment_id: AssessmentId) -> Self {
        Self::Assessment { assessment_id }
    }

    pub fn direct(target: ReviewTargetRef) -> Self {
        Self::Direct(target)
    }
}

pub(crate) fn resolve_assessment_target(
    repo: &Path,
    events: &[ShoreEvent],
    resolved: &ResolvedReviewUnit,
    selector: &AssessmentTargetSelector,
) -> Result<ReviewTargetRef> {
    let target = match selector {
        AssessmentTargetSelector::ReviewUnit => {
            resolve_observation_target(repo, resolved, &ObservationTargetSelector::review_unit())?
        }
        AssessmentTargetSelector::File { path } => {
            resolve_observation_target(repo, resolved, &ObservationTargetSelector::file(path))?
        }
        AssessmentTargetSelector::Range {
            path,
            side,
            start_line,
            end_line,
        } => resolve_observation_target(
            repo,
            resolved,
            &ObservationTargetSelector::range(path, *side, *start_line, *end_line),
        )?,
        AssessmentTargetSelector::Observation { observation_id } => {
            resolve_observation_ref(events, resolved, observation_id)?
        }
        AssessmentTargetSelector::InputRequest { input_request_id } => {
            resolve_input_request_ref(events, resolved, input_request_id)?
        }
        AssessmentTargetSelector::Assessment { assessment_id } => {
            resolve_assessment_ref(events, resolved, assessment_id)?
        }
        AssessmentTargetSelector::Direct(target) => {
            validate_direct_target(&resolved.revision_id, target)?;
            target.clone()
        }
    };

    Ok(target)
}

fn validate_direct_target(review_unit_id: &RevisionId, target: &ReviewTargetRef) -> Result<()> {
    let target_review_unit_id = review_unit_id_for_target(target);
    if target_review_unit_id != review_unit_id {
        return Err(ShoreError::WorkflowInputInvalid {
            reason: "assessment target must belong to the selected review unit".to_owned(),
        });
    }
    Ok(())
}

fn resolve_observation_ref(
    events: &[ShoreEvent],
    resolved: &ResolvedReviewUnit,
    observation_id: &ObservationId,
) -> Result<ReviewTargetRef> {
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewObservationRecorded)
    {
        if crate::model::subject_revision_id(&event.target.subject) != Some(&resolved.revision_id) {
            continue;
        }

        let payload: ReviewObservationRecordedPayload =
            serde_json::from_value(event.payload.clone())?;
        if &payload.observation_id == observation_id {
            return Ok(ReviewTargetRef::Observation {
                revision_id: resolved.revision_id.clone(),
                observation_id: observation_id.clone(),
            });
        }
    }

    Err(ShoreError::Message(format!(
        "unknown observation target: {}",
        observation_id.as_str()
    )))
}

fn resolve_input_request_ref(
    events: &[ShoreEvent],
    resolved: &ResolvedReviewUnit,
    input_request_id: &InputRequestId,
) -> Result<ReviewTargetRef> {
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::InputRequestOpened)
    {
        if crate::model::subject_revision_id(&event.target.subject) != Some(&resolved.revision_id) {
            continue;
        }

        let payload = decode_input_request_opened_payload(event.payload.clone())?;
        if &payload.input_request_id == input_request_id {
            return Ok(ReviewTargetRef::InputRequest {
                revision_id: resolved.revision_id.clone(),
                input_request_id: input_request_id.clone(),
            });
        }
    }

    Err(ShoreError::Message(format!(
        "unknown input request target: {}",
        input_request_id.as_str()
    )))
}

fn resolve_assessment_ref(
    events: &[ShoreEvent],
    resolved: &ResolvedReviewUnit,
    assessment_id: &AssessmentId,
) -> Result<ReviewTargetRef> {
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewAssessmentRecorded)
    {
        if crate::model::subject_revision_id(&event.target.subject) != Some(&resolved.revision_id) {
            continue;
        }

        let payload: ReviewAssessmentRecordedPayload =
            serde_json::from_value(event.payload.clone())?;
        if &payload.assessment_id == assessment_id {
            return Ok(ReviewTargetRef::Assessment {
                revision_id: resolved.revision_id.clone(),
                assessment_id: assessment_id.clone(),
            });
        }
    }

    Err(ShoreError::Message(format!(
        "unknown assessment target: {}",
        assessment_id.as_str()
    )))
}

pub(crate) fn review_unit_id_for_target(target: &ReviewTargetRef) -> &RevisionId {
    match target {
        ReviewTargetRef::Revision { revision_id }
        | ReviewTargetRef::File { revision_id, .. }
        | ReviewTargetRef::Range { revision_id, .. }
        | ReviewTargetRef::Observation { revision_id, .. }
        | ReviewTargetRef::InputRequest { revision_id, .. }
        | ReviewTargetRef::Assessment { revision_id, .. }
        | ReviewTargetRef::Event { revision_id, .. } => revision_id,
        ReviewTargetRef::Lineage { .. } => {
            unreachable!("lineage targets do not identify an exact revision")
        }
    }
}
