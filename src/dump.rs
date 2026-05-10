use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::git::ingest_tracked_diff;
use crate::model::{DiffSnapshot, ReviewNote, ReviewStream};
use crate::sidecar::{
    ParsedReviewNotes, ReviewNotesDiagnostic, apply_file_order, parse_hunk_agent_context,
    parse_review_notes_sidecar, read_legacy_hunk_agent_context_file,
    read_review_notes_sidecar_file, resolve_notes,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DumpDocument {
    pub schema: String,
    pub version: u32,
    pub input: DumpInputSummary,
    pub summary: DumpSummary,
    pub diagnostics: Vec<ReviewNotesDiagnostic>,
    pub snapshot: DiffSnapshot,
    pub notes: Vec<ReviewNote>,
    pub stream: ReviewStream,
}

impl DumpDocument {
    pub fn new(
        input: DumpInputSummary,
        snapshot: DiffSnapshot,
        notes: Vec<ReviewNote>,
        stream: ReviewStream,
        diagnostics: Vec<ReviewNotesDiagnostic>,
    ) -> Self {
        let summary = DumpSummary::from_parts(&snapshot, &notes, &stream, &diagnostics);
        Self {
            schema: "shore.dump".to_owned(),
            version: 1,
            input,
            summary,
            diagnostics,
            snapshot,
            notes,
            stream,
        }
    }

    pub fn from_repo(repo: impl AsRef<Path>) -> Result<Self> {
        let snapshot = ingest_tracked_diff(repo)?;
        let notes = Vec::new();
        let stream = ReviewStream::from_snapshot_with_resolved_notes(&snapshot, &notes);
        Ok(Self::new(
            DumpInputSummary {
                source: DumpInputSource::None,
            },
            snapshot,
            notes,
            stream,
            Vec::new(),
        ))
    }

    pub fn from_parsed_review_notes(
        repo: impl AsRef<Path>,
        parsed: ParsedReviewNotes,
    ) -> Result<Self> {
        Self::from_parsed_notes(repo, parsed, DumpInputSource::ReviewNotes)
    }

    pub fn from_review_notes_file(repo: impl AsRef<Path>, path: impl AsRef<Path>) -> Result<Self> {
        let input = read_review_notes_sidecar_file(path.as_ref())?;
        let parsed = parse_review_notes_sidecar(&input.text)?;
        Self::from_parsed_review_notes(repo, parsed)
    }

    pub fn from_legacy_hunk_agent_context(
        repo: impl AsRef<Path>,
        parsed: ParsedReviewNotes,
    ) -> Result<Self> {
        Self::from_parsed_notes(repo, parsed, DumpInputSource::LegacyHunkAgentContext)
    }

    pub fn from_legacy_hunk_agent_context_file(
        repo: impl AsRef<Path>,
        path: impl AsRef<Path>,
    ) -> Result<Self> {
        let input = read_legacy_hunk_agent_context_file(path.as_ref())?;
        let parsed = parse_hunk_agent_context(&input.text)?;
        Self::from_legacy_hunk_agent_context(repo, parsed)
    }

    fn from_parsed_notes(
        repo: impl AsRef<Path>,
        parsed: ParsedReviewNotes,
        source: DumpInputSource,
    ) -> Result<Self> {
        let snapshot = ingest_tracked_diff(repo)?;
        let ordered = apply_file_order(snapshot.files, &parsed.sidecar);
        let ordered_snapshot =
            DiffSnapshot::new(snapshot.review_id, snapshot.snapshot_id, ordered.files);
        let resolved = resolve_notes(&ordered_snapshot.files, &parsed.sidecar);
        let mut diagnostics = parsed.diagnostics;
        extend_unique_diagnostics(&mut diagnostics, ordered.diagnostics);
        extend_unique_diagnostics(&mut diagnostics, resolved.diagnostics);
        let stream =
            ReviewStream::from_snapshot_with_resolved_notes(&ordered_snapshot, &resolved.notes);

        Ok(Self::new(
            DumpInputSummary { source },
            ordered_snapshot,
            resolved.notes,
            stream,
            diagnostics,
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DumpInputSummary {
    pub source: DumpInputSource,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DumpInputSource {
    None,
    ReviewNotes,
    LegacyHunkAgentContext,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DumpSummary {
    pub file_count: usize,
    pub hunk_count: usize,
    pub row_count: usize,
    pub note_count: usize,
    pub diagnostic_count: usize,
}

impl DumpSummary {
    fn from_parts(
        snapshot: &DiffSnapshot,
        notes: &[ReviewNote],
        stream: &ReviewStream,
        diagnostics: &[ReviewNotesDiagnostic],
    ) -> Self {
        Self {
            file_count: snapshot.files.len(),
            hunk_count: snapshot.files.iter().map(|file| file.hunks.len()).sum(),
            row_count: stream.rows.len(),
            note_count: notes.len(),
            diagnostic_count: diagnostics.len(),
        }
    }
}

fn extend_unique_diagnostics(
    diagnostics: &mut Vec<ReviewNotesDiagnostic>,
    new_diagnostics: Vec<ReviewNotesDiagnostic>,
) {
    for diagnostic in new_diagnostics {
        if !diagnostics
            .iter()
            .any(|existing| existing.code == diagnostic.code && existing.path == diagnostic.path)
        {
            diagnostics.push(diagnostic);
        }
    }
}
