use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Result, ShoreError};
use crate::git::{git_info_exclude_path, git_path_is_ignored, git_worktree_root};
use crate::storage::{LocalStorage, TempSweepAge};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ShoreStorePaths {
    worktree_root: PathBuf,
    shore_dir: PathBuf,
}

impl ShoreStorePaths {
    pub(crate) fn resolve(repo: impl AsRef<Path>) -> Result<Self> {
        let worktree_root = git_worktree_root(repo.as_ref())?;
        let shore_dir = worktree_root.join(".shore");
        Ok(Self {
            worktree_root,
            shore_dir,
        })
    }

    pub(crate) fn worktree_root(&self) -> &Path {
        &self.worktree_root
    }

    pub(crate) fn shore_dir(&self) -> &Path {
        &self.shore_dir
    }

    pub(crate) fn state_path(&self) -> PathBuf {
        self.shore_dir.join("state.json")
    }
}

pub fn shore_dir_for_repo(repo: &Path) -> Result<PathBuf> {
    Ok(ShoreStorePaths::resolve(repo)?.shore_dir().to_path_buf())
}

pub(crate) fn ensure_store_dirs(shore_dir: &Path) -> Result<()> {
    for dir in [
        shore_dir.join("events"),
        shore_dir.join("artifacts/notes"),
        shore_dir.join("artifacts/revisions"),
        shore_dir.join("artifacts/snapshots"),
    ] {
        fs::create_dir_all(&dir).map_err(|error| io_error("create directory", &dir, error))?;
    }
    Ok(())
}

pub(crate) fn sweep_stale_temp_files(storage: &LocalStorage, shore_dir: &Path) -> Result<()> {
    storage.sweep_temp_files(shore_dir, TempSweepAge::workflow_startup())
}

pub(crate) fn prepare_shore_writer(paths: &ShoreStorePaths, storage: &LocalStorage) -> Result<()> {
    sweep_stale_temp_files(storage, paths.shore_dir())?;
    ensure_store_dirs(paths.shore_dir())?;
    // Record the private override's own exclude entry before the store exclude,
    // so it is captured explicitly even when a broader store exclude pattern is
    // present (and would otherwise mask the more specific probe).
    ensure_local_delegates_excluded(paths.worktree_root())?;
    ensure_shore_storage_excluded(paths.worktree_root())
}

/// Keeps `.shore/` storage out of Git status without modifying any tracked
/// project file.
///
/// Shoreline registers `.shore/` in the repository-local `.git/info/exclude`
/// rather than the worktree `.gitignore`, so initializing or writing review
/// state never dirties the working tree and never leaks an ignore-file edit
/// into a captured ReviewUnit. If `.shore/` is already ignored by any standard
/// source — a project `.gitignore` entry, the global excludes file, or an
/// existing local exclude entry — this is a no-op, so user-managed ignore files
/// are respected and never rewritten.
pub fn ensure_shore_storage_excluded(worktree_root: &Path) -> Result<()> {
    // Probe a path under `.shore/` so directory-only patterns (`.shore/`) match
    // regardless of whether the directory exists on disk yet, mirroring how
    // untracked discovery applies `--exclude-standard`.
    if git_path_is_ignored(worktree_root, ".shore/state.json")? {
        return Ok(());
    }
    append_info_exclude_line(worktree_root, ".shore/")
}

/// Keeps the private delegates override out of Git status without touching any
/// tracked file. Mirrors [`ensure_shore_storage_excluded`]: if the path is
/// already ignored by any standard source this is a no-op; otherwise it appends
/// the entry to the repository-local `.git/info/exclude`.
///
/// Only the `.local.json` override is excluded — the committed
/// `.shore/delegates.json` and `.shore/allowed-signers.json` are deliberately
/// tracked and never excluded.
pub(crate) fn ensure_local_delegates_excluded(worktree_root: &Path) -> Result<()> {
    if git_path_is_ignored(worktree_root, ".shore/delegates.local.json")? {
        return Ok(());
    }
    append_info_exclude_line(worktree_root, ".shore/delegates.local.json")
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
        assert_eq!(path_file_name(paths.shore_dir()), ".shore");
        assert_existing_paths_eq(path_parent(paths.shore_dir()), repo.path());
        assert_eq!(path_file_name(paths.state_path().as_path()), "state.json");
        assert_eq!(
            path_file_name(path_parent(paths.state_path().as_path())),
            ".shore"
        );
        assert_existing_paths_eq(
            path_parent(path_parent(paths.state_path().as_path())),
            repo.path(),
        );
    }

    #[test]
    fn public_shore_dir_helper_delegates_to_store_paths() {
        let repo = git_repo();

        let from_public_helper = shore_dir_for_repo(repo.path()).unwrap();
        let from_paths = ShoreStorePaths::resolve(repo.path())
            .unwrap()
            .shore_dir()
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
    fn prepare_shore_writer_creates_current_store_dirs_and_local_exclude_entry() {
        let repo = git_repo();
        let paths = ShoreStorePaths::resolve(repo.path()).unwrap();
        let storage = LocalStorage::new(paths.shore_dir());

        prepare_shore_writer(&paths, &storage).unwrap();

        assert!(paths.shore_dir().join("events").is_dir());
        assert!(paths.shore_dir().join("artifacts/notes").is_dir());
        assert!(paths.shore_dir().join("artifacts/revisions").is_dir());
        assert!(paths.shore_dir().join("artifacts/snapshots").is_dir());

        // Storage is ignored via the repository-local exclude, never the
        // tracked worktree .gitignore.
        assert!(
            !repo.path().join(".gitignore").exists(),
            "writer setup must not create a tracked .gitignore"
        );
        let exclude = fs::read_to_string(git_info_exclude_path(repo.path()).unwrap()).unwrap();
        assert!(
            exclude.lines().any(|line| line.trim() == ".shore/"),
            "local exclude should list .shore/, got:\n{exclude}"
        );
    }

    #[test]
    fn prepare_shore_writer_excludes_local_delegates_override() {
        let repo = git_repo();
        let paths = ShoreStorePaths::resolve(repo.path()).unwrap();
        let storage = LocalStorage::new(paths.shore_dir());

        prepare_shore_writer(&paths, &storage).unwrap();

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
    fn prepare_shore_writer_preserves_fresh_temp_files() {
        let repo = git_repo();
        let paths = ShoreStorePaths::resolve(repo.path()).unwrap();
        fs::create_dir_all(paths.shore_dir().join("events")).unwrap();
        let temp = paths.shore_dir().join("events/.shore-write.fresh.tmp");
        fs::write(&temp, "in flight").unwrap();
        let storage = LocalStorage::new(paths.shore_dir());

        prepare_shore_writer(&paths, &storage).unwrap();

        assert_eq!(fs::read_to_string(temp).unwrap(), "in flight");
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
