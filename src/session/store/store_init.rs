use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Result, ShoreError};
use crate::git::{git_info_exclude_path, git_path_is_ignored, git_worktree_root};
use crate::storage::{LocalStorage, TempSweepAge};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ShoreStorePaths {
    worktree_root: PathBuf,
    store_dir: PathBuf,
}

impl ShoreStorePaths {
    pub(crate) fn resolve(repo: impl AsRef<Path>) -> Result<Self> {
        let worktree_root = git_worktree_root(repo.as_ref())?;
        let store_dir = worktree_root.join(".shore/data");
        // Hard cutover: a pre-relocation flat store (events/state.json directly
        // under `.shore/`) is a loud, actionable error rather than a silent
        // dual-read. Detection keys on the layout, not the directory name, so a
        // `.shore/` that holds only committed config resolves cleanly.
        match detect_store_layout(&worktree_root.join(".shore")) {
            StoreLayout::Conflict => {
                return Err(ShoreError::Message(
                    "both a legacy flat .shore/ store and a migrated .shore/data/ store are \
                     present; this is a partial/interrupted migration — inspect both and remove \
                     the stale one (the migration removes the flat store only after a successful \
                     relocation)"
                        .to_owned(),
                ));
            }
            StoreLayout::Flat => {
                return Err(ShoreError::Message(
                    "legacy flat .shore/ store detected; run `just migrate-store` to relocate it \
                     to .shore/data/ and upgrade event writer fields"
                        .to_owned(),
                ));
            }
            StoreLayout::Fresh | StoreLayout::Nested => {}
        }
        Ok(Self {
            worktree_root,
            store_dir,
        })
    }

    pub(crate) fn worktree_root(&self) -> &Path {
        &self.worktree_root
    }

    pub(crate) fn store_dir(&self) -> &Path {
        &self.store_dir
    }

    pub(crate) fn state_path(&self) -> PathBuf {
        self.store_dir.join("state.json")
    }
}

pub fn store_dir_for_repo(repo: &Path) -> Result<PathBuf> {
    Ok(ShoreStorePaths::resolve(repo)?.store_dir().to_path_buf())
}

/// The worktree-local store entries that, when found directly under `.shore/`,
/// mark a pre-relocation flat store. This is the single source of truth shared
/// by the resolve-time layout guard and the migration's relocation step, so the
/// two never diverge on which shapes count as a legacy store (a registration-only
/// linked checkout is a store just as much as one with events). It deliberately
/// excludes the committed config siblings (`delegates.json`,
/// `allowed-signers.json`), so a config-only `.shore/` is not a store.
pub(crate) const FLAT_STORE_MARKERS: &[&str] = &[
    "events",
    "artifacts",
    "state.json",
    "store-registration.json",
];

/// True when any flat-store marker sits directly under `shore`
/// (`<worktree-root>/.shore`) — the pre-relocation layout.
fn flat_store_marker_present(shore: &Path) -> bool {
    FLAT_STORE_MARKERS
        .iter()
        .any(|entry| shore.join(entry).exists())
}

/// The on-disk layout of a `.shore/` directory, classified for the hard-cutover
/// guard. Detection keys on flat-store markers versus the nested `.shore/data/`,
/// never on the `.shore/` directory itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StoreLayout {
    /// No flat-store markers and no `.shore/data/`: a fresh repo or a `.shore/`
    /// that holds only committed config (`delegates.json`).
    Fresh,
    /// Flat-store markers (events/artifacts/state.json/store-registration.json
    /// directly under `.shore/`) and no `.shore/data/`: a pre-relocation store
    /// that must be migrated.
    Flat,
    /// `.shore/data/` present and no flat markers: the migrated steady state.
    Nested,
    /// Both flat markers and `.shore/data/`: an interrupted/partial migration.
    Conflict,
}

/// Classify the store layout under `shore` (`<worktree-root>/.shore`). A
/// config-only `.shore/` (committed `delegates.json` and no store) is `Fresh`,
/// because the probes look only for flat-store markers and the nested dir.
pub(crate) fn detect_store_layout(shore: &Path) -> StoreLayout {
    let nested = shore.join("data").exists();
    let flat = flat_store_marker_present(shore);
    match (flat, nested) {
        (true, true) => StoreLayout::Conflict,
        (true, false) => StoreLayout::Flat,
        (false, true) => StoreLayout::Nested,
        (false, false) => StoreLayout::Fresh,
    }
}

