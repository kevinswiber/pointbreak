mod support;

use std::ffi::OsStr;
use std::io::Write;
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore;

#[test]
fn input_request_open_defaults_to_operative_mode_and_emits_v1_json() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "input-request",
        "open",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--title",
        "Need approval",
        "--reason",
        "manual-decision-required",
        "--body",
        "approve this path?",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);

    assert_eq!(json["schema"], "shore.review-input-request-open");
    assert_eq!(json["version"], 1);
    assert!(
        json["inputRequestId"]
            .as_str()
            .unwrap()
            .starts_with("input-request:sha256:")
    );
    assert_eq!(json["trackId"], "human:kevin");
    assert_eq!(json["mode"], "operative");
    assert_eq!(json["reasonCode"], "manual_decision_required");
    assert_eq!(json["eventsCreatedByType"]["input_request_opened"], 1);
    assert!(json.get("bodyArtifactPath").is_none());
    assert_has_no_legacy_public_input_request_shape(&json);
    assert!(
        support::common_dir_store(repo.path())
            .join("state.json")
            .is_file()
    );
}

#[test]
fn input_request_open_accepts_advisory_and_rejects_blocking() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let advisory = shore([
        "review",
        "input-request",
        "open",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--title",
        "FYI",
        "--reason",
        "manual-decision-required",
        "--mode",
        "advisory",
    ]);
    assert!(
        advisory.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&advisory.stderr)
    );
    let json = parse_json(&advisory.stdout);
    assert_eq!(json["mode"], "advisory");

    let blocking = shore([
        "review",
        "input-request",
        "open",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--title",
        "Old mode",
        "--reason",
        "manual-decision-required",
        "--mode",
        "blocking",
    ]);
    assert!(!blocking.status.success());
    assert!(String::from_utf8_lossy(&blocking.stderr).contains("invalid value"));
}

#[test]
fn input_request_list_emits_v1_json_and_pretty_prints() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    request(&repo, "First");

    let output = shore([
        "review",
        "input-request",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
        "--pretty",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("{\n"));
    let json = parse_json(&output.stdout);

    assert_eq!(json["schema"], "shore.review-input-request-list");
    assert_eq!(json["version"], 1);
    assert_eq!(json["filters"]["status"], "open");
    assert_eq!(json["inputRequests"].as_array().unwrap().len(), 1);
    assert_eq!(json["inputRequests"][0]["title"], "First");
    assert_eq!(json["inputRequests"][0]["mode"], "operative");
    assert_eq!(json["inputRequests"][0]["status"], "open");
    assert_has_no_legacy_public_input_request_shape(&json);
}

#[test]
fn input_request_list_accepts_operative_and_advisory_mode_filters() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    request(&repo, "Default");
    shore([
        "review",
        "input-request",
        "open",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--title",
        "Advisory",
        "--reason",
        "manual-decision-required",
        "--mode",
        "advisory",
    ]);

    let operative = shore([
        "review",
        "input-request",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
        "--mode",
        "operative",
    ]);
    assert!(
        operative.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&operative.stderr)
    );
    let operative_json = parse_json(&operative.stdout);
    assert_eq!(operative_json["filters"]["mode"], "operative");
    assert_eq!(operative_json["inputRequests"][0]["title"], "Default");
    assert_eq!(operative_json["inputRequests"][0]["mode"], "operative");

    let advisory = shore([
        "review",
        "input-request",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
        "--mode",
        "advisory",
    ]);
    assert!(
        advisory.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&advisory.stderr)
    );
    let advisory_json = parse_json(&advisory.stdout);
    assert_eq!(advisory_json["filters"]["mode"], "advisory");
    assert_eq!(advisory_json["inputRequests"][0]["title"], "Advisory");
    assert_eq!(advisory_json["inputRequests"][0]["mode"], "advisory");

    let blocking = shore([
        "review",
        "input-request",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
        "--mode",
        "blocking",
    ]);
    assert!(!blocking.status.success());
    assert!(String::from_utf8_lossy(&blocking.stderr).contains("invalid value"));
}

