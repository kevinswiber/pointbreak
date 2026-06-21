mod fetch;
mod list;
mod open;
mod respond;
mod target;
mod view;

pub use self::fetch::{InputRequestFetchOptions, InputRequestFetchResult, fetch_input_request};
pub use self::list::{InputRequestListOptions, InputRequestListResult, list_input_requests};
pub use self::open::{InputRequestOpenOptions, InputRequestOpenResult, open_input_request};
pub use self::respond::{
    InputRequestRespondOptions, InputRequestRespondResult, respond_input_request,
};
pub use self::target::InputRequestTargetSelector;
#[cfg(test)]
use self::view::collect_input_request_projection_records;
#[cfg(test)]
use self::view::sort_input_request_views;
pub(crate) use self::view::{InputRequestProjectionOptions, project_input_requests};
pub use self::view::{
    InputRequestResponseView, InputRequestStatus, InputRequestStatusFilter, InputRequestView,
};
#[cfg(test)]
use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
#[cfg(test)]
use crate::model::{
    EventId, InputRequestId, InputRequestResponseId, LedgerId, RevisionId, TrackId,
};
#[cfg(test)]
use crate::session::current_timestamp;
#[cfg(test)]
use crate::session::event::{
    AssertionMode, EventTarget, EventType, InputRequestOpenedPayload, InputRequestReasonCode,
    InputRequestRespondedPayload, InputRequestResponseOutcome, ShoreEvent, Writer,
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
    fn input_request_status_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(InputRequestStatus::Open).unwrap(),
            serde_json::json!("open")
        );
        assert_eq!(
            serde_json::to_value(InputRequestStatus::Responded).unwrap(),
            serde_json::json!("responded")
        );
        assert_eq!(
            serde_json::to_value(InputRequestStatus::Ambiguous).unwrap(),
            serde_json::json!("ambiguous")
        );
        // The Serialize output must match the as_str() contract.
        for status in [
            InputRequestStatus::Open,
            InputRequestStatus::Responded,
            InputRequestStatus::Ambiguous,
        ] {
            assert_eq!(
                serde_json::to_value(status).unwrap(),
                serde_json::json!(status.as_str())
            );
        }
    }

    #[test]
    fn open_with_actor_id_attributes_override_and_changes_derived_id() {
        use crate::model::ActorId;

        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let with_a = open_input_request(
            open_request(repo.path(), "Need approval")
                .with_actor_id(ActorId::new("actor:agent:opener-a")),
        )
        .unwrap();
        let with_b = open_input_request(
            open_request(repo.path(), "Need approval")
                .with_actor_id(ActorId::new("actor:agent:opener-b")),
        )
        .unwrap();

        // The override flows into the content-addressed input-request id.
        assert_ne!(with_a.input_request_id, with_b.input_request_id);

        let opened = input_request_opened_events(repo.path());
        let actor_for = |id: &InputRequestId| {
            opened
                .iter()
                .find(|event| event.payload["inputRequestId"] == serde_json::json!(id.as_str()))
                .map(|event| event.writer.actor_id.as_str().to_owned())
                .unwrap()
        };
        assert_eq!(actor_for(&with_a.input_request_id), "actor:agent:opener-a");
        assert_eq!(actor_for(&with_b.input_request_id), "actor:agent:opener-b");
    }

    #[test]
    fn respond_with_actor_id_attributes_override_and_changes_derived_id() {
        use crate::model::ActorId;

        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let request = open_input_request(open_request(repo.path(), "Need approval")).unwrap();

        let with_a = respond_input_request(
            InputRequestRespondOptions::new(repo.path(), request.input_request_id.clone())
                .with_outcome(InputRequestResponseOutcome::Approved)
                .with_actor_id(ActorId::new("actor:agent:reviewer-a")),
        )
        .unwrap();
        let with_b = respond_input_request(
            InputRequestRespondOptions::new(repo.path(), request.input_request_id.clone())
                .with_outcome(InputRequestResponseOutcome::Approved)
                .with_actor_id(ActorId::new("actor:agent:reviewer-b")),
        )
        .unwrap();

        // The override flows into the content-addressed response id.
        assert_ne!(
            with_a.input_request_response_id,
            with_b.input_request_response_id
        );

        // Each durable event credits the chosen actor.
        let responded = responded_events(repo.path());
        let actor_for = |id: &InputRequestResponseId| {
            responded
                .iter()
                .find(|event| {
                    event.payload["inputRequestResponseId"] == serde_json::json!(id.as_str())
                })
                .map(|event| event.writer.actor_id.as_str().to_owned())
                .unwrap()
        };
        assert_eq!(
            actor_for(&with_a.input_request_response_id),
            "actor:agent:reviewer-a"
        );
        assert_eq!(
            actor_for(&with_b.input_request_response_id),
            "actor:agent:reviewer-b"
        );
    }

    #[test]
    fn respond_without_actor_id_uses_git_identity() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let request = open_input_request(open_request(repo.path(), "Need approval")).unwrap();

        respond_input_request(
            InputRequestRespondOptions::new(repo.path(), request.input_request_id)
                .with_outcome(InputRequestResponseOutcome::Approved),
        )
        .unwrap();

        let responded = responded_events(repo.path());
        assert_eq!(responded.len(), 1);
        // modified_repo() configures user.email shore-tests@example.com.
        assert_eq!(
            responded[0].writer.actor_id.as_str(),
            "actor:git-email:shore-tests@example.com"
        );
    }

    #[test]
    fn open_input_request_writes_event_and_updates_state() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired)
                .with_assertion_mode(AssertionMode::Operative)
                .with_target(InputRequestTargetSelector::review_unit()),
        )
        .unwrap();

        assert_eq!(result.review_unit_id, capture.revision_id);
        assert!(
            result
                .input_request_id
                .as_str()
                .starts_with("input-request:sha256:")
        );
        assert_eq!(result.assertion_mode, AssertionMode::Operative);
        assert_eq!(
            result.reason_code,
            InputRequestReasonCode::ManualDecisionRequired
        );
        assert_eq!(result.track_id.as_str(), "human:kevin");
        assert_eq!(result.events_created, 1);
        assert_eq!(result.events_existing, 0);
        assert_eq!(result.events_created_by_type["input_request_opened"], 1);

        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let state = SessionState::from_events(&events).unwrap();
        assert_eq!(state.input_request_count, 1);
        assert_eq!(state.open_input_request_count, 1);
        assert_eq!(state.open_operative_input_request_count, 1);
    }

    #[test]
    fn open_input_request_defaults_to_operative_envelope_assertion_mode() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired),
        )
        .unwrap();

        assert_eq!(result.assertion_mode, AssertionMode::Operative);

        let event = only_input_request_opened_event(repo.path());
        assert_eq!(event.assertion_mode, AssertionMode::Operative);
        assert!(event.payload.get("mode").is_none());
    }

    #[test]
    fn open_input_request_can_write_advisory_envelope_assertion_mode() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired),
        )
        .unwrap();

        let advisory = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Heads up")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired)
                .with_assertion_mode(AssertionMode::Advisory),
        )
        .unwrap();

        assert_eq!(advisory.assertion_mode, AssertionMode::Advisory);

        let events = input_request_opened_events(repo.path());
        assert_eq!(events.len(), 2, "{events:?}");

        let default_event = events
            .iter()
            .find(|event| event.payload["title"] == "Need approval")
            .unwrap();
        assert_eq!(default_event.assertion_mode, AssertionMode::Operative);
        assert!(default_event.payload.get("mode").is_none());

        let advisory_event = events
            .iter()
            .find(|event| event.payload["title"] == "Heads up")
            .unwrap();
        assert_eq!(advisory_event.assertion_mode, AssertionMode::Advisory);
        assert!(advisory_event.payload.get("mode").is_none());
    }

    #[test]
    fn open_input_request_id_material_uses_assertion_mode() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired),
        )
        .unwrap();
        let event = only_input_request_opened_event(repo.path());

        let expected_digest = sha256_json_prefixed(&serde_json::json!({
            "reviewUnitId": result.review_unit_id.as_str(),
            "trackId": result.track_id.as_str(),
            "target": result.target.clone(),
            "assertionMode": AssertionMode::Operative,
            "reasonCode": InputRequestReasonCode::ManualDecisionRequired,
            "title": "Need approval",
            "bodyContentHash": result.body_content_hash.as_deref(),
            "writerActorId": event.writer.actor_id.as_str(),
        }))
        .unwrap();

        assert_eq!(
            result.input_request_id,
            InputRequestId::new(format!("input-request:{expected_digest}"))
        );
    }

    #[test]
    fn open_input_request_is_idempotent_for_same_logical_input() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let options = InputRequestOpenOptions::new(repo.path())
            .with_track("agent:codex")
            .with_title("Pause")
            .with_reason_code(InputRequestReasonCode::UnsafeAction)
            .with_body("same body");

        let first = open_input_request(options.clone()).unwrap();
        let second = open_input_request(options).unwrap();

        assert_eq!(first.input_request_id, second.input_request_id);
        assert_eq!(first.events_created, 1);
        assert_eq!(second.events_created, 0);
        assert_eq!(second.events_existing, 1);
    }

    #[test]
    fn open_input_request_state_json_equals_full_replay_after_created_and_existing_paths() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let options = InputRequestOpenOptions::new(repo.path())
            .with_track("agent:codex")
            .with_title("operative-finding")
            .with_assertion_mode(AssertionMode::Operative)
            .with_reason_code(InputRequestReasonCode::UnsafeAction);

        let first = open_input_request(options.clone()).unwrap();
        assert_eq!(first.events_created, 1);
        let on_disk: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(resolved_store_dir(repo.path()).join("state.json")).unwrap(),
        )
        .unwrap();
        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let replay = serde_json::to_value(SessionState::from_events(&events).unwrap()).unwrap();
        assert_eq!(on_disk, replay, "Created path drifted");

        let second = open_input_request(options).unwrap();
        assert_eq!(second.events_existing, 1);
        let on_disk: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(resolved_store_dir(repo.path()).join("state.json")).unwrap(),
        )
        .unwrap();
        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let replay = serde_json::to_value(SessionState::from_events(&events).unwrap()).unwrap();
        assert_eq!(on_disk, replay, "Existing path drifted");
    }

    #[test]
    fn respond_input_request_state_json_equals_full_replay_after_created_and_existing_paths() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let request = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("to-resolve")
                .with_assertion_mode(AssertionMode::Operative)
                .with_reason_code(InputRequestReasonCode::UnsafeAction),
        )
        .unwrap();

        let options =
            InputRequestRespondOptions::new(repo.path(), request.input_request_id.clone())
                .with_outcome(InputRequestResponseOutcome::Approved);

        let first = respond_input_request(options.clone()).unwrap();
        assert_eq!(first.events_created, 1);
        let on_disk: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(resolved_store_dir(repo.path()).join("state.json")).unwrap(),
        )
        .unwrap();
        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let replay = serde_json::to_value(SessionState::from_events(&events).unwrap()).unwrap();
        assert_eq!(on_disk, replay, "Resolve Created path drifted");

        let second = respond_input_request(options).unwrap();
        assert_eq!(second.events_existing, 1);
        let on_disk: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(resolved_store_dir(repo.path()).join("state.json")).unwrap(),
        )
        .unwrap();
        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let replay = serde_json::to_value(SessionState::from_events(&events).unwrap()).unwrap();
        assert_eq!(on_disk, replay, "Resolve Existing path drifted");
    }

    #[test]
    fn open_input_request_requires_a_captured_review_unit() {
        let repo = modified_repo();

        let error = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired),
        )
        .unwrap_err();

        assert!(error.to_string().contains("no captured review unit"));
    }

    #[test]
    fn open_input_request_requires_explicit_review_unit_when_current_is_ambiguous() {
        let repo = modified_repo();
        let first = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        repo.write("src/lib.rs", "pub fn value() -> u32 {\n    3\n}\n");
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let ambiguous = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired),
        )
        .unwrap_err();
        assert!(
            ambiguous
                .to_string()
                .contains("multiple captured review units")
        );

        let explicit = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_review_unit_id(first.revision_id.clone())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired),
        )
        .unwrap();
        assert_eq!(explicit.review_unit_id, first.revision_id);
    }

    #[test]
    fn open_input_request_rejects_invalid_track() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let error = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("system:shore")
                .with_title("Need approval")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired),
        )
        .unwrap_err();

        assert!(error.to_string().contains("track namespace is reserved"));
    }

    #[test]
    fn open_input_request_rejects_file_target_absent_from_snapshot() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let error = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired)
                .with_target(InputRequestTargetSelector::file("missing.rs")),
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("not present in captured snapshot")
        );
    }

    #[test]
    fn open_input_request_supports_file_range_targets() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired)
                .with_target(InputRequestTargetSelector::range(
                    "src/lib.rs",
                    Side::New,
                    2,
                    Some(2),
                )),
        )
        .unwrap();

        assert_eq!(result.review_unit_id, capture.revision_id);
        assert!(matches!(
            result.target,
            ReviewTargetRef::Range { start_line: 2, .. }
        ));
    }

    #[test]
    fn open_input_request_validates_native_observation_target() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let observation = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Observation"),
        )
        .unwrap();

        let result = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired)
                .with_target(InputRequestTargetSelector::observation(
                    observation.observation_id.clone(),
                )),
        )
        .unwrap();

        assert_eq!(
            result.target,
            ReviewTargetRef::Observation {
                revision_id: capture.revision_id,
                observation_id: observation.observation_id,
            }
        );
    }

    #[test]
    fn open_input_request_rejects_unknown_observation_target() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let error = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired)
                .with_target(InputRequestTargetSelector::observation(ObservationId::new(
                    "obs:sha256:missing",
                ))),
        )
        .unwrap_err();

        assert!(error.to_string().contains("unknown observation target"));
    }

    #[test]
    fn large_input_request_body_is_stored_as_internal_body_artifact() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let body = "x".repeat(crate::session::body_artifact::BODY_INLINE_LIMIT + 1);

        let result = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired)
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

        let artifacts = std::fs::read_dir(resolved_store_dir(repo.path()).join("artifacts/notes"))
            .unwrap()
            .collect::<Vec<_>>();
        assert_eq!(artifacts.len(), 1);
    }

    #[test]
    fn explicit_same_idempotency_key_with_different_payload_conflicts() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("First")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired)
                .with_idempotency_key("retry-key"),
        )
        .unwrap();
        let error = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Second")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired)
                .with_idempotency_key("retry-key"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("event conflict"));
    }

    #[test]
    fn list_input_requests_defaults_to_open_input_requests() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = open_input_request(open_request(repo.path(), "First")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = open_input_request(open_request(repo.path(), "Second")).unwrap();

        let result = list_input_requests(InputRequestListOptions::new(repo.path())).unwrap();

        let ids = result
            .input_requests
            .iter()
            .map(|view| view.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![first.input_request_id, second.input_request_id]);
    }

    #[test]
    fn list_input_requests_collapses_duplicate_requests() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = open_input_request(
            open_request(repo.path(), "Need approval")
                .with_body("same body")
                .with_idempotency_key("retry-a"),
        )
        .unwrap();
        let second = open_input_request(
            open_request(repo.path(), "Need approval")
                .with_body("same body")
                .with_idempotency_key("retry-b"),
        )
        .unwrap();

        let result = list_input_requests(
            InputRequestListOptions::new(repo.path())
                .with_status(InputRequestStatusFilter::All)
                .with_include_body(true),
        )
        .unwrap();

        assert_eq!(first.input_request_id, second.input_request_id);
        assert_eq!(first.events_created, 1);
        assert_eq!(second.events_created, 1);
        assert_eq!(result.input_requests.len(), 1);
        assert_eq!(result.input_requests[0].id, first.input_request_id);
        assert_eq!(result.input_requests[0].body.as_deref(), Some("same body"));
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.code
                == crate::session::state::DUPLICATE_SEMANTIC_INPUT_REQUEST_OPEN_EVENT_CODE
        }));
    }

    #[test]
    fn fetch_input_request_hydrates_body_by_id() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = open_input_request(
            open_request(repo.path(), "Need details").with_body("full request body"),
        )
        .unwrap();

        let result = fetch_input_request(
            InputRequestFetchOptions::new(repo.path(), requested.input_request_id.clone())
                .with_include_body(true),
        )
        .unwrap();

        assert_eq!(result.input_request.id, requested.input_request_id);
        assert_eq!(
            result.input_request.body.as_deref(),
            Some("full request body")
        );
    }

    #[test]
    fn fetch_input_request_collapses_duplicate_requests() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = open_input_request(
            open_request(repo.path(), "Need approval")
                .with_body("same body")
                .with_idempotency_key("retry-a"),
        )
        .unwrap();
        let second = open_input_request(
            open_request(repo.path(), "Need approval")
                .with_body("same body")
                .with_idempotency_key("retry-b"),
        )
        .unwrap();

        let result = fetch_input_request(
            InputRequestFetchOptions::new(repo.path(), first.input_request_id.clone())
                .with_include_body(true),
        )
        .unwrap();

        assert_eq!(first.input_request_id, second.input_request_id);
        assert_eq!(result.input_request.id, first.input_request_id);
        assert_eq!(result.input_request.body.as_deref(), Some("same body"));
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.code
                == crate::session::state::DUPLICATE_SEMANTIC_INPUT_REQUEST_OPEN_EVENT_CODE
        }));
    }

    #[test]
    fn list_input_requests_all_includes_responded_and_ambiguous_statuses() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let open = open_input_request(open_request(repo.path(), "Open")).unwrap();
        let responded = open_input_request(open_request(repo.path(), "Responded")).unwrap();
        let ambiguous = open_input_request(open_request(repo.path(), "Ambiguous")).unwrap();
        write_response_event(
            repo.path(),
            &responded,
            "approved",
            InputRequestResponseOutcome::Approved,
        );
        write_response_event(
            repo.path(),
            &ambiguous,
            "approved",
            InputRequestResponseOutcome::Approved,
        );
        write_response_event(
            repo.path(),
            &ambiguous,
            "rejected",
            InputRequestResponseOutcome::Rejected,
        );

        let result = list_input_requests(
            InputRequestListOptions::new(repo.path()).with_status(InputRequestStatusFilter::All),
        )
        .unwrap();

        assert_status(&result, &open.input_request_id, InputRequestStatus::Open);
        assert_status(
            &result,
            &responded.input_request_id,
            InputRequestStatus::Responded,
        );
        assert_status(
            &result,
            &ambiguous.input_request_id,
            InputRequestStatus::Ambiguous,
        );
    }

    #[test]
    fn list_input_requests_filters_by_track_mode_and_file() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let matching = open_input_request(
            open_request(repo.path(), "Match")
                .with_track("agent:codex")
                .with_assertion_mode(AssertionMode::Advisory)
                .with_target(InputRequestTargetSelector::file("src/lib.rs")),
        )
        .unwrap();
        open_input_request(
            open_request(repo.path(), "Wrong track")
                .with_track("agent:claude")
                .with_assertion_mode(AssertionMode::Advisory)
                .with_target(InputRequestTargetSelector::file("src/lib.rs")),
        )
        .unwrap();
        open_input_request(
            open_request(repo.path(), "Wrong mode")
                .with_track("agent:codex")
                .with_assertion_mode(AssertionMode::Operative)
                .with_target(InputRequestTargetSelector::file("src/lib.rs")),
        )
        .unwrap();

        let result = list_input_requests(
            InputRequestListOptions::new(repo.path())
                .with_track("agent:codex")
                .with_mode(AssertionMode::Advisory)
                .with_file("src/lib.rs"),
        )
        .unwrap();

        assert_eq!(result.input_requests.len(), 1);
        assert_eq!(result.input_requests[0].id, matching.input_request_id);
    }

    #[test]
    fn list_input_requests_include_body_hydrates_artifact_backed_bodies() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let body = "large ".repeat(1000);
        let requested =
            open_input_request(open_request(repo.path(), "Large body").with_body(body.clone()))
                .unwrap();

        let result =
            list_input_requests(InputRequestListOptions::new(repo.path()).with_include_body(true))
                .unwrap();

        let view = result
            .input_requests
            .iter()
            .find(|view| view.id == requested.input_request_id)
            .unwrap();
        assert_eq!(view.body.as_deref(), Some(body.as_str()));
        assert!(
            !format!("{result:?}").contains("artifacts/notes/"),
            "list result must not expose internal artifact paths"
        );
    }

    #[test]
    fn fetch_input_request_rejects_unknown_id() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let error = fetch_input_request(InputRequestFetchOptions::new(
            repo.path(),
            InputRequestId::new("input-request:sha256:missing"),
        ))
        .unwrap_err();

        assert!(error.to_string().contains("unknown input request"));
    }

    #[test]
    fn responded_input_request_view_includes_response_details() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = open_input_request(open_request(repo.path(), "Responded")).unwrap();
        let response_event_id = write_response_event(
            repo.path(),
            &requested,
            "approved",
            InputRequestResponseOutcome::Approved,
        );

        let result = fetch_input_request(InputRequestFetchOptions::new(
            repo.path(),
            requested.input_request_id.clone(),
        ))
        .unwrap();

        assert_eq!(result.input_request.status, InputRequestStatus::Responded);
        assert_eq!(result.input_request.responses.len(), 1);
        assert_eq!(
            result.input_request.responses[0].event_id,
            response_event_id
        );
        assert_eq!(
            result.input_request.responses[0].outcome,
            InputRequestResponseOutcome::Approved
        );
    }

    #[test]
    fn multiple_response_events_make_input_request_ambiguous() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = open_input_request(open_request(repo.path(), "Ambiguous")).unwrap();
        write_response_event(
            repo.path(),
            &requested,
            "approved",
            InputRequestResponseOutcome::Approved,
        );
        write_response_event(
            repo.path(),
            &requested,
            "rejected",
            InputRequestResponseOutcome::Rejected,
        );

        let result = fetch_input_request(InputRequestFetchOptions::new(
            repo.path(),
            requested.input_request_id,
        ))
        .unwrap();

        assert_eq!(result.input_request.status, InputRequestStatus::Ambiguous);
        assert_eq!(result.input_request.responses.len(), 2);
    }

    #[test]
    fn duplicate_response_events_do_not_make_input_request_ambiguous() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = open_input_request(open_request(repo.path(), "Need approval")).unwrap();
        let first = respond_input_request(
            InputRequestRespondOptions::new(repo.path(), requested.input_request_id.clone())
                .with_outcome(InputRequestResponseOutcome::Approved)
                .with_reason("approved locally")
                .with_idempotency_key("retry-a"),
        )
        .unwrap();
        let second = respond_input_request(
            InputRequestRespondOptions::new(repo.path(), requested.input_request_id.clone())
                .with_outcome(InputRequestResponseOutcome::Approved)
                .with_reason("approved locally")
                .with_idempotency_key("retry-b"),
        )
        .unwrap();

        let result = fetch_input_request(InputRequestFetchOptions::new(
            repo.path(),
            requested.input_request_id,
        ))
        .unwrap();

        assert_eq!(
            first.input_request_response_id,
            second.input_request_response_id
        );
        assert_eq!(first.events_created, 1);
        assert_eq!(second.events_created, 1);
        assert_eq!(result.input_request.status, InputRequestStatus::Responded);
        assert_eq!(result.input_request.responses.len(), 1);
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.code
                == crate::session::state::DUPLICATE_SEMANTIC_INPUT_REQUEST_RESPONSE_EVENT_CODE
        }));
    }

    #[test]
    fn collect_input_request_projection_records_is_order_independent_and_collapses_duplicates() {
        let session_id = LedgerId::new("session:default");
        let review_unit_id = RevisionId::new("review-unit:sha256:test");
        let track_id = TrackId::new("human:kevin");
        let input_request_id = InputRequestId::new("input-request:sha256:alpha");
        let approve_response_id =
            InputRequestResponseId::new("input-request-response:sha256:approve");
        let reject_response_id =
            InputRequestResponseId::new("input-request-response:sha256:reject");

        let request_a = projection_request_event(
            &session_id,
            &review_unit_id,
            &track_id,
            &input_request_id,
            "retry-a",
            "2026-01-01T00:00:00Z",
        );
        let request_b = projection_request_event(
            &session_id,
            &review_unit_id,
            &track_id,
            &input_request_id,
            "retry-b",
            "2026-01-01T00:00:00Z",
        );
        let approve_x = projection_response_event(
            &session_id,
            &review_unit_id,
            &track_id,
            &input_request_id,
            &approve_response_id,
            "retry-x",
            "2026-01-02T00:00:00Z",
        );
        let approve_y = projection_response_event(
            &session_id,
            &review_unit_id,
            &track_id,
            &input_request_id,
            &approve_response_id,
            "retry-y",
            "2026-01-02T00:00:00Z",
        );
        let reject_z = projection_response_event(
            &session_id,
            &review_unit_id,
            &track_id,
            &input_request_id,
            &reject_response_id,
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

        let forward_records = collect_input_request_projection_records(&forward).unwrap();
        let reverse_records = collect_input_request_projection_records(&reverse).unwrap();

        assert_eq!(forward_records.request_records.len(), 1);
        assert_eq!(
            forward_records.request_records[&input_request_id]
                .event
                .event_id
                .as_str(),
            lowest_request_event_id,
        );
        assert_eq!(
            reverse_records.request_records[&input_request_id]
                .event
                .event_id
                .as_str(),
            lowest_request_event_id,
        );

        let forward_responses = &forward_records.responses[&input_request_id];
        let reverse_responses = &reverse_records.responses[&input_request_id];
        assert_eq!(forward_responses, reverse_responses);
        assert_eq!(forward_responses.len(), 2);

        let approve_view = forward_responses
            .iter()
            .find(|view| view.id == approve_response_id)
            .unwrap();
        assert_eq!(approve_view.event_id.as_str(), lowest_approve_event_id);
    }

    #[test]
    fn sort_input_request_views_uses_created_at_then_event_id() {
        let mut views = vec![
            input_request_view_for_sort("input-request:sha256:b", "evt:sha256:b", "unix-ms:2"),
            input_request_view_for_sort("input-request:sha256:c", "evt:sha256:c", "unix-ms:1"),
            input_request_view_for_sort("input-request:sha256:a", "evt:sha256:a", "unix-ms:1"),
        ];

        sort_input_request_views(&mut views);

        assert_eq!(
            views
                .iter()
                .map(|view| view.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "input-request:sha256:a",
                "input-request:sha256:c",
                "input-request:sha256:b"
            ]
        );
    }

    #[test]
    fn respond_input_request_records_response_and_closes_open_count() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = open_input_request(open_request(repo.path(), "Need approval")).unwrap();

        let responded = respond_input_request(
            InputRequestRespondOptions::new(repo.path(), requested.input_request_id.clone())
                .with_outcome(InputRequestResponseOutcome::Approved)
                .with_reason("approved locally"),
        )
        .unwrap();

        assert!(
            responded
                .input_request_response_id
                .as_str()
                .starts_with("input-request-response:sha256:")
        );
        assert_eq!(responded.outcome, InputRequestResponseOutcome::Approved);
        assert_eq!(
            responded.events_created_by_type["input_request_responded"],
            1
        );

        let state = SessionState::from_events(
            &EventStore::open(resolved_store_dir(repo.path()))
                .list_events()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(state.input_request_count, 1);
        assert_eq!(state.open_input_request_count, 0);
        assert_eq!(state.open_operative_input_request_count, 0);
    }

    #[test]
    fn resolving_unknown_input_request_fails() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let error = respond_input_request(
            InputRequestRespondOptions::new(
                repo.path(),
                InputRequestId::new("input-request:sha256:missing"),
            )
            .with_outcome(InputRequestResponseOutcome::Dismissed),
        )
        .unwrap_err();

        assert!(error.to_string().contains("unknown input request"));
    }

    #[test]
    fn respond_input_request_is_idempotent_for_same_logical_input() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = open_input_request(open_request(repo.path(), "Need approval")).unwrap();
        let options =
            InputRequestRespondOptions::new(repo.path(), requested.input_request_id.clone())
                .with_outcome(InputRequestResponseOutcome::Approved)
                .with_reason("approved locally");

        let first = respond_input_request(options.clone()).unwrap();
        let second = respond_input_request(options).unwrap();

        assert_eq!(
            first.input_request_response_id,
            second.input_request_response_id
        );
        assert_eq!(first.events_created, 1);
        assert_eq!(second.events_created, 0);
        assert_eq!(second.events_existing, 1);
    }

    #[test]
    fn explicit_same_response_idempotency_key_with_different_payload_conflicts() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = open_input_request(open_request(repo.path(), "Need approval")).unwrap();

        respond_input_request(
            InputRequestRespondOptions::new(repo.path(), requested.input_request_id.clone())
                .with_outcome(InputRequestResponseOutcome::Approved)
                .with_idempotency_key("retry-key"),
        )
        .unwrap();
        let error = respond_input_request(
            InputRequestRespondOptions::new(repo.path(), requested.input_request_id.clone())
                .with_outcome(InputRequestResponseOutcome::Rejected)
                .with_idempotency_key("retry-key"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("event conflict"));
    }

    #[test]
    fn resolving_twice_with_different_outcomes_is_append_only_and_ambiguous() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = open_input_request(open_request(repo.path(), "Need approval")).unwrap();

        respond_input_request(
            InputRequestRespondOptions::new(repo.path(), requested.input_request_id.clone())
                .with_outcome(InputRequestResponseOutcome::Approved),
        )
        .unwrap();
        respond_input_request(
            InputRequestRespondOptions::new(repo.path(), requested.input_request_id.clone())
                .with_outcome(InputRequestResponseOutcome::Rejected),
        )
        .unwrap();

        let result = fetch_input_request(InputRequestFetchOptions::new(
            repo.path(),
            requested.input_request_id,
        ))
        .unwrap();

        assert_eq!(result.input_request.status, InputRequestStatus::Ambiguous);
        assert_eq!(result.input_request.responses.len(), 2);
    }

    #[test]
    fn large_response_reason_is_stored_as_internal_body_artifact() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = open_input_request(open_request(repo.path(), "Need approval")).unwrap();
        let reason = "responded ".repeat(1000);

        let result = respond_input_request(
            InputRequestRespondOptions::new(repo.path(), requested.input_request_id)
                .with_outcome(InputRequestResponseOutcome::Approved)
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

    /// The store a workflow actually lands in for `repo` — the shared common-dir
    /// store by default. Reads that follow a workflow resolve here, not the raw
    /// worktree-local `.shore/data`.
    fn resolved_store_dir(repo: &Path) -> std::path::PathBuf {
        crate::git::git_common_dir(repo).unwrap().join("shore")
    }

    fn open_request(repo: &Path, title: &str) -> InputRequestOpenOptions {
        InputRequestOpenOptions::new(repo)
            .with_track("human:kevin")
            .with_title(title)
            .with_reason_code(InputRequestReasonCode::ManualDecisionRequired)
    }

    fn write_response_event(
        repo: &Path,
        requested: &InputRequestOpenResult,
        source_key: &str,
        outcome: InputRequestResponseOutcome,
    ) -> EventId {
        let response_id_material = format!("{}:{source_key}", requested.input_request_id.as_str());
        let response_id = InputRequestResponseId::new(format!(
            "input-request-response:sha256:{}",
            sha256_bytes_hex(response_id_material.as_bytes())
        ));
        let payload = InputRequestRespondedPayload {
            input_request_response_id: response_id,
            input_request_id: requested.input_request_id.clone(),
            outcome,
            reason: Some("responded".to_owned()),
            reason_artifact_path: None,
            reason_byte_size: Some(8),
            reason_content_hash: Some("sha256:responded".to_owned()),
            target_fingerprint: None,
        };
        let event = ShoreEvent::new(
            EventType::InputRequestResponded,
            InputRequestRespondedPayload::idempotency_key(&requested.input_request_id, source_key),
            EventTarget::for_subject(
                LedgerId::new("session:default"),
                TargetRef::Review(ReviewTargetRef::InputRequest {
                    revision_id: requested.review_unit_id.clone(),
                    input_request_id: requested.input_request_id.clone(),
                }),
                Some(requested.track_id.clone()),
            ),
            Writer::shore_local("test"),
            payload,
            current_timestamp(),
        )
        .unwrap();
        let event_id = event.event_id.clone();
        EventStore::open(resolved_store_dir(repo))
            .record_event_once(&event)
            .unwrap();
        event_id
    }

    fn only_input_request_opened_event(repo: &Path) -> ShoreEvent {
        let events = input_request_opened_events(repo);
        assert_eq!(events.len(), 1, "{events:?}");
        events.into_iter().next().unwrap()
    }

    fn input_request_opened_events(repo: &Path) -> Vec<ShoreEvent> {
        EventStore::open(resolved_store_dir(repo))
            .list_events()
            .unwrap()
            .into_iter()
            .filter(|event| event.event_type == EventType::InputRequestOpened)
            .collect::<Vec<_>>()
    }

    fn responded_events(repo: &Path) -> Vec<ShoreEvent> {
        EventStore::open(resolved_store_dir(repo))
            .list_events()
            .unwrap()
            .into_iter()
            .filter(|event| event.event_type == EventType::InputRequestResponded)
            .collect::<Vec<_>>()
    }

    fn projection_request_event(
        session_id: &LedgerId,
        review_unit_id: &RevisionId,
        track_id: &TrackId,
        input_request_id: &InputRequestId,
        source_key: &str,
        occurred_at: &str,
    ) -> ShoreEvent {
        let payload = InputRequestOpenedPayload {
            input_request_id: input_request_id.clone(),
            target: ReviewTargetRef::Revision {
                revision_id: review_unit_id.clone(),
            },
            reason_code: InputRequestReasonCode::ManualDecisionRequired,
            title: "projection".to_owned(),
            body: None,
            body_artifact_path: None,
            body_byte_size: None,
            body_content_hash: None,
            target_fingerprint: None,
        };
        ShoreEvent::new(
            EventType::InputRequestOpened,
            InputRequestOpenedPayload::idempotency_key(review_unit_id, track_id, source_key),
            EventTarget::for_subject(
                session_id.clone(),
                TargetRef::Review(ReviewTargetRef::Revision {
                    revision_id: review_unit_id.clone(),
                }),
                Some(track_id.clone()),
            ),
            Writer::shore_local("test"),
            payload,
            occurred_at.to_owned(),
        )
        .unwrap()
    }

    fn projection_response_event(
        session_id: &LedgerId,
        review_unit_id: &RevisionId,
        track_id: &TrackId,
        input_request_id: &InputRequestId,
        response_id: &InputRequestResponseId,
        source_key: &str,
        occurred_at: &str,
    ) -> ShoreEvent {
        let payload = InputRequestRespondedPayload {
            input_request_response_id: response_id.clone(),
            input_request_id: input_request_id.clone(),
            outcome: InputRequestResponseOutcome::Approved,
            reason: None,
            reason_artifact_path: None,
            reason_byte_size: None,
            reason_content_hash: None,
            target_fingerprint: None,
        };
        ShoreEvent::new(
            EventType::InputRequestResponded,
            InputRequestRespondedPayload::idempotency_key(input_request_id, source_key),
            EventTarget::for_subject(
                session_id.clone(),
                TargetRef::Review(ReviewTargetRef::InputRequest {
                    revision_id: review_unit_id.clone(),
                    input_request_id: input_request_id.clone(),
                }),
                Some(track_id.clone()),
            ),
            Writer::shore_local("test"),
            payload,
            occurred_at.to_owned(),
        )
        .unwrap()
    }

    fn assert_status(
        result: &InputRequestListResult,
        id: &InputRequestId,
        status: InputRequestStatus,
    ) {
        let view = result
            .input_requests
            .iter()
            .find(|view| &view.id == id)
            .unwrap();
        assert_eq!(view.status, status);
    }

    fn input_request_view_for_sort(
        input_request_id: &str,
        event_id: &str,
        created_at: &str,
    ) -> InputRequestView {
        let review_unit_id = RevisionId::new("review-unit:sha256:one");
        InputRequestView {
            id: InputRequestId::new(input_request_id),
            event_id: EventId::new(event_id),
            track_id: TrackId::new("agent:codex"),
            target: ReviewTargetRef::Revision {
                revision_id: review_unit_id,
            },
            mode: AssertionMode::Operative,
            reason_code: InputRequestReasonCode::ManualDecisionRequired,
            title: "sort".to_owned(),
            body: None,
            body_content_hash: None,
            status: InputRequestStatus::Open,
            responses: vec![],
            created_at: created_at.to_owned(),
            writer: Writer::shore_local("test"),
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
