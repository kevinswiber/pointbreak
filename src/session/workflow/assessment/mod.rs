mod add;
mod show;
mod target;
mod util;
mod view;

pub use self::add::{AssessmentAddOptions, AssessmentAddResult, record_assessment};
pub use self::show::{
    AssessmentShowFilters, AssessmentShowOptions, AssessmentShowResult, show_assessments,
};
pub use self::target::AssessmentTargetSelector;
pub(crate) use self::view::{AssessmentProjectionOptions, project_assessments};
pub use self::view::{
    AssessmentRecordStatus, AssessmentView, CurrentAssessmentStatus, CurrentAssessmentView,
};

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::*;
    use crate::model::ReviewTargetRef;
    use crate::session::event::{
        AssertionMode, EventType, ReviewAssessment, ReviewAssessmentRecordedPayload,
    };
    use crate::session::{
        CaptureOptions, EventStore, ObservationAddOptions, SessionState, capture_worktree_review,
        record_observation,
    };

    #[test]
    fn record_review_assessment_writes_event_and_updates_session_state() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_assessment(ReviewAssessment::Accepted)
                .with_summary("ship it"),
        )
        .unwrap();

        assert_eq!(result.review_unit_id, capture.revision_id);
        assert!(result.assessment_id.as_str().starts_with("assess:sha256:"));
        assert_eq!(result.track_id.as_str(), "human:kevin");
        assert_eq!(result.assessment, ReviewAssessment::Accepted);
        assert_eq!(
            result.events_created_by_type["review_assessment_recorded"],
            1
        );

        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let state = SessionState::from_events(&events).unwrap();
        assert_eq!(state.assessment_count, 1);
    }

    #[test]
    fn record_assessment_with_actor_id_attributes_override_and_changes_derived_id() {
        use crate::model::ActorId;

        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let with_a = record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_assessment(ReviewAssessment::Accepted)
                .with_actor_id(ActorId::new("actor:agent:assess-a")),
        )
        .unwrap();
        let with_b = record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_assessment(ReviewAssessment::Accepted)
                .with_actor_id(ActorId::new("actor:agent:assess-b")),
        )
        .unwrap();

        // The override flows into the content-addressed assessment id.
        assert_ne!(with_a.assessment_id, with_b.assessment_id);

        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let actor_for = |id: &crate::model::AssessmentId| {
            events
                .iter()
                .filter(|event| event.event_type == EventType::ReviewAssessmentRecorded)
                .find(|event| event.payload["assessmentId"] == serde_json::json!(id.as_str()))
                .map(|event| event.writer.actor_id.as_str().to_owned())
                .unwrap()
        };
        assert_eq!(actor_for(&with_a.assessment_id), "actor:agent:assess-a");
        assert_eq!(actor_for(&with_b.assessment_id), "actor:agent:assess-b");
    }

    #[test]
    fn record_review_assessment_event_uses_assertion_mode_operative() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_assessment(ReviewAssessment::Accepted)
                .with_summary("ship it"),
        )
        .unwrap();

        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let event = events
            .iter()
            .find(|event| event.event_type == EventType::ReviewAssessmentRecorded)
            .expect("assessment event exists");

        assert_eq!(event.assertion_mode, AssertionMode::Operative);

        let serialized = serde_json::to_value(event).unwrap();
        assert_eq!(serialized["assertionMode"], "operative");
    }

    #[test]
    fn record_review_assessment_is_idempotent_for_same_logical_input() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let options = AssessmentAddOptions::new(repo.path())
            .with_track("human:kevin")
            .with_assessment(ReviewAssessment::Accepted)
            .with_summary("same summary");

        let first = record_assessment(options.clone()).unwrap();
        let second = record_assessment(options).unwrap();

        assert_eq!(first.assessment_id, second.assessment_id);
        assert_eq!(first.events_created, 1);
        assert_eq!(second.events_created, 0);
        assert_eq!(second.events_existing, 1);
    }

    #[test]
    fn record_review_assessment_state_json_equals_full_replay_after_created_and_existing_paths() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let options = AssessmentAddOptions::new(repo.path())
            .with_track("agent:codex")
            .with_assessment(ReviewAssessment::Accepted)
            .with_summary("looks good");

        let first = record_assessment(options.clone()).unwrap();
        assert_eq!(first.events_created, 1);
        let on_disk: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(resolved_store_dir(repo.path()).join("state.json")).unwrap(),
        )
        .unwrap();
        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let replay = serde_json::to_value(SessionState::from_events(&events).unwrap()).unwrap();
        assert_eq!(on_disk, replay, "Assessment Created path drifted");

        let second = record_assessment(options).unwrap();
        assert_eq!(second.events_existing, 1);
        let on_disk: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(resolved_store_dir(repo.path()).join("state.json")).unwrap(),
        )
        .unwrap();
        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let replay = serde_json::to_value(SessionState::from_events(&events).unwrap()).unwrap();
        assert_eq!(on_disk, replay, "Assessment Existing path drifted");
    }

    #[test]
    fn record_review_assessment_replacement_records_new_assessment_with_replaces_link() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_assessment(ReviewAssessment::NeedsChanges)
                .with_summary("fix this"),
        )
        .unwrap();

        let replacement = record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_assessment(ReviewAssessment::Accepted)
                .with_summary("fixed")
                .replacing(first.assessment_id.clone()),
        )
        .unwrap();
        let payload = assessment_payload(&repo, &replacement.assessment_id);

        assert_eq!(payload.replaces_assessment_ids, vec![first.assessment_id]);
    }

    #[test]
    fn record_review_assessment_targeting_prior_assessment_uses_assessment_ref() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_assessment(ReviewAssessment::NeedsChanges)
                .with_summary("fix this"),
        )
        .unwrap();

        let follow_up = record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_assessment(ReviewAssessment::AcceptedWithFollowUp)
                .with_summary("accepted with follow-up")
                .with_target(ReviewTargetRef::Assessment {
                    revision_id: capture.revision_id,
                    assessment_id: first.assessment_id.clone(),
                }),
        )
        .unwrap();
        let payload = assessment_payload(&repo, &follow_up.assessment_id);

        assert!(matches!(
            payload.target,
            ReviewTargetRef::Assessment { assessment_id, .. } if assessment_id == first.assessment_id
        ));
    }

    #[test]
    fn current_assessment_view_is_unassessed_when_no_assessment_exists() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = show_assessments(AssessmentShowOptions::new(repo.path())).unwrap();

        assert!(matches!(
            result.current.status,
            CurrentAssessmentStatus::Unassessed
        ));
        assert!(result.current.records.is_empty());
        assert!(result.assessments.is_empty());
    }

    #[test]
    fn current_assessment_view_resolves_to_single_unreplaced_assessment_and_includes_variant() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let assessment = record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_assessment(ReviewAssessment::Accepted)
                .with_summary("ship it"),
        )
        .unwrap();

        let result = show_assessments(AssessmentShowOptions::new(repo.path())).unwrap();

        assert!(matches!(
            result.current.status,
            CurrentAssessmentStatus::Resolved(ReviewAssessment::Accepted)
        ));
        assert_eq!(result.current.records.len(), 1);
        assert_eq!(result.current.records[0].id, assessment.assessment_id);
        assert_eq!(
            result.assessments[0].status,
            AssessmentRecordStatus::Current
        );
    }

    #[test]
    fn current_assessment_view_marks_two_unreplaced_assessments_ambiguous_and_returns_both_variants()
     {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_assessment(ReviewAssessment::Accepted)
                .with_summary("ship it"),
        )
        .unwrap();
        record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_assessment(ReviewAssessment::NeedsChanges)
                .with_summary("needs fixes"),
        )
        .unwrap();

        let result = show_assessments(AssessmentShowOptions::new(repo.path())).unwrap();

        match &result.current.status {
            CurrentAssessmentStatus::Ambiguous(variants) => {
                assert_eq!(variants.len(), 2);
                assert!(variants.contains(&ReviewAssessment::Accepted));
                assert!(variants.contains(&ReviewAssessment::NeedsChanges));
            }
            other => panic!("expected Ambiguous, got {other:?}"),
        }
        assert_eq!(result.current.records.len(), 2);
    }

    #[test]
    fn current_assessment_view_keeps_same_variant_tracks_ambiguous() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_assessment(ReviewAssessment::Accepted)
                .with_summary("ship it"),
        )
        .unwrap();
        record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_assessment(ReviewAssessment::Accepted)
                .with_summary("also accepted"),
        )
        .unwrap();

        let result = show_assessments(AssessmentShowOptions::new(repo.path())).unwrap();

        assert_eq!(
            result.current.status,
            CurrentAssessmentStatus::Ambiguous(vec![
                ReviewAssessment::Accepted,
                ReviewAssessment::Accepted
            ])
        );
        assert_eq!(result.current.records.len(), 2);
    }

    #[test]
    fn current_assessment_view_ignores_state_change_observations() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Defer")
                .with_body("Out of scope")
                .with_tag("state-change:deferred"),
        )
        .unwrap();

        let result = show_assessments(AssessmentShowOptions::new(repo.path())).unwrap();

        assert!(matches!(
            result.current.status,
            CurrentAssessmentStatus::Unassessed
        ));
        assert!(result.current.records.is_empty());
    }

    #[test]
    fn current_assessment_view_excludes_replaced_records_by_default_and_includes_with_all() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_assessment(ReviewAssessment::NeedsChanges)
                .with_summary("fix this"),
        )
        .unwrap();
        let replacement = record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_assessment(ReviewAssessment::Accepted)
                .with_summary("fixed")
                .replacing(first.assessment_id.clone()),
        )
        .unwrap();

        let current = show_assessments(AssessmentShowOptions::new(repo.path())).unwrap();
        let all = show_assessments(AssessmentShowOptions::new(repo.path()).with_all(true)).unwrap();

        assert!(matches!(
            current.current.status,
            CurrentAssessmentStatus::Resolved(ReviewAssessment::Accepted)
        ));
        assert_eq!(
            current
                .assessments
                .iter()
                .map(|view| view.id.clone())
                .collect::<Vec<_>>(),
            vec![replacement.assessment_id.clone()]
        );
        assert_eq!(all.assessments.len(), 2);
        assert!(
            all.assessments
                .iter()
                .any(|view| view.id == first.assessment_id
                    && view.status == AssessmentRecordStatus::Replaced)
        );
    }

    fn assessment_payload(
        repo: &TestRepo,
        assessment_id: &crate::model::AssessmentId,
    ) -> ReviewAssessmentRecordedPayload {
        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        events
            .into_iter()
            .filter(|event| event.event_type == EventType::ReviewAssessmentRecorded)
            .map(|event| serde_json::from_value(event.payload).unwrap())
            .find(|payload: &ReviewAssessmentRecordedPayload| {
                &payload.assessment_id == assessment_id
            })
            .unwrap()
    }

    fn modified_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 {\n    1\n}\n");
        repo.git(&["add", "src/lib.rs"]);
        repo.git(&["commit", "-m", "base"]);
        repo.write("src/lib.rs", "pub fn value() -> u32 {\n    2\n}\n");
        repo
    }

    /// The store a workflow actually lands in for `repo` — the shared common-dir
    /// store by default. Reads that follow a workflow resolve here, not the raw
    /// worktree-local `.shore/data`.
    fn resolved_store_dir(repo: &Path) -> std::path::PathBuf {
        crate::git::git_common_dir(repo).unwrap().join("shore")
    }

    struct TestRepo {
        root: tempfile::TempDir,
    }

    impl TestRepo {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("create temp git repository directory");
            let repo = Self { root };

            repo.git(&["init"]);
            repo.git(&["config", "user.name", "Shore Tests"]);
            repo.git(&["config", "user.email", "shore-tests@example.com"]);
            repo.git(&["config", "commit.gpgsign", "false"]);

            repo
        }

        fn path(&self) -> &Path {
            self.root.path()
        }

        fn write(&self, path: &str, contents: &str) {
            let path = self.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }

        fn git(&self, args: &[&str]) {
            let output = Command::new("git")
                .args(args)
                .current_dir(self.path())
                .output()
                .unwrap();
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
