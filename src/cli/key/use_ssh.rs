use std::io::Write;
use std::path::PathBuf;

use clap::Args;
// Public keystore surface (the `store`/`ssh` submodules are private; the CLI is in
// the binary crate and reaches only the re-exported `pub` items at
// `pointbreak::keys::*`).
use pointbreak::keys::{KeyName, parse_ssh_ed25519_public_key, write_agent_reference};

use crate::cli::{json, output};

#[derive(Debug, Args)]
pub(super) struct UseSshArgs {
    /// An SSH Ed25519 *public* key: a path to a `*.pub` file (the
    /// `ssh-ed25519 AAAA… [comment]` line form) OR a `key::ssh-ed25519 AAAA…`
    /// literal passed inline.
    key: String,

    /// Label for the adopted key (its filename stem in the keystore). Defaults to
    /// `default` so the adopted key becomes the user-default signer.
    #[arg(long, default_value = "default")]
    name: String,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UseSshBody {
    name: String,
    did_key: String,
    path: PathBuf,
}

pub(super) fn run(
    args: UseSshArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    // Read the key text: a `key::…` literal passes through verbatim; anything else
    // is a filesystem path whose contents are the `ssh-ed25519 AAAA… [comment]`
    // line. The parser is the single validator for either spelling.
    let key_text = if args.key.starts_with("key::") {
        args.key.clone()
    } else {
        std::fs::read_to_string(&args.key)
            .map_err(|error| format!("read public key {}: {error}", args.key))?
    };

    // Validate it is plain `ssh-ed25519`. A non-ed25519 key surfaces the parser's
    // typed error as a clean non-zero-exit CLI failure here — the explicit,
    // user-facing validation point (distinct from the resolve layer's silent
    // degradation for an already-stored key).
    let signer_id = parse_ssh_ed25519_public_key(&key_text)?;
    let public_key = signer_id.ed25519_public_key()?;

    // Validate the --name before it becomes a filename (no path escape), then
    // persist the agent-backed reference (refuse-to-clobber via create_new).
    let name = KeyName::parse(&args.name)?;
    let handle = write_agent_reference(name.as_str(), public_key)?;

    let body = UseSshBody {
        name: args.name,
        did_key: handle.signer_id().as_str().to_owned(),
        path: handle.private_key_path().to_owned(),
    };
    let document = json::DiagnosticDocument::new("pointbreak.key-use-ssh", body, vec![]);
    let format =
        output::resolve_format(args.format_args.explicit(false), output::OutputFormat::Json)?;
    output::write_document_json_fallback(stdout, format, &document)?;

    // Enrollment hint on stderr (the JSON document is the machine-readable stdout
    // contract; this is human guidance), mirroring the agent-keygen notice.
    eprintln!(
        "shore: adopted SSH key as {} ({}); run `shore key enroll` to stage trust",
        handle.name(),
        handle.signer_id().as_str()
    );
    Ok(())
}
