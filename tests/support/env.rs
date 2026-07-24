//! Runtime resolution of the values Cargo bakes in via `env!` at compile time.
//!
//! `env!("CARGO_BIN_EXE_pointbreak")` and `env!("CARGO_MANIFEST_DIR")` embed the build
//! machine's paths, which do not exist when a cargo-nextest archive is built on one
//! machine and run on another (e.g. cross-compiled to Windows and executed there). At
//! runtime nextest sets `CARGO_BIN_EXE_pointbreak` to the extracted binary and remaps
//! `CARGO_MANIFEST_DIR` via `--workspace-remap`, so prefer those and fall back to the
//! compile-time value for ordinary in-place runs (where the two are identical).
//!
//! Shared by both `mod support;` consumers (via `support::{pointbreak_bin, manifest_dir}`)
//! and standalone integration tests that include only this file with
//! `#[path = "support/env.rs"] mod env;`.

use std::path::PathBuf;

/// Absolute path to the built `pointbreak` binary under test.
#[allow(dead_code)]
pub fn pointbreak_bin() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_pointbreak")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_pointbreak")))
}

/// Absolute path to this crate's manifest directory, the root for fixture lookups.
#[allow(dead_code)]
pub fn manifest_dir() -> PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

/// The `cargo` binary for tests that shell out to Cargo (metadata, install).
///
/// Prefers the runtime `CARGO` that Cargo and cargo-nextest set in a test's environment,
/// falling back to a bare `cargo` resolved on `PATH` — deliberately not the compile-time
/// `env!("CARGO")`, whose baked build-machine path does not exist on a cross-machine run.
#[allow(dead_code)]
pub fn cargo_bin() -> std::ffi::OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}
