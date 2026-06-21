mod options;
mod projection;
mod result;
mod summary;

use self::options::ResolvedHistoryFilters;
pub use self::options::{ReviewHistoryFilters, ReviewHistoryOptions};
use self::projection::history_from_events;
pub use self::result::ReviewHistoryResult;
pub use self::summary::ReviewHistoryEntry;
use crate::error::Result;
use crate::session::EventStore;
use crate::session::observation::validated_track_id;
use crate::session::store::resolution::resolve_read_store;

pub fn review_history(options: ReviewHistoryOptions) -> Result<ReviewHistoryResult> {
    let read_store = resolve_read_store(&options.repo)?;
    let track_id = options
        .track
        .as_deref()
        .map(validated_track_id)
        .transpose()?;
    let events = EventStore::open(read_store.store_dir()).list_events()?;

    let ref_matched_units = match &options.ref_filter {
        Some((name, mode)) => {
            let projection = crate::session::ReviewUnitCommitRangeProjection::from_events(&events)?;
            Some(super::review_unit_list::review_units_matching_ref(
                &projection,
                name,
                *mode,
                &options.repo,
            )?)
        }
        None => None,
    };

    let filters = ResolvedHistoryFilters {
        review_unit_id: options.review_unit_id,
        track_id,
        event_types: options.event_types,
        ref_matched_units,
        include_body: options.include_body,
        verification_policy: options.verification_policy,
        trust_set: options.trust_set,
        actor_attributes: options.actor_attributes,
        delegation_map: options.delegation_map,
    };
    let result = history_from_events(&events, filters, Some(read_store.store_dir()))?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::projection::{history_entry_from_event, history_from_events};
    use super::summary::ReviewHistorySummary;
    use super::*;
    use crate::model::{
        ActorId, AssessmentId, EngagementId, EventId, InputRequestId, InputRequestResponseId,
        LedgerId, ObjectId, ObservationId, ReviewEndpoint, ReviewTargetRef, ReviewUnitSource,
        RevisionId, Side, TargetRef, TrackId, ValidationCheckId, ValidationStatus,
        ValidationTarget, ValidationTrigger, WorktreeCaptureMode,
    };
    use crate::session::event::{
        AssertionMode, EventTarget, EventType, GitProvenance, ImportedNoteTarget,
        InputRequestOpenedPayload, InputRequestReasonCode, InputRequestRespondedPayload,
        InputRequestResponseOutcome, ReviewAssessment, ReviewAssessmentRecordedPayload,
        ReviewInitializedPayload, ReviewNoteImportedPayload, ReviewObservationRecordedPayload,
        Revision, ShoreEvent, SidecarSource, ValidationCheckRecordedPayload, WorkObjectProposal,
        WorkObjectProposedPayload, Writer,
    };
    use crate::session::state::DUPLICATE_SEMANTIC_OBSERVATION_EVENT_CODE;

    #[test]
    fn review_history_returns_empty_freshness_metadata_without_events() {
        let result = history_from_events(&[], ResolvedHistoryFilters::default(), None).unwrap();

        assert_eq!(result.event_count, 0);
        assert_eq!(result.history_count(), 0);
        assert!(result.event_set_hash.starts_with("sha256:"));
        assert!(result.entries.is_empty());
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn review_history_metadata_uses_full_event_set() {
        let first = review_initialized_event("one");
        let second = review_initialized_event("two");

        let result = history_from_events(
            &[first, second],
            ResolvedHistoryFilters {
                event_types: vec![EventType::ReviewInitialized],
                ..ResolvedHistoryFilters::default()
            },
            None,
        )
        .unwrap();

        assert_eq!(result.event_count, 2);
        assert_eq!(result.history_count(), 2);
        assert!(result.event_set_hash.starts_with("sha256:"));
    }

    #[test]
    fn history_entry_summarizes_current_event_families() {
        let cases = [
            (
                review_initialized_event("init"),
                EventType::ReviewInitialized,
                "review_initialized",
            ),
            (
                review_unit_captured_event(),
                EventType::WorkObjectProposed,
                "revision_captured",
            ),
            (
                observation_event_with_body("body"),
                EventType::ReviewObservationRecorded,
                "review_observation_recorded",
            ),
            (
                assessment_event(),
                EventType::ReviewAssessmentRecorded,
                "review_assessment_recorded",
            ),
            (
                input_request_opened_event(),
                EventType::InputRequestOpened,
                "input_request_opened",
            ),
            (
                input_request_responded_event(),
                EventType::InputRequestResponded,
                "input_request_responded",
            ),
            (
                review_note_imported_event(),
                EventType::ReviewNoteImported,
                "review_note_imported",
            ),
            (
                validation_check_recorded_event(),
                EventType::ValidationCheckRecorded,
                "validation_check_recorded",
            ),
        ];

        for (event, event_type, summary_kind) in cases {
            let entry =
                history_entry_from_event(&event, &ResolvedHistoryFilters::default(), None, None)
                    .unwrap();
            let summary_json = serde_json::to_value(&entry.summary).unwrap();

            assert_eq!(entry.event_type, event_type);
            assert_eq!(summary_json["kind"], summary_kind);
        }
    }

    #[test]
    fn history_entry_preserves_assessment_target_for_assessment_event() {
        let target = ReviewTargetRef::Assessment {
            revision_id: revision_id("one"),
            assessment_id: AssessmentId::new("assess:sha256:prior"),
        };
        let event = assessment_event_with_target(target.clone());

        let entry =
            history_entry_from_event(&event, &ResolvedHistoryFilters::default(), None, None)
                .unwrap();
        let json = serde_json::to_value(&entry).unwrap();

        match entry.summary {
            ReviewHistorySummary::ReviewAssessmentRecorded {
                target: summary_target,
                ..
            } => assert_eq!(summary_target, target),
            other => panic!("expected assessment summary, got {other:?}"),
        }
        assert_eq!(
            json["summary"]["relatedInputRequests"],
            serde_json::json!(["input-request:sha256:one"])
        );
        assert!(json["summary"].get("relatedInterventions").is_none());
    }

    #[test]
    fn history_includes_validation_check_recorded_summary() {
        let event = validation_check_recorded_event();

        let entry =
            history_entry_from_event(&event, &ResolvedHistoryFilters::default(), None, None)
                .unwrap();
        let json = serde_json::to_value(&entry.summary).unwrap();

        assert_eq!(json["kind"], "validation_check_recorded");
        assert_eq!(json["validationCheckId"], "validation:sha256:one");
        assert_eq!(json["checkName"], "cargo test");
        assert_eq!(json["status"], "passed");
    }

    #[test]
    fn history_entry_omits_internal_artifact_paths() {
        let event = observation_event_with_artifact_path("artifacts/notes/body.json");

        let entry =
            history_entry_from_event(&event, &ResolvedHistoryFilters::default(), None, None)
                .unwrap();
        let json = serde_json::to_string(&entry).unwrap();

        assert!(!json.contains("bodyArtifactPath"));
        assert!(!json.contains("artifacts/notes"));
    }

    #[test]
    fn history_sorts_by_occurred_at_then_event_id() {
        let late = event_with_time_and_key("2026-05-13T10:00:02Z", "late");
        let tie_b = event_with_time_and_key("2026-05-13T10:00:01Z", "b");
        let tie_a = event_with_time_and_key("2026-05-13T10:00:01Z", "a");

        let result = history_from_events(
            &[late, tie_b, tie_a],
            ResolvedHistoryFilters::default(),
            None,
        )
        .unwrap();

        assert_eq!(
            result
                .entries
                .iter()
                .map(|entry| entry.occurred_at.as_str())
                .collect::<Vec<_>>(),
            vec![
                "2026-05-13T10:00:01Z",
                "2026-05-13T10:00:01Z",
                "2026-05-13T10:00:02Z",
            ]
        );
        assert!(result.entries[0].event_id.as_str() < result.entries[1].event_id.as_str());
    }

    #[test]
    fn review_history_entry_subject_round_trips_review_target_ref_after_envelope_widening() {
        let event = observation_event("review-unit:sha256:one", "agent:codex", "Pinned");

        let entry =
            history_entry_from_event(&event, &ResolvedHistoryFilters::default(), None, None)
                .unwrap();

        assert_eq!(
            entry.subject,
            Some(ReviewTargetRef::Revision {
                revision_id: RevisionId::new("review-unit:sha256:one"),
            })
        );
    }

    #[test]
    fn review_history_entry_subject_narrows_task_target_ref_to_none() {
        use crate::model::{CheckpointId, TargetRef, TaskTargetRef};

        let mut event = review_initialized_event("narrow");
        event.target.subject = TargetRef::Task(TaskTargetRef::Checkpoint {
            checkpoint_id: CheckpointId::new("checkpoint:sha256:narrow"),
        });

        let entry =
            history_entry_from_event(&event, &ResolvedHistoryFilters::default(), None, None)
                .unwrap();

        assert!(entry.subject.is_none());
    }

    #[test]
    fn review_history_filters_out_task_event_types_unconditionally() {
        let init = review_initialized_event("init");
        let task_event = ShoreEvent {
            schema: "shore.event".to_owned(),
            version: 1,
            event_id: EventId::new("evt:sha256:task-checkpoint"),
            event_type: EventType::TaskCheckpointCaptured,
            idempotency_key: "task_checkpoint_captured:filter".to_owned(),
            target: EventTarget::for_ledger(LedgerId::new("session:claude:abc")),
            writer: Writer::shore_local("test"),
            occurred_at: "2026-05-18T10:00:01Z".to_owned(),
            payload_hash: "sha256:placeholder".to_owned(),
            assertion_mode: AssertionMode::Advisory,
            signer: None,
            signature: None,
            source_ref: None,
            ingest: None,
            payload: serde_json::Value::Null,
        };

        let result =
            history_from_events(&[init, task_event], ResolvedHistoryFilters::default(), None)
                .unwrap();

        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].event_type, EventType::ReviewInitialized);
    }

    #[test]
    fn review_history_filter_excludes_all_task_event_types() {
        let init = review_initialized_event("init");
        let task_events = [
            EventType::TaskCheckpointCaptured,
            EventType::TaskObservationRecorded,
        ]
        .into_iter()
        .enumerate()
        .map(|(idx, event_type)| ShoreEvent {
            schema: "shore.event".to_owned(),
            version: 1,
            event_id: EventId::new(format!("evt:sha256:task-{idx}")),
            event_type,
            idempotency_key: format!("task_kind_{idx}"),
            target: EventTarget::for_ledger(LedgerId::new("session:claude:abc")),
            writer: Writer::shore_local("test"),
            occurred_at: format!("2026-05-18T10:00:0{}Z", idx + 1),
            payload_hash: "sha256:placeholder".to_owned(),
            assertion_mode: AssertionMode::Advisory,
            signer: None,
            signature: None,
            source_ref: None,
            ingest: None,
            payload: serde_json::Value::Null,
        });

        let mut events: Vec<ShoreEvent> = vec![init];
        events.extend(task_events);

        let result = history_from_events(&events, ResolvedHistoryFilters::default(), None).unwrap();

        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].event_type, EventType::ReviewInitialized);
    }

    #[test]
    fn review_history_filter_excludes_task_attempt_proposals() {
        // A generative move can propose a task attempt under the shared
        // WorkObjectProposed type; its Task subject keeps it out of the
        // review-domain history rather than reaching the revision summary arm.
        let init = review_initialized_event("init");
        let task_payload = WorkObjectProposedPayload {
            engagement_id: EngagementId::new("engagement:sha256:task"),
            work_object: WorkObjectProposal::TaskAttempt {
                task_attempt_id: crate::model::WorkObjectId::new("task-attempt:sha256:t"),
                project_path: "/repo".to_owned(),
                claude_session_uuid: "uuid".to_owned(),
                initial_prompt_hash: "sha256:prompt".to_owned(),
                predecessor: None,
                base_snapshot_fingerprint: None,
                source_speaker: None,
            },
        };
        let task_attempt = ShoreEvent::new(
            EventType::WorkObjectProposed,
            "task_attempt_proposal",
            EventTarget::for_subject(
                LedgerId::new("ledger:claude:abc"),
                TargetRef::Task(crate::model::TaskTargetRef::TaskAttempt),
                None,
            ),
            Writer::shore_local("test"),
            task_payload,
            "2026-05-18T10:00:05Z",
        )
        .unwrap();

        let result = history_from_events(
            &[init, task_attempt],
            ResolvedHistoryFilters::default(),
            None,
        )
        .unwrap();

        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].event_type, EventType::ReviewInitialized);
    }

    #[test]
    fn review_history_projection_rejects_task_event_with_explicit_error() {
        let event = ShoreEvent {
            schema: "shore.event".to_owned(),
            version: 1,
            event_id: EventId::new("evt:sha256:checkpoint-1"),
            event_type: EventType::TaskCheckpointCaptured,
            idempotency_key: "task_checkpoint_captured:cp-1".to_owned(),
            target: EventTarget::for_ledger(LedgerId::new("session:claude:abc")),
            writer: Writer::shore_local("test"),
            occurred_at: "2026-05-18T00:00:00Z".to_owned(),
            payload_hash: "sha256:placeholder".to_owned(),
            assertion_mode: AssertionMode::Advisory,
            signer: None,
            signature: None,
            source_ref: None,
            ingest: None,
            payload: serde_json::Value::Null,
        };

        let error =
            history_entry_from_event(&event, &ResolvedHistoryFilters::default(), None, None)
                .expect_err("task events must not project to review-history");
        let message = error.to_string();

        assert!(
            message.contains("review-domain") || message.contains("task event"),
            "error message must document the review-domain contract; got: {message}"
        );
    }

    #[test]
    fn history_filters_by_review_unit_track_and_event_type() {
        let keep = observation_event("review-unit:sha256:one", "agent:codex", "Keep");
        let other_track = observation_event("review-unit:sha256:one", "agent:claude", "Drop track");
        let other_unit = observation_event("review-unit:sha256:two", "agent:codex", "Drop unit");
        let capture = review_unit_captured_event_for("review-unit:sha256:one");

        let filters = ResolvedHistoryFilters {
            review_unit_id: Some(RevisionId::new("review-unit:sha256:one")),
            track_id: Some(TrackId::new("agent:codex")),
            event_types: vec![EventType::ReviewObservationRecorded],
            include_body: false,
            ..ResolvedHistoryFilters::default()
        };

        let result =
            history_from_events(&[keep, other_track, other_unit, capture], filters, None).unwrap();

        assert_eq!(result.entries.len(), 1);
        assert_eq!(
            result.entries[0].track_id.as_ref().map(TrackId::as_str),
            Some("agent:codex")
        );
        assert_eq!(
            result.entries[0].event_type,
            EventType::ReviewObservationRecorded
        );
    }

    fn claude_code_delegates(
        valid_from: &str,
        valid_until: serde_json::Value,
    ) -> crate::session::DelegationMap {
        crate::session::delegation_map_from_value(serde_json::json!({
            "delegates": {
                "actor:agent:claude-code": [{
                    "principal": "actor:git-email:kevin@swiber.dev",
                    "validFrom": valid_from,
                    "validUntil": valid_until
                }]
            }
        }))
        .unwrap()
    }

    fn agent_written_observation() -> ShoreEvent {
        let mut event =
            observation_event("review-unit:sha256:one", "agent:claude-code", "Agent obs");
        event.writer.actor_id = ActorId::new("actor:agent:claude-code");
        event
    }

    #[test]
    fn history_entries_carry_resolved_principal_for_agent_writers_when_map_supplied() {
        let agent_event = agent_written_observation();
        // A human (git-email) writer for contrast.
        let mut human_event =
            observation_event("review-unit:sha256:one", "agent:codex", "Human obs");
        human_event.writer.actor_id = ActorId::new("actor:git-email:kevin@swiber.dev");

        let filters = ResolvedHistoryFilters {
            delegation_map: Some(claude_code_delegates(
                "2026-05-01T00:00:00Z",
                serde_json::Value::Null,
            )),
            ..ResolvedHistoryFilters::default()
        };
        let result = history_from_events(&[agent_event, human_event], filters, None).unwrap();

        let agent_entry = result
            .entries
            .iter()
            .find(|entry| entry.writer.actor_id.as_str() == "actor:agent:claude-code")
            .unwrap();
        let agent_json = serde_json::to_value(agent_entry).unwrap();
        assert_eq!(
            agent_json["principal"]["actorId"],
            "actor:git-email:kevin@swiber.dev"
        );
        assert_eq!(agent_json["principal"]["status"], "resolved");
        assert_eq!(agent_json["principal"]["source"], "delegates");

        let human_entry = result
            .entries
            .iter()
            .find(|entry| entry.writer.actor_id.as_str() == "actor:git-email:kevin@swiber.dev")
            .unwrap();
        assert!(
            serde_json::to_value(human_entry)
                .unwrap()
                .get("principal")
                .is_none(),
            "human (git-email) writers are their own principal — no principal object"
        );
    }

    #[test]
    fn history_without_map_emits_none_principal_for_agent_writers() {
        let result = history_from_events(
            &[agent_written_observation()],
            ResolvedHistoryFilters::default(),
            None,
        )
        .unwrap();
        let json = serde_json::to_value(&result.entries[0]).unwrap();
        assert_eq!(
            json["principal"],
            serde_json::json!({ "status": "none", "source": "none" })
        );
    }

    #[test]
    fn history_principal_is_occurred_at_scoped() {
        // observation_event's occurredAt is 2026-05-13T10:00:01Z.
        let closed = ResolvedHistoryFilters {
            delegation_map: Some(claude_code_delegates(
                "2026-01-01T00:00:00Z",
                serde_json::json!("2026-02-01T00:00:00Z"),
            )),
            ..ResolvedHistoryFilters::default()
        };
        let closed_result =
            history_from_events(&[agent_written_observation()], closed, None).unwrap();
        let closed_json = serde_json::to_value(&closed_result.entries[0]).unwrap();
        assert_eq!(
            closed_json["principal"]["status"], "none",
            "a window that closed before the event resolves none"
        );

        let covering = ResolvedHistoryFilters {
            delegation_map: Some(claude_code_delegates(
                "2026-05-01T00:00:00Z",
                serde_json::Value::Null,
            )),
            ..ResolvedHistoryFilters::default()
        };
        let covering_result =
            history_from_events(&[agent_written_observation()], covering, None).unwrap();
        let covering_json = serde_json::to_value(&covering_result.entries[0]).unwrap();
        assert_eq!(covering_json["principal"]["status"], "resolved");
    }

    #[test]
    fn history_omits_body_text_by_default() {
        let event = observation_event_with_body("inline body");

        let result =
            history_from_events(&[event], ResolvedHistoryFilters::default(), None).unwrap();
        let json = serde_json::to_value(&result.entries[0]).unwrap();

        assert!(json["summary"].get("body").is_none());
        assert!(json.to_string().contains("bodyContentHash"));
    }

    #[test]
    fn history_include_body_hydrates_inline_body_like_fields() {
        let filters = ResolvedHistoryFilters {
            include_body: true,
            ..ResolvedHistoryFilters::default()
        };
        let events = [
            observation_event_with_body("observation body"),
            assessment_event(),
            input_request_opened_event(),
            input_request_responded_event(),
            review_note_imported_event(),
        ];

        let result = history_from_events(&events, filters, None).unwrap();
        let entries = result
            .entries
            .iter()
            .map(|entry| serde_json::to_value(entry).unwrap())
            .collect::<Vec<_>>();

        assert!(entries.iter().any(|entry| {
            entry["summary"]["kind"] == "review_observation_recorded"
                && entry["summary"]["body"] == "observation body"
        }));
        assert!(entries.iter().any(|entry| {
            entry["summary"]["kind"] == "review_assessment_recorded"
                && entry["summary"]["summary"] == "ship it"
        }));
        assert!(entries.iter().any(|entry| {
            entry["summary"]["kind"] == "input_request_opened" && entry["summary"]["body"] == "body"
        }));
        assert!(entries.iter().any(|entry| {
            entry["summary"]["kind"] == "input_request_responded"
                && entry["summary"]["reason"] == "approved"
        }));
        assert!(entries.iter().any(|entry| {
            entry["summary"]["kind"] == "review_note_imported" && entry["summary"]["body"] == "body"
        }));
    }

    #[test]
    fn history_include_body_hydrates_artifact_body_without_exposing_path() {
        let dir = tempfile::tempdir().unwrap();
        let artifact_path = "artifacts/notes/body.json";
        let full_path = dir.path().join(artifact_path);
        std::fs::create_dir_all(full_path.parent().unwrap()).unwrap();
        std::fs::write(
            &full_path,
            r#"{"schema":"shore.note-body","version":1,"body":"artifact body"}"#,
        )
        .unwrap();
        let filters = ResolvedHistoryFilters {
            include_body: true,
            ..ResolvedHistoryFilters::default()
        };

        let result = history_from_events(
            &[observation_event_with_artifact_path(artifact_path)],
            filters,
            Some(dir.path()),
        )
        .unwrap();
        let json = serde_json::to_value(&result.entries[0]).unwrap();
        let serialized = serde_json::to_string(&result.entries[0]).unwrap();

        assert_eq!(json["summary"]["body"], "artifact body");
        assert!(!serialized.contains("bodyArtifactPath"));
        assert!(!serialized.contains("artifacts/notes"));
    }

    #[test]
    fn history_includes_duplicate_semantic_diagnostics() {
        let first = observation_event_with_id_and_key("obs:sha256:same", "retry-a");
        let second = observation_event_with_id_and_key("obs:sha256:same", "retry-b");

        let result =
            history_from_events(&[first, second], ResolvedHistoryFilters::default(), None).unwrap();

        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == DUPLICATE_SEMANTIC_OBSERVATION_EVENT_CODE
                && diagnostic.message.contains("obs:sha256:same")
        }));
        assert_eq!(
            result.entries.len(),
            2,
            "history preserves raw append-only facts"
        );
    }

    #[test]
    fn history_diagnostics_are_not_suppressed_by_filters() {
        let duplicate_a = observation_event_with_id_and_key("obs:sha256:same", "retry-a");
        let duplicate_b = observation_event_with_id_and_key("obs:sha256:same", "retry-b");
        let filters = ResolvedHistoryFilters {
            event_types: vec![EventType::WorkObjectProposed],
            ..ResolvedHistoryFilters::default()
        };

        let result = history_from_events(&[duplicate_a, duplicate_b], filters, None).unwrap();

        assert!(result.entries.is_empty());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code == DUPLICATE_SEMANTIC_OBSERVATION_EVENT_CODE })
        );
    }

    fn review_initialized_event(key: &str) -> ShoreEvent {
        let ledger_id = LedgerId::new("session:default");
        ShoreEvent::new(
            EventType::ReviewInitialized,
            ReviewInitializedPayload::idempotency_key(&ledger_id),
            EventTarget::for_ledger(ledger_id),
            Writer::shore_local("test"),
            ReviewInitializedPayload {},
            format!("2026-05-13T10:00:0{key}Z"),
        )
        .unwrap()
    }

    fn review_unit_captured_event() -> ShoreEvent {
        review_unit_captured_event_for("review-unit:sha256:one")
    }

    fn review_unit_captured_event_for(review_unit_id: &str) -> ShoreEvent {
        let review_unit_id = RevisionId::new(review_unit_id);
        let payload = WorkObjectProposedPayload {
            engagement_id: EngagementId::new(format!(
                "engagement:sha256:{}",
                crate::canonical_hash::sha256_bytes_hex(
                    (RevisionId::new(format!("rev:{}", review_unit_id.as_str())))
                        .as_str()
                        .as_bytes()
                )
            )),
            work_object: WorkObjectProposal::Revision {
                revision: Revision {
                    id: RevisionId::new(format!("rev:{}", review_unit_id.as_str())),
                    object_id: ObjectId::new(format!("snap:{}", review_unit_id.as_str())),
                    git_provenance: Some(GitProvenance {
                        source: ReviewUnitSource::GitWorktree {
                            mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                            include_untracked: true,
                        },
                        base: ReviewEndpoint::GitCommit {
                            commit_oid: "base".to_owned(),
                            tree_oid: "base-tree".to_owned(),
                        },
                        target: ReviewEndpoint::GitWorkingTree {
                            worktree_root: "/repo".to_owned(),
                        },
                    }),
                },
                snapshot_artifact_content_hash: "sha256:artifact".to_owned(),
                supersedes: vec![],
            },
        };
        ShoreEvent::new(
            EventType::WorkObjectProposed,
            "capture:one",
            EventTarget::for_revision(LedgerId::new("session:default"), review_unit_id, None),
            Writer::shore_local("test"),
            payload,
            "2026-05-13T10:00:00Z",
        )
        .unwrap()
    }

    fn event_with_time_and_key(occurred_at: &str, key: &str) -> ShoreEvent {
        let mut event = observation_event("review-unit:sha256:one", "agent:codex", key);
        event.occurred_at = occurred_at.to_owned();
        event
    }

    fn observation_event(review_unit_id: &str, track_id: &str, title: &str) -> ShoreEvent {
        let review_unit_id = RevisionId::new(review_unit_id);
        let payload = ReviewObservationRecordedPayload {
            observation_id: ObservationId::new(format!("obs:sha256:{title}")),
            target: ReviewTargetRef::Revision {
                revision_id: review_unit_id.clone(),
            },
            title: title.to_owned(),
            body: None,
            body_artifact_path: None,
            body_byte_size: None,
            body_content_hash: None,
            tags: vec![],
            confidence: None,
            supersedes_observation_ids: vec![],
        };
        tracked_event_for_unit(
            EventType::ReviewObservationRecorded,
            &format!("observation:{title}:{track_id}"),
            track_id,
            review_unit_id,
            payload,
            "2026-05-13T10:00:01Z",
        )
    }

    fn observation_event_with_body(body: &str) -> ShoreEvent {
        let payload = ReviewObservationRecordedPayload {
            observation_id: ObservationId::new("obs:sha256:one"),
            target: ReviewTargetRef::Revision {
                revision_id: revision_id("one"),
            },
            title: "Observation".to_owned(),
            body: Some(body.to_owned()),
            body_artifact_path: None,
            body_byte_size: Some(body.len() as u64),
            body_content_hash: Some("sha256:body".to_owned()),
            tags: vec!["correctness".to_owned()],
            confidence: Some("high".to_owned()),
            supersedes_observation_ids: vec![],
        };
        tracked_event(
            EventType::ReviewObservationRecorded,
            "observation:one",
            "agent:codex",
            payload,
            "2026-05-13T10:00:01Z",
        )
    }

    fn observation_event_with_artifact_path(path: &str) -> ShoreEvent {
        let payload = ReviewObservationRecordedPayload {
            observation_id: ObservationId::new("obs:sha256:artifact"),
            target: ReviewTargetRef::Revision {
                revision_id: revision_id("one"),
            },
            title: "Observation".to_owned(),
            body: None,
            body_artifact_path: Some(path.to_owned()),
            body_byte_size: Some(5000),
            body_content_hash: Some("sha256:body".to_owned()),
            tags: vec![],
            confidence: None,
            supersedes_observation_ids: vec![],
        };
        tracked_event(
            EventType::ReviewObservationRecorded,
            "observation:artifact",
            "agent:codex",
            payload,
            "2026-05-13T10:00:01Z",
        )
    }

    fn assessment_event() -> ShoreEvent {
        assessment_event_with_target(ReviewTargetRef::Revision {
            revision_id: revision_id("one"),
        })
    }

    fn assessment_event_with_target(target: ReviewTargetRef) -> ShoreEvent {
        let payload = ReviewAssessmentRecordedPayload {
            assessment_id: AssessmentId::new("assess:sha256:one"),
            target,
            assessment: ReviewAssessment::Accepted,
            summary: Some("ship it".to_owned()),
            summary_artifact_path: None,
            summary_byte_size: Some(7),
            summary_content_hash: Some("sha256:summary".to_owned()),
            replaces_assessment_ids: vec![],
            related_observation_ids: vec![ObservationId::new("obs:sha256:one")],
            related_input_request_ids: vec![InputRequestId::new("input-request:sha256:one")],
        };
        tracked_event(
            EventType::ReviewAssessmentRecorded,
            "assessment:one",
            "human:kevin",
            payload,
            "2026-05-13T10:00:02Z",
        )
    }

    fn observation_event_with_id_and_key(
        observation_id: &str,
        idempotency_key: &str,
    ) -> ShoreEvent {
        let payload = ReviewObservationRecordedPayload {
            observation_id: ObservationId::new(observation_id),
            target: ReviewTargetRef::Revision {
                revision_id: revision_id("one"),
            },
            title: "Duplicate".to_owned(),
            body: Some("same body".to_owned()),
            body_artifact_path: None,
            body_byte_size: Some(9),
            body_content_hash: Some("sha256:body".to_owned()),
            tags: vec![],
            confidence: None,
            supersedes_observation_ids: vec![],
        };
        tracked_event(
            EventType::ReviewObservationRecorded,
            idempotency_key,
            "agent:codex",
            payload,
            "2026-05-13T10:00:01Z",
        )
    }

    fn input_request_opened_event() -> ShoreEvent {
        let payload = InputRequestOpenedPayload {
            input_request_id: InputRequestId::new("input-request:sha256:one"),
            target: ReviewTargetRef::Revision {
                revision_id: revision_id("one"),
            },
            reason_code: InputRequestReasonCode::ManualDecisionRequired,
            title: "Need decision".to_owned(),
            body: Some("body".to_owned()),
            body_artifact_path: None,
            body_byte_size: Some(4),
            body_content_hash: Some("sha256:body".to_owned()),
            target_fingerprint: None,
        };
        tracked_event(
            EventType::InputRequestOpened,
            "input-request:open",
            "human:kevin",
            payload,
            "2026-05-13T10:00:02Z",
        )
        .with_assertion_mode(AssertionMode::Operative)
    }

    fn input_request_responded_event() -> ShoreEvent {
        let payload = InputRequestRespondedPayload {
            input_request_response_id: InputRequestResponseId::new(
                "input-request-response:sha256:one",
            ),
            input_request_id: InputRequestId::new("input-request:sha256:one"),
            outcome: InputRequestResponseOutcome::Approved,
            reason: Some("approved".to_owned()),
            reason_artifact_path: None,
            reason_byte_size: Some(8),
            reason_content_hash: Some("sha256:reason".to_owned()),
            target_fingerprint: None,
        };
        tracked_event(
            EventType::InputRequestResponded,
            "input-request:respond",
            "human:kevin",
            payload,
            "2026-05-13T10:00:03Z",
        )
    }

    fn validation_check_recorded_event() -> ShoreEvent {
        let payload = ValidationCheckRecordedPayload {
            validation_check_id: ValidationCheckId::new("validation:sha256:one"),
            target: ValidationTarget::Revision {
                revision_id: review_unit_id("one"),
            },
            check_name: "cargo test".to_owned(),
            command: None,
            status: ValidationStatus::Passed,
            exit_code: Some(0),
            trigger: ValidationTrigger::Manual,
            source_fingerprint: None,
            summary: Some("tests passed".to_owned()),
            summary_artifact_path: None,
            summary_byte_size: Some(12),
            summary_content_hash: Some("sha256:summary".to_owned()),
            started_at: Some("2026-05-13T09:59:00Z".to_owned()),
            completed_at: Some("2026-05-13T10:00:00Z".to_owned()),
            log_artifact_content_hashes: vec!["sha256:log".to_owned()],
        };
        tracked_event(
            EventType::ValidationCheckRecorded,
            "validation:one",
            "agent:codex",
            payload,
            "2026-05-13T10:00:04Z",
        )
    }

    fn review_note_imported_event() -> ShoreEvent {
        let payload = ReviewNoteImportedPayload {
            sidecar_source: SidecarSource::ReviewNotes,
            note_id: "note-1".to_owned(),
            file_path: "src/lib.rs".to_owned(),
            file_old_path: None,
            target: Some(ImportedNoteTarget {
                side: Side::New,
                start_line: 1,
                end_line: 1,
            }),
            title: "Imported note".to_owned(),
            body: Some("body".to_owned()),
            body_artifact_path: None,
            body_byte_size: Some(4),
            tags: vec!["imported".to_owned()],
            confidence: Some("medium".to_owned()),
            external_source: Some("review-notes.json".to_owned()),
            author: Some("reviewer".to_owned()),
            created_at: Some("2026-05-13T09:00:00Z".to_owned()),
            sidecar_content_hash: "sha256:sidecar".to_owned(),
        };
        ShoreEvent::new(
            EventType::ReviewNoteImported,
            "review-note:one",
            EventTarget::for_ledger(LedgerId::new("session:default")),
            Writer::shore_local("test"),
            payload,
            "2026-05-13T10:00:05Z",
        )
        .unwrap()
    }

    fn tracked_event<P>(
        event_type: EventType,
        idempotency_key: &str,
        track_id: &str,
        payload: P,
        occurred_at: &str,
    ) -> ShoreEvent
    where
        P: crate::session::event::EventPayload,
    {
        tracked_event_for_unit(
            event_type,
            idempotency_key,
            track_id,
            review_unit_id("one"),
            payload,
            occurred_at,
        )
    }

    fn tracked_event_for_unit<P>(
        event_type: EventType,
        idempotency_key: &str,
        track_id: &str,
        review_unit_id: RevisionId,
        payload: P,
        occurred_at: &str,
    ) -> ShoreEvent
    where
        P: crate::session::event::EventPayload,
    {
        let mut target = EventTarget::for_revision(
            LedgerId::new("session:default"),
            review_unit_id.clone(),
            None,
        );
        target.track_id = Some(TrackId::new(track_id));
        target.subject = TargetRef::Review(ReviewTargetRef::Revision {
            revision_id: review_unit_id.clone(),
        });
        ShoreEvent::new(
            event_type,
            idempotency_key,
            target,
            Writer::shore_local("test"),
            payload,
            occurred_at,
        )
        .unwrap()
    }

    fn review_unit_id(suffix: &str) -> RevisionId {
        RevisionId::new(format!("review-unit:sha256:{suffix}"))
    }

    fn revision_id(suffix: &str) -> RevisionId {
        RevisionId::new(format!("rev:sha256:{suffix}"))
    }
}
