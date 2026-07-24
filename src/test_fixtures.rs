use std::path::PathBuf;

/// Absolute path to this crate's manifest directory, resolved at runtime.
///
/// Prefers the runtime `CARGO_MANIFEST_DIR` — which cargo-nextest remaps via
/// `--workspace-remap` when the tests run from an archive built on another machine — and
/// falls back to the compile-time value for ordinary in-place runs (where they are
/// identical). Shared by the crate's `#[cfg(test)]` fixture lookups so a cross-compiled
/// (e.g. Windows) archive still finds fixtures relative to the remapped workspace root.
pub(crate) fn manifest_dir() -> PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

pub(crate) fn naming_cutover_bytes(relative: &str) -> Vec<u8> {
    std::fs::read(
        manifest_dir()
            .join("tests/fixtures/naming-cutover")
            .join(relative),
    )
    .unwrap_or_else(|error| panic!("read naming-cutover fixture {relative}: {error}"))
}

pub(crate) fn naming_cutover_contract_bytes(relative: &str) -> Vec<u8> {
    let mut bytes = naming_cutover_bytes(relative);
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    bytes
}
