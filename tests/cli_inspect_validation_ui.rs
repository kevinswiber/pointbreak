//! JSON + served-asset contract for the validation-evidence inspector UI
//! (issue #130), exercised at the HTTP level per issue #110.
//!
//! This file locks the exact wire shapes the `app.js` validation work (timeline
//! type, detail rows, unit-page section, lineage facts) reads: the
//! `validation_check_recorded` history summary on `/api/history` and the
//! `validationChecks` / `summary.validationCheckCount` / `validation_evidence`
//! rows on `/api/unit`. Validation evidence stays advisory: it is structurally
//! separate from `currentAssessment` and carries no merge/gate/acceptance keys.

mod support;

use support::git_repo::GitRepo;
use support::inspect::{Inspector, capture_supersession_round, representative_store, urlencode};
use support::shore;

/// Spawn the inspector against a representative store and return the served
/// `/app.js` bytes. `app.js` has no JS execution harness (issue #130), so the
/// UI-wiring guard is a string-level contract over the served asset.
fn spawn_and_get_app_js() -> String {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());
    inspector.get_text("/app.js")
}

/// The substring of `app.js` between two markers, for scoping an assertion to
/// one function or block. Panics if either marker is absent.
fn slice_between<'a>(haystack: &'a str, start: &str, end: &str) -> &'a str {
    let from = haystack
        .find(start)
        .unwrap_or_else(|| panic!("missing {start}"));
    let rest = &haystack[from..];
    let to = rest
        .find(end)
        .unwrap_or_else(|| panic!("missing {end} after {start}"));
    &rest[..to]
}