#[test]
fn input_request_list_collapses_duplicate_request_events() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let first = parse_json(
        &shore([
            "review",
            "input-request",
            "open",
            "--repo",
            repo.path().to_str().unwrap(),
            "--track",
            "human:kevin",
            "--title",
            "Need approval",
            "--reason",
            "manual-decision-required",
            "--body",
            "same body",
            "--idempotency-key",
            "retry-a",
        ])
        .stdout,
    );
    let second = parse_json(
        &shore([
            "review",
            "input-request",
            "open",
            "--repo",
            repo.path().to_str().unwrap(),
            "--track",
            "human:kevin",
            "--title",
            "Need approval",
            "--reason",
            "manual-decision-required",
            "--body",
            "same body",
            "--idempotency-key",
            "retry-b",
        ])
        .stdout,
    );

    let list = shore([
        "review",
        "input-request",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
        "--status",
        "all",
        "--include-body",
    ]);
    let json = parse_json(&list.stdout);
    let diagnostic = diagnostic_with_code(&json, "duplicate_semantic_input_request_open_event");
    let input_request_id = first["inputRequestId"].as_str().unwrap();

    assert_eq!(first["inputRequestId"], second["inputRequestId"]);
    assert_eq!(json["inputRequests"].as_array().unwrap().len(), 1);
    assert_eq!(json["inputRequests"][0]["id"], first["inputRequestId"]);
    assert_eq!(json["inputRequests"][0]["body"], "same body");
    assert!(
        diagnostic["message"]
            .as_str()
            .unwrap()
            .contains(input_request_id)
    );
}

#[test]
fn input_request_fetch_include_body_emits_v1_json_and_hydrates_body() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request_with_body(&repo, "Need details", "full request body");
    let input_request_id = requested["inputRequestId"].as_str().unwrap();

    let output = shore([
        "review",
        "input-request",
        "fetch",
        input_request_id,
        "--repo",
        repo.path().to_str().unwrap(),
        "--include-body",
        "--pretty",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("{\n"));
    let json = parse_json(&output.stdout);

    assert_eq!(json["schema"], "shore.review-input-request-fetch");
    assert_eq!(json["version"], 1);
    assert_eq!(json["inputRequest"]["id"], input_request_id);
    assert_eq!(json["inputRequest"]["mode"], "operative");
    assert_eq!(json["inputRequest"]["body"], "full request body");
    assert_has_no_legacy_public_input_request_shape(&json);
}

#[test]
fn input_request_respond_records_response_and_emits_v1_json() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request(&repo, "Need approval");
    let input_request_id = requested["inputRequestId"].as_str().unwrap();

    let output = shore([
        "review",
        "input-request",
        "respond",
        input_request_id,
        "--repo",
        repo.path().to_str().unwrap(),
        "--outcome",
        "approved",
        "--reason",
        "approved locally",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);

    assert_eq!(json["schema"], "shore.review-input-request-respond");
    assert_eq!(json["version"], 1);
    assert_eq!(json["inputRequestId"], input_request_id);
    assert!(
        json["inputRequestResponseId"]
            .as_str()
            .unwrap()
            .starts_with("input-request-response:sha256:")
    );
    assert_eq!(json["outcome"], "approved");
    assert_eq!(json["eventsCreatedByType"]["input_request_responded"], 1);
    assert!(json.get("reasonArtifactPath").is_none());
    assert_has_no_legacy_public_input_request_shape(&json);
}

#[test]
fn input_request_fetch_collapses_duplicate_response_events() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request(&repo, "Need approval");
    let input_request_id = requested["inputRequestId"].as_str().unwrap();

    let first = parse_json(
        &shore([
            "review",
            "input-request",
            "respond",
            input_request_id,
            "--repo",
            repo.path().to_str().unwrap(),
            "--outcome",
            "approved",
            "--reason",
            "approved locally",
            "--idempotency-key",
            "retry-a",
        ])
        .stdout,
    );
    let second = parse_json(
        &shore([
            "review",
            "input-request",
            "respond",
            input_request_id,
            "--repo",
            repo.path().to_str().unwrap(),
            "--outcome",
            "approved",
            "--reason",
            "approved locally",
            "--idempotency-key",
            "retry-b",
        ])
        .stdout,
    );

    let fetch = shore([
        "review",
        "input-request",
        "fetch",
        input_request_id,
        "--repo",
        repo.path().to_str().unwrap(),
    ]);
    let json = parse_json(&fetch.stdout);
    let diagnostic = diagnostic_with_code(&json, "duplicate_semantic_input_request_response_event");
    let response_id = first["inputRequestResponseId"].as_str().unwrap();

    assert_eq!(
        first["inputRequestResponseId"],
        second["inputRequestResponseId"]
    );
    assert_eq!(json["inputRequest"]["status"], "responded");
    assert_eq!(
        json["inputRequest"]["responses"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        json["inputRequest"]["responses"][0]["id"],
        first["inputRequestResponseId"]
    );
    assert!(
        diagnostic["message"]
            .as_str()
            .unwrap()
            .contains(response_id)
    );
    assert_has_no_legacy_public_input_request_shape(&json);
}

