mod support;

use std::ffi::OsStr;
use std::io::Write;
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore;

#[test]
fn disposition_add_records_review_unit_disposition_and_emits_v1_json() {
    let repo = modified_repo();
    let capture =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);

    let output = shore([
        "review",
        "disposition",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--disposition",
        "accepted",
        "--summary",
        "Ship this",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    assert_eq!(json["schema"], "shore.review-disposition-add");
    assert_eq!(json["version"], 1);
    assert_eq!(json["reviewUnitId"], capture["reviewUnit"]["id"]);
    assert!(
        json["dispositionId"]
            .as_str()
            .unwrap()
            .starts_with("disp:sha256:")
    );
    assert!(json["eventId"].as_str().unwrap().starts_with("evt:sha256:"));
    assert_eq!(json["trackId"], "human:kevin");
    assert_eq!(json["target"]["kind"], "review_unit");
    assert_eq!(json["disposition"], "accepted");
    assert!(
        json["summaryContentHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert_eq!(json["eventsCreated"], 1);
    assert_eq!(json["eventsExisting"], 0);
    assert_eq!(
        json["eventsCreatedByType"]["review_disposition_recorded"],
        1
    );
    assert!(json.get("statePath").is_none());
    assert!(json.get("summaryArtifactPath").is_none());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("artifacts/notes/"));
}

#[test]
fn disposition_add_records_file_range_disposition() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "disposition",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--disposition",
        "needs-changes",
        "--summary",
        "Fix line one",
        "--file",
        "src/lib.rs",
        "--side",
        "new",
        "--start-line",
        "1",
        "--end-line",
        "1",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    assert_eq!(json["target"]["kind"], "range");
    assert_eq!(json["target"]["filePath"], "src/lib.rs");
    assert_eq!(json["target"]["side"], "new");
    assert_eq!(json["target"]["startLine"], 1);
    assert_eq!(json["target"]["endLine"], 1);
}

#[test]
fn disposition_add_records_related_observations_and_interventions() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let observation = add_observation(&repo, "Related observation");
    let intervention = request_intervention(&repo, "Related intervention");

    let disposition = add_disposition([
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--disposition",
        "accepted-with-follow-up",
        "--summary",
        "Accept with follow-up",
        "--related-observation",
        observation["observationId"].as_str().unwrap(),
        "--related-intervention",
        intervention["interventionId"].as_str().unwrap(),
    ]);
    let show = parse_json(
        &shore([
            "review",
            "disposition",
            "show",
            "--repo",
            repo.path().to_str().unwrap(),
            "--all",
        ])
        .stdout,
    );

    assert_eq!(disposition["disposition"], "accepted_with_follow_up");
    assert_eq!(
        show["dispositions"][0]["relatedObservations"][0],
        observation["observationId"]
    );
    assert_eq!(
        show["dispositions"][0]["relatedInterventions"][0],
        intervention["interventionId"]
    );
}

#[test]
fn disposition_add_records_replacement() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let first = add_disposition([
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--disposition",
        "needs-changes",
        "--summary",
        "Fix this",
    ]);
    add_disposition([
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--disposition",
        "accepted",
        "--summary",
        "Fixed",
        "--replaces",
        first["dispositionId"].as_str().unwrap(),
    ]);

    let show = parse_json(
        &shore([
            "review",
            "disposition",
            "show",
            "--repo",
            repo.path().to_str().unwrap(),
            "--all",
        ])
        .stdout,
    );

    assert!(
        show["dispositions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|view| view["id"] == first["dispositionId"] && view["status"] == "replaced")
    );
}

#[test]
fn disposition_add_summary_stdin_reads_from_stdin() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore_with_stdin(
        [
            "review",
            "disposition",
            "add",
            "--repo",
            repo.path().to_str().unwrap(),
            "--track",
            "human:kevin",
            "--disposition",
            "accepted",
            "--summary-stdin",
        ],
        "summary from stdin",
    );
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let show = parse_json(
        &shore([
            "review",
            "disposition",
            "show",
            "--repo",
            repo.path().to_str().unwrap(),
            "--include-summary",
        ])
        .stdout,
    );
    assert_eq!(show["dispositions"][0]["summary"], "summary from stdin");
}

