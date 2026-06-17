use std::io::Write;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use clap::Args;
use shoreline::keys::load_signer_id;

use crate::cli::json;

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
    let document = json::DiagnosticDocument::new("shore.keys-show", body, vec![]);
    json::write_json(stdout, &document, false)
}
