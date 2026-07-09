use std::io::Write;
use std::path::PathBuf;

use clap::Args;
use pointbreak::documents::capture_document;
use pointbreak::model::{ReviewEndpoint, RevisionId};
use pointbreak::session::{
    CaptureOptions, CaptureResult, CommitRangeSpec, RootCommitSpec, StagedSpec, UnstagedSpec,
    WorktreeSpec, capture_review,
};

use crate::cli::output;
use crate::cli_tracing::TracingArgs;

/// Capture a revision from the working tree or a committed commit range.
#[derive(Debug, Args)]
pub(super) struct CaptureArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Capture the committed range from this rev (resolved to a commit, peeling
    /// annotated tags) to --target instead of the HEAD -> working-tree diff.
    /// The working tree and untracked files are not read.
    #[arg(long)]
    base: Option<String>,

    /// Capture the target commit against Git's empty tree.
    #[arg(long)]
    root: bool,

    /// Capture staged changes only.
    #[arg(long)]
    staged: bool,

    /// Capture unstaged tracked changes only.
    #[arg(long)]
    unstaged: bool,

    /// Include untracked files with worktree or unstaged capture.
    #[arg(long)]
    include_untracked: bool,

    /// Record a revision even when the selected source has no changed files.
    #[arg(long)]
    allow_empty: bool,

    /// Target rev (resolved to a commit). Defaults to HEAD with --base or --root.
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

    #[command(flatten)]
    format_args: output::FormatArgs,
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
    let explicit_sources = [args.base.is_some(), args.root, args.staged, args.unstaged]
        .into_iter()
        .filter(|selected| *selected)
        .count();
    if explicit_sources > 1 {
        return Err("--base, --root, --staged, and --unstaged are mutually exclusive".into());
    }
    if args.target.is_some() && args.base.is_none() && !args.root {
        return Err("--target requires --base or --root".into());
    }
    if args.include_untracked && (args.base.is_some() || args.root || args.staged) {
        return Err(
            "--include-untracked can only be used with worktree or unstaged capture".into(),
        );
    }
    let (options, skip) = capture_options(&args, tracing, stderr)?;
    let capture = capture_review(options)?;
    crate::cli::common::surface_best_effort_skip(&skip, stderr);
    // Best-effort: if this worktree is splitting off from a family store a sibling
    // worktree is linked to, say so on stderr. Never fails the capture.
    if let Ok(Some(advisory)) = pointbreak::session::family_link_advisory(&args.repo) {
        let _ = writeln!(stderr, "{advisory}");
    }
    // `capture_document` consumes the result by value; keep a clone for the text lane.
    let text_source = capture.clone();
    let document = capture_document(capture);
    let format = output::resolve_format(args.format_args.explicit(), output::OutputFormat::Json)?;
    output::write_document(stdout, format, &document, || {
        render_capture_text(&text_source)
    })
}

/// Text capture ack: a few-line confirmation shaped on the inspector's
/// revision-page header — revision short ref, base -> target, diffstat, event
/// counts. Renders from the public `CaptureResult`; wording is disposable.
fn render_capture_text(result: &CaptureResult) -> String {
    let stat = &result.diffstat;

    let statuses: Vec<String> = [
        (stat.added_files, "added"),
        (stat.modified_files, "modified"),
        (stat.deleted_files, "deleted"),
        (stat.renamed_files, "renamed"),
        (stat.copied_files, "copied"),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, label)| format!("{count} {label}"))
    .collect();

    let file_word = if stat.file_count == 1 {
        "file"
    } else {
        "files"
    };
    let mut diff_line = format!("{} {file_word}", stat.file_count);
    if !statuses.is_empty() {
        diff_line.push_str(&format!(" ({})", statuses.join(", ")));
    }
    diff_line.push_str(&format!(" · +{}/−{}", stat.added_lines, stat.removed_lines));
    if stat.binary_files > 0 {
        diff_line.push_str(&format!(" · {} binary", stat.binary_files));
    }
    if stat.mode_only_files > 0 {
        diff_line.push_str(&format!(" · {} mode-only", stat.mode_only_files));
    }

    [
        format!(
            "captured {} · base {} → {}",
            output::short_ref(result.revision_id.as_str()),
            endpoint_label(&result.base),
            endpoint_label(&result.target),
        ),
        diff_line,
        format!(
            "events: {} created, {} existing",
            result.events_created, result.events_existing
        ),
    ]
    .join("\n")
}

