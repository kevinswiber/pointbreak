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
//! the Linux/Windows CI legs), and the prospective native producer identity.
//! Both the live payload and the committed fixture pass through the same
//! [`Normalizer`] before comparison: content-addressed ids are canonicalized to
//! first-seen tokens (so cross-references stay meaningful), timestamps collapse
//! to a single token, environment-derived fields are blanked by key, native
//! producers are mapped to the historical fixture producer, and DAG geometry is zeroed. The
//! gate therefore asserts the wire *shape* and every stable value while ignoring
//! the volatile ones.
//!
//! The five fixtures are captured from one store build so their ids and
//! timestamps are mutually consistent (a coherent snapshot the JS tests can join
//! across). Refresh them with `BLESS_WIRE_FIXTURES=1`. When only normalized-away
//! values drift, blessing preserves the committed volatile identities and updates
//! the producer version; any stable wire-shape change refreshes the complete set
//! from one live store and requires its coupled front-end consumers to be updated.

mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;

use pointbreak::session::parse_event_instant;
use serde_json::Value;
use support::inspect::{Inspector, representative_store, urlencode};
use support::pointbreak;

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
        // The ref-continuity block echoes the recorded head and the ref's live
        // tip — real commit OIDs that vary run to run like `headOid`.
        "recordedHeadOid" => Some("<recordedHeadOid>"),
        "currentTipOid" => Some("<currentTipOid>"),
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

fn normalize_producers_for_historical_fixtures(value: &mut Value, version: &str) {
    match value {
        Value::Array(items) => {
            for item in items {
                normalize_producers_for_historical_fixtures(item, version);
            }
        }
        Value::Object(map) => {
            if let Some(Value::Object(producer)) = map.get_mut("producer")
                && producer.get("version").is_some_and(Value::is_string)
            {
                if producer
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| matches!(name, "shore" | "pointbreak"))
                {
                    producer.insert("name".to_owned(), Value::String("shore".to_owned()));
                }
                producer.insert("version".to_owned(), Value::String(version.to_owned()));
            }
            for child in map.values_mut() {
                normalize_producers_for_historical_fixtures(child, version);
            }
        }
        _ => {}
    }
}

fn normalized(value: &Value) -> Value {
    let mut clone = value.clone();
    normalize_producers_for_historical_fixtures(&mut clone, "<producerVersion>");
    Normalizer::new().walk(&mut clone);
    clone
}

/// Preserve the coherent identity set consumed by the web tests when all live
/// payloads differ only in normalized-away values. One stable wire change makes
/// the entire coupled set refresh from the same live store.
fn fixtures_for_bless(payloads: &[(Value, Option<Value>)]) -> Vec<Value> {
    if payloads.iter().any(|(live, committed)| match committed {
        Some(committed) => normalized(live) != normalized(committed),
        None => true,
    }) {
        return payloads.iter().map(|(live, _)| live.clone()).collect();
    }

    payloads
        .iter()
        .map(|(_, committed)| {
            let mut blessed = committed
                .as_ref()
                .expect("preserve mode requires every committed fixture")
                .clone();
            normalize_producers_for_historical_fixtures(&mut blessed, env!("CARGO_PKG_VERSION"));
            blessed
        })
        .collect()
}

fn fixture_for_bless(live: &Value, committed: &Value) -> Value {
    fixtures_for_bless(&[(live.clone(), Some(committed.clone()))])
        .pop()
        .expect("one fixture produces one blessed payload")
}

#[test]
fn normalizer_preserves_historical_fixture_across_native_producer_cutover() {
    let old = serde_json::json!({
        "version": 1,
        "writer": {
            "producer": {
                "name": "shore",
                "version": "0.5.0"
            }
        }
    });
    let current = serde_json::json!({
        "version": 1,
        "writer": {
            "producer": {
                "name": "pointbreak",
                "version": "0.6.0"
            }
        }
    });

    assert_eq!(normalized(&old), normalized(&current));
    assert_eq!(normalized(&current)["version"], 1);
}

