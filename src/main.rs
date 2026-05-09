use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use shore::dump::DumpDocument;

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
    let document = DumpDocument::from_repo(&args.repo)?;
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
