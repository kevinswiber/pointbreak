use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::AssessmentTargetSelector;
use super::target::resolve_assessment_target;
use super::util::sorted_unique;
use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::error::{Result, ShoreError};
use crate::model::{
    AssessmentId, EventId, InterventionId, ObservationId, ReviewTargetRef, ReviewUnitId, TargetRef,
    TrackId,
};
use crate::session::event::{
    EventTarget, EventType, InterventionRequestedPayload, ReviewAssessment,
    ReviewAssessmentRecordedPayload, ReviewObservationRecordedPayload, ShoreEvent,
};
use crate::session::observation::{resolve_review_unit, staged_body, validated_track_id};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store_init::{ShoreStorePaths, prepare_shore_writer};
use crate::session::{EventStore, EventWriteOutcome, current_timestamp, reviewer_from_git_config};
use crate::storage::{Durability, LocalStorage};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssessmentAddOptions {
    repo: PathBuf,
    review_unit_id: Option<ReviewUnitId>,
    track: Option<String>,
    assessment: Option<ReviewAssessment>,
    summary: Option<String>,
    target: AssessmentTargetSelector,
    replaces_assessment_ids: Vec<AssessmentId>,
    related_observation_ids: Vec<ObservationId>,
    related_intervention_ids: Vec<InterventionId>,
    idempotency_key: Option<String>,
}

