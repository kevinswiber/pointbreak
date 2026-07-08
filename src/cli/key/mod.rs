use std::io::Write;

use clap::{Args, Subcommand};

mod discover;
mod enroll;
mod init;
mod list;
mod show;
mod use_ssh;

use discover::DiscoverArgs;
use enroll::EnrollArgs;
use init::InitArgs;
use list::ListArgs;
use show::ShowArgs;
use use_ssh::UseSshArgs;

#[derive(Debug, Args)]
pub(super) struct KeyArgs {
    #[command(subcommand)]
    command: KeyCommand,
}

#[derive(Debug, Subcommand)]
enum KeyCommand {
    /// Generate a new signing key in the user-level keystore.
    Init(InitArgs),
    /// List local signing keys and their enrollment status.
    List(ListArgs),
    /// Print a key's did:key and/or raw public key.
    Show(ShowArgs),
    /// Discover advisory Git/OpenSSH signing evidence; does not authorize keys.
    Discover(DiscoverArgs),
    /// Adopt an existing SSH Ed25519 key as an agent-backed signer (sign via
    /// ssh-agent; no new key material). Parallel to `init`.
    UseSsh(UseSshArgs),
    /// Stage an allow-list entry binding a local key's did:key to an actor.
    /// Possession-style: this stages the working-tree `.shore/allowed-signers.json`
    /// edit only; commit it to authorize the binding.
    Enroll(EnrollArgs),
}

pub(super) fn run(args: KeyArgs, stdout: &mut dyn Write) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        KeyCommand::Init(args) => {
            tracing::debug!(command = "key.init", "command_start");
            init::run(args, stdout)
        }
        KeyCommand::List(args) => {
            tracing::debug!(command = "key.list", "command_start");
            list::run(args, stdout)
        }
        KeyCommand::Show(args) => {
            tracing::debug!(command = "key.show", "command_start");
            show::run(args, stdout)
        }
        KeyCommand::Discover(args) => {
            tracing::debug!(command = "key.discover", "command_start");
            discover::run(args, stdout)
        }
        KeyCommand::UseSsh(args) => {
            tracing::debug!(command = "key.use-ssh", "command_start");
            use_ssh::run(args, stdout)
        }
        KeyCommand::Enroll(args) => {
            tracing::debug!(command = "key.enroll", "command_start");
            enroll::run(args, stdout)
        }
    }
}
