use std::io::Write;
use std::path::PathBuf;

use clap::Args;
use serde::Serialize;
use shoreline::crypto::{EventSigner as _, SignerId};
use shoreline::keys::load_signer;
use shoreline::model::ActorId;
use shoreline::session::{
    ALLOWED_SIGNERS_REL_PATH, EnrollmentDiff, resolve_writer_actor_id, stage_enrollment,
};

use crate::cli::json::{self, DiagnosticDocument};

#[derive(Debug, Args)]
pub(super) struct EnrollArgs {
    /// Repository root or a path inside the repository whose working-tree
    /// `.shore/allowed-signers.json` receives the entry.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Local key name to enroll. Defaults to `default`.
    #[arg(default_value = "default")]
    name: String,

    /// Actor id to bind the key to. Defaults to the resolved writing actor
    /// (`SHORE_ACTOR_ID` or the local Git identity).
    #[arg(long)]
    actor: Option<String>,

    /// Pretty-print the JSON response.
    #[arg(long)]
    pretty: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EnrollBody {
    actor_id: String,
    signer_id: String,
    path: String,
    added: bool,
}

pub(super) fn run(
    args: EnrollArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    // Resolve the key's did:key. `load_signer` loads the named key as a production
    // signer; its `signer_id()` is the did:key.
    let signer = load_signer(&args.name)?;
    let signer_id: &SignerId = signer.signer_id();

    // Resolve the actor: explicit `--actor` wins, else the standard writer
    // resolution (`SHORE_ACTOR_ID` then Git identity).
    let actor = resolve_actor(&args);

    // Possession-style: stage the working-tree edit only. The human's commit is
    // the authorization; this never invokes git. Resolve the worktree root first
    // (the same way trust discovery does) so enrollment from a subdirectory lands
    // at the root `.shore/allowed-signers.json` the reader looks for — not an
    // invisible `<subdir>/.shore/allowed-signers.json`.
    let worktree_root =
        shoreline::git::git_worktree_root(&args.repo).unwrap_or_else(|_| args.repo.clone());
    let path = worktree_root.join(ALLOWED_SIGNERS_REL_PATH);
    let EnrollmentDiff { added } = stage_enrollment(&path, &actor, signer_id)?;

    let body = EnrollBody {
        actor_id: actor.as_str().to_owned(),
        signer_id: signer_id.as_str().to_owned(),
        path: path.display().to_string(),
        added,
    };
    let document = DiagnosticDocument::new("shore.keys-enroll", body, Vec::new());
    json::write_json(stdout, &document, args.pretty)
}

/// Resolve the actor to bind: `--actor` (validated by the writer's id check) else
/// the standard writer resolution, via the public `resolve_writer_actor_id` (the
/// same `SHORE_ACTOR_ID`-then-Git path every write command uses).
fn resolve_actor(args: &EnrollArgs) -> ActorId {
    let explicit = args.actor.as_deref().map(ActorId::new);
    resolve_writer_actor_id(&args.repo, explicit.as_ref())
}
