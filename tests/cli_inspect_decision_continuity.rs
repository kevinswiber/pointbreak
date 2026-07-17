//! Socket-level contracts for Inspector-private association and landing detail.
//! The shared revision document stays unchanged; `/api/revisions/{id}` joins
//! repository liveness at read time and presents the existing commit-range view.

mod support;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::inspect::{Inspector, urlencode};
use support::pointbreak;

fn run_json(args: &[&str]) -> Value {
    let output = pointbreak(args.iter().copied());
    assert!(
        output.status.success(),
        "pointbreak {args:?} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("command returns JSON")
}

fn capture(repo: &GitRepo, extra: &[&str]) -> String {
    let repo_path = repo.path().to_str().unwrap();
    let mut args = vec!["capture", "--repo", repo_path];
    args.extend_from_slice(extra);
    run_json(&args)["revision"]["id"]
        .as_str()
        .expect("capture returns revision id")
        .to_owned()
}

fn detail(repo: &GitRepo, revision_id: &str) -> Value {
    Inspector::spawn(repo.path()).get_json(&format!("/api/revisions/{}", urlencode(revision_id)))
}

fn record_commit(repo: &GitRepo, commit: &str) -> Value {
    run_json(&[
        "association",
        "record",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:test",
        "--commit",
        commit,
    ])
}

fn withdraw(repo: &GitRepo, association_id: &str) {
    run_json(&[
        "association",
        "withdraw",
        association_id,
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:test",
    ]);
}

fn record_validation(repo: &GitRepo, revision_id: &str, status: &str, completed_at: &str) -> Value {
    run_json(&[
        "validation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--revision",
        revision_id,
        "--track",
        "agent:test",
        "--check-name",
        "cargo test",
        "--status",
        status,
        "--completed-at",
        completed_at,
    ])
}

#[test]
fn recovered_validation_changes_effective_attention_without_rewriting_evidence() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    let revision_id = capture(&repo, &[]);

    let failed = record_validation(&repo, &revision_id, "failed", "2026-07-16T10:00:00Z");
    let passed = record_validation(&repo, &revision_id, "passed", "2026-07-16T10:01:00Z");

    let inspector = Inspector::spawn(repo.path());
    let overview_payload = inspector.get_json("/api/revisions");
    let overview = &overview_payload["entries"][0]["overview"];
    assert_eq!(overview["attention"]["failedValidationCount"], 0);
    assert_eq!(overview["attention"]["erroredValidationCount"], 0);
    assert_eq!(overview["counts"]["validationChecks"], 2);
    assert_eq!(overview["validationContinuity"]["recoveredCount"], 1);

    let detail = inspector.get_json(&format!("/api/revisions/{}", urlencode(&revision_id)));
    assert_eq!(detail["schema"], "pointbreak.review-revision");
    assert_eq!(detail["version"], 2);
    assert_eq!(detail["summary"]["validationCheckCount"], 2);
    assert_eq!(detail["validationChecks"].as_array().unwrap().len(), 2);
    assert_eq!(
        detail["validationContinuity"]["summary"]["recoveredCount"],
        1
    );
    assert_eq!(
        detail["validationContinuity"]["checks"][failed["validationCheckId"].as_str().unwrap()],
        "resolved_by_later_pass"
    );
    assert_eq!(
        detail["validationContinuity"]["checks"][passed["validationCheckId"].as_str().unwrap()],
        "current"
    );
    assert_eq!(detail["currentAssessment"]["status"], "unassessed");
}

#[test]
fn detail_distinguishes_floating_revision_from_anchored_capture_target() {
    let floating = GitRepo::new();
    floating.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    floating.commit_all("base");
    floating.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    let floating_id = capture(&floating, &[]);
    let floating_detail = detail(&floating, &floating_id);
    assert_eq!(floating_detail["commitRange"]["anchored"], false);
    assert_eq!(
        floating_detail["commitRange"]["liveness"]["perCommit"],
        serde_json::json!([])
    );

    let anchored = GitRepo::new();
    anchored.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    anchored.commit_all("base");
    anchored.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    anchored.commit_all("range target");
    let anchored_id = capture(&anchored, &["--base", "HEAD~1"]);
    let anchored_detail = detail(&anchored, &anchored_id);
    assert_eq!(anchored_detail["commitRange"]["anchored"], true);
    assert_eq!(
        anchored_detail["commitRange"]["currentCommits"][0]["source"],
        "capture_target"
    );
    assert_eq!(
        anchored_detail["commitRange"]["liveness"]["headline"]["condition"],
        "merged"
    );
}

