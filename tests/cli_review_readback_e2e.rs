mod support;
use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore_env;

const ENDORSER: &str = "actor:git-email:kevin@swiber.dev";

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

/// Drive the full chain through the real `shore` binary: init a key, optionally
/// enroll it under the ENDORSER and attest kind/roles, capture UNSIGNED (so the
/// detached endorsement carrier is never deduped against an inline member), then
/// endorse as the ENDORSER (signed by the minted key).
fn endorsed_repo(home: &str, enroll: bool, attest: bool) -> (GitRepo, String) {
    assert!(
        shore_env(
            ["keys", "init", "--name", "default"],
            &[("SHORE_HOME", home)]
        )
        .status
        .success()
    );
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn v() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn v() -> u32 { 2 }\n");
    let repo_arg = repo.path().to_str().unwrap().to_owned();

    if enroll {
        // Bind the default key's did:key to the ENDORSER (reader trust config).
        assert!(
            shore_env(
                [
                    "keys", "enroll", "default", "--actor", ENDORSER, "--repo", &repo_arg
                ],
                &[("SHORE_HOME", home)],
            )
            .status
            .success()
        );
    }
    if attest {
        assert!(
            shore_env(
                [
                    "identity", "attest", ENDORSER, "--kind", "human", "--role", "reviewer",
                    "--repo", &repo_arg,
                ],
                &[],
            )
            .status
            .success()
        );
    }
    // Capture as the git committer (shore-tests), UNSIGNED so there is no inline member
    // the detached endorsement carrier could be deduped against.
    assert!(
        shore_env(
            ["capture", "--repo", &repo_arg],
            &[("SHORE_HOME", home), ("SHORE_SIGNING", "off")],
        )
        .status
        .success()
    );
    let target = captured_event_id(repo.path());
    // Endorse as the ENDORSER, signed by default → attestingSigner = the enrolled did:key.
    assert!(
        shore_env(
            ["review", "endorse", &target, "--repo", &repo_arg],
            &[("SHORE_HOME", home), ("SHORE_ACTOR_ID", ENDORSER)],
        )
        .status
        .success()
    );
    (repo, target)
}

fn endorsement_for_target<'a>(doc: &'a Value, target: &str) -> &'a Value {
    doc["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["eventId"] == target)
        .expect("the endorsed entry")
        .get("endorsements")
        .and_then(|e| e.get(0))
        .expect("an endorsement readback")
}

#[test]
fn enrolled_endorser_reads_endorsement_trusted_with_endorser() {
    let home = tempfile::tempdir().unwrap();
    let (repo, target) = endorsed_repo(home.path().to_str().unwrap(), true, false);
    let out = shore_env(
        ["review", "history", "--repo", repo.path().to_str().unwrap()],
        &[("SHORE_HOME", home.path().to_str().unwrap())],
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    let endorsement = endorsement_for_target(&doc, &target);
    assert_eq!(endorsement["classification"], "endorsement-trusted");
    assert_eq!(endorsement["endorser"], ENDORSER);
}

#[test]
fn unenrolled_signer_reads_unknown_endorser() {
    let home = tempfile::tempdir().unwrap();
    let (repo, target) = endorsed_repo(home.path().to_str().unwrap(), false, false);
    let out = shore_env(
        ["review", "history", "--repo", repo.path().to_str().unwrap()],
        &[("SHORE_HOME", home.path().to_str().unwrap())],
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    let endorsement = endorsement_for_target(&doc, &target);
    assert_eq!(endorsement["classification"], "unknown_endorser");
    assert!(endorsement.get("endorser").is_none() || endorsement["endorser"].is_null());
}

#[test]
fn attested_kind_and_roles_surface_in_enrichment() {
    let home = tempfile::tempdir().unwrap();
    let (repo, target) = endorsed_repo(home.path().to_str().unwrap(), true, true);
    let out = shore_env(
        ["review", "history", "--repo", repo.path().to_str().unwrap()],
        &[("SHORE_HOME", home.path().to_str().unwrap())],
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    let endorsement = endorsement_for_target(&doc, &target);
    assert_eq!(endorsement["classification"], "endorsement-trusted");
    assert_eq!(endorsement["endorserAttributes"]["kind"], "human");
    assert_eq!(endorsement["endorserAttributes"]["roles"][0], "reviewer");
}
