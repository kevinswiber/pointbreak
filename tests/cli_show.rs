use std::fs;
use std::path::Path;
use std::process::Command;

#[allow(dead_code)]
#[path = "support/git_repo.rs"]
mod git_repo;

use git_repo::GitRepo;

#[test]
fn show_cli_help_lists_sidecar_flags() {
    let output = shore(["show", "--help"]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("--repo"));
    assert!(stdout.contains("--review-notes"));
    assert!(stdout.contains("--legacy-hunk-agent-context"));
}

#[test]
fn show_cli_rejects_native_and_legacy_sidecars_together() {
    let repo = dump_repo();
    let sidecar_dir = tempfile::tempdir().expect("create sidecar tempdir");
    let review_notes_path = sidecar_dir.path().join("review-notes.json");
    let legacy_path = sidecar_dir.path().join("agent-context.json");
    fs::write(&review_notes_path, "{}").expect("write review notes");
    fs::write(&legacy_path, "{}").expect("write Hunk context");

    let output = shore([
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
        "--review-notes",
        review_notes_path.to_str().unwrap(),
        "--legacy-hunk-agent-context",
        legacy_path.to_str().unwrap(),
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used with"));
}

#[test]
fn show_cli_rejects_unreadable_review_notes_before_terminal_setup() {
    let repo = dump_repo();
    let missing_path = repo.path().join("missing-review-notes.json");

    let output = shore([
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
        "--review-notes",
        missing_path.to_str().unwrap(),
    ]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("review notes"), "stderr:\n{stderr}");
    assert!(
        stderr.contains("missing-review-notes.json"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn dump_and_show_help_share_input_flag_spelling() {
    let dump = shore(["dump", "--help"]);
    let show = shore(["show", "--help"]);

    assert!(dump.status.success());
    assert!(show.status.success());
    let dump_stdout = String::from_utf8(dump.stdout).expect("dump stdout is utf-8");
    let show_stdout = String::from_utf8(show.stdout).expect("show stdout is utf-8");

    for flag in ["--repo", "--review-notes", "--legacy-hunk-agent-context"] {
        assert!(dump_stdout.contains(flag), "dump help missing {flag}");
        assert!(show_stdout.contains(flag), "show help missing {flag}");
    }
    assert!(dump_stdout.contains("--pretty"));
    assert!(dump_stdout.contains("--compact"));
    assert!(!show_stdout.contains("--pretty"));
    assert!(!show_stdout.contains("--compact"));
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
        .env_remove("SHORE_LOG")
        .env_remove("RUST_LOG")
        .current_dir(cwd)
        .output()
        .expect("run shore binary")
}

fn dump_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo
}
