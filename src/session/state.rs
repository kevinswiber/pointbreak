use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::{Result, ShoreError};
use crate::model::{
    DispositionId, EventId, InterventionId, InterventionResolutionId, ObservationId,
    ReviewArtifactId, ReviewId, ReviewUnitId, RevisionId, SnapshotId, WorkUnitId,
};
use crate::session::event::{
    EventType, InterventionMode, InterventionRequestedPayload, InterventionResolvedPayload,
    ReviewArtifactAcknowledgedPayload, ReviewArtifactPublishedPayload,
    ReviewDispositionRecordedPayload, ReviewObservationRecordedPayload, ReviewUnitCapturedPayload,
    RevisionPublishedPayload, ShoreEvent, SnapshotObservedPayload, VerdictDecision,
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
    pub sidecar_count: usize,
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
    #[serde(default)]
    pub review_artifact_count: usize,
    #[serde(default)]
    pub acknowledgement_count: usize,
    #[serde(default)]
    pub last_verdict_decision: Option<VerdictDecision>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

impl SessionState {
    pub fn from_events(events: &[ShoreEvent]) -> Result<Self> {
        let mut reducer = StateReducer::default();
        for event in events {
            reducer.apply(event)?;
        }
        reducer.finish(events.len())
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
    published_revision_ids: BTreeSet<RevisionId>,
    superseded_revision_ids: BTreeSet<RevisionId>,
    snapshots_by_revision_id: BTreeMap<RevisionId, SnapshotId>,
    captured_review_unit_ids: BTreeSet<ReviewUnitId>,
    sidecar_count: usize,
    note_count: usize,
    observation_events: BTreeMap<ObservationId, BTreeSet<EventId>>,
    disposition_events: BTreeMap<DispositionId, BTreeSet<EventId>>,
    intervention_modes: BTreeMap<InterventionId, InterventionMode>,
    intervention_request_events: BTreeMap<InterventionId, BTreeSet<EventId>>,
    intervention_resolution_events: BTreeMap<InterventionResolutionId, BTreeSet<EventId>>,
    resolved_intervention_ids: BTreeSet<InterventionId>,
    review_artifact_count: usize,
    acknowledgement_count: usize,
    published_artifacts: BTreeMap<ReviewArtifactId, (RevisionId, VerdictDecision)>,
    replaced_artifacts: BTreeSet<ReviewArtifactId>,
}

impl Default for StateReducer {
    fn default() -> Self {
        Self {
            review_id: ReviewId::new("review:default"),
            work_unit_id: WorkUnitId::new("work:default"),
            published_revision_ids: BTreeSet::new(),
            superseded_revision_ids: BTreeSet::new(),
            snapshots_by_revision_id: BTreeMap::new(),
            captured_review_unit_ids: BTreeSet::new(),
            sidecar_count: 0,
            note_count: 0,
            observation_events: BTreeMap::new(),
            disposition_events: BTreeMap::new(),
            intervention_modes: BTreeMap::new(),
            intervention_request_events: BTreeMap::new(),
            intervention_resolution_events: BTreeMap::new(),
            resolved_intervention_ids: BTreeSet::new(),
            review_artifact_count: 0,
            acknowledgement_count: 0,
            published_artifacts: BTreeMap::new(),
            replaced_artifacts: BTreeSet::new(),
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
            EventType::RevisionPublished => self.apply_revision_published(event)?,
            EventType::SnapshotObserved => self.apply_snapshot_observed(event)?,
            EventType::SidecarObserved => {
                self.sidecar_count += 1;
            }
            EventType::ReviewNoteImported => {
                self.note_count += 1;
            }
            EventType::ReviewArtifactPublished => {
                self.apply_review_artifact_published(event)?;
            }
            EventType::ReviewArtifactAcknowledged => {
                self.apply_review_artifact_acknowledged(event)?;
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

    fn apply_revision_published(&mut self, event: &ShoreEvent) -> Result<()> {
        let payload: RevisionPublishedPayload = serde_json::from_value(event.payload.clone())?;
        self.published_revision_ids.insert(payload.revision_id);
        for revision_id in payload.supersedes_revision_ids {
            self.superseded_revision_ids.insert(revision_id);
        }
        Ok(())
    }

    fn apply_snapshot_observed(&mut self, event: &ShoreEvent) -> Result<()> {
        let payload: SnapshotObservedPayload = serde_json::from_value(event.payload.clone())?;
        self.snapshots_by_revision_id
            .insert(payload.revision_id, payload.snapshot_id);
        Ok(())
    }

    fn apply_review_unit_captured(&mut self, event: &ShoreEvent) -> Result<()> {
        let payload: ReviewUnitCapturedPayload = serde_json::from_value(event.payload.clone())?;
        self.captured_review_unit_ids.insert(payload.review_unit_id);
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

    fn apply_review_artifact_published(&mut self, event: &ShoreEvent) -> Result<()> {
        let payload: ReviewArtifactPublishedPayload =
            serde_json::from_value(event.payload.clone())?;
        self.review_artifact_count += 1;
        for review_artifact_id in payload.replaces_review_artifact_ids {
            self.replaced_artifacts.insert(review_artifact_id);
        }
        self.published_artifacts.insert(
            payload.review_artifact_id,
            (payload.revision_id, payload.decision),
        );
        Ok(())
    }

    fn apply_review_artifact_acknowledged(&mut self, event: &ShoreEvent) -> Result<()> {
        let _: ReviewArtifactAcknowledgedPayload = serde_json::from_value(event.payload.clone())?;
        self.acknowledgement_count += 1;
        Ok(())
    }

    fn finish(self, event_count: usize) -> Result<SessionState> {
        let mut diagnostics = Vec::new();
        let unsuperseded_revision_ids = self
            .published_revision_ids
            .difference(&self.superseded_revision_ids)
            .cloned()
            .collect::<BTreeSet<_>>();
        let current_revision_id = match unsuperseded_revision_ids.len() {
            0 => None,
            1 => unsuperseded_revision_ids.iter().next().cloned(),
            _ => {
                diagnostics.push(ProjectionDiagnostic {
                    code: "ambiguous_current_revision".to_owned(),
                    message: "multiple unsuperseded revisions remain current".to_owned(),
                });
                None
            }
        };
        let current_snapshot_id = current_revision_id
            .as_ref()
            .and_then(|revision_id| self.snapshots_by_revision_id.get(revision_id))
            .cloned();
        let current_review_unit_id = match self.captured_review_unit_ids.len() {
            0 => None,
            1 => self.captured_review_unit_ids.iter().next().cloned(),
            _ => {
                diagnostics.push(ProjectionDiagnostic {
                    code: AMBIGUOUS_CURRENT_REVIEW_UNIT_CODE.to_owned(),
                    message: "multiple captured review units remain current".to_owned(),
                });
                None
            }
        };
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
        let last_verdict_decision = match current_revision_id.as_ref() {
            Some(revision_id) => {
                let candidate_ids = self
                    .published_artifacts
                    .iter()
                    .filter(|(review_artifact_id, (artifact_revision_id, _))| {
                        artifact_revision_id == revision_id
                            && !self.replaced_artifacts.contains(*review_artifact_id)
                    })
                    .map(|(review_artifact_id, (_, decision))| (review_artifact_id, *decision))
                    .collect::<Vec<_>>();
                match candidate_ids.as_slice() {
                    [] => None,
                    [(_, decision)] => Some(*decision),
                    _ => {
                        diagnostics.push(ProjectionDiagnostic {
                            code: "ambiguous_current_verdict".to_owned(),
                            message: format!(
                                "multiple unsuperseded verdicts remain current for revision {}: {}",
                                revision_id.as_str(),
                                candidate_ids
                                    .iter()
                                    .map(|(review_artifact_id, _)| review_artifact_id.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                        });
                        None
                    }
                }
            }
            None => None,
        };
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
            review_unit_count: self.captured_review_unit_ids.len(),
            current_review_unit_id,
            event_count,
            sidecar_count: self.sidecar_count,
            note_count: self.note_count,
            observation_count: self.observation_events.len(),
            disposition_count: self.disposition_events.len(),
            intervention_count: self.intervention_modes.len(),
            open_intervention_count,
            open_blocking_intervention_count,
            review_artifact_count: self.review_artifact_count,
            acknowledgement_count: self.acknowledgement_count,
            last_verdict_decision,
            diagnostics,
        })
    }
}

fn append_duplicate_semantic_diagnostics<'a>(
    diagnostics: &mut Vec<ProjectionDiagnostic>,
    code: &str,
    label: &str,
    groups: impl Iterator<Item = (&'a str, &'a BTreeSet<EventId>)>,
) {
    for (semantic_id, event_ids) in groups {
        if event_ids.len() < 2 {
            continue;
        }

        diagnostics.push(ProjectionDiagnostic {
            code: code.to_owned(),
            message: format!(
                "duplicate {label} semantic id {semantic_id} appears in {} events: {}",
                event_ids.len(),
                bounded_duplicate_event_list(event_ids),
            ),
        });
    }
}

fn bounded_duplicate_event_list(event_ids: &BTreeSet<EventId>) -> String {
    let mut displayed = event_ids
        .iter()
        .take(DUPLICATE_SEMANTIC_DIAGNOSTIC_EVENT_LIMIT)
        .map(EventId::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    let omitted_count = event_ids
        .len()
        .saturating_sub(DUPLICATE_SEMANTIC_DIAGNOSTIC_EVENT_LIMIT);
    if omitted_count > 0 {
        if !displayed.is_empty() {
            displayed.push_str(", ");
        }
        displayed.push_str(&format!("... {omitted_count} more"));
    }
    displayed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        AcknowledgementId, InterventionId, InterventionResolutionId, ObservationId,
        ReviewArtifactId, ReviewEndpoint, ReviewId, ReviewTargetRef, ReviewUnitId,
        ReviewUnitSource, RevisionId, Side, SnapshotId, TrackId, WorkUnitId, WorktreeCaptureMode,
    };
    use crate::session::event::{
        AcknowledgementNextAction, ImportedNoteTarget, ReviewArtifactAcknowledgedPayload,
        ReviewArtifactPublishedPayload, ReviewDisposition, ReviewDispositionRecordedPayload,
        ReviewNoteImportedPayload, ReviewObservationRecordedPayload, ReviewUnitCapturedPayload,
        VerdictDecision,
    };
    use crate::session::{
        EventTarget, EventType, InterventionMode, InterventionReasonCode,
        InterventionRequestedPayload, InterventionResolutionOutcome, InterventionResolvedPayload,
        ReviewInitializedPayload, RevisionPublishedPayload, ShoreEvent, SidecarObservedPayload,
        SidecarSource, SnapshotObservedPayload, Writer,
    };

    #[test]
    fn projection_tracks_current_revision_snapshot_and_sidecar_count_without_event_history() {
        let events = vec![
            review_initialized("review:default", "work:default"),
            revision_published("rev:worktree:sha256:one", vec![]),
            snapshot_observed("snap:git:sha256:one", "rev:worktree:sha256:one"),
            sidecar_observed("review_notes", "sha256:sidecar"),
        ];

        let projection = SessionState::from_events(&events).expect("projection builds");
        let json = serde_json::to_value(&projection).expect("projection serializes");

        assert_eq!(json["schema"], "shore.state");
        assert_eq!(json["version"], 1);
        assert_eq!(
            projection
                .current_revision_id
                .as_ref()
                .map(RevisionId::as_str),
            Some("rev:worktree:sha256:one")
        );
        assert_eq!(
            projection
                .current_snapshot_id
                .as_ref()
                .map(SnapshotId::as_str),
            Some("snap:git:sha256:one")
        );
        assert_eq!(projection.event_count, 4);
        assert_eq!(projection.sidecar_count, 1);
        assert_eq!(projection.note_count, 0);
        assert!(json.get("events").is_none());
    }

    #[test]
    fn projection_tracks_note_count_without_embedded_note_history() {
        let events = vec![
            review_initialized("review:default", "work:default"),
            review_note_imported("note:abc"),
            review_note_imported("note:def"),
        ];

        let projection = SessionState::from_events(&events).expect("projection builds");
        let json = serde_json::to_value(&projection).expect("projection serializes");

        assert_eq!(projection.note_count, 2);
        assert_eq!(json["noteCount"], 2);
        assert!(json.get("notes").is_none());
    }

    #[test]
    fn projection_uses_explicit_supersession_not_timestamp_ordering() {
        let events = vec![
            revision_published("rev:worktree:sha256:one", vec![]),
            revision_published("rev:worktree:sha256:two", vec!["rev:worktree:sha256:one"]),
        ];

        let projection = SessionState::from_events(&events).expect("projection builds");

        assert_eq!(
            projection
                .current_revision_id
                .as_ref()
                .map(RevisionId::as_str),
            Some("rev:worktree:sha256:two")
        );
    }

    #[test]
    fn projection_reports_ambiguous_current_revision() {
        let events = vec![
            revision_published("rev:worktree:sha256:one", vec![]),
            revision_published("rev:worktree:sha256:two", vec![]),
        ];

        let projection = SessionState::from_events(&events).expect("projection still builds");

        assert_eq!(projection.current_revision_id, None);
        assert!(
            projection
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "ambiguous_current_revision")
        );
    }

    #[test]
    fn state_projects_single_current_review_unit() {
        let events = vec![review_unit_captured_event("review-unit:sha256:one")];

        let state = SessionState::from_events(&events).unwrap();

        assert_eq!(state.review_unit_count, 1);
        assert_eq!(
            state.current_review_unit_id.as_ref().unwrap().as_str(),
            "review-unit:sha256:one"
        );
        assert!(state.diagnostics.is_empty());
    }

    #[test]
    fn state_projects_no_current_review_unit_without_captures() {
        let events = Vec::new();

        let state = SessionState::from_events(&events).unwrap();

        assert_eq!(state.review_unit_count, 0);
        assert!(state.current_review_unit_id.is_none());
        assert!(state.diagnostics.is_empty());
    }

    #[test]
    fn state_reports_ambiguous_current_review_units() {
        let events = vec![
            review_unit_captured_event("review-unit:sha256:one"),
            review_unit_captured_event("review-unit:sha256:two"),
        ];

        let state = SessionState::from_events(&events).unwrap();

        assert_eq!(state.review_unit_count, 2);
        assert!(state.current_review_unit_id.is_none());
        assert!(
            state
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == AMBIGUOUS_CURRENT_REVIEW_UNIT_CODE)
        );
    }

    #[test]
    fn projection_counts_observations_without_embedding_observation_history() {
        let events = vec![
            simple_review_unit_captured_event("review-unit:sha256:one"),
            observation_recorded_event("obs:sha256:one", "agent:codex"),
            observation_recorded_event("obs:sha256:two", "agent:claude"),
        ];

        let state = SessionState::from_events(&events).unwrap();
        let json = serde_json::to_value(&state).unwrap();

        assert_eq!(state.observation_count, 2);
        assert_eq!(json["observationCount"], 2);
        assert!(json.get("observations").is_none());
    }

    #[test]
    fn state_counts_duplicate_observation_semantic_id_once_and_diagnoses() {
        let events = vec![
            simple_review_unit_captured_event("review-unit:sha256:one"),
            observation_recorded_event_with_key("obs:sha256:one", "agent:codex", "retry-a"),
            observation_recorded_event_with_key("obs:sha256:one", "agent:codex", "retry-b"),
        ];

        let state = SessionState::from_events(&events).unwrap();

        assert_eq!(state.observation_count, 1);
        assert_diagnostic(
            &state,
            DUPLICATE_SEMANTIC_OBSERVATION_EVENT_CODE,
            "obs:sha256:one",
        );
    }

    #[test]
    fn duplicate_semantic_diagnostic_messages_are_bounded() {
        let mut events = vec![simple_review_unit_captured_event("review-unit:sha256:one")];
        events.extend((0..7).map(|index| {
            observation_recorded_event_with_key(
                "obs:sha256:bounded",
                "agent:codex",
                &format!("retry-{index}"),
            )
        }));
        let mut event_ids = events
            .iter()
            .skip(1)
            .map(|event| event.event_id.as_str().to_owned())
            .collect::<Vec<_>>();
        event_ids.sort();

        let state = SessionState::from_events(&events).unwrap();
        let diagnostic = state
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == DUPLICATE_SEMANTIC_OBSERVATION_EVENT_CODE)
            .expect("duplicate observation diagnostic emitted");

        assert!(diagnostic.message.contains("obs:sha256:bounded"));
        assert!(diagnostic.message.contains("appears in 7 events"));
        for event_id in event_ids
            .iter()
            .take(DUPLICATE_SEMANTIC_DIAGNOSTIC_EVENT_LIMIT)
        {
            assert!(
                diagnostic.message.contains(event_id),
                "diagnostic should include displayed event id {event_id}: {}",
                diagnostic.message
            );
        }
        for event_id in event_ids
            .iter()
            .skip(DUPLICATE_SEMANTIC_DIAGNOSTIC_EVENT_LIMIT)
        {
            assert!(
                !diagnostic.message.contains(event_id),
                "diagnostic should omit event id beyond display limit {event_id}: {}",
                diagnostic.message
            );
        }
        assert!(diagnostic.message.contains("2 more"));
    }

    #[test]
    fn state_diagnoses_duplicate_intervention_request_semantic_id() {
        let request_a = intervention_requested_event_with_key(
            "intervention:sha256:blocking",
            InterventionMode::Blocking,
            "retry-a",
        );
        let request_b = intervention_requested_event_with_key(
            "intervention:sha256:blocking",
            InterventionMode::Blocking,
            "retry-b",
        );

        let forward = SessionState::from_events(&[request_a.clone(), request_b.clone()]).unwrap();
        let reversed = SessionState::from_events(&[request_b, request_a]).unwrap();

        assert_eq!(forward.intervention_count, 1);
        assert_diagnostic(
            &forward,
            DUPLICATE_SEMANTIC_INTERVENTION_REQUEST_EVENT_CODE,
            "intervention:sha256:blocking",
        );
        assert_eq!(forward.diagnostics, reversed.diagnostics);
    }

    #[test]
    fn state_diagnoses_duplicate_intervention_resolution_semantic_id() {
        let resolution_a = intervention_resolved_event_with_key(
            "intervention:sha256:blocking",
            "intervention-resolution:sha256:approved",
            "retry-a",
        );
        let resolution_b = intervention_resolved_event_with_key(
            "intervention:sha256:blocking",
            "intervention-resolution:sha256:approved",
            "retry-b",
        );

        let forward =
            SessionState::from_events(&[resolution_a.clone(), resolution_b.clone()]).unwrap();
        let reversed = SessionState::from_events(&[resolution_b, resolution_a]).unwrap();

        assert_diagnostic(
            &forward,
            DUPLICATE_SEMANTIC_INTERVENTION_RESOLUTION_EVENT_CODE,
            "intervention-resolution:sha256:approved",
        );
        assert_eq!(forward.diagnostics, reversed.diagnostics);
    }

    #[test]
    fn state_counts_unique_dispositions_without_embedding_history() {
        let events = vec![
            simple_review_unit_captured_event("review-unit:sha256:one"),
            disposition_recorded_event_with_key("disp:sha256:one", "human:kevin", "retry-a"),
            disposition_recorded_event_with_key("disp:sha256:two", "human:kevin", "retry-b"),
        ];

        let state = SessionState::from_events(&events).unwrap();
        let json = serde_json::to_value(&state).unwrap();

        assert_eq!(state.disposition_count, 2);
        assert_eq!(json["dispositionCount"], 2);
        assert!(json.get("dispositions").is_none());
    }

    #[test]
    fn state_counts_duplicate_disposition_semantic_id_once_and_diagnoses() {
        let events = [
            disposition_recorded_event_with_key("disp:sha256:one", "human:kevin", "retry-a"),
            disposition_recorded_event_with_key("disp:sha256:one", "human:kevin", "retry-b"),
        ];

        let forward = SessionState::from_events(&events).unwrap();
        let reversed = SessionState::from_events(&[events[1].clone(), events[0].clone()]).unwrap();

        assert_eq!(forward.disposition_count, 1);
        assert_diagnostic(
            &forward,
            DUPLICATE_SEMANTIC_DISPOSITION_EVENT_CODE,
            "disp:sha256:one",
        );
        assert_eq!(forward.diagnostics, reversed.diagnostics);
    }

    #[test]
    fn review_artifact_published_increments_count_order_independently() {
        let events_a = vec![
            publish_event(),
            verdict_event("review-artifact:sha256:a"),
            verdict_event("review-artifact:sha256:b"),
        ];
        let events_b = vec![
            verdict_event("review-artifact:sha256:b"),
            publish_event(),
            verdict_event("review-artifact:sha256:a"),
        ];

        let state_a = SessionState::from_events(&events_a).expect("state builds");
        let state_b = SessionState::from_events(&events_b).expect("state builds");

        assert_eq!(state_a.review_artifact_count, 2);
        assert_eq!(state_a, state_b);
    }

    #[test]
    fn acknowledgement_count_is_order_independent() {
        let order_one = vec![
            ack_event("ack:sha256:a", "review-artifact:sha256:a"),
            ack_event("ack:sha256:b", "review-artifact:sha256:a"),
        ];
        let order_two = vec![
            ack_event("ack:sha256:b", "review-artifact:sha256:a"),
            ack_event("ack:sha256:a", "review-artifact:sha256:a"),
        ];

        let state_a = SessionState::from_events(&order_one).expect("state builds");
        let state_b = SessionState::from_events(&order_two).expect("state builds");

        assert_eq!(state_a.acknowledgement_count, 2);
        assert_eq!(state_a, state_b);
    }

    #[test]
    fn last_verdict_decision_resolves_when_one_artifact_unreplaced() {
        let events = vec![
            publish_event(),
            verdict_event_with("review-artifact:sha256:v1", VerdictDecision::Pass, vec![]),
            verdict_event_with(
                "review-artifact:sha256:v2",
                VerdictDecision::RequestChanges,
                vec!["review-artifact:sha256:v1"],
            ),
        ];

        let state = SessionState::from_events(&events).expect("state builds");

        assert_eq!(
            state.last_verdict_decision,
            Some(VerdictDecision::RequestChanges)
        );
    }

    #[test]
    fn last_verdict_decision_is_none_and_emits_diagnostic_when_ambiguous() {
        let events = vec![
            publish_event(),
            verdict_event_with("review-artifact:sha256:v1", VerdictDecision::Pass, vec![]),
            verdict_event_with("review-artifact:sha256:v2", VerdictDecision::Pass, vec![]),
        ];

        let state = SessionState::from_events(&events).expect("state builds");

        assert_eq!(state.last_verdict_decision, None);
        assert!(
            state
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "ambiguous_current_verdict")
        );
    }

    #[test]
    fn state_serialization_does_not_embed_event_lists_or_artifact_maps() {
        let events = vec![
            publish_event(),
            verdict_event("review-artifact:sha256:a"),
            ack_event("ack:sha256:a", "review-artifact:sha256:a"),
        ];

        let state = SessionState::from_events(&events).expect("state builds");
        let json = serde_json::to_value(&state).expect("state serializes");

        assert!(json.get("publishedReviewArtifactIds").is_none());
        assert!(json.get("acknowledgedReviewArtifactIds").is_none());
    }

    #[test]
    fn projection_rejects_unsupported_event_schema_version() {
        let mut event = revision_published("rev:worktree:sha256:one", vec![]);
        event.version = 2;

        let error = SessionState::from_events(&[event]).expect_err("unsupported event rejected");

        assert!(
            error
                .to_string()
                .contains("unsupported event schema/version")
        );
    }

    #[test]
    fn projection_has_typed_state_schema_version_validation() {
        let mut projection =
            SessionState::from_events(&[revision_published("rev:worktree:sha256:one", vec![])])
                .expect("projection builds");
        projection.version = 2;

        let error = projection
            .validate_schema_version()
            .expect_err("version 2 is unsupported");

        assert!(matches!(
            error,
            ShoreError::UnsupportedStateSchemaVersion { .. }
        ));
    }

    #[test]
    fn state_deserialization_defaults_new_bounded_verdict_fields() {
        let projection: SessionState = serde_json::from_value(serde_json::json!({
            "schema": "shore.state",
            "version": 1,
            "reviewId": "review:default",
            "workUnitId": "work:default",
            "currentRevisionId": null,
            "currentSnapshotId": null,
            "eventCount": 0,
            "sidecarCount": 0,
            "noteCount": 0,
            "diagnostics": []
        }))
        .expect("projection deserializes");

        assert_eq!(projection.review_artifact_count, 0);
        assert_eq!(projection.acknowledgement_count, 0);
        assert_eq!(projection.last_verdict_decision, None);
    }

    #[test]
    fn state_defaults_missing_observation_count_to_zero() {
        let json = serde_json::json!({
            "schema": "shore.state",
            "version": 1,
            "reviewId": "review:default",
            "workUnitId": "work:default",
            "currentRevisionId": null,
            "currentSnapshotId": null,
            "eventCount": 0,
            "sidecarCount": 0,
            "noteCount": 0,
            "reviewArtifactCount": 0,
            "acknowledgementCount": 0,
            "lastVerdictDecision": null,
            "diagnostics": []
        });

        let state: SessionState = serde_json::from_value(json).unwrap();

        assert_eq!(state.observation_count, 0);
    }

    #[test]
    fn state_defaults_missing_intervention_counts_to_zero() {
        let json = serde_json::json!({
            "schema": "shore.state",
            "version": 1,
            "reviewId": "review:default",
            "workUnitId": "work:default",
            "currentRevisionId": null,
            "currentSnapshotId": null,
            "eventCount": 0,
            "sidecarCount": 0,
            "noteCount": 0,
            "reviewArtifactCount": 0,
            "acknowledgementCount": 0,
            "lastVerdictDecision": null,
            "diagnostics": []
        });

        let state: SessionState = serde_json::from_value(json).unwrap();

        assert_eq!(state.intervention_count, 0);
        assert_eq!(state.open_intervention_count, 0);
        assert_eq!(state.open_blocking_intervention_count, 0);
    }

    #[test]
    fn state_defaults_missing_disposition_count_to_zero() {
        let json = serde_json::json!({
            "schema": "shore.state",
            "version": 1,
            "reviewId": "review:default",
            "workUnitId": "work:default",
            "currentRevisionId": null,
            "currentSnapshotId": null,
            "eventCount": 0,
            "sidecarCount": 0,
            "noteCount": 0,
            "reviewArtifactCount": 0,
            "acknowledgementCount": 0,
            "lastVerdictDecision": null,
            "diagnostics": []
        });

        let state: SessionState = serde_json::from_value(json).unwrap();

        assert_eq!(state.disposition_count, 0);
    }

    #[test]
    fn state_projects_open_intervention_counts_order_independently() {
        let request = intervention_requested_event(
            "intervention:sha256:blocking",
            InterventionMode::Blocking,
        );

        let state = SessionState::from_events(&[request]).unwrap();

        assert_eq!(state.intervention_count, 1);
        assert_eq!(state.open_intervention_count, 1);
        assert_eq!(state.open_blocking_intervention_count, 1);
    }

    #[test]
    fn state_excludes_resolved_interventions_from_open_counts() {
        let request = intervention_requested_event(
            "intervention:sha256:blocking",
            InterventionMode::Blocking,
        );
        let resolution = intervention_resolved_event("intervention:sha256:blocking");

        let forward = SessionState::from_events(&[request.clone(), resolution.clone()]).unwrap();
        let reversed = SessionState::from_events(&[resolution, request]).unwrap();

        assert_eq!(forward.intervention_count, 1);
        assert_eq!(forward.open_intervention_count, 0);
        assert_eq!(forward.open_blocking_intervention_count, 0);
        assert_eq!(forward, reversed);
    }

    fn review_initialized(review_id: &str, work_unit_id: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewInitialized,
            format!("review_initialized:{review_id}:{work_unit_id}"),
            target(review_id, work_unit_id),
            Writer::shore_local_author("0.1.0"),
            ReviewInitializedPayload {},
            "2026-05-09T20:42:45Z",
        )
        .expect("review initialized event builds")
    }

    fn revision_published(revision_id: &str, supersedes: Vec<&str>) -> ShoreEvent {
        ShoreEvent::new(
            EventType::RevisionPublished,
            format!("revision_published:explicit:work:default:{revision_id}"),
            target("review:default", "work:default"),
            Writer::shore_local_author("0.1.0"),
            RevisionPublishedPayload {
                revision_id: RevisionId::new(revision_id),
                supersedes_revision_ids: supersedes.into_iter().map(RevisionId::new).collect(),
            },
            "2026-05-09T20:42:45Z",
        )
        .expect("revision published event builds")
    }

    fn snapshot_observed(snapshot_id: &str, revision_id: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::SnapshotObserved,
            format!("snapshot_observed:work:default:{revision_id}:{snapshot_id}"),
            target("review:default", "work:default"),
            Writer::shore_local_author("0.1.0"),
            SnapshotObservedPayload {
                snapshot_id: SnapshotId::new(snapshot_id),
                revision_id: RevisionId::new(revision_id),
            },
            "2026-05-09T20:42:45Z",
        )
        .expect("snapshot observed event builds")
    }

    fn review_unit_captured_event(review_unit_id: &str) -> ShoreEvent {
        let review_unit_id = ReviewUnitId::new(review_unit_id);
        let revision_id = RevisionId::new(format!("rev:git:sha256:{}", review_unit_id.as_str()));
        let snapshot_id = SnapshotId::new(format!("snap:git:sha256:{}", review_unit_id.as_str()));
        ShoreEvent::new(
            EventType::ReviewUnitCaptured,
            format!("review_unit_captured:{}", review_unit_id.as_str()),
            EventTarget::for_review_unit(
                ReviewId::new("review:default"),
                review_unit_id.clone(),
                revision_id.clone(),
                snapshot_id.clone(),
            ),
            Writer::shore_local_author("0.1.0"),
            ReviewUnitCapturedPayload {
                review_unit_id,
                source: ReviewUnitSource::GitWorktree {
                    mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                    include_untracked: true,
                },
                base: ReviewEndpoint::GitCommit {
                    commit_oid: "abc".to_owned(),
                    tree_oid: "def".to_owned(),
                },
                target: ReviewEndpoint::GitWorkingTree {
                    worktree_root: "/repo".to_owned(),
                },
                revision_id,
                snapshot_id,
                snapshot_artifact_content_hash: "sha256:artifact".to_owned(),
            },
            "2026-05-12T00:00:00Z",
        )
        .expect("review unit captured event builds")
    }

    fn simple_review_unit_captured_event(review_unit_id: &str) -> ShoreEvent {
        review_unit_captured_event(review_unit_id)
    }

    fn observation_recorded_event(observation_id: &str, track_id: &str) -> ShoreEvent {
        observation_recorded_event_with_key(observation_id, track_id, observation_id)
    }

    fn observation_recorded_event_with_key(
        observation_id: &str,
        track_id: &str,
        source_key: &str,
    ) -> ShoreEvent {
        let review_unit_id = ReviewUnitId::new("review-unit:sha256:one");
        let target_ref = ReviewTargetRef::ReviewUnit {
            review_unit_id: review_unit_id.clone(),
        };
        ShoreEvent::new(
            EventType::ReviewObservationRecorded,
            format!(
                "review_observation_recorded:{}:{}:{}",
                review_unit_id.as_str(),
                track_id,
                source_key
            ),
            EventTarget {
                review_id: ReviewId::new("review:default"),
                work_unit_id: None,
                review_unit_id: Some(review_unit_id.clone()),
                revision_id: Some(RevisionId::new("rev:git:sha256:one")),
                snapshot_id: Some(SnapshotId::new("snap:git:sha256:one")),
                track_id: Some(TrackId::new(track_id.to_owned())),
                subject: Some(target_ref.clone()),
            },
            Writer::shore_local_reviewer("test"),
            ReviewObservationRecordedPayload {
                observation_id: ObservationId::new(observation_id.to_owned()),
                target: target_ref,
                title: "Observation".to_owned(),
                body: None,
                body_artifact_path: None,
                body_byte_size: None,
                body_content_hash: None,
                tags: vec![],
                confidence: None,
                supersedes_observation_ids: vec![],
            },
            "2026-05-12T00:00:00Z",
        )
        .unwrap()
    }

    fn disposition_recorded_event_with_key(
        disposition_id: &str,
        track_id: &str,
        source_key: &str,
    ) -> ShoreEvent {
        let review_unit_id = ReviewUnitId::new("review-unit:sha256:one");
        let track_id = TrackId::new(track_id.to_owned());
        let target_ref = ReviewTargetRef::ReviewUnit {
            review_unit_id: review_unit_id.clone(),
        };
        ShoreEvent::new(
            EventType::ReviewDispositionRecorded,
            ReviewDispositionRecordedPayload::idempotency_key(
                &review_unit_id,
                &track_id,
                source_key,
            ),
            EventTarget {
                review_id: ReviewId::new("review:default"),
                work_unit_id: None,
                review_unit_id: Some(review_unit_id.clone()),
                revision_id: Some(RevisionId::new("rev:git:sha256:one")),
                snapshot_id: Some(SnapshotId::new("snap:git:sha256:one")),
                track_id: Some(track_id),
                subject: Some(target_ref.clone()),
            },
            Writer::shore_local_reviewer("test"),
            ReviewDispositionRecordedPayload {
                disposition_id: DispositionId::new(disposition_id.to_owned()),
                target: target_ref,
                disposition: ReviewDisposition::Accepted,
                summary: None,
                summary_artifact_path: None,
                summary_byte_size: None,
                summary_content_hash: None,
                replaces_disposition_ids: vec![],
                related_observation_ids: vec![],
                related_intervention_ids: vec![],
                overrides: vec![],
            },
            "2026-05-12T00:00:00Z",
        )
        .unwrap()
    }

    fn intervention_requested_event(intervention_id: &str, mode: InterventionMode) -> ShoreEvent {
        intervention_requested_event_with_key(intervention_id, mode, intervention_id)
    }

    fn intervention_requested_event_with_key(
        intervention_id: &str,
        mode: InterventionMode,
        source_key: &str,
    ) -> ShoreEvent {
        let review_unit_id = ReviewUnitId::new("review-unit:sha256:one");
        let track_id = TrackId::new("human:kevin");
        let target_ref = ReviewTargetRef::ReviewUnit {
            review_unit_id: review_unit_id.clone(),
        };
        ShoreEvent::new(
            EventType::InterventionRequested,
            InterventionRequestedPayload::idempotency_key(&review_unit_id, &track_id, source_key),
            EventTarget {
                review_id: ReviewId::new("review:default"),
                work_unit_id: None,
                review_unit_id: Some(review_unit_id.clone()),
                revision_id: Some(RevisionId::new("rev:git:sha256:one")),
                snapshot_id: Some(SnapshotId::new("snap:git:sha256:one")),
                track_id: Some(track_id),
                subject: Some(target_ref.clone()),
            },
            Writer::shore_local_reviewer("test"),
            InterventionRequestedPayload {
                intervention_id: InterventionId::new(intervention_id),
                target: target_ref,
                mode,
                reason_code: InterventionReasonCode::ManualDecisionRequired,
                title: "Need approval".to_owned(),
                body: None,
                body_artifact_path: None,
                body_byte_size: None,
                body_content_hash: None,
            },
            "2026-05-12T00:00:00Z",
        )
        .unwrap()
    }

    fn intervention_resolved_event(intervention_id: &str) -> ShoreEvent {
        intervention_resolved_event_with_key(
            intervention_id,
            "intervention-resolution:sha256:approved",
            "intervention-resolution:sha256:approved",
        )
    }

    fn intervention_resolved_event_with_key(
        intervention_id: &str,
        resolution_id: &str,
        source_key: &str,
    ) -> ShoreEvent {
        let review_unit_id = ReviewUnitId::new("review-unit:sha256:one");
        let intervention_id = InterventionId::new(intervention_id);
        let resolution_id = InterventionResolutionId::new(resolution_id);
        ShoreEvent::new(
            EventType::InterventionResolved,
            InterventionResolvedPayload::idempotency_key(&intervention_id, source_key),
            EventTarget {
                review_id: ReviewId::new("review:default"),
                work_unit_id: None,
                review_unit_id: Some(review_unit_id.clone()),
                revision_id: Some(RevisionId::new("rev:git:sha256:one")),
                snapshot_id: Some(SnapshotId::new("snap:git:sha256:one")),
                track_id: None,
                subject: Some(ReviewTargetRef::Intervention {
                    review_unit_id,
                    intervention_id: intervention_id.clone(),
                }),
            },
            Writer::shore_local_reviewer("test"),
            InterventionResolvedPayload {
                intervention_resolution_id: resolution_id,
                intervention_id,
                outcome: InterventionResolutionOutcome::Approved,
                reason: None,
                reason_artifact_path: None,
                reason_byte_size: None,
                reason_content_hash: None,
            },
            "2026-05-12T00:01:00Z",
        )
        .unwrap()
    }

    fn assert_diagnostic(state: &SessionState, code: &str, message_fragment: &str) {
        assert!(
            state.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == code && diagnostic.message.contains(message_fragment)
            }),
            "missing diagnostic {code} containing {message_fragment}; diagnostics: {:?}",
            state.diagnostics
        );
    }

    fn sidecar_observed(source: &str, content_hash: &str) -> ShoreEvent {
        let mut diagnostic_levels = BTreeMap::new();
        diagnostic_levels.insert("warning".to_owned(), 0);

        ShoreEvent::new(
            EventType::SidecarObserved,
            format!("sidecar_observed:{source}:{content_hash}"),
            target("review:default", "work:default"),
            Writer::shore_local_author("0.1.0"),
            SidecarObservedPayload {
                source: match source {
                    "review_notes" => SidecarSource::ReviewNotes,
                    "legacy_hunk_agent_context" => SidecarSource::LegacyHunkAgentContext,
                    other => panic!("unknown sidecar source: {other}"),
                },
                path: "review-notes.json".to_owned(),
                byte_size: 2,
                content_hash: content_hash.to_owned(),
                schema: Some("shore.review-notes".to_owned()),
                imported_schema: None,
                version: Some(1),
                diagnostic_count: 0,
                diagnostic_levels,
            },
            "2026-05-09T20:42:45Z",
        )
        .expect("sidecar observed event builds")
    }

    fn review_note_imported(note_id: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewNoteImported,
            format!("review_note_imported:review_notes:work:default:{note_id}"),
            target("review:default", "work:default"),
            Writer::shore_local_author("0.1.0"),
            ReviewNoteImportedPayload {
                sidecar_source: SidecarSource::ReviewNotes,
                note_id: note_id.to_owned(),
                file_path: "src/lib.rs".to_owned(),
                file_old_path: None,
                target: Some(ImportedNoteTarget {
                    side: Side::New,
                    start_line: 1,
                    end_line: 1,
                }),
                title: "Imported note".to_owned(),
                body: Some("Body".to_owned()),
                body_artifact_path: None,
                body_byte_size: None,
                tags: vec![],
                confidence: None,
                external_source: Some("external".to_owned()),
                author: Some("reviewer".to_owned()),
                created_at: Some("2026-05-10T00:00:00Z".to_owned()),
                sidecar_content_hash: "sha256:sidecar".to_owned(),
            },
            "2026-05-09T20:42:45Z",
        )
        .expect("review note imported event builds")
    }

    fn publish_event() -> ShoreEvent {
        revision_published("rev:worktree:sha256:current", vec![])
    }

    fn verdict_event(review_artifact_id: &str) -> ShoreEvent {
        verdict_event_with(review_artifact_id, VerdictDecision::Pass, vec![])
    }

    fn verdict_event_with(
        review_artifact_id: &str,
        decision: VerdictDecision,
        replaces_review_artifact_ids: Vec<&str>,
    ) -> ShoreEvent {
        let review_artifact_id = ReviewArtifactId::new(review_artifact_id);
        ShoreEvent::new(
            EventType::ReviewArtifactPublished,
            ReviewArtifactPublishedPayload::idempotency_key(
                &WorkUnitId::new("work:default"),
                &review_artifact_id,
            ),
            target("review:default", "work:default"),
            Writer::shore_local_reviewer("0.1.0"),
            ReviewArtifactPublishedPayload {
                review_artifact_id,
                work_unit_id: WorkUnitId::new("work:default"),
                revision_id: RevisionId::new("rev:worktree:sha256:current"),
                decision,
                summary: Some("looks good".to_owned()),
                summary_artifact_path: None,
                summary_byte_size: Some(10),
                replaces_review_artifact_ids: replaces_review_artifact_ids
                    .into_iter()
                    .map(ReviewArtifactId::new)
                    .collect(),
                reviewer: Writer::shore_local_reviewer("0.1.0"),
            },
            "2026-05-10T00:00:00Z",
        )
        .expect("review artifact published event builds")
    }

    fn ack_event(acknowledgement_id: &str, review_artifact_id: &str) -> ShoreEvent {
        let acknowledgement_id = AcknowledgementId::new(acknowledgement_id);
        let review_artifact_id = ReviewArtifactId::new(review_artifact_id);
        ShoreEvent::new(
            EventType::ReviewArtifactAcknowledged,
            ReviewArtifactAcknowledgedPayload::idempotency_key(
                &review_artifact_id,
                &acknowledgement_id,
            ),
            target("review:default", "work:default"),
            Writer::shore_local_author("0.1.0"),
            ReviewArtifactAcknowledgedPayload {
                acknowledgement_id,
                review_artifact_id,
                next_action: AcknowledgementNextAction::Accept,
                reason: Some("accepted".to_owned()),
                reason_artifact_path: None,
                reason_byte_size: Some(8),
                acknowledger: Writer::shore_local_author("0.1.0"),
            },
            "2026-05-10T00:00:00Z",
        )
        .expect("review artifact acknowledged event builds")
    }

    fn target(review_id: &str, work_unit_id: &str) -> EventTarget {
        EventTarget::new(ReviewId::new(review_id), WorkUnitId::new(work_unit_id))
    }
}
