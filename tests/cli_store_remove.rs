mod support;

use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::{shore, shore_env};

/// A repo with a committed base and an uncommitted change, so `shore capture`
/// has a HEAD -> working-tree diff to capture.
fn modified_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("README.md", "base\n");
    repo.commit_all("base");
    repo.write("README.md", "changed\n");
    repo
}

fn parse_json(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).expect("stdout is json")
}

fn capture(repo: &Path) -> Value {
    let output = shore(["capture", "--repo", repo.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "capture stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    parse_json(&output.stdout)
}

/// Read back the single `artifact_removed` event JSON the CLI wrote to the
/// shared common-dir store (an integration test cannot reach the `pub(crate)`
/// `EventStore`, so it reads the event files directly).
fn artifact_removed_event(repo: &Path) -> Value {
    let events_dir = support::common_dir_store(repo).join("events");
    for entry in fs::read_dir(&events_dir).expect("events dir exists") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let value: Value = parse_json(&fs::read(&path).unwrap());
        if value["eventType"] == "t:16" {
            return value;
        }
    }
    panic!("no artifact_removed event written under {events_dir:?}");
}

#[test]
fn store_remove_by_snapshot_emits_removed_document() {
    let repo = modified_repo();
    let captured = capture(repo.path());
    let snapshot_id = captured["revision"]["objectId"].as_str().unwrap();
    let content_hash = captured["revision"]["objectArtifactContentHash"]
        .as_str()
        .unwrap();

    let output = shore([
        "store",
        "remove",
        "--repo",
        repo.path().to_str().unwrap(),
        "--snapshot",
        snapshot_id,
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("{\"schema\":\"shore.store-remove\""));
    let json = parse_json(stdout.as_bytes());
    assert_eq!(json["schema"], "shore.store-remove");
    assert_eq!(json["version"], 1);
    let removed = json["removed"].as_array().unwrap();
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0]["contentHash"], content_hash);
    assert_eq!(removed[0]["created"], true);
    assert_eq!(json["eventsCreated"], 1);

    // Path-free contract, matching `store status`.
    assert!(!stdout.contains(".shore"));
    assert!(!stdout.contains(".git"));
    assert!(!stdout.contains("state.json"));
    assert!(!stdout.contains("artifacts/"));
}

