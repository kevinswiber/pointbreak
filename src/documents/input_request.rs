// Document builders for the `shore review-input-request` family.
use crate::documents::{
    DiagnosticDocument, EventWriteDocument, InputRequestAssertionModeDocument,
    InputRequestViewDocument,
};
use crate::model::ReviewTargetRef;
use crate::session::event::{InputRequestReasonCode, InputRequestResponseOutcome};
use crate::session::{
    DelegationMap, InputRequestFetchResult, InputRequestListResult, InputRequestOpenResult,
    InputRequestRespondResult,
};

/// Documented body for `pointbreak.review-input-request-open`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputRequestOpenBody {
    revision_id: String,
    input_request_id: String,
    event_id: String,
    track_id: String,
    target: ReviewTargetRef,
    mode: InputRequestAssertionModeDocument,
    reason_code: InputRequestReasonCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_content_hash: Option<String>,
}

/// Documented body for `pointbreak.review-input-request-list`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputRequestListBody {
    revision_id: String,
    filters: InputRequestListFiltersDocument,
    input_requests: Vec<InputRequestViewDocument>,
}

/// Documented body for `pointbreak.review-input-request-show`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputRequestFetchBody {
    input_request: InputRequestViewDocument,
}

/// Documented body for `pointbreak.review-input-request-respond`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputRequestRespondBody {
    input_request_id: String,
    input_request_response_id: String,
    event_id: String,
    outcome: InputRequestResponseOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason_content_hash: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct InputRequestListFiltersDocument {
    #[serde(skip_serializing_if = "Option::is_none")]
    track_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<InputRequestAssertionModeDocument>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    status: &'static str,
    include_body: bool,
}

/// Build the `pointbreak.review-input-request-open` document from an open result.
pub fn input_request_open_document(
    result: InputRequestOpenResult,
) -> EventWriteDocument<InputRequestOpenBody> {
    EventWriteDocument::new(
        "pointbreak.review-input-request-open",
        InputRequestOpenBody {
            revision_id: result.revision_id.as_str().to_owned(),
            input_request_id: result.input_request_id.as_str().to_owned(),
            event_id: result.event_id.as_str().to_owned(),
            track_id: result.track_id.as_str().to_owned(),
            target: result.target,
            mode: result.assertion_mode.into(),
            reason_code: result.reason_code,
            body_content_hash: result.body_content_hash,
        },
        result.events_created,
        result.events_existing,
        result.events_created_by_type,
        result.diagnostics,
    )
}

/// Build the `pointbreak.review-input-request-list` document from a list result.
pub fn input_request_list_document(
    result: InputRequestListResult,
    delegation_map: Option<&DelegationMap>,
) -> DiagnosticDocument<InputRequestListBody> {
    DiagnosticDocument::new(
        "pointbreak.review-input-request-list",
        InputRequestListBody {
            revision_id: result.revision_id.as_str().to_owned(),
            filters: InputRequestListFiltersDocument {
                track_id: result
                    .filters
                    .track_id
                    .map(|track_id| track_id.as_str().to_owned()),
                mode: result
                    .filters
                    .mode
                    .map(InputRequestAssertionModeDocument::from),
                file: result.filters.file,
                status: result.filters.status.as_str(),
                include_body: result.filters.include_body,
            },
            input_requests: result
                .input_requests
                .into_iter()
                .map(|view| {
                    InputRequestViewDocument::from(view).with_resolved_principal(delegation_map)
                })
                .collect(),
        },
        result.diagnostics,
    )
}

/// Build the `pointbreak.review-input-request-show` document from a fetch result.
pub fn input_request_fetch_document(
    result: InputRequestFetchResult,
    delegation_map: Option<&DelegationMap>,
) -> DiagnosticDocument<InputRequestFetchBody> {
    DiagnosticDocument::new(
        "pointbreak.review-input-request-show",
        InputRequestFetchBody {
            input_request: InputRequestViewDocument::from(result.input_request)
                .with_resolved_principal(delegation_map),
        },
        result.diagnostics,
    )
}

/// Build the `pointbreak.review-input-request-respond` document from a respond result.
pub fn input_request_respond_document(
    result: InputRequestRespondResult,
) -> EventWriteDocument<InputRequestRespondBody> {
    EventWriteDocument::new(
        "pointbreak.review-input-request-respond",
        InputRequestRespondBody {
            input_request_id: result.input_request_id.as_str().to_owned(),
            input_request_response_id: result.input_request_response_id.as_str().to_owned(),
            event_id: result.event_id.as_str().to_owned(),
            outcome: result.outcome,
            reason_content_hash: result.reason_content_hash,
        },
        result.events_created,
        result.events_existing,
        result.events_created_by_type,
        result.diagnostics,
    )
}
