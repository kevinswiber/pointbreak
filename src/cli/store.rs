use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand};
use shoreline::session::{
    StoreLinkOptions, StoreLinkResult, StoreStatusInventory, StoreStatusOptions, StoreStatusResult,
    StoreStatusSensitivity, link_clone_local_store, store_status,
};

use crate::cli::json;

#[derive(Debug, Args)]
pub(super) struct StoreArgs {
    #[command(subcommand)]
    command: StoreCommand,
}

#[derive(Debug, Subcommand)]
enum StoreCommand {
    Link(StoreLinkArgs),
    Status(StoreStatusArgs),
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

pub(super) fn run(
    args: StoreArgs,
    stdout: &mut dyn Write,
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
