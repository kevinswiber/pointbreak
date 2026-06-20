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
    assert_eq!(json["eventCount"], 2);
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

    assert!(!stdout.contains(".shore/data/events"));
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
    assert!(!reader.join(".shore/data/events").exists());

    let output = shore(["review", "unit", "list", "--repo", reader.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "list stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json = parse_json(stdout.as_bytes());

    assert_eq!(json["eventCount"], 2);
    assert_eq!(json["reviewUnitCount"], 1);
    assert_eq!(
        json["entries"][0]["reviewUnitId"],
        capture["reviewUnit"]["id"]
    );
    assert!(json["diagnostics"].as_array().unwrap().is_empty());
    assert!(!stdout.contains(".git"));
    assert!(!stdout.contains(".shore/data"));
}

#[test]
fn review_unit_list_omits_ambient_ambiguous_current_diagnostic_from_linked_store() {
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
    assert_eq!(json["eventCount"], 4);
    assert_eq!(json["reviewUnitCount"], 2);
    assert!(ids.contains(&first["reviewUnit"]["id"].as_str().unwrap()));
    assert!(ids.contains(&second["reviewUnit"]["id"].as_str().unwrap()));
    assert!(
        !json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| {
                diagnostic["code"].as_str() == Some("ambiguous_current_review_unit")
            }),
        "routine ReviewUnit list should not emit ambient current ambiguity diagnostics"
    );
}

#[test]
fn unit_list_renders_commit_range_source_without_paths() {
    let repo = support::committed_repo();
    shore([
        "review",
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--base",
        "HEAD~1",
    ]);

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
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json = parse_json(&output.stdout);

    let entry = &json["entries"][0];
    assert_eq!(entry["source"]["kind"], "git_commit_range");
    assert_eq!(entry["source"]["mode"], "base_tree_to_target_tree");
    assert_eq!(entry["base"]["kind"], "git_commit");
    assert_eq!(entry["target"]["kind"], "git_commit");
    assert!(
        !stdout.contains("worktreeRoot"),
        "range capture unit list must not expose a worktree path"
    );
}

#[test]
fn unit_list_hides_orphans_by_default_and_surfaces_with_flags() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");

    // A range capture anchored to a commit on a soon-deleted branch → orphan.
    repo.git(["checkout", "-b", "feature"]);
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.commit_all("feature work");
    let orphan = parse_json(
        &shore([
            "review",
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--base",
            "main",
        ])
        .stdout,
    );
    let orphan_id = orphan["reviewUnit"]["id"].as_str().unwrap().to_owned();
    repo.git(["checkout", "main"]);
    repo.git(["branch", "-D", "feature"]);

    // A floating worktree capture on main → never hidden.
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    let floating =
        parse_json(&shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    let floating_id = floating["reviewUnit"]["id"].as_str().unwrap().to_owned();
    assert_ne!(orphan_id, floating_id);

    // Default: the orphan is hidden, the floating capture remains.
    let default_ids = unit_list_ids(&repo, &[]);
    assert!(default_ids.contains(&floating_id));
    assert!(!default_ids.contains(&orphan_id));

    // --all surfaces both.
    let all_ids = unit_list_ids(&repo, &["--all"]);
    assert!(all_ids.contains(&orphan_id));
    assert!(all_ids.contains(&floating_id));

    // --orphans surfaces only the orphan.
    assert_eq!(
        unit_list_ids(&repo, &["--orphans"]),
        vec![orphan_id.clone()]
    );

    // --orphans takes precedence over --all.
    assert_eq!(
        unit_list_ids(&repo, &["--orphans", "--all"]),
        vec![orphan_id]
    );
}

#[test]
fn unit_list_attaches_merge_status_and_accepts_integration_and_worktree_flags() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.commit_all("second");
    let repo_arg = repo.path().to_str().unwrap();

    // A range capture anchored to HEAD (a live tip) reads "open".
    let range =
        parse_json(&shore(["review", "capture", "--repo", repo_arg, "--base", "HEAD~1"]).stdout);
    let range_id = range["reviewUnit"]["id"].as_str().unwrap().to_owned();

    // A floating worktree capture reads "unknown"; its worktree path lets it
    // survive the worktree-identity scope.
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    let worktree = parse_json(&shore(["review", "capture", "--repo", repo_arg]).stdout);
    let worktree_id = worktree["reviewUnit"]["id"].as_str().unwrap().to_owned();

    // Default list: each entry carries a structural merge-status.
    let default = parse_json(&shore(["review", "unit", "list", "--repo", repo_arg]).stdout);
    let status_for = |id: &str| -> String {
        default["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["reviewUnitId"] == id)
            .unwrap()["mergeStatus"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(status_for(&range_id), "open");
    assert_eq!(status_for(&worktree_id), "unknown");

    // --integration-ref and --worktree parse; the worktree-identity scope keeps
    // the worktree capture.
    let scoped = shore([
        "review",
        "unit",
        "list",
        "--repo",
        repo_arg,
        "--integration-ref",
        "refs/heads/main",
        "--worktree",
        repo_arg,
    ]);
    assert!(
        scoped.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&scoped.stderr)
    );
    let scoped_ids: Vec<String> = parse_json(&scoped.stdout)["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["reviewUnitId"].as_str().unwrap().to_owned())
        .collect();
    assert!(scoped_ids.contains(&worktree_id));
}

/// Run `review unit list` with extra flags and return the entry ids in order.
fn unit_list_ids(repo: &GitRepo, extra: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "review".to_owned(),
        "unit".to_owned(),
        "list".to_owned(),
        "--repo".to_owned(),
        repo.path().to_str().unwrap().to_owned(),
    ];
    args.extend(extra.iter().map(|flag| (*flag).to_owned()));
    let output = shore(args);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    parse_json(&output.stdout)["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["reviewUnitId"].as_str().unwrap().to_owned())
        .collect()
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
