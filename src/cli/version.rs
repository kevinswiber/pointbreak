use std::io::Write;

use clap::Args;
use pointbreak::documents::{VersionBody, version_document};

use crate::cli::output::{self, FormatArgs};

/// Report CLI and document versions for compatibility checks.
#[derive(Debug, Args)]
pub(super) struct VersionArgs {
    #[command(flatten)]
    format: FormatArgs,
}

pub(super) fn run(
    args: VersionArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let format = output::resolve_format(args.format.explicit(), output::OutputFormat::Json)?;
    let document = version_document();
    let text_source =
        matches!(format.format, output::OutputFormat::Text).then(|| document.body().clone());
    output::write_document(stdout, format, &document, || {
        render_version_text(
            text_source
                .as_ref()
                .expect("text lane resolves the version source"),
        )
    })
}

fn render_version_text(body: &VersionBody) -> String {
    format!(
        "shore {}\ndocuments: {}",
        body.cli_version,
        body.documents.len()
    )
}
