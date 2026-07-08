//! Endpoint contract coverage for the `shore inspect` JSON API (issue #110),
//! exercised over real HTTP against a store built at test time. The harness
//! lives in `support::inspect` so multiple inspector suites share one
//! spawn-the-real-server fixture.

mod support;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::inspect::{Inspector, capture, representative_store, urlencode};
use support::shore_env;

/// Find the captured Revision event id via the public read path (`read_events`).
fn captured_event_id(repo: &std::path::Path) -> String {
    pointbreak::session::read_events(repo)
        .unwrap()
        .iter()
        .find(|e| e.event_type == pointbreak::session::event::EventType::WorkObjectProposed)
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

/// Spawn the inspector against a throwaway repo for served-asset contract checks.
/// The static asset routes never read the store, so a bare repo keeps the
/// served-copy assertions cheap. The returned [`GitRepo`] must be held for the
/// lifetime of the [`Inspector`].
fn served_asset_inspector() -> (GitRepo, Inspector) {
    let repo = GitRepo::new();
    let inspector = Inspector::spawn(repo.path());
    (repo, inspector)
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

    assert_eq!(history["schema"], "pointbreak.inspect-history");
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

    assert_eq!(history["schema"], "pointbreak.inspect-history");
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
    assert_eq!(obs["subject"]["revisionId"], store.revision_id.as_str());
}

#[test]
fn api_units_lists_captured_unit_with_counts_and_target_display() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());
    let units = inspector.get_json("/api/revisions");

    assert_eq!(units["schema"], "pointbreak.inspect-revisions");
    assert_eq!(units["revisionCount"], 1);
    let entry = &units["entries"][0];
    assert_eq!(entry["revisionId"], store.revision_id.as_str());
    assert_eq!(entry["snapshotId"], store.snapshot_id.as_str());

    // The path-private derived display block is spliced in (regression alongside
    // cli_inspect_target_display.rs).
    assert!(entry["targetDisplay"]["label"].is_string());
    assert_eq!(entry["targetDisplay"]["pathPrivate"], true);
    assert!(entry["targetDisplay"]["head"]["commitOidShort"].is_string());
}

#[test]
fn api_units_include_additive_overview_summary() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());
    let units = inspector.get_json("/api/revisions");
    let revision = inspector.get_json(&format!("/api/revisions/{}", urlencode(&store.revision_id)));

    let entry = &units["entries"][0];
    assert_eq!(entry["revisionId"], store.revision_id.as_str());
    assert_eq!(entry["snapshotId"], store.snapshot_id.as_str());
    assert!(entry["target"].is_object());
    assert!(entry["base"].is_object());
    assert!(entry["targetDisplay"].is_object());

    let overview = &entry["overview"];
    assert_eq!(overview["currentAssessment"]["status"], "resolved");
    assert_eq!(overview["currentAssessment"]["assessment"], "accepted");

    let attention = &overview["attention"];
    assert_eq!(attention["unassessed"], false);
    assert_eq!(attention["acceptedWithFollowUp"], false);
    assert_eq!(attention["openInputRequestCount"], 1);
    assert_eq!(attention["failedValidationCount"], 1);
    assert_eq!(attention["erroredValidationCount"], 0);
    assert_eq!(attention["staleFactCount"], 0);

    let counts = &overview["counts"];
    assert_eq!(counts["files"], revision["summary"]["fileCount"]);
    assert_eq!(counts["rows"], revision["summary"]["rowCount"]);
    assert_eq!(
        counts["observations"],
        revision["summary"]["observationCount"]
    );
    assert_eq!(
        counts["inputRequests"],
        revision["summary"]["inputRequestCount"]
    );
    assert_eq!(
        counts["assessments"],
        revision["summary"]["assessmentCount"]
    );
    assert_eq!(
        counts["validationChecks"],
        revision["summary"]["validationCheckCount"]
    );

    let latest_activity = &overview["latestActivity"];
    if !latest_activity.is_null() {
        assert!(latest_activity["kind"].is_string());
        assert!(latest_activity["title"].is_string());
        assert!(latest_activity["at"].is_string());
    }
}

