use std::path::{Path, PathBuf};

use super::view::{
    InterventionView, collect_request_records, collect_resolution_views,
    intervention_view_from_event,
};
use crate::error::{Result, ShoreError};
use crate::model::InterventionId;
use crate::session::EventStore;
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store_init::ShoreStorePaths;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterventionFetchOptions {
    repo: PathBuf,
    intervention_id: InterventionId,
    include_body: bool,
}

impl InterventionFetchOptions {
    pub fn new(repo: impl AsRef<Path>, intervention_id: InterventionId) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            intervention_id,
            include_body: false,
        }
    }

    pub fn with_include_body(mut self, include_body: bool) -> Self {
        self.include_body = include_body;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterventionFetchResult {
    pub intervention: InterventionView,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn fetch_intervention(options: InterventionFetchOptions) -> Result<InterventionFetchResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let shore_dir = paths.shore_dir();
    let events = EventStore::open(shore_dir).list_events()?;
    let mut request_records = collect_request_records(&events)?;
    let resolutions = collect_resolution_views(&events)?;

    if let Some(record) = request_records.remove(&options.intervention_id) {
        let view = intervention_view_from_event(
            shore_dir,
            record.event,
            record.payload,
            record.track_id,
            resolutions
                .get(&options.intervention_id)
                .cloned()
                .unwrap_or_default(),
            options.include_body,
        )?;
        let diagnostics = SessionState::from_events(&events)?.diagnostics;

        return Ok(InterventionFetchResult {
            intervention: view,
            diagnostics,
        });
    }

    Err(ShoreError::Message(format!(
        "unknown intervention: {}",
        options.intervention_id.as_str()
    )))
}
