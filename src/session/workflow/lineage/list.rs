use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::model::{ReviewUnitId, ReviewUnitLineageId};
use crate::session::EventStore;
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store::resolution::resolve_read_store;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineageListOptions {
    repo: PathBuf,
}

impl LineageListOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineageListResult {
    pub event_set_hash: String,
    pub event_count: usize,
    pub lineage_count: usize,
    pub entries: Vec<LineageListEntry>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineageListEntry {
    pub lineage_id: ReviewUnitLineageId,
    pub head_review_unit_id: Option<ReviewUnitId>,
    pub round_count: usize,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn list_lineages(options: LineageListOptions) -> Result<LineageListResult> {
    let read_store = resolve_read_store(&options.repo)?;
    let events = EventStore::open(read_store.store_dir()).list_events()?;
    let state = SessionState::from_events(&events)?;
    let projection =
        crate::session::projection::lineage::ReviewUnitLineageProjection::from_events(&events)?;
    let entries = projection
        .lineages
        .values()
        .map(|lineage| LineageListEntry {
            lineage_id: lineage.lineage_id.clone(),
            head_review_unit_id: lineage.head_review_unit_id.clone(),
            round_count: lineage.rounds.len(),
            diagnostics: lineage.diagnostics.clone(),
        })
        .collect::<Vec<_>>();
    let lineage_count = entries.len();
    let event_set_hash = state
        .event_set_hash
        .expect("SessionState::from_events sets event_set_hash");

    Ok(LineageListResult {
        event_set_hash,
        event_count: events.len(),
        lineage_count,
        entries,
        diagnostics: Vec::new(),
    })
}
