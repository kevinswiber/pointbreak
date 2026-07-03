// `add` is crate::session-visible so the store migrator can call the content-id
// builder (`build_validation_check_id`) and its `ValidationCheckIdMaterial` directly.
pub(in crate::session) mod add;
mod list;
mod view;

pub use self::add::{ValidationAddOptions, ValidationAddResult, record_validation_check};
pub use self::list::{
    ValidationListFilters, ValidationListOptions, ValidationListResult, list_validation_checks,
};
pub use self::view::ValidationCheckView;
pub(crate) use self::view::{
    ValidationCheckProjectionOptions, annotate_validation_supersession, project_validation_checks,
};

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use super::add::{ValidationCheckIdMaterial, build_validation_check_id};
    use super::*;
    use crate::model::{ReviewTargetRef, RevisionId, ValidationStatus, ValidationTarget};
    use crate::session::body_artifact::BODY_INLINE_LIMIT;
    use crate::session::event::{EventType, ValidationCheckRecordedPayload};
    use crate::session::{CaptureOptions, EventStore, capture_worktree_review};

    #[test]
    fn record_validation_check_writes_event_and_resolves_current_revision() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = record_validation_check(
            ValidationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_check_name("cargo test")
                .with_status(ValidationStatus::Passed),
        )
        .unwrap();

        assert_eq!(result.revision_id, capture.revision_id);
        assert_eq!(result.status, ValidationStatus::Passed);
        assert_eq!(result.events_created, 1);
        let event = validation_events(repo.path())
            .pop()
            .expect("validation event");
        assert_eq!(event.event_type, EventType::ValidationCheckRecorded);
        assert_eq!(
            crate::model::subject_revision_id(&event.reconstruct_subject().unwrap()),
            Some(&capture.revision_id)
        );
    }

    #[test]
    fn record_validation_check_constructs_revision_validation_target() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        record_validation_check(
            ValidationAddOptions::new(repo.path())
                .with_revision_id(capture.revision_id.clone())
                .with_track("agent:codex")
                .with_check_name("cargo test")
                .with_status(ValidationStatus::Passed),
        )
        .unwrap();

        let event = validation_events(repo.path())
            .pop()
            .expect("validation event");
        let payload: ValidationCheckRecordedPayload =
            serde_json::from_value(event.payload).unwrap();
        assert_eq!(
            payload.target,
            ValidationTarget::Revision {
                revision_id: capture.revision_id
            }
        );
    }

    #[test]
    fn record_validation_check_is_idempotent_on_retry_with_explicit_idempotency_key() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let options = || {
            ValidationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_check_name("cargo test")
                .with_status(ValidationStatus::Passed)
                .with_idempotency_key("manual-retry")
        };

        let first = record_validation_check(options()).unwrap();
        let second = record_validation_check(options()).unwrap();

        assert_eq!(first.validation_check_id, second.validation_check_id);
        assert_eq!(first.events_created, 1);
        assert_eq!(second.events_existing, 1);
        assert_eq!(validation_events(repo.path()).len(), 1);
    }

    #[test]
    fn record_validation_check_stages_large_summary_to_artifact() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let summary = "x".repeat(BODY_INLINE_LIMIT + 1);

        record_validation_check(
            ValidationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_check_name("cargo test")
                .with_status(ValidationStatus::Passed)
                .with_summary(summary),
        )
        .unwrap();

        let event = validation_events(repo.path())
            .pop()
            .expect("validation event");
        let payload: ValidationCheckRecordedPayload =
            serde_json::from_value(event.payload).unwrap();
        assert!(payload.summary.is_none());
        assert!(payload.summary_artifact_path.is_some());
        assert!(payload.summary_content_hash.is_some());
    }

    #[test]
    fn build_validation_check_id_uses_stable_material_digest() {
        let id = build_validation_check_id(ValidationCheckIdMaterial {
            revision_id: &RevisionId::new("review-unit:sha256:unit"),
            track_id: &crate::model::TrackId::new("agent:codex"),
            check_name: "cargo test",
            command: Some("cargo test --all"),
            status: ValidationStatus::Passed,
            exit_code: Some(0),
            trigger: crate::model::ValidationTrigger::Manual,
            source_fingerprint: Some("rev:sha256:fingerprint"),
            summary_content_hash: Some("sha256:summary"),
            summary_content_type: None,
            started_at: Some("2026-05-10T00:00:00Z"),
            completed_at: Some("2026-05-10T00:01:00Z"),
            log_artifact_content_hashes: &["sha256:bbbb".to_owned(), "sha256:aaaa".to_owned()],
            writer_actor_id: "actor:git-email:agent@example.com",
        })
        .unwrap();

        assert_eq!(
            id.as_str(),
            "validation:sha256:37f7365e260eb7cec20f6d6b28b755f9f34b4b2a6cf58cf84c43b6f40b07c5bc"
        );
    }

    #[test]
    fn validation_check_id_folds_the_kind_tag_free_subject() {
        // DD1: the content id folds the opaque revision subject id under
        // `subjectId`, never the `ValidationTarget` kind tag, so a future rename of
        // that tag is projection-only.
        use crate::canonical_hash::sha256_json_prefixed;
        use crate::model::id_prefix;
        use crate::session::event::review_subject_id;

        let revision_id = RevisionId::new("review-unit:sha256:unit");
        let track_id = crate::model::TrackId::new("agent:codex");
        let id = build_validation_check_id(ValidationCheckIdMaterial {
            revision_id: &revision_id,
            track_id: &track_id,
            check_name: "cargo test",
            command: None,
            status: ValidationStatus::Passed,
            exit_code: None,
            trigger: crate::model::ValidationTrigger::Manual,
            source_fingerprint: None,
            summary_content_hash: None,
            summary_content_type: None,
            started_at: None,
            completed_at: None,
            log_artifact_content_hashes: &[],
            writer_actor_id: "actor:test",
        })
        .unwrap();

        let subject = ReviewTargetRef::Revision {
            revision_id: revision_id.clone(),
        };
        let expected_material = serde_json::json!({
            "subjectId": review_subject_id(&subject).unwrap(),
            "trackId": track_id.as_str(),
            "checkName": "cargo test",
            "command": null,
            "status": ValidationStatus::Passed,
            "exitCode": null,
            "trigger": crate::model::ValidationTrigger::Manual,
            "sourceFingerprint": null,
            "summaryContentHash": null,
            "startedAt": null,
            "completedAt": null,
            "logArtifactContentHashes": [],
            "writerActorId": "actor:test",
        });
        let expected = format!(
            "{}:{}",
            id_prefix::VALIDATION,
            sha256_json_prefixed(&expected_material).unwrap()
        );
        assert_eq!(id.as_str(), expected);
    }

    #[test]
    fn list_validation_checks_filters_by_track_and_status() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_validation_check(
            ValidationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_check_name("cargo test")
                .with_status(ValidationStatus::Passed),
        )
        .unwrap();
        record_validation_check(
            ValidationAddOptions::new(repo.path())
                .with_track("agent:other")
                .with_check_name("cargo clippy")
                .with_status(ValidationStatus::Failed),
        )
        .unwrap();

        let result = list_validation_checks(
            ValidationListOptions::new(repo.path())
                .with_track("agent:codex")
                .with_status(ValidationStatus::Passed),
        )
        .unwrap();

        assert_eq!(result.validation_checks.len(), 1);
        assert_eq!(result.validation_checks[0].check_name, "cargo test");
        assert_eq!(result.validation_checks[0].status, ValidationStatus::Passed);
    }

    /// The store a workflow actually lands in for `repo` — the shared common-dir
    /// store by default. Reads that follow a workflow resolve here, not the raw
    /// worktree-local `.shore/data`.
    fn resolved_store_dir(repo: &std::path::Path) -> std::path::PathBuf {
        crate::git::git_common_dir(repo).unwrap().join("shore")
    }

    fn validation_events(repo: &Path) -> Vec<crate::session::event::ShoreEvent> {
        EventStore::open(resolved_store_dir(repo))
            .list_events()
            .unwrap()
            .into_iter()
            .filter(|event| event.event_type == EventType::ValidationCheckRecorded)
            .collect()
    }

    fn modified_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        repo
    }

    struct TestRepo {
        root: tempfile::TempDir,
    }

    impl TestRepo {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("temp repo");
            let repo = Self { root };
            repo.git(["init"]);
            repo.git(["config", "user.email", "agent@example.com"]);
            repo.git(["config", "user.name", "Agent"]);
            repo.git(["config", "commit.gpgsign", "false"]);
            repo
        }

        fn path(&self) -> &Path {
            self.root.path()
        }

        fn write(&self, path: &str, contents: &str) {
            let path = self.root.path().join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directories");
            }
            fs::write(path, contents).expect("write test fixture");
        }

        fn commit_all(&self, message: &str) {
            self.git(["add", "--all"]);
            self.git(["commit", "-m", message]);
        }

        fn git<I, S>(&self, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<OsStr>,
        {
            let args = args
                .into_iter()
                .map(|arg| arg.as_ref().to_owned())
                .collect::<Vec<_>>();
            let output = Command::new("git")
                .args(&args)
                .current_dir(self.root.path())
                .output()
                .unwrap_or_else(|error| panic!("run git {:?}: {error}", args));

            assert!(
                output.status.success(),
                "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
                args,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
