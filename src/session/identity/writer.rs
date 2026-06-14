use std::path::Path;
use std::process::Command;

use crate::crypto::SignerId;
use crate::model::ActorId;
use crate::session::event::{Writer, WriterProducer};

/// Environment variable that pins the writing actor to an explicit, fully
/// qualified `actor:<scheme>:<id>` identity, taking precedence over the local
/// Git identity. Intended for callers that drive `shore` on behalf of a known
/// actor — for example a federation bridge forwarding a remote reviewer's
/// decision, where the local Git identity would otherwise mis-attribute the
/// durable write to the host running the command.
pub(crate) const SHORE_ACTOR_ID_ENV: &str = "SHORE_ACTOR_ID";

pub(crate) fn writer_from_git_config(repo: &Path) -> Writer {
    writer_from_options(repo, None)
}

/// Build the local `Writer`, honoring an optional per-call actor override.
///
/// Precedence: an explicit override wins, then the `SHORE_ACTOR_ID` env var,
/// then the local Git identity. A malformed override (or env value) is ignored
/// and falls through to the next source, so a bad value can never silently
/// corrupt provenance. `None` reproduces the prior env-then-Git behavior
/// exactly.
pub(crate) fn writer_from_options(repo: &Path, explicit: Option<&ActorId>) -> Writer {
    Writer {
        actor_id: actor_id_for_repo(explicit.map(ActorId::as_str), repo),
        producer: shore_producer(),
    }
}

/// Resolve the writing actor for `repo`, reading `SHORE_ACTOR_ID` as the
/// process-level default beneath an optional explicit override.
fn actor_id_for_repo(explicit: Option<&str>, repo: &Path) -> ActorId {
    resolve_actor_id(
        explicit,
        std::env::var(SHORE_ACTOR_ID_ENV).ok().as_deref(),
        repo,
    )
}

/// Pure resolution seam (kept env-free for testing): the first of `explicit`
/// then `env` that is a valid fully-qualified actor id wins; otherwise derive
/// from Git config. Each candidate is validated independently, so a malformed
/// override falls through to the env value (then Git) rather than being trusted.
fn resolve_actor_id(explicit: Option<&str>, env: Option<&str>, repo: &Path) -> ActorId {
    for value in [explicit, env].into_iter().flatten() {
        let value = value.trim();
        if is_valid_actor_id(value) {
            return ActorId::new(value.to_owned());
        }
    }
    actor_id_from_git_config(repo)
}

/// A safe, fully-qualified actor id: either an `actor:` prefix with a non-empty
/// remainder, bounded length, and no whitespace or control characters, or a
/// syntactically valid Ed25519 `did:key`. An invalid value is ignored rather
/// than trusted, so a malformed override can never silently corrupt provenance.
pub(crate) fn is_valid_actor_id(value: &str) -> bool {
    value.len() <= 256 && {
        value.strip_prefix("actor:").is_some_and(|rest| {
            !rest.is_empty() && rest.chars().all(|c| !c.is_whitespace() && !c.is_control())
        }) || SignerId::parse(value).is_ok()
    }
}

/// Validity for a delegation *principal* actor id. Unlike `is_valid_actor_id`
/// — which gates an env/override value and forbids whitespace — a principal is a
/// human writer id that the system itself mints with internal spaces (for
/// example `actor:git-name:Kevin Swiber`, derived from `git config user.name`).
/// So this allows internal whitespace but still forbids control characters,
/// requires the `actor:` scheme with a non-empty remainder, or accepts a
/// syntactically valid Ed25519 `did:key`.
pub(crate) fn is_valid_principal_actor_id(value: &str) -> bool {
    value.len() <= 256 && {
        value
            .strip_prefix("actor:")
            .is_some_and(|rest| !rest.trim().is_empty() && rest.chars().all(|c| !c.is_control()))
            || SignerId::parse(value).is_ok()
    }
}

/// True when `value` is an `actor:agent:<name>` identity with a non-empty agent
/// segment. The agent scheme names acting software whose durable writes resolve
/// to a human principal through the delegation map; non-agent actors (git
/// identities, `did:key`s) are their own principal and carry no delegation
/// record. This is a scheme test only — it does not validate the rest of the id
/// (`is_valid_actor_id` does that).
pub(crate) fn is_agent_actor_id(value: &str) -> bool {
    value
        .strip_prefix("actor:agent:")
        .is_some_and(|rest| !rest.is_empty())
}

fn shore_producer() -> WriterProducer {
    WriterProducer {
        name: "shore".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    }
}

fn actor_id_from_git_config(repo: &Path) -> ActorId {
    git_config_value(repo, "user.email")
        .map(|email| ActorId::new(format!("actor:git-email:{email}")))
        .or_else(|| {
            git_config_value(repo, "user.name")
                .map(|name| ActorId::new(format!("actor:git-name:{name}")))
        })
        // V1 local workflows treat missing Git identity as one local actor.
        .unwrap_or_else(|| ActorId::new("actor:local"))
}

