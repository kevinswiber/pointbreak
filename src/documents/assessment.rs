// Document builders for `shore review-assessment add` and `show`.
use crate::documents::{
    AssessmentViewDocument, CurrentAssessmentDocument, DiagnosticDocument, EventWriteDocument,
};
use crate::model::ReviewTargetRef;
use crate::session::event::ReviewAssessment;
use crate::session::{
    AssessmentAddResult, AssessmentShowFilters, AssessmentShowResult, DelegationMap,
};

/// Documented body for `shore.review-assessment-add`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssessmentAddBody {
    review_unit_id: String,
    assessment_id: String,
    event_id: String,
    track_id: String,
    target: ReviewTargetRef,
    assessment: ReviewAssessment,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary_content_hash: Option<String>,
}

/// Documented body for `shore.review-assessment-show`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssessmentShowBody {
    review_unit_id: String,
    filters: AssessmentShowFiltersDocument,
    current: CurrentAssessmentDocument,
    assessments: Vec<AssessmentViewDocument>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AssessmentShowFiltersDocument {
    #[serde(skip_serializing_if = "Option::is_none")]
    track_id: Option<String>,
    all: bool,
    include_summary: bool,
}

/// Build an assessment-add document under `schema` from an add result.
pub fn assessment_add_document(
    schema: &'static str,
    result: AssessmentAddResult,
) -> EventWriteDocument<AssessmentAddBody> {
    EventWriteDocument::new(
        schema,
        AssessmentAddBody {
            review_unit_id: result.review_unit_id.as_str().to_owned(),
            assessment_id: result.assessment_id.as_str().to_owned(),
            event_id: result.event_id.as_str().to_owned(),
            track_id: result.track_id.as_str().to_owned(),
            target: result.target,
            assessment: result.assessment,
            summary_content_hash: result.summary_content_hash,
        },
        result.events_created,
        result.events_existing,
        result.events_created_by_type,
        result.diagnostics,
    )
}

/// Build an assessment-show document under `schema` from a show result.
pub fn assessment_show_document(
    schema: &'static str,
    result: AssessmentShowResult,
    delegation_map: Option<&DelegationMap>,
) -> DiagnosticDocument<AssessmentShowBody> {
    DiagnosticDocument::new(
        schema,
        AssessmentShowBody {
            review_unit_id: result.review_unit_id.as_str().to_owned(),
            filters: AssessmentShowFiltersDocument::from(result.filters),
            current: CurrentAssessmentDocument::from(result.current)
                .with_resolved_principal(delegation_map),
            assessments: result
                .assessments
                .into_iter()
                .map(|view| {
                    AssessmentViewDocument::from(view).with_resolved_principal(delegation_map)
                })
                .collect(),
        },
        result.diagnostics,
    )
}

impl From<AssessmentShowFilters> for AssessmentShowFiltersDocument {
    fn from(filters: AssessmentShowFilters) -> Self {
        Self {
            track_id: filters
                .track_id
                .map(|track_id| track_id.as_str().to_owned()),
            all: filters.include_all,
            include_summary: filters.include_summary,
        }
    }
}
