//! One-shot store migrator: re-keys an already-reshaped store from the
//! `Ledger`-shaped wire into the `Journal`-shaped wire in a single pass.
//!
//! This is throwaway, run-once tooling, not a shipped command. The stores it
//! migrates were already lifted into the reshaped envelope by a prior migration,
//! so the per-event transform here is narrow: rename the container wire key
//! (`ledgerId` -> `journalId`), migrate the container-id value prefix
//! (`session:`/`ledger:` -> `journal:`), rename the carrier subject tag
//! (`"ledger"` -> `"journal"`), and rename the stored snapshot-artifact body field
//! (`snapshot_id` -> `object_id`, value unchanged `obj:`). The container value
//! feeds journal-scoped idempotency keys, so migrating it re-keys those events;
//! the artifact-field rename re-hashes artifacts and remaps each referencing
//! capture's `snapshotArtifactContentHash`. Every event's record hash moves (the
//! serialized target changed), so inline signatures are re-signed and detached
//! co-signatures re-homed with held keys. There is no `sigVersion` bump.
//!
//! It reads each event as raw JSON (bypassing the strict read path), rewrites the
//! wire tokens, re-derives every dependent id, and writes the result into a fresh
//! store the strict read path accepts. Already-`journal`-shaped events and
//! artifacts (a store written partly by the post-rename binary) pass through
//! verbatim. A detached co-signature whose attester key is not held cannot be
//! re-attested and is dropped with a warning. `docs/store-migration.md` is the
//! durable architecture record.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use super::EventStore;
use super::snapshot_artifact::{
    build_snapshot_artifact_v2, decode_and_validate_snapshot_artifact, snapshot_artifact_path,
};
use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::crypto::EventSigner;
use crate::error::{Result, ShoreError};
use crate::keys::{FileEd25519Signer, KeyCustody, list_keys_in, load_signer_in};
use crate::model::{DiffSnapshot, EventId};
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
    /// Events copied through verbatim because they were already `journal`-shaped.
    pub events_passed_through: usize,
    /// Snapshot artifacts re-hashed under the renamed `object_id` field.
    pub artifacts_rehashed: usize,
    /// Inline signatures re-signed with the original signer's held key.
    pub inline_signatures_resigned: usize,
    /// Detached co-signatures re-attested with the attester's held key.
    pub cosignatures_reattested: usize,
    /// Detached co-signatures dropped because the attester's key is not held (or
    /// the target did not survive), counted and warned, never silent.
    pub cosignatures_dropped: usize,
    /// Whether the re-keyed store passed its self-check (`list_events` rebuilds
    /// cleanly under the strict read path and `SessionState::from_events`
    /// succeeds).
    pub self_check_passed: bool,
}

