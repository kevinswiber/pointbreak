use std::io::Write;
use std::path::PathBuf;

use clap::Args;
use shore::model::ReviewEndpoint;
use shore::session::{CaptureOptions, CaptureResult, capture_worktree_review};

use crate::cli::json;
use crate::cli_tracing::TracingArgs;

#[derive(Debug, Args)]
pub(super) struct CaptureArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureBody {
    review_unit: CaptureReviewUnitDocument,
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

pub(super) fn run(
    args: CaptureArgs,
    tracing: &TracingArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::debug!(command = "review.capture", "command_start");
    let result = capture_worktree_review(capture_options(&args, tracing));
    let document = capture_document(result?);
    json::write_json(stdout, &document, false)
}

fn capture_options(args: &CaptureArgs, tracing: &TracingArgs) -> CaptureOptions {
    let mut options = CaptureOptions::new(&args.repo);
    if let Some(log_file) = &tracing.log_file {
        options = options.with_excluded_helper_path(log_file);
    }
    options
}

fn capture_document(result: CaptureResult) -> json::EventWriteDocument<CaptureBody> {
    json::EventWriteDocument::new(
        "shore.review-capture",
        CaptureBody {
            review_unit: CaptureReviewUnitDocument {
                id: result.review_unit_id.as_str().to_owned(),
                base: result.base,
                target: result.target,
                revision_id: result.revision_id.as_str().to_owned(),
                snapshot_id: result.snapshot_id.as_str().to_owned(),
                snapshot_artifact_content_hash: result.snapshot_artifact_content_hash,
            },
        },
        result.events_created,
        result.events_existing,
        result.events_created_by_type,
        result.diagnostics,
    )
}
