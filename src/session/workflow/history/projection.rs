use std::path::Path;

use super::options::ResolvedHistoryFilters;
use super::result::ReviewHistoryResult;
use super::summary::{ReviewHistoryEntry, ReviewHistorySummary};
use crate::error::{Result, ShoreError};
use crate::model::TargetRef;
use crate::session::body_artifact::load_body_artifact;
use crate::session::event::{
    EventType, InputRequestRespondedPayload, ReviewAssessmentRecordedPayload,
    ReviewInitializedPayload, ReviewNoteImportedPayload, ReviewObservationRecordedPayload,
    ReviewUnitCapturedPayload, ReviewUnitLineageDeclaredPayload,
    ReviewUnitLineageRoundRecordedPayload, ShoreEvent, ValidationCheckRecordedPayload,
    decode_input_request_opened_payload,
};
use crate::session::projection::cosignature::{
    CosignatureIndex, endorsement_readbacks, enrich_endorser_attributes,
};
use crate::session::state::SessionState;
use crate::session::{principal_view_for, verify_event_signature};

pub(super) fn history_from_events(
    events: &[ShoreEvent],
    filters: ResolvedHistoryFilters,
    store_dir: Option<&Path>,
) -> Result<ReviewHistoryResult> {
    let state = SessionState::from_events(events)?;
    let event_set_hash = state
        .event_set_hash
        .clone()
        .expect("SessionState::from_events sets event_set_hash");
    // Build the co-signature index once per document, only when a policy is set —
    // the zero-policy path stays free of cost and output.
    let cosig_index = filters
        .verification_policy
        .is_some()
        .then(|| CosignatureIndex::build(events))
        .transpose()?;
    let mut entries = events
        .iter()
        .filter(|event| event_matches_filters(event, &filters))
        .map(|event| history_entry_from_event(event, &filters, cosig_index.as_ref(), store_dir))
        .collect::<Result<Vec<_>>>()?;

    entries.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then_with(|| left.event_id.as_str().cmp(right.event_id.as_str()))
    });

    Ok(ReviewHistoryResult {
        event_set_hash,
        event_count: events.len(),
        filters: filters.into(),
        entries,
        diagnostics: state.diagnostics,
    })
}

