use std::process::{Command, Output};

use serde_json::Value;

mod support;

use support::git_repo::GitRepo;

#[test]
fn cli_default_logging_has_empty_stderr() {
    let repo = dump_repo();

    let output = shore_without_log_env(["history", "--repo", repo.path().to_str().unwrap()]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .starts_with("{\"schema\":\"pointbreak.review-history\"")
    );
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}

#[test]
fn cli_log_filter_writes_trace_output_to_stderr_without_polluting_stdout() {
    let repo = dump_repo();

    let output = pointbreak([
        "--log",
        "pointbreak=debug",
        "history",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");

    assert!(stdout.starts_with("{\"schema\":\"pointbreak.review-history\""));
    assert!(!stdout.contains("shore::"));
    assert!(stderr.contains("shore") || stderr.contains("event"));
}

#[test]
fn cli_log_file_writes_trace_output_to_file_not_stdout_or_stderr() {
    let repo = dump_repo();
    let log_path = repo.path().join("pointbreak.log");

    let output = pointbreak([
        "--log",
        "pointbreak=debug",
        "--log-file",
        log_path.to_str().unwrap(),
        "history",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .starts_with("{\"schema\":\"pointbreak.review-history\"")
    );
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
    assert!(std::fs::read_to_string(log_path).unwrap().contains("shore"));
}

#[test]
fn cli_log_json_format_writes_parseable_json_lines() {
    let repo = dump_repo();
    let log_path = repo.path().join("pointbreak.jsonl");

    let output = pointbreak([
        "--log",
        "pointbreak=debug",
        "--log-format",
        "json",
        "--log-file",
        log_path.to_str().unwrap(),
        "history",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());

    let contents = std::fs::read_to_string(log_path).unwrap();
    let first_line = contents.lines().next().expect("json log line");
    serde_json::from_str::<Value>(first_line).expect("trace line is json");
    assert!(!first_line.contains("\u{1b}["));
}

#[test]
fn cli_log_filter_precedence_is_flag_then_shore_log_then_rust_log() {
    let repo = dump_repo();

    let flag_beats_env = shore_with_env(
        [
            "--log",
            "off",
            "history",
            "--repo",
            repo.path().to_str().unwrap(),
        ],
        [
            ("POINTBREAK_LOG", "pointbreak=debug"),
            ("RUST_LOG", "pointbreak=debug"),
        ],
    );
    assert!(String::from_utf8_lossy(&flag_beats_env.stderr).is_empty());

    let shore_log_beats_rust_log = shore_with_env(
        ["history", "--repo", repo.path().to_str().unwrap()],
        [("POINTBREAK_LOG", "off"), ("RUST_LOG", "pointbreak=debug")],
    );
    assert!(String::from_utf8_lossy(&shore_log_beats_rust_log.stderr).is_empty());
}

#[test]
fn cli_log_invalid_filter_exits_nonzero() {
    let output = pointbreak(["--log", "[", "history", "--repo", "."]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid log filter"));
}

fn pointbreak<I, S>(args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    command(args).output().expect("run pointbreak binary")
}

fn shore_without_log_env<I, S>(args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    command(args)
        .env_remove("POINTBREAK_LOG")
        .env_remove("RUST_LOG")
        .output()
        .expect("run pointbreak binary")
}

fn shore_with_env<I, S, E, K, V>(args: I, env: E) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
    E: IntoIterator<Item = (K, V)>,
    K: AsRef<std::ffi::OsStr>,
    V: AsRef<std::ffi::OsStr>,
{
    command(args)
        .envs(env)
        .output()
        .expect("run pointbreak binary")
}

fn command<I, S>(args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut command = Command::new(support::pointbreak_bin());
    command
        .args(args)
        // Isolate byte-asserting tracing tests from an ambient output-lane selector;
        // these tests deliberately keep POINTBREAK_LOG/RUST_LOG to exercise logging.
        .env_remove("POINTBREAK_FORMAT")
        .current_dir(std::env::current_dir().expect("current dir"));
    command
}

fn dump_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.write("src/untracked.rs", "pub fn untracked() -> u32 { 3 }\n");
    repo
}
