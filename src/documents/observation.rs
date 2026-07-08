// Document builders for `shore review-observation add` and `list`.
use crate::documents::{DiagnosticDocument, EventWriteDocument, ObservationViewDocument};
use crate::model::ReviewTargetRef;
use crate::session::{DelegationMap, ObservationAddResult, ObservationListResult};

/// Documented body for `pointbreak.review-observation-add`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservationAddBody {
    revision_id: String,
    observation_id: String,
    event_id: String,
    track_id: String,
    target: ReviewTargetRef,
    body_content_hash: Option<String>,
}

/// Documented body for `pointbreak.review-observation-list`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservationListBody {
    revision_id: String,
    filters: ObservationListFiltersDocument,
    observations: Vec<ObservationViewDocument>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ObservationListFiltersDocument {
    #[serde(skip_serializing_if = "Option::is_none")]
    track_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    include_body: bool,
}

/// Build the `pointbreak.review-observation-add` document from an add result.
pub fn observation_add_document(
    result: ObservationAddResult,
) -> EventWriteDocument<ObservationAddBody> {
    EventWriteDocument::new(
        "pointbreak.review-observation-add",
        ObservationAddBody {
            revision_id: result.revision_id.as_str().to_owned(),
            observation_id: result.observation_id.as_str().to_owned(),
            event_id: result.event_id.as_str().to_owned(),
            track_id: result.track_id.as_str().to_owned(),
            target: result.target,
            body_content_hash: result.body_content_hash,
        },
        result.events_created,
        result.events_existing,
        result.events_created_by_type,
        result.diagnostics,
    )
}

/// Build the `pointbreak.review-observation-list` document from a list result. The
/// optional delegation map enriches agent-written items with a resolved
/// principal object (reader-supplied config, never store content).
pub fn observation_list_document(
    result: ObservationListResult,
    delegation_map: Option<&DelegationMap>,
) -> DiagnosticDocument<ObservationListBody> {
    DiagnosticDocument::new(
        "pointbreak.review-observation-list",
        ObservationListBody {
            revision_id: result.revision_id.as_str().to_owned(),
            filters: ObservationListFiltersDocument {
                track_id: result
                    .filters
                    .track_id
                    .map(|track_id| track_id.as_str().to_owned()),
                file: result.filters.file,
                tags: result.filters.tags,
                include_body: result.filters.include_body,
            },
            observations: result
                .observations
                .into_iter()
                .map(|view| {
                    ObservationViewDocument::from(view).with_resolved_principal(delegation_map)
                })
                .collect(),
        },
        result.diagnostics,
    )
}
