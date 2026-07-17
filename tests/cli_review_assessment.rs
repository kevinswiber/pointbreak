mod support;

use std::ffi::OsStr;
use std::io::Write;
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use support::git_repo::GitRepo;
use support::pointbreak;

#[test]
fn assessment_add_and_show_run_at_the_top_level() {
    let repo = support::dump_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let add = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--assessment",
        "accepted",
        "--summary",
        "looks good",
    ]);
    assert!(
        add.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&add.stderr)
    );
    let added = parse_json(&add.stdout);
    assert_eq!(added["schema"], "pointbreak.review-assessment-add"); // INV-1

    let show = pointbreak([
        "assessment",
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);
    assert!(
        show.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&show.stderr)
    );
    let shown = parse_json(&show.stdout);
    assert_eq!(shown["schema"], "pointbreak.review-assessment-show");
}

#[test]
fn assessment_exact_revision_targets_a_superseded_revision_for_add_and_show() {
    let (repo, first_id, second_id) = support::superseded_dump_repo();
    let repo_arg = repo.path().to_str().unwrap();

    let legacy = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo_arg,
        "--revision",
        &first_id,
        "--track",
        "human:legacy",
        "--assessment",
        "accepted",
    ]);
    assert!(
        legacy.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&legacy.stderr)
    );
    assert_eq!(parse_json(&legacy.stdout)["revisionId"], second_id);

    let exact = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo_arg,
        "--exact-revision",
        &first_id,
        "--track",
        "human:exact",
        "--assessment",
        "needs-changes",
    ]);
    assert!(
        exact.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&exact.stderr)
    );
    assert_eq!(parse_json(&exact.stdout)["revisionId"], first_id);

    let legacy_show = pointbreak([
        "assessment",
        "show",
        "--repo",
        repo_arg,
        "--revision",
        &first_id,
        "--track",
        "human:legacy",
    ]);
    assert!(legacy_show.status.success());
    assert_eq!(parse_json(&legacy_show.stdout)["revisionId"], second_id);

    let exact_show = pointbreak([
        "assessment",
        "show",
        "--repo",
        repo_arg,
        "--exact-revision",
        &first_id,
        "--track",
        "human:exact",
    ]);
    assert!(exact_show.status.success());
    assert_eq!(parse_json(&exact_show.stdout)["revisionId"], first_id);
}

#[test]
fn assessment_exact_revision_rejects_conflicting_or_unknown_selectors_before_write() {
    let repo = support::dump_repo();
    let repo_arg = repo.path().to_str().unwrap();
    let capture = parse_json(&pointbreak(["capture", "--repo", repo_arg]).stdout);
    let revision_id = capture["revision"]["id"].as_str().unwrap();

    let conflicting = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo_arg,
        "--revision",
        revision_id,
        "--exact-revision",
        revision_id,
        "--track",
        "human:kevin",
        "--assessment",
        "accepted",
    ]);
    assert!(!conflicting.status.success());
    assert!(String::from_utf8_lossy(&conflicting.stderr).contains("cannot be used with"));

    let unknown = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo_arg,
        "--exact-revision",
        "rev:sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "--track",
        "human:kevin",
        "--assessment",
        "accepted",
    ]);
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown revision"));
}

#[test]
fn assessment_exact_revision_validates_relationships_against_the_named_revision() {
    let (repo, first_id, _) = support::superseded_dump_repo();
    let repo_arg = repo.path().to_str().unwrap();
    let observation = parse_json(
        &pointbreak([
            "observation",
            "add",
            "--repo",
            repo_arg,
            "--revision",
            &first_id,
            "--track",
            "human:kevin",
            "--title",
            "belongs to the successor",
        ])
        .stdout,
    );
    let before = parse_json(&pointbreak(["store", "status", "--repo", repo_arg]).stdout)
        ["inventory"]["eventCount"]
        .as_u64()
        .unwrap();

    let rejected = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo_arg,
        "--exact-revision",
        &first_id,
        "--track",
        "human:kevin",
        "--assessment",
        "accepted",
        "--related-observation",
        observation["observationId"].as_str().unwrap(),
    ]);

    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("unknown observation"));
    let after = parse_json(&pointbreak(["store", "status", "--repo", repo_arg]).stdout)["inventory"]
        ["eventCount"]
        .as_u64()
        .unwrap();
    assert_eq!(after, before, "relationship rejection must not append");
}

