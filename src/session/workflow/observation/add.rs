use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::target::{
    CurrentRevisionContext, ObservationTargetSelector, RevisionScope, RevisionSelection,
    resolve_observation_target, resolve_revision,
};
use super::util::{required_title, staged_body, validated_track_id};
use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::crypto::EventSigner;
use crate::error::{Result, ShoreError};
use crate::model::{
    ActorId, EventId, ObservationId, ReviewTargetRef, RevisionId, TargetRef, TrackId, id_prefix,
};
use crate::session::event::{
    BodyContentType, EventTarget, EventType, ReviewObservationRecordedPayload, ShoreEvent,
    review_subject_id,
};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store::content::ContentArtifacts;
use crate::session::store::resolution::{
    prepare_write_landing, resolve_write_store, resolve_write_validation_store,
};
use crate::session::store_init::ShoreStorePaths;
use crate::session::workflow::util::sorted_unique;
use crate::session::{
    BestEffortSkipSink, EventSigningOptions, EventStore, EventWriteOutcome, current_timestamp,
    sign_event_if_requested, writer_from_options,
};
use crate::storage::{Durability, LocalStorage};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationAddOptions {
    repo: PathBuf,
    revision_id: Option<RevisionId>,
    track: Option<String>,
    title: Option<String>,
    body: Option<String>,
    body_content_type: BodyContentType,
    target: ObservationTargetSelector,
    tags: Vec<String>,
    confidence: Option<String>,
    supersedes_observation_ids: Vec<ObservationId>,
    responds_to_observation_ids: Vec<ObservationId>,
    idempotency_key: Option<String>,
    actor_id: Option<ActorId>,
    signing: EventSigningOptions,
}

impl ObservationAddOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            revision_id: None,
            track: None,
            title: None,
            body: None,
            body_content_type: BodyContentType::TextPlain,
            target: ObservationTargetSelector::revision(),
            tags: Vec::new(),
            confidence: None,
            supersedes_observation_ids: Vec::new(),
            responds_to_observation_ids: Vec::new(),
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

    pub fn responding_to(mut self, observation_id: ObservationId) -> Self {
        self.responds_to_observation_ids.push(observation_id);
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
    pub revision_id: RevisionId,
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
    let resolved = resolve_revision(
        &events,
        RevisionSelection::from_revision_seed(options.revision_id.as_ref()),
        &CurrentRevisionContext::for_repo(&options.repo)?,
        RevisionScope::default(),
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
        body_content_type: options.body_content_type,
        tags: options.tags,
        confidence: options.confidence,
        supersedes_observation_ids: options.supersedes_observation_ids,
        responds_to_observation_ids: options.responds_to_observation_ids,
        idempotency_key: options.idempotency_key,
        actor_id: options.actor_id,
        signing: options.signing,
    })?;
    Ok(result)
}

struct ObservationWriteInput {
    repo: PathBuf,
    resolved: super::ResolvedRevision,
    target: ReviewTargetRef,
    track: Option<String>,
    title: String,
    body: Option<String>,
    body_content_type: BodyContentType,
    tags: Vec<String>,
    confidence: Option<String>,
    supersedes_observation_ids: Vec<ObservationId>,
    responds_to_observation_ids: Vec<ObservationId>,
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

    let event_store = EventStore::from_backend(write_store.backend());
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
    let body_content_type = if body_content_hash.is_some() {
        input.body_content_type
    } else {
        BodyContentType::TextPlain
    };
    let tags = input.tags.clone();
    let (body, body_artifact_path, body_artifact_bytes, body_byte_size) =
        staged_body(input.body.as_deref())?;
    // Normalize the fact-pointer lists once (sort *and* dedup) and feed the same
    // canonical lists to both the content id and the stored payload, so set-equal
    // or duplicate-bearing re-writes converge byte-identically (GH #324; ADR-0026
    // D2) instead of hard-conflicting on payload_hash.
    let supersedes_observation_ids = sorted_unique(input.supersedes_observation_ids);
    let responds_to_observation_ids = sorted_unique(input.responds_to_observation_ids);
    let observation_id = build_observation_id(ObservationIdMaterial {
        target: &input.target,
        track_id: &track_id,
        title: &input.title,
        body_content_hash: body_content_hash.as_deref(),
        body_content_type: body_content_type.identity_tag(),
        tags: &input.tags,
        confidence: input.confidence.as_deref(),
        supersedes_observation_ids: &supersedes_observation_ids,
        responds_to_observation_ids: &responds_to_observation_ids,
        writer_actor_id: writer.actor_id.as_str(),
    })?;
    let source_key = input
        .idempotency_key
        .as_deref()
        .unwrap_or_else(|| observation_id.as_str());
    let idempotency_key = ReviewObservationRecordedPayload::idempotency_key(
        &input.resolved.revision_id,
        &track_id,
        source_key,
    );

