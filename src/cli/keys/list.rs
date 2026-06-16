use std::io::Write;
use std::path::{Path, PathBuf};

use clap::Args;
use shoreline::keys::list_keys;
use shoreline::session::TrustSet;

use crate::cli::json;

#[derive(Debug, Args)]
pub(super) struct ListArgs {
    /// Repository whose committed allowed-signers file determines enrollment.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    pretty: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct KeyEntry {
    name: String,
    did_key: String,
    default: bool,
    enrolled: bool,
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
    let trust_set = inline_trust_set(&args.repo);
    let keys = list_keys()?
        .into_iter()
        .map(|info| {
            let did_key = info.signer_id().clone();
            KeyEntry {
                default: info.name() == "default",
                enrolled: trust_set
                    .as_ref()
                    .is_some_and(|trust| trust.contains_signer(&did_key)),
                did_key: did_key.as_str().to_owned(),
                name: info.name().to_owned(),
            }
        })
        .collect();
    let document = json::DiagnosticDocument::new("shore.keys-list", ListBody { keys }, vec![]);
    json::write_json(stdout, &document, args.pretty)
}

// Provisional inline loader: the shared `discover_trust_set` reader threaded
// through the verifying read paths replaces this once it lands.
fn inline_trust_set(repo: &Path) -> Option<TrustSet> {
    let worktree_root =
        shoreline::git::git_worktree_root(repo).unwrap_or_else(|_| repo.to_path_buf());
    let path = worktree_root.join(".shore/allowed-signers.json");
    if !path.exists() {
        return None;
    }
    TrustSet::from_allowed_signers_file(&path).ok()
}
