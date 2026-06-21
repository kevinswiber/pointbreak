use std::io::Write;
use std::path::PathBuf;

use clap::Args;
use shoreline::documents::{capture_document, capture_with_lineage_document};
use shoreline::model::{ReviewUnitLineageId, RevisionId};
use shoreline::session::{
    CaptureOptions, CommitRangeSpec, LineageAttachOptions, attach_review_unit_to_lineage,
    capture_review,
};

use crate::cli::json;
use crate::cli_tracing::TracingArgs;

#[derive(Debug, Args)]
pub(super) struct CaptureArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Capture the committed range from this rev (resolved to a commit, peeling
    /// annotated tags) to --target instead of the HEAD -> working-tree diff.
    /// The working tree and untracked files are not read.
    #[arg(long)]
    base: Option<String>,

    /// Range end rev (resolved to a commit). Defaults to HEAD; requires --base.
    #[arg(long)]
    target: Option<String>,

    /// Attach the captured ReviewUnit to this lineage.
    #[arg(long)]
    lineage: Option<String>,

    /// Previous captured ReviewUnit in the lineage.
    #[arg(long)]
    predecessor: Option<String>,

    /// Optional Change-Id metadata to record on the lineage round.
    #[arg(long)]
    change_id: Option<String>,

    /// Sign this write with a specific key: a keystore key name or a path to a
    /// key file. Overrides SHORE_SIGNING_KEY. A key that cannot be loaded leaves
    /// the write unsigned (exit 0) with an advisory diagnostic — signing never
    /// blocks.
    #[arg(long)]
    sign_key: Option<String>,
}

pub(super) fn run(
    args: CaptureArgs,
    tracing: &TracingArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let span = tracing::info_span!("shore.review.capture");
    let _entered = span.enter();
    tracing::debug!(command = "review.capture", "command_start");
    if args.target.is_some() && args.base.is_none() {
        return Err("--target requires --base".into());
    }
    if args.predecessor.is_some() && args.lineage.is_none() {
        return Err("predecessor requires --lineage".into());
    }
    let (options, skip) = capture_options(&args, tracing, stderr);
    let capture = capture_review(options)?;
    super::common::surface_best_effort_skip(&skip, stderr);
    let Some(lineage) = args.lineage.as_ref() else {
        let document = capture_document(capture);
        return json::write_json(stdout, &document, false);
    };
    let attach = attach_review_unit_to_lineage(capture_lineage_attach_options(
        &args,
        lineage,
        &capture.revision_id,
    ))?;
    let document = capture_with_lineage_document(capture, attach);
    json::write_json(stdout, &document, false)
}

fn capture_options(
    args: &CaptureArgs,
    tracing: &TracingArgs,
    stderr: &mut dyn Write,
) -> (CaptureOptions, super::common::SigningSkip) {
    let mut options = CaptureOptions::new(&args.repo);
    if let Some(range) = commit_range_spec(args) {
        options = options.with_commit_range(range);
    }
    if let Some(log_file) = &tracing.log_file {
        options = options.with_excluded_helper_path(log_file);
    }
    let mut skip = None;
    if let Some(resolved) =
        super::common::resolve_and_surface_signer(&args.repo, args.sign_key.as_deref(), stderr)
    {
        let (signed, signer_skip) = super::common::apply_resolved_signer(options, resolved);
        options = signed;
        skip = signer_skip;
    }
    (options, skip)
}

/// Build the commit-range spec from `--base`/`--target`. `None` keeps the
/// default worktree capture. `--target` without `--base` is rejected in `run`
/// before this point.
fn commit_range_spec(args: &CaptureArgs) -> Option<CommitRangeSpec> {
    let base = args.base.as_ref()?;
    let mut range = CommitRangeSpec::new(base.clone());
    if let Some(target) = &args.target {
        range = range.with_target_rev(target.clone());
    }
    Some(range)
}

fn capture_lineage_attach_options(
    args: &CaptureArgs,
    lineage: &str,
    review_unit_id: &RevisionId,
) -> LineageAttachOptions {
    let mut options =
        LineageAttachOptions::new(&args.repo, ReviewUnitLineageId::new(lineage.to_owned()))
            .with_review_unit_id(review_unit_id.clone());
    if let Some(predecessor) = &args.predecessor {
        options = options.with_predecessor_review_unit_id(RevisionId::new(predecessor.clone()));
    }
    if let Some(change_id) = &args.change_id {
        options = options.with_change_id(change_id.clone());
    }
    options
}
