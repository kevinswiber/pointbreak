// Document builder for the `shore attention list` command.
use crate::documents::DiagnosticDocument;
use crate::session::{AttentionItem, AttentionListResult};

/// Emitted schema for `shore attention list` (D1: attention is product-level
/// vocabulary, so the class stays off the de-review rename pile).
pub const ATTENTION_LIST_SCHEMA: &str = "pointbreak.attention-list";

/// Documented body for `pointbreak.attention-list`. The library `AttentionItem`
/// serializes directly into `items` — there is no parallel DTO, so the headless
/// projection and the emitted document share one spelling.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionListBody {
    event_set_hash: String,
    event_count: usize,
    filters: AttentionListFiltersDocument,
    items: Vec<AttentionItem>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AttentionListFiltersDocument {
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
}

/// Build the `pointbreak.attention-list` document from a projection result. The
/// envelope's diagnostics ride the shared `DiagnosticDocument` tail.
pub fn attention_list_document(
    result: AttentionListResult,
) -> DiagnosticDocument<AttentionListBody> {
    let revision = result.revision.as_ref().map(|id| id.as_str().to_owned());
    DiagnosticDocument::new(
        ATTENTION_LIST_SCHEMA,
        AttentionListBody {
            event_set_hash: result.event_set_hash,
            event_count: result.event_count,
            filters: AttentionListFiltersDocument { revision },
            items: result.items,
        },
        result.diagnostics,
    )
}
