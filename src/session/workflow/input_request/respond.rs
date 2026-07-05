use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::view::{InputRequestProjectionRecords, collect_input_request_projection_records};
use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::crypto::EventSigner;
use crate::error::{Result, ShoreError};
use crate::model::{
    ActorId, EventId, InputRequestId, InputRequestResponseId, ReviewTargetRef, TargetRef, id_prefix,
};
use crate::session::event::{
    BodyContentType, EventTarget, EventType, InputRequestRespondedPayload,
    InputRequestResponseOutcome, ShoreEvent, decode_input_request_opened_payload,
};
use crate::session::observation::staged_body;
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
pub struct InputRequestRespondOptions {
    repo: PathBuf,
    input_request_id: InputRequestId,
    outcome: Option<InputRequestResponseOutcome>,
    reason: Option<String>,
    reason_content_type: BodyContentType,
    idempotency_key: Option<String>,
    actor_id: Option<ActorId>,
    signing: EventSigningOptions,
}

impl InputRequestRespondOptions {
    pub fn new(repo: impl AsRef<Path>, input_request_id: InputRequestId) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            input_request_id,
            outcome: None,
            reason: None,
            reason_content_type: BodyContentType::TextPlain,
            idempotency_key: None,
            actor_id: None,
            signing: EventSigningOptions::default(),
        }
    }

    /// Attribute the durable write to an explicit actor, overriding the
    /// `SHORE_ACTOR_ID` env var and the local Git identity. A malformed id is
    /// ignored (falls back to env, then Git); `None` keeps the default
    /// resolution. The chosen actor is part of the response's content-addressed
    /// identity, so distinct actors produce distinct responses.
    pub fn with_actor_id(mut self, actor_id: ActorId) -> Self {
        self.actor_id = Some(actor_id);
        self
    }

    pub fn with_outcome(mut self, outcome: InputRequestResponseOutcome) -> Self {
        self.outcome = Some(outcome);
        self
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn with_reason_content_type(mut self, content_type: BodyContentType) -> Self {
        self.reason_content_type = content_type;
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
pub struct InputRequestRespondResult {
    pub input_request_id: InputRequestId,
    pub input_request_response_id: InputRequestResponseId,
    pub event_id: EventId,
    pub outcome: InputRequestResponseOutcome,
    pub reason_content_hash: Option<String>,
    pub events_created: usize,
    pub events_existing: usize,
    pub events_created_by_type: BTreeMap<String, usize>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn respond_input_request(
    options: InputRequestRespondOptions,
) -> Result<InputRequestRespondResult> {
    let write_store = resolve_write_store(&options.repo)?;
    let worktree_root = write_store.worktree_root();
    let store_dir = write_store.store_dir();
    let storage = LocalStorage::new(store_dir);
    prepare_write_landing(&write_store, &storage)?;

    // The write half lands in the resolved write store (the clone-local store in
    // linked mode) and rebuilds its state.json there.
    let event_store = EventStore::from_backend(write_store.backend());

    // The request being responded to may live only in the linked store: its
    // EventTarget fields are copied verbatim into the response, so the lookup
    // resolves the writer-visible union.
    let validation_store = resolve_write_validation_store(&options.repo)?;
    let validation_events = validation_store.validation_events()?;

    // A task-attempt input request belongs to the agent-resumption domain
    // (authored by the agent session / relay): it carries no review unit and no
    // track, so the review-shaped projection below would otherwise fail deep with
    // a confusing "missing track id". Detect it up front and name the boundary.
    reject_task_attempt_input_request(&validation_events, &options.input_request_id)?;

    let InputRequestProjectionRecords {
        mut request_records,
        ..
    } = collect_input_request_projection_records(&validation_events)?;
    let request_record = request_records
        .remove(&options.input_request_id)
        .ok_or_else(|| {
            ShoreError::Message(format!(
                "unknown input request: {}",
                options.input_request_id.as_str()
            ))
        })?;
    let request_event = request_record.event;
    let request_payload = request_record.payload;
    let outcome = options
        .outcome
        .ok_or_else(|| ShoreError::WorkflowInputInvalid {
            reason: "outcome is required".to_owned(),
        })?;
    let writer = writer_from_options(worktree_root, options.actor_id.as_ref());
    let reason_content_hash = options
        .reason
        .as_ref()
        .map(|reason| format!("sha256:{}", sha256_bytes_hex(reason.as_bytes())));
    let reason_content_type = if reason_content_hash.is_some() {
        options.reason_content_type
    } else {
        BodyContentType::TextPlain
    };
    let (reason, reason_artifact_path, reason_artifact_bytes, reason_byte_size) =
        staged_body(options.reason.as_deref())?;
    let input_request_response_id =
        build_input_request_response_id(InputRequestResponseIdMaterial {
            input_request_id: &request_payload.input_request_id,
            outcome,
            reason_content_hash: reason_content_hash.as_deref(),
            reason_content_type: reason_content_type.identity_tag(),
            writer_actor_id: writer.actor_id.as_str(),
        })?;
    let source_key = options
        .idempotency_key
        .as_deref()
        .unwrap_or_else(|| input_request_response_id.as_str());
    let idempotency_key = InputRequestRespondedPayload::idempotency_key(
        &request_payload.input_request_id,
        source_key,
    );

    if !event_store.event_exists(&idempotency_key)?
        && let (Some(artifact_path), Some(bytes)) = (
            reason_artifact_path.as_deref(),
            reason_artifact_bytes.as_ref(),
        )
    {
        // Body artifacts are content-addressed. A crash before the event commit can leave a
        // harmless orphan that a retry reuses or finds already present with the same bytes.
        ContentArtifacts::from_backend(write_store.backend())
            .put_note_body(artifact_path, bytes)?;
    }

    // The opened request is review-domain here (task-attempt requests were
    // rejected above); its subject — hence the revision — is reconstructed from
    // the opened event's payload, and threaded onto the response payload so the
    // response's own subject is reconstructable without re-reading the opened
    // envelope.
    let revision_id = request_event
        .subject_revision_id()?
        .ok_or_else(|| ShoreError::Message("input request event missing review unit".to_owned()))?;
    let mut event = ShoreEvent::new(
        EventType::InputRequestResponded,
        idempotency_key,
        EventTarget::for_subject(
            request_event.target.journal_id.clone(),
            TargetRef::Review(ReviewTargetRef::InputRequest {
                revision_id: revision_id.clone(),
                input_request_id: request_payload.input_request_id.clone(),
            }),
            request_event.target.track_id.clone(),
        )?,
        writer,
        InputRequestRespondedPayload {
            input_request_response_id: input_request_response_id.clone(),
            input_request_id: request_payload.input_request_id.clone(),
            revision_id: Some(revision_id),
            // Review-domain response: its subject is the revision, not a task subject.
            task_target: None,
            outcome,
            reason,
            reason_content_type,
            reason_artifact_path,
            reason_byte_size,
            reason_content_hash: reason_content_hash.clone(),
            target_fingerprint: None,
        },
        current_timestamp(),
    )?;
    sign_event_if_requested(&mut event, &options.signing)?;
    let event_id = event.event_id.clone();

    let mut events_created_by_type = BTreeMap::new();
    let write_outcome = event_store.record_event_once(&event)?;
    let (events_created, events_existing) = match write_outcome {
        EventWriteOutcome::Created => {
            events_created_by_type.insert("input_request_responded".to_owned(), 1);
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

    let result = InputRequestRespondResult {
        input_request_id: request_payload.input_request_id,
        input_request_response_id,
        event_id,
        outcome,
        reason_content_hash,
        events_created,
        events_existing,
        events_created_by_type,
        diagnostics: state.diagnostics,
    };
    Ok(result)
}

/// Error if `input_request_id` names a task-attempt input request. Those belong
/// to the agent-resumption domain — authored by the agent session / relay, not
/// by `shore input-request open` — and are not answerable through this
/// review-fact command. A task-attempt request has a work object but no review
/// unit, so the generic review-shaped lookup would otherwise reject it deep in
/// the projection on a missing track id.
fn reject_task_attempt_input_request(
    events: &[ShoreEvent],
    input_request_id: &InputRequestId,
) -> Result<()> {
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::InputRequestOpened)
    {
        let payload = decode_input_request_opened_payload(event.payload.clone())?;
        // A task-domain request carries its task subject in the payload; the
        // review-domain path leaves it absent.
        if &payload.input_request_id == input_request_id && payload.task_target.is_some() {
            return Err(ShoreError::WorkflowInputInvalid {
                reason: format!(
                    "input request {} targets a task attempt, not a revision; \
                     task-attempt input requests are answered by the agent session that owns \
                     them, not by shore input-request respond",
                    input_request_id.as_str()
                ),
            });
        }
    }
    Ok(())
}

pub(crate) struct InputRequestResponseIdMaterial<'a> {
    pub(crate) input_request_id: &'a InputRequestId,
    pub(crate) outcome: InputRequestResponseOutcome,
    pub(crate) reason_content_hash: Option<&'a str>,
    pub(crate) reason_content_type: Option<&'a str>,
    pub(crate) writer_actor_id: &'a str,
}

pub(crate) fn build_input_request_response_id(
    material: InputRequestResponseIdMaterial<'_>,
) -> Result<InputRequestResponseId> {
    let mut value = json!({
        "inputRequestId": material.input_request_id.as_str(),
        "outcome": material.outcome,
        "reasonContentHash": material.reason_content_hash,
        "writerActorId": material.writer_actor_id,
    });
    if let Some(reason_content_type) = material.reason_content_type {
        value["reasonContentType"] = json!(reason_content_type);
    }
    let digest = sha256_json_prefixed(&value)?;
    Ok(InputRequestResponseId::new(format!(
        "{}:{digest}",
        id_prefix::INPUT_REQUEST_RESPONSE
    )))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::process::Command;

    use super::*;
    use crate::model::{JournalId, TaskTargetRef, WorkObjectId};
    use crate::session::projection::test_support::task_input_request_event_with_target;

    #[test]
    fn respond_to_task_attempt_request_explains_the_domain_boundary() {
        let repo = TestRepo::new();
        // A task-attempt input request — the agent-resumption domain, authored by
        // the relay/adapter, not by `shore input-request open`.
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = JournalId::new("journal:demo");
        let input_request_id = InputRequestId::new("input-request:sha256:taskreq");
        let request = task_input_request_event_with_target(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:approve",
            "2026-06-13T00:00:00Z",
            TargetRef::Task(TaskTargetRef::TaskAttempt {
                task_attempt_id: task_attempt_id.clone(),
            }),
            "approve?",
        );
        EventStore::open(resolved_store_dir(repo.path()))
            .record_event_once(&request)
            .unwrap();

        let error = respond_input_request(
            InputRequestRespondOptions::new(repo.path(), input_request_id.clone())
                .with_outcome(InputRequestResponseOutcome::Approved),
        )
        .expect_err("a task-attempt request is not answerable via this command");

        let message = error.to_string();
        assert!(
            message.contains("task attempt"),
            "the error names the domain boundary; got: {message}"
        );
        assert!(
            message.contains("shore input-request respond"),
            "the error points at the command that does not apply; got: {message}"
        );
        assert!(
            !message.contains("review unit"),
            "the cryptic message is replaced; got: {message}"
        );
    }

    /// The store a workflow actually lands in for `repo` — the shared common-dir
    /// store by default. Seeds and reads that pair with a workflow resolve here,
    /// not the raw worktree-local `.shore/data`.
    fn resolved_store_dir(repo: &Path) -> PathBuf {
        crate::git::git_common_dir(repo).unwrap().join("shore")
    }

    struct TestRepo {
        root: tempfile::TempDir,
    }

    impl TestRepo {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("create temp git repository directory");
            let repo = Self { root };
            repo.git(["init"]);
            repo.git(["config", "user.name", "Shore Tests"]);
            repo.git(["config", "user.email", "shore-tests@example.com"]);
            repo.git(["config", "commit.gpgsign", "false"]);
            repo
        }

        fn path(&self) -> &Path {
            self.root.path()
        }

        fn git<I, S>(&self, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<OsStr>,
        {
            let output = Command::new("git")
                .args(args)
                .current_dir(self.root.path())
                .output()
                .expect("run git");
            assert!(
                output.status.success(),
                "git failed:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
