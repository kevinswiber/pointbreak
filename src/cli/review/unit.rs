use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use shoreline::documents::{unit_list_document, unit_show_document};
use shoreline::model::{ReviewUnitLineageId, RevisionId};
use shoreline::session::{
    EventVerificationPolicy, OrphanVisibility, RefFilterMode, ReviewUnitListOptions,
    ReviewUnitShowOptions, enrich_liveness, list_review_units, show_review_unit,
};

use crate::cli::json;

#[derive(Debug, Args)]
pub(super) struct UnitArgs {
    #[command(subcommand)]
    command: UnitCommand,
}

#[derive(Debug, Subcommand)]
enum UnitCommand {
    List(UnitListArgs),
    Show(UnitShowArgs),
}

#[derive(Debug, Args)]
pub(super) struct UnitListArgs {
    /// Repository root or a path inside the repository.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Filter to units associated with this ref; a short branch name is
    /// normalized to its full ref before matching.
    #[arg(long = "ref", alias = "branch")]
    ref_name: Option<String>,

    /// How `--ref` matches: by the recorded label (offline) or by reachability
    /// from the ref's live tip.
    #[arg(long, value_enum, default_value = "label")]
    by: RefFilterByArg,

    /// Show units even when every anchored commit is unreachable (orphaned).
    #[arg(long)]
    all: bool,

    /// Show only orphaned units (every anchored commit unreachable). Takes
    /// precedence over `--all`.
    #[arg(long)]
    orphans: bool,

    /// Reachability target for the "merged" status: a unit is merged only when an
    /// ancestor of this ref. Defaults to broad reachability (any live tip).
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

#[derive(Debug, Args)]
pub(super) struct UnitShowArgs {
    /// Repository root or a path inside the repository.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Select one ReviewUnit by id.
    #[arg(long)]
    review_unit: Option<String>,

    /// Select the current head of one ReviewUnit lineage.
    #[arg(long)]
    lineage: Option<String>,

    /// Filter narrative facts to one review track.
    #[arg(long)]
    track: Option<String>,

    /// Hydrate body-like text from inline payloads or body artifacts.
    #[arg(long)]
    include_body: bool,

    /// Pretty-print the JSON response.
    #[arg(long, conflicts_with = "compact")]
    pretty: bool,

    /// Emit compact JSON explicitly.
    #[arg(long)]
    compact: bool,
}

pub(super) fn run(
    args: UnitArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        UnitCommand::List(args) => {
            let span = tracing::info_span!("shore.review.unit.list");
            let _entered = span.enter();
            tracing::debug!(command = "review.unit.list", "command_start");
            review_unit_list_command(args, stdout)
        }
        UnitCommand::Show(args) => {
            let span = tracing::info_span!("shore.review.unit.show");
            let _entered = span.enter();
            tracing::debug!(command = "review.unit.show", "command_start");
            review_unit_show_command(args, stdout)
        }
    }
}

fn review_unit_list_command(
    args: UnitListArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = args.pretty;
    let mut options = ReviewUnitListOptions::new(&args.repo);
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
    let result = list_review_units(options)?;
    let document = unit_list_document(result);
    json::write_json(stdout, &document, pretty)
}

fn review_unit_show_command(
    args: UnitShowArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = args.pretty;
    let result = show_review_unit(review_unit_show_options(&args))?;

    // Liveness (merged/live/orphaned per OID + headline) is layered here, outside
    // the git-free document workflow: best-effort, omitted when reachability is
    // unknown.
    let liveness = enrich_liveness(&result.commit_range, &args.repo, None).ok();
    let document = unit_show_document(result);
    let mut value = serde_json::to_value(&document)?;
    if let Some(liveness) = liveness
        && let Some(commit_range) = value
            .get_mut("commitRange")
            .and_then(|cr| cr.as_object_mut())
    {
        commit_range.insert("liveness".to_owned(), serde_json::to_value(liveness)?);
    }
    json::write_json(stdout, &value, pretty)
}

fn review_unit_show_options(args: &UnitShowArgs) -> ReviewUnitShowOptions {
    let mut options = ReviewUnitShowOptions::new(&args.repo).with_include_body(args.include_body);
    if let Some(review_unit) = &args.review_unit {
        options = options.with_review_unit_id(RevisionId::new(review_unit.clone()));
    }
    if let Some(lineage) = &args.lineage {
        options = options.with_lineage_id(ReviewUnitLineageId::new(lineage.clone()));
    }
    if let Some(track) = &args.track {
        options = options.with_track(track.clone());
    }
    if let Some(map) = super::common::discover_delegation_map(&args.repo) {
        options = options.with_delegation_map(map);
    }
    // Advisory policy + reader trust: enable the per-event verificationStatus +
    // endorsement readback, reader-relative; render-only, never a gate.
    options = options
        .with_trust_set(super::common::discover_trust_set(&args.repo))
        .with_verification_policy(EventVerificationPolicy::advisory())
        .with_actor_attributes(super::common::discover_actor_attributes(&args.repo));
    options
}
