//! JSON payload builders for the inspector server.
//!
//! Each builder reuses a public `shoreline::session` projection so the
//! inspector reads the store through the same validated path as the
//! corresponding `shore review` command, rather than parsing raw `.shore/`
//! files. Errors are stringified so the server can surface them to the UI as
//! a JSON `error` body instead of crashing a connection thread.

use std::path::Path;

use serde::Serialize;
use shoreline::session::{
    ProjectionDiagnostic, ReviewHistoryEntry, ReviewHistoryOptions, ReviewUnitListEntry,
    ReviewUnitListOptions, list_review_units, review_history,
};

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
