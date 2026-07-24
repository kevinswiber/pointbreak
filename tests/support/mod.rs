use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Output};

#[allow(dead_code)]
pub mod env;
#[allow(dead_code)]
pub mod event_signature_fixtures;
#[allow(dead_code)]
pub mod git_repo;
#[allow(dead_code)]
pub mod inspect;
#[allow(dead_code)]
pub mod snapshots;

// Runtime-resolved `pointbreak` binary and manifest dir; see `env`. Re-exported so
// `mod support;` consumers keep calling `support::{pointbreak_bin, manifest_dir}`.
pub use env::{manifest_dir, pointbreak_bin};

#[allow(dead_code)]
pub fn pointbreak<I, S>(args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(pointbreak_bin())
        .args(args)
        .env_remove("POINTBREAK_LOG")
        .env_remove("RUST_LOG")
        // Isolate byte-asserting tests from a developer's ambient output-lane
        // selector; tests that exercise POINTBREAK_FORMAT set it explicitly via pointbreak_env.
        .env_remove("POINTBREAK_FORMAT")
        // Isolate color-asserting tests from an ambient NO_COLOR / CLICOLOR_FORCE;
        // color tests select the lane explicitly with `--color`.
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR_FORCE")
        // Isolate theme-asserting tests from a developer's ambient theme
        // selection; theme tests set these explicitly via pointbreak_env.
        .env_remove("POINTBREAK_THEME")
        .env_remove("BAT_THEME")
        .output()
        .expect("run pointbreak binary")
}

/// Run `pointbreak` with extra environment variables — e.g. `POINTBREAK_ACTOR_ID` to
/// attribute a write to a specific actor.
#[allow(dead_code)]
pub fn pointbreak_env<I, S>(args: I, env: &[(&str, &str)]) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(pointbreak_bin());
    command
        .args(args)
        .env_remove("POINTBREAK_LOG")
        .env_remove("RUST_LOG")
        // Clear ambient selectors first; a caller that passes POINTBREAK_FORMAT or
        // a theme variable in `env` re-sets it below and still wins.
        .env_remove("POINTBREAK_FORMAT")
        .env_remove("POINTBREAK_THEME")
        .env_remove("BAT_THEME");
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().expect("run pointbreak binary")
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
        serde_json::from_slice(&pointbreak(["capture", "--repo", repo_arg]).stdout)
            .expect("first capture emits JSON");
    let first_id = first["revision"]["id"]
        .as_str()
        .expect("first revision id")
        .to_owned();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    let second: serde_json::Value = serde_json::from_slice(
        &pointbreak(["capture", "--repo", repo_arg, "--supersedes", &first_id]).stdout,
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
/// (`<git-common-dir>/pointbreak`, i.e. `.git/pointbreak`). Every non-ephemeral worktree of
/// a clone — main and linked — reads and writes here, with no `store link`. Use
/// this for store-path assertions after a `pointbreak` write instead of the raw
/// worktree-local `.pointbreak/data`.
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
    Path::new(&common_dir).join("pointbreak")
}

#[track_caller]
#[allow(dead_code)]
pub fn assert_existing_paths_eq(actual: &Path, expected: &Path) {
    assert_eq!(
        actual.canonicalize().expect("canonicalize actual path"),
        expected.canonicalize().expect("canonicalize expected path")
    );
}
