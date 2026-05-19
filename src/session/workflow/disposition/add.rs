use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::canonical_hash::{sha256_bytes_hex, sha256_json_hex, sha256_json_prefixed};
use crate::error::{Result, ShoreError};
use crate::model::{
    DispositionId, EventId, InterventionId, ObservationId, ReviewTargetRef, ReviewUnitId,
    TargetRef, TrackId,
};
use crate::session::disposition::{
    DispositionOverrideSelector, DispositionRelationships, DispositionTargetSelector,
    resolve_disposition_relationships, resolve_disposition_target,
};
use crate::session::event::{
    EventTarget, EventType, ReviewDisposition, ReviewDispositionRecordedPayload, ShoreEvent,
};
use crate::session::observation::{resolve_review_unit, staged_body, validated_track_id};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store_init::{ShoreStorePaths, prepare_shore_writer};
use crate::session::{EventStore, EventWriteOutcome, current_timestamp, reviewer_from_git_config};
use crate::storage::{Durability, LocalStorage};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispositionAddOptions {
    pub(super) repo: PathBuf,
    pub(super) review_unit_id: Option<ReviewUnitId>,
    pub(super) track: Option<String>,
    pub(super) disposition: Option<ReviewDisposition>,
    pub(super) summary: Option<String>,
    pub(super) target: DispositionTargetSelector,
    pub(super) replaces_disposition_ids: Vec<DispositionId>,
    pub(super) related_observation_ids: Vec<ObservationId>,
    pub(super) related_intervention_ids: Vec<InterventionId>,
    pub(super) overrides: Vec<DispositionOverrideSelector>,
    pub(super) idempotency_key: Option<String>,
}

