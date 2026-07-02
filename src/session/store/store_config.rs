//! Store-mode configuration: the worktree-local opt-out for the sensitive,
//! throwaway case. A committed `.shore/store.json` and a git-excluded
//! `.shore/store.local.json` override compose git-config style — the exact
//! `delegates.json` / `delegates.local.json` precedent ([`with_local_override`]).
//!
//! The merge **precedence** mirrors delegates (local wins; both absent →
//! `StoreMode::default()`), but the failure posture deliberately diverges: a
//! malformed or unsupported-version config is a **hard error**, never the
//! advisory warn-and-ignore the delegates merge uses. The mode decides *where*
//! sensitive bytes land, so a silent fallback to the shared default would be a
//! privacy regression. Every such error is actionable — it names the offending
//! file, the valid modes, and the command that rewrites it.
//!
//! [`with_local_override`]: crate::session::identity::DelegationMap::with_local_override

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Result, ShoreError};
use crate::git::git_worktree_root;

const STORE_CONFIG_SCHEMA: &str = "shore.store-config";
const STORE_CONFIG_VERSION: u32 = 1;

/// Repo-relative paths to the store-config files. Mirrors `DELEGATES_REL_PATH` /
/// `DELEGATES_LOCAL_REL_PATH`: the committed default and the git-excluded private
/// override.
pub(crate) const STORE_CONFIG_REL_PATH: &str = ".shore/store.json";
pub(crate) const STORE_CONFIG_LOCAL_REL_PATH: &str = ".shore/store.local.json";

/// Where the resolved review store for a worktree lives. The opt-out the topology
/// collapse keeps for the sensitive-throwaway case: `Ephemeral` pins a worktree's
/// data to the discardable worktree-local `.shore/data`; `Shared` (the default)
/// lets the resolver place the store per its normal policy. This is a single bit
/// consulted by the resolver — it carries no store identity.
// `pub` (not `pub(crate)`): the binary/CLI crate names this type — it appears in
// the public `..._for_repo` wrapper signatures re-exported from `session::mod`,
// and a crate-internal type cannot be re-exported publicly.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StoreMode {
    /// The resolver places the store per its normal policy (the default).
    #[default]
    Shared,
    /// Pin the store worktree-local and discardable.
    Ephemeral,
}

/// The persisted store-config document. Modeled on `StoreManifest` (schema +
/// version + body) so an unsupported schema/version is a loud error, never a
/// silent misread.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoreConfig {
    schema: String,
    version: u32,
    mode: StoreMode,
}

impl StoreConfig {
    fn new(mode: StoreMode) -> Self {
        Self {
            schema: STORE_CONFIG_SCHEMA.to_owned(),
            version: STORE_CONFIG_VERSION,
            mode,
        }
    }

    fn validate_schema_version(&self, path: &Path) -> Result<()> {
        if self.schema == STORE_CONFIG_SCHEMA && self.version == STORE_CONFIG_VERSION {
            return Ok(());
        }
        // Name the offending file, like the malformed branch: with both a
        // committed and a local config possible, the user must know which file to
        // rewrite (the actionable-error contract — never a path-free message).
        Err(ShoreError::Message(format!(
            "store config {} has unsupported schema/version {} v{} (expected {} v{}); \
             rewrite it with `shore store mode shared` or `shore store mode ephemeral`",
            path.display(),
            self.schema,
            self.version,
            STORE_CONFIG_SCHEMA,
            STORE_CONFIG_VERSION
        )))
    }
}

/// Resolve the effective store mode under `<worktree-root>/.shore/`. Two files
/// compose, git-config style: the committed `.shore/store.json` and a
/// locally-excluded `.shore/store.local.json` override; the local file's `mode`
/// fully replaces the committed `mode` (mirroring
/// `DelegationMap::with_local_override`). When **neither** file exists, returns
/// `StoreMode::default()` (`Shared`) — zero-setup stores see zero change. A
/// malformed or unsupported-version file is a hard error (unlike the advisory
/// delegates merge: the mode gates where bytes land, so a misread must never
/// silently fall back).
pub(crate) fn resolve_store_mode(worktree_root: &Path) -> Result<StoreMode> {
    let committed = load_store_config(&worktree_root.join(STORE_CONFIG_REL_PATH))?;
    let local = load_store_config(&worktree_root.join(STORE_CONFIG_LOCAL_REL_PATH))?;
    // Local wins; otherwise committed; otherwise the default.
    Ok(local
        .or(committed)
        .map(|config| config.mode)
        .unwrap_or_default())
}