#[test]
fn api_snapshot_returns_snapshot_scoped_artifact() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());
    let snapshot = inspector.get_json(&format!("/api/snapshots/{}", urlencode(&store.snapshot_id)));

    // Object-scoped wire (#146): content hash + frozen diff only — no
    // identity/endpoint fields. Identity/target display live on /api/revisions(/{id}).
    assert!(
        snapshot["contentHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert!(snapshot.get("revisionId").is_none());
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
fn api_freshness_exposes_the_event_count_marker() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());
    let history = inspector.get_json("/api/history");
    let freshness = inspector.get_json("/api/freshness");

    assert_eq!(freshness["schema"], "pointbreak.inspect-freshness");
    // The cheap change key is the event-log head marker — the event count, equal
    // to what a full history read reports, but computed without the full read.
    // (Monotonic-on-append and stable-across-envelope-edit are proven at the
    // journal level, in the backend `head_marker` tests.)
    assert!(freshness["eventCount"].is_u64());
    assert_eq!(freshness["eventCount"], history["eventCount"]);
    // The full-read fields are gone from the cheap probe: the freshness path no
    // longer folds or hashes the log. The event-set hash stays the authoritative
    // confirm stamp on the full-read endpoints (asserted for /api/history above).
    assert!(freshness.get("eventSetHash").is_none());
    assert!(freshness.get("diagnosticCount").is_none());
}

#[test]
fn error_routes_over_real_socket() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());

    // Unknown route → 404 JSON.
    let (status, body) = inspector.get_error("/api/nope");
    assert!(status.contains("404"), "status: {status}");
    assert_eq!(body["error"], "no such route");

    // Missing required path member → 400 JSON.
    let (status, body) = inspector.get_error("/api/snapshots/");
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

    // The freshness probe is a bare event count — genuinely path-free.
    let freshness = inspector.get_json("/api/freshness");
    assert!(!freshness.to_string().contains(&repo_path));

    // The snapshot wire is object-scoped — it carries no endpoint/target at all,
    // so there is no worktree path to leak (the redaction logic is gone).
    let snapshot = inspector.get_json(&format!("/api/snapshots/{}", urlencode(&store.snapshot_id)));
    assert!(snapshot.get("target").is_none());
    assert!(
        !snapshot.to_string().contains(&repo_path),
        "object-scoped wire must not carry the raw worktree path"
    );

    // The derived targetDisplay label on /api/revisions is always path-private
    // (basename + short OID only), even though the verbatim `target.worktreeRoot`
    // it sits beside legitimately carries the path for a working-tree capture
    // (see finding: no-raw-paths scope widened post-0062).
    let units = inspector.get_json("/api/revisions");
    let target_display = &units["entries"][0]["targetDisplay"];
    assert!(!target_display.to_string().contains(&repo_path));
}

