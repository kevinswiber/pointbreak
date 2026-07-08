mod support;

use std::ffi::OsStr;
use std::io::Write;
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore;

#[test]
fn observation_add_and_list_run_at_the_top_level() {
    let repo = modified_repo();
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);

    let add = shore([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--title",
        "top-level observation",
        "--body",
        "checking the flatten",
    ]);
    assert!(
        add.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&add.stderr)
    );
    let added = parse_json(&add.stdout);
    assert_eq!(added["schema"], "pointbreak.review-observation-add"); // INV-1

    let list = shore([
        "observation",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);
    assert!(
        list.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&list.stderr)
    );
    let listed = parse_json(&list.stdout);
    assert_eq!(
        listed["observations"][0]["id"], added["observationId"],
        "the listed observation is the one just added"
    );
}

#[test]
fn observation_responds_to_resolves_a_bare_fragment_to_the_full_id() {
    let repo = modified_repo();
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);
    let first = parse_json(
        &shore([
            "observation",
            "add",
            "--repo",
            repo.path().to_str().unwrap(),
            "--track",
            "human:kevin",
            "--title",
            "first",
            "--body",
            "initial note",
        ])
        .stdout,
    );
    let first_id = first["observationId"].as_str().unwrap().to_owned();
    // first_id = "obs:sha256:<hex>".
    let fragment = &first_id["obs:sha256:".len()..][..8];

    let second = shore([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--title",
        "follow-up",
        "--body",
        "answering the first",
        "--responds-to",
        fragment,
    ]);
    assert!(
        second.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_id = parse_json(&second.stdout)["observationId"]
        .as_str()
        .unwrap()
        .to_owned();

    let listed = parse_json(
        &shore([
            "observation",
            "list",
            "--repo",
            repo.path().to_str().unwrap(),
        ])
        .stdout,
    );
    let second_entry = listed["observations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == second_id)
        .expect("the follow-up observation is listed");
    assert_eq!(
        second_entry["respondsTo"][0], first_id,
        "respondsTo must carry the resolved FULL id, not the bare fragment"
    );
}

#[test]
fn observation_add_records_review_wide_observation_and_emits_v1_json() {
    let repo = modified_repo();
    let capture = parse_json(&shore(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);

    let output = shore([
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
    assert_eq!(json["schema"], "pointbreak.review-observation-add");
    assert_eq!(json["version"], 1);
    assert_eq!(json["revisionId"], capture["revision"]["id"]);
    assert!(
        json["observationId"]
            .as_str()
            .unwrap()
            .starts_with("obs:sha256:")
    );
    assert!(json["eventId"].as_str().unwrap().starts_with("evt:sha256:"));
    assert_eq!(json["trackId"], "agent:codex");
    assert_eq!(json["target"]["kind"], "revision");
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
fn observation_add_responds_to_records_link() {
    let repo = modified_repo();
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);

    let a_out = shore([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "A",
    ]);
    assert!(
        a_out.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&a_out.stderr)
    );
    let a_id = parse_json(&a_out.stdout)["observationId"]
        .as_str()
        .unwrap()
        .to_owned();

    let out = shore([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "noted",
        "--responds-to",
        &a_id,
    ]);

    // clap accepts --responds-to and the add succeeds. (The payload link is asserted in the
    // library test; the respondsTo/respondedBy JSON surface is asserted by the list tests.)
    assert!(
        out.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn observation_markdown_body_content_type_round_trips() {
    let repo = modified_repo();
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "Markdown observation",
        "--body",
        "## Finding\n\n- keep **this** visible",
        "--body-content-type",
        "text/markdown",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let add = parse_json(&output.stdout);
    assert!(
        add["bodyContentHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"),
        "markdown bodies still carry a body content hash"
    );

    let list = parse_json(
        &shore([
            "observation",
            "list",
            "--repo",
            repo.path().to_str().unwrap(),
            "--include-body",
        ])
        .stdout,
    );
    let observation = &list["observations"][0];
    assert_eq!(observation["bodyContentType"], "text/markdown");
    assert_eq!(observation["body"], "## Finding\n\n- keep **this** visible");
}

#[test]
fn observation_add_records_range_observation() {
    let repo = modified_repo();
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
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
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
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
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);
    shore([
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
    assert_eq!(json["schema"], "pointbreak.review-observation-list");
    assert_eq!(json["version"], 1);
    assert_eq!(json["observations"].as_array().unwrap().len(), 1);
    assert_eq!(json["observations"][0]["trackId"], "agent:codex");
    assert_eq!(json["observations"][0]["title"], "First");
    assert_eq!(json["observations"][0]["status"], "active");
    assert!(json["observations"][0].get("body").is_none());
}

#[test]
fn observation_list_json_surfaces_responds_to_and_responded_by() {
    let repo = modified_repo();
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);
    let a_out = shore([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "A",
    ]);
    let a_id = parse_json(&a_out.stdout)["observationId"]
        .as_str()
        .unwrap()
        .to_owned();
    let b_out = shore([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "B",
        "--responds-to",
        &a_id,
    ]);
    let b_id = parse_json(&b_out.stdout)["observationId"]
        .as_str()
        .unwrap()
        .to_owned();

    let list = parse_json(
        &shore([
            "observation",
            "list",
            "--repo",
            repo.path().to_str().unwrap(),
        ])
        .stdout,
    );
    let observations = list["observations"].as_array().unwrap();
    let a = observations.iter().find(|o| o["id"] == a_id).unwrap();
    let b = observations.iter().find(|o| o["id"] == b_id).unwrap();

    assert!(
        b["respondsTo"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == &a_id)
    );
    assert!(
        a["respondedBy"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == &b_id)
    );
}

#[test]
fn observation_without_responses_omits_both_fields() {
    let repo = modified_repo();
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);
    shore([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "plain",
    ]);

    let list = parse_json(
        &shore([
            "observation",
            "list",
            "--repo",
            repo.path().to_str().unwrap(),
        ])
        .stdout,
    );
    let obs = &list["observations"][0];
    // Skip-empty: an observation with no response links emits neither key.
    assert!(obs.get("respondsTo").is_none());
    assert!(obs.get("respondedBy").is_none());
}

#[test]
fn observation_list_filters_by_track_and_file() {
    let repo = modified_repo();
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);
    shore([
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
fn observation_list_filters_by_tag() {
    let repo = modified_repo();
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);
    shore([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "Parser",
        "--tag",
        "correctness",
        "--tag",
        "parser",
    ]);
    shore([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "Docs",
        "--tag",
        "documentation",
    ]);

    let output = shore([
        "observation",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
        "--tag",
        "correctness",
        "--tag",
        "parser",
    ]);

    let json = parse_json(&output.stdout);
    assert_eq!(json["filters"]["tags"][0], "correctness");
    assert_eq!(json["filters"]["tags"][1], "parser");
    assert_eq!(json["observations"].as_array().unwrap().len(), 1);
    assert_eq!(json["observations"][0]["title"], "Parser");
}

#[test]
fn observation_list_include_body_hydrates_body() {
    let repo = modified_repo();
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);
    shore([
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
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
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
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);
    let body_file = repo.path().join("body.txt");
    std::fs::write(&body_file, "file body").unwrap();

    let output = shore([
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
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore_with_stdin(
        [
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
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);
    let args = [
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
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);

    let first = parse_json(
        &shore([
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
fn observation_add_errors_when_no_revision_has_been_captured() {
    let repo = modified_repo();

    let output = shore([
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
    assert!(String::from_utf8_lossy(&output.stderr).contains("no captured revision"));
}

#[test]
fn observation_add_rejects_unknown_file_target() {
    let repo = modified_repo();
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
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
fn observation_add_with_explicit_revision_succeeds_when_current_is_ambiguous() {
    let repo = modified_repo();
    let first = parse_json(&shore(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    repo.write("another.txt", "new untracked file\n");
    let second = parse_json(&shore(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    assert_ne!(first["revision"]["id"], second["revision"]["id"]);

    let output = shore([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--revision",
        first["revision"]["id"].as_str().unwrap(),
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
    assert_eq!(json["revisionId"], first["revision"]["id"]);
}

#[test]
fn observation_add_errors_when_current_revision_is_ambiguous_without_explicit_id() {
    let repo = modified_repo();
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);
    repo.write("another.txt", "new untracked file\n");
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
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
    assert!(String::from_utf8_lossy(&output.stderr).contains("multiple captured revisions"));
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

#[test]
fn observation_add_and_list_work_against_range_captured_unit() {
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

    let add = parse_json(
        &shore([
            "observation",
            "add",
            "--repo",
            repo.path().to_str().unwrap(),
            "--revision",
            revision_id,
            "--track",
            "agent:codex",
            "--title",
            "Range observation",
        ])
        .stdout,
    );
    assert_eq!(add["revisionId"], revision_id);
    assert_eq!(add["eventsCreatedByType"]["review_observation_recorded"], 1);

    let list = parse_json(
        &shore([
            "observation",
            "list",
            "--repo",
            repo.path().to_str().unwrap(),
            "--revision",
            revision_id,
        ])
        .stdout,
    );
    assert_eq!(list["revisionId"], revision_id);
    assert_eq!(list["observations"].as_array().unwrap().len(), 1);
    assert_eq!(list["observations"][0]["title"], "Range observation");
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
