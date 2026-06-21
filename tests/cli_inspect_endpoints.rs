//! Endpoint contract coverage for the `shore inspect` JSON API (issue #110),
//! exercised over real HTTP against a store built at test time. The harness
//! lives in `support::inspect` so multiple inspector suites share one
//! spawn-the-real-server fixture.

mod support;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::inspect::{Inspector, capture, representative_store, urlencode};
use support::shore_env;

/// Find the captured ReviewUnit event id via the public read path (`read_events`).
fn captured_event_id(repo: &std::path::Path) -> String {
    shoreline::session::read_events(repo)
        .unwrap()
        .iter()
        .find(|e| e.event_type == shoreline::session::event::EventType::WorkObjectProposed)
        .expect("a captured review unit")
        .event_id
        .as_str()
        .to_owned()
}

/// Trailing-millisecond stamp from an `occurredAt` string (e.g. `unix-ms:1234`),
/// for asserting chronological ordering without depending on the prefix shape.
fn occurred_ms(entry: &Value) -> u64 {
    let raw = entry["occurredAt"]
        .as_str()
        .expect("occurredAt is a string");
    raw.rsplit(|c: char| !c.is_ascii_digit())
        .find(|chunk| !chunk.is_empty())
        .and_then(|digits| digits.parse().ok())
        .unwrap_or_else(|| panic!("occurredAt carries no trailing ms: {raw}"))
}

fn entries_of_type<'a>(history: &'a Value, event_type: &str) -> Vec<&'a Value> {
    history["entries"]
        .as_array()
        .expect("entries is an array")
        .iter()
        .filter(|e| e["eventType"] == event_type)
        .collect()
}

/// Smoke: the shared harness spawns the real `shore inspect --port 0` server and
/// serves a well-formed history payload for a minimal store.
#[test]
fn inspector_harness_serves_history_for_minimal_store() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    capture(repo.path());

    let inspector = Inspector::spawn(repo.path());
    let history = inspector.get_json("/api/history");

    assert_eq!(history["schema"], "shore.inspect-history");
    // A minimal worktree capture records the capture event plus the auto-recorded
    // capture-time ref association (no separate `review_initialized` event exists).
    let entries = history["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    let event_types: Vec<&str> = entries
        .iter()
        .filter_map(|entry| entry["eventType"].as_str())
        .collect();
    assert!(
        event_types.contains(&"work_object_proposed"),
        "{event_types:?}"
    );
    assert!(
        event_types.contains(&"revision_ref_associated"),
        "{event_types:?}"
    );
}

#[test]
fn api_history_returns_chronological_typed_summaries() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());
    let history = inspector.get_json("/api/history");

    assert_eq!(history["schema"], "shore.inspect-history");
    assert!(
        history["eventSetHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    let entries = history["entries"].as_array().unwrap();
    // capture + auto-recorded ref association + observation + input-request
    // + 2 assessments + 2 validation checks.
    assert_eq!(history["eventCount"], 8);
    assert_eq!(history["historyCount"], 8);
    assert_eq!(entries.len(), 8, "one entry per recorded event");

    // Entries are chronological (occurredAt ascending).
    let stamps: Vec<u64> = entries.iter().map(occurred_ms).collect();
    assert!(
        stamps.windows(2).all(|w| w[0] <= w[1]),
        "entries must be sorted by occurredAt asc: {stamps:?}"
    );

    // Every entry carries the identity fields the UI reads.
    for entry in entries {
        assert!(entry["eventId"].as_str().unwrap().starts_with("evt:"));
        assert!(
            entry["payloadHash"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        );
        assert!(
            entry["writer"]["actorId"]
                .as_str()
                .unwrap()
                .starts_with("actor:")
        );
        // The summary is kind-tagged. The capture event carries a domain-named
        // kind distinct from its envelope event type; every other event's tag
        // matches its event type.
        if entry["eventType"] == "work_object_proposed" {
            assert_eq!(entry["summary"]["kind"], "revision_captured");
        } else {
            assert_eq!(entry["summary"]["kind"], entry["eventType"]);
        }
    }

    // The observation summary carries its title and range target.
    let observations = entries_of_type(&history, "review_observation_recorded");
    assert_eq!(observations.len(), 1);
    let obs = observations[0];
    assert_eq!(obs["summary"]["title"], "Observed change");
    assert_eq!(obs["summary"]["target"]["kind"], "range");
    assert_eq!(obs["summary"]["target"]["filePath"], "src/lib.rs");
    assert_eq!(obs["summary"]["target"]["startLine"], 2);
    assert_eq!(obs["summary"]["target"]["endLine"], 2);
    assert_eq!(obs["trackId"], "agent:codex");
    assert_eq!(obs["subject"]["revisionId"], store.review_unit_id.as_str());
}

#[test]
fn api_units_lists_captured_unit_with_counts_and_target_display() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());
    let units = inspector.get_json("/api/units");

    assert_eq!(units["schema"], "shore.inspect-units");
    assert_eq!(units["reviewUnitCount"], 1);
    let entry = &units["entries"][0];
    assert_eq!(entry["reviewUnitId"], store.review_unit_id.as_str());
    assert_eq!(entry["snapshotId"], store.snapshot_id.as_str());

    // The path-private derived display block is spliced in (regression alongside
    // cli_inspect_target_display.rs).
    assert!(entry["targetDisplay"]["label"].is_string());
    assert_eq!(entry["targetDisplay"]["pathPrivate"], true);
    assert!(entry["targetDisplay"]["head"]["commitOidShort"].is_string());
}