#[test]
fn inspect_history_endpoint_renders_endorsement_readback() {
    let home = tempfile::tempdir().unwrap();
    let env_home = home.path().to_str().unwrap();
    let env: [(&str, &str); 1] = [("SHORE_HOME", env_home)];
    assert!(
        shore_env(["key", "init", "--name", "default"], &env)
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
                "key",
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
            ["capture", "--repo", repo_arg],
            &[("SHORE_HOME", env_home), ("SHORE_SIGNING", "off")],
        )
        .status
        .success()
    );
    let target = captured_event_id(repo.path());
    assert!(
        shore_env(
            ["endorse", &target, "--repo", repo_arg],
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

#[test]
fn tokens_css_is_served_as_the_single_token_source() {
    let (_repo, inspector) = served_asset_inspector();

    // The fourth served asset is reachable and carries the semantic status
    // aliases, the radii scale, and the sans stack — the single token source.
    let (status, body) = inspector.raw_get("/tokens.css");
    assert!(
        status.contains("200 OK"),
        "tokens.css is served, got {status}"
    );
    assert!(
        body.contains("--success"),
        "tokens.css holds the semantic status aliases"
    );
    assert!(
        body.contains("--r-md"),
        "tokens.css carries the radii scale"
    );
    assert!(body.contains("--sans"), "tokens.css carries the sans stack");

    // index.html links tokens.css before app.css so the cascade resolves vars first.
    let index = inspector.get_text("/");
    let tokens_at = index.find("/tokens.css").expect("index links /tokens.css");
    let app_at = index.find("/app.css").expect("index links /app.css");
    assert!(
        tokens_at < app_at,
        "tokens.css must be linked before app.css"
    );
}

#[test]
fn app_css_no_longer_declares_the_root_token_block() {
    let (_repo, inspector) = served_asset_inspector();
    let app_css = inspector.get_text("/app.css");
    // The single :root lives in tokens.css now; app.css is component rules only.
    // Match the bare selector so `:root{` (no space) cannot slip past the guard.
    assert!(
        !app_css.contains(":root"),
        "app.css must not declare any :root token block (single-sourced in tokens.css)"
    );
}

#[test]
fn design_system_docs_state_the_component_state_contract() {
    let readme = include_str!("../src/cli/inspect/design-system/README.md");
    let styles = include_str!("../src/cli/inspect/design-system/styles.css");

    assert!(
        !styles.contains("mirror the live inspector 1:1"),
        "the gallery stylesheet must not claim complete live-app mirroring"
    );
    assert!(
        readme.contains("component/state preview"),
        "README names the narrower gallery contract"
    );
    assert!(
        readme.contains("not a full live-app mirror"),
        "README says runtime behavior belongs to the live inspector"
    );
    assert!(
        readme.contains("shared tokens"),
        "README keeps token sharing explicit"
    );
    assert!(
        readme.contains("status/readback/diff/shell/feedback"),
        "README names the critical parity surface set"
    );
    assert!(
        !readme.contains("PR #261") && !styles.contains("PR #261"),
        "gallery docs should describe current state, not a completed PR redo"
    );
}

#[test]
fn design_system_gallery_covers_live_shell_and_overlay_states() {
    let bodies = [
        include_str!("../src/cli/inspect/design-system/_bodies/navigation-topbar.body.html"),
        include_str!("../src/cli/inspect/design-system/_bodies/inputs-controls.body.html"),
        include_str!("../src/cli/inspect/design-system/_bodies/feedback-diagnostics.body.html"),
        include_str!("../src/cli/inspect/design-system/_bodies/data-diff.body.html"),
    ]
    .join("\n");
    let styles = include_str!("../src/cli/inspect/design-system/styles.css");
    let bake = include_str!("../src/cli/inspect/design-system/_bodies/bake.sh");

    for marker in [
        "id=\"topbar\"",
        "id=\"lens-switcher\"",
        "class=\"lens-tab\"",
        "class=\"split\"",
        "id=\"master\"",
        "id=\"detail\"",
        "class=\"route-diagnostic\"",
        "id=\"cmd-palette\"",
        "class=\"cmd-item\"",
        "class=\"cmd-empty\"",
        "id=\"key-help\"",
        "class=\"key-help-list\"",
        "class=\"advisory-note\"",
        "class=\"reader-scope-note\"",
        "class=\"modal\"",
        "class=\"modal-card\"",
        "data-ds-state=\"narrow\"",
    ] {
        assert!(bodies.contains(marker), "gallery bodies include {marker}");
    }

    for selector in [
        ".lens-tab",
        ".route-diagnostic",
        ".cmd-item",
        ".key-help-list",
        ".advisory-note",
        ".reader-scope-note",
        ".modal-card",
        ".split",
    ] {
        assert!(
            styles.contains(selector),
            "gallery styles include {selector}"
        );
    }

    for output in [
        "navigation/topbar-light.html",
        "inputs/controls-light.html",
        "feedback/diagnostics-light.html",
    ] {
        assert!(bake.contains(output), "bake matrix includes {output}");
    }

    let normalized_bake = bake.split_whitespace().collect::<Vec<_>>().join(" ");
    for named_pair in [
        "bake navigation-topbar.body.html navigation/topbar.html Navigation \"Navigation — top bar, tabs, stats\" \"\" \"\" \"Navigation — dark\" \"Dark theme\"",
        "bake inputs-controls.body.html inputs/controls.html Inputs \"Inputs — toolbar, buttons, toggles\" \"\" \"\" \"Inputs — dark\" \"Dark theme\"",
        "bake feedback-diagnostics.body.html feedback/diagnostics.html Feedback \"Feedback — diagnostics & errors\" \"\" \"\" \"Feedback — dark\" \"Dark theme\"",
    ] {
        assert!(
            normalized_bake.contains(named_pair),
            "dark theme twin carries explicit marker name + subtitle: {named_pair}"
        );
    }
}

#[test]
fn design_system_gallery_keeps_dag_edges_and_current_copy_in_sync() {
    let styles = include_str!("../src/cli/inspect/design-system/styles.css");
    let data_cards = include_str!("../src/cli/inspect/design-system/_bodies/data-cards.body.html");

    assert!(
        styles.contains("--dag-edge"),
        "gallery DAG styles should carry the same default edge token as the live inspector"
    );
    let edge_block = styles
        .split(".dag-edge {")
        .nth(1)
        .and_then(|rest| rest.split('}').next())
        .expect(".dag-edge block");
    assert!(
        edge_block.contains("stroke: var(--dag-edge)")
            && !edge_block.contains("stroke: var(--border)"),
        "gallery DAG edges should not fall back to only the quiet border token: {edge_block}"
    );
    let arrow_block = styles
        .split(".dag-arrow-head {")
        .nth(1)
        .and_then(|rest| rest.split('}').next())
        .expect(".dag-arrow-head block");
    assert!(
        arrow_block.contains("fill: var(--dag-edge)")
            && !arrow_block.contains("fill: var(--border)"),
        "gallery DAG arrowheads should share the stronger edge token: {arrow_block}"
    );
    assert!(
        data_cards.contains("current in thread"),
        "gallery revision-card examples should use the live current-state wording"
    );
    assert!(
        !data_cards.contains(">head</span>") && !data_cards.contains("revision thread · head "),
        "gallery examples should avoid the old bare head copy"
    );
}

// The theme flip, the OS-preference default, and the localStorage round-trip are
// runtime client behavior; with no JS execution harness in the served envelope they
// cannot be unit-tested. These assert the served-copy contracts that the wiring is
// present — stable, user-visible strings and attributes — over the HTTP harness.
#[test]
fn tokens_css_carries_a_light_theme_override_block() {
    let (_repo, inspector) = served_asset_inspector();
    let tokens = inspector.get_text("/tokens.css");
    // The light theme is a semantic-alias override, not a second :root.
    assert!(
        tokens.contains("[data-theme=\"light\"]"),
        "tokens.css carries the light-theme alias override block"
    );
    // color-scheme is declared so native controls/scrollbars match the theme.
    assert!(
        tokens.contains("color-scheme"),
        "tokens.css declares color-scheme"
    );
}

#[test]
fn topbar_exposes_an_accessible_theme_toggle() {
    let (_repo, inspector) = served_asset_inspector();
    let index = inspector.get_text("/");
    // A stable, user-visible accessible name for the control (assert the aria
    // label, a durable contract — not the button's id). The label now names the
    // control and carries its current value (`Color theme: <mode>`), so assert the
    // durable prefix — the value after it is runtime state.
    assert!(
        index.contains("aria-label=\"Color theme:"),
        "the topbar carries an accessible theme toggle"
    );
}

// The rendered glyphs/shapes and the density flip are runtime CSS/DOM behavior;
// these assert the served-copy CSS contracts that the redundancy layer and the
// tokens are present, over the HTTP harness.
#[test]
fn status_palette_has_a_non_color_redundancy_layer() {
    let (_repo, inspector) = served_asset_inspector();
    let app_css = inspector.get_text("/app.css");
    // Color is not the only channel: each status carries a glyph via ::before content.
    assert!(
        app_css.contains("::before") && app_css.contains("content:"),
        "status classes carry a per-state glyph so meaning never rides on hue alone"
    );
    // Head vs superseded differs in border style, not only hue.
    assert!(
        app_css.contains("dashed"),
        "superseded revisions read as dashed, a shape cue beyond color"
    );
    // ID-heavy mono columns get tabular figures + slashed zero.
    assert!(
        app_css.contains("tabular-nums") && app_css.contains("slashed-zero"),
        "mono/id columns disambiguate 0/O and align digits"
    );
}

#[test]
fn positive_advisory_states_use_text_not_checkmark_glyphs() {
    let (_repo, inspector) = served_asset_inspector();
    let app_css = inspector.get_text("/app.css");
    let gallery_css = include_str!("../src/cli/inspect/design-system/styles.css");
    let gallery_bodies = [
        include_str!("../src/cli/inspect/design-system/_bodies/data-diff.body.html"),
        include_str!("../src/cli/inspect/design-system/_bodies/data-review-facts.body.html"),
        include_str!("../src/cli/inspect/design-system/_bodies/data-timeline.body.html"),
        include_str!("../src/cli/inspect/design-system/_bodies/feedback-diagnostics.body.html"),
    ]
    .join("\n");

    for (label, css) in [("app", app_css.as_str()), ("gallery", gallery_css)] {
        assert!(
            !css.contains("content: \"✓\""),
            "{label} CSS should not use a positive checkmark glyph"
        );
        for selector in [
            ".fact-status.passed::before",
            ".fact-status.responded::before",
            ".fact-status.current::before",
            ".verdict-accepted .verdict-value::before",
            ".verdict-accepted_with_follow_up .verdict-value::before",
            ".s-added::before",
        ] {
            assert!(
                !css.contains(selector),
                "{label} CSS should leave positive state labels text-only: {selector}"
            );
        }
        // The signature readback is the one positive state that leads with a
        // marker: a neutral middot (never a checkmark, guarded above) so a valid
        // timeline row still carries a status mark alongside the non-positive
        // glyphs. The served inspector and the gallery mirror stay in sync on it.
        assert!(
            css.contains(".verify-valid::before") && css.contains("content: \"·\""),
            "{label} signature readback leads valid rows with a neutral middot, not a checkmark"
        );
        for glyph in [
            "content: \"✕\"",
            "content: \"!\"",
            "content: \"?\"",
            "content: \"~\"",
            "content: \"○\"",
        ] {
            assert!(
                css.contains(glyph),
                "{label} CSS keeps non-positive status glyph redundancy: {glyph}"
            );
        }
    }

    for label in [
        "accepted",
        "passed",
        "responded",
        "current",
        "signature valid",
        "added",
    ] {
        assert!(
            gallery_bodies.contains(label),
            "gallery examples keep the positive state text label visible: {label}"
        );
    }
}

#[test]
fn tokens_css_carries_the_type_scale() {
    let (_repo, inspector) = served_asset_inspector();
    let tokens = inspector.get_text("/tokens.css");
    for step in ["--fs-xs", "--fs-sm", "--fs-md", "--fs-base"] {
        assert!(
            tokens.contains(step),
            "tokens.css carries the {step} type-scale step"
        );
    }
}

#[test]
fn topbar_exposes_a_density_toggle() {
    let (_repo, inspector) = served_asset_inspector();
    let index = inspector.get_text("/");
    // The label names the control and carries its current value
    // (`Density: <mode>`); assert the durable prefix, the value is runtime state.
    assert!(
        index.contains("aria-label=\"Density:"),
        "the topbar carries an accessible comfortable/compact density toggle"
    );
}

// The advisory / read-only framing is rendered DOM text, not a `title`-only tooltip.
// It moved from a persistent top-bar badge into the store-identity popover note
// (issue #391 follow-up): the badge was redundant on a wholly read-only tool, but the
// substantive posture — recorded state only, reader-relative verification — stays in
// the served index.html as a real `<p>`, not hidden behind a `title` attribute.
#[test]
fn advisory_framing_is_rendered_text_not_tooltip_only() {
    let (_repo, inspector) = served_asset_inspector();
    let index = inspector.get_text("/");

    assert!(
        index.contains("never gates writes") && index.contains("reader-relative"),
        "the store-identity popover carries the read-only / advisory framing as rendered text"
    );
}

#[test]
fn api_history_windows_with_limit_and_continues_via_offset() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());

    let full = inspector.get_json("/api/history");
    let total = full["entries"].as_array().unwrap().len();
    let event_count = full["eventCount"].clone();
    assert!(
        total >= 2,
        "representative store should yield multiple history entries, got {total}"
    );

    let page1 = inspector.get_json("/api/history?limit=1");
    assert_eq!(page1["entries"].as_array().unwrap().len(), 1);
    assert_eq!(page1["offset"], 0);
    // Identity always reports the full event set, never the window.
    assert_eq!(page1["eventCount"], event_count);
    assert_eq!(page1["matchCount"], total);

    // The window is positional: the next page is offset=1. No opaque cursor.
    let page2 = inspector.get_json("/api/history?limit=1&offset=1");
    assert_eq!(page2["entries"].as_array().unwrap().len(), 1);
    assert_eq!(page2["offset"], 1);
    // Page two continues strictly after page one — no overlap.
    assert_ne!(
        page2["entries"][0]["eventId"],
        page1["entries"][0]["eventId"]
    );
    // The endpoint no longer carries an opaque cursor — paging is positional.
    assert!(full.as_object().unwrap().get("nextCursor").is_none());
    assert!(page1.as_object().unwrap().get("nextCursor").is_none());
}

