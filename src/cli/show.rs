use clap::Args;
use shoreline::dump::DumpDocument;
use shoreline::stream::ViewportSpec;

use crate::cli::input::{self, ReviewInputArgs};
use crate::cli_tracing::TracingArgs;

/// Open the interactive terminal UI over the working-tree review stream.
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
    let load_document = move || {
        let span = tracing::info_span!("shore.show.reload");
        let _entered = span.enter();
        input::load_dump_document(&input, input::dump_options(&input, &tracing))
    };
    crate::tui::terminal::run(app, &repo, load_document)
}

pub(super) fn document_for_show(
    args: &ShowArgs,
    tracing: &TracingArgs,
) -> shoreline::error::Result<DumpDocument> {
    let span = tracing::info_span!("shore.show.load");
    let _entered = span.enter();
    input::load_dump_document(&args.input, input::dump_options(&args.input, tracing))
}
