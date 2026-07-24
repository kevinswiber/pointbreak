//! Guard: the historical "review unit" vocabulary is retired from internal Rust
//! identifiers. The work object is a revision; every `ReviewUnit*`/`review_unit*`
//! type, function, field, module, and test name has moved to its `Revision*`
//! spelling.
//!
//! The only surviving occurrences are legacy *wire-string literals* — the
//! pre-rename event-type strings that decode tests assert are rejected, the
//! dropped envelope-key aliases that serialization tests assert are absent, and
//! the legacy input-request target `kind`. Those name a historical on-the-wire
//! spelling, not a live identifier, so they stay verbatim and are allow-listed
//! below. Anything else is a missed rename and fails this guard.

use std::fs;
use std::path::Path;

/// Legacy wire-string literals (quoted) that legitimately survive: stripped from
/// each candidate line before the residual check. Longest-first so a shorter
/// prefix never partially consumes a longer literal.
fn allowed_legacy_wire_literals() -> Vec<&'static str> {
    let mut literals = vec![
        // Pre-rename capture / association / lineage event-type wire strings the
        // decode-rejection tests assert no longer parse.
        "\"review_unit_captured\"",
        "\"review_unit_ref_associated\"",
        "\"review_unit_ref_withdrawn\"",
        "\"review_unit_commit_associated\"",
        "\"review_unit_commit_withdrawn\"",
        "\"review_unit_lineage_declared\"",
        "\"review_unit_lineage_round_recorded\"",
        // Dropped envelope-key alias a serialization test asserts is absent —
        // a retired camelCase wire spelling (never a live Rust identifier), so
        // both the `json.get(...)` probe and its describing message survive.
        "currentReviewUnitId",
        // Legacy input-request target kind a decode-rejection test feeds.
        "\"review_unit\"",
    ];
    literals.sort_by_key(|literal| std::cmp::Reverse(literal.len()));
    literals
}

fn assert_no_review_unit_identifier(path: &Path, allowed: &[&str]) {
    let contents = fs::read_to_string(path).unwrap_or_default();
    for (index, line) in contents.lines().enumerate() {
        // Build the forbidden tokens at runtime so a future blanket re-run of a
        // rename `sed` over this guard cannot silently rewrite them.
        let snake = format!("review{}unit", "_");
        let pascal = format!("Review{}nit", "U");
        if !line.contains(&snake) && !line.contains(&pascal) {
            continue;
        }
        let mut residual = line.to_owned();
        for literal in allowed {
            residual = residual.replace(literal, "");
        }
        assert!(
            !residual.contains(&snake) && !residual.contains(&pascal),
            "{}:{} still names the retired work object \"review unit\": {}",
            path.display(),
            index + 1,
            line.trim()
        );
    }
}

fn visit_rust_sources(dir: &Path, allowed: &[&str]) {
    for entry in fs::read_dir(dir).expect("read src dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            visit_rust_sources(&path, allowed);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            assert_no_review_unit_identifier(&path, allowed);
        }
    }
}

#[test]
fn no_review_unit_identifier_remains_in_source() {
    let src = env::manifest_dir().join("src");
    let allowed = allowed_legacy_wire_literals();
    visit_rust_sources(&src, &allowed);
}

// Runtime-resolved binary/manifest paths for cross-machine (e.g. Windows) archive runs.
#[path = "support/env.rs"]
#[allow(dead_code)]
mod env;
