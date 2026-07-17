use std::io::Write;
use std::path::PathBuf;

use clap::Args;
use pointbreak::crypto::SignerId;
use pointbreak::error::{Result as ShoreResult, ShoreError};
use pointbreak::keys::load_signer_id;
use pointbreak::model::ActorId;
use pointbreak::session::{
    EnrollmentDiff, allowed_signers_path_for_repo, is_valid_actor_id, resolve_writer_actor_id,
    stage_enrollment,
};
use serde::Serialize;

use crate::cli::json::DiagnosticDocument;
use crate::cli::output;

#[derive(Debug, Args)]
pub(super) struct EnrollArgs {
    /// Repository root or a path inside the repository whose working-tree
    /// `.pointbreak/allowed-signers.json` receives the entry.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Local key name to enroll. Defaults to `default` when `--signer` is absent.
    name: Option<String>,

    /// Explicit did:key signer id to enroll without reading the local keystore.
    #[arg(long, conflicts_with = "name")]
    signer: Option<String>,

    /// Actor id to bind the key to. Defaults to the resolved writing actor
    /// (`POINTBREAK_ACTOR_ID` or the local Git identity).
    #[arg(long)]
    actor: Option<String>,

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
    let signer_id = resolve_enrollment_signer(&args)?;

    // Resolve the actor: explicit `--actor` must be valid, else the standard
    // writer resolution (`POINTBREAK_ACTOR_ID` then Git identity).
    let actor = resolve_actor(&args)?;

    // Possession-style: stage the working-tree edit only. The human's commit is
    // the authorization; this never invokes git. Resolve the worktree root first
    // (the same way trust discovery does) so enrollment from a subdirectory lands
    // at the root `.pointbreak/allowed-signers.json` the reader looks for — not an
    // invisible `<subdir>/.pointbreak/allowed-signers.json`.
    let path = allowed_signers_path_for_repo(&args.repo)?;
    let EnrollmentDiff { added } = stage_enrollment(&path, &actor, &signer_id)?;

    let body = EnrollBody {
        actor_id: actor.as_str().to_owned(),
        signer_id: signer_id.as_str().to_owned(),
        path: path.display().to_string(),
        added,
    };
    let format = output::resolve_format(args.format_args.explicit(), output::OutputFormat::Json)?;
    let text =
        matches!(format.format, output::OutputFormat::Text).then(|| render_key_enroll_text(&body));
    let document = DiagnosticDocument::new("pointbreak.key-enroll", body, Vec::new());
    output::write_document(stdout, format, &document, || {
        text.expect("text lane resolves the digest source")
    })
}

/// Bespoke text lane for `key enroll`: a one-line receipt for the staged
/// working-tree edit (the human's commit is the authorization), or an
/// `already enrolled` line on an idempotent re-run.
fn render_key_enroll_text(body: &EnrollBody) -> String {
    if body.added {
        format!(
            "staged enrollment of {} for {} · commit {} to authorize",
            body.signer_id, body.actor_id, body.path
        )
    } else {
        format!("already enrolled: {} for {}", body.signer_id, body.actor_id)
    }
}

/// Resolve the signer to enroll. A direct `--signer` is already public trust
/// material; otherwise load the local key name's did:key from public material,
/// so agent-backed references enroll offline with no agent and no seed.
fn resolve_enrollment_signer(args: &EnrollArgs) -> ShoreResult<SignerId> {
    if let Some(raw_signer) = args.signer.as_deref() {
        return SignerId::parse(raw_signer).map_err(|error| ShoreError::WorkflowInputInvalid {
            reason: format!("--signer {raw_signer:?} is not a valid signer id: {error}"),
        });
    }

    load_signer_id(args.name.as_deref().unwrap_or("default"))
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
