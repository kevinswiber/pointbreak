mod support;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::{shore, shore_env};

fn parse_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("stdout is valid JSON")
}

fn modified_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo
}

/// Capture a review and add an observation written under
/// `actor:agent:claude-code`, returning the repo.
fn repo_with_agent_observation() -> GitRepo {
    let repo = modified_repo();
    let path = repo.path().to_str().unwrap().to_owned();
    let capture = parse_json(&shore(["capture", "--repo", &path]).stdout);
    let revision_id = capture["revision"]["id"].as_str().unwrap().to_owned();
    let out = shore_env(
        [
            "review",
            "observation",
            "add",
            "--repo",
            &path,
            "--revision",
            &revision_id,
            "--track",
            "agent:claude-code",
            "--title",
            "Agent observation",
        ],
        &[("SHORE_ACTOR_ID", "actor:agent:claude-code")],
    );
    assert!(
        out.status.success(),
        "observation add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    repo
}

fn write_delegates(repo: &GitRepo, contents: &str) {
    repo.write(".shore/delegates.json", contents);
}

fn write_local_delegates(repo: &GitRepo, contents: &str) {
    repo.write(".shore/delegates.local.json", contents);
}

const LOCAL_OVERRIDE_DELEGATES: &str = r#"{
  "delegates": {
    "actor:agent:claude-code": [
      {
        "principal": "actor:git-email:alice@example.com",
        "validFrom": "2020-01-01T00:00:00Z",
        "validUntil": null
      }
    ]
  }
}
"#;

const RESOLVING_DELEGATES: &str = r#"{
  "delegates": {
    "actor:agent:claude-code": [
      {
        "principal": "actor:git-email:kevin@swiber.dev",
        "validFrom": "2020-01-01T00:00:00Z",
        "validUntil": null
      }
    ]
  }
}
"#;

fn agent_history_entry(json: &Value) -> &Value {
    json["entries"]
        .as_array()
        .expect("entries array")
        .iter()
        .find(|entry| entry["writer"]["actorId"] == "actor:agent:claude-code")
        .expect("an entry written by the agent")
}

#[test]
fn cli_history_resolves_principal_from_checked_in_delegates_file() {
    let repo = repo_with_agent_observation();
    write_delegates(&repo, RESOLVING_DELEGATES);
    let path = repo.path().to_str().unwrap();

    let output = shore(["review", "history", "--repo", path]);
    assert!(output.status.success());
    let json = parse_json(&output.stdout);

    let entry = agent_history_entry(&json);
    assert_eq!(
        entry["principal"]["actorId"],
        "actor:git-email:kevin@swiber.dev"
    );
    assert_eq!(entry["principal"]["status"], "resolved");
    assert_eq!(entry["principal"]["source"], "delegates");
}

#[test]
fn cli_local_override_replaces_committed_principal_for_the_agent() {
    let repo = repo_with_agent_observation();
    write_delegates(&repo, RESOLVING_DELEGATES); // committed: claude-code -> kevin
    write_local_delegates(&repo, LOCAL_OVERRIDE_DELEGATES); // local: claude-code -> alice
    let path = repo.path().to_str().unwrap();

    let output = shore(["review", "history", "--repo", path]);
    assert!(output.status.success());
    let json = parse_json(&output.stdout);

    let entry = agent_history_entry(&json);
    assert_eq!(
        entry["principal"]["actorId"], "actor:git-email:alice@example.com",
        "the local override fully replaces the committed records for the agent"
    );
    assert_eq!(entry["principal"]["status"], "resolved");
    assert_eq!(entry["principal"]["source"], "delegates");
}

#[test]
fn cli_resolves_principal_when_repo_points_at_a_subdirectory() {
    // Read commands accept a path inside the repository; `.shore/delegates.json`
    // lives at the worktree root, so discovery must resolve the root rather than
    // join the raw `--repo` argument.
    let repo = repo_with_agent_observation();
    write_delegates(&repo, RESOLVING_DELEGATES);
    let subdir = repo.path().join("src");
    assert!(subdir.is_dir(), "the fixture writes src/lib.rs");

    let output = shore(["review", "history", "--repo", subdir.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);

    let entry = agent_history_entry(&json);
    assert_eq!(
        entry["principal"]["actorId"], "actor:git-email:kevin@swiber.dev",
        "principal resolves even when --repo is a subdirectory"
    );
    assert_eq!(entry["principal"]["status"], "resolved");
}

#[test]
fn cli_warns_and_proceeds_on_malformed_delegates_file() {
    let repo = repo_with_agent_observation();
    write_delegates(&repo, "{ not valid json");
    let path = repo.path().to_str().unwrap();

    let output = shore(["review", "history", "--repo", path]);
    assert!(
        output.status.success(),
        "a malformed delegates file must not block a read"
    );
    let json = parse_json(&output.stdout);

    // Document intact; agent entry degrades to the mirror posture (no resolution).
    let entry = agent_history_entry(&json);
    assert_eq!(entry["principal"]["status"], "none");
    assert!(entry["principal"].get("actorId").is_none());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(".shore/delegates.json"),
        "stderr names the offending file; got: {stderr}"
    );
}

#[test]
fn cli_without_delegates_file_degrades_agent_writers_to_none() {
    let repo = repo_with_agent_observation();
    let path = repo.path().to_str().unwrap();

    let output = shore(["review", "history", "--repo", path]);
    assert!(output.status.success());
    let json = parse_json(&output.stdout);

    let entry = agent_history_entry(&json);
    assert_eq!(entry["principal"]["status"], "none");
    assert_eq!(entry["principal"]["source"], "none");
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}
