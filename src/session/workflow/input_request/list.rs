use std::path::{Path, PathBuf};

use super::view::{
    InputRequestProjectionOptions, InputRequestStatusFilter, InputRequestView,
    project_input_requests,
};
use crate::error::Result;
use crate::model::{RevisionId, TrackId};
use crate::session::EventStore;
use crate::session::event::AssertionMode;
use crate::session::observation::{
    CurrentRevisionContext, RevisionScope, RevisionSelection, resolve_revision, validated_track_id,
};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store::resolution::resolve_read_store;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputRequestListOptions {
    repo: PathBuf,
    revision_id: Option<RevisionId>,
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
            revision_id: None,
            track: None,
            mode: None,
            file: None,
            status: InputRequestStatusFilter::Open,
            include_body: false,
        }
    }

    pub fn with_revision_id(mut self, id: RevisionId) -> Self {
        self.revision_id = Some(id);
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
    pub revision_id: RevisionId,
    pub filters: InputRequestListFilters,
    pub input_requests: Vec<InputRequestView>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn list_input_requests(options: InputRequestListOptions) -> Result<InputRequestListResult> {
    let read_store = resolve_read_store(&options.repo)?;
    let event_store = EventStore::from_backend(read_store.backend());
    let events = event_store.list_events()?;
    let resolved = resolve_revision(
        &events,
        RevisionSelection::from_revision_seed(options.revision_id.as_ref()),
        &CurrentRevisionContext::for_repo(&options.repo)?,
        RevisionScope::default(),
    )?;
    let track_filter = options
        .track
        .as_deref()
        .map(validated_track_id)
        .transpose()?;
    let input_requests = project_input_requests(InputRequestProjectionOptions {
        backend: read_store.backend(),
        events: &events,
        resolved: &resolved,
        track_filter: track_filter.clone(),
        mode_filter: options.mode,
        file_filter: options.file.as_deref(),
        status_filter: options.status,
        include_body: options.include_body,
    })?;
    let diagnostics = SessionState::from_events(&events)?.diagnostics;

    Ok(InputRequestListResult {
        revision_id: resolved.revision_id,
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
