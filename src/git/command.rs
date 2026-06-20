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

/// Three-valued ancestry from `merge-base --is-ancestor`, which signals only via
/// exit code with empty stdout: 0 ancestor, 1 not, 128 a missing/bad object. A
/// gc'd or absent object is a value here, never an error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Ancestry {
    Ancestor,
    NotAncestor,
    MissingObject,
}

/// One ref tip from `for-each-ref`: the full ref name and the OID it points at.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RefEntry {
    pub name: String,
    pub oid: String,
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

/// Three-valued reachability: is `ancestor_oid` an ancestor of `descendant_oid`?
/// `merge-base --is-ancestor` reports only via exit code with empty stdout, and a
/// missing/bad object (exit 128) is returned as [`Ancestry::MissingObject`]
/// rather than an error so liveness can keep folding.
pub(crate) fn git_is_ancestor(
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

/// Ref tips matching `patterns` (e.g. `&["refs/heads/*"]`), as `(oid, full ref)`
/// pairs. Empty `patterns` lists every ref.
pub(crate) fn git_for_each_ref(repo: &Path, patterns: &[&str]) -> Result<Vec<RefEntry>> {
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

/// Whether `oid` names an object present in the repository (`cat-file -e`).
pub(crate) fn git_object_exists(repo: &Path, oid: &str) -> Result<bool> {
    let (code, _) = run_git_status(repo, ["cat-file", "-e", oid], &[0, 1])?;
    Ok(code == 0)
}

/// The canonical full ref of HEAD (e.g. `refs/heads/feat/x`), or `None` when HEAD
/// is detached. The full ref — never the short name — is the canonical stored
/// `ref_name` spelling for association identity.
pub(crate) fn git_head_ref(repo: &Path) -> Result<Option<String>> {
    let (code, stdout) = run_git_status(repo, ["symbolic-ref", "--quiet", "HEAD"], &[0, 1])?;
    if code != 0 {
        return Ok(None);
    }
    let trimmed = trim_git_stdout(&stdout);
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(git_field_string(trimmed, "HEAD symbolic ref")?))
}

pub fn git_head_oid(repo: &Path) -> Result<String> {
    let output = run_git(repo, ["rev-parse", "HEAD"])?;
    git_stdout_string(repo, &output.stdout, "HEAD oid")
}

pub fn git_head_tree_oid(repo: &Path) -> Result<String> {
    let output = run_git(repo, ["rev-parse", "HEAD^{tree}"])?;
    git_stdout_string(repo, &output.stdout, "HEAD tree oid")
}

/// Resolve `rev` to a full commit OID, peeling annotated tags.
///
/// Rejects revs that do not exist or do not peel to a commit (blobs, trees)
/// with an error that names the rev, so CLI flags can surface it verbatim.
/// Resolution runs in the workflow (not the CLI) so library callers get the
/// same honest errors. `--end-of-options` keeps a rev that looks like a flag
/// (user input) from being parsed as an option.
pub(crate) fn git_rev_parse_commit_oid(repo: &Path, rev: &str) -> Result<String> {
    git_rev_parse_peeled(repo, rev, "commit", "commit oid")
}

/// Resolve a commit OID to its tree OID. Callers pass an already-resolved
/// commit OID (from [`git_rev_parse_commit_oid`]), never a raw user rev.
pub(crate) fn git_commit_tree_oid(repo: &Path, commit_oid: &str) -> Result<String> {
    git_rev_parse_peeled(repo, commit_oid, "tree", "commit tree oid")
}

/// List the full commit OIDs reachable in a `<a>..<b>` revision range via
/// `git rev-list`.
///
/// Returns the commits the range selects, in `rev-list` order (newest first); an
/// empty range yields an empty vec, not an error. The argument must denote a
/// range (contain `..`): a bare rev like `HEAD` would make `git rev-list` list
/// the whole reachable history, far broader than the `<a>..<b>` contract, so it
/// is refused. `--end-of-options` keeps a range expression that looks like a flag
/// (user input) from being parsed as an option. An unresolvable range surfaces an
/// honest, range-naming error so a CLI flag can echo it verbatim.
pub(crate) fn git_rev_list_range(repo: &Path, range: &str) -> Result<Vec<String>> {
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

/// Resolve `rev` peeled to `peel` (e.g. `commit`, `tree`) via
/// `git rev-parse --verify --end-of-options <rev>^{<peel>}`.
///
/// Substitutes an honest, rev-naming error for git's noisy stderr on failure:
/// one message covers both unknown and non-`peel` objects ("cannot resolve
/// '<rev>' to a <peel>").
fn git_rev_parse_peeled(repo: &Path, rev: &str, peel: &str, description: &str) -> Result<String> {
    let output = run_git(
        repo,
        [
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{rev}^{{{peel}}}"),
        ],
    )
    .map_err(|_| {
        ShoreError::Message(format!(
            "cannot resolve '{rev}' to a {peel} in this repository"
        ))
    })?;
    git_stdout_string(repo, &output.stdout, description)
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
    fn git_common_dir_is_shared_across_worktrees() {
        let fixture = LinkedWorktreeFixture::new();

        let main_common_dir = git_common_dir(fixture.main.path()).unwrap();
        let linked_common_dir = git_common_dir(&fixture.linked_path).unwrap();
        assert_eq!(
            canonicalize(&main_common_dir),
            canonicalize(&linked_common_dir)
        );

        let worktrees = git_worktree_list(fixture.main.path()).unwrap();
        let worktree_paths = worktrees
            .iter()
            .map(|worktree| canonicalize(&worktree.path))
            .collect::<Vec<_>>();
        assert!(worktree_paths.contains(&canonicalize(fixture.main.path())));
        assert!(worktree_paths.contains(&canonicalize(&fixture.linked_path)));
    }

    #[test]
    fn rev_parse_commit_oid_resolves_branches_relative_revs_and_annotated_tags() {
        let repo = TwoCommitRepo::new();

        let first_via_helper = git_rev_parse_commit_oid(repo.path(), "HEAD~1").unwrap();
        let first_expected = rev_parse(repo.path(), "HEAD~1");
        assert_eq!(first_via_helper, first_expected);

        let first_via_tag = git_rev_parse_commit_oid(repo.path(), "v1").unwrap();
        assert_eq!(
            first_via_tag, first_expected,
            "annotated tag must peel to its commit"
        );

        // Full-width oid (not abbreviated); width depends on object format.
        assert_eq!(first_via_helper, rev_parse(repo.path(), "HEAD~1"));
        assert!(!first_via_helper.is_empty());
    }

    #[test]
    fn rev_parse_commit_oid_rejects_unknown_rev_with_honest_error() {
        let repo = TwoCommitRepo::new();

        let error = git_rev_parse_commit_oid(repo.path(), "no-such-rev").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("no-such-rev"), "message: {message}");
        assert!(message.contains("commit"), "message: {message}");
    }

    #[test]
    fn rev_parse_commit_oid_rejects_non_commit_object() {
        let repo = TwoCommitRepo::new();

        let error = git_rev_parse_commit_oid(repo.path(), "HEAD:file.txt").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("HEAD:file.txt"), "message: {message}");
    }

    #[test]
    fn commit_tree_oid_resolves_tree_for_commit() {
        let repo = TwoCommitRepo::new();
        let head_oid = git_head_oid(repo.path()).unwrap();

        let tree_via_commit = git_commit_tree_oid(repo.path(), &head_oid).unwrap();
        let tree_via_head = git_head_tree_oid(repo.path()).unwrap();

        assert_eq!(tree_via_commit, tree_via_head);
        assert_ne!(tree_via_commit, head_oid);
    }

    fn rev_parse(repo: &Path, rev: &str) -> String {
        let output = run_git(repo, ["rev-parse", rev]).unwrap();
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    #[test]
    fn rev_list_range_lists_commits_in_the_range() {
        let repo = TwoCommitRepo::new();
        let head = rev_parse(repo.path(), "HEAD");
        let base = rev_parse(repo.path(), "HEAD~1");

        // `base..HEAD` excludes base, includes HEAD.
        let range = git_rev_list_range(repo.path(), &format!("{base}..{head}")).unwrap();
        assert_eq!(range, vec![head.clone()]);

        // An empty range (nothing reachable from base that is not reachable from
        // HEAD's first parent) yields an empty list, not an error.
        let empty = git_rev_list_range(repo.path(), &format!("{head}..{base}")).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn rev_list_range_rejects_an_unresolvable_range_with_honest_error() {
        let repo = TwoCommitRepo::new();

        let error = git_rev_list_range(repo.path(), "no-such-rev..HEAD").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("no-such-rev..HEAD"), "message: {message}");
    }

    #[test]
    fn rev_list_range_rejects_a_bare_rev_that_is_not_a_range() {
        let repo = TwoCommitRepo::new();

        // A bare rev like `HEAD` is not a range: `git rev-list HEAD` would list the
        // whole reachable history, far broader than the `<a>..<b>` contract.
        let error = git_rev_list_range(repo.path(), "HEAD").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("HEAD"), "message: {message}");
        assert!(
            message.contains(".."),
            "message names the expected range form: {message}"
        );
    }

    #[test]
    fn is_ancestor_is_three_valued() {
        let repo = TwoCommitRepo::new();
        let base = rev_parse(repo.path(), "HEAD~1");
        let tip = rev_parse(repo.path(), "HEAD");

        assert_eq!(
            git_is_ancestor(repo.path(), &base, &tip).unwrap(),
            Ancestry::Ancestor
        );
        assert_eq!(
            git_is_ancestor(repo.path(), &tip, &base).unwrap(),
            Ancestry::NotAncestor
        );
        let absent = "0".repeat(tip.len());
        assert_eq!(
            git_is_ancestor(repo.path(), &absent, &tip).unwrap(),
            Ancestry::MissingObject
        );
    }

    #[test]
    fn for_each_ref_lists_tips_including_nested_branches() {
        let repo = TwoCommitRepo::new();
        git(repo.path(), ["branch", "feat/x"]);

        // The `refs/heads/` prefix matches nested branch names; `refs/heads/*`
        // would not, because for-each-ref globs with WM_PATHNAME so `*` stops at
        // a slash.
        let entries = git_for_each_ref(repo.path(), &["refs/heads/"]).unwrap();
        let tip = rev_parse(repo.path(), "HEAD");

        assert!(
            entries
                .iter()
                .any(|entry| entry.name == "refs/heads/feat/x"),
            "for-each-ref must list the nested branch: {entries:?}"
        );
        assert!(entries.iter().any(|entry| entry.oid == tip));
    }

    #[test]
    fn object_exists_and_head_ref() {
        let repo = TwoCommitRepo::new();
        let head_oid = rev_parse(repo.path(), "HEAD");

        assert!(git_object_exists(repo.path(), &head_oid).unwrap());
        assert!(!git_object_exists(repo.path(), &"0".repeat(head_oid.len())).unwrap());

        let head_ref = git_head_ref(repo.path()).unwrap();
        assert!(
            head_ref
                .as_deref()
                .is_some_and(|name| name.starts_with("refs/heads/")),
            "attached HEAD must resolve to a full ref, got {head_ref:?}"
        );

        git(repo.path(), ["checkout", "--detach"]);
        assert_eq!(git_head_ref(repo.path()).unwrap(), None);
    }

    struct TwoCommitRepo {
        root: TempDir,
    }

    impl TwoCommitRepo {
        fn new() -> Self {
            let root = TempDir::new().expect("create temp git repository directory");
            let repo = Self { root };

            git(repo.path(), ["init"]);
            git(repo.path(), ["config", "user.name", "Shore Tests"]);
            git(
                repo.path(),
                ["config", "user.email", "shore-tests@example.com"],
            );
            git(repo.path(), ["config", "commit.gpgsign", "false"]);

            fs::write(repo.path().join("file.txt"), "one\n").expect("write first file");
            git(repo.path(), ["add", "--all"]);
            git(repo.path(), ["commit", "-m", "first"]);
            git(repo.path(), ["tag", "-a", "v1", "-m", "v1", "HEAD"]);

            fs::write(repo.path().join("file.txt"), "two\n").expect("write second file");
            git(repo.path(), ["add", "--all"]);
            git(repo.path(), ["commit", "-m", "second"]);

            repo
        }

        fn path(&self) -> &Path {
            self.root.path()
        }
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