impl AssessmentAddOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_unit_id: None,
            track: None,
            assessment: None,
            summary: None,
            target: AssessmentTargetSelector::review_unit(),
            replaces_assessment_ids: Vec::new(),
            related_observation_ids: Vec::new(),
            related_intervention_ids: Vec::new(),
            idempotency_key: None,
        }
    }

    pub fn with_review_unit_id(mut self, id: ReviewUnitId) -> Self {
        self.review_unit_id = Some(id);
        self
    }

    pub fn with_track(mut self, track: impl Into<String>) -> Self {
        self.track = Some(track.into());
        self
    }

    pub fn with_assessment(mut self, assessment: ReviewAssessment) -> Self {
        self.assessment = Some(assessment);
        self
    }

    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    pub fn with_target(mut self, target: ReviewTargetRef) -> Self {
        self.target = AssessmentTargetSelector::direct(target);
        self
    }

    pub fn with_target_selector(mut self, target: AssessmentTargetSelector) -> Self {
        self.target = target;
        self
    }

    pub fn replacing(mut self, assessment_id: AssessmentId) -> Self {
        self.replaces_assessment_ids.push(assessment_id);
        self
    }

    pub fn related_observation(mut self, observation_id: ObservationId) -> Self {
        self.related_observation_ids.push(observation_id);
        self
    }

    pub fn related_intervention(mut self, intervention_id: InterventionId) -> Self {
        self.related_intervention_ids.push(intervention_id);
        self
    }

    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssessmentAddResult {
    pub review_unit_id: ReviewUnitId,
    pub assessment_id: AssessmentId,
    pub event_id: EventId,
    pub track_id: TrackId,
    pub target: ReviewTargetRef,
    pub assessment: ReviewAssessment,
    pub summary_content_hash: Option<String>,
    pub events_created: usize,
    pub events_existing: usize,
    pub events_created_by_type: BTreeMap<String, usize>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn record_assessment(options: AssessmentAddOptions) -> Result<AssessmentAddResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let worktree_root = paths.worktree_root();
    let shore_dir = paths.shore_dir();
    let storage = LocalStorage::new(shore_dir);
    prepare_shore_writer(&paths, &storage)?;

    let event_store = EventStore::open(shore_dir);
    let events = event_store.list_events()?;
    let resolved = resolve_review_unit(&events, options.review_unit_id.as_ref())?;
    let target = resolve_assessment_target(worktree_root, &events, &resolved, &options.target)?;
    let track_id = validated_track_id(options.track.as_deref().ok_or_else(|| {
        ShoreError::WorkflowInputInvalid {
            reason: "track is required".to_owned(),
        }
    })?)?;
    let assessment = options
        .assessment
        .ok_or_else(|| ShoreError::WorkflowInputInvalid {
            reason: "assessment is required".to_owned(),
        })?;

    validate_assessment_relationships(
        &events,
        &resolved.review_unit_id,
        &options.replaces_assessment_ids,
        &options.related_observation_ids,
        &options.related_intervention_ids,
    )?;

    let writer = reviewer_from_git_config(worktree_root);
    let summary_content_hash = options
        .summary
        .as_ref()
        .map(|summary| format!("sha256:{}", sha256_bytes_hex(summary.as_bytes())));
    let (summary, summary_artifact_path, summary_artifact_bytes, summary_byte_size) =
        staged_body(options.summary.as_deref())?;
    let replaces_assessment_ids = sorted_unique(options.replaces_assessment_ids);
    let related_observation_ids = sorted_unique(options.related_observation_ids);
    let related_intervention_ids = sorted_unique(options.related_intervention_ids);
    let assessment_id = build_assessment_id(AssessmentIdMaterial {
        review_unit_id: &resolved.review_unit_id,
        track_id: &track_id,
        target: &target,
        assessment,
        summary_content_hash: summary_content_hash.as_deref(),
        replaces_assessment_ids: &replaces_assessment_ids,
        related_observation_ids: &related_observation_ids,
        related_intervention_ids: &related_intervention_ids,
        writer_actor_id: writer.actor_id.as_str(),
    })?;
    let source_key = options
        .idempotency_key
        .as_deref()
        .unwrap_or_else(|| assessment_id.as_str());
    let idempotency_key = ReviewAssessmentRecordedPayload::idempotency_key(
        &resolved.review_unit_id,
        &track_id,
        source_key,
    );

    if !event_store.event_exists(&idempotency_key)?
        && let (Some(artifact_path), Some(bytes)) = (
            summary_artifact_path.as_deref(),
            summary_artifact_bytes.as_ref(),
        )
    {
        storage.write_bytes_atomic(Path::new(artifact_path), bytes, Durability::Durable)?;
    }

    let event = ShoreEvent::new(
        EventType::ReviewAssessmentRecorded,
        idempotency_key,
        EventTarget {
            session_id: resolved.session_id,
            work_unit_id: None,
            work_object_id: None,
            work_object_type: None,
            review_unit_id: Some(resolved.review_unit_id.clone()),
            revision_id: Some(resolved.revision_id),
            snapshot_id: Some(resolved.snapshot_id),
            track_id: Some(track_id.clone()),
            subject: Some(TargetRef::Review(target.clone())),
        },
        writer,
        ReviewAssessmentRecordedPayload {
            assessment_id: assessment_id.clone(),
            target: target.clone(),
            assessment,
            summary,
            summary_artifact_path,
            summary_byte_size,
            summary_content_hash: summary_content_hash.clone(),
            replaces_assessment_ids,
            related_observation_ids,
            related_intervention_ids,
        },
        current_timestamp(),
    )?;
    let event_id = event.event_id.clone();

    let mut events_created_by_type = BTreeMap::new();
    let outcome = event_store.record_event_once(&event)?;
    let (events_created, events_existing) = match outcome {
        EventWriteOutcome::Created => {
            events_created_by_type.insert("review_assessment_recorded".to_owned(), 1);
            (1, 0)
        }
        EventWriteOutcome::Existing => (0, 1),
    };

    let state = SessionState::from_prior_events_and_committed(&events, &event, outcome)?;
    storage.write_json_atomic(&paths.state_path(), &state, Durability::Projection)?;

    Ok(AssessmentAddResult {
        review_unit_id: resolved.review_unit_id,
        assessment_id,
        event_id,
        track_id,
        target,
        assessment,
        summary_content_hash,
        events_created,
        events_existing,
        events_created_by_type,
        diagnostics: state.diagnostics,
    })
}

fn validate_assessment_relationships(
    events: &[ShoreEvent],
    review_unit_id: &ReviewUnitId,
    replaces_assessment_ids: &[AssessmentId],
    related_observation_ids: &[ObservationId],
    related_intervention_ids: &[InterventionId],
) -> Result<()> {
    for assessment_id in replaces_assessment_ids {
        if !has_assessment(events, review_unit_id, assessment_id)? {
            return Err(ShoreError::Message(format!(
                "unknown assessment: {}",
                assessment_id.as_str()
            )));
        }
    }
    for observation_id in related_observation_ids {
        if !has_observation(events, review_unit_id, observation_id)? {
            return Err(ShoreError::Message(format!(
                "unknown observation: {}",
                observation_id.as_str()
            )));
        }
    }
    for intervention_id in related_intervention_ids {
        if !has_intervention(events, review_unit_id, intervention_id)? {
            return Err(ShoreError::Message(format!(
                "unknown intervention: {}",
                intervention_id.as_str()
            )));
        }
    }
    Ok(())
}

