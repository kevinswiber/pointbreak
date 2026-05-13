use std::path::Path;
use std::process::Command;

use serde_json::Value;

#[allow(dead_code)]
#[path = "support/git_repo.rs"]
mod git_repo;

use git_repo::GitRepo;

#[test]
fn notes_apply_writes_json_to_stdout_with_correct_schema() {
    let repo = modified_repo();
    let sidecar = write_native_review_notes(&repo);

    let output = shore([
        "notes",
        "apply",
        "--repo",
        repo.path().to_str().unwrap(),
        "--review-notes",
        sidecar.to_str().unwrap(),
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    assert_eq!(json["schema"], "shore.notes-apply");
    assert_eq!(json["version"], 1);
    assert_eq!(json["noteCount"], 1);
    assert_eq!(json["notesCreated"], 1);
    assert_eq!(json["notesExisting"], 0);
}

#[test]
fn notes_apply_requires_exactly_one_sidecar_input() {
    let repo = modified_repo();

    let missing = shore(["notes", "apply", "--repo", repo.path().to_str().unwrap()]);
    assert!(!missing.status.success());
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("required"),
        "stderr:\n{}",
        String::from_utf8_lossy(&missing.stderr)
    );
}

#[test]
fn notes_apply_missing_input_error_names_path() {
    let repo = modified_repo();
    let missing_path = repo.path().join("missing-review-notes.json");

    let output = shore([
        "notes",
        "apply",
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
fn notes_apply_reimport_reports_idempotent_counts() {
    let repo = modified_repo();
    let sidecar = write_native_review_notes(&repo);

    let first = shore([
        "notes",
        "apply",
        "--repo",
        repo.path().to_str().unwrap(),
        "--review-notes",
        sidecar.to_str().unwrap(),
    ]);
    assert!(
        first.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let second = shore([
        "notes",
        "apply",
        "--repo",
        repo.path().to_str().unwrap(),
        "--review-notes",
        sidecar.to_str().unwrap(),
    ]);

    assert!(
        second.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let json = parse_json(&second.stdout);
    assert_eq!(json["notesCreated"], 0);
    assert_eq!(json["notesExisting"], 1);
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

fn parse_json(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).expect("stdout is valid JSON")
}

fn modified_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo
}

fn write_native_review_notes(repo: &GitRepo) -> std::path::PathBuf {
    repo.write_fixture("review-notes.json", native_review_notes_json())
}

fn native_review_notes_json() -> &'static str {
    r#"{
  "schema": "shore.review-notes",
  "version": 1,
  "files": [
    {
      "path": "src/lib.rs",
      "notes": [
        {
          "title": "Changed return value",
          "target": { "side": "new", "startLine": 1, "endLine": 1 }
        }
      ]
    }
  ]
}"#
}