#[test]
fn bless_preserves_volatile_identity_when_only_producer_version_changes() {
    let committed = serde_json::json!({
        "version": 1,
        "revisionId": "rev:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "writer": {"producer": {"name": "shore", "version": "0.5.0"}}
    });
    let live = serde_json::json!({
        "version": 1,
        "revisionId": "rev:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "writer": {"producer": {"name": "pointbreak", "version": env!("CARGO_PKG_VERSION")}}
    });

    let blessed = fixture_for_bless(&live, &committed);
    assert_eq!(blessed["revisionId"], committed["revisionId"]);
    assert_eq!(
        blessed["writer"]["producer"]["version"],
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(blessed["version"], 1);
}

#[test]
fn bless_uses_live_payload_when_normalized_wire_shape_changes() {
    let committed = serde_json::json!({
        "revisionId": "rev:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "stable": "old"
    });
    let live = serde_json::json!({
        "revisionId": "rev:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "stable": "new"
    });

    assert_eq!(fixture_for_bless(&live, &committed), live);
}

#[test]
fn bless_refreshes_every_coupled_fixture_when_one_wire_shape_changes() {
    let committed_id =
        "rev:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let live_id = "rev:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let payloads = vec![
        (
            serde_json::json!({"revisionId": live_id, "stable": "new"}),
            Some(serde_json::json!({"revisionId": committed_id, "stable": "old"})),
        ),
        (
            serde_json::json!({"revisionId": live_id, "stable": "same"}),
            Some(serde_json::json!({"revisionId": committed_id, "stable": "same"})),
        ),
    ];

    let blessed = fixtures_for_bless(&payloads);
    assert_eq!(blessed[0], payloads[0].0);
    assert_eq!(blessed[1], payloads[1].0);
}

#[test]
fn bless_refreshes_every_coupled_fixture_when_one_is_missing() {
    let committed_id =
        "rev:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let live_id = "rev:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let payloads = vec![
        (
            serde_json::json!({"revisionId": live_id, "stable": "same"}),
            Some(serde_json::json!({"revisionId": committed_id, "stable": "same"})),
        ),
        (
            serde_json::json!({"revisionId": live_id, "stable": "same"}),
            None,
        ),
    ];

    let blessed = fixtures_for_bless(&payloads);
    assert_eq!(blessed[0], payloads[0].0);
    assert_eq!(blessed[1], payloads[1].0);
}

/// Compare the coupled live `/api` payloads against their committed fixtures,
/// or refresh the complete fixture set with the aggregate safe-bless policy.
fn assert_or_bless(inspector: &Inspector, fixtures: &[(String, &str)]) {
    let bless = std::env::var_os("BLESS_WIRE_FIXTURES").is_some();
    let mut payloads = fixtures
        .iter()
        .map(|(path, fixture)| {
            let live: Value = serde_json::from_str(&inspector.get_text(path))
                .unwrap_or_else(|error| panic!("parse live {path}: {error}"));
            let fixture_path = fixtures_dir().join(fixture);
            let committed = match std::fs::read_to_string(&fixture_path) {
                Ok(raw) => Some(
                    serde_json::from_str(&raw)
                        .unwrap_or_else(|error| panic!("parse fixture {fixture}: {error}")),
                ),
                Err(error) if bless && error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => panic!("read fixture {}: {error}", fixture_path.display()),
            };
            (path, fixture, live, committed)
        })
        .collect::<Vec<_>>();

    if bless {
        std::fs::create_dir_all(fixtures_dir()).expect("create fixtures dir");
        let pairs = payloads
            .iter()
            .map(|(_, _, live, committed)| (live.clone(), committed.clone()))
            .collect::<Vec<_>>();
        for ((_, fixture, _, committed), blessed) in
            payloads.iter_mut().zip(fixtures_for_bless(&pairs))
        {
            *committed = Some(blessed);
            let mut pretty = serde_json::to_string_pretty(
                committed
                    .as_ref()
                    .expect("bless produces a committed fixture"),
            )
            .expect("serialize fixture");
            pretty.push('\n');
            std::fs::write(fixtures_dir().join(fixture), pretty).expect("write fixture");
        }
    }

    for (path, fixture, live, committed) in payloads {
        let committed = committed.expect("non-bless mode requires every committed fixture");
        assert_eq!(
            normalized(&live),
            normalized(&committed),
            "live {path} drifted from committed fixture {fixture}"
        );
    }
}

/// Every read-only `/api` payload the front-end unit tests draw on, captured
/// from one coherent `representative_store()` snapshot.
#[test]
fn api_payloads_match_committed_fixtures() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());

    let fixtures = [
        ("/api/threads".to_owned(), "threads.json"),
        ("/api/history".to_owned(), "history.json"),
        ("/api/revisions".to_owned(), "revisions.json"),
        (
            format!("/api/revisions/{}", urlencode(&store.revision_id)),
            "revision.json",
        ),
        (
            format!("/api/snapshots/{}", urlencode(&store.snapshot_id)),
            "snapshot.json",
        ),
    ];
    assert_or_bless(&inspector, &fixtures);
}

#[test]
fn shared_revision_document_stays_v2_and_excludes_private_continuity() {
    let store = representative_store();
    let repo = store.repo.path().to_str().unwrap();
    let output = pointbreak([
        "revision",
        "show",
        "--repo",
        repo,
        &store.revision_id,
        "--include-body",
    ]);
    assert!(
        output.status.success(),
        "revision show failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let shared: Value = serde_json::from_slice(&output.stdout).expect("revision show JSON");

    assert_eq!(shared["schema"], "pointbreak.review-revision");
    assert_eq!(shared["version"], 2);
    assert!(shared.get("validationContinuity").is_none());
    let keys = shared
        .as_object()
        .expect("shared revision document is an object")
        .keys()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        keys,
        std::collections::BTreeSet::from([
            "assessments",
            "commitRange",
            "currentAssessment",
            "diagnostics",
            "eventCount",
            "eventSetHash",
            "filters",
            "inputRequests",
            "observations",
            "revision",
            "rows",
            "schema",
            "summary",
            "validationChecks",
            "version",
        ]),
        "a shared-document addition requires an explicit schema decision"
    );

    let inspector = Inspector::spawn(store.repo.path());
    let private = inspector.get_json(&format!("/api/revisions/{}", urlencode(&store.revision_id)));
    assert!(private["validationContinuity"].is_object());
    assert_eq!(private["validationChecks"], shared["validationChecks"]);
}
