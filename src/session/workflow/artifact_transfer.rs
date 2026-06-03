use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::error::{Result, ShoreError};
use crate::model::SnapshotId;
use crate::session::body_artifact::{
    note_body_content_hash_from_path, validate_note_body_artifact_bytes,
};
use crate::session::event::{
    EventType, InputRequestRespondedPayload, ReviewAssessmentRecordedPayload,
    ReviewNoteImportedPayload, ReviewObservationRecordedPayload, ReviewUnitCapturedPayload,
    ShoreEvent, TaskObservationRecordedPayload, decode_input_request_opened_payload,
};
use crate::session::snapshot_artifact::{
    read_snapshot_artifact, read_snapshot_artifact_bytes, snapshot_artifact_path,
    validate_snapshot_artifact_content_hash,
};
use crate::session::store::SnapshotArtifact;
use crate::session::store_init::{ShoreStorePaths, prepare_shore_writer};
use crate::storage::{CreateFileOutcome, Durability, LocalStorage};

/// The kind of content-addressed artifact an event references.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactKind {
    /// A captured ReviewUnit snapshot artifact.
    Snapshot,
    /// A large note-shaped body artifact.
    Body,
}

/// An opaque reference to a content-addressed artifact required by one or more
/// events.
///
/// The stable surface exposes the artifact kind and content hash. Any locator
/// needed to read or write Shoreline's current on-disk layout stays private and
/// must be passed back to [`export_artifact`] / [`import_artifact`].
#[derive(Clone, Eq, PartialEq)]
pub struct ArtifactRef {
    locator: ArtifactLocator,
    content_hash: String,
}

impl ArtifactRef {
    /// The artifact's broad kind.
    pub fn kind(&self) -> ArtifactKind {
        match self.locator {
            ArtifactLocator::Snapshot { .. } => ArtifactKind::Snapshot,
            ArtifactLocator::Body { .. } => ArtifactKind::Body,
        }
    }

    /// The artifact's expected content hash, normalized as `sha256:<hex>`.
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }
}

impl fmt::Debug for ArtifactRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArtifactRef")
            .field("kind", &self.kind())
            .field("content_hash", &self.content_hash)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
enum ArtifactLocator {
    Snapshot { snapshot_id: SnapshotId },
    Body { relative_path: String },
}

/// Options for importing a content-addressed artifact into a repo's `.shore`
/// store.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportArtifactOptions {
    repo: PathBuf,
    artifact: ArtifactRef,
    bytes: Vec<u8>,
}

impl ImportArtifactOptions {
    /// Create artifact-import options from a destination repo, the expected
    /// artifact reference, and the bytes fetched from a source store.
    pub fn new(repo: impl AsRef<Path>, artifact: ArtifactRef, bytes: Vec<u8>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            artifact,
            bytes,
        }
    }
}

/// Whether an artifact import created a new blob or found the identical blob
/// already present.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportArtifactOutcome {
    /// The artifact bytes were written to the destination store.
    Created,
    /// The destination store already contained the identical artifact.
    Existing,
}

/// The result of importing one artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportArtifactResult {
    /// The artifact reference that was imported.
    pub artifact: ArtifactRef,
    /// Whether the import created a new artifact or found an existing one.
    pub outcome: ImportArtifactOutcome,
}

/// Enumerate the artifacts referenced by a set of events.
///
/// The returned references are deduplicated and deterministic. Body artifact
/// hashes are derived from `artifacts/notes/<hex>.json` locators and normalized
/// to `sha256:<hex>` so callers do not need to understand the filename/hash
/// prefix difference.
pub fn referenced_artifacts(events: &[ShoreEvent]) -> Result<Vec<ArtifactRef>> {
    let mut refs = BTreeMap::<String, ArtifactRef>::new();
    for event in events {
        referenced_artifacts_for_event(event, &mut refs)?;
    }
    Ok(refs.into_values().collect())
}