fn git_config_value(repo: &Path, key: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["config", "--get", key])
        .current_dir(repo)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    #[test]
    fn writer_from_git_config_uses_git_identity_and_shore_producer() {
        let repo = tempfile::tempdir().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(repo.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "author@example.com"])
            .current_dir(repo.path())
            .output()
            .unwrap();

        let writer = super::writer_from_git_config(repo.path());

        assert_eq!(
            writer.actor_id.as_str(),
            "actor:git-email:author@example.com"
        );
        assert_eq!(writer.producer.name, "shore");
    }

    #[test]
    fn writer_from_options_uses_email_then_name_then_actor_local() {
        let email_repo = tempfile::tempdir().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(email_repo.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "reviewer@example.com"])
            .current_dir(email_repo.path())
            .output()
            .unwrap();
        let email_writer = super::writer_from_options(email_repo.path(), None);
        assert_eq!(
            email_writer.actor_id.as_str(),
            "actor:git-email:reviewer@example.com"
        );

        let name_repo = tempfile::tempdir().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(name_repo.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", ""])
            .current_dir(name_repo.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "reviewer-name"])
            .current_dir(name_repo.path())
            .output()
            .unwrap();
        let name_writer = super::writer_from_options(name_repo.path(), None);
        assert_eq!(
            name_writer.actor_id.as_str(),
            "actor:git-name:reviewer-name"
        );

        let local_repo = tempfile::tempdir().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(local_repo.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", ""])
            .current_dir(local_repo.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", ""])
            .current_dir(local_repo.path())
            .output()
            .unwrap();
        let local_writer = super::writer_from_options(local_repo.path(), None);
        assert_eq!(local_writer.actor_id.as_str(), "actor:local");
    }

    fn git_repo_with_email(email: &str) -> tempfile::TempDir {
        let repo = tempfile::tempdir().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(repo.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", email])
            .current_dir(repo.path())
            .output()
            .unwrap();
        repo
    }

    #[test]
    fn explicit_actor_id_overrides_env_and_git_identity() {
        let repo = git_repo_with_email("host@example.com");
        let actor = super::resolve_actor_id(
            Some("actor:agent:remote-reviewer"),
            Some("actor:env:from-env"),
            repo.path(),
        );
        assert_eq!(actor.as_str(), "actor:agent:remote-reviewer");
    }

    #[test]
    fn env_actor_id_overrides_git_identity_when_no_explicit() {
        let repo = git_repo_with_email("host@example.com");
        let actor = super::resolve_actor_id(None, Some("actor:env:from-env"), repo.path());
        assert_eq!(actor.as_str(), "actor:env:from-env");
    }

    #[test]
    fn invalid_explicit_actor_id_falls_through_to_valid_env() {
        let repo = git_repo_with_email("host@example.com");
        let actor = super::resolve_actor_id(
            Some("not-an-actor"),
            Some("actor:env:from-env"),
            repo.path(),
        );
        assert_eq!(
            actor.as_str(),
            "actor:env:from-env",
            "a malformed explicit override must fall through to the valid env value"
        );
    }

    #[test]
    fn invalid_explicit_and_env_actor_ids_fall_back_to_git_identity() {
        let repo = git_repo_with_email("host@example.com");
        for bad in [
            "",
            "no-prefix",
            "actor:",
            "actor:has space",
            "actor:line\nbreak",
        ] {
            let actor = super::resolve_actor_id(Some(bad), Some("also bad"), repo.path());
            assert_eq!(
                actor.as_str(),
                "actor:git-email:host@example.com",
                "invalid override {bad:?} and invalid env should fall back to the Git identity"
            );
        }
    }

    #[test]
    fn missing_explicit_and_env_actor_id_uses_git_identity() {
        let repo = git_repo_with_email("host@example.com");
        let actor = super::resolve_actor_id(None, None, repo.path());
        assert_eq!(actor.as_str(), "actor:git-email:host@example.com");
    }

    #[test]
    fn is_agent_actor_id_matches_agent_scheme_only() {
        assert!(super::is_agent_actor_id("actor:agent:claude-code"));
        assert!(!super::is_agent_actor_id(
            "actor:git-email:kevin@swiber.dev"
        ));
        assert!(!super::is_agent_actor_id(
            "did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd"
        ));
        // A longer scheme that merely starts with "agent" is not agent-scheme.
        assert!(!super::is_agent_actor_id("actor:agentx:foo"));
        // The agent segment must be non-empty.
        assert!(!super::is_agent_actor_id("actor:agent:"));
    }

    #[test]
    fn actor_validation_accepts_actor_or_did_key_but_does_not_alias_them() {
        let did_key = "did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd";

        assert!(super::is_valid_actor_id(
            "actor:git-email:alice@example.com"
        ));
        assert!(super::is_valid_actor_id(did_key));
        assert_ne!(
            crate::model::ActorId::new(did_key),
            crate::model::ActorId::new("actor:git-email:alice@example.com")
        );
    }
}
