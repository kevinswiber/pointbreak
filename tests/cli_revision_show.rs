mod support;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::{shore, shore_env};

#[test]
fn revision_help_lists_show() {
    let output = shore(["revision", "--help"]);

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("show"));
}

#[test]
fn revision_show_positional_accepts_and_omits() {
    let repo = modified_repo();
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);
    let path = repo.path().to_str().unwrap();

    // Omitted: shows the current capture (the flag-absent behavior).
    let current = shore(["revision", "show", "--repo", path]);
    assert!(
        current.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&current.stderr)
    );
    let json = parse_json(&current.stdout);
    assert_eq!(json["schema"], "pointbreak.review-revision");
    let id = json["revision"]["id"].as_str().unwrap().to_owned();

    // Positional full id: selects that revision.
    let selected = shore(["revision", "show", &id, "--repo", path]);
    assert!(
        selected.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    assert_eq!(parse_json(&selected.stdout)["revision"]["id"], id);
}

#[test]
fn revision_show_positional_resolves_a_prefixed_short_id() {
    let repo = modified_repo();
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);
    let path = repo.path().to_str().unwrap();

    let current = parse_json(&shore(["revision", "show", "--repo", path]).stdout);
    let id = current["revision"]["id"].as_str().unwrap().to_owned();
    // id = "rev:…sha256:<hex>"; the bare-prefixed short form resolves it.
    let digest = id.rsplit_once("sha256:").unwrap().1;
    let prefixed_short = format!("rev:{}", &digest[..8]);

    let selected = shore(["revision", "show", &prefixed_short, "--repo", path]);
    assert!(
        selected.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    assert_eq!(parse_json(&selected.stdout)["revision"]["id"], id);
}

