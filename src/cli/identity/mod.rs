use std::io::Write;

use clap::{Args, Subcommand};

mod attest;
mod delegate;
mod whoami;

use attest::AttestArgs;
use delegate::DelegateArgs;
use whoami::WhoamiArgs;

#[derive(Debug, Args)]
pub(super) struct IdentityArgs {
    #[command(subcommand)]
    command: IdentityCommand,
}

#[derive(Debug, Subcommand)]
enum IdentityCommand {
    /// Stage a delegation record binding an agent actor to its responsible principal.
    /// Possession-style: stages the working-tree `.shore/delegates.json` edit only;
    /// commit it to authorize the delegation.
    Delegate(DelegateArgs),
    /// Stage an actor-attributes entry (kind + roles) for an actor. Possession-style:
    /// stages the working-tree `.shore/actor-attributes.json` edit only; commit to apply.
    Attest(AttestArgs),
    /// Show the actor identity that repository writes will use.
    Whoami(WhoamiArgs),
}

pub(super) fn run(
    args: IdentityArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        IdentityCommand::Delegate(args) => {
            tracing::debug!(command = "identity.delegate", "command_start");
            delegate::run(args, stdout, stderr)
        }
        IdentityCommand::Attest(args) => {
            tracing::debug!(command = "identity.attest", "command_start");
            attest::run(args, stdout, stderr)
        }
        IdentityCommand::Whoami(args) => {
            tracing::debug!(command = "identity.whoami", "command_start");
            whoami::run(args, stdout)
        }
    }
}
