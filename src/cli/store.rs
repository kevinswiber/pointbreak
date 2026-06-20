use std::io::Write;
use std::path::PathBuf;

use clap::{ArgGroup, Args, Subcommand, ValueEnum};
use shoreline::model::{ReviewUnitId, SnapshotId};
use shoreline::session::{
    CompactOptions, CompactResult, RemoveOptions, RemoveResult, RemoveSelector, RemovedContent,
    StoreLinkOptions, StoreLinkResult, StoreMode, StoreModeSource, StoreStatusInventory,
    StoreStatusOptions, StoreStatusResult, StoreStatusSensitivity, SweepOutcome, SweptBlob,
    compact_store, link_clone_local_store, remove_content, resolve_store_mode_for_repo,
    set_store_mode_for_repo, store_status,
};

use crate::cli::json;
use crate::cli::review::common::{
    SigningSkip, apply_resolved_signer, resolve_and_surface_signer, surface_best_effort_skip,
};

#[derive(Debug, Args)]
pub(super) struct StoreArgs {
    #[command(subcommand)]
    command: StoreCommand,
}

#[derive(Debug, Subcommand)]
enum StoreCommand {
    Link(StoreLinkArgs),
    Status(StoreStatusArgs),
    Mode(StoreModeArgs),
    Remove(StoreRemoveArgs),
    /// Alias of `compact`.
    Gc(StoreCompactArgs),
    Compact(StoreCompactArgs),
}

#[derive(Debug, Args)]
struct StoreLinkArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    pretty: bool,
}

#[derive(Debug, Args)]
struct StoreStatusArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    pretty: bool,
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
    /// Remove every artifact a review unit references.
    #[arg(long, group = "selector")]
    review_unit: Option<String>,
    /// Remove artifacts of units anchored on the commit this ref resolves to.
    #[arg(long, group = "selector")]
    r#ref: Option<String>,
    /// Remove artifacts of units anchored on a commit in the `<a>..<b>` range.
    #[arg(long, group = "selector")]
    range: Option<String>,
    /// Remove artifacts of commit-anchored units whose commits are all orphaned.
    #[arg(long, group = "selector")]
    orphans: bool,

    #[arg(long)]
    pretty: bool,

    /// Sign this write with a specific key: a keystore key name or a path to a
    /// key file. Removal is a write, so a signed store stays signed.
    #[arg(long)]
    sign_key: Option<String>,
}

#[derive(Debug, Args)]
struct StoreCompactArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long)]
    pretty: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StoreLinkBody {
    mode: String,
    store_ref: String,
    clone_ref: String,
    repository_family_ref: String,
    events_created: usize,
    events_existing: usize,
    artifacts_created: usize,
    artifacts_existing: usize,
    sensitivity: StoreStatusSensitivity,
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
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SweptBlobBody {
    content_hash: String,
    outcome: String,
}

pub(super) fn run(
    args: StoreArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        StoreCommand::Link(args) => {
            tracing::debug!(command = "store.link", "command_start");
            link(args, stdout)
        }
        StoreCommand::Status(args) => {
            tracing::debug!(command = "store.status", "command_start");
            status(args, stdout)
        }
        StoreCommand::Mode(args) => {
            tracing::debug!(command = "store.mode", "command_start");
            mode(args, stdout)
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

fn link(args: StoreLinkArgs, stdout: &mut dyn Write) -> Result<(), Box<dyn std::error::Error>> {
    let span = tracing::info_span!("shore.store.link");
    let _entered = span.enter();
    let result = link_clone_local_store(StoreLinkOptions::new(args.repo))?;
    let document =
        json::DiagnosticDocument::new("shore.store-link", StoreLinkBody::from(result), vec![]);
    json::write_json(stdout, &document, args.pretty)
}

fn status(args: StoreStatusArgs, stdout: &mut dyn Write) -> Result<(), Box<dyn std::error::Error>> {
    let span = tracing::info_span!("shore.store.status");
    let _entered = span.enter();
    let result = store_status(StoreStatusOptions::new(args.repo))?;
    let document =
        json::DiagnosticDocument::new("shore.store-status", StoreStatusBody::from(result), vec![]);
    json::write_json(stdout, &document, args.pretty)
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
    json::write_json(stdout, &document, args.pretty)
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
    json::write_json(stdout, &document, args.pretty)
}

fn compact(
    args: StoreCompactArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let span = tracing::info_span!("shore.store.compact");
    let _entered = span.enter();
    let result = compact_store(CompactOptions::new(args.repo))?;
    let document = json::DiagnosticDocument::new(
        "shore.store-compact",
        StoreCompactBody::from(result),
        vec![],
    );
    json::write_json(stdout, &document, args.pretty)
}

/// Decode the clap selector group (exactly one is required) into a workflow
/// selector. The clap `ArgGroup` enforces exactly-one; the trailing error is a
/// defensive fallback if that guarantee is ever bypassed.
fn selector_from_args(
    args: &StoreRemoveArgs,
) -> Result<RemoveSelector, Box<dyn std::error::Error>> {
    if let Some(id) = &args.snapshot {
        Ok(RemoveSelector::Snapshot(SnapshotId::new(id.clone())))
    } else if let Some(id) = &args.review_unit {
        Ok(RemoveSelector::ReviewUnit(ReviewUnitId::new(id.clone())))
    } else if let Some(reference) = &args.r#ref {
        Ok(RemoveSelector::Ref(reference.clone()))
    } else if let Some(range) = &args.range {
        Ok(RemoveSelector::Range(range.clone()))
    } else if args.orphans {
        Ok(RemoveSelector::Orphans)
    } else {
        Err("exactly one of --snapshot/--review-unit/--ref/--range/--orphans is required".into())
    }
}

impl From<StoreLinkResult> for StoreLinkBody {
    fn from(result: StoreLinkResult) -> Self {
        Self {
            mode: result.mode,
            store_ref: result.store_ref,
            clone_ref: result.clone_ref,
            repository_family_ref: result.repository_family_ref,
            events_created: result.events_created,
            events_existing: result.events_existing,
            artifacts_created: result.artifacts_created,
            artifacts_existing: result.artifacts_existing,
            sensitivity: result.sensitivity,
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
            }
            .to_owned(),
        }
    }
}
