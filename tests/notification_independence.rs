//! The notification-independence invariant: a liveness read is never a
//! precondition for, and never a side effect on, a write. The journal an actor
//! produces is byte-identical whether or not anyone reads its liveness, and a
//! change signal that is computed and then dropped neither un-writes nor fails
//! the durable write. This is the guarantee that keeps the attention layer from
//! sliding into executive control: the core emits the change fact (the token)
//! but never delivers it, and nothing the reader does can reach back into the
//! log.
//!
//! These are guard tests. On a correctly pull-only core they pass as written;
//! a failure here means a liveness read mutated the store, which is the bug
//! this suite forecloses.

mod support;

use std::path::Path;

use pointbreak::model::RevisionId;
use pointbreak::session::{LivenessToken, read_events};
use support::git_repo::GitRepo;
use support::inspect::capture;

/// The durable log as the pair that defines event identity, in store order.
fn event_fingerprints(repo: &Path) -> Vec<(String, String)> {
    read_events(repo)
        .expect("read events")
        .iter()
        .map(|event| {
            (
                event.event_id.as_str().to_owned(),
                event.payload_hash.clone(),
            )
        })
        .collect()
}

fn journal_hash(repo: &Path) -> String {
    LivenessToken::for_journal(&read_events(repo).expect("read events"))
        .expect("liveness token")
        .event_set_hash
}

/// A repository with one captured review, returning the repo and its unit id.
fn repo_with_one_capture() -> (GitRepo, RevisionId) {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    let revision_id = RevisionId::new(capture(repo.path()));
    (repo, revision_id)
}

#[test]
fn write_is_byte_identical_with_or_without_a_liveness_read() {
    let (repo, revision_id) = repo_with_one_capture();

    // The log as written, before any liveness read touches it.
    let before = event_fingerprints(repo.path());
    let hash_before = journal_hash(repo.path());
    assert!(!before.is_empty(), "the capture wrote at least one event");

    // Hammer the read side: journal- and work-object-scoped tokens, repeatedly.
    // A read changes when you look, never what is written.
    for _ in 0..16 {
        let events = read_events(repo.path()).expect("read events");
        let _ = LivenessToken::for_journal(&events).expect("journal token");
        let _ = LivenessToken::for_work_object(&events, &revision_id).expect("scoped token");
    }

    let after = event_fingerprints(repo.path());
    let hash_after = journal_hash(repo.path());

    assert_eq!(
        before, after,
        "liveness reads must not add, drop, or rewrite any event"
    );
    assert_eq!(
        hash_before, hash_after,
        "the event-set hash is stable across liveness reads"
    );
}

#[test]
fn dropped_change_signal_neither_unwrites_nor_fails_the_write() {
    let (repo, _revision_id) = repo_with_one_capture();

    // The durable write has already landed.
    let durable = event_fingerprints(repo.path());
    assert!(!durable.is_empty(), "the capture is durable");

    // Simulate "publish failed": compute the change signal and discard it
    // without acting on it. There is no publish path in the core, so this pins
    // that a future relay drop cannot reach back into the log.
    let token = LivenessToken::for_journal(&read_events(repo.path()).expect("read events"))
        .expect("liveness token");
    drop(token);

    // The write is untouched by the dropped signal.
    assert_eq!(
        durable,
        event_fingerprints(repo.path()),
        "a dropped change signal must not un-write the durable log"
    );

    // And a later transition still succeeds and is durable — a dropped signal
    // is never a precondition for the next write.
    let output = support::shore([
        "observation",
        "add",
        "--repo",
        repo.path().to_str().unwrap(),
        "--track",
        "agent:codex",
        "--title",
        "after a dropped signal",
        "--body",
        "the next write still lands",
    ]);
    assert!(
        output.status.success(),
        "observation add after a dropped signal failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        event_fingerprints(repo.path()).len() > durable.len(),
        "the post-signal write is durably appended"
    );
}