#[test]
fn served_app_js_registers_validation_timeline_type() {
    let app_js = spawn_and_get_app_js();

    // TYPES registration: the event id with a human label and a distinct,
    // non-fallback color (gray #9aa7b5 is the unknown-type fallback).
    let types_block = slice_between(&app_js, "const TYPES = [", "const TYPE_MAP");
    assert!(
        types_block.contains(r#"id: "validation_check_recorded""#),
        "TYPES must register validation_check_recorded"
    );
    assert!(
        types_block.contains(r#"label: "validation""#),
        "validation type needs a human label"
    );
    assert!(
        types_block.contains("#e88fb0"),
        "validation type needs a distinct non-fallback color"
    );

    // The timeline title path reads the history summary's checkName.
    let entry_title = slice_between(&app_js, "function entryTitle(e)", "function entryTags");
    assert!(
        entry_title.contains("checkName"),
        "entryTitle must read the validation checkName"
    );
}

#[test]
fn served_app_js_includes_validation_in_lineage_facts() {
    let app_js = spawn_and_get_app_js();

    let line = app_js
        .lines()
        .find(|l| l.contains("LINEAGE_FACT_TYPES"))
        .expect("LINEAGE_FACT_TYPES line");
    assert!(
        line.contains("validation_check_recorded"),
        "validation must be a lineage fact type"
    );

    // renderLineageFact derives a validation kind, title, and status tag.
    let render = slice_between(
        &app_js,
        "function renderLineageFact(e, stale)",
        "function renderLineagePage",
    );
    assert!(
        render.contains("validation"),
        "renderLineageFact needs a validation arm"
    );
    assert!(
        render.contains("checkName"),
        "validation card title reads checkName"
    );
    assert!(
        render.contains("s.status"),
        "validation card tags carry the status"
    );
}

#[test]
fn supersession_thread_join_keys_exist_for_validation_entries() {
    // A two-revision supersession thread with a validation check recorded on the
    // FIRST (now superseded) revision. This pins the data contract behind the
    // client-side fact join: the validation entry's subject revision id joins the
    // superseded revision in the thread, while the second revision is the head.
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");

    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    let first = capture_supersession_round(repo.path(), None);

    // Record the validation against the first revision while it is still the sole
    // head, so the fact stays anchored to `first` after it is superseded. (Passing
    // `--revision <superseded>` later would resolve forward to the current head.)
    let added = shore([
        "review",
        "validation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--revision",
        &first,
        "--track",
        "agent:codex",
        "--check-name",
        "cargo test",
        "--status",
        "passed",
    ]);
    assert!(
        added.status.success(),
        "validation add stderr:\n{}",
        String::from_utf8_lossy(&added.stderr)
    );

    let second = capture_supersession_round(repo.path(), Some(&first));

    let inspector = Inspector::spawn(repo.path());

    // Join side A: the supersession thread carries heads + superseded revisions.
    let objects = inspector.get_json("/api/objects");
    let thread = &objects["threads"][0];
    assert_eq!(thread["competing"], false);
    assert_eq!(thread["heads"].as_array().unwrap().len(), 1);
    assert_eq!(thread["heads"][0], second.as_str());
    assert_eq!(thread["superseded"][0], first.as_str());

    // Join side B: the validation history entry's subject names the first revision.
    let history = inspector.get_json("/api/history");
    let validation = history["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["eventType"] == "validation_check_recorded")
        .expect("validation entry present");
    assert_eq!(validation["subject"]["revisionId"], first.as_str());
}

#[test]
fn served_app_js_renders_unit_page_validation_section() {
    let app_js = spawn_and_get_app_js();

    // The unit page consumes the document array (projection reuse, not raw
    // event parsing) via a dedicated card renderer, and shows the count stat.
    assert!(
        app_js.contains("d.validationChecks"),
        "unit page must read d.validationChecks"
    );
    assert!(
        app_js.contains("renderValidationCheckCard"),
        "unit page must render validation cards"
    );
    assert!(
        app_js.contains("validationCheckCount"),
        "summary stat must read validationCheckCount"
    );
    // Advisory framing: a context-only caption, no verdict-style aggregate.
    assert!(
        app_js.contains("context only"),
        "validation section needs an advisory caption"
    );
}

#[test]
fn served_app_css_styles_validation_facts() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());
    let app_css = inspector.get_text("/app.css");

    assert!(app_css.contains(".anno-kind-validation"));
    for status in ["passed", "failed", "errored", "skipped"] {
        assert!(
            app_css.contains(&format!(".fact-status.{status}")),
            "missing .fact-status.{status}"
        );
    }
    assert!(app_css.contains(".validation-note"));
}

#[test]
fn served_app_js_poller_is_diagnostic_aware() {
    let app_js = spawn_and_get_app_js();

    // The freshness poller reloads on a diagnosticCount change, not only on an
    // eventSetHash change (#142). The JSON half — /api/freshness carries
    // diagnosticCount — is locked in cli_inspect_endpoints.rs.
    let poll = slice_between(
        &app_js,
        "async function pollFreshness()",
        "function renderAll",
    );
    assert!(
        poll.contains("diagnosticCount"),
        "pollFreshness must consider diagnosticCount"
    );

    // The last-seen diagnostic count is seeded in load(), like lastHash.
    let load = slice_between(
        &app_js,
        "async function load()",
        "async function pollFreshness",
    );
    assert!(
        load.contains("lastDiagnosticCount"),
        "load() must seed state.lastDiagnosticCount"
    );
}

#[test]
fn served_app_js_handles_validation_in_detail_view() {
    let app_js = spawn_and_get_app_js();

    // renderDetail surfaces the validation fields as labeled kv rows.
    let detail = slice_between(
        &app_js,
        "function renderDetail()",
        "function snapshotIdForUnit",
    );
    assert!(detail.contains("validation_check_recorded"));
    assert!(detail.contains("s.checkName"));
    assert!(detail.contains("s.trigger"));
    assert!(detail.contains("s.exitCode"));
    assert!(detail.contains("validationCheckId"));

    // validation:sha256:… ids render as a non-clickable chip (resolveRef has no
    // validation case, so they must never be wired as navigable).
    assert!(
        app_js.contains(r#"kind: "validation", clickable: false"#),
        "refInfo must classify validation ids as non-clickable"
    );
    let ref_re = slice_between(&app_js, "const REF_RE =", ";");
    assert!(
        ref_re.contains("validation"),
        "REF_RE must include the validation prefix so the id renders as a chip"
    );
}

#[test]
fn api_history_carries_typed_validation_summaries() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());
    let history = inspector.get_json("/api/history");

    let entries: Vec<_> = history["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["eventType"] == "validation_check_recorded")
        .collect();
    assert_eq!(entries.len(), 2);

    let passed = entries
        .iter()
        .find(|e| e["summary"]["checkName"] == "cargo test")
        .expect("passed validation entry");
    assert_eq!(passed["summary"]["kind"], "validation_check_recorded");
    assert_eq!(passed["summary"]["status"], "passed");
    assert_eq!(passed["summary"]["trigger"], "manual");
    assert!(
        passed["summary"]["validationCheckId"]
            .as_str()
            .unwrap()
            .starts_with("validation:sha256:")
    );
    assert_eq!(passed["summary"]["target"]["kind"], "revision");
    assert_eq!(
        passed["summary"]["target"]["revisionId"],
        store.review_unit_id.as_str()
    );
    // Top-level joins the UI relies on (timeline track filter, lineage join key).
    assert_eq!(
        passed["subject"]["revisionId"],
        store.review_unit_id.as_str()
    );
    assert_eq!(passed["trackId"], "agent:codex");

    let failed = entries
        .iter()
        .find(|e| e["summary"]["checkName"] == "cargo clippy")
        .expect("failed validation entry");
    assert_eq!(failed["summary"]["status"], "failed");
    assert_eq!(failed["summary"]["exitCode"], 1);
    assert_eq!(failed["summary"]["command"], "cargo clippy -- -D warnings");
    assert_eq!(failed["trackId"], "human:kevin");
}

#[test]
fn api_unit_serves_validation_checks_and_count() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());
    let unit = inspector.get_json(&format!(
        "/api/unit?id={}",
        urlencode(&store.review_unit_id)
    ));

    assert_eq!(unit["schema"], "shore.review-unit");
    assert_eq!(unit["summary"]["validationCheckCount"], 2);

    let checks = unit["validationChecks"].as_array().unwrap();
    assert_eq!(checks.len(), 2);
    for check in checks {
        for field in [
            "id",
            "eventId",
            "trackId",
            "checkName",
            "status",
            "trigger",
            "createdAt",
        ] {
            assert!(check[field].is_string(), "missing {field}: {check}");
        }
        // Writer is the post-0061 envelope: producer (not a `tool` key).
        assert!(check["writer"]["actorId"].is_string());
        assert!(check["writer"]["producer"]["name"].is_string());
        assert!(
            check["writer"].get("tool").is_none(),
            "writer must use producer, not the pre-0061 tool key"
        );
        // No artifact path field ever enters the contract.
        assert!(check.get("summaryArtifactPath").is_none());
    }

    let failed = checks
        .iter()
        .find(|c| c["checkName"] == "cargo clippy")
        .expect("failed check");
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["exitCode"], 1);
    assert_eq!(failed["command"], "cargo clippy -- -D warnings");
}

