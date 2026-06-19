use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::freshness::event_set_hash_for_events;
use crate::error::{Result, ShoreError};
use crate::model::{
    AssessmentId, EventId, InputRequestId, InputRequestResponseId, ObservationId, ReviewUnitId,
    RevisionId, SessionId, SnapshotId, ValidationCheckId, WorkUnitId,
};
use crate::session::EventWriteOutcome;
use crate::session::event::{
    AssertionMode, EventType, InputRequestRespondedPayload, ReviewAssessmentRecordedPayload,
    ReviewObservationRecordedPayload, ReviewUnitCapturedPayload, ShoreEvent,
    ValidationCheckRecordedPayload, decode_input_request_opened_payload,
};

const STATE_SCHEMA: &str = "shore.state";
const STATE_VERSION: u32 = 1;
pub const DUPLICATE_SEMANTIC_OBSERVATION_EVENT_CODE: &str = "duplicate_semantic_observation_event";
pub const DUPLICATE_SEMANTIC_INPUT_REQUEST_OPEN_EVENT_CODE: &str =
    "duplicate_semantic_input_request_open_event";
pub const DUPLICATE_SEMANTIC_INPUT_REQUEST_RESPONSE_EVENT_CODE: &str =
    "duplicate_semantic_input_request_response_event";
pub const DUPLICATE_SEMANTIC_ASSESSMENT_EVENT_CODE: &str = "duplicate_semantic_assessment_event";
pub const DUPLICATE_SEMANTIC_VALIDATION_EVENT_CODE: &str = "duplicate_semantic_validation_event";
const DUPLICATE_SEMANTIC_DIAGNOSTIC_EVENT_LIMIT: usize = 5;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    pub schema: String,
    pub version: u32,
    pub session_id: SessionId,
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
    pub assessment_count: usize,
    #[serde(default)]
    pub validation_check_count: usize,
    #[serde(default)]
    pub input_request_count: usize,
    #[serde(default)]
    pub open_input_request_count: usize,
    #[serde(default)]
    pub open_operative_input_request_count: usize,
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

    /// Builds the post-write bounded state from the already-loaded prior
    /// event batch plus the freshly committed event, without re-reading
    /// the event log. Produces the same value as
    /// `SessionState::from_events(&[prior + committed])` for
    /// `EventWriteOutcome::Created`, and the same value as
    /// `SessionState::from_events(&prior)` for existing-event outcomes (the
    /// committed event is already represented in `prior_events`).
    ///
    /// Assumes the V1 single-writer workflow contract: the `prior_events`
    /// batch was loaded by the same workflow that just called
    /// `record_event_once`, and no other writer mutated `.shore/data/events/`
    /// in between. Under that contract, existing-event outcomes always mean the
    /// matching event is already in `prior_events`.
    ///
    /// `.shore/data/events/` remains the canonical authority. Use `from_events`
    /// on read paths and for full rebuilds.
    pub(crate) fn from_prior_events_and_committed(
        prior_events: &[ShoreEvent],
        committed: &ShoreEvent,
        outcome: EventWriteOutcome,
    ) -> Result<Self> {
        let mut reducer = StateReducer::default();
        for event in prior_events {
            reducer.apply(event)?;
        }
        let (event_count, event_set_hash) = match outcome {
            EventWriteOutcome::Existing | EventWriteOutcome::ExistingDivergentSignature => {
                (prior_events.len(), event_set_hash_for_events(prior_events)?)
            }
            EventWriteOutcome::Created => {
                reducer.apply(committed)?;
                let event_set_hash = event_set_hash_for_events(
                    prior_events.iter().chain(std::iter::once(committed)),
                )?;
                (prior_events.len() + 1, event_set_hash)
            }
        };
        reducer.finish(event_count, event_set_hash)
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
    session_id: SessionId,
    work_unit_id: WorkUnitId,
    captured_review_units: BTreeMap<ReviewUnitId, (RevisionId, SnapshotId)>,
    note_count: usize,
    observation_events: BTreeMap<ObservationId, BTreeSet<EventId>>,
    assessment_events: BTreeMap<AssessmentId, BTreeSet<EventId>>,
    validation_check_events: BTreeMap<ValidationCheckId, BTreeSet<EventId>>,
    input_request_modes: BTreeMap<InputRequestId, AssertionMode>,
    input_request_open_events: BTreeMap<InputRequestId, BTreeSet<EventId>>,
    input_request_response_events: BTreeMap<InputRequestResponseId, BTreeSet<EventId>>,
    responded_input_request_ids: BTreeSet<InputRequestId>,
}

