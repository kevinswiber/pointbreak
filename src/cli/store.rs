use std::io::Write;
use std::path::PathBuf;

use clap::{ArgGroup, Args, Subcommand, ValueEnum};
use shoreline::model::{ObjectId, RevisionId};
use shoreline::session::{
    CompactOptions, CompactResult, MigrateToCommonDirOptions, MigrateToCommonDirResult,
    ProjectionDiagnostic, RemovalOperativeStatus, RemoveOptions, RemoveResult, RemoveSelector,
    RemovedContent, SkippedRemoval, StoreMode, StoreModeSource, StoreStatusInventory,
    StoreStatusOptions, StoreStatusResult, StoreStatusSensitivity, SweepOutcome, SweptBlob,
    compact_store, migrate_store_to_common_dir, remove_content, resolve_store_mode_for_repo,
    set_store_mode_for_repo, store_status,
};

use crate::cli::review::common::{
    SigningSkip, apply_resolved_signer, discover_trust_set, resolve_and_surface_signer,
    surface_best_effort_skip,
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
    Mode(StoreModeArgs),
    Migrate(StoreMigrateArgs),
    Remove(StoreRemoveArgs),
    /// Alias of `compact`.
    Gc(StoreCompactArgs),
    Compact(StoreCompactArgs),
}

#[derive(Debug, Args)]
struct StoreStatusArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    pretty: bool,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

#[derive(Debug, Args)]
struct StoreModeArgs {
    /// `shared`, `ephemeral`, or `show` (report the resolved mode without
    /// changing it).
    action: StoreModeAction,

    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    pretty: bool,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

#[derive(Debug, Args)]
struct StoreMigrateArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Fan an ephemeral or sensitivity-flagged worktree's store into the shared
    /// store anyway. Off by default: such a worktree is refused without this flag.
    #[arg(long)]
    include_ephemeral: bool,

    /// After the fold is independently verified (every source event and artifact
    /// file present in the shared store), delete the worktree-local .shore/data
    /// so reads resolve in one command. Off by default: the source is never
    /// discarded before the migration is confirmed.
    #[arg(long)]
    retire_source: bool,

    #[arg(long)]
    pretty: bool,

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

/// Exactly one selector is required; the content-targeted removal key is derived
/// solely from the content hash, so there is deliberately no `--idempotency-key`.
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

    #[arg(long)]
    pretty: bool,

    /// Sign this write with a specific key: a keystore key name or a path to a
    /// key file. Removal is a write, so a signed store stays signed.
    #[arg(long)]
    sign_key: Option<String>,

    #[command(flatten)]
    format_args: output::FormatArgs,
}

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

    #[arg(long)]
    pretty: bool,

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
    /// Absent (not zero) when `--include-ephemeral` skipped the gate scan.
    #[serde(skip_serializing_if = "Option::is_none")]
    sensitivity_excluded_path_count: Option<usize>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StoreStatusBody {
    mode: String,
    store_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    clone_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repository_family_ref: Option<String>,
    inventory: StoreStatusInventory,
    sensitivity: StoreStatusSensitivity,
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
        StoreCommand::Mode(args) => {
            tracing::debug!(command = "store.mode", "command_start");
            mode(args, stdout)
        }
        StoreCommand::Migrate(args) => {
            tracing::debug!(command = "store.migrate", "command_start");
            migrate(args, stdout)
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

fn status(args: StoreStatusArgs, stdout: &mut dyn Write) -> Result<(), Box<dyn std::error::Error>> {
    let span = tracing::info_span!("shore.store.status");
    let _entered = span.enter();
    let result = store_status(StoreStatusOptions::new(args.repo))?;
    let body = StoreStatusBody::from(result);
    let format = output::resolve_format(
        args.format_args.explicit(args.pretty),
        output::OutputFormat::Json,
    )?;
    // `DiagnosticDocument::new` takes the body by value; render the digest from it
    // first, only on the text lane (the machine lanes never pay for it).
    let digest = matches!(format.format, output::OutputFormat::Text)
        .then(|| render_store_status_text(&body));
    let document = json::DiagnosticDocument::new("shore.store-status", body, vec![]);
    output::write_document(stdout, format, &document, || {
        digest.expect("text lane resolves the digest")
    })
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

    lines.join("\n")
}

/// `N noun`, singular when `count == 1`.
fn count_label(count: usize, singular: &str, plural: &str) -> String {
    let noun = if count == 1 { singular } else { plural };
    format!("{count} {noun}")
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
    let document = json::DiagnosticDocument::new("shore.store-mode", body, vec![]);
    let format = output::resolve_format(
        args.format_args.explicit(args.pretty),
        output::OutputFormat::Json,
    )?;
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
    let document = json::DiagnosticDocument::new(
        "shore.store-migrate",
        StoreMigrateBody::from(result),
        vec![],
    );
    let format = output::resolve_format(
        args.format_args.explicit(args.pretty),
        output::OutputFormat::Json,
    )?;
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
    let document =
        json::DiagnosticDocument::new("shore.store-remove", StoreRemoveBody::from(result), vec![]);
    let format = output::resolve_format(
        args.format_args.explicit(args.pretty),
        output::OutputFormat::Json,
    )?;
    output::write_document_json_fallback(stdout, format, &document)
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

    let document = json::DiagnosticDocument::new(
        "shore.store-compact",
        StoreCompactBody::from(result),
        diagnostics,
    );
    let format = output::resolve_format(
        args.format_args.explicit(args.pretty),
        output::OutputFormat::Json,
    )?;
    output::write_document_json_fallback(stdout, format, &document)
}

/// Map a skipped removal's reason to its public `removal_claim_*` code, matching
/// the `shore.review-revision` claim-diagnostic spellings.
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
        Ok(RemoveSelector::Snapshot(ObjectId::new(id.clone())))
    } else if let Some(id) = &args.revision {
        Ok(RemoveSelector::Revision(RevisionId::new(id.clone())))
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
            sensitivity_excluded_path_count: result.sensitivity_excluded_path_count,
        }
    }
}

impl From<StoreStatusResult> for StoreStatusBody {
    fn from(result: StoreStatusResult) -> Self {
        Self {
            mode: result.mode,
            store_ref: result.store_ref,
            clone_ref: result.clone_ref,
            repository_family_ref: result.repository_family_ref,
            inventory: result.inventory,
            sensitivity: result.sensitivity,
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
