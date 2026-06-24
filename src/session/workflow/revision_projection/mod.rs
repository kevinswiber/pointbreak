use std::collections::{BTreeMap, HashMap};

use crate::error::Result;
use crate::model::{DiffSnapshot, EventId, ReviewId};
use crate::session::assessment::{AssessmentProjectionOptions, project_assessments};
use crate::session::event::ShoreEvent;
use crate::session::input_request::{
    InputRequestProjectionOptions, InputRequestStatusFilter, project_input_requests,
};
use crate::session::observation::{
    CurrentRevisionContext, ObservationProjectionOptions, RevisionScope, RevisionSelection,
    project_observations, resolve_revision, validated_track_id,
};
use crate::session::projection::cosignature::{
    CosignatureIndex, endorsement_readbacks, enrich_endorser_attributes,
};
use crate::session::projection::{
    ArtifactRemovalProjection, RemovalOperativeStatus, skipped_to_diagnostics,
};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store::resolution::resolve_read_store;
use crate::session::workflow::{ValidationCheckProjectionOptions, project_validation_checks};
use crate::session::{
    EventStore, RevisionCommitRangeProjection, RevisionCommitRangeView, verify_event_signature,
};

mod adapter_notes;
mod identity;
mod resolving;
mod rows;
mod snapshot;

pub use self::adapter_notes::AdapterNoteView;
use self::adapter_notes::project_adapter_notes;
use self::identity::principal_diagnostics;
pub use self::identity::{
    MemberReadback, RevisionProjectionIdentity, RevisionProjectionSummary, RevisionShowFilters,
    RevisionShowOptions, RevisionShowResult, SnapshotContentState,
};
use self::resolving::selected_revision_capture;
pub use self::rows::{RevisionProjectionRow, SnapshotOrder};
use self::rows::{
    build_adapter_note_rows, build_assessment_rows, build_input_request_rows,
    build_observation_rows, build_snapshot_rows, build_validation_rows, renumber_projection_rows,
};
use self::snapshot::{SnapshotContent, resolve_snapshot_content};

/// A removal is recorded for the bound snapshot content, but its bytes are still
/// stored: the suppression is reversible and a compact would reclaim them.
const SNAPSHOT_CONTENT_SUPPRESSED_PRESENT: &str = "snapshot_content_suppressed_present";
/// A removal is recorded for the bound snapshot content and its bytes have been
/// swept from the store.
const SNAPSHOT_CONTENT_PHYSICALLY_REMOVED: &str = "snapshot_content_physically_removed";
/// The bound content carries an unsigned removal claim that is not operative.
const REMOVAL_CLAIM_UNSIGNED: &str = "removal_claim_unsigned";
/// The bound content carries a removal signed by an untrusted key, not operative.
const REMOVAL_CLAIM_UNTRUSTED: &str = "removal_claim_untrusted";
/// The bound content carries a removal whose signature verifies invalid (the
/// integrity floor); not operative under any policy.
const REMOVAL_CLAIM_INVALID: &str = "removal_claim_invalid";
/// A removal targets a content hash that no event in this store references.
const SNAPSHOT_CONTENT_REMOVED_TARGET_MISSING: &str = "snapshot_content_removed_target_missing";
/// A capture re-binds a content hash that carries an operative removal.
const IDENTITY_REUSED_AFTER_REMOVAL: &str = "identity_reused_after_removal";

