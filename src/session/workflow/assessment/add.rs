use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::AssessmentTargetSelector;
use super::target::resolve_assessment_target;
use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::crypto::EventSigner;
use crate::error::{Result, ShoreError};
use crate::model::{
    ActorId, AssessmentId, EventId, InputRequestId, ObservationId, ReviewTargetRef, RevisionId,
    TargetRef, TrackId,
};
use crate::session::event::{
    EventTarget, EventType, ReviewAssessment, ReviewAssessmentRecordedPayload,
    ReviewObservationRecordedPayload, ShoreEvent, decode_input_request_opened_payload,
};
use crate::session::observation::{
    CurrentRevisionContext, RevisionScope, RevisionSelection, resolve_revision, staged_body,
    validated_track_id,
};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store::content::ContentArtifacts;
use crate::session::store::resolution::{
    prepare_write_landing, resolve_write_store, resolve_write_validation_store,
};
use crate::session::workflow::util::sorted_unique;
use crate::session::{
    BestEffortSkipSink, EventSigningOptions, EventStore, EventWriteOutcome, current_timestamp,
    sign_event_if_requested, writer_from_options,
};
use crate::storage::{Durability, LocalStorage};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssessmentAddOptions {
    repo: PathBuf,
    revision_id: Option<RevisionId>,
    track: Option<String>,
    assessment: Option<ReviewAssessment>,
    summary: Option<String>,
    target: AssessmentTargetSelector,
    replaces_assessment_ids: Vec<AssessmentId>,
    related_observation_ids: Vec<ObservationId>,
    related_input_request_ids: Vec<InputRequestId>,
    idempotency_key: Option<String>,
    actor_id: Option<ActorId>,
    signing: EventSigningOptions,
}

impl AssessmentAddOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            revision_id: None,
            track: None,
            assessment: None,
            summary: None,
            target: AssessmentTargetSelector::revision(),
            replaces_assessment_ids: Vec::new(),
            related_observation_ids: Vec::new(),
            related_input_request_ids: Vec::new(),
            idempotency_key: None,
            actor_id: None,
            signing: EventSigningOptions::default(),
        }
    }

    /// Attribute the durable write to an explicit actor, overriding the
    /// `SHORE_ACTOR_ID` env var and the local Git identity. A malformed id is
    /// ignored (falls back to env, then Git); `None` keeps the default
    /// resolution. The chosen actor is part of the assessment's
    /// content-addressed identity.
    pub fn with_actor_id(mut self, actor_id: ActorId) -> Self {
        self.actor_id = Some(actor_id);
        self
    }

    pub fn with_revision_id(mut self, id: RevisionId) -> Self {
        self.revision_id = Some(id);
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

    pub fn related_input_request(mut self, input_request_id: InputRequestId) -> Self {
        self.related_input_request_ids.push(input_request_id);
        self
    }

    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    pub fn sign_with<S>(mut self, signer: S) -> Self
    where
        S: EventSigner + Send + Sync + 'static,
    {
        self.signing = EventSigningOptions::sign_with(signer);
        self
    }

    pub fn sign_with_best_effort<S>(mut self, signer: S, skip_sink: BestEffortSkipSink) -> Self
    where
        S: EventSigner + Send + Sync + 'static,
    {
        self.signing = EventSigningOptions::sign_with_best_effort(signer, skip_sink);
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssessmentAddResult {
    pub revision_id: RevisionId,
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
    let write_store = resolve_write_store(&options.repo)?;
    let worktree_root = write_store.worktree_root();
    let store_dir = write_store.store_dir();
    let storage = LocalStorage::new(store_dir);
    prepare_write_landing(&write_store, &storage)?;

    // The write half lands in the resolved write store (the clone-local store in
    // linked mode) and rebuilds its state.json there.
    let event_store = EventStore::from_backend(write_store.backend());

    // Validation/derivation reads resolve the writer-visible union so the unit,
    // the target, and every relationship reference (`--replaces`,
    // `--related-observation`, `--related-input-request`) validate against
    // everything the writer can see, including linked-only facts.
    let validation_store = resolve_write_validation_store(&options.repo)?;
    let validation_events = validation_store.validation_events()?;
    let resolved = resolve_revision(
        &validation_events,
        RevisionSelection::from_revision_seed(options.revision_id.as_ref()),
        &CurrentRevisionContext::for_repo(&options.repo)?,
        RevisionScope::default(),
    )?;
    let target = resolve_assessment_target(
        worktree_root,
        &validation_events,
        &resolved,
        &options.target,
    )?;
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
        &validation_events,
        &resolved.revision_id,
        &options.replaces_assessment_ids,
        &options.related_observation_ids,
        &options.related_input_request_ids,
    )?;

    let writer = writer_from_options(worktree_root, options.actor_id.as_ref());
    let summary_content_hash = options
        .summary
        .as_ref()
        .map(|summary| format!("sha256:{}", sha256_bytes_hex(summary.as_bytes())));
    let (summary, summary_artifact_path, summary_artifact_bytes, summary_byte_size) =
        staged_body(options.summary.as_deref())?;
    let replaces_assessment_ids = sorted_unique(options.replaces_assessment_ids);
    let related_observation_ids = sorted_unique(options.related_observation_ids);
    let related_input_request_ids = sorted_unique(options.related_input_request_ids);
    let assessment_id = build_assessment_id(AssessmentIdMaterial {
        revision_id: &resolved.revision_id,
        track_id: &track_id,
        target: &target,
        assessment,
        summary_content_hash: summary_content_hash.as_deref(),
        replaces_assessment_ids: &replaces_assessment_ids,
        related_observation_ids: &related_observation_ids,
        related_input_request_ids: &related_input_request_ids,
        writer_actor_id: writer.actor_id.as_str(),
    })?;
    let source_key = options
        .idempotency_key
        .as_deref()
        .unwrap_or_else(|| assessment_id.as_str());
    let idempotency_key = ReviewAssessmentRecordedPayload::idempotency_key(
        &resolved.revision_id,
        &track_id,
        source_key,
    );

    if !event_store.event_exists(&idempotency_key)?
        && let (Some(artifact_path), Some(bytes)) = (
            summary_artifact_path.as_deref(),
            summary_artifact_bytes.as_ref(),
        )
    {
        ContentArtifacts::from_backend(write_store.backend())
            .put_note_body(artifact_path, bytes)?;
    }

    let mut event = ShoreEvent::new(
        EventType::ReviewAssessmentRecorded,
        idempotency_key,
        EventTarget::for_subject(
            resolved.journal_id,
            TargetRef::Review(target.clone()),
            Some(track_id.clone()),
        ),
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
            related_input_request_ids,
        },
        current_timestamp(),
    )?;
    sign_event_if_requested(&mut event, &options.signing)?;
    let event_id = event.event_id.clone();

    let mut events_created_by_type = BTreeMap::new();
    let outcome = event_store.record_event_once(&event)?;
    let (events_created, events_existing) = match outcome {
        EventWriteOutcome::Created => {
            events_created_by_type.insert("review_assessment_recorded".to_owned(), 1);
            (1, 0)
        }
        EventWriteOutcome::Existing | EventWriteOutcome::ExistingDivergentSignature => (0, 1),
    };

    let state = SessionState::from_events(&event_store.list_events()?)?;
    storage.write_json_atomic(
        &store_dir.join("state.json"),
        &state,
        Durability::Projection,
    )?;

    let result = AssessmentAddResult {
        revision_id: resolved.revision_id,
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
    };
    Ok(result)
}