pub(super) fn history_entry_from_event(
    event: &ShoreEvent,
    filters: &ResolvedHistoryFilters,
    cosig_index: Option<&CosignatureIndex<'_>>,
    store_dir: Option<&Path>,
) -> Result<ReviewHistoryEntry> {
    let summary = match event.event_type {
        EventType::ReviewInitialized => {
            let _payload: ReviewInitializedPayload = serde_json::from_value(event.payload.clone())?;
            ReviewHistorySummary::ReviewInitialized {}
        }
        EventType::ReviewUnitCaptured => {
            let payload: ReviewUnitCapturedPayload = serde_json::from_value(event.payload.clone())?;
            ReviewHistorySummary::ReviewUnitCaptured {
                review_unit_id: payload.review_unit_id,
                source: payload.source,
                base: payload.base,
                target: payload.target,
                revision_id: payload.revision_id,
                snapshot_id: payload.snapshot_id,
                snapshot_artifact_content_hash: payload.snapshot_artifact_content_hash,
            }
        }
        EventType::ReviewObservationRecorded => {
            let payload: ReviewObservationRecordedPayload =
                serde_json::from_value(event.payload.clone())?;
            ReviewHistorySummary::ReviewObservationRecorded {
                observation_id: payload.observation_id,
                target: payload.target,
                title: payload.title,
                body: optional_text(
                    store_dir,
                    filters.include_body,
                    payload.body,
                    payload.body_artifact_path.as_deref(),
                )?,
                body_byte_size: payload.body_byte_size,
                body_content_hash: payload.body_content_hash,
                tags: payload.tags,
                confidence: payload.confidence,
                supersedes: payload.supersedes_observation_ids,
            }
        }
        EventType::ReviewAssessmentRecorded => {
            let payload: ReviewAssessmentRecordedPayload =
                serde_json::from_value(event.payload.clone())?;
            ReviewHistorySummary::ReviewAssessmentRecorded {
                assessment_id: payload.assessment_id,
                target: payload.target,
                assessment: payload.assessment,
                summary: optional_text(
                    store_dir,
                    filters.include_body,
                    payload.summary,
                    payload.summary_artifact_path.as_deref(),
                )?,
                summary_byte_size: payload.summary_byte_size,
                summary_content_hash: payload.summary_content_hash,
                replaces: payload.replaces_assessment_ids,
                related_observations: payload.related_observation_ids,
                related_input_requests: payload.related_input_request_ids,
            }
        }
        EventType::InputRequestOpened => {
            let payload = decode_input_request_opened_payload(event.payload.clone())?;
            ReviewHistorySummary::InputRequestOpened {
                input_request_id: payload.input_request_id,
                target: payload.target,
                mode: event.assertion_mode,
                reason_code: payload.reason_code,
                title: payload.title,
                body: optional_text(
                    store_dir,
                    filters.include_body,
                    payload.body,
                    payload.body_artifact_path.as_deref(),
                )?,
                body_byte_size: payload.body_byte_size,
                body_content_hash: payload.body_content_hash,
            }
        }
        EventType::InputRequestResponded => {
            let payload: InputRequestRespondedPayload =
                serde_json::from_value(event.payload.clone())?;
            ReviewHistorySummary::InputRequestResponded {
                input_request_response_id: payload.input_request_response_id,
                input_request_id: payload.input_request_id,
                outcome: payload.outcome,
                reason: optional_text(
                    store_dir,
                    filters.include_body,
                    payload.reason,
                    payload.reason_artifact_path.as_deref(),
                )?,
                reason_byte_size: payload.reason_byte_size,
                reason_content_hash: payload.reason_content_hash,
            }
        }
        EventType::ReviewNoteImported => {
            let payload: ReviewNoteImportedPayload = serde_json::from_value(event.payload.clone())?;
            ReviewHistorySummary::ReviewNoteImported {
                sidecar_source: payload.sidecar_source,
                note_id: payload.note_id,
                file_path: payload.file_path,
                file_old_path: payload.file_old_path,
                target: payload.target,
                title: payload.title,
                body: optional_text(
                    store_dir,
                    filters.include_body,
                    payload.body,
                    payload.body_artifact_path.as_deref(),
                )?,
                body_byte_size: payload.body_byte_size.map(|size| size as u64),
                tags: payload.tags,
                confidence: payload.confidence,
                external_source: payload.external_source,
                author: payload.author,
                created_at: payload.created_at,
                sidecar_content_hash: payload.sidecar_content_hash,
            }
        }
        EventType::ReviewUnitLineageDeclared => {
            let payload: ReviewUnitLineageDeclaredPayload =
                serde_json::from_value(event.payload.clone())?;
            ReviewHistorySummary::ReviewUnitLineageDeclared {
                lineage_id: payload.lineage_id,
                basis: payload.basis,
            }
        }
        EventType::ReviewUnitLineageRoundRecorded => {
            let payload: ReviewUnitLineageRoundRecordedPayload =
                serde_json::from_value(event.payload.clone())?;
            ReviewHistorySummary::ReviewUnitLineageRoundRecorded {
                lineage_id: payload.lineage_id,
                round_id: payload.round_id,
                review_unit_id: payload.review_unit_id,
                predecessor_review_unit_id: payload.predecessor_review_unit_id,
                change_id: payload.change_id,
            }
        }
        EventType::ValidationCheckRecorded => {
            let payload: ValidationCheckRecordedPayload =
                serde_json::from_value(event.payload.clone())?;
            ReviewHistorySummary::ValidationCheckRecorded {
                validation_check_id: payload.validation_check_id,
                target: payload.target,
                check_name: payload.check_name,
                command: payload.command,
                status: payload.status,
                exit_code: payload.exit_code,
                trigger: payload.trigger,
                source_fingerprint: payload.source_fingerprint,
                summary: optional_text(
                    store_dir,
                    filters.include_body,
                    payload.summary,
                    payload.summary_artifact_path.as_deref(),
                )?,
                summary_content_hash: payload.summary_content_hash,
                started_at: payload.started_at,
                completed_at: payload.completed_at,
                log_artifact_content_hashes: payload.log_artifact_content_hashes,
            }
        }
        EventType::TaskAttemptCaptured
        | EventType::TaskCheckpointCaptured
        | EventType::TaskObservationRecorded
        | EventType::EventSignatureRecorded => {
            return Err(ShoreError::Message(
                "review history projects review-domain content events only; a task or co-signature event reached this match arm — upstream filter missing".to_owned(),
            ));
        }
    };

    Ok(ReviewHistoryEntry {
        event_id: event.event_id.clone(),
        event_type: event.event_type,
        occurred_at: event.occurred_at.clone(),
        payload_hash: event.payload_hash.clone(),
        session_id: event.target.session_id.clone(),
        review_unit_id: event.target.review_unit_id.clone(),
        revision_id: event.target.revision_id.clone(),
        snapshot_id: event.target.snapshot_id.clone(),
        track_id: event.target.track_id.clone(),
        subject: match event.target.subject.as_ref() {
            Some(TargetRef::Review(r)) => Some(r.clone()),
            Some(TargetRef::Task(_)) | None => None,
        },
        writer: event.writer.clone(),
        verification_status: filters
            .verification_policy
            .map(|_| verify_event_signature(event, &filters.trust_set))
            .transpose()?,
        endorsements: match cosig_index {
            Some(index) => {
                let mut readbacks = endorsement_readbacks(
                    &index.cosignatures_for_target(event, &filters.trust_set)?,
                );
                enrich_endorser_attributes(&mut readbacks, filters.actor_attributes.as_ref());
                readbacks
            }
            None => Vec::new(),
        },
        principal: principal_view_for(
            &event.writer.actor_id,
            filters.delegation_map.as_ref(),
            &event.occurred_at,
        ),
        summary,
    })
}

