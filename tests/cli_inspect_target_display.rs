//! Contract/regression tests for the path-private `targetDisplay` the inspector
//! derives at read time from already-captured fields.
//!
//! The derivation lives in the binary crate (`src/cli/inspect/api.rs`), so it is
//! not reachable from an integration test by a direct call. These tests instead
//! exercise the genuine production JSON end to end: they spawn the real
//! `shore inspect --port 0` server (which prints its bound URL and supports an
//! ephemeral port) and issue raw HTTP/1.1 GETs against `/api/revisions` and
//! `/api/revisions/{id}`. That locks the additive on-the-wire contract — a derived
//! worktree/head label spliced in without disturbing any existing field.

mod support;

use std::ffi::OsString;
use std::path::Path;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::inspect::{
    Inspector, WorktreeCapture, add_worktree, capture, capture_supersession_round, run_git,
    urlencode,
};
use support::shore;

/// Test A: a worktree on a symbolic branch derives `label = <basename>` and a
/// short head OID, while every prior field stays intact and no branch is claimed
/// as capture-time provenance.
#[test]
fn api_units_derives_label_for_symbolic_branch_worktree() {
    let fixture = WorktreeCapture::on_branch("wt-foo", "feature/foo");
    let inspector = Inspector::spawn(&fixture.worktree);

    let units = inspector.get_json("/api/revisions");
    let entry = &units["entries"][0];

    assert_eq!(entry["targetDisplay"]["label"], "wt-foo");
    assert_eq!(entry["targetDisplay"]["kind"], "working_tree");
    assert_eq!(entry["targetDisplay"]["pathPrivate"], true);

    let base_oid = entry["base"]["commitOid"].as_str().unwrap();
    assert_eq!(
        entry["targetDisplay"]["head"]["commitOidShort"],
        base_oid[..7]
    );

    // Additive: the verbatim endpoints and identity fields are all still present.
    assert!(
        entry["target"]["worktreeRoot"]
            .as_str()
            .unwrap()
            .ends_with("wt-foo")
    );
    assert_eq!(entry["target"]["kind"], "git_working_tree");
    assert!(entry["base"]["treeOid"].is_string());
    assert!(entry["source"].is_object());
    assert!(
        entry["snapshotContentHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );

    // No branch is claimed as capture-time provenance.
    assert!(entry["targetDisplay"]["head"]["liveBranch"].is_null());
    assert!(entry["targetDisplay"].get("branch").is_none());
}

/// Test A (continued): the same derived block also appears on the single-unit
/// `/api/revisions/{id}` document for a locally-readable unit, alongside the verbatim
/// target. Linked drill-in is covered separately by
/// `linked_inspector_drill_in_survives_deleted_source_worktree`.
#[test]
fn api_unit_splices_target_display_for_locally_readable_unit() {
    let fixture = WorktreeCapture::on_branch("wt-bar", "feature/bar");
    let inspector = Inspector::spawn(&fixture.worktree);

    let unit = inspector.get_json(&format!(
        "/api/revisions/{}",
        urlencode(&fixture.revision_id)
    ));
    let revision = &unit["revision"];

    assert_eq!(revision["targetDisplay"]["label"], "wt-bar");
    assert!(revision["targetDisplay"]["head"]["commitOidShort"].is_string());
    // The raw target endpoint is untouched by the splice.
    assert!(
        revision["target"]["worktreeRoot"]
            .as_str()
            .unwrap()
            .ends_with("wt-bar")
    );
    assert_eq!(revision["target"]["kind"], "git_working_tree");
}

/// A commit-range capture (`--base`) has a `git_commit` target, so the inspector
/// must label it with the short target OID — never the `"working tree"` floor —
/// and the wire block must stay path-private.
#[test]
fn inspector_units_render_commit_target_display_for_range_capture() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.commit_all("change");

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
        "capture stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let inspector = Inspector::spawn(repo.path());
    let units = inspector.get_json("/api/revisions");
    let entry = &units["entries"][0];

    assert_eq!(entry["targetDisplay"]["kind"], "git_commit");
    let target_oid = entry["target"]["commitOid"].as_str().unwrap();
    let base_oid = entry["base"]["commitOid"].as_str().unwrap();
    assert_eq!(entry["targetDisplay"]["label"], target_oid[..7]);
    assert_eq!(
        entry["targetDisplay"]["head"]["commitOidShort"],
        base_oid[..7]
    );
    assert_eq!(entry["targetDisplay"]["pathPrivate"], true);
    assert_ne!(entry["targetDisplay"]["label"], "working tree");
    assert!(
        !units.to_string().contains("worktreeRoot"),
        "range capture unit list must not expose a worktree path"
    );
}

/// Test B: a detached-HEAD capture still derives `label = <basename>` and a short
/// head OID, with no branch claimed.
#[test]
fn api_units_derives_label_for_detached_head_capture() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.git(["checkout", "--detach"]);
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    capture(repo.path());

    let inspector = Inspector::spawn(repo.path());
    let units = inspector.get_json("/api/revisions");
    let entry = &units["entries"][0];

    let worktree_root = entry["target"]["worktreeRoot"].as_str().unwrap();
    let expected_label = Path::new(worktree_root)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap();
    assert_eq!(entry["targetDisplay"]["label"], expected_label);

    let base_oid = entry["base"]["commitOid"].as_str().unwrap();
    assert_eq!(
        entry["targetDisplay"]["head"]["commitOidShort"],
        base_oid[..7]
    );
    assert!(entry["targetDisplay"]["head"]["liveBranch"].is_null());
}

