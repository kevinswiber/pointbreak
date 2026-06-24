//! Served-asset contract for the inspector's indexed, structured filter.
//!
//! The per-keystroke whole-event `JSON.stringify` scan is replaced by a
//! once-per-load search index and a small `field:value` + free-text grammar,
//! with the query serialized as `q=` in the hash. With no JS execution harness,
//! these guard the durable observable facts (the scan is gone, the index is
//! built in load(), the field vocabulary is present, the query round-trips
//! through the hash) — never the parser internals.

mod support;

use support::inspect::{Inspector, representative_store};

fn served_app_js() -> String {
    let store = representative_store();
    Inspector::spawn(store.repo.path()).get_text("/app.js")
}

#[test]
fn served_app_js_drops_the_per_keystroke_whole_event_stringify() {
    let js = served_app_js();
    // The O(store bytes)/keystroke whole-event scan is retired.
    assert!(
        !js.contains("JSON.stringify(e).toLowerCase()"),
        "the per-keystroke whole-event stringify scan must be gone"
    );
    // The index is built once in load() (a per-entry haystack), not per keystroke.
    let load = {
        let start = js.find("async function load()").expect("load()");
        let rest = &js[start..];
        let end = rest
            .find("async function pollFreshness")
            .expect("pollFreshness after load");
        &rest[..end]
    };
    assert!(
        load.contains("buildHaystack") || load.contains("__search"),
        "load() builds the per-entry search index once"
    );
}

#[test]
fn served_app_js_parses_a_field_value_query_grammar() {
    let js = served_app_js();
    // The structured fields the grammar recognizes (durable user-facing vocabulary).
    for field in ["type:", "track:", "revision:", "object:", "status:"] {
        assert!(js.contains(field), "the query grammar recognizes `{field}`");
    }
    // The id-shaped value classifier is reused (not re-implemented).
    assert!(
        js.contains("refInfo"),
        "field:value reuses the refInfo id classifier"
    );
}

#[test]
fn served_app_js_threads_the_query_through_the_hash() {
    let js = served_app_js();
    // The query is the single source of truth, serialized as q= via the router.
    assert!(
        js.contains("q="),
        "the query serializes to the q= hash param"
    );
    assert!(
        js.contains("navigate("),
        "search edits navigate (replaceState) instead of mutating filter state directly"
    );
}
