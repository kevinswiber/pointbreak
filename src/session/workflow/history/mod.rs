mod options;
mod projection;
mod result;
mod summary;

use self::options::ResolvedHistoryFilters;
pub use self::options::{ReviewHistoryFilters, ReviewHistoryOptions};
use self::projection::history_from_events;
pub use self::result::ReviewHistoryResult;
pub use self::summary::{ReviewHistoryEntry, ReviewHistorySummary};
use crate::error::Result;
use crate::session::EventStore;
use crate::session::observation::validated_track_id;
use crate::session::store_init::ShoreStorePaths;

pub fn review_history(options: ReviewHistoryOptions) -> Result<ReviewHistoryResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let track_id = options
        .track
        .as_deref()
        .map(validated_track_id)
        .transpose()?;
    let filters = ResolvedHistoryFilters {
        review_unit_id: options.review_unit_id,
        track_id,
        event_types: options.event_types,
        include_body: options.include_body,
    };
    let events = EventStore::open(paths.shore_dir()).list_events()?;
    history_from_events(&events, filters, Some(paths.shore_dir()))
}

#[cfg(test)]
mod tests {
    use super::projection::{history_entry_from_event, history_from_events};
    use super::*;
    use crate::model::{
        DispositionId, InterventionId, InterventionResolutionId, ObservationId, ReviewEndpoint,
        ReviewId, ReviewTargetRef, ReviewUnitId, ReviewUnitSource, RevisionId, Side, SnapshotId,
        TrackId, WorkUnitId, WorktreeCaptureMode,
    };
    use crate::session::event::{
        EventTarget, EventType, ImportedNoteTarget, InterventionMode, InterventionReasonCode,
        InterventionRequestedPayload, InterventionResolutionOutcome, InterventionResolvedPayload,
        ReviewDisposition, ReviewDispositionRecordedPayload, ReviewInitializedPayload,
        ReviewNoteImportedPayload, ReviewObservationRecordedPayload, ReviewUnitCapturedPayload,
        ShoreEvent, SidecarSource, Writer,
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
                EventType::ReviewUnitCaptured,
                "review_unit_captured",
            ),
            (
                observation_event_with_body("body"),
                EventType::ReviewObservationRecorded,
                "review_observation_recorded",
            ),
            (
                intervention_requested_event(),
                EventType::InterventionRequested,
                "intervention_requested",
            ),
            (
                intervention_resolved_event(),
                EventType::InterventionResolved,
                "intervention_resolved",
            ),
            (
                disposition_event(),
                EventType::ReviewDispositionRecorded,
                "review_disposition_recorded",
            ),
            (
                review_note_imported_event(),
                EventType::ReviewNoteImported,
                "review_note_imported",
            ),
        ];

        for (event, event_type, summary_kind) in cases {
            let entry = history_entry_from_event(&event, false, None).unwrap();
            let summary_json = serde_json::to_value(&entry.summary).unwrap();

            assert_eq!(entry.event_type, event_type);
            assert_eq!(summary_json["kind"], summary_kind);
        }
    }

    #[test]
    fn history_entry_omits_internal_artifact_paths() {
        let event = observation_event_with_artifact_path("artifacts/notes/body.json");

        let entry = history_entry_from_event(&event, false, None).unwrap();
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
    fn history_filters_by_review_unit_track_and_event_type() {
        let keep = observation_event("review-unit:sha256:one", "agent:codex", "Keep");
        let other_track = observation_event("review-unit:sha256:one", "agent:claude", "Drop track");
        let other_unit = observation_event("review-unit:sha256:two", "agent:codex", "Drop unit");
        let capture = review_unit_captured_event_for("review-unit:sha256:one");

        let filters = ResolvedHistoryFilters {
            review_unit_id: Some(ReviewUnitId::new("review-unit:sha256:one")),
            track_id: Some(TrackId::new("agent:codex")),
            event_types: vec![EventType::ReviewObservationRecorded],
            include_body: false,
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
            intervention_requested_event(),
            intervention_resolved_event(),
            disposition_event(),
            review_note_imported_event(),
        ];

        let result = history_from_events(&events, filters, None).unwrap();
        let entries = result
            .entries
            .iter()
            .map(|entry| serde_json::to_value(entry).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(entries[0]["summary"]["body"], "observation body");
        assert_eq!(entries[1]["summary"]["body"], "body");
        assert_eq!(entries[2]["summary"]["reason"], "approved");
        assert_eq!(entries[3]["summary"]["summary"], "ship it");
        assert_eq!(entries[4]["summary"]["body"], "body");
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
            event_types: vec![EventType::ReviewUnitCaptured],
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
        let review_id = ReviewId::new("review:default");
        let work_unit_id = WorkUnitId::new(format!("work:{key}"));
        ShoreEvent::new(
            EventType::ReviewInitialized,
            ReviewInitializedPayload::idempotency_key(&review_id, &work_unit_id),
            EventTarget::new(review_id, work_unit_id),
            Writer::shore_local_author("test"),
            ReviewInitializedPayload {},
            format!("2026-05-13T10:00:0{key}Z"),
        )
        .unwrap()
    }

    fn review_unit_captured_event() -> ShoreEvent {
        review_unit_captured_event_for("review-unit:sha256:one")
    }

    fn review_unit_captured_event_for(review_unit_id: &str) -> ShoreEvent {
        let review_unit_id = ReviewUnitId::new(review_unit_id);
        let payload = ReviewUnitCapturedPayload {
            review_unit_id: review_unit_id.clone(),
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
            revision_id: RevisionId::new(format!("rev:{}", review_unit_id.as_str())),
            snapshot_id: SnapshotId::new(format!("snap:{}", review_unit_id.as_str())),
            snapshot_artifact_content_hash: "sha256:artifact".to_owned(),
        };
        ShoreEvent::new(
            EventType::ReviewUnitCaptured,
            "capture:one",
            EventTarget::for_review_unit(
                ReviewId::new("review:default"),
                review_unit_id,
                payload.revision_id.clone(),
                payload.snapshot_id.clone(),
            ),
            Writer::shore_local_author("test"),
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
        let review_unit_id = ReviewUnitId::new(review_unit_id);
        let payload = ReviewObservationRecordedPayload {
            observation_id: ObservationId::new(format!("obs:sha256:{title}")),
            target: ReviewTargetRef::ReviewUnit {
                review_unit_id: review_unit_id.clone(),
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
            target: ReviewTargetRef::ReviewUnit {
                review_unit_id: review_unit_id("one"),
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
            target: ReviewTargetRef::ReviewUnit {
                review_unit_id: review_unit_id("one"),
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

    fn observation_event_with_id_and_key(
        observation_id: &str,
        idempotency_key: &str,
    ) -> ShoreEvent {
        let payload = ReviewObservationRecordedPayload {
            observation_id: ObservationId::new(observation_id),
            target: ReviewTargetRef::ReviewUnit {
                review_unit_id: review_unit_id("one"),
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

    fn intervention_requested_event() -> ShoreEvent {
        let payload = InterventionRequestedPayload {
            intervention_id: InterventionId::new("intervention:sha256:one"),
            target: ReviewTargetRef::ReviewUnit {
                review_unit_id: review_unit_id("one"),
            },
            mode: InterventionMode::Blocking,
            reason_code: InterventionReasonCode::ManualDecisionRequired,
            title: "Need decision".to_owned(),
            body: Some("body".to_owned()),
            body_artifact_path: None,
            body_byte_size: Some(4),
            body_content_hash: Some("sha256:body".to_owned()),
        };
        tracked_event(
            EventType::InterventionRequested,
            "intervention:request",
            "human:kevin",
            payload,
            "2026-05-13T10:00:02Z",
        )
    }

    fn intervention_resolved_event() -> ShoreEvent {
        let payload = InterventionResolvedPayload {
            intervention_resolution_id: InterventionResolutionId::new(
                "intervention-resolution:sha256:one",
            ),
            intervention_id: InterventionId::new("intervention:sha256:one"),
            outcome: InterventionResolutionOutcome::Approved,
            reason: Some("approved".to_owned()),
            reason_artifact_path: None,
            reason_byte_size: Some(8),
            reason_content_hash: Some("sha256:reason".to_owned()),
        };
        tracked_event(
            EventType::InterventionResolved,
            "intervention:resolved",
            "human:kevin",
            payload,
            "2026-05-13T10:00:03Z",
        )
    }

    fn disposition_event() -> ShoreEvent {
        let payload = ReviewDispositionRecordedPayload {
            disposition_id: DispositionId::new("disp:sha256:one"),
            target: ReviewTargetRef::ReviewUnit {
                review_unit_id: review_unit_id("one"),
            },
            disposition: ReviewDisposition::Accepted,
            summary: Some("ship it".to_owned()),
            summary_artifact_path: None,
            summary_byte_size: Some(7),
            summary_content_hash: Some("sha256:summary".to_owned()),
            replaces_disposition_ids: vec![],
            related_observation_ids: vec![ObservationId::new("obs:sha256:one")],
            related_intervention_ids: vec![InterventionId::new("intervention:sha256:one")],
            overrides: vec![],
        };
        tracked_event(
            EventType::ReviewDispositionRecorded,
            "disposition:one",
            "human:kevin",
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
            EventTarget::new(
                ReviewId::new("review:default"),
                WorkUnitId::new("work:default"),
            ),
            Writer::shore_local_reviewer("test"),
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
        review_unit_id: ReviewUnitId,
        payload: P,
        occurred_at: &str,
    ) -> ShoreEvent
    where
        P: crate::session::event::EventPayload,
    {
        let mut target = EventTarget::for_review_unit(
            ReviewId::new("review:default"),
            review_unit_id.clone(),
            revision_id("one"),
            snapshot_id("one"),
        );
        target.track_id = Some(TrackId::new(track_id));
        target.subject = Some(ReviewTargetRef::ReviewUnit { review_unit_id });
        ShoreEvent::new(
            event_type,
            idempotency_key,
            target,
            Writer::shore_local_reviewer("test"),
            payload,
            occurred_at,
        )
        .unwrap()
    }

    fn review_unit_id(suffix: &str) -> ReviewUnitId {
        ReviewUnitId::new(format!("review-unit:sha256:{suffix}"))
    }

    fn revision_id(suffix: &str) -> RevisionId {
        RevisionId::new(format!("rev:sha256:{suffix}"))
    }

    fn snapshot_id(suffix: &str) -> SnapshotId {
        SnapshotId::new(format!("snap:sha256:{suffix}"))
    }
}