/// Deleted-worktree fallback: after the captured worktree is force-removed, the
/// label still derives from the captured `worktreeRoot` basename when read from
/// a linked reader — proving derivation reads the captured field and never
/// probes the filesystem.
#[test]
fn api_units_label_survives_deleted_worktree() {
    let main = GitRepo::new();
    main.write("README.md", "base\n");
    main.commit_all("base");

    let parent = tempfile::tempdir().expect("worktree parent");
    let gone = parent.path().join("gone");
    add_worktree(main.path(), &gone, "gone");
    std::fs::write(gone.join("README.md"), "changed in gone\n").unwrap();
    capture(&gone);

    let reader = parent.path().join("reader");
    add_worktree(main.path(), &reader, "reader");

    // Force-remove the captured worktree's working directory.
    run_git(
        main.path(),
        [
            OsString::from("worktree"),
            OsString::from("remove"),
            OsString::from("--force"),
            gone.as_os_str().to_owned(),
        ],
    );
    assert!(!gone.exists());

    let inspector = Inspector::spawn(&reader);
    let units = inspector.get_json("/api/revisions");

    assert_eq!(units["revisionCount"], 1);
    let entry = &units["entries"][0];
    assert_eq!(entry["targetDisplay"]["label"], "gone");
    let base_oid = entry["base"]["commitOid"].as_str().unwrap();
    assert_eq!(
        entry["targetDisplay"]["head"]["commitOidShort"],
        base_oid[..7]
    );
}

#[test]
fn api_objects_threads_a_supersession_chain() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");

    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    let first = capture_supersession_round(repo.path(), None);

    let second = capture_supersession_round(repo.path(), Some(&first));

    let inspector = Inspector::spawn(repo.path());

    let objects = inspector.get_json("/api/threads");
    assert_eq!(objects["schema"], "shore.inspect-threads");
    assert!(objects["eventCount"].as_u64().unwrap() > 0);
    assert_eq!(objects["threadCount"], 1);
    assert_eq!(objects["diagnostics"].as_array().unwrap().len(), 0);

    let threads = objects["threads"].as_array().unwrap();
    assert_eq!(threads.len(), 1);
    let thread = &threads[0];
    assert_eq!(thread["competing"], false);

    let revisions = thread["revisions"].as_array().unwrap();
    assert_eq!(revisions.len(), 2);
    let revision_ids: Vec<&str> = revisions.iter().map(|r| r.as_str().unwrap()).collect();
    assert!(revision_ids.contains(&first.as_str()));
    assert!(revision_ids.contains(&second.as_str()));

    let heads = thread["heads"].as_array().unwrap();
    assert_eq!(heads.len(), 1);
    assert_eq!(heads[0], second);

    let superseded = thread["superseded"].as_array().unwrap();
    assert_eq!(superseded.len(), 1);
    assert_eq!(superseded[0], first);

    // The supersession edges are surfaced so the inspector can render the DAG and
    // name superseding successors: forward (revision -> what it supersedes) and
    // reverse (revision -> who supersedes it).
    assert_eq!(objects["supersedes"][&second][0], first);
    assert_eq!(objects["supersededBy"][&first][0], second);
    // A head supersedes nothing-it-was-superseded-by; a root is superseded by no one.
    assert!(objects["supersededBy"].get(&second).is_none());
    assert!(objects["supersedes"].get(&first).is_none());

    let objects_json = objects.to_string();
    assert!(
        !objects_json.contains(&repo.path().to_string_lossy().to_string()),
        "objects JSON must not expose raw repository paths"
    );

    // The removed lineage routes 404.
    let (status, _) = inspector.get_error("/api/lineages");
    assert!(status.contains("404"), "status: {status}");
    let (status, _) = inspector.get_error(&format!("/api/lineage?id={}", urlencode("anything")));
    assert!(status.contains("404"), "status: {status}");
}

