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
