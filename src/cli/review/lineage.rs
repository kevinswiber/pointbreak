use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand};
use shoreline::documents::{lineage_attach_document, lineage_show_document};
use shoreline::model::{ReviewUnitLineageId, RevisionId};
use shoreline::session::{
    LineageAttachOptions, LineageShowOptions, attach_review_unit_to_lineage, show_lineage,
};

use crate::cli::json;

#[derive(Debug, Args)]
pub(super) struct LineageArgs {
    #[command(subcommand)]
    command: LineageCommand,
}

#[derive(Debug, Subcommand)]
enum LineageCommand {
    Attach(LineageAttachArgs),
    Show(LineageShowArgs),
}

#[derive(Debug, Args)]
struct LineageAttachArgs {
    /// Repository root or a path inside the repository.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// ReviewUnit lineage ID.
    #[arg(long)]
    lineage: String,

    /// Captured ReviewUnit to attach as a lineage round.
    #[arg(long)]
    review_unit: String,

    /// Previous captured ReviewUnit in this lineage.
    #[arg(long)]
    predecessor: Option<String>,

    /// Optional Change-Id metadata to record on the lineage round.
    #[arg(long)]
    change_id: Option<String>,
}

#[derive(Debug, Args)]
struct LineageShowArgs {
    /// Repository root or a path inside the repository.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// ReviewUnit lineage ID.
    #[arg(long)]
    lineage: String,

    /// Pretty-print the JSON response.
    #[arg(long, conflicts_with = "compact")]
    pretty: bool,

    /// Emit compact JSON explicitly.
    #[arg(long)]
    compact: bool,
}

pub(super) fn run(
    args: LineageArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        LineageCommand::Attach(args) => {
            let span = tracing::info_span!("shore.review.lineage.attach");
            let _entered = span.enter();
            tracing::debug!(command = "review.lineage.attach", "command_start");
            review_lineage_attach(args, stdout)
        }
        LineageCommand::Show(args) => {
            let span = tracing::info_span!("shore.review.lineage.show");
            let _entered = span.enter();
            tracing::debug!(command = "review.lineage.show", "command_start");
            review_lineage_show(args, stdout)
        }
    }
}

fn review_lineage_attach(
    args: LineageAttachArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = attach_review_unit_to_lineage(lineage_attach_options(args));
    let document = lineage_attach_document(result?);
    json::write_json(stdout, &document, false)
}

fn review_lineage_show(
    args: LineageShowArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = args.pretty && !args.compact;
    let lineage_id = ReviewUnitLineageId::new(args.lineage);
    let result = show_lineage(LineageShowOptions::new(&args.repo, lineage_id));
    let document = lineage_show_document(result?);
    json::write_json(stdout, &document, pretty)
}

fn lineage_attach_options(args: LineageAttachArgs) -> LineageAttachOptions {
    let mut options = LineageAttachOptions::new(&args.repo, ReviewUnitLineageId::new(args.lineage))
        .with_review_unit_id(RevisionId::new(args.review_unit));
    if let Some(predecessor) = args.predecessor {
        options = options.with_predecessor_review_unit_id(RevisionId::new(predecessor));
    }
    if let Some(change_id) = args.change_id {
        options = options.with_change_id(change_id);
    }
    options
}
