// Document builders for `shore review validation add` and `list`.
use crate::documents::{DiagnosticDocument, EventWriteDocument, ValidationCheckViewDocument};
use crate::model::{ValidationStatus, ValidationTarget};
use crate::session::{DelegationMap, ValidationAddResult, ValidationListResult};

/// Documented advisory body for `shore.review-validation-add`.
///
/// Validation evidence is reported for review context only; this document does
/// not carry merge, gate, or acceptance authority.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationAddBody {
    review_unit_id: String,
    validation_check_id: String,
    event_id: String,
    track_id: String,
    target: ValidationTarget,
    status: ValidationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary_content_hash: Option<String>,
}

/// Documented body for `shore.review-validation-list`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationListBody {
    review_unit_id: String,
    filters: ValidationListFiltersDocument,
    validation_checks: Vec<ValidationCheckViewDocument>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidationListFiltersDocument {
    #[serde(skip_serializing_if = "Option::is_none")]
    track_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<ValidationStatus>,
    include_body: bool,
}

/// Build the `shore.review-validation-add` document from an add result.
pub fn validation_add_document(
    result: ValidationAddResult,
) -> EventWriteDocument<ValidationAddBody> {
    EventWriteDocument::new(
        "shore.review-validation-add",
        ValidationAddBody {
            review_unit_id: result.review_unit_id.as_str().to_owned(),
            validation_check_id: result.validation_check_id.as_str().to_owned(),
            event_id: result.event_id.as_str().to_owned(),
            track_id: result.track_id.as_str().to_owned(),
            target: result.target,
            status: result.status,
            summary_content_hash: result.summary_content_hash,
        },
        result.events_created,
        result.events_existing,
        result.events_created_by_type,
        result.diagnostics,
    )
}

/// Build the `shore.review-validation-list` document from a list result.
pub fn validation_list_document(
    result: ValidationListResult,
    delegation_map: Option<&DelegationMap>,
) -> DiagnosticDocument<ValidationListBody> {
    DiagnosticDocument::new(
        "shore.review-validation-list",
        ValidationListBody {
            review_unit_id: result.review_unit_id.as_str().to_owned(),
            filters: ValidationListFiltersDocument {
                track_id: result
                    .filters
                    .track_id
                    .map(|track_id| track_id.as_str().to_owned()),
                status: result.filters.status,
                include_body: result.filters.include_body,
            },
            validation_checks: result
                .validation_checks
                .into_iter()
                .map(|view| {
                    ValidationCheckViewDocument::from(view).with_resolved_principal(delegation_map)
                })
                .collect(),
        },
        result.diagnostics,
    )
}