impl DispositionAddOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_unit_id: None,
            track: None,
            disposition: None,
            summary: None,
            target: DispositionTargetSelector::review_unit(),
            replaces_disposition_ids: Vec::new(),
            related_observation_ids: Vec::new(),
            related_intervention_ids: Vec::new(),
            overrides: Vec::new(),
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

    pub fn with_disposition(mut self, disposition: ReviewDisposition) -> Self {
        self.disposition = Some(disposition);
        self
    }

    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    pub fn with_target(mut self, target: DispositionTargetSelector) -> Self {
        self.target = target;
        self
    }

    pub fn replacing(mut self, disposition_id: DispositionId) -> Self {
        self.replaces_disposition_ids.push(disposition_id);
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

    pub fn overriding_observation(mut self, observation_id: ObservationId) -> Self {
        self.overrides
            .push(DispositionOverrideSelector::observation(observation_id));
        self
    }

    pub fn overriding_intervention(mut self, intervention_id: InterventionId) -> Self {
        self.overrides
            .push(DispositionOverrideSelector::intervention(intervention_id));
        self
    }

    pub fn overriding_disposition(mut self, disposition_id: DispositionId) -> Self {
        self.overrides
            .push(DispositionOverrideSelector::disposition(disposition_id));
        self
    }

    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispositionAddResult {
    pub review_unit_id: ReviewUnitId,
    pub disposition_id: DispositionId,
    pub event_id: EventId,
    pub track_id: TrackId,
    pub target: ReviewTargetRef,
    pub disposition: ReviewDisposition,
    pub summary_content_hash: Option<String>,
    pub events_created: usize,
    pub events_existing: usize,
    pub events_created_by_type: BTreeMap<String, usize>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn record_disposition(options: DispositionAddOptions) -> Result<DispositionAddResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let worktree_root = paths.worktree_root();
    let shore_dir = paths.shore_dir();
    let storage = LocalStorage::new(shore_dir);
    prepare_shore_writer(&paths, &storage)?;

    let event_store = EventStore::open(shore_dir);
    let events = event_store.list_events()?;
    let resolved = resolve_review_unit(&events, options.review_unit_id.as_ref())?;
    let target = resolve_disposition_target(worktree_root, &events, &resolved, &options.target)?;
    let track_id = validated_track_id(options.track.as_deref().ok_or_else(|| {
        ShoreError::WorkflowInputInvalid {
            reason: "track is required".to_owned(),
        }
    })?)?;
    let disposition = options
        .disposition
        .ok_or_else(|| ShoreError::WorkflowInputInvalid {
            reason: "disposition is required".to_owned(),
        })?;
    let relationships = resolve_disposition_relationships(
        &events,
        &resolved,
        &DispositionRelationships {
            replaces_disposition_ids: options.replaces_disposition_ids,
            related_observation_ids: options.related_observation_ids,
            related_intervention_ids: options.related_intervention_ids,
            overrides: options.overrides,
        },
        disposition,
        options.summary.as_deref(),
    )?;
    let writer = reviewer_from_git_config(worktree_root);
    let summary_content_hash = options
        .summary
        .as_ref()
        .map(|summary| format!("sha256:{}", sha256_bytes_hex(summary.as_bytes())));
    let (summary, summary_artifact_path, summary_artifact_bytes, summary_byte_size) =
        staged_body(options.summary.as_deref())?;
    let disposition_id = build_disposition_id(DispositionIdMaterial {
        review_unit_id: &resolved.review_unit_id,
        track_id: &track_id,
        target: &target.target,
        disposition,
        summary_content_hash: summary_content_hash.as_deref(),
        replaces_disposition_ids: &relationships.replaces_disposition_ids,
        related_observation_ids: &relationships.related_observation_ids,
        related_intervention_ids: &relationships.related_intervention_ids,
        overrides: &relationships.overrides,
        writer_actor_id: writer.actor_id.as_str(),
    })?;
    let source_key = options
        .idempotency_key
        .as_deref()
        .unwrap_or_else(|| disposition_id.as_str());
    let idempotency_key = ReviewDispositionRecordedPayload::idempotency_key(
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
        // Summary artifacts are content-addressed. A crash before the event commit can leave a
        // harmless orphan that a retry reuses or overwrites with the same bytes.
        storage.write_bytes_atomic(Path::new(artifact_path), bytes, Durability::Durable)?;
    }

    let event = ShoreEvent::new(
        EventType::ReviewDispositionRecorded,
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
            subject: Some(TargetRef::Review(target.target.clone())),
        },
        writer,
        ReviewDispositionRecordedPayload {
            disposition_id: disposition_id.clone(),
            target: target.target.clone(),
            disposition,
            summary,
            summary_artifact_path,
            summary_byte_size,
            summary_content_hash: summary_content_hash.clone(),
            replaces_disposition_ids: relationships.replaces_disposition_ids,
            related_observation_ids: relationships.related_observation_ids,
            related_intervention_ids: relationships.related_intervention_ids,
            overrides: relationships.overrides,
        },
        current_timestamp(),
    )?;
    let event_id = event.event_id.clone();

    let mut events_created_by_type = BTreeMap::new();
    let outcome = event_store.record_event_once(&event)?;
    let (events_created, events_existing) = match outcome {
        EventWriteOutcome::Created => {
            events_created_by_type.insert("review_disposition_recorded".to_owned(), 1);
            (1, 0)
        }
        EventWriteOutcome::Existing => (0, 1),
    };

    let state = SessionState::from_prior_events_and_committed(&events, &event, outcome)?;
    storage.write_json_atomic(&paths.state_path(), &state, Durability::Projection)?;

    Ok(DispositionAddResult {
        review_unit_id: resolved.review_unit_id,
        disposition_id,
        event_id,
        track_id,
        target: target.target,
        disposition,
        summary_content_hash,
        events_created,
        events_existing,
        events_created_by_type,
        diagnostics: state.diagnostics,
    })
}

struct DispositionIdMaterial<'a> {
    review_unit_id: &'a ReviewUnitId,
    track_id: &'a TrackId,
    target: &'a ReviewTargetRef,
    disposition: ReviewDisposition,
    summary_content_hash: Option<&'a str>,
    replaces_disposition_ids: &'a [DispositionId],
    related_observation_ids: &'a [ObservationId],
    related_intervention_ids: &'a [InterventionId],
    overrides: &'a [ReviewTargetRef],
    writer_actor_id: &'a str,
}

fn build_disposition_id(material: DispositionIdMaterial<'_>) -> Result<DispositionId> {
    let mut replaces = material
        .replaces_disposition_ids
        .iter()
        .map(|disposition_id| disposition_id.as_str())
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
    let mut overrides = material
        .overrides
        .iter()
        .map(sha256_json_hex)
        .collect::<Result<Vec<_>>>()?;
    // Hash each override target before sorting so the disposition ID is independent of
    // serde's struct-field declaration order for ReviewTargetRef variants.
    overrides.sort();

    let digest = sha256_json_prefixed(&json!({
        "reviewUnitId": material.review_unit_id.as_str(),
        "trackId": material.track_id.as_str(),
        "target": material.target,
        "disposition": material.disposition,
        "summaryContentHash": material.summary_content_hash,
        "replacesDispositionIds": replaces,
        "relatedObservationIds": related_observations,
        "relatedInterventionIds": related_interventions,
        "overrides": overrides,
        "writerActorId": material.writer_actor_id,
    }))?;
    Ok(DispositionId::new(format!("disp:{digest}")))
}
