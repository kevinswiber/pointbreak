use std::path::{Path, PathBuf};

use super::view::{
    InterventionProjectionOptions, InterventionStatusFilter, InterventionView,
    project_interventions,
};
use crate::error::Result;
use crate::model::{ReviewUnitId, TrackId};
use crate::session::EventStore;
use crate::session::event::InterventionMode;
use crate::session::observation::{resolve_review_unit, validated_track_id};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store_init::ShoreStorePaths;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterventionListOptions {
    repo: PathBuf,
    review_unit_id: Option<ReviewUnitId>,
    track: Option<String>,
    mode: Option<InterventionMode>,
    file: Option<String>,
    status: InterventionStatusFilter,
    include_body: bool,
}

impl InterventionListOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_unit_id: None,
            track: None,
            mode: None,
            file: None,
            status: InterventionStatusFilter::Open,
            include_body: false,
        }
    }

    pub fn with_review_unit_id(mut self, id: ReviewUnitId) -> Self {
        self.review_unit_id = Some(id);
        self
    }

    pub fn with_track(mut self, track: impl Into<String>) -> Self {
        self.track = Some(track.into());
        self
    }

    pub fn with_mode(mut self, mode: InterventionMode) -> Self {
        self.mode = Some(mode);
        self
    }

    pub fn with_file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    pub fn with_status(mut self, status: InterventionStatusFilter) -> Self {
        self.status = status;
        self
    }

    pub fn with_include_body(mut self, include_body: bool) -> Self {
        self.include_body = include_body;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterventionListFilters {
    pub track_id: Option<TrackId>,
    pub mode: Option<InterventionMode>,
    pub file: Option<String>,
    pub status: InterventionStatusFilter,
    pub include_body: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterventionListResult {
    pub review_unit_id: ReviewUnitId,
    pub filters: InterventionListFilters,
    pub interventions: Vec<InterventionView>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn list_interventions(options: InterventionListOptions) -> Result<InterventionListResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let shore_dir = paths.shore_dir();
    let event_store = EventStore::open(shore_dir);
    let events = event_store.list_events()?;
    let resolved = resolve_review_unit(&events, options.review_unit_id.as_ref())?;
    let track_filter = options
        .track
        .as_deref()
        .map(validated_track_id)
        .transpose()?;
    let interventions = project_interventions(InterventionProjectionOptions {
        shore_dir,
        events: &events,
        resolved: &resolved,
        track_filter: track_filter.clone(),
        mode_filter: options.mode,
        file_filter: options.file.as_deref(),
        status_filter: options.status,
        include_body: options.include_body,
    })?;
    let diagnostics = SessionState::from_events(&events)?.diagnostics;

    Ok(InterventionListResult {
        review_unit_id: resolved.review_unit_id,
        filters: InterventionListFilters {
            track_id: track_filter,
            mode: options.mode,
            file: options.file,
            status: options.status,
            include_body: options.include_body,
        },
        interventions,
        diagnostics,
    })
}
