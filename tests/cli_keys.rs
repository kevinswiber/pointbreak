mod support;

use std::process::Command;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::Value;
use support::shore_env;

/// Mirror the encoding the `keys show --pubkey` command emits (base64 standard),
/// so the consistency test pins agreement rather than a hardcoded string.
fn encode_pubkey(bytes: &[u8]) -> String {
    BASE64.encode(bytes)
}

#[test]
fn keys_init_writes_key_and_emits_did_key_document() {
    let home = tempfile::tempdir().expect("create keystore home");
    let out = shore_env(
        ["keys", "init", "--name", "default"],
        &[("SHORE_HOME", home.path().to_str().unwrap())],
    );
    assert!(
        out.status.success(),
        "init stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: Value = serde_json::from_slice(&out.stdout).expect("stdout is json");
    assert_eq!(json["schema"], "shore.keys-init");
    assert_eq!(json["name"], "default");
    assert!(
        json["didKey"].as_str().unwrap().starts_with("did:key:z"),
        "did:key present: {json:#}"
    );
    // The keystore wrote a key file under the overridden home.
    let path = json["path"].as_str().expect("path field");
    assert!(
        std::path::Path::new(path).exists(),
        "key file exists at {path}"
    );
}

#[test]
fn keys_init_defaults_name_to_default() {
    let home = tempfile::tempdir().expect("create keystore home");
    let out = shore_env(
        ["keys", "init"],
        &[("SHORE_HOME", home.path().to_str().unwrap())],
    );
    assert!(out.status.success());
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["name"], "default");
}

#[test]
fn keys_init_twice_same_name_is_a_clean_error_not_a_panic() {
    let home = tempfile::tempdir().expect("create keystore home");
    let env = [("SHORE_HOME", home.path().to_str().unwrap())];
    let first = shore_env(["keys", "init", "--name", "default"], &env);
    assert!(first.status.success());

    let second = shore_env(["keys", "init", "--name", "default"], &env);
    assert!(!second.status.success(), "second init must fail");
    let stderr = String::from_utf8_lossy(&second.stderr);
    // A clean CLI error: a message on stderr, not a Rust panic.
    assert!(!stderr.contains("panicked"), "no panic: {stderr}");
    assert!(!stderr.is_empty(), "an error message is printed");
}

#[test]
fn keys_init_rejects_path_unsafe_name_without_escaping_the_keystore() {
    let home = tempfile::tempdir().expect("create keystore home");
    let out = shore_env(
        ["keys", "init", "--name", "../../id_ed25519"],
        &[("SHORE_HOME", home.path().to_str().unwrap())],
    );
    assert!(
        !out.status.success(),
        "a path-unsafe key name must be rejected"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("panicked"),
        "clean error, not a panic: {stderr}"
    );
    // Nothing was written outside the keystore root.
    assert!(!home.path().parent().unwrap().join("id_ed25519").exists());
}

