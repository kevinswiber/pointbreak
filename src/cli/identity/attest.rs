use std::io::Write;
use std::path::PathBuf;

use clap::Args;
use pointbreak::model::ActorId;
use pointbreak::session::{
    ACTOR_ATTRIBUTES_LOCAL_REL_PATH, ACTOR_ATTRIBUTES_REL_PATH, ActorAttributesStageOutcome,
    ActorAttributesWriteRecord, ensure_shore_gitignore, stage_actor_attributes,
};
use serde::Serialize;

use crate::cli::json::DiagnosticDocument;
use crate::cli::output;

#[derive(Debug, Args)]
pub(super) struct AttestArgs {
    /// Actor id to describe (any persisted actor id, agent or not).
    actor: String,
    /// The actor's kind (lowercase-kebab; reserved well-known: human / agent /
    /// service / reviewer-model). Required: ADR-0012 mandates exactly one kind.
    #[arg(long)]
    kind: String,
    /// A role token (lowercase-kebab). Repeatable; deduped + sorted. NOT additive:
    /// re-attesting replaces this actor's entire roles set (per-actor replace).
    #[arg(long = "role")]
    roles: Vec<String>,
    /// Free-text comment for the human maintaining the file (not interpreted).
    #[arg(long)]
    comment: Option<String>,
    /// Stage the private `.shore/actor-attributes.local.json` override (git-excluded).
    /// The local entry FULLY REPLACES the committed entry for this actor on this machine.
    #[arg(long)]
    local: bool,
    /// Repository root or a path inside it whose worktree-root `.shore/` receives the entry.
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    #[command(flatten)]
    format_args: output::FormatArgs,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AttestBody {
    actor: String,
    kind: String,
    roles: Vec<String>,
    comment: Option<String>,
    path: String,
    local: bool,
    changed: bool,
}

pub(super) fn run(
    args: AttestArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let actor = ActorId::new(&args.actor);
    let attrs = ActorAttributesWriteRecord::new(args.kind.clone())
        .with_roles(args.roles.clone())
        .with_comment(args.comment.clone());

    let worktree_root =
        pointbreak::git::git_worktree_root(&args.repo).unwrap_or_else(|_| args.repo.clone());
    let rel = if args.local {
        ACTOR_ATTRIBUTES_LOCAL_REL_PATH
    } else {
        ACTOR_ATTRIBUTES_REL_PATH
    };
    let path = worktree_root.join(rel);

    if args.local {
        ensure_shore_gitignore(&worktree_root)?; // INV-E
        let _ = writeln!(
            stderr,
            "note: this local entry fully replaces any committed attributes for {} locally",
            actor.as_str()
        );
    }

    let ActorAttributesStageOutcome { changed } = stage_actor_attributes(&path, &actor, &attrs)?;

    let _ = writeln!(
        stderr,
        "staged {}; review and `git commit` it to apply.",
        path.display()
    );

    let body = AttestBody {
        actor: actor.as_str().to_owned(),
        kind: args.kind,
        roles: args.roles,
        comment: args.comment,
        path: path.display().to_string(),
        local: args.local,
        changed,
    };
    let document = DiagnosticDocument::new("pointbreak.identity-attest", body, Vec::new());
    let format = output::resolve_format(args.format_args.explicit(), output::OutputFormat::Json)?;
    output::write_document_json_fallback(stdout, format, &document)
}
