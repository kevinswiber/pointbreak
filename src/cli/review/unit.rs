use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand};
use shoreline::documents::{unit_list_document, unit_show_document};
use shoreline::model::{ReviewUnitId, ReviewUnitLineageId};
use shoreline::session::{
    EventVerificationPolicy, ReviewUnitListOptions, ReviewUnitShowOptions, list_review_units,
    show_review_unit,
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

    /// Pretty-print the JSON response.
    #[arg(long, conflicts_with = "compact")]
    pretty: bool,

    /// Emit compact JSON explicitly.
    #[arg(long)]
    compact: bool,
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
    let result = list_review_units(ReviewUnitListOptions::new(&args.repo))?;
    let document = unit_list_document(result);
    json::write_json(stdout, &document, pretty)
}

fn review_unit_show_command(
    args: UnitShowArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = args.pretty;
    let result = show_review_unit(review_unit_show_options(&args));
    let document = unit_show_document(result?);
    json::write_json(stdout, &document, pretty)
}

fn review_unit_show_options(args: &UnitShowArgs) -> ReviewUnitShowOptions {
    let mut options = ReviewUnitShowOptions::new(&args.repo).with_include_body(args.include_body);
    if let Some(review_unit) = &args.review_unit {
        options = options.with_review_unit_id(ReviewUnitId::new(review_unit.clone()));
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
