//! The `git`-subprocess backend: the single production spawn funnel, the
//! process-lifetime discovery memo, the shared output/path helpers, and the
//! `GitBackend` implementation that shells out to the `git` binary.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

use crate::error::{Result, ShoreError};
use crate::git::backend::GitBackend;
use crate::git::command::{Ancestry, GitReflogEntry, GitWorktree, RefEntry};

/// The `git`-subprocess backend: every routable operation shells out to the
/// `git` binary through the spawn funnel in this module. Behavior is identical
/// to the historical free-function implementations, which moved here verbatim.
pub(crate) struct SubprocessBackend;

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

/// Invariant repository facts that Git resolves from disk but that never change
/// for a live repository within a single process: the worktree root, the common
/// Git directory, and the path to `info/exclude`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum RepoFact {
    WorktreeRoot,
    CommonDir,
    #[cfg(test)]
    InfoExcludePath,
}

/// Memoizes [`RepoFact`] lookups keyed by the working directory passed to Git.
///
/// Pointbreak re-derives these facts many times across one capture/ingest — store
/// resolution alone resolves the worktree root ~10 times for a single repository
/// — and each call previously spawned a fresh `git rev-parse`. Process spawning
/// is the dominant cost in the `sys`-bound test suite and in every CLI
/// invocation, so collapsing the repeats to one spawn per repository is a real
/// latency win for both.
///
/// The memo is sound because these three facts are immutable for a given
/// repository as long as it exists: Pointbreak never relocates a repository
/// mid-process, and the `info/exclude` *path* (not its mutable contents) is fixed
/// by the layout. Only successful lookups are cached, so a transient failure is
/// never memoized. Keys are canonicalized absolute paths (see [`repo_fact_key`]),
/// so different spellings of one repository — a relative `.`, a symlinked
/// temporary directory — collapse to a single entry and can never alias two
/// distinct repositories. Concurrent first callers may each resolve once: the
/// lock is released across the (subprocess) lookup rather than single-flighting
/// it, which at worst duplicates a spawn and never returns a wrong value.
fn repo_fact_cache() -> &'static Mutex<HashMap<(PathBuf, RepoFact), PathBuf>> {
    static CACHE: OnceLock<Mutex<HashMap<(PathBuf, RepoFact), PathBuf>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Canonicalizes `repo` into a cache key so every spelling of one repository — a
/// relative `.`, a symlinked temporary directory — maps to the same entry and two
/// distinct repositories never collide. Falls back to the raw path when
/// canonicalization fails (e.g. the directory does not exist yet), in which case
/// the lookup fails and nothing is cached.
fn repo_fact_key(repo: &Path, fact: RepoFact) -> (PathBuf, RepoFact) {
    let path = std::fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());
    (path, fact)
}

