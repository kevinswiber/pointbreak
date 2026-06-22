//! One-shot migration driver: re-key a reshaped store from the `Ledger`-shaped
//! wire into the `Journal`-shaped wire in a single pass. Owner-run; NOT part of
//! the `shore` binary — this is a run-once, throwaway operation, so it ships as a
//! tested library function plus this thin driver rather than a permanent CLI
//! subcommand.
//!
//! Usage:
//!   migrate-substrate <source-store-dir> <target-store-dir> <keystore-dir>
//!
//! `<source-store-dir>` and `<target-store-dir>` are the directories holding
//! `events/` and `artifacts/` (e.g. a repo's `.git/shore`); `<target-store-dir>`
//! must be fresh/empty. `<keystore-dir>` holds the signers' private keys used to
//! re-sign inline signatures and re-attest held-key co-signatures. All paths are
//! arguments — the driver carries no built-in locations.
use std::path::PathBuf;
use std::process::ExitCode;

use shoreline::session::{MigrateOptions, migrate_journal_rename};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [source, target, keystore] = args.as_slice() else {
        eprintln!("usage: migrate-substrate <source-store-dir> <target-store-dir> <keystore-dir>");
        return ExitCode::FAILURE;
    };

    let options = MigrateOptions {
        source_store_dir: PathBuf::from(source),
        target_store_dir: PathBuf::from(target),
        keystore_dir: PathBuf::from(keystore),
    };

    match migrate_journal_rename(options) {
        Ok(summary) => {
            println!(
                "migrated {source} -> {target}: events_migrated={} events_passed_through={} \
                 artifacts_rehashed={} inline_signatures_resigned={} cosignatures_reattested={} \
                 cosignatures_dropped={} self_check_passed={}",
                summary.events_migrated,
                summary.events_passed_through,
                summary.artifacts_rehashed,
                summary.inline_signatures_resigned,
                summary.cosignatures_reattested,
                summary.cosignatures_dropped,
                summary.self_check_passed,
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("migrate-substrate failed: {error}");
            ExitCode::FAILURE
        }
    }
}
