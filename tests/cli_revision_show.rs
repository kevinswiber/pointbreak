mod support;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::{shore, shore_env};

#[test]
fn review_help_lists_show() {
    let output = shore(["review", "--help"]);

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("show"));
}

#[test]
fn revision_show_emits_v1_json() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore(["review", "show", "--repo", repo.path().to_str().unwrap()]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);

    assert_eq!(json["schema"], "shore.review-revision");
    assert_eq!(json["version"], 1);
    assert!(
        json["eventSetHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert_eq!(json["eventCount"], 2);
    assert_eq!(json["revision"]["id"], json["filters"]["revisionId"]);
    assert_eq!(json["currentAssessment"]["status"], "unassessed");
    assert!(json["currentAssessment"].get("assessment").is_none());
    assert!(json["currentAssessment"].get("assessmentId").is_none());
    assert!(json.get("statePath").is_none());
}

#[test]
fn revision_show_rejects_invalid_track_before_json_output() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
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
fn revision_show_pretty_prints() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
        "--pretty",
    ]);

    assert!(String::from_utf8_lossy(&output.stdout).starts_with("{\n"));
}

#[test]
fn revision_show_rejects_pretty_and_compact_together() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
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
fn revision_show_supports_explicit_revision_when_ambiguous() {
    let repo = modified_repo();
    let first =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    let second =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);

    let ambiguous = shore(["review", "show", "--repo", repo.path().to_str().unwrap()]);
    assert!(!ambiguous.status.success());
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("multiple captured revisions"));

    let explicit = shore([
        "review",
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
        "--revision",
        first["revision"]["id"].as_str().unwrap(),
    ]);
    let json = parse_json(&explicit.stdout);

    assert_ne!(first["revision"]["id"], second["revision"]["id"]);
    assert_eq!(json["revision"]["id"], first["revision"]["id"]);
}

#[test]
fn revision_show_include_body_hydrates_without_internal_paths() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    add_observation_with_body(&repo, "agent:codex", "Body", "visible body");

    let output = shore([
        "review",
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
    assert!(!stdout.contains(".shore/data/events"));
    assert!(json.get("statePath").is_none());
    assert!(json.get("snapshotArtifactPath").is_none());
}

#[test]
fn revision_show_includes_input_requests_and_omits_legacy_fields() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = add_input_request_with_body(&repo, "visible request body");
    respond_to_input_request(
        &repo,
        requested["inputRequestId"].as_str().unwrap(),
        "approved",
    );

    let output = shore([
        "review",
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
        "--include-body",
    ]);
    let json = parse_json(&output.stdout);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let input_request_id = requested["inputRequestId"].as_str().unwrap();

    assert_eq!(json["inputRequests"].as_array().unwrap().len(), 1);
    assert_eq!(json["inputRequests"][0]["id"], input_request_id);
    assert_eq!(json["inputRequests"][0]["mode"], "operative");
    assert_eq!(json["inputRequests"][0]["body"], "visible request body");
    assert_eq!(
        json["inputRequests"][0]["responses"][0]["reason"],
        "approved"
    );
    assert_eq!(json["summary"]["inputRequestCount"], 1);
    assert!(!stdout.contains("artifacts/notes/"));
    assert!(!stdout.contains("artifacts/snapshots/"));
    assert!(!stdout.contains(".shore/data/events"));
    assert!(!stdout.contains("\"blocking\""));
    assert!(json.get("interventions").is_none());
    assert!(json["summary"].get("interventionCount").is_none());
    assert!(json["inputRequests"][0].get("resolutions").is_none());
    assert!(json["rows"].as_array().unwrap().iter().any(|row| {
        row["kind"] == "input_request"
            && row["relatedInputRequestIds"]
                .as_array()
                .unwrap()
                .iter()
                .any(|id| id == input_request_id)
            && row.get("relatedInterventionIds").is_none()
    }));
}

