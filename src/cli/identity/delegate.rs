use std::io::Write;
use std::path::PathBuf;

use clap::Args;
use serde::Serialize;
use shoreline::model::ActorId;
use shoreline::session::{
    DELEGATES_LOCAL_REL_PATH, DELEGATES_REL_PATH, DelegationMap, DelegationStageOutcome,
    DelegationWriteRecord, ensure_shore_gitignore, now_rfc3339_utc, stage_delegation,
};

use crate::cli::json::DiagnosticDocument;
use crate::cli::output;

#[derive(Debug, Args)]
pub(super) struct DelegateArgs {
    /// Agent actor id to enroll (`actor:agent:<name>`).
    agent: String,
    /// Responsible principal — the human/non-agent actor that answers for the agent.
    #[arg(long)]
    principal: String,
    /// Window start (RFC 3339 UTC). Defaults to now.
    #[arg(long)]
    from: Option<String>,
    /// Window end (RFC 3339 UTC). Defaults to an open window.
    #[arg(long)]
    until: Option<String>,
    /// Free-text comment for diff readers (never authority).
    #[arg(long)]
    comment: Option<String>,
    /// Stage the private `.shore/delegates.local.json` override instead of the
    /// committed file. The local entry FULLY REPLACES the committed records for this
    /// agent on this machine (git-config style), and is git-excluded.
    #[arg(long)]
    local: bool,
    /// Repository root or a path inside it whose worktree-root `.shore/` receives the entry.
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    /// Pretty-print the JSON response.
    #[arg(long)]
    pretty: bool,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DelegateBody {
    agent: String,
    principal: String,
    valid_from: String,
    valid_until: Option<String>,
    comment: Option<String>,
    path: String,
    local: bool,
    added: bool,
    /// How many committed records for this agent the local override shadows (INV-E).
    /// Present only for `--local`.
    #[serde(skip_serializing_if = "Option::is_none")]
    local_shadows_committed: Option<usize>,
}

pub(super) fn run(
    args: DelegateArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let agent = ActorId::new(&args.agent);
    let principal = ActorId::new(&args.principal);
    let valid_from = args.from.clone().unwrap_or_else(now_rfc3339_utc); // INV-C
    let record = DelegationWriteRecord::new(principal.clone(), valid_from.clone())
        .with_valid_until(args.until.clone())
        .with_comment(args.comment.clone());

    // Resolve the worktree root so a subdir enroll lands at the root file the reader
    // looks for (the same call keys/enroll.rs makes).
    let worktree_root =
        shoreline::git::git_worktree_root(&args.repo).unwrap_or_else(|_| args.repo.clone());

    let rel = if args.local {
        DELEGATES_LOCAL_REL_PATH
    } else {
        DELEGATES_REL_PATH
    };
    let path = worktree_root.join(rel);

    // INV-E: a local override is git-excluded before it is written.
    let mut local_shadows_committed = None;
    if args.local {
        ensure_shore_gitignore(&worktree_root)?;
        // Count committed records that this local entry will shadow (full-replace).
        let committed_path = worktree_root.join(DELEGATES_REL_PATH);
        if committed_path.exists()
            && let Ok(map) = DelegationMap::from_delegates_file(&committed_path)
        {
            let n = map.record_count_for(&agent);
            if n > 0 {
                local_shadows_committed = Some(n);
                let _ = writeln!(
                    stderr,
                    "note: this local entry replaces {n} committed record(s) for {} locally",
                    agent.as_str()
                );
            }
        }
    }

    let DelegationStageOutcome { added } = stage_delegation(&path, &agent, &record)?;

    let _ = writeln!(
        stderr,
        "staged {}; review and `git commit` it to authorize.",
        path.display()
    );

    let body = DelegateBody {
        agent: agent.as_str().to_owned(),
        principal: principal.as_str().to_owned(),
        valid_from,
        valid_until: args.until,
        comment: args.comment,
        path: path.display().to_string(),
        local: args.local,
        added,
        local_shadows_committed,
    };
    let document = DiagnosticDocument::new("shore.identity-enroll", body, Vec::new());
    let format = output::resolve_format(
        args.format_args.explicit(args.pretty),
        output::OutputFormat::Json,
    )?;
    output::write_document_json_fallback(stdout, format, &document)
}