pub(crate) fn ensure_store_dirs(store_dir: &Path) -> Result<()> {
    for dir in [
        store_dir.join("events"),
        store_dir.join("artifacts/notes"),
        store_dir.join("artifacts/revisions"),
        store_dir.join("artifacts/snapshots"),
    ] {
        fs::create_dir_all(&dir).map_err(|error| io_error("create directory", &dir, error))?;
    }
    Ok(())
}

pub(crate) fn sweep_stale_temp_files(storage: &LocalStorage, store_dir: &Path) -> Result<()> {
    storage.sweep_temp_files(store_dir, TempSweepAge::workflow_startup())
}

/// Shared writer setup against an explicit store dir and worktree root: sweep
/// stale temp files, ensure the store directory layout, and register the three
/// `.git/info/exclude` entries (the private delegates and actor-attributes
/// overrides, then the `.shore/data/` store). The write-landing seam's
/// `prepare_write_landing` calls this with the resolved write store dir
/// (clone-local in linked mode) and the worktree root, so every write workflow
/// shares one exclude body and they can never drift on which excludes are written.
pub(crate) fn prepare_store_writer_at(
    storage: &LocalStorage,
    store_dir: &Path,
    worktree_root: &Path,
) -> Result<()> {
    sweep_stale_temp_files(storage, store_dir)?;
    ensure_store_dirs(store_dir)?;
    // Record the private override's own exclude entry before the store exclude,
    // so it is captured explicitly even when a broader store exclude pattern is
    // present (and would otherwise mask the more specific probe).
    ensure_local_delegates_excluded(worktree_root)?;
    ensure_local_actor_attributes_excluded(worktree_root)?;
    ensure_local_store_config_excluded(worktree_root)?;
    ensure_shore_storage_excluded(worktree_root)
}

/// Keeps the `.shore/data/` store out of Git status without modifying any
/// tracked project file.
///
/// Shoreline registers `.shore/data/` in the repository-local
/// `.git/info/exclude` rather than the worktree `.gitignore`, so initializing or
/// writing review state never dirties the working tree and never leaks an
/// ignore-file edit into a captured ReviewUnit. The entry is the narrow
/// `.shore/data/` (not a wholesale `.shore/`) so committed config siblings —
/// `.shore/delegates.json`, `.shore/allowed-signers.json` — stay tracked. If
/// `.shore/data/` is already ignored by any standard source — a project
/// `.gitignore` entry, the global excludes file, or an existing local exclude
/// entry — this is a no-op, so user-managed ignore files are respected and never
/// rewritten.
pub fn ensure_shore_storage_excluded(worktree_root: &Path) -> Result<()> {
    // Probe a path under `.shore/data/` so directory-only patterns
    // (`.shore/data/`) match regardless of whether the directory exists on disk
    // yet, mirroring how untracked discovery applies `--exclude-standard`.
    if git_path_is_ignored(worktree_root, ".shore/data/state.json")? {
        return Ok(());
    }
    append_info_exclude_line(worktree_root, ".shore/data/")
}

/// Keeps the private delegates override out of Git status without touching any
/// tracked file. Mirrors [`ensure_shore_storage_excluded`]: if the path is
/// already ignored by any standard source this is a no-op; otherwise it appends
/// the entry to the repository-local `.git/info/exclude`.
///
/// Only the `.local.json` override is excluded — the committed
/// `.shore/delegates.json` and `.shore/allowed-signers.json` are deliberately
/// tracked and never excluded.
///
/// `pub` so the possession-based `--local` identity CLIs (`enroll`/`attest`) can
/// call it before any store write — that path may run before `prepare_store_writer_at`
/// (which also calls it) ever does.
pub fn ensure_local_delegates_excluded(worktree_root: &Path) -> Result<()> {
    if git_path_is_ignored(worktree_root, ".shore/delegates.local.json")? {
        return Ok(());
    }
    append_info_exclude_line(worktree_root, ".shore/delegates.local.json")
}

/// Keeps the private actor-attributes override out of Git status. Mirrors
/// [`ensure_local_delegates_excluded`]: a no-op if already ignored, else appends
/// to the repository-local `.git/info/exclude`. Only the `.local.json` override
/// is excluded — the committed `.shore/actor-attributes.json` is tracked.
/// `pub` for the same reason as [`ensure_local_delegates_excluded`]: the `--local`
/// `attest` CLI calls it before staging the override.
pub fn ensure_local_actor_attributes_excluded(worktree_root: &Path) -> Result<()> {
    if git_path_is_ignored(worktree_root, ".shore/actor-attributes.local.json")? {
        return Ok(());
    }
    append_info_exclude_line(worktree_root, ".shore/actor-attributes.local.json")
}

