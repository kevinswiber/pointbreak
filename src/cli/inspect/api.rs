//! JSON payload builders for the inspector server.
//!
//! Each builder reuses a public `shoreline::session` projection so the
//! inspector reads the store through the same validated path as the
//! corresponding `shore review` command, rather than parsing raw `.shore/`
//! files. Errors are stringified so the server can surface them to the UI as
//! a JSON `error` body instead of crashing a connection thread.

use std::path::Path;

use serde::Serialize;
use shoreline::model::{ReviewUnitId, SnapshotId};
use shoreline::session::{
    ProjectionDiagnostic, ReviewHistoryEntry, ReviewHistoryOptions, ReviewUnitListEntry,
    ReviewUnitListOptions, ReviewUnitShowOptions, list_review_units, read_snapshot_artifact,
    review_history, show_review_unit,
};

use crate::cli::review::unit::unit_show_document;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryPayload {
    schema: &'static str,
    event_set_hash: String,
    event_count: usize,
    history_count: usize,
    entries: Vec<ReviewHistoryEntry>,
    diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UnitsPayload {
    schema: &'static str,
    event_set_hash: String,
    event_count: usize,
    review_unit_count: usize,
    entries: Vec<ReviewUnitListEntry>,
    diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FreshnessPayload {
    schema: &'static str,
    event_set_hash: String,
    event_count: usize,
    diagnostic_count: usize,
}

/// Full chronological event timeline with hydrated bodies.
pub(super) fn history_json(repo: &Path) -> Result<String, String> {
    let result = review_history(ReviewHistoryOptions::new(repo).with_include_body(true))
        .map_err(|error| error.to_string())?;
    let history_count = result.history_count();
    let payload = HistoryPayload {
        schema: "shore.inspect-history",
        event_set_hash: result.event_set_hash,
        event_count: result.event_count,
        history_count,
        entries: result.entries,
        diagnostics: result.diagnostics,
    };
    serde_json::to_string(&payload).map_err(|error| error.to_string())
}

/// Captured ReviewUnits with their base/target/snapshot identity.
pub(super) fn units_json(repo: &Path) -> Result<String, String> {
    let result =
        list_review_units(ReviewUnitListOptions::new(repo)).map_err(|error| error.to_string())?;
    let payload = UnitsPayload {
        schema: "shore.inspect-units",
        event_set_hash: result.event_set_hash,
        event_count: result.event_count,
        review_unit_count: result.review_unit_count,
        entries: result.entries,
        diagnostics: result.diagnostics,
    };
    serde_json::to_string(&payload).map_err(|error| error.to_string())
}

/// The captured diff snapshot for one ReviewUnit, by snapshot id.
///
/// Reads the immutable snapshot artifact through the validated read path
/// (`read_snapshot_artifact` recomputes and checks the content hash), so the
/// inspector renders exactly the frozen diff that was reviewed.
pub(super) fn snapshot_json(repo: &Path, snapshot_id: &str) -> Result<String, String> {
    if snapshot_id.is_empty() {
        return Err("missing snapshot id".to_owned());
    }
    let artifact = read_snapshot_artifact(repo, &SnapshotId::new(snapshot_id.to_owned()))
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&artifact).map_err(|error| error.to_string())
}

/// The full composite projection for one ReviewUnit.
///
/// Reuses the exact `shore.review-unit` document the `shore review unit show`
/// command builds (`unit_show_document`), so the inspector renders the same
/// authoritative composite — current-assessment status, duplicate-collapsed
/// facts, supersession, adapter notes, and projection rows — rather than
/// re-deriving it client-side.
pub(super) fn unit_json(repo: &Path, review_unit_id: &str) -> Result<String, String> {
    if review_unit_id.is_empty() {
        return Err("missing review unit id".to_owned());
    }
    let result = show_review_unit(
        ReviewUnitShowOptions::new(repo)
            .with_review_unit_id(ReviewUnitId::new(review_unit_id.to_owned()))
            .with_include_body(true),
    )
    .map_err(|error| error.to_string())?;
    let document = unit_show_document(result);
    serde_json::to_string(&document).map_err(|error| error.to_string())
}

/// Cheap freshness probe for client-side auto-refresh polling.
///
/// Computes `eventSetHash` from the live event set (without hydrating bodies)
/// so the UI can detect store changes and re-fetch only when something moved.
pub(super) fn freshness_json(repo: &Path) -> Result<String, String> {
    let result = review_history(ReviewHistoryOptions::new(repo).with_include_body(false))
        .map_err(|error| error.to_string())?;
    let payload = FreshnessPayload {
        schema: "shore.inspect-freshness",
        event_set_hash: result.event_set_hash,
        event_count: result.event_count,
        diagnostic_count: result.diagnostics.len(),
    };
    serde_json::to_string(&payload).map_err(|error| error.to_string())
}
