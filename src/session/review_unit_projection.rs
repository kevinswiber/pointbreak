use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::{Result, ShoreError};
use crate::model::{
    DiffFile, DiffSnapshot, DispositionId, InterventionId, ObservationId, ResolutionStatus,
    ReviewEndpoint, ReviewId, ReviewTargetRef, ReviewUnitId, ReviewUnitSource, RevisionId, RowId,
    SnapshotId, TrackId,
};
use crate::session::body_artifact::load_body_artifact;
use crate::session::disposition::{
    CurrentDispositionView, DispositionProjectionOptions, DispositionView, project_dispositions,
};
use crate::session::event::{
    EventType, ImportedNoteTarget, ReviewNoteImportedPayload, ReviewUnitCapturedPayload, ShoreEvent,
};
use crate::session::intervention::{
    InterventionProjectionOptions, InterventionStatusFilter, InterventionView,
    project_interventions,
};
use crate::session::observation::{
    ObservationProjectionOptions, ObservationView, ResolvedReviewUnit, project_observations,
    resolve_review_unit, validated_track_id,
};
use crate::session::snapshot_artifact::read_snapshot_artifact;
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store_init::ShoreStorePaths;
use crate::sidecar::{
    ReviewNoteEntry, ReviewNoteTarget, ReviewNotesFile, ReviewNotesSidecar, resolve_notes,
};
use crate::storage::EventStore;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewUnitShowOptions {
    repo: PathBuf,
    review_unit_id: Option<ReviewUnitId>,
    track: Option<String>,
    include_body: bool,
}

