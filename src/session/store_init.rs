use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Result, ShoreError};
use crate::git::git_worktree_root;

pub fn shore_dir_for_repo(repo: &Path) -> Result<PathBuf> {
    Ok(git_worktree_root(repo)?.join(".shore"))
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

pub fn ensure_shore_ignored(worktree_root: &Path) -> Result<()> {
    let gitignore_path = worktree_root.join(".gitignore");
    let current = match fs::read_to_string(&gitignore_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(io_error("read .gitignore", &gitignore_path, error));
        }
    };

    if has_shore_ignore_entry(&current) {
        return Ok(());
    }

    let mut updated = current;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(".shore/\n");

    fs::write(&gitignore_path, updated)
        .map_err(|error| io_error("write .gitignore", &gitignore_path, error))
}

fn has_shore_ignore_entry(contents: &str) -> bool {
    contents
        .lines()
        .map(str::trim)
        .any(|line| matches!(line, ".shore" | ".shore/" | "/.shore" | "/.shore/"))
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> ShoreError {
    ShoreError::Message(format!("{action} {}: {error}", path.display()))
}