#[test]
fn assessment_add_observation_target_resolves_a_bare_fragment() {
    let repo = support::dump_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    let observation = parse_json(
        &pointbreak([
            "observation",
            "add",
            "--repo",
            repo.path().to_str().unwrap(),
            "--track",
            "human:kevin",
            "--title",
            "note",
            "--body",
            "body text",
        ])
        .stdout,
    );
    let observation_id = observation["observationId"].as_str().unwrap().to_owned();
    // observation_id = "obs:sha256:<hex>".
    let fragment = &observation_id["obs:sha256:".len()..][..8];

    let add = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--assessment",
        "accepted",
        "--summary",
        "targeting the note",
        "--observation",
        fragment,
    ]);
    assert!(
        add.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&add.stderr)
    );
    let added = parse_json(&add.stdout);
    assert_eq!(added["target"]["kind"], "observation");
    assert_eq!(
        added["target"]["observationId"], observation_id,
        "the target must echo the resolved FULL observation id, not the bare fragment"
    );
}

#[test]
fn shore_review_assessment_add_emits_assessment_id_and_event() {
    let repo = support::dump_repo();
    let repo_arg = repo.path().to_str().unwrap();
    let capture = pointbreak(["capture", "--repo", repo_arg]);
    assert!(
        capture.status.success(),
        "capture failed: {}",
        String::from_utf8_lossy(&capture.stderr)
    );

    let add = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo_arg,
        "--track",
        "human:kevin",
        "--assessment",
        "accepted",
        "--summary",
        "ship it",
    ]);
    assert!(
        add.status.success(),
        "assessment add failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );
    let output = parse_json(&add.stdout);

    assert_eq!(output["schema"], "pointbreak.review-assessment-add");
    assert!(
        output["assessmentId"]
            .as_str()
            .unwrap()
            .starts_with("assess:sha256:"),
        "got {:?}",
        output["assessmentId"]
    );
    assert_eq!(output["assessment"], "accepted");
    assert_eq!(
        output["eventsCreatedByType"]["review_assessment_recorded"],
        1
    );
}

#[test]
fn shore_review_assessment_show_resolves_to_single_assessment() {
    let repo = support::dump_repo();
    let repo_arg = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", repo_arg]);
    let assessment = add_assessment(repo_arg, "human:kevin", "accepted", "ship it");

    let show = pointbreak(["assessment", "show", "--repo", repo_arg]);
    assert!(
        show.status.success(),
        "assessment show failed: {}",
        String::from_utf8_lossy(&show.stderr)
    );
    let output = parse_json(&show.stdout);

    assert_eq!(output["schema"], "pointbreak.review-assessment-show");
    assert_eq!(output["current"]["status"], "resolved");
    assert_eq!(output["current"]["assessment"], "accepted");
    assert_eq!(
        output["current"]["assessmentId"],
        assessment["assessmentId"]
    );
    assert_eq!(output["assessments"].as_array().unwrap().len(), 1);
}

#[test]
fn shore_review_assessment_show_marks_ambiguous_with_two_writers() {
    let repo = support::dump_repo();
    let repo_arg = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", repo_arg]);
    add_assessment(repo_arg, "human:kevin", "accepted", "ship it");
    add_assessment(repo_arg, "agent:codex", "needs-changes", "fix it");

    let show = pointbreak(["assessment", "show", "--repo", repo_arg]);
    assert!(
        show.status.success(),
        "assessment show failed: {}",
        String::from_utf8_lossy(&show.stderr)
    );
    let output = parse_json(&show.stdout);

    assert_eq!(output["current"]["status"], "ambiguous");
    assert_eq!(output["current"]["candidates"].as_array().unwrap().len(), 2);
    assert_eq!(output["assessments"].as_array().unwrap().len(), 2);
}

#[test]
fn text_assessment_show_renders_current_call_not_json() {
    let repo = support::dump_repo();
    let repo_arg = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", repo_arg]);
    add_assessment(repo_arg, "human:kevin", "accepted", "ship it");

    let show = pointbreak(["assessment", "show", "--repo", repo_arg, "--format", "text"]);
    assert!(
        show.status.success(),
        "assessment show failed: {}",
        String::from_utf8_lossy(&show.stderr)
    );
    let stdout = String::from_utf8_lossy(&show.stdout);

    assert!(stdout.contains("current call"), "stdout:\n{stdout}");
    assert!(stdout.contains("accepted"), "stdout:\n{stdout}");
    assert!(!stdout.contains("\"schema\""), "stdout:\n{stdout}");
    assert!(
        stdout.lines().count() <= 10,
        "digest must be bounded, got {} lines:\n{stdout}",
        stdout.lines().count()
    );
}