#[test]
fn api_history_rejects_malformed_window_params() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());

    // A non-numeric limit is a usage error.
    assert!(
        inspector
            .get_error("/api/history?limit=abc")
            .0
            .contains("400")
    );
    // A non-numeric offset is a usage error too.
    assert!(
        inspector
            .get_error("/api/history?offset=abc")
            .0
            .contains("400")
    );
    // A stray `cursor=` is not a recognized param — it is ignored, not a 400.
    assert_eq!(
        inspector.get_json("/api/history?cursor=whatever")["matchCount"],
        inspector.get_json("/api/history")["matchCount"]
    );
}

#[test]
fn api_history_q_filters_entries_and_reports_facets_and_match_count() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());

    let full = inspector.get_json("/api/history");
    let observations = entries_of_type(&full, "review_observation_recorded").len();
    assert!(observations >= 1, "store should have observations");

    let filtered = inspector.get_json("/api/history?type=review_observation_recorded");
    // The page is observations only, and matchCount is the filtered (post-type) size.
    assert!(
        filtered["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["eventType"] == "review_observation_recorded")
    );
    assert_eq!(filtered["matchCount"], observations);
    // Facets still report ALL types — they exclude the `type` page filter (INV-3).
    assert_eq!(
        filtered["facets"]["review_observation_recorded"],
        observations
    );
    assert!(
        filtered["facets"]["review_assessment_recorded"]
            .as_u64()
            .unwrap_or(0)
            >= 1,
        "facets count assessments even under a type=observation page filter"
    );
}

