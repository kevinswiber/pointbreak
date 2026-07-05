mod support;

use std::fs;
use std::path::Path;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore;

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

fn remove_snapshot(repo: &Path, snapshot_id: &str) {
    let output = shore([
        "store",
        "remove",
        "--repo",
        repo.to_str().unwrap(),
        "--snapshot",
        snapshot_id,
    ]);
    assert!(
        output.status.success(),
        "remove stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn object_blob_count(repo: &Path) -> usize {
    let dir = support::common_dir_store(repo).join("artifacts/objects");
    fs::read_dir(&dir)
        .map(|entries| entries.count())
        .unwrap_or(0)
}

/// Rewrite the single `artifact_removed` event to look ingested (`ingest`
/// present), which drops its local-possession arm. The added field changes
/// neither `payloadHash` (it is not in the payload) nor `eventId`, so the event
/// stays valid on read.
fn mark_removal_ingested(repo: &Path) {
    let events_dir = support::common_dir_store(repo).join("events");
    for entry in fs::read_dir(&events_dir).expect("events dir exists") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let mut value: Value = parse_json(&fs::read(&path).unwrap());
        if value["eventType"] == "t:16" {
            value["ingest"] = serde_json::json!({
                "via": "ingest-events",
                "receivedAt": "2026-06-19T01:00:00Z"
            });
            fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            return;
        }
    }
    panic!("no artifact_removed event under {events_dir:?}");
}

#[test]
fn bare_compact_previews_and_refuses_without_yes() {
    let repo = modified_repo();
    let captured = capture(repo.path());
    let snapshot_id = captured["revision"]["objectId"].as_str().unwrap();
    let content_hash = captured["revision"]["objectArtifactContentHash"]
        .as_str()
        .unwrap();
    remove_snapshot(repo.path(), snapshot_id);

    let output = shore(["store", "compact", "--repo", repo.path().to_str().unwrap()]);

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    assert_eq!(json["schema"], "shore.store-compact");
    // The preview lists the would-erase blob, deletes nothing, and refuses.
    assert_eq!(json["dryRun"], true);
    assert_eq!(json["bytesReclaimed"].as_u64().unwrap(), 0);
    assert!(
        json["swept"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blob| blob["contentHash"] == content_hash)
    );
    assert!(
        json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["message"].as_str().unwrap().contains("--yes")),
        "a bare compact must surface a consent diagnostic: {json}"
    );
    assert_eq!(object_blob_count(repo.path()), 1, "nothing is deleted");
}

#[test]
fn compact_with_yes_deletes_eligible_blob() {
    let repo = modified_repo();
    let captured = capture(repo.path());
    let snapshot_id = captured["revision"]["objectId"].as_str().unwrap();
    let content_hash = captured["revision"]["objectArtifactContentHash"]
        .as_str()
        .unwrap();
    remove_snapshot(repo.path(), snapshot_id);

    let output = shore([
        "store",
        "compact",
        "--repo",
        repo.path().to_str().unwrap(),
        "--yes",
    ]);

    assert!(output.status.success());
    let json = parse_json(&output.stdout);
    assert_eq!(json["dryRun"], false);
    assert!(json["bytesReclaimed"].as_u64().unwrap() > 0);
    assert!(
        json["swept"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blob| blob["contentHash"] == content_hash && blob["outcome"] == "removed")
    );
    assert_eq!(object_blob_count(repo.path()), 0, "the blob is deleted");
}

#[test]
fn compact_dry_run_deletes_nothing() {
    let repo = modified_repo();
    let captured = capture(repo.path());
    let snapshot_id = captured["revision"]["objectId"].as_str().unwrap();
    remove_snapshot(repo.path(), snapshot_id);

    let output = shore([
        "store",
        "compact",
        "--repo",
        repo.path().to_str().unwrap(),
        "--dry-run",
    ]);

    assert!(output.status.success());
    let json = parse_json(&output.stdout);
    assert_eq!(json["dryRun"], true);
    assert_eq!(json["bytesReclaimed"].as_u64().unwrap(), 0);
    assert_eq!(
        object_blob_count(repo.path()),
        1,
        "a dry run deletes nothing"
    );
}

#[test]
fn compact_skips_ingested_unsigned_and_reports_reason() {
    let repo = modified_repo();
    let captured = capture(repo.path());
    let snapshot_id = captured["revision"]["objectId"].as_str().unwrap();
    let content_hash = captured["revision"]["objectArtifactContentHash"]
        .as_str()
        .unwrap();
    remove_snapshot(repo.path(), snapshot_id);
    // The removal arrives as ingested + unsigned: a non-operative claim.
    mark_removal_ingested(repo.path());

    let output = shore([
        "store",
        "compact",
        "--repo",
        repo.path().to_str().unwrap(),
        "--yes",
    ]);

    assert!(output.status.success());
    let json = parse_json(&output.stdout);
    // The blob is withheld and reported with its reason.
    assert_eq!(
        object_blob_count(repo.path()),
        1,
        "ineligible blob survives"
    );
    assert!(json["swept"].as_array().unwrap().is_empty());
    assert!(
        json["skippedIneligible"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| {
                s["contentHash"] == content_hash && s["reason"] == "removal_claim_unsigned"
            })
    );
}

#[test]
fn compact_with_yes_skips_a_drifted_blob_and_reports_mismatch() {
    let repo = modified_repo();
    let captured = capture(repo.path());
    let snapshot_id = captured["revision"]["objectId"].as_str().unwrap();
    let content_hash = captured["revision"]["objectArtifactContentHash"]
        .as_str()
        .unwrap();
    remove_snapshot(repo.path(), snapshot_id);

    // Tamper the erase-eligible blob's bytes so they no longer hash to the
    // content hash the payload->file join claims.
    let objects_dir = support::common_dir_store(repo.path()).join("artifacts/objects");
    let blob_path = fs::read_dir(&objects_dir)
        .unwrap()
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .next()
        .expect("one object artifact file");
    fs::write(&blob_path, b"tampered: no longer a valid object artifact").unwrap();

    let output = shore([
        "store",
        "compact",
        "--repo",
        repo.path().to_str().unwrap(),
        "--yes",
    ]);

    assert!(output.status.success());
    let json = parse_json(&output.stdout);
    // The blob survives and is reported as a hash mismatch, not erased.
    assert_eq!(object_blob_count(repo.path()), 1, "drifted blob survives");
    assert_eq!(json["bytesReclaimed"].as_u64().unwrap(), 0);
    assert!(
        json["swept"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blob| blob["contentHash"] == content_hash
                && blob["outcome"] == "hash_mismatch_skipped"),
        "swept reports the drifted blob: {json}"
    );
    assert!(
        json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "compact_hash_mismatch"),
        "a compact_hash_mismatch diagnostic is surfaced: {json}"
    );
}
