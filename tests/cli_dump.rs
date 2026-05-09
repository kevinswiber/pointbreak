use std::path::Path;
use std::process::Command;

use serde_json::Value;

#[allow(dead_code)]
#[path = "support/git_repo.rs"]
mod git_repo;

use git_repo::GitRepo;

#[test]
fn dump_cli_prints_compact_json_for_repo() {
    let repo = dump_repo();

    let output = shore(["dump", "--repo", repo.path().to_str().unwrap()]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert_eq!(stdout.lines().count(), 1);
    let json = parse_json(&stdout);
    assert_eq!(json["schema"], "shore.dump");
    assert_eq!(json["summary"]["file_count"], 2);
    assert_eq!(json["summary"]["note_count"], 0);
    assert!(
        !json["stream"]["rows"]
            .as_array()
            .expect("stream rows are an array")
            .is_empty()
    );
}

#[test]
fn dump_cli_defaults_repo_to_current_directory() {
    let repo = dump_repo();

    let output = shore_in(repo.path(), ["dump"]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    let json = parse_json(&stdout);
    assert_eq!(json["summary"]["file_count"], 2);
}

#[test]
fn dump_cli_accepts_repo_subdirectory() {
    let repo = dump_repo();
    let src = repo.path().join("src");

    let output = shore(["dump", "--repo", src.to_str().unwrap()]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    let json = parse_json(&stdout);
    assert_eq!(json["summary"]["file_count"], 2);
}

#[test]
fn dump_cli_pretty_and_compact_flags_control_formatting() {
    let repo = dump_repo();

    let pretty = shore(["dump", "--repo", repo.path().to_str().unwrap(), "--pretty"]);
    assert!(
        pretty.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&pretty.stderr)
    );
    let pretty_stdout = String::from_utf8(pretty.stdout).expect("pretty stdout is utf-8");
    assert!(pretty_stdout.lines().count() > 1);

    let compact = shore(["dump", "--repo", repo.path().to_str().unwrap(), "--compact"]);
    assert!(
        compact.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&compact.stderr)
    );
    let compact_stdout = String::from_utf8(compact.stdout).expect("compact stdout is utf-8");
    assert_eq!(compact_stdout.lines().count(), 1);
}

#[test]
fn dump_cli_rejects_unknown_flags() {
    let output = shore(["dump", "--unknown"]);

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("unexpected argument")
            || String::from_utf8_lossy(&output.stderr).contains("unknown")
    );
}

fn shore<I, S>(args: I) -> std::process::Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    shore_in(std::env::current_dir().expect("current dir"), args)
}

fn shore_in<I, S>(cwd: impl AsRef<Path>, args: I) -> std::process::Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_shore"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run shore binary")
}

fn parse_json(stdout: &str) -> Value {
    serde_json::from_str(stdout).expect("stdout is valid JSON")
}

fn dump_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.write("src/untracked.rs", "pub fn untracked() -> u32 { 3 }\n");
    repo
}
