use std::path::{Path, PathBuf};

use super::super::observation::{
    CurrentRevisionContext, RevisionScope, RevisionSelection, resolve_revision, validated_track_id,
};
use super::view::{
    ValidationCheckProjectionOptions, ValidationCheckView, project_validation_checks,
};
use crate::error::Result;
use crate::model::{RevisionId, TrackId, ValidationStatus};
use crate::session::EventStore;
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store::resolution::resolve_read_store;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationListOptions {
    repo: PathBuf,
    revision_id: Option<RevisionId>,
    track: Option<String>,
    status: Option<ValidationStatus>,
    include_body: bool,
}

impl ValidationListOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            revision_id: None,
            track: None,
            status: None,
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

    pub fn with_status(mut self, status: ValidationStatus) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_include_body(mut self, include_body: bool) -> Self {
        self.include_body = include_body;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationListFilters {
    pub track_id: Option<TrackId>,
    pub status: Option<ValidationStatus>,
    pub include_body: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationListResult {
    pub revision_id: RevisionId,
    pub filters: ValidationListFilters,
    pub validation_checks: Vec<ValidationCheckView>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn list_validation_checks(options: ValidationListOptions) -> Result<ValidationListResult> {
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
    let validation_checks = project_validation_checks(ValidationCheckProjectionOptions {
        backend: read_store.backend(),
        events: &events,
        revision_id: &resolved.revision_id,
        track_filter: track_filter.clone(),
        status_filter: options.status,
        include_body: options.include_body,
    })?;
    let diagnostics = SessionState::from_events(&events)?.diagnostics;

    Ok(ValidationListResult {
        revision_id: resolved.revision_id,
        filters: ValidationListFilters {
            track_id: track_filter,
            status: options.status,
            include_body: options.include_body,
        },
        validation_checks,
        diagnostics,
    })
}