#[test]
fn keys_list_reports_generated_keys_and_marks_default() {
    let home = tempfile::tempdir().expect("create keystore home");
    let env = [("SHORE_HOME", home.path().to_str().unwrap())];
    let _ = shore_env(["keys", "init", "--name", "default"], &env);
    let _ = shore_env(["keys", "init", "--name", "work"], &env);

    let repo = support::git_repo::GitRepo::new();
    let out = shore_env(
        ["keys", "list", "--repo", repo.path().to_str().unwrap()],
        &env,
    );
    assert!(
        out.status.success(),
        "list stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["schema"], "shore.keys-list");
    let keys = json["keys"].as_array().expect("keys array");
    assert_eq!(keys.len(), 2);

    let default = keys.iter().find(|k| k["name"] == "default").unwrap();
    assert_eq!(default["default"], true);
    assert_eq!(default["enrolled"], false); // no allowed-signers file
    let work = keys.iter().find(|k| k["name"] == "work").unwrap();
    assert_eq!(work["default"], false);
}

#[test]
fn keys_list_marks_enrolled_only_when_did_key_is_in_allowed_signers() {
    let home = tempfile::tempdir().expect("create keystore home");
    let env = [("SHORE_HOME", home.path().to_str().unwrap())];
    let init = shore_env(["keys", "init", "--name", "default"], &env);
    let init_json: Value = serde_json::from_slice(&init.stdout).unwrap();
    let did_key = init_json["didKey"].as_str().unwrap().to_owned();

    let repo = support::git_repo::GitRepo::new();
    // Enroll the default key's did:key under some actor, custom JSON (NOT OpenSSH format).
    let allowed =
        format!(r#"{{"allowedSigners":{{"actor:git-email:dev@example.com":["{did_key}"]}}}}"#);
    repo.write(".shore/allowed-signers.json", &allowed);

    let out = shore_env(
        ["keys", "list", "--repo", repo.path().to_str().unwrap()],
        &env,
    );
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    let default = json["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["name"] == "default")
        .unwrap();
    assert_eq!(default["enrolled"], true);
}

#[test]
fn keys_list_empty_keystore_is_empty_list_exit_zero() {
    let home = tempfile::tempdir().expect("create keystore home");
    let env = [("SHORE_HOME", home.path().to_str().unwrap())];
    let repo = support::git_repo::GitRepo::new();
    let out = shore_env(
        ["keys", "list", "--repo", repo.path().to_str().unwrap()],
        &env,
    );
    assert!(out.status.success());
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["schema"], "shore.keys-list");
    assert_eq!(json["keys"].as_array().unwrap().len(), 0);
}

#[test]
fn keys_show_default_with_did_prints_the_did_key() {
    let home = tempfile::tempdir().expect("create keystore home");
    let env = [("SHORE_HOME", home.path().to_str().unwrap())];
    let init = shore_env(["keys", "init", "--name", "default"], &env);
    let init_json: Value = serde_json::from_slice(&init.stdout).unwrap();
    let did_key = init_json["didKey"].as_str().unwrap().to_owned();

    let out = shore_env(["keys", "show", "default", "--did"], &env);
    assert!(
        out.status.success(),
        "show stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["schema"], "shore.keys-show");
    assert_eq!(json["name"], "default");
    assert_eq!(json["didKey"], did_key);
}

#[test]
fn keys_show_defaults_to_did_key_with_no_field_flags() {
    let home = tempfile::tempdir().expect("create keystore home");
    let env = [("SHORE_HOME", home.path().to_str().unwrap())];
    let _ = shore_env(["keys", "init"], &env);

    // No name and no field flags: defaults to `default` key, did:key field present.
    let out = shore_env(["keys", "show"], &env);
    assert!(out.status.success());
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(json["didKey"].as_str().unwrap().starts_with("did:key:z"));
}

#[test]
fn keys_show_pubkey_is_consistent_with_the_did_key() {
    let home = tempfile::tempdir().expect("create keystore home");
    let env = [("SHORE_HOME", home.path().to_str().unwrap())];
    let init = shore_env(["keys", "init", "--name", "default"], &env);
    let did_key = serde_json::from_slice::<Value>(&init.stdout).unwrap()["didKey"]
        .as_str()
        .unwrap()
        .to_owned();

    let out = shore_env(["keys", "show", "default", "--pubkey"], &env);
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    let pubkey_field = json["publicKey"].as_str().expect("publicKey present");

    // Derive the expected encoding from the did:key's own 32-byte payload so the
    // test pins consistency, not a hardcoded string.
    let signer_id = shoreline::crypto::SignerId::parse(did_key).unwrap();
    let bytes = signer_id.ed25519_public_key().unwrap();
    let expected = encode_pubkey(&bytes);
    assert_eq!(pubkey_field, expected);
}

#[test]
fn keys_show_missing_name_is_a_clean_error_not_a_panic() {
    let home = tempfile::tempdir().expect("create keystore home");
    let env = [("SHORE_HOME", home.path().to_str().unwrap())];
    let out = shore_env(["keys", "show", "does-not-exist", "--did"], &env);
    assert!(!out.status.success(), "missing key must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("panicked"), "no panic: {stderr}");
    assert!(!stderr.is_empty());
}

#[test]
fn keys_enroll_stages_working_tree_file_and_reports_actor_and_did() {
    let home = tempfile::tempdir().expect("create keystore home");
    let home_str = home.path().to_str().unwrap();
    let init = shore_env(
        ["keys", "init", "--name", "default"],
        &[("SHORE_HOME", home_str)],
    );
    let did = serde_json::from_slice::<Value>(&init.stdout).unwrap()["didKey"]
        .as_str()
        .unwrap()
        .to_owned();

    let repo = support::git_repo::GitRepo::new();
    let out = shore_env(
        ["keys", "enroll", "--repo", repo.path().to_str().unwrap()],
        &[
            ("SHORE_HOME", home_str),
            ("SHORE_ACTOR_ID", "actor:agent:claude-code"),
        ],
    );
    assert!(
        out.status.success(),
        "enroll stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["schema"], "shore.keys-enroll");
    assert_eq!(doc["actorId"], "actor:agent:claude-code");
    assert_eq!(doc["signerId"], did);
    assert_eq!(doc["added"], true);

    // The working-tree file exists and the existing reader loads the entry.
    let path = repo.path().join(".shore/allowed-signers.json");
    assert!(path.exists(), "enroll stages the working-tree file");
    let trust = shoreline::session::TrustSet::from_allowed_signers_file(&path).unwrap();
    let actor = shoreline::model::ActorId::new("actor:agent:claude-code");
    let signer = shoreline::crypto::SignerId::parse(&did).unwrap();
    assert!(trust.authorizes(&actor, &signer, "2026-06-16T00:00:00Z"));
}

#[test]
fn keys_enroll_works_for_an_agent_backed_reference() {
    let home = tempfile::tempdir().expect("create keystore home");
    let home_str = home.path().to_str().unwrap();
    // Adopt an agent-backed `default` reference: no agent running, no private key.
    let adopt = shore_env(
        ["keys", "use-ssh", &format!("key::{SSH_ED25519_PUBKEY}")],
        &[("SHORE_HOME", home_str)],
    );
    assert!(
        adopt.status.success(),
        "adopt stderr:\n{}",
        String::from_utf8_lossy(&adopt.stderr)
    );
    let did = serde_json::from_slice::<Value>(&adopt.stdout).unwrap()["didKey"]
        .as_str()
        .unwrap()
        .to_owned();

    let repo = support::git_repo::GitRepo::new();
    let out = shore_env(
        ["keys", "enroll", "--repo", repo.path().to_str().unwrap()],
        &[
            ("SHORE_HOME", home_str),
            ("SHORE_ACTOR_ID", "actor:git-email:dev@example.com"),
        ],
    );
    assert!(
        out.status.success(),
        "enroll an agent-backed key offline:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["schema"], "shore.keys-enroll");
    assert_eq!(doc["signerId"], did, "enrolled the offline-derived did:key");
    assert_eq!(doc["added"], true);

    let path = repo.path().join(".shore/allowed-signers.json");
    let trust = shoreline::session::TrustSet::from_allowed_signers_file(&path).unwrap();
    let actor = shoreline::model::ActorId::new("actor:git-email:dev@example.com");
    let signer = shoreline::crypto::SignerId::parse(&did).unwrap();
    assert!(trust.authorizes(&actor, &signer, "2026-06-16T00:00:00Z"));
}

#[test]
fn keys_list_reports_file_custody_for_a_seed_key() {
    let home = tempfile::tempdir().expect("create keystore home");
    let env = [("SHORE_HOME", home.path().to_str().unwrap())];
    let _ = shore_env(["keys", "init", "--name", "default"], &env);

    let repo = support::git_repo::GitRepo::new();
    let out = shore_env(
        ["keys", "list", "--repo", repo.path().to_str().unwrap()],
        &env,
    );
    assert!(
        out.status.success(),
        "list stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    let key = json["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["name"] == "default")
        .unwrap();
    assert_eq!(key["custody"], "file");
    // A file key has no agentLoaded field (the question is meaningless for it).
    assert!(key.get("agentLoaded").is_none() || key["agentLoaded"].is_null());
}

#[test]
fn keys_list_reports_agent_custody_and_enrollment_for_an_adopted_key() {
    let home = tempfile::tempdir().expect("create keystore home");
    let env = [("SHORE_HOME", home.path().to_str().unwrap())];
    let adopt = shore_env(
        ["keys", "use-ssh", &format!("key::{SSH_ED25519_PUBKEY}")],
        &env,
    );
    let did_key = serde_json::from_slice::<Value>(&adopt.stdout).unwrap()["didKey"]
        .as_str()
        .unwrap()
        .to_owned();

    // Enroll the adopted key's did:key (custom JSON allow-list, NOT OpenSSH format).
    let repo = support::git_repo::GitRepo::new();
    let allowed =
        format!(r#"{{"allowedSigners":{{"actor:git-email:dev@example.com":["{did_key}"]}}}}"#);
    repo.write(".shore/allowed-signers.json", &allowed);

    let out = shore_env(
        ["keys", "list", "--repo", repo.path().to_str().unwrap()],
        &env,
    );
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    let key = json["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["name"] == "default")
        .unwrap();
    assert_eq!(key["custody"], "agent");
    assert_eq!(key["enrolled"], true);
}

#[test]
fn keys_list_succeeds_and_agent_loaded_is_unknown_when_no_agent() {
    let home = tempfile::tempdir().expect("create keystore home");
    let env = [
        ("SHORE_HOME", home.path().to_str().unwrap()),
        // A dead socket path: connect fails. The probe must NOT gate the listing.
        ("SSH_AUTH_SOCK", "/nonexistent/shore-no-agent.sock"),
    ];
    let _ = shore_env(
        ["keys", "use-ssh", &format!("key::{SSH_ED25519_PUBKEY}")],
        &env,
    );

    let repo = support::git_repo::GitRepo::new();
    let out = shore_env(
        ["keys", "list", "--repo", repo.path().to_str().unwrap()],
        &env,
    );

    // The whole point: an unreachable agent does NOT fail the read command.
    assert_eq!(
        out.status.code(),
        Some(0),
        "list never gates on the agent probe"
    );
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    let key = json["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["name"] == "default")
        .unwrap();
    assert_eq!(key["custody"], "agent");
    // Unknown is represented as an omitted (or null) field, never an error.
    assert!(
        key.get("agentLoaded").is_none() || key["agentLoaded"].is_null(),
        "no agent -> agentLoaded unknown, never an error: {key:#}"
    );
}

#[test]
fn keys_enroll_re_enroll_reports_already_present_and_is_a_noop() {
    let home = tempfile::tempdir().expect("create keystore home");
    let home_str = home.path().to_str().unwrap();
    let _ = shore_env(
        ["keys", "init", "--name", "default"],
        &[("SHORE_HOME", home_str)],
    );
    let repo = support::git_repo::GitRepo::new();
    let env = [
        ("SHORE_HOME", home_str),
        ("SHORE_ACTOR_ID", "actor:agent:claude-code"),
    ];

    let first = shore_env(
        ["keys", "enroll", "--repo", repo.path().to_str().unwrap()],
        &env,
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&first.stdout).unwrap()["added"],
        true
    );
    let path = repo.path().join(".shore/allowed-signers.json");
    let before = std::fs::read(&path).unwrap();

    let second = shore_env(
        ["keys", "enroll", "--repo", repo.path().to_str().unwrap()],
        &env,
    );
    let doc: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(doc["added"], false, "second enroll reports already present");
    let after = std::fs::read(&path).unwrap();
    assert_eq!(before, after, "re-enroll leaves the file byte-identical");
}

#[test]
fn keys_enroll_does_not_commit_or_stage_to_git() {
    let home = tempfile::tempdir().expect("create keystore home");
    let home_str = home.path().to_str().unwrap();
    let _ = shore_env(
        ["keys", "init", "--name", "default"],
        &[("SHORE_HOME", home_str)],
    );
    let repo = support::git_repo::GitRepo::new();
    let _ = shore_env(
        ["keys", "enroll", "--repo", repo.path().to_str().unwrap()],
        &[
            ("SHORE_HOME", home_str),
            ("SHORE_ACTOR_ID", "actor:agent:claude-code"),
        ],
    );

    // The staged file is a pending working-tree change, never a commit.
    let status = Command::new("git")
        .args(["status", "--porcelain", "-uall"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&status.stdout);
    assert!(
        out.contains(".shore/allowed-signers.json"),
        "the enrolled file is a pending working-tree change: {out}"
    );
    let log = Command::new("git")
        .args(["rev-list", "--count", "--all"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&log.stdout).trim(),
        "0",
        "enroll never commits"
    );
}

#[test]
fn keys_enroll_explicit_actor_flag_overrides_resolution() {
    let home = tempfile::tempdir().expect("create keystore home");
    let home_str = home.path().to_str().unwrap();
    let _ = shore_env(
        ["keys", "init", "--name", "default"],
        &[("SHORE_HOME", home_str)],
    );
    let repo = support::git_repo::GitRepo::new();

    let out = shore_env(
        [
            "keys",
            "enroll",
            "--repo",
            repo.path().to_str().unwrap(),
            "--actor",
            "actor:agent:explicit-override",
        ],
        &[
            ("SHORE_HOME", home_str),
            ("SHORE_ACTOR_ID", "actor:git-email:resolved@example.com"),
        ],
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["actorId"], "actor:agent:explicit-override");
}

#[test]
fn keys_enroll_from_subdirectory_writes_to_worktree_root() {
    let home = tempfile::tempdir().expect("create keystore home");
    let home_str = home.path().to_str().unwrap();
    let _ = shore_env(
        ["keys", "init", "--name", "default"],
        &[("SHORE_HOME", home_str)],
    );

    let repo = support::git_repo::GitRepo::new();
    let subdir = repo.path().join("nested/dir");
    std::fs::create_dir_all(&subdir).unwrap();

    let out = shore_env(
        ["keys", "enroll", "--repo", subdir.to_str().unwrap()],
        &[
            ("SHORE_HOME", home_str),
            ("SHORE_ACTOR_ID", "actor:agent:claude-code"),
        ],
    );
    assert!(
        out.status.success(),
        "enroll stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Enrollment must land at the worktree root, where trust discovery reads it —
    // not under the subdirectory the command was pointed at.
    assert!(
        repo.path().join(".shore/allowed-signers.json").exists(),
        "enrolled at the worktree root"
    );
    assert!(
        !subdir.join(".shore/allowed-signers.json").exists(),
        "not written under the subdirectory (would be invisible to discovery)"
    );

    // keys list from the subdir now sees the key as enrolled (single discovery path).
    let init = shore_env(
        ["keys", "show", "default", "--did"],
        &[("SHORE_HOME", home_str)],
    );
    let did = serde_json::from_slice::<Value>(&init.stdout).unwrap()["didKey"]
        .as_str()
        .unwrap()
        .to_owned();
    let list = shore_env(
        ["keys", "list", "--repo", subdir.to_str().unwrap()],
        &[("SHORE_HOME", home_str)],
    );
    let listed: Value = serde_json::from_slice(&list.stdout).unwrap();
    let default = listed["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["didKey"] == did)
        .unwrap();
    assert_eq!(
        default["enrolled"], true,
        "enrollment is visible from the subdir"
    );
}

#[test]
fn keys_show_rejects_path_traversal_name_even_when_target_exists() {
    let home = tempfile::tempdir().expect("create keystore home");
    let home_str = home.path().to_str().unwrap();
    // Plant a valid key file as a sibling of keys/, reachable only via traversal.
    let init = shore_env(
        ["keys", "init", "--name", "planted"],
        &[("SHORE_HOME", home_str)],
    );
    assert!(init.status.success());
    std::fs::copy(
        home.path().join("keys/planted"),
        home.path().join("outside-key"),
    )
    .unwrap();

    let out = shore_env(
        ["keys", "show", "../outside-key", "--did"],
        &[("SHORE_HOME", home_str)],
    );
    assert!(
        !out.status.success(),
        "a path-traversal key name must be rejected even when the target file exists"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("panicked"),
        "clean error, not a panic: {stderr}"
    );
}

// A real `ssh-keygen -t ed25519`-produced public key (the same key Task 1.1's
// parser pins) so the did:key is stable across the parser and this command.
const SSH_ED25519_PUBKEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAID7lnwK7O5CFXew1hBuUnXz1+zK2pQtYEtxsbRMiOyvP dev@example";
// A real `ssh-keygen -t rsa` public key — used to prove the non-ed25519 rejection.
const SSH_RSA_PUBKEY: &str = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQDIruRAxOrjtLtG0Rl4Ez7e0JmAuFFda/QvUwLWt6JucZlgRRfnJDfneTAzDzxQGpB+ok1ff8DovRHozcdn9nXO4bXZgx/8zb0bTqhm0y7Zn2qulvZ8lEBiUuJNRiBjy9pEcPxYYBuMP0dphQzPzSmNVeJvDO00cSvmEgeAdSUPAzIexM9ME3HTSXvt9CsV1QMCo8x/GwnEeJZHCkb2wWEs1oxv9EPrqp2y+dkAB+LFDcoeNMdHBeLzQh3w9pm2WaQsn9KGc6gK4edCeFn7ymcZ8GgNkmAJka4XxRcD+Fg7+3+r98ABtfSdvLuv/ddAQzZjruMP5Z0444anG3qsOtKf test@host";

#[test]
fn keys_use_ssh_from_pubkey_path_writes_reference_and_emits_did_key() {
    let home = tempfile::tempdir().expect("create keystore home");
    let pubdir = tempfile::tempdir().expect("create pubkey dir");
    let pubfile = pubdir.path().join("id_ed25519.pub");
    std::fs::write(&pubfile, SSH_ED25519_PUBKEY).unwrap();

    let out = shore_env(
        ["keys", "use-ssh", pubfile.to_str().unwrap()],
        &[("SHORE_HOME", home.path().to_str().unwrap())],
    );
    assert!(
        out.status.success(),
        "use-ssh stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let json: Value = serde_json::from_slice(&out.stdout).expect("stdout is json");
    assert_eq!(json["schema"], "shore.keys-use-ssh");
    assert_eq!(json["name"], "default"); // default --name
    assert!(
        json["didKey"].as_str().unwrap().starts_with("did:key:z6Mk"),
        "did:key present: {json:#}"
    );
    let path = json["path"].as_str().expect("path field");
    assert!(
        std::path::Path::new(path).exists(),
        "reference file exists at {path}"
    );
}

#[test]
fn keys_use_ssh_accepts_a_key_literal() {
    let home = tempfile::tempdir().expect("create keystore home");
    let literal = format!("key::{SSH_ED25519_PUBKEY}");
    let out = shore_env(
        ["keys", "use-ssh", &literal],
        &[("SHORE_HOME", home.path().to_str().unwrap())],
    );
    assert!(
        out.status.success(),
        "use-ssh stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["schema"], "shore.keys-use-ssh");
    assert!(json["didKey"].as_str().unwrap().starts_with("did:key:z6Mk"));
}

#[test]
fn keys_use_ssh_path_and_literal_derive_the_same_did_key() {
    let home = tempfile::tempdir().expect("create keystore home");
    let pubdir = tempfile::tempdir().expect("create pubkey dir");
    let pubfile = pubdir.path().join("id_ed25519.pub");
    std::fs::write(&pubfile, SSH_ED25519_PUBKEY).unwrap();

    let from_path = shore_env(
        [
            "keys",
            "use-ssh",
            "--name",
            "viapath",
            pubfile.to_str().unwrap(),
        ],
        &[("SHORE_HOME", home.path().to_str().unwrap())],
    );
    let literal = format!("key::{SSH_ED25519_PUBKEY}");
    let from_literal = shore_env(
        ["keys", "use-ssh", "--name", "vialiteral", &literal],
        &[("SHORE_HOME", home.path().to_str().unwrap())],
    );
    let a: Value = serde_json::from_slice(&from_path.stdout).unwrap();
    let b: Value = serde_json::from_slice(&from_literal.stdout).unwrap();
    assert_eq!(
        a["didKey"], b["didKey"],
        "same key, same did:key whichever input form"
    );
}

#[test]
fn keys_use_ssh_writes_a_did_key_sidecar() {
    let home = tempfile::tempdir().expect("create keystore home");
    let literal = format!("key::{SSH_ED25519_PUBKEY}");
    let out = shore_env(
        ["keys", "use-ssh", &literal],
        &[("SHORE_HOME", home.path().to_str().unwrap())],
    );
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    let did_key = json["didKey"].as_str().unwrap();
    let reference = std::path::Path::new(json["path"].as_str().unwrap());
    let sidecar = reference.with_file_name("default.pub");
    let recorded = std::fs::read_to_string(&sidecar).unwrap();
    assert_eq!(recorded.trim(), did_key, ".pub sidecar records the did:key");
}

#[test]
fn keys_use_ssh_rejects_a_non_ed25519_key_with_a_clear_error() {
    let home = tempfile::tempdir().expect("create keystore home");
    let literal = format!("key::{SSH_RSA_PUBKEY}");
    let out = shore_env(
        ["keys", "use-ssh", &literal],
        &[("SHORE_HOME", home.path().to_str().unwrap())],
    );
    assert!(!out.status.success(), "an ssh-rsa key must be rejected");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("panicked"),
        "clean error, not a panic: {stderr}"
    );
    assert!(!stderr.is_empty(), "an error message is printed");
    assert!(!home.path().join("keys/default").exists());
}

#[test]
fn keys_use_ssh_collision_refuses_to_overwrite() {
    let home = tempfile::tempdir().expect("create keystore home");
    let env = [("SHORE_HOME", home.path().to_str().unwrap())];
    let literal = format!("key::{SSH_ED25519_PUBKEY}");
    let first = shore_env(["keys", "use-ssh", &literal], &env);
    assert!(first.status.success());

    let second = shore_env(["keys", "use-ssh", &literal], &env);
    assert!(
        !second.status.success(),
        "a --name collision must refuse to overwrite"
    );
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        !stderr.contains("panicked"),
        "clean error, not a panic: {stderr}"
    );
}

#[test]
fn keys_use_ssh_rejects_a_path_unsafe_name() {
    let home = tempfile::tempdir().expect("create keystore home");
    let literal = format!("key::{SSH_ED25519_PUBKEY}");
    let out = shore_env(
        ["keys", "use-ssh", "--name", "../../id_ed25519", &literal],
        &[("SHORE_HOME", home.path().to_str().unwrap())],
    );
    assert!(
        !out.status.success(),
        "a path-unsafe key name must be rejected"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("panicked"),
        "clean error, not a panic: {stderr}"
    );
}
