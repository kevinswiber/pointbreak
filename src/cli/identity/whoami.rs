use std::io::Write;
use std::path::PathBuf;

use clap::Args;
use pointbreak::documents::identity_whoami_document;
use pointbreak::session::resolve_writer_actor_id;

use crate::cli::output::{self, FormatArgs, OutputFormat};

#[derive(Debug, Args)]
pub(super) struct WhoamiArgs {
    /// Repository whose writer identity should be resolved.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[command(flatten)]
    format: FormatArgs,
}

pub(super) fn run(
    args: WhoamiArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let format = output::resolve_format(args.format.explicit(), OutputFormat::Json)?;
    let document = identity_whoami_document(resolve_writer_actor_id(&args.repo, None));
    let text_source =
        matches!(format.format, OutputFormat::Text).then(|| document.body().actor_id().to_owned());
    output::write_document(stdout, format, &document, || {
        text_source
            .as_deref()
            .expect("text lane resolves the identity source")
            .to_owned()
    })
}
