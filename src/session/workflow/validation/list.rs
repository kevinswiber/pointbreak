use std::path::{Path, PathBuf};

use super::super::observation::{
    CurrentReviewUnitContext, ReviewUnitScope, ReviewUnitSelection, resolve_review_unit,
    validated_track_id,
};
use super::view::{
    ValidationCheckProjectionOptions, ValidationCheckView, project_validation_checks,
};
use crate::error::Result;
use crate::model::{ReviewUnitId, ReviewUnitLineageId, TrackId, ValidationStatus};
use crate::session::EventStore;
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store::resolution::resolve_read_store;
use crate::session::workflow::read_store::divergence_diagnostics;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationListOptions {
    repo: PathBuf,
    review_unit_id: Option<ReviewUnitId>,
    lineage_id: Option<ReviewUnitLineageId>,
    track: Option<String>,
    status: Option<ValidationStatus>,
    include_body: bool,
}

impl ValidationListOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_unit_id: None,
            lineage_id: None,
            track: None,
            status: None,
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
    pub review_unit_id: ReviewUnitId,
    pub filters: ValidationListFilters,
    pub validation_checks: Vec<ValidationCheckView>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn list_validation_checks(options: ValidationListOptions) -> Result<ValidationListResult> {
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
    let validation_checks = project_validation_checks(ValidationCheckProjectionOptions {
        store_dir,
        events: &events,
        review_unit_id: &resolved.review_unit_id,
        track_filter: track_filter.clone(),
        status_filter: options.status,
        include_body: options.include_body,
    })?;
    let mut diagnostics = SessionState::from_events(&events)?.diagnostics;
    diagnostics.extend(divergence_diagnostics(&read_store));

    Ok(ValidationListResult {
        review_unit_id: resolved.review_unit_id,
        filters: ValidationListFilters {
            track_id: track_filter,
            status: options.status,
            include_body: options.include_body,
        },
        validation_checks,
        diagnostics,
    })
}
