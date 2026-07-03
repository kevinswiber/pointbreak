use std::path::Path;

use crate::error::{Result, ShoreError};
use crate::model::{ObservationId, ReviewTargetRef, Side};
use crate::session::event::{EventType, ReviewObservationRecordedPayload, ShoreEvent};
use crate::session::observation::{
    ObservationTargetSelector, ResolvedRevision, resolve_observation_target,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputRequestTargetSelector {
    Revision,
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

impl InputRequestTargetSelector {
    pub fn revision() -> Self {
        Self::Revision
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

pub(super) fn resolve_input_request_target(
    repo: &Path,
    events: &[ShoreEvent],
    resolved: &ResolvedRevision,
    selector: &InputRequestTargetSelector,
) -> Result<ReviewTargetRef> {
    match selector {
        InputRequestTargetSelector::Revision => {
            resolve_observation_target(repo, resolved, &ObservationTargetSelector::revision())
        }
        InputRequestTargetSelector::File { path } => {
            resolve_observation_target(repo, resolved, &ObservationTargetSelector::file(path))
        }
        InputRequestTargetSelector::Range {
            path,
            side,
            start_line,
            end_line,
        } => resolve_observation_target(
            repo,
            resolved,
            &ObservationTargetSelector::range(path, *side, *start_line, *end_line),
        ),
        InputRequestTargetSelector::Observation { observation_id } => {
            resolve_native_observation_target(events, resolved, observation_id)
        }
    }
}

fn resolve_native_observation_target(
    events: &[ShoreEvent],
    resolved: &ResolvedRevision,
    observation_id: &ObservationId,
) -> Result<ReviewTargetRef> {
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewObservationRecorded)
    {
        if event.subject_revision_id()?.as_ref() != Some(&resolved.revision_id) {
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