impl Default for StateReducer {
    fn default() -> Self {
        Self {
            session_id: SessionId::new("session:default"),
            work_unit_id: WorkUnitId::new("work:default"),
            captured_review_units: BTreeMap::new(),
            note_count: 0,
            observation_events: BTreeMap::new(),
            assessment_events: BTreeMap::new(),
            validation_check_events: BTreeMap::new(),
            input_request_modes: BTreeMap::new(),
            input_request_open_events: BTreeMap::new(),
            input_request_response_events: BTreeMap::new(),
            responded_input_request_ids: BTreeSet::new(),
        }
    }
}

impl StateReducer {
    fn apply(&mut self, event: &ShoreEvent) -> Result<()> {
        event.validate_schema_version()?;

        if event.event_type == EventType::ReviewInitialized {
            self.session_id = event.target.session_id.clone();
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
            EventType::ReviewAssessmentRecorded => self.apply_assessment_recorded(event)?,
            EventType::InputRequestOpened => self.apply_input_request_opened(event)?,
            EventType::InputRequestResponded => self.apply_input_request_responded(event)?,
            EventType::ReviewNoteImported => {
                self.note_count += 1;
            }
            EventType::ReviewUnitLineageDeclared | EventType::ReviewUnitLineageRoundRecorded => {
                // Lineage projections are derived by the dedicated lineage reducer.
            }
            EventType::ReviewUnitRefAssociated
            | EventType::ReviewUnitRefWithdrawn
            | EventType::ReviewUnitCommitAssociated
            | EventType::ReviewUnitCommitWithdrawn => {
                // Commit-range lifecycle is derived by the dedicated commit-range projection;
                // these events do not change session state.
            }
            EventType::ValidationCheckRecorded => self.apply_validation_check_recorded(event)?,
            EventType::TaskAttemptCaptured
            | EventType::TaskCheckpointCaptured
            | EventType::TaskObservationRecorded => {
                // Task-domain events do not contribute to review-session state.
            }
            EventType::EventSignatureRecorded => {
                // Detached co-signatures are read through the dedicated
                // co-signature-set projection; they do not change session state.
            }
        }

        Ok(())
    }