/// Keeps the private store-config override out of Git status. Mirrors
/// [`ensure_local_delegates_excluded`]: a no-op if already ignored, else appends
/// to the repository-local `.git/info/exclude`. Only the `.local.json` override
/// is excluded — the committed `.shore/store.json` is tracked. Crate-internal
/// (unlike the delegates/actor-attributes twins): there is no possession-based
/// `--local` store-config CLI, so the only caller is `prepare_store_writer_at`.
pub(crate) fn ensure_local_store_config_excluded(worktree_root: &Path) -> Result<()> {
    if git_path_is_ignored(worktree_root, ".shore/store.local.json")? {
        return Ok(());
    }
    append_info_exclude_line(worktree_root, ".shore/store.local.json")
}

/// Append `line` (newline-terminated) to the repository-local
/// `.git/info/exclude`, creating the file and its parent if needed. Callers
/// guard against duplicate entries before calling.
fn append_info_exclude_line(worktree_root: &Path, line: &str) -> Result<()> {
    let exclude_path = git_info_exclude_path(worktree_root)?;
    if let Some(parent) = exclude_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| io_error("create git info directory", parent, error))?;
    }

    let current = match fs::read_to_string(&exclude_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(io_error("read git exclude file", &exclude_path, error));
        }
    };

    let mut updated = current;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(line);
    updated.push('\n');

    fs::write(&exclude_path, updated)
        .map_err(|error| io_error("write git exclude file", &exclude_path, error))
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> ShoreError {
    ShoreError::Message(format!("{action} {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::*;

    #[test]
    fn shore_store_paths_resolve_from_subdirectory() {
        let repo = git_repo();
        fs::create_dir_all(repo.path().join("src/nested")).unwrap();
        let paths = ShoreStorePaths::resolve(repo.path().join("src/nested")).unwrap();

        assert_existing_paths_eq(paths.worktree_root(), repo.path());
        // The store dir is now <root>/.shore/data.
        assert_eq!(path_file_name(paths.store_dir()), "data");
        assert_eq!(path_file_name(path_parent(paths.store_dir())), ".shore");
        assert_existing_paths_eq(path_parent(path_parent(paths.store_dir())), repo.path());
        // state.json is <root>/.shore/data/state.json.
        assert_eq!(path_file_name(paths.state_path().as_path()), "state.json");
        assert_eq!(
            path_file_name(path_parent(paths.state_path().as_path())),
            "data"
        );
        assert_eq!(
            path_file_name(path_parent(path_parent(paths.state_path().as_path()))),
            ".shore"
        );
    }

    #[test]
    fn public_shore_dir_helper_delegates_to_store_paths() {
        let repo = git_repo();

        let from_public_helper = store_dir_for_repo(repo.path()).unwrap();
        let from_paths = ShoreStorePaths::resolve(repo.path())
            .unwrap()
            .store_dir()
            .to_path_buf();

        assert_eq!(from_public_helper, from_paths);
    }

    fn assert_existing_paths_eq(actual: &Path, expected: &Path) {
        assert_eq!(
            actual.canonicalize().expect("canonicalize actual path"),
            expected.canonicalize().expect("canonicalize expected path")
        );
    }

    fn path_parent(path: &Path) -> &Path {
        path.parent().expect("path has parent")
    }

    fn path_file_name(path: &Path) -> &str {
        path.file_name()
            .and_then(|name| name.to_str())
            .expect("path has utf-8 file name")
    }

    #[test]
    fn prepare_store_writer_at_creates_current_store_dirs_and_local_exclude_entry() {
        let repo = git_repo();
        let paths = ShoreStorePaths::resolve(repo.path()).unwrap();
        let storage = LocalStorage::new(paths.store_dir());

        prepare_store_writer_at(&storage, paths.store_dir(), paths.worktree_root()).unwrap();

        assert!(paths.store_dir().join("events").is_dir());
        assert!(paths.store_dir().join("artifacts/notes").is_dir());
        assert!(paths.store_dir().join("artifacts/revisions").is_dir());
        assert!(paths.store_dir().join("artifacts/snapshots").is_dir());

        // Storage is ignored via the repository-local exclude, never the
        // tracked worktree .gitignore.
        assert!(
            !repo.path().join(".gitignore").exists(),
            "writer setup must not create a tracked .gitignore"
        );
        let exclude = fs::read_to_string(git_info_exclude_path(repo.path()).unwrap()).unwrap();
        assert!(
            exclude.lines().any(|line| line.trim() == ".shore/data/"),
            "local exclude should list .shore/data/, got:\n{exclude}"
        );
    }

    #[test]
    fn prepare_store_writer_at_excludes_local_delegates_override() {
        let repo = git_repo();
        let paths = ShoreStorePaths::resolve(repo.path()).unwrap();
        let storage = LocalStorage::new(paths.store_dir());

        prepare_store_writer_at(&storage, paths.store_dir(), paths.worktree_root()).unwrap();

        let exclude = fs::read_to_string(git_info_exclude_path(repo.path()).unwrap()).unwrap();
        assert!(
            exclude
                .lines()
                .any(|line| line.trim() == ".shore/delegates.local.json"),
            "local delegates override must be git-excluded, got:\n{exclude}"
        );
        // Still no tracked .gitignore (same posture as the store exclusion).
        assert!(!repo.path().join(".gitignore").exists());
    }

    #[test]
    fn ensure_local_delegates_excluded_is_idempotent() {
        let repo = git_repo();
        ensure_local_delegates_excluded(repo.path()).unwrap();
        ensure_local_delegates_excluded(repo.path()).unwrap();
        let exclude = fs::read_to_string(git_info_exclude_path(repo.path()).unwrap()).unwrap();
        let hits = exclude
            .lines()
            .filter(|l| l.trim() == ".shore/delegates.local.json")
            .count();
        assert_eq!(hits, 1, "the entry is written at most once");
    }

    #[test]
    fn prepare_store_writer_at_excludes_local_store_config_override() {
        let repo = git_repo();
        let paths = ShoreStorePaths::resolve(repo.path()).unwrap();
        let storage = LocalStorage::new(paths.store_dir());

        prepare_store_writer_at(&storage, paths.store_dir(), paths.worktree_root()).unwrap();

        let exclude = fs::read_to_string(git_info_exclude_path(repo.path()).unwrap()).unwrap();
        assert!(
            exclude
                .lines()
                .any(|line| line.trim() == ".shore/store.local.json"),
            "local store-config override must be git-excluded, got:\n{exclude}"
        );
        // The committed config is never excluded.
        assert!(
            !exclude
                .lines()
                .any(|line| line.trim() == ".shore/store.json")
        );
    }

    #[test]
    fn ensure_local_store_config_excluded_is_idempotent() {
        let repo = git_repo();
        ensure_local_store_config_excluded(repo.path()).unwrap();
        ensure_local_store_config_excluded(repo.path()).unwrap();
        let exclude = fs::read_to_string(git_info_exclude_path(repo.path()).unwrap()).unwrap();
        let hits = exclude
            .lines()
            .filter(|l| l.trim() == ".shore/store.local.json")
            .count();
        assert_eq!(hits, 1, "the entry is written at most once");
    }

    #[test]
    fn prepare_store_writer_at_excludes_local_actor_attributes_override() {
        let repo = git_repo();
        let paths = ShoreStorePaths::resolve(repo.path()).unwrap();
        let storage = LocalStorage::new(paths.store_dir());

        prepare_store_writer_at(&storage, paths.store_dir(), paths.worktree_root()).unwrap();

        let exclude = fs::read_to_string(git_info_exclude_path(repo.path()).unwrap()).unwrap();
        assert!(
            exclude
                .lines()
                .any(|line| line.trim() == ".shore/actor-attributes.local.json"),
            "local actor-attributes override must be git-excluded, got:\n{exclude}"
        );
        // Committed config is never excluded.
        assert!(
            !exclude
                .lines()
                .any(|line| line.trim() == ".shore/actor-attributes.json")
        );
    }

    #[test]
    fn ensure_local_actor_attributes_excluded_is_idempotent() {
        let repo = git_repo();
        ensure_local_actor_attributes_excluded(repo.path()).unwrap();
        ensure_local_actor_attributes_excluded(repo.path()).unwrap();
        let exclude = fs::read_to_string(git_info_exclude_path(repo.path()).unwrap()).unwrap();
        let hits = exclude
            .lines()
            .filter(|l| l.trim() == ".shore/actor-attributes.local.json")
            .count();
        assert_eq!(hits, 1, "the entry is written at most once");
    }

    #[test]
    fn prepare_store_writer_at_preserves_fresh_temp_files() {
        let repo = git_repo();
        let paths = ShoreStorePaths::resolve(repo.path()).unwrap();
        fs::create_dir_all(paths.store_dir().join("events")).unwrap();
        let temp = paths.store_dir().join("events/.shore-write.fresh.tmp");
        fs::write(&temp, "in flight").unwrap();
        let storage = LocalStorage::new(paths.store_dir());

        prepare_store_writer_at(&storage, paths.store_dir(), paths.worktree_root()).unwrap();

        assert_eq!(fs::read_to_string(temp).unwrap(), "in flight");
    }

    #[test]
    fn legacy_flat_store_returns_migrate_hint() {
        let repo = git_repo();
        // Pre-migration FLAT store: events + state.json directly under .shore/,
        // no .shore/data/.
        fs::create_dir_all(repo.path().join(".shore/events")).unwrap();
        fs::write(repo.path().join(".shore/state.json"), "{}").unwrap();

        let err = ShoreStorePaths::resolve(repo.path())
            .expect_err("legacy flat .shore/ store must be a loud, actionable error");
        let message = err.to_string();
        assert!(
            message.contains("migrate-store"),
            "names the fix; got: {message}"
        );
        assert!(
            message.contains(".shore"),
            "names the legacy store; got: {message}"
        );
    }

    #[test]
    fn both_flat_and_nested_store_is_a_conflict_error() {
        let repo = git_repo();
        // Interrupted/partial migration left BOTH the flat store and the nested
        // one. Must be LOUD — never silently prefer .shore/data/ and orphan the
        // flat store.
        fs::create_dir_all(repo.path().join(".shore/events")).unwrap();
        fs::create_dir_all(repo.path().join(".shore/data/events")).unwrap();
        let err = ShoreStorePaths::resolve(repo.path())
            .expect_err("flat + nested store must be a conflict");
        let message = err.to_string();
        assert!(
            message.contains(".shore/data"),
            "names the nested store; got: {message}"
        );
        assert!(
            message.contains("both") || message.contains("conflict"),
            "reads as a conflict: {message}"
        );
    }

    #[test]
    fn migrated_nested_store_resolves_cleanly() {
        let repo = git_repo();
        // Post-migration steady state: only the nested store, no flat markers.
        fs::create_dir_all(repo.path().join(".shore/data/events")).unwrap();
        let paths = ShoreStorePaths::resolve(repo.path()).expect("nested store resolves");
        assert_eq!(path_file_name(paths.store_dir()), "data");
    }

    #[test]
    fn legacy_registration_only_store_returns_migrate_hint() {
        let repo = git_repo();
        // A registered linked checkout from before the relocation: only the
        // top-level registration file, no .shore/events, .shore/state.json, or
        // .shore/data/. The new code reads the registration from .shore/data/,
        // so without the guard this would silently resolve as worktree-local and
        // drop the registration.
        fs::create_dir_all(repo.path().join(".shore")).unwrap();
        fs::write(repo.path().join(".shore/store-registration.json"), "{}").unwrap();

        let err = ShoreStorePaths::resolve(repo.path())
            .expect_err("a top-level registration is a legacy store, not a clean resolve");
        let message = err.to_string();
        assert!(
            message.contains("migrate-store"),
            "names the fix; got: {message}"
        );
    }

    #[test]
    fn config_only_shore_dir_is_not_a_legacy_store() {
        let repo = git_repo();
        // .shore/ holds ONLY committed config (no store yet). Must NOT trip the
        // legacy guard — committed config now legitimately lives under .shore/.
        fs::create_dir_all(repo.path().join(".shore")).unwrap();
        fs::write(
            repo.path().join(".shore/delegates.json"),
            r#"{"delegates":{}}"#,
        )
        .unwrap();
        let paths = ShoreStorePaths::resolve(repo.path()).expect("config-only .shore/ resolves");
        assert_eq!(path_file_name(paths.store_dir()), "data");
    }

    #[test]
    fn fresh_repo_with_no_shore_dir_resolves_cleanly() {
        let repo = git_repo();
        let paths = ShoreStorePaths::resolve(repo.path()).expect("fresh repo resolves");
        assert_eq!(path_file_name(paths.store_dir()), "data");
    }

    fn git_repo() -> tempfile::TempDir {
        let repo = tempfile::tempdir().expect("create temp git repository directory");
        let output = Command::new("git")
            .arg("init")
            .current_dir(repo.path())
            .output()
            .expect("run git init");
        assert!(
            output.status.success(),
            "git init failed in {}:\nstdout:\n{}\nstderr:\n{}",
            repo.path().display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        repo
    }
}
