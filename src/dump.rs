use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::git::{IngestOptions, ingest_tracked_diff_with_options};
use crate::model::{DiffSnapshot, ReviewNote, ReviewStream};
use crate::session::event::{AcknowledgementNextAction, VerdictDecision, Writer};
use crate::session::{
    Acknowledgement, CurrentVerdictView, ReloadDiagnostic, ReviewArtifact, current_verdict_view,
    load_durable_notes_for_repo, load_or_rebuild_session_state, read_acknowledgements,
    read_review_artifacts, reload_diagnostics_for_document,
};
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_artifacts: Option<ReviewArtifactsSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reload_diagnostics: Option<ReloadDiagnosticsSection>,
}

#[derive(Clone, Debug, Default)]
pub struct DumpOptions {
    helper_paths: Vec<PathBuf>,
}

impl DumpOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn exclude_helper_path(mut self, path: impl AsRef<Path>) -> Self {
        self.helper_paths.push(path.as_ref().to_path_buf());
        self
    }

    fn ingest_options(&self) -> IngestOptions {
        self.helper_paths
            .iter()
            .fold(IngestOptions::new(), |options, path| {
                options.exclude_helper_path(path)
            })
    }
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
            review_artifacts: None,
            reload_diagnostics: None,
        }
    }

    pub fn from_repo(repo: impl AsRef<Path>) -> Result<Self> {
        Self::from_repo_with_options(repo, DumpOptions::new())
    }

    pub fn from_repo_with_options(repo: impl AsRef<Path>, options: DumpOptions) -> Result<Self> {
        let repo_path = repo.as_ref();

        // Try to load durable notes first; fall back to empty if none exist
        if let Some(parsed) = load_durable_notes_for_repo(repo_path)? {
            return Self::from_parsed_notes_with_snapshot_order(
                repo_path,
                parsed,
                DumpInputSource::Durable,
                options,
            );
        }

        // No durable notes; ingest diff and use empty note path
        let snapshot = ingest_tracked_diff_with_options(repo_path, options.ingest_options())?;
        let notes = Vec::new();
        let stream = ReviewStream::from_snapshot_with_resolved_notes(&snapshot, &notes);
        let mut document = Self::new(
            DumpInputSummary {
                source: DumpInputSource::None,
            },
            snapshot,
            notes,
            stream,
            Vec::new(),
        );
        attach_review_artifacts_section(&mut document, repo_path)?;
        attach_reload_diagnostics_section(&mut document, repo_path)?;
        Ok(document)
    }

    pub fn from_parsed_review_notes(
        repo: impl AsRef<Path>,
        parsed: ParsedReviewNotes,
    ) -> Result<Self> {
        Self::from_parsed_notes(
            repo,
            parsed,
            DumpInputSource::ReviewNotes,
            DumpOptions::new(),
        )
    }

    pub fn from_review_notes_file(repo: impl AsRef<Path>, path: impl AsRef<Path>) -> Result<Self> {
        Self::from_review_notes_file_with_options(repo, path, DumpOptions::new())
    }

    pub fn from_review_notes_file_with_options(
        repo: impl AsRef<Path>,
        path: impl AsRef<Path>,
        options: DumpOptions,
    ) -> Result<Self> {
        let input = read_review_notes_sidecar_file(path.as_ref())?;
        let parsed = parse_review_notes_sidecar(&input.text)?;
        Self::from_parsed_notes(repo, parsed, DumpInputSource::ReviewNotes, options)
    }

    pub fn from_legacy_hunk_agent_context(
        repo: impl AsRef<Path>,
        parsed: ParsedReviewNotes,
    ) -> Result<Self> {
        Self::from_parsed_notes(
            repo,
            parsed,
            DumpInputSource::LegacyHunkAgentContext,
            DumpOptions::new(),
        )
    }

    pub fn from_legacy_hunk_agent_context_file(
        repo: impl AsRef<Path>,
        path: impl AsRef<Path>,
    ) -> Result<Self> {
        Self::from_legacy_hunk_agent_context_file_with_options(repo, path, DumpOptions::new())
    }

    pub fn from_legacy_hunk_agent_context_file_with_options(
        repo: impl AsRef<Path>,
        path: impl AsRef<Path>,
        options: DumpOptions,
    ) -> Result<Self> {
        let input = read_legacy_hunk_agent_context_file(path.as_ref())?;
        let parsed = parse_hunk_agent_context(&input.text)?;
        Self::from_parsed_notes(
            repo,
            parsed,
            DumpInputSource::LegacyHunkAgentContext,
            options,
        )
    }

    fn from_parsed_notes(
        repo: impl AsRef<Path>,
        parsed: ParsedReviewNotes,
        source: DumpInputSource,
        options: DumpOptions,
    ) -> Result<Self> {
        let repo_path = repo.as_ref();
        let snapshot = ingest_tracked_diff_with_options(repo_path, options.ingest_options())?;
        let ordered = apply_file_order(snapshot.files, &parsed.sidecar);
        let ordered_snapshot =
            DiffSnapshot::new(snapshot.review_id, snapshot.snapshot_id, ordered.files);
        let resolved = resolve_notes(&ordered_snapshot.files, &parsed.sidecar);
        let mut diagnostics = parsed.diagnostics;
        extend_unique_diagnostics(&mut diagnostics, ordered.diagnostics);
        extend_unique_diagnostics(&mut diagnostics, resolved.diagnostics);
        let stream =
            ReviewStream::from_snapshot_with_resolved_notes(&ordered_snapshot, &resolved.notes);

        let mut document = Self::new(
            DumpInputSummary { source },
            ordered_snapshot,
            resolved.notes,
            stream,
            diagnostics,
        );
        attach_review_artifacts_section(&mut document, repo_path)?;
        attach_reload_diagnostics_section(&mut document, repo_path)?;
        Ok(document)
    }

    fn from_parsed_notes_with_snapshot_order(
        repo: impl AsRef<Path>,
        parsed: ParsedReviewNotes,
        source: DumpInputSource,
        options: DumpOptions,
    ) -> Result<Self> {
        let repo_path = repo.as_ref();
        let snapshot = ingest_tracked_diff_with_options(repo_path, options.ingest_options())?;
        let resolved = resolve_notes(&snapshot.files, &parsed.sidecar);
        let mut diagnostics = parsed.diagnostics;
        extend_unique_diagnostics(&mut diagnostics, resolved.diagnostics);
        let stream = ReviewStream::from_snapshot_with_resolved_notes(&snapshot, &resolved.notes);

        let mut document = Self::new(
            DumpInputSummary { source },
            snapshot,
            resolved.notes,
            stream,
            diagnostics,
        );
        attach_review_artifacts_section(&mut document, repo_path)?;
        attach_reload_diagnostics_section(&mut document, repo_path)?;
        Ok(document)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewArtifactsSection {
    pub verdicts: Vec<VerdictView>,
    pub acknowledgements: Vec<AcknowledgementView>,
    pub current_verdict: CurrentVerdictDumpView,
    pub summary: ReviewArtifactsSummary,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReloadDiagnosticsSection {
    pub entries: Vec<ReloadDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VerdictView {
    pub id: String,
    pub work_unit_id: String,
    pub revision_id: String,
    pub decision: VerdictDecision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub replaces: Vec<String>,
    // Writer keeps its existing camelCase contract to match current sidecar precedent.
    pub reviewer: Writer,
    pub replaced: bool,
    #[serde(default, skip_serializing_if = "crate::dump::is_false")]
    pub stale: bool,
}

impl VerdictView {
    fn from_artifact(
        artifact: &ReviewArtifact,
        replaced_ids: &HashSet<&str>,
        stale_artifact_ids: &HashSet<&str>,
    ) -> Self {
        Self {
            id: artifact.id.as_str().to_owned(),
            work_unit_id: artifact.work_unit_id.as_str().to_owned(),
            revision_id: artifact.revision_id.as_str().to_owned(),
            decision: artifact.decision,
            summary: artifact.summary.clone(),
            replaces: artifact
                .replaces_review_artifact_ids
                .iter()
                .map(|id| id.as_str().to_owned())
                .collect(),
            reviewer: artifact.reviewer.clone(),
            replaced: replaced_ids.contains(artifact.id.as_str()),
            stale: stale_artifact_ids.contains(artifact.id.as_str()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AcknowledgementView {
    pub id: String,
    pub review_artifact_id: String,
    pub next_action: AcknowledgementNextAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub acknowledger: Writer,
    #[serde(default, skip_serializing_if = "crate::dump::is_false")]
    pub stale: bool,
}

impl AcknowledgementView {
    fn from_acknowledgement(acknowledgement: &Acknowledgement, stale: bool) -> Self {
        Self {
            id: acknowledgement.id.as_str().to_owned(),
            review_artifact_id: acknowledgement.review_artifact_id.as_str().to_owned(),
            next_action: acknowledgement.next_action,
            reason: acknowledgement.reason.clone(),
            acknowledger: acknowledgement.acknowledger.clone(),
            stale,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CurrentVerdictDumpView {
    pub status: CurrentVerdictStatusName,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<VerdictDecision>,
    pub review_artifact_ids: Vec<String>,
}

impl CurrentVerdictDumpView {
    fn from_view(view: &CurrentVerdictView) -> Self {
        match view {
            CurrentVerdictView::Resolved {
                decision,
                review_artifact_id,
            } => Self {
                status: CurrentVerdictStatusName::Resolved,
                decision: Some(*decision),
                review_artifact_ids: vec![review_artifact_id.as_str().to_owned()],
            },
            CurrentVerdictView::Ambiguous {
                review_artifact_ids,
            } => Self {
                status: CurrentVerdictStatusName::Ambiguous,
                decision: None,
                review_artifact_ids: review_artifact_ids
                    .iter()
                    .map(|id| id.as_str().to_owned())
                    .collect(),
            },
            CurrentVerdictView::None => Self {
                status: CurrentVerdictStatusName::None,
                decision: None,
                review_artifact_ids: Vec::new(),
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurrentVerdictStatusName {
    Resolved,
    Ambiguous,
    None,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewArtifactsSummary {
    pub verdict_count: usize,
    pub acknowledgement_count: usize,
    pub unreplaced_verdict_count: usize,
}

fn attach_review_artifacts_section(document: &mut DumpDocument, repo: &Path) -> Result<()> {
    let Some(state) = load_or_rebuild_session_state(repo)? else {
        return Ok(());
    };

    let review_artifacts = read_review_artifacts(repo)?;
    let acknowledgements = read_acknowledgements(repo)?;
    let current_revision = state.current_revision_id.as_ref();
    let current_verdict = current_verdict_view(&review_artifacts, current_revision);
    let replaced_ids = review_artifacts
        .iter()
        .flat_map(|artifact| {
            artifact
                .replaces_review_artifact_ids
                .iter()
                .map(|id| id.as_str())
        })
        .collect::<HashSet<_>>();
    let stale_artifact_ids = review_artifacts
        .iter()
        .filter(|artifact| {
            current_revision.is_some_and(|revision| revision != &artifact.revision_id)
        })
        .map(|artifact| artifact.id.as_str())
        .collect::<HashSet<_>>();
    let known_artifact_ids = review_artifacts
        .iter()
        .map(|artifact| artifact.id.as_str())
        .collect::<HashSet<_>>();
    let unreplaced_verdict_count = review_artifacts
        .iter()
        .filter(|artifact| !replaced_ids.contains(artifact.id.as_str()))
        .count();

    document.review_artifacts = Some(ReviewArtifactsSection {
        verdicts: review_artifacts
            .iter()
            .map(|artifact| {
                VerdictView::from_artifact(artifact, &replaced_ids, &stale_artifact_ids)
            })
            .collect(),
        acknowledgements: acknowledgements
            .iter()
            .map(|acknowledgement| {
                let stale = stale_artifact_ids
                    .contains(acknowledgement.review_artifact_id.as_str())
                    || !known_artifact_ids.contains(acknowledgement.review_artifact_id.as_str());
                AcknowledgementView::from_acknowledgement(acknowledgement, stale)
            })
            .collect(),
        current_verdict: CurrentVerdictDumpView::from_view(&current_verdict),
        summary: ReviewArtifactsSummary {
            verdict_count: review_artifacts.len(),
            acknowledgement_count: acknowledgements.len(),
            unreplaced_verdict_count,
        },
    });

    Ok(())
}

fn attach_reload_diagnostics_section(document: &mut DumpDocument, repo: &Path) -> Result<()> {
    let diagnostics = reload_diagnostics_for_document(repo, document)?;
    if diagnostics.is_empty() {
        return Ok(());
    }

    document.reload_diagnostics = Some(ReloadDiagnosticsSection {
        entries: diagnostics,
    });
    Ok(())
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
    Durable,
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

pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}
