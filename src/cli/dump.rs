use std::io::Write;

use clap::Args;
use shoreline::dump::DumpDocument;

use crate::cli::input::{self, ReviewInputArgs};
use crate::cli::output;
use crate::cli_tracing::TracingArgs;

/// Emit the JSON review-stream document over the live working-tree diff and imported notes.
#[derive(Debug, Args)]
pub(super) struct DumpArgs {
    #[command(flatten)]
    pub(super) input: ReviewInputArgs,

    #[arg(long, conflicts_with = "compact")]
    pub(super) pretty: bool,

    #[arg(long)]
    pub(super) compact: bool,

    #[command(flatten)]
    pub(super) format_args: output::FormatArgs,
}

pub(super) fn run(
    args: DumpArgs,
    tracing: &TracingArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let span = tracing::info_span!("shore.dump");
    let _entered = span.enter();
    let document = document_for_dump(&args, tracing)?;
    let format = output::resolve_format(
        args.format_args.explicit(should_pretty_print(&args)),
        output::OutputFormat::Json,
    )?;
    output::write_document_json_fallback(stdout, format, &document)
}

pub(super) fn document_for_dump(
    args: &DumpArgs,
    tracing: &TracingArgs,
) -> shoreline::error::Result<DumpDocument> {
    input::load_dump_document(&args.input, input::dump_options(&args.input, tracing))
}

fn should_pretty_print(args: &DumpArgs) -> bool {
    args.pretty && !args.compact
}
