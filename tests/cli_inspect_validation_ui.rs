mod support;

use support::git_repo::GitRepo;
use support::inspect::{Inspector, capture_supersession_round, representative_store, urlencode};
use support::shore;

/// The substring of a served asset between two markers, for scoping an assertion to
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
fn served_tokens_css_pins_the_validation_event_hue() {
    // The validation event hue is single-sourced as an --evt-* token in
    // tokens.css (var(--evt-note) is the unknown-type fallback), so the distinct
    // validation hue is pinned there.
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());
    let tokens_css = inspector.get_text("/tokens.css");
    assert!(
        tokens_css.contains("--evt-validation: #e88fb0"),
        "tokens.css must pin the distinct validation hue"
    );
}

#[test]
fn served_tokens_css_light_theme_aliases_event_labels_to_status() {
    let store = representative_store();
    let inspector = Inspector::spawn(store.repo.path());
    let tokens_css = inspector.get_text("/tokens.css");

    // The :root --evt-* hues are dark-tuned and fall under WCAG AA as label text
    // on the pale light surfaces, so the light theme aliases each colored event
    // type to its AA-tuned status twin — keeping one color per concept across the
    // timeline and the annotation-kind labels.
    let light = slice_between(&tokens_css, "[data-theme=\"light\"]", "}");
    for alias in [
        "--evt-capture: var(--accent)",
        "--evt-observation: var(--success)",
        "--evt-assessment: var(--assess)",
        "--evt-request: var(--warning)",
        "--evt-response: var(--teal)",
        "--evt-validation: var(--validation)",
    ] {
        assert!(
            light.contains(alias),
            "light theme must alias the event label to its status twin: {alias}"
        );
    }
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
    let objects = inspector.get_json("/api/threads");
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
        store.revision_id.as_str()
    );
    // Top-level joins the UI relies on (timeline track filter, lineage join key).
    assert_eq!(passed["subject"]["revisionId"], store.revision_id.as_str());
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
    let unit = inspector.get_json(&format!("/api/revisions/{}", urlencode(&store.revision_id)));

    assert_eq!(unit["schema"], "shore.review-revision");
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
    let unit = inspector.get_json(&format!("/api/revisions/{}", urlencode(&store.revision_id)));

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
    let unit = inspector.get_json(&format!("/api/revisions/{}", urlencode(&store.revision_id)));

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
    let unit = inspector.get_json(&format!("/api/revisions/{}", urlencode(&store.revision_id)));

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