#[test]
fn store_remove_by_revision_reports_co_referencing_units() {
    // Two worktrees capturing the SAME working-tree change share one snapshot
    // content hash under distinct review unit ids: the working-tree target carries
    // each worktree's own root, so the two captures stay distinct even though their
    // snapshot bytes are identical (the cross-worktree coexistence case).
    // Both worktrees write through to the shared common-dir store by default.
    let main = GitRepo::new();
    main.write("README.md", "base\n");
    main.commit_all("base");

    let parent = tempfile::tempdir().unwrap();
    let wt1 = parent.path().join("wt1");
    let wt2 = parent.path().join("wt2");
    add_worktree(main.path(), &wt1, "wt1");
    add_worktree(main.path(), &wt2, "wt2");

    // The same uncommitted change in each worktree yields byte-identical snapshots.
    std::fs::write(wt1.join("README.md"), "change\n").unwrap();
    std::fs::write(wt2.join("README.md"), "change\n").unwrap();

    let cap1 = capture_worktree(&wt1);
    let cap2 = capture_worktree(&wt2);

    let unit1 = cap1["revision"]["id"].as_str().unwrap();
    let unit2 = cap2["revision"]["id"].as_str().unwrap().to_owned();
    let hash1 = cap1["revision"]["objectArtifactContentHash"]
        .as_str()
        .unwrap();
    let hash2 = cap2["revision"]["objectArtifactContentHash"]
        .as_str()
        .unwrap();
    assert_eq!(
        hash1, hash2,
        "the same change yields one shared content hash"
    );

    let output = shore([
        "store",
        "remove",
        "--repo",
        wt1.to_str().unwrap(),
        "--revision",
        unit1,
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    let entry = json["removed"]
        .as_array()
        .unwrap()
        .iter()
        .find(|blob| blob["contentHash"] == hash1)
        .expect("the shared snapshot hash is removed");
    let co_referencing: Vec<&str> = entry["coReferencingUnits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap())
        .collect();
    assert!(
        co_referencing.contains(&unit2.as_str()),
        "the sibling unit still names the shared blob: {co_referencing:?}"
    );
}

#[test]
fn store_remove_has_no_idempotency_key_flag() {
    let repo = modified_repo();
    let captured = capture(repo.path());
    let snapshot_id = captured["revision"]["objectId"].as_str().unwrap();

    let output = shore([
        "store",
        "remove",
        "--repo",
        repo.path().to_str().unwrap(),
        "--snapshot",
        snapshot_id,
        "--idempotency-key",
        "x",
    ]);

    assert!(
        !output.status.success(),
        "the removal key is non-overridable; an --idempotency-key flag must not exist"
    );
}

#[test]
fn store_remove_is_idempotent() {
    let repo = modified_repo();
    let captured = capture(repo.path());
    let snapshot_id = captured["revision"]["objectId"].as_str().unwrap();

    let first = shore([
        "store",
        "remove",
        "--repo",
        repo.path().to_str().unwrap(),
        "--snapshot",
        snapshot_id,
    ]);
    assert!(first.status.success());

    let second = shore([
        "store",
        "remove",
        "--repo",
        repo.path().to_str().unwrap(),
        "--snapshot",
        snapshot_id,
    ]);
    assert!(second.status.success());
    let json = parse_json(&second.stdout);
    assert_eq!(json["eventsCreated"], 0);
    assert_eq!(json["eventsExisting"], 1);
    assert_eq!(json["removed"][0]["created"], false);
}

#[test]
fn store_compact_deletes_removed_blob_and_emits_document() {
    let repo = modified_repo();
    let captured = capture(repo.path());
    let snapshot_id = captured["revision"]["objectId"].as_str().unwrap();
    let content_hash = captured["revision"]["objectArtifactContentHash"]
        .as_str()
        .unwrap();

    shore([
        "store",
        "remove",
        "--repo",
        repo.path().to_str().unwrap(),
        "--snapshot",
        snapshot_id,
    ]);
    // Erasure is consent-gated: `--yes` performs the delete (a bare compact
    // previews and refuses).
    let output = shore([
        "store",
        "compact",
        "--repo",
        repo.path().to_str().unwrap(),
        "--yes",
    ]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("{\"schema\":\"shore.store-compact\""));
    let json = parse_json(stdout.as_bytes());
    assert!(
        json["swept"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blob| blob["contentHash"] == content_hash && blob["outcome"] == "removed")
    );
    assert!(json["bytesReclaimed"].as_u64().unwrap() > 0);

    // The snapshot blob is physically gone.
    let snapshots = repo.path().join(".shore/data/artifacts/objects");
    let remaining = fs::read_dir(&snapshots)
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(remaining, 0, "the removed blob was physically deleted");
}

#[test]
fn store_gc_is_alias_of_compact() {
    let repo = modified_repo();
    let captured = capture(repo.path());
    let snapshot_id = captured["revision"]["objectId"].as_str().unwrap();

    shore([
        "store",
        "remove",
        "--repo",
        repo.path().to_str().unwrap(),
        "--snapshot",
        snapshot_id,
    ]);

    // `gc` is an alias of `compact` and inherits the consent gate.
    let gc = shore([
        "store",
        "gc",
        "--repo",
        repo.path().to_str().unwrap(),
        "--yes",
    ]);
    assert!(
        gc.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&gc.stderr)
    );
    let stdout = String::from_utf8(gc.stdout).unwrap();
    assert!(stdout.starts_with("{\"schema\":\"shore.store-compact\""));

    // A second sweep finds the blob already gone (idempotent).
    let second = shore([
        "store",
        "gc",
        "--repo",
        repo.path().to_str().unwrap(),
        "--yes",
    ]);
    let json = parse_json(&second.stdout);
    assert!(
        json["swept"]
            .as_array()
            .unwrap()
            .iter()
            .all(|blob| blob["outcome"] == "missing")
    );
}

#[test]
fn store_remove_unknown_snapshot_errors() {
    let repo = modified_repo();
    capture(repo.path());

    let output = shore([
        "store",
        "remove",
        "--repo",
        repo.path().to_str().unwrap(),
        "--snapshot",
        "snap:sha256:does-not-exist",
    ]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown snapshot"),
        "stderr should name the unknown snapshot:\n{stderr}"
    );
}

#[test]
fn store_remove_signs_the_event_when_a_sign_key_is_given() {
    let home = tempfile::tempdir().unwrap();
    let env_home = home.path().to_str().unwrap();
    let init = shore_env(
        ["keys", "init", "--name", "mykey"],
        &[("SHORE_HOME", env_home)],
    );
    assert!(init.status.success());

    let repo = modified_repo();
    let captured = parse_json(
        &shore_env(
            [
                "capture",
                "--repo",
                repo.path().to_str().unwrap(),
                "--sign-key",
                "mykey",
            ],
            &[("SHORE_HOME", env_home)],
        )
        .stdout,
    );
    let snapshot_id = captured["revision"]["objectId"].as_str().unwrap();

    let output = shore_env(
        [
            "store",
            "remove",
            "--repo",
            repo.path().to_str().unwrap(),
            "--snapshot",
            snapshot_id,
            "--sign-key",
            "mykey",
        ],
        &[("SHORE_HOME", env_home)],
    );
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The emitted removal event is signed — removal never writes an unsigned
    // event into a signed store.
    let event = artifact_removed_event(repo.path());
    assert!(
        event.get("signature").is_some(),
        "artifact_removed event must be signed: {event}"
    );
    assert!(event.get("signer").is_some());
}

fn capture_worktree(repo: &Path) -> Value {
    let output = shore(["capture", "--repo", repo.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "working-tree capture stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    parse_json(&output.stdout)
}

fn add_worktree(repo: &Path, path: &Path, branch: &str) {
    let output = Command::new("git")
        .args([
            OsString::from("worktree"),
            OsString::from("add"),
            OsString::from("-b"),
            OsString::from(branch),
            path.as_os_str().to_owned(),
        ])
        .current_dir(repo)
        .output()
        .unwrap_or_else(|error| panic!("run git worktree add in {}: {error}", repo.display()));
    assert!(
        output.status.success(),
        "git worktree add failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
