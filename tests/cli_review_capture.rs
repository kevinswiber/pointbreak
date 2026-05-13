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
