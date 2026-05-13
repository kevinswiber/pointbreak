mod support;

use std::ffi::OsStr;
use std::io::Write;
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore;

#[test]
fn observation_add_records_review_wide_observation_and_emits_v1_json() {
    let repo = modified_repo();
    let capture =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);

    let output = shore([
        "review",
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "Check return value",
        "--body",
        "The changed return value needs review.",
        "--tag",
        "correctness",
        "--confidence",
        "high",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    assert_eq!(json["schema"], "shore.review-observation-add");
    assert_eq!(json["version"], 1);
    assert_eq!(json["reviewUnitId"], capture["reviewUnit"]["id"]);
    assert!(
        json["observationId"]
            .as_str()
            .unwrap()
            .starts_with("obs:sha256:")
    );
    assert!(json["eventId"].as_str().unwrap().starts_with("evt:sha256:"));
    assert_eq!(json["trackId"], "agent:codex");
    assert_eq!(json["target"]["kind"], "review_unit");
    assert!(
        json["bodyContentHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert_eq!(json["eventsCreated"], 1);
    assert_eq!(json["eventsExisting"], 0);
    assert_eq!(
        json["eventsCreatedByType"]["review_observation_recorded"],
        1
    );
    assert!(json.get("statePath").is_none());
    assert!(json.get("bodyArtifactPath").is_none());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("artifacts/notes/"));
}

#[test]
fn observation_add_records_range_observation() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "Range finding",
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
fn observation_add_requires_track() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--title",
        "Missing track",
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--track"));
}

#[test]
fn observation_list_reads_recorded_observations() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    shore([
        "review",
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "First",
    ]);

    let output = shore([
        "review",
        "observation",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    assert_eq!(json["schema"], "shore.review-observation-list");
    assert_eq!(json["version"], 1);
    assert_eq!(json["observations"].as_array().unwrap().len(), 1);
    assert_eq!(json["observations"][0]["trackId"], "agent:codex");
    assert_eq!(json["observations"][0]["title"], "First");
    assert_eq!(json["observations"][0]["status"], "active");
    assert!(json["observations"][0].get("body").is_none());
}

#[test]
fn observation_list_filters_by_track_and_file() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    shore([
        "review",
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "File",
        "--file",
        "src/lib.rs",
    ]);
    shore([
        "review",
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:claude",
        "--title",
        "Other",
    ]);

    let output = shore([
        "review",
        "observation",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--file",
        "src/lib.rs",
    ]);

    let json = parse_json(&output.stdout);
    assert_eq!(json["filters"]["trackId"], "agent:codex");
    assert_eq!(json["filters"]["file"], "src/lib.rs");
    assert_eq!(json["observations"].as_array().unwrap().len(), 1);
}

#[test]
fn observation_list_include_body_hydrates_body() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    shore([
        "review",
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "Body",
        "--body",
        "details",
    ]);

    let output = shore([
        "review",
        "observation",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
        "--include-body",
    ]);

    let json = parse_json(&output.stdout);
    assert_eq!(json["observations"][0]["body"], "details");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("artifacts/notes/"));
}

#[test]
fn observation_list_pretty_prints_when_requested() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "observation",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
        "--pretty",
    ]);

    assert!(String::from_utf8_lossy(&output.stdout).starts_with("{\n"));
}

#[test]
fn observation_add_body_inputs_are_mutually_exclusive() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let body_file = repo.path().join("body.txt");
    std::fs::write(&body_file, "file body").unwrap();

    let output = shore([
        "review",
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "Body",
        "--body",
        "inline",
        "--body-file",
        body_file.to_str().unwrap(),
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used"));
}

#[test]
fn observation_add_body_stdin_reads_from_stdin() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore_with_stdin(
        [
            "review",
            "observation",
            "add",
            "--repo",
            repo.path().to_str().unwrap(),
            "--track",
            "agent:codex",
            "--title",
            "stdin body",
            "--body-stdin",
        ],
        "body from stdin",
    );

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let list = shore([
        "review",
        "observation",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
        "--include-body",
    ]);
    let json = parse_json(&list.stdout);
    assert_eq!(json["observations"][0]["body"], "body from stdin");
}

#[test]
fn observation_add_is_idempotent_on_rerun() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    let args = [
        "review",
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "Retry",
        "--idempotency-key",
        "retry-key",
    ];

    let first = parse_json(&shore(args).stdout);
    let second = parse_json(&shore(args).stdout);

    assert_eq!(first["observationId"], second["observationId"]);
    assert_eq!(first["eventsCreated"], 1);
    assert_eq!(second["eventsCreated"], 0);
    assert_eq!(second["eventsExisting"], 1);
}

#[test]
fn observation_list_collapses_duplicate_semantic_events() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let first = parse_json(
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
            "retry-a",
        ])
        .stdout,
    );
    let second = parse_json(
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
            "retry-b",
        ])
        .stdout,
    );

    let list = shore([
        "review",
        "observation",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
        "--include-body",
    ]);
    let json = parse_json(&list.stdout);
    let diagnostic = diagnostic_with_code(&json, "duplicate_semantic_observation_event");
    let observation_id = first["observationId"].as_str().unwrap();

    assert_eq!(first["observationId"], second["observationId"]);
    assert_eq!(json["observations"].as_array().unwrap().len(), 1);
    assert_eq!(json["observations"][0]["id"], first["observationId"]);
    assert_eq!(json["observations"][0]["body"], "same body");
    assert!(
        diagnostic["message"]
            .as_str()
            .unwrap()
            .contains(observation_id)
    );
}

#[test]
fn observation_add_errors_when_no_review_unit_has_been_captured() {
    let repo = modified_repo();

    let output = shore([
        "review",
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "No capture",
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no captured review unit"));
}

#[test]
fn observation_add_rejects_unknown_file_target() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "Bad file",
        "--file",
        "missing.rs",
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not present in captured snapshot"));
}

#[test]
fn observation_add_with_explicit_review_unit_succeeds_when_current_is_ambiguous() {
    let repo = modified_repo();
    let first =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    repo.write("another.txt", "new untracked file\n");
    let second =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    assert_ne!(first["reviewUnit"]["id"], second["reviewUnit"]["id"]);

    let output = shore([
        "review",
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--review-unit",
        first["reviewUnit"]["id"].as_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "Explicit target",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    assert_eq!(json["reviewUnitId"], first["reviewUnit"]["id"]);
}

#[test]
fn observation_add_errors_when_current_review_unit_is_ambiguous_without_explicit_id() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    repo.write("another.txt", "new untracked file\n");
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "Ambiguous",
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("multiple captured review units"));
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

fn modified_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo
}
