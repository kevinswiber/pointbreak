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

use support::git_repo::GitRepo;
use support::inspect::{Inspector, representative_store, urlencode};
use support::shore;

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
fn served_app_js_reads_the_server_revision_classification() {
    let app_js = served_app_js();

    // The per-revision supersession classification (head / superseded / its
    // superseders) is computed server-side and read as a payload field, not
    // re-derived in the browser. The data contract is asserted as JSON over HTTP
    // in cli_inspect_target_display (api_objects_carries_per_revision_classification);
    // here the durable served-copy seam is that the client reads the field.
    assert!(
        app_js.contains("revisionClassification"),
        "the client reads the server-computed revisionClassification field"
    );
}

#[test]
fn threads_lens_keyboard_order_uses_rendered_dag_order() {
    let app_js = served_app_js();

    assert!(
        app_js.contains("function threadRevisionOrder"),
        "threads lens stepping should have one helper for graph-native order"
    );
    let order_helper = slice_between(
        &app_js,
        "function threadRevisionOrder",
        "function threadLabel",
    );
    assert!(
        order_helper.contains("laidOut") && order_helper.contains("nodes"),
        "threadRevisionOrder should prefer laidOut.nodes when server geometry is present"
    );
    assert!(
        order_helper.contains(".sort(")
            && order_helper.contains(".y")
            && order_helper.contains(".x"),
        "rendered graph order should sort visually by y then x"
    );
    let lens_order = slice_between(&app_js, "function lensEntryIds", "function stepSelection");
    assert!(
        lens_order.contains("threadRevisionOrder(t)"),
        "Threads-lens keyboard stepping should use the same visual order as the rendered graph"
    );
}

#[test]
fn supersession_current_copy_distinguishes_git_head() {
    let app_js = served_app_js();

    assert!(
        app_js.contains("current in thread"),
        "supersession current state should avoid bare `head` beside the Git target head row"
    );
    assert!(
        app_js.contains("revision thread · current in thread"),
        "single-head thread copy should use the same current-in-thread wording"
    );
    assert!(
        !app_js.contains("<span class=\"badge head\">head</span>"),
        "bare head badge copy should not sit next to the Git target head readback"
    );
    assert!(
        !app_js.contains("revision thread · head "),
        "single-head thread labels should avoid bare `head` copy"
    );
}

#[test]
fn detail_fact_sections_repeat_stale_revision_context() {
    let app_js = served_app_js();

    assert!(
        app_js.contains("function staleFactSectionContext"),
        "revision detail should have one helper for repeated stale context"
    );
    assert!(
        app_js.contains("fact-stale-context")
            && app_js.contains("superseded by")
            && app_js.contains("supersededByRevision(revisionId)"),
        "detail fact sections should repeat the same superseded-by context near stale facts"
    );
    for section in [
        "factSection(\"Observations\", d.observations, renderObservationCard, staleContext)",
        "factSection(\"Input requests\", d.inputRequests, renderInputRequestCard, staleContext)",
        "factSection(\"Assessments\", d.assessments, renderAssessmentCard, staleContext)",
        "Validation checks",
    ] {
        assert!(
            app_js.contains(section),
            "detail stale context should be wired near `{section}`"
        );
    }
}

#[test]
fn served_app_js_renders_revision_overview_cues_from_api_payload() {
    let app_js = served_app_js();

    for helper in [
        "function overviewForRevision",
        "function assessmentCue",
        "function attentionCues",
        "function revisionSearchIndex",
    ] {
        assert!(
            app_js.contains(helper),
            "overview card rendering should define `{helper}`"
        );
    }

    let overview_helpers = slice_between(
        &app_js,
        "function overviewForRevision",
        "function renderUnits",
    );
    for token in [
        "currentAssessment",
        "acceptedWithFollowUp",
        "openInputRequestCount",
        "failedValidationCount",
        "erroredValidationCount",
        "review cues",
        "attention",
    ] {
        assert!(
            overview_helpers.contains(token),
            "overview helpers should read overview token `{token}`"
        );
    }
    let render_units = slice_between(&app_js, "function renderUnits", "function renderRevisions");
    assert!(
        render_units.contains("u.overview") && render_units.contains("renderRevisionOverview"),
        "renderUnits should pass each entry overview into the card renderer"
    );

    let render_thread = slice_between(
        &app_js,
        "function renderThreadCard",
        "function renderThreadSvg",
    );
    assert!(
        render_thread.contains("renderThreadRevisionOverview"),
        "thread cards should surface overview cues for their revisions"
    );
}

