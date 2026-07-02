use std::path::{Path, PathBuf};

use super::target::{CurrentRevisionContext, RevisionScope, RevisionSelection, resolve_revision};
use super::util::validated_track_id;
use super::view::{ObservationProjectionOptions, ObservationView, project_observations};
use crate::error::Result;
use crate::model::{RevisionId, TrackId};
use crate::session::projection::body_content::BodyRemovalLens;
use crate::session::projection::cosignature::CosignatureIndex;
use crate::session::signing::{RemovalPolicy, TrustSet};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store::resolution::resolve_read_store;
use crate::session::{ArtifactRemovalProjection, EventStore};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationListOptions {
    repo: PathBuf,
    revision_id: Option<RevisionId>,
    track: Option<String>,
    file: Option<String>,
    tags: Vec<String>,
    include_body: bool,
}

impl ObservationListOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            revision_id: None,
            track: None,
            file: None,
            tags: Vec::new(),
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
    pub revision_id: RevisionId,
    pub filters: ObservationListFilters,
    pub observations: Vec<ObservationView>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn list_observations(options: ObservationListOptions) -> Result<ObservationListResult> {
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
    let removal = ArtifactRemovalProjection::from_events(&events)?;
    let cosig_index = CosignatureIndex::build(&events)?;
    let trust_set = TrustSet::default();
    let removal_lens =
        BodyRemovalLens::new(&removal, &trust_set, RemovalPolicy::default(), &cosig_index);
    let observations = project_observations(ObservationProjectionOptions {
        backend: read_store.backend(),
        events: &events,
        resolved: &resolved,
        track_filter: track_filter.clone(),
        file_filter: options.file.as_deref(),
        tag_filters: &options.tags,
        include_body: options.include_body,
        removal_lens: &removal_lens,
    })?;
    let diagnostics = SessionState::from_events(&events)?.diagnostics;

    Ok(ObservationListResult {
        revision_id: resolved.revision_id,
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
