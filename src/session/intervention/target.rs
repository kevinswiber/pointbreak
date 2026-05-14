use std::path::Path;

use crate::error::{Result, ShoreError};
use crate::model::{ObservationId, ReviewTargetRef, Side};
use crate::session::event::{EventType, ReviewObservationRecordedPayload, ShoreEvent};
use crate::session::observation::{
    ObservationTargetSelector, ResolvedReviewUnit, resolve_observation_target,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InterventionTargetSelector {
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
}

impl InterventionTargetSelector {
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
}

pub(super) fn resolve_intervention_target(
    repo: &Path,
    events: &[ShoreEvent],
    resolved: &ResolvedReviewUnit,
    selector: &InterventionTargetSelector,
) -> Result<ReviewTargetRef> {
    match selector {
        InterventionTargetSelector::ReviewUnit => {
            resolve_observation_target(repo, resolved, &ObservationTargetSelector::review_unit())
        }
        InterventionTargetSelector::File { path } => {
            resolve_observation_target(repo, resolved, &ObservationTargetSelector::file(path))
        }
        InterventionTargetSelector::Range {
            path,
            side,
            start_line,
            end_line,
        } => resolve_observation_target(
            repo,
            resolved,
            &ObservationTargetSelector::range(path, *side, *start_line, *end_line),
        ),
        InterventionTargetSelector::Observation { observation_id } => {
            resolve_native_observation_target(events, resolved, observation_id)
        }
    }
}

fn resolve_native_observation_target(
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