#[test]
fn shore_review_assessment_add_rejects_state_change_value() {
    let repo = support::dump_repo();
    let repo_arg = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", repo_arg]);

    let bad = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo_arg,
        "--track",
        "human:kevin",
        "--assessment",
        "deferred",
        "--summary",
        "x",
    ]);

    assert!(!bad.status.success(), "expected deferred to fail");
    assert!(
        String::from_utf8_lossy(&bad.stderr).contains("invalid value"),
        "expected clap invalid value error; got stderr: {}",
        String::from_utf8_lossy(&bad.stderr)
    );
}

#[test]
fn shore_review_assessment_add_records_file_range_target() {
    let repo = support::dump_repo();
    let repo_arg = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", repo_arg]);

    let output = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo_arg,
        "--track",
        "human:kevin",
        "--assessment",
        "needs-changes",
        "--summary",
        "Fix line one",
        "--file",
        "src/lib.rs",
        "--side",
        "new",
        "--start-line",
        "1",
        "--end-line",
        "1",
    ]);
    assert!(
        output.status.success(),
        "assessment add failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);

    assert_eq!(json["target"]["kind"], "range");
    assert_eq!(json["target"]["filePath"], "src/lib.rs");
    assert_eq!(json["target"]["side"], "new");
    assert_eq!(json["target"]["startLine"], 1);
    assert_eq!(json["target"]["endLine"], 1);
}

#[test]
fn shore_review_assessment_add_records_related_facts_and_replacement() {
    let repo = support::dump_repo();
    let repo_arg = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", repo_arg]);
    let observation = add_observation(&repo, "Related observation");
    let input_request = open_input_request(&repo, "Related input request");
    let first = add_assessment(repo_arg, "human:kevin", "needs-changes", "Fix this");

    let second = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo_arg,
        "--track",
        "human:kevin",
        "--assessment",
        "accepted-with-follow-up",
        "--summary",
        "Accept with follow-up",
        "--replaces",
        first["assessmentId"].as_str().unwrap(),
        "--related-observation",
        observation["observationId"].as_str().unwrap(),
        "--related-input-request",
        input_request["inputRequestId"].as_str().unwrap(),
    ]);
    assert!(
        second.status.success(),
        "assessment add failed: {}",
        String::from_utf8_lossy(&second.stderr)
    );

    let show = assessment_show(&repo, &["--all"]);
    assert_eq!(show["assessments"].as_array().unwrap().len(), 2);
    assert!(
        show["assessments"]
            .as_array()
            .unwrap()
            .iter()
            .any(|view| { view["id"] == first["assessmentId"] && view["status"] == "replaced" })
    );
    let current = show["assessments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|view| view["status"] == "current")
        .unwrap();
    assert_eq!(current["assessment"], "accepted_with_follow_up");
    assert_eq!(
        current["relatedObservations"][0],
        observation["observationId"]
    );
    assert_eq!(
        current["relatedInputRequests"][0],
        input_request["inputRequestId"]
    );
}

