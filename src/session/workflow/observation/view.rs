use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::target::ResolvedRevision;
use crate::error::{Result, ShoreError};
use crate::model::{EventId, ObservationId, ReviewTargetRef, TrackId};
use crate::session::event::{
    BodyContentType, EventType, ReviewObservationRecordedPayload, ShoreEvent, Writer,
};
use crate::session::projection::body_content::{
    BodyContentState, BodyRemovalLens, resolve_body_content,
};
use crate::session::store::backend::StoreBackend;

struct ObservationEventRecord<'a> {
    event: &'a ShoreEvent,
    payload: ReviewObservationRecordedPayload,
    track_id: TrackId,
}

pub(crate) struct ObservationProjectionOptions<'a> {
    pub backend: &'a StoreBackend,
    pub events: &'a [ShoreEvent],
    pub resolved: &'a ResolvedRevision,
    pub track_filter: Option<TrackId>,
    pub file_filter: Option<&'a str>,
    pub tag_filters: &'a [String],
    pub include_body: bool,
    /// The reader's removal lens: an operative removal over an externalized
    /// body renders an explained removed state instead of the bytes.
    pub removal_lens: &'a BodyRemovalLens<'a>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationView {
    pub id: ObservationId,
    pub event_id: EventId,
    pub track_id: TrackId,
    pub target: ReviewTargetRef,
    pub title: String,
    pub body: Option<String>,
    pub body_content_type: BodyContentType,
    pub tags: Vec<String>,
    pub confidence: Option<String>,
    pub status: ObservationStatus,
    pub supersedes: Vec<ObservationId>,
    pub responds_to: Vec<ObservationId>,
    pub responded_by: Vec<ObservationId>,
    pub body_content_hash: Option<String>,
    pub body_content_state: BodyContentState,
    pub created_at: String,
    pub writer: Writer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationStatus {
    Active,
    Superseded,
}

pub(crate) fn project_observations(
    options: ObservationProjectionOptions<'_>,
) -> Result<Vec<ObservationView>> {
    let mut observation_records: BTreeMap<ObservationId, ObservationEventRecord<'_>> =
        BTreeMap::new();
    let mut superseded_ids = BTreeSet::new();
    let mut responded_by: BTreeMap<ObservationId, BTreeSet<ObservationId>> = BTreeMap::new();

    for event in options
        .events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewObservationRecorded)
    {
        if event.subject_revision_id()?.as_ref() != Some(&options.resolved.revision_id) {
            continue;
        }

        let payload: ReviewObservationRecordedPayload =
            serde_json::from_value(event.payload.clone())?;
        superseded_ids.extend(payload.supersedes_observation_ids.iter().cloned());
        // Collect response edges here, before the track filter below, so a reviewer-track ack of an
        // author-track observation still surfaces on the author-track view (the reverse-map is
        // cross-track within the revision).
        for target in &payload.responds_to_observation_ids {
            responded_by
                .entry(target.clone())
                .or_default()
                .insert(payload.observation_id.clone());
        }

        let track_id =
            event.target.track_id.clone().ok_or_else(|| {
                ShoreError::Message("observation event missing track id".to_owned())
            })?;
        if options
            .track_filter
            .as_ref()
            .is_some_and(|filter| filter != &track_id)
        {
            continue;
        }
        if options
            .file_filter
            .is_some_and(|file| !target_matches_file(&payload.target, file))
        {
            continue;
        }
        if !options
            .tag_filters
            .iter()
            .all(|tag| payload.tags.iter().any(|candidate| candidate == tag))
        {
            continue;
        }

        let observation_id = payload.observation_id.clone();
        let replace_record = observation_records
            .get(&observation_id)
            .is_none_or(|record| {
                // Event IDs are deterministic storage addresses, not causal order. Pick the
                // lowest one only as a stable representative for duplicate semantic facts.
                event.event_id.as_str() < record.event.event_id.as_str()
            });
        if replace_record {
            observation_records.insert(
                observation_id,
                ObservationEventRecord {
                    event,
                    payload,
                    track_id,
                },
            );
        }
    }

    let mut observations = Vec::new();
    for (_, record) in observation_records {
        let content = resolve_body_content(
            options.backend,
            options.removal_lens,
            options.include_body,
            record.payload.body.clone(),
            record.payload.body_artifact_path.as_deref(),
        )?;
        let body_content_state = content.state();
        let body = content.into_text();

        observations.push(ObservationView {
            id: record.payload.observation_id,
            event_id: record.event.event_id.clone(),
            track_id: record.track_id,
            target: record.payload.target,
            title: record.payload.title,
            body,
            body_content_type: record.payload.body_content_type,
            tags: record.payload.tags,
            confidence: record.payload.confidence,
            status: ObservationStatus::Active,
            supersedes: record.payload.supersedes_observation_ids,
            responds_to: record.payload.responds_to_observation_ids,
            responded_by: Vec::new(),
            body_content_hash: record.payload.body_content_hash,
            body_content_state,
            created_at: record.event.occurred_at.clone(),
            writer: record.event.writer.clone(),
        });
    }

    for observation in &mut observations {
        if superseded_ids.contains(&observation.id) {
            observation.status = ObservationStatus::Superseded;
        }
        observation.responded_by = responded_by
            .get(&observation.id)
            .map(|responders| responders.iter().cloned().collect())
            .unwrap_or_default();
    }
    sort_observation_views(&mut observations);
    Ok(observations)
}

pub(crate) fn target_matches_file(target: &ReviewTargetRef, file: &str) -> bool {
    match target {
        ReviewTargetRef::File { file_path, .. } | ReviewTargetRef::Range { file_path, .. } => {
            file_path == file
        }
        ReviewTargetRef::Revision { .. }
        | ReviewTargetRef::Observation { .. }
        | ReviewTargetRef::InputRequest { .. }
        | ReviewTargetRef::Assessment { .. }
        | ReviewTargetRef::Event { .. } => false,
    }
}

pub(super) fn sort_observation_views(observations: &mut [ObservationView]) {
    observations.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.event_id.as_str().cmp(right.event_id.as_str()))
    });
}
