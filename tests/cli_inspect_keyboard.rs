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
