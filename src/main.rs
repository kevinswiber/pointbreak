use std::fs;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use shore::dump::DumpDocument;
use shore::sidecar::{parse_hunk_agent_context, parse_review_notes_sidecar};

#[cfg_attr(not(test), allow(dead_code))]
mod tui;

#[derive(Debug, Parser)]
#[command(name = "shore", version, about = "Inspect review streams")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Dump(DumpArgs),
}

#[derive(Debug, Args)]
struct DumpArgs {
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    #[arg(long, conflicts_with = "legacy_hunk_agent_context")]
    review_notes: Option<PathBuf>,

    #[arg(long, conflicts_with = "review_notes")]
    legacy_hunk_agent_context: Option<PathBuf>,

    #[arg(long, conflicts_with = "compact")]
    pretty: bool,

    #[arg(long)]
    compact: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().command {
        Command::Dump(args) => dump(args),
    }
}

fn dump(args: DumpArgs) -> Result<(), Box<dyn std::error::Error>> {
    let document = match (&args.review_notes, &args.legacy_hunk_agent_context) {
        (Some(review_notes), None) => {
            let json = fs::read_to_string(review_notes)?;
            let parsed = parse_review_notes_sidecar(&json)?;
            DumpDocument::from_parsed_review_notes(&args.repo, parsed)?
        }
        (None, Some(agent_context)) => {
            let json = fs::read_to_string(agent_context)?;
            let parsed = parse_hunk_agent_context(&json)?;
            DumpDocument::from_legacy_hunk_agent_context(&args.repo, parsed)?
        }
        (None, None) => DumpDocument::from_repo(&args.repo)?,
        (Some(_), Some(_)) => unreachable!("clap rejects mutually exclusive sidecar flags"),
    };
    let json = if should_pretty_print(&args) {
        serde_json::to_string_pretty(&document)?
    } else {
        serde_json::to_string(&document)?
    };
    println!("{json}");
    Ok(())
}

fn should_pretty_print(args: &DumpArgs) -> bool {
    args.pretty || (!args.compact && std::io::stdout().is_terminal())
}
