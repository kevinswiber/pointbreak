mod support;

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore;

#[test]
fn review_unit_list_emits_v1_json_with_freshness_metadata() {
    let repo = modified_repo();
    let capture =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);

    let output = shore([
        "review",
        "unit",
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

    assert_eq!(json["schema"], "shore.review-unit-list");
    assert_eq!(json["version"], 1);
    assert!(
        json["eventSetHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert_eq!(json["eventCount"], 1);
    assert_eq!(json["reviewUnitCount"], 1);

    let entry = &json["entries"][0];
    assert_eq!(entry["reviewUnitId"], capture["reviewUnit"]["id"]);
    assert!(!entry["capturedAt"].as_str().unwrap().is_empty());
    assert!(entry["revisionId"].as_str().unwrap().starts_with("rev:"));
    assert!(entry["snapshotId"].as_str().unwrap().starts_with("snap:"));
    assert!(entry["source"].is_object());
    assert!(entry["base"].is_object());
    assert!(entry["target"].is_object());
    assert!(
        entry["snapshotArtifactContentHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
}

#[test]
fn review_unit_list_does_not_expose_storage_paths() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "unit",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json = parse_json(&output.stdout);

    assert!(!stdout.contains(".shore/events"));
    assert!(!stdout.contains("artifacts/"));
    assert!(json.get("statePath").is_none());
    assert!(json["entries"][0].get("payloadHash").is_none());
    assert!(json["entries"][0].get("eventId").is_none());
}

#[test]
fn review_unit_list_pretty_prints() {
    let repo = modified_repo();
    shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);

    let output = shore([
        "review",
        "unit",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
        "--pretty",
    ]);

    assert!(String::from_utf8_lossy(&output.stdout).starts_with("{\n"));
}

#[test]
fn review_unit_list_returns_multiple_entries_in_capture_order() {
    let repo = modified_repo();
    let first =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    let second =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);

    let output = shore([
        "review",
        "unit",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);
    let json = parse_json(&output.stdout);
    let entries = json["entries"].as_array().unwrap();

    assert_ne!(first["reviewUnit"]["id"], second["reviewUnit"]["id"]);
    assert_eq!(json["reviewUnitCount"], 2);
    assert_eq!(entries.len(), 2);
    let ids: Vec<&str> = entries
        .iter()
        .map(|entry| entry["reviewUnitId"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&first["reviewUnit"]["id"].as_str().unwrap()));
    assert!(ids.contains(&second["reviewUnit"]["id"].as_str().unwrap()));
    assert!(
        entries[0]["capturedAt"].as_str().unwrap() <= entries[1]["capturedAt"].as_str().unwrap()
    );
}

#[test]
fn review_unit_list_succeeds_without_events() {
    let repo = GitRepo::new();

    let output = shore([
        "review",
        "unit",
        "list",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);
    let json = parse_json(&output.stdout);

    assert!(output.status.success());
    assert_eq!(json["eventCount"], 0);
    assert_eq!(json["reviewUnitCount"], 0);
    assert!(json["entries"].as_array().unwrap().is_empty());
}

#[test]
fn review_unit_list_reads_imported_facts_from_linked_store() {
    let fixture = CloneWorktreeFixture::new();
    fs::write(fixture.seed.join("README.md"), "changed in seed\n").unwrap();
    let capture = parse_json(
        &shore([
            "review",
            "capture",
            "--repo",
            fixture.seed.to_str().unwrap(),
        ])
        .stdout,
    );

    let link = shore(["store", "link", "--repo", fixture.seed.to_str().unwrap()]);
    assert!(
        link.status.success(),
        "link stderr:\n{}",
        String::from_utf8_lossy(&link.stderr)
    );
    run_git_os(
        fixture.main.path(),
        [
            OsString::from("worktree"),
            OsString::from("remove"),
            OsString::from("--force"),
            fixture.seed.as_os_str().to_owned(),
        ],
    );
    let reader = fixture.add_worktree("reader");
    let reader_link = shore(["store", "link", "--repo", reader.to_str().unwrap()]);
    assert!(
        reader_link.status.success(),
        "reader link stderr:\n{}",
        String::from_utf8_lossy(&reader_link.stderr)
    );
    assert!(!reader.join(".shore/events").exists());

    let output = shore(["review", "unit", "list", "--repo", reader.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "list stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json = parse_json(stdout.as_bytes());

    assert_eq!(json["eventCount"], 1);
    assert_eq!(json["reviewUnitCount"], 1);
    assert_eq!(
        json["entries"][0]["reviewUnitId"],
        capture["reviewUnit"]["id"]
    );
    assert!(json["diagnostics"].as_array().unwrap().is_empty());
    assert!(!stdout.contains(".git"));
    assert!(!stdout.contains(".shore"));
}

#[test]
fn review_unit_list_keeps_ambiguous_current_diagnostic_from_linked_store() {
    let fixture = CloneWorktreeFixture::new();
    fs::write(fixture.seed.join("README.md"), "changed once\n").unwrap();
    let first = parse_json(
        &shore([
            "review",
            "capture",
            "--repo",
            fixture.seed.to_str().unwrap(),
        ])
        .stdout,
    );
    fs::write(fixture.seed.join("README.md"), "changed twice\n").unwrap();
    let second = parse_json(
        &shore([
            "review",
            "capture",
            "--repo",
            fixture.seed.to_str().unwrap(),
        ])
        .stdout,
    );

    let link = shore(["store", "link", "--repo", fixture.seed.to_str().unwrap()]);
    assert!(
        link.status.success(),
        "link stderr:\n{}",
        String::from_utf8_lossy(&link.stderr)
    );
    let reader = fixture.add_worktree("reader");
    let reader_link = shore(["store", "link", "--repo", reader.to_str().unwrap()]);
    assert!(
        reader_link.status.success(),
        "reader link stderr:\n{}",
        String::from_utf8_lossy(&reader_link.stderr)
    );

    let output = shore(["review", "unit", "list", "--repo", reader.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "list stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    let ids = json["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["reviewUnitId"].as_str().unwrap())
        .collect::<Vec<_>>();

    assert_ne!(first["reviewUnit"]["id"], second["reviewUnit"]["id"]);
    assert_eq!(json["eventCount"], 2);
    assert_eq!(json["reviewUnitCount"], 2);
    assert!(ids.contains(&first["reviewUnit"]["id"].as_str().unwrap()));
    assert!(ids.contains(&second["reviewUnit"]["id"].as_str().unwrap()));
    assert!(
        json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| {
                diagnostic["code"].as_str() == Some("ambiguous_current_review_unit")
            }),
        "expected ambiguous current ReviewUnit diagnostic"
    );
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

struct CloneWorktreeFixture {
    main: GitRepo,
    _worktree_parent: tempfile::TempDir,
    seed: PathBuf,
}

impl CloneWorktreeFixture {
    fn new() -> Self {
        let main = GitRepo::new();
        main.write("README.md", "base\n");
        main.commit_all("base");

        let worktree_parent = tempfile::tempdir().expect("create worktree parent");
        let seed = worktree_parent.path().join("seed");
        add_worktree(main.path(), &seed, "seed");

        Self {
            main,
            _worktree_parent: worktree_parent,
            seed,
        }
    }

    fn add_worktree(&self, branch: &str) -> PathBuf {
        let path = self._worktree_parent.path().join(branch);
        add_worktree(self.main.path(), &path, branch);
        path
    }
}

fn add_worktree(repo: &Path, path: &Path, branch: &str) {
    run_git_os(
        repo,
        [
            OsString::from("worktree"),
            OsString::from("add"),
            OsString::from("-b"),
            OsString::from(branch),
            path.as_os_str().to_owned(),
        ],
    );
}

fn run_git_os<I>(cwd: &Path, args: I)
where
    I: IntoIterator<Item = OsString>,
{
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|error| panic!("run git in {}: {error}", cwd.display()));
    assert!(
        output.status.success(),
        "git failed in {}\nstdout:\n{}\nstderr:\n{}",
        cwd.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