#[test]
fn revision_show_emits_v2_json() {
    let repo = modified_repo();
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore(["revision", "show", "--repo", repo.path().to_str().unwrap()]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);

    assert_eq!(json["schema"], "pointbreak.review-revision");
    assert_eq!(json["version"], 2);
    assert!(json.get("adapterNotes").is_none());
    assert!(json["summary"].get("adapterNoteCount").is_none());
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
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "revision",
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
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "revision",
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
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "revision",
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
    let first = parse_json(&shore(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    let second = parse_json(&shore(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);

    let ambiguous = shore(["revision", "show", "--repo", repo.path().to_str().unwrap()]);
    assert!(!ambiguous.status.success());
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("multiple captured revisions"));

    let explicit = shore([
        "revision",
        "show",
        first["revision"]["id"].as_str().unwrap(),
        "--repo",
        repo.path().to_str().unwrap(),
    ]);
    let json = parse_json(&explicit.stdout);

    assert_ne!(first["revision"]["id"], second["revision"]["id"]);
    assert_eq!(json["revision"]["id"], first["revision"]["id"]);
}

#[test]
fn revision_show_include_body_hydrates_without_internal_paths() {
    let repo = modified_repo();
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);
    add_observation_with_body(&repo, "agent:codex", "Body", "visible body");

    let output = shore([
        "revision",
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
    assert!(!stdout.contains("artifacts/objects/"));
    assert!(!stdout.contains(".shore/data/events"));
    assert!(json.get("statePath").is_none());
    assert!(json.get("snapshotArtifactPath").is_none());
}

#[test]
fn revision_show_includes_input_requests_and_omits_legacy_fields() {
    let repo = modified_repo();
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = add_input_request_with_body(&repo, "visible request body");
    respond_to_input_request(
        &repo,
        requested["inputRequestId"].as_str().unwrap(),
        "approved",
    );

    let output = shore([
        "revision",
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
    assert!(!stdout.contains("artifacts/objects/"));
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
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);
    add_observation(&repo, "agent:codex", "Narrative");

    let json =
        parse_json(&shore(["revision", "show", "--repo", repo.path().to_str().unwrap()]).stdout);

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
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);
    add_observation(&repo, "agent:codex", "Codex");
    add_observation(&repo, "agent:claude", "Claude");

    let all =
        parse_json(&shore(["revision", "show", "--repo", repo.path().to_str().unwrap()]).stdout);
    let codex = parse_json(
        &shore([
            "revision",
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
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);
    add_assessment(&repo);

    let json =
        parse_json(&shore(["revision", "show", "--repo", repo.path().to_str().unwrap()]).stdout);

    assert_eq!(json["currentAssessment"]["status"], "resolved");
    assert_eq!(json["currentAssessment"]["assessment"], "accepted");
    assert_eq!(json["assessments"].as_array().unwrap().len(), 1);
}

#[test]
fn unit_show_projects_range_capture_with_bound_snapshot() {
    let repo = support::committed_repo();
    let capture = parse_json(
        &shore([
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
        "revision",
        "show",
        revision_id,
        "--repo",
        repo.path().to_str().unwrap(),
    ]);

    // The command succeeding proves load_bound_object_artifact validated the
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
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);
    let range = parse_json(
        &shore([
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--base",
            "HEAD~1",
        ])
        .stdout,
    );

    let ambiguous = shore(["revision", "show", "--repo", repo.path().to_str().unwrap()]);
    assert!(!ambiguous.status.success());
    assert!(
        String::from_utf8_lossy(&ambiguous.stderr).contains("multiple captured revisions"),
        "stderr:\n{}",
        String::from_utf8_lossy(&ambiguous.stderr)
    );

    let json = parse_json(
        &shore([
            "revision",
            "show",
            range["revision"]["id"].as_str().unwrap(),
            "--repo",
            repo.path().to_str().unwrap(),
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
        shore_env(["key", "init", "--name", "default"], &env)
            .status
            .success()
    );

    let repo = modified_repo();
    let repo_arg = repo.path().to_str().unwrap();
    assert!(
        shore_env(["capture", "--repo", repo_arg], &env)
            .status
            .success()
    );
    assert!(
        shore_env(
            [
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

    let out = shore_env(["revision", "show", "--repo", repo_arg], &env);
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
        shore_env(["key", "init", "--name", "default"], &env)
            .status
            .success()
    );
    let repo = modified_repo();
    let repo_arg = repo.path().to_str().unwrap();
    // Enroll the default key under kevin + attest kind/roles (reader config).
    assert!(
        shore_env(
            [
                "key",
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
            ["capture", "--repo", repo_arg],
            &[("SHORE_HOME", env_home), ("SHORE_SIGNING", "off")],
        )
        .status
        .success()
    );
    let target = captured_event_id(repo.path());
    assert!(
        shore_env(
            ["endorse", &target, "--repo", repo_arg],
            &[
                ("SHORE_HOME", env_home),
                ("SHORE_ACTOR_ID", "actor:git-email:kevin@swiber.dev"),
            ],
        )
        .status
        .success()
    );

    let out = shore_env(
        ["revision", "show", "--repo", repo_arg],
        &[("SHORE_HOME", env_home)],
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    let endorsement = &doc["revision"]["endorsements"][0];
    assert_eq!(endorsement["classification"], "endorsement-trusted");
    assert_eq!(endorsement["endorser"], "actor:git-email:kevin@swiber.dev");
    assert_eq!(endorsement["endorserAttributes"]["kind"], "human");
}

#[test]
fn text_digest_is_bounded_and_never_renders_rows() {
    let repo = modified_repo();
    let repo_arg = repo.path().to_str().unwrap();
    shore(["capture", "--repo", repo_arg]);
    add_observation(&repo, "agent:codex", "Narrative");
    add_input_request_with_body(&repo, "please decide");
    add_assessment(&repo);

    let output = shore(["revision", "show", "--repo", repo_arg, "--format", "text"]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("current call"), "stdout:\n{stdout}");
    assert!(stdout.contains("rev:"), "stdout:\n{stdout}");
    assert!(stdout.contains("observation"), "stdout:\n{stdout}");
    assert!(!stdout.contains("file_header"), "stdout:\n{stdout}");
    assert!(!stdout.contains("\"rows\""), "stdout:\n{stdout}");
    assert!(
        stdout.lines().count() < 40,
        "digest must be bounded, got {} lines:\n{stdout}",
        stdout.lines().count()
    );
}

#[test]
fn text_digest_reports_signed_by_enrolled_key() {
    const ENROLLED: &str = "actor:git-email:kevin@swiber.dev";

    // Enrolled key signs the assessment → the current call verifies valid.
    let yes_home = tempfile::tempdir().unwrap();
    let yes_home_s = yes_home.path().to_str().unwrap();
    let yes_env: [(&str, &str); 1] = [("SHORE_HOME", yes_home_s)];
    assert!(
        shore_env(["key", "init", "--name", "default"], &yes_env)
            .status
            .success()
    );
    let yes_repo = modified_repo();
    let yes_repo_arg = yes_repo.path().to_str().unwrap();
    assert!(
        shore_env(
            [
                "key",
                "enroll",
                "default",
                "--actor",
                ENROLLED,
                "--repo",
                yes_repo_arg,
            ],
            &yes_env,
        )
        .status
        .success()
    );
    assert!(
        shore_env(["capture", "--repo", yes_repo_arg], &yes_env)
            .status
            .success()
    );
    assert!(
        shore_env(
            [
                "assessment",
                "add",
                "--repo",
                yes_repo_arg,
                "--track",
                "human:kevin",
                "--assessment",
                "accepted",
                "--summary",
                "ship it",
            ],
            &[("SHORE_HOME", yes_home_s), ("SHORE_ACTOR_ID", ENROLLED)],
        )
        .status
        .success()
    );
    let yes_out = shore_env(
        [
            "revision",
            "show",
            "--repo",
            yes_repo_arg,
            "--format",
            "text",
        ],
        &yes_env,
    );
    let yes_stdout = String::from_utf8_lossy(&yes_out.stdout);
    assert!(
        yes_stdout.contains("signed by enrolled key: yes"),
        "stdout:\n{yes_stdout}"
    );

    // Unsigned assessment → not signed by an enrolled key.
    let no_home = tempfile::tempdir().unwrap();
    let no_home_s = no_home.path().to_str().unwrap();
    let no_repo = modified_repo();
    let no_repo_arg = no_repo.path().to_str().unwrap();
    assert!(
        shore_env(
            ["capture", "--repo", no_repo_arg],
            &[("SHORE_HOME", no_home_s), ("SHORE_SIGNING", "off")],
        )
        .status
        .success()
    );
    assert!(
        shore_env(
            [
                "assessment",
                "add",
                "--repo",
                no_repo_arg,
                "--track",
                "human:kevin",
                "--assessment",
                "accepted",
                "--summary",
                "ship it",
            ],
            &[("SHORE_HOME", no_home_s), ("SHORE_SIGNING", "off")],
        )
        .status
        .success()
    );
    let no_out = shore_env(
        [
            "revision",
            "show",
            "--repo",
            no_repo_arg,
            "--format",
            "text",
        ],
        &[("SHORE_HOME", no_home_s)],
    );
    let no_stdout = String::from_utf8_lossy(&no_out.stdout);
    assert!(
        no_stdout.contains("signed by enrolled key: no"),
        "stdout:\n{no_stdout}"
    );
}

#[test]
fn text_digest_clamps_long_open_request_titles() {
    let repo = modified_repo();
    let repo_arg = repo.path().to_str().unwrap();
    shore(["capture", "--repo", repo_arg]);
    let long_title = "x".repeat(320);
    shore([
        "input-request",
        "open",
        "--repo",
        repo_arg,
        "--track",
        "agent:codex",
        "--title",
        &long_title,
        "--reason",
        "manual-decision-required",
    ]);

    let output = shore(["revision", "show", "--repo", repo_arg, "--format", "text"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let longest = stdout
        .lines()
        .map(|line| line.chars().count())
        .max()
        .unwrap();

    assert!(stdout.contains("open input requests"), "stdout:\n{stdout}");
    // The one user-controlled string in the digest must not blow the line bound.
    assert!(
        !stdout.contains(&long_title),
        "the full 320-char title must be clamped:\n{stdout}"
    );
    assert!(
        longest < 150,
        "digest lines must stay bounded, longest was {longest}:\n{stdout}"
    );
}

#[test]
fn text_digest_groups_fact_counts_by_track() {
    let repo = multi_file_repo();
    let repo_arg = repo.path().to_str().unwrap();
    shore(["capture", "--repo", repo_arg]);
    add_observation(&repo, "agent:codex", "Codex finding");
    add_observation(&repo, "agent:claude", "Claude finding");

    let output = shore(["revision", "show", "--repo", repo_arg, "--format", "text"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("tracks:"), "stdout:\n{stdout}");
    assert!(stdout.contains("agent:codex"), "stdout:\n{stdout}");
    assert!(stdout.contains("agent:claude"), "stdout:\n{stdout}");
}

/// Find the captured Revision event id via the public read path (`read_events`).
fn captured_event_id(repo_path: &std::path::Path) -> String {
    pointbreak::session::read_events(repo_path)
        .unwrap()
        .iter()
        .find(|e| e.event_type == pointbreak::session::event::EventType::WorkObjectProposed)
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

fn parse_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("valid json")
}