#[test]
fn api_unit_projects_validation_evidence_rows() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());
    let unit = inspector.get_json(&format!(
        "/api/unit?id={}",
        urlencode(&store.review_unit_id)
    ));

    let check_ids: Vec<&str> = unit["validationChecks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();

    let validation_rows: Vec<_> = unit["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["kind"] == "validation_evidence")
        .collect();
    assert!(
        !validation_rows.is_empty(),
        "expected validation_evidence rows"
    );

    for row in &validation_rows {
        assert_eq!(row["projectionPhase"], "narrative");
        let related = row["relatedValidationCheckIds"].as_array().unwrap();
        assert!(!related.is_empty());
        for id in related {
            assert!(
                check_ids.contains(&id.as_str().unwrap()),
                "row references unknown validation check {id}"
            );
        }
    }
}

#[test]
fn api_unit_resolves_current_assessment_and_fact_arrays() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());
    let unit = inspector.get_json(&format!(
        "/api/unit?id={}",
        urlencode(&store.review_unit_id)
    ));

    // The superseding assessment resolves the current assessment.
    let ca = &unit["currentAssessment"];
    assert_eq!(ca["status"], "resolved");
    assert_eq!(ca["assessment"], "accepted");
    let current_id = ca["assessmentId"].as_str().unwrap();
    let assessments = unit["assessments"].as_array().unwrap();
    assert_eq!(assessments.len(), 2);
    let current = assessments
        .iter()
        .find(|a| a["id"] == current_id)
        .expect("current assessment present in array");
    assert_eq!(current["status"], "current");
    assert_eq!(current["assessment"], "accepted");

    // The range observation's target is preserved on the composite.
    let observations = unit["observations"].as_array().unwrap();
    assert_eq!(observations.len(), 1);
    let target = &observations[0]["target"];
    // The observation was recorded against a file range; the composite keeps it.
    assert_eq!(target["filePath"], "src/lib.rs");
    assert_eq!(target["startLine"], 2);
    assert_eq!(target["endLine"], 2);

    assert_eq!(unit["inputRequests"].as_array().unwrap().len(), 1);
}

#[test]
fn validation_is_structurally_separate_from_assessment_authority() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());
    let unit = inspector.get_json(&format!(
        "/api/unit?id={}",
        urlencode(&store.review_unit_id)
    ));

    // currentAssessment references an assessment id, never a validation id, and
    // carries no validation fields.
    let ca = &unit["currentAssessment"];
    assert!(ca["assessmentId"].as_str().unwrap().starts_with("assess:"));
    let ca_text = ca.to_string();
    assert!(!ca_text.contains("validation"));
    assert!(!ca_text.contains("validationCheck"));

    // validationChecks carry no merge/gate/acceptance authority keys.
    for check in unit["validationChecks"].as_array().unwrap() {
        let text = check.to_string();
        for forbidden in ["assessment", "gate", "merge", "accept"] {
            assert!(
                !text.contains(forbidden),
                "validation check must not carry `{forbidden}` authority: {check}"
            );
        }
    }
}