#[test]
fn shore_review_assessment_add_diagnoses_cross_actor_replacement() {
    let repo = support::dump_repo();
    let repo_arg = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", repo_arg]);

    let first = parse_json(
        &support::pointbreak_env(
            [
                "assessment",
                "add",
                "--repo",
                repo_arg,
                "--track",
                "agent:codex",
                "--assessment",
                "needs-changes",
                "--summary",
                "Fix this",
            ],
            &[("POINTBREAK_ACTOR_ID", "actor:agent:codex")],
        )
        .stdout,
    );
    let second = support::pointbreak_env(
        [
            "assessment",
            "add",
            "--repo",
            repo_arg,
            "--track",
            "human:local",
            "--assessment",
            "accepted",
            "--summary",
            "Resolved after review",
            "--replaces",
            first["assessmentId"].as_str().unwrap(),
        ],
        &[("POINTBREAK_ACTOR_ID", "actor:git-email:human@example.com")],
    );
    assert!(
        second.status.success(),
        "assessment add failed: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second = parse_json(&second.stdout);
    let diagnostic = second["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == "assessment_cross_actor_replacement")
        .unwrap_or_else(|| panic!("expected cross-actor replacement diagnostic: {second}"));
    let message = diagnostic["message"].as_str().unwrap();
    for expected in [
        first["assessmentId"].as_str().unwrap(),
        "actor:agent:codex",
        "actor:git-email:human@example.com",
        "remain in history",
    ] {
        assert!(
            message.contains(expected),
            "diagnostic must contain {expected:?}:\n{message}"
        );
    }

    let show = assessment_show(&repo, &["--all"]);
    let replaced = show["assessments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|assessment| assessment["id"] == first["assessmentId"])
        .unwrap();
    assert_eq!(replaced["status"], "replaced");
}

#[test]
fn shore_review_assessment_add_flags_competing_candidates_without_replaces() {
    let repo = support::dump_repo();
    let repo_arg = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", repo_arg]);

    let first = add_assessment(repo_arg, "human:kevin", "accepted", "ship it");
    assert!(
        first["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| diagnostic["code"] != "assessment_competing_candidates"),
        "first assessment must not flag competing candidates: {first}"
    );

    let second = add_assessment(repo_arg, "agent:codex", "needs-changes", "fix it");
    let diagnostic = second["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == "assessment_competing_candidates")
        .unwrap_or_else(|| panic!("expected competing-candidates diagnostic: {second}"));
    let message = diagnostic["message"].as_str().unwrap();
    assert!(
        message.contains(first["assessmentId"].as_str().unwrap()),
        "message must name the unreplaced candidate:\n{message}"
    );
    assert!(
        message.contains("--replaces"),
        "message must point at --replaces:\n{message}"
    );
}

#[test]
fn shore_review_assessment_add_replacing_every_candidate_stays_quiet() {
    let repo = support::dump_repo();
    let repo_arg = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", repo_arg]);
    let first = add_assessment(repo_arg, "human:kevin", "needs-changes", "fix it");

    let second = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo_arg,
        "--track",
        "human:kevin",
        "--assessment",
        "accepted",
        "--summary",
        "fixed",
        "--replaces",
        first["assessmentId"].as_str().unwrap(),
    ]);
    assert!(
        second.status.success(),
        "assessment add failed: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let output = parse_json(&second.stdout);
    assert!(
        output["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| !matches!(
                diagnostic["code"].as_str(),
                Some("assessment_competing_candidates" | "assessment_cross_actor_replacement")
            )),
        "a same-actor full replacement must stay quiet: {output}"
    );
}

#[test]
fn shore_review_assessment_add_idempotent_rerun_of_replaced_assessment_stays_quiet() {
    let repo = support::dump_repo();
    let repo_arg = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", repo_arg]);
    let first = add_assessment(repo_arg, "human:kevin", "needs-changes", "fix it");

    let second = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo_arg,
        "--track",
        "human:kevin",
        "--assessment",
        "accepted",
        "--summary",
        "fixed",
        "--replaces",
        first["assessmentId"].as_str().unwrap(),
    ]);
    assert!(
        second.status.success(),
        "assessment add failed: {}",
        String::from_utf8_lossy(&second.stderr)
    );

    // A byte-identical rerun of the replaced assessment records no new event
    // and leaves the revision resolved on the replacement, so it must not
    // read as a fresh competitor.
    let rerun = add_assessment(repo_arg, "human:kevin", "needs-changes", "fix it");
    assert_eq!(
        rerun["eventsCreated"], 0,
        "rerun must be idempotent: {rerun}"
    );
    assert_eq!(
        rerun["eventsExisting"], 1,
        "rerun must be idempotent: {rerun}"
    );
    assert!(
        rerun["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| diagnostic["code"] != "assessment_competing_candidates"),
        "an idempotent rerun of a replaced assessment must stay quiet: {rerun}"
    );
}

#[test]
fn shore_review_assessment_add_flags_only_candidates_left_unreplaced() {
    let repo = support::dump_repo();
    let repo_arg = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", repo_arg]);
    let first = add_assessment(repo_arg, "human:kevin", "accepted", "ship it");
    let second = add_assessment(repo_arg, "agent:codex", "needs-changes", "fix it");

    let third = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo_arg,
        "--track",
        "agent:codex",
        "--assessment",
        "accepted",
        "--summary",
        "fixed",
        "--replaces",
        second["assessmentId"].as_str().unwrap(),
    ]);
    assert!(
        third.status.success(),
        "assessment add failed: {}",
        String::from_utf8_lossy(&third.stderr)
    );
    let output = parse_json(&third.stdout);
    let diagnostic = output["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == "assessment_competing_candidates")
        .unwrap_or_else(|| panic!("expected competing-candidates diagnostic: {output}"));
    let message = diagnostic["message"].as_str().unwrap();
    assert!(
        message.contains(first["assessmentId"].as_str().unwrap()),
        "message must name the candidate left standing:\n{message}"
    );
    assert!(
        !message.contains(second["assessmentId"].as_str().unwrap()),
        "message must not name the replaced candidate:\n{message}"
    );
}

#[test]
fn shore_review_assessment_add_targets_input_request_and_emits_related_input_requests() {
    let repo = support::dump_repo();
    let repo_arg = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", repo_arg]);
    let request = open_input_request(&repo, "Needs clarification");
    let request_id = request["inputRequestId"].as_str().unwrap();

    let assessment = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo_arg,
        "--track",
        "human:kevin",
        "--assessment",
        "needs-clarification",
        "--summary",
        "Need a decision here",
        "--input-request",
        request_id,
        "--related-input-request",
        request_id,
    ]);
    assert!(
        assessment.status.success(),
        "assessment add failed: {}",
        String::from_utf8_lossy(&assessment.stderr)
    );
    let assessment = parse_json(&assessment.stdout);

    assert_eq!(assessment["target"]["kind"], "input_request");
    assert_eq!(assessment["target"]["inputRequestId"], request_id);
    assert!(assessment["target"].get("interventionId").is_none());

    let show = assessment_show(&repo, &["--all"]);
    let current = show["assessments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|view| view["status"] == "current")
        .unwrap();
    assert_eq!(current["relatedInputRequests"][0], request_id);
    assert!(current.get("relatedInterventions").is_none());

    let request = pointbreak(["input-request", "show", "--repo", repo_arg, request_id]);
    assert!(
        request.status.success(),
        "request fetch failed: {}",
        String::from_utf8_lossy(&request.stderr)
    );
    let request = parse_json(&request.stdout);
    assert_eq!(request["inputRequest"]["status"], "open");
    assert!(
        request["inputRequest"]["responses"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn shore_review_assessment_add_rejects_old_intervention_flags() {
    let repo = support::dump_repo();
    let repo_arg = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", repo_arg]);
    let request = open_input_request(&repo, "Legacy flag target");
    let request_id = request["inputRequestId"].as_str().unwrap();

    let old_target_flag = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo_arg,
        "--track",
        "human:kevin",
        "--assessment",
        "needs-clarification",
        "--intervention",
        request_id,
    ]);
    let old_related_flag = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo_arg,
        "--track",
        "human:kevin",
        "--assessment",
        "needs-clarification",
        "--related-intervention",
        request_id,
    ]);

    assert!(!old_target_flag.status.success());
    assert!(
        String::from_utf8_lossy(&old_target_flag.stderr).contains("unexpected argument"),
        "expected clap unknown-flag error; got stderr: {}",
        String::from_utf8_lossy(&old_target_flag.stderr)
    );
    assert!(!old_related_flag.status.success());
    assert!(
        String::from_utf8_lossy(&old_related_flag.stderr).contains("unexpected argument"),
        "expected clap unknown-flag error; got stderr: {}",
        String::from_utf8_lossy(&old_related_flag.stderr)
    );
}

