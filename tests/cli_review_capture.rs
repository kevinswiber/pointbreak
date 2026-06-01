use std::process::Command;

use serde_json::Value;

#[allow(dead_code)]
#[path = "support/git_repo.rs"]
mod git_repo;

use git_repo::GitRepo;

#[test]
fn review_capture_creates_review_unit_from_subdir() {
    let repo = modified_repo();
    let subdir = repo.path().join("src");

    let output = shore(["review", "capture", "--repo", subdir.to_str().unwrap()]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    assert_eq!(json["schema"], "shore.review-capture");
    assert_eq!(json["version"], 1);
    assert!(
        json["reviewUnit"]["id"]
            .as_str()
            .unwrap()
            .starts_with("review-unit:sha256:")
    );
    assert!(
        json["reviewUnit"]["revisionId"]
            .as_str()
            .unwrap()
            .starts_with("rev:")
    );
    assert!(
        json["reviewUnit"]["snapshotId"]
            .as_str()
            .unwrap()
            .starts_with("snap:")
    );
    assert_eq!(json["reviewUnit"]["base"]["kind"], "git_commit");
    assert_eq!(json["reviewUnit"]["target"]["kind"], "git_working_tree");
    assert!(
        json["reviewUnit"]["snapshotArtifactContentHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert!(json.get("statePath").is_none());
    assert!(json.get("snapshotArtifactPath").is_none());
    assert_eq!(json["eventsCreatedByType"]["review_unit_captured"], 1);
}

#[test]
fn review_capture_is_idempotent_for_unchanged_diff() {
    let repo = modified_repo();

    let first =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    let second =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);

    assert_eq!(first["reviewUnit"]["id"], second["reviewUnit"]["id"]);
    assert_eq!(second["eventsCreated"], 0);
    assert!(second["eventsExisting"].as_u64().unwrap() >= 1);
}

#[test]
fn review_capture_changes_when_untracked_content_changes() {
    let repo = modified_repo();

    let first =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    repo.write("untracked.txt", "new review content\n");
    let second =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);

    assert_ne!(first["reviewUnit"]["id"], second["reviewUnit"]["id"]);
}

#[test]
fn review_capture_on_linked_store_stays_batch_only_with_guidance() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");

    let link = shore(["store", "link", "--repo", repo.path().to_str().unwrap()]);
    assert!(
        link.status.success(),
        "link stderr:\n{}",
        String::from_utf8_lossy(&link.stderr)
    );

    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    let capture = shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    assert!(
        capture.status.success(),
        "capture stderr:\n{}",
        String::from_utf8_lossy(&capture.stderr)
    );
    let capture_stdout = String::from_utf8(capture.stdout).unwrap();
    let capture_json = parse_json(capture_stdout.as_bytes());

    let diagnostics = capture_json["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic["code"].as_str() == Some("clone_local_capture_batch_only")
                && diagnostic["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("shore store link"))
        }),
        "expected batch-only linked capture guidance in {diagnostics:?}"
    );
    assert!(!capture_stdout.contains(".git"));
    assert!(!capture_stdout.contains(".shore"));

    let status = shore(["store", "status", "--repo", repo.path().to_str().unwrap()]);
    assert!(
        status.status.success(),
        "status stderr:\n{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status_json = parse_json(&status.stdout);
    assert_eq!(status_json["mode"], "linked");
    assert_eq!(status_json["inventory"]["eventCount"], 0);
    assert!(repo.path().join(".shore/events").is_dir());
}

#[test]
fn capture_preserves_inline_rows_for_normal_added_file() {
    let repo = bounded_added_file_repo();
    let _capture =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);

    let snapshots_dir = shoreline::session::shore_dir_for_repo(repo.path())
        .expect("shore dir resolves")
        .join("artifacts/snapshots");
    let artifact_path = std::fs::read_dir(&snapshots_dir)
        .expect("snapshots dir exists")
        .filter_map(|entry| entry.ok())
        .find(|entry| entry.path().extension().and_then(|s| s.to_str()) == Some("json"))
        .map(|entry| entry.path())
        .expect("at least one snapshot artifact");
    let artifact: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&artifact_path).expect("read artifact"))
            .expect("artifact JSON parses");

    let files = artifact["snapshot"]["files"]
        .as_array()
        .expect("files array");
    let added = files
        .iter()
        .find(|f| f["new_path"].as_str() == Some("notes/added.txt"))
        .expect("captured added file present");
    let hunks = added["hunks"].as_array().expect("hunks array");

    // V1: every captured row stays inline in the artifact JSON; no elision.
    assert_eq!(hunks.len(), 1);
    let rows = hunks[0]["rows"].as_array().expect("rows array");
    assert_eq!(rows.len(), 50);
    let metadata = added["metadata_rows"]
        .as_array()
        .expect("metadata_rows array");
    assert!(metadata.is_empty());
}

fn bounded_added_file_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("README.md", "base\n");
    repo.commit_all("base");
    let body = (1..=50).map(|n| format!("line {n}\n")).collect::<String>();
    repo.write("notes/added.txt", body);
    repo
}

fn shore<I, S>(args: I) -> std::process::Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_shore"))
        .args(args)
        .env_remove("SHORE_LOG")
        .env_remove("RUST_LOG")
        .output()
        .expect("run shore binary")
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
