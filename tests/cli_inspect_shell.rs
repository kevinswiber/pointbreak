//! Served-asset contract for the inspector's single master-detail shell.
//!
//! The four parallel views collapse into one shell: a master pane that swaps
//! between the timeline / list / threads lenses, and a detail pane that is a pure
//! projection of the single selection. With no JS execution harness, these guard
//! the durable served markup (the one master + one detail pane, the lens
//! switcher, the absence of the four old view sections) and the preserved
//! advisory framing — never private render-function internals.

mod support;

use support::inspect::{Inspector, representative_store};

fn served_app_js() -> String {
    let store = representative_store();
    Inspector::spawn(store.repo.path()).get_text("/app.js")
}

#[test]
fn index_html_is_one_master_detail_shell_not_four_views() {
    let store = representative_store();
    let html = Inspector::spawn(store.repo.path()).get_text("/");
    // One master pane + one detail pane (the list-detail skeleton), not four sections.
    assert!(
        html.contains("id=\"master\"") && html.contains("id=\"detail\""),
        "the shell is one master pane + one detail pane"
    );
    // The four parallel view sections are gone (collapsed into the lens dispatch).
    for old in [
        "id=\"view-timeline\"",
        "id=\"view-units\"",
        "id=\"view-revisions\"",
        "id=\"view-unit\"",
    ] {
        assert!(
            !html.contains(old),
            "the parallel `{old}` section is collapsed into the shell"
        );
    }
    // The master pane switches between the three lenses (durable data-attr values).
    for lens in ["timeline", "list", "threads"] {
        assert!(
            html.contains(&format!("data-lens=\"{lens}\"")),
            "the lens switcher offers the `{lens}` lens"
        );
    }
}

#[test]
fn served_app_js_dispatches_lens_and_selection_through_the_router() {
    let js = served_app_js();
    // The detail pane is a pure projection of the single selection (one source of truth).
    assert!(
        js.contains("state.selected"),
        "detail renders from state.selected"
    );
    assert!(
        js.contains("state.lens"),
        "the master pane renders from state.lens"
    );
    // Cross-ref chips navigate via the router, not by mutating filters in place.
    assert!(
        js.contains("navigate("),
        "chip resolution calls the navigate() router"
    );
}

#[test]
fn served_assets_preserve_the_advisory_framing_and_competing_peers() {
    let store = representative_store();
    let insp = Inspector::spawn(store.repo.path());
    let html = insp.get_text("/");
    let js = insp.get_text("/app.js");
    // The advisory readback framing survives the shell rework.
    assert!(
        js.contains("never gates a write") && js.contains("reader-relative"),
        "the advisory readback framing is preserved"
    );
    // Competing revisions render as equal-weight peers, headline withheld.
    assert!(
        js.contains("competing revisions"),
        "a forked thread renders competing revisions as peers"
    );
    // The persistent top-bar advisory affordance stays visible.
    assert!(
        html.contains("advisory"),
        "the top-bar advisory affordance persists"
    );
}
