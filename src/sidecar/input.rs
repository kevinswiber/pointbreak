use std::path::Path;

use crate::error::{Result, ShoreError};

#[derive(Debug)]
pub(crate) struct SidecarInput {
    pub(crate) bytes: Vec<u8>,
    pub(crate) text: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SidecarInputKind {
    ReviewNotes,
    LegacyHunkAgentContext,
}

impl SidecarInputKind {
    fn label(self) -> &'static str {
        match self {
            Self::ReviewNotes => "review notes",
            Self::LegacyHunkAgentContext => "legacy Hunk agent context",
        }
    }
}

pub(crate) fn read_sidecar_input(path: &Path, kind: SidecarInputKind) -> Result<SidecarInput> {
    let bytes = std::fs::read(path).map_err(|error| {
        ShoreError::Message(format!(
            "read {} input {}: {error}",
            kind.label(),
            path.display()
        ))
    })?;
    let text = String::from_utf8(bytes.clone()).map_err(|error| {
        ShoreError::Message(format!(
            "read {} input {} as utf-8: {error}",
            kind.label(),
            path.display()
        ))
    })?;

    Ok(SidecarInput { bytes, text })
}

pub(crate) fn read_review_notes_sidecar_file(path: &Path) -> Result<SidecarInput> {
    read_sidecar_input(path, SidecarInputKind::ReviewNotes)
}

pub(crate) fn read_legacy_hunk_agent_context_file(path: &Path) -> Result<SidecarInput> {
    read_sidecar_input(path, SidecarInputKind::LegacyHunkAgentContext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_review_notes_file_error_names_path_and_kind() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("missing-review-notes.json");

        let error = read_review_notes_sidecar_file(&path).expect_err("missing file fails");
        let message = error.to_string();

        assert!(message.contains("review notes"));
        assert!(message.contains("missing-review-notes.json"));
    }

    #[test]
    fn missing_legacy_hunk_file_error_names_path_and_kind() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("missing-agent-context.json");

        let error = read_legacy_hunk_agent_context_file(&path).expect_err("missing file fails");
        let message = error.to_string();

        assert!(message.contains("legacy Hunk agent context"));
        assert!(message.contains("missing-agent-context.json"));
    }
}