fn optional_text(
    store_dir: Option<&Path>,
    include_body: bool,
    inline: Option<String>,
    artifact_path: Option<&str>,
) -> Result<Option<String>> {
    if !include_body {
        return Ok(None);
    }
    if inline.is_some() {
        return Ok(inline);
    }
    match artifact_path {
        Some(path) => {
            let store_dir = store_dir.ok_or_else(|| {
                ShoreError::Message(
                    "shore directory is required to hydrate body artifact".to_owned(),
                )
            })?;
            load_body_artifact(store_dir, path)
        }
        None => Ok(None),
    }
}

fn event_matches_filters(event: &ShoreEvent, filters: &ResolvedHistoryFilters) -> bool {
    // Review history is a review-domain content projection by name and contract. Task-domain
    // events have a sibling projection; detached co-signatures are read through the dedicated
    // co-signature-set projection. Neither is summarized in this content stream.
    if matches!(
        event.event_type,
        EventType::TaskAttemptCaptured
            | EventType::TaskCheckpointCaptured
            | EventType::TaskObservationRecorded
            | EventType::EventSignatureRecorded
    ) {
        return false;
    }
    if filters
        .review_unit_id
        .as_ref()
        .is_some_and(|review_unit_id| event.target.review_unit_id.as_ref() != Some(review_unit_id))
    {
        return false;
    }
    if filters
        .track_id
        .as_ref()
        .is_some_and(|track_id| event.target.track_id.as_ref() != Some(track_id))
    {
        return false;
    }
    if !filters.event_types.is_empty() && !filters.event_types.contains(&event.event_type) {
        return false;
    }
    true
}
