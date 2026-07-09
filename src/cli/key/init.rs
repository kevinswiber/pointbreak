use std::io::Write;
use std::path::PathBuf;

use clap::Args;
// Public keystore surface (the `store` submodule is private; the CLI is in the
// binary crate and reaches only the re-exported `pub` items at `pointbreak::keys::*`).
use pointbreak::keys::generate_key;

use crate::cli::{json, output};

#[derive(Debug, Args)]
pub(super) struct InitArgs {
    /// Label for the generated key (its filename stem in the keystore).
    #[arg(long, default_value = "default")]
    name: String,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct InitBody {
    name: String,
    did_key: String,
    path: PathBuf,
}

pub(super) fn run(
    args: InitArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    // `generate_key` validates the name via `KeyName::parse` before it becomes a
    // filename; an invalid `--name` (path separators, `..`, leading dot, …) surfaces
    // here as a clean non-zero-exit CLI error, never a path escape.
    let handle = generate_key(&args.name)?;
    let body = InitBody {
        name: args.name,
        did_key: handle.signer_id().as_str().to_owned(),
        path: handle.private_key_path().to_owned(),
    };
    let document = json::DiagnosticDocument::new("pointbreak.key-init", body, vec![]);
    let format = output::resolve_format(args.format_args.explicit(), output::OutputFormat::Json)?;
    output::write_document_json_fallback(stdout, format, &document)
}
