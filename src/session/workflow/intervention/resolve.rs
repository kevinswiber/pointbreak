use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::view::{InterventionProjectionRecords, collect_intervention_projection_records};
use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::error::{Result, ShoreError};
use crate::model::{EventId, InterventionId, InterventionResolutionId, ReviewTargetRef, TargetRef};
use crate::session::event::{
    EventTarget, EventType, InterventionResolutionOutcome, InterventionResolvedPayload, ShoreEvent,
};
use crate::session::observation::staged_body;
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store_init::{ShoreStorePaths, prepare_shore_writer};
use crate::session::{EventStore, EventWriteOutcome, current_timestamp, reviewer_from_git_config};
use crate::storage::{Durability, LocalStorage};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterventionResolveOptions {
    repo: PathBuf,
    intervention_id: InterventionId,
    outcome: Option<InterventionResolutionOutcome>,
    reason: Option<String>,
    idempotency_key: Option<String>,
}

impl InterventionResolveOptions {
    pub fn new(repo: impl AsRef<Path>, intervention_id: InterventionId) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            intervention_id,
            outcome: None,
            reason: None,
            idempotency_key: None,
        }
    }

    pub fn with_outcome(mut self, outcome: InterventionResolutionOutcome) -> Self {
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterventionResolveResult {
    pub intervention_id: InterventionId,
    pub intervention_resolution_id: InterventionResolutionId,
    pub event_id: EventId,
    pub outcome: InterventionResolutionOutcome,
    pub reason_content_hash: Option<String>,
    pub events_created: usize,
    pub events_existing: usize,
    pub events_created_by_type: BTreeMap<String, usize>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn resolve_intervention(
    options: InterventionResolveOptions,
) -> Result<InterventionResolveResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let worktree_root = paths.worktree_root();
    let shore_dir = paths.shore_dir();
    let storage = LocalStorage::new(shore_dir);
    prepare_shore_writer(&paths, &storage)?;

    let event_store = EventStore::open(shore_dir);
    let events = event_store.list_events()?;
    let InterventionProjectionRecords {
        mut request_records,
        ..
    } = collect_intervention_projection_records(&events)?;
    let request_record = request_records
        .remove(&options.intervention_id)
        .ok_or_else(|| {
            ShoreError::Message(format!(
                "unknown intervention: {}",
                options.intervention_id.as_str()
            ))
        })?;
    let request_event = request_record.event;
    let request_payload = request_record.payload;
    let outcome = options
        .outcome
        .ok_or_else(|| ShoreError::WorkflowInputInvalid {
            reason: "outcome is required".to_owned(),
        })?;
    let writer = reviewer_from_git_config(worktree_root);
    let reason_content_hash = options
        .reason
        .as_ref()
        .map(|reason| format!("sha256:{}", sha256_bytes_hex(reason.as_bytes())));
    let (reason, reason_artifact_path, reason_artifact_bytes, reason_byte_size) =
        staged_body(options.reason.as_deref())?;
    let intervention_resolution_id =
        build_intervention_resolution_id(InterventionResolutionIdMaterial {
            intervention_id: &request_payload.intervention_id,
            outcome,
            reason_content_hash: reason_content_hash.as_deref(),
            writer_actor_id: writer.actor_id.as_str(),
        })?;
    let source_key = options
        .idempotency_key
        .as_deref()
        .unwrap_or_else(|| intervention_resolution_id.as_str());
    let idempotency_key =
        InterventionResolvedPayload::idempotency_key(&request_payload.intervention_id, source_key);

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
            ShoreError::Message("intervention event missing review unit".to_owned())
        })?;
    let event = ShoreEvent::new(
        EventType::InterventionResolved,
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
            subject: Some(TargetRef::Review(ReviewTargetRef::Intervention {
                review_unit_id,
                intervention_id: request_payload.intervention_id.clone(),
            })),
        },
        writer,
        InterventionResolvedPayload {
            intervention_resolution_id: intervention_resolution_id.clone(),
            intervention_id: request_payload.intervention_id.clone(),
            outcome,
            reason,
            reason_artifact_path,
            reason_byte_size,
            reason_content_hash: reason_content_hash.clone(),
        },
        current_timestamp(),
    )?;
    let event_id = event.event_id.clone();

    let mut events_created_by_type = BTreeMap::new();
    let write_outcome = event_store.record_event_once(&event)?;
    let (events_created, events_existing) = match write_outcome {
        EventWriteOutcome::Created => {
            events_created_by_type.insert("intervention_resolved".to_owned(), 1);
            (1, 0)
        }
        EventWriteOutcome::Existing => (0, 1),
    };

    let state = SessionState::from_prior_events_and_committed(&events, &event, write_outcome)?;
    storage.write_json_atomic(&paths.state_path(), &state, Durability::Projection)?;

    Ok(InterventionResolveResult {
        intervention_id: request_payload.intervention_id,
        intervention_resolution_id,
        event_id,
        outcome,
        reason_content_hash,
        events_created,
        events_existing,
        events_created_by_type,
        diagnostics: state.diagnostics,
    })
}

struct InterventionResolutionIdMaterial<'a> {
    intervention_id: &'a InterventionId,
    outcome: InterventionResolutionOutcome,
    reason_content_hash: Option<&'a str>,
    writer_actor_id: &'a str,
}

fn build_intervention_resolution_id(
    material: InterventionResolutionIdMaterial<'_>,
) -> Result<InterventionResolutionId> {
    let digest = sha256_json_prefixed(&json!({
        "interventionId": material.intervention_id.as_str(),
        "outcome": material.outcome,
        "reasonContentHash": material.reason_content_hash,
        "writerActorId": material.writer_actor_id,
    }))?;
    Ok(InterventionResolutionId::new(format!(
        "intervention-resolution:{digest}"
    )))
}