#[test]
fn input_request_list_filters_responded_requests() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request(&repo, "Need approval");
    let input_request_id = requested["inputRequestId"].as_str().unwrap();
    let response = shore([
        "review",
        "input-request",
        "respond",
        input_request_id,
        "--repo",
        repo.path().to_str().unwrap(),
        "--outcome",
        "approved",
    ]);
    assert!(
        response.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&response.stderr)
    );

    let output = shore([
        "review",
        "input-request",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
        "--status",
        "responded",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);

    assert_eq!(json["schema"], "shore.review-input-request-list");
    assert_eq!(json["filters"]["status"], "responded");
    assert_eq!(json["inputRequests"].as_array().unwrap().len(), 1);
    assert_eq!(json["inputRequests"][0]["id"], input_request_id);
}

#[test]
fn input_request_open_body_stdin_reads_from_stdin() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore_with_stdin(
        [
            "review",
            "input-request",
            "open",
            "--repo",
            repo.path().to_str().unwrap(),
            "--track",
            "human:kevin",
            "--title",
            "stdin body",
            "--reason",
            "manual-decision-required",
            "--body-stdin",
        ],
        "body from stdin",
    );
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requested = parse_json(&output.stdout);

    let fetch = shore([
        "review",
        "input-request",
        "fetch",
        requested["inputRequestId"].as_str().unwrap(),
        "--repo",
        repo.path().to_str().unwrap(),
        "--include-body",
    ]);
    let json = parse_json(&fetch.stdout);
    assert_eq!(json["inputRequest"]["body"], "body from stdin");
}

#[test]
fn input_request_respond_reason_stdin_reads_from_stdin() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request(&repo, "Need approval");
    let input_request_id = requested["inputRequestId"].as_str().unwrap();

    let output = shore_with_stdin(
        [
            "review",
            "input-request",
            "respond",
            input_request_id,
            "--repo",
            repo.path().to_str().unwrap(),
            "--outcome",
            "approved",
            "--reason-stdin",
        ],
        "reason from stdin",
    );
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    assert!(
        json["reasonContentHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
}

#[test]
fn input_request_open_requires_track() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "input-request",
        "open",
        "--repo",
        repo.path().to_str().unwrap(),
        "--title",
        "Need approval",
        "--reason",
        "manual-decision-required",
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--track"));
}

#[test]
fn input_request_open_rejects_invalid_reason_and_mode_values() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let bad_reason = shore([
        "review",
        "input-request",
        "open",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--title",
        "Need approval",
        "--reason",
        "not-a-reason",
    ]);
    let bad_mode = shore([
        "review",
        "input-request",
        "open",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--title",
        "Need approval",
        "--reason",
        "manual-decision-required",
        "--mode",
        "not-a-mode",
    ]);

    assert!(!bad_reason.status.success());
    assert!(String::from_utf8_lossy(&bad_reason.stderr).contains("invalid value"));
    assert!(!bad_mode.status.success());
    assert!(String::from_utf8_lossy(&bad_mode.stderr).contains("invalid value"));
}

#[test]
fn input_request_respond_rejects_invalid_outcome() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request(&repo, "Need approval");

    let output = shore([
        "review",
        "input-request",
        "respond",
        requested["inputRequestId"].as_str().unwrap(),
        "--repo",
        repo.path().to_str().unwrap(),
        "--outcome",
        "not-an-outcome",
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid value"));
}

#[test]
fn input_request_open_observation_target_conflicts_with_file_and_lines() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let observation = shore([
        "review",
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "Observation",
    ]);
    let observation_json = parse_json(&observation.stdout);

    let output = shore([
        "review",
        "input-request",
        "open",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--title",
        "Need approval",
        "--reason",
        "manual-decision-required",
        "--observation",
        observation_json["observationId"].as_str().unwrap(),
        "--file",
        "src/lib.rs",
        "--start-line",
        "1",
    ]);

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("observation target cannot be combined")
    );
}

