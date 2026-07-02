use std::io::Write;
use std::path::PathBuf;

use clap::Args;
use shoreline::documents::capture_document;
use shoreline::model::RevisionId;
use shoreline::session::{CaptureOptions, CommitRangeSpec, capture_review};

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

    /// Record this capture as superseding one or more earlier revisions (an
    /// evolution forward-pointer). May be repeated; the set is order-independent.
    #[arg(long = "supersedes")]
    supersedes: Vec<String>,

    /// Scope the capture to the given git pathspec(s): both the tracked diff
    /// and untracked-file synthesis include only matching files. May be
    /// repeated; the recorded set is order-independent. Pathspecs are
    /// interpreted relative to the repository root (native git pathspec
    /// syntax, including magic like ":(exclude)..."). A scope that matches no
    /// changed files is an error.
    #[arg(long = "path", value_name = "PATHSPEC")]
    paths: Vec<String>,

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
    let (options, skip) = capture_options(&args, tracing, stderr);
    let capture = capture_review(options)?;
    super::common::surface_best_effort_skip(&skip, stderr);
    let document = capture_document(capture);
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
    if !args.supersedes.is_empty() {
        options = options.with_supersedes(
            args.supersedes
                .iter()
                .map(|id| RevisionId::new(id.clone()))
                .collect(),
        );
    }
    if !args.paths.is_empty() {
        options = options.with_pathspecs(args.paths.clone());
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
