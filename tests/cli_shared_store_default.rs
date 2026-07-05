//! Acceptance suite for the shared-store default: every worktree of a clone —
//! main and linked — reads and writes the same common-dir store (`.git/shore`)
//! with NO `shore store link` step. The surviving opt-out is an ephemeral
//! worktree (its own discardable `.shore/data`). A pre-default worktree-local
//! store is routed to `shore store migrate`.
//!
//! No test here invokes `shore store link`, and no raw worktree / `.git` /
//! `.shore/data` path may leak into JSON output.

mod support;

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::{common_dir_store, shore};

fn run_json(args: &[&str]) -> Value {
    let output = shore(args.iter().copied());
    assert!(
        output.status.success(),
        "shore {args:?} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("shore stdout is json")
}

fn run_git_os<I>(cwd: &Path, args: I)
where
    I: IntoIterator<Item = OsString>,
{
    let args: Vec<OsString> = args.into_iter().collect();
    let output = Command::new("git")
        .args(&args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|error| panic!("run git {:?} in {}: {error}", args, cwd.display()));
    assert!(
        output.status.success(),
        "git {:?} failed in {}\nstderr:\n{}",
        args,
        cwd.display(),
        String::from_utf8_lossy(&output.stderr)
    );
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

fn add_detached_worktree(repo: &Path, path: &Path, at_rev: &str) {
    run_git_os(
        repo,
        [
            OsString::from("worktree"),
            OsString::from("add"),
            OsString::from("--detach"),
            path.as_os_str().to_owned(),
            OsString::from(at_rev),
        ],
    );
}

/// No raw storage path leaks into a JSON document.
fn assert_no_storage_path_leak(json: &Value) {
    let text = json.to_string();
    assert!(!text.contains(".shore/data"), "leaked .shore/data: {text}");
    assert!(!text.contains("/.git/"), "leaked a .git path: {text}");
}

// 1. A fresh MAIN worktree resolves and round-trips a capture in place, with no
//    `store link`, and the store lives in the common dir (`.git/shore`).
#[test]
fn main_worktree_capture_round_trips_through_the_common_dir_store() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    let repo_arg = repo.path().to_str().unwrap();

    let capture = run_json(&["capture", "--repo", repo_arg]);
    let unit_id = capture["revision"]["id"].as_str().unwrap().to_owned();

    // The shared common-dir store holds the events; the worktree-local
    // `.shore/data` is not the store.
    assert!(
        common_dir_store(repo.path()).join("events").is_dir(),
        "capture lands in the common-dir store"
    );
    assert!(!repo.path().join(".shore/data/events").exists());

    let list = run_json(&["review", "revisions", "--repo", repo_arg]);
    assert_eq!(list["revisionCount"], 1);
    assert_eq!(
        list["entries"][0]["revisionId"],
        Value::String(unit_id.clone())
    );
    assert_no_storage_path_leak(&list);

    let show = run_json(&["review", "show", "--repo", repo_arg]);
    assert_eq!(show["revision"]["id"], Value::String(unit_id.clone()));
    assert_no_storage_path_leak(&show);

    let history = run_json(&["review", "history", "--repo", repo_arg]);
    assert!(
        history["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry.to_string().contains(&unit_id)),
        "history resolves the captured unit"
    );
    assert_no_storage_path_leak(&history);
}

// 2. A capture in one worktree is visible from a SIBLING worktree of the same
//    clone, with no `store link`.
#[test]
fn capture_is_visible_from_a_sibling_worktree_without_a_link() {
    let main = GitRepo::new();
    main.write("README.md", "base\n");
    main.commit_all("base");

    let parent = tempfile::tempdir().expect("worktree parent");
    let seed = parent.path().join("seed");
    let reader = parent.path().join("reader");
    add_worktree(main.path(), &seed, "seed");
    add_worktree(main.path(), &reader, "reader");

    std::fs::write(seed.join("README.md"), "changed in seed\n").unwrap();
    let capture = run_json(&["capture", "--repo", seed.to_str().unwrap()]);
    let unit_id = capture["revision"]["id"].as_str().unwrap().to_owned();

    // The sibling reader sees the seed's unit through the shared store, no link.
    let list = run_json(&["review", "revisions", "--repo", reader.to_str().unwrap()]);
    let ids: Vec<&str> = list["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["revisionId"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&unit_id.as_str()),
        "sibling sees the seed's unit"
    );
    assert_no_storage_path_leak(&list);
}

// 3. An ephemeral worktree writes to its own discardable `.shore/data`; its
//    captures are absent from the shared store, and removing the worktree
//    discards its bytes.
#[test]
fn ephemeral_worktree_keeps_its_capture_out_of_the_shared_store() {
    let main = GitRepo::new();
    main.write("README.md", "base\n");
    main.commit_all("base");

    let parent = tempfile::tempdir().expect("worktree parent");
    let ephemeral = parent.path().join("ephemeral");
    add_worktree(main.path(), &ephemeral, "ephemeral");
    let ephemeral_arg = ephemeral.to_str().unwrap();

    // Opt this worktree out into ephemeral mode, then capture.
    let mode = run_json(&["store", "mode", "ephemeral", "--repo", ephemeral_arg]);
    assert_eq!(mode["mode"], "ephemeral");
    std::fs::write(ephemeral.join("README.md"), "changed ephemerally\n").unwrap();
    let capture = run_json(&["capture", "--repo", ephemeral_arg]);
    let unit_id = capture["revision"]["id"].as_str().unwrap().to_owned();

    // The capture landed in the worktree-local store, not the shared one.
    assert!(ephemeral.join(".shore/data/events").is_dir());
    assert!(
        !common_dir_store(&ephemeral).join("events").exists(),
        "the ephemeral capture is absent from the shared common-dir store"
    );

    // A sibling main-clone worktree does not see the ephemeral unit.
    let list = run_json(&[
        "review",
        "revisions",
        "--repo",
        main.path().to_str().unwrap(),
    ]);
    let ids: Vec<&str> = list["entries"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .map(|entry| entry["revisionId"].as_str().unwrap())
                .collect()
        })
        .unwrap_or_default();
    assert!(
        !ids.contains(&unit_id.as_str()),
        "ephemeral unit stays private"
    );

    // Removing the worktree discards its bytes (the store lived inside it).
    run_git_os(
        main.path(),
        [
            OsString::from("worktree"),
            OsString::from("remove"),
            OsString::from("--force"),
            ephemeral.as_os_str().to_owned(),
        ],
    );
    assert!(!ephemeral.exists());
    assert!(!common_dir_store(main.path()).join("events").exists());
}

// 4. A non-ephemeral worktree carrying a pre-default `.shore/data` store errors
//    on any read, naming `.shore/data` AND `shore store migrate`; after
//    migration the record resolves from the shared store.
#[test]
fn legacy_worktree_local_store_errors_until_migrated() {
    let repo = GitRepo::new();
    repo.write("README.md", "base\n");
    repo.commit_all("base");
    repo.write("README.md", "changed\n");
    let repo_arg = repo.path().to_str().unwrap();

    // Seed a pre-default worktree-local store: capture while ephemeral so the
    // write lands in `.shore/data`, then restore the shared default. The
    // populated `.shore/data` is now a legacy store on a non-ephemeral worktree.
    run_json(&["store", "mode", "ephemeral", "--repo", repo_arg]);
    let capture = run_json(&["capture", "--repo", repo_arg]);
    let unit_id = capture["revision"]["id"].as_str().unwrap().to_owned();
    run_json(&["store", "mode", "shared", "--repo", repo_arg]);
    assert!(repo.path().join(".shore/data/events").is_dir());

    // Any read now errors, naming both the legacy path and the migrate command.
    let read = shore(["review", "revisions", "--repo", repo_arg]);
    assert!(!read.status.success(), "a legacy store must fail the read");
    let stderr = String::from_utf8_lossy(&read.stderr);
    assert!(
        stderr.contains(".shore/data"),
        "the error names the legacy store: {stderr}"
    );
    assert!(
        stderr.contains("shore store migrate"),
        "the error names the fix: {stderr}"
    );

    // Migration folds the legacy store into the shared store, non-destructively:
    // the events land in `.git/shore` while `.shore/data` is left intact.
    let migrate = run_json(&["store", "migrate", "--repo", repo_arg]);
    assert!(migrate["eventsCreated"].as_u64().unwrap() >= 1);
    assert!(common_dir_store(repo.path()).join("events").is_dir());
    assert!(
        repo.path().join(".shore/data/events").is_dir(),
        "migration is non-destructive: the source store is preserved"
    );

    // Once the migrated legacy store is retired, the record resolves from the
    // shared store. (Clearing the source is the operator's step after migrating;
    // migration never deletes it.)
    std::fs::remove_dir_all(repo.path().join(".shore/data")).unwrap();
    let list = run_json(&["review", "revisions", "--repo", repo_arg]);
    let ids: Vec<&str> = list["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["revisionId"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&unit_id.as_str()),
        "the record resolves from the shared store after migration"
    );
    assert_no_storage_path_leak(&list);
}

// 4b. The one-command completion: `store migrate --retire-source` folds,
//     verifies, and deletes `.shore/data` so the very next read resolves —
//     no undocumented manual `rm` step.
#[test]
fn legacy_worktree_local_store_resolves_after_migrate_retire_source() {
    let repo = GitRepo::new();
    repo.write("README.md", "base\n");
    repo.commit_all("base");
    repo.write("README.md", "changed\n");
    let repo_arg = repo.path().to_str().unwrap();

    run_json(&["store", "mode", "ephemeral", "--repo", repo_arg]);
    let capture = run_json(&["capture", "--repo", repo_arg]);
    let unit_id = capture["revision"]["id"].as_str().unwrap().to_owned();
    run_json(&["store", "mode", "shared", "--repo", repo_arg]);
    assert!(repo.path().join(".shore/data/events").is_dir());

    // The guard fires and its hint names the one-command completion.
    let read = shore(["review", "revisions", "--repo", repo_arg]);
    assert!(!read.status.success(), "a legacy store must fail the read");
    let stderr = String::from_utf8_lossy(&read.stderr);
    assert!(
        stderr.contains("--retire-source"),
        "the hint names the one-command completion: {stderr}"
    );

    // ONE command completes the switch: fold + verify + retire.
    let migrate = run_json(&["store", "migrate", "--retire-source", "--repo", repo_arg]);
    assert_eq!(migrate["sourceRetired"], serde_json::Value::Bool(true));
    assert!(!repo.path().join(".shore/data").exists());

    let list = run_json(&["review", "revisions", "--repo", repo_arg]);
    let ids: Vec<&str> = list["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["revisionId"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&unit_id.as_str()),
        "the record resolves immediately after the one-command migration"
    );
}

// 5. Sibling captures: each worktree's `unit show` with NO `--revision`
//    resolves its OWN worktree's capture (worktree read scoping holds).
#[test]
fn each_worktree_unit_show_resolves_its_own_capture() {
    let main = GitRepo::new();
    main.write("README.md", "base\n");
    main.commit_all("base");

    let parent = tempfile::tempdir().expect("worktree parent");
    let alpha = parent.path().join("alpha");
    let beta = parent.path().join("beta");
    add_worktree(main.path(), &alpha, "alpha");
    add_worktree(main.path(), &beta, "beta");

    std::fs::write(alpha.join("README.md"), "alpha change\n").unwrap();
    let alpha_capture = run_json(&["capture", "--repo", alpha.to_str().unwrap()]);
    let alpha_id = alpha_capture["revision"]["id"].as_str().unwrap().to_owned();

    std::fs::write(beta.join("README.md"), "beta change\n").unwrap();
    let beta_capture = run_json(&["capture", "--repo", beta.to_str().unwrap()]);
    let beta_id = beta_capture["revision"]["id"].as_str().unwrap().to_owned();

    assert_ne!(
        alpha_id, beta_id,
        "the two worktrees capture distinct units"
    );

    // Both units live in the one shared store, yet each worktree's own
    // `unit show` (no `--revision`) resolves its OWN capture.
    let alpha_show = run_json(&["review", "show", "--repo", alpha.to_str().unwrap()]);
    assert_eq!(
        alpha_show["revision"]["id"],
        Value::String(alpha_id.clone())
    );
    assert_no_storage_path_leak(&alpha_show);

    let beta_show = run_json(&["review", "show", "--repo", beta.to_str().unwrap()]);
    assert_eq!(beta_show["revision"]["id"], Value::String(beta_id.clone()));
    assert_no_storage_path_leak(&beta_show);
}

// 6. Two worktrees capture the SAME commit range: both succeed sharing one
//    object artifact, and because commit-range provenance carries no worktree
//    path the two captures converge on ONE revision id. The read surface presents
//    them as ONE grouped unit exposing that id in `groupedRevisionIds`.
#[test]
fn two_worktrees_capturing_the_same_range_group_into_one_unit() {
    let main = GitRepo::new();
    main.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    main.commit_all("base");
    main.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    main.commit_all("change"); // HEAD = change, HEAD~1 = base

    let parent = tempfile::tempdir().expect("worktree parent");
    let wt_a = parent.path().join("wt-a");
    let wt_b = parent.path().join("wt-b");
    add_detached_worktree(main.path(), &wt_a, "HEAD");
    add_detached_worktree(main.path(), &wt_b, "HEAD");

    let a = run_json(&[
        "capture",
        "--repo",
        wt_a.to_str().unwrap(),
        "--base",
        "HEAD~1",
    ]);
    let b = run_json(&[
        "capture",
        "--repo",
        wt_b.to_str().unwrap(),
        "--base",
        "HEAD~1",
    ]);

    let a_id = a["revision"]["id"].as_str().unwrap().to_owned();
    let b_id = b["revision"]["id"].as_str().unwrap().to_owned();
    // Same commit range, no worktree path in the provenance: the two captures
    // converge on one revision id.
    assert_eq!(a_id, b_id);
    // One shared object artifact: byte-identical content hashes.
    assert_eq!(
        a["revision"]["objectArtifactContentHash"],
        b["revision"]["objectArtifactContentHash"]
    );

    // Exactly one object artifact on disk in the shared store.
    let snapshots_dir = common_dir_store(main.path()).join("artifacts/objects");
    let snapshot_count = std::fs::read_dir(&snapshots_dir)
        .expect("snapshots dir exists")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|s| s.to_str()) == Some("json"))
        .count();
    assert_eq!(
        snapshot_count, 1,
        "the two captures dedup to one shared artifact"
    );

    // The read surface groups them into ONE entry exposing both ids.
    let list = run_json(&[
        "review",
        "revisions",
        "--repo",
        main.path().to_str().unwrap(),
    ]);
    let entries = list["entries"].as_array().unwrap();
    assert_eq!(
        entries.len(),
        1,
        "the two captures present as one grouped unit"
    );
    let grouped: Vec<&str> = entries[0]["groupedRevisionIds"]
        .as_array()
        .expect("groupedRevisionIds is present")
        .iter()
        .map(|id| id.as_str().unwrap())
        .collect();
    assert!(
        grouped.contains(&a_id.as_str()),
        "grouping exposes the first id"
    );
    assert!(
        grouped.contains(&b_id.as_str()),
        "grouping exposes the second id"
    );
    assert_no_storage_path_leak(&list);
}
