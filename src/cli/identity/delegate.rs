use std::io::Write;
use std::path::PathBuf;

use clap::Args;
use pointbreak::model::ActorId;
use pointbreak::session::{
    DelegationMap, DelegationStageOutcome, DelegationWriteRecord, ensure_pointbreak_gitignore,
    now_rfc3339_utc, stage_delegation,
};
use serde::Serialize;

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
    /// Stage the private `.pointbreak/delegates.local.json` override instead of the
    /// committed file. The local entry FULLY REPLACES the committed records for this
    /// agent on this machine (git-config style), and is git-excluded.
    #[arg(long)]
    local: bool,
    /// Repository root or a path inside it whose worktree-root `.pointbreak/` receives the entry.
    #[arg(long, default_value = ".")]
    repo: PathBuf,
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
        pointbreak::git::git_worktree_root(&args.repo).unwrap_or_else(|_| args.repo.clone());
    let paths = pointbreak::paths::RepositoryPaths::from_worktree_root(&worktree_root);
    let path = if args.local {
        paths.delegates_local()
    } else {
        paths.delegates()
    };

    // INV-E: a local override is git-excluded before it is written.
    let mut local_shadows_committed = None;
    if args.local {
        ensure_pointbreak_gitignore(&worktree_root)?;
        // Count committed records that this local entry will shadow (full-replace).
        let committed_path = paths.delegates();
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
    let format = output::resolve_format(args.format_args.explicit(), output::OutputFormat::Json)?;
    // Bespoke text lane: a one-line staging receipt; an idempotent re-run says so.
    let text = matches!(format.format, output::OutputFormat::Text)
        .then(|| render_identity_delegate_text(&body));
    let document = DiagnosticDocument::new("pointbreak.identity-delegate", body, Vec::new());
    output::write_document(stdout, format, &document, || {
        text.expect("text lane resolves the digest source")
    })
}

/// Bespoke text lane for `identity delegate`: who answers for whom, the window,
/// and the staged file (the human's commit is the authorization).
fn render_identity_delegate_text(body: &DelegateBody) -> String {
    if !body.added {
        return format!("already delegated: {} → {}", body.agent, body.principal);
    }
    let mut line = format!(
        "staged delegation: {} → {} · from {}",
        body.agent, body.principal, body.valid_from
    );
    if let Some(until) = &body.valid_until {
        line.push_str(&format!(" · until {until}"));
    }
    if body.local {
        line.push_str(" · local override");
        if let Some(shadows) = body.local_shadows_committed
            && shadows > 0
        {
            line.push_str(&format!(" (shadows {shadows} committed)"));
        }
    }
    line.push_str(&format!(" · commit {} to authorize", body.path));
    line
}
