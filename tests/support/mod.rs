use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Output};

#[allow(dead_code)]
pub mod event_signature_fixtures;
#[allow(dead_code)]
pub mod git_repo;
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
        .env_remove("RUST_LOG");
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

#[track_caller]
#[allow(dead_code)]
pub fn assert_existing_paths_eq(actual: &Path, expected: &Path) {
    assert_eq!(
        actual.canonicalize().expect("canonicalize actual path"),
        expected.canonicalize().expect("canonicalize expected path")
    );
}
