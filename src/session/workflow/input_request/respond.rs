use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::view::{InputRequestProjectionRecords, collect_input_request_projection_records};
use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::crypto::EventSigner;
use crate::error::{Result, ShoreError};
use crate::model::{
    ActorId, EventId, InputRequestId, InputRequestResponseId, ReviewTargetRef, TargetRef,
};
use crate::session::event::{
    EventTarget, EventType, InputRequestRespondedPayload, InputRequestResponseOutcome, ShoreEvent,
};
use crate::session::observation::staged_body;
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store_init::{ShoreStorePaths, prepare_shore_writer};
use crate::session::{
    EventSigningOptions, EventStore, EventWriteOutcome, current_timestamp, sign_event_if_requested,
    writer_from_options,
};
use crate::storage::{Durability, LocalStorage};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputRequestRespondOptions {
    repo: PathBuf,
    input_request_id: InputRequestId,
    outcome: Option<InputRequestResponseOutcome>,
    reason: Option<String>,
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
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let worktree_root = paths.worktree_root();
    let shore_dir = paths.shore_dir();
    let storage = LocalStorage::new(shore_dir);
    prepare_shore_writer(&paths, &storage)?;

    let event_store = EventStore::open(shore_dir);
    let events = event_store.list_events()?;
    let InputRequestProjectionRecords {
        mut request_records,
        ..
    } = collect_input_request_projection_records(&events)?;
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
    let (reason, reason_artifact_path, reason_artifact_bytes, reason_byte_size) =
        staged_body(options.reason.as_deref())?;
    let input_request_response_id =
        build_input_request_response_id(InputRequestResponseIdMaterial {
            input_request_id: &request_payload.input_request_id,
            outcome,
            reason_content_hash: reason_content_hash.as_deref(),
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
        // harmless orphan that a retry reuses or overwrites with the same bytes.
        storage.write_bytes_atomic(Path::new(artifact_path), bytes, Durability::Durable)?;
    }

    let review_unit_id =
        request_event.target.review_unit_id.clone().ok_or_else(|| {
            ShoreError::Message("input request event missing review unit".to_owned())
        })?;
    let mut event = ShoreEvent::new(
        EventType::InputRequestResponded,
        idempotency_key,
        EventTarget {
            session_id: request_event.target.session_id.clone(),
            work_unit_id: None,
            work_object_id: None,
            work_object_type: None,
            review_unit_id: Some(review_unit_id.clone()),
            revision_id: request_event.target.revision_id.clone(),
            snapshot_id: request_event.target.snapshot_id.clone(),
            track_id: request_event.target.track_id.clone(),
            subject: Some(TargetRef::Review(ReviewTargetRef::InputRequest {
                review_unit_id,
                input_request_id: request_payload.input_request_id.clone(),
            })),
        },
        writer,
        InputRequestRespondedPayload {
            input_request_response_id: input_request_response_id.clone(),
            input_request_id: request_payload.input_request_id.clone(),
            outcome,
            reason,
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

    let state = SessionState::from_prior_events_and_committed(&events, &event, write_outcome)?;
    storage.write_json_atomic(&paths.state_path(), &state, Durability::Projection)?;

    Ok(InputRequestRespondResult {
        input_request_id: request_payload.input_request_id,
        input_request_response_id,
        event_id,
        outcome,
        reason_content_hash,
        events_created,
        events_existing,
        events_created_by_type,
        diagnostics: state.diagnostics,
    })
}

struct InputRequestResponseIdMaterial<'a> {
    input_request_id: &'a InputRequestId,
    outcome: InputRequestResponseOutcome,
    reason_content_hash: Option<&'a str>,
    writer_actor_id: &'a str,
}

fn build_input_request_response_id(
    material: InputRequestResponseIdMaterial<'_>,
) -> Result<InputRequestResponseId> {
    let digest = sha256_json_prefixed(&json!({
        "inputRequestId": material.input_request_id.as_str(),
        "outcome": material.outcome,
        "reasonContentHash": material.reason_content_hash,
        "writerActorId": material.writer_actor_id,
    }))?;
    Ok(InputRequestResponseId::new(format!(
        "input-request-response:{digest}"
    )))
}
