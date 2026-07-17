use std::io::Write;
use std::path::PathBuf;

use clap::{ArgGroup, Args, Subcommand, ValueEnum};
use pointbreak::model::{ObjectId, RevisionId};
use pointbreak::session::{
    CompactOptions, CompactResult, MigrateToCommonDirOptions, MigrateToCommonDirResult,
    ProjectionDiagnostic, RemovalOperativeStatus, RemoveOptions, RemoveResult, RemoveSelector,
    RemovedContent, SkippedRemoval, StoreForgetOptions, StoreForgetResult, StoreLinkOptions,
    StoreLinkPreview, StoreLinkResult, StoreListEntry, StoreListResult, StoreMode, StoreModeSource,
    StoreSensitivityPathGroup, StoreStatusInventory, StoreStatusOptions, StoreStatusResult,
    StoreStatusSensitivity, StoreUnlinkOptions, StoreUnlinkResult, SweepOutcome, SweptBlob,
    compact_store, explain_store_sensitivity, forget_family_store, link_store_to_family,
    list_family_stores, migrate_store_to_common_dir, preview_link_to_family, remove_content,
    resolve_store_mode_for_repo, set_store_mode_for_repo, store_paths_for_repo, store_status,
    unlink_store_from_family,
};

use crate::cli::common::{
    SigningSkip, apply_resolved_signer, count_label, discover_trust_set,
    resolve_and_surface_signer, surface_best_effort_skip,
};
use crate::cli::{json, output};

#[derive(Debug, Args)]
pub(super) struct StoreArgs {
    #[command(subcommand)]
    command: StoreCommand,
}

#[derive(Debug, Subcommand)]
enum StoreCommand {
    Status(StoreStatusArgs),
    Paths(StorePathsArgs),
    Mode(StoreModeArgs),
    Migrate(StoreMigrateArgs),
    Link(StoreLinkArgs),
    Unlink(StoreUnlinkArgs),
    Forget(StoreForgetArgs),
    List(StoreListArgs),
    Remove(StoreRemoveArgs),
    /// Alias of `compact`.
    Gc(StoreCompactArgs),
    Compact(StoreCompactArgs),
}

/// Show the store's status: mode, size, and sensitivity findings.
#[derive(Debug, Args)]
struct StoreStatusArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Also list the real worktree paths each sensitivity finding matched, so an
    /// excludeGlobs decision is actionable. Local-only, printed to your terminal.
    /// Text-only — it keeps the `pointbreak.store-status` JSON document uniformly
    /// path-free — so it cannot be combined with a JSON format.
    #[arg(long)]
    show_paths: bool,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

/// Report the operational paths selected by Pointbreak's shared path authorities.
#[derive(Debug, Args)]
struct StorePathsArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

/// Get or set the store's mode (shared or ephemeral).
#[derive(Debug, Args)]
struct StoreModeArgs {
    /// `shared`, `ephemeral`, or `show` (report the resolved mode without
    /// changing it).
    action: StoreModeAction,

    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

/// Fold a legacy per-worktree store into the shared common-dir store.
#[derive(Debug, Args)]
struct StoreMigrateArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Fan an ephemeral or sensitivity-flagged worktree's store into the shared
    /// store anyway. Off by default: such a worktree is refused without this flag.
    #[arg(long)]
    include_ephemeral: bool,

    /// After the fold is independently verified (every source event and artifact
    /// file present in the shared store), delete the worktree-local .pointbreak/data
    /// so reads resolve in one command. Off by default: the source is never
    /// discarded before the migration is confirmed.
    #[arg(long)]
    retire_source: bool,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

/// Promote this clone to the opt-in user-level family store tier.
#[derive(Debug, Args)]
struct StoreLinkArgs {
    /// Family slug placing this clone's store at `<pointbreak-home-root>/stores/<slug>/`.
    /// Omit to have the workflow suggest one from the repo; it never picks silently.
    slug: Option<String>,

    /// Link an Ephemeral-mode worktree anyway (refused by default).
    #[arg(long)]
    include_ephemeral: bool,

    /// Link a worktree the sensitivity gate flagged `block` anyway.
    #[arg(long)]
    include_sensitive: bool,

    /// After the fold is independently verified, delete the clone-local `.git/pointbreak` history.
    #[arg(long)]
    retire_source: bool,

    /// Preview gates 1–5 and the fold preflight without writing anything — no
    /// scaffold, no fold, no binding flip. Emits a `pointbreak.store-link-preview`
    /// document; exits non-zero with the first blocking reason if the link would
    /// not succeed.
    #[arg(long)]
    dry_run: bool,

    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

/// Detach this clone from its linked family store.
#[derive(Debug, Args)]
struct StoreUnlinkArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

/// The whole-store destructive verb: dry-run by default, `--yes` to execute.
#[derive(Debug, Args)]
struct StoreForgetArgs {
    /// Family slug to forget.
    slug: String,

    /// Perform the deletion. Without it (the default), forget previews and deletes nothing.
    #[arg(long)]
    yes: bool,

    /// Forget even when live clones are still registered.
    #[arg(long)]
    force: bool,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

/// List every user-level family store on this machine. Deliberately repo-less: no
/// `--repo` flag, and it never resolves a git repo or a per-clone store.
#[derive(Debug, Args)]
struct StoreListArgs {
    #[command(flatten)]
    format_args: output::FormatArgs,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum StoreModeAction {
    Shared,
    Ephemeral,
    Show,
}

/// Records a removal claim against content-addressed artifacts, scoped by exactly
/// one selector; it does not erase any bytes — run `pointbreak store compact` (or its
/// `gc` alias) to reclaim disk space afterward. The removal key is derived solely
/// from the content hash, so there is deliberately no `--idempotency-key`.
#[derive(Debug, Args)]
#[command(group(ArgGroup::new("selector").required(true).multiple(false)))]
struct StoreRemoveArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Remove a single snapshot's bound artifact.
    #[arg(long, group = "selector")]
    snapshot: Option<String>,
    /// Remove every artifact a revision references.
    #[arg(long, group = "selector")]
    revision: Option<String>,
    /// Remove artifacts of revisions anchored on the commit this ref resolves to.
    #[arg(long, group = "selector")]
    r#ref: Option<String>,
    /// Remove artifacts of revisions anchored on a commit in the `<a>..<b>` range.
    #[arg(long, group = "selector")]
    range: Option<String>,
    /// Remove artifacts of commit-anchored revisions whose commits are all orphaned.
    #[arg(long, group = "selector")]
    orphans: bool,

