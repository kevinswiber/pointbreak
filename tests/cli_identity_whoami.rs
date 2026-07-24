mod support;

use std::process::{Command, Output};

use support::git_repo::GitRepo;

fn whoami_command(repo: &GitRepo) -> Command {
    let mut command = Command::new(support::pointbreak_bin());
    command
        .args(["identity", "whoami", "--repo"])
        .arg(repo.path())
        .env_remove("POINTBREAK_ACTOR_ID")
        .env_remove("POINTBREAK_FORMAT")
        .env_remove("POINTBREAK_LOG")
        .env_remove("RUST_LOG")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null");
    command
}

fn output(command: &mut Command) -> Output {
    command.output().expect("run pointbreak identity whoami")
}

fn actor_id(output: &Output) -> String {
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .expect("whoami JSON")
        .get("actorId")
        .and_then(serde_json::Value::as_str)
        .expect("actorId string")
        .to_owned()
}

#[test]
fn identity_whoami_json_is_the_exact_v1_contract() {
    let repo = GitRepo::new();
    let output = output(&mut whoami_command(&repo));

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        concat!(
            "{\"schema\":\"pointbreak.identity-whoami\",\"version\":1,",
            "\"actorId\":\"actor:git-email:shore-tests@example.com\"}\n"
        )
    );
}

#[test]
fn identity_whoami_honors_an_inherited_actor() {
    let repo = GitRepo::new();
    let mut command = whoami_command(&repo);
    command.env("POINTBREAK_ACTOR_ID", "actor:agent:inherited");

    assert_eq!(actor_id(&output(&mut command)), "actor:agent:inherited");
}

#[test]
fn identity_whoami_falls_back_through_git_name_and_local() {
    let name_repo = GitRepo::new();
    name_repo.git(["config", "user.email", ""]);
    name_repo.git(["config", "user.name", "Local Reviewer"]);
    assert_eq!(
        actor_id(&output(&mut whoami_command(&name_repo))),
        "actor:git-name:Local Reviewer"
    );

    let local_repo = GitRepo::new();
    local_repo.git(["config", "user.email", ""]);
    local_repo.git(["config", "user.name", ""]);
    assert_eq!(
        actor_id(&output(&mut whoami_command(&local_repo))),
        "actor:local"
    );
}

#[test]
fn identity_whoami_sanitized_invocation_uses_git_email() {
    let repo = GitRepo::new();
    let mut command = whoami_command(&repo);
    command.env("POINTBREAK_ACTOR_ID", "actor:agent:ambient");
    command.env_remove("POINTBREAK_ACTOR_ID");

    assert_eq!(
        actor_id(&output(&mut command)),
        "actor:git-email:shore-tests@example.com"
    );
}

#[test]
fn identity_whoami_text_is_human_readable_and_has_no_actor_override() {
    let repo = GitRepo::new();
    let mut text = whoami_command(&repo);
    text.args(["--format", "text"]);
    let text = output(&mut text);
    assert!(text.status.success());
    assert_eq!(
        String::from_utf8(text.stdout).unwrap(),
        "actor:git-email:shore-tests@example.com\n"
    );

    let mut override_attempt = whoami_command(&repo);
    override_attempt.args(["--actor-id", "actor:agent:spoofed"]);
    let override_attempt = output(&mut override_attempt);
    assert!(!override_attempt.status.success());
    assert!(String::from_utf8_lossy(&override_attempt.stderr).contains("--actor-id"));
}

#[test]
fn version_registry_advertises_identity_whoami_v1() {
    let output = support::pointbreak(["version"]);
    assert!(output.status.success());
    let version: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(version["documents"]["pointbreak.identity-whoami"], 1);
}
