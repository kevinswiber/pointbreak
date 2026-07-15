use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Output};

#[allow(dead_code)]
pub mod event_signature_fixtures;
#[allow(dead_code)]
pub mod git_repo;
#[allow(dead_code)]
pub mod inspect;
#[allow(dead_code)]
pub mod snapshots;

#[allow(dead_code)]
pub fn shore<I, S>(args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_shore"))
        .args(args)
        .env_remove("SHORE_LOG")
        .env_remove("RUST_LOG")
        // Isolate byte-asserting tests from a developer's ambient output-lane
        // selector; tests that exercise SHORE_FORMAT set it explicitly via shore_env.
        .env_remove("SHORE_FORMAT")
        // Isolate color-asserting tests from an ambient NO_COLOR / CLICOLOR_FORCE;
        // color tests select the lane explicitly with `--color`.
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR_FORCE")
        // Isolate theme-asserting tests from a developer's ambient theme
        // selection; theme tests set these explicitly via shore_env.
        .env_remove("SHORE_THEME")
        .env_remove("BAT_THEME")
        .output()
        .expect("run shore binary")
}

/// Run `shore` with extra environment variables — e.g. `SHORE_ACTOR_ID` to
/// attribute a write to a specific actor.
#[allow(dead_code)]
pub fn shore_env<I, S>(args: I, env: &[(&str, &str)]) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(env!("CARGO_BIN_EXE_shore"));
    command
        .args(args)
        .env_remove("SHORE_LOG")
        .env_remove("RUST_LOG")
        // Clear ambient selectors first; a caller that passes SHORE_FORMAT or
        // a theme variable in `env` re-sets it below and still wins.
        .env_remove("SHORE_FORMAT")
        .env_remove("SHORE_THEME")
        .env_remove("BAT_THEME");
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().expect("run shore binary")
}

#[allow(dead_code)]
pub fn dump_repo() -> git_repo::GitRepo {
    let repo = git_repo::GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo
}

/// Capture two worktree states where the second supersedes the first, returning
/// the repository and both full revision ids for selector-behavior tests.
#[allow(dead_code)]
pub fn superseded_dump_repo() -> (git_repo::GitRepo, String, String) {
    let repo = dump_repo();
    let repo_arg = repo.path().to_str().expect("temporary path is utf-8");
    let first: serde_json::Value =
        serde_json::from_slice(&shore(["capture", "--repo", repo_arg]).stdout)
            .expect("first capture emits JSON");
    let first_id = first["revision"]["id"]
        .as_str()
        .expect("first revision id")
        .to_owned();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    let second: serde_json::Value = serde_json::from_slice(
        &shore(["capture", "--repo", repo_arg, "--supersedes", &first_id]).stdout,
    )
    .expect("second capture emits JSON");
    let second_id = second["revision"]["id"]
        .as_str()
        .expect("second revision id")
        .to_owned();
    (repo, first_id, second_id)
}

/// A repository with two commits (clean worktree), so `--base HEAD~1` captures
/// the committed range. Shared by the commit-range read-surface suites.
#[allow(dead_code)]
pub fn committed_repo() -> git_repo::GitRepo {
    let repo = git_repo::GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.commit_all("change");
    repo
}

/// The shared common-dir store a clone resolves by default
/// (`<git-common-dir>/shore`, i.e. `.git/shore`). Every non-ephemeral worktree of
/// a clone — main and linked — reads and writes here, with no `store link`. Use
/// this for store-path assertions after a `shore` write instead of the raw
/// worktree-local `.shore/data`.
#[allow(dead_code)]
pub fn common_dir_store(repo_root: &Path) -> std::path::PathBuf {
    let output = Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(repo_root)
        .output()
        .expect("run git rev-parse --git-common-dir");
    assert!(
        output.status.success(),
        "git rev-parse --git-common-dir failed in {}",
        repo_root.display()
    );
    let common_dir = String::from_utf8(output.stdout)
        .expect("git-common-dir is utf-8")
        .trim()
        .to_owned();
    Path::new(&common_dir).join("shore")
}

#[track_caller]
#[allow(dead_code)]
pub fn assert_existing_paths_eq(actual: &Path, expected: &Path) {
    assert_eq!(
        actual.canonicalize().expect("canonicalize actual path"),
        expected.canonicalize().expect("canonicalize expected path")
    );
}
