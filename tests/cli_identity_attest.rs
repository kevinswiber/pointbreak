mod support;
use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore_env;

#[test]
fn attest_stages_attributes_and_reader_resolves_kind_and_roles() {
    let repo = GitRepo::new();
    let out = shore_env(
        [
            "identity",
            "attest",
            "actor:git-email:kevin@swiber.dev",
            "--kind",
            "human",
            "--role",
            "author",
            "--role",
            "reviewer",
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
    assert_eq!(doc["schema"], "shore.identity-attest");
    assert_eq!(doc["actor"], "actor:git-email:kevin@swiber.dev");
    assert_eq!(doc["kind"], "human");
    assert_eq!(doc["changed"], true);

    // Read back through the PUBLIC reader (INV-F).
    let path = repo.path().join(".shore/actor-attributes.json");
    let map = shoreline::session::ActorAttributesMap::from_attributes_file(&path).unwrap();
    let resolved = map.resolve(&shoreline::model::ActorId::new(
        "actor:git-email:kevin@swiber.dev",
    ));
    assert_eq!(resolved.kind(), Some("human"));
    assert!(resolved.has_role("author") && resolved.has_role("reviewer"));
}

#[test]
fn attest_normalizes_tokens() {
    let repo = GitRepo::new();
    let out = shore_env(
        [
            "identity",
            "attest",
            "actor:agent:review-bot",
            "--kind",
            "Reviewer-Model",
            "--role",
            "Reviewer",
            "--role",
            "reviewer",
            "--repo",
            repo.path().to_str().unwrap(),
        ],
        &[],
    );
    assert!(out.status.success());
    let map = shoreline::session::ActorAttributesMap::from_attributes_file(
        repo.path().join(".shore/actor-attributes.json"),
    )
    .unwrap();
    let r = map.resolve(&shoreline::model::ActorId::new("actor:agent:review-bot"));
    assert_eq!(r.kind(), Some("reviewer-model"));
    assert_eq!(
        r.roles().iter().cloned().collect::<Vec<_>>(),
        vec!["reviewer"]
    ); // deduped+sorted
}

#[test]
fn attest_requires_kind() {
    let repo = GitRepo::new();
    let out = shore_env(
        [
            "identity",
            "attest",
            "actor:agent:x",
            "--repo",
            repo.path().to_str().unwrap(),
        ],
        &[],
    );
    assert!(
        !out.status.success(),
        "--kind is required (ADR-0012: exactly one kind per actor)"
    );
}

#[test]
fn attest_rejects_bad_role_token_and_writes_nothing() {
    let repo = GitRepo::new();
    let out = shore_env(
        [
            "identity",
            "attest",
            "actor:agent:x",
            "--kind",
            "agent",
            "--role",
            "Has Space",
            "--repo",
            repo.path().to_str().unwrap(),
        ],
        &[],
    );
    assert!(!out.status.success());
    assert!(!String::from_utf8_lossy(&out.stderr).contains("panicked"));
    assert!(!repo.path().join(".shore/actor-attributes.json").exists());
}

#[test]
fn attest_local_writes_override_excludes_it_and_surfaces_full_replace_caveat() {
    let repo = GitRepo::new();
    let out = shore_env(
        [
            "identity",
            "attest",
            "actor:git-email:kevin@swiber.dev",
            "--kind",
            "human",
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
    assert!(
        repo.path()
            .join(".shore/actor-attributes.local.json")
            .exists()
    );
    // The override is git-excluded via the generated .shore/.gitignore.
    let ignore = std::fs::read_to_string(repo.path().join(".shore/.gitignore")).unwrap();
    assert!(ignore.lines().any(|l| l.trim() == "*.local.json"));
    let check = std::process::Command::new("git")
        .args(["check-ignore", ".shore/actor-attributes.local.json"])
        .current_dir(repo.path())
        .status()
        .unwrap();
    assert!(check.success(), "the local override must be git-ignored");
    // INV-E: the git-config full-replace semantics must be surfaced (a local entry
    // fully replaces the committed entry for this actor locally — never a merge).
    let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        stderr.contains("replace") || stderr.contains("shadow"),
        "must surface the full-replace caveat: {stderr}"
    );
}

#[test]
fn attest_never_commits() {
    let repo = GitRepo::new();
    let _ = shore_env(
        [
            "identity",
            "attest",
            "actor:agent:x",
            "--kind",
            "agent",
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
    assert_eq!(String::from_utf8_lossy(&log.stdout).trim(), "0");
}
