mod support;
use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore_env;

/// Find the captured ReviewUnit event id from the store the CLI wrote, via the
/// PUBLIC read path (INV-F: `tests/` see only `pub` — `EventStore` is `pub(crate)`,
/// so use `read_events`, which returns `Vec<ShoreEvent>` with public `event_id`/`event_type`).
fn captured_event_id(repo_path: &std::path::Path) -> String {
    let events = shoreline::session::read_events(repo_path).unwrap();
    events
        .iter()
        .find(|e| e.event_type == shoreline::session::event::EventType::ReviewUnitCaptured)
        .expect("a captured review unit event")
        .event_id
        .as_str()
        .to_owned()
}

#[test]
fn endorse_with_signing_off_is_a_hard_error() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn v() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn v() -> u32 { 2 }\n");
    let _ = shore_env(
        ["review", "capture", "--repo", repo.path().to_str().unwrap()],
        &[],
    );

    // SHORE_SIGNING=off → no signer resolves → endorsement has no content → hard error.
    let out = shore_env(
        [
            "review",
            "endorse",
            "evt:sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "--repo",
            repo.path().to_str().unwrap(),
        ],
        &[("SHORE_SIGNING", "off")],
    );
    assert!(
        !out.status.success(),
        "unsigned endorsement must exit non-zero (INV-D)"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("panicked"),
        "clean error, not a panic: {stderr}"
    );
    assert!(
        !stderr.is_empty(),
        "names why: no signer ⇒ nothing to attest"
    );
}

#[test]
fn endorse_help_states_unsigned_is_an_error() {
    let out = shore_env(["review", "endorse", "--help"], &[]);
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout).to_lowercase();
    // The help must state the no-unsigned-degrade contract (INV-D).
    assert!(help.contains("sign"), "help mentions signing: {help}");
}

/// Capture a one-file change in a fresh repo (sharing `home_str`'s keystore) and
/// return the captured ReviewUnit event id — the endorse-target boilerplate.
fn capture_target(home_str: &str) -> (GitRepo, String) {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn v() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn v() -> u32 { 2 }\n");
    let cap = shore_env(
        ["review", "capture", "--repo", repo.path().to_str().unwrap()],
        &[("SHORE_HOME", home_str)],
    );
    assert!(
        cap.status.success(),
        "capture stderr:\n{}",
        String::from_utf8_lossy(&cap.stderr)
    );
    let target = captured_event_id(repo.path());
    (repo, target)
}

#[test]
fn endorse_with_a_key_emits_review_endorse_document_and_writes_a_carrier() {
    let home = tempfile::tempdir().unwrap();
    let home_str = home.path().to_str().unwrap();
    // A signing key in an overridden keystore.
    let _ = shore_env(
        ["keys", "init", "--name", "default"],
        &[("SHORE_HOME", home_str)],
    );

    let (repo, target) = capture_target(home_str);
    let out = shore_env(
        [
            "review",
            "endorse",
            &target,
            "--repo",
            repo.path().to_str().unwrap(),
        ],
        &[
            ("SHORE_HOME", home_str),
            ("SHORE_ACTOR_ID", "actor:git-email:kevin@swiber.dev"),
        ],
    );
    assert!(
        out.status.success(),
        "endorse stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["schema"], "shore.review-endorse");
    assert_eq!(doc["targetEventId"], target);
    assert_eq!(doc["actorId"], "actor:git-email:kevin@swiber.dev"); // endorser's own actor (INV-D)
    assert_eq!(doc["eventsCreated"], 1);
    assert!(
        doc["attestingSigner"]
            .as_str()
            .unwrap()
            .starts_with("did:key:z")
    );

    // A detached co-signature carrier now exists in the store (public read path).
    let events = shoreline::session::read_events(repo.path()).unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.event_type == shoreline::session::event::EventType::EventSignatureRecorded)
    );
}

#[test]
fn endorse_is_idempotent_for_the_same_signer_and_target() {
    let home = tempfile::tempdir().unwrap();
    let home_str = home.path().to_str().unwrap();
    let _ = shore_env(
        ["keys", "init", "--name", "default"],
        &[("SHORE_HOME", home_str)],
    );
    let (repo, target) = capture_target(home_str);
    let env = [
        ("SHORE_HOME", home_str),
        ("SHORE_ACTOR_ID", "actor:git-email:kevin@swiber.dev"),
    ];

    let first = shore_env(
        [
            "review",
            "endorse",
            &target,
            "--repo",
            repo.path().to_str().unwrap(),
        ],
        &env,
    );
    let second = shore_env(
        [
            "review",
            "endorse",
            &target,
            "--repo",
            repo.path().to_str().unwrap(),
        ],
        &env,
    );
    let a: Value = serde_json::from_slice(&first.stdout).unwrap();
    let b: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(a["eventsCreated"], 1);
    assert_eq!(b["eventsCreated"], 0);
    assert_eq!(b["eventsExisting"], 1);
    assert_eq!(
        a["eventId"], b["eventId"],
        "same signer over same target → same carrier"
    );
}
