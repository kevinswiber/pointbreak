mod support;

use std::ffi::OsStr;
use std::io::Write;
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use support::git_repo::GitRepo;
use support::pointbreak;

#[test]
fn observation_add_and_list_run_at_the_top_level() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let add = pointbreak([
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

    let list = pointbreak([
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
fn observation_exact_revision_targets_and_validates_the_named_snapshot() {
    let repo = GitRepo::new();
    repo.write("README.md", "base\n");
    repo.commit_all("base");
    repo.write("only-a.rs", "fn a() {}\n");
    let first = parse_json(
        &pointbreak([
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--include-untracked",
        ])
        .stdout,
    );
    let first_id = first["revision"]["id"].as_str().unwrap().to_owned();
    repo.remove("only-a.rs");
    repo.write("only-b.rs", "fn b() {}\n");
    let second = parse_json(
        &pointbreak([
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--include-untracked",
            "--supersedes",
            &first_id,
        ])
        .stdout,
    );
    let second_id = second["revision"]["id"].as_str().unwrap().to_owned();

    let legacy = pointbreak([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--revision",
        &first_id,
        "--track",
        "human:legacy",
        "--title",
        "head seed",
    ]);
    assert!(
        legacy.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&legacy.stderr)
    );
    assert_eq!(parse_json(&legacy.stdout)["revisionId"], second_id);

    let exact = pointbreak([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--exact-revision",
        &first_id,
        "--track",
        "human:exact",
        "--title",
        "exact target",
        "--file",
        "only-a.rs",
    ]);
    assert!(
        exact.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&exact.stderr)
    );
    let exact = parse_json(&exact.stdout);
    assert_eq!(exact["revisionId"], first_id);
    assert_eq!(exact["target"]["filePath"], "only-a.rs");

    let wrong_snapshot = pointbreak([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--revision",
        &first_id,
        "--track",
        "human:legacy",
        "--title",
        "wrong snapshot",
        "--file",
        "only-a.rs",
    ]);
    assert!(!wrong_snapshot.status.success());
    assert!(String::from_utf8_lossy(&wrong_snapshot.stderr).contains("not present"));
}

#[test]
fn observation_exact_revision_rejects_conflicting_or_unknown_selectors_before_write() {
    let repo = modified_repo();
    let capture =
        parse_json(&pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    let revision_id = capture["revision"]["id"].as_str().unwrap();

    let conflicting = pointbreak([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--revision",
        revision_id,
        "--exact-revision",
        revision_id,
        "--track",
        "human:kevin",
        "--title",
        "conflict",
    ]);
    assert!(!conflicting.status.success());
    assert!(String::from_utf8_lossy(&conflicting.stderr).contains("cannot be used with"));

    let unknown = pointbreak([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--exact-revision",
        "rev:sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "--track",
        "human:kevin",
        "--title",
        "unknown",
    ]);
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown revision"));
}

#[test]
fn observation_responds_to_resolves_a_bare_fragment_to_the_full_id() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let first = parse_json(
        &pointbreak([
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

    let second = pointbreak([
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
        &pointbreak([
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
    let capture =
        parse_json(&pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);

    let output = pointbreak([
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let a_out = pointbreak([
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

    let out = pointbreak([
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = pointbreak([
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
        &pointbreak([
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = pointbreak([
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = pointbreak([
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    pointbreak([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "First",
    ]);

    let output = pointbreak([
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let a_out = pointbreak([
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
    let b_out = pointbreak([
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
        &pointbreak([
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    pointbreak([
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
        &pointbreak([
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    pointbreak([
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
    pointbreak([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:claude",
        "--title",
        "Other",
    ]);

    let output = pointbreak([
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    pointbreak([
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
    pointbreak([
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

    let output = pointbreak([
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    pointbreak([
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

    let output = pointbreak([
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
fn observation_list_json_pretty_prints_when_requested() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = pointbreak([
        "observation",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
        "--format",
        "json-pretty",
    ]);

    assert!(String::from_utf8_lossy(&output.stdout).starts_with("{\n"));
}

#[test]
fn observation_add_body_inputs_are_mutually_exclusive() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let body_file = repo.path().join("body.txt");
    std::fs::write(&body_file, "file body").unwrap();

    let output = pointbreak([
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

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

    let list = pointbreak([
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
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

    let first = parse_json(&pointbreak(args).stdout);
    let second = parse_json(&pointbreak(args).stdout);

    assert_eq!(first["observationId"], second["observationId"]);
    assert_eq!(first["eventsCreated"], 1);
    assert_eq!(second["eventsCreated"], 0);
    assert_eq!(second["eventsExisting"], 1);
}

#[test]
fn observation_list_collapses_duplicate_semantic_events() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let first = parse_json(
        &pointbreak([
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
        &pointbreak([
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

    let list = pointbreak([
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

    let output = pointbreak([
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = pointbreak([
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
    let first =
        parse_json(&pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    let second =
        parse_json(&pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    assert_ne!(first["revision"]["id"], second["revision"]["id"]);

    let output = pointbreak([
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
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = pointbreak([
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
    let mut child = Command::new(support::pointbreak_bin())
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

fn parse_json(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).expect("stdout is valid JSON")
}

#[test]
fn observation_add_and_list_work_against_range_captured_unit() {
    let repo = support::committed_repo();
    let capture = parse_json(
        &pointbreak([
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
        &pointbreak([
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
        &pointbreak([
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

#[test]
fn text_observation_list_digest_lists_titles_one_line_each() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let add = pointbreak([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--title",
        "the digest headline",
        "--body",
        "supporting detail",
        "--confidence",
        "high",
    ]);
    assert!(
        add.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&add.stderr)
    );

    let output = pointbreak([
        "observation",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
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
    assert!(stdout.contains("1 observation"), "count headline: {stdout}");
    assert!(stdout.contains("obs:"), "short observation id: {stdout}");
    assert!(
        stdout.contains("the digest headline"),
        "title is the content headline: {stdout}"
    );
    assert!(stdout.contains("human:kevin"), "track: {stdout}");
    assert!(stdout.contains("high"), "confidence when present: {stdout}");
    assert_eq!(
        stdout.trim_end().lines().count(),
        2,
        "header plus one line per observation: {stdout}"
    );
}

#[test]
fn text_observation_list_digest_reports_empty_with_filter() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = pointbreak([
        "observation",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:nobody",
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
        stdout.contains("no observations"),
        "empty line, never silence: {stdout}"
    );
    assert!(
        stdout.contains("agent:nobody"),
        "the active track filter is named: {stdout}"
    );
    assert!(
        !stdout.contains("\"schema\""),
        "text lane is not JSON: {stdout}"
    );
}

#[test]
fn text_observation_add_receipt_names_the_fact() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = pointbreak([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--title",
        "wave three receipt",
        "--body",
        "detail",
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
    assert!(
        stdout.contains("recorded observation"),
        "receipt verb: {stdout}"
    );
    assert!(stdout.contains("wave three receipt"), "title: {stdout}");
    assert!(stdout.contains("obs:"), "short observation id: {stdout}");
}
