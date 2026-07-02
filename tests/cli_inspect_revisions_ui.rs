mod support;

use support::git_repo::GitRepo;
use support::inspect::{Inspector, representative_store, urlencode};
use support::shore;

fn served_index_html() -> String {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());
    inspector.get_text("/")
}

#[test]
fn inspector_serves_markdown_body_content_type() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    let repo_arg = repo.path().to_str().unwrap();
    let capture = run_shore_json(&["review", "capture", "--repo", repo_arg]);
    let revision_id = capture["revision"]["id"].as_str().unwrap();

    run_shore(&[
        "review",
        "observation",
        "add",
        "--repo",
        repo_arg,
        "--track",
        "agent:codex",
        "--title",
        "Markdown observation",
        "--body",
        "## Finding\n\n- render **markdown**",
        "--body-content-type",
        "text/markdown",
    ]);

    let inspector = Inspector::spawn(repo.path());
    let history = inspector.get_json("/api/history");
    let summary = history["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["eventType"] == "review_observation_recorded")
        .and_then(|entry| entry["summary"].as_object())
        .expect("history contains the markdown observation summary");
    assert_eq!(summary["bodyContentType"], "text/markdown");
    assert_eq!(summary["body"], "## Finding\n\n- render **markdown**");

    let revision = inspector.get_json(&format!("/api/revisions/{}", urlencode(revision_id)));
    let observation = &revision["observations"][0];
    assert_eq!(observation["bodyContentType"], "text/markdown");
    assert_eq!(observation["body"], "## Finding\n\n- render **markdown**");
}

#[test]
fn served_documents_carry_no_revision_wire_key() {
    // The output documents are renamed to the revision vocabulary: no camelCase
    // or snake review-unit wire key survives on any served contract. (Hyphenated
    // id *values* like `review-unit:sha256:…` are not keys and are intentionally
    // not matched: the forbidden tokens are the underscore/camelCase spellings.)
    let camel = ["review", "Unit"].concat();
    let snake = ["review", "_unit"].concat();
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());

    for path in ["/api/revisions", "/api/history", "/api/threads"] {
        let body = inspector.get_text(path);
        assert!(
            !body.contains(&camel) && !body.contains(&snake),
            "{path} must not emit a review-unit wire key:\n{body}"
        );
    }
    let units = inspector.get_json("/api/revisions");
    assert!(units["entries"][0]["revisionId"].is_string());
    assert!(units["revisionCount"].is_u64());

    let revision_id = units["entries"][0]["revisionId"].as_str().unwrap();
    let unit_body = inspector.get_text(&format!("/api/revisions/{}", urlencode(revision_id)));
    assert!(
        !unit_body.contains(&camel) && !unit_body.contains(&snake),
        "/api/revisions/<id> must not emit a review-unit wire key:\n{unit_body}"
    );
    let unit: serde_json::Value = serde_json::from_str(&unit_body).unwrap();
    assert!(
        unit["revision"]["id"].is_string(),
        "the unit document object key is `revision`"
    );
}

fn run_shore(args: &[&str]) {
    let output = shore(args);
    assert!(
        output.status.success(),
        "shore {args:?} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_shore_json(args: &[&str]) -> serde_json::Value {
    let output = shore(args);
    assert!(
        output.status.success(),
        "shore {args:?} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("parse shore {args:?} JSON: {error}"))
}

#[test]
fn revisions_list_speaks_snapshot_vocabulary_and_member_doc_keeps_shared_keys() {
    // The vocabulary boundary: inspector-private wire DTOs speak snapshot
    // (`snapshotId`, `snapshotContentHash`); embedded shared documents keep the
    // substrate vocabulary (`objectId`, `objectArtifactContentHash`). The
    // /api/revisions list entries are inspector-owned; /api/revisions/{id}
    // re-serves the shared review document verbatim plus additive splices.
    let object_id_key = ["object", "Id"].concat();
    let object_hash_key = ["objectArtifact", "ContentHash"].concat();
    let snapshot_id_key = ["snapshot", "Id"].concat();
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());

    let units = inspector.get_json("/api/revisions");
    assert!(
        units["entries"][0]["snapshotId"].is_string(),
        "the units entry serves the content id under `snapshotId`"
    );
    assert!(
        units["entries"][0]["snapshotContentHash"].is_string(),
        "the units entry serves the captured content hash under `snapshotContentHash`"
    );
    let units_body = inspector.get_text("/api/revisions");
    assert!(
        !units_body.contains(&object_id_key),
        "/api/revisions must not emit an `objectId` key:\n{units_body}"
    );
    assert!(
        !units_body.contains(&object_hash_key),
        "/api/revisions must not emit an `objectArtifactContentHash` key:\n{units_body}"
    );

    let revision_id = units["entries"][0]["revisionId"].as_str().unwrap();
    let unit_body = inspector.get_text(&format!("/api/revisions/{}", urlencode(revision_id)));
    assert!(
        !unit_body.contains(&snapshot_id_key),
        "/api/revisions/<id> re-serves the shared document; no `snapshotId` key:\n{unit_body}"
    );
    let unit: serde_json::Value = serde_json::from_str(&unit_body).unwrap();
    assert!(
        unit["revision"]["objectId"].is_string(),
        "the unit document serves the shared content id under `objectId`"
    );
}

#[test]
fn served_index_html_offers_the_threads_lens_not_a_lineages_tab() {
    let html = served_index_html();

    // The retired Lineages tab never returns.
    assert!(
        !html.contains("data-view=\"lineages\"") && !html.contains(">Lineages<"),
        "the Lineages tab is replaced"
    );
    // The parallel-tab model is gone: the master pane swaps lenses instead. The
    // supersession-thread affordance is now the `threads` lens of the one shell.
    assert!(
        !html.contains("data-view="),
        "the parallel-view tab model is replaced by the lens switcher"
    );
    assert!(
        html.contains("data-lens=\"threads\"") && html.contains("data-lens=\"list\""),
        "the lens switcher offers the threads + list lenses"
    );
    // The retired lineage filter never returns; object filtering is now a token
    // in the structured query grammar (`object:`), not a dropdown.
    assert!(
        !html.contains("id=\"filter-lineage\""),
        "no lineage filter remains"
    );
}
