use std::fs;
use std::process::Command;

use serde_json::Value;

#[allow(dead_code)]
#[path = "support/git_repo.rs"]
mod git_repo;

use git_repo::GitRepo;

#[test]
fn review_publish_creates_shore_state_at_worktree_root_from_subdir() {
    let repo = modified_repo();
    let subdir = repo.path().join("src");

    let output = shore(["review", "publish", "--repo", subdir.to_str().unwrap()]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(repo.path().join(".shore/events").is_dir());
    assert!(repo.path().join(".shore/state.json").is_file());

    let json = parse_json(&output.stdout);
    assert_eq!(json["schema"], "shore.publish");
    assert_eq!(json["version"], 1);
    assert_eq!(json["reviewId"], "review:default");
    assert_eq!(json["workUnitId"], "work:default");
    assert!(
        json["revisionId"]
            .as_str()
            .unwrap()
            .starts_with("rev:worktree:sha256:")
    );
    assert!(
        json["snapshotId"]
            .as_str()
            .unwrap()
            .starts_with("snap:git:sha256:")
    );
    assert_eq!(json["eventsCreated"], 3);
    assert_eq!(json["eventsExisting"], 0);
    assert_eq!(json["eventsCreatedByType"]["review_initialized"], 1);
    assert_eq!(json["eventsCreatedByType"]["revision_published"], 1);
    assert_eq!(json["eventsCreatedByType"]["snapshot_observed"], 1);
    assert_eq!(
        json["statePath"].as_str().unwrap(),
        fs::canonicalize(repo.path().join(".shore/state.json"))
            .unwrap()
            .to_string_lossy()
    );
    assert!(json["diagnostics"].as_array().unwrap().is_empty());
}

#[test]
fn review_publish_rejects_native_and_legacy_sidecars_together() {
    let repo = modified_repo();
    let output = shore([
        "review",
        "publish",
        "--repo",
        repo.path().to_str().unwrap(),
        "--review-notes",
        "review-notes.json",
        "--legacy-hunk-agent-context",
        "agent-context.json",
    ]);

    assert!(!output.status.success());
    assert!(!repo.path().join(".shore").exists());
}

#[test]
fn review_publish_rejects_malformed_sidecar_before_shore_writes() {
    let repo = modified_repo();
    let sidecar_path = repo.path().join("bad-review-notes.json");
    fs::write(&sidecar_path, "{").expect("write malformed sidecar");

    let output = shore([
        "review",
        "publish",
        "--repo",
        repo.path().to_str().unwrap(),
        "--review-notes",
        sidecar_path.to_str().unwrap(),
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("json parse failed"));
    assert!(!repo.path().join(".shore").exists());
}

#[test]
fn review_publish_missing_review_notes_error_names_path_before_shore_writes() {
    let repo = modified_repo();
    let missing_path = repo.path().join("missing-review-notes.json");

    let output = shore([
        "review",
        "publish",
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
    assert!(!repo.path().join(".shore").exists());
}

#[test]
fn review_publish_missing_legacy_hunk_agent_context_error_names_path_before_shore_writes() {
    let repo = modified_repo();
    let missing_path = repo.path().join("missing-agent-context.json");

    let output = shore([
        "review",
        "publish",
        "--repo",
        repo.path().to_str().unwrap(),
        "--legacy-hunk-agent-context",
        missing_path.to_str().unwrap(),
    ]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("legacy Hunk agent context"),
        "stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("missing-agent-context.json"),
        "stderr:\n{stderr}"
    );
    assert!(!repo.path().join(".shore").exists());
}

#[test]
fn review_publish_with_review_notes_records_sidecar_observation() {
    let repo = modified_repo();
    let sidecar_path = repo.path().join("review-notes.json");
    fs::write(&sidecar_path, native_review_notes_json()).expect("write review notes");

    let output = shore([
        "review",
        "publish",
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
    let json = parse_json(&output.stdout);
    assert_eq!(json["eventsCreated"], 4);
    assert_eq!(json["eventsCreatedByType"]["sidecar_observed"], 1);

    let state: Value =
        serde_json::from_str(&repo.read(".shore/state.json")).expect("state is json");
    assert_eq!(state["sidecarCount"], 1);
}

#[test]
fn review_publish_excludes_explicit_in_repo_log_file_from_fingerprint() {
    let repo = modified_repo();
    let log_path = repo.path().join("shore.log");

    let first = shore([
        "--log",
        "shore=debug",
        "--log-file",
        log_path.to_str().unwrap(),
        "review",
        "publish",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);

    assert!(
        first.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_json = parse_json(&first.stdout);
    assert!(log_path.exists());

    fs::write(&log_path, "changed log contents\n").expect("write changed log");
    let second = shore([
        "--log",
        "shore=debug",
        "--log-file",
        log_path.to_str().unwrap(),
        "review",
        "publish",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);

    assert!(
        second.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_json = parse_json(&second.stdout);
    assert_eq!(first_json["revisionId"], second_json["revisionId"]);
    assert_eq!(second_json["eventsCreated"], 0);
}

fn shore<I, S>(args: I) -> std::process::Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_shore"))
        .args(args)
        .env_remove("SHORE_LOG")
        .env_remove("RUST_LOG")
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
