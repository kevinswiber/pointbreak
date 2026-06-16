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

use support::inspect::{Inspector, representative_store, urlencode};

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
    assert_eq!(passed["summary"]["target"]["kind"], "review_unit");
    assert_eq!(
        passed["summary"]["target"]["reviewUnitId"],
        store.review_unit_id.as_str()
    );
    // Top-level joins the UI relies on (timeline track filter, lineage join key).
    assert_eq!(passed["reviewUnitId"], store.review_unit_id.as_str());
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
