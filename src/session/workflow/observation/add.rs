use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::target::{
    CurrentReviewUnitContext, ObservationTargetSelector, ReviewUnitScope, ReviewUnitSelection,
    resolve_observation_target, resolve_review_unit,
};
use super::util::{required_title, staged_body, validated_track_id};
use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::crypto::EventSigner;
use crate::error::{Result, ShoreError};
use crate::model::{
    ActorId, EventId, ObservationId, ReviewTargetRef, ReviewUnitId, ReviewUnitLineageId, TargetRef,
    TrackId,
};
use crate::session::event::{EventTarget, EventType, ReviewObservationRecordedPayload, ShoreEvent};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store::resolution::{
    prepare_write_landing, resolve_write_store, resolve_write_validation_store,
};
use crate::session::store_init::ShoreStorePaths;
use crate::session::{
    BestEffortSkipSink, EventSigningOptions, EventStore, EventWriteOutcome, current_timestamp,
    sign_event_if_requested, writer_from_options,
};
use crate::storage::{Durability, LocalStorage};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationAddOptions {
    repo: PathBuf,
    review_unit_id: Option<ReviewUnitId>,
    lineage_id: Option<ReviewUnitLineageId>,
    track: Option<String>,
    title: Option<String>,
    body: Option<String>,
    target: ObservationTargetSelector,
    tags: Vec<String>,
    confidence: Option<String>,
    supersedes_observation_ids: Vec<ObservationId>,
    idempotency_key: Option<String>,
    actor_id: Option<ActorId>,
    signing: EventSigningOptions,
}

