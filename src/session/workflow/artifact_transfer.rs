use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::error::{Result, ShoreError};
use crate::model::{ObjectId, id_prefix};
use crate::session::body_artifact::{body_artifact_field, note_body_content_hash_from_path};
use crate::session::event::{EventType, ShoreEvent, WorkObjectProposal, WorkObjectProposedPayload};
use crate::session::object_artifact::{
    decode_and_validate_object_artifact, read_bound_object_artifact_bytes,
};
use crate::session::store::content::ContentArtifacts;
use crate::session::store::resolution::{
    prepare_write_landing, resolve_read_store, resolve_write_store,
};
use crate::storage::{CreateOutcome, LocalStorage};

/// The kind of content-addressed artifact an event references.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactKind {
    /// A captured Revision's content object artifact.
    Object,
    /// A large note-shaped body artifact.
    Body,
}

/// An opaque reference to a content-addressed artifact required by one or more
/// events.
///
/// The stable surface exposes the artifact kind and content hash. Any locator
/// needed to read or write Shoreline's current on-disk layout stays private and
/// must be passed back to [`export_artifact`] / [`import_artifact`]. Remote
/// consumers derive these refs from forwarded events with
/// [`referenced_artifacts`], fetch bytes by [`ArtifactRef::content_hash`], and
/// pass the same refs to [`import_artifact`].
#[derive(Clone, Eq, PartialEq)]
pub struct ArtifactRef {
    locator: ArtifactLocator,
    content_hash: String,
}

