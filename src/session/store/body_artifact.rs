use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use crate::canonical_hash::sha256_bytes_hex;
use crate::error::{Result, ShoreError};
use crate::session::event::EventType;
use crate::session::store::backend::StoreBackend;
use crate::session::store::content::ContentArtifacts;

/// Inline/artifact threshold for note-shaped event bodies (observations,
/// input request bodies / response reasons, assessment summaries, imported
/// review notes).
///
/// Bodies whose byte length is at most this value remain inline in the event
/// payload. Larger bodies are externalized to `artifacts/notes/<sha256>.json`
/// under the `shore.note-body` envelope.
///
/// This value is internal storage tuning and may change without a deprecation
/// cycle. The inline-or-artifact bifurcation itself is the stable contract.
///
/// See `docs/adr/adr-0001-note-body-materialization.md`.
pub(crate) const BODY_INLINE_LIMIT: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NoteBodyEnvelope {
    pub schema: String,
    pub version: u32,
    pub body: String,
    /// Ordered content-coding tokens (compression/encryption) applied to the
    /// stored note-body file in list order at write and reversed on read;
    /// default `[]` is the identity encoding. The content hash is sha256 over
    /// the decoded `body` alone, so this field never enters it. Reserved — no
    /// codec populates it yet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_encoding: Vec<String>,
}

impl NoteBodyEnvelope {
    pub(crate) fn new(body: String) -> Self {
        Self {
            schema: "shore.note-body".to_owned(),
            version: 1,
            body,
            content_encoding: Vec::new(),
        }
    }

    pub(crate) fn to_json_bytes(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(self)?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BodyArtifactOutcome {
    Inline {
        byte_size: u64,
    },
    Artifact {
        relative_path: String,
        byte_size: u64,
        body_envelope: NoteBodyEnvelope,
    },
}

/// Decide whether a note-shaped body stays inline or is externalized to
/// `artifacts/notes/<sha256(body)>.json`.
///
/// Returns [`BodyArtifactOutcome::Inline`] when the body's byte length is at
/// most [`BODY_INLINE_LIMIT`]; otherwise returns
/// [`BodyArtifactOutcome::Artifact`] with a content-addressed relative path
/// and a [`NoteBodyEnvelope`] (`schema = "shore.note-body"`, `version = 1`).
///
/// Replay (`EventStore::list_events()` + [`load_body_artifact`]) is the
/// authoritative read primitive. See
/// `docs/adr/adr-0001-note-body-materialization.md`.
pub(crate) fn stage_body_artifact(body_bytes: &[u8]) -> Result<BodyArtifactOutcome> {
    let body = std::str::from_utf8(body_bytes)
        .map_err(|err| ShoreError::Message(format!("body artifact must be utf-8: {err}")))?;
    let byte_size = body_bytes.len() as u64;

    if body_bytes.len() <= BODY_INLINE_LIMIT {
        return Ok(BodyArtifactOutcome::Inline { byte_size });
    }

    let body_hash = sha256_bytes_hex(body_bytes);
    Ok(BodyArtifactOutcome::Artifact {
        relative_path: format!("artifacts/notes/{body_hash}.json"),
        byte_size,
        body_envelope: NoteBodyEnvelope::new(body.to_owned()),
    })
}

pub(crate) fn load_body_artifact(
    backend: &StoreBackend,
    relative_path: &str,
) -> Result<Option<String>> {
    validate_body_artifact_read_path(relative_path)?;

    let body = ContentArtifacts::from_backend(backend).read_note_body(relative_path)?;

    Ok(Some(body))
}

pub(crate) fn note_body_content_hash_from_path(relative_path: &str) -> Result<String> {
    let stem = validate_note_body_artifact_path(relative_path)?;
    Ok(format!("sha256:{stem}"))
}

pub(crate) fn parse_note_body_artifact(bytes: &[u8]) -> Result<NoteBodyEnvelope> {
    let artifact: NoteBodyEnvelope = serde_json::from_slice(bytes)?;
    if artifact.schema != "shore.note-body" || artifact.version != 1 {
        return Err(ShoreError::Message(format!(
            "Unsupported note body artifact schema/version: {} v{}",
            artifact.schema, artifact.version
        )));
    }
    Ok(artifact)
}

pub(crate) fn validate_note_body_artifact_bytes(
    relative_path: &str,
    expected_content_hash: &str,
    bytes: &[u8],
) -> Result<NoteBodyEnvelope> {
    let path_content_hash = note_body_content_hash_from_path(relative_path)?;
    if path_content_hash != expected_content_hash {
        return Err(ShoreError::Message(format!(
            "note body artifact locator hash mismatch for {expected_content_hash}"
        )));
    }

    let artifact = parse_note_body_artifact(bytes)?;
    let actual_content_hash = format!("sha256:{}", sha256_bytes_hex(artifact.body.as_bytes()));
    if actual_content_hash != expected_content_hash {
        return Err(ShoreError::Message(format!(
            "note body artifact content hash mismatch for {expected_content_hash}"
        )));
    }

    Ok(artifact)
}

fn validate_note_body_artifact_path(relative_path: &str) -> Result<&str> {
    if Path::new(relative_path).components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(invalid_artifact_path(relative_path));
    }

    let Some(stem) = relative_path
        .strip_prefix("artifacts/notes/")
        .and_then(|path| path.strip_suffix(".json"))
    else {
        return Err(invalid_artifact_path(relative_path));
    };
    if stem.len() != 64 || !stem.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid_artifact_path(relative_path));
    }

    Ok(stem)
}

