use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::target::{ObservationTargetSelector, resolve_observation_target, resolve_review_unit};
use super::util::{required_title, staged_body, validated_track_id};
use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::error::{Result, ShoreError};
use crate::model::{EventId, ObservationId, ReviewTargetRef, ReviewUnitId, TargetRef, TrackId};
use crate::session::event::{EventTarget, EventType, ReviewObservationRecordedPayload, ShoreEvent};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store_init::{ShoreStorePaths, prepare_shore_writer};
use crate::session::{EventStore, EventWriteOutcome, current_timestamp, reviewer_from_git_config};
use crate::storage::{Durability, LocalStorage};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationAddOptions {
    repo: PathBuf,
    review_unit_id: Option<ReviewUnitId>,
    track: Option<String>,
    title: Option<String>,
    body: Option<String>,
    target: ObservationTargetSelector,
    tags: Vec<String>,
    confidence: Option<String>,
    supersedes_observation_ids: Vec<ObservationId>,
    idempotency_key: Option<String>,
}

impl ObservationAddOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_unit_id: None,
            track: None,
            title: None,
            body: None,
            target: ObservationTargetSelector::review_unit(),
            tags: Vec::new(),
            confidence: None,
            supersedes_observation_ids: Vec::new(),
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
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let worktree_root = paths.worktree_root();
    let shore_dir = paths.shore_dir();
    let event_store = EventStore::open(shore_dir);
    let events = event_store.list_events()?;
    let resolved = resolve_review_unit(&events, options.review_unit_id.as_ref())?;
    let target = resolve_observation_target(worktree_root, &resolved, &options.target)?;
    let title = required_title(options.title.as_deref())?;

    write_observation_event(ObservationWriteInput {
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
    })
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
}

fn write_observation_event(input: ObservationWriteInput) -> Result<ObservationAddResult> {
    let paths = ShoreStorePaths::resolve(&input.repo)?;
    let worktree_root = paths.worktree_root();
    let shore_dir = paths.shore_dir();
    let storage = LocalStorage::new(shore_dir);
    prepare_shore_writer(&paths, &storage)?;

    let event_store = EventStore::open(shore_dir);
    let events = event_store.list_events()?;
    let track_id = validated_track_id(input.track.as_deref().ok_or_else(|| {
        ShoreError::WorkflowInputInvalid {
            reason: "track is required".to_owned(),
        }
    })?)?;
    let writer = reviewer_from_git_config(worktree_root);
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

    let event = ShoreEvent::new(
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
    let event_id = event.event_id.clone();

    let mut events_created_by_type = BTreeMap::new();
    let outcome = event_store.record_event_once(&event)?;
    let (events_created, events_existing) = match outcome {
        EventWriteOutcome::Created => {
            events_created_by_type.insert("review_observation_recorded".to_owned(), 1);
            (1, 0)
        }
        EventWriteOutcome::Existing => (0, 1),
    };

    let state = SessionState::from_prior_events_and_committed(&events, &event, outcome)?;
    storage.write_json_atomic(&paths.state_path(), &state, Durability::Projection)?;

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