/// Load and validate a store-config file if present; absent → `None`.
fn load_store_config(path: &Path) -> Result<Option<StoreConfig>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(ShoreError::Message(format!(
                "read store config {}: {error}",
                path.display()
            )));
        }
    };
    // A malformed config is a HARD error (privacy: never silently fall back to
    // Shared and land bytes in the shared store). The message must be ACTIONABLE
    // — name the file, the parse problem, the valid `mode` values, and the
    // command that rewrites it.
    let config: StoreConfig = serde_json::from_slice(&bytes).map_err(|error| {
        ShoreError::Message(format!(
            "store config {} is malformed: {error}; \
             expected a JSON document with a \"mode\" of \"shared\" or \"ephemeral\" \
             (e.g. run `shore store mode shared` to rewrite it)",
            path.display()
        ))
    })?;
    config.validate_schema_version(path)?;
    Ok(Some(config))
}

/// Persist the committed `.shore/store.json` for `worktree_root` with `mode`.
/// Pretty-printed with a trailing newline, like `write_delegates`, so a committed
/// config diffs cleanly. The CLI is the only caller; resolution never writes.
pub(crate) fn write_store_config(worktree_root: &Path, mode: StoreMode) -> Result<()> {
    let path = worktree_root.join(STORE_CONFIG_REL_PATH);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            ShoreError::Message(format!("create {}: {error}", parent.display()))
        })?;
    }
    let mut bytes = serde_json::to_vec_pretty(&StoreConfig::new(mode))?;
    bytes.push(b'\n');
    std::fs::write(&path, &bytes)
        .map_err(|error| ShoreError::Message(format!("write {}: {error}", path.display())))
}

/// Which layer the effective store mode was sourced from, so a reporting command
/// can explain *why* a worktree resolves the mode it does without leaking a path.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StoreModeSource {
    /// Neither config file is present; the built-in default applies.
    Default,
    /// The committed `.shore/store.json` supplied the mode.
    Committed,
    /// The git-excluded `.shore/store.local.json` override supplied the mode.
    Local,
}

/// The effective store mode for a worktree together with the layer it came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoreModeOutcome {
    pub mode: StoreMode,
    pub source: StoreModeSource,
}

/// Resolve the effective store mode for `repo` (the worktree root or any path
/// inside it) and report which layer it came from. The library entry point the
/// `store mode show` CLI consumes — it keeps the worktree-root resolution and
/// source classification on the library side of the boundary so the binary crate
/// never names the crate-internal config helpers.
pub fn resolve_store_mode_for_repo(repo: &Path) -> Result<StoreModeOutcome> {
    let worktree_root = git_worktree_root(repo)?;
    // Validate + resolve first, so a malformed/unsupported file errors before we
    // attribute a source; then classify by presence using the same precedence as
    // `resolve_store_mode` (local wins, else committed, else default).
    let mode = resolve_store_mode(&worktree_root)?;
    let source = if worktree_root.join(STORE_CONFIG_LOCAL_REL_PATH).exists() {
        StoreModeSource::Local
    } else if worktree_root.join(STORE_CONFIG_REL_PATH).exists() {
        StoreModeSource::Committed
    } else {
        StoreModeSource::Default
    };
    Ok(StoreModeOutcome { mode, source })
}

