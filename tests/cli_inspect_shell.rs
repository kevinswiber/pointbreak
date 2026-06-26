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

fn served_app_css() -> String {
    let store = representative_store();
    Inspector::spawn(store.repo.path()).get_text("/app.css")
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
fn lens_tab_clicks_preserve_the_current_selection() {
    let js = served_app_js();
    let wire_controls = js
        .split("function wireControls()")
        .nth(1)
        .and_then(|tail| tail.split("function init()").next())
        .expect("wireControls block exists");

    assert!(
        wire_controls.contains("navigate({ lens: LENSES.includes(tab.dataset.lens)"),
        "lens tabs should navigate through the router"
    );
    assert!(
        !wire_controls.contains("selected: { kind: null, id: null }"),
        "mouse lens switches should preserve state.selected"
    );
}

#[test]
fn keyboard_revision_navigation_uses_the_filtered_revision_set() {
    let js = served_app_js();
    let lens_entries = js
        .split("function lensEntryIds()")
        .nth(1)
        .and_then(|tail| tail.split("function stepSelection").next())
        .expect("lensEntryIds block exists");

    assert!(
        lens_entries.contains(".filter(matchesRevisionFilters)"),
        "list cursor stepping should use the same filtered revisions that renderUnits shows"
    );
    assert!(
        lens_entries.contains(".filter(threadMatchesRevisionFilters)")
            && lens_entries.contains("filteredThreadRevisionIds(t, threadRevisionOrder(t))"),
        "thread cursor stepping should use the same filtered threads that renderRevisions shows, ordered by the rendered graph"
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

#[test]
fn served_css_has_a_narrow_viewport_shell_contract() {
    let css = served_app_css();
    assert!(
        css.contains("@media") && css.contains("max-width"),
        "served CSS should carry an explicit narrow viewport breakpoint"
    );
    assert!(
        css.contains("grid-template-columns: minmax(0, 1fr)")
            || css.contains("grid-template-columns: 1fr"),
        "narrow shell should stop forcing two minmax(360px, 1fr) columns"
    );
    assert!(
        css.contains("#topbar")
            && css.contains("flex-wrap: wrap")
            && css.contains(".stats")
            && css.contains("justify-content: flex-start"),
        "topbar and stats should be able to wrap instead of causing narrow overflow"
    );
}

#[test]
fn served_css_has_a_narrow_diff_modal_contract() {
    let css = served_app_css();
    let narrow = css
        .split("@media (max-width: 760px)")
        .nth(1)
        .and_then(|tail| tail.split(".units").next())
        .expect("narrow viewport media block exists");
    assert!(
        narrow.contains(".diff-layout") && narrow.contains("flex-direction: column"),
        "narrow diff modal should stack navigator and body instead of forcing a side-by-side row"
    );
    assert!(
        narrow.contains(".diff-nav")
            && narrow.contains("border-bottom: 1px solid var(--border)")
            && narrow.contains("border-right: 0"),
        "stacked diff navigator should become a top region with bottom divider"
    );
}

#[test]
fn served_css_keeps_visible_focus_for_custom_interactive_surfaces() {
    let css = served_app_css();
    for selector in [
        ".diff-nav-file:focus-visible",
        ".diff-nav-fact:focus-visible",
        ".dag-node:focus-visible",
        ".ref[data-ref-kind]:focus-visible",
    ] {
        assert!(
            css.contains(selector),
            "served CSS should style visible focus for {selector}"
        );
    }

    let focus_visible_blocks = css
        .split('}')
        .filter(|block| block.contains(":focus-visible"))
        .collect::<Vec<_>>();
    assert!(
        focus_visible_blocks
            .iter()
            .all(|block| !block.contains("outline: none")),
        "focus-visible rules should not remove every outline without replacement"
    );
}
