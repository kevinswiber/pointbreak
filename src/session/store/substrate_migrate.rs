//! One-shot store migrator: converges an already-reshaped store onto the final
//! `object`-shaped wire in a single pass.
//!
//! This is throwaway, run-once tooling, not a shipped command. The stores it
//! migrates were already lifted into the reshaped, `journal`-shaped envelope by a
//! prior migration, so the per-event transform here is narrow but heavy:
//!
//! 1. **Content-id re-derivation.** Every content id (observation, assessment,
//!    validation, input-request, input-request-response, the four
//!    association/withdrawal ids) is the prefix over a canonical-JSON digest. The
//!    current builders fold the revision under `"revisionId"` (the digest formerly
//!    folded `"reviewUnitId"`). The id value is opaque to the read path, so an
//!    un-migrated store still *reads*, but a future re-record of the same fact
//!    would mint the current-builder id and **fork** instead of converging. This
//!    pass re-derives every content id from the current payload, remaps every
//!    reference to it (a withdrawal folds its association id, an assessment folds
//!    its related observations/input-requests, a response folds its request, a
//!    superseding observation folds its predecessors), re-keys the idempotency key
//!    where it embedded the changed id, and re-signs. It re-derives from the
//!    current payload rather than flipping the digest key because the stored ids
//!    are frozen at the material they were minted from — earlier wire reshapes
//!    re-keyed the payloads without re-deriving the opaque content ids, so a stored
//!    id is not reproducible from the current payload. The convergence target is
//!    the id a fresh re-record mints today, which the re-derivation reproduces.
//! 2. **Artifact re-hash.** The diff artifact's `schema` value moved to
//!    `shore.object` and its directory to `artifacts/objects/`. The schema is part
//!    of the hashed body, so every artifact re-hashes; each referencing capture's
//!    `snapshotArtifactContentHash` payload key is renamed to
//!    `objectArtifactContentHash` and its value remapped to the re-hashed artifact.
//!
//! A content id appears in its idempotency key only for the default dedupe key, so
//! re-keying is a substring substitution that is a no-op for an explicit dedupe
//! key (the event id then stays stable, which is correct — the explicit key still
//! converges). The capture's idempotency key folds only the revision id, so a
//! capture re-signs in place with a stable event id even though its payload hash
//! moves.
//!
//! The migrator re-derives each content id with the **same digest the builders
//! use** (an inline replica, pinned to the live builders by the convergence
//! tests), and self-checks each written event: the recorded id must equal the
//! digest of its own stored payload, so a future re-record reading the same
//! payload converges. Events are processed in dependency order (a fixpoint over the
//! reference graph), so each reference is remapped before the event that folds it.
//!
//! It reads each event as raw JSON (bypassing the strict read path), rewrites the
//! ids and artifact bindings, re-derives every dependent id, and writes the result
//! into a fresh store the strict read path accepts. Every event's record hash
//! moves, so inline signatures are re-signed and detached co-signatures re-homed
//! with held keys. There is no `sigVersion` bump. A detached co-signature whose
//! attester key is not held cannot be re-attested and is dropped with a warning.
//! `docs/store-migration.md` is the durable architecture record.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value};

use super::EventStore;
use super::object_artifact::{
    build_object_artifact_v2, decode_and_validate_object_artifact, object_artifact_path,
};
use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::crypto::EventSigner;
use crate::error::{Result, ShoreError};
use crate::keys::{FileEd25519Signer, KeyCustody, list_keys_in, load_signer_in};
use crate::model::{DiffSnapshot, EventId, ObjectId};
use crate::session::event::{
    EventSignature, EventSignatureRecordedPayload, EventTarget, EventToBeSigned, EventType,
    ShoreEvent, Writer, event_signature_pre_authentication_encoding,
};
use crate::session::{EventSigningOptions, sign_event_if_requested};
use crate::storage::{Durability, LocalStorage};

/// Inputs for one migration pass. Generic: all three locations are parameters,
/// with no built-in repo, key, or path assumptions.
#[derive(Clone, Debug)]
pub struct MigrateOptions {
    /// The source store directory to read (the dir holding `events/` and
    /// `artifacts/`).
    pub source_store_dir: PathBuf,
    /// A fresh, empty store directory to write the re-keyed store into.
    pub target_store_dir: PathBuf,
    /// The keystore directory holding the signers' private keys, used to re-sign
    /// inline signatures and re-attest held-key co-signatures.
    pub keystore_dir: PathBuf,
}

/// What one migration pass did. The owner-run step reads this to confirm the
/// migration was lossless and the re-keyed store self-validated.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrateSummary {
    /// Events written to the re-keyed store (transformed, passed-through, and
    /// re-attested co-signatures).
    pub events_migrated: usize,
    /// Events copied through verbatim because they carry no content id, artifact
    /// binding, or signature that the rename touches.
    pub events_passed_through: usize,
    /// Content ids re-derived under the new digest key (associations, withdrawals,
    /// observations, validations, assessments, input-requests, responses).
    pub content_ids_rederived: usize,
    /// Diff artifacts re-hashed under the renamed `object` schema + directory.
    pub artifacts_rehashed: usize,
    /// Inline signatures re-signed with the original signer's held key.
    pub inline_signatures_resigned: usize,
    /// Detached co-signatures re-attested with the attester's held key.
    pub cosignatures_reattested: usize,
    /// Detached co-signatures dropped because the attester's key is not held (or
    /// the target did not survive), counted and warned, never silent.
    pub cosignatures_dropped: usize,
    /// Whether the re-keyed store passed its self-check (`list_events` rebuilds
    /// cleanly under the strict read path, `SessionState::from_events` succeeds,
    /// and no stale wire token survives).
    pub self_check_passed: bool,
}

/// Migrate the store at `source_store_dir` into a fresh `object`-shaped store at
/// `target_store_dir`, re-signing with keys from `keystore_dir`.
pub fn migrate_object_rename(options: MigrateOptions) -> Result<MigrateSummary> {
    let raw = read_raw_events(&options.source_store_dir)?;
    let keystore = build_keystore_index(&options.keystore_dir)?;
    let mut summary = MigrateSummary::default();

    // Pass 0: re-hash diff artifacts under the renamed `object` schema/directory
    // and build the old-content-hash -> new-content-hash remap each referencing
    // capture is rewritten against.
    let artifact_remap = migrate_artifacts(&options, &mut summary)?;

    // Pass 1: re-emit every non-co-signature event in dependency order. A content
    // event is processed once every content id it references is already re-derived;
    // the fixpoint guarantees that order regardless of `occurredAt` ties. Captures
    // and content-id-free events have no references and land in the first round.
    let target = EventStore::open(&options.target_store_dir);
    let mut content_remap: BTreeMap<String, String> = BTreeMap::new();
    let mut old_to_new: BTreeMap<String, ShoreEvent> = BTreeMap::new();

    let mut pending: Vec<&Value> = raw
        .iter()
        .filter(|value| value["eventType"] != "event_signature_recorded")
        .collect();
    pending.sort_by(|a, b| occurred_at_str(a).cmp(occurred_at_str(b)));

    while !pending.is_empty() {
        let mut progressed = false;
        let mut still_pending: Vec<&Value> = Vec::with_capacity(pending.len());
        for value in pending {
            if !references_resolved(value, &content_remap)? {
                still_pending.push(value);
                continue;
            }
            let old_event_id = event_id_of(value)?;
            let event = transform_pass_one(
                value,
                &artifact_remap,
                &mut content_remap,
                &keystore,
                &options,
                &mut summary,
            )?;
            record_into(&target, &event)?;
            old_to_new.insert(old_event_id, event);
            progressed = true;
        }
        if !progressed {
            return Err(migrate_error(
                "unresolved content-id references or a dependency cycle in the event graph",
            ));
        }
        pending = still_pending;
    }

    // Pass 2: re-home detached co-signatures, in dependency order (every target is
    // written above).
    for value in &raw {
        if value["eventType"] != "event_signature_recorded" {
            continue;
        }
        rehome_cosignature(
            value,
            &target,
            &old_to_new,
            &keystore,
            &options,
            &mut summary,
        )?;
    }

    // Copy note/body artifacts verbatim: they are content-addressed by a body hash
    // the rename never changes, so the migrated events still resolve them.
    copy_dir_verbatim(
        &options.source_store_dir.join("artifacts/notes"),
        &options.target_store_dir.join("artifacts/notes"),
    )?;

    // Self-check: the re-keyed store must list cleanly under the strict read path,
    // rebuild its projection, and carry no stale wire token.
    let events = target.list_events()?;
    let _state = crate::session::SessionState::from_events(&events)?;
    verify_no_stale_wire(&options.target_store_dir, &events)?;
    summary.self_check_passed = true;

    Ok(summary)
}

