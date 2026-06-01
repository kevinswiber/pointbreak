use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Result, ShoreError};

#[derive(Debug)]
pub(crate) struct GitOutput {
    pub stdout: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitWorktree {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub bare: bool,
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
    git_stdout_path(repo, &output.stdout, "worktree root")
}

pub(crate) fn git_common_dir(repo: &Path) -> Result<PathBuf> {
    let output = match run_git(
        repo,
        ["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ) {
        Ok(output) => output,
        Err(error) if git_path_format_is_unsupported(&error) => {
            return git_common_dir_without_path_format(repo);
        }
        Err(error) => return Err(error),
    };
    git_stdout_path(repo, &output.stdout, "git common-dir")
}

fn git_common_dir_without_path_format(repo: &Path) -> Result<PathBuf> {
    let output = run_git(repo, ["rev-parse", "--git-common-dir"])?;
    let path = git_stdout_path(repo, &output.stdout, "git common-dir")?;
    absolute_git_cwd_path(repo, path)
}

fn git_path_format_is_unsupported(error: &ShoreError) -> bool {
    let ShoreError::GitCommand { stderr, .. } = error else {
        return false;
    };

    stderr.contains("--path-format")
        || stderr.contains("unknown option")
        || stderr.contains("unknown switch")
}

fn absolute_git_cwd_path(repo: &Path, path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path);
    }

    let cwd = if repo.is_absolute() {
        repo.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| ShoreError::Message(format!("resolve current directory: {error}")))?
            .join(repo)
    };
    let candidate = cwd.join(path);
    candidate.canonicalize().map_err(|error| {
        ShoreError::Message(format!(
            "canonicalize git common-dir {}: {error}",
            candidate.display()
        ))
    })
}

pub(crate) fn git_absolute_git_dir(repo: &Path) -> Result<PathBuf> {
    let output = run_git(repo, ["rev-parse", "--absolute-git-dir"])?;
    git_stdout_path(repo, &output.stdout, "absolute git-dir")
}

pub fn git_info_exclude_path(repo: &Path) -> Result<PathBuf> {
    let output = run_git(repo, ["rev-parse", "--git-path", "info/exclude"])?;
    let relative = git_stdout_path(repo, &output.stdout, "info/exclude path")?;

    // `git rev-parse --git-path` resolves against the working directory we ran
    // it from (the worktree root). Joining keeps relative results anchored to
    // `repo` while preserving absolute results (linked worktrees share the
    // common `info/exclude`), since `Path::join` discards the base for an
    // absolute child.
    Ok(repo.join(relative))
}

/// Reports whether `pathspec` is ignored by the standard Git exclude sources
/// (the worktree `.gitignore`, the global excludes file, and the repository
/// `.git/info/exclude`). This mirrors the `--exclude-standard` rules used when
/// Shoreline discovers untracked files.
pub fn git_path_is_ignored(repo: &Path, pathspec: &str) -> Result<bool> {
    // `git check-ignore` prints matching paths to stdout and exits 1 (no error)
    // when nothing matches, so a non-empty stdout is the "ignored" signal.
    let output = run_git_allowing_statuses(repo, ["check-ignore", pathspec], &[0, 1])?;
    Ok(!output.stdout.is_empty())
}

pub fn git_head_oid(repo: &Path) -> Result<String> {
    let output = run_git(repo, ["rev-parse", "HEAD"])?;
    git_stdout_string(repo, &output.stdout, "HEAD oid")
}

pub(crate) fn git_object_format(repo: &Path) -> Result<String> {
    let output = run_git(repo, ["rev-parse", "--show-object-format"])?;
    git_stdout_string(repo, &output.stdout, "object format")
}

pub fn git_head_tree_oid(repo: &Path) -> Result<String> {
    let output = run_git(repo, ["rev-parse", "HEAD^{tree}"])?;
    git_stdout_string(repo, &output.stdout, "HEAD tree oid")
}

pub(crate) fn git_worktree_list(repo: &Path) -> Result<Vec<GitWorktree>> {
    let output = run_git(repo, ["worktree", "list", "--porcelain", "-z"])?;
    parse_git_worktree_list_z(&output.stdout)
}

