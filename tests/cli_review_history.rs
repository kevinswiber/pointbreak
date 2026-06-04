mod support;

use serde_json::Value;
use shoreline::model::ValidationStatus;
use shoreline::session::{ValidationAddOptions, record_validation_check};
use support::git_repo::GitRepo;
use support::shore;

#[test]
fn review_history_emits_v1_json_with_freshness_metadata() {
    let repo = modified_repo();
    let capture =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);

    let output = shore(["review", "history", "--repo", repo.path().to_str().unwrap()]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);

    assert_eq!(json["schema"], "shore.review-history");
    assert_eq!(json["version"], 1);
    assert!(
        json["eventSetHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert_eq!(json["eventCount"], 1);
    assert_eq!(json["historyCount"], 1);
    assert_eq!(json["entries"][0]["eventType"], "review_unit_captured");
    assert_eq!(
        json["entries"][0]["reviewUnitId"],
        capture["reviewUnit"]["id"]
    );
    assert!(json.get("statePath").is_none());
}

#[test]
fn review_history_pretty_prints() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "history",
        "--repo",
        repo.path().to_str().unwrap(),
        "--pretty",
    ]);

    assert!(String::from_utf8_lossy(&output.stdout).starts_with("{\n"));
}

#[test]
fn review_history_filters_by_track_and_event_type() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    add_observation(&repo, "agent:codex", "Keep");
    add_observation(&repo, "agent:claude", "Drop");

    let output = shore([
        "review",
        "history",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--event-type",
        "review-observation-recorded",
    ]);
    let json = parse_json(&output.stdout);

    assert_eq!(json["filters"]["trackId"], "agent:codex");
    assert_eq!(
        json["filters"]["eventTypes"],
        serde_json::json!(["review_observation_recorded"])
    );
    assert_eq!(json["entries"].as_array().unwrap().len(), 1);
    assert_eq!(json["entries"][0]["summary"]["title"], "Keep");
}

#[test]
fn review_history_include_body_hydrates_text_without_artifact_paths() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    add_observation_with_body(&repo, "agent:codex", "Body", "history details");

    let output = shore([
        "review",
        "history",
        "--repo",
        repo.path().to_str().unwrap(),
        "--include-body",
    ]);
    let json = parse_json(&output.stdout);

    assert_eq!(json["filters"]["includeBody"], true);
    assert!(json["entries"].to_string().contains("history details"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("artifacts/notes/"));
}

#[test]
fn review_history_filters_input_request_events_and_hydrates_text() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = add_input_request_with_body(&repo, "request details");
    respond_to_input_request(
        &repo,
        requested["inputRequestId"].as_str().unwrap(),
        "approved",
    );

    let output = shore([
        "review",
        "history",
        "--repo",
        repo.path().to_str().unwrap(),
        "--event-type",
        "input-request-opened",
        "--event-type",
        "input-request-responded",
        "--include-body",
    ]);
    let json = parse_json(&output.stdout);

    assert_eq!(
        json["filters"]["eventTypes"],
        serde_json::json!(["input_request_opened", "input_request_responded"])
    );
    assert_eq!(json["entries"].as_array().unwrap().len(), 2);
    assert!(json["entries"].as_array().unwrap().iter().any(|entry| {
        entry["eventType"] == "input_request_opened"
            && entry["summary"]["kind"] == "input_request_opened"
            && entry["summary"]["inputRequestId"] == requested["inputRequestId"]
            && entry["summary"]["mode"] == "operative"
            && entry["summary"]["body"] == "request details"
    }));
    assert!(json["entries"].as_array().unwrap().iter().any(|entry| {
        entry["eventType"] == "input_request_responded"
            && entry["summary"]["kind"] == "input_request_responded"
            && entry["summary"]["inputRequestId"] == requested["inputRequestId"]
            && entry["summary"]["reason"] == "approved"
    }));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("artifacts/notes/"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("\"blocking\""));
}

#[test]
fn cli_review_history_filters_validation_check_recorded() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    add_validation_check(&repo);
    add_observation(&repo, "agent:codex", "Other event");

    let output = shore([
        "review",
        "history",
        "--repo",
        repo.path().to_str().unwrap(),
        "--event-type",
        "validation-check-recorded",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = parse_json(&output.stdout);
    let kinds: Vec<&str> = value["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["summary"]["kind"].as_str().unwrap())
        .collect();
    assert!(
        kinds
            .iter()
            .all(|kind| *kind == "validation_check_recorded")
    );
    assert_eq!(kinds.len(), 1);
}

#[test]
fn review_history_input_request_opened_uses_envelope_mode_values() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let operative = add_input_request_with_body(&repo, "operative details");
    let advisory = add_input_request_with_mode(&repo, "FYI", "advisory");

    let output = shore([
        "review",
        "history",
        "--repo",
        repo.path().to_str().unwrap(),
        "--event-type",
        "input-request-opened",
    ]);
    let json = parse_json(&output.stdout);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(json["entries"].as_array().unwrap().len(), 2);
    assert!(json["entries"].as_array().unwrap().iter().any(|entry| {
        entry["summary"]["inputRequestId"] == operative["inputRequestId"]
            && entry["summary"]["mode"] == "operative"
    }));
    assert!(json["entries"].as_array().unwrap().iter().any(|entry| {
        entry["summary"]["inputRequestId"] == advisory["inputRequestId"]
            && entry["summary"]["mode"] == "advisory"
    }));
    assert!(!stdout.contains("\"blocking\""));
}

#[test]
fn review_history_rejects_legacy_input_request_event_filter_names() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    for event_type in ["intervention-requested", "intervention-resolved"] {
        let output = shore([
            "review",
            "history",
            "--repo",
            repo.path().to_str().unwrap(),
            "--event-type",
            event_type,
        ]);

        assert!(!output.status.success(), "{event_type} should be rejected");
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("invalid value"));
        assert!(stderr.contains("input-request-opened"));
        assert!(stderr.contains("input-request-responded"));
    }
}

