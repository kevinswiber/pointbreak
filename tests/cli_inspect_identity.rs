//! Contract coverage for the `shore inspect` `/api/identity` endpoint (issue #391),
//! exercised over real HTTP against a store built at test time. Reuses the shared
//! `support::inspect` harness (spawn the real `shore inspect --port 0` server).
//!
//! The family (user-level) placement is covered by the lib unit tests in
//! `src/session/workflow/store_identity.rs` (the `SHORE_HOME` seam); the integration
//! harness does not inject `SHORE_HOME`, so it exercises the clone-local and
//! linked-worktree cases here.

mod support;

use support::git_repo::GitRepo;
use support::inspect::{Inspector, WorktreeCapture, capture};

#[test]
fn identity_reports_clone_placement_and_repo_basename() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn v() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn v() -> u32 { 2 }\n");
    capture(repo.path());

    let inspector = Inspector::spawn(repo.path());
    let id = inspector.get_json("/api/identity");

    assert_eq!(id["schema"], "pointbreak.inspect-identity");
    let expected = repo.path().file_name().unwrap().to_str().unwrap();
    assert_eq!(id["repository"], expected);
    assert_eq!(id["placement"]["tier"], "clone");
    assert_eq!(id["placement"]["label"], "clone store");
    // No family under the clone-local tier; no worktree row in the main worktree.
    assert!(id.get("family").is_none() || id["family"].is_null());
    assert!(id.get("worktree").is_none() || id["worktree"].is_null());
}

#[test]
fn identity_body_never_leaks_an_absolute_path() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn v() -> u32 { 1 }\n");
    repo.commit_all("base");
    capture(repo.path());

    let inspector = Inspector::spawn(repo.path());
    let raw = inspector.get_text("/api/identity");
    let abs = repo.path().to_str().unwrap();
    assert!(
        !raw.contains(abs),
        "identity body leaked an absolute path: {raw}"
    );
}

#[test]
fn identity_distinguishes_repository_from_a_linked_worktree() {
    let capture = WorktreeCapture::on_branch("feat-foo", "feat/foo");
    let inspector = Inspector::spawn(&capture.worktree);
    let id = inspector.get_json("/api/identity");

    let main_basename = capture._main.path().file_name().unwrap().to_str().unwrap();
    assert_eq!(id["repository"], main_basename);
    assert_eq!(id["worktree"], "feat-foo");
    // A linked worktree resolves the shared common-dir store: still the clone tier.
    assert_eq!(id["placement"]["tier"], "clone");
}
