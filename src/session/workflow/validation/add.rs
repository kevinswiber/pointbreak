use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::super::observation::{
    CurrentRevisionContext, RevisionScope, RevisionSelection, resolve_revision, staged_body,
    validated_track_id,
};
use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::crypto::EventSigner;
use crate::error::{Result, ShoreError};
use crate::model::{
    ActorId, EventId, RevisionId, TrackId, ValidationCheckId, ValidationStatus, ValidationTarget,
    ValidationTrigger, id_prefix,
};
use crate::session::event::{
    BodyContentType, EventTarget, EventType, ShoreEvent, ValidationCheckRecordedPayload,
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
pub struct ValidationAddOptions {
    repo: PathBuf,
    revision_id: Option<RevisionId>,
    track: Option<String>,
    check_name: Option<String>,
    command: Option<String>,
    status: Option<ValidationStatus>,
    exit_code: Option<i64>,
    trigger: ValidationTrigger,
    source_fingerprint: Option<String>,
    summary: Option<String>,
    summary_content_type: BodyContentType,
    started_at: Option<String>,
    completed_at: Option<String>,
    log_artifact_content_hashes: Vec<String>,
    idempotency_key: Option<String>,
    actor_id: Option<ActorId>,
    signing: EventSigningOptions,
}

impl ValidationAddOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            revision_id: None,
            track: None,
            check_name: None,
            command: None,
            status: None,
            exit_code: None,
            trigger: ValidationTrigger::Manual,
            source_fingerprint: None,
            summary: None,
            summary_content_type: BodyContentType::TextPlain,
            started_at: None,
            completed_at: None,
            log_artifact_content_hashes: Vec::new(),
            idempotency_key: None,
            actor_id: None,
            signing: EventSigningOptions::default(),
        }
    }

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

    pub fn with_check_name(mut self, check_name: impl Into<String>) -> Self {
        self.check_name = Some(check_name.into());
        self
    }

    pub fn with_command(mut self, command: impl Into<String>) -> Self {
        self.command = Some(command.into());
        self
    }

    pub fn with_status(mut self, status: ValidationStatus) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_exit_code(mut self, exit_code: i64) -> Self {
        self.exit_code = Some(exit_code);
        self
    }

    pub fn with_trigger(mut self, trigger: ValidationTrigger) -> Self {
        self.trigger = trigger;
        self
    }

    pub fn with_source_fingerprint(mut self, source_fingerprint: impl Into<String>) -> Self {
        self.source_fingerprint = Some(source_fingerprint.into());
        self
    }

    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    pub fn with_summary_content_type(mut self, content_type: BodyContentType) -> Self {
        self.summary_content_type = content_type;
        self
    }

    pub fn with_started_at(mut self, started_at: impl Into<String>) -> Self {
        self.started_at = Some(started_at.into());
        self
    }

    pub fn with_completed_at(mut self, completed_at: impl Into<String>) -> Self {
        self.completed_at = Some(completed_at.into());
        self
    }

    pub fn with_log_artifact_content_hash(mut self, content_hash: impl Into<String>) -> Self {
        self.log_artifact_content_hashes.push(content_hash.into());
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
pub struct ValidationAddResult {
    pub revision_id: RevisionId,
    pub validation_check_id: ValidationCheckId,
    pub event_id: EventId,
    pub track_id: TrackId,
    pub target: ValidationTarget,
    pub status: ValidationStatus,
    pub summary_content_hash: Option<String>,
    pub events_created: usize,
    pub events_existing: usize,
    pub events_created_by_type: BTreeMap<String, usize>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn record_validation_check(options: ValidationAddOptions) -> Result<ValidationAddResult> {
    // Unit existence resolves the writer-visible union so validation evidence
    // attaches to a linked-only unit; the write half writes through to that same
    // store (the clone-local store in linked mode).
    let validation_store = resolve_write_validation_store(&options.repo)?;
    let events = validation_store.validation_events()?;
    let resolved = resolve_revision(
        &events,
        RevisionSelection::from_revision_seed(options.revision_id.as_ref()),
        &CurrentRevisionContext::for_repo(&options.repo)?,
        RevisionScope::default(),
    )?;
    let check_name = required_check_name(options.check_name.as_deref())?;
    let status = options
        .status
        .ok_or_else(|| ShoreError::WorkflowInputInvalid {
            reason: "status is required".to_owned(),
        })?;

    let result = write_validation_check_event(ValidationWriteInput {
        repo: options.repo,
        resolved,
        track: options.track,
        check_name,
        command: options.command,
        status,
        exit_code: options.exit_code,
        trigger: options.trigger,
        source_fingerprint: options.source_fingerprint,
        summary: options.summary,
        summary_content_type: options.summary_content_type,
        started_at: options.started_at,
        completed_at: options.completed_at,
        log_artifact_content_hashes: options.log_artifact_content_hashes,
        idempotency_key: options.idempotency_key,
        actor_id: options.actor_id,
        signing: options.signing,
    })?;
    Ok(result)
}

struct ValidationWriteInput {
    repo: PathBuf,
    resolved: super::super::observation::ResolvedRevision,
    track: Option<String>,
    check_name: String,
    command: Option<String>,
    status: ValidationStatus,
    exit_code: Option<i64>,
    trigger: ValidationTrigger,
    source_fingerprint: Option<String>,
    summary: Option<String>,
    summary_content_type: BodyContentType,
    started_at: Option<String>,
    completed_at: Option<String>,
    log_artifact_content_hashes: Vec<String>,
    idempotency_key: Option<String>,
    actor_id: Option<ActorId>,
    signing: EventSigningOptions,
}

fn write_validation_check_event(input: ValidationWriteInput) -> Result<ValidationAddResult> {
    let write_store = resolve_write_store(&input.repo)?;
    let worktree_root = write_store.worktree_root();
    let store_dir = write_store.store_dir();
    let storage = LocalStorage::new(store_dir);
    prepare_write_landing(&write_store, &storage)?;

    let event_store = EventStore::from_backend(write_store.backend());
    let track_id = validated_track_id(input.track.as_deref().ok_or_else(|| {
        ShoreError::WorkflowInputInvalid {
            reason: "track is required".to_owned(),
        }
    })?)?;
    let writer = writer_from_options(worktree_root, input.actor_id.as_ref());
    let summary_content_hash = input
        .summary
        .as_ref()
        .map(|summary| format!("sha256:{}", sha256_bytes_hex(summary.as_bytes())));
    let summary_content_type = if summary_content_hash.is_some() {
        input.summary_content_type
    } else {
        BodyContentType::TextPlain
    };
    let (summary, summary_artifact_path, summary_artifact_bytes, summary_byte_size) =
        staged_body(input.summary.as_deref())?;
    let mut log_artifact_content_hashes = input.log_artifact_content_hashes;
    log_artifact_content_hashes.sort();
    log_artifact_content_hashes.dedup();
    let target = ValidationTarget::Revision {
        revision_id: input.resolved.revision_id.clone(),
    };
    let validation_check_id = build_validation_check_id(ValidationCheckIdMaterial {
        revision_id: &input.resolved.revision_id,
        track_id: &track_id,
        target: &target,
        check_name: &input.check_name,
        command: input.command.as_deref(),
        status: input.status,
        exit_code: input.exit_code,
        trigger: input.trigger,
        source_fingerprint: input.source_fingerprint.as_deref(),
        summary_content_hash: summary_content_hash.as_deref(),
        summary_content_type: summary_content_type.identity_tag(),
        started_at: input.started_at.as_deref(),
        completed_at: input.completed_at.as_deref(),
        log_artifact_content_hashes: &log_artifact_content_hashes,
        writer_actor_id: writer.actor_id.as_str(),
    })?;
    let source_key = input
        .idempotency_key
        .as_deref()
        .unwrap_or_else(|| validation_check_id.as_str());
    let idempotency_key = ValidationCheckRecordedPayload::idempotency_key(
        &input.resolved.revision_id,
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

    // `for_revision` already addresses the `Review(Revision)` subject; the track
    // is the only envelope field the validation write adds.
    let event_target = EventTarget::for_revision(
        input.resolved.journal_id,
        input.resolved.revision_id.clone(),
        Some(track_id.clone()),
    )?;

    let mut event = ShoreEvent::new(
        EventType::ValidationCheckRecorded,
        idempotency_key,
        event_target,
        writer,
        ValidationCheckRecordedPayload {
            validation_check_id: validation_check_id.clone(),
            target: target.clone(),
            check_name: input.check_name,
            command: input.command,
            status: input.status,
            exit_code: input.exit_code,
            trigger: input.trigger,
            source_fingerprint: input.source_fingerprint,
            summary,
            summary_content_type,
            summary_artifact_path,
            summary_byte_size,
            summary_content_hash: summary_content_hash.clone(),
            started_at: input.started_at,
            completed_at: input.completed_at,
            log_artifact_content_hashes,
        },
        current_timestamp(),
    )?;
    sign_event_if_requested(&mut event, &input.signing)?;
    let event_id = event.event_id.clone();

    let mut events_created_by_type = BTreeMap::new();
    let outcome = event_store.record_event_once(&event)?;
    let (events_created, events_existing) = match outcome {
        EventWriteOutcome::Created => {
            events_created_by_type.insert("validation_check_recorded".to_owned(), 1);
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

    Ok(ValidationAddResult {
        revision_id: input.resolved.revision_id,
        validation_check_id,
        event_id,
        track_id,
        target,
        status: input.status,
        summary_content_hash,
        events_created,
        events_existing,
        events_created_by_type,
        diagnostics: state.diagnostics,
    })
}

pub(super) struct ValidationCheckIdMaterial<'a> {
    pub revision_id: &'a RevisionId,
    pub track_id: &'a TrackId,
    pub target: &'a ValidationTarget,
    pub check_name: &'a str,
    pub command: Option<&'a str>,
    pub status: ValidationStatus,
    pub exit_code: Option<i64>,
    pub trigger: ValidationTrigger,
    pub source_fingerprint: Option<&'a str>,
    pub summary_content_hash: Option<&'a str>,
    pub summary_content_type: Option<&'a str>,
    pub started_at: Option<&'a str>,
    pub completed_at: Option<&'a str>,
    pub log_artifact_content_hashes: &'a [String],
    pub writer_actor_id: &'a str,
}

pub(super) fn build_validation_check_id(
    material: ValidationCheckIdMaterial<'_>,
) -> Result<ValidationCheckId> {
    let mut log_hashes = material.log_artifact_content_hashes.to_vec();
    log_hashes.sort();
    log_hashes.dedup();
    let mut value = json!({
        "revisionId": material.revision_id.as_str(),
        "trackId": material.track_id.as_str(),
        "target": material.target,
        "checkName": material.check_name,
        "command": material.command,
        "status": material.status,
        "exitCode": material.exit_code,
        "trigger": material.trigger,
        "sourceFingerprint": material.source_fingerprint,
        "summaryContentHash": material.summary_content_hash,
        "startedAt": material.started_at,
        "completedAt": material.completed_at,
        "logArtifactContentHashes": log_hashes,
        "writerActorId": material.writer_actor_id,
    });
    if let Some(summary_content_type) = material.summary_content_type {
        value["summaryContentType"] = json!(summary_content_type);
    }
    let digest = sha256_json_prefixed(&value)?;
    Ok(ValidationCheckId::new(format!(
        "{}:{digest}",
        id_prefix::VALIDATION
    )))
}

fn required_check_name(value: Option<&str>) -> Result<String> {
    let value = value.unwrap_or_default().trim();
    if value.is_empty() {
        return Err(ShoreError::WorkflowInputInvalid {
            reason: "check name is required".to_owned(),
        });
    }
    Ok(value.to_owned())
}
