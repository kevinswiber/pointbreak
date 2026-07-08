mod support;

use support::inspect::{Inspector, representative_store};

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
fn served_assets_preserve_the_advisory_framing_and_competing_peers() {
    let store = representative_store();
    let html = Inspector::spawn(store.repo.path()).get_text("/");
    // The advisory / read-only framing now lives in the store-identity popover note
    // (issue #391 follow-up) rather than a persistent top-bar badge, but it stays in
    // the served markup as rendered text, not a `title`-only tooltip.
    assert!(
        html.contains("never gates writes") && html.contains("reader-relative"),
        "the served shell carries the read-only / advisory framing as rendered text"
    );
}

#[test]
fn served_topbar_uses_the_pointbreak_logo_mark() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());
    let html = inspector.get_text("/");
    assert!(
        html.contains("class=\"brand-mark\"")
            && html.contains("aria-hidden=\"true\"")
            && html.contains("Pointbreak<span class=\"brand-accent\">Review</span>"),
        "the served shell should keep readable text while rendering the decorative logo mark"
    );

    let logo = inspector.get_text("/pointbreak-logo-mono.svg");
    assert!(
        logo.contains("currentColor") && logo.contains("<svg"),
        "the served logo asset should be the currentColor mono Pointbreak mark"
    );
}

#[test]
fn served_shell_exposes_the_pointbreak_favicon() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());
    let html = inspector.get_text("/");
    assert!(
        html.contains(
            r#"<link rel="icon" href="/favicon.svg" type="image/svg+xml" sizes="any" />"#
        ),
        "the served shell should declare the SVG Pointbreak favicon"
    );

    let favicon = inspector.get_text("/favicon.svg");
    assert!(
        favicon.contains("prefers-color-scheme: dark")
            && favicon.contains("#0369a1")
            && favicon.contains("#38bdf8"),
        "the served favicon should carry explicit light/dark Pointbreak colors"
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
    // The narrow detail is a slide-over sheet, not a stacked half-height row:
    // full-height over the list, transform-driven so the open/close animates,
    // with the list's scroll position preserved beneath it.
    let narrow = css
        .split("@media (max-width: 760px)")
        .nth(1)
        .expect("narrow viewport media block exists");
    assert!(
        narrow.contains("#detail") && narrow.contains("position: fixed"),
        "narrow detail should be a full-height sheet over the list"
    );
    assert!(
        narrow.contains("translateX"),
        "the sheet should slide via a transform bound to the open state"
    );
    assert!(
        css.contains("prefers-reduced-motion"),
        "the sheet slide should honor prefers-reduced-motion"
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
