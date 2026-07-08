use std::process::Command;

use serde_json::Value;

#[allow(dead_code)]
#[path = "support/git_repo.rs"]
mod git_repo;

use git_repo::GitRepo;

#[test]
fn review_capture_creates_revision_from_subdir() {
    let repo = modified_repo();
    let subdir = repo.path().join("src");

    let output = shore(["capture", "--repo", subdir.to_str().unwrap()]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    assert_eq!(json["schema"], "pointbreak.review-capture");
    assert_eq!(json["version"], 1);
    assert!(
        json["revision"]["id"]
            .as_str()
            .unwrap()
            .starts_with("rev:sha256:")
    );
    assert!(
        json["revision"]["revisionId"]
            .as_str()
            .unwrap()
            .starts_with("rev:")
    );
    assert!(
        json["revision"]["objectId"]
            .as_str()
            .unwrap()
            .starts_with("obj:")
    );
    assert_eq!(json["revision"]["base"]["kind"], "git_commit");
    assert_eq!(json["revision"]["target"]["kind"], "git_working_tree");
    assert!(
        json["revision"]["objectArtifactContentHash"]
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

    let first = parse_json(&shore(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    let second = parse_json(
        &shore([
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--allow-empty",
        ])
        .stdout,
    );

    assert_eq!(first["revision"]["id"], second["revision"]["id"]);
    assert_eq!(second["eventsCreated"], 0);
    assert!(second["eventsExisting"].as_u64().unwrap() >= 1);
}

#[test]
fn review_capture_rejects_empty_default_capture_without_allow_empty() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");

    let output = shore(["capture", "--repo", repo.path().to_str().unwrap()]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("capture produced no changed files")
            && stderr.contains("--allow-empty")
            && stderr.contains("--include-untracked"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn review_capture_allow_empty_records_empty_snapshot() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");

    let output = shore([
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--allow-empty",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    let artifact = pointbreak::session::read_object_artifact(
        repo.path(),
        &pointbreak::model::ObjectId::new(json["revision"]["objectId"].as_str().unwrap()),
    )
    .expect("object artifact for empty capture");
    assert!(artifact.snapshot.files.is_empty());
}

#[test]
fn review_capture_ignores_untracked_content_by_default() {
    let repo = modified_repo();

    let first = parse_json(&shore(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    repo.write("untracked.txt", "new review content\n");
    let second = parse_json(&shore(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);

    assert_eq!(first["revision"]["id"], second["revision"]["id"]);
}

#[test]
fn review_capture_include_untracked_changes_when_untracked_content_changes() {
    let repo = modified_repo();

    let first = parse_json(&shore(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    repo.write("untracked.txt", "new review content\n");
    let second = parse_json(
        &shore([
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--include-untracked",
        ])
        .stdout,
    );

    assert_ne!(first["revision"]["id"], second["revision"]["id"]);
    let shown = show_revision_json(repo.path(), second["revision"]["id"].as_str().unwrap());
    assert_eq!(shown["revision"]["source"]["includeUntracked"], true);
}

#[test]
fn review_capture_writes_through_to_the_shared_store_without_a_link_step() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");

    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    let capture = shore(["capture", "--repo", repo.path().to_str().unwrap()]);
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
    let _capture = parse_json(&shore(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);

    let snapshots_dir = common_dir_store(&repo).join("artifacts/objects");
    let artifact_path = std::fs::read_dir(&snapshots_dir)
        .expect("snapshots dir exists")
        .filter_map(|entry| entry.ok())
        .find(|entry| entry.path().extension().and_then(|s| s.to_str()) == Some("json"))
        .map(|entry| entry.path())
        .expect("at least one object artifact");
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
    assert_eq!(json["revision"]["base"]["kind"], "git_commit");
    assert_eq!(json["revision"]["target"]["kind"], "git_commit");
    assert_eq!(json["revision"]["target"]["commitOid"], head_oid);
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

    assert_eq!(json["revision"]["base"]["commitOid"], first_oid);
    assert_eq!(json["revision"]["target"]["commitOid"], second_oid);
    assert_ne!(
        json["revision"]["target"]["commitOid"], head_oid,
        "target must not default to HEAD when --target is given"
    );
}

#[test]
fn cli_capture_root_outputs_added_files_for_one_commit_repo() {
    let repo = GitRepo::new();
    repo.write("README.md", "hello\n");
    repo.commit_all("initial");

    let output = shore(["capture", "--repo", repo.path().to_str().unwrap(), "--root"]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    assert_eq!(json["schema"], "pointbreak.review-capture");
    assert_eq!(json["revision"]["base"]["kind"], "git_tree");
    assert_eq!(json["revision"]["target"]["kind"], "git_commit");
    assert_eq!(json["diffstat"]["addedFiles"], 1);
}

#[test]
fn cli_capture_root_with_target_pins_target_commit() {
    let repo = committed_repo();
    let first_oid = rev(&repo, "HEAD~1");

    let output = shore([
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--root",
        "--target",
        "HEAD~1",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    assert_eq!(json["revision"]["base"]["kind"], "git_tree");
    assert_eq!(json["revision"]["target"]["commitOid"], first_oid);
}

#[test]
fn cli_capture_root_with_path_scopes_added_files() {
    let repo = GitRepo::new();
    repo.write("a/one.txt", "one\n");
    repo.write("b/two.txt", "two\n");
    repo.commit_all("initial");

    let output = shore([
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--root",
        "--path",
        "a",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    assert_eq!(json["diffstat"]["addedFiles"], 1);
    let snapshot_id =
        pointbreak::model::ObjectId::new(json["revision"]["objectId"].as_str().unwrap());
    let artifact = pointbreak::session::read_object_artifact(repo.path(), &snapshot_id)
        .expect("object artifact for the scoped root capture");
    let paths: Vec<&str> = artifact
        .snapshot
        .files
        .iter()
        .filter_map(|file| file.new_path.as_deref())
        .collect();
    assert_eq!(paths, vec!["a/one.txt"]);

    let revision_id = json["revision"]["id"].as_str().unwrap();
    let shown = parse_json(
        &shore([
            "revision",
            "show",
            revision_id,
            "--repo",
            repo.path().to_str().unwrap(),
        ])
        .stdout,
    );
    assert_eq!(shown["revision"]["source"]["pathspecs"][0], "a");
}

#[test]
fn cli_capture_root_rejects_base() {
    let output = shore(["capture", "--root", "--base", "HEAD"]);

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("--base, --root, --staged, and --unstaged are mutually exclusive"),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cli_capture_staged_ignores_unstaged_and_untracked_files() {
    let repo = GitRepo::new();
    repo.write("tracked.txt", "base\n");
    repo.commit_all("base");
    repo.write("staged.txt", "staged\n");
    repo.git(["add", "staged.txt"]);
    repo.write("tracked.txt", "unstaged\n");
    repo.write("untracked.txt", "untracked\n");

    let output = shore([
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--staged",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    let shown = show_revision_json(repo.path(), json["revision"]["id"].as_str().unwrap());
    assert_eq!(shown["revision"]["source"]["kind"], "git_staged");
    assert_eq!(json["revision"]["base"]["kind"], "git_commit");
    assert_eq!(json["revision"]["target"]["kind"], "git_index");
    let snapshot_id =
        pointbreak::model::ObjectId::new(json["revision"]["objectId"].as_str().unwrap());
    let artifact = pointbreak::session::read_object_artifact(repo.path(), &snapshot_id)
        .expect("object artifact for staged capture");
    let paths: Vec<&str> = artifact
        .snapshot
        .files
        .iter()
        .filter_map(|file| file.new_path.as_deref())
        .collect();
    assert_eq!(paths, vec!["staged.txt"]);
}

#[test]
fn cli_capture_staged_in_unborn_repo_uses_empty_tree_base() {
    let repo = GitRepo::new();
    repo.write("README.md", "hello\n");
    repo.git(["add", "README.md"]);

    let output = shore([
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--staged",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    let shown = show_revision_json(repo.path(), json["revision"]["id"].as_str().unwrap());
    assert_eq!(shown["revision"]["source"]["kind"], "git_staged");
    assert_eq!(json["revision"]["base"]["kind"], "git_tree");
    assert_eq!(json["revision"]["target"]["kind"], "git_index");
    assert_eq!(json["diffstat"]["addedFiles"], 1);
}

#[test]
fn cli_capture_unstaged_excludes_staged_and_untracked_by_default() {
    let repo = GitRepo::new();
    repo.write("tracked.txt", "base\n");
    repo.commit_all("base");
    repo.write("staged.txt", "staged\n");
    repo.git(["add", "staged.txt"]);
    repo.write("tracked.txt", "unstaged\n");
    repo.write("untracked.txt", "untracked\n");

    let output = shore([
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--unstaged",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    let shown = show_revision_json(repo.path(), json["revision"]["id"].as_str().unwrap());
    assert_eq!(shown["revision"]["source"]["kind"], "git_unstaged");
    assert_eq!(shown["revision"]["source"]["includeUntracked"], false);
    assert_eq!(json["revision"]["base"]["kind"], "git_index");
    assert_eq!(json["revision"]["target"]["kind"], "git_working_tree");
    let snapshot_id =
        pointbreak::model::ObjectId::new(json["revision"]["objectId"].as_str().unwrap());
    let artifact = pointbreak::session::read_object_artifact(repo.path(), &snapshot_id)
        .expect("object artifact for unstaged capture");
    let paths: Vec<&str> = artifact
        .snapshot
        .files
        .iter()
        .filter_map(|file| file.new_path.as_deref())
        .collect();
    assert_eq!(paths, vec!["tracked.txt"]);
}

#[test]
fn cli_capture_unstaged_include_untracked_does_not_mutate_the_index() {
    let repo = GitRepo::new();
    repo.write("tracked.txt", "base\n");
    repo.commit_all("base");
    repo.write("tracked.txt", "unstaged\n");
    repo.write("untracked.txt", "untracked\n");

    let output = shore([
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--unstaged",
        "--include-untracked",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    let shown = show_revision_json(repo.path(), json["revision"]["id"].as_str().unwrap());
    assert_eq!(shown["revision"]["source"]["kind"], "git_unstaged");
    assert_eq!(shown["revision"]["source"]["includeUntracked"], true);
    let snapshot_id =
        pointbreak::model::ObjectId::new(json["revision"]["objectId"].as_str().unwrap());
    let artifact = pointbreak::session::read_object_artifact(repo.path(), &snapshot_id)
        .expect("object artifact for unstaged capture");
    let paths: Vec<&str> = artifact
        .snapshot
        .files
        .iter()
        .filter_map(|file| file.new_path.as_deref())
        .collect();
    assert_eq!(paths, vec!["tracked.txt", "untracked.txt"]);

    let status = repo.git(["status", "--porcelain=v2", "--untracked-files=all"]);
    assert!(
        status.stdout.contains("? untracked.txt"),
        "untracked file should remain unstaged:\n{}",
        status.stdout
    );
}

#[test]
fn cli_capture_unstaged_include_untracked_works_in_unborn_repo() {
    let repo = GitRepo::new();
    repo.write("README.md", "hello\n");

    let output = shore([
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--unstaged",
        "--include-untracked",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    let shown = show_revision_json(repo.path(), json["revision"]["id"].as_str().unwrap());
    assert_eq!(shown["revision"]["source"]["kind"], "git_unstaged");
    assert_eq!(json["revision"]["base"]["kind"], "git_index");
    assert_eq!(json["revision"]["target"]["kind"], "git_working_tree");
    assert_eq!(json["diffstat"]["addedFiles"], 1);
}

#[test]
fn cli_capture_include_untracked_requires_unstaged() {
    let repo = modified_repo();

    let staged = shore([
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--staged",
        "--include-untracked",
    ]);
    let root = shore([
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--root",
        "--include-untracked",
    ]);

    for output in [staged, root] {
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("--include-untracked can only be used with worktree or unstaged capture"),
            "stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn cli_capture_include_untracked_works_with_default_worktree_capture() {
    let repo = modified_repo();
    repo.write("untracked.txt", "untracked\n");

    let output = shore([
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--include-untracked",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    let shown = show_revision_json(repo.path(), json["revision"]["id"].as_str().unwrap());
    assert_eq!(shown["revision"]["source"]["kind"], "git_worktree");
    assert_eq!(shown["revision"]["source"]["includeUntracked"], true);
    let snapshot_id =
        pointbreak::model::ObjectId::new(json["revision"]["objectId"].as_str().unwrap());
    let artifact = pointbreak::session::read_object_artifact(repo.path(), &snapshot_id)
        .expect("object artifact for untracked-inclusive worktree capture");
    let paths: Vec<&str> = artifact
        .snapshot
        .files
        .iter()
        .filter_map(|file| file.new_path.as_deref())
        .collect();
    assert_eq!(paths, vec!["src/lib.rs", "untracked.txt"]);
}

#[test]
fn cli_capture_include_untracked_works_in_unborn_repo_without_root() {
    let repo = GitRepo::new();
    repo.write("README.md", "hello\n");

    let output = shore([
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--include-untracked",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    let shown = show_revision_json(repo.path(), json["revision"]["id"].as_str().unwrap());
    assert_eq!(shown["revision"]["source"]["kind"], "git_worktree");
    assert_eq!(shown["revision"]["source"]["includeUntracked"], true);
    assert_eq!(json["revision"]["base"]["kind"], "git_tree");
    assert_eq!(json["revision"]["target"]["kind"], "git_working_tree");
    assert_eq!(json["diffstat"]["addedFiles"], 1);
}

#[test]
fn capture_with_dirty_worktree_and_base_ignores_worktree_state() {
    let repo = committed_repo();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 999 }\n");
    repo.write("untracked.txt", "untracked\n");

    let capture = parse_json(
        &shore([
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--base",
            "HEAD~1",
        ])
        .stdout,
    );
    let snapshot_id =
        pointbreak::model::ObjectId::new(capture["revision"]["objectId"].as_str().unwrap());

    let artifact = pointbreak::session::read_object_artifact(repo.path(), &snapshot_id)
        .expect("object artifact for the range capture");
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
fn capture_target_without_base_or_root_is_rejected() {
    let repo = committed_repo();

    let output = shore([
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--target",
        "HEAD",
    ]);

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--target requires --base or --root"),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn capture_with_unresolvable_base_fails_honestly() {
    let repo = committed_repo();

    let output = shore([
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
    shore(["capture", "--repo", repo.path().to_str().unwrap()]);

    let show =
        parse_json(&shore(["revision", "show", "--repo", repo.path().to_str().unwrap()]).stdout);

    assert_eq!(show["revision"]["source"]["kind"], "git_worktree");
    assert_eq!(show["revision"]["target"]["kind"], "git_working_tree");
}

#[test]
fn capture_accepts_supersedes_and_records_no_lineage_attach() {
    let repo = modified_repo();

    // The first revision (no predecessors).
    let first = parse_json(&shore(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    let first_id = first["revision"]["id"].as_str().unwrap().to_owned();
    // A capture never attaches lineage now.
    assert!(first.get("lineageAttach").is_none());

    // A changed file yields a distinct revision that supersedes the first.
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    let second = parse_json(
        &shore([
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--supersedes",
            &first_id,
        ])
        .stdout,
    );
    let second_id = second["revision"]["id"].as_str().unwrap().to_owned();
    assert_ne!(first_id, second_id);
    assert!(second.get("lineageAttach").is_none());

    // The supersession is readable: --revision on the superseded id resolves to
    // the single current head (the second revision).
    let resolved = parse_json(
        &shore([
            "revision",
            "show",
            &first_id,
            "--repo",
            repo.path().to_str().unwrap(),
        ])
        .stdout,
    );
    assert_eq!(resolved["revision"]["id"], second_id.as_str());
}

#[test]
fn rebased_recapture_with_stable_object_id_writes_distinct_artifact() {
    let repo = GitRepo::new();
    let base = (1..=12)
        .map(|line| format!("preamble {line}\n"))
        .chain(["pub fn value() -> u32 { 1 }\n".to_owned()])
        .chain((1..=6).map(|line| format!("trailer {line}\n")))
        .collect::<String>();
    repo.write("src/lib.rs", &base);
    repo.commit_all("base");

    let reviewed = base.replace("pub fn value() -> u32 { 1 }", "pub fn value() -> u32 { 2 }");
    repo.write("src/lib.rs", reviewed);
    let first = parse_json(&shore(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    let first_id = first["revision"]["id"].as_str().unwrap().to_owned();
    let first_object = first["revision"]["objectId"].as_str().unwrap().to_owned();
    let first_artifact_hash = first["revision"]["objectArtifactContentHash"]
        .as_str()
        .unwrap()
        .to_owned();

    repo.git(["checkout", "--", "src/lib.rs"]);
    let rebased_base = format!("inserted upstream line\n{base}");
    repo.write("src/lib.rs", &rebased_base);
    repo.commit_all("upstream insert");
    let rebased_reviewed =
        rebased_base.replace("pub fn value() -> u32 { 1 }", "pub fn value() -> u32 { 2 }");
    repo.write("src/lib.rs", rebased_reviewed);

    let recapture = shore([
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--supersedes",
        &first_id,
    ]);

    assert!(
        recapture.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&recapture.stderr)
    );
    let second = parse_json(&recapture.stdout);
    assert_eq!(second["revision"]["objectId"], first_object);
    assert_ne!(second["revision"]["id"], first["revision"]["id"]);
    assert_ne!(
        second["revision"]["objectArtifactContentHash"]
            .as_str()
            .unwrap(),
        first_artifact_hash
    );
}

#[test]
fn capture_with_base_twice_reports_existing_event() {
    let repo = committed_repo();

    let first = parse_json(
        &shore([
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
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--base",
            "HEAD~1",
        ])
        .stdout,
    );

    assert_eq!(first["revision"]["id"], second["revision"]["id"]);
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

#[test]
fn text_capture_ack_shows_short_revision_and_diffstat() {
    let repo = modified_repo();
    let output = shore([
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--format",
        "text",
    ]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("captured"), "stdout:\n{stdout}");
    // short_ref form of the revision id, e.g. rev:1ace028b.
    assert!(stdout.contains("rev:"), "stdout:\n{stdout}");
    // modified_repo changes exactly one file.
    assert!(stdout.contains("1 file"), "stdout:\n{stdout}");
    // Bespoke rendering, not the JSON fallback.
    assert!(!stdout.contains("\"schema\""), "stdout:\n{stdout}");
    assert!(stdout.lines().count() <= 8, "stdout:\n{stdout}");
}

fn bounded_added_file_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("README.md", "base\n");
    repo.commit_all("base");
    let body = (1..=50).map(|n| format!("line {n}\n")).collect::<String>();
    repo.write("notes/added.txt", body);
    repo.git(["add", "notes/added.txt"]);
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

fn show_revision_json(repo: &std::path::Path, revision_id: &str) -> Value {
    parse_json(
        &shore([
            "revision",
            "show",
            revision_id,
            "--repo",
            repo.to_str().unwrap(),
        ])
        .stdout,
    )
}

fn modified_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo
}

fn two_dir_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("a/one.txt", "one\n");
    repo.write("b/two.txt", "two\n");
    repo.commit_all("base");
    repo.write("a/one.txt", "one changed\n");
    repo.write("b/two.txt", "two changed\n");
    repo
}

#[test]
fn capture_is_available_at_the_top_level() {
    let repo = modified_repo();

    let output = shore(["capture", "--repo", repo.path().to_str().unwrap()]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document = parse_json(&output.stdout);
    assert_eq!(document["schema"], "pointbreak.review-capture"); // INV-1: schema tag is frozen
}

#[test]
fn abbreviated_supersedes_resolves_to_the_full_id_before_it_is_stored() {
    let repo = modified_repo();
    let first = parse_json(&shore(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    let first_id = first["revision"]["id"].as_str().unwrap().to_owned();
    // first_id = "rev:sha256:<64hex>" (capture.rs's revision ids always take this shape).
    let fragment = &first_id["rev:sha256:".len()..][..8];

    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    let second = parse_json(
        &shore([
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--supersedes",
            fragment,
        ])
        .stdout,
    );
    let second_id = second["revision"]["id"].as_str().unwrap().to_owned();

    let resolved = parse_json(
        &shore([
            "revision",
            "show",
            &first_id,
            "--repo",
            repo.path().to_str().unwrap(),
        ])
        .stdout,
    );
    assert_eq!(
        resolved["revision"]["id"], second_id,
        "if the raw fragment had been stored instead of the resolved full id, \
         `--revision {first_id}` would not resolve to the second revision"
    );
}

#[test]
fn review_capture_with_path_scopes_the_captured_revision() {
    let repo = two_dir_repo();

    let scoped = shore([
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--path",
        "a",
    ]);
    assert!(
        scoped.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&scoped.stderr)
    );
    let scoped = parse_json(&scoped.stdout);

    let unscoped = parse_json(&shore(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);

    // A scoped capture is a distinct revision from the whole-repo capture of the
    // same worktree (their content and provenance both differ here).
    assert_ne!(scoped["revision"]["id"], unscoped["revision"]["id"]);
}

#[test]
fn review_capture_path_is_repeatable() {
    let repo = two_dir_repo();

    let output = shore([
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--path",
        "a",
        "--path",
        "b",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn review_capture_with_path_matching_nothing_fails_with_a_pathspec_error() {
    let repo = two_dir_repo();

    let output = shore([
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--path",
        "docs",
    ]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("produced no changed files") && stderr.contains("docs"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn scoped_capture_surfaces_its_pathspecs_in_show_revisions_and_history() {
    let repo = two_dir_repo();
    let captured = parse_json(
        &shore([
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--path",
            "a",
        ])
        .stdout,
    );
    let revision_id = captured["revision"]["id"].as_str().unwrap();

    let shown = parse_json(
        &shore([
            "revision",
            "show",
            revision_id,
            "--repo",
            repo.path().to_str().unwrap(),
        ])
        .stdout,
    );
    assert_eq!(shown["revision"]["source"]["pathspecs"][0], "a");

    let listed =
        parse_json(&shore(["revision", "list", "--repo", repo.path().to_str().unwrap()]).stdout);
    let entry = listed["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["revisionId"] == revision_id)
        .expect("scoped revision listed");
    assert_eq!(entry["source"]["pathspecs"][0], "a");

    let history = parse_json(
        &shore([
            "history",
            "--repo",
            repo.path().to_str().unwrap(),
            "--revision",
            revision_id,
        ])
        .stdout,
    );
    let capture_entry = history["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["summary"]["kind"] == "revision_captured")
        .expect("capture entry in history");
    assert_eq!(capture_entry["summary"]["source"]["pathspecs"][0], "a");
}

#[test]
fn differently_scoped_range_captures_stay_distinct_in_revisions() {
    // Two path-scoped captures of the same range are different review units;
    // the list surface must show both, each with its own source.pathspecs,
    // instead of collapsing them into one row on the shared target OID.
    let repo = two_dir_repo();
    repo.commit_all("change");

    for scope in ["a", "b"] {
        let output = shore([
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--base",
            "HEAD~1",
            "--target",
            "HEAD",
            "--path",
            scope,
        ]);
        assert!(
            output.status.success(),
            "stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let listed =
        parse_json(&shore(["revision", "list", "--repo", repo.path().to_str().unwrap()]).stdout);
    assert_eq!(listed["revisionCount"], 2);
    let mut scopes: Vec<String> = listed["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            entry["source"]["pathspecs"][0]
                .as_str()
                .expect("each scoped entry surfaces its own pathspecs")
                .to_owned()
        })
        .collect();
    scopes.sort();
    assert_eq!(scopes, vec!["a".to_owned(), "b".to_owned()]);
}

#[test]
fn unscoped_capture_surfaces_no_pathspecs_key() {
    let repo = two_dir_repo();
    let captured = parse_json(&shore(["capture", "--repo", repo.path().to_str().unwrap()]).stdout);
    let revision_id = captured["revision"]["id"].as_str().unwrap();

    let shown = parse_json(
        &shore([
            "revision",
            "show",
            revision_id,
            "--repo",
            repo.path().to_str().unwrap(),
        ])
        .stdout,
    );
    assert!(shown["revision"]["source"].get("pathspecs").is_none());
}

#[test]
fn review_capture_path_composes_with_base_and_target() {
    let repo = two_dir_repo();
    repo.commit_all("change");

    let output = shore([
        "capture",
        "--repo",
        repo.path().to_str().unwrap(),
        "--base",
        "HEAD~1",
        "--target",
        "HEAD",
        "--path",
        "a",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    assert_eq!(json["revision"]["base"]["kind"], "git_commit");
    assert_eq!(json["revision"]["target"]["kind"], "git_commit");
}
