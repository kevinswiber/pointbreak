use std::path::{Path, PathBuf};

use super::{
    AssessmentProjectionOptions, AssessmentView, CurrentAssessmentView, project_assessments,
};
use crate::error::Result;
use crate::model::{ReviewUnitLineageId, RevisionId, TrackId};
use crate::session::EventStore;
use crate::session::observation::{
    CurrentReviewUnitContext, ReviewUnitScope, ReviewUnitSelection, resolve_review_unit,
    validated_track_id,
};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store::resolution::resolve_read_store;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssessmentShowOptions {
    pub(super) repo: PathBuf,
    pub(super) review_unit_id: Option<RevisionId>,
    pub(super) lineage_id: Option<ReviewUnitLineageId>,
    pub(super) track: Option<String>,
    pub(super) include_summary: bool,
    pub(super) include_all: bool,
}

impl AssessmentShowOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_unit_id: None,
            lineage_id: None,
            track: None,
            include_summary: false,
            include_all: false,
        }
    }

    pub fn with_review_unit_id(mut self, id: RevisionId) -> Self {
        self.review_unit_id = Some(id);
        self
    }

    pub fn with_lineage_id(mut self, id: ReviewUnitLineageId) -> Self {
        self.lineage_id = Some(id);
        self
    }

    pub fn with_track(mut self, track: impl Into<String>) -> Self {
        self.track = Some(track.into());
        self
    }

    pub fn with_include_summary(mut self, include_summary: bool) -> Self {
        self.include_summary = include_summary;
        self
    }

    pub fn with_all(mut self, include_all: bool) -> Self {
        self.include_all = include_all;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssessmentShowResult {
    pub review_unit_id: RevisionId,
    pub filters: AssessmentShowFilters,
    pub current: CurrentAssessmentView,
    pub assessments: Vec<AssessmentView>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssessmentShowFilters {
    pub track_id: Option<TrackId>,
    pub include_summary: bool,
    pub include_all: bool,
}

pub fn show_assessments(options: AssessmentShowOptions) -> Result<AssessmentShowResult> {
    let read_store = resolve_read_store(&options.repo)?;
    let store_dir = read_store.store_dir();
    let events = EventStore::open(store_dir).list_events()?;
    let resolved = resolve_review_unit(
        &events,
        ReviewUnitSelection::from_review_unit_or_lineage(
            options.review_unit_id.as_ref(),
            options.lineage_id.as_ref(),
        )?,
        &CurrentReviewUnitContext::for_repo(&options.repo)?,
        ReviewUnitScope::default(),
    )?;
    let track_filter = options
        .track
        .as_deref()
        .map(validated_track_id)
        .transpose()?;
    let (current, assessments) = project_assessments(AssessmentProjectionOptions {
        store_dir,
        events: &events,
        resolved: &resolved,
        track_filter: track_filter.clone(),
        include_summary: options.include_summary,
        include_all: options.include_all,
    })?;
    let diagnostics = SessionState::from_events(&events)?.diagnostics;

    Ok(AssessmentShowResult {
        review_unit_id: resolved.revision_id,
        filters: AssessmentShowFilters {
            track_id: track_filter,
            include_summary: options.include_summary,
            include_all: options.include_all,
        },
        current,
        assessments,
        diagnostics,
    })
}