/// Short readable label for a capture endpoint, matching the document's endpoint
/// vocabulary (commit vs. working tree).
fn endpoint_label(endpoint: &ReviewEndpoint) -> String {
    match endpoint {
        ReviewEndpoint::GitCommit { commit_oid, .. } => {
            format!("{} (commit)", output::short_ref(commit_oid))
        }
        ReviewEndpoint::GitTree { tree_oid } => {
            format!("{} (tree)", output::short_ref(tree_oid))
        }
        ReviewEndpoint::GitIndex { tree_oid } => {
            format!("{} (index)", output::short_ref(tree_oid))
        }
        ReviewEndpoint::GitWorkingTree { .. } => "worktree".to_owned(),
    }
}

fn capture_options(
    args: &CaptureArgs,
    tracing: &TracingArgs,
    stderr: &mut dyn Write,
) -> Result<(CaptureOptions, crate::cli::common::SigningSkip), Box<dyn std::error::Error>> {
    let mut options = CaptureOptions::new(&args.repo);
    if args.root {
        options = options.with_root_commit(root_commit_spec(args));
    } else if args.staged {
        options = options.with_staged(StagedSpec::new());
    } else if args.unstaged {
        options = options.with_unstaged(unstaged_spec(args));
    } else if let Some(range) = commit_range_spec(args) {
        options = options.with_commit_range(range);
    } else if args.include_untracked {
        options = options.with_worktree(WorktreeSpec::new().with_include_untracked());
    }
    if !args.supersedes.is_empty() {
        let ids = crate::cli::id_resolver::IdResolver::new(&args.repo);
        let mut supersedes = Vec::with_capacity(args.supersedes.len());
        for raw in &args.supersedes {
            supersedes.push(RevisionId::new(ids.rev(raw)?));
        }
        options = options.with_supersedes(supersedes);
    }
    if !args.paths.is_empty() {
        options = options.with_pathspecs(args.paths.clone());
    }
    if args.allow_empty {
        options = options.with_allow_empty();
    }
    if let Some(log_file) = &tracing.log_file {
        options = options.with_excluded_helper_path(log_file);
    }
    let mut skip = None;
    if let Some(resolved) =
        crate::cli::common::resolve_and_surface_signer(&args.repo, args.sign_key.as_deref(), stderr)
    {
        let (signed, signer_skip) = crate::cli::common::apply_resolved_signer(options, resolved);
        options = signed;
        skip = signer_skip;
    }
    Ok((options, skip))
}

/// Build the commit-range spec from `--base`/`--target`. `None` keeps the
/// default worktree capture. `--target` without `--base` or `--root` is rejected
/// in `run` before this point.
fn commit_range_spec(args: &CaptureArgs) -> Option<CommitRangeSpec> {
    let base = args.base.as_ref()?;
    let mut range = CommitRangeSpec::new(base.clone());
    if let Some(target) = &args.target {
        range = range.with_target_rev(target.clone());
    }
    Some(range)
}

fn root_commit_spec(args: &CaptureArgs) -> RootCommitSpec {
    let mut root = RootCommitSpec::new();
    if let Some(target) = &args.target {
        root = root.with_target_rev(target.clone());
    }
    root
}

fn unstaged_spec(args: &CaptureArgs) -> UnstagedSpec {
    let mut unstaged = UnstagedSpec::new();
    if args.include_untracked {
        unstaged = unstaged.with_include_untracked();
    }
    unstaged
}
