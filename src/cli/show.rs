use clap::Args;
use shore::dump::DumpDocument;
use shore::stream::ViewportSpec;

use crate::cli::input::{self, ReviewInputArgs};
use crate::cli_tracing::TracingArgs;

#[derive(Debug, Args)]
pub(super) struct ShowArgs {
    #[command(flatten)]
    pub(super) input: ReviewInputArgs,
}

pub(super) fn run(args: ShowArgs, tracing: &TracingArgs) -> Result<(), Box<dyn std::error::Error>> {
    let document = document_for_show(&args, tracing)?;
    let input = args.input.clone();
    let tracing = tracing.clone();
    let viewport = ViewportSpec::new(80, 24);
    let app = crate::tui::app::TuiApp::new(document, viewport);
    let repo = input.repo.clone();
    let load_document =
        move || input::load_dump_document(&input, input::dump_options(&input, &tracing));
    crate::tui::terminal::run(app, &repo, load_document)
}

pub(super) fn document_for_show(
    args: &ShowArgs,
    tracing: &TracingArgs,
) -> shore::error::Result<DumpDocument> {
    input::load_dump_document(&args.input, input::dump_options(&args.input, tracing))
}