#[test]
fn api_objects_surfaces_competing_heads_for_a_fork() {
    // A root revision superseded by two distinct successors: the thread has two
    // competing heads (a fork), and the root names BOTH superseding successors.
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");

    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    let root = capture_supersession_round(repo.path(), None);
    let branch_a = capture_supersession_round(repo.path(), Some(&root));
    let branch_b = capture_supersession_round(repo.path(), Some(&root));
    assert_ne!(branch_a, branch_b, "the two successors must be distinct");

    let inspector = Inspector::spawn(repo.path());
    let objects = inspector.get_json("/api/threads");

    assert_eq!(objects["threadCount"], 1);
    let thread = &objects["threads"][0];
    assert_eq!(thread["competing"], true);

    let heads: Vec<&str> = thread["heads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h.as_str().unwrap())
        .collect();
    assert_eq!(heads.len(), 2, "two competing heads: {heads:?}");
    assert!(heads.contains(&branch_a.as_str()));
    assert!(heads.contains(&branch_b.as_str()));

    assert_eq!(
        thread["superseded"].as_array().unwrap(),
        std::slice::from_ref(&root)
    );

    // The root names ALL of its superseding successors (fork-tolerant; not a single head).
    let superseders: Vec<&str> = objects["supersededBy"][&root]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert_eq!(
        superseders.len(),
        2,
        "names all successors: {superseders:?}"
    );
    assert!(superseders.contains(&branch_a.as_str()));
    assert!(superseders.contains(&branch_b.as_str()));

    // No fork-induced diagnostic: competing heads are surfaced, never an error.
    assert_eq!(objects["diagnostics"].as_array().unwrap().len(), 0);
}

#[test]
fn api_objects_carries_per_revision_classification() {
    // Root superseded by two successors -> {root: superseded by both, A/B: heads}.
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    let root = capture_supersession_round(repo.path(), None);
    let branch_a = capture_supersession_round(repo.path(), Some(&root));
    let branch_b = capture_supersession_round(repo.path(), Some(&root));

    let inspector = Inspector::spawn(repo.path());
    let objects = inspector.get_json("/api/threads");
    let cls = &objects["revisionClassification"];

    assert_eq!(cls[&root]["state"], "superseded");
    let supers: Vec<&str> = cls[&root]["supersededBy"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(supers.contains(&branch_a.as_str()) && supers.contains(&branch_b.as_str()));

    assert_eq!(cls[&branch_a]["state"], "head");
    assert_eq!(cls[&branch_b]["state"], "head");
    assert_eq!(cls[&branch_a]["supersedes"][0], root.as_str());

    // Additive: every existing field is byte-unchanged.
    assert_eq!(objects["schema"], "shore.inspect-threads");
    assert_eq!(objects["threads"][0]["competing"], true);
}

/// The issue-140 user story over a real socket: a linked reader (whose source
/// worktree has been deleted) can drill from the unit list into the unit
/// composite, snapshot diff, history, freshness, and lineages — all served
/// from the linked clone-local store, never the deleted worktree.
#[test]
fn linked_inspector_drill_in_survives_deleted_source_worktree() {
    let main = GitRepo::new();
    main.write("README.md", "base\n");
    main.commit_all("base");

    let parent = tempfile::tempdir().expect("worktree parent");
    let gone = parent.path().join("gone");
    add_worktree(main.path(), &gone, "gone");
    std::fs::write(gone.join("README.md"), "changed in gone\n").unwrap();
    let capture = capture_json(&gone);
    let unit_id = capture["revision"]["id"].as_str().unwrap().to_owned();
    let snapshot_id = capture["revision"]["objectId"].as_str().unwrap().to_owned();
    record_review_facts(&gone);

    let reader = parent.path().join("reader");
    add_worktree(main.path(), &reader, "reader");

    run_git(
        main.path(),
        [
            OsString::from("worktree"),
            OsString::from("remove"),
            OsString::from("--force"),
            gone.as_os_str().to_owned(),
        ],
    );
    assert!(!gone.exists());

    let inspector = Inspector::spawn(&reader);

    let units = inspector.get_json("/api/revisions");
    assert_eq!(units["revisionCount"], 1);
    assert_eq!(units["entries"][0]["revisionId"], unit_id.as_str());
    assert_eq!(units["entries"][0]["targetDisplay"]["label"], "gone");

    let unit = inspector.get_json(&format!("/api/revisions/{}", urlencode(&unit_id)));
    assert_eq!(unit["revision"]["id"], unit_id.as_str());
    assert_eq!(unit["summary"]["observationCount"], 1);
    assert_eq!(unit["summary"]["inputRequestCount"], 1);
    assert_eq!(unit["summary"]["assessmentCount"], 1);
    assert_eq!(unit["summary"]["validationCheckCount"], 1);
    assert!(unit["currentAssessment"]["status"].is_string());

    // The snapshot wire is object-scoped: content hash + diff only, no
    // target/targetDisplay (those are on /api/revisions(/{id}), asserted above).
    let snapshot = inspector.get_json(&format!("/api/snapshots/{}", urlencode(&snapshot_id)));
    assert!(
        snapshot["contentHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert!(snapshot.get("target").is_none());
    assert!(snapshot.get("targetDisplay").is_none());

    let history = inspector.get_json("/api/history");
    assert!(history["eventCount"].as_u64().unwrap() > 0);
    assert_eq!(history["eventCount"], units["eventCount"]);

    let freshness = inspector.get_json("/api/freshness");
    // The freshness probe's change key is the event-log head marker (the event
    // count), equal to the full read's count but without folding or hashing.
    assert_eq!(freshness["eventCount"], history["eventCount"]);

    let objects = inspector.get_json("/api/threads");
    assert_eq!(objects["threadCount"], 1);
    let thread = &objects["threads"][0];
    assert_eq!(thread["competing"], false);
    assert_eq!(thread["revisions"].as_array().unwrap().len(), 1);
    assert_eq!(thread["heads"][0], unit_id.as_str());
}

#[test]
fn linked_inspector_unit_error_message_stays_path_free_for_unknown_unit() {
    let main = GitRepo::new();
    main.write("README.md", "base\n");
    main.commit_all("base");

    let parent = tempfile::tempdir().expect("worktree parent");
    let seed = parent.path().join("seed");
    add_worktree(main.path(), &seed, "seed");
    std::fs::write(seed.join("README.md"), "changed in seed\n").unwrap();
    capture(&seed);
    let reader = parent.path().join("reader");
    add_worktree(main.path(), &reader, "reader");

    let inspector = Inspector::spawn(&reader);
    let (status, body) = inspector.get_error("/api/revisions/review-unit%3Asha256%3Amissing");

    assert!(!status.contains("200"), "status: {status}");
    assert_eq!(
        body["error"],
        "revision not found or unreadable: review-unit:sha256:missing"
    );
    assert!(!body["error"].as_str().unwrap().contains('/'));
}

// --- fixture-specific helpers (shared harness lives in `support::inspect`) ----

/// Capture the repo, returning the full capture document.
fn capture_json(repo: &Path) -> Value {
    let output = shore(["review", "capture", "--repo", repo.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "capture stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse capture JSON")
}

/// Record one observation, input request, assessment, and validation check
/// against the repo's single captured Revision.
fn record_review_facts(repo: &Path) {
    let repo_arg = repo.to_str().unwrap();
    for args in [
        vec![
            "review",
            "observation",
            "add",
            "--repo",
            repo_arg,
            "--track",
            "agent:test-fixture",
            "--title",
            "linked observation",
            "--body",
            "captured before the source worktree was deleted",
        ],
        vec![
            "review",
            "input-request",
            "open",
            "--repo",
            repo_arg,
            "--track",
            "agent:test-fixture",
            "--title",
            "Need approval",
            "--reason",
            "manual-decision-required",
            "--body",
            "approve this path?",
        ],
        vec![
            "review",
            "assessment",
            "add",
            "--repo",
            repo_arg,
            "--track",
            "human:kevin",
            "--assessment",
            "accepted",
            "--summary",
            "ship it",
        ],
        vec![
            "review",
            "validation",
            "add",
            "--repo",
            repo_arg,
            "--track",
            "agent:test-fixture",
            "--check-name",
            "cargo test",
            "--status",
            "passed",
        ],
    ] {
        let output = shore(args.iter().copied());
        assert!(
            output.status.success(),
            "shore {args:?} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
