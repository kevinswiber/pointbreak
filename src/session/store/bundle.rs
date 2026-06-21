use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::error::{Result, ShoreError};
use crate::session::event::{EventType, IngestVia, ShoreEvent, stamp_ingest_provenance};
use crate::session::snapshot_artifact::decode_and_validate_snapshot_artifact;
use crate::session::store::body_artifact::NoteBodyEnvelope;
use crate::session::store::{EventStore, SnapshotArtifact};
use crate::session::{
    EventVerificationPolicy, IngestEventVerification, SessionState, TrustSet, current_timestamp,
    verify_events_for_ingest,
};
use crate::storage::{CreateFileOutcome, Durability, LocalStorage};

const EXPORT_MANIFEST_SCHEMA: &str = "shore.store-export-manifest";
const EXPORT_MANIFEST_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportManifest {
    pub schema: String,
    pub version: u32,
    pub fidelity_status: ExportFidelityStatus,
    pub events: Vec<ExportEvent>,
    pub artifacts: Vec<ExportArtifact>,
    pub diagnostics: Vec<ExportManifestDiagnostic>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExportFidelityStatus {
    Full,
    Incomplete,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportEvent {
    pub event_id: String,
    pub event_type: String,
    pub idempotency_key: String,
    pub payload_hash: String,
    pub event_envelope_hash: String,
    pub event_file_hash: String,
    pub artifact_refs: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportArtifact {
    pub artifact_ref: String,
    pub artifact_kind: ExportArtifactKind,
    pub schema: String,
    pub version: u32,
    pub content_hash: String,
    pub byte_size: u64,
    pub required_by_event_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExportArtifactKind {
    Snapshot,
    NoteBody,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportManifestDiagnostic {
    pub code: String,
    pub artifact_ref: String,
    pub event_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImportBundleResult {
    pub events_created: usize,
    pub events_existing: usize,
    pub artifacts_created: usize,
    pub artifacts_existing: usize,
    pub verification: Vec<IngestEventVerification>,
    pub commit_order: Vec<ImportCommitStep>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ImportCommitStep {
    Artifacts,
    Events,
    State,
}

pub(crate) fn build_export_manifest(store_dir: impl AsRef<Path>) -> Result<ExportManifest> {
    let store_dir = store_dir.as_ref();
    let event_store = EventStore::open(store_dir);
    let mut event_entries = Vec::new();
    let mut artifact_requirements = BTreeMap::<String, ArtifactRequirement>::new();
    let mut diagnostics = Vec::new();

    for path in event_file_paths(store_dir)? {
        let bytes = std::fs::read(&path).map_err(|error| {
            ShoreError::Message(format!("failed to read event for export manifest: {error}"))
        })?;
        let event = event_store.read_event(&path)?;
        let artifact_refs = event_artifact_refs(&event);
        let event_id = event.event_id.as_str().to_owned();
        let event_envelope_hash = sha256_json_prefixed(&serde_json::to_value(&event)?)?;

        for requirement in artifact_refs
            .iter()
            .map(|artifact_ref| ArtifactRequirement::from_ref(artifact_ref, &event_id))
        {
            artifact_requirements
                .entry(requirement.artifact_ref.clone())
                .and_modify(|existing| existing.required_by_event_ids.push(event_id.clone()))
                .or_insert(requirement);
        }

        event_entries.push(ExportEvent {
            event_id,
            event_type: event_type_string(event.event_type)?,
            idempotency_key: event.idempotency_key,
            payload_hash: event.payload_hash,
            event_envelope_hash,
            event_file_hash: format!("sha256:{}", sha256_bytes_hex(&bytes)),
            artifact_refs,
        });
    }

    let mut artifacts = Vec::new();
    for requirement in artifact_requirements.into_values() {
        match read_required_artifact(store_dir, &requirement)? {
            Some(artifact) => artifacts.push(artifact),
            None => diagnostics.push(ExportManifestDiagnostic {
                code: "missing_referenced_artifact".to_owned(),
                artifact_ref: requirement.artifact_ref,
                event_id: requirement
                    .required_by_event_ids
                    .first()
                    .expect("requirement has at least one event id")
                    .clone(),
            }),
        }
    }

    Ok(ExportManifest {
        schema: EXPORT_MANIFEST_SCHEMA.to_owned(),
        version: EXPORT_MANIFEST_VERSION,
        fidelity_status: if diagnostics.is_empty() {
            ExportFidelityStatus::Full
        } else {
            ExportFidelityStatus::Incomplete
        },
        events: event_entries,
        artifacts,
        diagnostics,
    })
}

pub(crate) fn import_store_bundle(
    source_store_dir: impl AsRef<Path>,
    target_store_dir: impl AsRef<Path>,
) -> Result<ImportBundleResult> {
    import_store_bundle_with_verification(
        source_store_dir,
        target_store_dir,
        EventVerificationPolicy::advisory(),
        TrustSet::default(),
    )
}

pub(crate) fn import_store_bundle_with_verification(
    source_store_dir: impl AsRef<Path>,
    target_store_dir: impl AsRef<Path>,
    verification_policy: EventVerificationPolicy,
    trust_set: TrustSet,
) -> Result<ImportBundleResult> {
    let source_store_dir = source_store_dir.as_ref();
    let target_store_dir = target_store_dir.as_ref();
    let manifest = build_export_manifest(source_store_dir)?;
    if manifest.fidelity_status != ExportFidelityStatus::Full {
        return Err(ShoreError::Message(
            "strict import requires a full-fidelity export manifest".to_owned(),
        ));
    }

    let events = read_source_events(source_store_dir)?;
    let source_events = events
        .iter()
        .map(|source| source.event.clone())
        .collect::<Vec<_>>();
    let verification = verify_events_for_ingest(&source_events, verification_policy, &trust_set)?;
    let artifacts = read_source_artifacts(source_store_dir, &manifest)?;
    let target_event_store = EventStore::open(target_store_dir);
    preflight_event_conflicts(&target_event_store, &events)?;

    let (artifacts_created, artifacts_existing) = commit_artifacts(target_store_dir, &artifacts)?;
    let (events_created, events_existing) = commit_events(&target_event_store, &events)?;
    rebuild_target_state(target_store_dir, &target_event_store)?;

    Ok(ImportBundleResult {
        events_created,
        events_existing,
        artifacts_created,
        artifacts_existing,
        verification,
        commit_order: vec![
            ImportCommitStep::Artifacts,
            ImportCommitStep::Events,
            ImportCommitStep::State,
        ],
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SourceEvent {
    event: ShoreEvent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SourceArtifact {
    relative_path: PathBuf,
    bytes: Vec<u8>,
}

fn read_source_events(source_store_dir: &Path) -> Result<Vec<SourceEvent>> {
    let event_store = EventStore::open(source_store_dir);
    event_file_paths(source_store_dir)?
        .into_iter()
        .map(|path| {
            Ok(SourceEvent {
                event: event_store.read_event(&path)?,
            })
        })
        .collect()
}

fn read_source_artifacts(
    source_store_dir: &Path,
    manifest: &ExportManifest,
) -> Result<Vec<SourceArtifact>> {
    manifest
        .artifacts
        .iter()
        .map(|artifact| match artifact.artifact_kind {
            ExportArtifactKind::Snapshot => {
                read_source_snapshot_artifact(source_store_dir, artifact)
            }
            ExportArtifactKind::NoteBody => {
                read_source_note_body_artifact(source_store_dir, artifact)
            }
        })
        .collect()
}

fn read_source_snapshot_artifact(
    source_store_dir: &Path,
    artifact: &ExportArtifact,
) -> Result<SourceArtifact> {
    let snapshot_id = artifact
        .artifact_ref
        .strip_prefix("snapshot:")
        .ok_or_else(|| {
            ShoreError::Message(format!(
                "invalid snapshot artifact ref {}",
                artifact.artifact_ref
            ))
        })?;
    let Some((path, parsed)) = find_snapshot_artifact(source_store_dir, snapshot_id)? else {
        return Err(ShoreError::Message(format!(
            "missing snapshot artifact {}",
            artifact.artifact_ref
        )));
    };
    if parsed.content_hash != artifact.content_hash {
        return Err(ShoreError::Message(format!(
            "snapshot artifact content hash mismatch for {}",
            artifact.artifact_ref
        )));
    }

    Ok(SourceArtifact {
        relative_path: snapshot_relative_path(snapshot_id),
        bytes: std::fs::read(&path).map_err(|error| {
            ShoreError::Message(format!("failed to read source snapshot artifact: {error}"))
        })?,
    })
}

fn read_source_note_body_artifact(
    source_store_dir: &Path,
    artifact: &ExportArtifact,
) -> Result<SourceArtifact> {
    let relative_path = note_body_relative_path(&artifact.artifact_ref);
    validate_relative_note_body_path(&relative_path)?;
    let path = source_store_dir.join(&relative_path);
    let bytes = std::fs::read(&path).map_err(|error| {
        ShoreError::Message(format!("failed to read source note body artifact: {error}"))
    })?;
    let parsed: NoteBodyEnvelope = serde_json::from_slice(&bytes)?;
    let content_hash = format!("sha256:{}", sha256_bytes_hex(parsed.body.as_bytes()));
    if parsed.schema != artifact.schema
        || parsed.version != artifact.version
        || content_hash != artifact.content_hash
    {
        return Err(ShoreError::Message(format!(
            "note body artifact content mismatch for {}",
            artifact.artifact_ref
        )));
    }

    Ok(SourceArtifact {
        relative_path,
        bytes,
    })
}

fn preflight_event_conflicts(target_store: &EventStore, events: &[SourceEvent]) -> Result<()> {
    for source in events {
        let path = target_store.event_path_for_idempotency_key(&source.event.idempotency_key);
        if !path.exists() {
            continue;
        }

        let existing = target_store.read_event(&path)?;
        if existing.payload_hash != source.event.payload_hash {
            return Err(ShoreError::Message(format!(
                "event conflict for idempotency key {}",
                source.event.idempotency_key
            )));
        }
    }

    Ok(())
}

fn commit_artifacts(
    target_store_dir: &Path,
    artifacts: &[SourceArtifact],
) -> Result<(usize, usize)> {
    let storage = LocalStorage::new(target_store_dir);
    let mut created = 0;
    let mut existing = 0;

    for artifact in artifacts {
        match storage.create_file_exclusive(
            &artifact.relative_path,
            &artifact.bytes,
            Durability::Durable,
        )? {
            CreateFileOutcome::Created => created += 1,
            CreateFileOutcome::AlreadyExists => {
                let existing_bytes = storage.read_bytes(&artifact.relative_path)?;
                if existing_bytes != artifact.bytes {
                    return Err(ShoreError::Message(format!(
                        "artifact conflict for {}",
                        artifact.relative_path.display()
                    )));
                }
                existing += 1;
            }
        }
    }

    Ok((created, existing))
}

fn commit_events(target_store: &EventStore, events: &[SourceEvent]) -> Result<(usize, usize)> {
    let source_events: Vec<ShoreEvent> = events.iter().map(|source| source.event.clone()).collect();
    let stamped =
        stamp_ingest_provenance(&source_events, IngestVia::BundleApply, &current_timestamp());
    let mut created = 0;
    let mut existing = 0;

    for event in &stamped {
        match target_store.record_event_once(event)? {
            crate::session::EventWriteOutcome::Created => created += 1,
            crate::session::EventWriteOutcome::Existing
            | crate::session::EventWriteOutcome::ExistingDivergentSignature => existing += 1,
        }
    }

    Ok((created, existing))
}

fn rebuild_target_state(target_store_dir: &Path, target_store: &EventStore) -> Result<()> {
    let events = target_store.list_events()?;
    let state = SessionState::from_events(&events)?;
    LocalStorage::new(target_store_dir).write_json_atomic(
        Path::new("state.json"),
        &state,
        Durability::Projection,
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ArtifactRequirement {
    artifact_ref: String,
    kind: ExportArtifactKind,
    locator: ArtifactLocator,
    content_hash: Option<String>,
    required_by_event_ids: Vec<String>,
}

impl ArtifactRequirement {
    fn from_ref(artifact_ref: &str, event_id: &str) -> Self {
        if let Some(snapshot_id) = artifact_ref.strip_prefix("snapshot:") {
            return Self {
                artifact_ref: artifact_ref.to_owned(),
                kind: ExportArtifactKind::Snapshot,
                locator: ArtifactLocator::Snapshot {
                    snapshot_id: snapshot_id.to_owned(),
                },
                content_hash: None,
                required_by_event_ids: vec![event_id.to_owned()],
            };
        }

        Self {
            artifact_ref: artifact_ref.to_owned(),
            kind: ExportArtifactKind::NoteBody,
            locator: ArtifactLocator::NoteBody {
                relative_path: note_body_relative_path(artifact_ref),
            },
            content_hash: artifact_ref
                .strip_prefix("note-body:")
                .map(std::borrow::ToOwned::to_owned),
            required_by_event_ids: vec![event_id.to_owned()],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ArtifactLocator {
    Snapshot { snapshot_id: String },
    NoteBody { relative_path: PathBuf },
}

fn event_file_paths(store_dir: &Path) -> Result<Vec<PathBuf>> {
    let events_dir = store_dir.join("events");
    let entries = match std::fs::read_dir(&events_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(ShoreError::Message(format!(
                "failed to list events for export manifest: {error}"
            )));
        }
    };

    let mut paths = entries
        .map(|entry| {
            entry.map(|entry| entry.path()).map_err(|error| {
                ShoreError::Message(format!("failed to read event entry: {error}"))
            })
        })
        .collect::<Result<Vec<_>>>()?;
    paths.retain(|path| is_event_file(path));
    paths.sort();
    Ok(paths)
}

fn is_event_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.len() == 69 && name.ends_with(".json"))
}

fn event_artifact_refs(event: &ShoreEvent) -> Vec<String> {
    let mut refs = BTreeSet::new();
    // A generative move binds its content-object snapshot through the proposed
    // work object: the object id lives on the revision the proposal wraps.
    if event.event_type == EventType::WorkObjectProposed
        && let Some(object_id) = event
            .payload
            .get("workObject")
            .and_then(|work_object| work_object.get("revision"))
            .and_then(|revision| revision.get("objectId"))
            .and_then(|value| value.as_str())
    {
        refs.insert(format!("snapshot:{object_id}"));
    }

    for path in note_body_artifact_paths_for_event(event.event_type, &event.payload) {
        if let Some(stem) = note_body_hash_from_path(path) {
            refs.insert(format!("note-body:sha256:{stem}"));
        }
    }
    refs.into_iter().collect()
}

fn note_body_artifact_paths_for_event(
    event_type: EventType,
    payload: &serde_json::Value,
) -> Vec<&str> {
    match event_type {
        EventType::ReviewObservationRecorded
        | EventType::InputRequestOpened
        | EventType::ReviewNoteImported
        | EventType::TaskObservationRecorded => optional_payload_path(payload, "bodyArtifactPath"),
        EventType::InputRequestResponded => optional_payload_path(payload, "reasonArtifactPath"),
        EventType::ReviewAssessmentRecorded | EventType::ValidationCheckRecorded => {
            optional_payload_path(payload, "summaryArtifactPath")
        }
        _ => Vec::new(),
    }
}

fn optional_payload_path<'a>(payload: &'a serde_json::Value, field: &str) -> Vec<&'a str> {
    payload
        .get(field)
        .and_then(|value| value.as_str())
        .into_iter()
        .collect()
}

fn note_body_hash_from_path(path: &str) -> Option<&str> {
    path.strip_prefix("artifacts/notes/")
        .and_then(|path| path.strip_suffix(".json"))
        .filter(|stem| stem.len() == 64 && stem.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn note_body_relative_path(artifact_ref: &str) -> PathBuf {
    let stem = artifact_ref
        .strip_prefix("note-body:sha256:")
        .unwrap_or_default();
    PathBuf::from("artifacts")
        .join("notes")
        .join(format!("{stem}.json"))
}

fn read_required_artifact(
    store_dir: &Path,
    requirement: &ArtifactRequirement,
) -> Result<Option<ExportArtifact>> {
    match &requirement.locator {
        ArtifactLocator::Snapshot { snapshot_id } => {
            read_snapshot_export_artifact(store_dir, snapshot_id, requirement)
        }
        ArtifactLocator::NoteBody { relative_path } => {
            read_note_body_export_artifact(store_dir, relative_path, requirement)
        }
    }
}

fn read_snapshot_export_artifact(
    store_dir: &Path,
    snapshot_id: &str,
    requirement: &ArtifactRequirement,
) -> Result<Option<ExportArtifact>> {
    let Some((path, artifact)) = find_snapshot_artifact(store_dir, snapshot_id)? else {
        return Ok(None);
    };
    let byte_size = file_size(&path)?;

    Ok(Some(ExportArtifact {
        artifact_ref: requirement.artifact_ref.clone(),
        artifact_kind: requirement.kind,
        schema: artifact.schema,
        version: artifact.version,
        content_hash: artifact.content_hash,
        byte_size,
        required_by_event_ids: requirement.required_by_event_ids.clone(),
    }))
}

fn snapshot_relative_path(snapshot_id: &str) -> PathBuf {
    PathBuf::from("artifacts")
        .join("snapshots")
        .join(format!("{}.json", sha256_bytes_hex(snapshot_id.as_bytes())))
}

fn find_snapshot_artifact(
    store_dir: &Path,
    snapshot_id: &str,
) -> Result<Option<(PathBuf, SnapshotArtifact)>> {
    let artifact_dir = store_dir.join("artifacts/snapshots");
    let entries = match std::fs::read_dir(&artifact_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(ShoreError::Message(format!(
                "failed to list snapshot artifacts for export manifest: {error}"
            )));
        }
    };

    for entry in entries {
        let path = entry
            .map_err(|error| {
                ShoreError::Message(format!("failed to read snapshot artifact entry: {error}"))
            })?
            .path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }

        let bytes = std::fs::read(&path).map_err(|error| {
            ShoreError::Message(format!("failed to read snapshot artifact: {error}"))
        })?;
        // Cheap snapshot-id match first (so an unrelated corrupt artifact in the
        // dir does not abort the scan), then validate the match through the shared
        // dual-read decode path (v1 + v2).
        let parsed: SnapshotArtifact = serde_json::from_slice(&bytes)?;
        if parsed.snapshot.snapshot_id.as_str() == snapshot_id {
            let artifact = decode_and_validate_snapshot_artifact(&bytes)?;
            return Ok(Some((path, artifact)));
        }
    }

    Ok(None)
}

fn read_note_body_export_artifact(
    store_dir: &Path,
    relative_path: &Path,
    requirement: &ArtifactRequirement,
) -> Result<Option<ExportArtifact>> {
    validate_relative_note_body_path(relative_path)?;
    let path = store_dir.join(relative_path);
    if !path.exists() {
        return Ok(None);
    }

    let bytes = std::fs::read(&path).map_err(|error| {
        ShoreError::Message(format!("failed to read note body artifact: {error}"))
    })?;
    let artifact: NoteBodyEnvelope = serde_json::from_slice(&bytes)?;
    if artifact.schema != "shore.note-body" || artifact.version != 1 {
        return Err(ShoreError::Message(format!(
            "unsupported note body artifact schema/version: {} v{}",
            artifact.schema, artifact.version
        )));
    }
    let content_hash = format!("sha256:{}", sha256_bytes_hex(artifact.body.as_bytes()));
    if requirement.content_hash.as_deref() != Some(content_hash.as_str()) {
        return Err(ShoreError::Message(format!(
            "note body artifact content hash mismatch for {}",
            requirement.artifact_ref
        )));
    }

    Ok(Some(ExportArtifact {
        artifact_ref: requirement.artifact_ref.clone(),
        artifact_kind: requirement.kind,
        schema: artifact.schema,
        version: artifact.version,
        content_hash,
        byte_size: bytes.len() as u64,
        required_by_event_ids: requirement.required_by_event_ids.clone(),
    }))
}

fn validate_relative_note_body_path(path: &Path) -> Result<()> {
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) || !path.starts_with("artifacts/notes")
    {
        return Err(ShoreError::Message(format!(
            "invalid note body artifact locator: {}",
            path.display()
        )));
    }

    Ok(())
}

fn file_size(path: &Path) -> Result<u64> {
    Ok(std::fs::metadata(path)
        .map_err(|error| ShoreError::Message(format!("failed to stat export artifact: {error}")))?
        .len())
}

fn event_type_string(event_type: EventType) -> Result<String> {
    Ok(serde_json::to_value(event_type)?
        .as_str()
        .expect("event type serializes as string")
        .to_owned())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use serde_json::json;

    use super::*;
    use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
    use crate::crypto::{EventVerificationStatus, SignerId};
    use crate::model::{
        EventId, LedgerId, RevisionId, TrackId, ValidationCheckId, ValidationStatus,
        ValidationTarget, ValidationTrigger,
    };
    use crate::session::body_artifact::BODY_INLINE_LIMIT;
    use crate::session::event::{
        AssertionMode, EventSignature, EventTarget, EventType, IngestProvenance, IngestVia,
        ShoreEvent, ValidationCheckRecordedPayload, Writer,
    };
    use crate::session::{
        CaptureOptions, EventStore, EventVerificationPolicy, ObservationAddOptions, TrustSet,
        capture_worktree_review, event_signature_trust_set, record_observation,
        verify_event_signature,
    };

    #[test]
    fn export_manifest_includes_events_and_snapshot_artifacts() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let store = EventStore::open(resolved_store_dir(repo.path()));
        let capture_event = store
            .list_events()
            .unwrap()
            .into_iter()
            .find(|event| event.event_type == EventType::WorkObjectProposed)
            .expect("capture event");

        let manifest = build_export_manifest(resolved_store_dir(repo.path())).unwrap();

        assert_eq!(manifest.schema, "shore.store-export-manifest");
        assert_eq!(manifest.version, 1);
        assert_eq!(manifest.fidelity_status, ExportFidelityStatus::Full);
        // A worktree capture records the capture event plus the auto-recorded
        // capture-time ref association.
        assert_eq!(manifest.events.len(), 2);
        assert!(manifest.diagnostics.is_empty());

        let event = manifest
            .events
            .iter()
            .find(|event| event.event_type == "work_object_proposed")
            .expect("capture event in manifest");
        assert_eq!(event.event_id, capture_event.event_id.as_str());
        assert_eq!(event.event_type, "work_object_proposed");
        assert_eq!(event.idempotency_key, capture_event.idempotency_key);
        assert_eq!(event.payload_hash, capture_event.payload_hash);
        assert!(event.event_envelope_hash.starts_with("sha256:"));
        assert!(event.event_file_hash.starts_with("sha256:"));
        assert_ne!(event.event_envelope_hash, event.event_file_hash);

        let snapshot_ref = format!("snapshot:{}", capture.object_id.as_str());
        assert_eq!(event.artifact_refs, vec![snapshot_ref.clone()]);
        let artifact = manifest
            .artifacts
            .iter()
            .find(|artifact| artifact.artifact_ref == snapshot_ref)
            .expect("snapshot artifact");
        assert_eq!(artifact.artifact_kind, ExportArtifactKind::Snapshot);
        assert_eq!(artifact.schema, "shore.snapshot");
        assert_eq!(artifact.version, 2);
        assert_eq!(
            artifact.content_hash,
            capture.snapshot_artifact_content_hash
        );
        assert!(artifact.byte_size > 0);
        assert_eq!(
            artifact.required_by_event_ids,
            vec![capture_event.event_id.as_str().to_owned()]
        );

        let json = serde_json::to_string(&manifest).unwrap();
        assert!(!json.contains(".shore/data"));
        assert!(!json.contains("artifacts/snapshots"));
        assert!(!json.contains("events/"));
    }

    #[test]
    fn export_manifest_includes_referenced_note_body_artifacts() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let body = "x".repeat(BODY_INLINE_LIMIT + 1);
        let observation = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Large body")
                .with_body(body),
        )
        .unwrap();

        let manifest = build_export_manifest(resolved_store_dir(repo.path())).unwrap();

        assert_eq!(manifest.fidelity_status, ExportFidelityStatus::Full);
        let body_hash = observation.body_content_hash.expect("body hash");
        let artifact_ref = format!("note-body:{body_hash}");
        let event = manifest
            .events
            .iter()
            .find(|event| event.event_id == observation.event_id.as_str())
            .expect("observation event");
        assert_eq!(event.artifact_refs, vec![artifact_ref.clone()]);

        let artifact = manifest
            .artifacts
            .iter()
            .find(|artifact| artifact.artifact_ref == artifact_ref)
            .expect("note body artifact");
        assert_eq!(artifact.artifact_kind, ExportArtifactKind::NoteBody);
        assert_eq!(artifact.schema, "shore.note-body");
        assert_eq!(artifact.version, 1);
        assert_eq!(artifact.content_hash, body_hash);
        assert!(artifact.byte_size > BODY_INLINE_LIMIT as u64);
        assert_eq!(
            artifact.required_by_event_ids,
            vec![observation.event_id.as_str().to_owned()]
        );

        let json = serde_json::to_string(&manifest).unwrap();
        assert!(!json.contains("artifacts/notes"));
        assert!(!json.contains("Large body"));
    }

    #[test]
    fn store_bundle_import_preserves_externalized_validation_summary_artifacts() {
        let source = tempfile::tempdir().unwrap();
        let source_store_dir = source.path().join(".shore/data");
        let summary = "validation summary\n".repeat(BODY_INLINE_LIMIT);
        let (summary_artifact_path, summary_content_hash, summary_byte_size) =
            write_note_body_artifact(&source_store_dir, summary.clone());
        let validation_event = validation_event_with_summary_artifact(
            &summary_artifact_path,
            &summary_content_hash,
            summary_byte_size,
        );
        let validation_event_id = validation_event.event_id.as_str().to_owned();
        write_event_to_store(&source_store_dir, validation_event);

        let manifest = build_export_manifest(&source_store_dir).unwrap();

        assert_eq!(manifest.fidelity_status, ExportFidelityStatus::Full);
        let artifact_ref = format!("note-body:{summary_content_hash}");
        let event = manifest
            .events
            .iter()
            .find(|event| event.event_id == validation_event_id)
            .expect("validation event");
        assert_eq!(event.artifact_refs, vec![artifact_ref.clone()]);
        assert!(
            manifest
                .artifacts
                .iter()
                .any(|artifact| artifact.artifact_ref == artifact_ref)
        );

        let target = tempfile::tempdir().unwrap();
        let target_store_dir = target.path().join(".shore/data");
        import_store_bundle(&source_store_dir, &target_store_dir).unwrap();

        let imported_bytes = fs::read(target_store_dir.join(&summary_artifact_path)).unwrap();
        let imported: NoteBodyEnvelope = serde_json::from_slice(&imported_bytes).unwrap();
        assert_eq!(imported.body, summary);
    }

    #[test]
    fn artifact_refs_ignore_unenumerated_artifact_path_payload_fields() {
        let path = note_body_path_for_hash("0".repeat(64));
        let mut event = review_initialized_event("schema-enumeration", 1);
        event.payload = json!({
            "unexpectedArtifactPath": path,
            "nested": {
                "anotherArtifactPath": note_body_path_for_hash("1".repeat(64))
            }
        });

        assert_eq!(event_artifact_refs(&event), Vec::<String>::new());
    }

    #[test]
    fn artifact_refs_collect_enumerated_note_body_payload_fields() {
        let path = note_body_path_for_hash("2".repeat(64));
        let mut event = review_initialized_event("known-note-body-field", 1);
        event.event_type = EventType::ReviewObservationRecorded;
        event.payload = json!({
            "bodyArtifactPath": path,
        });

        assert_eq!(
            event_artifact_refs(&event),
            vec![format!("note-body:sha256:{}", "2".repeat(64))]
        );
    }

    #[test]
    fn export_manifest_marks_missing_artifacts_as_not_full_fidelity() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        remove_snapshot_artifacts(repo.path());

        let manifest = build_export_manifest(resolved_store_dir(repo.path())).unwrap();

        assert_eq!(manifest.fidelity_status, ExportFidelityStatus::Incomplete);
        assert_eq!(manifest.artifacts.len(), 0);
        assert_eq!(manifest.diagnostics.len(), 1);
        assert_eq!(manifest.diagnostics[0].code, "missing_referenced_artifact");
        assert!(
            manifest.events.iter().any(|event| event
                .artifact_refs
                .iter()
                .any(|artifact_ref| artifact_ref.starts_with("snapshot:"))),
            "the capture event references the snapshot artifact"
        );
    }

    #[test]
    fn strict_import_treats_same_event_payload_as_noop() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let target = tempfile::tempdir().unwrap();
        let target_store_dir = target.path().join(".shore/data");

        let first =
            import_store_bundle(resolved_store_dir(repo.path()), &target_store_dir).unwrap();
        let second =
            import_store_bundle(resolved_store_dir(repo.path()), &target_store_dir).unwrap();

        // The capture event plus the auto-recorded ref association.
        assert_eq!(first.events_created, 2);
        assert_eq!(first.events_existing, 0);
        assert_eq!(first.artifacts_created, 1);
        assert_eq!(second.events_created, 0);
        assert_eq!(second.events_existing, 2);
        assert_eq!(second.artifacts_created, 0);
        assert_eq!(second.artifacts_existing, 1);
    }

    #[test]
    fn bundle_import_uses_verification_policy_before_event_commit() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let source_store_dir = source.path().join(".shore/data");
        let target_store_dir = target.path().join(".shore/data");
        write_event_to_store(
            &source_store_dir,
            invalid_signed_review_initialized_event("signed-invalid"),
        );

        let advisory = import_store_bundle_with_verification(
            &source_store_dir,
            &target_store_dir,
            EventVerificationPolicy::advisory(),
            TrustSet::default(),
        )
        .unwrap();

        assert_eq!(advisory.events_created, 1);
        assert_eq!(
            advisory.verification[0].status,
            EventVerificationStatus::Invalid
        );

        let strict_target = tempfile::tempdir().unwrap();
        let strict_target_store_dir = strict_target.path().join(".shore/data");
        let error = import_store_bundle_with_verification(
            &source_store_dir,
            &strict_target_store_dir,
            EventVerificationPolicy::integrity_strict(),
            TrustSet::default(),
        )
        .expect_err("integrity-strict rejects invalid signature");

        assert!(
            error.to_string().contains("invalid"),
            "unexpected error: {error}"
        );
        assert!(
            !strict_target_store_dir.join("events").exists()
                || EventStore::open(&strict_target_store_dir)
                    .list_events()
                    .unwrap()
                    .is_empty()
        );
    }

    #[test]
    fn strict_import_rejects_same_idempotency_key_with_different_payload() {
        let source_one = tempfile::tempdir().unwrap();
        let source_two = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let idempotency_key = "review_initialized:session:default:work:default";
        write_event_to_store(
            &source_one.path().join(".shore/data"),
            review_initialized_event(idempotency_key, 1),
        );
        write_event_to_store(
            &source_two.path().join(".shore/data"),
            review_initialized_event(idempotency_key, 2),
        );
        import_store_bundle(
            source_one.path().join(".shore/data"),
            target.path().join(".shore/data"),
        )
        .unwrap();

        let error = import_store_bundle(
            source_two.path().join(".shore/data"),
            target.path().join(".shore/data"),
        )
        .expect_err("conflicting event is rejected");

        assert!(error.to_string().contains("event conflict"));
        let stored = EventStore::open(target.path().join(".shore/data"))
            .list_events()
            .unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].payload["value"], json!(1));
    }

    #[test]
    fn strict_import_rejects_events_when_referenced_artifact_is_missing() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        remove_snapshot_artifacts(repo.path());
        let target = tempfile::tempdir().unwrap();

        let error = import_store_bundle(
            resolved_store_dir(repo.path()),
            target.path().join(".shore/data"),
        )
        .expect_err("incomplete bundle is rejected");

        assert!(error.to_string().contains("full-fidelity"));
    }

    #[test]
    fn strict_import_commits_artifacts_before_events_and_rebuilds_state() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        fs::write(
            resolved_store_dir(repo.path()).join("state.json"),
            r#"{"sourceState":"must not be imported as authority"}"#,
        )
        .unwrap();
        let target = tempfile::tempdir().unwrap();
        let target_store_dir = target.path().join(".shore/data");

        let result =
            import_store_bundle(resolved_store_dir(repo.path()), &target_store_dir).unwrap();

        assert_eq!(
            result.commit_order,
            vec![
                ImportCommitStep::Artifacts,
                ImportCommitStep::Events,
                ImportCommitStep::State,
            ]
        );
        assert!(target_store_dir.join("artifacts/snapshots").is_dir());
        assert!(target_store_dir.join("events").is_dir());
        let rebuilt_state = fs::read_to_string(target_store_dir.join("state.json")).unwrap();
        assert!(!rebuilt_state.contains("must not be imported"));
        assert!(rebuilt_state.contains("ledgerId"));
    }

    #[test]
    fn bundle_apply_stamps_bundle_apply_provenance_on_target_events() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let target = tempfile::tempdir().unwrap();
        let target_store_dir = target.path().join(".shore/data");

        import_store_bundle(resolved_store_dir(repo.path()), &target_store_dir).unwrap();

        let stored = EventStore::open(&target_store_dir).list_events().unwrap();
        assert!(!stored.is_empty());
        for event in &stored {
            let stamp = event
                .ingest
                .as_ref()
                .expect("every bundle-applied event is stamped");
            assert_eq!(stamp.via, IngestVia::BundleApply);
            assert!(stamp.received_at.starts_with("unix-ms:"));
        }
        // The source store is never modified.
        let source = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        assert!(source.iter().all(|event| event.ingest.is_none()));
    }

    #[test]
    fn bundle_apply_overwrites_inbound_ingest_stamp() {
        // The source store's copy carries its own importer's stamp; the target
        // re-stamps with via: bundle-apply — foreign bookkeeping is not a fact.
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let mut event = review_initialized_event("inbound-stamped", 1);
        event.ingest = Some(IngestProvenance {
            via: IngestVia::IngestEvents,
            received_at: "unix-ms:1".to_owned(),
        });
        write_event_to_store(&source.path().join(".shore/data"), event);

        import_store_bundle(
            source.path().join(".shore/data"),
            target.path().join(".shore/data"),
        )
        .unwrap();

        let stored = EventStore::open(target.path().join(".shore/data"))
            .list_events()
            .unwrap();
        let stamp = stored[0].ingest.as_ref().unwrap();
        assert_eq!(stamp.via, IngestVia::BundleApply);
        assert_ne!(stamp.received_at, "unix-ms:1");
    }

    #[test]
    fn bundle_apply_reimport_keeps_first_stamp_and_reports_existing() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let target = tempfile::tempdir().unwrap();
        let target_store_dir = target.path().join(".shore/data");

        import_store_bundle(resolved_store_dir(repo.path()), &target_store_dir).unwrap();
        let first_stamps: Vec<_> = EventStore::open(&target_store_dir)
            .list_events()
            .unwrap()
            .into_iter()
            .map(|event| event.ingest)
            .collect();
        assert!(first_stamps.iter().all(Option::is_some));

        let second =
            import_store_bundle(resolved_store_dir(repo.path()), &target_store_dir).unwrap();
        assert_eq!(second.events_created, 0);
        assert!(second.events_existing > 0);

        let second_stamps: Vec<_> = EventStore::open(&target_store_dir)
            .list_events()
            .unwrap()
            .into_iter()
            .map(|event| event.ingest)
            .collect();
        assert_eq!(second_stamps, first_stamps);
    }

    #[test]
    fn bundle_apply_preflight_and_signature_semantics_unaffected_by_stamps() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let signed: ShoreEvent = serde_json::from_str(include_str!(
            "../../../tests/fixtures/event_signatures/friendly-valid-event.json"
        ))
        .unwrap();
        write_event_to_store(&source.path().join(".shore/data"), signed);
        let trust = event_signature_trust_set(
            serde_json::from_str(include_str!(
                "../../../tests/fixtures/event_signatures/did-key-ed25519.json"
            ))
            .unwrap(),
        )
        .unwrap();

        let result = import_store_bundle_with_verification(
            source.path().join(".shore/data"),
            target.path().join(".shore/data"),
            EventVerificationPolicy::advisory(),
            trust.clone(),
        )
        .unwrap();
        assert_eq!(result.events_created, 1);
        assert_eq!(
            result.verification[0].status,
            EventVerificationStatus::Valid
        );

        // The stamped target copy still verifies valid.
        let stored = EventStore::open(target.path().join(".shore/data"))
            .list_events()
            .unwrap();
        assert!(stored[0].ingest.is_some());
        assert_eq!(
            verify_event_signature(&stored[0], &trust).unwrap(),
            EventVerificationStatus::Valid
        );

        // Preflight does not treat the stamp difference (target stamped,
        // source unstamped) as a conflict: payload_hash comparison only.
        let again = import_store_bundle_with_verification(
            source.path().join(".shore/data"),
            target.path().join(".shore/data"),
            EventVerificationPolicy::advisory(),
            trust,
        )
        .unwrap();
        assert_eq!(again.events_created, 0);
        assert_eq!(again.events_existing, 1);
    }

    fn modified_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        repo
    }

    /// The store a capture/workflow actually lands in for `repo` — the shared
    /// common-dir store by default. A repo used as an export/import bundle source
    /// is read from here, not the raw worktree-local `.shore/data`.
    fn resolved_store_dir(repo: &std::path::Path) -> std::path::PathBuf {
        crate::git::git_common_dir(repo).unwrap().join("shore")
    }

    fn review_initialized_event(idempotency_key: &str, value: u32) -> ShoreEvent {
        let payload = json!({ "value": value });
        ShoreEvent {
            schema: "shore.event".to_owned(),
            version: 1,
            event_id: EventId::new(format!(
                "evt:sha256:{}",
                sha256_bytes_hex(idempotency_key.as_bytes())
            )),
            event_type: EventType::ReviewInitialized,
            idempotency_key: idempotency_key.to_owned(),
            target: EventTarget::for_ledger(LedgerId::new("ledger:default")),
            writer: Writer::shore_local("test"),
            occurred_at: "2026-05-30T00:00:00Z".to_owned(),
            payload_hash: sha256_json_prefixed(&payload).unwrap(),
            assertion_mode: AssertionMode::Advisory,
            signer: None,
            signature: None,
            source_ref: None,
            ingest: None,
            payload,
        }
    }

    fn invalid_signed_review_initialized_event(idempotency_key: &str) -> ShoreEvent {
        let mut event = review_initialized_event(idempotency_key, 1);
        event.signer = Some(
            SignerId::parse("did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd").unwrap(),
        );
        event.signature = Some(
            EventSignature::new_ed25519_v1(
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==",
            )
            .unwrap(),
        );
        event
    }

    fn validation_event_with_summary_artifact(
        summary_artifact_path: &str,
        summary_content_hash: &str,
        summary_byte_size: u64,
    ) -> ShoreEvent {
        let review_unit_id = RevisionId::new("review-unit:sha256:bundle");
        let track_id = TrackId::new("agent:codex");
        let mut target = EventTarget::for_revision(
            LedgerId::new("session:default"),
            review_unit_id.clone(),
            None,
        );
        target.track_id = Some(track_id.clone());

        ShoreEvent::new(
            EventType::ValidationCheckRecorded,
            ValidationCheckRecordedPayload::idempotency_key(
                &review_unit_id,
                &track_id,
                "validation:sha256:bundle",
            ),
            target,
            Writer::shore_local("test"),
            ValidationCheckRecordedPayload {
                validation_check_id: ValidationCheckId::new("validation:sha256:bundle"),
                target: ValidationTarget::Revision {
                    revision_id: review_unit_id,
                },
                check_name: "cargo nextest run".to_owned(),
                command: None,
                status: ValidationStatus::Passed,
                exit_code: Some(0),
                trigger: ValidationTrigger::Manual,
                source_fingerprint: None,
                summary: None,
                summary_artifact_path: Some(summary_artifact_path.to_owned()),
                summary_byte_size: Some(summary_byte_size),
                summary_content_hash: Some(summary_content_hash.to_owned()),
                started_at: None,
                completed_at: None,
                log_artifact_content_hashes: Vec::new(),
            },
            "2026-05-30T00:00:00Z",
        )
        .unwrap()
    }

    fn write_event_to_store(store_dir: &Path, event: ShoreEvent) {
        EventStore::open(store_dir)
            .record_event_once(&event)
            .expect("write test event");
    }

    fn remove_snapshot_artifacts(repo: &Path) {
        let artifact_dir = resolved_store_dir(repo).join("artifacts/snapshots");
        for entry in fs::read_dir(artifact_dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension() == Some(OsStr::new("json")) {
                fs::remove_file(path).unwrap();
            }
        }
    }

    fn note_body_path_for_hash(hash: String) -> String {
        format!("artifacts/notes/{hash}.json")
    }

    fn write_note_body_artifact(store_dir: &Path, body: String) -> (String, String, u64) {
        let byte_size = body.len() as u64;
        let content_hash = format!("sha256:{}", sha256_bytes_hex(body.as_bytes()));
        let artifact_path = note_body_path_for_hash(
            content_hash
                .strip_prefix("sha256:")
                .expect("sha256 content hash")
                .to_owned(),
        );
        let bytes = NoteBodyEnvelope::new(body).to_json_bytes().unwrap();
        LocalStorage::new(store_dir)
            .write_bytes_atomic(Path::new(&artifact_path), &bytes, Durability::Durable)
            .expect("write note body artifact");
        (artifact_path, content_hash, byte_size)
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

        fn write(&self, path: impl AsRef<Path>, contents: impl AsRef<[u8]>) {
            let path = self.root.path().join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directories");
            }
            fs::write(path, contents).expect("write test repository file");
        }

        fn commit_all(&self, message: &str) {
            self.git(["add", "--all"]);
            self.git(["commit", "-m", message]);
        }

        fn git<I, S>(&self, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<OsStr>,
        {
            let args = args
                .into_iter()
                .map(|arg| arg.as_ref().to_owned())
                .collect::<Vec<_>>();
            let output = Command::new("git")
                .args(&args)
                .current_dir(self.root.path())
                .output()
                .unwrap_or_else(|error| panic!("run git {:?}: {error}", args));

            assert!(
                output.status.success(),
                "git {:?} failed\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
                args,
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