/// Migrate the store at `source_store_dir` into a fresh `journal`-shaped store at
/// `target_store_dir`, re-signing with keys from `keystore_dir`.
pub fn migrate_journal_rename(options: MigrateOptions) -> Result<MigrateSummary> {
    let raw = read_raw_events(&options.source_store_dir)?;
    let keystore = build_keystore_index(&options.keystore_dir)?;
    let mut summary = MigrateSummary::default();

    // Pass 0: re-emit snapshot artifacts under the renamed object field and build
    // the old-content-hash -> new-content-hash remap that each referencing capture
    // is rewritten against.
    let artifact_remap = migrate_artifacts(&options, &mut summary)?;

    // Pass 1: re-emit every non-co-signature event, building the old-event-id ->
    // new-event map the co-signature re-home reads.
    let target = EventStore::open(&options.target_store_dir);
    let mut old_to_new: BTreeMap<String, ShoreEvent> = BTreeMap::new();
    for value in &raw {
        if value["eventType"] == "event_signature_recorded" {
            // Every co-signature re-homes in the third pass, once the full
            // old->new event-id map is built.
            continue;
        }

        let old_event_id = event_id_of(value)?;
        if !event_needs_migration(value, &artifact_remap) {
            // An already-`journal`-shaped event passes through verbatim.
            let event = passthrough_event(value)?;
            record_into(&target, &event)?;
            old_to_new.insert(old_event_id, event);
            summary.events_passed_through += 1;
            summary.events_migrated += 1;
            continue;
        }

        let event = transform_event(value, &artifact_remap)?;
        let event = resign_if_signed(event, value, &keystore, &options, &mut summary)?;
        record_into(&target, &event)?;
        old_to_new.insert(old_event_id, event);
        summary.events_migrated += 1;
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

    // Self-check: the re-keyed store must list cleanly under the strict read path
    // and rebuild its projection.
    let events = target.list_events()?;
    let _state = crate::session::SessionState::from_events(&events)?;
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

/// Migrate every snapshot artifact: rename the body's `snapshot_id` field to
/// `object_id` (value unchanged `obj:`), recompute the content hash, and write it
/// back under the same `obj:`-keyed path. Returns the old-content-hash ->
/// new-content-hash remap the referencing captures are rewritten against. An
/// already-`object_id`-shaped artifact passes through verbatim; a content-removed
/// (absent) artifact is simply not present and its referencing capture keeps the
/// old binding hash.
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

    let target_dir = options.target_store_dir.join("artifacts/snapshots");
    std::fs::create_dir_all(&target_dir)
        .map_err(|error| migrate_error(&format!("create {}: {error}", target_dir.display())))?;
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

        if artifact_is_journal_shaped(&value) {
            // Already migrated: validate before laundering it through as a trusted
            // clean artifact (recipe point 7a applies to the passthrough branch too).
            // The strict decoder re-checks the version and the content hash, so a
            // tampered already-`object_id` artifact in a mixed store is rejected
            // rather than copied verbatim — the self-check only rebuilds events, so
            // it would not otherwise catch a bad copied artifact.
            decode_and_validate_snapshot_artifact(&bytes)?;
            std::fs::write(target_dir.join(entry.file_name()), &bytes).map_err(|error| {
                migrate_error(&format!(
                    "write {}: {error}",
                    entry.file_name().to_string_lossy()
                ))
            })?;
            continue;
        }

        let old_content_hash = string_field(&value, "contentHash")?;
        validate_legacy_artifact_integrity(&value, &old_content_hash)?;

        let mut snapshot_value = value["snapshot"].clone();
        if let Some(object) = snapshot_value.as_object_mut()
            && let Some(id) = object.remove("snapshot_id")
        {
            object.insert("object_id".to_owned(), id);
        }
        let snapshot: DiffSnapshot = serde_json::from_value(snapshot_value)?;
        let artifact = build_snapshot_artifact_v2(snapshot)?;

        let target_path =
            snapshot_artifact_path(&options.target_store_dir, &artifact.snapshot.snapshot_id);
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

/// An artifact body is already `journal`-shaped iff its snapshot carries the
/// renamed `object_id` field and not the legacy `snapshot_id`.
fn artifact_is_journal_shaped(value: &Value) -> bool {
    value["snapshot"].get("object_id").is_some() && value["snapshot"].get("snapshot_id").is_none()
}

/// Validate the legacy artifact's stored content hash over its own body, so a
/// tampered artifact is rejected rather than laundered into the clean format.
fn validate_legacy_artifact_integrity(value: &Value, content_hash: &str) -> Result<()> {
    let mut material = value.clone();
    let Some(object) = material.as_object_mut() else {
        return Err(migrate_error("snapshot artifact must be an object"));
    };
    object.remove("contentHash");
    let expected = sha256_json_prefixed(&material)?;
    if expected != content_hash {
        return Err(migrate_error(&format!(
            "snapshot artifact content hash mismatch (stored {content_hash}, recomputed {expected})"
        )));
    }
    Ok(())
}

/// Whether an event still carries a stale `ledger`/`session` wire token in a wire
/// position: a `ledgerId`/`sessionId` container key, a `session:`/`ledger:`
/// container value, a `"ledger"` carrier subject tag, or a capture reference to a
/// re-keyed artifact. Never matches a free-text body that merely mentions the
/// term.
fn event_needs_migration(value: &Value, artifact_remap: &BTreeMap<String, String>) -> bool {
    let target = &value["target"];
    if target.get("ledgerId").is_some() || target.get("sessionId").is_some() {
        return true;
    }
    if let Some(container) = target.get("journalId").and_then(Value::as_str)
        && is_stale_container_value(container)
    {
        return true;
    }
    if target.get("subject").and_then(Value::as_str) == Some("ledger") {
        return true;
    }
    capture_artifact_hash(value).is_some_and(|hash| artifact_remap.contains_key(hash))
}

/// Re-emit a stale event under the `journal`-shaped wire: rewrite the target,
/// migrate the container value in the idempotency key, remap a capture's artifact
/// binding hash, then re-derive every dependent id. Signatures are reproduced by
/// the caller.
fn transform_event(value: &Value, artifact_remap: &BTreeMap<String, String>) -> Result<ShoreEvent> {
    let mut migrated = value.clone();
    let old_container = container_value_of(value).map(str::to_owned);

    if let Some(target) = migrated["target"].as_object_mut() {
        rewrite_target(target);
    }

    // Re-key: substitute the old container value in the idempotency key. A
    // subject-addressed event whose key does not embed the container value is
    // untouched here and keeps its event id (only its record hash moves).
    if let Some(old) = &old_container {
        let new = migrate_container_value(old);
        if &new != old
            && let Some(key) = migrated["idempotencyKey"].as_str()
            && key.contains(old.as_str())
        {
            migrated["idempotencyKey"] = Value::String(key.replace(old.as_str(), &new));
        }
    }

    // Remap a capture's artifact binding hash to the re-hashed artifact.
    if let Some(work_object) = migrated["payload"]
        .get_mut("workObject")
        .and_then(Value::as_object_mut)
    {
        let remapped = work_object
            .get("snapshotArtifactContentHash")
            .and_then(Value::as_str)
            .and_then(|hash| artifact_remap.get(hash))
            .cloned();
        if let Some(new_hash) = remapped {
            work_object.insert(
                "snapshotArtifactContentHash".to_owned(),
                Value::String(new_hash),
            );
        }
    }

    // Clear the stale signature; the caller re-signs over the reshaped record.
    migrated["signer"] = Value::Null;
    migrated["signature"] = Value::Null;

    let mut event: ShoreEvent = serde_json::from_value(migrated)?;
    // Re-derive the dependent ids with the same functions a native write uses, so
    // the strict write path accepts them: the payload hash over the (possibly
    // remapped) payload, the event id over the (possibly re-keyed) idempotency key.
    event.payload_hash = sha256_json_prefixed(&event.payload)?;
    event.event_id = EventId::new(format!(
        "evt:sha256:{}",
        sha256_bytes_hex(event.idempotency_key.as_bytes())
    ));
    Ok(event)
}

/// Rewrite a target object's container key/value and carrier subject tag in place.
fn rewrite_target(target: &mut serde_json::Map<String, Value>) {
    let container = target
        .remove("journalId")
        .or_else(|| target.remove("ledgerId"))
        .or_else(|| target.remove("sessionId"));
    if let Some(Value::String(value)) = container {
        target.insert(
            "journalId".to_owned(),
            Value::String(migrate_container_value(&value)),
        );
    }
    if target.get("subject").and_then(Value::as_str) == Some("ledger") {
        target.insert("subject".to_owned(), Value::String("journal".to_owned()));
    }
}

/// The container id value carried by an event's target, under whichever key.
fn container_value_of(value: &Value) -> Option<&str> {
    let target = &value["target"];
    target["journalId"]
        .as_str()
        .or_else(|| target["ledgerId"].as_str())
        .or_else(|| target["sessionId"].as_str())
}

/// Migrate a container id's leading segment `session:`/`ledger:` -> `journal:`,
/// touching only the leading container segment, never a `sourceRef` source id or
/// the `claude:`/`default` body.
fn migrate_container_value(value: &str) -> String {
    if let Some(rest) = value.strip_prefix("ledger:") {
        format!("journal:{rest}")
    } else if let Some(rest) = value.strip_prefix("session:") {
        format!("journal:{rest}")
    } else {
        value.to_owned()
    }
}

fn is_stale_container_value(value: &str) -> bool {
    value.starts_with("session:") || value.starts_with("ledger:")
}

/// A capture event's artifact binding hash, if it carries one.
fn capture_artifact_hash(value: &Value) -> Option<&str> {
    value["payload"]["workObject"]["snapshotArtifactContentHash"].as_str()
}

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
    // serialized target changed) is re-attested or dropped.
    let new_target_record_hash = new_target.event_record_hash()?;
    if !event_needs_migration(value, &BTreeMap::new())
        && new_target.event_id.as_str() == old_target_event_id
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
    ShoreError::Message(format!("journal migrate: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{KeyName, generate_key_in};

    struct Fixture {
        _root: tempfile::TempDir,
        source: PathBuf,
        target: PathBuf,
        keystore: PathBuf,
        signer_did: String,
        attester_did: String,
    }

    fn fixture() -> Fixture {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let target = root.path().join("target");
        let keystore = root.path().join("keys");
        std::fs::create_dir_all(source.join("events")).unwrap();
        std::fs::create_dir_all(source.join("artifacts/snapshots")).unwrap();
        std::fs::create_dir_all(&keystore).unwrap();

        let signer = generate_key_in(&keystore, &KeyName::parse("agent-claude-code").unwrap())
            .unwrap()
            .signer_id()
            .as_str()
            .to_owned();
        let attester = generate_key_in(&keystore, &KeyName::parse("reviewer").unwrap())
            .unwrap()
            .signer_id()
            .as_str()
            .to_owned();

        Fixture {
            _root: root,
            source,
            target,
            keystore,
            signer_did: signer,
            attester_did: attester,
        }
    }

    impl Fixture {
        fn options(&self) -> MigrateOptions {
            MigrateOptions {
                source_store_dir: self.source.clone(),
                target_store_dir: self.target.clone(),
                keystore_dir: self.keystore.clone(),
            }
        }

        fn write_event(&self, value: &Value) {
            let key = value["idempotencyKey"].as_str().unwrap();
            let stem = sha256_bytes_hex(key.as_bytes());
            std::fs::write(
                self.source.join("events").join(format!("{stem}.json")),
                serde_json::to_vec(value).unwrap(),
            )
            .unwrap();
        }

        /// Write a `ledger`-shaped snapshot artifact (body field `snapshot_id`,
        /// value `obj:`), returning its content hash for capture binding.
        fn write_snapshot_artifact(&self, object_id: &str) -> String {
            let mut body = serde_json::json!({
                "schema": "shore.snapshot",
                "version": 2,
                "snapshot": { "review_id": "review:default", "snapshot_id": object_id, "files": [] },
            });
            let hash = sha256_json_prefixed(&body).unwrap();
            body.as_object_mut()
                .unwrap()
                .insert("contentHash".to_owned(), Value::String(hash.clone()));
            let stem = sha256_bytes_hex(object_id.as_bytes());
            std::fs::write(
                self.source
                    .join("artifacts/snapshots")
                    .join(format!("{stem}.json")),
                serde_json::to_vec(&body).unwrap(),
            )
            .unwrap();
            hash
        }
    }

    /// A `ledger`-shaped `review_initialized` event whose idempotency key embeds
    /// the container value, so migrating the value re-keys it. As in a real store,
    /// the event id is derived from the idempotency key.
    fn ledger_review_initialized(container: &str, signer_did: Option<&str>) -> Value {
        let key = format!("review_initialized:{container}");
        let mut event = serde_json::json!({
            "schema": "shore.event",
            "version": 1,
            "eventId": event_id_for(&key),
            "eventType": "review_initialized",
            "idempotencyKey": key,
            "target": { "ledgerId": container, "subject": "ledger" },
            "writer": { "actorId": "actor:agent:claude-code", "producer": { "name": "shore", "version": "0.1.0" } },
            "occurredAt": "unix-ms:1781808954000",
            "payloadHash": "sha256:legacy",
            "payload": {}
        });
        if let Some(did) = signer_did {
            event["signer"] = Value::String(did.to_owned());
            event["signature"] =
                serde_json::json!({ "alg": "ed25519", "sigVersion": 1, "sig": "AAAA" });
        }
        event
    }

    /// A `ledger`-shaped `work_object_proposed` capture whose key is the revision
    /// id (does not embed the container), binding a snapshot artifact by hash.
    fn ledger_capture(
        revision_id: &str,
        object_id: &str,
        container: &str,
        artifact_hash: &str,
    ) -> Value {
        serde_json::json!({
            "schema": "shore.event",
            "version": 1,
            "eventId": event_id_for(&format!("work_object_proposed:{revision_id}")),
            "eventType": "work_object_proposed",
            "idempotencyKey": format!("work_object_proposed:{revision_id}"),
            "target": {
                "ledgerId": container,
                "subject": { "review": { "kind": "revision", "revisionId": revision_id } }
            },
            "writer": { "actorId": "actor:agent:claude-code", "producer": { "name": "shore", "version": "0.1.0" } },
            "occurredAt": "unix-ms:1781808954225",
            "payloadHash": "sha256:legacy",
            "payload": {
                "engagementId": "engagement:sha256:e",
                "workObject": {
                    "kind": "revision",
                    "revision": { "id": revision_id, "objectId": object_id },
                    "snapshotArtifactContentHash": artifact_hash
                }
            }
        })
    }

    /// A `ledger`-shaped detached co-signature over a target event id + record
    /// hash.
    fn ledger_cosignature(
        attester_did: &str,
        target_event_id: &str,
        target_record_hash: &str,
    ) -> Value {
        let key = format!("event_signature_recorded:{target_record_hash}:{attester_did}:ZZZZ");
        serde_json::json!({
            "schema": "shore.event",
            "version": 1,
            "eventId": event_id_for(&key),
            "eventType": "event_signature_recorded",
            "idempotencyKey": key,
            "target": { "ledgerId": "ledger:default" },
            "writer": { "actorId": "actor:git-email:reviewer@example.com", "producer": { "name": "shore", "version": "0.1.0" } },
            "occurredAt": "unix-ms:1781821504936",
            "payload": {
                "attestation": { "alg": "ed25519", "sigVersion": 1, "sig": "ZZZZ" },
                "attestingSigner": attester_did,
                "targetEventId": target_event_id,
                "targetEventRecordHash": target_record_hash
            },
            "payloadHash": "sha256:legacy"
        })
    }

    /// The legacy event id derived from an idempotency key (the migrator re-derives
    /// the same way), used to point a co-signature at its target.
    fn event_id_for(idempotency_key: &str) -> String {
        format!(
            "evt:sha256:{}",
            sha256_bytes_hex(idempotency_key.as_bytes())
        )
    }

    /// The legacy record hash of a written event, read back from the source store
    /// after re-deriving it through the reshaping transform's inverse is not
    /// possible; instead a co-signature in these fixtures targets an event whose
    /// record hash the migrator recomputes — so the carrier always re-attests. We
    /// only need a stable placeholder string for the legacy `targetEventRecordHash`.
    const LEGACY_RECORD_HASH: &str = "sha256:legacyrecord";

    #[test]
    fn migrates_both_container_value_prefixes_only() {
        // ledger:default -> journal:default AND session:claude:uuid ->
        // journal:claude:uuid; a sourceRef-shaped id is not a container and is
        // untouched by the value migration.
        assert_eq!(migrate_container_value("ledger:default"), "journal:default");
        assert_eq!(
            migrate_container_value("session:claude:uuid-1"),
            "journal:claude:uuid-1"
        );
        assert_eq!(
            migrate_container_value("session:abc/tool_result:1"),
            "journal:abc/tool_result:1"
        );
        // The container-value migration only runs over `JournalId::new` container
        // ids (via `container_value_of`), never over a `sourceRef` source id, so a
        // `session:`-prefixed sourceRef in a payload is never reached.
        assert_eq!(
            migrate_container_value("journal:default"),
            "journal:default"
        );
    }

    #[test]
    fn migrates_a_signed_journal_store_losslessly() {
        let fx = fixture();
        let object_id = "obj:sha256:aaa";
        let artifact_hash = fx.write_snapshot_artifact(object_id);
        fx.write_event(&ledger_review_initialized(
            "ledger:default",
            Some(&fx.signer_did),
        ));
        let capture = ledger_capture("rev:sha256:r", object_id, "ledger:default", &artifact_hash);
        fx.write_event(&capture);
        let init_event_id = event_id_for("review_initialized:ledger:default");
        fx.write_event(&ledger_cosignature(
            &fx.attester_did,
            &init_event_id,
            LEGACY_RECORD_HASH,
        ));

        let summary = migrate_journal_rename(fx.options()).unwrap();

        // 2 events (review_initialized + capture) + 1 re-attested co-signature.
        assert_eq!(summary.events_migrated, 3);
        assert_eq!(summary.cosignatures_reattested, 1);
        assert_eq!(summary.cosignatures_dropped, 0);
        assert_eq!(summary.inline_signatures_resigned, 1);
        assert_eq!(summary.artifacts_rehashed, 1);
        assert!(summary.self_check_passed);

        // No stale wire token survives.
        assert_no_stale_wire(&fx.target);
    }

    #[test]
    fn rekeys_journal_scoped_events_and_remaps_cosignatures() {
        let fx = fixture();
        fx.write_event(&ledger_review_initialized("ledger:default", None));
        let init_event_id = event_id_for("review_initialized:ledger:default");
        fx.write_event(&ledger_cosignature(
            &fx.attester_did,
            &init_event_id,
            LEGACY_RECORD_HASH,
        ));

        migrate_journal_rename(fx.options()).unwrap();

        // The journal-scoped event re-keyed: its file now lives at the new key's
        // stem, and the old stem is gone.
        let new_stem = sha256_bytes_hex(b"review_initialized:journal:default");
        let old_stem = sha256_bytes_hex(b"review_initialized:ledger:default");
        assert!(
            fx.target
                .join("events")
                .join(format!("{new_stem}.json"))
                .exists(),
            "re-keyed review_initialized must exist at the journal stem"
        );
        assert!(
            !fx.target
                .join("events")
                .join(format!("{old_stem}.json"))
                .exists(),
            "the ledger-keyed stem must not survive"
        );
        assert_no_stale_wire(&fx.target);
    }

    #[test]
    fn migrates_a_session_prefixed_claude_container() {
        let fx = fixture();
        fx.write_event(&ledger_review_initialized("session:claude:uuid-1", None));

        migrate_journal_rename(fx.options()).unwrap();

        let new_stem = sha256_bytes_hex(b"review_initialized:journal:claude:uuid-1");
        assert!(
            fx.target
                .join("events")
                .join(format!("{new_stem}.json"))
                .exists()
        );
        assert_no_stale_wire(&fx.target);
    }

    #[test]
    fn renames_artifact_object_field_and_remaps_referencing_capture() {
        let fx = fixture();
        let object_id = "obj:sha256:bbb";
        let old_hash = fx.write_snapshot_artifact(object_id);
        fx.write_event(&ledger_capture(
            "rev:sha256:r",
            object_id,
            "ledger:default",
            &old_hash,
        ));

        migrate_journal_rename(fx.options()).unwrap();

        // The migrated artifact body serializes object_id (value unchanged obj:).
        let stem = sha256_bytes_hex(object_id.as_bytes());
        let artifact: Value = serde_json::from_slice(
            &std::fs::read(
                fx.target
                    .join("artifacts/snapshots")
                    .join(format!("{stem}.json")),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(artifact["snapshot"].get("object_id").is_some());
        assert!(artifact["snapshot"].get("snapshot_id").is_none());
        let new_hash = artifact["contentHash"].as_str().unwrap();
        assert_ne!(new_hash, old_hash, "the artifact content hash changed");

        // The capture event now binds the new artifact hash.
        let capture = read_only_event(&fx.target, "work_object_proposed");
        assert_eq!(
            capture["payload"]["workObject"]["snapshotArtifactContentHash"],
            *new_hash
        );
    }

    #[test]
    fn already_journal_shaped_events_pass_through() {
        let fx = fixture();
        // One stale (ledger) event and one already-journal event.
        fx.write_event(&ledger_review_initialized("ledger:default", None));
        let mut clean = ledger_review_initialized("journal:other", None);
        // Make it genuinely journal-shaped: journalId key + journal: value, with
        // the event id derived from its (unchanged) idempotency key.
        clean["target"] = serde_json::json!({ "journalId": "journal:other", "subject": "journal" });
        clean["idempotencyKey"] = Value::String("review_initialized:journal:other".to_owned());
        clean["eventId"] = Value::String(event_id_for("review_initialized:journal:other"));
        // A real journal-shaped event carries a real payload hash (the passthrough
        // path writes it verbatim through the validating strict write).
        clean["payloadHash"] = Value::String(sha256_json_prefixed(&clean["payload"]).unwrap());
        fx.write_event(&clean);

        let summary = migrate_journal_rename(fx.options()).unwrap();

        assert_eq!(summary.events_migrated, 2);
        assert_eq!(summary.events_passed_through, 1);
        assert!(summary.self_check_passed);
        assert_no_stale_wire(&fx.target);
    }

    #[test]
    fn foreign_key_cosignature_is_dropped_with_a_warning() {
        let fx = fixture();
        fx.write_event(&ledger_review_initialized("ledger:default", None));
        let init_event_id = event_id_for("review_initialized:ledger:default");
        fx.write_event(&ledger_cosignature(
            "did:key:zForeignUnheldAttester",
            &init_event_id,
            LEGACY_RECORD_HASH,
        ));

        let summary = migrate_journal_rename(fx.options()).unwrap();

        assert_eq!(summary.cosignatures_dropped, 1);
        assert_eq!(summary.cosignatures_reattested, 0);
        assert!(summary.self_check_passed);
    }

    #[test]
    fn rejects_a_tampered_artifact() {
        let fx = fixture();
        // Write an artifact, then corrupt its body without re-stamping the hash.
        let stem = sha256_bytes_hex(b"obj:sha256:ccc");
        let body = serde_json::json!({
            "schema": "shore.snapshot",
            "version": 2,
            "snapshot": { "review_id": "review:default", "snapshot_id": "obj:sha256:ccc", "files": [] },
            "contentHash": "sha256:not-the-real-hash"
        });
        std::fs::write(
            fx.source
                .join("artifacts/snapshots")
                .join(format!("{stem}.json")),
            serde_json::to_vec(&body).unwrap(),
        )
        .unwrap();

        let error = migrate_journal_rename(fx.options()).unwrap_err();
        assert!(
            error.to_string().contains("content hash mismatch"),
            "got: {error}"
        );
    }

    #[test]
    fn passthrough_rejects_a_tampered_journal_artifact() {
        // An already-`object_id` artifact in a mixed store is NOT laundered through
        // unchecked: a corrupted body with a stale contentHash is rejected by the
        // strict decoder before it can land in the target store.
        let fx = fixture();
        let stem = sha256_bytes_hex(b"obj:sha256:ddd");
        let body = serde_json::json!({
            "schema": "shore.snapshot",
            "version": 2,
            "snapshot": { "review_id": "review:default", "object_id": "obj:sha256:ddd", "files": [] },
            "contentHash": "sha256:stale-not-the-real-hash"
        });
        std::fs::write(
            fx.source
                .join("artifacts/snapshots")
                .join(format!("{stem}.json")),
            serde_json::to_vec(&body).unwrap(),
        )
        .unwrap();

        let error = migrate_journal_rename(fx.options()).unwrap_err();
        assert!(error.to_string().contains("content hash"), "got: {error}");
    }

    #[test]
    fn migration_leaves_an_event_source_ref_untouched() {
        // The container-value migration is callsite-scoped to the target container
        // id; a session:-prefixed EXTERNAL source id on the event's sourceRef is not
        // a container and must survive migration byte-for-byte.
        let fx = fixture();
        let mut capture = ledger_capture(
            "rev:sha256:r",
            "obj:sha256:eee",
            "ledger:default",
            "sha256:none",
        );
        capture["sourceRef"] = serde_json::json!({
            "sourceSystem": "claude_code",
            "sourceId": "session:abc/tool_result:1"
        });
        fx.write_event(&capture);

        migrate_journal_rename(fx.options()).unwrap();

        let migrated = read_only_event(&fx.target, "work_object_proposed");
        assert_eq!(
            migrated["sourceRef"]["sourceId"], "session:abc/tool_result:1",
            "an external sourceRef id must never be migrated as a container value"
        );
        // The container, by contrast, did migrate.
        assert_eq!(migrated["target"]["journalId"], "journal:default");
    }

    fn read_only_event(target: &Path, event_type: &str) -> Value {
        let dir = target.join("events");
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            if value["eventType"] == event_type {
                return value;
            }
        }
        panic!("no {event_type} event in {}", dir.display());
    }

    /// Assert no migrated event carries a stale `ledgerId` key, a `session:`/
    /// `ledger:` container value, a `"ledger"` carrier tag, or a `snapshot_id`
    /// artifact field.
    fn assert_no_stale_wire(target: &Path) {
        for entry in std::fs::read_dir(target.join("events")).unwrap() {
            let value: Value =
                serde_json::from_slice(&std::fs::read(entry.unwrap().path()).unwrap()).unwrap();
            assert!(value["target"].get("ledgerId").is_none(), "stale ledgerId");
            assert!(
                value["target"].get("sessionId").is_none(),
                "stale sessionId"
            );
            if let Some(container) = container_value_of(&value) {
                assert!(
                    !is_stale_container_value(container),
                    "stale container value: {container}"
                );
            }
            assert_ne!(
                value["target"].get("subject").and_then(Value::as_str),
                Some("ledger"),
                "stale ledger carrier tag"
            );
        }
        let snapshots = target.join("artifacts/snapshots");
        if snapshots.is_dir() {
            for entry in std::fs::read_dir(&snapshots).unwrap() {
                let value: Value =
                    serde_json::from_slice(&std::fs::read(entry.unwrap().path()).unwrap()).unwrap();
                assert!(
                    value["snapshot"].get("snapshot_id").is_none(),
                    "stale snapshot_id artifact field"
                );
            }
        }
    }
}
