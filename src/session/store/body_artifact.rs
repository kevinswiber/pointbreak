use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use crate::canonical_hash::sha256_bytes_hex;
use crate::error::{Result, ShoreError};

/// Inline/artifact threshold for note-shaped event bodies (observations,
/// intervention bodies / resolution reasons, assessment summaries, imported
/// review notes).
///
/// Bodies whose byte length is at most this value remain inline in the event
/// payload. Larger bodies are externalized to `artifacts/notes/<sha256>.json`
/// under the `shore.note-body` envelope.
///
/// This value is internal storage tuning and may change without a deprecation
/// cycle. The inline-or-artifact bifurcation itself is the stable contract.
///
/// See `docs/adr-0001-note-body-materialization.md`.
pub(crate) const BODY_INLINE_LIMIT: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NoteBodyEnvelope {
    pub schema: String,
    pub version: u32,
    pub body: String,
}

impl NoteBodyEnvelope {
    pub(crate) fn new(body: String) -> Self {
        Self {
            schema: "shore.note-body".to_owned(),
            version: 1,
            body,
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
/// `docs/adr-0001-note-body-materialization.md`.
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

pub(crate) fn load_body_artifact(shore_dir: &Path, relative_path: &str) -> Result<Option<String>> {
    if !relative_path.starts_with("artifacts/notes/")
        || Path::new(relative_path).components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(ShoreError::Message(format!(
            "Invalid artifact path: {}",
            relative_path
        )));
    }

    let artifact_bytes = std::fs::read(shore_dir.join(relative_path)).map_err(|err| {
        ShoreError::Message(format!(
            "Failed to read artifact {}: {}",
            relative_path, err
        ))
    })?;
    let artifact: NoteBodyEnvelope = serde_json::from_slice(&artifact_bytes)?;
    if artifact.schema != "shore.note-body" || artifact.version != 1 {
        return Err(ShoreError::Message(format!(
            "Unsupported note body artifact schema/version: {} v{}",
            artifact.schema, artifact.version
        )));
    }

    Ok(Some(artifact.body))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn load_rejects_path_escape_with_parent_dir() {
        let dir = tempfile::tempdir().unwrap();
        let err = load_body_artifact(dir.path(), "../escape.json").unwrap_err();
        assert!(err.to_string().contains("Invalid artifact path"));
    }

    #[test]
    fn load_rejects_path_outside_artifacts_notes() {
        let dir = tempfile::tempdir().unwrap();
        let err = load_body_artifact(dir.path(), "elsewhere/x.json").unwrap_err();
        assert!(err.to_string().contains("Invalid artifact path"));
    }

    #[test]
    fn load_rejects_wrong_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("artifacts/notes/x.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"schema":"wrong","version":1,"body":"x"}"#).unwrap();
        let err = load_body_artifact(dir.path(), "artifacts/notes/x.json").unwrap_err();
        assert!(err.to_string().contains("Unsupported note body artifact"));
    }

    #[test]
    fn load_returns_body_when_schema_and_version_match() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("artifacts/notes/x.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"schema":"shore.note-body","version":1,"body":"the body"}"#,
        )
        .unwrap();
        let body = load_body_artifact(dir.path(), "artifacts/notes/x.json").unwrap();
        assert_eq!(body, Some("the body".to_owned()));
    }
}
