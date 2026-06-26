//! Served-asset contract for the inspector's URL-hash route grammar.
//!
//! The client has no JS execution harness (the served envelope is vanilla
//! `include_str!`'d assets with no bundler/test runner), so these guard the
//! durable, user-observable route-grammar tokens and behaviors over the
//! `Inspector` HTTP harness: the hash is the single serialization of view
//! state, distinct navigations push history while refinements replace it,
//! Back/Forward and manual hash edits both re-derive the view, and an absent
//! deep link falls back up the hierarchy with a visible diagnostic rather than
//! a 404 or a blank view. They assert stable route tokens and visible copy,
//! never private function names.

mod support;

use support::inspect::{Inspector, representative_store};

fn served_app_js() -> String {
    let store = representative_store();
    Inspector::spawn(store.repo.path()).get_text("/app.js")
}

#[test]
fn app_js_serializes_view_state_into_the_url_hash() {
    let js = served_app_js();
    // The client reads and writes the URL fragment as its single source of view state.
    assert!(
        js.contains("location.hash"),
        "the router reads location.hash"
    );
    // Distinct navigations push history; search-as-you-type replaces it.
    assert!(
        js.contains("pushState") && js.contains("replaceState"),
        "navigations pushState; refinements replaceState"
    );
    // The whole view re-derives on Back/Forward and on a manual hash edit.
    assert!(
        js.contains("popstate") && js.contains("hashchange"),
        "popstate and hashchange both re-derive the view"
    );
}

#[test]
fn app_js_emits_the_canonical_route_tokens() {
    let js = served_app_js();
    // Lens-primary landing + entity-primary paths.
    assert!(js.contains("#/timeline"), "default landing lens token");
    assert!(
        js.contains("#/revision/"),
        "entity-primary revision path token"
    );
    assert!(js.contains("#/event/"), "entity-primary event path token");
    // The three lenses are the recognized master-pane projections.
    for lens in ["timeline", "list", "threads"] {
        assert!(
            js.contains(lens),
            "lens token `{lens}` is part of the grammar"
        );
    }
    // Query-param vocabulary the router serializes.
    for tok in [
        "lens=", "sel=", "track=", "object=", "order=", "types=", "q=", "diff=", "focus=",
    ] {
        assert!(
            js.contains(tok),
            "the router serializes the `{tok}` query token"
        );
    }
}

#[test]
fn app_js_falls_back_up_the_hierarchy_with_a_visible_diagnostic() {
    let store = representative_store();
    let insp = Inspector::spawn(store.repo.path());
    let html = insp.get_text("/");
    let js = insp.get_text("/app.js");
    // A deep link to an absent entity surfaces a diagnostic, never 404/blank.
    assert!(
        html.contains("id=\"route-diagnostic\""),
        "a stable slot exists for the fallback diagnostic"
    );
    // The router writes a human-readable fallback notice (durable user-visible
    // copy, not a private fn name): keep the literal substring stable.
    assert!(
        js.contains("fell back to"),
        "the fallback resolver surfaces a visible `fell back to …` diagnostic"
    );
}

#[test]
fn copy_current_view_link_uses_the_resolved_canonical_route() {
    let js = served_app_js();
    assert!(
        js.contains("function copyCurrentViewLink()"),
        "copy-link has a single helper for the canonical resolved route"
    );
    assert!(
        js.contains("location.origin + location.pathname + serializeState()"),
        "copy-link copies the resolved serialized state, not the raw incoming URL"
    );
    assert!(
        !js.contains("copyText(location.href)"),
        "copy-link must not recopy a broken raw fallback URL"
    );
}

#[test]
fn unsupported_asof_and_journal_routes_show_live_state_readback() {
    let js = served_app_js();
    assert!(
        js.contains("unsupportedAsOf") && js.contains("unsupportedJournal"),
        "reserved freshness params are parsed into route metadata before canonicalization"
    );
    assert!(
        js.contains("showing live state"),
        "unsupported pinned-link routes get visible live-state readback copy"
    );
    assert!(
        js.contains("as-of links are not supported by this server")
            || js.contains("journal links are not supported by this server"),
        "the diagnostic explains unsupported pinned route params instead of silently accepting them"
    );
    assert!(
        !js.contains("params.push(\"asof=\"") && !js.contains("params.push(\"journal=\""),
        "the router does not emit pinned/as-of route tokens without server support"
    );
}

#[test]
fn diff_focus_route_is_singular() {
    let js = served_app_js();
    assert!(
        js.contains("?diff=<objectId> ?focus=<factId>"),
        "route grammar documents one focus target, not a set"
    );
    assert!(
        js.contains("focus: p.focus ? p.focus : null"),
        "parseHash stores a single focus id"
    );
    assert!(
        js.contains("params.push(\"focus=\" + encodeURIComponent(state.focus))"),
        "serializeState emits one focus value"
    );
    assert!(
        !js.contains("p.focus.split(\" \")") && !js.contains("state.focus.join(\" \")"),
        "focus parsing/serialization should not preserve a set-shaped route"
    );
}

#[test]
fn future_diff_lens_route_is_deferred_behind_a_named_seam() {
    let js = served_app_js();
    assert!(
        js.contains("DIFF_LENS_ROUTE_SEAM"),
        "app.js should carry one named seam for the future full-page diff route/data decision"
    );
    assert!(
        js.contains("full-page diff lens") && js.contains("deferred"),
        "the seam should say the full-page diff lens remains deferred"
    );
    assert!(
        js.contains("diff=") && js.contains("route-preserving diff overlay"),
        "the current diff= route stays the route-preserving quick overlay"
    );
    assert!(
        !js.contains("#/diff/"),
        "this pass must not add a dedicated #/diff/ route"
    );
    assert!(
        !js.contains("lens=diff")
            && !js.contains("[\"timeline\", \"list\", \"threads\", \"diff\"]"),
        "this pass must not add diff as a master lens route token"
    );
}
