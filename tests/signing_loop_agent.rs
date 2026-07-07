mod support;

use std::path::Path;

use pointbreak::crypto::EventVerificationStatus;
use pointbreak::keys::{
    KeyName, generate_key_in, load_signer_id_in, load_signer_in, write_agent_reference_in,
};
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
fn agent_backed_enrolled_renders_valid_unenrolled_renders_untrusted_key() {
    // A file key supplies a real keypair; its PUBLIC key backs the agent reference,
    // so the two share one did:key. The enrollment side derives that did:key from the
    // agent reference's public material offline (no seed, no agent); the signing side
    // uses the matching file signer to produce the byte-identical Ed25519 signature a
    // live agent holding the same key would return. The deterministic agent-signature
    // round-trip itself is pinned where the in-process fake agent is reachable; this
    // test proves the offline-enroll + render-valid half for an agent-backed key.
    let keys_home = tempfile::tempdir().unwrap();
    let file_key = generate_key_in(keys_home.path(), &KeyName::parse("filekey").unwrap()).unwrap();
    let public = file_key.signer_id().ed25519_public_key().unwrap();
    let signer = load_signer_in(keys_home.path(), "filekey").unwrap();

    // Adopt an agent-backed reference for the SAME public key — no agent running.
    let agent_ref = write_agent_reference_in(
        keys_home.path(),
        &KeyName::parse("agentref").unwrap(),
        public,
    )
    .unwrap();
    assert_eq!(
        agent_ref.signer_id(),
        file_key.signer_id(),
        "the agent reference and the file key share one did:key"
    );

    // The did:key the ENROLL side derives offline from the agent reference equals it.
    let enrolled_did = load_signer_id_in(keys_home.path(), "agentref").unwrap();
    assert_eq!(&enrolled_did, agent_ref.signer_id());

    // Sign a real capture with the matching key (the agent's signature stand-in).
    let actor = ActorId::new("actor:git-email:alice@example.com");
    let origin = modified_repo();
    capture_worktree_review(
        CaptureOptions::new(origin.path())
            .with_actor_id(actor.clone())
            .sign_with(signer),
    )
    .unwrap();

    // Enroll the agent-backed key's offline-derived did:key into the allow-list.
    let path = origin.path().join(".shore/allowed-signers.json");
    stage_enrollment(&path, &actor, &enrolled_did).unwrap();

    // Loop closes: the enrolled agent-backed key's signed event renders valid.
    let trust = TrustSet::from_allowed_signers_file(&path).unwrap();
    assert_eq!(
        capture_status(origin.path(), trust),
        Some(EventVerificationStatus::Valid),
        "an enrolled agent-backed key's event renders valid"
    );
    // Un-enrolled: the same signed event renders untrusted_key.
    assert_eq!(
        capture_status(origin.path(), TrustSet::default()),
        Some(EventVerificationStatus::UntrustedKey),
        "an un-enrolled agent-backed key's event renders untrusted_key"
    );
}