#[test]
fn landing_association_moves_from_live_feature_branch_to_merged_default_branch() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.git(["checkout", "-b", "feature/landing"]);
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    let revision_id = capture(&repo, &[]);
    repo.commit_all("land reviewed work");
    record_commit(&repo, "HEAD");

    let live = detail(&repo, &revision_id);
    assert_eq!(
        live["commitRange"]["currentCommits"][0]["source"],
        "association"
    );
    assert_eq!(
        live["commitRange"]["liveness"]["headline"]["condition"],
        "live"
    );
    assert_eq!(
        live["commitRange"]["liveness"]["perCommit"][0]["liveBranch"],
        "feature/landing"
    );

    repo.git(["checkout", "main"]);
    repo.git(["merge", "--ff-only", "feature/landing"]);
    let merged = detail(&repo, &revision_id);
    assert_eq!(
        merged["commitRange"]["liveness"]["headline"]["condition"],
        "merged"
    );
    assert_eq!(
        merged["revision"]["targetDisplay"]["head"]["liveBranch"],
        "main"
    );
}

#[test]
fn detail_retains_successive_and_withdrawn_commit_and_ref_edges() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    let revision_id = capture(&repo, &[]);

    repo.commit_all("landing one");
    let first = record_commit(&repo, "HEAD");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    repo.commit_all("landing two");
    record_commit(&repo, "HEAD");
    repo.git(["branch", "release/reviewed"]);
    let second_oid = repo.git(["rev-parse", "HEAD"]).stdout.trim().to_owned();
    run_json(&[
        "association",
        "record",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:test",
        "--ref",
        "release/reviewed",
        "--head",
        &second_oid,
    ]);

    let successive = detail(&repo, &revision_id);
    assert_eq!(
        successive["commitRange"]["currentCommits"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        successive["commitRange"]["currentRefs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    withdraw(&repo, first["commitAssociationId"].as_str().unwrap());
    let main_ref_id = successive["commitRange"]["currentRefs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["refName"] == "refs/heads/main")
        .unwrap()["refAssociationId"]
        .as_str()
        .unwrap()
        .to_owned();
    withdraw(&repo, &main_ref_id);

    let withdrawn = detail(&repo, &revision_id);
    assert_eq!(
        withdrawn["commitRange"]["withdrawnCommits"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        withdrawn["commitRange"]["withdrawnRefs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn missing_commit_object_is_missing_but_git_failure_stays_unprojected() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    let revision_id = capture(&repo, &[]);
    repo.commit_all("landing");
    let landing_oid = repo.git(["rev-parse", "HEAD"]).stdout.trim().to_owned();
    record_commit(&repo, &landing_oid);
    repo.git(["reset", "--hard", "HEAD~1"]);
    let git_dir = repo
        .git(["rev-parse", "--git-dir"])
        .stdout
        .trim()
        .to_owned();
    std::fs::remove_file(
        repo.path()
            .join(git_dir)
            .join("objects")
            .join(&landing_oid[..2])
            .join(&landing_oid[2..]),
    )
    .expect("remove loose landing object");

    let response = detail(&repo, &revision_id);
    assert_eq!(
        response["commitRange"]["liveness"]["perCommit"][0]["condition"],
        "missing"
    );
    assert!(
        response["commitRange"]["liveness"]["perCommit"][0]
            .get("reason")
            .is_none(),
        "missing is a first-class condition, not an orphan reason"
    );
}

#[test]
fn competing_live_landing_claims_surface_diagnostic_and_withhold_headline() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.git(["checkout", "-b", "landing/a"]);
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.commit_all("landing a");
    let landing_a = repo.git(["rev-parse", "HEAD"]).stdout.trim().to_owned();
    repo.git(["checkout", "main"]);
    repo.git(["checkout", "-b", "landing/b"]);
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    repo.commit_all("landing b");
    let landing_b = repo.git(["rev-parse", "HEAD"]).stdout.trim().to_owned();
    repo.git(["checkout", "main"]);
    repo.write("src/lib.rs", "pub fn value() -> u32 { 4 }\n");
    let revision_id = capture(&repo, &[]);
    record_commit(&repo, &landing_a);
    record_commit(&repo, &landing_b);

    let response = detail(&repo, &revision_id);
    assert!(
        response["commitRange"]["liveness"]
            .get("headline")
            .is_none()
    );
    assert!(
        response["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| { item["code"] == "divergent_commit_association" })
    );
}
