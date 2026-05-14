use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::freshness::event_set_hash_for_events;
use crate::error::{Result, ShoreError};
use crate::model::{
    DispositionId, EventId, InterventionId, InterventionResolutionId, ObservationId, ReviewId,
    ReviewUnitId, RevisionId, SnapshotId, WorkUnitId,
};
use crate::session::event::{
    EventType, InterventionMode, InterventionRequestedPayload, InterventionResolvedPayload,
    ReviewDispositionRecordedPayload, ReviewObservationRecordedPayload, ReviewUnitCapturedPayload,
    ShoreEvent,
};

const STATE_SCHEMA: &str = "shore.state";
const STATE_VERSION: u32 = 1;
pub const AMBIGUOUS_CURRENT_REVIEW_UNIT_CODE: &str = "ambiguous_current_review_unit";
pub const DUPLICATE_SEMANTIC_OBSERVATION_EVENT_CODE: &str = "duplicate_semantic_observation_event";
pub const DUPLICATE_SEMANTIC_INTERVENTION_REQUEST_EVENT_CODE: &str =
    "duplicate_semantic_intervention_request_event";
pub const DUPLICATE_SEMANTIC_INTERVENTION_RESOLUTION_EVENT_CODE: &str =
    "duplicate_semantic_intervention_resolution_event";
pub const DUPLICATE_SEMANTIC_DISPOSITION_EVENT_CODE: &str = "duplicate_semantic_disposition_event";
const DUPLICATE_SEMANTIC_DIAGNOSTIC_EVENT_LIMIT: usize = 5;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    pub schema: String,
    pub version: u32,
    pub review_id: ReviewId,
    pub work_unit_id: WorkUnitId,
    pub current_revision_id: Option<RevisionId>,
    pub current_snapshot_id: Option<SnapshotId>,
    #[serde(default)]
    pub review_unit_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_review_unit_id: Option<ReviewUnitId>,
    pub event_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_set_hash: Option<String>,
    pub note_count: usize,
    #[serde(default)]
    pub observation_count: usize,
    #[serde(default)]
    pub disposition_count: usize,
    #[serde(default)]
    pub intervention_count: usize,
    #[serde(default)]
    pub open_intervention_count: usize,
    #[serde(default)]
    pub open_blocking_intervention_count: usize,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

impl SessionState {
    pub fn from_events(events: &[ShoreEvent]) -> Result<Self> {
        let event_set_hash = event_set_hash_for_events(events)?;
        let mut reducer = StateReducer::default();
        for event in events {
            reducer.apply(event)?;
        }
        reducer.finish(events.len(), event_set_hash)
    }

