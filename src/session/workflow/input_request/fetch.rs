use std::path::{Path, PathBuf};

use super::view::{
    InputRequestProjectionRecords, InputRequestView, collect_input_request_projection_records,
    input_request_view_from_event,
};
use crate::error::{Result, ShoreError};
use crate::model::InputRequestId;
use crate::session::EventStore;
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store_init::ShoreStorePaths;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputRequestFetchOptions {
    repo: PathBuf,
    input_request_id: InputRequestId,
    include_body: bool,
}

impl InputRequestFetchOptions {
    pub fn new(repo: impl AsRef<Path>, input_request_id: InputRequestId) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            input_request_id,
            include_body: false,
        }
    }

    pub fn with_include_body(mut self, include_body: bool) -> Self {
        self.include_body = include_body;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputRequestFetchResult {
    pub input_request: InputRequestView,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn fetch_input_request(options: InputRequestFetchOptions) -> Result<InputRequestFetchResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let shore_dir = paths.shore_dir();
    let events = EventStore::open(shore_dir).list_events()?;
    let InputRequestProjectionRecords {
        mut request_records,
        responses,
    } = collect_input_request_projection_records(&events)?;

    if let Some(record) = request_records.remove(&options.input_request_id) {
        let view = input_request_view_from_event(
            shore_dir,
            record.event,
            record.payload,
            record.track_id,
            responses
                .get(&options.input_request_id)
                .cloned()
                .unwrap_or_default(),
            options.include_body,
        )?;
        let diagnostics = SessionState::from_events(&events)?.diagnostics;

        return Ok(InputRequestFetchResult {
            input_request: view,
            diagnostics,
        });
    }

    Err(ShoreError::Message(format!(
        "unknown input request: {}",
        options.input_request_id.as_str()
    )))
}
