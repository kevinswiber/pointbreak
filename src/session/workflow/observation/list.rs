use std::path::{Path, PathBuf};

use super::target::resolve_review_unit;
use super::util::validated_track_id;
use super::view::{ObservationProjectionOptions, ObservationView, project_observations};
use crate::error::Result;
use crate::model::{ReviewUnitId, TrackId};
use crate::session::EventStore;
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store_init::ShoreStorePaths;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationListOptions {
    repo: PathBuf,
    review_unit_id: Option<ReviewUnitId>,
    track: Option<String>,
    file: Option<String>,
    tags: Vec<String>,
    include_body: bool,
}

impl ObservationListOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_unit_id: None,
            track: None,
            file: None,
            tags: Vec::new(),
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

    pub fn with_file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    pub fn with_include_body(mut self, include_body: bool) -> Self {
        self.include_body = include_body;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationListFilters {
    pub track_id: Option<TrackId>,
    pub file: Option<String>,
    pub tags: Vec<String>,
    pub include_body: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationListResult {
    pub review_unit_id: ReviewUnitId,
    pub filters: ObservationListFilters,
    pub observations: Vec<ObservationView>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn list_observations(options: ObservationListOptions) -> Result<ObservationListResult> {
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
    let observations = project_observations(ObservationProjectionOptions {
        shore_dir,
        events: &events,
        resolved: &resolved,
        track_filter: track_filter.clone(),
        file_filter: options.file.as_deref(),
        tag_filters: &options.tags,
        include_body: options.include_body,
    })?;
    let diagnostics = SessionState::from_events(&events)?.diagnostics;

    Ok(ObservationListResult {
        review_unit_id: resolved.review_unit_id,
        filters: ObservationListFilters {
            track_id: track_filter,
            file: options.file,
            tags: options.tags,
            include_body: options.include_body,
        },
        observations,
        diagnostics,
    })
}
