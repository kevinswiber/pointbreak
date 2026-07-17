mod support;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::pointbreak;

#[test]
fn validation_add_and_list_run_at_the_top_level() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let add = pointbreak([
        "validation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--check-name",
        "unit-tests",
        "--status",
        "passed",
    ]);
    assert!(
        add.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&add.stderr)
    );
    let added = parse_json(&add.stdout);
    assert_eq!(added["schema"], "pointbreak.review-validation-add"); // INV-1

    let list = pointbreak([
        "validation",
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
        listed["validationChecks"][0]["id"], added["validationCheckId"],
        "the listed check is the one just added"
    );
}

#[test]
fn validation_exact_revision_targets_a_superseded_revision() {
    let (repo, first_id, second_id) = support::superseded_dump_repo();
    let repo_arg = repo.path().to_str().unwrap();

    let legacy = pointbreak([
        "validation",
        "add",
        "--repo",
        repo_arg,
        "--revision",
        &first_id,
        "--track",
        "human:legacy",
        "--check-name",
        "legacy",
        "--status",
        "passed",
    ]);
    assert!(
        legacy.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&legacy.stderr)
    );
    assert_eq!(parse_json(&legacy.stdout)["revisionId"], second_id);

    let exact = pointbreak([
        "validation",
        "add",
        "--repo",
        repo_arg,
        "--exact-revision",
        &first_id,
        "--track",
        "human:exact",
        "--check-name",
        "exact",
        "--status",
        "passed",
    ]);
    assert!(
        exact.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&exact.stderr)
    );
    assert_eq!(parse_json(&exact.stdout)["revisionId"], first_id);
}

#[test]
fn validation_exact_revision_rejects_conflicting_or_unknown_selectors_before_write() {
    let repo = modified_repo();
    let repo_arg = repo.path().to_str().unwrap();
    let capture = parse_json(&pointbreak(["capture", "--repo", repo_arg]).stdout);
    let revision_id = capture["revision"]["id"].as_str().unwrap();

    let conflicting = pointbreak([
        "validation",
        "add",
        "--repo",
        repo_arg,
        "--revision",
        revision_id,
        "--exact-revision",
        revision_id,
        "--track",
        "human:kevin",
        "--check-name",
        "conflict",
        "--status",
        "passed",
    ]);
    assert!(!conflicting.status.success());
    assert!(String::from_utf8_lossy(&conflicting.stderr).contains("cannot be used with"));

    let unknown = pointbreak([
        "validation",
        "add",
        "--repo",
        repo_arg,
        "--exact-revision",
        "rev:sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "--track",
        "human:kevin",
        "--check-name",
        "unknown",
        "--status",
        "passed",
    ]);
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown revision"));
}

#[test]
fn validation_add_revision_resolves_a_bare_fragment_before_it_is_stored() {
    let repo = modified_repo();
    let captured =
        parse_json(&pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    let full_id = captured["revision"]["id"].as_str().unwrap().to_owned();
    // full_id = "rev:sha256:<64hex>".
    let fragment = &full_id["rev:sha256:".len()..][..8];

    let add = pointbreak([
        "validation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--revision",
        fragment,
        "--track",
        "human:kevin",
        "--check-name",
        "unit-tests",
        "--status",
        "passed",
    ]);
    assert!(
        add.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&add.stderr)
    );
    let added = parse_json(&add.stdout);
    assert_eq!(
        added["revisionId"], full_id,
        "the recorded check must reference the resolved FULL revision id, not the bare fragment"
    );
}

#[test]
fn cli_review_validation_add_emits_validation_add_document() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = pointbreak([
        "validation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--check-name",
        "cargo test",
        "--status",
        "passed",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = parse_json(&output.stdout);
    assert_eq!(value["schema"], "pointbreak.review-validation-add");
    assert_eq!(value["eventsCreated"], 1);
    assert_eq!(value["status"], "passed");
    assert_eq!(value["target"]["kind"], "revision");
}

#[test]
fn cli_review_validation_list_emits_list_document() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let add = pointbreak([
        "validation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--check-name",
        "cargo test",
        "--status",
        "passed",
    ]);
    assert!(
        add.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&add.stderr)
    );

    let output = pointbreak([
        "validation",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = parse_json(&output.stdout);
    assert_eq!(value["schema"], "pointbreak.review-validation-list");
    assert!(value["validationChecks"].is_array());
    assert_eq!(value["validationChecks"][0]["checkName"], "cargo test");
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

#[test]
fn text_validation_list_digest_lists_checks_one_line_each() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let add = pointbreak([
        "validation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--check-name",
        "unit-tests",
        "--status",
        "passed",
    ]);
    assert!(
        add.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&add.stderr)
    );

    let output = pointbreak([
        "validation",
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
    assert!(
        stdout.contains("1 validation check"),
        "count headline: {stdout}"
    );
    assert!(stdout.contains("validation:"), "short check id: {stdout}");
    assert!(stdout.contains("unit-tests"), "check name: {stdout}");
    assert!(stdout.contains("passed"), "status: {stdout}");
    assert!(stdout.contains("manual"), "trigger: {stdout}");
    assert_eq!(
        stdout.trim_end().lines().count(),
        2,
        "header plus one line per check: {stdout}"
    );
}

#[test]
fn text_validation_list_digest_reports_empty() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = pointbreak([
        "validation",
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
        stdout.contains("no validation checks"),
        "empty line, never silence: {stdout}"
    );
    assert!(
        !stdout.contains("\"schema\""),
        "text lane is not JSON: {stdout}"
    );
}

#[test]
fn text_validation_add_receipt_names_check_and_status() {
    let repo = modified_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = pointbreak([
        "validation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--check-name",
        "unit-tests",
        "--status",
        "passed",
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
        stdout.contains("recorded validation"),
        "receipt verb: {stdout}"
    );
    assert!(stdout.contains("unit-tests"), "check name: {stdout}");
    assert!(stdout.contains("passed"), "status: {stdout}");
    assert!(stdout.contains("validation:"), "short check id: {stdout}");
}