    pub fn validate_schema_version(&self) -> Result<()> {
        if self.schema == STATE_SCHEMA && self.version == STATE_VERSION {
            return Ok(());
        }

        Err(ShoreError::UnsupportedStateSchemaVersion {
            schema: self.schema.clone(),
            version: self.version,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionDiagnostic {
    pub code: String,
    pub message: String,
}

#[derive(Debug)]
struct StateReducer {
    review_id: ReviewId,
    work_unit_id: WorkUnitId,
    captured_review_units: BTreeMap<ReviewUnitId, (RevisionId, SnapshotId)>,
    note_count: usize,
    observation_events: BTreeMap<ObservationId, BTreeSet<EventId>>,
    disposition_events: BTreeMap<DispositionId, BTreeSet<EventId>>,
    intervention_modes: BTreeMap<InterventionId, InterventionMode>,
    intervention_request_events: BTreeMap<InterventionId, BTreeSet<EventId>>,
    intervention_resolution_events: BTreeMap<InterventionResolutionId, BTreeSet<EventId>>,
    resolved_intervention_ids: BTreeSet<InterventionId>,
}

impl Default for StateReducer {
    fn default() -> Self {
        Self {
            review_id: ReviewId::new("review:default"),
            work_unit_id: WorkUnitId::new("work:default"),
            captured_review_units: BTreeMap::new(),
            note_count: 0,
            observation_events: BTreeMap::new(),
            disposition_events: BTreeMap::new(),
            intervention_modes: BTreeMap::new(),
            intervention_request_events: BTreeMap::new(),
            intervention_resolution_events: BTreeMap::new(),
            resolved_intervention_ids: BTreeSet::new(),
        }
    }
}

impl StateReducer {
    fn apply(&mut self, event: &ShoreEvent) -> Result<()> {
        event.validate_schema_version()?;

        if event.event_type == EventType::ReviewInitialized {
            self.review_id = event.target.review_id.clone();
            if let Some(work_unit_id) = &event.target.work_unit_id {
                self.work_unit_id = work_unit_id.clone();
            }
            return Ok(());
        }

        self.set_identity_from_event_if_default(event);

        match event.event_type {
            EventType::ReviewInitialized => {}
            EventType::ReviewUnitCaptured => self.apply_review_unit_captured(event)?,
            EventType::ReviewObservationRecorded => self.apply_observation_recorded(event)?,
            EventType::ReviewDispositionRecorded => self.apply_disposition_recorded(event)?,
            EventType::InterventionRequested => self.apply_intervention_requested(event)?,
            EventType::InterventionResolved => self.apply_intervention_resolved(event)?,
            EventType::ReviewNoteImported => {
                self.note_count += 1;
            }
        }

        Ok(())
    }

    fn set_identity_from_event_if_default(&mut self, event: &ShoreEvent) {
        if self.review_id.as_str() == "review:default" {
            self.review_id = event.target.review_id.clone();
        }
        if self.work_unit_id.as_str() == "work:default"
            && let Some(work_unit_id) = &event.target.work_unit_id
        {
            self.work_unit_id = work_unit_id.clone();
        }
    }

    fn apply_review_unit_captured(&mut self, event: &ShoreEvent) -> Result<()> {
        let payload: ReviewUnitCapturedPayload = serde_json::from_value(event.payload.clone())?;
        self.captured_review_units.insert(
            payload.review_unit_id,
            (payload.revision_id, payload.snapshot_id),
        );
        Ok(())
    }

    fn apply_observation_recorded(&mut self, event: &ShoreEvent) -> Result<()> {
        let payload: ReviewObservationRecordedPayload =
            serde_json::from_value(event.payload.clone())?;
        self.observation_events
            .entry(payload.observation_id)
            .or_default()
            .insert(event.event_id.clone());
        Ok(())
    }

    fn apply_disposition_recorded(&mut self, event: &ShoreEvent) -> Result<()> {
        let payload: ReviewDispositionRecordedPayload =
            serde_json::from_value(event.payload.clone())?;
        self.disposition_events
            .entry(payload.disposition_id)
            .or_default()
            .insert(event.event_id.clone());
        Ok(())
    }

    fn apply_intervention_requested(&mut self, event: &ShoreEvent) -> Result<()> {
        let payload: InterventionRequestedPayload = serde_json::from_value(event.payload.clone())?;
        let intervention_id = payload.intervention_id;
        self.intervention_request_events
            .entry(intervention_id.clone())
            .or_default()
            .insert(event.event_id.clone());
        self.intervention_modes
            .entry(intervention_id)
            .or_insert(payload.mode);
        Ok(())
    }

    fn apply_intervention_resolved(&mut self, event: &ShoreEvent) -> Result<()> {
        let payload: InterventionResolvedPayload = serde_json::from_value(event.payload.clone())?;
        self.intervention_resolution_events
            .entry(payload.intervention_resolution_id)
            .or_default()
            .insert(event.event_id.clone());
        self.resolved_intervention_ids
            .insert(payload.intervention_id);
        Ok(())
    }

    fn finish(self, event_count: usize, event_set_hash: String) -> Result<SessionState> {
        let mut diagnostics = Vec::new();
        let current_review_unit = match self.captured_review_units.len() {
            0 => None,
            1 => self.captured_review_units.iter().next(),
            _ => {
                diagnostics.push(ProjectionDiagnostic {
                    code: AMBIGUOUS_CURRENT_REVIEW_UNIT_CODE.to_owned(),
                    message: "multiple captured review units remain current".to_owned(),
                });
                None
            }
        };
        let current_review_unit_id =
            current_review_unit.map(|(review_unit_id, _)| review_unit_id.clone());
        let current_revision_id =
            current_review_unit.map(|(_, (revision_id, _))| revision_id.clone());
        let current_snapshot_id =
            current_review_unit.map(|(_, (_, snapshot_id))| snapshot_id.clone());
        let open_intervention_count = self
            .intervention_modes
            .keys()
            .filter(|intervention_id| !self.resolved_intervention_ids.contains(*intervention_id))
            .count();
        let open_blocking_intervention_count = self
            .intervention_modes
            .iter()
            .filter(|(intervention_id, mode)| {
                **mode == InterventionMode::Blocking
                    && !self.resolved_intervention_ids.contains(*intervention_id)
            })
            .count();

        append_duplicate_semantic_diagnostics(
            &mut diagnostics,
            DUPLICATE_SEMANTIC_OBSERVATION_EVENT_CODE,
            "observation",
            self.observation_events
                .iter()
                .map(|(observation_id, event_ids)| (observation_id.as_str(), event_ids)),
        );
        append_duplicate_semantic_diagnostics(
            &mut diagnostics,
            DUPLICATE_SEMANTIC_INTERVENTION_REQUEST_EVENT_CODE,
            "intervention request",
            self.intervention_request_events
                .iter()
                .map(|(intervention_id, event_ids)| (intervention_id.as_str(), event_ids)),
        );
        append_duplicate_semantic_diagnostics(
            &mut diagnostics,
            DUPLICATE_SEMANTIC_INTERVENTION_RESOLUTION_EVENT_CODE,
            "intervention resolution",
            self.intervention_resolution_events
                .iter()
                .map(|(resolution_id, event_ids)| (resolution_id.as_str(), event_ids)),
        );
        append_duplicate_semantic_diagnostics(
            &mut diagnostics,
            DUPLICATE_SEMANTIC_DISPOSITION_EVENT_CODE,
            "disposition",
            self.disposition_events
                .iter()
                .map(|(disposition_id, event_ids)| (disposition_id.as_str(), event_ids)),
        );

        Ok(SessionState {
            schema: STATE_SCHEMA.to_owned(),
            version: STATE_VERSION,
            review_id: self.review_id,
            work_unit_id: self.work_unit_id,
            current_revision_id,
            current_snapshot_id,
            review_unit_count: self.captured_review_units.len(),
            current_review_unit_id,
            event_count,
            event_set_hash: Some(event_set_hash),
            note_count: self.note_count,
            observation_count: self.observation_events.len(),
            disposition_count: self.disposition_events.len(),
            intervention_count: self.intervention_modes.len(),
            open_intervention_count,
            open_blocking_intervention_count,
            diagnostics,
        })
    }
}

fn append_duplicate_semantic_diagnostics<'a>(
    diagnostics: &mut Vec<ProjectionDiagnostic>,
    code: &str,
    label: &str,
    events_by_id: impl Iterator<Item = (&'a str, &'a BTreeSet<EventId>)>,
) {
    for (semantic_id, event_ids) in events_by_id {
        if event_ids.len() < 2 {
            continue;
        }
        let mut event_id_list = event_ids
            .iter()
            .take(DUPLICATE_SEMANTIC_DIAGNOSTIC_EVENT_LIMIT)
            .map(|event_id| event_id.as_str())
            .collect::<Vec<_>>();
        if event_ids.len() > DUPLICATE_SEMANTIC_DIAGNOSTIC_EVENT_LIMIT {
            event_id_list.push("...");
        }
        diagnostics.push(ProjectionDiagnostic {
            code: code.to_owned(),
            message: format!(
                "duplicate {label} semantic id {semantic_id} appears in events: {}",
                event_id_list.join(", ")
            ),
        });
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::model::{ReviewEndpoint, ReviewTargetRef, ReviewUnitSource, WorktreeCaptureMode};
    use crate::session::event::{EventTarget, ReviewObservationRecordedPayload, Writer};

    #[test]
    fn projection_defaults_without_events() {
        let state = SessionState::from_events(&[]).unwrap();

        assert_eq!(state.schema, "shore.state");
        assert_eq!(state.version, 1);
        assert_eq!(state.current_review_unit_id, None);
        assert_eq!(state.current_revision_id, None);
        assert_eq!(state.current_snapshot_id, None);
        assert_eq!(state.event_count, 0);
    }

    #[test]
    fn projection_defaults_include_event_set_hash() {
        let state = SessionState::from_events(&[]).unwrap();

        assert_eq!(state.event_count, 0);
        assert!(
            state
                .event_set_hash
                .as_deref()
                .is_some_and(|hash| hash.starts_with("sha256:"))
        );
    }

    #[test]
    fn projection_event_set_hash_is_order_independent() {
        let first = review_unit_captured_event("review-unit:sha256:one", "rev:one", "snap:one");
        let second = observation_event("retry-a", "obs:sha256:one");

        let forward = SessionState::from_events(&[first.clone(), second.clone()]).unwrap();
        let reversed = SessionState::from_events(&[second, first]).unwrap();

        assert_eq!(forward.event_set_hash, reversed.event_set_hash);
        assert_eq!(forward.event_count, 2);
        assert_eq!(reversed.event_count, 2);
    }

    #[test]
    fn state_json_includes_event_set_hash_but_not_raw_events() {
        let state = SessionState::from_events(&[review_unit_captured_event(
            "review-unit:sha256:one",
            "rev:one",
            "snap:one",
        )])
        .unwrap();

        let json = serde_json::to_value(&state).unwrap();

        assert!(
            json["eventSetHash"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        );
        assert!(json.get("events").is_none());
    }

    #[test]
    fn projection_tracks_current_review_unit_from_capture() {
        let event = review_unit_captured_event("review-unit:sha256:one", "rev:one", "snap:one");

        let state = SessionState::from_events(&[event]).unwrap();

        assert_eq!(
            state.current_review_unit_id.as_ref().unwrap().as_str(),
            "review-unit:sha256:one"
        );
        assert_eq!(
            state.current_revision_id.as_ref().unwrap().as_str(),
            "rev:one"
        );
        assert_eq!(
            state.current_snapshot_id.as_ref().unwrap().as_str(),
            "snap:one"
        );
        assert_eq!(state.review_unit_count, 1);
    }

    #[test]
    fn projection_reports_ambiguous_current_review_unit() {
        let events = vec![
            review_unit_captured_event("review-unit:sha256:one", "rev:one", "snap:one"),
            review_unit_captured_event("review-unit:sha256:two", "rev:two", "snap:two"),
        ];

        let state = SessionState::from_events(&events).unwrap();

        assert_eq!(state.current_review_unit_id, None);
        assert_eq!(state.current_revision_id, None);
        assert_eq!(state.current_snapshot_id, None);
        assert!(state.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == AMBIGUOUS_CURRENT_REVIEW_UNIT_CODE
                && diagnostic
                    .message
                    .contains("multiple captured review units")
        }));
    }

    #[test]
    fn duplicate_semantic_observation_events_are_counted_once_with_diagnostic() {
        let events = vec![
            observation_event("retry-a", "obs:sha256:same"),
            observation_event("retry-b", "obs:sha256:same"),
        ];

        let state = SessionState::from_events(&events).unwrap();

        assert_eq!(state.observation_count, 1);
        assert!(state.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == DUPLICATE_SEMANTIC_OBSERVATION_EVENT_CODE
                && diagnostic.message.contains("obs:sha256:same")
        }));
    }

    #[test]
    fn state_json_no_longer_contains_legacy_verdict_fields() {
        let events = vec![review_unit_captured_event(
            "review-unit:sha256:one",
            "rev:one",
            "snap:one",
        )];

        let state = SessionState::from_events(&events).unwrap();
        let json = serde_json::to_value(&state).unwrap();

        assert!(json.get("reviewArtifactCount").is_none());
        assert!(json.get("acknowledgementCount").is_none());
        assert!(json.get("lastVerdictDecision").is_none());
        assert!(json.get("sidecarCount").is_none());
    }

    #[test]
    fn state_deserializes_missing_additive_ledger_fields_as_defaults() {
        let json = json!({
            "schema": "shore.state",
            "version": 1,
            "reviewId": "review:default",
            "workUnitId": "work:default",
            "currentRevisionId": null,
            "currentSnapshotId": null,
            "eventCount": 0,
            "noteCount": 0,
            "diagnostics": []
        });

        let state: SessionState = serde_json::from_value(json).unwrap();

        assert_eq!(state.review_unit_count, 0);
        assert_eq!(state.event_set_hash, None);
        assert_eq!(state.observation_count, 0);
        assert_eq!(state.disposition_count, 0);
        assert_eq!(state.intervention_count, 0);
    }

    fn review_unit_captured_event(
        review_unit_id: &str,
        revision_id: &str,
        snapshot_id: &str,
    ) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewUnitCaptured,
            format!("review_unit_captured:{review_unit_id}"),
            EventTarget::for_review_unit(
                ReviewId::new("review:default"),
                ReviewUnitId::new(review_unit_id),
                RevisionId::new(revision_id),
                SnapshotId::new(snapshot_id),
            ),
            Writer::shore_local_author("0.1.0"),
            ReviewUnitCapturedPayload {
                review_unit_id: ReviewUnitId::new(review_unit_id),
                source: ReviewUnitSource::GitWorktree {
                    mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                    include_untracked: true,
                },
                base: ReviewEndpoint::GitCommit {
                    commit_oid: "base".to_owned(),
                    tree_oid: "base-tree".to_owned(),
                },
                target: ReviewEndpoint::GitWorkingTree {
                    worktree_root: "/tmp/repo".to_owned(),
                },
                revision_id: RevisionId::new(revision_id),
                snapshot_id: SnapshotId::new(snapshot_id),
                snapshot_artifact_content_hash: "sha256:artifact".to_owned(),
            },
            "2026-05-10T00:00:00Z",
        )
        .unwrap()
    }

    fn observation_event(source_key: &str, observation_id: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewObservationRecorded,
            format!("review_observation_recorded:{source_key}"),
            EventTarget::for_review_unit(
                ReviewId::new("review:default"),
                ReviewUnitId::new("review-unit:sha256:one"),
                RevisionId::new("rev:one"),
                SnapshotId::new("snap:one"),
            ),
            Writer::shore_local_reviewer("0.1.0"),
            ReviewObservationRecordedPayload {
                observation_id: ObservationId::new(observation_id),
                target: ReviewTargetRef::ReviewUnit {
                    review_unit_id: ReviewUnitId::new("review-unit:sha256:one"),
                },
                title: "Observation".to_owned(),
                body: None,
                body_artifact_path: None,
                body_byte_size: None,
                body_content_hash: None,
                tags: Vec::new(),
                confidence: None,
                supersedes_observation_ids: Vec::new(),
            },
            "2026-05-10T00:00:00Z",
        )
        .unwrap()
    }
}