#[test]
fn api_history_q_full_text_search_narrows_the_page() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());

    let full = inspector.get_json("/api/history");
    let total = full["matchCount"].as_u64().unwrap();

    // A `q` term present on some but not all entries narrows the page and the count.
    let filtered = inspector.get_json("/api/history?q=type%3Aobservation");
    let filtered_count = filtered["matchCount"].as_u64().unwrap();
    assert!(filtered_count <= total);
    assert!(
        filtered["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["eventType"] == "review_observation_recorded")
    );
}

#[test]
fn api_history_offset_windows_the_filtered_set() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());

    let total = inspector.get_json("/api/history")["matchCount"]
        .as_u64()
        .unwrap();
    let page = inspector.get_json("/api/history?offset=1&limit=2");
    assert_eq!(page["offset"], 1);
    assert!(page["entries"].as_array().unwrap().len() <= 2);
    assert_eq!(page["matchCount"], total);
}

#[test]
fn api_history_at_locates_the_page_and_sets_match_index() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());

    let full = inspector.get_json("/api/history");
    let target = full["entries"][0]["eventId"].as_str().unwrap().to_owned();
    let located = inspector.get_json(&format!("/api/history?limit=2&at={}", urlencode(&target)));
    assert!(located["matchIndex"].is_u64());
    assert!(
        located["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["eventId"] == target.as_str())
    );
}