impl ReviewUnitShowOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_unit_id: None,
            track: None,
            include_body: false,
        }
    }

    pub fn with_review_unit_id(mut self, review_unit_id: ReviewUnitId) -> Self {
        self.review_unit_id = Some(review_unit_id);
        self
    }

    pub fn with_track(mut self, track: impl Into<String>) -> Self {
        self.track = Some(track.into());
        self
    }

    pub fn with_include_body(mut self, include_body: bool) -> Self {
        self.include_body = include_body;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewUnitShowResult {
    pub event_set_hash: String,
    pub event_count: usize,
    pub review_unit: ReviewUnitProjectionIdentity,
    pub snapshot: DiffSnapshot,
    pub filters: ReviewUnitShowFilters,
    pub summary: ReviewUnitProjectionSummary,
    pub current_disposition: CurrentDispositionView,
    pub observations: Vec<ObservationView>,
    pub interventions: Vec<InterventionView>,
    pub dispositions: Vec<DispositionView>,
    pub adapter_notes: Vec<AdapterNoteView>,
    pub rows: Vec<ReviewUnitProjectionRow>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewUnitProjectionIdentity {
    pub id: ReviewUnitId,
    pub review_id: ReviewId,
    pub source: ReviewUnitSource,
    pub base: ReviewEndpoint,
    pub target: ReviewEndpoint,
    pub revision_id: RevisionId,
    pub snapshot_id: SnapshotId,
    pub snapshot_artifact_content_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewUnitShowFilters {
    pub review_unit_id: ReviewUnitId,
    pub track_id: Option<TrackId>,
    pub include_body: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReviewUnitProjectionSummary {
    pub file_count: usize,
    pub row_count: usize,
    pub narrative_row_count: usize,
    pub snapshot_row_count: usize,
    pub snapshot_remainder_row_count: usize,
    pub observation_count: usize,
    pub intervention_count: usize,
    pub disposition_count: usize,
    pub adapter_note_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterNoteView {
    pub id: String,
    pub title: String,
    pub body: Option<String>,
    pub target: Option<ImportedNoteTarget>,
    pub status: AdapterNoteStatus,
    pub file_path: String,
    pub file_old_path: Option<String>,
    pub tags: Vec<String>,
    pub confidence: Option<String>,
    pub external_source: Option<String>,
    pub author: Option<String>,
    pub created_at: Option<String>,
    pub sidecar_content_hash: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterNoteStatus {
    Exact,
    Relocated,
    FileLevel,
    Stale,
    Orphaned,
    Unresolved,
}

impl AdapterNoteStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Relocated => "relocated",
            Self::FileLevel => "file_level",
            Self::Stale => "stale",
            Self::Orphaned => "orphaned",
            Self::Unresolved => "unresolved",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewUnitProjectionRow {
    pub id: RowId,
    pub kind: ReviewUnitProjectionRowKind,
    pub projection_phase: ProjectionPhase,
    pub projection_order: usize,
    pub snapshot_order: Option<SnapshotOrder>,
    pub coverage: ProjectionCoverage,
    pub target: Option<ReviewTargetRef>,
    pub file_path: Option<String>,
    pub old_path: Option<String>,
    pub related_observation_ids: Vec<ObservationId>,
    pub related_intervention_ids: Vec<InterventionId>,
    pub related_disposition_ids: Vec<DispositionId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewUnitProjectionRowKind {
    FileHeader,
    Metadata,
    HunkHeader,
    Diff,
    Observation,
    Intervention,
    Disposition,
    AdapterNote,
    EmptyState,
}

impl ReviewUnitProjectionRowKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FileHeader => "file_header",
            Self::Metadata => "metadata",
            Self::HunkHeader => "hunk_header",
            Self::Diff => "diff",
            Self::Observation => "observation",
            Self::Intervention => "intervention",
            Self::Disposition => "disposition",
            Self::AdapterNote => "adapter_note",
            Self::EmptyState => "empty_state",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionPhase {
    Narrative,
    SnapshotRemainder,
}

impl ProjectionPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Narrative => "narrative",
            Self::SnapshotRemainder => "snapshot_remainder",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionCoverage {
    Context,
    Reviewed,
    Unreviewed,
}

impl ProjectionCoverage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Context => "context",
            Self::Reviewed => "reviewed",
            Self::Unreviewed => "unreviewed",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotOrder {
    pub file_index: usize,
    pub metadata_index: Option<usize>,
    pub hunk_index: Option<usize>,
    pub row_index: Option<usize>,
}

pub fn show_review_unit(options: ReviewUnitShowOptions) -> Result<ReviewUnitShowResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let track_id = options
        .track
        .as_deref()
        .map(validated_track_id)
        .transpose()?;
    let events = EventStore::open(paths.shore_dir()).list_events()?;
    let resolved = resolve_review_unit(&events, options.review_unit_id.as_ref())?;
    let review_unit = selected_review_unit_capture(&events, &resolved)?;
    let snapshot = load_bound_snapshot_artifact(paths.worktree_root(), &review_unit)?;
    let observations = project_observations(ObservationProjectionOptions {
        shore_dir: paths.shore_dir(),
        events: &events,
        resolved: &resolved,
        track_filter: track_id.clone(),
        file_filter: None,
        include_body: options.include_body,
    })?;
    let interventions = project_interventions(InterventionProjectionOptions {
        shore_dir: paths.shore_dir(),
        events: &events,
        resolved: &resolved,
        track_filter: track_id.clone(),
        mode_filter: None,
        file_filter: None,
        status_filter: InterventionStatusFilter::All,
        include_body: options.include_body,
    })?;
    let (current_disposition, dispositions) = project_dispositions(DispositionProjectionOptions {
        shore_dir: paths.shore_dir(),
        events: &events,
        resolved: &resolved,
        track_filter: track_id.clone(),
        include_summary: options.include_body,
        include_all: true,
    })?;
    let adapter_notes =
        project_adapter_notes(&events, paths.shore_dir(), &snapshot, options.include_body)?;
    let (snapshot_rows, mut summary) = build_snapshot_rows(&snapshot, &review_unit.id);
    let mut narrative_rows = Vec::new();
    let observation_rows = build_observation_rows(&observations);
    summary.observation_count = observations.len();
    narrative_rows.extend(observation_rows);
    let intervention_rows = build_intervention_rows(&interventions);
    summary.intervention_count = interventions.len();
    narrative_rows.extend(intervention_rows);
    let disposition_rows = build_disposition_rows(&dispositions);
    summary.disposition_count = dispositions.len();
    narrative_rows.extend(disposition_rows);
    let adapter_note_rows = build_adapter_note_rows(&adapter_notes, &review_unit.id);
    summary.adapter_note_count = adapter_notes.len();
    narrative_rows.extend(adapter_note_rows);
    summary.narrative_row_count = narrative_rows.len();
    summary.row_count = summary.narrative_row_count + summary.snapshot_remainder_row_count;
    let mut rows = narrative_rows;
    rows.extend(snapshot_rows);
    renumber_projection_rows(&mut rows);
    let state = SessionState::from_events(&events)?;
    let event_set_hash = state
        .event_set_hash
        .clone()
        .expect("SessionState::from_events sets event_set_hash");

    Ok(ReviewUnitShowResult {
        event_set_hash,
        event_count: events.len(),
        review_unit,
        snapshot,
        filters: ReviewUnitShowFilters {
            review_unit_id: resolved.review_unit_id,
            track_id,
            include_body: options.include_body,
        },
        summary,
        current_disposition,
        observations,
        interventions,
        dispositions,
        adapter_notes,
        rows,
        diagnostics: state.diagnostics,
    })
}

fn load_bound_snapshot_artifact(
    repo: &Path,
    review_unit: &ReviewUnitProjectionIdentity,
) -> Result<DiffSnapshot> {
    let artifact = read_snapshot_artifact(repo, &review_unit.snapshot_id)?;
    if artifact.review_unit_id != review_unit.id
        || artifact.source != review_unit.source
        || artifact.base != review_unit.base
        || artifact.target != review_unit.target
        || artifact.snapshot.snapshot_id != review_unit.snapshot_id
    {
        return Err(ShoreError::Message(format!(
            "snapshot artifact metadata mismatch for {}",
            review_unit.id.as_str()
        )));
    }
    if artifact.content_hash != review_unit.snapshot_artifact_content_hash {
        return Err(ShoreError::Message(format!(
            "snapshot artifact content hash mismatch for {}",
            review_unit.id.as_str()
        )));
    }

    Ok(artifact.snapshot)
}

fn project_adapter_notes(
    events: &[ShoreEvent],
    shore_dir: &Path,
    snapshot: &DiffSnapshot,
    include_body: bool,
) -> Result<Vec<AdapterNoteView>> {
    let mut payloads = Vec::new();
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewNoteImported)
    {
        payloads.push(serde_json::from_value::<ReviewNoteImportedPayload>(
            event.payload.clone(),
        )?);
    }

    let statuses = adapter_note_statuses(snapshot, &payloads);
    let mut views = payloads
        .iter()
        .map(|payload| {
            let body = if include_body {
                adapter_note_body(shore_dir, payload)?
            } else {
                None
            };
            Ok(AdapterNoteView {
                id: payload.note_id.clone(),
                title: payload.title.clone(),
                body,
                target: payload.target.clone(),
                status: statuses
                    .get(&payload.note_id)
                    .copied()
                    .unwrap_or(AdapterNoteStatus::Unresolved),
                file_path: payload.file_path.clone(),
                file_old_path: payload.file_old_path.clone(),
                tags: payload.tags.clone(),
                confidence: payload.confidence.clone(),
                external_source: payload.external_source.clone(),
                author: payload.author.clone(),
                created_at: payload.created_at.clone(),
                sidecar_content_hash: payload.sidecar_content_hash.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    views.sort_by(|left, right| {
        left.file_path
            .cmp(&right.file_path)
            .then_with(|| {
                left.target
                    .as_ref()
                    .map(|target| target.start_line)
                    .cmp(&right.target.as_ref().map(|target| target.start_line))
            })
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(views)
}

fn adapter_note_statuses(
    snapshot: &DiffSnapshot,
    payloads: &[ReviewNoteImportedPayload],
) -> BTreeMap<String, AdapterNoteStatus> {
    let sidecar = ReviewNotesSidecar {
        schema: Some("shore.review-notes".to_owned()),
        version: 1,
        summary: None,
        files: payloads
            .iter()
            .map(|payload| ReviewNotesFile {
                path: payload.file_path.clone(),
                old_path: payload.file_old_path.clone(),
                summary: None,
                notes: vec![review_note_entry_from_payload(payload, None)],
            })
            .collect(),
    };
    resolve_notes(&snapshot.files, &sidecar)
        .notes
        .into_iter()
        .map(|note| {
            (
                note.id.as_str().to_owned(),
                adapter_note_status(&note.anchor.status),
            )
        })
        .collect()
}

fn review_note_entry_from_payload(
    payload: &ReviewNoteImportedPayload,
    body: Option<String>,
) -> ReviewNoteEntry {
    ReviewNoteEntry {
        id: Some(payload.note_id.clone()),
        title: Some(payload.title.clone()),
        body,
        target: payload.target.as_ref().map(imported_note_target),
        tags: payload.tags.clone(),
        confidence: payload.confidence.clone(),
        source: payload.external_source.clone(),
        author: payload.author.clone(),
        created_at: payload.created_at.clone(),
    }
}

fn imported_note_target(target: &ImportedNoteTarget) -> ReviewNoteTarget {
    ReviewNoteTarget {
        side: target.side,
        start_line: target.start_line,
        end_line: target.end_line,
    }
}

fn adapter_note_status(status: &ResolutionStatus) -> AdapterNoteStatus {
    match status {
        ResolutionStatus::Exact => AdapterNoteStatus::Exact,
        ResolutionStatus::Relocated => AdapterNoteStatus::Relocated,
        ResolutionStatus::FileLevel => AdapterNoteStatus::FileLevel,
        ResolutionStatus::Stale => AdapterNoteStatus::Stale,
        ResolutionStatus::Orphaned => AdapterNoteStatus::Orphaned,
        ResolutionStatus::Unresolved => AdapterNoteStatus::Unresolved,
    }
}

fn adapter_note_body(
    shore_dir: &Path,
    payload: &ReviewNoteImportedPayload,
) -> Result<Option<String>> {
    if payload.body.is_some() {
        return Ok(payload.body.clone());
    }
    match payload.body_artifact_path.as_deref() {
        Some(path) => load_body_artifact(shore_dir, path),
        None => Ok(None),
    }
}

fn build_snapshot_rows(
    snapshot: &DiffSnapshot,
    review_unit_id: &ReviewUnitId,
) -> (Vec<ReviewUnitProjectionRow>, ReviewUnitProjectionSummary) {
    let mut rows = Vec::new();

    if snapshot.files.is_empty() {
        rows.push(snapshot_row(
            rows.len(),
            ReviewUnitProjectionRowKind::EmptyState,
            None,
            ProjectionCoverage::Context,
            None,
            None,
            None,
        ));
    }

    for (file_index, file) in snapshot.files.iter().enumerate() {
        let file_path = snapshot_file_path(file);
        let old_path = file.old_path.clone();
        let file_target = file_path.as_ref().map(|file_path| ReviewTargetRef::File {
            review_unit_id: review_unit_id.clone(),
            file_path: file_path.clone(),
        });
        rows.push(snapshot_row(
            rows.len(),
            ReviewUnitProjectionRowKind::FileHeader,
            Some(SnapshotOrder {
                file_index,
                metadata_index: None,
                hunk_index: None,
                row_index: None,
            }),
            ProjectionCoverage::Unreviewed,
            file_target,
            file_path.clone(),
            old_path.clone(),
        ));

        for (metadata_index, _) in file.metadata_rows.iter().enumerate() {
            rows.push(snapshot_row(
                rows.len(),
                ReviewUnitProjectionRowKind::Metadata,
                Some(SnapshotOrder {
                    file_index,
                    metadata_index: Some(metadata_index),
                    hunk_index: None,
                    row_index: None,
                }),
                ProjectionCoverage::Unreviewed,
                None,
                file_path.clone(),
                old_path.clone(),
            ));
        }

        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            rows.push(snapshot_row(
                rows.len(),
                ReviewUnitProjectionRowKind::HunkHeader,
                Some(SnapshotOrder {
                    file_index,
                    metadata_index: None,
                    hunk_index: Some(hunk_index),
                    row_index: None,
                }),
                ProjectionCoverage::Unreviewed,
                None,
                file_path.clone(),
                old_path.clone(),
            ));

            for (row_index, _) in hunk.rows.iter().enumerate() {
                rows.push(snapshot_row(
                    rows.len(),
                    ReviewUnitProjectionRowKind::Diff,
                    Some(SnapshotOrder {
                        file_index,
                        metadata_index: None,
                        hunk_index: Some(hunk_index),
                        row_index: Some(row_index),
                    }),
                    ProjectionCoverage::Unreviewed,
                    None,
                    file_path.clone(),
                    old_path.clone(),
                ));
            }
        }
    }

    let summary = ReviewUnitProjectionSummary {
        file_count: snapshot.files.len(),
        row_count: rows.len(),
        snapshot_row_count: rows.len(),
        snapshot_remainder_row_count: rows.len(),
        ..ReviewUnitProjectionSummary::default()
    };

    (rows, summary)
}

fn build_observation_rows(observations: &[ObservationView]) -> Vec<ReviewUnitProjectionRow> {
    observations
        .iter()
        .enumerate()
        .map(|(index, observation)| {
            let (file_path, old_path) = target_paths(&observation.target);
            ReviewUnitProjectionRow {
                id: RowId::new(format!("row:{index:06}")),
                kind: ReviewUnitProjectionRowKind::Observation,
                projection_phase: ProjectionPhase::Narrative,
                projection_order: index,
                snapshot_order: None,
                coverage: ProjectionCoverage::Reviewed,
                target: Some(observation.target.clone()),
                file_path,
                old_path,
                related_observation_ids: vec![observation.id.clone()],
                related_intervention_ids: Vec::new(),
                related_disposition_ids: Vec::new(),
            }
        })
        .collect()
}

fn build_intervention_rows(interventions: &[InterventionView]) -> Vec<ReviewUnitProjectionRow> {
    interventions
        .iter()
        .enumerate()
        .map(|(index, intervention)| {
            let (file_path, old_path) = target_paths(&intervention.target);
            ReviewUnitProjectionRow {
                id: RowId::new(format!("row:{index:06}")),
                kind: ReviewUnitProjectionRowKind::Intervention,
                projection_phase: ProjectionPhase::Narrative,
                projection_order: index,
                snapshot_order: None,
                coverage: ProjectionCoverage::Reviewed,
                target: Some(intervention.target.clone()),
                file_path,
                old_path,
                related_observation_ids: Vec::new(),
                related_intervention_ids: vec![intervention.id.clone()],
                related_disposition_ids: Vec::new(),
            }
        })
        .collect()
}

fn build_disposition_rows(dispositions: &[DispositionView]) -> Vec<ReviewUnitProjectionRow> {
    dispositions
        .iter()
        .enumerate()
        .map(|(index, disposition)| {
            let (file_path, old_path) = target_paths(&disposition.target);
            ReviewUnitProjectionRow {
                id: RowId::new(format!("row:{index:06}")),
                kind: ReviewUnitProjectionRowKind::Disposition,
                projection_phase: ProjectionPhase::Narrative,
                projection_order: index,
                snapshot_order: None,
                coverage: ProjectionCoverage::Reviewed,
                target: Some(disposition.target.clone()),
                file_path,
                old_path,
                related_observation_ids: disposition.related_observations.clone(),
                related_intervention_ids: disposition.related_interventions.clone(),
                related_disposition_ids: vec![disposition.id.clone()],
            }
        })
        .collect()
}

fn build_adapter_note_rows(
    adapter_notes: &[AdapterNoteView],
    review_unit_id: &ReviewUnitId,
) -> Vec<ReviewUnitProjectionRow> {
    adapter_notes
        .iter()
        .enumerate()
        .map(|(index, note)| {
            let target = note.target.as_ref().map(|target| ReviewTargetRef::Range {
                review_unit_id: review_unit_id.clone(),
                file_path: note.file_path.clone(),
                side: target.side,
                start_line: target.start_line,
                end_line: target.end_line,
            });
            ReviewUnitProjectionRow {
                id: RowId::new(format!("row:{index:06}")),
                kind: ReviewUnitProjectionRowKind::AdapterNote,
                projection_phase: ProjectionPhase::Narrative,
                projection_order: index,
                snapshot_order: None,
                coverage: ProjectionCoverage::Reviewed,
                target,
                file_path: Some(note.file_path.clone()),
                old_path: note.file_old_path.clone(),
                related_observation_ids: Vec::new(),
                related_intervention_ids: Vec::new(),
                related_disposition_ids: Vec::new(),
            }
        })
        .collect()
}

fn snapshot_row(
    projection_order: usize,
    kind: ReviewUnitProjectionRowKind,
    snapshot_order: Option<SnapshotOrder>,
    coverage: ProjectionCoverage,
    target: Option<ReviewTargetRef>,
    file_path: Option<String>,
    old_path: Option<String>,
) -> ReviewUnitProjectionRow {
    ReviewUnitProjectionRow {
        id: RowId::new(format!("row:{projection_order:06}")),
        kind,
        projection_phase: ProjectionPhase::SnapshotRemainder,
        projection_order,
        snapshot_order,
        coverage,
        target,
        file_path,
        old_path,
        related_observation_ids: Vec::new(),
        related_intervention_ids: Vec::new(),
        related_disposition_ids: Vec::new(),
    }
}

fn renumber_projection_rows(rows: &mut [ReviewUnitProjectionRow]) {
    for (index, row) in rows.iter_mut().enumerate() {
        row.id = RowId::new(format!("row:{index:06}"));
        row.projection_order = index;
    }
}

fn snapshot_file_path(file: &DiffFile) -> Option<String> {
    file.new_path.clone().or_else(|| file.old_path.clone())
}

fn target_paths(target: &ReviewTargetRef) -> (Option<String>, Option<String>) {
    match target {
        ReviewTargetRef::File { file_path, .. } | ReviewTargetRef::Range { file_path, .. } => {
            (Some(file_path.clone()), None)
        }
        ReviewTargetRef::ReviewUnit { .. }
        | ReviewTargetRef::Observation { .. }
        | ReviewTargetRef::Intervention { .. }
        | ReviewTargetRef::Disposition { .. }
        | ReviewTargetRef::Event { .. } => (None, None),
    }
}

fn selected_review_unit_capture(
    events: &[ShoreEvent],
    resolved: &ResolvedReviewUnit,
) -> Result<ReviewUnitProjectionIdentity> {
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewUnitCaptured)
    {
        let payload: ReviewUnitCapturedPayload = serde_json::from_value(event.payload.clone())?;
        if payload.review_unit_id == resolved.review_unit_id {
            return Ok(ReviewUnitProjectionIdentity {
                id: payload.review_unit_id,
                review_id: event.target.review_id.clone(),
                source: payload.source,
                base: payload.base,
                target: payload.target,
                revision_id: payload.revision_id,
                snapshot_id: payload.snapshot_id,
                snapshot_artifact_content_hash: payload.snapshot_artifact_content_hash,
            });
        }
    }

    Err(ShoreError::Message(format!(
        "captured review unit event missing for {}",
        resolved.review_unit_id.as_str()
    )))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::*;
    use crate::canonical_hash::sha256_json_prefixed;
    use crate::model::{DiffSnapshot, ReviewId, ReviewUnitId, SnapshotId};
    use crate::session::{
        CaptureOptions, CurrentDispositionStatus, DispositionAddOptions, DispositionShowOptions,
        ImportNotesOptions, InterventionListOptions, InterventionReasonCode,
        InterventionRequestOptions, InterventionResolutionOutcome, InterventionResolveOptions,
        InterventionStatus, InterventionStatusFilter, ObservationAddOptions,
        ObservationListOptions, ObservationTargetSelector, ReviewDisposition,
        capture_worktree_review, import_notes, list_interventions, list_observations,
        record_disposition, record_observation, request_intervention, resolve_intervention,
        show_dispositions,
    };

    #[test]
    fn show_review_unit_errors_when_no_review_unit_is_captured() {
        let repo = modified_repo();

        let error = show_review_unit(ReviewUnitShowOptions::new(repo.path()))
            .expect_err("no captured ReviewUnit should fail");

        assert!(error.to_string().contains("no captured review unit"));
    }

    #[test]
    fn show_review_unit_resolves_single_current_review_unit_and_freshness() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = show_review_unit(ReviewUnitShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.review_unit.id, capture.review_unit_id);
        assert_eq!(result.review_unit.revision_id, capture.revision_id);
        assert_eq!(result.review_unit.snapshot_id, capture.snapshot_id);
        assert_eq!(result.filters.review_unit_id, capture.review_unit_id);
        assert_eq!(result.event_count, 1);
        assert!(result.event_set_hash.starts_with("sha256:"));
    }

    #[test]
    fn show_review_unit_requires_explicit_id_when_current_is_ambiguous() {
        let repo = modified_repo();
        let first = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
        let second = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let error = show_review_unit(ReviewUnitShowOptions::new(repo.path()))
            .expect_err("multiple captures should be ambiguous");
        assert!(error.to_string().contains("multiple captured review units"));

        let explicit = show_review_unit(
            ReviewUnitShowOptions::new(repo.path())
                .with_review_unit_id(first.review_unit_id.clone()),
        )
        .unwrap();

        assert_ne!(first.review_unit_id, second.review_unit_id);
        assert_eq!(explicit.review_unit.id, first.review_unit_id);
        assert_eq!(explicit.event_count, 2);
    }

    #[test]
    fn show_review_unit_uses_captured_snapshot_after_worktree_drift() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 99 }\n");

        let result = show_review_unit(ReviewUnitShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.review_unit.id, capture.review_unit_id);
        assert_eq!(
            result.snapshot.files[0].new_path.as_deref(),
            Some("src/lib.rs")
        );
        assert!(format!("{:?}", result.snapshot).contains("2"));
        assert!(!format!("{:?}", result.snapshot).contains("99"));
    }

    #[test]
    fn show_review_unit_rejects_snapshot_artifact_hash_mismatch() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        tamper_snapshot_artifact_target(repo.path(), &capture.snapshot_id, "/other/repo");

        let error = show_review_unit(ReviewUnitShowOptions::new(repo.path()))
            .expect_err("tampered artifact should fail");

        assert!(error.to_string().contains("content hash"));
    }

    #[test]
    fn show_review_unit_rejects_snapshot_artifact_metadata_mismatch() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        tamper_snapshot_artifact_target_and_rehash(
            repo.path(),
            &capture.snapshot_id,
            "/other/repo",
        );

        let error = show_review_unit(ReviewUnitShowOptions::new(repo.path()))
            .expect_err("metadata mismatch should fail");

        assert!(
            error
                .to_string()
                .contains("snapshot artifact metadata mismatch")
        );
    }

    #[test]
    fn show_review_unit_rejects_event_artifact_binding_mismatch() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        rewrite_capture_event_snapshot_artifact_hash(
            repo.path(),
            &capture.review_unit_id,
            "sha256:bad",
        );

        let error = show_review_unit(ReviewUnitShowOptions::new(repo.path()))
            .expect_err("event/artifact mismatch should fail");

        assert!(error.to_string().contains("snapshot artifact content hash"));
    }

    #[test]
    fn show_review_unit_emits_snapshot_rows_in_captured_order() {
        let repo = multi_file_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = show_review_unit(ReviewUnitShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.rows[0].kind.as_str(), "file_header");
        assert_eq!(
            result.rows[0].projection_phase.as_str(),
            "snapshot_remainder"
        );
        assert_eq!(result.rows[0].coverage.as_str(), "unreviewed");
        assert_eq!(result.rows[0].projection_order, 0);
        assert_eq!(
            result.rows[0].snapshot_order.as_ref().unwrap().file_index,
            0
        );
        assert!(result.rows.iter().any(|row| row.kind.as_str() == "diff"));
    }

    #[test]
    fn show_review_unit_emits_empty_state_row_for_empty_snapshot() {
        let (rows, summary) = build_snapshot_rows(
            &DiffSnapshot::new(
                ReviewId::new("review:empty"),
                SnapshotId::new("snap:empty"),
                Vec::new(),
            ),
            &ReviewUnitId::new("review-unit:empty"),
        );

        assert_eq!(summary.file_count, 0);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind.as_str(), "empty_state");
    }

    #[test]
    fn show_review_unit_rows_do_not_expose_storage_paths() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = show_review_unit(ReviewUnitShowOptions::new(repo.path())).unwrap();
        let debug = format!("{result:?}");

        assert!(!debug.contains("artifacts/snapshots"));
        assert!(!debug.contains(".shore/events"));
    }

    #[test]
    fn show_review_unit_includes_active_observations() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Check this")
                .with_body("Observation body"),
        )
        .unwrap();

        let result = show_review_unit(ReviewUnitShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.observations.len(), 1);
        assert_eq!(result.observations[0].title, "Check this");
        assert_eq!(result.observations[0].body, None);
        assert_eq!(result.summary.observation_count, 1);
        assert!(
            result
                .rows
                .iter()
                .any(|row| row.kind.as_str() == "observation")
        );
    }

    #[test]
    fn show_review_unit_hydrates_observation_bodies_when_requested() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Body")
                .with_body("Observation body"),
        )
        .unwrap();

        let result =
            show_review_unit(ReviewUnitShowOptions::new(repo.path()).with_include_body(true))
                .unwrap();

        assert_eq!(
            result.observations[0].body.as_deref(),
            Some("Observation body")
        );
        assert!(!format!("{result:?}").contains("artifacts/notes/"));
    }

    #[test]
    fn show_review_unit_observations_match_list_semantics_for_duplicates_and_supersession() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        add_duplicate_observations_with_distinct_idempotency_keys(&repo);
        add_superseding_observation(&repo);

        let unit = show_review_unit(ReviewUnitShowOptions::new(repo.path())).unwrap();
        let list = list_observations(ObservationListOptions::new(repo.path())).unwrap();

        assert_eq!(unit.observations, list.observations);
        assert_eq!(unit.diagnostics, list.diagnostics);
    }

    #[test]
    fn show_review_unit_includes_open_and_resolved_interventions() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let request = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Need decision")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired),
        )
        .unwrap();
        resolve_intervention(
            InterventionResolveOptions::new(repo.path(), request.intervention_id.clone())
                .with_outcome(InterventionResolutionOutcome::Approved)
                .with_reason("ok"),
        )
        .unwrap();

        let unit = show_review_unit(ReviewUnitShowOptions::new(repo.path())).unwrap();

        assert_eq!(unit.interventions.len(), 1);
        assert_eq!(unit.interventions[0].id, request.intervention_id);
        assert_eq!(unit.interventions[0].status, InterventionStatus::Resolved);
        assert_eq!(unit.summary.intervention_count, 1);
        assert!(
            unit.rows
                .iter()
                .any(|row| row.kind.as_str() == "intervention")
        );
    }

    #[test]
    fn show_review_unit_interventions_match_list_semantics() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        add_duplicate_intervention_requests(&repo);
        add_ambiguous_intervention_resolutions(&repo);

        let unit = show_review_unit(ReviewUnitShowOptions::new(repo.path())).unwrap();
        let list = list_interventions(
            InterventionListOptions::new(repo.path()).with_status(InterventionStatusFilter::All),
        )
        .unwrap();

        assert_eq!(unit.interventions, list.interventions);
        assert_eq!(unit.diagnostics, list.diagnostics);
    }

    #[test]
    fn show_review_unit_includes_current_disposition() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let disposition = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Accepted)
                .with_summary("ship it"),
        )
        .unwrap();

        let unit = show_review_unit(ReviewUnitShowOptions::new(repo.path())).unwrap();

        assert_eq!(
            unit.current_disposition.status,
            CurrentDispositionStatus::Resolved
        );
        assert_eq!(unit.dispositions.len(), 1);
        assert_eq!(unit.dispositions[0].id, disposition.disposition_id);
        assert_eq!(unit.summary.disposition_count, 1);
        assert!(
            unit.rows
                .iter()
                .any(|row| row.kind.as_str() == "disposition")
        );
    }

    #[test]
    fn show_review_unit_dispositions_match_show_semantics() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        add_replaced_and_duplicate_dispositions(&repo);

        let unit =
            show_review_unit(ReviewUnitShowOptions::new(repo.path()).with_include_body(true))
                .unwrap();
        let show = show_dispositions(
            DispositionShowOptions::new(repo.path())
                .with_include_summary(true)
                .with_all(true),
        )
        .unwrap();

        assert_eq!(unit.current_disposition, show.current);
        assert_eq!(unit.dispositions, show.dispositions);
        assert_eq!(unit.diagnostics, show.diagnostics);
    }

    #[test]
    fn show_review_unit_includes_imported_adapter_notes() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let notes_path = repo.write_fixture("review-notes.json", native_review_notes_json());
        import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(notes_path)).unwrap();

        let result = show_review_unit(ReviewUnitShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.adapter_notes.len(), 1);
        assert_eq!(result.adapter_notes[0].title, "Imported note");
        assert_eq!(result.summary.adapter_note_count, 1);
        assert!(
            result
                .rows
                .iter()
                .any(|row| row.kind.as_str() == "adapter_note")
        );
    }

    #[test]
    fn show_review_unit_adapter_notes_hydrate_body_only_when_requested() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        import_large_review_note_body(&repo);

        let compact = show_review_unit(ReviewUnitShowOptions::new(repo.path())).unwrap();
        let hydrated =
            show_review_unit(ReviewUnitShowOptions::new(repo.path()).with_include_body(true))
                .unwrap();

        assert_eq!(compact.adapter_notes[0].body, None);
        assert_eq!(
            hydrated.adapter_notes[0].body.as_deref(),
            Some("large imported body")
        );
        assert!(!format!("{hydrated:?}").contains("artifacts/notes/"));
    }

    #[test]
    fn show_review_unit_adapter_notes_surface_stale_and_orphan_status() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        import_stale_and_orphan_review_notes(&repo);

        let result = show_review_unit(ReviewUnitShowOptions::new(repo.path())).unwrap();

        assert!(
            result
                .adapter_notes
                .iter()
                .any(|note| note.status.as_str() == "stale")
        );
        assert!(
            result
                .adapter_notes
                .iter()
                .any(|note| note.status.as_str() == "orphaned")
        );
    }

    #[test]
    fn adapter_note_status_preserves_resolution_detail() {
        assert_eq!(
            adapter_note_status(&ResolutionStatus::Exact).as_str(),
            "exact"
        );
        assert_eq!(
            adapter_note_status(&ResolutionStatus::Relocated).as_str(),
            "relocated"
        );
        assert_eq!(
            adapter_note_status(&ResolutionStatus::FileLevel).as_str(),
            "file_level"
        );
        assert_eq!(
            adapter_note_status(&ResolutionStatus::Stale).as_str(),
            "stale"
        );
        assert_eq!(
            adapter_note_status(&ResolutionStatus::Orphaned).as_str(),
            "orphaned"
        );
        assert_eq!(
            adapter_note_status(&ResolutionStatus::Unresolved).as_str(),
            "unresolved"
        );
    }

    #[test]
    fn show_review_unit_places_reviewed_material_before_snapshot_remainder() {
        let repo = multi_hunk_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Important")
                .with_target(ObservationTargetSelector::file("src/lib.rs")),
        )
        .unwrap();

        let result = show_review_unit(ReviewUnitShowOptions::new(repo.path())).unwrap();

        let first_snapshot_remainder = result
            .rows
            .iter()
            .position(|row| row.projection_phase.as_str() == "snapshot_remainder")
            .unwrap();
        let observation_row = result
            .rows
            .iter()
            .position(|row| row.kind.as_str() == "observation")
            .unwrap();

        assert!(observation_row < first_snapshot_remainder);
        assert_eq!(result.summary.narrative_row_count, first_snapshot_remainder);
        assert!(result.summary.snapshot_remainder_row_count > 0);
    }

    #[test]
    fn show_review_unit_keeps_unreviewed_snapshot_rows_complete() {
        let repo = multi_file_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Review wide"),
        )
        .unwrap();

        let result = show_review_unit(ReviewUnitShowOptions::new(repo.path())).unwrap();

        let snapshot_row_count = result
            .rows
            .iter()
            .filter(|row| row.snapshot_order.is_some())
            .count();
        assert_eq!(snapshot_row_count, result.summary.snapshot_row_count);
        assert!(
            result
                .rows
                .iter()
                .any(|row| row.coverage.as_str() == "unreviewed")
        );
    }

    #[test]
    fn show_review_unit_track_filter_narrows_narrative_without_mutating_snapshot_remainder() {
        let repo = multi_file_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        add_observation(&repo, "agent:codex", "Codex");
        add_observation(&repo, "agent:claude", "Claude");

        let all = show_review_unit(ReviewUnitShowOptions::new(repo.path())).unwrap();
        let codex =
            show_review_unit(ReviewUnitShowOptions::new(repo.path()).with_track("agent:codex"))
                .unwrap();

        assert!(all.summary.narrative_row_count > codex.summary.narrative_row_count);
        assert_eq!(
            all.summary.snapshot_remainder_row_count,
            codex.summary.snapshot_remainder_row_count
        );
        assert!(
            codex
                .observations
                .iter()
                .all(|obs| obs.track_id.as_str() == "agent:codex")
        );
    }

    fn modified_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        repo
    }

    fn multi_file_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.write("src/other.rs", "pub fn other() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        repo.write("src/other.rs", "pub fn other() -> u32 { 2 }\n");
        repo
    }

    fn multi_hunk_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write(
            "src/lib.rs",
            (1..=30)
                .map(|line| format!("pub fn value_{line}() -> u32 {{ {line} }}\n"))
                .collect::<String>(),
        );
        repo.commit_all("base");
        repo.write(
            "src/lib.rs",
            (1..=30)
                .map(|line| {
                    let value = if line == 2 || line == 28 {
                        line + 100
                    } else {
                        line
                    };
                    format!("pub fn value_{line}() -> u32 {{ {value} }}\n")
                })
                .collect::<String>(),
        );
        repo
    }

    fn add_observation(repo: &TestRepo, track: &str, title: &str) {
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track(track)
                .with_title(title),
        )
        .unwrap();
    }

    fn add_duplicate_observations_with_distinct_idempotency_keys(repo: &TestRepo) {
        let first = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Same finding")
                .with_body("same body")
                .with_idempotency_key("retry-a"),
        )
        .unwrap();
        let second = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Same finding")
                .with_body("same body")
                .with_idempotency_key("retry-b"),
        )
        .unwrap();

        assert_eq!(first.observation_id, second.observation_id);
    }

    fn add_superseding_observation(repo: &TestRepo) {
        let original = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Original"),
        )
        .unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Correction")
                .superseding(original.observation_id),
        )
        .unwrap();
    }

    fn add_duplicate_intervention_requests(repo: &TestRepo) {
        let first = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Same decision")
                .with_body("same body")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired)
                .with_idempotency_key("intervention-retry-a"),
        )
        .unwrap();
        let second = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Same decision")
                .with_body("same body")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired)
                .with_idempotency_key("intervention-retry-b"),
        )
        .unwrap();

        assert_eq!(first.intervention_id, second.intervention_id);
    }

    fn add_ambiguous_intervention_resolutions(repo: &TestRepo) {
        let request = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Ambiguous")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired),
        )
        .unwrap();
        resolve_intervention(
            InterventionResolveOptions::new(repo.path(), request.intervention_id.clone())
                .with_outcome(InterventionResolutionOutcome::Approved),
        )
        .unwrap();
        resolve_intervention(
            InterventionResolveOptions::new(repo.path(), request.intervention_id)
                .with_outcome(InterventionResolutionOutcome::Rejected),
        )
        .unwrap();
    }

    fn add_replaced_and_duplicate_dispositions(repo: &TestRepo) {
        let duplicate_options = DispositionAddOptions::new(repo.path())
            .with_track("human:kevin")
            .with_disposition(ReviewDisposition::NeedsClarification)
            .with_summary("same summary");
        let first = record_disposition(
            duplicate_options
                .clone()
                .with_idempotency_key("disposition-retry-a"),
        )
        .unwrap();
        let second =
            record_disposition(duplicate_options.with_idempotency_key("disposition-retry-b"))
                .unwrap();

        assert_eq!(first.disposition_id, second.disposition_id);

        record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::AcceptedWithFollowUp)
                .with_summary("replacement")
                .replacing(first.disposition_id),
        )
        .unwrap();
    }

    fn import_large_review_note_body(repo: &TestRepo) {
        let path = repo.write_fixture(
            "large-review-notes.json",
            review_notes_json_with_notes(
                "src/lib.rs",
                vec![review_note_json(
                    "large",
                    "Large imported note",
                    "large imported body",
                    "new",
                    1,
                    1,
                )],
            ),
        );
        import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(path)).unwrap();
    }

    fn import_stale_and_orphan_review_notes(repo: &TestRepo) {
        let path = repo.write_fixture(
            "stale-orphan-review-notes.json",
            format!(
                r#"{{
  "schema": "shore.review-notes",
  "version": 1,
  "files": [
    {{
      "path": "src/lib.rs",
      "notes": [
        {}
      ]
    }},
    {{
      "path": "src/gone.rs",
      "notes": [
        {}
      ]
    }}
  ]
}}"#,
                review_note_json("stale", "Stale imported note", "stale", "new", 99, 99),
                review_note_json("orphan", "Orphan imported note", "orphan", "new", 1, 1)
            ),
        );
        import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(path)).unwrap();
    }

    fn native_review_notes_json() -> String {
        review_notes_json_with_notes(
            "src/lib.rs",
            vec![review_note_json(
                "imported",
                "Imported note",
                "Imported body",
                "new",
                1,
                1,
            )],
        )
    }

    fn review_notes_json_with_notes(path: &str, notes: Vec<String>) -> String {
        format!(
            r#"{{
  "schema": "shore.review-notes",
  "version": 1,
  "files": [
    {{
      "path": "{path}",
      "notes": [
        {}
      ]
    }}
  ]
}}"#,
            notes.join(",\n        ")
        )
    }

    fn review_note_json(
        id: &str,
        title: &str,
        body: &str,
        side: &str,
        start_line: u32,
        end_line: u32,
    ) -> String {
        format!(
            r#"{{
          "id": "{id}",
          "title": "{title}",
          "body": "{body}",
          "target": {{
            "side": "{side}",
            "startLine": {start_line},
            "endLine": {end_line}
          }},
          "tags": ["fixture"],
          "confidence": "high",
          "source": "review-notes.json",
          "author": "codex",
          "createdAt": "2026-05-13T00:00:00Z"
        }}"#
        )
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

        fn write_fixture(&self, path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> PathBuf {
            let path = self.root.path().join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directories");
            }
            fs::write(&path, contents).expect("write test fixture");
            path
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
                "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
                args,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    fn tamper_snapshot_artifact_target(repo: &Path, snapshot_id: &SnapshotId, target_root: &str) {
        let path = snapshot_artifact_path(repo, snapshot_id);
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("read snapshot artifact"))
                .expect("parse snapshot artifact json");

        assert_eq!(json["snapshot"]["snapshot_id"], snapshot_id.as_str());
        json["target"]["worktreeRoot"] = target_root.into();

        fs::write(
            &path,
            serde_json::to_vec_pretty(&json).expect("serialize tampered snapshot artifact"),
        )
        .expect("write tampered snapshot artifact");
    }

    fn tamper_snapshot_artifact_target_and_rehash(
        repo: &Path,
        snapshot_id: &SnapshotId,
        target_root: &str,
    ) {
        let path = snapshot_artifact_path(repo, snapshot_id);
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("read snapshot artifact"))
                .expect("parse snapshot artifact json");

        assert_eq!(json["snapshot"]["snapshot_id"], snapshot_id.as_str());
        json["target"]["worktreeRoot"] = target_root.into();
        json["contentHash"] = snapshot_artifact_hash_from_json(&json).into();

        fs::write(
            &path,
            serde_json::to_vec_pretty(&json).expect("serialize tampered snapshot artifact"),
        )
        .expect("write tampered snapshot artifact");
    }

    fn snapshot_artifact_hash_from_json(json: &serde_json::Value) -> String {
        let mut material = json.clone();
        material
            .as_object_mut()
            .expect("snapshot artifact is an object")
            .remove("contentHash")
            .expect("snapshot artifact has contentHash");
        sha256_json_prefixed(&material).expect("hash snapshot artifact material")
    }

    fn rewrite_capture_event_snapshot_artifact_hash(
        repo: &Path,
        review_unit_id: &ReviewUnitId,
        hash: &str,
    ) {
        let path = capture_event_path(repo, review_unit_id);
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("read capture event"))
                .expect("parse capture event json");

        json["payload"]["snapshotArtifactContentHash"] = hash.into();
        json["payloadHash"] = sha256_json_prefixed(&json["payload"])
            .expect("hash rewritten capture event payload")
            .into();

        fs::write(
            &path,
            serde_json::to_vec_pretty(&json).expect("serialize rewritten capture event"),
        )
        .expect("write rewritten capture event");
    }

    fn snapshot_artifact_path(repo: &Path, snapshot_id: &SnapshotId) -> PathBuf {
        fs::read_dir(repo.join(".shore/artifacts/snapshots"))
            .expect("read snapshot artifacts directory")
            .map(|entry| entry.expect("read snapshot artifact dir entry").path())
            .find(|path| {
                let Ok(bytes) = fs::read(path) else {
                    return false;
                };
                let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                    return false;
                };
                json["snapshot"]["snapshot_id"] == snapshot_id.as_str()
            })
            .expect("find snapshot artifact")
    }

    fn capture_event_path(repo: &Path, review_unit_id: &ReviewUnitId) -> PathBuf {
        fs::read_dir(repo.join(".shore/events"))
            .expect("read events directory")
            .map(|entry| entry.expect("read event dir entry").path())
            .find(|path| {
                let Ok(bytes) = fs::read(path) else {
                    return false;
                };
                let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                    return false;
                };
                json["eventType"] == "review_unit_captured"
                    && json["payload"]["reviewUnitId"] == review_unit_id.as_str()
            })
            .expect("find capture event")
    }
}