fn read_raw_events(source_store_dir: &Path) -> Result<Vec<Value>> {
    let store = EventStore::open(source_store_dir);
    let mut events = Vec::new();
    for name in store.list_event_file_names()? {
        let path = source_store_dir.join("events").join(&name);
        let bytes = std::fs::read(&path)
            .map_err(|error| migrate_error(&format!("read {}: {error}", path.display())))?;
        events.push(serde_json::from_slice(&bytes)?);
    }
    Ok(events)
}

fn build_keystore_index(keystore_dir: &Path) -> Result<BTreeMap<String, String>> {
    let mut index = BTreeMap::new();
    for info in list_keys_in(keystore_dir)? {
        if info.custody() == KeyCustody::File {
            index.insert(info.signer_id().as_str().to_owned(), info.name().to_owned());
        }
    }
    Ok(index)
}

// ---------------------------------------------------------------------------
// Pass 0: artifacts
// ---------------------------------------------------------------------------

/// Re-hash every diff artifact under the renamed `object` schema and directory.
/// The body's `object_id` field already carries the renamed identity (an earlier
/// migration), so only the `schema` value (and thus the content hash) and the
/// directory change. Returns the old-content-hash -> new-content-hash remap each
/// referencing capture is rewritten against. An already-`shore.object` artifact is
/// validated and copied verbatim; a content-removed (absent) artifact is simply
/// not present and its referencing capture keeps the old binding hash.
fn migrate_artifacts(
    options: &MigrateOptions,
    summary: &mut MigrateSummary,
) -> Result<BTreeMap<String, String>> {
    let mut remap = BTreeMap::new();
    let source_dir = options.source_store_dir.join("artifacts/snapshots");
    let entries = match std::fs::read_dir(&source_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(remap),
        Err(error) => {
            return Err(migrate_error(&format!(
                "read {}: {error}",
                source_dir.display()
            )));
        }
    };

    std::fs::create_dir_all(options.target_store_dir.join("artifacts/objects")).map_err(
        |error| {
            migrate_error(&format!(
                "create {}: {error}",
                options.target_store_dir.join("artifacts/objects").display()
            ))
        },
    )?;
    let storage = LocalStorage::new(&options.target_store_dir);

    for entry in entries {
        let entry = entry.map_err(|error| migrate_error(&format!("read dir entry: {error}")))?;
        if !entry
            .file_type()
            .map(|kind| kind.is_file())
            .unwrap_or(false)
        {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let bytes = std::fs::read(&path)
            .map_err(|error| migrate_error(&format!("read {}: {error}", path.display())))?;
        let value: Value = serde_json::from_slice(&bytes)?;

        if value["schema"] == "shore.object" {
            // Already migrated: validate before laundering it through as a trusted
            // clean artifact. The strict decoder re-checks the version and content
            // hash, so a tampered already-`object` artifact in a mixed store is
            // rejected rather than copied verbatim — the event self-check would not
            // otherwise catch a bad copied artifact.
            let artifact = decode_and_validate_object_artifact(&bytes)?;
            let target_path =
                object_artifact_path(&options.target_store_dir, &artifact.snapshot.object_id);
            storage.create_file_exclusive(&target_path, &bytes, Durability::Durable)?;
            continue;
        }

        let old_content_hash = string_field(&value, "contentHash")?;
        validate_legacy_artifact_integrity(&value, &old_content_hash)?;

        let snapshot: DiffSnapshot = serde_json::from_value(value["snapshot"].clone())?;
        let artifact = build_object_artifact_v2(snapshot)?;

        let target_path =
            object_artifact_path(&options.target_store_dir, &artifact.snapshot.object_id);
        storage.create_file_exclusive(
            &target_path,
            &serde_json::to_vec(&artifact)?,
            Durability::Durable,
        )?;
        remap.insert(old_content_hash, artifact.content_hash.clone());
        summary.artifacts_rehashed += 1;
    }

    Ok(remap)
}

/// Validate a legacy artifact's stored content hash over its own body, so a
/// tampered artifact is rejected rather than laundered into the clean format.
fn validate_legacy_artifact_integrity(value: &Value, content_hash: &str) -> Result<()> {
    let mut material = value.clone();
    let Some(object) = material.as_object_mut() else {
        return Err(migrate_error("diff artifact must be an object"));
    };
    object.remove("contentHash");
    let expected = sha256_json_prefixed(&material)?;
    if expected != content_hash {
        return Err(migrate_error(&format!(
            "diff artifact content hash mismatch (stored {content_hash}, recomputed {expected})"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Pass 1: events
// ---------------------------------------------------------------------------

/// Re-emit one non-co-signature event under the `object`-shaped wire, dispatching
/// on its kind. The caller has confirmed every content id this event references is
/// already re-derived.
fn transform_pass_one(
    value: &Value,
    artifact_remap: &BTreeMap<String, String>,
    content_remap: &mut BTreeMap<String, String>,
    keystore: &BTreeMap<String, String>,
    options: &MigrateOptions,
    summary: &mut MigrateSummary,
) -> Result<ShoreEvent> {
    let event_type = value["eventType"]
        .as_str()
        .ok_or_else(|| migrate_error("event is missing eventType"))?;

    if event_type == "work_object_proposed" {
        let event = transform_capture(value, artifact_remap)?;
        let event = resign_if_signed(event, value, keystore, options, summary)?;
        summary.events_migrated += 1;
        return Ok(event);
    }

    if let Some(kind) = ContentKind::from_event_type(event_type) {
        let event = transform_content_event(value, kind, content_remap)?;
        let event = resign_if_signed(event, value, keystore, options, summary)?;
        summary.content_ids_rederived += 1;
        summary.events_migrated += 1;
        return Ok(event);
    }

    // A content-id-free, artifact-free event (e.g. a journal-scoped structural
    // event): nothing the rename touches, so it passes through verbatim.
    let event = passthrough_event(value)?;
    summary.events_passed_through += 1;
    summary.events_migrated += 1;
    Ok(event)
}

/// Re-emit a capture: rename the artifact binding key
/// (`snapshotArtifactContentHash` -> `objectArtifactContentHash`) and remap its
/// value to the re-hashed artifact. The capture's idempotency key folds only the
/// revision id, so the event id is stable; only the payload hash (and thus the
/// record hash) moves, so the caller re-signs.
fn transform_capture(
    value: &Value,
    artifact_remap: &BTreeMap<String, String>,
) -> Result<ShoreEvent> {
    let mut migrated = value.clone();

    if let Some(work_object) = migrated["payload"]
        .get_mut("workObject")
        .and_then(Value::as_object_mut)
    {
        let old_hash = work_object
            .remove("snapshotArtifactContentHash")
            .or_else(|| work_object.remove("objectArtifactContentHash"));
        if let Some(Value::String(old_hash)) = old_hash {
            // The re-hashed artifact's hash when present; a content-removed artifact
            // is absent from the remap and keeps its old binding hash (consistent
            // with the matching artifact-removal event).
            let new_hash = artifact_remap.get(&old_hash).cloned().unwrap_or(old_hash);
            work_object.insert(
                "objectArtifactContentHash".to_owned(),
                Value::String(new_hash),
            );
        }
    }

    migrated["signer"] = Value::Null;
    migrated["signature"] = Value::Null;

    let mut event: ShoreEvent = serde_json::from_value(migrated)?;
    event.payload_hash = sha256_json_prefixed(&event.payload)?;
    event.event_id = derive_event_id(&event.idempotency_key);
    Ok(event)
}

/// Re-emit a content event: re-derive its content id from the current payload over
/// the current digest with references already remapped, rewrite the id and
/// idempotency key, then re-derive the payload hash and event id.
///
/// The migrator re-derives from the **current** payload rather than flipping the
/// legacy digest key, because the stored content ids are frozen at the material
/// they were minted from — earlier wire reshapes re-keyed the payloads (the
/// revision id value, the target shape, the terminology) without re-deriving the
/// opaque content ids. So a stored id is not reproducible from the current payload
/// under either key, and the convergence target is the id a fresh re-record mints
/// today (the current builder over the current payload), which this re-derivation
/// reproduces. The post-pass self-check confirms every written id matches its
/// payload, and the convergence tests confirm the replica matches the live builder.
fn transform_content_event(
    value: &Value,
    kind: ContentKind,
    content_remap: &mut BTreeMap<String, String>,
) -> Result<ShoreEvent> {
    let own_field = kind.own_field();
    let stored_id = value["payload"][own_field]
        .as_str()
        .ok_or_else(|| migrate_error(&format!("{} event is missing {own_field}", kind.as_str())))?
        .to_owned();

    // Remap every reference this event folds, then re-derive its own id from the
    // remapped material under the current digest.
    let mut migrated = value.clone();
    remap_references(&mut migrated, content_remap)?;
    let new_id = compute_content_id(&migrated, kind, REVISION_KEY)?;
    content_remap.insert(stored_id, new_id.clone());

    migrated["payload"][own_field] = Value::String(new_id);

    // Re-key the idempotency key: substitute every re-derived id it embeds. The
    // default dedupe key embeds the event's own id (or its association/request id);
    // an explicit dedupe key embeds neither, so this is a no-op and the event id
    // stays stable.
    if let Some(key) = migrated["idempotencyKey"].as_str() {
        let mut rekeyed = key.to_owned();
        for (old, new) in content_remap.iter() {
            if rekeyed.contains(old.as_str()) {
                rekeyed = rekeyed.replace(old.as_str(), new.as_str());
            }
        }
        migrated["idempotencyKey"] = Value::String(rekeyed);
    }

    migrated["signer"] = Value::Null;
    migrated["signature"] = Value::Null;

    let mut event: ShoreEvent = serde_json::from_value(migrated)?;
    event.payload_hash = sha256_json_prefixed(&event.payload)?;
    event.event_id = derive_event_id(&event.idempotency_key);
    Ok(event)
}

/// The set of content events whose id is a digest re-derived by this migrator.
#[derive(Clone, Copy)]
enum ContentKind {
    Observation,
    Assessment,
    Validation,
    InputRequestOpened,
    InputRequestResponded,
    CommitAssociated,
    RefAssociated,
    CommitWithdrawn,
    RefWithdrawn,
}

impl ContentKind {
    fn from_event_type(event_type: &str) -> Option<Self> {
        Some(match event_type {
            "review_observation_recorded" => Self::Observation,
            "review_assessment_recorded" => Self::Assessment,
            "validation_check_recorded" => Self::Validation,
            "input_request_opened" => Self::InputRequestOpened,
            "input_request_responded" => Self::InputRequestResponded,
            "revision_commit_associated" => Self::CommitAssociated,
            "revision_ref_associated" => Self::RefAssociated,
            "revision_commit_withdrawn" => Self::CommitWithdrawn,
            "revision_ref_withdrawn" => Self::RefWithdrawn,
            _ => return None,
        })
    }

    fn own_field(self) -> &'static str {
        match self {
            Self::Observation => "observationId",
            Self::Assessment => "assessmentId",
            Self::Validation => "validationCheckId",
            Self::InputRequestOpened => "inputRequestId",
            Self::InputRequestResponded => "inputRequestResponseId",
            Self::CommitAssociated => "commitAssociationId",
            Self::RefAssociated => "refAssociationId",
            Self::CommitWithdrawn => "commitWithdrawalId",
            Self::RefWithdrawn => "refWithdrawalId",
        }
    }

    fn id_prefix(self) -> &'static str {
        match self {
            Self::Observation => "obs",
            Self::Assessment => "assess",
            Self::Validation => "validation",
            Self::InputRequestOpened => "input-request",
            Self::InputRequestResponded => "input-request-response",
            Self::CommitAssociated => "assoc-commit",
            Self::RefAssociated => "assoc-ref",
            Self::CommitWithdrawn => "withdraw-commit",
            Self::RefWithdrawn => "withdraw-ref",
        }
    }

    fn as_str(self) -> &'static str {
        self.own_field()
    }
}

const REVISION_KEY: &str = "revisionId";

/// Recompute a content id from an event, folding the revision under `revision_key`
/// (`revisionId`). The `revision_key` parameter keeps the digest material explicit
/// and lets a test fold the legacy key. The material mirrors each builder
/// field-for-field; `sha256_json_prefixed` sorts object keys, so only array
/// elements are sorted here.
fn compute_content_id(value: &Value, kind: ContentKind, revision_key: &str) -> Result<String> {
    let payload = &value["payload"];
    let mut material = Map::new();

    match kind {
        ContentKind::Observation => {
            material.insert(revision_key.to_owned(), revision_id(value)?);
            material.insert("trackId".to_owned(), track_id(value)?);
            material.insert("target".to_owned(), payload_target(value)?);
            material.insert("title".to_owned(), required(payload, "title")?);
            material.insert(
                "bodyContentHash".to_owned(),
                optional(payload, "bodyContentHash"),
            );
            material.insert("tags".to_owned(), sorted_array(payload, "tags"));
            material.insert("confidence".to_owned(), optional(payload, "confidence"));
            material.insert(
                "supersedesObservationIds".to_owned(),
                sorted_array(payload, "supersedesObservationIds"),
            );
            material.insert("writerActorId".to_owned(), writer_actor_id(value)?);
        }
        ContentKind::Assessment => {
            material.insert(revision_key.to_owned(), revision_id(value)?);
            material.insert("trackId".to_owned(), track_id(value)?);
            material.insert("target".to_owned(), payload_target(value)?);
            material.insert("assessment".to_owned(), required(payload, "assessment")?);
            material.insert(
                "summaryContentHash".to_owned(),
                optional(payload, "summaryContentHash"),
            );
            material.insert(
                "replacesAssessmentIds".to_owned(),
                sorted_array(payload, "replacesAssessmentIds"),
            );
            material.insert(
                "relatedObservationIds".to_owned(),
                sorted_array(payload, "relatedObservationIds"),
            );
            material.insert(
                "relatedInputRequestIds".to_owned(),
                sorted_array(payload, "relatedInputRequestIds"),
            );
            material.insert("writerActorId".to_owned(), writer_actor_id(value)?);
        }
        ContentKind::Validation => {
            material.insert(revision_key.to_owned(), revision_id(value)?);
            material.insert("trackId".to_owned(), track_id(value)?);
            material.insert("target".to_owned(), payload_target(value)?);
            material.insert("checkName".to_owned(), required(payload, "checkName")?);
            material.insert("command".to_owned(), optional(payload, "command"));
            material.insert("status".to_owned(), required(payload, "status")?);
            material.insert("exitCode".to_owned(), optional(payload, "exitCode"));
            material.insert("trigger".to_owned(), required(payload, "trigger")?);
            material.insert(
                "sourceFingerprint".to_owned(),
                optional(payload, "sourceFingerprint"),
            );
            material.insert(
                "summaryContentHash".to_owned(),
                optional(payload, "summaryContentHash"),
            );
            material.insert("startedAt".to_owned(), optional(payload, "startedAt"));
            material.insert("completedAt".to_owned(), optional(payload, "completedAt"));
            material.insert(
                "logArtifactContentHashes".to_owned(),
                sorted_deduped_array(payload, "logArtifactContentHashes"),
            );
            material.insert("writerActorId".to_owned(), writer_actor_id(value)?);
        }
        ContentKind::InputRequestOpened => {
            material.insert(revision_key.to_owned(), revision_id(value)?);
            material.insert("trackId".to_owned(), track_id(value)?);
            material.insert("target".to_owned(), payload_target(value)?);
            material.insert("assertionMode".to_owned(), assertion_mode(value));
            material.insert("reasonCode".to_owned(), required(payload, "reasonCode")?);
            material.insert("title".to_owned(), required(payload, "title")?);
            material.insert(
                "bodyContentHash".to_owned(),
                optional(payload, "bodyContentHash"),
            );
            material.insert("writerActorId".to_owned(), writer_actor_id(value)?);
        }
        ContentKind::InputRequestResponded => {
            material.insert(
                "inputRequestId".to_owned(),
                required(payload, "inputRequestId")?,
            );
            material.insert("outcome".to_owned(), required(payload, "outcome")?);
            material.insert(
                "reasonContentHash".to_owned(),
                optional(payload, "reasonContentHash"),
            );
            material.insert("writerActorId".to_owned(), writer_actor_id(value)?);
        }
        ContentKind::CommitAssociated => {
            material.insert(revision_key.to_owned(), revision_id(value)?);
            material.insert("commitOid".to_owned(), commit_oid(value)?);
        }
        ContentKind::RefAssociated => {
            material.insert(revision_key.to_owned(), revision_id(value)?);
            material.insert("refName".to_owned(), required(payload, "refName")?);
            material.insert("headOid".to_owned(), required(payload, "headOid")?);
        }
        ContentKind::CommitWithdrawn => {
            material.insert(revision_key.to_owned(), revision_id(value)?);
            material.insert(
                "commitAssociationId".to_owned(),
                required(payload, "commitAssociationId")?,
            );
        }
        ContentKind::RefWithdrawn => {
            material.insert(revision_key.to_owned(), revision_id(value)?);
            material.insert(
                "refAssociationId".to_owned(),
                required(payload, "refAssociationId")?,
            );
        }
    }

    let digest = sha256_json_prefixed(&Value::Object(material))?;
    Ok(format!("{}:{digest}", kind.id_prefix()))
}

/// Whether every content id this event references has been re-derived. A capture,
/// association, or content-id-free event references nothing and resolves
/// immediately.
fn references_resolved(value: &Value, content_remap: &BTreeMap<String, String>) -> Result<bool> {
    let own_field = own_field_of(value);
    let mut resolved = true;
    let mut probe = value.clone();
    visit_reference_subtrees(&mut probe, own_field, &mut |string| {
        if is_rekeyable_content_id(string) && !content_remap.contains_key(string) {
            resolved = false;
        }
        Ok(())
    })?;
    Ok(resolved)
}

/// Remap every content-id reference this event folds (in the envelope subject, the
/// payload target, and the payload reference fields) through the old->new map. A
/// reference to a content id not yet defined is a dependency-order or dangling
/// reference and stops the migration.
fn remap_references(value: &mut Value, content_remap: &BTreeMap<String, String>) -> Result<()> {
    let own_field = own_field_of(value);
    visit_reference_subtrees(value, own_field, &mut |string| {
        if is_rekeyable_content_id(string) {
            match content_remap.get(string.as_str()) {
                Some(new) => *string = new.clone(),
                None => {
                    return Err(migrate_error(&format!(
                        "reference to content id {string} that has no re-derived form \
                         (dependency-order or dangling reference)"
                    )));
                }
            }
        }
        Ok(())
    })
}

/// The event's own content-id field name, if it is a content event. The reference
/// walk skips it: a field like `inputRequestId` or `commitAssociationId` is the own
/// id for one event type but a reference for another, so the own field must not be
/// scanned as a reference (it is not in the remap until the event is processed).
fn own_field_of(value: &Value) -> Option<&'static str> {
    event_type_str(value)
        .and_then(ContentKind::from_event_type)
        .map(ContentKind::own_field)
}

/// Walk the structural id-bearing subtrees of an event — the envelope target
/// (which carries the subject), the payload target, and the payload reference
/// fields except the event's own id — applying `visit` to every string. These
/// subtrees never hold free text, so a content-id substitution there cannot
/// corrupt a body/summary/title.
fn visit_reference_subtrees(
    value: &mut Value,
    own_field: Option<&str>,
    visit: &mut dyn FnMut(&mut String) -> Result<()>,
) -> Result<()> {
    visit_strings(&mut value["target"], visit)?;
    if let Some(payload) = value.get_mut("payload").and_then(Value::as_object_mut) {
        if let Some(target) = payload.get_mut("target") {
            visit_strings(target, visit)?;
        }
        for field in [
            "supersedesObservationIds",
            "replacesAssessmentIds",
            "relatedObservationIds",
            "relatedInputRequestIds",
            "commitAssociationId",
            "refAssociationId",
            "inputRequestId",
        ] {
            if Some(field) == own_field {
                continue;
            }
            if let Some(reference) = payload.get_mut(field) {
                visit_strings(reference, visit)?;
            }
        }
    }
    Ok(())
}

fn visit_strings(
    value: &mut Value,
    visit: &mut dyn FnMut(&mut String) -> Result<()>,
) -> Result<()> {
    match value {
        Value::String(string) => visit(string)?,
        Value::Array(items) => {
            for item in items {
                visit_strings(item, visit)?;
            }
        }
        Value::Object(object) => {
            for (_, child) in object {
                visit_strings(child, visit)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Whether a string is a content id this migrator re-derives (so a reference to it
/// must be remapped). The longer `input-request-response:` prefix is distinct from
/// `input-request:` (the byte after `input-request` is `-`, not `:`), so the set is
/// unambiguous.
fn is_rekeyable_content_id(string: &str) -> bool {
    const PREFIXES: [&str; 8] = [
        "obs:",
        "assess:",
        "input-request:",
        "input-request-response:",
        "assoc-commit:",
        "assoc-ref:",
        "withdraw-commit:",
        "withdraw-ref:",
    ];
    PREFIXES.iter().any(|prefix| string.starts_with(prefix))
}

// ---------------------------------------------------------------------------
// Digest material accessors
// ---------------------------------------------------------------------------

fn revision_id(value: &Value) -> Result<Value> {
    value["payload"]["target"]["revisionId"]
        .as_str()
        .map(|id| Value::String(id.to_owned()))
        .ok_or_else(|| migrate_error("event payload target is missing revisionId"))
}

fn track_id(value: &Value) -> Result<Value> {
    value["target"]["trackId"]
        .as_str()
        .map(|id| Value::String(id.to_owned()))
        .ok_or_else(|| migrate_error("event envelope target is missing trackId"))
}

fn writer_actor_id(value: &Value) -> Result<Value> {
    value["writer"]["actorId"]
        .as_str()
        .map(|id| Value::String(id.to_owned()))
        .ok_or_else(|| migrate_error("event writer is missing actorId"))
}

fn payload_target(value: &Value) -> Result<Value> {
    match value["payload"].get("target") {
        Some(target) => Ok(target.clone()),
        None => Err(migrate_error("event payload is missing target")),
    }
}

fn commit_oid(value: &Value) -> Result<Value> {
    value["payload"]["commit"]["commitOid"]
        .as_str()
        .map(|oid| Value::String(oid.to_owned()))
        .ok_or_else(|| migrate_error("commit association payload is missing commit.commitOid"))
}

/// The envelope `assertionMode` (advisory is skip-serialized, so an absent field
/// is the default advisory), serialized as the builder folds it.
fn assertion_mode(value: &Value) -> Value {
    Value::String(
        value
            .get("assertionMode")
            .and_then(Value::as_str)
            .unwrap_or("advisory")
            .to_owned(),
    )
}

fn required(payload: &Value, field: &str) -> Result<Value> {
    payload
        .get(field)
        .cloned()
        .ok_or_else(|| migrate_error(&format!("payload is missing required digest field {field}")))
}

/// An optional digest field: present value, or `null` when absent (matching the
/// builder's `Option::None -> null`).
fn optional(payload: &Value, field: &str) -> Value {
    payload.get(field).cloned().unwrap_or(Value::Null)
}

/// A repeated digest field: the array's string elements sorted, or `[]` when
/// absent (matching the builder's empty-vec fold, never `null`).
fn sorted_array(payload: &Value, field: &str) -> Value {
    let mut items: Vec<String> = payload
        .get(field)
        .and_then(Value::as_array)
        .map(|array| {
            array
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    items.sort();
    Value::Array(items.into_iter().map(Value::String).collect())
}

fn sorted_deduped_array(payload: &Value, field: &str) -> Value {
    let mut items: Vec<String> = payload
        .get(field)
        .and_then(Value::as_array)
        .map(|array| {
            array
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    items.sort();
    items.dedup();
    Value::Array(items.into_iter().map(Value::String).collect())
}

// ---------------------------------------------------------------------------
// Signing
// ---------------------------------------------------------------------------

fn passthrough_event(value: &Value) -> Result<ShoreEvent> {
    Ok(serde_json::from_value(value.clone())?)
}

/// Re-sign an event that was inline-signed, with the original signer's held key.
fn resign_if_signed(
    mut event: ShoreEvent,
    legacy: &Value,
    keystore: &BTreeMap<String, String>,
    options: &MigrateOptions,
    summary: &mut MigrateSummary,
) -> Result<ShoreEvent> {
    let Some(signer_did) = original_signer_did(legacy) else {
        return Ok(event);
    };
    match held_signer(keystore, options, &signer_did)? {
        Some(signer) => {
            sign_event_if_requested(&mut event, &EventSigningOptions::sign_with(signer))?;
            summary.inline_signatures_resigned += 1;
        }
        None => {
            eprintln!(
                "warning: inline signer {signer_did} is not held; leaving event {} unsigned",
                event.event_id.as_str()
            );
        }
    }
    Ok(event)
}

/// Re-home a detached co-signature: preserve a clean carrier whose target's record
/// hash is unchanged, otherwise re-attest it over the reshaped target (held
/// attester key) or drop it.
fn rehome_cosignature(
    value: &Value,
    target_store: &EventStore,
    old_to_new: &BTreeMap<String, ShoreEvent>,
    keystore: &BTreeMap<String, String>,
    options: &MigrateOptions,
    summary: &mut MigrateSummary,
) -> Result<()> {
    let attester_did = value["payload"]["attestingSigner"]
        .as_str()
        .ok_or_else(|| migrate_error("co-signature is missing attestingSigner"))?;
    let old_target_event_id = value["payload"]["targetEventId"]
        .as_str()
        .ok_or_else(|| migrate_error("co-signature is missing targetEventId"))?;
    let old_target_record_hash = value["payload"]["targetEventRecordHash"]
        .as_str()
        .ok_or_else(|| migrate_error("co-signature is missing targetEventRecordHash"))?;

    let Some(new_target) = old_to_new.get(old_target_event_id) else {
        eprintln!(
            "warning: co-signature target {old_target_event_id} did not survive migration; dropping the carrier"
        );
        summary.cosignatures_dropped += 1;
        return Ok(());
    };

    // A clean carrier whose target's record hash is unchanged still binds a valid
    // attestation; preserve it verbatim even if the attester key is not held. Any
    // carrier whose target's record hash moved (every migrated target, since the
    // serialized record changed) is re-attested or dropped.
    let new_target_record_hash = new_target.event_record_hash()?;
    if new_target.event_id.as_str() == old_target_event_id
        && new_target_record_hash == old_target_record_hash
    {
        let event = passthrough_event(value)?;
        record_into(target_store, &event)?;
        summary.events_passed_through += 1;
        summary.events_migrated += 1;
        return Ok(());
    }

    let Some(signer) = held_signer(keystore, options, attester_did)? else {
        eprintln!(
            "warning: co-signature attester {attester_did} is not held; dropping the carrier"
        );
        summary.cosignatures_dropped += 1;
        return Ok(());
    };

    // Re-attest over the reshaped target: the attestation signs the target's
    // signer-inclusive TBS view (naming the attester), and the carrier binds the
    // target's signer-exclusive record hash. Both recompute against the new target.
    let attester_id = signer.signer_id().clone();
    let tbs = EventToBeSigned::from_event(new_target, &attester_id)?;
    let pae = event_signature_pre_authentication_encoding(&tbs)?;
    let attestation = EventSignature::ed25519_v1(signer.sign_event_message(&pae)?);

    let target_event_record_hash = new_target.event_record_hash()?;
    let idempotency_key = EventSignatureRecordedPayload::idempotency_key(
        &target_event_record_hash,
        &attester_id,
        attestation.sig.as_str(),
    );
    let payload = EventSignatureRecordedPayload {
        target_event_id: new_target.event_id.clone(),
        target_event_record_hash,
        attesting_signer: attester_id,
        attestation,
        inclusion_proof: None,
    };
    let carrier = ShoreEvent::new(
        EventType::EventSignatureRecorded,
        idempotency_key,
        EventTarget::for_journal(new_target.target.journal_id.clone()),
        writer_of(value)?,
        payload,
        occurred_at_of(value)?,
    )?;
    record_into(target_store, &carrier)?;
    summary.cosignatures_reattested += 1;
    summary.events_migrated += 1;
    Ok(())
}

fn original_signer_did(value: &Value) -> Option<String> {
    value.get("signature")?;
    if let Some(signer) = value.get("signer").and_then(Value::as_str) {
        return Some(signer.to_owned());
    }
    value["writer"]["actorId"].as_str().map(str::to_owned)
}

fn held_signer(
    keystore: &BTreeMap<String, String>,
    options: &MigrateOptions,
    did: &str,
) -> Result<Option<FileEd25519Signer>> {
    match keystore.get(did) {
        Some(name) => Ok(Some(load_signer_in(&options.keystore_dir, name)?)),
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Self-check
// ---------------------------------------------------------------------------

/// Confirm no stale wire token survives in the migrated store: no event carries a
/// legacy `reviewUnitId` or `snapshotArtifactContentHash` object **key** (a
/// free-text body may legitimately mention the words, so only structural keys are
/// scanned), no artifact lives under the old `artifacts/snapshots/` directory or
/// self-declares the `shore.snapshot` schema, and every capture's artifact binding
/// resolves to a present, hash-valid artifact (or a content-removed one).
fn verify_no_stale_wire(target_store_dir: &Path, events: &[ShoreEvent]) -> Result<()> {
    for event in events {
        let value = serde_json::to_value(event)?;
        for key in ["reviewUnitId", "snapshotArtifactContentHash"] {
            if json_contains_key(&value, key) {
                return Err(migrate_error(&format!(
                    "migrated event {} still carries the stale wire key {key}",
                    event.event_id.as_str()
                )));
            }
        }
    }

    let snapshots_dir = target_store_dir.join("artifacts/snapshots");
    if snapshots_dir.exists() {
        return Err(migrate_error(
            "migrated store still has the legacy artifacts/snapshots/ directory",
        ));
    }

    let objects_dir = target_store_dir.join("artifacts/objects");
    if let Ok(entries) = std::fs::read_dir(&objects_dir) {
        for entry in entries {
            let path = entry
                .map_err(|error| migrate_error(&format!("read dir entry: {error}")))?
                .path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let bytes = std::fs::read(&path)
                .map_err(|error| migrate_error(&format!("read {}: {error}", path.display())))?;
            // The strict decoder re-validates version + content hash; it also
            // confirms the body parses as the object-scoped artifact.
            decode_and_validate_object_artifact(&bytes)?;
            let value: Value = serde_json::from_slice(&bytes)?;
            if value["schema"] == "shore.snapshot" {
                return Err(migrate_error(&format!(
                    "migrated artifact {} still declares schema shore.snapshot",
                    path.display()
                )));
            }
        }
    }

    // Every content event must be convergence-ready: the recorded id must equal
    // the digest of its own stored payload, so a fresh re-record reading the same
    // payload dedups rather than forks. This catches an inconsistent reference
    // remap or idempotency-key rewrite that left an id and its payload disagreeing.
    for event in events {
        let value = serde_json::to_value(event)?;
        let Some(kind) = event_type_str(&value).and_then(ContentKind::from_event_type) else {
            continue;
        };
        let stored_id = value["payload"][kind.own_field()]
            .as_str()
            .ok_or_else(|| migrate_error("migrated content event is missing its id"))?;
        let recomputed = compute_content_id(&value, kind, REVISION_KEY)?;
        if recomputed != stored_id {
            return Err(migrate_error(&format!(
                "migrated {} id {stored_id} does not match its own payload digest {recomputed}",
                kind.as_str()
            )));
        }
    }

    // Each capture's binding hash must resolve to its artifact's recomputed hash
    // (a content-removed artifact is absent and skipped).
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::WorkObjectProposed)
    {
        let Some(binding) = event.payload["workObject"]["objectArtifactContentHash"].as_str()
        else {
            continue;
        };
        let Some(object_id) = event.payload["workObject"]["revision"]["objectId"].as_str() else {
            continue;
        };
        let path = object_artifact_path(target_store_dir, &ObjectId::new(object_id));
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(migrate_error(&format!("read {}: {error}", path.display())));
            }
        };
        let artifact = decode_and_validate_object_artifact(&bytes)?;
        if artifact.content_hash != binding {
            return Err(migrate_error(&format!(
                "capture binding {binding} does not match artifact hash {} for {object_id}",
                artifact.content_hash
            )));
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// Whether `key` appears anywhere as an object key in `value` (a structural scan
/// that ignores string values, so a free-text mention of the word is not a match).
fn json_contains_key(value: &Value, key: &str) -> bool {
    match value {
        Value::Object(object) => {
            object.contains_key(key) || object.values().any(|child| json_contains_key(child, key))
        }
        Value::Array(items) => items.iter().any(|item| json_contains_key(item, key)),
        _ => false,
    }
}

fn derive_event_id(idempotency_key: &str) -> EventId {
    EventId::new(format!(
        "evt:sha256:{}",
        sha256_bytes_hex(idempotency_key.as_bytes())
    ))
}

fn record_into(store: &EventStore, event: &ShoreEvent) -> Result<()> {
    store.record_event_once(event)?;
    Ok(())
}

fn event_id_of(value: &Value) -> Result<String> {
    value["eventId"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| migrate_error("event is missing eventId"))
}

fn occurred_at_str(value: &Value) -> &str {
    value["occurredAt"].as_str().unwrap_or("")
}

fn event_type_str(value: &Value) -> Option<&str> {
    value["eventType"].as_str()
}

fn writer_of(value: &Value) -> Result<Writer> {
    Ok(serde_json::from_value(value["writer"].clone())?)
}

fn occurred_at_of(value: &Value) -> Result<String> {
    value["occurredAt"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| migrate_error("event is missing occurredAt"))
}

fn string_field(value: &Value, field: &str) -> Result<String> {
    value[field]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| migrate_error(&format!("artifact is missing {field}")))
}

fn copy_dir_verbatim(source: &Path, target: &Path) -> Result<()> {
    let entries = match std::fs::read_dir(source) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(migrate_error(&format!(
                "read {}: {error}",
                source.display()
            )));
        }
    };
    std::fs::create_dir_all(target)
        .map_err(|error| migrate_error(&format!("create {}: {error}", target.display())))?;
    for entry in entries {
        let entry = entry.map_err(|error| migrate_error(&format!("read dir entry: {error}")))?;
        if entry
            .file_type()
            .map(|kind| kind.is_file())
            .unwrap_or(false)
        {
            let to = target.join(entry.file_name());
            std::fs::copy(entry.path(), &to)
                .map_err(|error| migrate_error(&format!("copy {}: {error}", to.display())))?;
        }
    }
    Ok(())
}

fn migrate_error(message: &str) -> ShoreError {
    ShoreError::Message(format!("object migrate: {message}"))
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use serde_json::json;

    use super::*;
    use crate::model::{ValidationStatus, ValidationTrigger};
    use crate::session::event::{
        AssertionMode, InputRequestReasonCode, InputRequestResponseOutcome, ReviewAssessment,
        ShoreEvent,
    };
    use crate::session::{
        AssessmentAddOptions, AssociateCommitOptions, CaptureOptions, InputRequestOpenOptions,
        InputRequestRespondOptions, InputRequestTargetSelector, ObservationAddOptions,
        ValidationAddOptions, WithdrawCommitOptions, associate_commit, capture_worktree_review,
        open_input_request, record_assessment, record_observation, record_validation_check,
        respond_input_request, withdraw_commit,
    };

    /// A minimal git repo with one staged change, the substrate every workflow
    /// landing needs.
    struct TestRepo {
        root: tempfile::TempDir,
    }

    impl TestRepo {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("temp repo dir");
            let repo = Self { root };
            repo.git(["init"]);
            repo.git(["config", "user.name", "Shore Tests"]);
            repo.git(["config", "user.email", "shore-tests@example.com"]);
            repo.git(["config", "commit.gpgsign", "false"]);
            repo.write("src/lib.rs", "pub fn value() -> u32 {\n    1\n}\n");
            repo.commit_all("base");
            repo.write("src/lib.rs", "pub fn value() -> u32 {\n    2\n}\n");
            repo
        }

        fn path(&self) -> &Path {
            self.root.path()
        }

        fn store_dir(&self) -> PathBuf {
            crate::git::git_common_dir(self.path())
                .unwrap()
                .join("shore")
        }

        fn write(&self, path: &str, contents: &str) {
            let path = self.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }

        fn commit_all(&self, message: &str) {
            self.git(["add", "."]);
            self.git(["commit", "-m", message]);
        }

        fn git<I, S>(&self, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<std::ffi::OsStr>,
        {
            let output = Command::new("git")
                .args(args)
                .current_dir(self.path())
                .output()
                .expect("run git");
            assert!(output.status.success(), "git failed: {output:?}");
        }
    }

    fn read_event(store_dir: &Path, event_type: &str) -> Value {
        for entry in std::fs::read_dir(store_dir.join("events")).unwrap() {
            let value: Value =
                serde_json::from_slice(&std::fs::read(entry.unwrap().path()).unwrap()).unwrap();
            if value["eventType"] == event_type {
                return value;
            }
        }
        panic!("no {event_type} event in {}", store_dir.display());
    }

    /// Each content event's recorded id must equal the digest this migrator
    /// recomputes from the event's payload. The events are minted by the live
    /// workflow (the production builders), so this pins the migrator's inline
    /// digest replica to every live builder field-for-field — the guarantee that
    /// a migrated id converges with a fresh re-record.
    #[test]
    fn the_digest_replica_matches_every_live_builder() {
        let repo = TestRepo::new();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let observation = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:tester")
                .with_title("a finding")
                .with_body("the body"),
        )
        .unwrap();

        record_validation_check(
            ValidationAddOptions::new(repo.path())
                .with_track("agent:tester")
                .with_check_name("just test")
                .with_command("cargo test")
                .with_status(ValidationStatus::Passed)
                .with_trigger(ValidationTrigger::Manual),
        )
        .unwrap();

        let request = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("agent:tester")
                .with_title("a question")
                .with_body("which way?")
                .with_target(InputRequestTargetSelector::Revision)
                .with_assertion_mode(AssertionMode::Operative)
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired),
        )
        .unwrap();

        respond_input_request(
            InputRequestRespondOptions::new(repo.path(), request.input_request_id.clone())
                .with_outcome(InputRequestResponseOutcome::Approved)
                .with_reason("approved"),
        )
        .unwrap();

        record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_track("agent:tester")
                .with_assessment(ReviewAssessment::Accepted)
                .with_summary("looks good"),
        )
        .unwrap();

        let association =
            associate_commit(AssociateCommitOptions::new(repo.path(), "HEAD").with_track("h:k"))
                .unwrap();
        withdraw_commit(
            WithdrawCommitOptions::new(repo.path(), association.commit_association_id.clone())
                .with_track("h:k"),
        )
        .unwrap();

        let store_dir = repo.store_dir();
        for (event_type, kind) in [
            ("review_observation_recorded", ContentKind::Observation),
            ("validation_check_recorded", ContentKind::Validation),
            ("input_request_opened", ContentKind::InputRequestOpened),
            (
                "input_request_responded",
                ContentKind::InputRequestResponded,
            ),
            ("review_assessment_recorded", ContentKind::Assessment),
            ("revision_commit_associated", ContentKind::CommitAssociated),
            ("revision_commit_withdrawn", ContentKind::CommitWithdrawn),
        ] {
            let value = read_event(&store_dir, event_type);
            let stored_id = value["payload"][kind.own_field()].as_str().unwrap();
            let recomputed = compute_content_id(&value, kind, REVISION_KEY).unwrap();
            assert_eq!(
                recomputed, stored_id,
                "replica diverged from the live builder for {event_type}"
            );
        }

        // Sanity: the observation id the workflow returned is the one we pinned.
        let observation_value = read_event(&store_dir, "review_observation_recorded");
        assert_eq!(
            observation_value["payload"]["observationId"],
            json!(observation.observation_id.as_str())
        );
    }

    /// End-to-end convergence: a pre-rename store, migrated, dedups a fresh
    /// re-record of the same fact. This is the decisive gate — the self-check
    /// cannot catch a wrong content id (ids are opaque to the read path), so the
    /// live workflow re-recording to `Existing` is the proof the migrated id is the
    /// one the builder mints today.
    #[test]
    fn a_migrated_observation_converges_with_a_fresh_re_record() {
        let repo = TestRepo::new();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let observation = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:tester")
                .with_title("a finding")
                .with_body("the body"),
        )
        .unwrap();

        let store_dir = repo.store_dir();
        let reference_observation_id = observation.observation_id.as_str().to_owned();

        // Downgrade the reference store into a synthetic pre-rename store: the diff
        // artifact reverts to the `shore.snapshot` schema under `artifacts/snapshots/`,
        // the capture binds it by the legacy `snapshotArtifactContentHash` key, and
        // the observation's content id is replaced by an unrelated "frozen" id (as a
        // real store carries ids minted from pre-reshape material).
        let legacy = tempfile::tempdir().unwrap();
        let legacy_dir = legacy.path();
        std::fs::create_dir_all(legacy_dir.join("events")).unwrap();
        std::fs::create_dir_all(legacy_dir.join("artifacts/snapshots")).unwrap();

        // Downgrade the artifact.
        let object_id = capture.object_id.as_str();
        let object_stem = sha256_bytes_hex(object_id.as_bytes());
        let object_artifact: Value = serde_json::from_slice(
            &std::fs::read(
                store_dir
                    .join("artifacts/objects")
                    .join(format!("{object_stem}.json")),
            )
            .unwrap(),
        )
        .unwrap();
        let mut snapshot_body = object_artifact.clone();
        let snapshot_object = snapshot_body.as_object_mut().unwrap();
        snapshot_object.insert("schema".to_owned(), json!("shore.snapshot"));
        snapshot_object.remove("contentHash");
        let legacy_hash = sha256_json_prefixed(&snapshot_body).unwrap();
        snapshot_body
            .as_object_mut()
            .unwrap()
            .insert("contentHash".to_owned(), json!(legacy_hash));
        std::fs::write(
            legacy_dir
                .join("artifacts/snapshots")
                .join(format!("{object_stem}.json")),
            serde_json::to_vec(&snapshot_body).unwrap(),
        )
        .unwrap();

        // Downgrade the capture: rename the binding key + value, stable event id.
        let mut capture_event = read_event(&store_dir, "work_object_proposed");
        let work_object = capture_event["payload"]["workObject"]
            .as_object_mut()
            .unwrap();
        work_object.remove("objectArtifactContentHash");
        work_object.insert("snapshotArtifactContentHash".to_owned(), json!(legacy_hash));
        write_legacy_event(legacy_dir, &finalize_legacy(capture_event));

        // Downgrade the observation: replace its id with a frozen placeholder.
        let mut observation_event = read_event(&store_dir, "review_observation_recorded");
        let frozen_id = format!(
            "obs:sha256:{}",
            sha256_bytes_hex(format!("legacy:{reference_observation_id}").as_bytes())
        );
        observation_event["payload"]["observationId"] = json!(frozen_id);
        let key = observation_event["idempotencyKey"]
            .as_str()
            .unwrap()
            .replace(&reference_observation_id, &frozen_id);
        observation_event["idempotencyKey"] = json!(key);
        write_legacy_event(legacy_dir, &finalize_legacy(observation_event));

        // Migrate the legacy store back into the repo's store, then re-record.
        std::fs::remove_dir_all(&store_dir).unwrap();
        let summary = migrate_object_rename(MigrateOptions {
            source_store_dir: legacy_dir.to_path_buf(),
            target_store_dir: store_dir.clone(),
            keystore_dir: legacy_dir.join("keys"),
        })
        .unwrap();
        assert!(summary.self_check_passed);
        assert_eq!(summary.artifacts_rehashed, 1);

        // The migrated observation recovered the live-builder id.
        let migrated_observation = read_event(&store_dir, "review_observation_recorded");
        assert_eq!(
            migrated_observation["payload"]["observationId"],
            json!(reference_observation_id),
            "the migrator must recover the content id the live builder mints"
        );

        // The decisive gate: re-recording the same fact dedups.
        let re_recorded = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:tester")
                .with_title("a finding")
                .with_body("the body"),
        )
        .unwrap();
        assert_eq!(
            re_recorded.events_created, 0,
            "a fresh re-record must converge with the migrated observation, not fork"
        );
        assert_eq!(
            re_recorded.observation_id.as_str(),
            reference_observation_id
        );
    }

    /// Re-derive the payload hash and event id of a downgraded event so the strict
    /// read path accepts it on the way back through migration. The event id is
    /// stable iff the idempotency key is unchanged.
    fn finalize_legacy(mut value: Value) -> Value {
        value["payloadHash"] = json!(sha256_json_prefixed(&value["payload"]).unwrap());
        let key = value["idempotencyKey"].as_str().unwrap();
        value["eventId"] = json!(derive_event_id(key).as_str());
        value["signer"] = Value::Null;
        value["signature"] = Value::Null;
        value
    }

    fn write_legacy_event(legacy_dir: &Path, value: &Value) {
        let key = value["idempotencyKey"].as_str().unwrap();
        let stem = sha256_bytes_hex(key.as_bytes());
        std::fs::write(
            legacy_dir.join("events").join(format!("{stem}.json")),
            serde_json::to_vec(value).unwrap(),
        )
        .unwrap();
    }

    /// A capture re-signs in place with a stable event id while its artifact
    /// re-hashes under the `object` schema and moves to `artifacts/objects/`, and
    /// the capture's binding is remapped to the re-hashed artifact.
    #[test]
    fn a_capture_re_hashes_its_artifact_with_a_stable_event_id() {
        let repo = TestRepo::new();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let store_dir = repo.store_dir();
        let reference_capture = read_event(&store_dir, "work_object_proposed");
        let reference_event_id = reference_capture["eventId"].as_str().unwrap().to_owned();

        // Downgrade to a legacy store (shore.snapshot artifact + legacy key).
        let legacy = tempfile::tempdir().unwrap();
        let legacy_dir = legacy.path();
        std::fs::create_dir_all(legacy_dir.join("events")).unwrap();
        std::fs::create_dir_all(legacy_dir.join("artifacts/snapshots")).unwrap();

        let object_id = capture.object_id.as_str();
        let object_stem = sha256_bytes_hex(object_id.as_bytes());
        let object_artifact: Value = serde_json::from_slice(
            &std::fs::read(
                store_dir
                    .join("artifacts/objects")
                    .join(format!("{object_stem}.json")),
            )
            .unwrap(),
        )
        .unwrap();
        let mut snapshot_body = object_artifact.clone();
        let object = snapshot_body.as_object_mut().unwrap();
        object.insert("schema".to_owned(), json!("shore.snapshot"));
        object.remove("contentHash");
        let legacy_hash = sha256_json_prefixed(&snapshot_body).unwrap();
        snapshot_body
            .as_object_mut()
            .unwrap()
            .insert("contentHash".to_owned(), json!(legacy_hash));
        std::fs::write(
            legacy_dir
                .join("artifacts/snapshots")
                .join(format!("{object_stem}.json")),
            serde_json::to_vec(&snapshot_body).unwrap(),
        )
        .unwrap();

        let mut capture_event = reference_capture.clone();
        let work_object = capture_event["payload"]["workObject"]
            .as_object_mut()
            .unwrap();
        work_object.remove("objectArtifactContentHash");
        work_object.insert("snapshotArtifactContentHash".to_owned(), json!(legacy_hash));
        write_legacy_event(legacy_dir, &finalize_legacy(capture_event));

        let target = tempfile::tempdir().unwrap();
        let summary = migrate_object_rename(MigrateOptions {
            source_store_dir: legacy_dir.to_path_buf(),
            target_store_dir: target.path().to_path_buf(),
            keystore_dir: legacy_dir.join("keys"),
        })
        .unwrap();
        assert!(summary.self_check_passed);
        assert_eq!(summary.artifacts_rehashed, 1);

        // The artifact moved to artifacts/objects/ under the object schema and the
        // legacy directory is gone.
        assert!(!target.path().join("artifacts/snapshots").exists());
        let migrated_artifact: Value = serde_json::from_slice(
            &std::fs::read(
                target
                    .path()
                    .join("artifacts/objects")
                    .join(format!("{object_stem}.json")),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(migrated_artifact["schema"], "shore.object");
        let new_hash = migrated_artifact["contentHash"].as_str().unwrap();
        assert_ne!(new_hash, legacy_hash, "the artifact re-hashed");

        // The capture re-signs in place: stable event id, binding remapped.
        let migrated_capture = read_event(target.path(), "work_object_proposed");
        assert_eq!(migrated_capture["eventId"], reference_event_id);
        assert_eq!(
            migrated_capture["payload"]["workObject"]["objectArtifactContentHash"],
            json!(new_hash)
        );
        assert!(
            migrated_capture["payload"]["workObject"]
                .get("snapshotArtifactContentHash")
                .is_none()
        );
    }

    #[test]
    fn a_tampered_legacy_artifact_is_rejected() {
        let legacy = tempfile::tempdir().unwrap();
        let legacy_dir = legacy.path();
        std::fs::create_dir_all(legacy_dir.join("events")).unwrap();
        std::fs::create_dir_all(legacy_dir.join("artifacts/snapshots")).unwrap();

        let object_id = "obj:sha256:tampered";
        let stem = sha256_bytes_hex(object_id.as_bytes());
        let body = json!({
            "schema": "shore.snapshot",
            "version": 2,
            "snapshot": { "review_id": "review:default", "object_id": object_id, "files": [] },
            "contentHash": "sha256:not-the-real-hash"
        });
        std::fs::write(
            legacy_dir
                .join("artifacts/snapshots")
                .join(format!("{stem}.json")),
            serde_json::to_vec(&body).unwrap(),
        )
        .unwrap();

        let target = tempfile::tempdir().unwrap();
        let error = migrate_object_rename(MigrateOptions {
            source_store_dir: legacy_dir.to_path_buf(),
            target_store_dir: target.path().to_path_buf(),
            keystore_dir: legacy_dir.join("keys"),
        })
        .unwrap_err();
        assert!(
            error.to_string().contains("content hash mismatch"),
            "got: {error}"
        );
    }

    #[test]
    fn a_foreign_key_cosignature_is_dropped_with_a_warning() {
        let legacy = tempfile::tempdir().unwrap();
        let legacy_dir = legacy.path();
        std::fs::create_dir_all(legacy_dir.join("events")).unwrap();
        std::fs::create_dir_all(legacy_dir.join("keys")).unwrap();

        // A content-id-free structural event for the carrier to target.
        let target_key = "review_observation_recorded:rev:sha256:r:agent:t:obs:sha256:own";
        let observation = json!({
            "schema": "shore.event",
            "version": 1,
            "eventId": derive_event_id(target_key).as_str(),
            "eventType": "review_observation_recorded",
            "idempotencyKey": target_key,
            "target": {
                "journalId": "journal:default",
                "subject": { "review": { "kind": "revision", "revisionId": "rev:sha256:r" } },
                "trackId": "agent:t"
            },
            "writer": { "actorId": "actor:agent:t", "producer": { "name": "shore", "version": "0.1.0" } },
            "occurredAt": "unix-ms:1",
            "payload": {
                "observationId": "obs:sha256:own",
                "target": { "kind": "revision", "revisionId": "rev:sha256:r" },
                "title": "t"
            },
            "payloadHash": "sha256:placeholder"
        });
        let observation = finalize_legacy(observation);
        let target_event_id = observation["eventId"].as_str().unwrap().to_owned();
        // Recompute its real record hash so the carrier targets it faithfully.
        let target_event: ShoreEvent = serde_json::from_value(observation.clone()).unwrap();
        let target_record_hash = target_event.event_record_hash().unwrap();
        write_legacy_event(legacy_dir, &observation);

        let carrier_key =
            format!("event_signature_recorded:{target_record_hash}:did:key:zForeign:SIG");
        let carrier = json!({
            "schema": "shore.event",
            "version": 1,
            "eventId": derive_event_id(&carrier_key).as_str(),
            "eventType": "event_signature_recorded",
            "idempotencyKey": carrier_key,
            "target": { "journalId": "journal:default", "subject": "journal" },
            "writer": { "actorId": "actor:agent:t", "producer": { "name": "shore", "version": "0.1.0" } },
            "occurredAt": "unix-ms:2",
            "payload": {
                "attestation": { "alg": "ed25519", "sigVersion": 1, "sig": "SIG" },
                "attestingSigner": "did:key:zForeign",
                "targetEventId": target_event_id,
                "targetEventRecordHash": target_record_hash
            },
            "payloadHash": "sha256:placeholder"
        });
        let carrier = finalize_legacy(carrier);
        write_legacy_event(legacy_dir, &carrier);

        let target = tempfile::tempdir().unwrap();
        let summary = migrate_object_rename(MigrateOptions {
            source_store_dir: legacy_dir.to_path_buf(),
            target_store_dir: target.path().to_path_buf(),
            keystore_dir: legacy_dir.join("keys"),
        })
        .unwrap();
        assert_eq!(summary.cosignatures_dropped, 1);
        assert_eq!(summary.cosignatures_reattested, 0);
        assert!(summary.self_check_passed);
    }
}
