use std::io::Write;
use std::path::PathBuf;

use clap::{Args, ValueEnum};
use shoreline::documents::unit_list_document;
use shoreline::session::{OrphanVisibility, RefFilterMode, RevisionListOptions, list_revisions};

use crate::cli::json;

#[derive(Debug, Args)]
pub(super) struct RevisionsArgs {
    /// Repository root or a path inside the repository.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// List only the revisions sharing this content object id (a listing lens —
    /// coincident content may span threads; never a head-selector).
    #[arg(long)]
    object: Option<String>,

    /// Filter to revisions associated with this ref; a short branch name is
    /// normalized to its full ref before matching.
    #[arg(long = "ref", alias = "branch")]
    ref_name: Option<String>,

    /// How `--ref` matches: by the recorded label (offline) or by reachability
    /// from the ref's live tip.
    #[arg(long, value_enum, default_value = "label")]
    by: RefFilterByArg,

    /// Show revisions even when every anchored commit is unreachable (orphaned).
    #[arg(long)]
    all: bool,

    /// Show only orphaned revisions (every anchored commit unreachable). Takes
    /// precedence over `--all`.
    #[arg(long)]
    orphans: bool,

    /// Reachability target for the "merged" status: a revision is merged only when
    /// an ancestor of this ref. Defaults to broad reachability (any live tip).
    #[arg(long = "integration-ref")]
    integration_ref: Option<String>,

    /// Scope the listing to captures belonging to the worktree at this path.
    #[arg(long)]
    worktree: Option<PathBuf>,

    /// Pretty-print the JSON response.
    #[arg(long, conflicts_with = "compact")]
    pretty: bool,

    /// Emit compact JSON explicitly.
    #[arg(long)]
    compact: bool,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum RefFilterByArg {
    #[default]
    Label,
    Liveness,
}

impl From<RefFilterByArg> for RefFilterMode {
    fn from(by: RefFilterByArg) -> Self {
        match by {
            RefFilterByArg::Label => RefFilterMode::Label,
            RefFilterByArg::Liveness => RefFilterMode::Liveness,
        }
    }
}

pub(super) fn run(
    args: RevisionsArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let span = tracing::info_span!("shore.review.revisions");
    let _entered = span.enter();
    tracing::debug!(command = "review.revisions", "command_start");

    let pretty = args.pretty;
    let mut options = RevisionListOptions::new(&args.repo);
    if let Some(ref_name) = args.ref_name {
        options = options.with_ref_filter(ref_name, args.by.into());
    }
    let visibility = if args.orphans {
        OrphanVisibility::OrphansOnly
    } else if args.all {
        OrphanVisibility::All
    } else {
        OrphanVisibility::HideOrphans
    };
    options = options.with_orphan_visibility(visibility);
    if let Some(integration_ref) = args.integration_ref {
        options = options.with_integration_ref(integration_ref);
    }
    if let Some(worktree) = args.worktree {
        options = options.with_worktree_scope(worktree);
    }
    let mut result = list_revisions(options)?;

    // `--object` is a listing lens: filter to revisions over the same content
    // object id (coincident content, which may span threads). It never resolves a
    // head and never force-disambiguates.
    if let Some(object) = args.object.as_deref() {
        result
            .entries
            .retain(|entry| entry.object_id.as_str() == object);
        result.revision_count = result.entries.len();
    }

    let document = unit_list_document(result);
    json::write_json(stdout, &document, pretty)
}
