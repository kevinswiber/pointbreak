mod support {
    pub mod review_example_pack;
}

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use support::review_example_pack::{export_pack, materialize_pack, verify_pack};

#[derive(Debug, Parser)]
#[command(about = "Maintain the checked Pointbreak Review example pack")]
struct Cli {
    #[command(subcommand)]
    command: PackCommand,
}

#[derive(Debug, Subcommand)]
enum PackCommand {
    Export {
        #[arg(long)]
        repo: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Verify {
        #[arg(long)]
        pack: PathBuf,
    },
    Materialize {
        #[arg(long)]
        pack: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().command {
        PackCommand::Export { repo, output } => export_pack(&repo, &output)?,
        PackCommand::Verify { pack } => verify_pack(&pack)?,
        PackCommand::Materialize { pack, output } => {
            materialize_pack(&pack, &output)?;
            println!("pointbreak inspect --repo {}", output.display());
        }
    }
    Ok(())
}
