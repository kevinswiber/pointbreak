// Document builder for `shore review-capture`.
use crate::documents::EventWriteDocument;
use crate::model::ReviewEndpoint;
use crate::session::{CaptureResult, LineageAttachResult, ProjectionDiagnostic};

/// Documented body for `shore.review-capture`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureBody {
    review_unit: CaptureReviewUnitDocument,
}

/// Documented body for `shore.review-capture` when the capture is also attached to a lineage.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureWithLineageBody {
    review_unit: CaptureReviewUnitDocument,
    lineage_attach: CaptureLineageAttachDocument,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureReviewUnitDocument {
    id: String,
    base: ReviewEndpoint,
    target: ReviewEndpoint,
    revision_id: String,
    snapshot_id: String,
    snapshot_artifact_content_hash: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureLineageAttachDocument {
    lineage_id: String,
    head_review_unit_id: Option<String>,
    events_created: usize,
    events_existing: usize,
    events_created_by_type: std::collections::BTreeMap<String, usize>,
    diagnostics: Vec<ProjectionDiagnostic>,
}

/// Build the `shore.review-capture` document from a capture result.
pub fn capture_document(result: CaptureResult) -> EventWriteDocument<CaptureBody> {
    EventWriteDocument::new(
        "shore.review-capture",
        CaptureBody {
            review_unit: CaptureReviewUnitDocument {
                id: result.revision_id.as_str().to_owned(),
                base: result.base,
                target: result.target,
                revision_id: result.revision_id.as_str().to_owned(),
                snapshot_id: result.object_id.as_str().to_owned(),
                snapshot_artifact_content_hash: result.snapshot_artifact_content_hash,
            },
        },
        result.events_created,
        result.events_existing,
        result.events_created_by_type,
        result.diagnostics,
    )
}

/// Build the `shore.review-capture` document for capture-and-attach convenience.
pub fn capture_with_lineage_document(
    capture: CaptureResult,
    attach: LineageAttachResult,
) -> EventWriteDocument<CaptureWithLineageBody> {
    EventWriteDocument::new(
        "shore.review-capture",
        CaptureWithLineageBody {
            review_unit: CaptureReviewUnitDocument {
                id: capture.revision_id.as_str().to_owned(),
                base: capture.base,
                target: capture.target,
                revision_id: capture.revision_id.as_str().to_owned(),
                snapshot_id: capture.object_id.as_str().to_owned(),
                snapshot_artifact_content_hash: capture.snapshot_artifact_content_hash,
            },
            lineage_attach: CaptureLineageAttachDocument {
                lineage_id: attach.lineage_id.as_str().to_owned(),
                head_review_unit_id: attach
                    .head_review_unit_id
                    .map(|review_unit_id| review_unit_id.as_str().to_owned()),
                events_created: attach.events_created,
                events_existing: attach.events_existing,
                events_created_by_type: attach.events_created_by_type,
                diagnostics: attach.diagnostics,
            },
        },
        capture.events_created,
        capture.events_existing,
        capture.events_created_by_type,
        capture.diagnostics,
    )
}