    fn set_identity_from_event_if_default(&mut self, event: &ShoreEvent) {
        if self.session_id.as_str() == "session:default" {
            self.session_id = event.target.session_id.clone();
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

    fn apply_assessment_recorded(&mut self, event: &ShoreEvent) -> Result<()> {
        let payload: ReviewAssessmentRecordedPayload =
            serde_json::from_value(event.payload.clone())?;
        self.assessment_events
            .entry(payload.assessment_id)
            .or_default()
            .insert(event.event_id.clone());
        Ok(())
    }

    fn apply_validation_check_recorded(&mut self, event: &ShoreEvent) -> Result<()> {
        let payload: ValidationCheckRecordedPayload =
            serde_json::from_value(event.payload.clone())?;
        self.validation_check_events
            .entry(payload.validation_check_id)
            .or_default()
            .insert(event.event_id.clone());
        Ok(())
    }

    fn apply_input_request_opened(&mut self, event: &ShoreEvent) -> Result<()> {
        let payload = decode_input_request_opened_payload(event.payload.clone())?;
        let input_request_id = payload.input_request_id;
        self.input_request_open_events
            .entry(input_request_id.clone())
            .or_default()
            .insert(event.event_id.clone());
        self.input_request_modes
            .entry(input_request_id)
            .or_insert(event.assertion_mode);
        Ok(())
    }

    fn apply_input_request_responded(&mut self, event: &ShoreEvent) -> Result<()> {
        let payload: InputRequestRespondedPayload = serde_json::from_value(event.payload.clone())?;
        self.input_request_response_events
            .entry(payload.input_request_response_id)
            .or_default()
            .insert(event.event_id.clone());
        self.responded_input_request_ids
            .insert(payload.input_request_id);
        Ok(())
    }

    fn finish(self, event_count: usize, event_set_hash: String) -> Result<SessionState> {
        let mut diagnostics = Vec::new();
        let current_review_unit = match self.captured_review_units.len() {
            0 => None,
            1 => self.captured_review_units.iter().next(),
            _ => None,
        };
        let current_review_unit_id =
            current_review_unit.map(|(review_unit_id, _)| review_unit_id.clone());
        let current_revision_id =
            current_review_unit.map(|(_, (revision_id, _))| revision_id.clone());
        let current_snapshot_id =
            current_review_unit.map(|(_, (_, snapshot_id))| snapshot_id.clone());
        let open_input_request_count = self
            .input_request_modes
            .keys()
            .filter(|input_request_id| {
                !self.responded_input_request_ids.contains(*input_request_id)
            })
            .count();
        let open_operative_input_request_count = self
            .input_request_modes
            .iter()
            .filter(|(input_request_id, mode)| {
                **mode == AssertionMode::Operative
                    && !self.responded_input_request_ids.contains(*input_request_id)
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
            DUPLICATE_SEMANTIC_INPUT_REQUEST_OPEN_EVENT_CODE,
            "input request",
            self.input_request_open_events
                .iter()
                .map(|(input_request_id, event_ids)| (input_request_id.as_str(), event_ids)),
        );
        append_duplicate_semantic_diagnostics(
            &mut diagnostics,
            DUPLICATE_SEMANTIC_INPUT_REQUEST_RESPONSE_EVENT_CODE,
            "input request response",
            self.input_request_response_events
                .iter()
                .map(|(resolution_id, event_ids)| (resolution_id.as_str(), event_ids)),
        );
        append_duplicate_semantic_diagnostics(
            &mut diagnostics,
            DUPLICATE_SEMANTIC_ASSESSMENT_EVENT_CODE,
            "assessment",
            self.assessment_events
                .iter()
                .map(|(assessment_id, event_ids)| (assessment_id.as_str(), event_ids)),
        );
        append_duplicate_semantic_diagnostics(
            &mut diagnostics,
            DUPLICATE_SEMANTIC_VALIDATION_EVENT_CODE,
            "validation check",
            self.validation_check_events
                .iter()
                .map(|(validation_check_id, event_ids)| (validation_check_id.as_str(), event_ids)),
        );
        Ok(SessionState {
            schema: STATE_SCHEMA.to_owned(),
            version: STATE_VERSION,
            session_id: self.session_id,
            work_unit_id: self.work_unit_id,
            current_revision_id,
            current_snapshot_id,
            review_unit_count: self.captured_review_units.len(),
            current_review_unit_id,
            event_count,
            event_set_hash: Some(event_set_hash),
            note_count: self.note_count,
            observation_count: self.observation_events.len(),
            assessment_count: self.assessment_events.len(),
            validation_check_count: self.validation_check_events.len(),
            input_request_count: self.input_request_modes.len(),
            open_input_request_count,
            open_operative_input_request_count,
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
    use crate::model::{
        AssessmentId, ReviewEndpoint, ReviewTargetRef, ReviewUnitSource, ValidationCheckId,
        ValidationStatus, ValidationTarget, ValidationTrigger, WorktreeCaptureMode,
    };
    use crate::session::EventWriteOutcome;
    use crate::session::event::{
        AssertionMode, EventTarget, InputRequestOpenedPayload, InputRequestReasonCode,
        InputRequestResponseOutcome, ReviewAssessment, ReviewAssessmentRecordedPayload,
        ReviewObservationRecordedPayload, ValidationCheckRecordedPayload, Writer,
    };

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
    fn projection_keeps_multi_capture_current_unset_without_ambient_diagnostic() {
        let events = vec![
            review_unit_captured_event("review-unit:sha256:one", "rev:one", "snap:one"),
            review_unit_captured_event("review-unit:sha256:two", "rev:two", "snap:two"),
        ];

        let state = SessionState::from_events(&events).unwrap();

        assert_eq!(state.current_review_unit_id, None);
        assert_eq!(state.current_revision_id, None);
        assert_eq!(state.current_snapshot_id, None);
        assert!(
            !state
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "ambiguous_current_review_unit")
        );
    }

    #[test]
    fn from_prior_events_and_committed_matches_full_replay_for_created() {
        let prior = vec![review_unit_captured_event(
            "review-unit:sha256:one",
            "rev:one",
            "snap:one",
        )];
        let committed = observation_event("retry-a", "obs:sha256:one");
        let mut all = prior.clone();
        all.push(committed.clone());

        let incremental = SessionState::from_prior_events_and_committed(
            &prior,
            &committed,
            EventWriteOutcome::Created,
        )
        .unwrap();
        let full = SessionState::from_events(&all).unwrap();

        assert_eq!(incremental, full);
    }

    #[test]
    fn from_prior_events_and_committed_matches_full_replay_for_existing() {
        let committed = observation_event("retry-a", "obs:sha256:one");
        let prior = vec![
            review_unit_captured_event("review-unit:sha256:one", "rev:one", "snap:one"),
            committed.clone(),
        ];

        let incremental = SessionState::from_prior_events_and_committed(
            &prior,
            &committed,
            EventWriteOutcome::Existing,
        )
        .unwrap();
        let full = SessionState::from_events(&prior).unwrap();

        assert_eq!(incremental, full);
        assert_eq!(incremental.event_count, 2);
    }

    #[test]
    fn from_prior_events_and_committed_empty_prior_with_created() {
        let committed = review_unit_captured_event("review-unit:sha256:one", "rev:one", "snap:one");

        let incremental = SessionState::from_prior_events_and_committed(
            &[],
            &committed,
            EventWriteOutcome::Created,
        )
        .unwrap();
        let full = SessionState::from_events(std::slice::from_ref(&committed)).unwrap();

        assert_eq!(incremental, full);
        assert_eq!(incremental.event_count, 1);
    }

    #[test]
    fn from_prior_events_and_committed_event_set_hash_is_order_independent() {
        let a = review_unit_captured_event("review-unit:sha256:one", "rev:one", "snap:one");
        let b = observation_event("retry-a", "obs:sha256:one");

        let from_a_then_b = SessionState::from_prior_events_and_committed(
            std::slice::from_ref(&a),
            &b,
            EventWriteOutcome::Created,
        )
        .unwrap();
        let from_b_then_a = SessionState::from_prior_events_and_committed(
            std::slice::from_ref(&b),
            &a,
            EventWriteOutcome::Created,
        )
        .unwrap();
        let full_ab = SessionState::from_events(&[a, b]).unwrap();

        assert_eq!(from_a_then_b.event_set_hash, full_ab.event_set_hash);
        assert_eq!(from_b_then_a.event_set_hash, full_ab.event_set_hash);
    }

    #[test]
    fn from_prior_events_and_committed_preserves_duplicate_semantic_diagnostic() {
        let prior = vec![
            review_unit_captured_event("review-unit:sha256:one", "rev:one", "snap:one"),
            observation_event("retry-a", "obs:sha256:same"),
            observation_event("retry-b", "obs:sha256:same"),
        ];
        let committed = observation_event("retry-c", "obs:sha256:same");
        let mut all = prior.clone();
        all.push(committed.clone());

        let incremental = SessionState::from_prior_events_and_committed(
            &prior,
            &committed,
            EventWriteOutcome::Created,
        )
        .unwrap();
        let full = SessionState::from_events(&all).unwrap();

        assert_eq!(incremental, full);
        assert!(
            incremental
                .diagnostics
                .iter()
                .any(|d| { d.code == DUPLICATE_SEMANTIC_OBSERVATION_EVENT_CODE }),
            "expected duplicate-semantic-observation diagnostic, got {:?}",
            incremental.diagnostics
        );
    }

    #[test]
    fn session_state_reducer_ignores_task_attempt_captured_event() {
        let event = ShoreEvent {
            schema: "shore.event".to_owned(),
            version: 1,
            event_id: EventId::new("evt:sha256:task-1"),
            event_type: EventType::TaskAttemptCaptured,
            idempotency_key: "task_attempt_captured:task-1".to_owned(),
            target: EventTarget::new(
                SessionId::new("session:claude:abc"),
                WorkUnitId::new("work:default"),
            ),
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

        let state = SessionState::from_events(&[event]).expect("task event applies as no-op");

        assert_eq!(state.review_unit_count, 0);
        assert_eq!(state.note_count, 0);
        assert_eq!(state.observation_count, 0);
        assert_eq!(state.assessment_count, 0);
        assert_eq!(state.input_request_count, 0);
        assert_eq!(state.open_input_request_count, 0);
        assert_eq!(state.open_operative_input_request_count, 0);
        assert!(state.current_review_unit_id.is_none());
        assert!(state.current_revision_id.is_none());
        assert!(state.current_snapshot_id.is_none());
    }

    #[test]
    fn session_state_serializes_assessment_count_with_assessment_count_wire_key() {
        let state = SessionState::from_events(&[]).unwrap();
        let json = serde_json::to_value(&state).unwrap();

        assert!(
            json.get("assessmentCount").is_some(),
            "missing assessmentCount in {json}"
        );
        let legacy_count_key = format!("{}Count", "disposition");
        assert!(
            json.get(&legacy_count_key).is_none(),
            "legacy {legacy_count_key} must not serialize after the assessment split"
        );
    }

    #[test]
    fn session_state_increments_assessment_count_for_review_assessment_recorded_event() {
        let events = vec![
            review_unit_captured_event("review-unit:sha256:one", "rev:one", "snap:one"),
            assessment_event("assess:sha256:one"),
        ];

        let state = SessionState::from_events(&events).unwrap();

        assert_eq!(state.event_count, 2);
        assert_eq!(state.review_unit_count, 1);
        assert_eq!(state.note_count, 0);
        assert_eq!(state.observation_count, 0);
        assert_eq!(state.assessment_count, 1);
        assert_eq!(state.input_request_count, 0);
        assert_eq!(state.open_input_request_count, 0);
        assert_eq!(state.open_operative_input_request_count, 0);
        assert!(state.diagnostics.is_empty());
    }

    #[test]
    fn session_state_increments_validation_check_count_for_validation_check_recorded_event() {
        let events = vec![
            review_unit_captured_event("review-unit:sha256:one", "rev:one", "snap:one"),
            validation_event("retry-a", "validation:sha256:one"),
        ];

        let state = SessionState::from_events(&events).unwrap();

        assert_eq!(state.validation_check_count, 1);
        assert_eq!(state.event_count, 2);
        assert!(state.diagnostics.is_empty());
    }

    #[test]
    fn session_state_serializes_validation_check_count_wire_key() {
        let state = SessionState::from_events(&[]).unwrap();
        let value = serde_json::to_value(state).unwrap();

        assert_eq!(value["validationCheckCount"], 0);
    }

    #[test]
    fn duplicate_semantic_validation_events_are_counted_once_with_validation_diagnostic_code() {
        let events = vec![
            validation_event("retry-a", "validation:sha256:same"),
            validation_event("retry-b", "validation:sha256:same"),
        ];

        let state = SessionState::from_events(&events).unwrap();

        assert_eq!(state.validation_check_count, 1);
        assert!(state.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == DUPLICATE_SEMANTIC_VALIDATION_EVENT_CODE
                && diagnostic.message.contains("validation:sha256:same")
        }));
    }

    #[test]
    fn session_state_omits_legacy_outcome_count_wire_key_after_split() {
        let state = SessionState::from_events(&[assessment_event("assess:sha256:one")]).unwrap();
        let json = serde_json::to_value(&state).unwrap();
        let legacy_count_key = format!("{}Count", "disposition");

        assert_eq!(state.assessment_count, 1);
        assert!(json.get(legacy_count_key).is_none());
    }

    #[test]
    fn duplicate_semantic_assessment_events_are_counted_once_with_assessment_diagnostic_code() {
        let events = vec![
            assessment_event_with_source("retry-a", "assess:sha256:same"),
            assessment_event_with_source("retry-b", "assess:sha256:same"),
        ];

        let state = SessionState::from_events(&events).unwrap();

        assert_eq!(state.assessment_count, 1);
        assert!(state.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == DUPLICATE_SEMANTIC_ASSESSMENT_EVENT_CODE
                && diagnostic.message.contains("assess:sha256:same")
        }));
    }

    #[test]
    fn duplicate_semantic_assessment_event_code_constant_exists() {
        assert_eq!(
            DUPLICATE_SEMANTIC_ASSESSMENT_EVENT_CODE,
            "duplicate_semantic_assessment_event"
        );
    }

    #[test]
    fn session_state_serializes_input_request_counts_not_intervention_counts() {
        let events = vec![input_request_opened_event_with_assertion_mode(
            "retry-a",
            "input-request:sha256:one",
            AssertionMode::Operative,
        )];

        let state = SessionState::from_events(&events).unwrap();
        let json = serde_json::to_value(&state).unwrap();

        assert_eq!(state.input_request_count, 1);
        assert_eq!(state.open_input_request_count, 1);
        assert_eq!(state.open_operative_input_request_count, 1);
        assert_eq!(json["inputRequestCount"], 1);
        assert_eq!(json["openInputRequestCount"], 1);
        assert_eq!(json["openOperativeInputRequestCount"], 1);
        assert!(json.get("interventionCount").is_none());
        assert!(json.get("openInterventionCount").is_none());
        assert!(json.get("openBlockingInterventionCount").is_none());
    }

    #[test]
    fn session_state_counts_open_operative_input_requests() {
        let events = vec![
            input_request_opened_event_with_assertion_mode(
                "retry-a",
                "input-request:sha256:operative",
                AssertionMode::Operative,
            ),
            input_request_opened_event_with_assertion_mode(
                "retry-b",
                "input-request:sha256:advisory",
                AssertionMode::Advisory,
            ),
        ];

        let state = SessionState::from_events(&events).unwrap();
        let json = serde_json::to_value(&state).unwrap();

        assert_eq!(state.input_request_count, 2);
        assert_eq!(state.open_input_request_count, 2);
        assert_eq!(state.open_operative_input_request_count, 1);
        assert_eq!(json["openOperativeInputRequestCount"], 1);
        assert!(json.get("openBlockingInputRequestCount").is_none());
    }

    #[test]
    fn input_request_response_closes_open_state_count() {
        let events = vec![
            input_request_opened_event_with_assertion_mode(
                "retry-a",
                "input-request:sha256:one",
                AssertionMode::Operative,
            ),
            input_request_responded_event(
                "retry-a",
                "input-request-response:sha256:one",
                "input-request:sha256:one",
            ),
        ];

        let state = SessionState::from_events(&events).unwrap();

        assert_eq!(state.input_request_count, 1);
        assert_eq!(state.open_input_request_count, 0);
        assert_eq!(state.open_operative_input_request_count, 0);
    }

    #[test]
    fn input_request_response_closes_open_operative_state_count() {
        let events = vec![
            input_request_opened_event_with_assertion_mode(
                "retry-a",
                "input-request:sha256:one",
                AssertionMode::Operative,
            ),
            input_request_responded_event(
                "retry-r",
                "input-request-response:sha256:one",
                "input-request:sha256:one",
            ),
        ];

        let state = SessionState::from_events(&events).unwrap();

        assert_eq!(state.open_input_request_count, 0);
        assert_eq!(state.open_operative_input_request_count, 0);
    }

    #[test]
    fn duplicate_semantic_input_request_events_use_input_request_diagnostic_codes() {
        let events = vec![
            input_request_opened_event_with_assertion_mode(
                "retry-a",
                "input-request:sha256:same",
                AssertionMode::Operative,
            ),
            input_request_opened_event_with_assertion_mode(
                "retry-b",
                "input-request:sha256:same",
                AssertionMode::Operative,
            ),
            input_request_responded_event(
                "retry-a",
                "input-request-response:sha256:same",
                "input-request:sha256:same",
            ),
            input_request_responded_event(
                "retry-b",
                "input-request-response:sha256:same",
                "input-request:sha256:same",
            ),
        ];

        let state = SessionState::from_events(&events).unwrap();

        assert_eq!(state.input_request_count, 1);
        assert_eq!(state.open_input_request_count, 0);
        assert!(state.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == DUPLICATE_SEMANTIC_INPUT_REQUEST_OPEN_EVENT_CODE
                && diagnostic.message.contains("input-request:sha256:same")
        }));
        assert!(state.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == DUPLICATE_SEMANTIC_INPUT_REQUEST_RESPONSE_EVENT_CODE
                && diagnostic
                    .message
                    .contains("input-request-response:sha256:same")
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
            "sessionId": "session:default",
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
        assert_eq!(state.assessment_count, 0);
        assert_eq!(state.input_request_count, 0);
        assert_eq!(state.validation_check_count, 0);
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
                SessionId::new("session:default"),
                ReviewUnitId::new(review_unit_id),
                RevisionId::new(revision_id),
                SnapshotId::new(snapshot_id),
            ),
            Writer::shore_local("0.1.0"),
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
                SessionId::new("session:default"),
                ReviewUnitId::new("review-unit:sha256:one"),
                RevisionId::new("rev:one"),
                SnapshotId::new("snap:one"),
            ),
            Writer::shore_local("0.1.0"),
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

