use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::target::{InputRequestTargetSelector, resolve_input_request_target};
use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::error::{Result, ShoreError};
use crate::model::{EventId, InputRequestId, ReviewTargetRef, ReviewUnitId, TargetRef, TrackId};
use crate::session::event::{
    AssertionMode, EventTarget, EventType, InputRequestOpenedPayload, InputRequestReasonCode,
    ShoreEvent,
};
use crate::session::observation::{
    required_title, resolve_review_unit, staged_body, validated_track_id,
};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store_init::{ShoreStorePaths, prepare_shore_writer};
use crate::session::{EventStore, EventWriteOutcome, current_timestamp, reviewer_from_git_config};
use crate::storage::{Durability, LocalStorage};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputRequestOpenOptions {
    repo: PathBuf,
    review_unit_id: Option<ReviewUnitId>,
    track: Option<String>,
    title: Option<String>,
    body: Option<String>,
    target: InputRequestTargetSelector,
    assertion_mode: AssertionMode,
    reason_code: Option<InputRequestReasonCode>,
    idempotency_key: Option<String>,
}

impl InputRequestOpenOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_unit_id: None,
            track: None,
            title: None,
            body: None,
            target: InputRequestTargetSelector::review_unit(),
            assertion_mode: AssertionMode::Operative,
            reason_code: None,
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

    pub fn with_target(mut self, target: InputRequestTargetSelector) -> Self {
        self.target = target;
        self
    }

    pub fn with_assertion_mode(mut self, assertion_mode: AssertionMode) -> Self {
        self.assertion_mode = assertion_mode;
        self
    }

    pub fn with_reason_code(mut self, reason_code: InputRequestReasonCode) -> Self {
        self.reason_code = Some(reason_code);
        self
    }

    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputRequestOpenResult {
    pub review_unit_id: ReviewUnitId,
    pub input_request_id: InputRequestId,
    pub event_id: EventId,
    pub track_id: TrackId,
    pub target: ReviewTargetRef,
    pub assertion_mode: AssertionMode,
    pub reason_code: InputRequestReasonCode,
    pub body_content_hash: Option<String>,
    pub events_created: usize,
    pub events_existing: usize,
    pub events_created_by_type: BTreeMap<String, usize>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn open_input_request(options: InputRequestOpenOptions) -> Result<InputRequestOpenResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let worktree_root = paths.worktree_root();
    let shore_dir = paths.shore_dir();
    let storage = LocalStorage::new(shore_dir);
    prepare_shore_writer(&paths, &storage)?;

    let event_store = EventStore::open(shore_dir);
    let events = event_store.list_events()?;
    let resolved = resolve_review_unit(&events, options.review_unit_id.as_ref())?;
    let target = resolve_input_request_target(worktree_root, &events, &resolved, &options.target)?;
    let track_id = validated_track_id(options.track.as_deref().ok_or_else(|| {
        ShoreError::WorkflowInputInvalid {
            reason: "track is required".to_owned(),
        }
    })?)?;
    let title = required_title(options.title.as_deref())?;
    let reason_code = options
        .reason_code
        .ok_or_else(|| ShoreError::WorkflowInputInvalid {
            reason: "reason code is required".to_owned(),
        })?;
    let writer = reviewer_from_git_config(worktree_root);
    let body_content_hash = options
        .body
        .as_ref()
        .map(|body| format!("sha256:{}", sha256_bytes_hex(body.as_bytes())));
    let (body, body_artifact_path, body_artifact_bytes, body_byte_size) =
        staged_body(options.body.as_deref())?;
    let input_request_id = build_input_request_id(InputRequestIdMaterial {
        review_unit_id: &resolved.review_unit_id,
        track_id: &track_id,
        target: &target,
        assertion_mode: options.assertion_mode,
        reason_code,
        title: &title,
        body_content_hash: body_content_hash.as_deref(),
        writer_actor_id: writer.actor_id.as_str(),
    })?;
    let source_key = options
        .idempotency_key
        .as_deref()
        .unwrap_or_else(|| input_request_id.as_str());
    let idempotency_key =
        InputRequestOpenedPayload::idempotency_key(&resolved.review_unit_id, &track_id, source_key);

    if !event_store.event_exists(&idempotency_key)?
        && let (Some(artifact_path), Some(bytes)) =
            (body_artifact_path.as_deref(), body_artifact_bytes.as_ref())
    {
        // Body artifacts are content-addressed. A crash before the event commit can leave a
        // harmless orphan that a retry reuses or overwrites with the same bytes.
        storage.write_bytes_atomic(Path::new(artifact_path), bytes, Durability::Durable)?;
    }

    let event = ShoreEvent::new(
        EventType::InputRequestOpened,
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
        InputRequestOpenedPayload {
            input_request_id: input_request_id.clone(),
            target: target.clone(),
            reason_code,
            title,
            body,
            body_artifact_path,
            body_byte_size,
            body_content_hash: body_content_hash.clone(),
            target_fingerprint: None,
        },
        current_timestamp(),
    )?
    .with_assertion_mode(options.assertion_mode);
    let event_id = event.event_id.clone();

    let mut events_created_by_type = BTreeMap::new();
    let outcome = event_store.record_event_once(&event)?;
    let (events_created, events_existing) = match outcome {
        EventWriteOutcome::Created => {
            events_created_by_type.insert("input_request_opened".to_owned(), 1);
            (1, 0)
        }
        EventWriteOutcome::Existing => (0, 1),
    };

    let state = SessionState::from_prior_events_and_committed(&events, &event, outcome)?;
    storage.write_json_atomic(&paths.state_path(), &state, Durability::Projection)?;

    Ok(InputRequestOpenResult {
        review_unit_id: resolved.review_unit_id,
        input_request_id,
        event_id,
        track_id,
        target,
        assertion_mode: options.assertion_mode,
        reason_code,
        body_content_hash,
        events_created,
        events_existing,
        events_created_by_type,
        diagnostics: state.diagnostics,
    })
}

struct InputRequestIdMaterial<'a> {
    review_unit_id: &'a ReviewUnitId,
    track_id: &'a TrackId,
    target: &'a ReviewTargetRef,
    assertion_mode: AssertionMode,
    reason_code: InputRequestReasonCode,
    title: &'a str,
    body_content_hash: Option<&'a str>,
    writer_actor_id: &'a str,
}

fn build_input_request_id(material: InputRequestIdMaterial<'_>) -> Result<InputRequestId> {
    let digest = sha256_json_prefixed(&json!({
        "reviewUnitId": material.review_unit_id.as_str(),
        "trackId": material.track_id.as_str(),
        "target": material.target,
        "assertionMode": material.assertion_mode,
        "reasonCode": material.reason_code,
        "title": material.title,
        "bodyContentHash": material.body_content_hash,
        "writerActorId": material.writer_actor_id,
    }))?;
    Ok(InputRequestId::new(format!("input-request:{digest}")))
}
