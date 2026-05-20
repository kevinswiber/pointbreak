use super::ReviewUnitProjectionSummary;
use super::adapter_notes::AdapterNoteView;
use crate::model::{
    AssessmentId, DiffFile, DiffSnapshot, InputRequestId, ObservationId, ReviewTargetRef,
    ReviewUnitId, RowId,
};
use crate::session::assessment::AssessmentView;
use crate::session::input_request::InputRequestView;
use crate::session::observation::ObservationView;

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
    pub related_intervention_ids: Vec<InputRequestId>,
    pub related_assessment_ids: Vec<AssessmentId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewUnitProjectionRowKind {
    FileHeader,
    Metadata,
    HunkHeader,
    Diff,
    Observation,
    Intervention,
    Assessment,
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
            Self::Assessment => "assessment",
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

pub(super) fn build_snapshot_rows(
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

pub(super) fn build_observation_rows(
    observations: &[ObservationView],
) -> Vec<ReviewUnitProjectionRow> {
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
                related_assessment_ids: Vec::new(),
            }
        })
        .collect()
}

pub(super) fn build_intervention_rows(
    interventions: &[InputRequestView],
) -> Vec<ReviewUnitProjectionRow> {
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
                related_assessment_ids: Vec::new(),
            }
        })
        .collect()
}

pub(super) fn build_assessment_rows(
    assessments: &[AssessmentView],
) -> Vec<ReviewUnitProjectionRow> {
    assessments
        .iter()
        .enumerate()
        .map(|(index, assessment)| {
            let (file_path, old_path) = target_paths(&assessment.target);
            ReviewUnitProjectionRow {
                id: RowId::new(format!("row:{index:06}")),
                kind: ReviewUnitProjectionRowKind::Assessment,
                projection_phase: ProjectionPhase::Narrative,
                projection_order: index,
                snapshot_order: None,
                coverage: ProjectionCoverage::Reviewed,
                target: Some(assessment.target.clone()),
                file_path,
                old_path,
                related_observation_ids: assessment.related_observations.clone(),
                related_intervention_ids: assessment.related_input_requests.clone(),
                related_assessment_ids: vec![assessment.id.clone()],
            }
        })
        .collect()
}

pub(super) fn build_adapter_note_rows(
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
                related_assessment_ids: Vec::new(),
            }
        })
        .collect()
}

pub(super) fn snapshot_row(
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
        related_assessment_ids: Vec::new(),
    }
}

pub(super) fn renumber_projection_rows(rows: &mut [ReviewUnitProjectionRow]) {
    for (index, row) in rows.iter_mut().enumerate() {
        row.id = RowId::new(format!("row:{index:06}"));
        row.projection_order = index;
    }
}

pub(super) fn snapshot_file_path(file: &DiffFile) -> Option<String> {
    file.new_path.clone().or_else(|| file.old_path.clone())
}

pub(super) fn target_paths(target: &ReviewTargetRef) -> (Option<String>, Option<String>) {
    match target {
        ReviewTargetRef::File { file_path, .. } | ReviewTargetRef::Range { file_path, .. } => {
            (Some(file_path.clone()), None)
        }
        ReviewTargetRef::ReviewUnit { .. }
        | ReviewTargetRef::Observation { .. }
        | ReviewTargetRef::InputRequest { .. }
        | ReviewTargetRef::Assessment { .. }
        | ReviewTargetRef::Event { .. } => (None, None),
    }
}
