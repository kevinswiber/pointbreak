mod add;
mod show;
mod target;
mod util;
mod view;

pub use self::add::{DispositionAddOptions, DispositionAddResult, record_disposition};
pub use self::show::{
    DispositionShowFilters, DispositionShowOptions, DispositionShowResult, show_dispositions,
};
pub use self::target::{DispositionOverrideSelector, DispositionTargetSelector};
pub(crate) use self::target::{
    DispositionRelationships, resolve_disposition_relationships, resolve_disposition_target,
};
pub use self::view::{
    CurrentDispositionStatus, CurrentDispositionView, DispositionRecordStatus, DispositionView,
};
pub(crate) use self::view::{DispositionProjectionOptions, project_dispositions};

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::*;
    use crate::model::{
        DispositionId, InterventionId, ObservationId, ReviewId, ReviewTargetRef, ReviewUnitId,
        RevisionId, Side, SnapshotId, TrackId,
    };
    use crate::session::event::{
        EventTarget, EventType, InterventionMode, InterventionReasonCode, ReviewDisposition,
        ReviewDispositionRecordedPayload, ShoreEvent, Writer,
    };
    use crate::session::intervention::{InterventionRequestOptions, InterventionTargetSelector};
    use crate::session::observation::{
        ObservationAddOptions, ObservationTargetSelector, resolve_review_unit,
    };
    use crate::session::{
        CaptureOptions, EventStore, capture_worktree_review, record_observation,
        request_intervention,
    };

    #[test]
    fn resolves_current_review_unit_as_default_disposition_target() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let resolved = resolve_review_unit(&events, None).unwrap();

        let target = resolve_disposition_target(
            repo.path(),
            &events,
            &resolved,
            &DispositionTargetSelector::review_unit(),
        )
        .unwrap();

        assert_eq!(
            target.target,
            ReviewTargetRef::ReviewUnit {
                review_unit_id: capture.review_unit_id
            }
        );
    }

    #[test]
    fn resolves_file_and_range_targets_against_captured_snapshot() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let resolved = resolve_review_unit(&events, None).unwrap();

        let file = resolve_disposition_target(
            repo.path(),
            &events,
            &resolved,
            &DispositionTargetSelector::file("src/lib.rs"),
        )
        .unwrap();
        let range = resolve_disposition_target(
            repo.path(),
            &events,
            &resolved,
            &DispositionTargetSelector::range("src/lib.rs", Side::New, 2, Some(3)),
        )
        .unwrap();

        assert_eq!(
            file.target,
            ReviewTargetRef::File {
                review_unit_id: capture.review_unit_id.clone(),
                file_path: "src/lib.rs".to_owned()
            }
        );
        assert_eq!(
            range.target,
            ReviewTargetRef::Range {
                review_unit_id: capture.review_unit_id,
                file_path: "src/lib.rs".to_owned(),
                side: Side::New,
                start_line: 2,
                end_line: 3
            }
        );
    }

    #[test]
    fn resolves_observation_intervention_and_disposition_targets() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let observation = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Observation")
                .with_target(ObservationTargetSelector::file("src/lib.rs")),
        )
        .unwrap();
        let intervention = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired)
                .with_mode(InterventionMode::Blocking)
                .with_target(InterventionTargetSelector::review_unit()),
        )
        .unwrap();
        let disposition_id = DispositionId::new("disp:sha256:existing");
        let mut events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        events.push(disposition_event(&capture.review_unit_id, &disposition_id));
        let resolved = resolve_review_unit(&events, None).unwrap();

        let observation_target = resolve_disposition_target(
            repo.path(),
            &events,
            &resolved,
            &DispositionTargetSelector::observation(observation.observation_id.clone()),
        )
        .unwrap();
        let intervention_target = resolve_disposition_target(
            repo.path(),
            &events,
            &resolved,
            &DispositionTargetSelector::intervention(intervention.intervention_id.clone()),
        )
        .unwrap();
        let disposition_target = resolve_disposition_target(
            repo.path(),
            &events,
            &resolved,
            &DispositionTargetSelector::disposition(disposition_id.clone()),
        )
        .unwrap();

        assert_eq!(
            observation_target.target,
            ReviewTargetRef::Observation {
                review_unit_id: capture.review_unit_id.clone(),
                observation_id: observation.observation_id
            }
        );
        assert_eq!(
            intervention_target.target,
            ReviewTargetRef::Intervention {
                review_unit_id: capture.review_unit_id.clone(),
                intervention_id: intervention.intervention_id
            }
        );
        assert_eq!(
            disposition_target.target,
            ReviewTargetRef::Disposition {
                review_unit_id: capture.review_unit_id,
                disposition_id
            }
        );
    }

    #[test]
    fn rejects_unknown_related_observation_intervention_or_replacement() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let resolved = resolve_review_unit(&events, None).unwrap();

        let missing_observation = resolve_disposition_relationships(
            &events,
            &resolved,
            &DispositionRelationships {
                related_observation_ids: vec![ObservationId::new("obs:sha256:missing")],
                ..DispositionRelationships::default()
            },
            ReviewDisposition::Accepted,
            Some("summary"),
        )
        .unwrap_err();
        let missing_intervention = resolve_disposition_relationships(
            &events,
            &resolved,
            &DispositionRelationships {
                related_intervention_ids: vec![InterventionId::new("intervention:sha256:missing")],
                ..DispositionRelationships::default()
            },
            ReviewDisposition::Accepted,
            Some("summary"),
        )
        .unwrap_err();
        let missing_replacement = resolve_disposition_relationships(
            &events,
            &resolved,
            &DispositionRelationships {
                replaces_disposition_ids: vec![DispositionId::new("disp:sha256:missing")],
                ..DispositionRelationships::default()
            },
            ReviewDisposition::Accepted,
            Some("summary"),
        )
        .unwrap_err();

        assert!(
            missing_observation
                .to_string()
                .contains("unknown observation")
        );
        assert!(
            missing_intervention
                .to_string()
                .contains("unknown intervention")
        );
        assert!(
            missing_replacement
                .to_string()
                .contains("unknown disposition")
        );
    }

    #[test]
    fn overridden_requires_summary_and_override_reference() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let resolved = resolve_review_unit(&events, None).unwrap();

        let missing_summary = resolve_disposition_relationships(
            &events,
            &resolved,
            &DispositionRelationships {
                overrides: vec![DispositionOverrideSelector::observation(
                    ObservationId::new("obs:sha256:missing"),
                )],
                ..DispositionRelationships::default()
            },
            ReviewDisposition::Overridden,
            None,
        )
        .unwrap_err();
        let missing_override = resolve_disposition_relationships(
            &events,
            &resolved,
            &DispositionRelationships::default(),
            ReviewDisposition::Overridden,
            Some("manual override"),
        )
        .unwrap_err();

        assert!(missing_summary.to_string().contains("summary is required"));
        assert!(
            missing_override
                .to_string()
                .contains("override reference is required")
        );
    }

    #[test]
    fn record_disposition_writes_event_and_updates_state() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Accepted)
                .with_summary("Ship this"),
        )
        .unwrap();

        assert_eq!(result.review_unit_id, capture.review_unit_id);
        assert!(result.disposition_id.as_str().starts_with("disp:sha256:"));
        assert_eq!(result.track_id.as_str(), "human:kevin");
        assert_eq!(result.disposition, ReviewDisposition::Accepted);
        assert_eq!(
            result.events_created_by_type["review_disposition_recorded"],
            1
        );

        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let state = crate::session::SessionState::from_events(&events).unwrap();
        assert_eq!(state.disposition_count, 1);
    }

    #[test]
    fn record_disposition_is_idempotent_for_same_logical_input() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let options = DispositionAddOptions::new(repo.path())
            .with_track("human:kevin")
            .with_disposition(ReviewDisposition::Accepted)
            .with_summary("same summary");

        let first = record_disposition(options.clone()).unwrap();
        let second = record_disposition(options).unwrap();

        assert_eq!(first.disposition_id, second.disposition_id);
        assert_eq!(first.events_created, 1);
        assert_eq!(second.events_created, 0);
        assert_eq!(second.events_existing, 1);
    }

    #[test]
    fn explicit_same_idempotency_key_with_different_payload_conflicts() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Accepted)
                .with_summary("first")
                .with_idempotency_key("retry-key"),
        )
        .unwrap();
        let error = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::NeedsChanges)
                .with_summary("second")
                .with_idempotency_key("retry-key"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("event conflict"));
    }

    #[test]
    fn large_summary_is_stored_as_internal_body_artifact() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let summary = "x".repeat(crate::session::body_artifact::BODY_INLINE_LIMIT + 1);

        let result = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::AcceptedWithFollowUp)
                .with_summary(summary),
        )
        .unwrap();

        assert!(
            result
                .summary_content_hash
                .as_deref()
                .unwrap()
                .starts_with("sha256:")
        );
        assert!(
            !format!("{result:?}").contains("artifacts/notes/"),
            "workflow result must not expose internal artifact paths"
        );
    }

    #[test]
    fn replacement_records_new_disposition_with_replaces_link() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::NeedsChanges)
                .with_summary("Fix this"),
        )
        .unwrap();

        let replacement = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Accepted)
                .with_summary("Fixed")
                .replacing(first.disposition_id.clone()),
        )
        .unwrap();
        let payload = disposition_payload(&repo, &replacement.disposition_id);

        assert_eq!(payload.replaces_disposition_ids, vec![first.disposition_id]);
    }

    #[test]
    fn show_disposition_deduplicates_and_sorts_replaces() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::NeedsChanges)
                .with_summary("First"),
        )
        .unwrap();
        let second = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::NeedsClarification)
                .with_summary("Second"),
        )
        .unwrap();
        let replacement = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Accepted)
                .with_summary("Fixed")
                .replacing(second.disposition_id.clone())
                .replacing(first.disposition_id.clone())
                .replacing(first.disposition_id.clone()),
        )
        .unwrap();
        let mut expected = vec![first.disposition_id, second.disposition_id];
        expected.sort();

        let result =
            show_dispositions(DispositionShowOptions::new(repo.path()).with_all(true)).unwrap();
        let view = result
            .dispositions
            .iter()
            .find(|view| view.id == replacement.disposition_id)
            .expect("replacement disposition appears in all view");

        assert_eq!(view.replaces, expected);
    }

    #[test]
    fn override_references_are_metadata_not_replacement() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::NeedsChanges)
                .with_summary("Fix this"),
        )
        .unwrap();

        let override_result = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Overridden)
                .with_summary("Human override")
                .overriding_disposition(first.disposition_id.clone()),
        )
        .unwrap();
        let payload = disposition_payload(&repo, &override_result.disposition_id);

        assert!(payload.replaces_disposition_ids.is_empty());
        assert_eq!(
            payload.overrides,
            vec![ReviewTargetRef::Disposition {
                review_unit_id: override_result.review_unit_id,
                disposition_id: first.disposition_id
            }]
        );
    }

    #[test]
    fn override_order_does_not_change_disposition_id() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::NeedsChanges)
                .with_summary("First"),
        )
        .unwrap();
        let second = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::NeedsClarification)
                .with_summary("Second"),
        )
        .unwrap();

        let forward = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Overridden)
                .with_summary("Manual override")
                .overriding_disposition(first.disposition_id.clone())
                .overriding_disposition(second.disposition_id.clone()),
        )
        .unwrap();
        let reversed = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Overridden)
                .with_summary("Manual override")
                .overriding_disposition(second.disposition_id)
                .overriding_disposition(first.disposition_id),
        )
        .unwrap();

        assert_eq!(forward.disposition_id, reversed.disposition_id);
        assert_eq!(forward.events_created, 1);
        assert_eq!(reversed.events_created, 0);
    }

    #[test]
    fn show_disposition_reports_none_when_no_disposition_exists() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = show_dispositions(DispositionShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.current.status, CurrentDispositionStatus::None);
        assert!(result.current.dispositions.is_empty());
        assert!(result.dispositions.is_empty());
    }

    #[test]
    fn show_disposition_reports_one_unreplaced_current_disposition() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let disposition = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Accepted)
                .with_summary("Ship it"),
        )
        .unwrap();

        let result = show_dispositions(DispositionShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.current.status, CurrentDispositionStatus::Resolved);
        assert_eq!(result.current.dispositions.len(), 1);
        assert_eq!(
            result.current.dispositions[0].id,
            disposition.disposition_id
        );
        assert_eq!(
            result.dispositions[0].status,
            DispositionRecordStatus::Current
        );
    }

    #[test]
    fn show_disposition_reports_ambiguous_for_multiple_unreplaced_dispositions() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Accepted)
                .with_summary("Ship it"),
        )
        .unwrap();
        record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_disposition(ReviewDisposition::NeedsChanges)
                .with_summary("Needs one fix"),
        )
        .unwrap();

        let result = show_dispositions(DispositionShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.current.status, CurrentDispositionStatus::Ambiguous);
        assert_eq!(result.current.dispositions.len(), 2);
    }

    #[test]
    fn show_disposition_excludes_replaced_records_by_default_and_includes_with_all() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::NeedsChanges)
                .with_summary("Fix this"),
        )
        .unwrap();
        let replacement = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Accepted)
                .with_summary("Fixed")
                .replacing(first.disposition_id.clone()),
        )
        .unwrap();

        let current = show_dispositions(DispositionShowOptions::new(repo.path())).unwrap();
        let all =
            show_dispositions(DispositionShowOptions::new(repo.path()).with_all(true)).unwrap();

        assert_eq!(current.current.status, CurrentDispositionStatus::Resolved);
        assert_eq!(
            current
                .dispositions
                .iter()
                .map(|view| view.id.clone())
                .collect::<Vec<_>>(),
            vec![replacement.disposition_id.clone()]
        );
        assert_eq!(all.dispositions.len(), 2);
        assert!(
            all.dispositions
                .iter()
                .any(|view| view.id == first.disposition_id
                    && view.status == DispositionRecordStatus::Replaced)
        );
    }

    #[test]
    fn show_disposition_filters_by_track() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let human = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Accepted)
                .with_summary("Ship it"),
        )
        .unwrap();
        record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_disposition(ReviewDisposition::NeedsChanges)
                .with_summary("Needs one fix"),
        )
        .unwrap();

        let result =
            show_dispositions(DispositionShowOptions::new(repo.path()).with_track("human:kevin"))
                .unwrap();

        assert_eq!(result.current.status, CurrentDispositionStatus::Resolved);
        assert_eq!(result.dispositions.len(), 1);
        assert_eq!(result.dispositions[0].id, human.disposition_id);
        assert_eq!(
            result.filters.track_id.as_ref().unwrap().as_str(),
            "human:kevin"
        );
    }

    #[test]
    fn show_disposition_hydrates_summary_only_when_requested() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let summary = "x".repeat(crate::session::body_artifact::BODY_INLINE_LIMIT + 1);
        record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Accepted)
                .with_summary(summary.clone()),
        )
        .unwrap();

        let without_summary = show_dispositions(DispositionShowOptions::new(repo.path())).unwrap();
        let with_summary =
            show_dispositions(DispositionShowOptions::new(repo.path()).with_include_summary(true))
                .unwrap();

        assert!(without_summary.dispositions[0].summary.is_none());
        assert_eq!(
            with_summary.dispositions[0].summary.as_deref(),
            Some(summary.as_str())
        );
        assert!(!format!("{with_summary:?}").contains("artifacts/notes/"));
    }

    #[test]
    fn show_disposition_collapses_duplicate_semantic_events() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let options = DispositionAddOptions::new(repo.path())
            .with_track("human:kevin")
            .with_disposition(ReviewDisposition::Accepted)
            .with_summary("same summary");
        let first = record_disposition(options.clone().with_idempotency_key("retry-a")).unwrap();
        record_disposition(options.with_idempotency_key("retry-b")).unwrap();

        let result = show_dispositions(DispositionShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.dispositions.len(), 1);
        assert_eq!(result.dispositions[0].id, first.disposition_id);
        assert!(result.diagnostics.iter().any(|diagnostic| diagnostic.code
            == crate::session::state::DUPLICATE_SEMANTIC_DISPOSITION_EVENT_CODE));
    }

    #[test]
    fn show_disposition_sorts_by_created_at_then_event_id() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Accepted)
                .with_summary("First"),
        )
        .unwrap();
        let second = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_disposition(ReviewDisposition::NeedsChanges)
                .with_summary("Second"),
        )
        .unwrap();

        let result = show_dispositions(DispositionShowOptions::new(repo.path())).unwrap();

        assert_eq!(
            result
                .dispositions
                .iter()
                .map(|view| view.id.clone())
                .collect::<Vec<_>>(),
            vec![first.disposition_id, second.disposition_id]
        );
    }

    #[test]
    fn show_disposition_uses_replaces_not_overrides_for_current_projection() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::NeedsChanges)
                .with_summary("Fix this"),
        )
        .unwrap();
        record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Overridden)
                .with_summary("Manual override")
                .overriding_disposition(first.disposition_id),
        )
        .unwrap();

        let result = show_dispositions(DispositionShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.current.status, CurrentDispositionStatus::Ambiguous);
        assert_eq!(result.current.dispositions.len(), 2);
    }

    fn disposition_event(
        review_unit_id: &ReviewUnitId,
        disposition_id: &DispositionId,
    ) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewDispositionRecorded,
            ReviewDispositionRecordedPayload::idempotency_key(
                review_unit_id,
                &TrackId::new("human:kevin"),
                disposition_id.as_str(),
            ),
            EventTarget {
                review_id: ReviewId::new("review:default"),
                work_unit_id: None,
                review_unit_id: Some(review_unit_id.clone()),
                revision_id: Some(RevisionId::new("rev:git:sha256:one")),
                snapshot_id: Some(SnapshotId::new("snap:git:sha256:one")),
                track_id: Some(TrackId::new("human:kevin")),
                subject: Some(ReviewTargetRef::ReviewUnit {
                    review_unit_id: review_unit_id.clone(),
                }),
            },
            Writer::shore_local_reviewer("test"),
            ReviewDispositionRecordedPayload {
                disposition_id: disposition_id.clone(),
                target: ReviewTargetRef::ReviewUnit {
                    review_unit_id: review_unit_id.clone(),
                },
                disposition: ReviewDisposition::Accepted,
                summary: Some("Accepted".to_owned()),
                summary_artifact_path: None,
                summary_byte_size: Some(8),
                summary_content_hash: Some("sha256:accepted".to_owned()),
                replaces_disposition_ids: vec![],
                related_observation_ids: vec![],
                related_intervention_ids: vec![],
                overrides: vec![],
            },
            "2026-05-12T00:00:00Z",
        )
        .unwrap()
    }

    fn disposition_payload(
        repo: &TestRepo,
        disposition_id: &DispositionId,
    ) -> ReviewDispositionRecordedPayload {
        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        events
            .into_iter()
            .filter(|event| event.event_type == EventType::ReviewDispositionRecorded)
            .map(|event| serde_json::from_value(event.payload).unwrap())
            .find(|payload: &ReviewDispositionRecordedPayload| {
                &payload.disposition_id == disposition_id
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
