mod support;

use std::path::Path;

use pointbreak::crypto::EventVerificationStatus;
use pointbreak::keys::{KeyName, generate_key_in, load_signer_in};
use pointbreak::model::ActorId;
use pointbreak::session::event::EventType;
use pointbreak::session::{
    CaptureOptions, EventVerificationPolicy, ReviewHistoryOptions, TrustSet,
    capture_worktree_review, review_history, stage_enrollment,
};
use support::git_repo::GitRepo;

/// A repo with a committed base and an uncommitted change, so `capture` has a
/// HEAD -> working-tree diff.
fn modified_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo
}

/// Read the captured event back under the given trust set with an advisory policy.
fn capture_status(repo: &Path, trust: TrustSet) -> Option<EventVerificationStatus> {
    let history = review_history(
        ReviewHistoryOptions::new(repo)
            .with_verification_policy(EventVerificationPolicy::advisory())
            .with_trust_set(trust),
    )
    .expect("history reads back");
    history
        .entries
        .iter()
        .find(|entry| entry.event_type == EventType::WorkObjectProposed)
        .and_then(|entry| entry.verification_status)
}

#[test]
fn enrolled_signer_renders_valid_unenrolled_renders_untrusted_key() {
    let keys_home = tempfile::tempdir().unwrap();
    let key = generate_key_in(keys_home.path(), &KeyName::parse("loopkey").unwrap()).unwrap();
    let signer = load_signer_in(keys_home.path(), "loopkey").unwrap();
    let actor = ActorId::new("actor:git-email:alice@example.com");

    // Signed capture by the actor (signer != actor, so event.signer is recorded).
    let origin = modified_repo();
    capture_worktree_review(
        CaptureOptions::new(origin.path())
            .with_actor_id(actor.clone())
            .sign_with(signer),
    )
    .unwrap();

    // Enroll the signer into the committed allow-list.
    let path = origin.path().join(".shore/allowed-signers.json");
    stage_enrollment(&path, &actor, key.signer_id()).unwrap();

    // Read under the discovered (committed) trust set: the loop closes -> valid.
    let trust = TrustSet::from_allowed_signers_file(&path).unwrap();
    assert_eq!(
        capture_status(origin.path(), trust),
        Some(EventVerificationStatus::Valid),
        "an enrolled signer's event renders valid"
    );

    // The same signed event under an empty trust set renders untrusted_key.
    assert_eq!(
        capture_status(origin.path(), TrustSet::default()),
        Some(EventVerificationStatus::UntrustedKey),
        "an un-enrolled signer's event renders untrusted_key"
    );
}

#[test]
fn unsigned_event_renders_unsigned_regardless_of_trust_set() {
    let origin = modified_repo();
    let actor = ActorId::new("actor:git-email:alice@example.com");
    capture_worktree_review(CaptureOptions::new(origin.path()).with_actor_id(actor)).unwrap();
    assert_eq!(
        capture_status(origin.path(), TrustSet::default()),
        Some(EventVerificationStatus::Unsigned),
    );
}
