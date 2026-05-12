use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::model::ActorId;
use crate::session::event::{Writer, WriterRole, WriterTool};

pub(crate) fn writer_from_git_config(repo: &Path) -> Writer {
    Writer {
        actor_id: actor_id_from_git_config(repo),
        role: WriterRole::Author,
        tool: shore_tool(),
    }
}

pub(crate) fn reviewer_from_git_config(repo: &Path) -> Writer {
    Writer {
        actor_id: actor_id_from_git_config(repo),
        role: WriterRole::Reviewer,
        tool: shore_tool(),
    }
}

fn shore_tool() -> WriterTool {
    WriterTool {
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

pub(crate) fn current_timestamp() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("unix-ms:{millis}")
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    #[test]
    fn reviewer_from_git_config_uses_email_then_name_then_actor_local() {
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
        let email_writer = super::reviewer_from_git_config(email_repo.path());
        assert_eq!(
            email_writer.actor_id.as_str(),
            "actor:git-email:reviewer@example.com"
        );
        assert_eq!(email_writer.role, crate::session::WriterRole::Reviewer);

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
        let name_writer = super::reviewer_from_git_config(name_repo.path());
        assert_eq!(
            name_writer.actor_id.as_str(),
            "actor:git-name:reviewer-name"
        );
        assert_eq!(name_writer.role, crate::session::WriterRole::Reviewer);

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
        let local_writer = super::reviewer_from_git_config(local_repo.path());
        assert_eq!(local_writer.actor_id.as_str(), "actor:local");
        assert_eq!(local_writer.role, crate::session::WriterRole::Reviewer);
    }
}
