use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::error::{Result, ShoreError};
use crate::git::git_worktree_root;
use crate::model::{AcknowledgementId, ActorId, ReviewArtifactId, RevisionId};
use crate::session::body_artifact::{BodyArtifactOutcome, stage_body_artifact};
use crate::session::event::{
    AcknowledgementNextAction, EventTarget, EventType, ReviewArtifactAcknowledgedPayload,
    ReviewArtifactPublishedPayload, ReviewInitializedPayload, VerdictDecision, Writer, WriterRole,
    WriterTool,
};
use crate::session::{
    ProjectionDiagnostic, SessionState, ShoreEvent, current_timestamp, ensure_shore_ignored,
    ensure_store_dirs, read_review_artifacts, reviewer_from_git_config, writer_from_git_config,
};
use crate::storage::{Durability, EventStore, EventWriteOutcome, LocalStorage, TempSweepAge};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishVerdictOptions {
    repo: PathBuf,
    decision: Option<VerdictDecision>,
    summary: Option<String>,
    summary_file: Option<PathBuf>,
    target_revision: Option<RevisionId>,
    replaces: Vec<ReviewArtifactId>,
    reviewer_id: Option<ActorId>,
}

impl PublishVerdictOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            decision: None,
            summary: None,
            summary_file: None,
            target_revision: None,
            replaces: Vec::new(),
            reviewer_id: None,
        }
    }

    pub fn with_decision(mut self, decision: VerdictDecision) -> Self {
        self.decision = Some(decision);
        self
    }

    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    pub fn with_summary_file(mut self, path: impl AsRef<Path>) -> Self {
        self.summary_file = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn with_target_revision(mut self, revision_id: RevisionId) -> Self {
        self.target_revision = Some(revision_id);
        self
    }

    pub fn replacing(mut self, review_artifact_ids: Vec<ReviewArtifactId>) -> Self {
        self.replaces = review_artifact_ids;
        self
    }

    pub fn with_reviewer_id(mut self, actor_id: ActorId) -> Self {
        self.reviewer_id = Some(actor_id);
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishVerdictResult {
    pub review_artifact_id: ReviewArtifactId,
    pub events_created: usize,
    pub events_existing: usize,
    pub state_path: PathBuf,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcknowledgeReviewOptions {
    repo: PathBuf,
    review_artifact_id: ReviewArtifactId,
    next_action: Option<AcknowledgementNextAction>,
    reason: Option<String>,
    reason_file: Option<PathBuf>,
    actor_id: Option<ActorId>,
}

impl AcknowledgeReviewOptions {
    pub fn new(repo: impl AsRef<Path>, review_artifact_id: ReviewArtifactId) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_artifact_id,
            next_action: None,
            reason: None,
            reason_file: None,
            actor_id: None,
        }
    }

    pub fn with_next_action(mut self, next_action: AcknowledgementNextAction) -> Self {
        self.next_action = Some(next_action);
        self
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn with_reason_file(mut self, path: impl AsRef<Path>) -> Self {
        self.reason_file = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn with_actor_id(mut self, actor_id: ActorId) -> Self {
        self.actor_id = Some(actor_id);
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcknowledgeReviewResult {
    pub acknowledgement_id: AcknowledgementId,
    pub events_created: usize,
    pub events_existing: usize,
    pub state_path: PathBuf,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn publish_verdict(options: PublishVerdictOptions) -> Result<PublishVerdictResult> {
    let decision = options
        .decision
        .ok_or_else(|| ShoreError::Message("decision is required".to_owned()))?;
    let worktree_root = git_worktree_root(&options.repo)?;
    let summary = resolve_summary(&options)?;

    let shore_dir = worktree_root.join(".shore");
    let storage = LocalStorage::new(&shore_dir);
    storage.sweep_temp_files(&shore_dir, TempSweepAge::zero())?;
    ensure_store_dirs(&shore_dir)?;
    ensure_shore_ignored(&worktree_root)?;

    let event_store = EventStore::open(&shore_dir);
    let existing_state = SessionState::from_events(&event_store.list_events()?)?;
    let review_id = existing_state.review_id.clone();
    let work_unit_id = existing_state.work_unit_id.clone();
    let target = EventTarget::new(review_id.clone(), work_unit_id.clone());
    let target_revision = resolve_target_revision(&options, &existing_state)?;
    let reviewer = reviewer_writer(&worktree_root, options.reviewer_id.clone());
    let occurred_at = current_timestamp();

    let (summary_value, summary_artifact_path, summary_artifact_bytes, summary_byte_size) =
        match summary.as_deref() {
            Some(summary) => match stage_body_artifact(summary.as_bytes())? {
                BodyArtifactOutcome::Inline { byte_size } => {
                    (Some(summary.to_owned()), None, None, Some(byte_size))
                }
                BodyArtifactOutcome::Artifact {
                    relative_path,
                    byte_size,
                    body_envelope,
                } => (
                    None,
                    Some(relative_path),
                    Some(body_envelope.to_json_bytes()?),
                    Some(byte_size),
                ),
            },
            None => (None, None, None, None),
        };

    let review_artifact_id = build_review_artifact_id(
        &work_unit_id,
        &target_revision,
        decision,
        summary.as_deref().unwrap_or(""),
        &options.replaces,
        reviewer.actor_id.as_str(),
    )?;
    let idempotency_key =
        ReviewArtifactPublishedPayload::idempotency_key(&work_unit_id, &review_artifact_id);

    if !event_store.event_exists(&idempotency_key)?
        && let (Some(artifact_path), Some(bytes)) = (
            summary_artifact_path.as_deref(),
            summary_artifact_bytes.as_ref(),
        )
    {
        storage.write_bytes_atomic(Path::new(artifact_path), bytes, Durability::Durable)?;
    }

    ensure_review_initialized(
        &event_store,
        &target,
        &reviewer,
        &occurred_at,
        &review_id,
        &work_unit_id,
    )?;

    let event = ShoreEvent::new(
        EventType::ReviewArtifactPublished,
        idempotency_key,
        target,
        reviewer.clone(),
        ReviewArtifactPublishedPayload {
            review_artifact_id: review_artifact_id.clone(),
            work_unit_id,
            revision_id: target_revision,
            decision,
            summary: summary_value,
            summary_artifact_path,
            summary_byte_size,
            replaces_review_artifact_ids: options.replaces,
            reviewer,
        },
        occurred_at,
    )?;

    let (events_created, events_existing) = match event_store.record_event_once(&event)? {
        EventWriteOutcome::Created => (1, 0),
        EventWriteOutcome::Existing => (0, 1),
    };

    let state = SessionState::from_events(&event_store.list_events()?)?;
    let state_path = shore_dir.join("state.json");
    storage.write_json_atomic(&state_path, &state, Durability::Projection)?;

    Ok(PublishVerdictResult {
        review_artifact_id,
        events_created,
        events_existing,
        state_path,
        diagnostics: state.diagnostics,
    })
}

pub fn acknowledge_review(options: AcknowledgeReviewOptions) -> Result<AcknowledgeReviewResult> {
    let next_action = options
        .next_action
        .ok_or_else(|| ShoreError::Message("next_action is required".to_owned()))?;
    let worktree_root = git_worktree_root(&options.repo)?;
    let reason = resolve_optional_text(
        options.reason.clone(),
        options.reason_file.as_ref(),
        "reason",
    )?;

    let shore_dir = worktree_root.join(".shore");
    let storage = LocalStorage::new(&shore_dir);
    storage.sweep_temp_files(&shore_dir, TempSweepAge::zero())?;
    ensure_store_dirs(&shore_dir)?;
    ensure_shore_ignored(&worktree_root)?;

    let event_store = EventStore::open(&shore_dir);
    let events = event_store.list_events()?;
    let existing_state = SessionState::from_events(&events)?;
    let review_id = existing_state.review_id.clone();
    let work_unit_id = existing_state.work_unit_id.clone();
    let target = EventTarget::new(review_id.clone(), work_unit_id.clone());
    let known_review_artifact_ids = published_review_artifact_ids(&worktree_root)?;
    if !known_review_artifact_ids.contains(&options.review_artifact_id) {
        return Err(ShoreError::Message(format!(
            "unknown review artifact: {}",
            options.review_artifact_id.as_str()
        )));
    }

    let acknowledger = acknowledger_writer(&worktree_root, options.actor_id.clone());
    let occurred_at = current_timestamp();
    let (reason_value, reason_artifact_path, reason_artifact_bytes, reason_byte_size) =
        match reason.as_deref() {
            Some(reason) => match stage_body_artifact(reason.as_bytes())? {
                BodyArtifactOutcome::Inline { byte_size } => {
                    (Some(reason.to_owned()), None, None, Some(byte_size))
                }
                BodyArtifactOutcome::Artifact {
                    relative_path,
                    byte_size,
                    body_envelope,
                } => (
                    None,
                    Some(relative_path),
                    Some(body_envelope.to_json_bytes()?),
                    Some(byte_size),
                ),
            },
            None => (None, None, None, None),
        };

    let acknowledgement_id = build_acknowledgement_id(
        &options.review_artifact_id,
        acknowledger.actor_id.as_str(),
        next_action,
        reason.as_deref().unwrap_or(""),
    )?;
    let idempotency_key = ReviewArtifactAcknowledgedPayload::idempotency_key(
        &options.review_artifact_id,
        &acknowledgement_id,
    );

    if !event_store.event_exists(&idempotency_key)?
        && let (Some(artifact_path), Some(bytes)) = (
            reason_artifact_path.as_deref(),
            reason_artifact_bytes.as_ref(),
        )
    {
        storage.write_bytes_atomic(Path::new(artifact_path), bytes, Durability::Durable)?;
    }

    ensure_review_initialized(
        &event_store,
        &target,
        &acknowledger,
        &occurred_at,
        &review_id,
        &work_unit_id,
    )?;

    let event = ShoreEvent::new(
        EventType::ReviewArtifactAcknowledged,
        idempotency_key,
        target,
        acknowledger.clone(),
        ReviewArtifactAcknowledgedPayload {
            acknowledgement_id: acknowledgement_id.clone(),
            review_artifact_id: options.review_artifact_id,
            next_action,
            reason: reason_value,
            reason_artifact_path,
            reason_byte_size,
            acknowledger,
        },
        occurred_at,
    )?;
    let (events_created, events_existing) = match event_store.record_event_once(&event)? {
        EventWriteOutcome::Created => (1, 0),
        EventWriteOutcome::Existing => (0, 1),
    };

    let state = SessionState::from_events(&event_store.list_events()?)?;
    let state_path = shore_dir.join("state.json");
    storage.write_json_atomic(&state_path, &state, Durability::Projection)?;

    Ok(AcknowledgeReviewResult {
        acknowledgement_id,
        events_created,
        events_existing,
        state_path,
        diagnostics: state.diagnostics,
    })
}

fn resolve_summary(options: &PublishVerdictOptions) -> Result<Option<String>> {
    resolve_optional_text(
        options.summary.clone(),
        options.summary_file.as_ref(),
        "summary",
    )
}

fn resolve_optional_text(
    inline_text: Option<String>,
    file_path: Option<&PathBuf>,
    field_name: &str,
) -> Result<Option<String>> {
    match (inline_text, file_path) {
        (Some(_), Some(_)) => Err(ShoreError::Message(format!(
            "only one of {field_name} or {field_name}_file can be supplied"
        ))),
        (Some(text), None) => Ok(normalize_optional_text(Some(text))),
        (None, Some(path)) => Ok(normalize_optional_text(Some(
            std::fs::read_to_string(path)
                .map_err(|err| ShoreError::Message(format!("read {}: {err}", path.display())))?,
        ))),
        (None, None) => Ok(None),
    }
}

fn normalize_optional_text(text: Option<String>) -> Option<String> {
    text.filter(|value| !value.is_empty())
}

fn resolve_target_revision(
    options: &PublishVerdictOptions,
    existing_state: &SessionState,
) -> Result<RevisionId> {
    options
        .target_revision
        .clone()
        .or_else(|| existing_state.current_revision_id.clone())
        .ok_or_else(|| ShoreError::Message("no current revision".to_owned()))
}

fn reviewer_writer(repo: &Path, reviewer_id: Option<ActorId>) -> Writer {
    reviewer_id.map_or_else(
        || reviewer_from_git_config(repo),
        |actor_id| Writer {
            actor_id,
            role: WriterRole::Reviewer,
            tool: WriterTool {
                name: "shore".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
        },
    )
}

fn acknowledger_writer(repo: &Path, actor_id: Option<ActorId>) -> Writer {
    actor_id.map_or_else(
        || writer_from_git_config(repo),
        |actor_id| Writer {
            actor_id,
            role: WriterRole::Author,
            tool: WriterTool {
                name: "shore".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
        },
    )
}

fn ensure_review_initialized(
    event_store: &EventStore,
    target: &EventTarget,
    writer: &Writer,
    occurred_at: &str,
    review_id: &crate::model::ReviewId,
    work_unit_id: &crate::model::WorkUnitId,
) -> Result<()> {
    match event_store.record_event_once(&ShoreEvent::new(
        EventType::ReviewInitialized,
        ReviewInitializedPayload::idempotency_key(review_id, work_unit_id),
        target.clone(),
        writer.clone(),
        ReviewInitializedPayload {},
        occurred_at.to_owned(),
    )?)? {
        EventWriteOutcome::Created | EventWriteOutcome::Existing => Ok(()),
    }
}

fn build_review_artifact_id(
    work_unit_id: &crate::model::WorkUnitId,
    revision_id: &RevisionId,
    decision: VerdictDecision,
    summary: &str,
    replaces: &[ReviewArtifactId],
    reviewer_actor_id: &str,
) -> Result<ReviewArtifactId> {
    let mut replaces = replaces
        .iter()
        .map(|review_artifact_id| review_artifact_id.as_str())
        .collect::<Vec<_>>();
    replaces.sort_unstable();

    let summary_hash = format!("sha256:{}", sha256_bytes_hex(summary.as_bytes()));
    let digest = sha256_json_prefixed(&json!({
        "workUnitId": work_unit_id.as_str(),
        "revisionId": revision_id.as_str(),
        "decision": decision,
        "summaryHash": summary_hash,
        "replacesReviewArtifactIds": replaces,
        "reviewerActorId": reviewer_actor_id,
    }))?;
    Ok(ReviewArtifactId::new(format!("review-artifact:{digest}")))
}

fn build_acknowledgement_id(
    review_artifact_id: &ReviewArtifactId,
    acknowledger_actor_id: &str,
    next_action: AcknowledgementNextAction,
    reason: &str,
) -> Result<AcknowledgementId> {
    let reason_hash = format!("sha256:{}", sha256_bytes_hex(reason.as_bytes()));
    let digest = sha256_json_prefixed(&json!({
        "reviewArtifactId": review_artifact_id.as_str(),
        "acknowledgerActorId": acknowledger_actor_id,
        "nextAction": next_action,
        "reasonHash": reason_hash,
    }))?;
    Ok(AcknowledgementId::new(format!("ack:{digest}")))
}

fn published_review_artifact_ids(repo: &Path) -> Result<BTreeSet<ReviewArtifactId>> {
    Ok(read_review_artifacts(repo)?
        .into_iter()
        .map(|review_artifact| review_artifact.id)
        .collect())
}