#[test]
fn api_history_unparamd_shape_is_unchanged_plus_additive_fields() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());

    let full = inspector.get_json("/api/history");
    let entries = full["entries"].as_array().unwrap().len();
    // Unchanged: schema + historyCount = entries.
    assert_eq!(full["schema"], "pointbreak.inspect-history");
    assert_eq!(full["historyCount"], entries);
    // Positional paging surface: facets/matchCount/offset always present, matchIndex
    // only for at=, and no opaque cursor (dropped in favor of offset/at).
    assert!(full["facets"].is_object());
    assert_eq!(full["offset"], 0);
    assert_eq!(full["matchCount"], entries);
    assert!(full.get("matchIndex").is_none() || full["matchIndex"].is_null());
    assert!(full.as_object().unwrap().get("nextCursor").is_none());
}

#[test]
fn api_history_rejects_malformed_offset_and_order() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());

    assert!(
        inspector
            .get_error("/api/history?offset=abc")
            .0
            .contains("400")
    );
    assert!(
        inspector
            .get_error("/api/history?order=sideways")
            .0
            .contains("400")
    );
}

#[test]
fn api_history_reflects_a_new_event_after_an_append() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());

    let before = inspector.get_json("/api/history")["matchCount"]
        .as_u64()
        .unwrap();

    // Append a new event: a second capture over a changed worktree records a new
    // work_object_proposed event, bumping event_log_head_marker so the projection
    // cache invalidates (INV-5).
    store.repo.write(
        "src/lib.rs",
        "pub fn value() -> u32 {\n    99\n}\n\npub fn other() -> u32 {\n    7\n}\n",
    );
    capture(store.repo.path());

    let after = inspector.get_json("/api/history")["matchCount"]
        .as_u64()
        .unwrap();
    assert!(
        after > before,
        "the cache invalidates on a new event (marker changed): {before} -> {after}"
    );
}

