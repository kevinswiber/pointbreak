//! Served-asset contract for the inspector's Cmd/Ctrl-K command palette.
//!
//! The palette unifies jump-to-entity + actions in one searchable overlay and
//! provides the jump capability that lets the non-scaling dropdowns retire. With
//! no JS execution harness, these guard the durable served markup (the overlay
//! slot, its aria roles, a visible placeholder) and the wiring (both Cmd-K and
//! Ctrl-K, router navigation, a copy-view-link action) — never internals.

mod support;

use support::inspect::{Inspector, representative_store};

fn served() -> (String, String) {
    let store = representative_store();
    let insp = Inspector::spawn(store.repo.path());
    (insp.get_text("/"), insp.get_text("/app.js"))
}

#[test]
fn index_html_carries_the_command_palette_overlay() {
    let (html, _js) = served();
    assert!(
        html.contains("id=\"cmd-palette\""),
        "the palette overlay slot exists"
    );
    // A combobox + listbox with a visible, user-facing placeholder.
    assert!(
        html.contains("role=\"combobox\"") || html.contains("role=\"listbox\""),
        "the palette is an aria combobox/listbox"
    );
    assert!(
        html.contains("Jump to") || html.contains("Type a command"),
        "the palette input carries a visible placeholder"
    );
}

#[test]
fn served_app_js_wires_cmd_k_and_routes_through_the_router() {
    let (_html, js) = served();
    // Cmd-K (mac) and Ctrl-K (win/linux) both open it.
    assert!(
        js.contains("metaKey") && js.contains("ctrlKey"),
        "both Cmd-K and Ctrl-K open the palette"
    );
    assert!(
        js.contains("\"k\"") || js.contains("=== \"k\"") || js.contains("key === \"k\""),
        "the K shortcut toggles the palette"
    );
    // Palette actions navigate via the router (single source of truth).
    assert!(
        js.contains("navigate("),
        "palette commands navigate via the router"
    );
    // It copies the current view link (the hash) — a shareable-view action.
    assert!(
        js.contains("location.hash") || js.contains("clipboard"),
        "a command copies the current view link"
    );
}

#[test]
fn served_app_js_prioritizes_actions_and_contextual_revision_labels() {
    let (_html, js) = served();
    let build = js
        .split("function buildCommands()")
        .nth(1)
        .and_then(|tail| tail.split("function togglePalette").next())
        .expect("buildCommands block exists");

    let first_action = build
        .find("Copy current view link")
        .expect("copy current view link action exists");
    let bulk_revisions = build
        .find("for (const u of sortedRevisionEntriesForCommands")
        .expect("bulk revision command loop exists");
    assert!(
        first_action < bulk_revisions,
        "common actions should be registered before bulk revision entries"
    );
    assert!(
        build.contains("currentSelectionCommand()"),
        "palette should add a current-selection command when one exists"
    );

    let helpers = js
        .split("function currentSelectionCommand()")
        .nth(1)
        .and_then(|tail| tail.split("function togglePalette").next())
        .expect("palette command helper block exists");
    assert!(
        helpers.contains("Open current selection")
            && helpers.contains("revisionCommandLabel")
            && helpers.contains("revisionCommandHint")
            && helpers.contains("targetDisplay")
            && helpers.contains("overview"),
        "revision commands should use target/overview context, not only short ids"
    );
}

#[test]
fn served_app_js_opens_palette_through_the_shared_overlay_manager() {
    let (_html, js) = served();
    assert!(
        js.contains("openOverlay(\"palette\", \"#cmd-input\")"),
        "palette should use the shared overlay manager and focus its input"
    );
    assert!(
        js.contains("closeKeyHelp({ restoreFocus: false })"),
        "opening the palette should close or suspend keyboard help"
    );
    assert!(
        js.contains("if (state.diff) closeDiff();"),
        "opening the palette should prevent a stacked diff overlay"
    );
}

#[test]
fn served_palette_exposes_the_active_option_to_the_combobox() {
    let (_html, js) = served();
    let render_palette = js
        .split("function renderPalette()")
        .nth(1)
        .and_then(|tail| tail.split("function movePaletteActive").next())
        .expect("renderPalette block exists");

    assert!(
        render_palette.contains("aria-activedescendant"),
        "palette input should name the active option"
    );
    assert!(
        render_palette.contains("id=\"cmd-option-"),
        "palette options should receive stable DOM ids"
    );
    assert!(
        render_palette.contains("aria-selected"),
        "palette options should continue exposing listbox selection state"
    );
}
