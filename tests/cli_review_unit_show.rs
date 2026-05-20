mod support;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore;

#[test]
fn review_unit_help_lists_show() {
    let output = shore(["review", "unit", "--help"]);

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("show"));
}

#[test]
fn review_unit_show_emits_v1_json() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "unit",
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);

    assert_eq!(json["schema"], "shore.review-unit");
    assert_eq!(json["version"], 1);
    assert!(
        json["eventSetHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert_eq!(json["eventCount"], 1);
    assert_eq!(json["reviewUnit"]["id"], json["filters"]["reviewUnitId"]);
    assert_eq!(json["currentAssessment"]["status"], "unassessed");
    assert!(json["currentAssessment"].get("assessment").is_none());
    assert!(json["currentAssessment"].get("assessmentId").is_none());
    assert!(json.get("statePath").is_none());
}

#[test]
fn review_unit_show_rejects_invalid_track_before_json_output() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "unit",
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "Agent Codex",
    ]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("track"));
}

#[test]
fn review_unit_show_pretty_prints() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "unit",
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
        "--pretty",
    ]);

    assert!(String::from_utf8_lossy(&output.stdout).starts_with("{\n"));
}

#[test]
fn review_unit_show_rejects_pretty_and_compact_together() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "unit",
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
        "--pretty",
        "--compact",
    ]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used with"));
}

#[test]
fn review_unit_show_supports_explicit_review_unit_when_ambiguous() {
    let repo = modified_repo();
    let first =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    let second =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);

    let ambiguous = shore([
        "review",
        "unit",
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);
    assert!(!ambiguous.status.success());
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("multiple captured review units"));

    let explicit = shore([
        "review",
        "unit",
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
        "--review-unit",
        first["reviewUnit"]["id"].as_str().unwrap(),
    ]);
    let json = parse_json(&explicit.stdout);

    assert_ne!(first["reviewUnit"]["id"], second["reviewUnit"]["id"]);
    assert_eq!(json["reviewUnit"]["id"], first["reviewUnit"]["id"]);
}

#[test]
fn review_unit_show_include_body_hydrates_without_internal_paths() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    add_observation_with_body(&repo, "agent:codex", "Body", "visible body");

    let output = shore([
        "review",
        "unit",
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
        "--include-body",
    ]);
    let json = parse_json(&output.stdout);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(json["filters"]["includeBody"], true);
    assert!(stdout.contains("visible body"));
    assert!(!stdout.contains("artifacts/notes/"));
    assert!(!stdout.contains("artifacts/snapshots/"));
    assert!(!stdout.contains(".shore/events"));
    assert!(json.get("statePath").is_none());
    assert!(json.get("snapshotArtifactPath").is_none());
}

#[test]
fn review_unit_show_rows_are_narrative_first_and_snapshot_complete() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    add_observation(&repo, "agent:codex", "Narrative");

    let json = parse_json(
        &shore([
            "review",
            "unit",
            "show",
            "--repo",
            repo.path().to_str().unwrap(),
        ])
        .stdout,
    );

    let rows = json["rows"].as_array().unwrap();
    let first_remainder = rows
        .iter()
        .position(|row| row["projectionPhase"] == "snapshot_remainder")
        .unwrap();
    let narrative = rows
        .iter()
        .position(|row| row["projectionPhase"] == "narrative")
        .unwrap();

    assert!(narrative < first_remainder);
    assert!(
        json["summary"]["snapshotRemainderRowCount"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(
        rows.iter()
            .filter(|row| row["snapshotOrder"].is_object())
            .count() as u64,
        json["summary"]["snapshotRowCount"].as_u64().unwrap()
    );
}

#[test]
fn review_unit_show_track_filter_echoes_and_narrows_narrative_only() {
    let repo = multi_file_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    add_observation(&repo, "agent:codex", "Codex");
    add_observation(&repo, "agent:claude", "Claude");

    let all = parse_json(
        &shore([
            "review",
            "unit",
            "show",
            "--repo",
            repo.path().to_str().unwrap(),
        ])
        .stdout,
    );
    let codex = parse_json(
        &shore([
            "review",
            "unit",
            "show",
            "--repo",
            repo.path().to_str().unwrap(),
            "--track",
            "agent:codex",
        ])
        .stdout,
    );

    assert_eq!(codex["filters"]["trackId"], "agent:codex");
    assert_eq!(codex["observations"].as_array().unwrap().len(), 1);
    assert_eq!(codex["observations"][0]["title"], "Codex");
    assert!(
        all["summary"]["narrativeRowCount"].as_u64().unwrap()
            > codex["summary"]["narrativeRowCount"].as_u64().unwrap()
    );
    assert_eq!(
        all["summary"]["snapshotRemainderRowCount"],
        codex["summary"]["snapshotRemainderRowCount"]
    );
}

#[test]
fn review_unit_show_includes_current_assessment_status() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    add_assessment(&repo);

    let json = parse_json(
        &shore([
            "review",
            "unit",
            "show",
            "--repo",
            repo.path().to_str().unwrap(),
        ])
        .stdout,
    );

    assert_eq!(json["currentAssessment"]["status"], "resolved");
    assert_eq!(json["currentAssessment"]["assessment"], "accepted");
    assert_eq!(json["assessments"].as_array().unwrap().len(), 1);
}

#[test]
fn review_unit_show_includes_adapter_notes_without_storage_paths() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let review_notes = repo.write_fixture("review-notes.json", native_review_notes_json());
    shore([
        "notes",
        "apply",
        "--repo",
        repo.path().to_str().unwrap(),
        "--review-notes",
        review_notes.to_str().unwrap(),
    ]);

    let output = shore([
        "review",
        "unit",
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);
    let json = parse_json(&output.stdout);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(json["adapterNotes"].as_array().unwrap().len(), 1);
    assert_eq!(json["adapterNotes"][0]["title"], "Changed return value");
    assert_eq!(json["adapterNotes"][0]["status"], "exact");
    assert!(
        json["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["kind"] == "adapter_note")
    );
    assert!(!stdout.contains("artifacts/notes/"));
    assert!(!stdout.contains(".shore/events"));
}

fn modified_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo
}

fn multi_file_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.write("src/other.rs", "pub fn other() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.write("src/other.rs", "pub fn other() -> u32 { 2 }\n");
    repo
}

fn add_observation(repo: &GitRepo, track: &str, title: &str) -> Value {
    parse_json(
        &shore([
            "review",
            "observation",
            "add",
            "--repo",
            repo.path().to_str().unwrap(),
            "--track",
            track,
            "--title",
            title,
        ])
        .stdout,
    )
}

fn add_observation_with_body(repo: &GitRepo, track: &str, title: &str, body: &str) -> Value {
    parse_json(
        &shore([
            "review",
            "observation",
            "add",
            "--repo",
            repo.path().to_str().unwrap(),
            "--track",
            track,
            "--title",
            title,
            "--body",
            body,
        ])
        .stdout,
    )
}

fn add_assessment(repo: &GitRepo) -> Value {
    parse_json(
        &shore([
            "review",
            "assessment",
            "add",
            "--repo",
            repo.path().to_str().unwrap(),
            "--track",
            "human:kevin",
            "--assessment",
            "accepted",
            "--summary",
            "ship it",
        ])
        .stdout,
    )
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

fn parse_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("valid json")
}