fn parse_git_worktree_list_z(output: &[u8]) -> Result<Vec<GitWorktree>> {
    let mut worktrees = Vec::new();
    let mut current = None;

    for field in output.split(|byte| *byte == b'\0') {
        if field.is_empty() {
            if let Some(worktree) = current.take() {
                worktrees.push(worktree);
            }
            continue;
        }

        if let Some(path) = field.strip_prefix(b"worktree ") {
            if let Some(worktree) = current.replace(GitWorktree {
                path: git_path_from_bytes(path)?,
                head: None,
                branch: None,
                detached: false,
                bare: false,
            }) {
                worktrees.push(worktree);
            }
            continue;
        }

        let Some(worktree) = current.as_mut() else {
            return Err(ShoreError::Message(
                "git worktree list returned field before worktree path".to_owned(),
            ));
        };

        if let Some(head) = field.strip_prefix(b"HEAD ") {
            worktree.head = Some(git_field_string(head, "worktree HEAD")?);
        } else if let Some(branch) = field.strip_prefix(b"branch ") {
            worktree.branch = Some(git_field_string(branch, "worktree branch")?);
        } else if field == b"detached" {
            worktree.detached = true;
        } else if field == b"bare" {
            worktree.bare = true;
        }
    }

    if let Some(worktree) = current {
        worktrees.push(worktree);
    }

    Ok(worktrees)
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

fn git_stdout_path(repo: &Path, stdout: &[u8], description: &str) -> Result<PathBuf> {
    let trimmed = trim_git_stdout(stdout);
    if trimmed.is_empty() {
        return Err(ShoreError::Message(format!(
            "git rev-parse returned empty {description} for {}",
            repo.display()
        )));
    }

    git_path_from_bytes(trimmed)
}

fn git_stdout_string(repo: &Path, stdout: &[u8], description: &str) -> Result<String> {
    let trimmed = trim_git_stdout(stdout);
    if trimmed.is_empty() {
        return Err(ShoreError::Message(format!(
            "git rev-parse returned empty {description} for {}",
            repo.display()
        )));
    }

    git_field_string(trimmed, description)
}

fn trim_git_stdout(stdout: &[u8]) -> &[u8] {
    let mut end = stdout.len();
    while end > 0 && matches!(stdout[end - 1], b'\r' | b'\n') {
        end -= 1;
    }

    &stdout[..end]
}

fn git_field_string(bytes: &[u8], description: &str) -> Result<String> {
    String::from_utf8(bytes.to_vec()).map_err(|error| {
        ShoreError::Message(format!("git returned non-utf8 {description}: {error}"))
    })
}

#[cfg(unix)]
fn git_path_from_bytes(bytes: &[u8]) -> Result<PathBuf> {
    use std::os::unix::ffi::OsStringExt;

    Ok(std::ffi::OsString::from_vec(bytes.to_vec()).into())
}

#[cfg(not(unix))]
fn git_path_from_bytes(bytes: &[u8]) -> Result<PathBuf> {
    let path = String::from_utf8(bytes.to_vec()).map_err(|error| {
        ShoreError::Message(format!("git returned non-utf8 path bytes: {error}"))
    })?;
    Ok(PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn git_identity_helpers_distinguish_common_and_worktree_git_dirs() {
        let fixture = LinkedWorktreeFixture::new();

        let main_common_dir = git_common_dir(fixture.main.path()).unwrap();
        let linked_common_dir = git_common_dir(&fixture.linked_path).unwrap();
        assert_eq!(
            canonicalize(&main_common_dir),
            canonicalize(&linked_common_dir)
        );

        let main_git_dir = git_absolute_git_dir(fixture.main.path()).unwrap();
        let linked_git_dir = git_absolute_git_dir(&fixture.linked_path).unwrap();
        assert_ne!(canonicalize(&main_git_dir), canonicalize(&linked_git_dir));

        let object_format = git_object_format(fixture.main.path()).unwrap();
        assert!(
            matches!(object_format.as_str(), "sha1" | "sha256"),
            "unexpected object format: {object_format}"
        );

        let worktrees = git_worktree_list(fixture.main.path()).unwrap();
        let worktree_paths = worktrees
            .iter()
            .map(|worktree| canonicalize(&worktree.path))
            .collect::<Vec<_>>();
        assert!(worktree_paths.contains(&canonicalize(fixture.main.path())));
        assert!(worktree_paths.contains(&canonicalize(&fixture.linked_path)));
    }

    #[cfg(unix)]
    #[test]
    fn worktree_list_parser_preserves_non_utf8_paths() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let raw_path = b"/tmp/shoreline-\xff-worktree";
        let output = [
            b"worktree ".as_slice(),
            raw_path.as_slice(),
            b"\0HEAD 0123456789012345678901234567890123456789\0branch refs/heads/main\0\0",
        ]
        .concat();

        let worktrees = parse_git_worktree_list_z(&output).unwrap();

        assert_eq!(worktrees.len(), 1);
        assert_eq!(
            worktrees[0].path.as_os_str().as_bytes(),
            OsString::from_vec(raw_path.to_vec()).as_os_str().as_bytes()
        );
    }

    struct LinkedWorktreeFixture {
        main: TempDir,
        _linked_parent: TempDir,
        linked_path: PathBuf,
    }

    impl LinkedWorktreeFixture {
        fn new() -> Self {
            let main = TempDir::new().expect("create main repository directory");
            git(main.path(), ["init"]);
            git(main.path(), ["config", "user.name", "Shore Tests"]);
            git(
                main.path(),
                ["config", "user.email", "shore-tests@example.com"],
            );
            git(main.path(), ["config", "commit.gpgsign", "false"]);
            fs::write(main.path().join("README.md"), "base\n").expect("write base file");
            git(main.path(), ["add", "--all"]);
            git(main.path(), ["commit", "-m", "base"]);

            let linked_parent = TempDir::new().expect("create linked worktree parent");
            let linked_path = linked_parent.path().join("linked");
            git_os(
                main.path(),
                [
                    OsString::from("worktree"),
                    OsString::from("add"),
                    OsString::from("-b"),
                    OsString::from("linked"),
                    linked_path.as_os_str().to_owned(),
                ],
            );

            Self {
                main,
                _linked_parent: linked_parent,
                linked_path,
            }
        }
    }

    fn git<I, S>(cwd: &Path, args: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        run_git(cwd, args).unwrap();
    }

    fn git_os<I>(cwd: &Path, args: I)
    where
        I: IntoIterator<Item = OsString>,
    {
        run_git(cwd, args).unwrap();
    }

    fn canonicalize(path: &Path) -> PathBuf {
        path.canonicalize()
            .unwrap_or_else(|error| panic!("canonicalize {}: {error}", path.display()))
    }
}