#[test]
fn served_app_js_serializes_attention_filters_through_query_text() {
    let app_js = served_app_js();

    assert!(
        app_js.contains("\"attention\""),
        "attention filtering should be part of the structured q= query grammar"
    );
    for token in [
        "attention:open-request",
        "attention:unassessed",
        "attention:validation-context",
        "attention:follow-up",
    ] {
        assert!(
            app_js.contains(token),
            "attention filter token `{token}` should be serialized through q="
        );
    }
}

#[test]
fn revision_overview_card_copy_stays_advisory_not_gate_like() {
    let app_js = served_app_js();
    let overview_helpers = slice_between(
        &app_js,
        "function overviewForRevision",
        "function renderUnits",
    );
    let render_units = slice_between(&app_js, "function renderUnits", "function renderRevisions");
    let overview_surface = [overview_helpers, render_units].join("\n");

    for forbidden in ["blocking", "merge status", "required"] {
        assert!(
            !overview_surface.contains(forbidden),
            "overview cards must not use unqualified gate-like word `{forbidden}`"
        );
    }
}

#[test]
fn served_app_js_renders_markdown_bodies() {
    let app_js = served_app_js();

    assert!(
        app_js.contains("function renderMarkdown"),
        "the inspector asset includes the Markdown renderer"
    );
    assert!(
        app_js.contains("text/markdown") && app_js.contains("bodyContentType"),
        "the inspector routes bodyContentType into Markdown rendering"
    );
    assert!(
        app_js.contains("safeMarkdownHref"),
        "Markdown links must pass through the safe href filter"
    );
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

    let revision = inspector.get_json(&format!("/api/revision?id={}", urlencode(revision_id)));
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

    for path in ["/api/revisions", "/api/history", "/api/objects"] {
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
    let unit_body = inspector.get_text(&format!("/api/revision?id={}", urlencode(revision_id)));
    assert!(
        !unit_body.contains(&camel) && !unit_body.contains(&snake),
        "/api/revision must not emit a review-unit wire key:\n{unit_body}"
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
fn served_documents_carry_no_snapshot_id_wire_key() {
    // The served identity key finishes Snapshot->Object: the content id serves
    // as `objectId` (matching the `obj:` value and `currentObjectId`), never the
    // legacy `snapshotId`. The "object artifact" concept is untouched
    // (`objectArtifactContentHash` does not contain the forbidden token), and
    // the /api/object route serves the artifact body, which has no identity key.
    let forbidden = ["snapshot", "Id"].concat();
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());

    let units = inspector.get_json("/api/revisions");
    assert!(
        units["entries"][0]["objectId"].is_string(),
        "the units entry serves the content id under `objectId`"
    );
    let units_body = inspector.get_text("/api/revisions");
    assert!(
        !units_body.contains(&forbidden),
        "/api/revisions must not emit a `snapshotId` key:\n{units_body}"
    );

    let revision_id = units["entries"][0]["revisionId"].as_str().unwrap();
    let unit_body = inspector.get_text(&format!("/api/revision?id={}", urlencode(revision_id)));
    assert!(
        !unit_body.contains(&forbidden),
        "/api/revision must not emit a `snapshotId` key:\n{unit_body}"
    );
    let unit: serde_json::Value = serde_json::from_str(&unit_body).unwrap();
    assert!(
        unit["revision"]["objectId"].is_string(),
        "the unit document serves the content id under `objectId`"
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
    let js = served_app_js();
    assert!(
        js.contains("object:"),
        "object filtering is a field token in the structured query grammar"
    );
}
