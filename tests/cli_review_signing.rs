mod support;

use std::path::Path;
use std::process::Output;

use serde_json::Value;
use shoreline::crypto::EventVerificationStatus;
use shoreline::session::event::EventType;
use shoreline::session::{EventVerificationPolicy, ReviewHistoryOptions, TrustSet, review_history};
use support::git_repo::GitRepo;
use support::shore_env;

/// A repo with a committed base and an uncommitted change, so `shore capture`
/// has a HEAD -> working-tree diff to capture.
fn modified_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo
}

fn parse(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).expect("stdout is json")
}

/// Verify the captured event back from the store the CLI wrote, under the given
/// trust set. The CLI `review history` path does not render verificationStatus
/// (it sets no verification policy); the loop-closure that threads trust into the
/// read paths lands later. Here the test crate reads the same store through the
/// library with an advisory policy to assert the signature ladder directly.
fn capture_status(repo: &Path, trust: TrustSet) -> Option<EventVerificationStatus> {
    let history = review_history(
        ReviewHistoryOptions::new(repo)
            .with_verification_policy(EventVerificationPolicy::advisory())
            .with_trust_set(trust),
    )
    .expect("review history reads back");
    history
        .entries
        .iter()
        .find(|entry| entry.event_type == EventType::WorkObjectProposed)
        .and_then(|entry| entry.verification_status)
}

#[test]
fn write_with_no_key_is_unsigned_and_exit_zero() {
    let home = tempfile::tempdir().unwrap();
    let repo = modified_repo();
    let out = shore_env(
        ["capture", "--repo", repo.path().to_str().unwrap()],
        &[("SHORE_HOME", home.path().to_str().unwrap())],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        capture_status(repo.path(), TrustSet::default()),
        Some(EventVerificationStatus::Unsigned),
        "no key configured -> unsigned"
    );
}

#[test]
fn write_with_signing_off_is_unsigned_and_exit_zero_even_with_a_default_key() {
    let home = tempfile::tempdir().unwrap();
    let env_home = home.path().to_str().unwrap();
    // A "default" key exists, but SHORE_SIGNING=off forces no signing.
    let init = shore_env(
        ["keys", "init", "--name", "default"],
        &[("SHORE_HOME", env_home)],
    );
    assert!(init.status.success());

    let repo = modified_repo();
    let out = shore_env(
        ["capture", "--repo", repo.path().to_str().unwrap()],
        &[("SHORE_HOME", env_home), ("SHORE_SIGNING", "off")],
    );
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        capture_status(repo.path(), TrustSet::default()),
        Some(EventVerificationStatus::Unsigned),
        "SHORE_SIGNING=off -> unsigned even with a default key"
    );
}

