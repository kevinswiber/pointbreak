//! The retired-event-kind load invariant: an old signed store carrying a
//! recorded `review_note_imported` (t:07) event must keep loading and reading
//! cleanly forever. The write path and every payload projection are retired;
//! what remains is the reserved type code plus a parse-level tombstone, so the
//! event lists at envelope level and nothing consumes its payload.

mod support;

use std::path::Path;

use serde_json::Value;
use support::{dump_repo, pointbreak};

const FIXTURE_STORE: &str = "tests/fixtures/legacy_stores/review_note_imported/store";

/// A fresh repo whose canonical store contains the checked-in legacy event bytes.
fn repo_with_legacy_store() -> support::git_repo::GitRepo {
    let repo = dump_repo();
    let source = support::manifest_dir().join(FIXTURE_STORE);
    let target = repo.path().join(".git/pointbreak");
    copy_dir(&source, &target);
    repo
}

fn copy_dir(source: &Path, target: &Path) {
    std::fs::create_dir_all(target).expect("create store dir");
    for entry in std::fs::read_dir(source).expect("read fixture dir") {
        let entry = entry.expect("fixture dir entry");
        let to = target.join(entry.file_name());
        if entry.file_type().expect("fixture file type").is_dir() {
            copy_dir(&entry.path(), &to);
        } else {
            std::fs::copy(entry.path(), &to).expect("copy fixture file");
        }
    }
}

fn parse_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("valid json")
}

#[test]
fn legacy_event_bytes_at_the_canonical_store_still_load_and_read() {
    let repo = repo_with_legacy_store();
    let repo_arg = repo.path().to_str().unwrap();

    // history: the whole event set loads through the strict reader, and the
    // t:07 event renders at envelope level with a bare tombstone summary — no
    // payload-derived fields.
    let history = pointbreak(["history", "--repo", repo_arg, "--format", "json"]);
    assert!(
        history.status.success(),
        "history stderr:\n{}",
        String::from_utf8_lossy(&history.stderr)
    );
    let history = parse_json(&history.stdout);
    assert_eq!(history["eventCount"], 4);
    let entries = history["entries"].as_array().expect("history entries");
    let note_entry = entries
        .iter()
        .find(|entry| entry["eventType"] == "review_note_imported")
        .expect("the t:07 event is listed");
    assert_eq!(
        note_entry["summary"],
        serde_json::json!({ "kind": "review_note_imported" }),
        "the tombstone summary must carry no payload-derived fields"
    );

    // The retired kind is no longer a history filter value.
    let filtered = pointbreak([
        "history",
        "--repo",
        repo_arg,
        "--event-type",
        "review-note-imported",
    ]);
    assert!(
        !filtered.status.success(),
        "--event-type review-note-imported must be rejected"
    );

    // revision list + show: the captured revision still projects; the document
    // is version 2 with no adapter-note fields. `--all` keeps the revision
    // visible even though its anchored commits belong to the original
    // repository (orphaned relative to this scratch clone).
    let list = pointbreak([
        "revision", "list", "--all", "--repo", repo_arg, "--format", "json",
    ]);
    assert!(
        list.status.success(),
        "revision list stderr:\n{}",
        String::from_utf8_lossy(&list.stderr)
    );
    assert_eq!(parse_json(&list.stdout)["revisionCount"], 1);

    let show = pointbreak(["revision", "show", "--repo", repo_arg, "--format", "json"]);
    assert!(
        show.status.success(),
        "revision show stderr:\n{}",
        String::from_utf8_lossy(&show.stderr)
    );
    let show = parse_json(&show.stdout);
    assert_eq!(show["schema"], "pointbreak.review-revision");
    assert_eq!(show["version"], 2);
    assert!(show.get("adapterNotes").is_none());
    assert!(show["summary"].get("adapterNoteCount").is_none());

    // store status: the whole-store scan decodes the retired kind fine.
    let status = pointbreak(["store", "status", "--repo", repo_arg, "--format", "json"]);
    assert!(
        status.status.success(),
        "store status stderr:\n{}",
        String::from_utf8_lossy(&status.stderr)
    );
}