#[test]
fn input_request_open_body_inputs_are_mutually_exclusive() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let body_file = repo.path().join("body.txt");
    std::fs::write(&body_file, "file body").unwrap();

    let output = shore([
        "review",
        "input-request",
        "open",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--title",
        "Need approval",
        "--reason",
        "manual-decision-required",
        "--body",
        "inline body",
        "--body-file",
        body_file.to_str().unwrap(),
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used"));
}

#[test]
fn input_request_respond_reason_inputs_are_mutually_exclusive() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request(&repo, "Need approval");
    let reason_file = repo.path().join("reason.txt");
    std::fs::write(&reason_file, "file reason").unwrap();

    let output = shore([
        "review",
        "input-request",
        "respond",
        requested["inputRequestId"].as_str().unwrap(),
        "--repo",
        repo.path().to_str().unwrap(),
        "--outcome",
        "approved",
        "--reason",
        "inline reason",
        "--reason-file",
        reason_file.to_str().unwrap(),
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used"));
}

#[test]
fn input_request_open_requires_review_unit_when_current_is_ambiguous() {
    let repo = modified_repo();
    let first =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    repo.write("another.txt", "new untracked file\n");
    let second =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    assert_ne!(first["reviewUnit"]["id"], second["reviewUnit"]["id"]);

    let ambiguous = shore([
        "review",
        "input-request",
        "open",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--title",
        "Need approval",
        "--reason",
        "manual-decision-required",
    ]);
    assert!(!ambiguous.status.success());
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("multiple captured review units"));

    let explicit = shore([
        "review",
        "input-request",
        "open",
        "--repo",
        repo.path().to_str().unwrap(),
        "--review-unit",
        first["reviewUnit"]["id"].as_str().unwrap(),
        "--track",
        "human:kevin",
        "--title",
        "Need approval",
        "--reason",
        "manual-decision-required",
    ]);
    assert!(
        explicit.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&explicit.stderr)
    );
    let json = parse_json(&explicit.stdout);
    assert_eq!(json["reviewUnitId"], first["reviewUnit"]["id"]);
}

#[test]
fn legacy_intervention_command_is_not_registered() {
    let output = shore(["review", "intervention", "list"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unrecognized subcommand"),
        "stderr:\n{stderr}"
    );
    assert!(stderr.contains("input-request"), "stderr:\n{stderr}");
}

#[test]
fn other_unknown_review_subcommands_keep_clap_suggestions() {
    let output = shore(["review", "observatin"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unrecognized subcommand"),
        "stderr:\n{stderr}"
    );
    assert!(stderr.contains("observation"), "stderr:\n{stderr}");
    assert!(stderr.contains("Usage: shore review"), "stderr:\n{stderr}");
}

fn request(repo: &GitRepo, title: &str) -> Value {
    request_with_body(repo, title, "")
}

fn request_with_body(repo: &GitRepo, title: &str, body: &str) -> Value {
    let mut args = vec![
        "review",
        "input-request",
        "open",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--title",
        title,
        "--reason",
        "manual-decision-required",
    ];
    if !body.is_empty() {
        args.extend(["--body", body]);
    }
    parse_json(&shore(args).stdout)
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
        .expect("shore stdin is piped")
        .write_all(stdin.as_bytes())
        .expect("write shore stdin");
    child.wait_with_output().expect("run shore binary")
}

fn parse_json(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).expect("stdout is valid JSON")
}

fn diagnostic_with_code<'a>(json: &'a Value, code: &str) -> &'a Value {
    json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == code)
        .unwrap_or_else(|| panic!("missing diagnostic code {code}: {json}"))
}

fn assert_has_no_legacy_public_input_request_shape(json: &Value) {
    let output = serde_json::to_string(json).unwrap();
    assert!(!output.contains("shore.review-intervention"));
    assert!(!output.contains("interventionId"));
    assert!(!output.contains("interventionResolutionId"));
    assert!(!output.contains("\"resolutions\""));
    assert!(!output.contains("resolution_id"));
    assert!(json.get("intervention").is_none());
    assert!(json.get("interventions").is_none());
}

fn modified_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo
}
