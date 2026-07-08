mod support;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore;

#[test]
fn validation_add_and_list_run_at_the_top_level() {
    let repo = modified_repo();
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);

    let add = shore([
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

    let list = shore([
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
fn validation_add_revision_resolves_a_bare_fragment_before_it_is_stored() {
    let repo = modified_repo();
    let captured = parse_json(&shore(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    let full_id = captured["revision"]["id"].as_str().unwrap().to_owned();
    // full_id = "rev:sha256:<64hex>".
    let fragment = &full_id["rev:sha256:".len()..][..8];

    let add = shore([
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
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
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
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);
    let add = shore([
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

    let output = shore([
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