fn cached_repo_fact(
    repo: &Path,
    fact: RepoFact,
    resolve: impl FnOnce() -> Result<PathBuf>,
) -> Result<PathBuf> {
    let key = repo_fact_key(repo, fact);
    {
        let cache = repo_fact_cache()
            .lock()
            .expect("repo fact cache mutex is not poisoned");
        if let Some(hit) = cache.get(&key) {
            return Ok(hit.clone());
        }
    }

    // Resolve outside the lock: the guard above is dropped with its block, so the
    // (process-spawning) lookup never runs while holding the mutex.
    let value = resolve()?;
    repo_fact_cache()
        .lock()
        .expect("repo fact cache mutex is not poisoned")
        .insert(key, value.clone());
    Ok(value)
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

#[cfg(test)]
pub(crate) fn git_info_exclude_path(repo: &Path) -> Result<PathBuf> {
    cached_repo_fact(repo, RepoFact::InfoExcludePath, || {
        let output = run_git(repo, ["rev-parse", "--git-path", "info/exclude"])?;
        let relative = git_stdout_path(repo, &output.stdout, "info/exclude path")?;

        // `git rev-parse --git-path` resolves against the working directory we ran
        // it from (the worktree root). Joining keeps relative results anchored to
        // `repo` while preserving absolute results (linked worktrees share the
        // common `info/exclude`), since `Path::join` discards the base for an
        // absolute child.
        Ok(repo.join(relative))
    })
}

fn parse_commit_subject_batch(
    commit_oids: &BTreeSet<String>,
    output: &[u8],
) -> Result<BTreeMap<String, String>> {
    let mut subjects = BTreeMap::new();
    let mut cursor = 0;

    for requested_oid in commit_oids {
        let header_end = output[cursor..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|offset| cursor + offset)
            .ok_or_else(|| ShoreError::Message("truncated git cat-file batch header".to_owned()))?;
        let header = std::str::from_utf8(&output[cursor..header_end]).map_err(|error| {
            ShoreError::Message(format!("git returned non-utf8 cat-file header: {error}"))
        })?;
        cursor = header_end + 1;

        if header.ends_with(" missing") || header.ends_with(" ambiguous") {
            continue;
        }

        let mut fields = header.rsplitn(3, ' ');
        let size = fields
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or_else(|| ShoreError::Message(format!("invalid git cat-file header: {header}")))?;
        let object_type = fields
            .next()
            .ok_or_else(|| ShoreError::Message(format!("invalid git cat-file header: {header}")))?;
        if cursor + size > output.len() {
            return Err(ShoreError::Message(
                "truncated git cat-file batch object".to_owned(),
            ));
        }
        let object = &output[cursor..cursor + size];
        cursor += size;
        if output.get(cursor) == Some(&b'\n') {
            cursor += 1;
        }

        if object_type != "commit" {
            continue;
        }
        let Some(message_start) = object
            .windows(2)
            .position(|window| window == b"\n\n")
            .map(|position| position + 2)
        else {
            continue;
        };
        let message = &object[message_start..];
        let first_line_end = message
            .iter()
            .position(|byte| *byte == b'\n')
            .unwrap_or(message.len());
        let Ok(subject) = std::str::from_utf8(&message[..first_line_end]) else {
            continue;
        };
        let subject = subject.trim();
        if !subject.is_empty() {
            subjects.insert(requested_oid.clone(), subject.to_owned());
        }
    }

    Ok(subjects)
}

/// Whether `reference` resolves to a commit object (`rev-parse --verify --quiet
/// <reference>^{commit}`), the two-valued check the default-branch resolution
/// needs: exit 0 resolves, exit 1 does not (missing ref, or a non-commit).
fn git_ref_resolves_to_commit(repo: &Path, reference: &str) -> Result<bool> {
    let (code, _) = run_git_status(
        repo,
        [
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{reference}^{{commit}}"),
        ],
        &[0, 1],
    )?;
    Ok(code == 0)
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
    let (_, stdout) = run_git_status(cwd, args, allowed_statuses)?;
    Ok(GitOutput { stdout })
}

/// Runs git and surfaces both the exit code and stdout, erroring only when the
/// code is outside `allowed_statuses`. Unlike [`run_git_allowing_statuses`],
/// this keeps the exit code, which is the only signal some plumbing commands
/// emit (`merge-base --is-ancestor`, `cat-file -e`, `symbolic-ref --quiet`).
pub(crate) fn run_git_status<I, S>(
    cwd: &Path,
    args: I,
    allowed_statuses: &[i32],
) -> Result<(i32, Vec<u8>)>
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

    Ok((
        status_code.expect("an allowed status implies a concrete exit code"),
        output.stdout,
    ))
}

