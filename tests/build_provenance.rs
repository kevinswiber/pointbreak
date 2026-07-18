#[allow(dead_code)]
#[path = "../build.rs"]
mod build_script;

use std::fs;
use std::path::Path;
use std::process::Command;

struct GitFixture {
    root: tempfile::TempDir,
}

impl GitFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("create git fixture");
        run_git(root.path(), &["init", "--initial-branch=main"]);
        run_git(root.path(), &["config", "user.name", "Pointbreak Test"]);
        run_git(
            root.path(),
            &["config", "user.email", "pointbreak@example.test"],
        );
        fs::write(root.path().join("tracked.txt"), "one\n").expect("write tracked file");
        run_git(root.path(), &["add", "tracked.txt"]);
        run_git(root.path(), &["commit", "-m", "initial"]);
        Self { root }
    }

    fn path(&self) -> &Path {
        self.root.path()
    }

    fn head(&self) -> String {
        git_stdout(self.path(), &["rev-parse", "HEAD"])
    }
}

#[test]
fn exact_tag_is_clean_git_identity_with_full_commit() {
    let repo = GitFixture::new();
    run_git(repo.path(), &["tag", "v1.2.3"]);

    let identity = build_script::derive_identity(repo.path(), "1.2.3").unwrap();
    let head = repo.head();

    assert_eq!(identity.source, "git");
    assert_eq!(identity.commit.as_deref(), Some(head.as_str()));
    assert_eq!(identity.describe, "v1.2.3");
    assert!(!identity.dirty);
}

#[test]
fn post_tag_linked_worktree_reports_distance_hash_and_full_head() {
    let repo = GitFixture::new();
    run_git(repo.path(), &["tag", "v1.2.3"]);
    fs::write(repo.path().join("tracked.txt"), "two\n").expect("update tracked file");
    run_git(repo.path(), &["commit", "-am", "post tag"]);

    let worktree_parent = tempfile::tempdir().expect("create worktree parent");
    let worktree = worktree_parent.path().join("linked");
    run_git(
        repo.path(),
        &[
            "worktree",
            "add",
            "--detach",
            worktree.to_str().expect("utf8 worktree path"),
            "HEAD",
        ],
    );

    let identity = build_script::derive_identity(&worktree, "1.2.3").unwrap();
    let head = repo.head();

    assert!(worktree.join(".git").is_file());
    assert_eq!(identity.source, "git");
    assert_eq!(identity.commit.as_deref(), Some(head.as_str()));
    assert!(
        identity.describe.starts_with("v1.2.3-1-g"),
        "unexpected describe: {}",
        identity.describe
    );
    assert!(!identity.dirty);
}

#[test]
fn tracked_and_index_changes_cannot_claim_a_clean_tag() {
    let repo = GitFixture::new();
    run_git(repo.path(), &["tag", "v1.2.3"]);
    fs::write(repo.path().join("tracked.txt"), "dirty\n").expect("dirty tracked file");

    let worktree_dirty = build_script::derive_identity(repo.path(), "1.2.3").unwrap();
    assert!(worktree_dirty.dirty);
    assert_eq!(worktree_dirty.describe, "v1.2.3-dirty");

    run_git(repo.path(), &["add", "tracked.txt"]);
    let index_dirty = build_script::derive_identity(repo.path(), "1.2.3").unwrap();
    assert!(index_dirty.dirty);
    assert_eq!(index_dirty.describe, "v1.2.3-dirty");
}

#[test]
fn package_root_does_not_inherit_parent_checkout_metadata() {
    let parent = GitFixture::new();
    let package_root = parent.path().join("target/package/pointbreak-1.2.3");
    fs::create_dir_all(&package_root).expect("create package root");

    let identity = build_script::derive_identity(&package_root, "1.2.3").unwrap();

    assert_eq!(identity.source, "package");
    assert_eq!(identity.commit, None);
    assert_eq!(identity.describe, "package:1.2.3");
    assert!(!identity.dirty);
}

#[test]
fn manifest_root_git_metadata_is_fail_closed_when_malformed() {
    let root = tempfile::tempdir().expect("create malformed fixture");
    fs::create_dir(root.path().join(".git")).expect("create partial git metadata");

    let error = build_script::derive_identity(root.path(), "1.2.3").unwrap_err();

    assert!(error.contains("Git metadata"), "unexpected error: {error}");
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed in {}:\n{}",
        repo.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("run git");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("git stdout is utf8")
        .trim()
        .to_owned()
}
