//! HTTP contract for the inspector-private, fork-gated fact-level supersession
//! graphs spliced into `/api/revisions/{id}` (#234). Asserts TOPOLOGY over a real
//! ambiguous-assessment fork and a real superseded-observation chain, the tagged
//! edge `kind`, that a non-forked revision omits the field, and that the shared
//! `shore revision show` document is untouched. Never asserts exact pixels.

mod support;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::inspect::{Inspector, capture, representative_store};
use support::shore;

/// A repo with a base commit and a working-tree change, ready to capture.
fn repo_with_change() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo
}

fn assessment_id(out: &std::process::Output) -> String {
    let json: Value = serde_json::from_slice(&out.stdout).expect("assessment add JSON");
    json["assessmentId"]
        .as_str()
        .expect("assessmentId")
        .to_owned()
}

#[test]
fn ambiguous_assessments_emit_a_tagged_fact_graph() {
    let repo = repo_with_change();
    let arg = repo.path().to_str().unwrap();
    let rev = capture(repo.path());

    // A (needs-changes) replaced by B (accepted); then C (needs-changes) competes
    // with B, neither replacing the other -> current = {B, C} -> Ambiguous.
    let a = assessment_id(&shore([
        "assessment",
        "add",
        "--repo",
        arg,
        "--track",
        "agent:codex",
        "--assessment",
        "needs-changes",
        "--summary",
        "not yet",
    ]));
    let b = assessment_id(&shore([
        "assessment",
        "add",
        "--repo",
        arg,
        "--track",
        "agent:codex",
        "--assessment",
        "accepted",
        "--summary",
        "ship it",
        "--replaces",
        &a,
    ]));
    let _c = shore([
        "assessment",
        "add",
        "--repo",
        arg,
        "--track",
        "human:kevin",
        "--assessment",
        "needs-changes",
        "--summary",
        "hold on",
    ]);

    let doc = Inspector::spawn(repo.path()).get_json(&format!("/api/revisions/{rev}"));

    // Sanity: the contested state the graph visualizes.
    assert_eq!(doc["currentAssessment"]["status"], "ambiguous");

    let laid = &doc["factSupersession"]["assessments"]["laidOut"];
    let nodes = laid["nodes"].as_array().expect("assessments.laidOut.nodes");
    assert_eq!(nodes.len(), 3, "A (replaced) + B, C (competing current)");
    let heads: Vec<&Value> = nodes.iter().filter(|n| n["isHead"] == true).collect();
    let superseded: Vec<&Value> = nodes.iter().filter(|n| n["isSuperseded"] == true).collect();
    assert_eq!(heads.len(), 2, "B and C are competing current heads");
    assert_eq!(superseded.len(), 1, "A is replaced");
    assert_eq!(superseded[0]["id"].as_str().unwrap(), a);

    let edges = laid["edges"].as_array().expect("assessments.laidOut.edges");
    assert_eq!(edges.len(), 1, "only B replaces A");
    assert_eq!(edges[0]["from"].as_str().unwrap(), b);
    assert_eq!(edges[0]["to"].as_str().unwrap(), a);
    assert_eq!(edges[0]["kind"], "replaces", "the tagged edge model");

    // This revision has no superseded observation -> no observation graph.
    assert!(doc["factSupersession"].get("observations").is_none());
}

#[test]
fn superseded_observations_emit_a_tagged_fact_graph() {
    let repo = repo_with_change();
    let arg = repo.path().to_str().unwrap();
    let rev = capture(repo.path());

    let first: Value = serde_json::from_slice(
        &shore([
            "observation",
            "add",
            "--repo",
            arg,
            "--track",
            "agent:codex",
            "--title",
            "first note",
            "--body",
            "original",
        ])
        .stdout,
    )
    .expect("observation add JSON");
    let o1 = first["observationId"]
        .as_str()
        .expect("observationId")
        .to_owned();
    let second: Value = serde_json::from_slice(
        &shore([
            "observation",
            "add",
            "--repo",
            arg,
            "--track",
            "agent:codex",
            "--title",
            "correction",
            "--body",
            "revised",
            "--supersedes",
            &o1,
        ])
        .stdout,
    )
    .expect("observation add JSON");
    let o2 = second["observationId"]
        .as_str()
        .expect("observationId")
        .to_owned();

    let doc = Inspector::spawn(repo.path()).get_json(&format!("/api/revisions/{rev}"));

    let laid = &doc["factSupersession"]["observations"]["laidOut"];
    let nodes = laid["nodes"]
        .as_array()
        .expect("observations.laidOut.nodes");
    assert_eq!(nodes.len(), 2);
    let heads: Vec<&Value> = nodes.iter().filter(|n| n["isHead"] == true).collect();
    let superseded: Vec<&Value> = nodes.iter().filter(|n| n["isSuperseded"] == true).collect();
    assert_eq!(heads.len(), 1);
    assert_eq!(heads[0]["id"].as_str().unwrap(), o2, "the active head");
    assert_eq!(superseded.len(), 1);
    assert_eq!(superseded[0]["id"].as_str().unwrap(), o1);

    let edges = laid["edges"]
        .as_array()
        .expect("observations.laidOut.edges");
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["from"].as_str().unwrap(), o2);
    assert_eq!(edges[0]["to"].as_str().unwrap(), o1);
    assert_eq!(edges[0]["kind"], "supersedes");

    // Unassessed revision -> no assessment graph.
    assert!(doc["factSupersession"].get("assessments").is_none());
}

#[test]
fn non_forked_revision_omits_fact_supersession() {
    // representative_store: a RESOLVED assessment (one replaces the other -> 1
    // current) and a non-superseded observation -> neither fact type forks.
    let store = representative_store();
    let doc = Inspector::spawn(store.repo.path())
        .get_json(&format!("/api/revisions/{}", store.revision_id));
    assert_eq!(doc["currentAssessment"]["status"], "resolved");
    assert!(
        doc.get("factSupersession").is_none(),
        "no fork -> field omitted (byte-identical)"
    );
}

#[test]
fn shared_review_show_document_has_no_fact_supersession() {
    // The fork lives only on the inspector wire, never in pointbreak.review-revision.
    let repo = repo_with_change();
    let arg = repo.path().to_str().unwrap();
    let rev = capture(repo.path());
    let a = assessment_id(&shore([
        "assessment",
        "add",
        "--repo",
        arg,
        "--track",
        "agent:codex",
        "--assessment",
        "accepted",
        "--summary",
        "lgtm",
    ]));
    let _ = a;
    let _ = shore([
        "assessment",
        "add",
        "--repo",
        arg,
        "--track",
        "human:kevin",
        "--assessment",
        "needs-changes",
        "--summary",
        "hold",
    ]);

    let show: Value =
        serde_json::from_slice(&shore(["revision", "show", &rev, "--repo", arg]).stdout)
            .expect("revision show JSON");
    assert!(
        show.get("factSupersession").is_none(),
        "shared doc must not carry the graph"
    );
}
