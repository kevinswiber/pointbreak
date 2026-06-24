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
