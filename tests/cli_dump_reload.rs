mod support;

use std::process::Output;

use serde_json::Value;
use support::{dump_repo, shore};

#[test]
fn dump_omits_reload_diagnostics_when_no_shore_dir() {
    let repo = dump_repo();

    let output = shore(["dump", "--repo", repo.path().to_str().unwrap()]);
    let json = parse_json(&output);

    assert!(
        json.get("reload_diagnostics").is_none(),
        "expected no reload_diagnostics field; got: {json:#?}"
    );
}

#[test]
fn dump_omits_reload_diagnostics_and_false_stale_flags_when_no_staleness() {
    let repo = dump_repo();
    let repo_arg = repo.path().to_str().unwrap();

    capture_review(repo_arg);

    let output = shore(["dump", "--repo", repo_arg]);
    let json = parse_json(&output);

    assert!(
        json.get("reload_diagnostics").is_none(),
        "expected no reload_diagnostics field; got: {json:#?}"
    );
    assert!(
        json.get("review_artifacts").is_none(),
        "review_artifacts should not be emitted; got: {json:#?}"
    );
}

#[test]
fn dump_emits_reload_diagnostics_for_orphaned_durable_note() {
    let repo = dump_repo();
    let repo_arg = repo.path().to_str().unwrap();
    let sidecar_dir = tempfile::tempdir().unwrap();
    let sidecar_path = sidecar_dir.path().join("review-notes.json");
    std::fs::write(&sidecar_path, review_notes_json("src/lib.rs")).unwrap();

    let apply = shore([
        "notes",
        "apply",
        "--repo",
        repo_arg,
        "--review-notes",
        sidecar_path.to_str().unwrap(),
    ]);
    assert!(
        apply.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&apply.stderr)
    );

    repo.commit_all("clear diff");

    let output = shore(["dump", "--repo", repo_arg]);
    let json = parse_json(&output);

    assert!(
        reload_codes(&json).contains(&"note_orphaned"),
        "expected note_orphaned diagnostic; got: {json:#?}"
    );
}

#[test]
fn dump_with_sidecar_input_still_emits_reload_diagnostics() {
    let repo = dump_repo();
    let repo_arg = repo.path().to_str().unwrap();
    let sidecar_dir = tempfile::tempdir().unwrap();
    let sidecar_path = sidecar_dir.path().join("review-notes.json");
    std::fs::write(&sidecar_path, review_notes_json("src/gone.rs")).unwrap();

    let output = shore([
        "dump",
        "--repo",
        repo_arg,
        "--review-notes",
        sidecar_path.to_str().unwrap(),
    ]);
    let json = parse_json(&output);

    assert!(
        reload_codes(&json).contains(&"note_orphaned"),
        "sidecar path must still emit reload_diagnostics; got: {json:#?}"
    );
}

fn capture_review(repo_arg: &str) {
    let output = shore(["review", "capture", "--repo", repo_arg]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn reload_codes(json: &Value) -> Vec<&str> {
    json["reload_diagnostics"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["code"].as_str().unwrap())
        .collect()
}

fn parse_json(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn review_notes_json(path: &str) -> String {
    format!(
        r#"{{
  "schema": "shore.review-notes",
  "version": 1,
  "files": [
    {{
      "path": "{path}",
      "notes": [
        {{
          "title": "Review note",
          "target": {{ "side": "new", "startLine": 1, "endLine": 1 }}
        }}
      ]
    }}
  ]
}}"#
    )
}
