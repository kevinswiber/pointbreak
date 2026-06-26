//! Served-asset contract for the inspector's keyboard layer + operable chips.
//!
//! One delegated keydown layer replaces the lone Escape listener: it steps the
//! selection, focuses search, jumps lenses, layers Escape, toggles a cheat
//! sheet, and makes the focusable reference chips operable. With no JS execution
//! harness, these guard the durable wiring (the keydown layer, the input gate,
//! chip Enter/Space activation, the cheat-sheet overlay's visible title) — never
//! handler internals.

mod support;

use support::inspect::{Inspector, representative_store};

fn served_app_js() -> String {
    let store = representative_store();
    Inspector::spawn(store.repo.path()).get_text("/app.js")
}

#[test]
fn served_app_js_wires_a_global_keyboard_layer() {
    let js = served_app_js();
    // A single delegated keydown handler replaces the lone Escape listener.
    assert!(js.contains("keydown"), "the app wires a keydown layer");
    // The layer is gated so typing in the search box is not captured (durable
    // guard: it inspects the active element's tag).
    assert!(
        js.contains("tagName") || js.contains("INPUT") || js.contains("TEXTAREA"),
        "the keyboard layer ignores keystrokes while an input/textarea is focused"
    );
}

#[test]
fn served_chips_are_operable_by_keyboard() {
    let js = served_app_js();
    // The already-focusable reference chips (role=link tabindex=0) gain Enter/Space activation.
    assert!(
        js.contains("Enter") && js.contains("\" \"")
            || js.contains("Spacebar")
            || js.contains("\"Space\""),
        "focused reference chips activate on Enter/Space"
    );
    // Chip activation still resolves the named reference (durable: it routes to
    // resolveRef, which navigates via the router).
    assert!(
        js.contains("resolveRef"),
        "keyboard chip activation resolves the reference like the click path"
    );
}

#[test]
fn served_assets_carry_a_keyboard_cheat_sheet() {
    let store = representative_store();
    let html = Inspector::spawn(store.repo.path()).get_text("/");
    // A stable overlay slot with a visible, user-facing title (not a private fn name).
    assert!(
        html.contains("id=\"key-help\""),
        "a keyboard cheat-sheet overlay slot exists"
    );
    assert!(
        html.contains("Keyboard shortcuts"),
        "the cheat sheet carries a visible title"
    );
}

#[test]
fn served_keyboard_help_lists_shipped_shortcuts() {
    let store = representative_store();
    let html = Inspector::spawn(store.repo.path()).get_text("/");
    let help = html
        .split("id=\"key-help\"")
        .nth(1)
        .and_then(|tail| tail.split("<script").next())
        .expect("keyboard help overlay markup exists");

    for shortcut in [
        "<kbd>Cmd</kbd>",
        "<kbd>Ctrl</kbd>",
        "<kbd>Shift</kbd>",
        "<kbd>K</kbd>",
        "<kbd>P</kbd>",
        "<kbd>n</kbd>",
        "<kbd>p</kbd>",
        "<kbd>]</kbd>",
        "<kbd>[</kbd>",
        "<kbd>j</kbd>",
        "<kbd>k</kbd>",
        "<kbd>/</kbd>",
        "<kbd>g</kbd>",
        "<kbd>Esc</kbd>",
        "<kbd>?</kbd>",
    ] {
        assert!(
            help.contains(shortcut),
            "keyboard help should list {shortcut}"
        );
    }
}

#[test]
fn served_app_js_exposes_filter_type_toggles_as_pressed_buttons() {
    let js = served_app_js();
    let render_type_toggles = js
        .split("function renderTypeToggles()")
        .nth(1)
        .and_then(|tail| tail.split("function objectThreads()").next())
        .expect("renderTypeToggles block exists");

    assert!(
        render_type_toggles.contains("aria-pressed"),
        "type filter buttons should expose pressed state"
    );
    assert!(
        render_type_toggles.contains("aria-label"),
        "type filter buttons should expose short accessible names"
    );
}

#[test]
fn served_lens_buttons_do_not_use_selected_state_without_tab_semantics() {
    let store = representative_store();
    let html = Inspector::spawn(store.repo.path()).get_text("/");
    let lens = html
        .split("id=\"lens-switcher\"")
        .nth(1)
        .and_then(|tail| tail.split("</nav>").next())
        .expect("lens switcher markup exists");
    let tab_model = lens.contains("role=\"tablist\"") && lens.contains("role=\"tab\"");
    let pressed_button_model = lens.contains("aria-pressed") && !lens.contains("aria-selected");

    assert!(
        tab_model || pressed_button_model,
        "lens switcher should either be a real tablist or use pressed button state"
    );

    let js = served_app_js();
    let render_lens_switcher = js
        .split("function renderLensSwitcher()")
        .nth(1)
        .and_then(|tail| tail.split("function syncControls()").next())
        .expect("renderLensSwitcher block exists");
    assert!(
        tab_model || render_lens_switcher.contains("aria-pressed"),
        "lens switcher render path should update the chosen state attribute"
    );
}

#[test]
fn served_app_js_uses_one_overlay_focus_manager() {
    let js = served_app_js();
    assert!(
        js.contains("function openOverlay(")
            && js.contains("function closeOverlay(")
            && js.contains("function trapOverlayFocus("),
        "diff, palette, and help should share one overlay/focus manager"
    );
    assert!(
        js.contains("if (trapOverlayFocus(ev)) return;"),
        "the keyboard layer should trap Tab inside the active overlay"
    );
    assert!(
        js.contains("openOverlay(\"help\", \"#key-help-close\")"),
        "keyboard help opens through the shared manager with an initial focus target"
    );
    assert!(
        js.contains("closeActiveOverlay({ restoreFocus: false })"),
        "opening one overlay should close or suspend another overlay first"
    );
}