/// A swept note body must not kill the inspector: the revision detail hydrates
/// with `include_body` hardcoded, and the history base cache hydrates every
/// body — both previously hard-errored on a removed-and-swept blob.
#[test]
fn inspector_serves_revision_and_history_over_a_swept_body() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    let revision_id = capture(repo.path());
    let arg = repo.path().to_str().unwrap();

    let body = "x".repeat(5000);
    let observation = shore_env(
        [
            "observation",
            "add",
            "--repo",
            arg,
            "--track",
            "agent:codex",
            "--title",
            "a large observation",
            "--body",
            &body,
        ],
        &[],
    );
    assert!(
        observation.status.success(),
        "observation add stderr:\n{}",
        String::from_utf8_lossy(&observation.stderr)
    );
    let removed = shore_env(
        ["store", "remove", "--repo", arg, "--revision", &revision_id],
        &[],
    );
    assert!(removed.status.success());
    let compacted = shore_env(["store", "compact", "--repo", arg, "--yes"], &[]);
    assert!(compacted.status.success());

    let inspector = Inspector::spawn(repo.path());

    let revision = inspector.get_json(&format!("/api/revisions/{}", urlencode(&revision_id)));
    let entry = &revision["observations"][0];
    assert!(entry.get("body").is_none());
    assert_eq!(entry["bodyContentState"], "physically_removed");

    let history = inspector.get_json("/api/history");
    assert_eq!(history["schema"], "pointbreak.inspect-history");
    assert!(
        !history["entries"].as_array().unwrap().is_empty(),
        "the always-hydrating history base cache must survive a swept body"
    );
}
