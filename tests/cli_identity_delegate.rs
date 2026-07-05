mod support;
use serde_json::Value;
use support::git_repo::GitRepo;
use support::{shore, shore_env};

#[test]
fn identity_help_lists_the_group() {
    let out = shore(["identity", "--help"]);
    assert!(
        out.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(
        help.contains("delegate"),
        "identity help lists delegate: {help}"
    );
    assert!(
        help.contains("attest"),
        "identity help lists attest: {help}"
    );
}

#[test]
fn delegate_stages_delegates_file_and_reader_resolves_principal() {
    let repo = GitRepo::new();
    let out = shore_env(
        [
            "identity",
            "delegate",
            "actor:agent:claude-code",
            "--principal",
            "actor:git-email:kevin@swiber.dev",
            "--from",
            "2026-06-10T00:00:00Z",
            "--repo",
            repo.path().to_str().unwrap(),
        ],
        &[],
    );
    assert!(
        out.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["schema"], "shore.identity-enroll");
    assert_eq!(doc["agent"], "actor:agent:claude-code");
    assert_eq!(doc["principal"], "actor:git-email:kevin@swiber.dev");
    assert_eq!(doc["added"], true);

    // Read back through the PUBLIC reader (INV-F).
    let path = repo.path().join(".shore/delegates.json");
    let map = shoreline::session::DelegationMap::from_delegates_file(&path).unwrap();
    assert_eq!(
        map.resolve(
            &shoreline::model::ActorId::new("actor:agent:claude-code"),
            "2026-06-11T00:00:00Z"
        ),
        shoreline::session::PrincipalResolution::Resolved(shoreline::model::ActorId::new(
            "actor:git-email:kevin@swiber.dev"
        ))
    );
}

#[test]
fn delegate_defaults_from_to_now_rfc3339() {
    let repo = GitRepo::new();
    let out = shore_env(
        [
            "identity",
            "delegate",
            "actor:agent:claude-code",
            "--principal",
            "actor:git-email:kevin@swiber.dev",
            "--repo",
            repo.path().to_str().unwrap(),
        ],
        &[],
    );
    assert!(out.status.success());
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    let from = doc["validFrom"].as_str().unwrap();
    assert!(
        from.ends_with('Z') && from.contains('T'),
        "RFC 3339 default-now: {from}"
    );
    // The staged record re-reads (never a unix-ms: form, INV-C).
    assert!(
        shoreline::session::DelegationMap::from_delegates_file(
            repo.path().join(".shore/delegates.json")
        )
        .is_ok()
    );
}

#[test]
fn delegate_rejects_agent_principal_depth0() {
    let repo = GitRepo::new();
    let out = shore_env(
        [
            "identity",
            "delegate",
            "actor:agent:claude-code",
            "--principal",
            "actor:agent:subagent",
            "--repo",
            repo.path().to_str().unwrap(),
        ],
        &[],
    );
    assert!(
        !out.status.success(),
        "agent-scheme principal must be rejected (depth-0)"
    );
    assert!(!String::from_utf8_lossy(&out.stderr).contains("panicked"));
    assert!(!repo.path().join(".shore/delegates.json").exists());
}

#[test]
fn delegate_local_writes_override_excludes_it_and_surfaces_full_replace_caveat() {
    let repo = GitRepo::new();
    // A committed record first.
    let _ = shore_env(
        [
            "identity",
            "delegate",
            "actor:agent:claude-code",
            "--principal",
            "actor:git-email:kevin@swiber.dev",
            "--repo",
            repo.path().to_str().unwrap(),
        ],
        &[],
    );
    // Then a local override for the same agent.
    let out = shore_env(
        [
            "identity",
            "delegate",
            "actor:agent:claude-code",
            "--principal",
            "actor:git-email:alice@example.com",
            "--local",
            "--repo",
            repo.path().to_str().unwrap(),
        ],
        &[],
    );
    assert!(
        out.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The local file exists and is git-excluded via the generated .shore/.gitignore.
    assert!(repo.path().join(".shore/delegates.local.json").exists());
    let ignore = std::fs::read_to_string(repo.path().join(".shore/.gitignore")).unwrap();
    assert!(ignore.lines().any(|l| l.trim() == "*.local.json"));
    let check = std::process::Command::new("git")
        .args(["check-ignore", ".shore/delegates.local.json"])
        .current_dir(repo.path())
        .status()
        .unwrap();
    assert!(check.success(), "the local override must be git-ignored");
    // The full-replace caveat is surfaced (committed record(s) shadowed locally) — INV-E.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.to_lowercase().contains("replace") || stderr.to_lowercase().contains("shadow"),
        "must surface the full-replace caveat: {stderr}"
    );
}

#[test]
fn delegate_never_commits() {
    let repo = GitRepo::new();
    let _ = shore_env(
        [
            "identity",
            "delegate",
            "actor:agent:claude-code",
            "--principal",
            "actor:git-email:kevin@swiber.dev",
            "--repo",
            repo.path().to_str().unwrap(),
        ],
        &[],
    );
    let log = std::process::Command::new("git")
        .args(["rev-list", "--count", "--all"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&log.stdout).trim(),
        "0",
        "delegate never commits"
    );
}

#[test]
fn identity_enroll_is_unregistered() {
    let out = shore(["identity", "enroll", "--help"]);
    assert!(!out.status.success(), "identity enroll should be retired");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("unrecognized subcommand"),
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
