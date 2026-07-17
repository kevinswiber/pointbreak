use std::io::Write;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use clap::Args;
use pointbreak::keys::load_signer_id;

use crate::cli::{json, output};

#[derive(Debug, Args)]
pub(super) struct ShowArgs {
    /// Name of the key to display (defaults to `default`).
    #[arg(default_value = "default")]
    name: String,

    /// Include the key's did:key (the default when no field flag is given).
    #[arg(long)]
    did: bool,

    /// Include the key's raw Ed25519 public key (base64).
    #[arg(long)]
    pubkey: bool,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ShowBody {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    did_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    public_key: Option<String>,
}

pub(super) fn run(
    args: ShowArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    // Resolve the did:key from public material so an agent-backed reference (no
    // private seed on disk) shows offline, like `list`/`enroll`. The `--pubkey`
    // bytes derive from the same did:key.
    let signer_id = load_signer_id(&args.name)?;

    // Default to the did:key when neither field flag is set.
    let want_did = args.did || !args.pubkey;
    let want_pubkey = args.pubkey;

    let did_key = want_did.then(|| signer_id.as_str().to_owned());
    let public_key = want_pubkey
        .then(|| signer_id.ed25519_public_key())
        .transpose()?
        .map(|bytes| BASE64.encode(bytes));

    let body = ShowBody {
        name: args.name,
        did_key,
        public_key,
    };
    let format = output::resolve_format(args.format_args.explicit(), output::OutputFormat::Json)?;
    let text =
        matches!(format.format, output::OutputFormat::Text).then(|| render_key_show_text(&body));
    let document = json::DiagnosticDocument::new("pointbreak.key-show", body, vec![]);
    output::write_document(stdout, format, &document, || {
        text.expect("text lane resolves the digest source")
    })
}

/// Bespoke text lane for `key show`: the key name plus whichever identity
/// fields the flags selected, on one line.
fn render_key_show_text(body: &ShowBody) -> String {
    let mut parts = vec![body.name.clone()];
    if let Some(did_key) = &body.did_key {
        parts.push(did_key.clone());
    }
    if let Some(public_key) = &body.public_key {
        parts.push(format!("pubkey {public_key}"));
    }
    parts.join(" · ")
}
