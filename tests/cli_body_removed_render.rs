mod support;

use std::path::Path;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore;

/// A repo with a committed base and an uncommitted change, so `shore capture`
/// has a HEAD -> working-tree diff to capture.
fn modified_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("README.md", "base\n");
    repo.commit_all("base");
    repo.write("README.md", "changed\n");
    repo
}

fn parse_json(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).expect("stdout is json")
}

fn assert_success(output: &std::process::Output, what: &str) {
    assert!(
        output.status.success(),
        "{what} stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Capture plus one externalized (> 4096-byte) observation body; returns the
/// revision id and the body's normalized content hash (the removal key).
fn capture_with_externalized_observation_body(repo: &Path) -> (String, String) {
    let arg = repo.to_str().unwrap();
    let capture = shore(["capture", "--repo", arg]);
    assert_success(&capture, "capture");
    let capture = parse_json(&capture.stdout);
    let revision_id = capture["revision"]["id"].as_str().unwrap().to_owned();

    let body = "x".repeat(5000);
    let observation = shore([
        "review",
        "observation",
        "add",
        "--repo",
        arg,
        "--track",
        "agent:codex",
        "--title",
        "a large observation",
        "--body",
        &body,
    ]);
    assert_success(&observation, "observation add");
    let observation = parse_json(&observation.stdout);
    let body_hash = observation["bodyContentHash"]
        .as_str()
        .expect("a >4096-byte body is stored as a note artifact")
        .to_owned();
    (revision_id, body_hash)
}

fn remove_revision(repo: &Path, revision_id: &str) {
    let output = shore([
        "store",
        "remove",
        "--repo",
        repo.to_str().unwrap(),
        "--revision",
        revision_id,
    ]);
    assert_success(&output, "store remove");
}

fn compact(repo: &Path) {
    let output = shore([
        "store",
        "compact",
        "--repo",
        repo.to_str().unwrap(),
        "--yes",
    ]);
    assert_success(&output, "store compact");
}

fn observation_entry(document: &Value) -> &Value {
    document["observations"]
        .as_array()
        .expect("observations array")
        .first()
        .expect("one observation")
}

fn has_diagnostic(document: &Value, code: &str, hash: &str) -> bool {
    document["diagnostics"]
        .as_array()
        .expect("diagnostics array")
        .iter()
        .any(|d| d["code"] == code && d["message"].as_str().unwrap_or_default().contains(hash))
}

#[test]
fn swept_body_renders_physically_removed_across_read_surfaces() {
    let repo = modified_repo();
    let arg = repo.path().to_str().unwrap();
    let (revision_id, body_hash) = capture_with_externalized_observation_body(repo.path());
    remove_revision(repo.path(), &revision_id);
    compact(repo.path());

    // review show: explained absence, not a hard error.
    let show = shore(["review", "show", "--repo", arg, "--include-body"]);
    assert_success(&show, "review show");
    let show = parse_json(&show.stdout);
    let observation = observation_entry(&show);
    assert!(observation.get("body").is_none(), "no fabricated body");
    assert_eq!(observation["bodyContentState"], "physically_removed");
    assert_eq!(observation["bodyContentHash"], body_hash.as_str());
    assert!(has_diagnostic(
        &show,
        "body_content_physically_removed",
        &body_hash
    ));

    // observation list: same explained state on the leaf surface.
    let list = shore([
        "review",
        "observation",
        "list",
        "--repo",
        arg,
        "--include-body",
    ]);
    assert_success(&list, "observation list");
    let list = parse_json(&list.stdout);
    let listed = observation_entry(&list);
    assert_eq!(listed["bodyContentState"], "physically_removed");
    assert!(has_diagnostic(
        &list,
        "body_content_physically_removed",
        &body_hash
    ));

    // review history: the entry carries the state; the read survives.
    let history = shore(["review", "history", "--repo", arg, "--include-body"]);
    assert_success(&history, "review history");
    let history = parse_json(&history.stdout);
    let entry = history["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|entry| entry["summary"]["kind"] == "review_observation_recorded")
        .expect("observation history entry");
    assert!(entry["summary"].get("body").is_none());
    assert_eq!(entry["summary"]["bodyContentState"], "physically_removed");
    assert!(has_diagnostic(
        &history,
        "body_content_physically_removed",
        &body_hash
    ));
}

#[test]
fn unswept_removal_renders_suppressed_present() {
    let repo = modified_repo();
    let arg = repo.path().to_str().unwrap();
    let (revision_id, body_hash) = capture_with_externalized_observation_body(repo.path());
    remove_revision(repo.path(), &revision_id);

    let show = shore(["review", "show", "--repo", arg, "--include-body"]);
    assert_success(&show, "review show");
    let show = parse_json(&show.stdout);
    let observation = observation_entry(&show);
    assert!(observation.get("body").is_none());
    assert_eq!(observation["bodyContentState"], "suppressed_present");
    assert!(has_diagnostic(
        &show,
        "body_content_suppressed_present",
        &body_hash
    ));
}

#[test]
fn missing_blob_without_removal_still_hard_errors() {
    let repo = modified_repo();
    let arg = repo.path().to_str().unwrap();
    let (_revision_id, body_hash) = capture_with_externalized_observation_body(repo.path());
    let hex = body_hash.strip_prefix("sha256:").unwrap();
    std::fs::remove_file(
        support::common_dir_store(repo.path())
            .join("artifacts")
            .join("notes")
            .join(format!("{hex}.json")),
    )
    .expect("delete the note blob without a removal event");

    let show = shore(["review", "show", "--repo", arg, "--include-body"]);

    assert!(
        !show.status.success(),
        "absent bytes without an operative removal must keep the hard error"
    );
    assert!(
        String::from_utf8_lossy(&show.stderr).contains("import referenced artifacts"),
        "stderr:\n{}",
        String::from_utf8_lossy(&show.stderr)
    );
}

#[test]
fn wire_stays_silent_without_removal() {
    let repo = modified_repo();
    let arg = repo.path().to_str().unwrap();
    let (_revision_id, _body_hash) = capture_with_externalized_observation_body(repo.path());

    let show = shore(["review", "show", "--repo", arg, "--include-body"]);
    assert_success(&show, "review show");
    let list = shore([
        "review",
        "observation",
        "list",
        "--repo",
        arg,
        "--include-body",
    ]);
    assert_success(&list, "observation list");

    for (name, output) in [("show", &show.stdout), ("list", &list.stdout)] {
        let text = String::from_utf8_lossy(output);
        assert!(
            !text.contains("ContentState"),
            "{name} output must omit state fields without a removal:\n{text}"
        );
    }
}