pub(crate) fn run_git_with_stdin<I, S>(
    cwd: &Path,
    args: I,
    stdin: &[u8],
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
    let mut child = Command::new("git")
        .args(&args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| ShoreError::Message(format!("run git {:?}: {error}", args)))?;

    child
        .stdin
        .as_mut()
        .expect("git stdin is piped")
        .write_all(stdin)
        .map_err(|error| ShoreError::Message(format!("write git {:?} stdin: {error}", args)))?;

    let output = child
        .wait_with_output()
        .map_err(|error| ShoreError::Message(format!("wait for git {:?}: {error}", args)))?;
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

pub(crate) fn git_stdout_string(repo: &Path, stdout: &[u8], description: &str) -> Result<String> {
    let trimmed = trim_git_stdout(stdout);
    if trimmed.is_empty() {
        return Err(ShoreError::Message(format!(
            "git rev-parse returned empty {description} for {}",
            repo.display()
        )));
    }

    git_field_string(trimmed, description)
}

pub(crate) fn trim_git_stdout(stdout: &[u8]) -> &[u8] {
    let mut end = stdout.len();
    while end > 0 && matches!(stdout[end - 1], b'\r' | b'\n') {
        end -= 1;
    }

    &stdout[..end]
}

pub(crate) fn git_field_string(bytes: &[u8], description: &str) -> Result<String> {
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

impl GitBackend for SubprocessBackend {
    fn worktree_root(&self, repo: &Path) -> Result<PathBuf> {
        cached_repo_fact(repo, RepoFact::WorktreeRoot, || {
            let output = run_git(repo, ["rev-parse", "--show-toplevel"])?;
            git_stdout_path(repo, &output.stdout, "worktree root")
        })
    }

    fn common_dir(&self, repo: &Path) -> Result<PathBuf> {
        cached_repo_fact(repo, RepoFact::CommonDir, || {
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
        })
    }

    fn is_ancestor(
        &self,
        repo: &Path,
        ancestor_oid: &str,
        descendant_oid: &str,
    ) -> Result<Ancestry> {
        let (code, _) = run_git_status(
            repo,
            ["merge-base", "--is-ancestor", ancestor_oid, descendant_oid],
            &[0, 1, 128],
        )?;
        Ok(match code {
            0 => Ancestry::Ancestor,
            1 => Ancestry::NotAncestor,
            _ => Ancestry::MissingObject,
        })
    }

    fn independent_commits(&self, repo: &Path, oids: &[String]) -> Result<Vec<String>> {
        if oids.len() <= 1 {
            return Ok(oids.to_vec());
        }
        let mut args = vec!["merge-base".to_owned(), "--independent".to_owned()];
        args.extend(oids.iter().cloned());
        let output = run_git(repo, args)?;
        let text = git_field_string(&output.stdout, "merge-base --independent output")?;
        Ok(text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect())
    }

    fn commit_changed_paths(&self, repo: &Path, commit_oid: &str) -> Result<Vec<String>> {
        let output = run_git(
            repo,
            [
                "diff-tree",
                "--no-commit-id",
                "--name-only",
                "-z",
                "-r",
                "--root",
                "-m",
                commit_oid,
            ],
        )?;
        Ok(output
            .stdout
            .split(|byte| *byte == b'\0')
            .filter(|field| !field.is_empty())
            .filter_map(|field| std::str::from_utf8(field).ok())
            .map(str::to_owned)
            .collect())
    }

    fn commit_subjects(
        &self,
        repo: &Path,
        commit_oids: &BTreeSet<String>,
    ) -> Result<BTreeMap<String, String>> {
        if commit_oids.is_empty() {
            return Ok(BTreeMap::new());
        }

        let mut input = commit_oids.iter().cloned().collect::<Vec<_>>().join("\n");
        input.push('\n');
        let output = run_git_with_stdin(repo, ["cat-file", "--batch"], input.as_bytes(), &[0])?;
        parse_commit_subject_batch(commit_oids, &output.stdout)
    }

    fn for_each_ref(&self, repo: &Path, patterns: &[&str]) -> Result<Vec<RefEntry>> {
        let mut args = vec![
            "for-each-ref".to_owned(),
            "--format=%(objectname) %(refname)".to_owned(),
        ];
        args.extend(patterns.iter().map(|pattern| (*pattern).to_owned()));
        let output = run_git(repo, args)?;
        let text = git_field_string(&output.stdout, "for-each-ref output")?;
        Ok(text
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() {
                    return None;
                }
                let (oid, name) = line.split_once(' ')?;
                Some(RefEntry {
                    name: name.to_owned(),
                    oid: oid.to_owned(),
                })
            })
            .collect())
    }

    fn ref_state_lines(&self, repo: &Path) -> Result<String> {
        let output = run_git(
            repo,
            [
                "for-each-ref",
                "--format=%(objectname) %(refname) %(symref)",
                "refs/heads/",
                "refs/remotes/",
            ],
        )?;
        git_field_string(&output.stdout, "for-each-ref state output")
    }

    fn object_exists(&self, repo: &Path, oid: &str) -> Result<bool> {
        let (code, _) = run_git_status(repo, ["cat-file", "-e", oid], &[0, 1])?;
        Ok(code == 0)
    }

    fn default_branch_ref(&self, repo: &Path) -> Result<Option<String>> {
        let (code, stdout) = run_git_status(
            repo,
            ["symbolic-ref", "refs/remotes/origin/HEAD"],
            &[0, 1, 128],
        )?;
        if code == 0 {
            let trimmed = trim_git_stdout(&stdout);
            if !trimmed.is_empty() {
                let target = git_field_string(trimmed, "origin/HEAD target")?;
                // Only accept origin/HEAD when its target still resolves to a commit; a
                // dangling symbolic ref (the remote-tracking branch was pruned) would
                // otherwise shadow a valid local default and, downstream, make a narrow
                // integration ref unresolvable and suppress the whole liveness block.
                if git_ref_resolves_to_commit(repo, &target)? {
                    return Ok(Some(target));
                }
            }
        }

        for candidate in ["refs/heads/main", "refs/heads/master"] {
            if git_ref_resolves_to_commit(repo, candidate)? {
                return Ok(Some(candidate.to_owned()));
            }
        }

        Ok(None)
    }

    fn rev_list_range(&self, repo: &Path, range: &str) -> Result<Vec<String>> {
        if !range.contains("..") {
            return Err(ShoreError::Message(format!(
                "'{range}' is not a commit range; expected the form '<a>..<b>'"
            )));
        }
        let output = run_git(repo, ["rev-list", "--end-of-options", range]).map_err(|_| {
            ShoreError::Message(format!(
                "cannot resolve commit range '{range}' in this repository"
            ))
        })?;
        let listing = git_field_string(trim_git_stdout(&output.stdout), "rev-list output")?;
        Ok(listing
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect())
    }

    fn rev_list_reachable(&self, repo: &Path, tips: &[String]) -> Result<HashSet<String>> {
        if tips.is_empty() {
            return Ok(HashSet::new());
        }
        let mut args = vec!["rev-list".to_owned(), "--end-of-options".to_owned()];
        args.extend(tips.iter().cloned());
        let output = run_git(repo, args)?;
        let listing = git_field_string(trim_git_stdout(&output.stdout), "rev-list output")?;
        Ok(listing
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect())
    }

    fn rev_list_reflog_reachable(&self, repo: &Path) -> Result<HashSet<String>> {
        let (code, stdout) = run_git_status(repo, ["rev-list", "--reflog"], &[0, 129])?;
        if code != 0 {
            return Ok(HashSet::new());
        }
        let listing = git_field_string(trim_git_stdout(&stdout), "rev-list --reflog output")?;
        Ok(listing
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect())
    }

    fn reflog_entries(&self, repo: &Path, ref_name: &str) -> Result<Vec<GitReflogEntry>> {
        let (code, stdout) = run_git_status(
            repo,
            [
                "log",
                "-g",
                "--format=%H%x09%gs",
                "--end-of-options",
                ref_name,
                "--",
            ],
            &[0, 128],
        )?;
        if code != 0 {
            return Ok(Vec::new());
        }
        let listing = git_field_string(trim_git_stdout(&stdout), "reflog output")?;
        Ok(listing
            .lines()
            .filter_map(|line| {
                let line = line.trim_end();
                if line.is_empty() {
                    return None;
                }
                let (oid, subject) = line.split_once('\t').unwrap_or((line, ""));
                Some(GitReflogEntry {
                    new_oid: oid.to_owned(),
                    subject: subject.to_owned(),
                })
            })
            .collect())
    }

    fn worktree_list(&self, repo: &Path) -> Result<Vec<GitWorktree>> {
        let output = run_git(repo, ["worktree", "list", "--porcelain", "-z"])?;
        parse_git_worktree_list_z(&output.stdout)
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn init_repo() -> TempDir {
        let dir = TempDir::new().expect("create temp git repository directory");
        run_git(dir.path(), ["init"]).unwrap();
        run_git(dir.path(), ["config", "user.name", "Shore Tests"]).unwrap();
        run_git(
            dir.path(),
            ["config", "user.email", "shore-tests@example.com"],
        )
        .unwrap();
        run_git(dir.path(), ["config", "commit.gpgsign", "false"]).unwrap();
        std::fs::write(dir.path().join("file.txt"), "one\n").unwrap();
        run_git(dir.path(), ["add", "--all"]).unwrap();
        run_git(dir.path(), ["commit", "-m", "first"]).unwrap();
        dir
    }

    fn repo_fact_is_cached(repo: &Path, fact: RepoFact) -> bool {
        repo_fact_cache()
            .lock()
            .expect("repo fact cache mutex is not poisoned")
            .contains_key(&repo_fact_key(repo, fact))
    }

    #[test]
    fn invariant_repo_facts_are_resolved_once_and_memoized() {
        let repo = init_repo();
        let backend = SubprocessBackend;

        // A freshly created repository (unique temp dir) starts cold for every
        // fact, so the first lookup is a genuine miss that spawns Git.
        for fact in [
            RepoFact::WorktreeRoot,
            RepoFact::CommonDir,
            RepoFact::InfoExcludePath,
        ] {
            assert!(
                !repo_fact_is_cached(repo.path(), fact),
                "{fact:?} must be cold before the first lookup"
            );
        }

        let root_first = backend.worktree_root(repo.path()).unwrap();
        let common_first = backend.common_dir(repo.path()).unwrap();
        let exclude_first = git_info_exclude_path(repo.path()).unwrap();

        // After one lookup each fact is memoized, so subsequent calls are served
        // from the cache rather than spawning Git again.
        for fact in [
            RepoFact::WorktreeRoot,
            RepoFact::CommonDir,
            RepoFact::InfoExcludePath,
        ] {
            assert!(
                repo_fact_is_cached(repo.path(), fact),
                "{fact:?} must be memoized after the first lookup"
            );
        }

        // The memoized value matches the freshly resolved one — caching changes
        // cost, never the answer.
        assert_eq!(backend.worktree_root(repo.path()).unwrap(), root_first);
        assert_eq!(backend.common_dir(repo.path()).unwrap(), common_first);
        assert_eq!(git_info_exclude_path(repo.path()).unwrap(), exclude_first);
    }

    #[cfg(unix)]
    #[test]
    fn worktree_list_parser_preserves_non_utf8_paths() {
        use std::ffi::OsString;
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
}
