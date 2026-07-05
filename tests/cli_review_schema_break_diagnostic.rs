//! A stored event whose type/envelope was retired at a breaking change must not
//! hard-fail the CLI read surfaces. `shore review history` / `show` skip it and
//! surface a `ProjectionDiagnostic` instead, exiting 0 so the rest of the
//! review still renders.

mod support;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::{common_dir_store, shore};

/// A repo with one captured Revision plus a raw retired-type event file dropped
/// into the resolved store. The probe rejects the raw file before full decode,
/// so it needs no valid signature or hash.
fn store_with_retired_event() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");

    let capture = shore(["capture", "--repo", repo.path().to_str().unwrap()]);
    assert!(
        capture.status.success(),
        "capture failed:\n{}",
        String::from_utf8_lossy(&capture.stderr)
    );

    let events_dir = common_dir_store(repo.path()).join("events");
    std::fs::create_dir_all(&events_dir).unwrap();
    std::fs::write(
        events_dir.join(format!("{}.json", "a".repeat(64))),
        br#"{"eventType":"review_disposition_recorded"}"#,
    )
    .unwrap();

    repo
}

fn has_schema_break_diagnostic(json: &Value) -> bool {
    json["diagnostics"]
        .as_array()
        .expect("diagnostics is an array")
        .iter()
        .any(|d| d["code"] == "unsupported_event_type")
}

#[test]
fn review_history_surfaces_schema_break_diagnostic_and_exits_zero() {
    let repo = store_with_retired_event();

    let output = shore(["review", "history", "--repo", repo.path().to_str().unwrap()]);

    assert!(
        output.status.success(),
        "history must exit 0 over a retired event:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).expect("history JSON");
    assert!(
        has_schema_break_diagnostic(&json),
        "history diagnostics missing the schema break: {json}"
    );
}

#[test]
fn review_revisions_surfaces_schema_break_diagnostic_and_exits_zero() {
    let repo = store_with_retired_event();

    let output = shore([
        "review",
        "revisions",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);

    assert!(
        output.status.success(),
        "revisions must exit 0 over a retired event:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).expect("revisions JSON");
    assert!(
        has_schema_break_diagnostic(&json),
        "revisions diagnostics missing the schema break: {json}"
    );
}

#[test]
fn review_show_surfaces_schema_break_diagnostic_and_exits_zero() {
    let repo = store_with_retired_event();

    let output = shore(["review", "show", "--repo", repo.path().to_str().unwrap()]);

    assert!(
        output.status.success(),
        "show must exit 0 over a retired event:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).expect("show JSON");
    assert!(
        has_schema_break_diagnostic(&json),
        "show diagnostics missing the schema break: {json}"
    );
}
