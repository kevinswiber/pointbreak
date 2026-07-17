mod support;

use std::ffi::OsStr;
use std::io::Write;
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use support::git_repo::GitRepo;
use support::pointbreak;

#[test]
fn input_request_open_runs_at_top_level() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let output = pointbreak([
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
    // INV-1: the wire schema is frozen — the surface rename does not touch it.
    assert_eq!(json["schema"], "pointbreak.review-input-request-open");
}

#[test]
fn input_request_show_reads_back_with_frozen_fetch_schema() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request_with_body(&repo, "Need details", "full request body");
    let id = requested["inputRequestId"].as_str().unwrap();
    let output = pointbreak([
        "input-request",
        "show",
        id,
        "--repo",
        repo.path().to_str().unwrap(),
        "--include-body",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    // INV-1: the read-one schema stays `…-fetch`; only the argv verb changed.
    assert_eq!(json["schema"], "pointbreak.review-input-request-show");
    assert_eq!(json["inputRequest"]["id"], id);
    assert_eq!(json["inputRequest"]["body"], "full request body");
}

#[test]
fn input_request_show_accepts_a_prefixed_short_id() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let full = request(&repo, "Need approval")["inputRequestId"]
        .as_str()
        .unwrap()
        .to_owned();
    // full = "input-request:sha256:<64hex>"; prefixed-short = "input-request:<first 8 hex>".
    let hex = full.rsplit(':').next().unwrap();
    let short = format!("input-request:{}", &hex[..8]);
    let output = pointbreak([
        "input-request",
        "show",
        &short,
        "--repo",
        repo.path().to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(parse_json(&output.stdout)["inputRequest"]["id"], full);
}

#[test]
fn open_accepts_insufficient_evidence_reason() {
    let repo = modified_repo();
    let repo_arg = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", repo_arg]);

    let opened = pointbreak([
        "input-request",
        "open",
        "--repo",
        repo_arg,
        "--track",
        "agent:reviewer",
        "--title",
        "Runtime trace required",
        "--reason",
        "insufficient-evidence",
        "--format",
        "json",
    ]);
    assert!(
        opened.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&opened.stderr)
    );
    let opened_json = parse_json(&opened.stdout);
    assert_eq!(opened_json["reasonCode"], "insufficient_evidence");

    let listed = pointbreak([
        "input-request",
        "list",
        "--repo",
        repo_arg,
        "--format",
        "json",
    ]);
    assert!(
        listed.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let listed_json = parse_json(&listed.stdout);
    assert_eq!(
        listed_json["inputRequests"][0]["reasonCode"],
        "insufficient_evidence"
    );
}

#[test]
fn input_request_open_defaults_to_operative_mode_and_emits_v1_json() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = pointbreak([
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

    assert_eq!(json["schema"], "pointbreak.review-input-request-open");
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let advisory = pointbreak([
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

    let blocking = pointbreak([
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
fn input_request_list_emits_v1_json_and_json_pretty_prints() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    request(&repo, "First");

    let output = pointbreak([
        "input-request",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
        "--format",
        "json-pretty",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("{\n"));
    let json = parse_json(&output.stdout);

    assert_eq!(json["schema"], "pointbreak.review-input-request-list");
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    request(&repo, "Default");
    pointbreak([
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

    let operative = pointbreak([
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

    let advisory = pointbreak([
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

    let blocking = pointbreak([
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let first = parse_json(
        &pointbreak([
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
        &pointbreak([
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

    let list = pointbreak([
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request_with_body(&repo, "Need details", "full request body");
    let input_request_id = requested["inputRequestId"].as_str().unwrap();

    let output = pointbreak([
        "input-request",
        "show",
        input_request_id,
        "--repo",
        repo.path().to_str().unwrap(),
        "--include-body",
        "--format",
        "json-pretty",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("{\n"));
    let json = parse_json(&output.stdout);

    assert_eq!(json["schema"], "pointbreak.review-input-request-show");
    assert_eq!(json["version"], 1);
    assert_eq!(json["inputRequest"]["id"], input_request_id);
    assert_eq!(json["inputRequest"]["mode"], "operative");
    assert_eq!(json["inputRequest"]["body"], "full request body");
    assert_has_no_legacy_public_input_request_shape(&json);
}

#[test]
fn input_request_respond_records_response_and_emits_v1_json() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request(&repo, "Need approval");
    let input_request_id = requested["inputRequestId"].as_str().unwrap();

    let output = pointbreak([
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

    assert_eq!(json["schema"], "pointbreak.review-input-request-respond");
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request(&repo, "Need approval");
    let input_request_id = requested["inputRequestId"].as_str().unwrap();

    let first = parse_json(
        &pointbreak([
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
        &pointbreak([
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

    let fetch = pointbreak([
        "input-request",
        "show",
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request(&repo, "Need approval");
    let input_request_id = requested["inputRequestId"].as_str().unwrap();
    let response = pointbreak([
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

    let output = pointbreak([
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

    assert_eq!(json["schema"], "pointbreak.review-input-request-list");
    assert_eq!(json["filters"]["status"], "responded");
    assert_eq!(json["inputRequests"].as_array().unwrap().len(), 1);
    assert_eq!(json["inputRequests"][0]["id"], input_request_id);
}

#[test]
fn input_request_open_body_stdin_reads_from_stdin() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore_with_stdin(
        [
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

    let fetch = pointbreak([
        "input-request",
        "show",
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request(&repo, "Need approval");
    let input_request_id = requested["inputRequestId"].as_str().unwrap();

    let output = shore_with_stdin(
        [
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = pointbreak([
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let bad_reason = pointbreak([
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
    let bad_mode = pointbreak([
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request(&repo, "Need approval");

    let output = pointbreak([
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let observation = pointbreak([
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

    let output = pointbreak([
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let body_file = repo.path().join("body.txt");
    std::fs::write(&body_file, "file body").unwrap();

    let output = pointbreak([
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request(&repo, "Need approval");
    let reason_file = repo.path().join("reason.txt");
    std::fs::write(&reason_file, "file reason").unwrap();

    let output = pointbreak([
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
fn input_request_open_requires_revision_when_current_is_ambiguous() {
    let repo = modified_repo();
    let first =
        parse_json(&pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    let second =
        parse_json(&pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    assert_ne!(first["revision"]["id"], second["revision"]["id"]);

    let ambiguous = pointbreak([
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
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("multiple captured revisions"));

    let explicit = pointbreak([
        "input-request",
        "open",
        "--repo",
        repo.path().to_str().unwrap(),
        "--revision",
        first["revision"]["id"].as_str().unwrap(),
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
    assert_eq!(json["revisionId"], first["revision"]["id"]);
}

#[test]
fn unknown_top_level_commands_keep_clap_suggestions() {
    // A typo of a top-level command still draws clap's did-you-mean suggestion;
    // the removed-command hint table never suppresses it.
    let output = pointbreak(["revison"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unrecognized subcommand"),
        "stderr:\n{stderr}"
    );
    assert!(stderr.contains("revision"), "stderr:\n{stderr}");
    assert!(stderr.contains("Usage: pointbreak"), "stderr:\n{stderr}");
}

fn request(repo: &GitRepo, title: &str) -> Value {
    request_with_body(repo, title, "")
}

fn request_with_body(repo: &GitRepo, title: &str, body: &str) -> Value {
    let mut args = vec![
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
    parse_json(&pointbreak(args).stdout)
}

fn shore_with_stdin<I, S>(args: I, stdin: &str) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut child = Command::new(env!("CARGO_BIN_EXE_pointbreak"))
        .args(args)
        .env_remove("POINTBREAK_LOG")
        .env_remove("RUST_LOG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn pointbreak binary");
    child
        .stdin
        .as_mut()
        .expect("pointbreak stdin is piped")
        .write_all(stdin.as_bytes())
        .expect("write pointbreak stdin");
    child.wait_with_output().expect("run pointbreak binary")
}

#[test]
fn text_input_request_list_shows_open_titles() {
    let repo = modified_repo();
    let repo_arg = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", repo_arg]);
    pointbreak([
        "input-request",
        "open",
        "--repo",
        repo_arg,
        "--track",
        "agent:codex",
        "--title",
        "Advisory question",
        "--reason",
        "manual-decision-required",
        "--mode",
        "advisory",
    ]);
    pointbreak([
        "input-request",
        "open",
        "--repo",
        repo_arg,
        "--track",
        "agent:codex",
        "--title",
        "Operative gate",
        "--reason",
        "manual-decision-required",
        "--mode",
        "operative",
    ]);

    let output = pointbreak([
        "input-request",
        "list",
        "--repo",
        repo_arg,
        "--format",
        "text",
    ]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("open input requests"), "stdout:\n{stdout}");
    assert!(stdout.contains("advisory"), "stdout:\n{stdout}");
    assert!(stdout.contains("operative"), "stdout:\n{stdout}");
    assert!(stdout.contains("input-request:"), "stdout:\n{stdout}");
    assert!(!stdout.contains("\"schema\""), "stdout:\n{stdout}");
}

#[test]
fn text_respond_ack_confirms_outcome() {
    let repo = modified_repo();
    let repo_arg = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", repo_arg]);
    let requested = parse_json(
        &pointbreak([
            "input-request",
            "open",
            "--repo",
            repo_arg,
            "--track",
            "agent:codex",
            "--title",
            "Need a decision",
            "--reason",
            "manual-decision-required",
        ])
        .stdout,
    );
    let input_request_id = requested["inputRequestId"].as_str().unwrap();

    let output = pointbreak([
        "input-request",
        "respond",
        input_request_id,
        "--repo",
        repo_arg,
        "--outcome",
        "approved",
        "--reason",
        "looks fine",
        "--format",
        "text",
    ]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("responded"), "stdout:\n{stdout}");
    assert!(stdout.contains("approved"), "stdout:\n{stdout}");
    assert!(
        stdout.lines().count() <= 4,
        "ack must be terse, got {} lines:\n{stdout}",
        stdout.lines().count()
    );
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
    assert!(!output.contains("pointbreak.review-intervention"));
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

/// Respond to `input_request_id` with `reason`; returns the respond document.
fn respond_with_reason(repo: &GitRepo, input_request_id: &str, reason: &str) -> Value {
    let output = pointbreak([
        "input-request",
        "respond",
        input_request_id,
        "--repo",
        repo.path().to_str().unwrap(),
        "--outcome",
        "approved",
        "--reason",
        reason,
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    parse_json(&output.stdout)
}

/// Delete the note-body blob for a `sha256:<hex>` content hash without
/// recording a removal event.
fn delete_note_blob(repo: &GitRepo, content_hash: &str) {
    let hex = content_hash.strip_prefix("sha256:").unwrap();
    std::fs::remove_file(
        support::common_dir_store(repo.path())
            .join("artifacts")
            .join("notes")
            .join(format!("{hex}.json")),
    )
    .expect("delete the note blob without a removal event");
}

#[test]
fn externalized_response_reason_hydrates_with_include_body() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request(&repo, "Need approval");
    let input_request_id = requested["inputRequestId"].as_str().unwrap();
    let reason = "x".repeat(5000);
    let responded = respond_with_reason(&repo, input_request_id, &reason);
    assert!(
        responded["reasonContentHash"].as_str().is_some(),
        "a >4096-byte reason must externalize"
    );
    let arg = repo.path().to_str().unwrap();

    let list = pointbreak([
        "input-request",
        "list",
        "--repo",
        arg,
        "--status",
        "all",
        "--include-body",
    ]);
    assert!(
        list.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&list.stderr)
    );
    let list = parse_json(&list.stdout);
    assert_eq!(
        list["inputRequests"][0]["responses"][0]["reason"],
        reason.as_str(),
        "list --include-body must hydrate the externalized reason"
    );

    let fetch = pointbreak([
        "input-request",
        "show",
        input_request_id,
        "--repo",
        arg,
        "--include-body",
    ]);
    assert!(
        fetch.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&fetch.stderr)
    );
    let fetch = parse_json(&fetch.stdout);
    assert_eq!(
        fetch["inputRequest"]["responses"][0]["reason"],
        reason.as_str(),
        "fetch --include-body must hydrate the externalized reason"
    );

    let show = pointbreak(["revision", "show", "--repo", arg, "--include-body"]);
    assert!(
        show.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&show.stderr)
    );
    let show = parse_json(&show.stdout);
    assert_eq!(
        show["inputRequests"][0]["responses"][0]["reason"],
        reason.as_str(),
        "show --include-body must hydrate the externalized reason"
    );
}

#[test]
fn response_reason_renders_only_with_include_body() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request(&repo, "Need approval");
    let input_request_id = requested["inputRequestId"].as_str().unwrap();
    respond_with_reason(&repo, input_request_id, "a short inline reason");
    let arg = repo.path().to_str().unwrap();

    let without = pointbreak(["input-request", "list", "--repo", arg, "--status", "all"]);
    assert!(
        without.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&without.stderr)
    );
    let without = parse_json(&without.stdout);
    let response = &without["inputRequests"][0]["responses"][0];
    assert!(
        response.get("reason").is_none(),
        "an inline reason must stay absent without --include-body, \
         matching the request body on the same surface: {response}"
    );
    assert!(response["reasonContentHash"].as_str().is_some());

    let with = pointbreak([
        "input-request",
        "list",
        "--repo",
        arg,
        "--status",
        "all",
        "--include-body",
    ]);
    let with = parse_json(&with.stdout);
    assert_eq!(
        with["inputRequests"][0]["responses"][0]["reason"],
        "a short inline reason"
    );
}

#[test]
fn missing_reason_artifact_errors_only_when_reading() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request(&repo, "Need approval");
    let input_request_id = requested["inputRequestId"].as_str().unwrap();
    let responded = respond_with_reason(&repo, input_request_id, &"x".repeat(5000));
    delete_note_blob(
        &repo,
        responded["reasonContentHash"]
            .as_str()
            .expect("a >4096-byte reason must externalize"),
    );
    let arg = repo.path().to_str().unwrap();

    let hydrating = pointbreak([
        "input-request",
        "show",
        input_request_id,
        "--repo",
        arg,
        "--include-body",
    ]);
    assert!(
        !hydrating.status.success(),
        "absent reason bytes without an operative removal must keep the hard error"
    );
    assert!(
        String::from_utf8_lossy(&hydrating.stderr).contains("import referenced artifacts"),
        "stderr:\n{}",
        String::from_utf8_lossy(&hydrating.stderr)
    );

    let state_only = pointbreak(["input-request", "show", input_request_id, "--repo", arg]);
    assert!(
        state_only.status.success(),
        "without --include-body no read is attempted, so no error: stderr:\n{}",
        String::from_utf8_lossy(&state_only.stderr)
    );
}

#[test]
fn missing_reason_artifact_on_an_unrelated_request_does_not_poison_other_reads() {
    let repo = modified_repo();
    let arg = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", arg]);
    let broken = request(&repo, "Broken");
    let broken_id = broken["inputRequestId"].as_str().unwrap();
    let responded = respond_with_reason(&repo, broken_id, &"x".repeat(5000));
    delete_note_blob(
        &repo,
        responded["reasonContentHash"]
            .as_str()
            .expect("a >4096-byte reason must externalize"),
    );
    let healthy = request(&repo, "Healthy");
    let healthy_id = healthy["inputRequestId"].as_str().unwrap();

    // The broken request is responded, so `--status open` excludes it; the
    // list must not resolve a reason it will not return.
    let open_list = pointbreak([
        "input-request",
        "list",
        "--repo",
        arg,
        "--status",
        "open",
        "--include-body",
    ]);
    assert!(
        open_list.status.success(),
        "a status-excluded request must not be hydrated: stderr:\n{}",
        String::from_utf8_lossy(&open_list.stderr)
    );
    let open_list = parse_json(&open_list.stdout);
    assert_eq!(open_list["inputRequests"].as_array().unwrap().len(), 1);
    assert_eq!(open_list["inputRequests"][0]["title"], "Healthy");

    // Fetching a different request must not resolve the broken one's reason.
    let fetch_healthy = pointbreak([
        "input-request",
        "show",
        healthy_id,
        "--repo",
        arg,
        "--include-body",
    ]);
    assert!(
        fetch_healthy.status.success(),
        "fetch of an unrelated request must not read the broken blob: stderr:\n{}",
        String::from_utf8_lossy(&fetch_healthy.stderr)
    );

    // A later revision does not carry the broken request; show/list of the
    // latest revision must not resolve it either.
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    let recapture = pointbreak(["capture", "--repo", arg]);
    assert!(
        recapture.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&recapture.stderr)
    );
    let recapture = parse_json(&recapture.stdout);
    let latest_revision = recapture["revision"]["id"].as_str().unwrap();
    let show = pointbreak([
        "revision",
        "show",
        latest_revision,
        "--repo",
        arg,
        "--include-body",
    ]);
    assert!(
        show.status.success(),
        "show of the latest revision must not read the older revision's broken blob: stderr:\n{}",
        String::from_utf8_lossy(&show.stderr)
    );
}

#[test]
fn text_input_request_open_receipt_names_the_request() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let output = pointbreak([
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
        "--format",
        "text",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        !stdout.contains("\"schema\""),
        "text lane is not JSON: {stdout}"
    );
    assert!(stdout.contains("opened"), "receipt verb: {stdout}");
    assert!(stdout.contains("Need approval"), "title: {stdout}");
    assert!(stdout.contains("input-request:"), "short id: {stdout}");
    assert!(stdout.contains("operative"), "default mode named: {stdout}");
}

#[test]
fn text_input_request_show_digest_renders_request_and_body() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request_with_body(&repo, "Need details", "full request body");
    let id = requested["inputRequestId"].as_str().unwrap();

    let output = pointbreak([
        "input-request",
        "show",
        id,
        "--repo",
        repo.path().to_str().unwrap(),
        "--include-body",
        "--format",
        "text",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        !stdout.contains("\"schema\""),
        "text lane is not JSON: {stdout}"
    );
    assert!(stdout.contains("Need details"), "title: {stdout}");
    assert!(stdout.contains("open"), "status: {stdout}");
    assert!(
        stdout.contains("full request body"),
        "hydrated body under --include-body: {stdout}"
    );
}
