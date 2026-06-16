mod support;

use serde_json::Value;
use support::shore_env;

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
