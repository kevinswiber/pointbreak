mod fetch;
mod list;
mod request;
mod resolve;
mod target;
mod view;

pub use self::fetch::{InterventionFetchOptions, InterventionFetchResult, fetch_intervention};
pub use self::list::{InterventionListOptions, InterventionListResult, list_interventions};
pub use self::request::{
    InterventionRequestOptions, InterventionRequestResult, request_intervention,
};
pub use self::resolve::{
    InterventionResolveOptions, InterventionResolveResult, resolve_intervention,
};
pub use self::target::InterventionTargetSelector;
#[cfg(test)]
pub use self::view::InterventionStatus;
#[cfg(test)]
use self::view::collect_intervention_projection_records;
#[cfg(test)]
use self::view::sort_intervention_views;
pub(crate) use self::view::{InterventionProjectionOptions, project_interventions};
pub use self::view::{InterventionResolutionView, InterventionStatusFilter, InterventionView};
#[cfg(test)]
use crate::canonical_hash::sha256_bytes_hex;
#[cfg(test)]
use crate::model::{
    EventId, InterventionId, InterventionResolutionId, ReviewUnitId, SessionId, TrackId,
};
#[cfg(test)]
use crate::session::current_timestamp;
#[cfg(test)]
use crate::session::event::{
    EventTarget, EventType, InterventionMode, InterventionReasonCode, InterventionRequestedPayload,
    InterventionResolutionOutcome, InterventionResolvedPayload, ShoreEvent, Writer,
};

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::*;
    use crate::model::{ObservationId, ReviewTargetRef, Side, TargetRef};
    use crate::session::{
        CaptureOptions, EventStore, ObservationAddOptions, SessionState, capture_worktree_review,
        record_observation,
    };

    #[test]
    fn request_intervention_writes_event_and_updates_state() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired)
                .with_mode(InterventionMode::Blocking)
                .with_target(InterventionTargetSelector::review_unit()),
        )
        .unwrap();

        assert_eq!(result.review_unit_id, capture.review_unit_id);
        assert!(
            result
                .intervention_id
                .as_str()
                .starts_with("intervention:sha256:")
        );
        assert_eq!(result.mode, InterventionMode::Blocking);
        assert_eq!(
            result.reason_code,
            InterventionReasonCode::ManualDecisionRequired
        );
        assert_eq!(result.track_id.as_str(), "human:kevin");
        assert_eq!(result.events_created, 1);
        assert_eq!(result.events_existing, 0);
        assert_eq!(result.events_created_by_type["intervention_requested"], 1);

        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let state = SessionState::from_events(&events).unwrap();
        assert_eq!(state.intervention_count, 1);
        assert_eq!(state.open_intervention_count, 1);
        assert_eq!(state.open_blocking_intervention_count, 1);
    }

    #[test]
    fn request_intervention_is_idempotent_for_same_logical_input() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let options = InterventionRequestOptions::new(repo.path())
            .with_track("agent:codex")
            .with_title("Pause")
            .with_reason_code(InterventionReasonCode::UnsafeAction)
            .with_body("same body");

        let first = request_intervention(options.clone()).unwrap();
        let second = request_intervention(options).unwrap();

        assert_eq!(first.intervention_id, second.intervention_id);
        assert_eq!(first.events_created, 1);
        assert_eq!(second.events_created, 0);
        assert_eq!(second.events_existing, 1);
    }

    #[test]
    fn request_intervention_state_json_equals_full_replay_after_created_and_existing_paths() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let options = InterventionRequestOptions::new(repo.path())
            .with_track("agent:codex")
            .with_title("blocking-finding")
            .with_mode(InterventionMode::Blocking)
            .with_reason_code(InterventionReasonCode::UnsafeAction);

        let first = request_intervention(options.clone()).unwrap();
        assert_eq!(first.events_created, 1);
        let on_disk: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(repo.path().join(".shore/state.json")).unwrap(),
        )
        .unwrap();
        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let replay = serde_json::to_value(SessionState::from_events(&events).unwrap()).unwrap();
        assert_eq!(on_disk, replay, "Created path drifted");

        let second = request_intervention(options).unwrap();
        assert_eq!(second.events_existing, 1);
        let on_disk: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(repo.path().join(".shore/state.json")).unwrap(),
        )
        .unwrap();
        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let replay = serde_json::to_value(SessionState::from_events(&events).unwrap()).unwrap();
        assert_eq!(on_disk, replay, "Existing path drifted");
    }

    #[test]
    fn resolve_intervention_state_json_equals_full_replay_after_created_and_existing_paths() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let request = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("to-resolve")
                .with_mode(InterventionMode::Blocking)
                .with_reason_code(InterventionReasonCode::UnsafeAction),
        )
        .unwrap();

        let options = InterventionResolveOptions::new(repo.path(), request.intervention_id.clone())
            .with_outcome(InterventionResolutionOutcome::Approved);

        let first = resolve_intervention(options.clone()).unwrap();
        assert_eq!(first.events_created, 1);
        let on_disk: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(repo.path().join(".shore/state.json")).unwrap(),
        )
        .unwrap();
        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let replay = serde_json::to_value(SessionState::from_events(&events).unwrap()).unwrap();
        assert_eq!(on_disk, replay, "Resolve Created path drifted");

        let second = resolve_intervention(options).unwrap();
        assert_eq!(second.events_existing, 1);
        let on_disk: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(repo.path().join(".shore/state.json")).unwrap(),
        )
        .unwrap();
        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let replay = serde_json::to_value(SessionState::from_events(&events).unwrap()).unwrap();
        assert_eq!(on_disk, replay, "Resolve Existing path drifted");
    }

    #[test]
    fn request_intervention_requires_a_captured_review_unit() {
        let repo = modified_repo();

        let error = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired),
        )
        .unwrap_err();

        assert!(error.to_string().contains("no captured review unit"));
    }

    #[test]
    fn request_intervention_requires_explicit_review_unit_when_current_is_ambiguous() {
        let repo = modified_repo();
        let first = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        repo.write("src/lib.rs", "pub fn value() -> u32 {\n    3\n}\n");
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let ambiguous = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired),
        )
        .unwrap_err();
        assert!(
            ambiguous
                .to_string()
                .contains("multiple captured review units")
        );

        let explicit = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_review_unit_id(first.review_unit_id.clone())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired),
        )
        .unwrap();
        assert_eq!(explicit.review_unit_id, first.review_unit_id);
    }

    #[test]
    fn request_intervention_rejects_invalid_track() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let error = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("system:shore")
                .with_title("Need approval")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired),
        )
        .unwrap_err();

        assert!(error.to_string().contains("track namespace is reserved"));
    }

    #[test]
    fn request_intervention_rejects_file_target_absent_from_snapshot() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let error = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired)
                .with_target(InterventionTargetSelector::file("missing.rs")),
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("not present in captured snapshot")
        );
    }

    #[test]
    fn request_intervention_supports_file_range_targets() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired)
                .with_target(InterventionTargetSelector::range(
                    "src/lib.rs",
                    Side::New,
                    2,
                    Some(2),
                )),
        )
        .unwrap();

        assert_eq!(result.review_unit_id, capture.review_unit_id);
        assert!(matches!(
            result.target,
            ReviewTargetRef::Range { start_line: 2, .. }
        ));
    }

    #[test]
    fn request_intervention_validates_native_observation_target() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let observation = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Observation"),
        )
        .unwrap();

        let result = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired)
                .with_target(InterventionTargetSelector::observation(
                    observation.observation_id.clone(),
                )),
        )
        .unwrap();

        assert_eq!(
            result.target,
            ReviewTargetRef::Observation {
                review_unit_id: capture.review_unit_id,
                observation_id: observation.observation_id,
            }
        );
    }

    #[test]
    fn request_intervention_rejects_unknown_observation_target() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let error = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired)
                .with_target(InterventionTargetSelector::observation(ObservationId::new(
                    "obs:sha256:missing",
                ))),
        )
        .unwrap_err();

        assert!(error.to_string().contains("unknown observation target"));
    }

    #[test]
    fn large_intervention_body_is_stored_as_internal_body_artifact() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let body = "x".repeat(crate::session::body_artifact::BODY_INLINE_LIMIT + 1);

        let result = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired)
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

        let artifacts = std::fs::read_dir(repo.path().join(".shore/artifacts/notes"))
            .unwrap()
            .collect::<Vec<_>>();
        assert_eq!(artifacts.len(), 1);
    }

    #[test]
    fn explicit_same_idempotency_key_with_different_payload_conflicts() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("First")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired)
                .with_idempotency_key("retry-key"),
        )
        .unwrap();
        let error = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Second")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired)
                .with_idempotency_key("retry-key"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("event conflict"));
    }

    #[test]
    fn list_interventions_defaults_to_open_interventions() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = request_intervention(open_request(repo.path(), "First")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = request_intervention(open_request(repo.path(), "Second")).unwrap();

        let result = list_interventions(InterventionListOptions::new(repo.path())).unwrap();

        let ids = result
            .interventions
            .iter()
            .map(|view| view.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![first.intervention_id, second.intervention_id]);
    }

    #[test]
    fn list_interventions_collapses_duplicate_requests() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = request_intervention(
            open_request(repo.path(), "Need approval")
                .with_body("same body")
                .with_idempotency_key("retry-a"),
        )
        .unwrap();
        let second = request_intervention(
            open_request(repo.path(), "Need approval")
                .with_body("same body")
                .with_idempotency_key("retry-b"),
        )
        .unwrap();

        let result = list_interventions(
            InterventionListOptions::new(repo.path())
                .with_status(InterventionStatusFilter::All)
                .with_include_body(true),
        )
        .unwrap();

        assert_eq!(first.intervention_id, second.intervention_id);
        assert_eq!(first.events_created, 1);
        assert_eq!(second.events_created, 1);
        assert_eq!(result.interventions.len(), 1);
        assert_eq!(result.interventions[0].id, first.intervention_id);
        assert_eq!(result.interventions[0].body.as_deref(), Some("same body"));
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.code
                == crate::session::state::DUPLICATE_SEMANTIC_INTERVENTION_REQUEST_EVENT_CODE
        }));
    }

    #[test]
    fn fetch_intervention_hydrates_body_by_id() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = request_intervention(
            open_request(repo.path(), "Need details").with_body("full request body"),
        )
        .unwrap();

        let result = fetch_intervention(
            InterventionFetchOptions::new(repo.path(), requested.intervention_id.clone())
                .with_include_body(true),
        )
        .unwrap();

        assert_eq!(result.intervention.id, requested.intervention_id);
        assert_eq!(
            result.intervention.body.as_deref(),
            Some("full request body")
        );
    }

    #[test]
    fn fetch_intervention_collapses_duplicate_requests() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = request_intervention(
            open_request(repo.path(), "Need approval")
                .with_body("same body")
                .with_idempotency_key("retry-a"),
        )
        .unwrap();
        let second = request_intervention(
            open_request(repo.path(), "Need approval")
                .with_body("same body")
                .with_idempotency_key("retry-b"),
        )
        .unwrap();

        let result = fetch_intervention(
            InterventionFetchOptions::new(repo.path(), first.intervention_id.clone())
                .with_include_body(true),
        )
        .unwrap();

        assert_eq!(first.intervention_id, second.intervention_id);
        assert_eq!(result.intervention.id, first.intervention_id);
        assert_eq!(result.intervention.body.as_deref(), Some("same body"));
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.code
                == crate::session::state::DUPLICATE_SEMANTIC_INTERVENTION_REQUEST_EVENT_CODE
        }));
    }

    #[test]
    fn list_interventions_all_includes_resolved_and_ambiguous_statuses() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let open = request_intervention(open_request(repo.path(), "Open")).unwrap();
        let resolved = request_intervention(open_request(repo.path(), "Resolved")).unwrap();
        let ambiguous = request_intervention(open_request(repo.path(), "Ambiguous")).unwrap();
        write_resolution_event(
            repo.path(),
            &resolved,
            "approved",
            InterventionResolutionOutcome::Approved,
        );
        write_resolution_event(
            repo.path(),
            &ambiguous,
            "approved",
            InterventionResolutionOutcome::Approved,
        );
        write_resolution_event(
            repo.path(),
            &ambiguous,
            "rejected",
            InterventionResolutionOutcome::Rejected,
        );

        let result = list_interventions(
            InterventionListOptions::new(repo.path()).with_status(InterventionStatusFilter::All),
        )
        .unwrap();

        assert_status(&result, &open.intervention_id, InterventionStatus::Open);
        assert_status(
            &result,
            &resolved.intervention_id,
            InterventionStatus::Resolved,
        );
        assert_status(
            &result,
            &ambiguous.intervention_id,
            InterventionStatus::Ambiguous,
        );
    }

    #[test]
    fn list_interventions_filters_by_track_mode_and_file() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let matching = request_intervention(
            open_request(repo.path(), "Match")
                .with_track("agent:codex")
                .with_mode(InterventionMode::Advisory)
                .with_target(InterventionTargetSelector::file("src/lib.rs")),
        )
        .unwrap();
        request_intervention(
            open_request(repo.path(), "Wrong track")
                .with_track("agent:claude")
                .with_mode(InterventionMode::Advisory)
                .with_target(InterventionTargetSelector::file("src/lib.rs")),
        )
        .unwrap();
        request_intervention(
            open_request(repo.path(), "Wrong mode")
                .with_track("agent:codex")
                .with_mode(InterventionMode::Blocking)
                .with_target(InterventionTargetSelector::file("src/lib.rs")),
        )
        .unwrap();

        let result = list_interventions(
            InterventionListOptions::new(repo.path())
                .with_track("agent:codex")
                .with_mode(InterventionMode::Advisory)
                .with_file("src/lib.rs"),
        )
        .unwrap();

        assert_eq!(result.interventions.len(), 1);
        assert_eq!(result.interventions[0].id, matching.intervention_id);
    }

    #[test]
    fn list_interventions_include_body_hydrates_artifact_backed_bodies() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let body = "large ".repeat(1000);
        let requested =
            request_intervention(open_request(repo.path(), "Large body").with_body(body.clone()))
                .unwrap();

        let result =
            list_interventions(InterventionListOptions::new(repo.path()).with_include_body(true))
                .unwrap();

        let view = result
            .interventions
            .iter()
            .find(|view| view.id == requested.intervention_id)
            .unwrap();
        assert_eq!(view.body.as_deref(), Some(body.as_str()));
        assert!(
            !format!("{result:?}").contains("artifacts/notes/"),
            "list result must not expose internal artifact paths"
        );
    }

    #[test]
    fn fetch_intervention_rejects_unknown_id() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let error = fetch_intervention(InterventionFetchOptions::new(
            repo.path(),
            InterventionId::new("intervention:sha256:missing"),
        ))
        .unwrap_err();

        assert!(error.to_string().contains("unknown intervention"));
    }

    #[test]
    fn resolved_intervention_view_includes_resolution_details() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = request_intervention(open_request(repo.path(), "Resolved")).unwrap();
        let resolution_event_id = write_resolution_event(
            repo.path(),
            &requested,
            "approved",
            InterventionResolutionOutcome::Approved,
        );

        let result = fetch_intervention(InterventionFetchOptions::new(
            repo.path(),
            requested.intervention_id.clone(),
        ))
        .unwrap();

        assert_eq!(result.intervention.status, InterventionStatus::Resolved);
        assert_eq!(result.intervention.resolutions.len(), 1);
        assert_eq!(
            result.intervention.resolutions[0].event_id,
            resolution_event_id
        );
        assert_eq!(
            result.intervention.resolutions[0].outcome,
            InterventionResolutionOutcome::Approved
        );
    }

    #[test]
    fn multiple_resolution_events_make_intervention_ambiguous() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = request_intervention(open_request(repo.path(), "Ambiguous")).unwrap();
        write_resolution_event(
            repo.path(),
            &requested,
            "approved",
            InterventionResolutionOutcome::Approved,
        );
        write_resolution_event(
            repo.path(),
            &requested,
            "rejected",
            InterventionResolutionOutcome::Rejected,
        );

        let result = fetch_intervention(InterventionFetchOptions::new(
            repo.path(),
            requested.intervention_id,
        ))
        .unwrap();

        assert_eq!(result.intervention.status, InterventionStatus::Ambiguous);
        assert_eq!(result.intervention.resolutions.len(), 2);
    }

    #[test]
    fn duplicate_resolution_events_do_not_make_intervention_ambiguous() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = request_intervention(open_request(repo.path(), "Need approval")).unwrap();
        let first = resolve_intervention(
            InterventionResolveOptions::new(repo.path(), requested.intervention_id.clone())
                .with_outcome(InterventionResolutionOutcome::Approved)
                .with_reason("approved locally")
                .with_idempotency_key("retry-a"),
        )
        .unwrap();
        let second = resolve_intervention(
            InterventionResolveOptions::new(repo.path(), requested.intervention_id.clone())
                .with_outcome(InterventionResolutionOutcome::Approved)
                .with_reason("approved locally")
                .with_idempotency_key("retry-b"),
        )
        .unwrap();

        let result = fetch_intervention(InterventionFetchOptions::new(
            repo.path(),
            requested.intervention_id,
        ))
        .unwrap();

        assert_eq!(
            first.intervention_resolution_id,
            second.intervention_resolution_id
        );
        assert_eq!(first.events_created, 1);
        assert_eq!(second.events_created, 1);
        assert_eq!(result.intervention.status, InterventionStatus::Resolved);
        assert_eq!(result.intervention.resolutions.len(), 1);
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.code
                == crate::session::state::DUPLICATE_SEMANTIC_INTERVENTION_RESOLUTION_EVENT_CODE
        }));
    }

    #[test]
    fn collect_intervention_projection_records_is_order_independent_and_collapses_duplicates() {
        let session_id = SessionId::new("session:default");
        let review_unit_id = ReviewUnitId::new("review-unit:sha256:test");
        let track_id = TrackId::new("human:kevin");
        let intervention_id = InterventionId::new("intervention:sha256:alpha");
        let approve_resolution_id =
            InterventionResolutionId::new("intervention-resolution:sha256:approve");
        let reject_resolution_id =
            InterventionResolutionId::new("intervention-resolution:sha256:reject");

        let request_a = projection_request_event(
            &session_id,
            &review_unit_id,
            &track_id,
            &intervention_id,
            "retry-a",
            "2026-01-01T00:00:00Z",
        );
        let request_b = projection_request_event(
            &session_id,
            &review_unit_id,
            &track_id,
            &intervention_id,
            "retry-b",
            "2026-01-01T00:00:00Z",
        );
        let approve_x = projection_resolution_event(
            &session_id,
            &review_unit_id,
            &track_id,
            &intervention_id,
            &approve_resolution_id,
            "retry-x",
            "2026-01-02T00:00:00Z",
        );
        let approve_y = projection_resolution_event(
            &session_id,
            &review_unit_id,
            &track_id,
            &intervention_id,
            &approve_resolution_id,
            "retry-y",
            "2026-01-02T00:00:00Z",
        );
        let reject_z = projection_resolution_event(
            &session_id,
            &review_unit_id,
            &track_id,
            &intervention_id,
            &reject_resolution_id,
            "retry-z",
            "2026-01-03T00:00:00Z",
        );

        let lowest_request_event_id =
            std::cmp::min(request_a.event_id.as_str(), request_b.event_id.as_str()).to_owned();
        let lowest_approve_event_id =
            std::cmp::min(approve_x.event_id.as_str(), approve_y.event_id.as_str()).to_owned();

        let forward = vec![
            request_a.clone(),
            request_b.clone(),
            approve_x.clone(),
            approve_y.clone(),
            reject_z.clone(),
        ];
        let reverse: Vec<ShoreEvent> = forward.iter().rev().cloned().collect();

        let forward_records = collect_intervention_projection_records(&forward).unwrap();
        let reverse_records = collect_intervention_projection_records(&reverse).unwrap();

        assert_eq!(forward_records.request_records.len(), 1);
        assert_eq!(
            forward_records.request_records[&intervention_id]
                .event
                .event_id
                .as_str(),
            lowest_request_event_id,
        );
        assert_eq!(
            reverse_records.request_records[&intervention_id]
                .event
                .event_id
                .as_str(),
            lowest_request_event_id,
        );

        let forward_resolutions = &forward_records.resolutions[&intervention_id];
        let reverse_resolutions = &reverse_records.resolutions[&intervention_id];
        assert_eq!(forward_resolutions, reverse_resolutions);
        assert_eq!(forward_resolutions.len(), 2);

        let approve_view = forward_resolutions
            .iter()
            .find(|view| view.id == approve_resolution_id)
            .unwrap();
        assert_eq!(approve_view.event_id.as_str(), lowest_approve_event_id);
    }

    #[test]
    fn sort_intervention_views_uses_created_at_then_event_id() {
        let mut views = vec![
            intervention_view_for_sort("intervention:sha256:b", "evt:sha256:b", "unix-ms:2"),
            intervention_view_for_sort("intervention:sha256:c", "evt:sha256:c", "unix-ms:1"),
            intervention_view_for_sort("intervention:sha256:a", "evt:sha256:a", "unix-ms:1"),
        ];

        sort_intervention_views(&mut views);

        assert_eq!(
            views
                .iter()
                .map(|view| view.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "intervention:sha256:a",
                "intervention:sha256:c",
                "intervention:sha256:b"
            ]
        );
    }

    #[test]
    fn resolve_intervention_records_resolution_and_closes_open_count() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = request_intervention(open_request(repo.path(), "Need approval")).unwrap();

        let resolved = resolve_intervention(
            InterventionResolveOptions::new(repo.path(), requested.intervention_id.clone())
                .with_outcome(InterventionResolutionOutcome::Approved)
                .with_reason("approved locally"),
        )
        .unwrap();

        assert!(
            resolved
                .intervention_resolution_id
                .as_str()
                .starts_with("intervention-resolution:sha256:")
        );
        assert_eq!(resolved.outcome, InterventionResolutionOutcome::Approved);
        assert_eq!(resolved.events_created_by_type["intervention_resolved"], 1);

        let state = SessionState::from_events(
            &EventStore::open(repo.path().join(".shore"))
                .list_events()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(state.intervention_count, 1);
        assert_eq!(state.open_intervention_count, 0);
        assert_eq!(state.open_blocking_intervention_count, 0);
    }

    #[test]
    fn resolving_unknown_intervention_fails() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let error = resolve_intervention(
            InterventionResolveOptions::new(
                repo.path(),
                InterventionId::new("intervention:sha256:missing"),
            )
            .with_outcome(InterventionResolutionOutcome::Dismissed),
        )
        .unwrap_err();

        assert!(error.to_string().contains("unknown intervention"));
    }

    #[test]
    fn resolve_intervention_is_idempotent_for_same_logical_input() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = request_intervention(open_request(repo.path(), "Need approval")).unwrap();
        let options =
            InterventionResolveOptions::new(repo.path(), requested.intervention_id.clone())
                .with_outcome(InterventionResolutionOutcome::Approved)
                .with_reason("approved locally");

        let first = resolve_intervention(options.clone()).unwrap();
        let second = resolve_intervention(options).unwrap();

        assert_eq!(
            first.intervention_resolution_id,
            second.intervention_resolution_id
        );
        assert_eq!(first.events_created, 1);
        assert_eq!(second.events_created, 0);
        assert_eq!(second.events_existing, 1);
    }

    #[test]
    fn explicit_same_resolution_idempotency_key_with_different_payload_conflicts() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = request_intervention(open_request(repo.path(), "Need approval")).unwrap();

        resolve_intervention(
            InterventionResolveOptions::new(repo.path(), requested.intervention_id.clone())
                .with_outcome(InterventionResolutionOutcome::Approved)
                .with_idempotency_key("retry-key"),
        )
        .unwrap();
        let error = resolve_intervention(
            InterventionResolveOptions::new(repo.path(), requested.intervention_id.clone())
                .with_outcome(InterventionResolutionOutcome::Rejected)
                .with_idempotency_key("retry-key"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("event conflict"));
    }

    #[test]
    fn resolving_twice_with_different_outcomes_is_append_only_and_ambiguous() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = request_intervention(open_request(repo.path(), "Need approval")).unwrap();

        resolve_intervention(
            InterventionResolveOptions::new(repo.path(), requested.intervention_id.clone())
                .with_outcome(InterventionResolutionOutcome::Approved),
        )
        .unwrap();
        resolve_intervention(
            InterventionResolveOptions::new(repo.path(), requested.intervention_id.clone())
                .with_outcome(InterventionResolutionOutcome::Rejected),
        )
        .unwrap();

        let result = fetch_intervention(InterventionFetchOptions::new(
            repo.path(),
            requested.intervention_id,
        ))
        .unwrap();

        assert_eq!(result.intervention.status, InterventionStatus::Ambiguous);
        assert_eq!(result.intervention.resolutions.len(), 2);
    }

    #[test]
    fn large_resolution_reason_is_stored_as_internal_body_artifact() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = request_intervention(open_request(repo.path(), "Need approval")).unwrap();
        let reason = "resolved ".repeat(1000);

        let result = resolve_intervention(
            InterventionResolveOptions::new(repo.path(), requested.intervention_id)
                .with_outcome(InterventionResolutionOutcome::Approved)
                .with_reason(reason),
        )
        .unwrap();

        assert!(
            result
                .reason_content_hash
                .as_deref()
                .unwrap()
                .starts_with("sha256:")
        );
        assert!(
            !format!("{result:?}").contains("artifacts/notes/"),
            "resolve result must not expose internal artifact paths"
        );
    }

    fn modified_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 {\n    1\n}\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 {\n    2\n}\n");
        repo
    }

    fn open_request(repo: &Path, title: &str) -> InterventionRequestOptions {
        InterventionRequestOptions::new(repo)
            .with_track("human:kevin")
            .with_title(title)
            .with_reason_code(InterventionReasonCode::ManualDecisionRequired)
    }

    fn write_resolution_event(
        repo: &Path,
        requested: &InterventionRequestResult,
        source_key: &str,
        outcome: InterventionResolutionOutcome,
    ) -> EventId {
        let resolution_id_material = format!("{}:{source_key}", requested.intervention_id.as_str());
        let resolution_id = InterventionResolutionId::new(format!(
            "intervention-resolution:sha256:{}",
            sha256_bytes_hex(resolution_id_material.as_bytes())
        ));
        let payload = InterventionResolvedPayload {
            intervention_resolution_id: resolution_id,
            intervention_id: requested.intervention_id.clone(),
            outcome,
            reason: Some("resolved".to_owned()),
            reason_artifact_path: None,
            reason_byte_size: Some(8),
            reason_content_hash: Some("sha256:resolved".to_owned()),
        };
        let event = ShoreEvent::new(
            EventType::InterventionResolved,
            InterventionResolvedPayload::idempotency_key(&requested.intervention_id, source_key),
            EventTarget {
                session_id: SessionId::new("session:default"),
                work_unit_id: None,
                work_object_id: None,
                work_object_type: None,
                review_unit_id: Some(requested.review_unit_id.clone()),
                revision_id: None,
                snapshot_id: None,
                track_id: Some(requested.track_id.clone()),
                subject: Some(TargetRef::Review(ReviewTargetRef::Intervention {
                    review_unit_id: requested.review_unit_id.clone(),
                    intervention_id: requested.intervention_id.clone(),
                })),
            },
            Writer::shore_local_reviewer("test"),
            payload,
            current_timestamp(),
        )
        .unwrap();
        let event_id = event.event_id.clone();
        EventStore::open(repo.join(".shore"))
            .record_event_once(&event)
            .unwrap();
        event_id
    }

    fn projection_request_event(
        session_id: &SessionId,
        review_unit_id: &ReviewUnitId,
        track_id: &TrackId,
        intervention_id: &InterventionId,
        source_key: &str,
        occurred_at: &str,
    ) -> ShoreEvent {
        let payload = InterventionRequestedPayload {
            intervention_id: intervention_id.clone(),
            target: ReviewTargetRef::ReviewUnit {
                review_unit_id: review_unit_id.clone(),
            },
            mode: InterventionMode::Advisory,
            reason_code: InterventionReasonCode::ManualDecisionRequired,
            title: "projection".to_owned(),
            body: None,
            body_artifact_path: None,
            body_byte_size: None,
            body_content_hash: None,
        };
        ShoreEvent::new(
            EventType::InterventionRequested,
            InterventionRequestedPayload::idempotency_key(review_unit_id, track_id, source_key),
            EventTarget {
                session_id: session_id.clone(),
                work_unit_id: None,
                work_object_id: None,
                work_object_type: None,
                review_unit_id: Some(review_unit_id.clone()),
                revision_id: None,
                snapshot_id: None,
                track_id: Some(track_id.clone()),
                subject: Some(TargetRef::Review(ReviewTargetRef::ReviewUnit {
                    review_unit_id: review_unit_id.clone(),
                })),
            },
            Writer::shore_local_reviewer("test"),
            payload,
            occurred_at.to_owned(),
        )
        .unwrap()
    }

    fn projection_resolution_event(
        session_id: &SessionId,
        review_unit_id: &ReviewUnitId,
        track_id: &TrackId,
        intervention_id: &InterventionId,
        resolution_id: &InterventionResolutionId,
        source_key: &str,
        occurred_at: &str,
    ) -> ShoreEvent {
        let payload = InterventionResolvedPayload {
            intervention_resolution_id: resolution_id.clone(),
            intervention_id: intervention_id.clone(),
            outcome: InterventionResolutionOutcome::Approved,
            reason: None,
            reason_artifact_path: None,
            reason_byte_size: None,
            reason_content_hash: None,
        };
        ShoreEvent::new(
            EventType::InterventionResolved,
            InterventionResolvedPayload::idempotency_key(intervention_id, source_key),
            EventTarget {
                session_id: session_id.clone(),
                work_unit_id: None,
                work_object_id: None,
                work_object_type: None,
                review_unit_id: Some(review_unit_id.clone()),
                revision_id: None,
                snapshot_id: None,
                track_id: Some(track_id.clone()),
                subject: Some(TargetRef::Review(ReviewTargetRef::Intervention {
                    review_unit_id: review_unit_id.clone(),
                    intervention_id: intervention_id.clone(),
                })),
            },
            Writer::shore_local_reviewer("test"),
            payload,
            occurred_at.to_owned(),
        )
        .unwrap()
    }

    fn assert_status(
        result: &InterventionListResult,
        id: &InterventionId,
        status: InterventionStatus,
    ) {
        let view = result
            .interventions
            .iter()
            .find(|view| &view.id == id)
            .unwrap();
        assert_eq!(view.status, status);
    }

    fn intervention_view_for_sort(
        intervention_id: &str,
        event_id: &str,
        created_at: &str,
    ) -> InterventionView {
        let review_unit_id = ReviewUnitId::new("review-unit:sha256:one");
        InterventionView {
            id: InterventionId::new(intervention_id),
            event_id: EventId::new(event_id),
            track_id: TrackId::new("agent:codex"),
            target: ReviewTargetRef::ReviewUnit { review_unit_id },
            mode: InterventionMode::Blocking,
            reason_code: InterventionReasonCode::ManualDecisionRequired,
            title: "sort".to_owned(),
            body: None,
            body_content_hash: None,
            status: InterventionStatus::Open,
            resolutions: vec![],
            created_at: created_at.to_owned(),
            writer: Writer::shore_local_reviewer("test"),
        }
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
