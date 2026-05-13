use std::path::PathBuf;

use clap::Args;
use shore::dump::{DumpDocument, DumpOptions};

use crate::cli_tracing::TracingArgs;

#[derive(Clone, Debug, Args)]
pub(super) struct ReviewInputArgs {
    #[arg(long, default_value = ".")]
    pub(super) repo: PathBuf,

    #[arg(long)]
    pub(super) review_notes: Option<PathBuf>,
}

pub(super) fn load_dump_document(
    args: &ReviewInputArgs,
    options: DumpOptions,
) -> shore::error::Result<DumpDocument> {
    let document = match &args.review_notes {
        Some(review_notes) => {
            DumpDocument::from_review_notes_file_with_options(&args.repo, review_notes, options)?
        }
        None => DumpDocument::from_repo_with_options(&args.repo, options)?,
    };
    Ok(document)
}

pub(super) fn dump_options(args: &ReviewInputArgs, tracing: &TracingArgs) -> DumpOptions {
    let mut options = DumpOptions::new();
    if let Some(review_notes) = &args.review_notes {
        options = options.exclude_helper_path(review_notes);
    }
    if let Some(log_file) = &tracing.log_file {
        options = options.exclude_helper_path(log_file);
    }
    options
}
