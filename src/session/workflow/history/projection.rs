use std::path::Path;

use super::options::ResolvedHistoryFilters;
use super::result::ReviewHistoryResult;
use super::summary::{ReviewHistoryEntry, ReviewHistorySummary};
use crate::error::{Result, ShoreError};
use crate::model::TargetRef;
use crate::session::body_artifact::load_body_artifact;
use crate::session::event::{
    EventType, InterventionRequestedPayload, InterventionResolvedPayload,
    ReviewDispositionRecordedPayload, ReviewInitializedPayload, ReviewNoteImportedPayload,
    ReviewObservationRecordedPayload, ReviewUnitCapturedPayload, ShoreEvent,
};
use crate::session::state::SessionState;

pub(super) fn history_from_events(
    events: &[ShoreEvent],
    filters: ResolvedHistoryFilters,
    shore_dir: Option<&Path>,
) -> Result<ReviewHistoryResult> {
    let state = SessionState::from_events(events)?;
    let event_set_hash = state
        .event_set_hash
        .clone()
        .expect("SessionState::from_events sets event_set_hash");
    let mut entries = events
        .iter()
        .filter(|event| event_matches_filters(event, &filters))
        .map(|event| history_entry_from_event(event, filters.include_body, shore_dir))
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
    include_body: bool,
    shore_dir: Option<&Path>,
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
                    shore_dir,
                    include_body,
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
        EventType::InterventionRequested => {
            let payload: InterventionRequestedPayload =
                serde_json::from_value(event.payload.clone())?;
            ReviewHistorySummary::InterventionRequested {
                intervention_id: payload.intervention_id,
                target: payload.target,
                mode: payload.mode,
                reason_code: payload.reason_code,
                title: payload.title,
                body: optional_text(
                    shore_dir,
                    include_body,
                    payload.body,
                    payload.body_artifact_path.as_deref(),
                )?,
                body_byte_size: payload.body_byte_size,
                body_content_hash: payload.body_content_hash,
            }
        }
        EventType::InterventionResolved => {
            let payload: InterventionResolvedPayload =
                serde_json::from_value(event.payload.clone())?;
            ReviewHistorySummary::InterventionResolved {
                intervention_resolution_id: payload.intervention_resolution_id,
                intervention_id: payload.intervention_id,
                outcome: payload.outcome,
                reason: optional_text(
                    shore_dir,
                    include_body,
                    payload.reason,
                    payload.reason_artifact_path.as_deref(),
                )?,
                reason_byte_size: payload.reason_byte_size,
                reason_content_hash: payload.reason_content_hash,
            }
        }
        EventType::ReviewDispositionRecorded => {
            let payload: ReviewDispositionRecordedPayload =
                serde_json::from_value(event.payload.clone())?;
            ReviewHistorySummary::ReviewDispositionRecorded {
                disposition_id: payload.disposition_id,
                target: payload.target,
                disposition: payload.disposition,
                summary: optional_text(
                    shore_dir,
                    include_body,
                    payload.summary,
                    payload.summary_artifact_path.as_deref(),
                )?,
                summary_byte_size: payload.summary_byte_size,
                summary_content_hash: payload.summary_content_hash,
                replaces: payload.replaces_disposition_ids,
                related_observations: payload.related_observation_ids,
                related_interventions: payload.related_intervention_ids,
                overrides: payload.overrides,
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
                    shore_dir,
                    include_body,
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
        EventType::TaskAttemptCaptured
        | EventType::TaskCheckpointCaptured
        | EventType::TaskObservationRecorded => {
            return Err(ShoreError::Message(
                "review history projects review-domain events only; task event reached this match arm — upstream filter missing".to_owned(),
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
        summary,
    })
}

fn optional_text(
    shore_dir: Option<&Path>,
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
            let shore_dir = shore_dir.ok_or_else(|| {
                ShoreError::Message(
                    "shore directory is required to hydrate body artifact".to_owned(),
                )
            })?;
            load_body_artifact(shore_dir, path)
        }
        None => Ok(None),
    }
}

fn event_matches_filters(event: &ShoreEvent, filters: &ResolvedHistoryFilters) -> bool {
    // Review history is a review-domain projection by name and contract; task-domain events
    // are not summarized here. A sibling task-history projection is a Phase 5 concern.
    if matches!(
        event.event_type,
        EventType::TaskAttemptCaptured
            | EventType::TaskCheckpointCaptured
            | EventType::TaskObservationRecorded
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
