mod add;
mod list;
mod target;
mod util;
mod view;

pub use self::add::{ObservationAddOptions, ObservationAddResult, record_observation};
pub use self::list::{ObservationListOptions, ObservationListResult, list_observations};
pub use self::target::ObservationTargetSelector;
pub(crate) use self::target::{
    CurrentRevisionContext, ResolvedRevision, RevisionScope, RevisionSelection,
    resolve_observation_target, resolve_revision, revision_ids_in_worktree,
};
pub(crate) use self::util::{required_title, staged_body, validated_track_id};
#[cfg(test)]
use self::view::sort_observation_views;
pub(crate) use self::view::{
    ObservationProjectionOptions, project_observations, target_matches_file,
};
pub use self::view::{ObservationStatus, ObservationView};

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::*;
    use crate::model::{
        EngagementId, EventId, JournalId, ObjectId, ObservationId, ReviewEndpoint, ReviewTargetRef,
        RevisionId, RevisionSource, Side, TrackId, WorktreeCaptureMode,
    };
    use crate::session::event::{
        EventTarget, EventType, GitProvenance, Revision, ShoreEvent, WorkObjectProposal,
        WorkObjectProposedPayload, Writer,
    };
    use crate::session::store::content::ContentArtifacts;
    use crate::session::{
        CaptureOptions, CaptureResult, EventStore, SessionState, capture_worktree_review,
    };

    #[test]
    fn track_policy_accepts_lowercase_local_and_namespaced_ids() {
        assert_eq!(validated_track_id("codex").unwrap().as_str(), "codex");
        assert_eq!(
            validated_track_id("agent:codex").unwrap().as_str(),
            "agent:codex"
        );
        assert_eq!(
            validated_track_id("human:kevin").unwrap().as_str(),
            "human:kevin"
        );
    }

    #[test]
    fn track_policy_rejects_reserved_or_unsafe_ids() {
        for bad in [
            "",
            "All",
            "all",
            "*",
            "none",
            "null",
            "default",
            "agent/codex",
            "agent codex",
            "system:shore",
            "import:hunk",
        ] {
            assert!(validated_track_id(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn track_policy_rejects_overlong_ids() {
        let too_long = "a".repeat(129);

        assert!(validated_track_id(&too_long).is_err());
    }

    #[test]
    fn resolves_single_current_revision_when_not_explicit() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let event_store = EventStore::open(resolved_store_dir(repo.path()));
        let events = event_store.list_events().unwrap();

        let context = CurrentRevisionContext::for_repo(repo.path()).unwrap();
        let resolved = resolve_revision(
            &events,
            RevisionSelection::Current,
            &context,
            RevisionScope::CurrentWorktree,
        )
        .unwrap();

        assert_eq!(resolved.revision_id, capture.revision_id);
        assert_eq!(resolved.object_id, capture.object_id);
    }

    #[test]
    fn resolving_current_revision_errors_when_none_captured() {
        let events = Vec::new();

        let error = resolve_revision(
            &events,
            RevisionSelection::Current,
            &any_context(),
            RevisionScope::All,
        )
        .unwrap_err();

        assert!(error.to_string().contains("no captured revision"));
    }

    #[test]
    fn resolving_current_revision_errors_when_ambiguous() {
        let events = vec![
            revision_captured_event_with_ids("rev:one", "snap:one"),
            revision_captured_event_with_ids("rev:two", "snap:two"),
        ];

        let error = resolve_revision(
            &events,
            RevisionSelection::Current,
            &any_context(),
            RevisionScope::All,
        )
        .unwrap_err();

        assert!(error.to_string().contains("multiple captured revisions"));
    }

    #[test]
    fn explicit_unknown_revision_is_rejected() {
        let events = vec![revision_captured_event_with_ids("rev:one", "snap:one")];

        let error = resolve_revision(
            &events,
            RevisionSelection::Exact(&RevisionId::new("review-unit:sha256:missing")),
            &any_context(),
            RevisionScope::All,
        )
        .unwrap_err();

        assert!(error.to_string().contains("unknown revision"));
    }

    #[test]
    fn head_seed_that_is_a_current_head_resolves_exactly() {
        // A <- B (B supersedes A): B is the head; seeding B resolves B.
        let events = vec![revision_event("a", &[]), revision_event("b", &["a"])];

        let resolved = resolve_revision(
            &events,
            RevisionSelection::Head(&rev("b")),
            &any_context(),
            RevisionScope::All,
        )
        .unwrap();

        assert_eq!(resolved.revision_id, rev("b"));
    }

    #[test]
    fn head_seed_on_a_superseded_revision_resolves_its_thread_head() {
        // Seeding the superseded A resolves to its thread's current head, B.
        let events = vec![revision_event("a", &[]), revision_event("b", &["a"])];

        let resolved = resolve_revision(
            &events,
            RevisionSelection::Head(&rev("a")),
            &any_context(),
            RevisionScope::All,
        )
        .unwrap();

        assert_eq!(resolved.revision_id, rev("b"));
    }

    #[test]
    fn head_seed_under_a_fork_force_disambiguates_thread_scoped() {
        // A <- {B, C} (competing heads) plus an unrelated thread Z. A non-head seed
        // lists the thread's competing heads (B, C), never the unrelated head Z.
        let events = vec![
            revision_event("a", &[]),
            revision_event("b", &["a"]),
            revision_event("c", &["a"]),
            revision_event("z", &[]),
        ];

        let error = resolve_revision(
            &events,
            RevisionSelection::Head(&rev("a")),
            &any_context(),
            RevisionScope::All,
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("rev:sha256:b"));
        assert!(message.contains("rev:sha256:c"));
        assert!(!message.contains("rev:sha256:z"));

        // A head seed escapes the fork by resolving to itself exactly.
        let resolved = resolve_revision(
            &events,
            RevisionSelection::Head(&rev("b")),
            &any_context(),
            RevisionScope::All,
        )
        .unwrap();
        assert_eq!(resolved.revision_id, rev("b"));
    }

    #[test]
    fn target_selector_builds_review_wide_file_and_range_refs() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let resolved = resolved_from_capture(&capture);

        let review_wide = resolve_observation_target(
            repo.path(),
            &resolved,
            &ObservationTargetSelector::revision(),
        )
        .unwrap();
        let file = resolve_observation_target(
            repo.path(),
            &resolved,
            &ObservationTargetSelector::file("src/lib.rs"),
        )
        .unwrap();
        let range = resolve_observation_target(
            repo.path(),
            &resolved,
            &ObservationTargetSelector::range("src/lib.rs", Side::New, 2, Some(3)),
        )
        .unwrap();

        assert!(matches!(review_wide, ReviewTargetRef::Revision { .. }));
        assert!(matches!(file, ReviewTargetRef::File { .. }));
        assert!(matches!(
            range,
            ReviewTargetRef::Range {
                start_line: 2,
                end_line: 3,
                ..
            }
        ));
    }

    #[test]
    fn target_selector_rejects_file_not_in_captured_snapshot() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let resolved = resolved_from_capture(&capture);

        let error = resolve_observation_target(
            repo.path(),
            &resolved,
            &ObservationTargetSelector::file("missing.rs"),
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("not present in captured snapshot")
        );
    }

    #[test]
    fn target_selector_rejects_invalid_range_shape() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let resolved = resolved_from_capture(&capture);

        let zero = resolve_observation_target(
            repo.path(),
            &resolved,
            &ObservationTargetSelector::range("src/lib.rs", Side::New, 0, Some(1)),
        )
        .unwrap_err();
        let reversed = resolve_observation_target(
            repo.path(),
            &resolved,
            &ObservationTargetSelector::range("src/lib.rs", Side::New, 3, Some(2)),
        )
        .unwrap_err();

        assert!(zero.to_string().contains("start line"));
        assert!(reversed.to_string().contains("end line"));
    }

    #[test]
    fn record_observation_writes_event_and_updates_state() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Check return value")
                .with_target(ObservationTargetSelector::file("src/lib.rs")),
        )
        .unwrap();

        assert_eq!(result.revision_id, capture.revision_id);
        assert!(result.observation_id.as_str().starts_with("obs:sha256:"));
        assert_eq!(result.track_id.as_str(), "agent:codex");
        assert_eq!(result.events_created, 1);
        assert_eq!(result.events_existing, 0);
        assert_eq!(
            result.events_created_by_type["review_observation_recorded"],
            1
        );
        assert!(result.body_content_hash.is_none());

        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let state = SessionState::from_events(&events).unwrap();
        assert_eq!(state.observation_count, 1);
    }

    #[test]
    fn record_observation_with_actor_id_attributes_override_and_changes_derived_id() {
        use crate::model::ActorId;

        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let with_a = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Check return value")
                .with_actor_id(ActorId::new("actor:agent:obs-a")),
        )
        .unwrap();
        let with_b = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Check return value")
                .with_actor_id(ActorId::new("actor:agent:obs-b")),
        )
        .unwrap();

        // The override flows into the content-addressed observation id.
        assert_ne!(with_a.observation_id, with_b.observation_id);

        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let actor_for = |id: &crate::model::ObservationId| {
            events
                .iter()
                .filter(|event| event.event_type == EventType::ReviewObservationRecorded)
                .find(|event| event.payload["observationId"] == serde_json::json!(id.as_str()))
                .map(|event| event.writer.actor_id.as_str().to_owned())
                .unwrap()
        };
        assert_eq!(actor_for(&with_a.observation_id), "actor:agent:obs-a");
        assert_eq!(actor_for(&with_b.observation_id), "actor:agent:obs-b");
    }

    #[test]
    fn record_observation_without_actor_id_uses_git_identity() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Check return value"),
        )
        .unwrap();

        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let observation = events
            .iter()
            .find(|event| event.event_type == EventType::ReviewObservationRecorded)
            .unwrap();
        assert_eq!(
            observation.writer.actor_id.as_str(),
            "actor:git-email:shore-tests@example.com"
        );
    }

    #[test]
    fn record_observation_is_idempotent_for_same_logical_input() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let options = ObservationAddOptions::new(repo.path())
            .with_track("agent:codex")
            .with_title("Same finding")
            .with_body("same body")
            .with_target(ObservationTargetSelector::revision());

        let first = record_observation(options.clone()).unwrap();
        let second = record_observation(options).unwrap();

        assert_eq!(first.observation_id, second.observation_id);
        assert_eq!(first.events_created, 1);
        assert_eq!(second.events_created, 0);
        assert_eq!(second.events_existing, 1);
    }

    #[test]
    fn record_observation_state_json_equals_full_replay_after_created_and_existing_paths() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let options = ObservationAddOptions::new(repo.path())
            .with_track("agent:codex")
            .with_title("equal-after-write")
            .with_body("same body");

        let first = record_observation(options.clone()).unwrap();
        assert_eq!(first.events_created, 1);
        assert_eq!(first.events_existing, 0);
        let on_disk: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(resolved_store_dir(repo.path()).join("state.json")).unwrap(),
        )
        .unwrap();
        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let replay = serde_json::to_value(SessionState::from_events(&events).unwrap()).unwrap();
        assert_eq!(on_disk, replay, "Created path drifted from full replay");

        let second = record_observation(options).unwrap();
        assert_eq!(second.events_created, 0);
        assert_eq!(second.events_existing, 1);
        let on_disk: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(resolved_store_dir(repo.path()).join("state.json")).unwrap(),
        )
        .unwrap();
        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let replay = serde_json::to_value(SessionState::from_events(&events).unwrap()).unwrap();
        assert_eq!(on_disk, replay, "Existing path drifted from full replay");
    }

    #[test]
    fn explicit_same_idempotency_key_with_different_payload_conflicts() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("First")
                .with_idempotency_key("retry-key"),
        )
        .unwrap();
        let error = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Second")
                .with_idempotency_key("retry-key"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("event conflict"));
    }

    #[test]
    fn large_observation_body_is_stored_as_internal_body_artifact() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let body = "x".repeat(crate::session::body_artifact::BODY_INLINE_LIMIT + 1);

        let result = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Large body")
                .with_body(body),
        )
        .unwrap();

        assert!(
            result
                .body_content_hash
                .as_deref()
                .unwrap()
                .starts_with("sha256:")
        );
        assert!(
            !format!("{result:?}").contains("artifacts/notes/"),
            "workflow result must not expose internal artifact paths"
        );

        let artifacts = ContentArtifacts::local(&resolved_store_dir(repo.path()))
            .list_refs("artifacts/notes")
            .unwrap();
        assert_eq!(artifacts.len(), 1);
    }

    #[test]
    fn correction_records_new_observation_with_supersedes_link() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let original = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Original"),
        )
        .unwrap();
        let correction = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Correction")
                .superseding(original.observation_id.clone()),
        )
        .unwrap();

        assert_ne!(original.observation_id, correction.observation_id);

        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let correction_event = events
            .iter()
            .find(|event| event.event_id == correction.event_id)
            .unwrap();
        assert_eq!(
            correction_event.payload["supersedesObservationIds"][0],
            original.observation_id.as_str()
        );
    }

    #[test]
    fn observation_records_responds_to_link_in_payload() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let base = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("A"),
        )
        .unwrap();
        let ack = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("noted — tracking as issue #N")
                .responding_to(base.observation_id.clone()),
        )
        .unwrap();

        // The payload records the responds-to link. (These ids differ only because the titles
        // differ; that responds_to itself changes the id is a separate concern, exercised elsewhere.)
        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let ack_event = events
            .iter()
            .find(|event| event.event_id == ack.event_id)
            .unwrap();
        assert_eq!(
            ack_event.payload["respondsToObservationIds"][0],
            base.observation_id.as_str()
        );
    }

    #[test]
    fn responds_to_yields_a_distinct_observation_id() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let plain = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("x"),
        )
        .unwrap();
        let acking = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("x")
                .responding_to(ObservationId::new("obs:sha256:deadbeef")),
        )
        .unwrap();
        assert_ne!(plain.observation_id, acking.observation_id);
    }

    #[test]
    fn list_observations_returns_observations_for_current_revision() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("First"),
        )
        .unwrap();
        let second = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:claude")
                .with_title("Second"),
        )
        .unwrap();

        let result = list_observations(ObservationListOptions::new(repo.path())).unwrap();

        assert_eq!(result.revision_id, capture.revision_id);
        let mut actual_ids = result
            .observations
            .iter()
            .map(|observation| observation.id.as_str().to_owned())
            .collect::<Vec<_>>();
        actual_ids.sort();
        let mut expected_ids = vec![
            first.observation_id.as_str().to_owned(),
            second.observation_id.as_str().to_owned(),
        ];
        expected_ids.sort();
        assert_eq!(actual_ids, expected_ids);
    }

    #[test]
    fn list_observations_collapses_duplicate_semantic_events() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Same finding")
                .with_body("same body")
                .with_idempotency_key("retry-a"),
        )
        .unwrap();
        let second = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Same finding")
                .with_body("same body")
                .with_idempotency_key("retry-b"),
        )
        .unwrap();

        let result =
            list_observations(ObservationListOptions::new(repo.path()).with_include_body(true))
                .unwrap();

        assert_eq!(first.observation_id, second.observation_id);
        assert_eq!(first.events_created, 1);
        assert_eq!(second.events_created, 1);
        assert_eq!(result.observations.len(), 1);
        assert_eq!(result.observations[0].id, first.observation_id);
        assert_eq!(result.observations[0].body.as_deref(), Some("same body"));
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == crate::session::state::DUPLICATE_SEMANTIC_OBSERVATION_EVENT_CODE
        }));
    }

    #[test]
    fn list_observations_resolves_the_store_from_a_subdirectory() {
        // Listing from a nested path resolves the worktree root and reads the same
        // resolved (shared common-dir) store the write landed in.
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let added = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("subdir read"),
        )
        .unwrap();

        let result = list_observations(ObservationListOptions::new(repo.path().join("src")))
            .expect("observations load from subdirectory");

        assert_eq!(result.observations[0].id, added.observation_id);
    }

    #[test]
    fn list_observations_filters_by_track_and_file() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("File")
                .with_target(ObservationTargetSelector::file("src/lib.rs")),
        )
        .unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:claude")
                .with_title("Review wide"),
        )
        .unwrap();

        let result = list_observations(
            ObservationListOptions::new(repo.path())
                .with_track("agent:codex")
                .with_file("src/lib.rs"),
        )
        .unwrap();

        assert_eq!(result.observations.len(), 1);
        assert_eq!(result.observations[0].track_id.as_str(), "agent:codex");
    }

    #[test]
    fn list_observations_omits_body_by_default_and_hydrates_when_requested() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Body")
                .with_body("large ".repeat(1000)),
        )
        .unwrap();

        let without_body = list_observations(ObservationListOptions::new(repo.path())).unwrap();
        let with_body =
            list_observations(ObservationListOptions::new(repo.path()).with_include_body(true))
                .unwrap();

        assert!(without_body.observations[0].body.is_none());
        assert!(
            with_body.observations[0]
                .body
                .as_deref()
                .unwrap()
                .starts_with("large ")
        );
        assert!(
            !format!("{with_body:?}").contains("artifacts/notes/"),
            "list result must not expose internal artifact paths"
        );
    }

    #[test]
    fn list_observations_marks_superseded_observations() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let original = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Original"),
        )
        .unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Correction")
                .superseding(original.observation_id.clone()),
        )
        .unwrap();

        let result = list_observations(ObservationListOptions::new(repo.path())).unwrap();
        let original_view = result
            .observations
            .iter()
            .find(|observation| observation.id == original.observation_id)
            .unwrap();

        assert_eq!(original_view.status, ObservationStatus::Superseded);
    }

    #[test]
    fn responded_by_reverse_map_is_derived_cross_track_and_target_stays_active() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        // author-track observation A
        let a = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:author")
                .with_title("A"),
        )
        .unwrap();
        // reviewer-track observation B responds to A (CROSS-TRACK)
        let b = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:reviewer")
                .with_title("B")
                .responding_to(a.observation_id.clone()),
        )
        .unwrap();

        // Project filtered to A's track, which EXCLUDES B from the returned set. If responds_to edges
        // were collected AFTER the track filter, B's edge would be dropped and A.responded_by would be
        // empty — so this filtered projection is what proves collection happens before the filter.
        let filtered =
            list_observations(ObservationListOptions::new(repo.path()).with_track("agent:author"))
                .unwrap();
        let a_view = filtered
            .observations
            .iter()
            .find(|v| v.id == a.observation_id)
            .unwrap();

        // (1) derived reverse-map names B, even though B's track was filtered out of the returned set
        assert!(a_view.responded_by.iter().any(|id| id == &b.observation_id));
        // (2) target A stays Active — no status flip
        assert_eq!(a_view.status, ObservationStatus::Active);

        // (3) B carries the forward pointer — check via an unfiltered list (B is filtered out above)
        let all = list_observations(ObservationListOptions::new(repo.path())).unwrap();
        let b_view = all
            .observations
            .iter()
            .find(|v| v.id == b.observation_id)
            .unwrap();
        assert!(b_view.responds_to.iter().any(|id| id == &a.observation_id));
    }

    #[test]
    fn list_observations_sorts_by_occurred_at_then_event_id() {
        let mut observations = vec![
            observation_view_for_sort("obs:sha256:b", "evt:sha256:b", "unix-ms:2"),
            observation_view_for_sort("obs:sha256:c", "evt:sha256:c", "unix-ms:1"),
            observation_view_for_sort("obs:sha256:a", "evt:sha256:a", "unix-ms:1"),
        ];

        sort_observation_views(&mut observations);

        assert_eq!(
            observations
                .iter()
                .map(|observation| observation.id.as_str())
                .collect::<Vec<_>>(),
            vec!["obs:sha256:a", "obs:sha256:c", "obs:sha256:b"]
        );
    }

    fn resolved_from_capture(capture: &CaptureResult) -> ResolvedRevision {
        ResolvedRevision {
            journal_id: capture.journal_id.clone(),
            revision_id: capture.revision_id.clone(),
            object_id: capture.object_id.clone(),
            object_artifact_content_hash: capture.object_artifact_content_hash.clone(),
        }
    }

    /// A context for selection tests that do not exercise worktree scoping (they
    /// widen to `RevisionScope::All` or resolve `Exact`/`LineageHead`).
    fn any_context() -> CurrentRevisionContext {
        CurrentRevisionContext {
            worktree_root: "/repo".to_owned(),
            head_ref: None,
        }
    }

    fn revision_captured_event_with_ids(revision_id: &str, object_id: &str) -> ShoreEvent {
        // The envelope subject and the payload revision address one and the same
        // revision, as a real capture stamps both from one minted id.
        let revision_id = RevisionId::new(revision_id);
        let object_id = ObjectId::new(object_id);
        ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("work_object_proposed:{}", revision_id.as_str()),
            EventTarget::for_revision(JournalId::new("journal:default"), revision_id.clone(), None),
            Writer::shore_local("0.1.0"),
            WorkObjectProposedPayload {
                engagement_id: EngagementId::new(format!(
                    "engagement:sha256:{}",
                    crate::canonical_hash::sha256_bytes_hex(revision_id.as_str().as_bytes())
                )),
                work_object: WorkObjectProposal::Revision {
                    revision: Revision {
                        id: revision_id.clone(),
                        object_id: object_id.clone(),
                        git_provenance: Some(GitProvenance {
                            source: RevisionSource::GitWorktree {
                                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                                include_untracked: true,
                                pathspecs: Vec::new(),
                            },
                            base: ReviewEndpoint::GitCommit {
                                commit_oid: "abc".to_owned(),
                                tree_oid: "def".to_owned(),
                            },
                            target: ReviewEndpoint::GitWorkingTree {
                                worktree_root: "/repo".to_owned(),
                            },
                        }),
                    },
                    object_artifact_content_hash: "sha256:artifact".to_owned(),
                    supersedes: vec![],
                },
            },
            "2026-05-12T00:00:00Z",
        )
        .unwrap()
    }

    fn rev(suffix: &str) -> RevisionId {
        RevisionId::new(format!("rev:sha256:{suffix}"))
    }

    /// A review-domain generative move proposing a revision that supersedes the
    /// given predecessors (by suffix). Feeds the supersession head-selection.
    fn revision_event(suffix: &str, supersedes: &[&str]) -> ShoreEvent {
        let revision_id = rev(suffix);
        ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("work_object_proposed:{}", revision_id.as_str()),
            EventTarget::for_revision(JournalId::new("journal:default"), revision_id.clone(), None),
            Writer::shore_local("0.1.0"),
            WorkObjectProposedPayload {
                engagement_id: EngagementId::new(format!("engagement:sha256:{suffix}")),
                work_object: WorkObjectProposal::Revision {
                    revision: Revision {
                        id: revision_id,
                        object_id: ObjectId::new(format!("obj:sha256:{suffix}")),
                        git_provenance: Some(GitProvenance {
                            source: RevisionSource::GitWorktree {
                                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                                include_untracked: true,
                                pathspecs: Vec::new(),
                            },
                            base: ReviewEndpoint::GitCommit {
                                commit_oid: "abc".to_owned(),
                                tree_oid: "def".to_owned(),
                            },
                            target: ReviewEndpoint::GitWorkingTree {
                                worktree_root: "/repo".to_owned(),
                            },
                        }),
                    },
                    object_artifact_content_hash: "sha256:artifact".to_owned(),
                    supersedes: supersedes.iter().map(|s| rev(s)).collect(),
                },
            },
            "2026-05-12T00:00:00Z",
        )
        .unwrap()
    }

    fn observation_view_for_sort(
        observation_id: &str,
        event_id: &str,
        created_at: &str,
    ) -> ObservationView {
        let revision_id = RevisionId::new("review-unit:sha256:one");
        ObservationView {
            id: crate::model::ObservationId::new(observation_id),
            event_id: EventId::new(event_id),
            track_id: TrackId::new("agent:codex"),
            target: ReviewTargetRef::Revision { revision_id },
            title: "sort".to_owned(),
            body: None,
            body_content_type: Default::default(),
            tags: vec![],
            confidence: None,
            status: ObservationStatus::Active,
            supersedes: vec![],
            responds_to: vec![],
            responded_by: vec![],
            body_content_hash: None,
            body_content_state: Default::default(),
            created_at: created_at.to_owned(),
            writer: Writer::shore_local("test"),
        }
    }

    fn modified_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 {\n    1\n}\n");
        repo.commit_all("base");
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

            repo.git(["init"]);
            repo.git(["config", "user.name", "Shore Tests"]);
            repo.git(["config", "user.email", "shore-tests@example.com"]);
            repo.git(["config", "commit.gpgsign", "false"]);

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

        fn commit_all(&self, message: &str) {
            self.git(["add", "."]);
            self.git(["commit", "-m", message]);
        }

        fn git<I, S>(&self, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<std::ffi::OsStr>,
        {
            let output = Command::new("git")
                .args(args)
                .current_dir(self.path())
                .output()
                .expect("run git command");
            assert!(
                output.status.success(),
                "git failed\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