fn validate_assessment_relationships(
    events: &[ShoreEvent],
    revision_id: &RevisionId,
    replaces_assessment_ids: &[AssessmentId],
    related_observation_ids: &[ObservationId],
    related_input_request_ids: &[InputRequestId],
) -> Result<()> {
    for assessment_id in replaces_assessment_ids {
        if !has_assessment(events, revision_id, assessment_id)? {
            return Err(ShoreError::Message(format!(
                "unknown assessment: {}",
                assessment_id.as_str()
            )));
        }
    }
    for observation_id in related_observation_ids {
        if !has_observation(events, revision_id, observation_id)? {
            return Err(ShoreError::Message(format!(
                "unknown observation: {}",
                observation_id.as_str()
            )));
        }
    }
    for input_request_id in related_input_request_ids {
        if !has_input_request(events, revision_id, input_request_id)? {
            return Err(ShoreError::Message(format!(
                "unknown input request: {}",
                input_request_id.as_str()
            )));
        }
    }
    Ok(())
}

fn has_assessment(
    events: &[ShoreEvent],
    revision_id: &RevisionId,
    assessment_id: &AssessmentId,
) -> Result<bool> {
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewAssessmentRecorded)
    {
        if crate::model::subject_revision_id(&event.target.subject) != Some(revision_id) {
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
    revision_id: &RevisionId,
    observation_id: &ObservationId,
) -> Result<bool> {
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewObservationRecorded)
    {
        if crate::model::subject_revision_id(&event.target.subject) != Some(revision_id) {
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

fn has_input_request(
    events: &[ShoreEvent],
    revision_id: &RevisionId,
    input_request_id: &InputRequestId,
) -> Result<bool> {
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::InputRequestOpened)
    {
        if crate::model::subject_revision_id(&event.target.subject) != Some(revision_id) {
            continue;
        }
        let payload = decode_input_request_opened_payload(event.payload.clone())?;
        if &payload.input_request_id == input_request_id {
            return Ok(true);
        }
    }
    Ok(false)
}

struct AssessmentIdMaterial<'a> {
    revision_id: &'a RevisionId,
    track_id: &'a TrackId,
    target: &'a ReviewTargetRef,
    assessment: ReviewAssessment,
    summary_content_hash: Option<&'a str>,
    replaces_assessment_ids: &'a [AssessmentId],
    related_observation_ids: &'a [ObservationId],
    related_input_request_ids: &'a [InputRequestId],
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
    let mut related_input_requests = material
        .related_input_request_ids
        .iter()
        .map(|input_request_id| input_request_id.as_str())
        .collect::<Vec<_>>();
    related_input_requests.sort();

    let digest = sha256_json_prefixed(&json!({
        "revisionId": material.revision_id.as_str(),
        "trackId": material.track_id.as_str(),
        "target": material.target,
        "assessment": material.assessment,
        "summaryContentHash": material.summary_content_hash,
        "replacesAssessmentIds": replaces,
        "relatedObservationIds": related_observations,
        "relatedInputRequestIds": related_input_requests,
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
            "assess:sha256:7ad3d9d70d09ace46116a6fb440b582f50d2742399e7d985e9551fa7f336e7b9";

        let revision_id = RevisionId::new("review-unit:sha256:one");
        let track_id = TrackId::new("human:kevin");
        let target = ReviewTargetRef::Revision {
            revision_id: revision_id.clone(),
        };

        let assessment_id = build_assessment_id(AssessmentIdMaterial {
            revision_id: &revision_id,
            track_id: &track_id,
            target: &target,
            assessment: ReviewAssessment::Accepted,
            summary_content_hash: Some("sha256:summary"),
            replaces_assessment_ids: &[],
            related_observation_ids: &[],
            related_input_request_ids: &[],
            writer_actor_id: "human:kevin",
        })
        .unwrap();

        assert_eq!(assessment_id.as_str(), EXPECTED_ASSESSMENT_ID_FOR_FIXTURE);
    }
}
