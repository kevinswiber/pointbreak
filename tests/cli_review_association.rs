mod support;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore;

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

fn capture(repo: &GitRepo) {
    let output = shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    assert!(
        output.status.success(),
        "capture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn associate_commit_records_then_reports_existing_on_rerun() {
    let repo = modified_repo();
    capture(&repo);
    let repo_path = repo.path().to_str().unwrap();

    let first = shore([
        "review",
        "association",
        "associate-commit",
        "--repo",
        repo_path,
        "--track",
        "agent:codex",
        "--commit",
        "HEAD",
    ]);
    assert!(
        first.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let json = parse_json(&first.stdout);
    assert_eq!(json["schema"], "shore.review-association-commit");
    assert_eq!(json["eventsCreated"], 1);
    assert_eq!(json["eventsCreatedByType"]["revision_commit_associated"], 1);
    let association_id = json["commitAssociationId"].as_str().unwrap();
    assert!(association_id.starts_with("assoc-commit:"));
    assert!(json["eventId"].as_str().unwrap().starts_with("evt:sha256:"));

    let again = shore([
        "review",
        "association",
        "associate-commit",
        "--repo",
        repo_path,
        "--track",
        "agent:codex",
        "--commit",
        "HEAD",
    ]);
    let json = parse_json(&again.stdout);
    assert_eq!(json["eventsCreated"], 0);
    assert_eq!(json["eventsExisting"], 1);
    assert_eq!(json["commitAssociationId"], association_id);
}

#[test]
fn withdraw_commit_removes_from_current_list() {
    let repo = modified_repo();
    capture(&repo);
    let repo_path = repo.path().to_str().unwrap();

    let associate = parse_json(
        &shore([
            "review",
            "association",
            "associate-commit",
            "--repo",
            repo_path,
            "--track",
            "agent:codex",
            "--commit",
            "HEAD",
        ])
        .stdout,
    );
    let association_id = associate["commitAssociationId"].as_str().unwrap();

    let current_before = parse_json(
        &shore([
            "review",
            "association",
            "list",
            "--repo",
            repo_path,
            "--axis",
            "commit",
            "--current",
        ])
        .stdout,
    );
    assert_eq!(
        current_before["currentCommits"].as_array().unwrap().len(),
        1
    );

    let withdraw = shore([
        "review",
        "association",
        "withdraw-commit",
        "--repo",
        repo_path,
        "--track",
        "agent:codex",
        "--withdraws",
        association_id,
    ]);
    assert!(
        withdraw.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&withdraw.stderr)
    );
    let json = parse_json(&withdraw.stdout);
    assert_eq!(json["schema"], "shore.review-association-commit-withdrawn");
    assert_eq!(json["commitAssociationId"], association_id);

    let current_after = parse_json(
        &shore([
            "review",
            "association",
            "list",
            "--repo",
            repo_path,
            "--axis",
            "commit",
            "--current",
        ])
        .stdout,
    );
    assert!(
        current_after["currentCommits"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn associate_ref_stores_full_ref_and_head() {
    let repo = modified_repo();
    capture(&repo);
    let repo_path = repo.path().to_str().unwrap();
    let head_oid = repo.git(["rev-parse", "HEAD"]).stdout.trim().to_owned();

    let output = shore([
        "review",
        "association",
        "associate-ref",
        "--repo",
        repo_path,
        "--track",
        "agent:codex",
        "--ref",
        "refs/heads/feat/x",
        "--head",
        &head_oid,
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    assert_eq!(json["schema"], "shore.review-association-ref");
    assert_eq!(json["refName"], "refs/heads/feat/x");
    assert_eq!(json["headOid"], head_oid);
    assert!(
        json["refAssociationId"]
            .as_str()
            .unwrap()
            .starts_with("assoc-ref:")
    );
}

#[test]
fn associate_ref_normalizes_a_short_branch_name() {
    let repo = modified_repo();
    capture(&repo);
    let repo_path = repo.path().to_str().unwrap();

    let json = parse_json(
        &shore([
            "review",
            "association",
            "associate-ref",
            "--repo",
            repo_path,
            "--track",
            "agent:codex",
            "--branch",
            "feat/short",
            "--head",
            "abc123",
        ])
        .stdout,
    );
    assert_eq!(json["refName"], "refs/heads/feat/short");
}

#[test]
fn list_axis_commit_excludes_ref_associations() {
    let repo = modified_repo();
    capture(&repo);
    let repo_path = repo.path().to_str().unwrap();

    shore([
        "review",
        "association",
        "associate-commit",
        "--repo",
        repo_path,
        "--track",
        "agent:codex",
        "--commit",
        "HEAD",
    ]);
    shore([
        "review",
        "association",
        "associate-ref",
        "--repo",
        repo_path,
        "--track",
        "agent:codex",
        "--ref",
        "refs/heads/feat/x",
        "--head",
        "abc123",
    ]);

    let json = parse_json(
        &shore([
            "review",
            "association",
            "list",
            "--repo",
            repo_path,
            "--axis",
            "commit",
        ])
        .stdout,
    );
    assert_eq!(json["currentCommits"].as_array().unwrap().len(), 1);
    assert!(json["currentRefs"].as_array().unwrap().is_empty());
}

#[test]
fn history_filters_to_the_commit_associated_event_type() {
    let repo = modified_repo();
    capture(&repo);
    let repo_path = repo.path().to_str().unwrap();
    shore([
        "review",
        "association",
        "associate-commit",
        "--repo",
        repo_path,
        "--track",
        "agent:codex",
        "--commit",
        "HEAD",
    ]);

    let json = parse_json(
        &shore([
            "review",
            "history",
            "--repo",
            repo_path,
            "--event-type",
            "review-unit-commit-associated",
        ])
        .stdout,
    );
    let entries = json["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["eventType"], "revision_commit_associated");
    assert_eq!(entries[0]["summary"]["kind"], "revision_commit_associated");
}

#[test]
fn unit_list_ref_label_filter_matches_normalized_short_branch() {
    let repo = modified_repo();
    repo.git(["branch", "-M", "feat/x"]);
    capture(&repo); // auto-records refs/heads/feat/x
    let repo_path = repo.path().to_str().unwrap();

    // A short branch name is normalized to the stored full ref.
    let matched = parse_json(
        &shore([
            "review",
            "revisions",
            "--repo",
            repo_path,
            "--branch",
            "feat/x",
            "--by",
            "label",
        ])
        .stdout,
    );
    assert_eq!(matched["entries"].as_array().unwrap().len(), 1);
    assert_eq!(
        matched["entries"][0]["commitRange"]["currentRefs"][0]["refName"],
        "refs/heads/feat/x"
    );

    let unmatched = parse_json(
        &shore([
            "review",
            "revisions",
            "--repo",
            repo_path,
            "--ref",
            "refs/heads/other",
        ])
        .stdout,
    );
    assert!(unmatched["entries"].as_array().unwrap().is_empty());
}

#[test]
fn unit_list_ref_liveness_filter_matches_reachable_commit() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.commit_all("change");
    repo.git(["branch", "-M", "main"]);
    let repo_path = repo.path().to_str().unwrap();

    // A commit-range capture anchors the target (HEAD) commit.
    let capture = shore(["review", "capture", "--repo", repo_path, "--base", "HEAD~1"]);
    assert!(capture.status.success());

    let json = parse_json(
        &shore([
            "review",
            "revisions",
            "--repo",
            repo_path,
            "--ref",
            "refs/heads/main",
            "--by",
            "liveness",
        ])
        .stdout,
    );
    assert_eq!(
        json["entries"].as_array().unwrap().len(),
        1,
        "the anchored target commit is reachable from main"
    );
}

#[test]
fn unit_show_includes_commit_range_and_liveness_block() {
    let repo = modified_repo();
    repo.git(["branch", "-M", "main"]);
    capture(&repo);
    let repo_path = repo.path().to_str().unwrap();
    shore([
        "review",
        "association",
        "associate-commit",
        "--repo",
        repo_path,
        "--track",
        "agent:codex",
        "--commit",
        "HEAD",
    ]);

    let json = parse_json(&shore(["review", "show", "--repo", repo_path]).stdout);
    let commit_range = &json["commitRange"];
    assert_eq!(commit_range["anchored"], true);
    assert_eq!(commit_range["currentCommits"].as_array().unwrap().len(), 1);
    // The liveness block is layered CLI-side from the live repo.
    let per_commit = commit_range["liveness"]["perCommit"].as_array().unwrap();
    assert_eq!(per_commit.len(), 1);
    assert!(per_commit[0]["condition"].is_string());
}

#[test]
fn association_verbs_reject_a_replaces_flag() {
    let repo = modified_repo();
    capture(&repo);
    let repo_path = repo.path().to_str().unwrap();

    let output = shore([
        "review",
        "association",
        "associate-commit",
        "--repo",
        repo_path,
        "--track",
        "agent:codex",
        "--commit",
        "HEAD",
        "--replaces",
        "anything",
    ]);
    assert!(
        !output.status.success(),
        "withdraw-only family must not accept --replaces"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--replaces")
            || String::from_utf8_lossy(&output.stderr).contains("unexpected"),
        "clap should reject --replaces: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