#[test]
fn shore_review_assessment_add_reports_unknown_input_request_target() {
    let repo = support::dump_repo();
    let repo_arg = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", repo_arg]);

    let output = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo_arg,
        "--track",
        "human:kevin",
        "--assessment",
        "needs-clarification",
        // A well-formed full id that resolves (passthrough) but is absent from the
        // store, so the library — not the short-id resolver — reports it unknown.
        "--input-request",
        "input-request:sha256:0000000000000000000000000000000000000000000000000000000000000000",
    ]);

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("unknown input request target"),
        "expected input-request error; got stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn shore_review_assessment_add_summary_stdin_reads_from_stdin() {
    let repo = support::dump_repo();
    let repo_arg = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", repo_arg]);

    let output = shore_with_stdin(
        [
            "assessment",
            "add",
            "--repo",
            repo_arg,
            "--track",
            "human:kevin",
            "--assessment",
            "accepted",
            "--summary-stdin",
        ],
        "summary from stdin",
    );
    assert!(
        output.status.success(),
        "assessment add failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let show = assessment_show(&repo, &["--include-summary"]);
    assert_eq!(show["assessments"][0]["summary"], "summary from stdin");
}

#[test]
fn shore_review_assessment_add_targets_prior_assessment() {
    let repo = support::dump_repo();
    let repo_arg = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", repo_arg]);
    let first = add_assessment(repo_arg, "human:kevin", "needs-changes", "Fix this");

    let output = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo_arg,
        "--track",
        "human:kevin",
        "--assessment",
        "accepted",
        "--summary",
        "Earlier assessment is settled",
        "--target-assessment",
        first["assessmentId"].as_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "assessment add failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);

    assert_eq!(json["target"]["kind"], "assessment");
    assert_eq!(json["target"]["assessmentId"], first["assessmentId"]);
}

