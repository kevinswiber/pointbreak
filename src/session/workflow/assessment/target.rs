use std::path::Path;

use crate::error::{Result, ShoreError};
use crate::model::{
    AssessmentId, InterventionId, ObservationId, ReviewTargetRef, ReviewUnitId, Side,
};
use crate::session::event::{
    EventType, InterventionRequestedPayload, ReviewAssessmentRecordedPayload,
    ReviewObservationRecordedPayload, ShoreEvent,
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
    Intervention {
        intervention_id: InterventionId,
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

    pub fn intervention(intervention_id: InterventionId) -> Self {
        Self::Intervention { intervention_id }
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
        AssessmentTargetSelector::Intervention { intervention_id } => {
            resolve_intervention_ref(events, resolved, intervention_id)?
        }
        AssessmentTargetSelector::Assessment { assessment_id } => {
            resolve_assessment_ref(events, resolved, assessment_id)?
        }
        AssessmentTargetSelector::Direct(target) => {
            validate_direct_target(&resolved.review_unit_id, target)?;
            target.clone()
        }
    };

    Ok(target)
}

fn validate_direct_target(review_unit_id: &ReviewUnitId, target: &ReviewTargetRef) -> Result<()> {
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
        if event.target.review_unit_id.as_ref() != Some(&resolved.review_unit_id) {
            continue;
        }

        let payload: ReviewObservationRecordedPayload =
            serde_json::from_value(event.payload.clone())?;
        if &payload.observation_id == observation_id {
            return Ok(ReviewTargetRef::Observation {
                review_unit_id: resolved.review_unit_id.clone(),
                observation_id: observation_id.clone(),
            });
        }
    }

    Err(ShoreError::Message(format!(
        "unknown observation target: {}",
        observation_id.as_str()
    )))
}

fn resolve_intervention_ref(
    events: &[ShoreEvent],
    resolved: &ResolvedReviewUnit,
    intervention_id: &InterventionId,
) -> Result<ReviewTargetRef> {
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::InterventionRequested)
    {
        if event.target.review_unit_id.as_ref() != Some(&resolved.review_unit_id) {
            continue;
        }

        let payload: InterventionRequestedPayload = serde_json::from_value(event.payload.clone())?;
        if &payload.intervention_id == intervention_id {
            return Ok(ReviewTargetRef::Intervention {
                review_unit_id: resolved.review_unit_id.clone(),
                intervention_id: intervention_id.clone(),
            });
        }
    }

    Err(ShoreError::Message(format!(
        "unknown intervention target: {}",
        intervention_id.as_str()
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
        if event.target.review_unit_id.as_ref() != Some(&resolved.review_unit_id) {
            continue;
        }

        let payload: ReviewAssessmentRecordedPayload =
            serde_json::from_value(event.payload.clone())?;
        if &payload.assessment_id == assessment_id {
            return Ok(ReviewTargetRef::Assessment {
                review_unit_id: resolved.review_unit_id.clone(),
                assessment_id: assessment_id.clone(),
            });
        }
    }

    Err(ShoreError::Message(format!(
        "unknown assessment target: {}",
        assessment_id.as_str()
    )))
}

pub(crate) fn review_unit_id_for_target(target: &ReviewTargetRef) -> &ReviewUnitId {
    match target {
        ReviewTargetRef::ReviewUnit { review_unit_id }
        | ReviewTargetRef::File { review_unit_id, .. }
        | ReviewTargetRef::Range { review_unit_id, .. }
        | ReviewTargetRef::Observation { review_unit_id, .. }
        | ReviewTargetRef::Intervention { review_unit_id, .. }
        | ReviewTargetRef::Assessment { review_unit_id, .. }
        | ReviewTargetRef::Event { review_unit_id, .. } => review_unit_id,
    }
}