/// Persist `mode` to the committed `.shore/store.json` for `repo` (the worktree
/// root or any path inside it). The library entry point the `store mode
/// shared|ephemeral` CLI consumes. Opting into `Ephemeral` also ensures the
/// committed `.shore/.gitignore`, so the soon-to-exist worktree-local
/// `.shore/data/` store is covered before its first write; the committed
/// `store.json` itself is tracked and never excluded.
pub fn set_store_mode_for_repo(repo: &Path, mode: StoreMode) -> Result<()> {
    let worktree_root = git_worktree_root(repo)?;
    if mode == StoreMode::Ephemeral {
        crate::session::store::store_init::ensure_shore_gitignore(&worktree_root)?;
    }
    write_store_config(&worktree_root, mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &std::path::Path, rel: &str, contents: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn store_mode_defaults_to_shared() {
        assert_eq!(StoreMode::default(), StoreMode::Shared);
    }

    #[test]
    fn absent_both_files_resolves_to_shared() {
        // Zero-setup stores see zero change: no config means the default mode.
        let root = tempfile::tempdir().unwrap();
        assert_eq!(resolve_store_mode(root.path()).unwrap(), StoreMode::Shared);
    }

    #[test]
    fn committed_config_round_trips_through_the_reader() {
        // A persisted `.shore/store.json` reads back to the mode it stored.
        let root = tempfile::tempdir().unwrap();
        write_store_config(root.path(), StoreMode::Ephemeral).unwrap();
        assert!(root.path().join(".shore/store.json").is_file());
        assert_eq!(
            resolve_store_mode(root.path()).unwrap(),
            StoreMode::Ephemeral
        );
    }

    #[test]
    fn camel_case_mode_strings_are_used_on_the_wire() {
        // The serialized document spells the variants in camelCase.
        let root = tempfile::tempdir().unwrap();
        write_store_config(root.path(), StoreMode::Ephemeral).unwrap();
        let raw = std::fs::read_to_string(root.path().join(".shore/store.json")).unwrap();
        assert!(raw.contains("\"mode\": \"ephemeral\""), "got: {raw}");
        assert!(
            raw.contains("\"schema\": \"shore.store-config\""),
            "got: {raw}"
        );
    }

    #[test]
    fn local_override_wins_over_committed() {
        // committed = shared, local = ephemeral -> effective ephemeral (local wins,
        // mirroring DelegationMap::with_local_override).
        let root = tempfile::tempdir().unwrap();
        write(root.path(), ".shore/store.json", SHARED_DOC);
        write(root.path(), ".shore/store.local.json", EPHEMERAL_DOC);
        assert_eq!(
            resolve_store_mode(root.path()).unwrap(),
            StoreMode::Ephemeral
        );
    }

    #[test]
    fn local_alone_is_used_when_committed_absent() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), ".shore/store.local.json", EPHEMERAL_DOC);
        assert_eq!(
            resolve_store_mode(root.path()).unwrap(),
            StoreMode::Ephemeral
        );
    }

    #[test]
    fn committed_alone_is_used_when_local_absent() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), ".shore/store.json", EPHEMERAL_DOC);
        assert_eq!(
            resolve_store_mode(root.path()).unwrap(),
            StoreMode::Ephemeral
        );
    }

    #[test]
    fn unsupported_schema_version_is_rejected_with_actionable_message() {
        // Mirror manifest.rs validate_schema_version: a wrong schema/version errors.
        // The hard error MUST be actionable so users know how to fix it — naming
        // the offending file (with a committed and a local config both possible,
        // a path-free message can't say which to rewrite) and the fix command.
        let root = tempfile::tempdir().unwrap();
        write(
            root.path(),
            ".shore/store.local.json",
            r#"{"schema":"shore.store-config","version":999,"mode":"shared"}"#,
        );
        let err = resolve_store_mode(root.path()).unwrap_err().to_string();
        assert!(err.contains("store mode"), "names the fix command: {err}");
        assert!(
            err.contains("store.local.json"),
            "names the offending file: {err}"
        );
    }

    #[test]
    fn malformed_config_is_rejected_with_actionable_message() {
        // Not valid JSON / wrong shape → hard error naming the file + the fix
        // (never a silent fallback to Shared — privacy).
        let root = tempfile::tempdir().unwrap();
        write(root.path(), ".shore/store.json", "{ not json");
        let err = resolve_store_mode(root.path()).unwrap_err().to_string();
        assert!(err.contains("store.json"), "names the file: {err}");
        assert!(
            err.contains("shared") && err.contains("ephemeral"),
            "names the valid modes: {err}"
        );
    }

    const SHARED_DOC: &str = r#"{"schema":"shore.store-config","version":1,"mode":"shared"}"#;
    const EPHEMERAL_DOC: &str = r#"{"schema":"shore.store-config","version":1,"mode":"ephemeral"}"#;
}