#[test]
fn write_is_unsigned_and_exit_zero_when_keygen_is_forced_to_fail() {
    // An agent actor would auto-keygen, but SHORE_HOME points at a regular file so
    // the keystore directory cannot be created and keygen fails.
    let file_home = tempfile::NamedTempFile::new().unwrap();
    let repo = modified_repo();
    let out = shore_env(
        ["capture", "--repo", repo.path().to_str().unwrap()],
        &[
            ("SHORE_HOME", file_home.path().to_str().unwrap()),
            ("SHORE_ACTOR_ID", "actor:agent:claude-code"),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "keygen failure must not gate the write"
    );
    assert_eq!(
        capture_status(repo.path(), TrustSet::default()),
        Some(EventVerificationStatus::Unsigned),
    );
}

#[test]
fn present_but_unenrolled_key_signs_and_verifies_untrusted_key() {
    let home = tempfile::tempdir().unwrap();
    let env_home = home.path().to_str().unwrap();
    let init = shore_env(
        ["keys", "init", "--name", "default"],
        &[("SHORE_HOME", env_home)],
    );
    assert!(init.status.success());

    let repo = modified_repo();
    let out = shore_env(
        ["capture", "--repo", repo.path().to_str().unwrap()],
        &[("SHORE_HOME", env_home)],
    );
    assert_eq!(out.status.code(), Some(0));
    // Signed by a key not in any allow-list: tamper-evident, strictly better than unsigned.
    assert_eq!(
        capture_status(repo.path(), TrustSet::default()),
        Some(EventVerificationStatus::UntrustedKey),
    );
}

#[test]
fn ci_ephemeral_did_key_self_certifies_valid_under_empty_trust_set() {
    let home = tempfile::tempdir().unwrap();
    let env_home = home.path().to_str().unwrap();
    let init = shore_env(
        ["keys", "init", "--name", "ci"],
        &[("SHORE_HOME", env_home)],
    );
    assert!(init.status.success());
    let did_key = parse(&init)["didKey"].as_str().unwrap().to_owned();

    let repo = modified_repo();
    // The writing actor IS the signing key's did:key -> self-certifying.
    let out = shore_env(
        [
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--sign-key",
            "ci",
        ],
        &[
            ("SHORE_HOME", env_home),
            ("SHORE_ACTOR_ID", did_key.as_str()),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Verifies valid under an EMPTY trust set, with no enrollment.
    assert_eq!(
        capture_status(repo.path(), TrustSet::default()),
        Some(EventVerificationStatus::Valid),
    );
}

#[test]
fn sign_key_flag_signs_the_write() {
    let home = tempfile::tempdir().unwrap();
    let env_home = home.path().to_str().unwrap();
    let init = shore_env(
        ["keys", "init", "--name", "mykey"],
        &[("SHORE_HOME", env_home)],
    );
    assert!(init.status.success());

    let repo = modified_repo();
    let out = shore_env(
        [
            "capture",
            "--repo",
            repo.path().to_str().unwrap(),
            "--sign-key",
            "mykey",
        ],
        &[("SHORE_HOME", env_home)],
    );
    assert_eq!(out.status.code(), Some(0));
    // Signed (un-enrolled) -> untrusted_key, not unsigned.
    assert_eq!(
        capture_status(repo.path(), TrustSet::default()),
        Some(EventVerificationStatus::UntrustedKey),
    );
}

#[test]
fn all_six_write_paths_stay_exit_zero_without_a_key() {
    let home = tempfile::tempdir().unwrap();
    let env = [("SHORE_HOME", home.path().to_str().unwrap())];
    let repo = modified_repo();
    let repo_arg = repo.path().to_str().unwrap().to_owned();

    let capture = shore_env(["capture", "--repo", &repo_arg], &env);
    assert_eq!(
        capture.status.code(),
        Some(0),
        "capture: {}",
        String::from_utf8_lossy(&capture.stderr)
    );

    let observation = shore_env(
        [
            "observation",
            "add",
            "--repo",
            &repo_arg,
            "--track",
            "agent:codex",
            "--title",
            "t",
            "--body",
            "b",
        ],
        &env,
    );
    assert_eq!(observation.status.code(), Some(0));

    let assessment = shore_env(
        [
            "assessment",
            "add",
            "--repo",
            &repo_arg,
            "--track",
            "human:kevin",
            "--assessment",
            "accepted",
            "--summary",
            "ship it",
        ],
        &env,
    );
    assert_eq!(assessment.status.code(), Some(0));

    let validation = shore_env(
        [
            "validation",
            "add",
            "--repo",
            &repo_arg,
            "--track",
            "agent:codex",
            "--check-name",
            "cargo test",
            "--status",
            "passed",
        ],
        &env,
    );
    assert_eq!(validation.status.code(), Some(0));

    let open = shore_env(
        [
            "review",
            "input-request",
            "open",
            "--repo",
            &repo_arg,
            "--track",
            "human:kevin",
            "--title",
            "Need approval",
            "--reason",
            "manual-decision-required",
            "--body",
            "ok?",
        ],
        &env,
    );
    assert_eq!(
        open.status.code(),
        Some(0),
        "open: {}",
        String::from_utf8_lossy(&open.stderr)
    );
    let input_request_id = parse(&open)["inputRequestId"].as_str().unwrap().to_owned();

    let respond = shore_env(
        [
            "review",
            "input-request",
            "respond",
            &input_request_id,
            "--repo",
            &repo_arg,
            "--outcome",
            "approved",
            "--reason",
            "approved locally",
        ],
        &env,
    );
    assert_eq!(
        respond.status.code(),
        Some(0),
        "respond: {}",
        String::from_utf8_lossy(&respond.stderr)
    );
}

#[test]
fn all_six_write_paths_accept_sign_key_flag() {
    let home = tempfile::tempdir().unwrap();
    let env_home = home.path().to_str().unwrap();
    let env = [("SHORE_HOME", env_home)];
    assert!(
        shore_env(["keys", "init", "--name", "default"], &env)
            .status
            .success()
    );

    let repo = modified_repo();
    let repo_arg = repo.path().to_str().unwrap().to_owned();

    let capture = shore_env(
        ["capture", "--repo", &repo_arg, "--sign-key", "default"],
        &env,
    );
    assert_eq!(capture.status.code(), Some(0));

    let observation = shore_env(
        [
            "observation",
            "add",
            "--repo",
            &repo_arg,
            "--track",
            "agent:codex",
            "--title",
            "t",
            "--body",
            "b",
            "--sign-key",
            "default",
        ],
        &env,
    );
    assert_eq!(observation.status.code(), Some(0));

    let assessment = shore_env(
        [
            "assessment",
            "add",
            "--repo",
            &repo_arg,
            "--track",
            "human:kevin",
            "--assessment",
            "accepted",
            "--summary",
            "ship it",
            "--sign-key",
            "default",
        ],
        &env,
    );
    assert_eq!(assessment.status.code(), Some(0));

    let validation = shore_env(
        [
            "validation",
            "add",
            "--repo",
            &repo_arg,
            "--track",
            "agent:codex",
            "--check-name",
            "cargo test",
            "--status",
            "passed",
            "--sign-key",
            "default",
        ],
        &env,
    );
    assert_eq!(validation.status.code(), Some(0));

    let open = shore_env(
        [
            "review",
            "input-request",
            "open",
            "--repo",
            &repo_arg,
            "--track",
            "human:kevin",
            "--title",
            "Need approval",
            "--reason",
            "manual-decision-required",
            "--body",
            "ok?",
            "--sign-key",
            "default",
        ],
        &env,
    );
    assert_eq!(
        open.status.code(),
        Some(0),
        "open: {}",
        String::from_utf8_lossy(&open.stderr)
    );
    let input_request_id = parse(&open)["inputRequestId"].as_str().unwrap().to_owned();

    let respond = shore_env(
        [
            "review",
            "input-request",
            "respond",
            &input_request_id,
            "--repo",
            &repo_arg,
            "--outcome",
            "approved",
            "--reason",
            "approved locally",
            "--sign-key",
            "default",
        ],
        &env,
    );
    assert_eq!(
        respond.status.code(),
        Some(0),
        "respond: {}",
        String::from_utf8_lossy(&respond.stderr)
    );
}

/// A real `ssh-keygen -t ed25519` public key (the same golden key the parser pins).
const SSH_ED25519_PUBKEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAID7lnwK7O5CFXew1hBuUnXz1+zK2pQtYEtxsbRMiOyvP dev@example";

#[test]
fn agent_unavailable_write_is_unsigned_and_exit_zero() {
    let home = tempfile::tempdir().unwrap();
    let env_home = home.path().to_str().unwrap();
    // Adopt an agent-backed `default` reference (no live agent needed to write it).
    let adopt = shore_env(
        ["keys", "use-ssh", &format!("key::{SSH_ED25519_PUBKEY}")],
        &[("SHORE_HOME", env_home)],
    );
    assert!(
        adopt.status.success(),
        "adopt stderr:\n{}",
        String::from_utf8_lossy(&adopt.stderr)
    );

    let repo = modified_repo();
    // A dead SSH_AUTH_SOCK makes the agent pre-flight fail deterministically (and
    // portably — a bogus path fails to open on every OS), so the write degrades to
    // unsigned without gating.
    let out = shore_env(
        ["capture", "--repo", repo.path().to_str().unwrap()],
        &[
            ("SHORE_HOME", env_home),
            ("SSH_AUTH_SOCK", "/nonexistent/shore-no-agent.sock"),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "agent unavailable must not gate the write:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("signing_agent_unavailable"),
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        capture_status(repo.path(), TrustSet::default()),
        Some(EventVerificationStatus::Unsigned),
    );
}