fn has_assessment(
    events: &[ShoreEvent],
    review_unit_id: &ReviewUnitId,
    assessment_id: &AssessmentId,
) -> Result<bool> {
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewAssessmentRecorded)
    {
        if event.target.review_unit_id.as_ref() != Some(review_unit_id) {
            continue;
        }
        let payload: ReviewAssessmentRecordedPayload =
            serde_json::from_value(event.payload.clone())?;
        if &payload.assessment_id == assessment_id {
            return Ok(true);
        }
    }
    Ok(false)
}

fn has_observation(
    events: &[ShoreEvent],
    review_unit_id: &ReviewUnitId,
    observation_id: &ObservationId,
) -> Result<bool> {
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewObservationRecorded)
    {
        if event.target.review_unit_id.as_ref() != Some(review_unit_id) {
            continue;
        }
        let payload: ReviewObservationRecordedPayload =
            serde_json::from_value(event.payload.clone())?;
        if &payload.observation_id == observation_id {
            return Ok(true);
        }
    }
    Ok(false)
}

fn has_intervention(
    events: &[ShoreEvent],
    review_unit_id: &ReviewUnitId,
    intervention_id: &InterventionId,
) -> Result<bool> {
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::InterventionRequested)
    {
        if event.target.review_unit_id.as_ref() != Some(review_unit_id) {
            continue;
        }
        let payload: InterventionRequestedPayload = serde_json::from_value(event.payload.clone())?;
        if &payload.intervention_id == intervention_id {
            return Ok(true);
        }
    }
    Ok(false)
}

struct AssessmentIdMaterial<'a> {
    review_unit_id: &'a ReviewUnitId,
    track_id: &'a TrackId,
    target: &'a ReviewTargetRef,
    assessment: ReviewAssessment,
    summary_content_hash: Option<&'a str>,
    replaces_assessment_ids: &'a [AssessmentId],
    related_observation_ids: &'a [ObservationId],
    related_intervention_ids: &'a [InterventionId],
    writer_actor_id: &'a str,
}

fn build_assessment_id(material: AssessmentIdMaterial<'_>) -> Result<AssessmentId> {
    let mut replaces = material
        .replaces_assessment_ids
        .iter()
        .map(|assessment_id| assessment_id.as_str())
        .collect::<Vec<_>>();
    replaces.sort();
    let mut related_observations = material
        .related_observation_ids
        .iter()
        .map(|observation_id| observation_id.as_str())
        .collect::<Vec<_>>();
    related_observations.sort();
    let mut related_interventions = material
        .related_intervention_ids
        .iter()
        .map(|intervention_id| intervention_id.as_str())
        .collect::<Vec<_>>();
    related_interventions.sort();

    let digest = sha256_json_prefixed(&json!({
        "reviewUnitId": material.review_unit_id.as_str(),
        "trackId": material.track_id.as_str(),
        "target": material.target,
        "assessment": material.assessment,
        "summaryContentHash": material.summary_content_hash,
        "replacesAssessmentIds": replaces,
        "relatedObservationIds": related_observations,
        "relatedInterventionIds": related_interventions,
        "writerActorId": material.writer_actor_id,
    }))?;
    Ok(AssessmentId::new(format!("assess:{digest}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_assessment_id_uses_stable_material_digest() {
        const EXPECTED_ASSESSMENT_ID_FOR_FIXTURE: &str =
            "assess:sha256:91d7fb904f67c97690fdb39363246526b3001203f4addc94248c7095fbaea677";

        let review_unit_id = ReviewUnitId::new("review-unit:sha256:one");
        let track_id = TrackId::new("human:kevin");
        let target = ReviewTargetRef::ReviewUnit {
            review_unit_id: review_unit_id.clone(),
        };

        let assessment_id = build_assessment_id(AssessmentIdMaterial {
            review_unit_id: &review_unit_id,
            track_id: &track_id,
            target: &target,
            assessment: ReviewAssessment::Accepted,
            summary_content_hash: Some("sha256:summary"),
            replaces_assessment_ids: &[],
            related_observation_ids: &[],
            related_intervention_ids: &[],
            writer_actor_id: "human:kevin",
        })
        .unwrap();

        assert_eq!(assessment_id.as_str(), EXPECTED_ASSESSMENT_ID_FOR_FIXTURE);
    }
}