#[test]
fn disposition_show_reports_none_when_empty() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "disposition",
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

    assert_eq!(json["schema"], "shore.review-disposition-show");
    assert_eq!(json["version"], 1);
    assert_eq!(json["current"]["status"], "none");
    assert!(json["current"].get("dispositionId").is_none());
    assert!(json["dispositions"].as_array().unwrap().is_empty());
}

#[test]
fn disposition_show_reports_current_resolved_disposition() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let disposition = add_disposition([
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--disposition",
        "accepted",
        "--summary",
        "Ship it",
    ]);

    let output = shore([
        "review",
        "disposition",
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);
    let json = parse_json(&output.stdout);

    assert_eq!(json["current"]["status"], "resolved");
    assert_eq!(
        json["current"]["dispositionId"],
        disposition["dispositionId"]
    );
    assert_eq!(json["current"]["disposition"], "accepted");
    assert_eq!(json["dispositions"].as_array().unwrap().len(), 1);
}

#[test]
fn disposition_show_reports_ambiguous_candidates() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    add_disposition([
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--disposition",
        "accepted",
        "--summary",
        "Ship it",
    ]);
    add_disposition([
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--disposition",
        "needs-changes",
        "--summary",
        "Needs one fix",
    ]);

    let json = parse_json(
        &shore([
            "review",
            "disposition",
            "show",
            "--repo",
            repo.path().to_str().unwrap(),
        ])
        .stdout,
    );

    assert_eq!(json["current"]["status"], "ambiguous");
    assert_eq!(json["current"]["candidates"].as_array().unwrap().len(), 2);
}

#[test]
fn disposition_show_filters_by_track() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let human = add_disposition([
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--disposition",
        "accepted",
        "--summary",
        "Ship it",
    ]);
    add_disposition([
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--disposition",
        "needs-changes",
        "--summary",
        "Needs one fix",
    ]);

    let json = parse_json(
        &shore([
            "review",
            "disposition",
            "show",
            "--repo",
            repo.path().to_str().unwrap(),
            "--track",
            "human:kevin",
        ])
        .stdout,
    );

    assert_eq!(json["filters"]["trackId"], "human:kevin");
    assert_eq!(json["current"]["status"], "resolved");
    assert_eq!(json["dispositions"][0]["id"], human["dispositionId"]);
}

#[test]
fn disposition_show_include_summary_hydrates_summary() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    add_disposition([
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--disposition",
        "accepted",
        "--summary",
        "Ship it",
    ]);

    let without = parse_json(
        &shore([
            "review",
            "disposition",
            "show",
            "--repo",
            repo.path().to_str().unwrap(),
        ])
        .stdout,
    );
    let with = parse_json(
        &shore([
            "review",
            "disposition",
            "show",
            "--repo",
            repo.path().to_str().unwrap(),
            "--include-summary",
        ])
        .stdout,
    );

    assert!(without["dispositions"][0].get("summary").is_none());
    assert_eq!(with["dispositions"][0]["summary"], "Ship it");
}

#[test]
fn disposition_show_pretty_prints_when_requested() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "disposition",
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
        "--pretty",
    ]);

    assert!(String::from_utf8_lossy(&output.stdout).starts_with("{\n"));
}

#[test]
fn disposition_add_requires_track() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "disposition",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--disposition",
        "accepted",
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--track"));
}

#[test]
fn disposition_add_rejects_invalid_status() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "disposition",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--disposition",
        "not-a-disposition",
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid value"));
}

#[test]
fn disposition_add_rejects_conflicting_target_selectors() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let observation = add_observation(&repo, "Target conflict");

    let output = shore([
        "review",
        "disposition",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--disposition",
        "accepted",
        "--observation",
        observation["observationId"].as_str().unwrap(),
        "--file",
        "src/lib.rs",
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("target cannot be combined"));
}

