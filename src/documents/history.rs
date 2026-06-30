// Document builder for `shore review-history`.
use crate::documents::DiagnosticDocument;
use crate::session::{ReviewHistoryEntry, ReviewHistoryFilters, ReviewHistoryResult};

/// Documented body for `shore.review-history`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryBody {
    event_set_hash: String,
    event_count: usize,
    history_count: usize,
    filters: ReviewHistoryFilters,
    entries: Vec<ReviewHistoryEntry>,
    /// Opaque continuation token for the next page when a window was applied and
    /// entries remain; `null` for an unwindowed or final read. Additive.
    next_cursor: Option<String>,
}

/// Build the `shore.review-history` document from a history result.
pub fn history_document(result: ReviewHistoryResult) -> DiagnosticDocument<HistoryBody> {
    let history_count = result.history_count();
    DiagnosticDocument::new(
        "shore.review-history",
        HistoryBody {
            event_set_hash: result.event_set_hash,
            event_count: result.event_count,
            history_count,
            filters: result.filters,
            entries: result.entries,
            next_cursor: result.next_cursor,
        },
        result.diagnostics,
    )
}
