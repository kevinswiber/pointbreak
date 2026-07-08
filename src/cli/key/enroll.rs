use std::io::Write;
use std::path::PathBuf;

use clap::Args;
use pointbreak::crypto::SignerId;
use pointbreak::error::{Result as ShoreResult, ShoreError};
use pointbreak::keys::load_signer_id;
use pointbreak::model::ActorId;
use pointbreak::session::{
    ALLOWED_SIGNERS_REL_PATH, EnrollmentDiff, is_valid_actor_id, resolve_writer_actor_id,
    stage_enrollment,
};
use serde::Serialize;

use crate::cli::json::DiagnosticDocument;
use crate::cli::output;

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

    #[command(flatten)]
    format_args: output::FormatArgs,
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
    // Resolve the key's did:key from public material, so an agent-backed reference
    // (which has no private key on disk) enrolls offline — no agent, no seed. A seed
    // key resolves to the same did:key its loaded signer would.
    let signer_id: SignerId = load_signer_id(&args.name)?;

    // Resolve the actor: explicit `--actor` must be valid, else the standard
    // writer resolution (`SHORE_ACTOR_ID` then Git identity).
    let actor = resolve_actor(&args)?;

    // Possession-style: stage the working-tree edit only. The human's commit is
    // the authorization; this never invokes git. Resolve the worktree root first
    // (the same way trust discovery does) so enrollment from a subdirectory lands
    // at the root `.shore/allowed-signers.json` the reader looks for — not an
    // invisible `<subdir>/.shore/allowed-signers.json`.
    let worktree_root =
        pointbreak::git::git_worktree_root(&args.repo).unwrap_or_else(|_| args.repo.clone());
    let path = worktree_root.join(ALLOWED_SIGNERS_REL_PATH);
    let EnrollmentDiff { added } = stage_enrollment(&path, &actor, &signer_id)?;

    let body = EnrollBody {
        actor_id: actor.as_str().to_owned(),
        signer_id: signer_id.as_str().to_owned(),
        path: path.display().to_string(),
        added,
    };
    let document = DiagnosticDocument::new("pointbreak.key-enroll", body, Vec::new());
    let format = output::resolve_format(
        args.format_args.explicit(args.pretty),
        output::OutputFormat::Json,
    )?;
    output::write_document_json_fallback(stdout, format, &document)
}

/// Resolve the actor to bind: `--actor` is a strict command input, while a
/// missing flag keeps the standard writer resolution path every write command
/// uses.
fn resolve_actor(args: &EnrollArgs) -> ShoreResult<ActorId> {
    if let Some(raw_actor) = args.actor.as_deref() {
        let actor = raw_actor.trim();
        if !is_valid_actor_id(actor) {
            return Err(ShoreError::WorkflowInputInvalid {
                reason: format!(
                    "--actor {raw_actor:?} is not a valid actor id; expected \
                     actor:<scheme>:<value> (for example, actor:agent:codex) \
                     or a did:key signer id"
                ),
            });
        }
        return Ok(ActorId::new(actor.to_owned()));
    }

    Ok(resolve_writer_actor_id(&args.repo, None))
}
