use std::path::{Path, PathBuf};

use super::view::{
    InputRequestProjectionRecords, InputRequestView, collect_input_request_projection_records,
    input_request_view_from_event, response_views_from_records,
};
use crate::error::{Result, ShoreError};
use crate::model::InputRequestId;
use crate::session::projection::body_content::{BodyRemovalLens, body_content_diagnostics};
use crate::session::projection::cosignature::CosignatureIndex;
use crate::session::signing::{RemovalPolicy, TrustSet};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store::resolution::resolve_read_store;
use crate::session::{ArtifactRemovalProjection, EventStore};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputRequestFetchOptions {
    repo: PathBuf,
    input_request_id: InputRequestId,
    include_body: bool,
    trust_set: TrustSet,
    removal_policy: RemovalPolicy,
}

impl InputRequestFetchOptions {
    pub fn new(repo: impl AsRef<Path>, input_request_id: InputRequestId) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            input_request_id,
            include_body: false,
            trust_set: TrustSet::default(),
            removal_policy: RemovalPolicy::default(),
        }
    }

    pub fn with_include_body(mut self, include_body: bool) -> Self {
        self.include_body = include_body;
        self
    }

    /// Supply the reader's trust set for removal-state resolution
    /// (reader-relativity; the empty default reads every signer as untrusted).
    pub fn with_trust_set(mut self, trust_set: TrustSet) -> Self {
        self.trust_set = trust_set;
        self
    }

    /// Supply the render-time removal policy. A non-operative removal claim
    /// renders the bytes; an operative one renders the explained removed
    /// state. Render-only: it never gates the compact erasure sweep.
    pub fn with_removal_policy(mut self, removal_policy: RemovalPolicy) -> Self {
        self.removal_policy = removal_policy;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputRequestFetchResult {
    pub input_request: InputRequestView,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn fetch_input_request(options: InputRequestFetchOptions) -> Result<InputRequestFetchResult> {
    let read_store = resolve_read_store(&options.repo)?;
    let events = EventStore::from_backend(read_store.backend()).list_events()?;
    let removal = ArtifactRemovalProjection::from_events(&events)?;
    let cosig_index = CosignatureIndex::build(&events)?;
    let removal_lens = BodyRemovalLens::new(
        &removal,
        &options.trust_set,
        options.removal_policy,
        &cosig_index,
    );
    let InputRequestProjectionRecords {
        mut request_records,
        responses,
    } = collect_input_request_projection_records(&events)?;

    if let Some(record) = request_records.remove(&options.input_request_id) {
        // Only the fetched request's responses resolve their reasons; other
        // requests' artifacts are never read here.
        let responses = match responses.get(&options.input_request_id) {
            Some(records) => response_views_from_records(
                read_store.backend(),
                &removal_lens,
                options.include_body,
                records,
            )?,
            None => Vec::new(),
        };
        let view = input_request_view_from_event(
            read_store.backend(),
            &removal_lens,
            record.event,
            record.payload,
            record.track_id,
            responses,
            options.include_body,
        )?;
        let mut diagnostics = SessionState::from_events(&events)?.diagnostics;
        diagnostics.extend(body_content_diagnostics(
            std::iter::once((view.body_content_state, view.body_content_hash.as_deref())).chain(
                view.responses.iter().map(|resp| {
                    (
                        resp.reason_content_state,
                        resp.reason_content_hash.as_deref(),
                    )
                }),
            ),
        ));

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