#[test]
fn disposition_add_rejects_side_without_file() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "disposition",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--disposition",
        "accepted",
        "--summary",
        "Ship it",
        "--side",
        "old",
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("side requires file"));
}

#[test]
fn disposition_add_rejects_unknown_replacement_id() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "disposition",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--disposition",
        "accepted",
        "--summary",
        "Ship it",
        "--replaces",
        "disp:sha256:missing",
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown disposition"));
}

#[test]
fn overridden_requires_summary_and_override_reference() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let missing_summary = shore([
        "review",
        "disposition",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--disposition",
        "overridden",
        "--overrides-disposition",
        "disp:sha256:missing",
    ]);
    let missing_override = shore([
        "review",
        "disposition",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--disposition",
        "overridden",
        "--summary",
        "Manual override",
    ]);

    assert!(!missing_summary.status.success());
    assert!(String::from_utf8_lossy(&missing_summary.stderr).contains("summary is required"));
    assert!(!missing_override.status.success());
    assert!(
        String::from_utf8_lossy(&missing_override.stderr)
            .contains("override reference is required")
    );
}

#[test]
fn superseded_status_does_not_require_replaces() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "disposition",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--disposition",
        "superseded",
        "--summary",
        "Superseded by external work",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn duplicate_disposition_semantic_events_show_once_with_diagnostic() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let first = add_disposition([
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--disposition",
        "accepted",
        "--summary",
        "same summary",
        "--idempotency-key",
        "retry-a",
    ]);
    let second = add_disposition([
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--disposition",
        "accepted",
        "--summary",
        "same summary",
        "--idempotency-key",
        "retry-b",
    ]);

    let show = parse_json(
        &shore([
            "review",
            "disposition",
            "show",
            "--repo",
            repo.path().to_str().unwrap(),
            "--include-summary",
        ])
        .stdout,
    );
    let diagnostic = diagnostic_with_code(&show, "duplicate_semantic_disposition_event");

    assert_eq!(first["dispositionId"], second["dispositionId"]);
    assert_eq!(show["dispositions"].as_array().unwrap().len(), 1);
    assert_eq!(show["dispositions"][0]["id"], first["dispositionId"]);
    assert_eq!(show["dispositions"][0]["summary"], "same summary");
    assert!(
        diagnostic["message"]
            .as_str()
            .unwrap()
            .contains(first["dispositionId"].as_str().unwrap())
    );
}

fn add_disposition<const N: usize>(args: [&str; N]) -> Value {
    let mut command = vec!["review", "disposition", "add"];
    command.extend(args);
    let output = shore(command);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    parse_json(&output.stdout)
}

fn add_observation(repo: &GitRepo, title: &str) -> Value {
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
            title,
        ])
        .stdout,
    )
}

fn request_intervention(repo: &GitRepo, title: &str) -> Value {
    parse_json(
        &shore([
            "review",
            "intervention",
            "request",
            "--repo",
            repo.path().to_str().unwrap(),
            "--track",
            "human:kevin",
            "--title",
            title,
            "--reason",
            "manual-decision-required",
        ])
        .stdout,
    )
}

fn modified_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 {\n    1\n}\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 {\n    2\n}\n");
    repo
}

fn parse_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes)
        .unwrap_or_else(|error| panic!("parse json: {error}\n{}", String::from_utf8_lossy(bytes)))
}

fn diagnostic_with_code<'a>(json: &'a Value, code: &str) -> &'a Value {
    json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == code)
        .unwrap_or_else(|| panic!("missing diagnostic {code}: {json:#}"))
}

fn shore_with_stdin<I, S>(args: I, stdin: &str) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut child = Command::new(env!("CARGO_BIN_EXE_shore"))
        .args(args)
        .env_remove("SHORE_LOG")
        .env_remove("RUST_LOG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn shore binary");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().expect("wait for shore binary")
}
