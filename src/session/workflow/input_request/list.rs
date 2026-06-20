use std::path::{Path, PathBuf};

use super::view::{
    InputRequestProjectionOptions, InputRequestStatusFilter, InputRequestView,
    project_input_requests,
};
use crate::error::Result;
use crate::model::{ReviewUnitId, ReviewUnitLineageId, TrackId};
use crate::session::EventStore;
use crate::session::event::AssertionMode;
use crate::session::observation::{
    CurrentReviewUnitContext, ReviewUnitScope, ReviewUnitSelection, resolve_review_unit,
    validated_track_id,
};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store::resolution::resolve_read_store;
use crate::session::workflow::read_store::divergence_diagnostics;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputRequestListOptions {
    repo: PathBuf,
    review_unit_id: Option<ReviewUnitId>,
    lineage_id: Option<ReviewUnitLineageId>,
    track: Option<String>,
    mode: Option<AssertionMode>,
    file: Option<String>,
    status: InputRequestStatusFilter,
    include_body: bool,
}

impl InputRequestListOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_unit_id: None,
            lineage_id: None,
            track: None,
            mode: None,
            file: None,
            status: InputRequestStatusFilter::Open,
            include_body: false,
        }
    }

    pub fn with_review_unit_id(mut self, id: ReviewUnitId) -> Self {
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

    pub fn with_mode(mut self, mode: AssertionMode) -> Self {
        self.mode = Some(mode);
        self
    }

    pub fn with_file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    pub fn with_status(mut self, status: InputRequestStatusFilter) -> Self {
        self.status = status;
        self
    }

    pub fn with_include_body(mut self, include_body: bool) -> Self {
        self.include_body = include_body;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputRequestListFilters {
    pub track_id: Option<TrackId>,
    pub mode: Option<AssertionMode>,
    pub file: Option<String>,
    pub status: InputRequestStatusFilter,
    pub include_body: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputRequestListResult {
    pub review_unit_id: ReviewUnitId,
    pub filters: InputRequestListFilters,
    pub input_requests: Vec<InputRequestView>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn list_input_requests(options: InputRequestListOptions) -> Result<InputRequestListResult> {
    let read_store = resolve_read_store(&options.repo)?;
    let store_dir = read_store.store_dir();
    let event_store = EventStore::open(store_dir);
    let events = event_store.list_events()?;
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
    let input_requests = project_input_requests(InputRequestProjectionOptions {
        store_dir,
        events: &events,
        resolved: &resolved,
        track_filter: track_filter.clone(),
        mode_filter: options.mode,
        file_filter: options.file.as_deref(),
        status_filter: options.status,
        include_body: options.include_body,
    })?;
    let mut diagnostics = SessionState::from_events(&events)?.diagnostics;
    diagnostics.extend(divergence_diagnostics(&read_store));

    Ok(InputRequestListResult {
        review_unit_id: resolved.review_unit_id,
        filters: InputRequestListFilters {
            track_id: track_filter,
            mode: options.mode,
            file: options.file,
            status: options.status,
            include_body: options.include_body,
        },
        input_requests,
        diagnostics,
    })
}
