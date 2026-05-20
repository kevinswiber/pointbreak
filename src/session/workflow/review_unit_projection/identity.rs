use std::path::{Path, PathBuf};

use super::{AdapterNoteView, ReviewUnitProjectionRow};
use crate::model::{
    DiffSnapshot, ReviewEndpoint, ReviewUnitId, ReviewUnitSource, RevisionId, SessionId,
    SnapshotId, TrackId,
};
use crate::session::assessment::{AssessmentView, CurrentAssessmentView};
use crate::session::intervention::InterventionView;
use crate::session::observation::ObservationView;
use crate::session::state::ProjectionDiagnostic;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewUnitShowOptions {
    pub(super) repo: PathBuf,
    pub(super) review_unit_id: Option<ReviewUnitId>,
    pub(super) track: Option<String>,
    pub(super) include_body: bool,
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
    pub current_assessment: CurrentAssessmentView,
    pub observations: Vec<ObservationView>,
    pub interventions: Vec<InterventionView>,
    pub assessments: Vec<AssessmentView>,
    pub adapter_notes: Vec<AdapterNoteView>,
    pub rows: Vec<ReviewUnitProjectionRow>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewUnitProjectionIdentity {
    pub id: ReviewUnitId,
    pub session_id: SessionId,
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
    pub assessment_count: usize,
    pub adapter_note_count: usize,
}
