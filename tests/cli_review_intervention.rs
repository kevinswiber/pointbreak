mod support;

use std::ffi::OsStr;
use std::io::Write;
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore;

#[test]
fn intervention_request_records_blocking_request_and_emits_v1_json() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "intervention",
        "request",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--title",
        "Need approval",
        "--reason",
        "manual-decision-required",
        "--mode",
        "blocking",
        "--body",
        "approve this path?",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);

    assert_eq!(json["schema"], "shore.review-intervention-request");
    assert_eq!(json["version"], 1);
    assert!(
        json["interventionId"]
            .as_str()
            .unwrap()
            .starts_with("intervention:sha256:")
    );
    assert_eq!(json["trackId"], "human:kevin");
    assert_eq!(json["mode"], "blocking");
    assert_eq!(json["reasonCode"], "manual_decision_required");
    assert_eq!(json["eventsCreatedByType"]["intervention_requested"], 1);
    assert!(json.get("bodyArtifactPath").is_none());
    assert!(repo.path().join(".shore/state.json").is_file());
}

#[test]
fn intervention_list_emits_v1_json_and_pretty_prints() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    request(&repo, "First");

    let output = shore([
        "review",
        "intervention",
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

    assert_eq!(json["schema"], "shore.review-intervention-list");
    assert_eq!(json["version"], 1);
    assert_eq!(json["filters"]["status"], "open");
    assert_eq!(json["interventions"].as_array().unwrap().len(), 1);
    assert_eq!(json["interventions"][0]["title"], "First");
    assert_eq!(json["interventions"][0]["status"], "open");
}

#[test]
fn intervention_fetch_include_body_emits_v1_json_and_hydrates_body() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request_with_body(&repo, "Need details", "full request body");
    let intervention_id = requested["interventionId"].as_str().unwrap();

    let output = shore([
        "review",
        "intervention",
        "fetch",
        intervention_id,
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

    assert_eq!(json["schema"], "shore.review-intervention-fetch");
    assert_eq!(json["version"], 1);
    assert_eq!(json["intervention"]["id"], intervention_id);
    assert_eq!(json["intervention"]["body"], "full request body");
}

#[test]
fn intervention_resolve_records_resolution_and_emits_v1_json() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request(&repo, "Need approval");
    let intervention_id = requested["interventionId"].as_str().unwrap();

    let output = shore([
        "review",
        "intervention",
        "resolve",
        intervention_id,
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

    assert_eq!(json["schema"], "shore.review-intervention-resolve");
    assert_eq!(json["version"], 1);
    assert_eq!(json["interventionId"], intervention_id);
    assert!(
        json["interventionResolutionId"]
            .as_str()
            .unwrap()
            .starts_with("intervention-resolution:sha256:")
    );
    assert_eq!(json["outcome"], "approved");
    assert_eq!(json["eventsCreatedByType"]["intervention_resolved"], 1);
    assert!(json.get("reasonArtifactPath").is_none());
}

#[test]
fn intervention_request_body_stdin_reads_from_stdin() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore_with_stdin(
        [
            "review",
            "intervention",
            "request",
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
        "intervention",
        "fetch",
        requested["interventionId"].as_str().unwrap(),
        "--repo",
        repo.path().to_str().unwrap(),
        "--include-body",
    ]);
    let json = parse_json(&fetch.stdout);
    assert_eq!(json["intervention"]["body"], "body from stdin");
}

#[test]
fn intervention_resolve_reason_stdin_reads_from_stdin() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request(&repo, "Need approval");
    let intervention_id = requested["interventionId"].as_str().unwrap();

    let output = shore_with_stdin(
        [
            "review",
            "intervention",
            "resolve",
            intervention_id,
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
fn intervention_request_requires_track() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "intervention",
        "request",
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
fn intervention_rejects_legacy_hunk_agent_context_flag_without_mutation() {
    let repo = GitRepo::new();

    let output = shore([
        "review",
        "intervention",
        "request",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--title",
        "Need approval",
        "--reason",
        "manual-decision-required",
        "--legacy-hunk-agent-context",
        "agent-context.json",
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected argument"));
    assert!(!repo.path().join(".shore").exists());
}

#[test]
fn intervention_request_rejects_invalid_reason_and_mode_values() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let bad_reason = shore([
        "review",
        "intervention",
        "request",
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
        "intervention",
        "request",
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
fn intervention_resolve_rejects_invalid_outcome() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request(&repo, "Need approval");

    let output = shore([
        "review",
        "intervention",
        "resolve",
        requested["interventionId"].as_str().unwrap(),
        "--repo",
        repo.path().to_str().unwrap(),
        "--outcome",
        "not-an-outcome",
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid value"));
}

#[test]
fn intervention_request_observation_target_conflicts_with_file_and_lines() {
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
        "intervention",
        "request",
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
fn intervention_request_body_inputs_are_mutually_exclusive() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let body_file = repo.path().join("body.txt");
    std::fs::write(&body_file, "file body").unwrap();

    let output = shore([
        "review",
        "intervention",
        "request",
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
fn intervention_resolve_reason_inputs_are_mutually_exclusive() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let requested = request(&repo, "Need approval");
    let reason_file = repo.path().join("reason.txt");
    std::fs::write(&reason_file, "file reason").unwrap();

    let output = shore([
        "review",
        "intervention",
        "resolve",
        requested["interventionId"].as_str().unwrap(),
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
fn intervention_request_requires_review_unit_when_current_is_ambiguous() {
    let repo = modified_repo();
    let first =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    repo.write("another.txt", "new untracked file\n");
    let second =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    assert_ne!(first["reviewUnit"]["id"], second["reviewUnit"]["id"]);

    let ambiguous = shore([
        "review",
        "intervention",
        "request",
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
        "intervention",
        "request",
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

fn request(repo: &GitRepo, title: &str) -> Value {
    request_with_body(repo, title, "")
}

fn request_with_body(repo: &GitRepo, title: &str, body: &str) -> Value {
    let mut args = vec![
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

fn modified_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo
}