    if !event_store.event_exists(&idempotency_key)?
        && let (Some(artifact_path), Some(bytes)) =
            (body_artifact_path.as_deref(), body_artifact_bytes.as_ref())
    {
        ContentArtifacts::from_backend(write_store.backend())
            .put_note_body(artifact_path, bytes)?;
    }

    let mut event = ShoreEvent::new(
        EventType::ReviewObservationRecorded,
        idempotency_key,
        EventTarget::for_subject(
            input.resolved.journal_id,
            TargetRef::Review(input.target.clone()),
            Some(track_id.clone()),
        )?,
        writer,
        ReviewObservationRecordedPayload {
            observation_id: observation_id.clone(),
            target: input.target.clone(),
            title: input.title,
            body,
            body_content_type,
            body_artifact_path,
            body_byte_size,
            body_content_hash: body_content_hash.clone(),
            tags: input.tags,
            confidence: input.confidence,
            supersedes_observation_ids,
            responds_to_observation_ids,
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
        revision_id: input.resolved.revision_id,
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

pub(crate) struct ObservationIdMaterial<'a> {
    pub(crate) track_id: &'a TrackId,
    pub(crate) target: &'a ReviewTargetRef,
    pub(crate) title: &'a str,
    pub(crate) body_content_hash: Option<&'a str>,
    pub(crate) body_content_type: Option<&'a str>,
    pub(crate) tags: &'a [String],
    pub(crate) confidence: Option<&'a str>,
    pub(crate) supersedes_observation_ids: &'a [ObservationId],
    pub(crate) responds_to_observation_ids: &'a [ObservationId],
    pub(crate) writer_actor_id: &'a str,
}

pub(crate) fn build_observation_id(material: ObservationIdMaterial<'_>) -> Result<ObservationId> {
    let mut tags = material.tags.to_vec();
    tags.sort();
    // Sort *and* dedup so a set-equal or duplicate-bearing fact-pointer list folds
    // to one id; the stored payload normalizes identically (GH #324).
    let mut supersedes = material
        .supersedes_observation_ids
        .iter()
        .map(|observation_id| observation_id.as_str())
        .collect::<Vec<_>>();
    supersedes.sort();
    supersedes.dedup();
    let mut responds_to = material
        .responds_to_observation_ids
        .iter()
        .map(|observation_id| observation_id.as_str())
        .collect::<Vec<_>>();
    responds_to.sort();
    responds_to.dedup();
    // Fold the opaque subject id (kind-tag-free), never the renamable structural
    // target, so a future rename of the target's kind tag is projection-only (DD1).
    let mut value = json!({
        "subjectId": review_subject_id(material.target)?,
        "trackId": material.track_id.as_str(),
        "title": material.title,
        "bodyContentHash": material.body_content_hash,
        "tags": tags,
        "confidence": material.confidence,
        "supersedesObservationIds": supersedes,
        "writerActorId": material.writer_actor_id,
    });
    if let Some(body_content_type) = material.body_content_type {
        value["bodyContentType"] = json!(body_content_type);
    }
    // Fold the response links only when present. An empty list must not add a key
    // to the id material, or every existing observation (none of which respond to
    // anything) would get a new id and break idempotency for already-stored events.
    if !responds_to.is_empty() {
        value["respondsToObservationIds"] = json!(responds_to);
    }
    let digest = sha256_json_prefixed(&value)?;
    Ok(ObservationId::new(format!(
        "{}:{digest}",
        id_prefix::OBSERVATION
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_material() -> (TrackId, ReviewTargetRef) {
        let track_id = TrackId::new("agent:codex");
        let target = ReviewTargetRef::Revision {
            revision_id: RevisionId::new("rev:sha256:fixed"),
        };
        (track_id, target)
    }

    /// The id a byte-identical observation with no response links produces. Pins
    /// the empty case: folding an empty response list must leave the id untouched
    /// (otherwise every already-stored observation would silently re-key).
    const EMPTY_RESPONSE_LINK_ID: &str =
        "obs:sha256:c9dab47033e41a95e1f4c020658bb336bc3139d9124fd667187d2df6a95bccad";

    #[test]
    fn empty_response_links_keep_the_historical_observation_id() {
        let (track_id, target) = synthetic_material();
        let id = build_observation_id(ObservationIdMaterial {
            track_id: &track_id,
            target: &target,
            title: "x",
            body_content_hash: None,
            body_content_type: None,
            tags: &[],
            confidence: None,
            supersedes_observation_ids: &[],
            responds_to_observation_ids: &[],
            writer_actor_id: "actor:test",
        })
        .unwrap();
        assert_eq!(id.as_str(), EMPTY_RESPONSE_LINK_ID);
    }

    #[test]
    fn observation_id_folds_the_kind_tag_free_subject() {
        // DD1: the content id folds the opaque subject id (kind-tag-free) under
        // `subjectId`, never the structural target, so a future rename of the
        // target's kind tag is projection-only. `review_subject_id` strips the
        // renamable `kind` tag (proven in `event::subject_id`).
        let (track_id, target) = synthetic_material();
        let id = build_observation_id(ObservationIdMaterial {
            track_id: &track_id,
            target: &target,
            title: "x",
            body_content_hash: None,
            body_content_type: None,
            tags: &[],
            confidence: None,
            supersedes_observation_ids: &[],
            responds_to_observation_ids: &[],
            writer_actor_id: "actor:test",
        })
        .unwrap();

        let expected_material = json!({
            "subjectId": review_subject_id(&target).unwrap(),
            "trackId": track_id.as_str(),
            "title": "x",
            "bodyContentHash": null,
            "tags": [],
            "confidence": null,
            "supersedesObservationIds": [],
            "writerActorId": "actor:test",
        });
        let expected = ObservationId::new(format!(
            "{}:{}",
            id_prefix::OBSERVATION,
            sha256_json_prefixed(&expected_material).unwrap()
        ));
        assert_eq!(id, expected);
    }

    #[test]
    fn response_links_fold_order_independently() {
        let (track_id, target) = synthetic_material();
        let a = ObservationId::new("obs:sha256:aaa");
        let b = ObservationId::new("obs:sha256:bbb");
        let build = |links: &[ObservationId]| {
            build_observation_id(ObservationIdMaterial {
                track_id: &track_id,
                target: &target,
                title: "x",
                body_content_hash: None,
                body_content_type: None,
                tags: &[],
                confidence: None,
                supersedes_observation_ids: &[],
                responds_to_observation_ids: links,
                writer_actor_id: "actor:test",
            })
            .unwrap()
        };
        // Set-equal links fold to one id regardless of the order they were authored in.
        assert_eq!(build(&[a.clone(), b.clone()]), build(&[b, a]));
    }

    #[test]
    fn response_links_dedupe_duplicate_entries() {
        let (track_id, target) = synthetic_material();
        let a = ObservationId::new("obs:sha256:aaa");
        let b = ObservationId::new("obs:sha256:bbb");
        let build = |links: &[ObservationId]| {
            build_observation_id(ObservationIdMaterial {
                track_id: &track_id,
                target: &target,
                title: "x",
                body_content_hash: None,
                body_content_type: None,
                tags: &[],
                confidence: None,
                supersedes_observation_ids: &[],
                responds_to_observation_ids: links,
                writer_actor_id: "actor:test",
            })
            .unwrap()
        };
        // A duplicate-bearing list folds to the same id as its deduped set: the
        // builder normalizes with sort *and* dedup, not a bare sort.
        assert_eq!(build(&[a.clone(), a.clone(), b.clone()]), build(&[a, b]));
    }
}
