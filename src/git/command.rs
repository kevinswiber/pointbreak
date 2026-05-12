use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Result, ShoreError};

#[derive(Debug)]
pub(crate) struct GitOutput {
    pub stdout: Vec<u8>,
}

pub(crate) fn run_git<I, S>(cwd: &Path, args: I) -> Result<GitOutput>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run_git_allowing_statuses(cwd, args, &[0])
}

pub fn git_worktree_root(repo: &Path) -> Result<PathBuf> {
    let output = run_git(repo, ["rev-parse", "--show-toplevel"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let root = stdout.trim_end_matches(['\r', '\n']);
    if root.is_empty() {
        return Err(ShoreError::Message(format!(
            "git rev-parse returned empty worktree root for {}",
            repo.display()
        )));
    }

    Ok(PathBuf::from(root))
}

pub fn git_head_oid(repo: &Path) -> Result<String> {
    let output = run_git(repo, ["rev-parse", "HEAD"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let oid = stdout.trim_end_matches(['\r', '\n']);
    if oid.is_empty() {
        return Err(ShoreError::Message(format!(
            "git rev-parse returned empty HEAD oid for {}",
            repo.display()
        )));
    }

    Ok(oid.to_owned())
}

pub fn git_head_tree_oid(repo: &Path) -> Result<String> {
    let output = run_git(repo, ["rev-parse", "HEAD^{tree}"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let oid = stdout.trim_end_matches(['\r', '\n']);
    if oid.is_empty() {
        return Err(ShoreError::Message(format!(
            "git rev-parse returned empty HEAD tree oid for {}",
            repo.display()
        )));
    }

    Ok(oid.to_owned())
}

pub(crate) fn run_git_allowing_statuses<I, S>(
    cwd: &Path,
    args: I,
    allowed_statuses: &[i32],
) -> Result<GitOutput>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args = args
        .into_iter()
        .map(|arg| arg.as_ref().to_owned())
        .collect::<Vec<_>>();
    let output = Command::new("git")
        .args(&args)
        .current_dir(cwd)
        .output()
        .map_err(|error| ShoreError::Message(format!("run git {:?}: {error}", args)))?;

    let status_code = output.status.code();
    if !status_code.is_some_and(|code| allowed_statuses.contains(&code)) {
        return Err(ShoreError::GitCommand {
            command: format!("{args:?}"),
            status: output.status.to_string(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    Ok(GitOutput {
        stdout: output.stdout,
    })
}