/// Export an artifact's validated bytes from a source repo.
pub fn export_artifact(repo: impl AsRef<Path>, artifact: &ArtifactRef) -> Result<Vec<u8>> {
    match &artifact.locator {
        ArtifactLocator::Snapshot { snapshot_id } => {
            let stored = read_snapshot_artifact(repo.as_ref(), snapshot_id)?;
            if stored.content_hash != artifact.content_hash {
                return Err(ShoreError::Message(format!(
                    "snapshot artifact content hash mismatch for {}",
                    artifact.content_hash
                )));
            }
            read_snapshot_artifact_bytes(repo, snapshot_id)
        }
        ArtifactLocator::Body { relative_path } => {
            let paths = ShoreStorePaths::resolve(repo.as_ref())?;
            read_body_artifact_bytes(paths.shore_dir(), relative_path, &artifact.content_hash)
        }
    }
}

/// Import an artifact into a destination repo after validating its content
/// hash.
///
/// The write is idempotent: importing the same valid artifact again returns
/// [`ImportArtifactOutcome::Existing`]. A conflicting existing artifact or
/// bytes that do not match the reference hash are rejected.
pub fn import_artifact(options: ImportArtifactOptions) -> Result<ImportArtifactResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let storage = LocalStorage::new(paths.shore_dir());
    prepare_shore_writer(&paths, &storage)?;

    let outcome = match &options.artifact.locator {
        ArtifactLocator::Snapshot { snapshot_id } => import_snapshot_artifact(
            paths.shore_dir(),
            &storage,
            snapshot_id,
            &options.artifact.content_hash,
            &options.bytes,
        )?,
        ArtifactLocator::Body { relative_path } => import_body_artifact(
            &storage,
            relative_path,
            &options.artifact.content_hash,
            &options.bytes,
        )?,
    };

    Ok(ImportArtifactResult {
        artifact: options.artifact,
        outcome,
    })
}

fn referenced_artifacts_for_event(
    event: &ShoreEvent,
    refs: &mut BTreeMap<String, ArtifactRef>,
) -> Result<()> {
    match event.event_type {
        EventType::ReviewUnitCaptured => {
            let payload: ReviewUnitCapturedPayload = serde_json::from_value(event.payload.clone())?;
            insert_artifact_ref(
                refs,
                format!("snapshot:{}", payload.snapshot_id.as_str()),
                ArtifactRef {
                    locator: ArtifactLocator::Snapshot {
                        snapshot_id: payload.snapshot_id,
                    },
                    content_hash: payload.snapshot_artifact_content_hash,
                },
            )
        }
        EventType::InputRequestOpened => {
            let payload = decode_input_request_opened_payload(event.payload.clone())?;
            insert_body_ref(refs, payload.body_artifact_path.as_deref())
        }
        EventType::InputRequestResponded => {
            let payload: InputRequestRespondedPayload =
                serde_json::from_value(event.payload.clone())?;
            insert_body_ref(refs, payload.reason_artifact_path.as_deref())
        }
        EventType::ReviewObservationRecorded => {
            let payload: ReviewObservationRecordedPayload =
                serde_json::from_value(event.payload.clone())?;
            insert_body_ref(refs, payload.body_artifact_path.as_deref())
        }
        EventType::ReviewAssessmentRecorded => {
            let payload: ReviewAssessmentRecordedPayload =
                serde_json::from_value(event.payload.clone())?;
            insert_body_ref(refs, payload.summary_artifact_path.as_deref())
        }
        EventType::ReviewNoteImported => {
            let payload: ReviewNoteImportedPayload = serde_json::from_value(event.payload.clone())?;
            insert_body_ref(refs, payload.body_artifact_path.as_deref())
        }
        EventType::TaskObservationRecorded => {
            let payload: TaskObservationRecordedPayload =
                serde_json::from_value(event.payload.clone())?;
            insert_body_ref(refs, payload.body_artifact_path.as_deref())
        }
        EventType::ReviewInitialized
        | EventType::TaskAttemptCaptured
        | EventType::TaskCheckpointCaptured => Ok(()),
    }
}

