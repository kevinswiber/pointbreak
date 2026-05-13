use std::io::Write;

use clap::Args;
use shore::dump::DumpDocument;

use crate::cli::input::{self, ReviewInputArgs};
use crate::cli::json;
use crate::cli_tracing::TracingArgs;

#[derive(Debug, Args)]
pub(super) struct DumpArgs {
    #[command(flatten)]
    pub(super) input: ReviewInputArgs,

    #[arg(long, conflicts_with = "compact")]
    pub(super) pretty: bool,

    #[arg(long)]
    pub(super) compact: bool,
}

pub(super) fn run(
    args: DumpArgs,
    tracing: &TracingArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let document = document_for_dump(&args, tracing)?;
    json::write_json(stdout, &document, should_pretty_print(&args))
}

pub(super) fn document_for_dump(
    args: &DumpArgs,
    tracing: &TracingArgs,
) -> shore::error::Result<DumpDocument> {
    input::load_dump_document(&args.input, input::dump_options(&args.input, tracing))
}

fn should_pretty_print(args: &DumpArgs) -> bool {
    args.pretty && !args.compact
}