#[test]
fn shore_review_assessment_add_rejects_invalid_input() {
    let repo = support::dump_repo();
    let repo_arg = repo.path().to_str().unwrap();
    pointbreak(["capture", "--repo", repo_arg]);
    let observation = add_observation(&repo, "Target conflict");

    let missing_track = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo_arg,
        "--assessment",
        "accepted",
    ]);
    let conflicting_target = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo_arg,
        "--track",
        "human:kevin",
        "--assessment",
        "accepted",
        "--observation",
        observation["observationId"].as_str().unwrap(),
        "--file",
        "src/lib.rs",
    ]);
    let side_without_file = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo_arg,
        "--track",
        "human:kevin",
        "--assessment",
        "accepted",
        "--summary",
        "Ship it",
        "--side",
        "old",
    ]);
    let unknown_replacement = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo_arg,
        "--track",
        "human:kevin",
        "--assessment",
        "accepted",
        "--summary",
        "Ship it",
        // Well-formed full id that resolves (passthrough) but is absent, so the
        // library reports the unknown replacement rather than the resolver.
        "--replaces",
        "assess:sha256:0000000000000000000000000000000000000000000000000000000000000000",
    ]);

    assert!(!missing_track.status.success());
    assert!(String::from_utf8_lossy(&missing_track.stderr).contains("--track"));
    assert!(!conflicting_target.status.success());
    assert!(
        String::from_utf8_lossy(&conflicting_target.stderr).contains("target cannot be combined")
    );
    assert!(!side_without_file.status.success());
    assert!(String::from_utf8_lossy(&side_without_file.stderr).contains("side requires file"));
    assert!(!unknown_replacement.status.success());
    assert!(String::from_utf8_lossy(&unknown_replacement.stderr).contains("unknown assessment"));
}

fn add_assessment(repo_arg: &str, track: &str, assessment: &str, summary: &str) -> Value {
    let output = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo_arg,
        "--track",
        track,
        "--assessment",
        assessment,
        "--summary",
        summary,
    ]);
    assert!(
        output.status.success(),
        "assessment add failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    parse_json(&output.stdout)
}

fn assessment_show(repo: &GitRepo, args: &[&str]) -> Value {
    let mut command = vec![
        "assessment",
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
    ];
    command.extend(args);
    let output = pointbreak(command);
    assert!(
        output.status.success(),
        "assessment show failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    parse_json(&output.stdout)
}

fn add_observation(repo: &GitRepo, title: &str) -> Value {
    parse_json(
        &pointbreak([
            "observation",
            "add",
            "--repo",
            repo.path().to_str().unwrap(),
            "--track",
            "agent:codex",
            "--title",
            title,
        ])
        .stdout,
    )
}

fn open_input_request(repo: &GitRepo, title: &str) -> Value {
    parse_json(
        &pointbreak([
            "input-request",
            "open",
            "--repo",
            repo.path().to_str().unwrap(),
            "--track",
            "human:kevin",
            "--title",
            title,
            "--reason",
            "manual-decision-required",
        ])
        .stdout,
    )
}

fn parse_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes)
        .unwrap_or_else(|error| panic!("parse json: {error}\n{}", String::from_utf8_lossy(bytes)))
}

fn shore_with_stdin<I, S>(args: I, stdin: &str) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut child = Command::new(env!("CARGO_BIN_EXE_pointbreak"))
        .args(args)
        .env_remove("POINTBREAK_LOG")
        .env_remove("RUST_LOG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn pointbreak binary");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child
        .wait_with_output()
        .expect("wait for pointbreak binary")
}

#[test]
fn text_assessment_add_receipt_names_the_call() {
    let repo = support::dump_repo();
    pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);

    let output = pointbreak([
        "assessment",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "human:kevin",
        "--assessment",
        "accepted",
        "--summary",
        "looks good",
        "--format",
        "text",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        !stdout.contains("\"schema\""),
        "text lane is not JSON: {stdout}"
    );
    assert!(stdout.contains("accepted"), "the call: {stdout}");
    assert!(stdout.contains("assess:"), "short assessment id: {stdout}");
    assert!(stdout.contains("rev:"), "short revision id: {stdout}");
    assert!(stdout.contains("human:kevin"), "track: {stdout}");
}