    fn assessment_event(assessment_id: &str) -> ShoreEvent {
        assessment_event_with_source(assessment_id, assessment_id)
    }

    fn assessment_event_with_source(source_key: &str, assessment_id: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewAssessmentRecorded,
            format!("review_assessment_recorded:{source_key}"),
            EventTarget::for_review_unit(
                SessionId::new("session:default"),
                ReviewUnitId::new("review-unit:sha256:one"),
                RevisionId::new("rev:one"),
                SnapshotId::new("snap:one"),
            ),
            Writer::shore_local("0.1.0"),
            ReviewAssessmentRecordedPayload {
                assessment_id: AssessmentId::new(assessment_id),
                target: ReviewTargetRef::ReviewUnit {
                    review_unit_id: ReviewUnitId::new("review-unit:sha256:one"),
                },
                assessment: ReviewAssessment::Accepted,
                summary: None,
                summary_artifact_path: None,
                summary_byte_size: None,
                summary_content_hash: None,
                replaces_assessment_ids: Vec::new(),
                related_observation_ids: Vec::new(),
                related_input_request_ids: Vec::new(),
            },
            "2026-05-10T00:00:01Z",
        )
        .unwrap()
    }

    fn validation_event(source_key: &str, validation_check_id: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ValidationCheckRecorded,
            format!("validation_check_recorded:{source_key}"),
            EventTarget::for_review_unit(
                SessionId::new("session:default"),
                ReviewUnitId::new("review-unit:sha256:one"),
                RevisionId::new("rev:one"),
                SnapshotId::new("snap:one"),
            ),
            Writer::shore_local("0.1.0"),
            ValidationCheckRecordedPayload {
                validation_check_id: ValidationCheckId::new(validation_check_id),
                target: ValidationTarget::ReviewUnit {
                    review_unit_id: ReviewUnitId::new("review-unit:sha256:one"),
                },
                check_name: "cargo test".to_owned(),
                command: None,
                status: ValidationStatus::Passed,
                exit_code: Some(0),
                trigger: ValidationTrigger::Manual,
                source_fingerprint: None,
                summary: None,
                summary_artifact_path: None,
                summary_byte_size: None,
                summary_content_hash: None,
                started_at: None,
                completed_at: None,
                log_artifact_content_hashes: Vec::new(),
            },
            "2026-05-10T00:00:02Z",
        )
        .unwrap()
    }

    fn input_request_opened_event_with_assertion_mode(
        source_key: &str,
        input_request_id: &str,
        assertion_mode: AssertionMode,
    ) -> ShoreEvent {
        let mut target = EventTarget::for_review_unit(
            SessionId::new("session:default"),
            ReviewUnitId::new("review-unit:sha256:one"),
            RevisionId::new("rev:one"),
            SnapshotId::new("snap:one"),
        );
        target.track_id = Some(crate::model::TrackId::new("agent:codex"));
        ShoreEvent::new(
            EventType::InputRequestOpened,
            format!("input_request_opened:{source_key}"),
            target,
            Writer::shore_local("0.1.0"),
            InputRequestOpenedPayload {
                input_request_id: InputRequestId::new(input_request_id),
                target: ReviewTargetRef::ReviewUnit {
                    review_unit_id: ReviewUnitId::new("review-unit:sha256:one"),
                },
                reason_code: InputRequestReasonCode::ManualDecisionRequired,
                title: "Need input".to_owned(),
                body: None,
                body_artifact_path: None,
                body_byte_size: None,
                body_content_hash: None,
                target_fingerprint: None,
            },
            "2026-05-10T00:00:02Z",
        )
        .unwrap()
        .with_assertion_mode(assertion_mode)
    }

    fn input_request_responded_event(
        source_key: &str,
        input_request_response_id: &str,
        input_request_id: &str,
    ) -> ShoreEvent {
        let mut target = EventTarget::for_review_unit(
            SessionId::new("session:default"),
            ReviewUnitId::new("review-unit:sha256:one"),
            RevisionId::new("rev:one"),
            SnapshotId::new("snap:one"),
        );
        target.subject = Some(crate::model::TargetRef::Review(
            ReviewTargetRef::InputRequest {
                review_unit_id: ReviewUnitId::new("review-unit:sha256:one"),
                input_request_id: InputRequestId::new(input_request_id),
            },
        ));
        ShoreEvent::new(
            EventType::InputRequestResponded,
            format!("input_request_responded:{source_key}"),
            target,
            Writer::shore_local("0.1.0"),
            InputRequestRespondedPayload {
                input_request_response_id: InputRequestResponseId::new(input_request_response_id),
                input_request_id: InputRequestId::new(input_request_id),
                outcome: InputRequestResponseOutcome::Approved,
                reason: None,
                reason_artifact_path: None,
                reason_byte_size: None,
                reason_content_hash: None,
                target_fingerprint: None,
            },
            "2026-05-10T00:00:03Z",
        )
        .unwrap()
    }
}
