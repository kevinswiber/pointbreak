//! Served-asset contract for the inspector's Revisions/Threads view (the
//! supersession-DAG affordance that replaces the retired Lineages tab),
//! exercised at the HTTP level per issue #110.
//!
//! `app.js`/`index.html` have no JS execution harness (LB-13), so the UI-wiring
//! guard is a string-level contract over the served assets: the Revisions view
//! reads `/api/objects`, renders competing heads instead of a null head, reads
//! the supersession edges for the stale badge, and the retired lineage routes /
//! event types / functions are gone.

mod support;

use support::inspect::{Inspector, representative_store, urlencode};

/// Spawn the inspector against a representative store and return the served
/// `/app.js` bytes.
fn served_app_js() -> String {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());
    inspector.get_text("/app.js")
}

fn served_index_html() -> String {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());
    inspector.get_text("/")
}

/// The substring of an asset between two markers, for scoping an assertion to one
/// function or block. Panics if either marker is absent.
fn slice_between<'a>(haystack: &'a str, start: &str, end: &str) -> &'a str {
    let from = haystack
        .find(start)
        .unwrap_or_else(|| panic!("missing {start}"));
    let rest = &haystack[from..];
    let to = rest
        .find(end)
        .unwrap_or_else(|| panic!("missing {end} after {start}"));
    &rest[..to]
}

#[test]
fn served_app_js_replaces_lineages_fetch_with_objects() {
    let app_js = served_app_js();

    // The gating boot load fetches /api/objects (the supersession threads), not the
    // retired /api/lineages, and the dead /api/lineage?id= drill-in call is gone.
    let load = slice_between(
        &app_js,
        "async function load()",
        "async function pollFreshness",
    );
    assert!(
        load.contains("/api/objects"),
        "load() must fetch /api/objects"
    );
    assert!(
        !app_js.contains("/api/lineage"),
        "the retired /api/lineage(s) routes must not be fetched"
    );
    // The orphaned lineage state/render machinery is gone.
    assert!(
        !app_js.contains("state.lineages"),
        "state.lineages is replaced by state.objects"
    );
    assert!(
        !app_js.contains("renderLineagePage") && !app_js.contains("renderMiniLineageStack"),
        "the linear lineage-stack render is retired"
    );
}

#[test]
fn served_app_js_renders_revision_threads() {
    let app_js = served_app_js();

    // A render pass over the supersession threads from /api/objects, fed by the
    // objectThreads() helper that reads state.objects.threads.
    assert!(
        app_js.contains("function renderRevisions"),
        "the Revisions view needs a renderRevisions pass"
    );
    let threads_helper = slice_between(
        &app_js,
        "function objectThreads",
        "function supersededByRevision",
    );
    assert!(
        threads_helper.contains("state.objects") && threads_helper.contains("threads"),
        "objectThreads() must read the threads from state.objects"
    );
    let render = slice_between(&app_js, "function renderRevisions", "function threadLabel");
    assert!(
        render.contains("objectThreads") && render.contains("renderThreadCard"),
        "renderRevisions must iterate the threads into thread cards"
    );
}

#[test]
fn served_app_js_renders_competing_heads_not_a_null_head() {
    let app_js = served_app_js();

    // A fork surfaces competing revisions, never a "head: —" null head.
    assert!(
        app_js.contains("competing revisions"),
        "a forked thread renders a competing-revisions badge"
    );
    // The retired null-on-fork pattern (head = … ? refChip : "—") is gone.
    assert!(
        !app_js.contains("headRevisionId"),
        "the null-on-fork headRevisionId render is retired"
    );
}

#[test]
fn served_app_js_timeline_drops_retired_lineage_event_types() {
    let app_js = served_app_js();

    let types_block = slice_between(&app_js, "const TYPES = [", "const TYPE_MAP");
    assert!(
        !types_block.contains("review_unit_lineage"),
        "the two retired lineage event types must leave the timeline TYPES"
    );
    // The capture row uses the reshaped event type.
    assert!(
        types_block.contains("work_object_proposed"),
        "the capture timeline type is work_object_proposed"
    );
    assert!(
        !types_block.contains("review_unit_captured"),
        "the pre-reshape review_unit_captured type is gone"
    );
    // The lineage-round resolver/navigation is retired.
    assert!(
        !app_js.contains("navigateToLineageRound") && !app_js.contains("LINEAGE_FACT_TYPES"),
        "the lineage-round resolvers are retired"
    );
}

#[test]
fn served_app_js_stale_badge_reads_supersession() {
    let app_js = served_app_js();

    // The stale badge is computed from the supersession reverse edges (naming all
    // superseding successors), not a single lineage head.
    assert!(
        app_js.contains("supersededBy"),
        "the stale badge reads the supersededBy edges off /api/objects"
    );
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

    for path in ["/api/units", "/api/history", "/api/objects"] {
        let body = inspector.get_text(path);
        assert!(
            !body.contains(&camel) && !body.contains(&snake),
            "{path} must not emit a review-unit wire key:\n{body}"
        );
    }
    let units = inspector.get_json("/api/units");
    assert!(units["entries"][0]["revisionId"].is_string());
    assert!(units["revisionCount"].is_u64());

    let revision_id = units["entries"][0]["revisionId"].as_str().unwrap();
    let unit_body = inspector.get_text(&format!("/api/unit?id={}", urlencode(revision_id)));
    assert!(
        !unit_body.contains(&camel) && !unit_body.contains(&snake),
        "/api/unit must not emit a review-unit wire key:\n{unit_body}"
    );
    let unit: serde_json::Value = serde_json::from_str(&unit_body).unwrap();
    assert!(
        unit["revision"]["id"].is_string(),
        "the unit document object key is `revision`"
    );
}

#[test]
fn served_documents_carry_no_snapshot_id_wire_key() {
    // The served identity key finishes Snapshot->Object: the content id serves
    // as `objectId` (matching the `obj:` value and `currentObjectId`), never the
    // legacy `snapshotId`. The "snapshot artifact" concept is untouched
    // (`snapshotArtifactContentHash` does not contain the forbidden token), and
    // the /api/snapshot route serves the artifact body, which has no identity key.
    let forbidden = ["snapshot", "Id"].concat();
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());

    let units = inspector.get_json("/api/units");
    assert!(
        units["entries"][0]["objectId"].is_string(),
        "the units entry serves the content id under `objectId`"
    );
    let units_body = inspector.get_text("/api/units");
    assert!(
        !units_body.contains(&forbidden),
        "/api/units must not emit a `snapshotId` key:\n{units_body}"
    );

    let revision_id = units["entries"][0]["revisionId"].as_str().unwrap();
    let unit_body = inspector.get_text(&format!("/api/unit?id={}", urlencode(revision_id)));
    assert!(
        !unit_body.contains(&forbidden),
        "/api/unit must not emit a `snapshotId` key:\n{unit_body}"
    );
    let unit: serde_json::Value = serde_json::from_str(&unit_body).unwrap();
    assert!(
        unit["revision"]["objectId"].is_string(),
        "the unit document serves the content id under `objectId`"
    );
}

#[test]
fn served_index_html_replaces_lineages_tab_with_revisions() {
    let html = served_index_html();

    assert!(
        !html.contains("data-view=\"lineages\"") && !html.contains(">Lineages<"),
        "the Lineages tab is replaced"
    );
    assert!(
        html.contains("data-view=\"revisions\""),
        "a Revisions tab is present"
    );
    // The timeline filter is repointed from lineage to object.
    assert!(
        html.contains("id=\"filter-object\"") && !html.contains("id=\"filter-lineage\""),
        "the lineage filter becomes the object filter"
    );
}
