use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::target::{InputRequestTargetSelector, resolve_input_request_target};
use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::crypto::EventSigner;
use crate::error::{Result, ShoreError};
use crate::model::{
    ActorId, EventId, InputRequestId, ReviewTargetRef, RevisionId, TargetRef, TrackId, id_prefix,
};
use crate::session::event::{
    AssertionMode, BodyContentType, EventTarget, EventType, InputRequestOpenedPayload,
    InputRequestReasonCode, ShoreEvent,
};
use crate::session::observation::{
    CurrentRevisionContext, RevisionScope, RevisionSelection, required_title, resolve_revision,
    staged_body, validated_track_id,
};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store::content::ContentArtifacts;
use crate::session::store::resolution::{
    prepare_write_landing, resolve_write_store, resolve_write_validation_store,
};
use crate::session::{
    BestEffortSkipSink, EventSigningOptions, EventStore, EventWriteOutcome, current_timestamp,
    sign_event_if_requested, writer_from_options,
};
use crate::storage::{Durability, LocalStorage};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputRequestOpenOptions {
    repo: PathBuf,
    revision_id: Option<RevisionId>,
    track: Option<String>,
    title: Option<String>,
    body: Option<String>,
    body_content_type: BodyContentType,
    target: InputRequestTargetSelector,
    assertion_mode: AssertionMode,
    reason_code: Option<InputRequestReasonCode>,
    idempotency_key: Option<String>,
    actor_id: Option<ActorId>,
    signing: EventSigningOptions,
}

impl InputRequestOpenOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            revision_id: None,
            track: None,
            title: None,
            body: None,
            body_content_type: BodyContentType::TextPlain,
            target: InputRequestTargetSelector::revision(),
            assertion_mode: AssertionMode::Operative,
            reason_code: None,
            idempotency_key: None,
            actor_id: None,
            signing: EventSigningOptions::default(),
        }
    }

    /// Attribute the durable write to an explicit actor, overriding the
    /// `SHORE_ACTOR_ID` env var and the local Git identity. A malformed id is
    /// ignored (falls back to env, then Git); `None` keeps the default
    /// resolution. The chosen actor is part of the input request's
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

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn with_body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn with_body_content_type(mut self, content_type: BodyContentType) -> Self {
        self.body_content_type = content_type;
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
pub struct InputRequestOpenResult {
    pub revision_id: RevisionId,
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
    let write_store = resolve_write_store(&options.repo)?;
    let worktree_root = write_store.worktree_root();
    let store_dir = write_store.store_dir();
    let storage = LocalStorage::new(store_dir);
    prepare_write_landing(&write_store, &storage)?;

    // Validation/derivation reads resolve the writer-visible union (linked store
    // ∪ unsynced local events) so a request opened in a linked checkout validates
    // its unit and observation-ref/file targets against everything the writer sees.
    let validation_store = resolve_write_validation_store(&options.repo)?;
    let validation_events = validation_store.validation_events()?;
    let resolved = resolve_revision(
        &validation_events,
        RevisionSelection::from_revision_seed(options.revision_id.as_ref()),
        &CurrentRevisionContext::for_repo(&options.repo)?,
        RevisionScope::default(),
    )?;
    let target = resolve_input_request_target(
        worktree_root,
        &validation_events,
        &resolved,
        &options.target,
    )?;

    // The write half lands in the resolved write store (the clone-local store in
    // linked mode) and rebuilds its state.json there.
    let event_store = EventStore::from_backend(write_store.backend());
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
    let writer = writer_from_options(worktree_root, options.actor_id.as_ref());
    let body_content_hash = options
        .body
        .as_ref()
        .map(|body| format!("sha256:{}", sha256_bytes_hex(body.as_bytes())));
    let body_content_type = if body_content_hash.is_some() {
        options.body_content_type
    } else {
        BodyContentType::TextPlain
    };
    let (body, body_artifact_path, body_artifact_bytes, body_byte_size) =
        staged_body(options.body.as_deref())?;
    let input_request_id = build_input_request_id(InputRequestIdMaterial {
        revision_id: &resolved.revision_id,
        track_id: &track_id,
        target: &target,
        assertion_mode: options.assertion_mode,
        reason_code,
        title: &title,
        body_content_hash: body_content_hash.as_deref(),
        body_content_type: body_content_type.identity_tag(),
        writer_actor_id: writer.actor_id.as_str(),
    })?;
    let source_key = options
        .idempotency_key
        .as_deref()
        .unwrap_or_else(|| input_request_id.as_str());
    let idempotency_key =
        InputRequestOpenedPayload::idempotency_key(&resolved.revision_id, &track_id, source_key);

    if !event_store.event_exists(&idempotency_key)?
        && let (Some(artifact_path), Some(bytes)) =
            (body_artifact_path.as_deref(), body_artifact_bytes.as_ref())
    {
        // Body artifacts are content-addressed. A crash before the event commit can leave a
        // harmless orphan that a retry reuses or finds already present with the same bytes.
        ContentArtifacts::from_backend(write_store.backend())
            .put_note_body(artifact_path, bytes)?;
    }

    let mut event = ShoreEvent::new(
        EventType::InputRequestOpened,
        idempotency_key,
        EventTarget::for_subject(
            resolved.journal_id,
            TargetRef::Review(target.clone()),
            Some(track_id.clone()),
        )?,
        writer,
        InputRequestOpenedPayload {
            input_request_id: input_request_id.clone(),
            target: target.clone(),
            // `shore review input-request open` only opens review-domain requests.
            task_target: None,
            reason_code,
            title,
            body,
            body_content_type,
            body_artifact_path,
            body_byte_size,
            body_content_hash: body_content_hash.clone(),
            target_fingerprint: None,
        },
        current_timestamp(),
    )?
    .with_assertion_mode(options.assertion_mode);
    sign_event_if_requested(&mut event, &options.signing)?;
    let event_id = event.event_id.clone();

    let mut events_created_by_type = BTreeMap::new();
    let outcome = event_store.record_event_once(&event)?;
    let (events_created, events_existing) = match outcome {
        EventWriteOutcome::Created => {
            events_created_by_type.insert("input_request_opened".to_owned(), 1);
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

    let result = InputRequestOpenResult {
        revision_id: resolved.revision_id,
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
    };
    Ok(result)
}

struct InputRequestIdMaterial<'a> {
    revision_id: &'a RevisionId,
    track_id: &'a TrackId,
    target: &'a ReviewTargetRef,
    assertion_mode: AssertionMode,
    reason_code: InputRequestReasonCode,
    title: &'a str,
    body_content_hash: Option<&'a str>,
    body_content_type: Option<&'a str>,
    writer_actor_id: &'a str,
}

fn build_input_request_id(material: InputRequestIdMaterial<'_>) -> Result<InputRequestId> {
    let mut value = json!({
        "revisionId": material.revision_id.as_str(),
        "trackId": material.track_id.as_str(),
        "target": material.target,
        "assertionMode": material.assertion_mode,
        "reasonCode": material.reason_code,
        "title": material.title,
        "bodyContentHash": material.body_content_hash,
        "writerActorId": material.writer_actor_id,
    });
    if let Some(body_content_type) = material.body_content_type {
        value["bodyContentType"] = json!(body_content_type);
    }
    let digest = sha256_json_prefixed(&value)?;
    Ok(InputRequestId::new(format!(
        "{}:{digest}",
        id_prefix::INPUT_REQUEST
    )))
}