#[test]
fn review_history_filters_by_review_unit() {
    let repo = modified_repo();
    let first =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    let second =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);

    let output = shore([
        "review",
        "history",
        "--repo",
        repo.path().to_str().unwrap(),
        "--review-unit",
        first["reviewUnit"]["id"].as_str().unwrap(),
        "--event-type",
        "review-unit-captured",
    ]);
    let json = parse_json(&output.stdout);

    assert_ne!(first["reviewUnit"]["id"], second["reviewUnit"]["id"]);
    assert_eq!(json["eventCount"], 2);
    assert_eq!(json["historyCount"], 1);
    assert_eq!(
        json["entries"][0]["reviewUnitId"],
        first["reviewUnit"]["id"]
    );
}

#[test]
fn review_history_reports_duplicate_semantic_diagnostics_without_collapsing_entries() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    add_observation_with_key(&repo, "retry-a");
    add_observation_with_key(&repo, "retry-b");

    let output = shore([
        "review",
        "history",
        "--repo",
        repo.path().to_str().unwrap(),
        "--event-type",
        "review-observation-recorded",
    ]);
    let json = parse_json(&output.stdout);

    assert_eq!(json["entries"].as_array().unwrap().len(), 2);
    assert!(
        json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| { diagnostic["code"] == "duplicate_semantic_observation_event" })
    );
}

#[test]
fn review_history_succeeds_without_events() {
    let repo = GitRepo::new();

    let output = shore(["review", "history", "--repo", repo.path().to_str().unwrap()]);
    let json = parse_json(&output.stdout);

    assert!(output.status.success());
    assert_eq!(json["eventCount"], 0);
    assert_eq!(json["historyCount"], 0);
    assert!(json["entries"].as_array().unwrap().is_empty());
}

#[test]
fn review_history_includes_imported_review_notes() {
    let repo = modified_repo();
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
        "history",
        "--repo",
        repo.path().to_str().unwrap(),
        "--event-type",
        "review-note-imported",
    ]);
    let json = parse_json(&output.stdout);

    assert_eq!(json["historyCount"], 1);
    assert_eq!(json["entries"][0]["eventType"], "review_note_imported");
    assert_eq!(
        json["entries"][0]["summary"]["title"],
        "Changed return value"
    );
}

fn parse_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("parse CLI JSON")
}

fn modified_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
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

fn add_validation_check(repo: &GitRepo) {
    record_validation_check(
        ValidationAddOptions::new(repo.path())
            .with_track("agent:codex")
            .with_check_name("cargo test")
            .with_status(ValidationStatus::Passed),
    )
    .unwrap();
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

fn add_observation_with_key(repo: &GitRepo, key: &str) -> Value {
    parse_json(
        &shore([
            "review",
            "observation",
            "add",
            "--repo",
            repo.path().to_str().unwrap(),
            "--track",
            "agent:codex",
            "--title",
            "Duplicate",
            "--body",
            "same body",
            "--idempotency-key",
            key,
        ])
        .stdout,
    )
}

fn add_input_request_with_body(repo: &GitRepo, body: &str) -> Value {
    parse_json(
        &shore([
            "review",
            "input-request",
            "open",
            "--repo",
            repo.path().to_str().unwrap(),
            "--track",
            "agent:codex",
            "--title",
            "Need decision",
            "--reason",
            "manual-decision-required",
            "--body",
            body,
        ])
        .stdout,
    )
}

fn add_input_request_with_mode(repo: &GitRepo, title: &str, mode: &str) -> Value {
    parse_json(
        &shore([
            "review",
            "input-request",
            "open",
            "--repo",
            repo.path().to_str().unwrap(),
            "--track",
            "agent:codex",
            "--title",
            title,
            "--reason",
            "manual-decision-required",
            "--mode",
            mode,
        ])
        .stdout,
    )
}

fn respond_to_input_request(repo: &GitRepo, input_request_id: &str, reason: &str) -> Value {
    parse_json(
        &shore([
            "review",
            "input-request",
            "respond",
            "--repo",
            repo.path().to_str().unwrap(),
            input_request_id,
            "--outcome",
            "approved",
            "--reason",
            reason,
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