#[test]
fn revision_show_rows_are_narrative_first_and_snapshot_complete() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    add_observation(&repo, "agent:codex", "Narrative");

    let json =
        parse_json(&shore(["review", "show", "--repo", repo.path().to_str().unwrap()]).stdout);

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
fn revision_show_track_filter_echoes_and_narrows_narrative_only() {
    let repo = multi_file_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    add_observation(&repo, "agent:codex", "Codex");
    add_observation(&repo, "agent:claude", "Claude");

    let all =
        parse_json(&shore(["review", "show", "--repo", repo.path().to_str().unwrap()]).stdout);
    let codex = parse_json(
        &shore([
            "review",
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
fn revision_show_includes_current_assessment_status() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    add_assessment(&repo);

    let json =
        parse_json(&shore(["review", "show", "--repo", repo.path().to_str().unwrap()]).stdout);

    assert_eq!(json["currentAssessment"]["status"], "resolved");
    assert_eq!(json["currentAssessment"]["assessment"], "accepted");
    assert_eq!(json["assessments"].as_array().unwrap().len(), 1);
}

#[test]
fn revision_show_includes_adapter_notes_without_storage_paths() {
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

    let output = shore(["review", "show", "--repo", repo.path().to_str().unwrap()]);
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
    assert!(!stdout.contains(".shore/data/events"));
}

#[test]
fn unit_show_projects_range_capture_with_bound_snapshot() {
    let repo = support::committed_repo();
    let capture = parse_json(
        &shore([
            "review",
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--base",
            "HEAD~1",
        ])
        .stdout,
    );
    let revision_id = capture["revision"]["id"].as_str().unwrap();

    let output = shore([
        "review",
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
        "--revision",
        revision_id,
    ]);

    // The command succeeding proves load_bound_snapshot_artifact validated the
    // bound snapshot against the range identity.
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json = parse_json(&output.stdout);
    assert_eq!(json["revision"]["source"]["kind"], "git_commit_range");
    assert_eq!(json["revision"]["base"]["kind"], "git_commit");
    assert_eq!(json["revision"]["target"]["kind"], "git_commit");
    assert!(json["summary"]["snapshotRowCount"].as_u64().unwrap() > 0);
    assert!(
        stdout.contains("src/lib.rs"),
        "snapshot rows must include the committed file"
    );
}

#[test]
fn unit_show_disambiguates_worktree_and_range_units() {
    let repo = support::committed_repo();
    // A dirty worktree yields a worktree unit; --base yields a range unit.
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let range = parse_json(
        &shore([
            "review",
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--base",
            "HEAD~1",
        ])
        .stdout,
    );

    let ambiguous = shore(["review", "show", "--repo", repo.path().to_str().unwrap()]);
    assert!(!ambiguous.status.success());
    assert!(
        String::from_utf8_lossy(&ambiguous.stderr).contains("multiple captured revisions"),
        "stderr:\n{}",
        String::from_utf8_lossy(&ambiguous.stderr)
    );

    let json = parse_json(
        &shore([
            "review",
            "show",
            "--repo",
            repo.path().to_str().unwrap(),
            "--revision",
            range["revision"]["id"].as_str().unwrap(),
        ])
        .stdout,
    );
    assert_eq!(json["revision"]["id"], range["revision"]["id"]);
    assert_eq!(json["revision"]["source"]["kind"], "git_commit_range");
}

#[test]
fn unit_show_renders_verification_status_on_members_and_capture() {
    let home = tempfile::tempdir().unwrap();
    let env_home = home.path().to_str().unwrap();
    let env: [(&str, &str); 1] = [("SHORE_HOME", env_home)];
    // A present-but-unenrolled key → signs, verifies untrusted_key under the empty trust set.
    assert!(
        shore_env(["keys", "init", "--name", "default"], &env)
            .status
            .success()
    );

    let repo = modified_repo();
    let repo_arg = repo.path().to_str().unwrap();
    assert!(
        shore_env(["review", "capture", "--repo", repo_arg], &env)
            .status
            .success()
    );
    assert!(
        shore_env(
            [
                "review",
                "observation",
                "add",
                "--repo",
                repo_arg,
                "--track",
                "agent:codex",
                "--title",
                "t",
                "--body",
                "b",
            ],
            &env,
        )
        .status
        .success()
    );

    let out = shore_env(["review", "show", "--repo", repo_arg], &env);
    assert!(
        out.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    // The capture identity (the captured event) carries the status.
    assert_eq!(doc["revision"]["verificationStatus"], "untrusted_key");
    // Each narrative member carries the status of its own event.
    assert_eq!(
        doc["observations"][0]["verificationStatus"],
        "untrusted_key"
    );
}

#[test]
fn unit_show_renders_endorsement_on_capture_identity() {
    let home = tempfile::tempdir().unwrap();
    let env_home = home.path().to_str().unwrap();
    let env: [(&str, &str); 1] = [("SHORE_HOME", env_home)];
    assert!(
        shore_env(["keys", "init", "--name", "default"], &env)
            .status
            .success()
    );
    let repo = modified_repo();
    let repo_arg = repo.path().to_str().unwrap();
    // Enroll the default key under kevin + attest kind/roles (reader config).
    assert!(
        shore_env(
            [
                "keys",
                "enroll",
                "default",
                "--actor",
                "actor:git-email:kevin@swiber.dev",
                "--repo",
                repo_arg,
            ],
            &env,
        )
        .status
        .success()
    );
    assert!(
        shore_env(
            [
                "identity",
                "attest",
                "actor:git-email:kevin@swiber.dev",
                "--kind",
                "human",
                "--role",
                "reviewer",
                "--repo",
                repo_arg,
            ],
            &[],
        )
        .status
        .success()
    );
    // Capture UNSIGNED so the detached endorsement carrier is not deduped.
    assert!(
        shore_env(
            ["review", "capture", "--repo", repo_arg],
            &[("SHORE_HOME", env_home), ("SHORE_SIGNING", "off")],
        )
        .status
        .success()
    );
    let target = captured_event_id(repo.path());
    assert!(
        shore_env(
            ["review", "endorse", &target, "--repo", repo_arg],
            &[
                ("SHORE_HOME", env_home),
                ("SHORE_ACTOR_ID", "actor:git-email:kevin@swiber.dev"),
            ],
        )
        .status
        .success()
    );

    let out = shore_env(
        ["review", "show", "--repo", repo_arg],
        &[("SHORE_HOME", env_home)],
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    let endorsement = &doc["revision"]["endorsements"][0];
    assert_eq!(endorsement["classification"], "endorsement-trusted");
    assert_eq!(endorsement["endorser"], "actor:git-email:kevin@swiber.dev");
    assert_eq!(endorsement["endorserAttributes"]["kind"], "human");
}

/// Find the captured Revision event id via the public read path (`read_events`).
fn captured_event_id(repo_path: &std::path::Path) -> String {
    shoreline::session::read_events(repo_path)
        .unwrap()
        .iter()
        .find(|e| e.event_type == shoreline::session::event::EventType::WorkObjectProposed)
        .expect("a captured review unit event")
        .event_id
        .as_str()
        .to_owned()
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

fn parse_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("valid json")
}