impl ArtifactRef {
    /// The artifact's broad kind.
    pub fn kind(&self) -> ArtifactKind {
        match self.locator {
            ArtifactLocator::Object { .. } => ArtifactKind::Object,
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
    Object { object_id: ObjectId },
    Body { relative_path: String },
}

/// Options for importing a content-addressed artifact into a repo's `.shore/data`
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
///
/// Reads resolve through the linked clone-local store when one is registered
/// for the worktree. Imports stay worktree-local; see [`import_artifact`].
pub fn export_artifact(repo: impl AsRef<Path>, artifact: &ArtifactRef) -> Result<Vec<u8>> {
    match &artifact.locator {
        ArtifactLocator::Object { object_id } => {
            let bytes = read_bound_object_artifact_bytes(repo, object_id, &artifact.content_hash)?;
            let stored = decode_and_validate_object_artifact(&bytes)?;
            if stored.content_hash != artifact.content_hash {
                return Err(ShoreError::Message(format!(
                    "object artifact content hash mismatch for {}",
                    artifact.content_hash
                )));
            }
            Ok(bytes)
        }
        ArtifactLocator::Body { relative_path } => {
            let read_store = resolve_read_store(repo.as_ref())?;
            ContentArtifacts::from_backend(read_store.backend())
                .read_note_body_bytes(relative_path, &artifact.content_hash)
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
    let write_store = resolve_write_store(&options.repo)?;
    let storage = LocalStorage::new(write_store.store_dir());
    // The dir layout + `.git/info/exclude` are a worktree/file concern with no
    // non-file analogue, so landing prep stays on `LocalStorage`; the content
    // I/O below flows through the resolved backend handle.
    prepare_write_landing(&write_store, &storage)?;

    let content = ContentArtifacts::from_backend(write_store.backend());
    let outcome = match &options.artifact.locator {
        ArtifactLocator::Object { object_id } => {
            content.import_object(object_id, &options.artifact.content_hash, &options.bytes)?
        }
        ArtifactLocator::Body { relative_path } => content.import_body(
            relative_path,
            &options.artifact.content_hash,
            &options.bytes,
        )?,
    };

    Ok(ImportArtifactResult {
        artifact: options.artifact,
        outcome: match outcome {
            CreateOutcome::Created => ImportArtifactOutcome::Created,
            CreateOutcome::AlreadyExists => ImportArtifactOutcome::Existing,
        },
    })
}

fn referenced_artifacts_for_event(
    event: &ShoreEvent,
    refs: &mut BTreeMap<String, ArtifactRef>,
) -> Result<()> {
    // The object family externalizes a captured Revision's content object; it
    // needs the typed `ObjectId`, so it stays a typed, strict arm that errors on
    // a malformed payload.
    if event.event_type == EventType::WorkObjectProposed {
        let payload: WorkObjectProposedPayload = serde_json::from_value(event.payload.clone())?;
        if let WorkObjectProposal::Revision {
            revision,
            object_artifact_content_hash,
            ..
        } = payload.work_object
        {
            insert_artifact_ref(
                refs,
                format!(
                    "{}:{object_artifact_content_hash}",
                    id_prefix::ARTIFACT_OBJECT
                ),
                ArtifactRef {
                    locator: ArtifactLocator::Object {
                        object_id: revision.object_id,
                    },
                    content_hash: object_artifact_content_hash,
                },
            )?;
        }
        // A task-attempt proposal references no content-addressed artifact.
        return Ok(());
    }

    // Every body-bearing family externalizes exactly one path field, named by the
    // shared registry. Read it leniently from raw JSON (aligned with the bundle
    // path); `insert_body_ref` still validates the path shape and hash.
    if let Some(field) = body_artifact_field(event.event_type) {
        let path = event
            .payload
            .get(field.payload_field())
            .and_then(|value| value.as_str());
        return insert_body_ref(refs, path);
    }

    Ok(())
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
        format!("{}:{content_hash}", id_prefix::ARTIFACT_BODY),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        JournalId, RevisionId, TrackId, ValidationCheckId, ValidationStatus, ValidationTarget,
        ValidationTrigger,
    };
    use crate::session::event::{EventTarget, EventType, ValidationCheckRecordedPayload, Writer};

    #[test]
    fn referenced_artifacts_includes_validation_summary_body() {
        let hash = "a".repeat(64);
        let event = validation_event_with_summary_path(&format!("artifacts/notes/{hash}.json"));

        let refs = referenced_artifacts(&[event]).unwrap();

        assert!(refs.iter().any(|artifact| {
            artifact.kind() == ArtifactKind::Body
                && artifact.content_hash() == format!("sha256:{hash}")
        }));
    }

    #[test]
    fn every_registry_body_family_yields_a_body_ref() {
        let hash = "b".repeat(64);
        let path = format!("artifacts/notes/{hash}.json");

        for event_type in EventType::ALL {
            let Some(field) = body_artifact_field(event_type) else {
                continue;
            };
            let field_name = field.payload_field();
            let mut event = base_event();
            event.event_type = event_type;
            event.payload = serde_json::json!({ field_name: path });

            let refs = referenced_artifacts(&[event]).unwrap();
            assert!(
                refs.iter().any(|a| a.kind() == ArtifactKind::Body
                    && a.content_hash() == format!("sha256:{hash}")),
                "path 1 dropped the body artifact for {event_type:?}"
            );
        }
    }

    #[test]
    fn non_body_families_yield_no_body_ref() {
        for event_type in EventType::ALL {
            if body_artifact_field(event_type).is_some()
                || event_type == EventType::WorkObjectProposed
            {
                continue; // body families and the object family are covered elsewhere
            }
            let mut event = base_event();
            event.event_type = event_type;
            event.payload = serde_json::json!({ "bodyArtifactPath": "artifacts/notes/x.json" });

            let refs = referenced_artifacts(&[event]).unwrap();
            assert!(
                refs.iter().all(|a| a.kind() != ArtifactKind::Body),
                "path 1 spuriously enumerated a body artifact for non-body {event_type:?}"
            );
        }
    }

    #[test]
    fn malformed_work_object_proposed_payload_still_errors() {
        // The object family stays typed + strict: a malformed payload must error,
        // not be silently skipped.
        let mut event = base_event();
        event.event_type = EventType::WorkObjectProposed;
        event.payload = serde_json::json!({ "workObject": "not-an-object" });

        assert!(referenced_artifacts(&[event]).is_err());
    }

    /// A minimal valid event to clone/overwrite; the enumeration reads only
    /// `event_type` + `payload`, so any well-formed base works.
    fn base_event() -> ShoreEvent {
        validation_event_with_summary_path("artifacts/notes/placeholder.json")
    }

    fn validation_event_with_summary_path(path: &str) -> ShoreEvent {
        let revision_id = RevisionId::new("review-unit:sha256:one");
        let target = EventTarget::for_revision(
            JournalId::new("journal:default"),
            revision_id.clone(),
            Some(TrackId::new("agent:codex")),
        )
        .unwrap();
        ShoreEvent::new(
            EventType::ValidationCheckRecorded,
            "validation_check_recorded:one",
            target,
            Writer::shore_local("0.1.0"),
            ValidationCheckRecordedPayload {
                validation_check_id: ValidationCheckId::new("validation:sha256:one"),
                target: ValidationTarget::Revision { revision_id },
                check_name: "cargo test".to_owned(),
                command: None,
                status: ValidationStatus::Passed,
                exit_code: Some(0),
                trigger: ValidationTrigger::Manual,
                source_fingerprint: None,
                summary: None,
                summary_content_type: Default::default(),
                summary_artifact_path: Some(path.to_owned()),
                summary_byte_size: Some(10),
                summary_content_hash: Some("sha256:summary".to_owned()),
                started_at: None,
                completed_at: None,
                log_artifact_content_hashes: Vec::new(),
            },
            "2026-05-13T10:00:00Z",
        )
        .unwrap()
    }
}
