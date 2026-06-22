// Document builder for `shore review-capture`.
use crate::documents::EventWriteDocument;
use crate::model::ReviewEndpoint;
use crate::session::CaptureResult;

/// Documented body for `shore.review-capture`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureBody {
    revision: CaptureRevisionDocument,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureRevisionDocument {
    id: String,
    base: ReviewEndpoint,
    target: ReviewEndpoint,
    revision_id: String,
    object_id: String,
    snapshot_artifact_content_hash: String,
}

/// Build the `shore.review-capture` document from a capture result.
pub fn capture_document(result: CaptureResult) -> EventWriteDocument<CaptureBody> {
    EventWriteDocument::new(
        "shore.review-capture",
        CaptureBody {
            revision: CaptureRevisionDocument {
                id: result.revision_id.as_str().to_owned(),
                base: result.base,
                target: result.target,
                revision_id: result.revision_id.as_str().to_owned(),
                object_id: result.object_id.as_str().to_owned(),
                snapshot_artifact_content_hash: result.snapshot_artifact_content_hash,
            },
        },
        result.events_created,
        result.events_existing,
        result.events_created_by_type,
        result.diagnostics,
    )
}
