use std::fs;
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

#[test]
fn dump_cli_loads_native_review_notes() {
    let repo = dump_repo();
    let sidecar_dir = tempfile::tempdir().expect("create sidecar tempdir");
    let sidecar_path = sidecar_dir.path().join("review-notes.json");
    fs::write(&sidecar_path, native_review_notes_json()).expect("write review notes");

    let output = shore([
        "dump",
        "--repo",
        repo.path().to_str().unwrap(),
        "--review-notes",
        sidecar_path.to_str().unwrap(),
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    let json = parse_json(&stdout);
    assert_eq!(json["input"]["source"], "review_notes");
    assert_eq!(json["summary"]["note_count"], 1);
    assert_eq!(json["summary"]["diagnostic_count"], 0);
    assert_eq!(
        file_header_paths(&json),
        vec!["src/untracked.rs", "src/lib.rs"]
    );
    assert!(has_note_row(&json));
}

#[test]
fn dump_cli_loads_durable_imported_notes_by_default() {
    let repo = dump_repo();
    let sidecar_path = repo.write_fixture("review-notes.json", native_review_notes_json());

    let apply = shore([
        "notes",
        "apply",
        "--repo",
        repo.path().to_str().unwrap(),
        "--review-notes",
        sidecar_path.to_str().unwrap(),
    ]);
    assert!(
        apply.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&apply.stderr)
    );

    let output = shore(["dump", "--repo", repo.path().to_str().unwrap()]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    let json = parse_json(&stdout);
    assert_eq!(json["input"]["source"], "durable");
    assert_eq!(json["summary"]["note_count"], 1);
    assert!(
        json["notes"]
            .as_array()
            .expect("notes are an array")
            .iter()
            .any(|note| note["title"] == "Untracked note")
    );
}

#[test]
fn dump_cli_explicit_review_notes_overrides_durable_default() {
    let repo = dump_repo();
    let durable_dir = tempfile::tempdir().expect("create durable tempdir");
    let durable_path = durable_dir.path().join("review-notes.json");
    fs::write(&durable_path, native_review_notes_json()).expect("write durable review notes");
    let apply = shore([
        "notes",
        "apply",
        "--repo",
        repo.path().to_str().unwrap(),
        "--review-notes",
        durable_path.to_str().unwrap(),
    ]);
    assert!(
        apply.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&apply.stderr)
    );

    let explicit_dir = tempfile::tempdir().expect("create explicit tempdir");
    let explicit_path = explicit_dir.path().join("override-review-notes.json");
    fs::write(&explicit_path, explicit_review_notes_json()).expect("write explicit review notes");

    let output = shore([
        "dump",
        "--repo",
        repo.path().to_str().unwrap(),
        "--review-notes",
        explicit_path.to_str().unwrap(),
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    let json = parse_json(&stdout);
    assert_eq!(json["input"]["source"], "review_notes");
    assert_eq!(json["summary"]["note_count"], 1);
    assert!(
        json["notes"]
            .as_array()
            .expect("notes are an array")
            .iter()
            .any(|note| note["title"] == "Explicit sidecar title")
    );
}

#[test]
fn dump_cli_excludes_explicit_in_repo_review_notes_from_stream() {
    let repo = dump_repo();
    let sidecar_path = repo.path().join("review-notes.json");
    fs::write(&sidecar_path, native_review_notes_json()).expect("write review notes");

    let output = shore([
        "dump",
        "--repo",
        repo.path().to_str().unwrap(),
        "--review-notes",
        sidecar_path.to_str().unwrap(),
        "--pretty",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    let json = parse_json(&stdout);
    let paths = snapshot_paths(&json);
    assert!(!paths.iter().any(|path| path == "review-notes.json"));
    assert!(paths.iter().any(|path| path == "src/untracked.rs"));
}

#[test]
fn dump_cli_includes_recoverable_review_notes_diagnostics() {
    let repo = dump_repo();
    let sidecar_dir = tempfile::tempdir().expect("create sidecar tempdir");
    let sidecar_path = sidecar_dir.path().join("review-notes.json");
    fs::write(&sidecar_path, recoverable_review_notes_json()).expect("write review notes");

    let output = shore([
        "dump",
        "--repo",
        repo.path().to_str().unwrap(),
        "--review-notes",
        sidecar_path.to_str().unwrap(),
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    let json = parse_json(&stdout);
    assert_eq!(json["summary"]["diagnostic_count"], 2);
    assert_eq!(json["diagnostics"][0]["code"], "missing_version");
    assert_eq!(json["diagnostics"][1]["code"], "missing_note_title");
}

#[test]
fn dump_cli_rejects_malformed_review_notes_json() {
    let repo = dump_repo();
    let sidecar_dir = tempfile::tempdir().expect("create sidecar tempdir");
    let sidecar_path = sidecar_dir.path().join("review-notes.json");
    fs::write(&sidecar_path, "{").expect("write malformed review notes");

    let output = shore([
        "dump",
        "--repo",
        repo.path().to_str().unwrap(),
        "--review-notes",
        sidecar_path.to_str().unwrap(),
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("json parse failed"));
}

#[test]
fn dump_cli_missing_review_notes_error_names_path() {
    let repo = dump_repo();
    let missing_path = repo.path().join("missing-review-notes.json");

    let output = shore([
        "dump",
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
fn dump_cli_excludes_explicit_in_repo_log_file_from_stream() {
    let repo = dump_repo();
    let log_path = repo.path().join("shore.log");

    let output = shore([
        "--log",
        "shore=debug",
        "--log-file",
        log_path.to_str().unwrap(),
        "dump",
        "--repo",
        repo.path().to_str().unwrap(),
        "--pretty",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(log_path.exists());
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    let json = parse_json(&stdout);
    let paths = snapshot_paths(&json);
    assert!(!paths.iter().any(|path| path == "shore.log"));
    assert!(paths.iter().any(|path| path == "src/untracked.rs"));
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

fn parse_json(stdout: &str) -> Value {
    serde_json::from_str(stdout).expect("stdout is valid JSON")
}

fn file_header_paths(json: &Value) -> Vec<String> {
    json["stream"]["rows"]
        .as_array()
        .expect("stream rows are an array")
        .iter()
        .filter_map(|row| {
            row["kind"]
                .as_object()?
                .get("file_header")?
                .get("path")?
                .as_str()
                .map(str::to_owned)
        })
        .collect()
}

fn snapshot_paths(json: &Value) -> Vec<String> {
    json["snapshot"]["files"]
        .as_array()
        .expect("snapshot files are an array")
        .iter()
        .map(|file| {
            file["new_path"]
                .as_str()
                .or_else(|| file["old_path"].as_str())
                .expect("file has a path")
                .to_owned()
        })
        .collect()
}

fn has_note_row(json: &Value) -> bool {
    json["stream"]["rows"]
        .as_array()
        .expect("stream rows are an array")
        .iter()
        .any(|row| {
            row["kind"]
                .as_object()
                .is_some_and(|kind| kind.contains_key("note"))
        })
}

fn dump_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.write("src/untracked.rs", "pub fn untracked() -> u32 { 3 }\n");
    repo
}

fn native_review_notes_json() -> &'static str {
    r#"{
  "schema": "shore.review-notes",
  "version": 1,
  "summary": "CLI review notes",
  "files": [
    {
      "path": "src/untracked.rs",
      "notes": [
        {
          "id": "note:untracked",
          "title": "Untracked note",
          "body": "Review this new file.",
          "target": {
            "side": "new",
            "startLine": 1,
            "endLine": 1
          },
          "author": "human reviewer",
          "source": "reviewer"
        }
      ]
    },
    {
      "path": "src/lib.rs",
      "notes": []
    }
  ]
}"#
}

fn explicit_review_notes_json() -> &'static str {
    r#"{
  "schema": "shore.review-notes",
  "version": 1,
  "files": [
    {
      "path": "src/lib.rs",
      "notes": [
        {
          "title": "Explicit sidecar title",
          "body": "Explicit override body.",
          "target": { "side": "new", "startLine": 1, "endLine": 1 }
        }
      ]
    }
  ]
}"#
}

fn recoverable_review_notes_json() -> &'static str {
    r#"{
  "schema": "shore.review-notes",
  "files": [
    {
      "path": "src/lib.rs",
      "notes": [
        {
          "body": "Missing title remains recoverable.",
          "target": {
            "side": "new",
            "startLine": 1,
            "endLine": 1
          }
        }
      ]
    }
  ]
}"#
}