fn validate_body_artifact_read_path(relative_path: &str) -> Result<()> {
    if !relative_path.starts_with("artifacts/notes/")
        || Path::new(relative_path).components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(invalid_artifact_path(relative_path));
    }

    Ok(())
}

fn invalid_artifact_path(relative_path: &str) -> ShoreError {
    ShoreError::Message(format!("Invalid artifact path: {relative_path}"))
}

/// Which payload field, if any, carries a note-body artifact path for an event
/// family — the single registry both artifact-enumeration paths derive from
/// ([`referenced_artifacts`](crate::session::referenced_artifacts) in
/// `workflow::artifact_transfer` and `note_body_artifact_paths_for_event` in
/// `store::bundle`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::session) enum BodyArtifactField {
    Body,
    Reason,
    Summary,
}

impl BodyArtifactField {
    /// The camelCase payload field carrying the artifact path.
    pub(in crate::session) fn payload_field(self) -> &'static str {
        match self {
            Self::Body => "bodyArtifactPath",
            Self::Reason => "reasonArtifactPath",
            Self::Summary => "summaryArtifactPath",
        }
    }
}

/// The note-body slot an event family externalizes to, or `None` for families
/// with no externalized body. Exhaustive with no wildcard: a new [`EventType`]
/// variant fails to compile until it declares its slot here, so a body-bearing
/// family can never reach only one enumeration path (the failure mode where a
/// new family was registered on one path but silently missed on the other).
pub(in crate::session) fn body_artifact_field(event_type: EventType) -> Option<BodyArtifactField> {
    match event_type {
        EventType::ReviewObservationRecorded
        | EventType::InputRequestOpened
        | EventType::ReviewNoteImported
        | EventType::TaskObservationRecorded => Some(BodyArtifactField::Body),
        EventType::InputRequestResponded => Some(BodyArtifactField::Reason),
        EventType::ReviewAssessmentRecorded | EventType::ValidationCheckRecorded => {
            Some(BodyArtifactField::Summary)
        }
        // Not note-body families. `WorkObjectProposed` externalizes an *object*
        // artifact, enumerated separately with its typed `ObjectId`.
        EventType::WorkObjectProposed
        | EventType::ReviewInitialized
        | EventType::RevisionRefAssociated
        | EventType::RevisionRefWithdrawn
        | EventType::RevisionCommitAssociated
        | EventType::RevisionCommitWithdrawn
        | EventType::TaskCheckpointCaptured
        | EventType::EventSignatureRecorded
        | EventType::ArtifactRemoved => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_artifact_field_pins_every_family() {
        use BodyArtifactField::{Body, Reason, Summary};
        for event_type in EventType::ALL {
            let expected = match event_type {
                EventType::ReviewObservationRecorded
                | EventType::InputRequestOpened
                | EventType::ReviewNoteImported
                | EventType::TaskObservationRecorded => Some(Body),
                EventType::InputRequestResponded => Some(Reason),
                EventType::ReviewAssessmentRecorded | EventType::ValidationCheckRecorded => {
                    Some(Summary)
                }
                _ => None,
            };
            assert_eq!(
                body_artifact_field(event_type),
                expected,
                "registry classification drifted for {event_type:?}"
            );
        }
    }

    #[test]
    fn payload_field_names_match_the_wire() {
        assert_eq!(BodyArtifactField::Body.payload_field(), "bodyArtifactPath");
        assert_eq!(
            BodyArtifactField::Reason.payload_field(),
            "reasonArtifactPath"
        );
        assert_eq!(
            BodyArtifactField::Summary.payload_field(),
            "summaryArtifactPath"
        );
    }

    #[test]
    fn validation_check_is_a_summary_body_family() {
        // Regression pin: this family must be body-bearing so both enumeration
        // paths carry it (it was once missed on one path).
        assert_eq!(
            body_artifact_field(EventType::ValidationCheckRecorded),
            Some(BodyArtifactField::Summary)
        );
    }

    /// The file backend (rooted at a temp dir the guard keeps alive) and the
    /// injection-only in-memory backend, so the note-body read is proven to flow
    /// through the handle for either backend.
    fn each_backend() -> Vec<(Option<tempfile::TempDir>, StoreBackend)> {
        let root = tempfile::tempdir().unwrap();
        let store_dir = root.path().join(".shore/data");
        vec![
            (Some(root), StoreBackend::Local(store_dir)),
            (None, StoreBackend::memory()),
        ]
    }

    #[test]
    fn body_inline_limit_is_the_documented_4096_bytes() {
        assert_eq!(BODY_INLINE_LIMIT, 4096);
    }

    #[test]
    fn body_of_exactly_inline_limit_bytes_returns_inline() {
        let body = vec![b'x'; BODY_INLINE_LIMIT];
        let outcome = stage_body_artifact(&body).unwrap();
        match outcome {
            BodyArtifactOutcome::Inline { byte_size } => {
                assert_eq!(byte_size, BODY_INLINE_LIMIT as u64);
            }
            other => panic!("expected inline at threshold, got {other:?}"),
        }
    }

    #[test]
    fn body_of_inline_limit_plus_one_bytes_returns_artifact() {
        let body = vec![b'x'; BODY_INLINE_LIMIT + 1];
        let outcome = stage_body_artifact(&body).unwrap();
        match outcome {
            BodyArtifactOutcome::Artifact {
                relative_path,
                byte_size,
                body_envelope: _,
            } => {
                assert!(relative_path.starts_with("artifacts/notes/"));
                assert_eq!(byte_size, (BODY_INLINE_LIMIT + 1) as u64);
            }
            other => panic!("expected artifact at threshold + 1, got {other:?}"),
        }
    }

    #[test]
    fn small_body_returns_inline_no_artifact() {
        let small = "tiny body";
        let outcome = stage_body_artifact(small.as_bytes()).unwrap();
        match outcome {
            BodyArtifactOutcome::Inline { byte_size } => assert_eq!(byte_size, small.len() as u64),
            other => panic!("expected inline, got {other:?}"),
        }
    }

    #[test]
    fn large_body_returns_artifact_descriptor() {
        let large = "x".repeat(BODY_INLINE_LIMIT + 1);
        let outcome = stage_body_artifact(large.as_bytes()).unwrap();
        match outcome {
            BodyArtifactOutcome::Artifact {
                relative_path,
                byte_size,
                body_envelope,
            } => {
                assert!(relative_path.starts_with("artifacts/notes/"));
                assert_eq!(byte_size, large.len() as u64);
                assert_eq!(body_envelope.body, large);
            }
            other => panic!("expected artifact, got {other:?}"),
        }
    }

    #[test]
    fn content_encoding_is_excluded_from_note_body_content_hash() {
        // The note-body content hash is sha256 over the decoded body alone;
        // contentEncoding describes how the stored envelope decodes to that
        // body. An envelope tagged with an encoding must validate against the
        // same content hash as the plain envelope.
        let body = "x".repeat(BODY_INLINE_LIMIT + 1);
        let body_stem = sha256_bytes_hex(body.as_bytes());
        let relative_path = format!("artifacts/notes/{body_stem}.json");
        let content_hash = format!("sha256:{body_stem}");

        let plain = NoteBodyEnvelope::new(body.clone());
        let mut encoded = NoteBodyEnvelope::new(body);
        encoded.content_encoding = vec!["zstd".to_owned()];

        validate_note_body_artifact_bytes(
            &relative_path,
            &content_hash,
            &plain.to_json_bytes().unwrap(),
        )
        .expect("the plain envelope validates against the body hash");
        let validated = validate_note_body_artifact_bytes(
            &relative_path,
            &content_hash,
            &encoded.to_json_bytes().unwrap(),
        )
        .expect("an encoding-tagged envelope validates against the same body hash");
        assert_eq!(validated.content_encoding, vec!["zstd".to_owned()]);
    }

    #[test]
    fn load_rejects_path_escape_with_parent_dir() {
        // Path validation runs before any store access, so the backend is moot.
        let err = load_body_artifact(&StoreBackend::memory(), "../escape.json").unwrap_err();
        assert!(err.to_string().contains("Invalid artifact path"));
    }

    #[test]
    fn load_rejects_path_outside_artifacts_notes() {
        let err = load_body_artifact(&StoreBackend::memory(), "elsewhere/x.json").unwrap_err();
        assert!(err.to_string().contains("Invalid artifact path"));
    }

    #[test]
    fn load_rejects_wrong_schema_over_every_backend() {
        for (_guard, backend) in each_backend() {
            backend
                .content_store()
                .put_once(
                    "artifacts/notes/x.json",
                    br#"{"schema":"wrong","version":1,"body":"x"}"#,
                )
                .unwrap();
            let err = load_body_artifact(&backend, "artifacts/notes/x.json").unwrap_err();
            assert!(err.to_string().contains("Unsupported note body artifact"));
        }
    }

    #[test]
    fn load_returns_body_when_schema_and_version_match_over_every_backend() {
        for (_guard, backend) in each_backend() {
            backend
                .content_store()
                .put_once(
                    "artifacts/notes/x.json",
                    br#"{"schema":"shore.note-body","version":1,"body":"the body"}"#,
                )
                .unwrap();
            let body = load_body_artifact(&backend, "artifacts/notes/x.json").unwrap();
            assert_eq!(body, Some("the body".to_owned()));
        }
    }
}
