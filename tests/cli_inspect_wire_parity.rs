//! Wire-parity gate for the inspector front-end test fixtures.
//!
//! The committed JSON fixtures under `src/cli/inspect/web/test/fixtures/` are
//! real `/api/*` payload snapshots from `representative_store()`. This test
//! re-derives each payload from the same store and asserts it still equals the
//! committed snapshot, so a wire-shape change cannot silently rot the JS
//! fixtures the front-end unit tests load. It is a wire-shape gate owned by the
//! Rust side, not an assertion on front-end source text.
//!
//! Many payload fields are environment- or time-derived and differ on every run:
//! content-addressed event/revision/fact ids, event timestamps, the
//! per-test tempdir worktree path and its display label, the base git object
//! ids, the rolling `eventSetHash`/`payloadHash`, and the text-metric'd DAG node
//! geometry (which further differs across platforms, so it must not be pinned on
//! the Linux/Windows CI legs). Both the live payload and the committed fixture
//! pass through the same [`Normalizer`] before comparison: content-addressed ids
//! are canonicalized to first-seen tokens (so cross-references stay meaningful),
//! timestamps collapse to a single token, environment-derived fields are blanked
//! by key, and DAG geometry is zeroed. The gate therefore asserts the wire
//! *shape* and every stable value while ignoring the volatile ones.
//!
//! The five fixtures are captured from one store build so their ids and
//! timestamps are mutually consistent (a coherent snapshot the JS tests can join
//! across). Re-capture them from the live wire with `BLESS_WIRE_FIXTURES=1`.

mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;

use pointbreak::session::parse_event_instant;
use serde_json::Value;
use support::inspect::{Inspector, representative_store, urlencode};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/cli/inspect/web/test/fixtures"
    ))
}

/// A content-addressed opaque id of the form `<prefix>:sha256:<hex>`.
fn is_opaque_id(value: &str) -> bool {
    for prefix in [
        "rev",
        "obj",
        "evt",
        "assess",
        "engagement",
        "assoc-ref",
        "obs",
        "input-request",
        "validation",
    ] {
        if let Some(rest) = value
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_prefix(":sha256:"))
        {
            return !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_hexdigit());
        }
    }
    false
}

/// Either legal event wall-clock timestamp form.
fn is_timestamp(value: &str) -> bool {
    parse_event_instant(value).is_some()
}

/// Server-laid DAG geometry: text-metric'd, so environment- and platform-derived.
fn is_geometry_key(key: &str) -> bool {
    matches!(key, "w" | "x" | "y" | "h")
}

/// Fields whose value is environment- or time-derived and is blanked wholesale.
fn blanked_value(key: &str) -> Option<&'static str> {
    match key {
        "worktreeRoot" => Some("<worktreeRoot>"),
        "commitOid" => Some("<commitOid>"),
        "headOid" => Some("<headOid>"),
        "commitOidShort" => Some("<commitOidShort>"),
        "label" => Some("<label>"),
        "eventSetHash" => Some("<eventSetHash>"),
        "payloadHash" => Some("<payloadHash>"),
        _ => None,
    }
}

/// Canonicalizes the volatile fields of an `/api` payload so two runs over
/// `representative_store()` compare equal. Opaque ids map to stable first-seen
/// tokens (preserving cross-references within a payload); timestamps collapse to
/// one token; environment-derived fields are blanked by key; DAG geometry is
/// zeroed. A fresh `Normalizer` is used per payload, so ids are scoped to one
/// document.
struct Normalizer {
    tokens: BTreeMap<String, String>,
    next: usize,
}

impl Normalizer {
    fn new() -> Self {
        Self {
            tokens: BTreeMap::new(),
            next: 0,
        }
    }

    fn canon(&mut self, raw: &str) -> Option<String> {
        if is_timestamp(raw) {
            return Some("<ts>".to_owned());
        }
        if is_opaque_id(raw) {
            if let Some(token) = self.tokens.get(raw) {
                return Some(token.clone());
            }
            let token = format!("<id#{}>", self.next);
            self.next += 1;
            self.tokens.insert(raw.to_owned(), token.clone());
            return Some(token);
        }
        None
    }

    fn walk(&mut self, value: &mut Value) {
        match value {
            Value::String(text) => {
                if let Some(canon) = self.canon(text) {
                    *text = canon;
                }
            }
            Value::Array(items) => {
                for item in items.iter_mut() {
                    self.walk(item);
                }
            }
            Value::Object(map) => {
                let mut out = serde_json::Map::new();
                for (key, mut value) in std::mem::take(map) {
                    if is_geometry_key(&key) && value.is_number() {
                        value = Value::from(0);
                    } else if let Some(placeholder) = blanked_value(&key) {
                        value = Value::String(placeholder.to_owned());
                    } else {
                        self.walk(&mut value);
                    }
                    let key = self.canon(&key).unwrap_or(key);
                    out.insert(key, value);
                }
                *map = out;
            }
            _ => {}
        }
    }
}

fn normalized(value: &Value) -> Value {
    let mut clone = value.clone();
    Normalizer::new().walk(&mut clone);
    clone
}

/// Compare a live `/api` payload against its committed fixture (both normalized),
/// or rewrite the fixture from the live wire when `BLESS_WIRE_FIXTURES` is set.
fn assert_or_bless(inspector: &Inspector, path: &str, fixture: &str) {
    let live: Value = serde_json::from_str(&inspector.get_text(path))
        .unwrap_or_else(|error| panic!("parse live {path}: {error}"));
    let fixture_path = fixtures_dir().join(fixture);

    if std::env::var_os("BLESS_WIRE_FIXTURES").is_some() {
        std::fs::create_dir_all(fixtures_dir()).expect("create fixtures dir");
        let mut pretty = serde_json::to_string_pretty(&live).expect("serialize fixture");
        pretty.push('\n');
        std::fs::write(&fixture_path, pretty).expect("write fixture");
    }

    let committed: Value = serde_json::from_str(
        &std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|error| panic!("read fixture {}: {error}", fixture_path.display())),
    )
    .unwrap_or_else(|error| panic!("parse fixture {fixture}: {error}"));

    assert_eq!(
        normalized(&live),
        normalized(&committed),
        "live {path} drifted from committed fixture {fixture}"
    );
}

/// Every read-only `/api` payload the front-end unit tests draw on, captured
/// from one coherent `representative_store()` snapshot.
#[test]
fn api_payloads_match_committed_fixtures() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());

    assert_or_bless(&inspector, "/api/threads", "threads.json");
    assert_or_bless(&inspector, "/api/history", "history.json");
    assert_or_bless(&inspector, "/api/revisions", "revisions.json");
    assert_or_bless(
        &inspector,
        &format!("/api/revisions/{}", urlencode(&store.revision_id)),
        "revision.json",
    );
    assert_or_bless(
        &inspector,
        &format!("/api/snapshots/{}", urlencode(&store.snapshot_id)),
        "snapshot.json",
    );
}