fn insert_body_ref(
    refs: &mut BTreeMap<String, ArtifactRef>,
    relative_path: Option<&str>,
) -> Result<()> {
    let Some(relative_path) = relative_path else {
        return Ok(());
    };
    let content_hash = note_body_content_hash_from_path(relative_path)?;
    insert_artifact_ref(
        refs,
        format!("body:{content_hash}"),
        ArtifactRef {
            locator: ArtifactLocator::Body {
                relative_path: relative_path.to_owned(),
            },
            content_hash,
        },
    )
}

fn insert_artifact_ref(
    refs: &mut BTreeMap<String, ArtifactRef>,
    key: String,
    artifact: ArtifactRef,
) -> Result<()> {
    if let Some(existing) = refs.get(&key) {
        if existing == &artifact {
            return Ok(());
        }
        return Err(ShoreError::Message(format!(
            "conflicting artifact reference for {}",
            artifact.content_hash
        )));
    }
    refs.insert(key, artifact);
    Ok(())
}

fn read_body_artifact_bytes(
    shore_dir: &Path,
    relative_path: &str,
    expected_content_hash: &str,
) -> Result<Vec<u8>> {
    let path = shore_dir.join(relative_path);
    let bytes = std::fs::read(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            return ShoreError::Message(format!(
                "missing artifact {expected_content_hash}; import referenced artifacts before reading"
            ));
        }
        ShoreError::Message(format!("read artifact {}: {error}", path.display()))
    })?;
    validate_note_body_artifact_bytes(relative_path, expected_content_hash, &bytes)?;
    Ok(bytes)
}

fn import_snapshot_artifact(
    shore_dir: &Path,
    storage: &LocalStorage,
    snapshot_id: &SnapshotId,
    expected_content_hash: &str,
    bytes: &[u8],
) -> Result<ImportArtifactOutcome> {
    let artifact: SnapshotArtifact = serde_json::from_slice(bytes)?;
    validate_snapshot_artifact_content_hash(&artifact)?;
    if artifact.snapshot.snapshot_id != *snapshot_id {
        return Err(ShoreError::Message(format!(
            "snapshot artifact locator mismatch for {}",
            snapshot_id.as_str()
        )));
    }
    if artifact.content_hash != expected_content_hash {
        return Err(ShoreError::Message(format!(
            "snapshot artifact content hash mismatch for {expected_content_hash}"
        )));
    }

    let path = snapshot_artifact_path(shore_dir, snapshot_id);
    match storage.create_file_exclusive(&path, bytes, Durability::Durable)? {
        CreateFileOutcome::Created => Ok(ImportArtifactOutcome::Created),
        CreateFileOutcome::AlreadyExists => {
            let existing: SnapshotArtifact = storage.read_json(&path)?;
            validate_snapshot_artifact_content_hash(&existing)?;
            if existing == artifact {
                Ok(ImportArtifactOutcome::Existing)
            } else {
                Err(ShoreError::Message(format!(
                    "snapshot artifact conflict for {}",
                    snapshot_id.as_str()
                )))
            }
        }
    }
}

fn import_body_artifact(
    storage: &LocalStorage,
    relative_path: &str,
    expected_content_hash: &str,
    bytes: &[u8],
) -> Result<ImportArtifactOutcome> {
    let artifact = validate_note_body_artifact_bytes(relative_path, expected_content_hash, bytes)?;
    let path = Path::new(relative_path);
    match storage.create_file_exclusive(path, bytes, Durability::Durable)? {
        CreateFileOutcome::Created => Ok(ImportArtifactOutcome::Created),
        CreateFileOutcome::AlreadyExists => {
            let existing_bytes = storage.read_bytes(path)?;
            let existing = validate_note_body_artifact_bytes(
                relative_path,
                expected_content_hash,
                &existing_bytes,
            )?;
            if existing == artifact {
                Ok(ImportArtifactOutcome::Existing)
            } else {
                Err(ShoreError::Message(format!(
                    "note body artifact conflict for {expected_content_hash}"
                )))
            }
        }
    }
}