impl ObservationAddOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_unit_id: None,
            lineage_id: None,
            track: None,
            title: None,
            body: None,
            target: ObservationTargetSelector::review_unit(),
            tags: Vec::new(),
            confidence: None,
            supersedes_observation_ids: Vec::new(),
            idempotency_key: None,
            actor_id: None,
            signing: EventSigningOptions::default(),
        }
    }

    /// Attribute the durable write to an explicit actor, overriding the
    /// `SHORE_ACTOR_ID` env var and the local Git identity. A malformed id is
    /// ignored (falls back to env, then Git); `None` keeps the default
    /// resolution. The chosen actor is part of the observation's
    /// content-addressed identity.
    pub fn with_actor_id(mut self, actor_id: ActorId) -> Self {
        self.actor_id = Some(actor_id);
        self
    }

    pub fn with_review_unit_id(mut self, id: ReviewUnitId) -> Self {
        self.review_unit_id = Some(id);
        self
    }

    pub fn with_lineage_id(mut self, id: ReviewUnitLineageId) -> Self {
        self.lineage_id = Some(id);
        self
    }

    pub fn with_track(mut self, track: impl Into<String>) -> Self {
        self.track = Some(track.into());
        self
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn with_body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn with_target(mut self, target: ObservationTargetSelector) -> Self {
        self.target = target;
        self
    }

    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    pub fn with_confidence(mut self, confidence: impl Into<String>) -> Self {
        self.confidence = Some(confidence.into());
        self
    }

    pub fn superseding(mut self, observation_id: ObservationId) -> Self {
        self.supersedes_observation_ids.push(observation_id);
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
pub struct ObservationAddResult {
    pub review_unit_id: ReviewUnitId,
    pub observation_id: ObservationId,
    pub event_id: EventId,
    pub track_id: TrackId,
    pub target: ReviewTargetRef,
    pub tags: Vec<String>,
    pub body_content_hash: Option<String>,
    pub events_created: usize,
    pub events_existing: usize,
    pub events_created_by_type: BTreeMap<String, usize>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn record_observation(options: ObservationAddOptions) -> Result<ObservationAddResult> {
    // Validation/derivation reads resolve the writer-visible union (linked store
    // ∪ unsynced local events) so a fact attached in a linked checkout validates
    // against everything the writer can see. The write half below writes through
    // to that same store (the clone-local store in linked mode), so the fact is
    // visible to reads in place.
    let validation_store = resolve_write_validation_store(&options.repo)?;
    let events = validation_store.validation_events()?;
    let worktree_root = ShoreStorePaths::resolve(&options.repo)?
        .worktree_root()
        .to_path_buf();
    let resolved = resolve_review_unit(
        &events,
        ReviewUnitSelection::from_review_unit_or_lineage(
            options.review_unit_id.as_ref(),
            options.lineage_id.as_ref(),
        )?,
        &CurrentReviewUnitContext::for_repo(&options.repo)?,
        ReviewUnitScope::default(),
    )?;
    let target = resolve_observation_target(&worktree_root, &resolved, &options.target)?;
    let title = required_title(options.title.as_deref())?;

    let result = write_observation_event(ObservationWriteInput {
        repo: options.repo,
        resolved,
        target,
        track: options.track,
        title,
        body: options.body,
        tags: options.tags,
        confidence: options.confidence,
        supersedes_observation_ids: options.supersedes_observation_ids,
        idempotency_key: options.idempotency_key,
        actor_id: options.actor_id,
        signing: options.signing,
    })?;
    Ok(result)
}

struct ObservationWriteInput {
    repo: PathBuf,
    resolved: super::ResolvedReviewUnit,
    target: ReviewTargetRef,
    track: Option<String>,
    title: String,
    body: Option<String>,
    tags: Vec<String>,
    confidence: Option<String>,
    supersedes_observation_ids: Vec<ObservationId>,
    idempotency_key: Option<String>,
    actor_id: Option<ActorId>,
    signing: EventSigningOptions,
}

fn write_observation_event(input: ObservationWriteInput) -> Result<ObservationAddResult> {
    let write_store = resolve_write_store(&input.repo)?;
    let worktree_root = write_store.worktree_root();
    let store_dir = write_store.store_dir();
    let storage = LocalStorage::new(store_dir);
    prepare_write_landing(&write_store, &storage)?;

    let event_store = EventStore::open(store_dir);
    let track_id = validated_track_id(input.track.as_deref().ok_or_else(|| {
        ShoreError::WorkflowInputInvalid {
            reason: "track is required".to_owned(),
        }
    })?)?;
    let writer = writer_from_options(worktree_root, input.actor_id.as_ref());
    let body_content_hash = input
        .body
        .as_ref()
        .map(|body| format!("sha256:{}", sha256_bytes_hex(body.as_bytes())));
    let tags = input.tags.clone();
    let (body, body_artifact_path, body_artifact_bytes, body_byte_size) =
        staged_body(input.body.as_deref())?;
    let observation_id = build_observation_id(ObservationIdMaterial {
        review_unit_id: &input.resolved.review_unit_id,
        track_id: &track_id,
        target: &input.target,
        title: &input.title,
        body_content_hash: body_content_hash.as_deref(),
        tags: &input.tags,
        confidence: input.confidence.as_deref(),
        supersedes_observation_ids: &input.supersedes_observation_ids,
        writer_actor_id: writer.actor_id.as_str(),
    })?;
    let source_key = input
        .idempotency_key
        .as_deref()
        .unwrap_or_else(|| observation_id.as_str());
    let idempotency_key = ReviewObservationRecordedPayload::idempotency_key(
        &input.resolved.review_unit_id,
        &track_id,
        source_key,
    );

    if !event_store.event_exists(&idempotency_key)?
        && let (Some(artifact_path), Some(bytes)) =
            (body_artifact_path.as_deref(), body_artifact_bytes.as_ref())
    {
        storage.write_bytes_atomic(Path::new(artifact_path), bytes, Durability::Durable)?;
    }

    let mut event = ShoreEvent::new(
        EventType::ReviewObservationRecorded,
        idempotency_key,
        EventTarget {
            session_id: input.resolved.session_id,
            work_unit_id: None,
            work_object_id: None,
            work_object_type: None,
            review_unit_id: Some(input.resolved.review_unit_id.clone()),
            revision_id: Some(input.resolved.revision_id),
            snapshot_id: Some(input.resolved.snapshot_id),
            track_id: Some(track_id.clone()),
            subject: Some(TargetRef::Review(input.target.clone())),
        },
        writer,
        ReviewObservationRecordedPayload {
            observation_id: observation_id.clone(),
            target: input.target.clone(),
            title: input.title,
            body,
            body_artifact_path,
            body_byte_size,
            body_content_hash: body_content_hash.clone(),
            tags: input.tags,
            confidence: input.confidence,
            supersedes_observation_ids: input.supersedes_observation_ids,
        },
        current_timestamp(),
    )?;
    sign_event_if_requested(&mut event, &input.signing)?;
    let event_id = event.event_id.clone();

    let mut events_created_by_type = BTreeMap::new();
    let outcome = event_store.record_event_once(&event)?;
    let (events_created, events_existing) = match outcome {
        EventWriteOutcome::Created => {
            events_created_by_type.insert("review_observation_recorded".to_owned(), 1);
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

    Ok(ObservationAddResult {
        review_unit_id: input.resolved.review_unit_id,
        observation_id,
        event_id,
        track_id,
        target: input.target,
        tags,
        body_content_hash,
        events_created,
        events_existing,
        events_created_by_type,
        diagnostics: state.diagnostics,
    })
}

struct ObservationIdMaterial<'a> {
    review_unit_id: &'a ReviewUnitId,
    track_id: &'a TrackId,
    target: &'a ReviewTargetRef,
    title: &'a str,
    body_content_hash: Option<&'a str>,
    tags: &'a [String],
    confidence: Option<&'a str>,
    supersedes_observation_ids: &'a [ObservationId],
    writer_actor_id: &'a str,
}

fn build_observation_id(material: ObservationIdMaterial<'_>) -> Result<ObservationId> {
    let mut tags = material.tags.to_vec();
    tags.sort();
    let mut supersedes = material
        .supersedes_observation_ids
        .iter()
        .map(|observation_id| observation_id.as_str())
        .collect::<Vec<_>>();
    supersedes.sort();
    let digest = sha256_json_prefixed(&json!({
        "reviewUnitId": material.review_unit_id.as_str(),
        "trackId": material.track_id.as_str(),
        "target": material.target,
        "title": material.title,
        "bodyContentHash": material.body_content_hash,
        "tags": tags,
        "confidence": material.confidence,
        "supersedesObservationIds": supersedes,
        "writerActorId": material.writer_actor_id,
    }))?;
    Ok(ObservationId::new(format!("obs:{digest}")))
}
