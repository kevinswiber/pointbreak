mod support;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore;

fn parse_json(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).expect("stdout is valid JSON")
}

/// A repo with one captured revision carrying an open input request and two
/// current assessments from distinct tracks (an ambiguity).
fn store_with_attention(repo: &GitRepo) -> String {
    let repo_arg = repo.path().to_str().unwrap().to_owned();
    let capture = parse_json(&shore(["capture", "--repo", &repo_arg]).stdout);
    let revision_id = capture["revision"]["id"].as_str().unwrap().to_owned();

    let open = shore([
        "input-request",
        "open",
        "--repo",
        &repo_arg,
        "--track",
        "human:kevin",
        "--title",
        "Need a decision",
        "--reason",
        "insufficient-evidence",
    ]);
    assert!(
        open.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&open.stderr)
    );

    for (track, assessment) in [
        ("human:kevin", "accepted"),
        ("agent:codex", "needs-changes"),
    ] {
        let added = shore([
            "assessment",
            "add",
            "--repo",
            &repo_arg,
            "--track",
            track,
            "--assessment",
            assessment,
        ]);
        assert!(
            added.status.success(),
            "stderr:\n{}",
            String::from_utf8_lossy(&added.stderr)
        );
    }

    revision_id
}

#[test]
fn attention_list_emits_versioned_document() {
    let repo = modified_repo();
    store_with_attention(&repo);

    let output = shore([
        "attention",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);

    assert_eq!(json["schema"], "pointbreak.attention-list");
    assert_eq!(json["version"], 1);
    assert!(
        json["eventSetHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert!(json["filters"]["revision"].is_null());

    let kinds: Vec<&str> = json["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"open_input_request"), "kinds: {kinds:?}");
    assert!(kinds.contains(&"ambiguous_assessment"), "kinds: {kinds:?}");

    // The open request rides the operative default -> primary tier.
    let open_item = json["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["kind"] == "open_input_request")
        .unwrap();
    assert_eq!(open_item["tier"], "primary");
    assert_eq!(open_item["reasonCode"], "insufficient_evidence");
    assert!(
        open_item["id"]
            .as_str()
            .unwrap()
            .starts_with("open_input_request:input-request:sha256:")
    );
}

#[test]
fn attention_list_scopes_by_revision_short_id() {
    let repo = modified_repo();
    let revision_id = store_with_attention(&repo);

    let hex = revision_id.rsplit(':').next().unwrap();
    let short = format!("rev:{}", &hex[..8]);

    let output = shore([
        "attention",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
        "--revision",
        &short,
        "--format",
        "json",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);

    // The short id resolves to the full revision id and echoes in filters.
    assert_eq!(json["filters"]["revision"], revision_id.as_str());

    // Every anchored item names the scoped revision; the ambiguity and the open
    // request both anchor to it, so both survive scoping.
    for item in json["items"].as_array().unwrap() {
        if let Some(rev) = item["revisionId"].as_str() {
            assert_eq!(rev, revision_id.as_str());
        }
    }
    let kinds: Vec<&str> = json["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"open_input_request"));
    assert!(kinds.contains(&"ambiguous_assessment"));
}

#[test]
fn text_digest_renders_counts_items_and_empty_state() {
    let repo = modified_repo();
    let repo_arg = repo.path().to_str().unwrap().to_owned();
    shore(["capture", "--repo", &repo_arg]);
    for (title, mode) in [
        ("Operative gate", "operative"),
        ("Advisory question", "advisory"),
    ] {
        let opened = shore([
            "input-request",
            "open",
            "--repo",
            &repo_arg,
            "--track",
            "agent:codex",
            "--title",
            title,
            "--reason",
            "manual-decision-required",
            "--mode",
            mode,
        ]);
        assert!(
            opened.status.success(),
            "stderr:\n{}",
            String::from_utf8_lossy(&opened.stderr)
        );
    }

    let output = shore(["attention", "list", "--repo", &repo_arg, "--format", "text"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // A count headline, never raw JSON.
    assert!(!stdout.contains("\"schema\""), "stdout:\n{stdout}");
    assert!(stdout.contains("attention"), "stdout:\n{stdout}");
    // Each line names the kebab kind label and a shortened anchor id.
    assert!(stdout.contains("open-input-request"), "stdout:\n{stdout}");
    assert!(
        !stdout.contains("open_input_request"),
        "the digest uses the kebab spelling, not the wire snake_case:\n{stdout}"
    );
    // Primary items list before secondary items.
    let primary = stdout.find("primary").expect("a primary marker");
    let secondary = stdout.find("secondary").expect("a secondary marker");
    assert!(primary < secondary, "primary before secondary:\n{stdout}");

    // Empty store: never silent.
    let empty = GitRepo::new();
    empty.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    empty.commit_all("base");
    empty.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    shore(["capture", "--repo", empty.path().to_str().unwrap()]);
    let empty_out = shore([
        "attention",
        "list",
        "--repo",
        empty.path().to_str().unwrap(),
        "--format",
        "text",
    ]);
    let empty_stdout = String::from_utf8_lossy(&empty_out.stdout);
    assert!(
        empty_stdout
            .to_lowercase()
            .contains("nothing needs attention"),
        "stdout:\n{empty_stdout}"
    );
    assert!(!empty_stdout.trim().is_empty());
}

fn modified_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo
}

#[test]
fn accepted_after_failed_validation_clears_the_attention_item() {
    let repo = modified_repo();
    let repo_arg = repo.path().to_str().unwrap().to_owned();
    let capture = shore(["capture", "--repo", &repo_arg]);
    assert!(
        capture.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&capture.stderr)
    );
    let revision_id = parse_json(&capture.stdout)["revision"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let failed = shore([
        "validation",
        "add",
        "--repo",
        &repo_arg,
        "--track",
        "agent:codex",
        "--check-name",
        "red proof",
        "--status",
        "failed",
        "--revision",
        &revision_id,
    ]);
    assert!(
        failed.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&failed.stderr)
    );

    // Before the judgment: the failure claims attention.
    let before = parse_json(&shore(["attention", "list", "--repo", &repo_arg]).stdout);
    assert_eq!(before["items"].as_array().unwrap().len(), 1);

    let accepted = shore([
        "assessment",
        "add",
        "--repo",
        &repo_arg,
        "--track",
        "agent:codex",
        "--assessment",
        "accepted",
        "--revision",
        &revision_id,
    ]);
    assert!(
        accepted.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&accepted.stderr)
    );

    // After: the judgment subsumes the failure; the queue is empty.
    let after = parse_json(&shore(["attention", "list", "--repo", &repo_arg]).stdout);
    assert_eq!(after["items"].as_array().unwrap().len(), 0);
}

#[test]
fn assessed_successor_clears_the_stale_assessment_item() {
    let repo = modified_repo();
    let repo_arg = repo.path().to_str().unwrap().to_owned();
    let first = shore(["capture", "--repo", &repo_arg]);
    assert!(
        first.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_revision = parse_json(&first.stdout)["revision"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let accepted = shore([
        "assessment",
        "add",
        "--repo",
        &repo_arg,
        "--track",
        "agent:codex",
        "--assessment",
        "accepted",
        "--revision",
        &first_revision,
    ]);
    assert!(
        accepted.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&accepted.stderr)
    );

    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    let second = shore([
        "capture",
        "--repo",
        &repo_arg,
        "--supersedes",
        &first_revision,
    ]);
    assert!(
        second.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_revision = parse_json(&second.stdout)["revision"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // The accepted decision now anchors to a superseded revision whose
    // successor is unjudged: the stale_assessment item claims attention.
    let before = parse_json(&shore(["attention", "list", "--repo", &repo_arg]).stdout);
    let kinds: Vec<String> = before["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["kind"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(kinds, vec!["stale_assessment".to_owned()]);

    let re_judged = shore([
        "assessment",
        "add",
        "--repo",
        &repo_arg,
        "--track",
        "agent:codex",
        "--assessment",
        "accepted",
        "--revision",
        &second_revision,
    ]);
    assert!(
        re_judged.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&re_judged.stderr)
    );

    // The successor has been re-judged: the stale decision is resolved.
    let after = parse_json(&shore(["attention", "list", "--repo", &repo_arg]).stdout);
    assert_eq!(after["items"].as_array().unwrap().len(), 0);
}
