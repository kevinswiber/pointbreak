use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::dump::DumpDocument;
use crate::error::Result;
use crate::model::ResolutionStatus;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReloadDiagnostic {
    pub code: ReloadDiagnosticCode,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReloadDiagnosticCode {
    NoteOrphaned,
    NoteStale,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReloadOutcome {
    pub document: DumpDocument,
    pub diagnostics: Vec<ReloadDiagnostic>,
}

pub fn reload_session<F>(repo: impl AsRef<Path>, load: F) -> Result<ReloadOutcome>
where
    F: FnOnce() -> Result<DumpDocument>,
{
    let document = load()?;
    let diagnostics = reload_diagnostics_for_document(repo.as_ref(), &document)?;
    Ok(ReloadOutcome {
        document,
        diagnostics,
    })
}

pub(crate) fn reload_diagnostics_for_document(
    _repo: &Path,
    document: &DumpDocument,
) -> Result<Vec<ReloadDiagnostic>> {
    let mut diagnostics = Vec::new();

    for note in &document.notes {
        match note.anchor.status {
            ResolutionStatus::Stale => diagnostics.push(ReloadDiagnostic {
                code: ReloadDiagnosticCode::NoteStale,
                message: format!("note {} is stale", note.id.as_str()),
            }),
            ResolutionStatus::Orphaned => diagnostics.push(ReloadDiagnostic {
                code: ReloadDiagnosticCode::NoteOrphaned,
                message: format!("note {} is orphaned", note.id.as_str()),
            }),
            _ => {}
        }
    }

    Ok(diagnostics)
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::reload_session;
    use crate::dump::DumpDocument;

    #[test]
    fn reload_session_returns_empty_diagnostics_when_no_shore_dir() {
        let repo = init_git_repo();

        let outcome = reload_session(repo.path(), || DumpDocument::from_repo(repo.path()))
            .expect("reload succeeds");

        assert!(outcome.diagnostics.is_empty());
    }

    fn init_git_repo() -> tempfile::TempDir {
        let repo = tempfile::tempdir().expect("create repo");
        run_git(repo.path(), &["init"]);
        run_git(repo.path(), &["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.path().join(".gitignore"), ".shore/\n").expect("write fixture file");
        run_git(repo.path(), &["add", ".gitignore"]);
        run_git(
            repo.path(),
            &[
                "-c",
                "commit.gpgsign=false",
                "-c",
                "user.name=Test User",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                "init",
            ],
        );
        repo
    }

    fn run_git(repo: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