pub fn show_revision(options: RevisionShowOptions) -> Result<RevisionShowResult> {
    let read_store = resolve_read_store(&options.repo)?;
    let track_id = options
        .track
        .as_deref()
        .map(validated_track_id)
        .transpose()?;
    let store = EventStore::from_backend(read_store.backend());
    let (events, skip_diagnostics) = if options.read_for_display {
        let (events, skipped) = store.list_events_lenient()?;
        (events, skipped_to_diagnostics(skipped))
    } else {
        (store.list_events()?, Vec::new())
    };
    let resolved = resolve_revision(
        &events,
        RevisionSelection::from_revision_seed(options.revision_id.as_ref()),
        &CurrentRevisionContext::for_repo(&options.repo)?,
        RevisionScope::default(),
    )?;
    let revision = selected_revision_capture(&events, &resolved)?;
    let removal = ArtifactRemovalProjection::from_events(&events)?;
    // Built once near the removal projection and shared downstream: the
    // suppression decision, the claim diagnostics, and the endorsement readback
    // block all read this one index (do not build a second).
    let cosig_index = CosignatureIndex::build(&events)?;
    // The bound hash's operative status is decided once here so the same decision
    // drives both suppression and the claim diagnostics.
    let bound_status = removal.operative_status(
        &revision.object_artifact_content_hash,
        &options.trust_set,
        options.removal_policy,
        &cosig_index,
    )?;
    let snapshot_content = resolve_snapshot_content(&options.repo, &revision, bound_status)?;
    let snapshot_content_state = SnapshotContentState::from(&snapshot_content);
    let (snapshot, removed_snapshot_content_hash) = match snapshot_content {
        SnapshotContent::Present(snapshot) => (snapshot, None),
        SnapshotContent::SuppressedPresent { content_hash }
        | SnapshotContent::PhysicallyRemoved { content_hash } => (
            DiffSnapshot::new(
                ReviewId::new(revision.journal_id.as_str()),
                revision.object_id.clone(),
                Vec::new(),
            ),
            Some(content_hash),
        ),
    };
    let observations = project_observations(ObservationProjectionOptions {
        backend: read_store.backend(),
        events: &events,
        resolved: &resolved,
        track_filter: track_id.clone(),
        file_filter: None,
        tag_filters: &[],
        include_body: options.include_body,
    })?;
    let input_requests = project_input_requests(InputRequestProjectionOptions {
        backend: read_store.backend(),
        events: &events,
        resolved: &resolved,
        track_filter: track_id.clone(),
        mode_filter: None,
        file_filter: None,
        status_filter: InputRequestStatusFilter::All,
        include_body: options.include_body,
    })?;
    let (current_assessment, assessments) = project_assessments(AssessmentProjectionOptions {
        backend: Some(read_store.backend()),
        events: &events,
        resolved: &resolved,
        track_filter: track_id.clone(),
        include_summary: options.include_body,
        include_all: true,
    })?;
    let validation_checks = project_validation_checks(ValidationCheckProjectionOptions {
        backend: read_store.backend(),
        events: &events,
        revision_id: &resolved.revision_id,
        track_filter: track_id.clone(),
        status_filter: None,
        include_body: options.include_body,
    })?;
    let adapter_notes = project_adapter_notes(
        &events,
        read_store.backend(),
        &snapshot,
        options.include_body,
    )?;
    let (snapshot_rows, mut summary) = if removed_snapshot_content_hash.is_some() {
        // Removed content has no snapshot rows; the explained absence is carried
        // by the result field and the diagnostic below, not a misleading
        // empty-state row.
        (Vec::new(), RevisionProjectionSummary::default())
    } else {
        build_snapshot_rows(&snapshot, &revision.id)
    };
    let mut narrative_rows = Vec::new();
    let observation_rows = build_observation_rows(&observations);
    summary.observation_count = observations.len();
    narrative_rows.extend(observation_rows);
    let input_request_rows = build_input_request_rows(&input_requests);
    summary.input_request_count = input_requests.len();
    narrative_rows.extend(input_request_rows);
    let assessment_rows = build_assessment_rows(&assessments);
    summary.assessment_count = assessments.len();
    narrative_rows.extend(assessment_rows);
    let validation_rows = build_validation_rows(&validation_checks);
    summary.validation_check_count = validation_checks.len();
    narrative_rows.extend(validation_rows);
    let adapter_note_rows = build_adapter_note_rows(&adapter_notes, &revision.id);
    summary.adapter_note_count = adapter_notes.len();
    narrative_rows.extend(adapter_note_rows);
    summary.narrative_row_count = narrative_rows.len();
    summary.row_count = summary.narrative_row_count + summary.snapshot_remainder_row_count;
    let mut rows = narrative_rows;
    rows.extend(snapshot_rows);
    renumber_projection_rows(&mut rows);
    let state = SessionState::from_events(&events)?;
    let event_set_hash = state
        .event_set_hash
        .clone()
        .expect("SessionState::from_events sets event_set_hash");
    let mut diagnostics = state.diagnostics;
    diagnostics.extend(skip_diagnostics);
    if let Some(content_hash) = &removed_snapshot_content_hash {
        match snapshot_content_state {
            SnapshotContentState::SuppressedPresent => diagnostics.push(ProjectionDiagnostic {
                code: SNAPSHOT_CONTENT_SUPPRESSED_PRESENT.to_owned(),
                message: format!(
                    "snapshot content {content_hash} is suppressed by a recorded removal; the bytes \
                     are still stored and a compact would reclaim them"
                ),
            }),
            SnapshotContentState::PhysicallyRemoved => diagnostics.push(ProjectionDiagnostic {
                code: SNAPSHOT_CONTENT_PHYSICALLY_REMOVED.to_owned(),
                message: format!(
                    "snapshot content {content_hash} was removed and its bytes have been swept from \
                     the store"
                ),
            }),
            SnapshotContentState::Present => {}
        }
    }

    // The bound hash's claim diagnostic, mapped from the operative status decided
    // once above (reused, not recomputed). A non-operative claim renders the bytes
    // and surfaces here instead of silently suppressing.
    let bound_hash = &revision.object_artifact_content_hash;
    match bound_status {
        RemovalOperativeStatus::ClaimUnsigned => diagnostics.push(ProjectionDiagnostic {
            code: REMOVAL_CLAIM_UNSIGNED.to_owned(),
            message: format!(
                "removal of snapshot content {bound_hash} is unsigned and not operative; ratify it \
                 with `shore review endorse` or extend trust"
            ),
        }),
        RemovalOperativeStatus::ClaimUntrusted => diagnostics.push(ProjectionDiagnostic {
            code: REMOVAL_CLAIM_UNTRUSTED.to_owned(),
            message: format!(
                "removal of snapshot content {bound_hash} is signed by an untrusted key and not \
                 operative; ratify it or extend trust"
            ),
        }),
        RemovalOperativeStatus::ClaimInvalid => diagnostics.push(ProjectionDiagnostic {
            code: REMOVAL_CLAIM_INVALID.to_owned(),
            message: format!(
                "removal of snapshot content {bound_hash} has an invalid signature (integrity \
                 floor) and is not operative; re-issue it cleanly"
            ),
        }),
        RemovalOperativeStatus::NoClaim
        | RemovalOperativeStatus::OperativePossession
        | RemovalOperativeStatus::OperativeTrusted => {}
    }

    // Projection-scope removal diagnostics, sharing the hoisted cosig index: a
    // removal of a never-referenced hash, and a capture re-binding an operatively
    // removed hash.
    for content_hash in removal.target_missing_diagnostics(&events)? {
        diagnostics.push(ProjectionDiagnostic {
            code: SNAPSHOT_CONTENT_REMOVED_TARGET_MISSING.to_owned(),
            message: format!(
                "removal targets content {content_hash}, which no event in this store references"
            ),
        });
    }
    for reuse in removal.identity_reuse_diagnostics(
        &events,
        &options.trust_set,
        options.removal_policy,
        &cosig_index,
    )? {
        diagnostics.push(ProjectionDiagnostic {
            code: IDENTITY_REUSED_AFTER_REMOVAL.to_owned(),
            message: format!(
                "revision {} re-binds removed snapshot content {} ({})",
                reuse.revision_id.as_str(),
                reuse.content_hash,
                reuse.kind.as_str()
            ),
        });
    }

    // Git-free commit-range lifecycle: fold the association events into the resolved
    // unit's view and surface its diagnostics. Liveness is layered by repo-holding
    // callers, never here.
    let commit_range = RevisionCommitRangeProjection::from_events(&events)?
        .unit(&resolved.revision_id)
        .cloned()
        .unwrap_or_else(|| RevisionCommitRangeView {
            revision_id: resolved.revision_id.clone(),
            anchored: false,
            current_commits: Vec::new(),
            current_refs: Vec::new(),
            withdrawn_commits: Vec::new(),
            withdrawn_refs: Vec::new(),
            diagnostics: Vec::new(),
        });
    diagnostics.extend(commit_range.diagnostics.clone());

    if let Some(map) = options.delegation_map.as_ref() {
        let members = observations
            .iter()
            .map(|view| (&view.writer.actor_id, view.created_at.as_str()))
            .chain(input_requests.iter().flat_map(|request| {
                std::iter::once((&request.writer.actor_id, request.created_at.as_str())).chain(
                    request
                        .responses
                        .iter()
                        .map(|response| (&response.writer.actor_id, response.created_at.as_str())),
                )
            }))
            .chain(
                assessments
                    .iter()
                    .map(|view| (&view.writer.actor_id, view.created_at.as_str())),
            )
            .chain(
                validation_checks
                    .iter()
                    .map(|view| (&view.writer.actor_id, view.created_at.as_str())),
            );
        diagnostics.extend(principal_diagnostics(members, map));
    }

    // Reader-relative readback, keyed by event id and computed once over the events
    // already in scope. Presence of a verification policy enables it; advisory render
    // only, never a gate. The document layer attaches it by event id.
    let mut member_readbacks: BTreeMap<EventId, MemberReadback> = BTreeMap::new();
    if options.verification_policy.is_some() {
        let by_id: HashMap<&str, &ShoreEvent> =
            events.iter().map(|e| (e.event_id.as_str(), e)).collect();
        // Reuses the `cosig_index` hoisted near the removal projection above.
        let mut record = |event_id: &EventId| -> Result<()> {
            if let Some(event) = by_id.get(event_id.as_str()) {
                let entry = member_readbacks.entry(event_id.clone()).or_default();
                entry.verification_status =
                    Some(verify_event_signature(event, &options.trust_set)?);
                // Trust-only classification, then sibling enrichment.
                let mut readbacks = endorsement_readbacks(
                    &cosig_index.cosignatures_for_target(event, &options.trust_set)?,
                );
                enrich_endorser_attributes(&mut readbacks, options.actor_attributes.as_ref());
                entry.endorsements = readbacks;
            }
            Ok(())
        };
        record(&revision.capture_event_id)?;
        for view in &observations {
            record(&view.event_id)?;
        }
        for request in &input_requests {
            record(&request.event_id)?;
            for response in &request.responses {
                record(&response.event_id)?;
            }
        }
        for view in &assessments {
            record(&view.event_id)?;
        }
        for view in &validation_checks {
            record(&view.event_id)?;
        }
    }

    Ok(RevisionShowResult {
        event_set_hash,
        event_count: events.len(),
        revision,
        snapshot,
        removed_snapshot_content_hash,
        snapshot_content_state,
        filters: RevisionShowFilters {
            revision_id: resolved.revision_id,
            track_id,
            include_body: options.include_body,
        },
        summary,
        current_assessment,
        observations,
        input_requests,
        assessments,
        validation_checks,
        adapter_notes,
        rows,
        commit_range,
        member_readbacks,
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::rows::RevisionProjectionRowKind;
    use super::*;
    use crate::canonical_hash::sha256_json_prefixed;
    use crate::crypto::{EventSignatureBytes, EventSigner};
    use crate::model::{
        DiffSnapshot, EngagementId, ObjectId, ReviewId, RevisionId, ValidationCheckId,
        ValidationStatus, ValidationTarget, ValidationTrigger,
    };
    use crate::session::event::{
        ArtifactRemovedPayload, EventSignature, EventTarget, EventToBeSigned, EventType,
        IngestProvenance, IngestVia, InputRequestReasonCode, InputRequestResponseOutcome,
        ReviewAssessment, Revision, ShoreEvent, ValidationCheckRecordedPayload, WorkObjectProposal,
        WorkObjectProposedPayload, Writer, event_signature_pre_authentication_encoding,
    };
    use crate::session::signing::test_support::DeterministicSigner;
    use crate::session::{
        AssessmentAddOptions, AssessmentShowOptions, CaptureOptions, CaptureResult,
        CurrentAssessmentStatus, EventStore, ImportNotesOptions, InputRequestListOptions,
        InputRequestOpenOptions, InputRequestRespondOptions, InputRequestStatus,
        InputRequestStatusFilter, ObservationAddOptions, ObservationListOptions,
        ObservationTargetSelector, RemovalPolicy, capture_worktree_review, import_notes,
        list_input_requests, list_observations, open_input_request, record_assessment,
        record_observation, respond_input_request, show_assessments,
    };

    #[test]
    fn show_revision_errors_when_no_revision_is_captured() {
        let repo = modified_repo();

        let error = show_revision(RevisionShowOptions::new(repo.path()))
            .expect_err("no captured Revision should fail");

        assert!(error.to_string().contains("no captured revision"));
    }

    #[test]
    fn show_revision_resolves_single_current_revision_and_freshness() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.revision.id, capture.revision_id);
        assert_eq!(result.revision.revision_id, capture.revision_id);
        assert_eq!(result.revision.object_id, capture.object_id);
        assert_eq!(result.filters.revision_id, capture.revision_id);
        // Capture event plus the auto-recorded capture-time ref association.
        assert_eq!(result.event_count, 2);
        assert!(result.event_set_hash.starts_with("sha256:"));
    }

    #[test]
    fn show_revision_strict_by_default_lenient_when_opted_in() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        // A retired-type event in the store: the probe rejects it before decode.
        let events_dir = resolve_read_store(repo.path())
            .unwrap()
            .store_dir()
            .join("events");
        fs::write(
            events_dir.join(format!("{}.json", "a".repeat(64))),
            br#"{"eventType":"review_disposition_recorded"}"#,
        )
        .unwrap();

        // Default (the relay/strict case): a retired event hard-fails the read.
        assert!(show_revision(RevisionShowOptions::new(repo.path())).is_err());

        // Opted in (CLI/inspector): the retired event is skipped and surfaced.
        let result =
            show_revision(RevisionShowOptions::new(repo.path()).with_read_for_display(true))
                .unwrap();
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "unsupported_event_type")
        );
    }

    #[test]
    fn show_revision_includes_validation_checks() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_validation_event(repo.path(), &capture, "validation:sha256:one");

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.validation_checks.len(), 1);
        assert_eq!(result.summary.validation_check_count, 1);
        let row = result
            .rows
            .iter()
            .find(|row| row.kind == RevisionProjectionRowKind::ValidationEvidence)
            .expect("validation evidence row");
        assert_eq!(
            row.related_validation_check_ids,
            vec![result.validation_checks[0].id.clone()]
        );
        let first_snapshot_remainder = result
            .rows
            .iter()
            .position(|row| row.projection_phase.as_str() == "snapshot_remainder")
            .expect("snapshot remainder starts");
        let validation_row = result
            .rows
            .iter()
            .position(|row| row.kind == RevisionProjectionRowKind::ValidationEvidence)
            .unwrap();
        assert!(validation_row < first_snapshot_remainder);
    }

    #[test]
    fn non_validation_rows_have_empty_related_validation_check_ids() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_revision_id(capture.revision_id)
                .with_track("agent:codex")
                .with_title("Observation"),
        )
        .unwrap();

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert!(result.rows.iter().all(|row| {
            row.kind == RevisionProjectionRowKind::ValidationEvidence
                || row.related_validation_check_ids.is_empty()
        }));
    }

    fn capture_with_agent_observation() -> (TestRepo, RevisionId) {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_revision_id(capture.revision_id.clone())
                .with_track("agent:claude-code")
                .with_actor_id(crate::model::ActorId::new("actor:agent:claude-code"))
                .with_title("Agent observation"),
        )
        .unwrap();
        (repo, capture.revision_id)
    }

    #[test]
    fn unit_show_emits_diagnostic_for_unresolvable_agent_principal() {
        let (repo, revision_id) = capture_with_agent_observation();
        // A map that does not know this agent → no_delegation_record.
        let map = crate::session::delegation_map_from_value(serde_json::json!({
            "delegates": {}
        }))
        .unwrap();

        let result = show_revision(
            RevisionShowOptions::new(repo.path())
                .with_revision_id(revision_id)
                .with_delegation_map(map),
        )
        .unwrap();

        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "principal_unresolvable")
            .expect("an unresolvable agent principal emits a diagnostic");
        assert!(diagnostic.message.contains("actor:agent:claude-code"));
        assert!(diagnostic.message.contains("no_delegation_record"));
    }

    #[test]
    fn unit_show_emits_diagnostic_for_ambiguous_principal() {
        let (repo, revision_id) = capture_with_agent_observation();
        // Two overlapping open windows with distinct principals → ambiguous.
        let map = crate::session::delegation_map_from_value(serde_json::json!({
            "delegates": {
                "actor:agent:claude-code": [
                    { "principal": "actor:git-email:kevin@swiber.dev",
                      "validFrom": "2020-01-01T00:00:00Z", "validUntil": null },
                    { "principal": "actor:git-email:alice@example.com",
                      "validFrom": "2020-01-01T00:00:00Z", "validUntil": null }
                ]
            }
        }))
        .unwrap();

        let result = show_revision(
            RevisionShowOptions::new(repo.path())
                .with_revision_id(revision_id)
                .with_delegation_map(map),
        )
        .unwrap();

        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "principal_ambiguous")
            .expect("an ambiguous agent principal emits a diagnostic");
        assert!(
            diagnostic
                .message
                .contains("actor:git-email:kevin@swiber.dev")
        );
        assert!(
            diagnostic
                .message
                .contains("actor:git-email:alice@example.com")
        );
    }

    #[test]
    fn unit_show_without_map_emits_no_principal_diagnostics() {
        let (repo, revision_id) = capture_with_agent_observation();
        let result =
            show_revision(RevisionShowOptions::new(repo.path()).with_revision_id(revision_id))
                .unwrap();
        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.code.starts_with("principal_")),
            "no map supplied → no principal diagnostics"
        );
    }

    #[test]
    fn show_revision_requires_explicit_id_when_current_is_ambiguous() {
        let repo = modified_repo();
        let first = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
        let second = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let error = show_revision(RevisionShowOptions::new(repo.path()))
            .expect_err("multiple captures should be ambiguous");
        assert!(error.to_string().contains("multiple captured revisions"));

        let explicit = show_revision(
            RevisionShowOptions::new(repo.path()).with_revision_id(first.revision_id.clone()),
        )
        .unwrap();

        assert_ne!(first.revision_id, second.revision_id);
        assert_eq!(explicit.revision.id, first.revision_id);
        // Two worktree captures, each with its auto-recorded ref association.
        assert_eq!(explicit.event_count, 4);
    }

    #[test]
    fn show_revision_uses_captured_snapshot_after_worktree_drift() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 99 }\n");

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.revision.id, capture.revision_id);
        assert_eq!(
            result.snapshot.files[0].new_path.as_deref(),
            Some("src/lib.rs")
        );
        assert!(format!("{:?}", result.snapshot).contains("2"));
        assert!(!format!("{:?}", result.snapshot).contains("99"));
    }

    #[test]
    fn show_revision_rejects_object_artifact_hash_mismatch() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        tamper_object_artifact_snapshot_field(repo.path(), &capture.object_id);

        let error = show_revision(RevisionShowOptions::new(repo.path()))
            .expect_err("tampered artifact should fail");

        assert!(error.to_string().contains("content hash"));
    }

    #[test]
    fn show_revision_rejects_event_artifact_binding_mismatch() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        rewrite_capture_event_object_artifact_hash(repo.path(), &capture.revision_id, "sha256:bad");

        let error = show_revision(RevisionShowOptions::new(repo.path()))
            .expect_err("event/artifact mismatch should fail");

        assert!(error.to_string().contains("object artifact content hash"));
    }

    #[test]
    fn show_revision_emits_snapshot_rows_in_captured_order() {
        let repo = multi_file_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.rows[0].kind.as_str(), "file_header");
        assert_eq!(
            result.rows[0].projection_phase.as_str(),
            "snapshot_remainder"
        );
        assert_eq!(result.rows[0].coverage.as_str(), "unreviewed");
        assert_eq!(result.rows[0].projection_order, 0);
        assert_eq!(
            result.rows[0].snapshot_order.as_ref().unwrap().file_index,
            0
        );
        assert!(result.rows.iter().any(|row| row.kind.as_str() == "diff"));
    }

    #[test]
    fn show_revision_emits_empty_state_row_for_empty_snapshot() {
        let (rows, summary) = build_snapshot_rows(
            &DiffSnapshot::new(
                ReviewId::new("review:empty"),
                ObjectId::new("snap:empty"),
                Vec::new(),
            ),
            &RevisionId::new("review-unit:empty"),
        );

        assert_eq!(summary.file_count, 0);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind.as_str(), "empty_state");
    }

    #[test]
    fn show_revision_rows_do_not_expose_storage_paths() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();
        let debug = format!("{result:?}");

        assert!(!debug.contains("artifacts/objects"));
        assert!(!debug.contains(".shore/data/events"));
    }

    #[test]
    fn show_revision_includes_active_observations() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Check this")
                .with_body("Observation body"),
        )
        .unwrap();

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.observations.len(), 1);
        assert_eq!(result.observations[0].title, "Check this");
        assert_eq!(result.observations[0].body, None);
        assert_eq!(result.summary.observation_count, 1);
        assert!(
            result
                .rows
                .iter()
                .any(|row| row.kind.as_str() == "observation")
        );
    }

    #[test]
    fn show_revision_hydrates_observation_bodies_when_requested() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Body")
                .with_body("Observation body"),
        )
        .unwrap();

        let result =
            show_revision(RevisionShowOptions::new(repo.path()).with_include_body(true)).unwrap();

        assert_eq!(
            result.observations[0].body.as_deref(),
            Some("Observation body")
        );
        assert!(!format!("{result:?}").contains("artifacts/notes/"));
    }

    #[test]
    fn show_revision_observations_match_list_semantics_for_duplicates_and_supersession() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        add_duplicate_observations_with_distinct_idempotency_keys(&repo);
        add_superseding_observation(&repo);

        let unit = show_revision(RevisionShowOptions::new(repo.path())).unwrap();
        let list = list_observations(ObservationListOptions::new(repo.path())).unwrap();

        assert_eq!(unit.observations, list.observations);
        assert_eq!(unit.diagnostics, list.diagnostics);
    }

    #[test]
    fn show_revision_includes_open_and_responded_input_requests() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let request = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Need decision")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired),
        )
        .unwrap();
        respond_input_request(
            InputRequestRespondOptions::new(repo.path(), request.input_request_id.clone())
                .with_outcome(InputRequestResponseOutcome::Approved)
                .with_reason("ok"),
        )
        .unwrap();

        let unit = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert_eq!(unit.input_requests.len(), 1);
        assert_eq!(unit.input_requests[0].id, request.input_request_id);
        assert_eq!(unit.input_requests[0].status, InputRequestStatus::Responded);
        assert_eq!(unit.summary.input_request_count, 1);
        assert!(
            unit.rows
                .iter()
                .any(|row| row.kind.as_str() == "input_request")
        );
    }

    #[test]
    fn show_revision_input_requests_match_list_semantics() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        add_duplicate_input_requests(&repo);
        add_ambiguous_input_request_responses(&repo);

        let unit = show_revision(RevisionShowOptions::new(repo.path())).unwrap();
        let list = list_input_requests(
            InputRequestListOptions::new(repo.path()).with_status(InputRequestStatusFilter::All),
        )
        .unwrap();

        assert_eq!(unit.input_requests, list.input_requests);
        assert_eq!(unit.diagnostics, list.diagnostics);
    }

    #[test]
    fn show_revision_includes_current_assessment() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let assessment = record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_assessment(ReviewAssessment::Accepted)
                .with_summary("ship it"),
        )
        .unwrap();

        let unit = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert_eq!(
            unit.current_assessment.status,
            CurrentAssessmentStatus::Resolved(ReviewAssessment::Accepted)
        );
        assert_eq!(unit.assessments.len(), 1);
        assert_eq!(unit.assessments[0].id, assessment.assessment_id);
        assert_eq!(unit.summary.assessment_count, 1);
        assert!(
            unit.rows
                .iter()
                .any(|row| row.kind.as_str() == "assessment")
        );
    }

    #[test]
    fn show_revision_assessments_match_show_semantics() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        add_replaced_and_duplicate_assessments(&repo);

        let unit =
            show_revision(RevisionShowOptions::new(repo.path()).with_include_body(true)).unwrap();
        let show = show_assessments(
            AssessmentShowOptions::new(repo.path())
                .with_include_summary(true)
                .with_all(true),
        )
        .unwrap();

        assert_eq!(unit.current_assessment, show.current);
        assert_eq!(unit.assessments, show.assessments);
        assert_eq!(unit.diagnostics, show.diagnostics);
    }

    #[test]
    fn show_revision_includes_imported_adapter_notes() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let notes_path = repo.write_fixture("review-notes.json", native_review_notes_json());
        import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(notes_path)).unwrap();

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.adapter_notes.len(), 1);
        assert_eq!(result.adapter_notes[0].title, "Imported note");
        assert_eq!(result.summary.adapter_note_count, 1);
        assert!(
            result
                .rows
                .iter()
                .any(|row| row.kind.as_str() == "adapter_note")
        );
    }

    #[test]
    fn show_revision_adapter_notes_hydrate_body_only_when_requested() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        import_large_review_note_body(&repo);

        let compact = show_revision(RevisionShowOptions::new(repo.path())).unwrap();
        let hydrated =
            show_revision(RevisionShowOptions::new(repo.path()).with_include_body(true)).unwrap();

        assert_eq!(compact.adapter_notes[0].body, None);
        assert_eq!(
            hydrated.adapter_notes[0].body.as_deref(),
            Some("large imported body")
        );
        assert!(!format!("{hydrated:?}").contains("artifacts/notes/"));
    }

    #[test]
    fn show_revision_adapter_notes_surface_stale_and_orphan_status() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        import_stale_and_orphan_review_notes(&repo);

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert!(
            result
                .adapter_notes
                .iter()
                .any(|note| note.status.as_str() == "stale")
        );
        assert!(
            result
                .adapter_notes
                .iter()
                .any(|note| note.status.as_str() == "orphaned")
        );
    }

    #[test]
    fn adapter_note_status_preserves_resolution_detail() {
        use super::adapter_notes::adapter_note_status;
        use crate::model::ResolutionStatus;
        assert_eq!(
            adapter_note_status(&ResolutionStatus::Exact).as_str(),
            "exact"
        );
        assert_eq!(
            adapter_note_status(&ResolutionStatus::Relocated).as_str(),
            "relocated"
        );
        assert_eq!(
            adapter_note_status(&ResolutionStatus::FileLevel).as_str(),
            "file_level"
        );
        assert_eq!(
            adapter_note_status(&ResolutionStatus::Stale).as_str(),
            "stale"
        );
        assert_eq!(
            adapter_note_status(&ResolutionStatus::Orphaned).as_str(),
            "orphaned"
        );
        assert_eq!(
            adapter_note_status(&ResolutionStatus::Unresolved).as_str(),
            "unresolved"
        );
    }

    #[test]
    fn show_revision_places_reviewed_material_before_snapshot_remainder() {
        let repo = multi_hunk_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Important")
                .with_target(ObservationTargetSelector::file("src/lib.rs")),
        )
        .unwrap();

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        let first_snapshot_remainder = result
            .rows
            .iter()
            .position(|row| row.projection_phase.as_str() == "snapshot_remainder")
            .unwrap();
        let observation_row = result
            .rows
            .iter()
            .position(|row| row.kind.as_str() == "observation")
            .unwrap();

        assert!(observation_row < first_snapshot_remainder);
        assert_eq!(result.summary.narrative_row_count, first_snapshot_remainder);
        assert!(result.summary.snapshot_remainder_row_count > 0);
    }

    #[test]
    fn show_revision_keeps_unreviewed_snapshot_rows_complete() {
        let repo = multi_file_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Review wide"),
        )
        .unwrap();

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        let snapshot_row_count = result
            .rows
            .iter()
            .filter(|row| row.snapshot_order.is_some())
            .count();
        assert_eq!(snapshot_row_count, result.summary.snapshot_row_count);
        assert!(
            result
                .rows
                .iter()
                .any(|row| row.coverage.as_str() == "unreviewed")
        );
    }

    #[test]
    fn show_revision_track_filter_narrows_narrative_without_mutating_snapshot_remainder() {
        let repo = multi_file_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        add_observation(&repo, "agent:codex", "Codex");
        add_observation(&repo, "agent:claude", "Claude");

        let all = show_revision(RevisionShowOptions::new(repo.path())).unwrap();
        let codex =
            show_revision(RevisionShowOptions::new(repo.path()).with_track("agent:codex")).unwrap();

        assert!(all.summary.narrative_row_count > codex.summary.narrative_row_count);
        assert_eq!(
            all.summary.snapshot_remainder_row_count,
            codex.summary.snapshot_remainder_row_count
        );
        assert!(
            codex
                .observations
                .iter()
                .all(|obs| obs.track_id.as_str() == "agent:codex")
        );
    }

    #[test]
    fn show_revision_surfaces_floating_then_anchored() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let floating = show_revision(RevisionShowOptions::new(repo.path())).unwrap();
        assert!(!floating.commit_range.anchored);
        assert!(floating.commit_range.current_commits.is_empty());

        record_commit_association(repo.path(), &capture, "oidA");

        let anchored = show_revision(RevisionShowOptions::new(repo.path())).unwrap();
        assert!(anchored.commit_range.anchored);
        assert_eq!(anchored.commit_range.current_commits.len(), 1);
        assert_eq!(anchored.commit_range.current_commits[0].commit_oid, "oidA");
    }

    #[test]
    fn show_revision_extends_diagnostics_with_commit_range_diagnostics() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_commit_association(repo.path(), &capture, "oidA");
        record_commit_association(repo.path(), &capture, "oidB");

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "divergent_commit_association")
        );
    }

    fn record_commit_association(repo: &Path, capture: &CaptureResult, commit_oid: &str) {
        use crate::session::event::{RevisionCommitAssociatedPayload, build_commit_association_id};
        let commit_association_id =
            build_commit_association_id(&capture.revision_id, commit_oid).unwrap();
        let mut target = EventTarget::for_revision(
            crate::model::JournalId::new("journal:default"),
            capture.revision_id.clone(),
            None,
        );
        target.track_id = Some(crate::model::TrackId::new("agent:codex"));
        let event = ShoreEvent::new(
            EventType::RevisionCommitAssociated,
            RevisionCommitAssociatedPayload::idempotency_key(&capture.revision_id, commit_oid),
            target,
            Writer::shore_local("0.1.0"),
            RevisionCommitAssociatedPayload {
                commit_association_id,
                target: crate::model::ReviewTargetRef::Revision {
                    revision_id: capture.revision_id.clone(),
                },
                commit: crate::model::ReviewEndpoint::GitCommit {
                    commit_oid: commit_oid.to_owned(),
                    tree_oid: format!("{commit_oid}-tree"),
                },
            },
            "2026-05-10T00:00:00Z",
        )
        .unwrap();

        EventStore::open(resolved_store_dir(repo))
            .record_event_once(&event)
            .unwrap();
    }

    fn modified_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        repo
    }

    fn record_validation_event(repo: &Path, capture: &CaptureResult, validation_check_id: &str) {
        let mut target = EventTarget::for_revision(
            crate::model::JournalId::new("journal:default"),
            capture.revision_id.clone(),
            None,
        );
        target.track_id = Some(crate::model::TrackId::new("agent:codex"));
        let event = ShoreEvent::new(
            EventType::ValidationCheckRecorded,
            format!("validation_check_recorded:{validation_check_id}"),
            target,
            Writer::shore_local("0.1.0"),
            ValidationCheckRecordedPayload {
                validation_check_id: ValidationCheckId::new(validation_check_id),
                target: ValidationTarget::Revision {
                    revision_id: capture.revision_id.clone(),
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
                started_at: None,
                completed_at: Some("2026-05-10T00:00:00Z".to_owned()),
                log_artifact_content_hashes: Vec::new(),
            },
            "2026-05-10T00:00:00Z",
        )
        .unwrap();

        EventStore::open(resolved_store_dir(repo))
            .record_event_once(&event)
            .unwrap();
    }

    fn multi_file_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.write("src/other.rs", "pub fn other() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        repo.write("src/other.rs", "pub fn other() -> u32 { 2 }\n");
        repo
    }

    fn multi_hunk_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write(
            "src/lib.rs",
            (1..=30)
                .map(|line| format!("pub fn value_{line}() -> u32 {{ {line} }}\n"))
                .collect::<String>(),
        );
        repo.commit_all("base");
        repo.write(
            "src/lib.rs",
            (1..=30)
                .map(|line| {
                    let value = if line == 2 || line == 28 {
                        line + 100
                    } else {
                        line
                    };
                    format!("pub fn value_{line}() -> u32 {{ {value} }}\n")
                })
                .collect::<String>(),
        );
        repo
    }

    fn add_observation(repo: &TestRepo, track: &str, title: &str) {
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track(track)
                .with_title(title),
        )
        .unwrap();
    }

    fn add_duplicate_observations_with_distinct_idempotency_keys(repo: &TestRepo) {
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

        assert_eq!(first.observation_id, second.observation_id);
    }

    fn add_superseding_observation(repo: &TestRepo) {
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
                .superseding(original.observation_id),
        )
        .unwrap();
    }

    fn add_duplicate_input_requests(repo: &TestRepo) {
        let first = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Same decision")
                .with_body("same body")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired)
                .with_idempotency_key("input-request-retry-a"),
        )
        .unwrap();
        let second = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Same decision")
                .with_body("same body")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired)
                .with_idempotency_key("input-request-retry-b"),
        )
        .unwrap();

        assert_eq!(first.input_request_id, second.input_request_id);
    }

    fn add_ambiguous_input_request_responses(repo: &TestRepo) {
        let request = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Ambiguous")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired),
        )
        .unwrap();
        respond_input_request(
            InputRequestRespondOptions::new(repo.path(), request.input_request_id.clone())
                .with_outcome(InputRequestResponseOutcome::Approved),
        )
        .unwrap();
        respond_input_request(
            InputRequestRespondOptions::new(repo.path(), request.input_request_id)
                .with_outcome(InputRequestResponseOutcome::Rejected),
        )
        .unwrap();
    }

    fn add_replaced_and_duplicate_assessments(repo: &TestRepo) {
        let duplicate_options = AssessmentAddOptions::new(repo.path())
            .with_track("human:kevin")
            .with_assessment(ReviewAssessment::NeedsClarification)
            .with_summary("same summary");
        let first = record_assessment(
            duplicate_options
                .clone()
                .with_idempotency_key("assessment-retry-a"),
        )
        .unwrap();
        let second =
            record_assessment(duplicate_options.with_idempotency_key("assessment-retry-b"))
                .unwrap();

        assert_eq!(first.assessment_id, second.assessment_id);

        record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_assessment(ReviewAssessment::AcceptedWithFollowUp)
                .with_summary("replacement")
                .replacing(first.assessment_id),
        )
        .unwrap();
    }

    fn import_large_review_note_body(repo: &TestRepo) {
        let path = repo.write_fixture(
            "large-review-notes.json",
            review_notes_json_with_notes(
                "src/lib.rs",
                vec![review_note_json(
                    "large",
                    "Large imported note",
                    "large imported body",
                    "new",
                    1,
                    1,
                )],
            ),
        );
        import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(path)).unwrap();
    }

    fn import_stale_and_orphan_review_notes(repo: &TestRepo) {
        let path = repo.write_fixture(
            "stale-orphan-review-notes.json",
            format!(
                r#"{{
  "schema": "shore.review-notes",
  "version": 1,
  "files": [
    {{
      "path": "src/lib.rs",
      "notes": [
        {}
      ]
    }},
    {{
      "path": "src/gone.rs",
      "notes": [
        {}
      ]
    }}
  ]
}}"#,
                review_note_json("stale", "Stale imported note", "stale", "new", 99, 99),
                review_note_json("orphan", "Orphan imported note", "orphan", "new", 1, 1)
            ),
        );
        import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(path)).unwrap();
    }

    fn native_review_notes_json() -> String {
        review_notes_json_with_notes(
            "src/lib.rs",
            vec![review_note_json(
                "imported",
                "Imported note",
                "Imported body",
                "new",
                1,
                1,
            )],
        )
    }

    fn review_notes_json_with_notes(path: &str, notes: Vec<String>) -> String {
        format!(
            r#"{{
  "schema": "shore.review-notes",
  "version": 1,
  "files": [
    {{
      "path": "{path}",
      "notes": [
        {}
      ]
    }}
  ]
}}"#,
            notes.join(",\n        ")
        )
    }

    fn review_note_json(
        id: &str,
        title: &str,
        body: &str,
        side: &str,
        start_line: u32,
        end_line: u32,
    ) -> String {
        format!(
            r#"{{
          "id": "{id}",
          "title": "{title}",
          "body": "{body}",
          "target": {{
            "side": "{side}",
            "startLine": {start_line},
            "endLine": {end_line}
          }},
          "tags": ["fixture"],
          "confidence": "high",
          "source": "review-notes.json",
          "author": "codex",
          "createdAt": "2026-05-13T00:00:00Z"
        }}"#
        )
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

        fn write(&self, path: impl AsRef<Path>, contents: impl AsRef<[u8]>) {
            let path = self.root.path().join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directories");
            }
            fs::write(path, contents).expect("write test repository file");
        }

        fn write_fixture(&self, path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> PathBuf {
            let path = self.root.path().join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directories");
            }
            fs::write(&path, contents).expect("write test fixture");
            path
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

    /// The store a workflow actually lands in for `repo` — the shared common-dir
    /// store by default. Reads that follow a workflow resolve here, not the raw
    /// worktree-local `.shore/data`.
    fn resolved_store_dir(repo: &Path) -> std::path::PathBuf {
        crate::git::git_common_dir(repo).unwrap().join("shore")
    }

    fn tamper_object_artifact_snapshot_field(repo: &Path, object_id: &ObjectId) {
        let path = object_artifact_path(repo, object_id);
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("read object artifact"))
                .expect("parse object artifact json");

        assert_eq!(json["snapshot"]["object_id"], object_id.as_str());
        // Perturb a field inside the v2 content hash without re-stamping it.
        // `DiffFile` is snake_case, unlike the camelCase artifact wrapper.
        json["snapshot"]["files"][0]["new_path"] = "/evil".into();

        fs::write(
            &path,
            serde_json::to_vec_pretty(&json).expect("serialize tampered object artifact"),
        )
        .expect("write tampered object artifact");
    }

    fn rewrite_capture_event_object_artifact_hash(
        repo: &Path,
        revision_id: &RevisionId,
        hash: &str,
    ) {
        let path = capture_event_path(repo, revision_id);
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("read capture event"))
                .expect("parse capture event json");

        json["payload"]["workObject"]["objectArtifactContentHash"] = hash.into();
        json["payloadHash"] = sha256_json_prefixed(&json["payload"])
            .expect("hash rewritten capture event payload")
            .into();

        fs::write(
            &path,
            serde_json::to_vec_pretty(&json).expect("serialize rewritten capture event"),
        )
        .expect("write rewritten capture event");
    }

    fn record_artifact_removed(repo: &Path, content_hash: &str) {
        let event = ShoreEvent::new(
            EventType::ArtifactRemoved,
            ArtifactRemovedPayload::idempotency_key(content_hash),
            EventTarget::for_journal(crate::model::JournalId::new("journal:default")),
            Writer::shore_local("0.1.0"),
            ArtifactRemovedPayload {
                content_hash: content_hash.to_owned(),
            },
            "2026-05-10T00:00:00Z",
        )
        .unwrap();
        EventStore::open(resolved_store_dir(repo))
            .record_event_once(&event)
            .unwrap();
    }

    /// Record an `ArtifactRemoved` that arrived through a foreign-event seam
    /// (`ingest = Some`): it has no local-possession arm, so the default policy
    /// reads it as a non-operative claim.
    fn record_ingested_artifact_removed(repo: &Path, content_hash: &str) {
        let mut event = ShoreEvent::new(
            EventType::ArtifactRemoved,
            ArtifactRemovedPayload::idempotency_key(content_hash),
            EventTarget::for_journal(crate::model::JournalId::new("journal:default")),
            Writer::shore_local("0.1.0"),
            ArtifactRemovedPayload {
                content_hash: content_hash.to_owned(),
            },
            "2026-05-10T00:00:00Z",
        )
        .unwrap();
        event.ingest = Some(IngestProvenance {
            via: IngestVia::IngestEvents,
            received_at: "2026-05-10T01:00:00Z".to_owned(),
        });
        EventStore::open(resolved_store_dir(repo))
            .record_event_once(&event)
            .unwrap();
    }

    /// Record an `ArtifactRemoved` carrying an inline signature, optionally
    /// ingested and optionally tampered. `validate_event` does not verify
    /// signatures, so the read-time reader-relative check classifies it: a valid
    /// signature under the empty default trust reads `UntrustedKey`, a tampered
    /// one reads `Invalid`.
    fn record_signed_artifact_removed(repo: &Path, content_hash: &str, ingest: bool, tamper: bool) {
        let signer = DeterministicSigner::from_seed([91u8; 32]);
        let mut event = ShoreEvent::new(
            EventType::ArtifactRemoved,
            ArtifactRemovedPayload::idempotency_key(content_hash),
            EventTarget::for_journal(crate::model::JournalId::new("journal:default")),
            Writer::shore_local("0.1.0"),
            ArtifactRemovedPayload {
                content_hash: content_hash.to_owned(),
            },
            "2026-05-10T00:00:00Z",
        )
        .unwrap();
        let tbs = EventToBeSigned::from_event(&event, signer.signer_id()).unwrap();
        let pae = event_signature_pre_authentication_encoding(&tbs).unwrap();
        let sig = signer.sign_event_message(&pae).unwrap();
        event.signer = Some(signer.signer_id().clone());
        event.signature = Some(if tamper {
            EventSignature::ed25519_v1(EventSignatureBytes::from_bytes(&[0u8; 64]))
        } else {
            EventSignature::ed25519_v1(sig)
        });
        if ingest {
            event.ingest = Some(IngestProvenance {
                via: IngestVia::IngestEvents,
                received_at: "2026-05-10T01:00:00Z".to_owned(),
            });
        }
        EventStore::open(resolved_store_dir(repo))
            .record_event_once(&event)
            .unwrap();
    }

    /// Fabricate a second, distinct-revision capture binding the SAME content
    /// hash as `capture` (the content/object identity-reuse case).
    fn fabricate_distinct_sibling_capture(repo: &Path, capture: &CaptureResult) {
        let sibling_unit = RevisionId::new(format!("{}-reuse", capture.revision_id.as_str()));
        let sibling_object = ObjectId::new(format!("{}-reuse", capture.object_id.as_str()));
        let event = ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("work_object_proposed:{}", sibling_unit.as_str()),
            EventTarget::for_revision(capture.journal_id.clone(), sibling_unit.clone(), None),
            Writer::shore_local("0.1.0"),
            WorkObjectProposedPayload {
                engagement_id: EngagementId::new("engagement:sha256:reuse"),
                work_object: WorkObjectProposal::Revision {
                    revision: Revision {
                        id: sibling_unit,
                        object_id: sibling_object,
                        git_provenance: None,
                    },
                    object_artifact_content_hash: capture.object_artifact_content_hash.clone(),
                    supersedes: vec![],
                },
            },
            "2026-05-10T00:00:00Z",
        )
        .unwrap();
        EventStore::open(resolved_store_dir(repo))
            .record_event_once(&event)
            .unwrap();
    }

    fn delete_snapshot_blob(repo: &Path, object_id: &ObjectId) {
        let path = object_artifact_path(repo, object_id);
        fs::remove_file(path).expect("delete snapshot blob");
    }

    #[test]
    fn ingested_unsigned_removal_emits_removal_claim_unsigned() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_ingested_artifact_removed(repo.path(), &capture.object_artifact_content_hash);

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "removal_claim_unsigned")
        );
        // The non-operative claim renders the bytes rather than suppressing them.
        assert!(!result.snapshot.files.is_empty());
    }

    #[test]
    fn ingested_untrusted_removal_emits_removal_claim_untrusted() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_signed_artifact_removed(
            repo.path(),
            &capture.object_artifact_content_hash,
            true,
            false,
        );

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "removal_claim_untrusted")
        );
    }

    #[test]
    fn invalid_signature_removal_emits_removal_claim_invalid() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        // Locally authored (possessed) but the inline signature is tampered: the
        // integrity floor classifies it invalid even with possession.
        record_signed_artifact_removed(
            repo.path(),
            &capture.object_artifact_content_hash,
            false,
            true,
        );

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "removal_claim_invalid")
        );
    }

    #[test]
    fn removal_of_unreferenced_hash_emits_removed_target_missing() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        // A possessed removal over a hash no event references.
        record_artifact_removed(repo.path(), "sha256:ghost-unreferenced");

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert!(result.diagnostics.iter().any(|d| {
            d.code == "snapshot_content_removed_target_missing"
                && d.message.contains("sha256:ghost-unreferenced")
        }));
    }

    #[test]
    fn second_capture_over_removed_hash_emits_identity_reused_after_removal() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        // A distinct capture re-binds the same content hash, then the hash is
        // removed (possessed → operative).
        fabricate_distinct_sibling_capture(repo.path(), &capture);
        record_artifact_removed(repo.path(), &capture.object_artifact_content_hash);

        let result = show_revision(
            RevisionShowOptions::new(repo.path()).with_revision_id(capture.revision_id.clone()),
        )
        .unwrap();

        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "identity_reused_after_removal")
        );
    }

    #[test]
    fn ingested_unsigned_removal_renders_present_under_default_policy() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_ingested_artifact_removed(repo.path(), &capture.object_artifact_content_hash);

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        // An ingested, unverified removal is non-operative under the default
        // policy: the snapshot renders rather than being suppressed.
        assert_eq!(result.snapshot_content_state, SnapshotContentState::Present);
        assert!(!result.snapshot_is_removed());
        assert!(!result.snapshot.files.is_empty());
    }

    #[test]
    fn possessed_unsigned_removal_still_suppresses_under_default() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        // Locally authored (ingest = None): the zero-key local possession floor.
        record_artifact_removed(repo.path(), &capture.object_artifact_content_hash);

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert_eq!(
            result.snapshot_content_state,
            SnapshotContentState::SuppressedPresent
        );
        assert!(result.snapshot_is_removed());
    }

    #[test]
    fn possessed_removal_renders_present_under_trusted_strict() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_artifact_removed(repo.path(), &capture.object_artifact_content_hash);

        let result = show_revision(
            RevisionShowOptions::new(repo.path()).with_removal_policy(RemovalPolicy::TrustedStrict),
        )
        .unwrap();

        // TrustedStrict drops the possession arm; the possessed unsigned removal
        // no longer suppresses.
        assert_eq!(result.snapshot_content_state, SnapshotContentState::Present);
        assert!(!result.snapshot_is_removed());
    }

    #[test]
    fn removed_and_swept_snapshot_renders_physically_removed_not_a_hard_error() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_artifact_removed(repo.path(), &capture.object_artifact_content_hash);
        delete_snapshot_blob(repo.path(), &capture.object_id);

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert!(result.snapshot_is_removed());
        assert_eq!(
            result.removed_snapshot_content_hash.as_deref(),
            Some(capture.object_artifact_content_hash.as_str())
        );
        assert!(result.snapshot.files.is_empty());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "snapshot_content_physically_removed"),
            "expected a physically-removed diagnostic, got {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn result_carries_snapshot_content_state() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        // A present capture resolves to Present and is not removed.
        let present = show_revision(RevisionShowOptions::new(repo.path())).unwrap();
        assert_eq!(
            present.snapshot_content_state,
            SnapshotContentState::Present
        );
        assert!(!present.snapshot_is_removed());

        // A removal with the blob still on disk resolves to SuppressedPresent.
        record_artifact_removed(repo.path(), &capture.object_artifact_content_hash);
        let suppressed = show_revision(RevisionShowOptions::new(repo.path())).unwrap();
        assert_eq!(
            suppressed.snapshot_content_state,
            SnapshotContentState::SuppressedPresent
        );
        assert!(suppressed.snapshot_is_removed());

        // Once the blob is swept, it resolves to PhysicallyRemoved.
        delete_snapshot_blob(repo.path(), &capture.object_id);
        let removed = show_revision(RevisionShowOptions::new(repo.path())).unwrap();
        assert_eq!(
            removed.snapshot_content_state,
            SnapshotContentState::PhysicallyRemoved
        );
        assert!(removed.snapshot_is_removed());
    }

    #[test]
    fn suppressed_present_diagnostic_does_not_claim_bytes_are_gone() {
        // A removal is recorded but the blob is NOT swept (no compact): the
        // diagnostic must report suppression without claiming the bytes are gone.
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_artifact_removed(repo.path(), &capture.object_artifact_content_hash);

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert!(result.snapshot_is_removed());
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "snapshot_content_suppressed_present")
            .expect("expected a suppressed-present diagnostic");
        assert!(
            !diagnostic.message.contains("no longer stored"),
            "the suppressed-present message must not claim the bytes are gone: {}",
            diagnostic.message
        );
    }

    #[test]
    fn truly_missing_unremoved_snapshot_still_errors() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        // Delete the blob WITHOUT a removal fact: not-yet-synced, not removed.
        delete_snapshot_blob(repo.path(), &capture.object_id);

        let err = show_revision(RevisionShowOptions::new(repo.path())).unwrap_err();
        assert!(
            err.to_string().contains("import referenced artifacts"),
            "expected the hard missing-artifact error, got: {err}"
        );
    }

    fn object_artifact_path(repo: &Path, object_id: &ObjectId) -> PathBuf {
        fs::read_dir(resolved_store_dir(repo).join("artifacts/objects"))
            .expect("read object artifacts directory")
            .map(|entry| entry.expect("read object artifact dir entry").path())
            .find(|path| {
                let Ok(bytes) = fs::read(path) else {
                    return false;
                };
                let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                    return false;
                };
                json["snapshot"]["object_id"] == object_id.as_str()
            })
            .expect("find object artifact")
    }

    fn capture_event_path(repo: &Path, revision_id: &RevisionId) -> PathBuf {
        fs::read_dir(resolved_store_dir(repo).join("events"))
            .expect("read events directory")
            .map(|entry| entry.expect("read event dir entry").path())
            .find(|path| {
                let Ok(bytes) = fs::read(path) else {
                    return false;
                };
                let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                    return false;
                };
                json["eventType"] == "work_object_proposed"
                    && json["payload"]["workObject"]["revision"]["id"] == revision_id.as_str()
            })
            .expect("find capture event")
    }
}
