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
            .starts_with("rev:sha256:")
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
            .starts_with("obj:")
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
    assert_eq!(json["eventsCreatedByType"]["work_object_proposed"], 1);
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
fn review_capture_writes_through_to_the_shared_store_without_a_link_step() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");

    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    let capture = shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    assert!(
        capture.status.success(),
        "capture stderr:\n{}",
        String::from_utf8_lossy(&capture.stderr)
    );
    let capture_stdout = String::from_utf8(capture.stdout).unwrap();
    let _capture_json = parse_json(capture_stdout.as_bytes());

    // No storage paths leak into the capture JSON.
    assert!(!capture_stdout.contains(".git"));
    assert!(!capture_stdout.contains(".shore/data"));

    // The capture landed in the shared common-dir store with no link step.
    assert!(
        common_dir_store(&repo).join("events").is_dir(),
        "the capture lands in the shared common-dir store"
    );

    // `store status` sees the captured event in place — eventCount reflects the
    // write-through capture, with the single-store view (no clone/family refs).
    let status = shore(["store", "status", "--repo", repo.path().to_str().unwrap()]);
    assert!(
        status.status.success(),
        "status stderr:\n{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status_json = parse_json(&status.stdout);
    assert_eq!(status_json["mode"], "local");
    assert_eq!(status_json["inventory"]["eventCount"], 2);
    assert!(status_json.get("cloneRef").is_none() || status_json["cloneRef"].is_null());
}

#[test]
fn capture_preserves_inline_rows_for_normal_added_file() {
    let repo = bounded_added_file_repo();
    let _capture =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);

    let snapshots_dir = common_dir_store(&repo).join("artifacts/snapshots");
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

#[test]
fn capture_with_base_captures_committed_range_on_clean_worktree() {
    let repo = committed_repo();
    let head_oid = rev(&repo, "HEAD");

    let output = shore([
        "review",
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--base",
        "HEAD~1",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json = parse_json(stdout.as_bytes());
    assert_eq!(json["reviewUnit"]["base"]["kind"], "git_commit");
    assert_eq!(json["reviewUnit"]["target"]["kind"], "git_commit");
    assert_eq!(json["reviewUnit"]["target"]["commitOid"], head_oid);
    assert!(
        !stdout.contains("worktreeRoot"),
        "range capture document must not carry a worktree path: {stdout}"
    );
}

#[test]
fn capture_with_base_and_target_pins_both_endpoints() {
    let repo = committed_repo();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    repo.commit_all("third");
    let first_oid = rev(&repo, "HEAD~2");
    let second_oid = rev(&repo, "HEAD~1");
    let head_oid = rev(&repo, "HEAD");

    let json = parse_json(
        &shore([
            "review",
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--base",
            &first_oid,
            "--target",
            "HEAD~1",
        ])
        .stdout,
    );

    assert_eq!(json["reviewUnit"]["base"]["commitOid"], first_oid);
    assert_eq!(json["reviewUnit"]["target"]["commitOid"], second_oid);
    assert_ne!(
        json["reviewUnit"]["target"]["commitOid"], head_oid,
        "target must not default to HEAD when --target is given"
    );
}

#[test]
fn capture_with_dirty_worktree_and_base_ignores_worktree_state() {
    let repo = committed_repo();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 999 }\n");
    repo.write("untracked.txt", "untracked\n");

    let capture = parse_json(
        &shore([
            "review",
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--base",
            "HEAD~1",
        ])
        .stdout,
    );
    let snapshot_id =
        shoreline::model::ObjectId::new(capture["reviewUnit"]["snapshotId"].as_str().unwrap());

    let artifact = shoreline::session::read_snapshot_artifact(repo.path(), &snapshot_id)
        .expect("snapshot artifact for the range capture");
    let paths: Vec<&str> = artifact
        .snapshot
        .files
        .iter()
        .filter_map(|file| file.new_path.as_deref())
        .collect();
    assert_eq!(paths, vec!["src/lib.rs"]);
    assert!(!paths.contains(&"untracked.txt"));
}

#[test]
fn capture_target_without_base_is_rejected() {
    let repo = committed_repo();

    let output = shore([
        "review",
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--target",
        "HEAD",
    ]);

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--target requires --base"),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn capture_with_unresolvable_base_fails_honestly() {
    let repo = committed_repo();

    let output = shore([
        "review",
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--base",
        "no-such-rev",
    ]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no-such-rev"), "stderr:\n{stderr}");
    assert!(stderr.contains("commit"), "stderr:\n{stderr}");
}

#[test]
fn capture_with_non_commit_rev_fails_honestly() {
    let repo = committed_repo();

    let output = shore([
        "review",
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--base",
        "HEAD:src/lib.rs",
    ]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("HEAD:src/lib.rs"), "stderr:\n{stderr}");
}

#[test]
fn capture_without_base_keeps_worktree_behavior() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let show = parse_json(
        &shore([
            "review",
            "unit",
            "show",
            "--repo",
            repo.path().to_str().unwrap(),
        ])
        .stdout,
    );

    assert_eq!(show["reviewUnit"]["source"]["kind"], "git_worktree");
    assert_eq!(show["reviewUnit"]["target"]["kind"], "git_working_tree");
}

#[test]
fn capture_with_base_and_lineage_attaches_round() {
    let repo = committed_repo();

    let json = parse_json(
        &shore([
            "review",
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--base",
            "HEAD~1",
            "--lineage",
            "review-unit-lineage:random:test",
        ])
        .stdout,
    );

    assert_eq!(
        json["lineageAttach"]["eventsCreatedByType"]["review_unit_lineage_declared"],
        1
    );
    assert_eq!(
        json["lineageAttach"]["eventsCreatedByType"]["review_unit_lineage_round_recorded"],
        1
    );
}

#[test]
fn capture_with_base_twice_reports_existing_event() {
    let repo = committed_repo();

    let first = parse_json(
        &shore([
            "review",
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--base",
            "HEAD~1",
        ])
        .stdout,
    );
    let second = parse_json(
        &shore([
            "review",
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--base",
            "HEAD~1",
        ])
        .stdout,
    );

    assert_eq!(first["reviewUnit"]["id"], second["reviewUnit"]["id"]);
    assert_eq!(second["eventsCreated"], 0);
    // The capture event plus the auto-recorded HEAD-tipping ref association.
    assert_eq!(second["eventsExisting"], 2);
}

fn committed_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.commit_all("change");
    repo
}

fn rev(repo: &GitRepo, rev: &str) -> String {
    repo.git(["rev-parse", rev]).stdout.trim().to_owned()
}

/// The shared common-dir store a clone resolves by default
/// (`<git-common-dir>/shore`). Every non-ephemeral worktree reads and writes
/// here, so post-capture store assertions look here instead of the raw
/// worktree-local `.shore/data`.
fn common_dir_store(repo: &GitRepo) -> std::path::PathBuf {
    let common_dir = repo
        .git(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .stdout
        .trim()
        .to_owned();
    std::path::Path::new(&common_dir).join("shore")
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
