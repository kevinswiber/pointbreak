use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::consume::{load_or_rebuild_session_state, read_acknowledgements, read_review_artifacts};
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
    VerdictStale,
    AcknowledgementOrphan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReloadOutcome {
    pub document: DumpDocument,
    pub diagnostics: Vec<ReloadDiagnostic>,
}

// `ReloadOutcome.diagnostics` is the canonical in-process representation.
// Task 3.1 projects the same information onto `DumpDocument` for serialized output.
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
    repo: &Path,
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

    let Some(state) = load_or_rebuild_session_state(repo)? else {
        return Ok(diagnostics);
    };

    let current_revision = state.current_revision_id.as_ref();
    let artifacts = read_review_artifacts(repo)?;
    let acknowledgements = read_acknowledgements(repo)?;

    for artifact in &artifacts {
        if current_revision.is_some_and(|revision| revision != &artifact.revision_id) {
            diagnostics.push(ReloadDiagnostic {
                code: ReloadDiagnosticCode::VerdictStale,
                message: format!(
                    "verdict {} targets revision {} instead of current {}",
                    artifact.id.as_str(),
                    artifact.revision_id.as_str(),
                    current_revision
                        .map(|revision| revision.as_str())
                        .unwrap_or("(none)"),
                ),
            });
        }
    }

    let known_artifact_ids = artifacts
        .iter()
        .map(|artifact| artifact.id.as_str().to_owned())
        .collect::<HashSet<_>>();
    for acknowledgement in &acknowledgements {
        if !known_artifact_ids.contains(acknowledgement.review_artifact_id.as_str()) {
            diagnostics.push(ReloadDiagnostic {
                code: ReloadDiagnosticCode::AcknowledgementOrphan,
                message: format!(
                    "acknowledgement {} targets unknown review artifact {}",
                    acknowledgement.id.as_str(),
                    acknowledgement.review_artifact_id.as_str(),
                ),
            });
        }
    }

    Ok(diagnostics)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::process::Command;

    use super::{ReloadDiagnosticCode, reload_session};
    use crate::dump::DumpDocument;
    use crate::model::{AcknowledgementId, ReviewArtifactId, ReviewId, RevisionId, WorkUnitId};
    use crate::session::ShoreEvent;
    use crate::session::event::{
        AcknowledgementNextAction, EventTarget, EventType, ReviewArtifactAcknowledgedPayload,
        ReviewArtifactPublishedPayload, ReviewInitializedPayload, RevisionPublishedPayload, Writer,
    };
    use crate::storage::EventStore;

    #[test]
    fn reload_session_returns_empty_diagnostics_when_no_shore_dir() {
        let repo = init_git_repo();

        let outcome = reload_session(repo.path(), || DumpDocument::from_repo(repo.path()))
            .expect("reload succeeds");

        assert!(outcome.diagnostics.is_empty());
        assert!(outcome.document.review_artifacts.is_none());
    }

    #[test]
    fn reload_session_flags_stale_verdict_when_revision_id_no_longer_current() {
        let repo = test_repo_with(vec![
            review_initialized(),
            revision_published("rev:old", vec![]),
            review_artifact_published("artifact:old", "rev:old"),
            revision_published("rev:new", vec!["rev:old"]),
        ]);

        let outcome = reload_session(repo.path(), || DumpDocument::from_repo(repo.path()))
            .expect("reload succeeds");

        assert!(
            outcome
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == ReloadDiagnosticCode::VerdictStale)
        );
    }

    #[test]
    fn reload_session_flags_orphan_acknowledgement_when_target_verdict_is_missing() {
        let repo = test_repo_with(vec![
            review_initialized(),
            review_artifact_acknowledged("ack:orphan", "artifact:missing"),
        ]);

        let outcome = reload_session(repo.path(), || DumpDocument::from_repo(repo.path()))
            .expect("reload succeeds");

        assert!(
            outcome
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == ReloadDiagnosticCode::AcknowledgementOrphan)
        );
    }

    #[test]
    fn reload_session_does_not_panic_on_invalid_event_payload() {
        let repo = test_repo_with(vec![review_initialized()]);
        let invalid_path = repo
            .path()
            .join(".shore/events")
            .join(format!("{}.json", "0".repeat(64)));
        fs::write(&invalid_path, b"{not valid json").expect("write malformed event");

        let error = reload_session(repo.path(), || DumpDocument::from_repo(repo.path()))
            .expect_err("reload should fail on malformed event");

        assert!(!error.to_string().is_empty());
        assert!(
            !repo.path().join(".shore/state.json").exists(),
            "reload should not create state.json on failure"
        );
    }

    fn test_repo_with(events: Vec<ShoreEvent>) -> tempfile::TempDir {
        let repo = init_git_repo();
        let shore_dir = repo.path().join(".shore");
        std::fs::create_dir_all(shore_dir.join("events")).unwrap();
        let store = EventStore::open(&shore_dir);
        for event in events {
            store.record_event_once(&event).unwrap();
        }
        repo
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

    fn review_initialized() -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewInitialized,
            "review_initialized:review:default:work:default",
            EventTarget::new(
                ReviewId::new("review:default"),
                WorkUnitId::new("work:default"),
            ),
            Writer::shore_local_author("0.1.0"),
            ReviewInitializedPayload {},
            "2026-05-10T00:00:00Z",
        )
        .unwrap()
    }

    fn revision_published(revision_id: &str, supersedes: Vec<&str>) -> ShoreEvent {
        ShoreEvent::new(
            EventType::RevisionPublished,
            format!("revision_published:explicit:work:default:{revision_id}"),
            EventTarget::new(
                ReviewId::new("review:default"),
                WorkUnitId::new("work:default"),
            ),
            Writer::shore_local_author("0.1.0"),
            RevisionPublishedPayload {
                revision_id: RevisionId::new(revision_id),
                supersedes_revision_ids: supersedes.into_iter().map(RevisionId::new).collect(),
            },
            "2026-05-10T00:00:00Z",
        )
        .unwrap()
    }

    fn review_artifact_published(review_artifact_id: &str, revision_id: &str) -> ShoreEvent {
        let review_artifact_id = ReviewArtifactId::new(review_artifact_id);
        let work_unit_id = WorkUnitId::new("work:default");
        ShoreEvent::new(
            EventType::ReviewArtifactPublished,
            ReviewArtifactPublishedPayload::idempotency_key(&work_unit_id, &review_artifact_id),
            EventTarget::new(ReviewId::new("review:default"), work_unit_id.clone()),
            Writer::shore_local_reviewer("0.1.0"),
            ReviewArtifactPublishedPayload {
                review_artifact_id,
                work_unit_id,
                revision_id: RevisionId::new(revision_id),
                decision: crate::session::event::VerdictDecision::Pass,
                summary: Some("looks good".to_owned()),
                summary_artifact_path: None,
                summary_byte_size: None,
                replaces_review_artifact_ids: Vec::new(),
                reviewer: Writer::shore_local_reviewer("0.1.0"),
            },
            "2026-05-10T00:00:00Z",
        )
        .unwrap()
    }

    fn review_artifact_acknowledged(
        acknowledgement_id: &str,
        review_artifact_id: &str,
    ) -> ShoreEvent {
        let review_artifact_id = ReviewArtifactId::new(review_artifact_id);
        let acknowledgement_id = AcknowledgementId::new(acknowledgement_id);
        ShoreEvent::new(
            EventType::ReviewArtifactAcknowledged,
            ReviewArtifactAcknowledgedPayload::idempotency_key(
                &review_artifact_id,
                &acknowledgement_id,
            ),
            EventTarget::new(
                ReviewId::new("review:default"),
                WorkUnitId::new("work:default"),
            ),
            Writer::shore_local_author("0.1.0"),
            ReviewArtifactAcknowledgedPayload {
                acknowledgement_id,
                review_artifact_id,
                next_action: AcknowledgementNextAction::Accept,
                reason: Some("ok".to_owned()),
                reason_artifact_path: None,
                reason_byte_size: None,
                acknowledger: Writer::shore_local_author("0.1.0"),
            },
            "2026-05-10T00:00:00Z",
        )
        .unwrap()
    }
}