    /// Sign this write with a specific key: a keystore key name or a path to a
    /// key file. Removal is a write, so a signed store stays signed.
    #[arg(long)]
    sign_key: Option<String>,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

/// Physically delete artifacts already claimed for removal.
#[derive(Debug, Args)]
struct StoreCompactArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Preview the erase set and skipped removals; delete nothing.
    #[arg(long, conflicts_with = "yes")]
    dry_run: bool,

    /// Perform the erasure. Without it (and without `--dry-run`), compact
    /// previews and refuses — physical erasure is the point of no return.
    #[arg(long)]
    yes: bool,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StoreMigrateBody {
    events_created: usize,
    events_existing: usize,
    artifacts_created: usize,
    artifacts_existing: usize,
    source_empty: bool,
    source_retired: bool,
    verified_events: usize,
    verified_artifacts: usize,
    folded_absent_artifact_count: usize,
    /// Absent (not zero) when `--include-ephemeral` skipped the gate scan.
    #[serde(skip_serializing_if = "Option::is_none")]
    sensitivity_excluded_path_count: Option<usize>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StoreLinkBody {
    family_ref: String,
    clone_ref: String,
    created_family: bool,
    folded_events_created: usize,
    folded_events_existing: usize,
    folded_artifacts_created: usize,
    folded_removal_event_count: usize,
    folded_absent_artifact_count: usize,
    source_retired: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    filesystem_warning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    history_overlap_warning: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StoreLinkPreviewBody {
    family_ref: String,
    clone_ref: String,
    would_create_family: bool,
    source_present: bool,
    export_fidelity: String,
    folded_events_to_create: usize,
    folded_events_existing: usize,
    folded_artifacts_to_create: usize,
    folded_artifacts_existing: usize,
    folded_removal_event_count: usize,
    folded_absent_artifact_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    filesystem_warning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    history_overlap_warning: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StoreUnlinkBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_family_ref: Option<String>,
    deregistered: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StoreForgetBody {
    family_ref: String,
    dry_run: bool,
    deleted: bool,
    live_clone_count: usize,
    orphaned: bool,
    inventory: StoreStatusInventory,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StoreListBody {
    families: Vec<StoreListEntryBody>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StoreListEntryBody {
    family_ref: String,
    live_clone_count: usize,
    orphaned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_write: Option<String>,
    inventory: StoreStatusInventory,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StoreStatusBody {
    mode: String,
    store_ref: String,
    store_identity: String,
    context_identity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    clone_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repository_family_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    live_clone_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    orphaned: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_write: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    family_link_advisory: Option<String>,
    inventory: StoreStatusInventory,
    sensitivity: StoreStatusSensitivity,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StorePathsBody {
    tier: &'static str,
    worktree_store: PathBuf,
    common_store: PathBuf,
    binding: PathBuf,
    home: PathBuf,
    keys: PathBuf,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StoreModeBody {
    /// Serializes camelCase: "shared" | "ephemeral".
    mode: StoreMode,
    /// Serializes camelCase: "default" | "committed" | "local".
    source: StoreModeSource,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StoreRemoveBody {
    removed: Vec<RemovedContentBody>,
    events_created: usize,
    events_existing: usize,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RemovedContentBody {
    content_hash: String,
    created: bool,
    co_referencing_units: Vec<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StoreCompactBody {
    swept: Vec<SweptBlobBody>,
    bytes_reclaimed: u64,
    dry_run: bool,
    skipped_ineligible: Vec<SkippedRemovalBody>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SweptBlobBody {
    content_hash: String,
    outcome: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SkippedRemovalBody {
    content_hash: String,
    /// The `removal_claim_*` reason the blob was withheld from erasure.
    reason: String,
}

pub(super) fn run(
    args: StoreArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        StoreCommand::Status(args) => {
            tracing::debug!(command = "store.status", "command_start");
            status(args, stdout)
        }
        StoreCommand::Paths(args) => {
            tracing::debug!(command = "store.paths", "command_start");
            paths(args, stdout)
        }
        StoreCommand::Mode(args) => {
            tracing::debug!(command = "store.mode", "command_start");
            mode(args, stdout)
        }
        StoreCommand::Migrate(args) => {
            tracing::debug!(command = "store.migrate", "command_start");
            migrate(args, stdout)
        }
        StoreCommand::Link(args) => {
            tracing::debug!(command = "store.link", "command_start");
            link(args, stdout)
        }
        StoreCommand::Unlink(args) => {
            tracing::debug!(command = "store.unlink", "command_start");
            unlink(args, stdout)
        }
        StoreCommand::Forget(args) => {
            tracing::debug!(command = "store.forget", "command_start");
            forget(args, stdout)
        }
        StoreCommand::List(args) => {
            tracing::debug!(command = "store.list", "command_start");
            list(args, stdout)
        }
        StoreCommand::Remove(args) => {
            tracing::debug!(command = "store.remove", "command_start");
            remove(args, stdout, stderr)
        }
        StoreCommand::Gc(args) | StoreCommand::Compact(args) => {
            tracing::debug!(command = "store.compact", "command_start");
            compact(args, stdout)
        }
    }
}

fn paths(args: StorePathsArgs, stdout: &mut dyn Write) -> Result<(), Box<dyn std::error::Error>> {
    let resolved = store_paths_for_repo(&args.repo)?;
    let body = StorePathsBody {
        tier: resolved.tier(),
        worktree_store: resolved.worktree_store().to_path_buf(),
        common_store: resolved.common_store().to_path_buf(),
        binding: resolved.binding().to_path_buf(),
        home: resolved.home().to_path_buf(),
        keys: resolved.keys().to_path_buf(),
    };
    let format = output::resolve_format(args.format_args.explicit(), output::OutputFormat::Json)?;
    let digest = matches!(format.format, output::OutputFormat::Text).then(|| {
        format!(
            "tier: {}\nworktree store: {}\ncommon store: {}\nbinding: {}\nhome: {}\nkeys: {}",
            body.tier,
            body.worktree_store.display(),
            body.common_store.display(),
            body.binding.display(),
            body.home.display(),
            body.keys.display(),
        )
    });
    let document = json::DiagnosticDocument::new("pointbreak.store-paths", body, vec![]);
    output::write_document(stdout, format, &document, || {
        digest.expect("text lane resolves the digest")
    })
}

fn status(args: StoreStatusArgs, stdout: &mut dyn Write) -> Result<(), Box<dyn std::error::Error>> {
    let span = tracing::info_span!("shore.store.status");
    let _entered = span.enter();

    let explicit = args.format_args.explicit();
    if args.show_paths {
        // `--show-paths` is text-only, but NOT as a security barrier: the listing
        // goes to the operator's own terminal, the paths are their own local
        // files, and plain text is as machine-readable as JSON — a program can
        // read either lane. The restriction keeps the versioned
        // `pointbreak.store-status` JSON document a single, uniformly path-free shape,
        // so a tool that pipes that document into a log or relay never depends on
        // a flag to stay path-free. The redaction that genuinely matters is on the
        // STORED/forwarded data (events in `.git/pointbreak`, the default document);
        // this local listing never writes to the store, so it sits outside that
        // contract. So refuse an explicit JSON selection rather than silently
        // dropping the paths, and otherwise force the text lane (overriding the
        // JSON default) so `store status --show-paths` alone prints them.
        if matches!(
            explicit,
            Some(output::OutputFormat::Json | output::OutputFormat::JsonPretty)
        ) {
            return Err(
                "`--show-paths` lists real worktree paths on the text lane only, to keep the \
                 `pointbreak.store-status` JSON document uniformly path-free; it cannot be combined \
                 with a JSON format. Drop the JSON selection or pass `--format text`."
                    .into(),
            );
        }
        return status_with_paths(&args.repo, stdout);
    }

    let result = store_status(StoreStatusOptions::new(args.repo))?;
    let body = StoreStatusBody::from(result);
    let format = output::resolve_format(explicit, output::OutputFormat::Json)?;
    // `DiagnosticDocument::new` takes the body by value; render the digest from it
    // first, only on the text lane (the machine lanes never pay for it).
    let digest = matches!(format.format, output::OutputFormat::Text)
        .then(|| render_store_status_text(&body));
    let document = json::DiagnosticDocument::new("pointbreak.store-status", body, vec![]);
    output::write_document(stdout, format, &document, || {
        digest.expect("text lane resolves the digest")
    })
}

/// The text-only `store status --show-paths` lane: the standard digest followed
/// by the real matched worktree paths per finding. The paths come from a
/// dedicated non-serializable scan (`explain_store_sensitivity`) and go straight
/// to stdout — they never enter a `DiagnosticDocument` or any emitted JSON, so
/// the sensitivity no-path contract still holds on the machine lanes.
fn status_with_paths(
    repo: &std::path::Path,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = store_status(StoreStatusOptions::new(repo))?;
    let body = StoreStatusBody::from(result);
    writeln!(stdout, "{}", render_store_status_text(&body))?;

    let groups = explain_store_sensitivity(repo)?;
    writeln!(stdout, "\n{}", render_sensitivity_paths(&groups))?;
    Ok(())
}

/// Cap the paths listed per finding kind under `--show-paths`, mirroring the
/// digest's bounded finding summary; the surplus is tallied, not listed.
const MAX_SHOWN_PATHS_PER_KIND: usize = 20;

/// Render the local-only matched-path listing for `--show-paths`: `kind (outcome)`
/// headers with the real relative paths beneath, bounded per kind. When nothing
/// matched, say so explicitly so the operator knows the scan ran.
fn render_sensitivity_paths(groups: &[StoreSensitivityPathGroup]) -> String {
    if groups.is_empty() {
        return "matched paths: none".to_owned();
    }
    let mut lines =
        vec!["matched paths (local only; never written to the store or emitted JSON):".to_owned()];
    for group in groups {
        lines.push(format!("  {} ({}):", group.kind, group.policy_outcome));
        for path in group.paths.iter().take(MAX_SHOWN_PATHS_PER_KIND) {
            lines.push(format!("    {path}"));
        }
        let remaining = group.paths.len().saturating_sub(MAX_SHOWN_PATHS_PER_KIND);
        if remaining > 0 {
            lines.push(format!("    … and {remaining} more"));
        }
    }
    lines.join("\n")
}

/// The text digest for `store status`: a one-line-first summary a person reads
/// over SSH instead of the multi-KB machine document. Store mode/ref (path-free
/// tokens, never a filesystem path — the same privacy contract the JSON lane
/// holds), event/artifact counts with a human byte total, and the sensitivity
/// outcome with a bounded finding summary. Reads only the CLI-local
/// `StoreStatusBody` (INV-12); sizes via `output::format_bytes`. Every field it
/// surfaces is a bounded token or a count — no user-controlled free text.
fn render_store_status_text(body: &StoreStatusBody) -> String {
    let mut lines: Vec<String> = Vec::new();

    // Store identity — mode and ref tokens only; the JSON lane's no-path contract
    // guarantees these carry no filesystem path.
    let mut identity = format!("store: {}", body.mode);
    if body.store_ref != body.mode {
        identity.push_str(&format!(" ({})", body.store_ref));
    }
    lines.push(identity);

    // Counts and total size, plus the largest artifact when the store has one.
    let inventory = &body.inventory;
    let mut counts = format!(
        "{} · {} · {} total",
        count_label(inventory.event_count, "event", "events"),
        count_label(inventory.artifact_count, "artifact", "artifacts"),
        output::format_bytes(inventory.total_bytes),
    );
    if let Some(largest) = inventory.largest_artifacts.first() {
        counts.push_str(&format!(
            " · largest artifact {}",
            output::format_bytes(largest.byte_size),
        ));
    }
    lines.push(counts);

    // Sensitivity outcome and a bounded finding summary (up to three kinds).
    let sensitivity = &body.sensitivity;
    let mut line = format!(
        "sensitivity: {} · {}",
        sensitivity.policy_outcome,
        count_label(sensitivity.findings.len(), "finding", "findings"),
    );
    if !sensitivity.findings.is_empty() {
        let shown = sensitivity
            .findings
            .iter()
            .take(3)
            .map(|finding| format!("{} ×{}", finding.kind, finding.count))
            .collect::<Vec<_>>()
            .join(", ");
        line.push_str(&format!(": {shown}"));
        let remaining = sensitivity.findings.len().saturating_sub(3);
        if remaining > 0 {
            line.push_str(&format!(" … and {remaining} more"));
        }
    }
    lines.push(line);

    if let Some(advisory) = &body.family_link_advisory {
        lines.push(format!("advisory: {advisory}"));
    }

    lines.join("\n")
}

fn mode(args: StoreModeArgs, stdout: &mut dyn Write) -> Result<(), Box<dyn std::error::Error>> {
    let span = tracing::info_span!("shore.store.mode");
    let _entered = span.enter();
    match args.action {
        StoreModeAction::Shared => set_store_mode_for_repo(&args.repo, StoreMode::Shared)?,
        StoreModeAction::Ephemeral => set_store_mode_for_repo(&args.repo, StoreMode::Ephemeral)?,
        StoreModeAction::Show => {} // no write; just report the resolved mode below
    }
    // Re-read after any set so `show` and `set` report one consistent shape: after
    // a `shared`/`ephemeral` the committed file now exists, so the source is
    // `committed`.
    let outcome = resolve_store_mode_for_repo(&args.repo)?;
    let body = StoreModeBody {
        mode: outcome.mode,
        source: outcome.source,
    };
    let document = json::DiagnosticDocument::new("pointbreak.store-mode", body, vec![]);
    let format = output::resolve_format(args.format_args.explicit(), output::OutputFormat::Json)?;
    output::write_document_json_fallback(stdout, format, &document)
}

fn migrate(
    args: StoreMigrateArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let span = tracing::info_span!("shore.store.migrate");
    let _entered = span.enter();
    let result = migrate_store_to_common_dir(
        MigrateToCommonDirOptions::new(args.repo)
            .with_include_ephemeral(args.include_ephemeral)
            .with_retire_source(args.retire_source),
    )?;
    let body = StoreMigrateBody::from(result);
    let mut diagnostics = Vec::new();
    if body.folded_absent_artifact_count > 0 {
        diagnostics.push(ProjectionDiagnostic {
            code: "family_fold_absent_artifact".to_owned(),
            message: format!(
                "{} referenced artifact(s) were absent from the source (no longer on disk, no \
                 removal claim) and were folded without their content",
                body.folded_absent_artifact_count
            ),
        });
    }
    let format = output::resolve_format(args.format_args.explicit(), output::OutputFormat::Json)?;
    let text = matches!(format.format, output::OutputFormat::Text)
        .then(|| render_store_migrate_text(&body));
    let document = json::DiagnosticDocument::new("pointbreak.store-migrate", body, diagnostics);
    output::write_document(stdout, format, &document, || {
        text.expect("text lane resolves the digest source")
    })
}

/// Bespoke text lane for `store migrate`: a bounded fold receipt — created vs.
/// existing counts, the verification tally, and the conditional outcomes
/// (empty/retired source, content-less folds, sensitivity exclusions).
fn render_store_migrate_text(body: &StoreMigrateBody) -> String {
    let mut lines = vec![
        format!(
            "folded {} ({} existing) · {} ({} existing)",
            count_label(body.events_created, "event", "events"),
            body.events_existing,
            count_label(body.artifacts_created, "artifact", "artifacts"),
            body.artifacts_existing,
        ),
        format!(
            "verified {} · {}",
            count_label(body.verified_events, "event", "events"),
            count_label(body.verified_artifacts, "artifact", "artifacts"),
        ),
    ];
    if body.source_empty {
        lines.push("source store was empty".to_owned());
    }
    if body.source_retired {
        lines.push("source store retired".to_owned());
    }
    if body.folded_absent_artifact_count > 0 {
        lines.push(format!(
            "{} absent from the source · folded without content",
            count_label(body.folded_absent_artifact_count, "artifact", "artifacts"),
        ));
    }
    if let Some(count) = body.sensitivity_excluded_path_count {
        lines.push(format!(
            "sensitivity: {} excluded",
            count_label(count, "path", "paths")
        ));
    }
    lines.join("\n")
}

fn link(args: StoreLinkArgs, stdout: &mut dyn Write) -> Result<(), Box<dyn std::error::Error>> {
    let span = tracing::info_span!("shore.store.link");
    let _entered = span.enter();
    let trust = discover_trust_set(&args.repo);
    let options = StoreLinkOptions::new(args.repo.clone(), args.slug)
        .with_include_ephemeral(args.include_ephemeral)
        .with_include_sensitive(args.include_sensitive)
        .with_retire_source(args.retire_source)
        .with_trust_set(trust);

    let format = output::resolve_format(args.format_args.explicit(), output::OutputFormat::Json)?;

    if args.dry_run {
        // Preview only: gates + fold preflight, zero writes. A blocking gate or a
        // blocking fold preflight propagates as `Err` (non-zero exit + its message);
        // a clean path emits the preview document (exit 0).
        let preview = preview_link_to_family(options)?;
        let body = StoreLinkPreviewBody::from(preview);
        let mut diagnostics = Vec::new();
        if body.folded_removal_event_count > 0 {
            diagnostics.push(ProjectionDiagnostic {
                code: "family_fold_removal_possession_lost".to_owned(),
                message: format!(
                    "{} unsigned removal event(s) would be folded and lose possession-based \
                     suppression; re-issue `pointbreak store remove` in the family store to restore it",
                    body.folded_removal_event_count
                ),
            });
        }
        if body.folded_absent_artifact_count > 0 {
            diagnostics.push(ProjectionDiagnostic {
                code: "family_fold_absent_artifact".to_owned(),
                message: format!(
                    "{} referenced artifact(s) are absent from the source (no longer on disk, no \
                     removal claim) and would be folded without their content",
                    body.folded_absent_artifact_count
                ),
            });
        }
        if let Some(warning) = &body.filesystem_warning {
            diagnostics.push(ProjectionDiagnostic {
                code: "family_store_filesystem_warning".to_owned(),
                message: warning.clone(),
            });
        }
        if let Some(warning) = &body.history_overlap_warning {
            diagnostics.push(ProjectionDiagnostic {
                code: "family_history_overlap_warning".to_owned(),
                message: warning.clone(),
            });
        }
        let document =
            json::DiagnosticDocument::new("pointbreak.store-link-preview", body, diagnostics);
        return output::write_document_json_fallback(stdout, format, &document);
    }

    let result = link_store_to_family(options)?;
    let body = StoreLinkBody::from(result);

    let mut diagnostics = Vec::new();
    if body.folded_removal_event_count > 0 {
        diagnostics.push(ProjectionDiagnostic {
            code: "family_fold_removal_possession_lost".to_owned(),
            message: format!(
                "{} unsigned removal event(s) were folded and lost possession-based suppression; \
                 re-issue `pointbreak store remove` in the family store to restore it",
                body.folded_removal_event_count
            ),
        });
    }
    if body.folded_absent_artifact_count > 0 {
        diagnostics.push(ProjectionDiagnostic {
            code: "family_fold_absent_artifact".to_owned(),
            message: format!(
                "{} referenced artifact(s) were absent from the source (no longer on disk, no \
                 removal claim) and were folded without their content",
                body.folded_absent_artifact_count
            ),
        });
    }
    if let Some(warning) = &body.filesystem_warning {
        diagnostics.push(ProjectionDiagnostic {
            code: "family_store_filesystem_warning".to_owned(),
            message: warning.clone(),
        });
    }
    if let Some(warning) = &body.history_overlap_warning {
        diagnostics.push(ProjectionDiagnostic {
            code: "family_history_overlap_warning".to_owned(),
            message: warning.clone(),
        });
    }

    let digest =
        matches!(format.format, output::OutputFormat::Text).then(|| render_store_link_text(&body));
    let document = json::DiagnosticDocument::new("pointbreak.store-link", body, diagnostics);
    output::write_document(stdout, format, &document, || {
        digest.expect("text lane resolves the digest")
    })
}

/// The text digest for `store link`: a bounded, path-free summary naming the
/// family, whether it was created, the fold counts, and any advisory warning.
fn render_store_link_text(body: &StoreLinkBody) -> String {
    let mut lines = vec![format!("linked to family: {}", body.family_ref)];
    if body.created_family {
        lines.push("family store created".to_owned());
    }
    lines.push(format!(
        "folded {} event(s), {} already present",
        body.folded_events_created, body.folded_events_existing
    ));
    if let Some(warning) = &body.history_overlap_warning {
        lines.push(format!("warning: {warning}"));
    }
    lines.join("\n")
}

fn unlink(args: StoreUnlinkArgs, stdout: &mut dyn Write) -> Result<(), Box<dyn std::error::Error>> {
    let span = tracing::info_span!("shore.store.unlink");
    let _entered = span.enter();
    let result = unlink_store_from_family(StoreUnlinkOptions::new(args.repo))?;
    let document = json::DiagnosticDocument::new(
        "pointbreak.store-unlink",
        StoreUnlinkBody::from(result),
        vec![],
    );
    let format = output::resolve_format(args.format_args.explicit(), output::OutputFormat::Json)?;
    output::write_document_json_fallback(stdout, format, &document)
}

fn forget(args: StoreForgetArgs, stdout: &mut dyn Write) -> Result<(), Box<dyn std::error::Error>> {
    let span = tracing::info_span!("shore.store.forget");
    let _entered = span.enter();
    let result = forget_family_store(
        StoreForgetOptions::new(args.slug)
            .with_yes(args.yes)
            .with_force(args.force),
    )?;
    let body = StoreForgetBody::from(result);
    let format = output::resolve_format(args.format_args.explicit(), output::OutputFormat::Json)?;
    let digest = matches!(format.format, output::OutputFormat::Text)
        .then(|| render_store_forget_text(&body));
    let document = json::DiagnosticDocument::new("pointbreak.store-forget", body, vec![]);
    output::write_document(stdout, format, &document, || {
        digest.expect("text lane resolves the digest")
    })
}

/// The text digest for `store forget`: names the family and its live-clone count,
/// and states plainly whether anything was deleted.
fn render_store_forget_text(body: &StoreForgetBody) -> String {
    let mut lines = vec![format!(
        "family: {} · {} live clone(s)",
        body.family_ref, body.live_clone_count
    )];
    if body.dry_run {
        lines.push("dry run: nothing deleted (re-run with --yes to delete)".to_owned());
    } else if body.deleted {
        lines.push("deleted".to_owned());
    }
    lines.join("\n")
}

fn list(args: StoreListArgs, stdout: &mut dyn Write) -> Result<(), Box<dyn std::error::Error>> {
    let span = tracing::info_span!("shore.store.list");
    let _entered = span.enter();
    let result = list_family_stores()?;
    let body = StoreListBody::from(result);
    let document = json::DiagnosticDocument::new("pointbreak.store-list", body, vec![]);
    let format = output::resolve_format(args.format_args.explicit(), output::OutputFormat::Json)?;
    output::write_document_json_fallback(stdout, format, &document)
}

fn remove(
    args: StoreRemoveArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let span = tracing::info_span!("shore.store.remove");
    let _entered = span.enter();
    let selector = selector_from_args(&args)?;
    let mut options = RemoveOptions::new(args.repo.clone(), selector);
    // Removal is a write: resolve the signer exactly as the review write verbs do
    // so a signed store stays signed; never default to unsigned.
    let mut skip: SigningSkip = None;
    if let Some(resolved) = resolve_and_surface_signer(&args.repo, args.sign_key.as_deref(), stderr)
    {
        let (signed, signer_skip) = apply_resolved_signer(options, resolved);
        options = signed;
        skip = signer_skip;
    }
    let result = remove_content(options)?;
    surface_best_effort_skip(&skip, stderr);
    let body = StoreRemoveBody::from(result);
    let format = output::resolve_format(args.format_args.explicit(), output::OutputFormat::Json)?;
    let text = matches!(format.format, output::OutputFormat::Text)
        .then(|| render_store_remove_text(&body));
    let document = json::DiagnosticDocument::new("pointbreak.store-remove", body, vec![]);
    output::write_document(stdout, format, &document, || {
        text.expect("text lane resolves the digest source")
    })
}

/// Bespoke text lane for `store remove`: a claim-receipt headline, then one
/// line per targeted content hash — recorded vs. already claimed, plus the
/// co-referencing units a shared blob would also affect.
fn render_store_remove_text(body: &StoreRemoveBody) -> String {
    if body.removed.is_empty() {
        return "no removal claims recorded".to_owned();
    }
    let mut lines = vec![format!(
        "{} · {} created ({} existing)",
        count_label(body.removed.len(), "removal claim", "removal claims"),
        count_label(body.events_created, "event", "events"),
        body.events_existing,
    )];
    for removed in &body.removed {
        let mut line = format!(
            "  {} · {}",
            output::short_ref(&removed.content_hash),
            if removed.created {
                "recorded"
            } else {
                "already claimed"
            },
        );
        if !removed.co_referencing_units.is_empty() {
            line.push_str(&format!(
                " · co-referenced by {}",
                count_label(removed.co_referencing_units.len(), "unit", "units")
            ));
        }
        lines.push(line);
    }
    lines.join("\n")
}

fn compact(
    args: StoreCompactArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let span = tracing::info_span!("shore.store.compact");
    let _entered = span.enter();
    // Resolve the reader's trust so the fixed erase-eligibility rule can lift a
    // relayed removal via a trusted signer or endorsement.
    let trust = discover_trust_set(&args.repo);
    // The consent gate lives here, not in the library: erasure runs only with
    // `--yes`; a bare invocation (and `--dry-run`) previews and deletes nothing.
    let perform = args.yes && !args.dry_run;
    let result = compact_store(
        CompactOptions::new(args.repo)
            .with_trust_set(trust)
            .with_dry_run(!perform),
    )?;

    let mut diagnostics = Vec::new();
    if !perform && !args.dry_run {
        // Only blobs that would actually be erased count toward the consent
        // prompt; a hash-mismatched blob is withheld, not erased.
        let would_erase = result
            .swept
            .iter()
            .filter(|blob| blob.outcome == SweepOutcome::Removed)
            .count();
        diagnostics.push(ProjectionDiagnostic {
            code: "compact_consent_required".to_owned(),
            message: format!(
                "{would_erase} blob(s) would be erased; re-run with --yes to erase, or --dry-run to preview. \
                 {} skipped (see skippedIneligible).",
                result.skipped_ineligible.len()
            ),
        });
    }
    for skipped in &result.skipped_ineligible {
        let code = removal_claim_reason_code(skipped.reason);
        diagnostics.push(ProjectionDiagnostic {
            code: code.to_owned(),
            message: format!(
                "removal of {} is not erase-eligible ({code}); it was withheld from erasure",
                skipped.content_hash
            ),
        });
    }
    // An erase-eligible blob whose on-disk bytes no longer hash to their claimed
    // content hash is withheld (HashMismatchSkipped), never deleted; surface each
    // drift as its own diagnostic.
    for blob in &result.swept {
        if blob.outcome == SweepOutcome::HashMismatchSkipped {
            diagnostics.push(ProjectionDiagnostic {
                code: "compact_hash_mismatch".to_owned(),
                message: format!(
                    "blob {} no longer matches its claimed content hash; it was withheld from erasure",
                    blob.content_hash
                ),
            });
        }
    }

    let body = StoreCompactBody::from(result);
    let format = output::resolve_format(args.format_args.explicit(), output::OutputFormat::Json)?;
    let text = matches!(format.format, output::OutputFormat::Text)
        .then(|| render_store_compact_text(&body, args.dry_run));
    let document = json::DiagnosticDocument::new("pointbreak.store-compact", body, diagnostics);
    output::write_document(stdout, format, &document, || {
        text.expect("text lane resolves the digest source")
    })
}

/// Bespoke text lane for `store compact` (and its `gc` alias): a one-line
/// erasure receipt. A performing run reports what was erased and reclaimed; an
/// explicit `--dry-run` previews; a bare run previews and points at `--yes`
/// (the consent gate). Every non-erased sweep outcome is accounted for as a
/// bounded clause — already-absent blobs, hash-mismatch withholds, and
/// not-erase-eligible removals — so the receipt never under-counts silently.
fn render_store_compact_text(body: &StoreCompactBody, explicit_dry_run: bool) -> String {
    let erased = body
        .swept
        .iter()
        .filter(|blob| blob.outcome == "removed")
        .count();
    let mismatched = body
        .swept
        .iter()
        .filter(|blob| blob.outcome == "hash_mismatch_skipped")
        .count();
    let missing = body
        .swept
        .iter()
        .filter(|blob| blob.outcome == "missing")
        .count();
    let mut clauses = Vec::new();
    if body.dry_run {
        clauses.push(format!(
            "{} would be erased",
            count_label(erased, "blob", "blobs")
        ));
    } else {
        clauses.push(format!("erased {}", count_label(erased, "blob", "blobs")));
        clauses.push(format!(
            "reclaimed {}",
            output::format_bytes(body.bytes_reclaimed)
        ));
    }
    if missing > 0 {
        clauses.push(format!(
            "{} already absent",
            count_label(missing, "blob", "blobs")
        ));
    }
    if mismatched > 0 {
        clauses.push(format!(
            "{} withheld (hash mismatch)",
            count_label(mismatched, "blob", "blobs")
        ));
    }
    if !body.skipped_ineligible.is_empty() {
        clauses.push(format!(
            "{} not erase-eligible",
            count_label(body.skipped_ineligible.len(), "removal", "removals")
        ));
    }
    let line = clauses.join(" · ");
    if body.dry_run && explicit_dry_run {
        format!("dry run: {line}")
    } else if body.dry_run {
        format!("{line} · re-run with --yes to erase (--dry-run previews)")
    } else {
        line
    }
}

/// Map a skipped removal's reason to its public `removal_claim_*` code, matching
/// the `pointbreak.review-revision` claim-diagnostic spellings.
fn removal_claim_reason_code(status: RemovalOperativeStatus) -> &'static str {
    match status {
        RemovalOperativeStatus::ClaimUnsigned => "removal_claim_unsigned",
        RemovalOperativeStatus::ClaimUntrusted => "removal_claim_untrusted",
        RemovalOperativeStatus::ClaimInvalid => "removal_claim_invalid",
        // Operative/no-claim statuses are never withheld; fall back defensively.
        RemovalOperativeStatus::NoClaim
        | RemovalOperativeStatus::OperativePossession
        | RemovalOperativeStatus::OperativeTrusted => "removal_claim_ineligible",
    }
}

/// Decode the clap selector group (exactly one is required) into a workflow
/// selector. The clap `ArgGroup` enforces exactly-one; the trailing error is a
/// defensive fallback if that guarantee is ever bypassed.
fn selector_from_args(
    args: &StoreRemoveArgs,
) -> Result<RemoveSelector, Box<dyn std::error::Error>> {
    if let Some(id) = &args.snapshot {
        let ids = crate::cli::id_resolver::IdResolver::new(&args.repo);
        Ok(RemoveSelector::Snapshot(ObjectId::new(ids.object(id)?)))
    } else if let Some(id) = &args.revision {
        let ids = crate::cli::id_resolver::IdResolver::new(&args.repo);
        Ok(RemoveSelector::Revision(RevisionId::new(ids.rev(id)?)))
    } else if let Some(reference) = &args.r#ref {
        Ok(RemoveSelector::Ref(reference.clone()))
    } else if let Some(range) = &args.range {
        Ok(RemoveSelector::Range(range.clone()))
    } else if args.orphans {
        Ok(RemoveSelector::Orphans)
    } else {
        Err("exactly one of --snapshot/--revision/--ref/--range/--orphans is required".into())
    }
}

impl From<MigrateToCommonDirResult> for StoreMigrateBody {
    fn from(result: MigrateToCommonDirResult) -> Self {
        Self {
            events_created: result.events_created,
            events_existing: result.events_existing,
            artifacts_created: result.artifacts_created,
            artifacts_existing: result.artifacts_existing,
            source_empty: result.source_empty,
            source_retired: result.source_retired,
            verified_events: result.verified_events,
            verified_artifacts: result.verified_artifacts,
            folded_absent_artifact_count: result.absent_artifact_count,
            sensitivity_excluded_path_count: result.sensitivity_excluded_path_count,
        }
    }
}

impl From<StoreStatusResult> for StoreStatusBody {
    fn from(result: StoreStatusResult) -> Self {
        Self {
            mode: result.mode,
            store_ref: result.store_ref,
            store_identity: result.store_identity,
            context_identity: result.context_identity,
            clone_ref: result.clone_ref,
            repository_family_ref: result.repository_family_ref,
            live_clone_count: result.live_clone_count,
            orphaned: result.orphaned,
            last_write: result.last_write,
            family_link_advisory: result.family_link_advisory,
            inventory: result.inventory,
            sensitivity: result.sensitivity,
        }
    }
}

impl From<StoreLinkResult> for StoreLinkBody {
    fn from(result: StoreLinkResult) -> Self {
        Self {
            family_ref: result.family_ref,
            clone_ref: result.clone_ref,
            created_family: result.created_family,
            folded_events_created: result.folded_events_created,
            folded_events_existing: result.folded_events_existing,
            folded_artifacts_created: result.folded_artifacts_created,
            folded_removal_event_count: result.folded_removal_event_count,
            folded_absent_artifact_count: result.folded_absent_artifact_count,
            source_retired: result.source_retired,
            filesystem_warning: result.filesystem_warning,
            history_overlap_warning: result.history_overlap_warning,
        }
    }
}

impl From<StoreLinkPreview> for StoreLinkPreviewBody {
    fn from(preview: StoreLinkPreview) -> Self {
        Self {
            family_ref: preview.family_ref,
            clone_ref: preview.clone_ref,
            would_create_family: preview.would_create_family,
            source_present: preview.source_present,
            export_fidelity: preview.export_fidelity,
            folded_events_to_create: preview.folded_events_to_create,
            folded_events_existing: preview.folded_events_existing,
            folded_artifacts_to_create: preview.folded_artifacts_to_create,
            folded_artifacts_existing: preview.folded_artifacts_existing,
            folded_removal_event_count: preview.folded_removal_event_count,
            folded_absent_artifact_count: preview.folded_absent_artifact_count,
            filesystem_warning: preview.filesystem_warning,
            history_overlap_warning: preview.history_overlap_warning,
        }
    }
}

impl From<StoreUnlinkResult> for StoreUnlinkBody {
    fn from(result: StoreUnlinkResult) -> Self {
        Self {
            previous_family_ref: result.previous_family_ref,
            deregistered: result.deregistered,
        }
    }
}

impl From<StoreForgetResult> for StoreForgetBody {
    fn from(result: StoreForgetResult) -> Self {
        Self {
            family_ref: result.family_ref,
            dry_run: result.dry_run,
            deleted: result.deleted,
            live_clone_count: result.live_clone_count,
            orphaned: result.orphaned,
            inventory: result.inventory,
        }
    }
}

impl From<StoreListResult> for StoreListBody {
    fn from(result: StoreListResult) -> Self {
        Self {
            families: result
                .families
                .into_iter()
                .map(StoreListEntryBody::from)
                .collect(),
        }
    }
}

impl From<StoreListEntry> for StoreListEntryBody {
    fn from(entry: StoreListEntry) -> Self {
        Self {
            family_ref: entry.family_ref,
            live_clone_count: entry.live_clone_count,
            orphaned: entry.orphaned,
            last_write: entry.last_write,
            inventory: entry.inventory,
        }
    }
}

impl From<RemoveResult> for StoreRemoveBody {
    fn from(result: RemoveResult) -> Self {
        Self {
            removed: result
                .removed
                .into_iter()
                .map(RemovedContentBody::from)
                .collect(),
            events_created: result.events_created,
            events_existing: result.events_existing,
        }
    }
}

impl From<RemovedContent> for RemovedContentBody {
    fn from(content: RemovedContent) -> Self {
        Self {
            content_hash: content.content_hash,
            created: content.created,
            co_referencing_units: content
                .co_referencing_units
                .iter()
                .map(|id| id.as_str().to_owned())
                .collect(),
        }
    }
}

impl From<CompactResult> for StoreCompactBody {
    fn from(result: CompactResult) -> Self {
        Self {
            swept: result.swept.into_iter().map(SweptBlobBody::from).collect(),
            bytes_reclaimed: result.bytes_reclaimed,
            dry_run: result.dry_run,
            skipped_ineligible: result
                .skipped_ineligible
                .into_iter()
                .map(SkippedRemovalBody::from)
                .collect(),
        }
    }
}

impl From<SkippedRemoval> for SkippedRemovalBody {
    fn from(skipped: SkippedRemoval) -> Self {
        Self {
            reason: removal_claim_reason_code(skipped.reason).to_owned(),
            content_hash: skipped.content_hash,
        }
    }
}

impl From<SweptBlob> for SweptBlobBody {
    fn from(blob: SweptBlob) -> Self {
        Self {
            content_hash: blob.content_hash,
            outcome: match blob.outcome {
                SweepOutcome::Removed => "removed",
                SweepOutcome::Missing => "missing",
                SweepOutcome::HashMismatchSkipped => "hash_mismatch_skipped",
            }
            .to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn swept(outcome: &str) -> SweptBlobBody {
        SweptBlobBody {
            content_hash: format!("sha256:{}", "ab".repeat(32)),
            outcome: outcome.to_owned(),
        }
    }

    /// Every sweep outcome is accounted for in the compact receipt — including
    /// `missing`, which the CLI's filesystem backend cannot produce in an
    /// integration fixture (a physically absent blob drops out of `list_refs`
    /// before the sweep classifies it, so only a race or a non-filesystem
    /// backend yields it).
    #[test]
    fn compact_receipt_accounts_for_every_sweep_outcome() {
        let body = StoreCompactBody {
            swept: vec![
                swept("removed"),
                swept("removed"),
                swept("missing"),
                swept("hash_mismatch_skipped"),
            ],
            bytes_reclaimed: 2_048,
            dry_run: false,
            skipped_ineligible: vec![],
        };
        assert_eq!(
            render_store_compact_text(&body, false),
            "erased 2 blobs · reclaimed 2.0 KB · 1 blob already absent · 1 blob withheld (hash mismatch)"
        );
    }

    #[test]
    fn compact_receipt_distinguishes_dry_run_from_consent_preview() {
        let body = StoreCompactBody {
            swept: vec![swept("removed"), swept("missing")],
            bytes_reclaimed: 0,
            dry_run: true,
            skipped_ineligible: vec![],
        };
        assert_eq!(
            render_store_compact_text(&body, true),
            "dry run: 1 blob would be erased · 1 blob already absent"
        );
        assert_eq!(
            render_store_compact_text(&body, false),
            "1 blob would be erased · 1 blob already absent · re-run with --yes to erase (--dry-run previews)"
        );
    }
}
