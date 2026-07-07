use std::io::Write;
use std::path::PathBuf;

use clap::Args;
use pointbreak::keys::{KeyCustody, agent_has_key, list_keys};

use crate::cli::common::discover_trust_set;
use crate::cli::{json, output};

#[derive(Debug, Args)]
pub(super) struct ListArgs {
    /// Repository whose committed allowed-signers file determines enrollment.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    pretty: bool,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct KeyEntry {
    name: String,
    did_key: String,
    default: bool,
    enrolled: bool,
    /// "file" (seed on disk) or "agent" (agent-backed reference).
    custody: &'static str,
    /// Best-effort, non-gating: `Some(true)` if a reachable ssh-agent currently
    /// holds this agent-backed key, `Some(false)` if reachable but absent, omitted
    /// for a file key OR when the agent is unreachable/locked/errored. A read
    /// command never fails on this probe.
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_loaded: Option<bool>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ListBody {
    keys: Vec<KeyEntry>,
}

pub(super) fn run(
    args: ListArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    // Single trust-set discovery path, shared with the verifying read commands.
    let trust_set = discover_trust_set(&args.repo);
    let keys = list_keys()?
        .into_iter()
        .map(|info| {
            let did_key = info.signer_id().clone();
            // For an agent-backed key, best-effort "is it loaded in a reachable
            // agent?" — the `pub` probe swallows every failure into `None`, so the
            // listing never gates on the agent. A file key omits the field.
            let (custody, agent_loaded) = match info.custody() {
                KeyCustody::File => ("file", None),
                KeyCustody::Agent => (
                    "agent",
                    did_key.ed25519_public_key().ok().and_then(agent_has_key),
                ),
            };
            KeyEntry {
                default: info.name() == "default",
                enrolled: trust_set.contains_signer(&did_key),
                custody,
                agent_loaded,
                did_key: did_key.as_str().to_owned(),
                name: info.name().to_owned(),
            }
        })
        .collect();
    let document = json::DiagnosticDocument::new("shore.keys-list", ListBody { keys }, vec![]);
    let format = output::resolve_format(
        args.format_args.explicit(args.pretty),
        output::OutputFormat::Json,
    )?;
    output::write_document_json_fallback(stdout, format, &document)
}