#[test]
fn api_snapshot_returns_snapshot_scoped_artifact() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());
    let snapshot = inspector.get_json(&format!(
        "/api/snapshot?id={}",
        urlencode(&store.snapshot_id)
    ));

    // Snapshot-scoped wire (#146): content hash + frozen diff only — no
    // identity/endpoint fields. Identity/target display live on /api/unit(s).
    assert!(
        snapshot["contentHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert!(snapshot.get("reviewUnitId").is_none());
    assert!(snapshot.get("target").is_none());
    assert!(snapshot.get("base").is_none());
    assert!(snapshot.get("source").is_none());
    assert!(snapshot.get("worktreeRootRedacted").is_none());

    // The captured diff has a real file with a hunk consistent with the edit.
    let files = snapshot["snapshot"]["files"].as_array().unwrap();
    assert!(!files.is_empty());
    let hunks = files[0]["hunks"].as_array().unwrap();
    assert!(!hunks.is_empty());
}

#[test]
fn api_freshness_carries_diagnostic_count_and_matches_history_hash() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());
    let history = inspector.get_json("/api/history");
    let freshness = inspector.get_json("/api/freshness");

    assert_eq!(freshness["schema"], "shore.inspect-freshness");
    // Cheap-probe parity: the freshness hash equals the full history's hash.
    assert_eq!(freshness["eventSetHash"], history["eventSetHash"]);
    assert_eq!(freshness["eventCount"], history["eventCount"]);
    // Post-0062: freshness carries a diagnostic count (0 for a clean local store).
    assert_eq!(freshness["diagnosticCount"], 0);
    assert!(freshness["diagnosticCount"].is_u64());
}

#[test]
fn error_routes_over_real_socket() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());

    // Unknown route → 404 JSON.
    let (status, body) = inspector.get_error("/api/nope");
    assert!(status.contains("404"), "status: {status}");
    assert_eq!(body["error"], "no such route");

    // Missing required ?id= → 400 JSON.
    let (status, body) = inspector.get_error("/api/snapshot");
    assert!(status.contains("400"), "status: {status}");
    assert!(body["error"].as_str().unwrap().contains("id"));

    // Non-GET method → 405.
    let status = inspector.request("POST", "/api/history");
    assert!(status.contains("405"), "status: {status}");
}

#[test]
fn payloads_never_expose_raw_repository_paths_on_path_private_surfaces() {
    let store = representative_store();
    let repo_path = store.repo.path().to_string_lossy().to_string();
    let inspector = Inspector::spawn(store.repo.path());

    // The freshness probe is counts/hash only — genuinely path-free.
    let freshness = inspector.get_json("/api/freshness");
    assert!(!freshness.to_string().contains(&repo_path));

    // The snapshot wire is snapshot-scoped — it carries no endpoint/target at all,
    // so there is no worktree path to leak (the redaction logic is gone).
    let snapshot = inspector.get_json(&format!(
        "/api/snapshot?id={}",
        urlencode(&store.snapshot_id)
    ));
    assert!(snapshot.get("target").is_none());
    assert!(
        !snapshot.to_string().contains(&repo_path),
        "snapshot-scoped wire must not carry the raw worktree path"
    );

    // The derived targetDisplay label on /api/units is always path-private
    // (basename + short OID only), even though the verbatim `target.worktreeRoot`
    // it sits beside legitimately carries the path for a working-tree capture
    // (see finding: no-raw-paths scope widened post-0062).
    let units = inspector.get_json("/api/units");
    let target_display = &units["entries"][0]["targetDisplay"];
    assert!(!target_display.to_string().contains(&repo_path));
}

#[test]
fn inspect_history_endpoint_renders_endorsement_readback() {
    let home = tempfile::tempdir().unwrap();
    let env_home = home.path().to_str().unwrap();
    let env: [(&str, &str); 1] = [("SHORE_HOME", env_home)];
    assert!(
        shore_env(["keys", "init", "--name", "default"], &env)
            .status
            .success()
    );
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn v() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn v() -> u32 { 2 }\n");
    let repo_arg = repo.path().to_str().unwrap();
    // Enroll the default key under kevin (reader trust config in the repo).
    assert!(
        shore_env(
            [
                "keys",
                "enroll",
                "default",
                "--actor",
                "actor:git-email:kevin@swiber.dev",
                "--repo",
                repo_arg,
            ],
            &env,
        )
        .status
        .success()
    );
    // Capture UNSIGNED so the detached endorsement carrier is not deduped, then endorse.
    assert!(
        shore_env(
            ["review", "capture", "--repo", repo_arg],
            &[("SHORE_HOME", env_home), ("SHORE_SIGNING", "off")],
        )
        .status
        .success()
    );
    let target = captured_event_id(repo.path());
    assert!(
        shore_env(
            ["review", "endorse", &target, "--repo", repo_arg],
            &[
                ("SHORE_HOME", env_home),
                ("SHORE_ACTOR_ID", "actor:git-email:kevin@swiber.dev"),
            ],
        )
        .status
        .success()
    );

    // The inspector reads the repo's reader config (allowed-signers.json), so it
    // resolves the same reader-relative classification as the CLI.
    let inspector = Inspector::spawn(repo.path());
    let history = inspector.get_json("/api/history");
    let endorsement = history["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|e| e.get("endorsements").and_then(|x| x.get(0)))
        .expect("an endorsement readback in the inspector history");
    assert_eq!(endorsement["classification"], "endorsement-trusted");
    assert_eq!(endorsement["endorser"], "actor:git-email:kevin@swiber.dev");
}
