//! Sibling task-domain projection over `ShoreEvent`s.
//!
//! Reads already-written task-domain events into a per-attempt summary.
//! `SessionState` remains review-domain; `review_history` filters task events
//! out unconditionally. This module is the sibling task entry point.

use std::collections::BTreeMap;

use crate::crypto::EventVerificationStatus;
use crate::error::Result;
use crate::model::{
    ActorId, CheckpointId, EventId, InputRequestId, InputRequestResponseId, ObservationId,
    ReviewTargetRef, TargetRef, TaskTargetRef, WorkObjectId, WorkObjectType,
};
use crate::session::event::{
    AssertionMode, EventTarget, EventType, InputRequestOpenedPayload, InputRequestReasonCode,
    InputRequestRespondedPayload, InputRequestResponseOutcome, ShoreEvent, SourceRef,
    TaskAttemptCapturedPayload, TaskCheckpointCapturedPayload, TaskObservationRecordedPayload,
    Writer, decode_input_request_opened_payload,
};
use crate::session::{
    DelegationMap, PrincipalPolicy, PrincipalResolution, TrustSet, is_agent_actor_id,
    principal_resolution_for_writer, principal_sufficient, verify_event_signature,
};

/// Envelope-level fields preserved on every projected event, so the projection
/// does not silently lose envelope identity / authorship / source provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskProjectionEventEnvelope {
    pub event_id: EventId,
    pub event_type: EventType,
    pub occurred_at: String,
    pub payload_hash: String,
    pub writer: Writer,
    pub assertion_mode: AssertionMode,
    pub source_ref: Option<SourceRef>,
    pub target: EventTarget,
}

impl TaskProjectionEventEnvelope {
    fn from_event(event: &ShoreEvent) -> Self {
        Self {
            event_id: event.event_id.clone(),
            event_type: event.event_type,
            occurred_at: event.occurred_at.clone(),
            payload_hash: event.payload_hash.clone(),
            writer: event.writer.clone(),
            assertion_mode: event.assertion_mode,
            source_ref: event.source_ref.clone(),
            target: event.target.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskObservationSummary {
    pub envelope: TaskProjectionEventEnvelope,
    pub observation_id: ObservationId,
    pub checkpoint_id: Option<CheckpointId>,
    pub title: String,
    pub body: Option<String>,
    pub body_artifact_path: Option<String>,
    pub body_byte_size: Option<u64>,
    pub body_content_hash: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskCheckpointSummary {
    pub envelope: TaskProjectionEventEnvelope,
    pub checkpoint_id: CheckpointId,
    pub assistant_message_id: String,
    pub tool_use_ids: Vec<String>,
    /// Opaque fingerprint of the code state at this checkpoint, preserved
    /// from the payload so the resumption projection can compare it to a
    /// response's `target_fingerprint`.
    pub checkpoint_fingerprint: Option<String>,
    pub observations: Vec<TaskObservationSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskProjectionDiagnostic {
    pub code: String,
    pub message: String,
    pub event_id: Option<EventId>,
    /// Bounded machine-readable detail for diagnostics whose code carries a
    /// closed reason vocabulary (ADR-0009's
    /// `agent_resumption_response_identity_not_binding`); `None` elsewhere.
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskAttemptSummary {
    pub reader_actor_id: ActorId,
    pub task_attempt_id: WorkObjectId,
    pub attempt_event: TaskProjectionEventEnvelope,
    pub project_path: String,
    pub claude_session_uuid: String,
    pub initial_prompt_hash: String,
    pub predecessor: Option<WorkObjectId>,
    /// Opaque fingerprint of the code state at the start of the attempt,
    /// preserved verbatim from `TaskAttemptCapturedPayload`.
    pub base_snapshot_fingerprint: Option<String>,
    pub latest_checkpoint: Option<TaskCheckpointSummary>,
    pub checkpoints: Vec<TaskCheckpointSummary>,
    pub observations_without_checkpoint: Vec<TaskObservationSummary>,
    pub diagnostics: Vec<TaskProjectionDiagnostic>,
}

/// Roll up `TaskAttemptCaptured`, `TaskCheckpointCaptured`, and
/// `TaskObservationRecorded` events for a single `TaskAttempt` into a
/// human/agent-readable summary.
///
/// Returns `Ok(None)` if no `TaskAttemptCaptured` event for the requested
/// `task_attempt_id` is present.
#[allow(dead_code)]
pub(crate) fn task_attempt_summary_from_events(
    events: &[ShoreEvent],
    task_attempt_id: &WorkObjectId,
    reader_actor_id: &ActorId,
) -> Result<Option<TaskAttemptSummary>> {
    let mut attempt: Option<(TaskProjectionEventEnvelope, TaskAttemptCapturedPayload)> = None;
    let mut checkpoint_envelopes: BTreeMap<
        CheckpointId,
        (TaskProjectionEventEnvelope, TaskCheckpointCapturedPayload),
    > = BTreeMap::new();
    let mut observation_records: Vec<(
        TaskProjectionEventEnvelope,
        TaskObservationRecordedPayload,
    )> = Vec::new();

    for event in events {
        event.validate_schema_version()?;

        if !targets_task_attempt(event, task_attempt_id) {
            continue;
        }

        match event.event_type {
            EventType::TaskAttemptCaptured => {
                let payload: TaskAttemptCapturedPayload =
                    serde_json::from_value(event.payload.clone())?;
                if payload.task_attempt_id == *task_attempt_id {
                    attempt = Some((TaskProjectionEventEnvelope::from_event(event), payload));
                }
            }
            EventType::TaskCheckpointCaptured => {
                let payload: TaskCheckpointCapturedPayload =
                    serde_json::from_value(event.payload.clone())?;
                if payload.parent_task_attempt_id == *task_attempt_id {
                    checkpoint_envelopes.insert(
                        payload.checkpoint_id.clone(),
                        (TaskProjectionEventEnvelope::from_event(event), payload),
                    );
                }
            }
            EventType::TaskObservationRecorded => {
                let payload: TaskObservationRecordedPayload =
                    serde_json::from_value(event.payload.clone())?;
                observation_records.push((TaskProjectionEventEnvelope::from_event(event), payload));
            }
            _ => continue,
        }
    }

    let Some((attempt_envelope, attempt_payload)) = attempt else {
        return Ok(None);
    };

    let mut diagnostics: Vec<TaskProjectionDiagnostic> = Vec::new();
    let mut observations_by_checkpoint: BTreeMap<CheckpointId, Vec<TaskObservationSummary>> =
        BTreeMap::new();
    let mut observations_without_checkpoint: Vec<TaskObservationSummary> = Vec::new();

    for (envelope, payload) in observation_records {
        let summary = TaskObservationSummary {
            envelope: envelope.clone(),
            observation_id: payload.observation_id,
            checkpoint_id: payload.checkpoint_id.clone(),
            title: payload.title,
            body: payload.body,
            body_artifact_path: payload.body_artifact_path,
            body_byte_size: payload.body_byte_size,
            body_content_hash: payload.body_content_hash,
        };

        match &summary.checkpoint_id {
            Some(checkpoint_id) => {
                if !checkpoint_envelopes.contains_key(checkpoint_id) {
                    diagnostics.push(TaskProjectionDiagnostic {
                        code: "observation_checkpoint_missing".to_owned(),
                        message: format!(
                            "observation {} names checkpoint {} which has no \
                             TaskCheckpointCaptured event under this attempt",
                            summary.observation_id.as_str(),
                            checkpoint_id.as_str()
                        ),
                        event_id: Some(envelope.event_id.clone()),
                        reason: None,
                    });
                    observations_without_checkpoint.push(summary);
                } else {
                    observations_by_checkpoint
                        .entry(checkpoint_id.clone())
                        .or_default()
                        .push(summary);
                }
            }
            None => observations_without_checkpoint.push(summary),
        }
    }

    sort_observations_recent_first(&mut observations_without_checkpoint);
    for bucket in observations_by_checkpoint.values_mut() {
        sort_observations_recent_first(bucket);
    }

    let mut checkpoints: Vec<TaskCheckpointSummary> = checkpoint_envelopes
        .into_values()
        .map(|(envelope, payload)| {
            let observations = observations_by_checkpoint
                .remove(&payload.checkpoint_id)
                .unwrap_or_default();
            TaskCheckpointSummary {
                envelope,
                checkpoint_id: payload.checkpoint_id,
                assistant_message_id: payload.assistant_message_id,
                tool_use_ids: payload.tool_use_ids,
                checkpoint_fingerprint: payload.checkpoint_fingerprint,
                observations,
            }
        })
        .collect();

    checkpoints
        .sort_by(|left, right| envelope_chronological_order(&left.envelope, &right.envelope));

    let latest_checkpoint = checkpoints
        .iter()
        .max_by(|left, right| envelope_chronological_order(&left.envelope, &right.envelope))
        .cloned();

    Ok(Some(TaskAttemptSummary {
        reader_actor_id: reader_actor_id.clone(),
        task_attempt_id: task_attempt_id.clone(),
        attempt_event: attempt_envelope,
        project_path: attempt_payload.project_path,
        claude_session_uuid: attempt_payload.claude_session_uuid,
        initial_prompt_hash: attempt_payload.initial_prompt_hash,
        predecessor: attempt_payload.predecessor,
        base_snapshot_fingerprint: attempt_payload.base_snapshot_fingerprint,
        latest_checkpoint,
        checkpoints,
        observations_without_checkpoint,
        diagnostics,
    }))
}

fn targets_task_attempt(event: &ShoreEvent, task_attempt_id: &WorkObjectId) -> bool {
    matches!(
        event.event_type,
        EventType::TaskAttemptCaptured
            | EventType::TaskCheckpointCaptured
            | EventType::TaskObservationRecorded
    ) && event.target.work_object_id.as_ref() == Some(task_attempt_id)
        && event.target.work_object_type == Some(WorkObjectType::TaskAttempt)
}

fn envelope_chronological_order(
    left: &TaskProjectionEventEnvelope,
    right: &TaskProjectionEventEnvelope,
) -> std::cmp::Ordering {
    left.occurred_at
        .cmp(&right.occurred_at)
        .then_with(|| left.event_id.as_str().cmp(right.event_id.as_str()))
}

fn sort_observations_recent_first(observations: &mut [TaskObservationSummary]) {
    observations.sort_by(|left, right| {
        right
            .envelope
            .occurred_at
            .cmp(&left.envelope.occurred_at)
            .then_with(|| {
                left.envelope
                    .event_id
                    .as_str()
                    .cmp(right.envelope.event_id.as_str())
            })
    });
}

/// One open task-targeted input request. The envelope is authoritative; the
/// payload's review-shaped `target` is preserved so callers can detect the
/// current shape mismatch without losing data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskInputRequestView {
    pub input_request_id: InputRequestId,
    pub envelope: TaskProjectionEventEnvelope,
    pub target: TargetRef,
    pub payload_review_target: ReviewTargetRef,
    pub mode: AssertionMode,
    pub reason_code: InputRequestReasonCode,
    pub title: String,
    pub body: Option<String>,
    pub body_artifact_path: Option<String>,
    pub body_byte_size: Option<u64>,
    pub body_content_hash: Option<String>,
    /// Opaque fingerprint the requester observed when opening the input
    /// request, preserved verbatim from `InputRequestOpenedPayload`.
    pub target_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskInputRequestsProjection {
    pub reader_actor_id: ActorId,
    pub task_attempt_id: WorkObjectId,
    pub open_input_requests: Vec<TaskInputRequestView>,
    pub diagnostics: Vec<TaskProjectionDiagnostic>,
}

/// Project unresponded `InputRequestOpened` events whose durable envelope
/// targets the given `TaskAttempt`. Responded input requests (matched by
/// `input_request_id`) are excluded.
#[allow(dead_code)]
pub(crate) fn open_task_input_requests_from_events(
    events: &[ShoreEvent],
    task_attempt_id: &WorkObjectId,
    reader_actor_id: &ActorId,
) -> Result<TaskInputRequestsProjection> {
    let mut requests: Vec<(
        TaskProjectionEventEnvelope,
        InputRequestOpenedPayload,
        TargetRef,
    )> = Vec::new();
    let mut responded: std::collections::BTreeSet<InputRequestId> =
        std::collections::BTreeSet::new();

    for event in events {
        event.validate_schema_version()?;

        match event.event_type {
            EventType::InputRequestOpened => {
                if !envelope_targets_task_attempt(&event.target, task_attempt_id) {
                    continue;
                }
                let Some(subject) = event.target.subject.clone() else {
                    continue;
                };
                if !matches!(subject, TargetRef::Task(_)) {
                    continue;
                }
                let payload = decode_input_request_opened_payload(event.payload.clone())?;
                requests.push((
                    TaskProjectionEventEnvelope::from_event(event),
                    payload,
                    subject,
                ));
            }
            EventType::InputRequestResponded => {
                let payload: InputRequestRespondedPayload =
                    serde_json::from_value(event.payload.clone())?;
                responded.insert(payload.input_request_id);
            }
            _ => {}
        }
    }

    let mut diagnostics: Vec<TaskProjectionDiagnostic> = Vec::new();
    let mut open_input_requests: Vec<TaskInputRequestView> = Vec::new();

    for (envelope, payload, subject) in requests {
        if responded.contains(&payload.input_request_id) {
            continue;
        }

        // The payload's review-shaped `target` is preserved verbatim so callers
        // see the current shape mismatch instead of having it silently
        // promoted to the task target.
        diagnostics.push(TaskProjectionDiagnostic {
            code: "task_input_request_payload_target_is_review_shaped".to_owned(),
            message: format!(
                "input request {} targets a TaskAttempt via the envelope but its \
                 payload `target` is still a ReviewTargetRef",
                payload.input_request_id.as_str()
            ),
            event_id: Some(envelope.event_id.clone()),
            reason: None,
        });

        let mode = envelope.assertion_mode;
        open_input_requests.push(TaskInputRequestView {
            input_request_id: payload.input_request_id,
            envelope,
            target: subject,
            payload_review_target: payload.target,
            mode,
            reason_code: payload.reason_code,
            title: payload.title,
            body: payload.body,
            body_artifact_path: payload.body_artifact_path,
            body_byte_size: payload.body_byte_size,
            body_content_hash: payload.body_content_hash,
            target_fingerprint: payload.target_fingerprint,
        });
    }

    open_input_requests
        .sort_by(|left, right| envelope_chronological_order(&left.envelope, &right.envelope));

    Ok(TaskInputRequestsProjection {
        reader_actor_id: reader_actor_id.clone(),
        task_attempt_id: task_attempt_id.clone(),
        open_input_requests,
        diagnostics,
    })
}

fn envelope_targets_task_attempt(target: &EventTarget, task_attempt_id: &WorkObjectId) -> bool {
    target.work_object_id.as_ref() == Some(task_attempt_id)
        && target.work_object_type == Some(WorkObjectType::TaskAttempt)
}

/// Agent-resumption state. Diagnostic-rich rather than scalar so the caller
/// can present the reason the projection blocked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgentResumptionState {
    NoAttempt,
    Ready,
    Blocked,
    Ambiguous,
    Stale,
}

/// Why a task-targeted input request is considered fresh (or not) for the
/// resumption rule. The identity variants cover the checkpoint-by-identity
/// fallback; the fingerprint variant generalizes staleness to opaque-string
/// code-state fingerprints.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FreshnessBasis {
    /// No fingerprint check applies because the input request targets the
    /// `TaskAttempt` itself rather than a specific checkpoint.
    TaskAttemptNoFingerprintCheck,
    /// Target checkpoint is the latest checkpoint on the attempt.
    CheckpointMatchesLatest,
    /// Target checkpoint is not the latest checkpoint on the attempt.
    CheckpointStaleNewerExists,
    /// Target was a checkpoint, but no checkpoint exists at all on the attempt.
    CheckpointWithoutAttemptCheckpoint,
    /// Response's `target_fingerprint` disagrees with the latest
    /// checkpoint's `checkpoint_fingerprint`; both are `Some`. Opaque-string
    /// `==` comparison only -- no domain semantics.
    CheckpointFingerprintMismatch,
    /// Response's `target_fingerprint` agrees with the latest checkpoint's
    /// `checkpoint_fingerprint`; both are `Some`. Fingerprint agreement
    /// overrides the identity-based staleness fallback so a responder acting
    /// on the right code state under a stale checkpoint id stays fresh.
    CheckpointFingerprintMatches,
}

impl FreshnessBasis {
    fn is_fresh(self) -> bool {
        matches!(
            self,
            FreshnessBasis::TaskAttemptNoFingerprintCheck
                | FreshnessBasis::CheckpointMatchesLatest
                | FreshnessBasis::CheckpointFingerprintMatches
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentInputRequestResponsePolicyView {
    pub envelope: TaskProjectionEventEnvelope,
    pub response_id: InputRequestResponseId,
    pub outcome: InputRequestResponseOutcome,
    pub identity_treated_as_binding: bool,
    pub fresh_for_target: bool,
    pub operative_reason: Option<String>,
    /// Free-text responder justification carried verbatim from the
    /// `InputRequestRespondedPayload`. Preserved here so the projection does
    /// not lose the responder's stated reason.
    pub reason: Option<String>,
    pub reason_artifact_path: Option<String>,
    pub reason_byte_size: Option<u64>,
    pub reason_content_hash: Option<String>,
    /// Opaque fingerprint the responder acted on, preserved verbatim from the
    /// `InputRequestRespondedPayload`. The freshness rule compares this to the
    /// latest checkpoint's `checkpoint_fingerprint`.
    pub target_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentResumptionProjection {
    pub reader_actor_id: ActorId,
    pub task_attempt_id: WorkObjectId,
    pub may_resume: bool,
    pub state: AgentResumptionState,
    pub latest_checkpoint: Option<CheckpointId>,
    pub selected_input_request: Option<TaskInputRequestView>,
    pub selected_response: Option<AgentInputRequestResponsePolicyView>,
    pub treated_as_operative: bool,
    pub freshness: Option<FreshnessBasis>,
    pub diagnostics: Vec<TaskProjectionDiagnostic>,
}

/// ADR-0009: the named reader-side projection policy for resumption binding.
/// This is ADR-0003's "a specific projection policy treats them as operative"
/// given a name. It is not the store's `EventVerificationPolicy` — acceptance
/// and bindingness are separate questions.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ResumptionBindingPolicy {
    /// Arm (a) local possession and arm (b) verified signer both bind.
    #[default]
    LocalAndVerified,
    /// Only arm (b) binds: nothing binds without a key — including the
    /// store's own unsigned responses. For stores whose possession does not
    /// imply authorship (shared checkouts, cp -r copies).
    ///
    /// Like `agent_resumption_from_events` itself, this preset has no
    /// production caller yet — the in-module fixture suite is the only
    /// consumer until a workflow-level resumption read surface lands.
    #[allow(dead_code)]
    VerifiedOnly,
}

impl ResumptionBindingPolicy {
    fn permits_local_possession(self) -> bool {
        matches!(self, ResumptionBindingPolicy::LocalAndVerified)
    }
}

/// The per-response binding evidence: verification status (arm b) and
/// ingest-stamp presence (arm a). Computed where the full `ShoreEvent` is in
/// hand; the predicate never reads a self-asserted field.
fn response_binding_evidence(
    event: &ShoreEvent,
    trust_set: &TrustSet,
) -> Result<(EventVerificationStatus, bool)> {
    Ok((
        verify_event_signature(event, trust_set)?,
        event.ingest.is_some(),
    ))
}

/// Read-side projection that answers: may this agent resume the named
/// `TaskAttempt`? The output is intentionally diagnostic-rich; the policy
/// never relies on a scheduler, lease, write gate, or global "current task".
#[allow(dead_code)]
pub(crate) fn agent_resumption_from_events(
    events: &[ShoreEvent],
    task_attempt_id: &WorkObjectId,
    reader_actor_id: &ActorId,
    trust_set: &TrustSet,
    binding_policy: ResumptionBindingPolicy,
) -> Result<AgentResumptionProjection> {
    // Default principal policy (`None`) is provably outcome-neutral, so the
    // ADR-0009 entry point is this thin wrapper.
    agent_resumption_with_principal_policy(
        events,
        task_attempt_id,
        reader_actor_id,
        trust_set,
        binding_policy,
        None,
        PrincipalPolicy::None,
    )
}

/// ADR-0010: the resumption projection composed with principal sufficiency.
/// `binding' := binding(ADR-0009) AND principal_sufficient(ADR-0010)` —
/// narrowing only. The delegation map and principal policy are reader-supplied
/// config the agent does not control; the ADR-0009 arms remain the trust basis.
#[allow(dead_code)]
pub(crate) fn agent_resumption_with_principal_policy(
    events: &[ShoreEvent],
    task_attempt_id: &WorkObjectId,
    reader_actor_id: &ActorId,
    trust_set: &TrustSet,
    binding_policy: ResumptionBindingPolicy,
    delegation_map: Option<&DelegationMap>,
    principal_policy: PrincipalPolicy,
) -> Result<AgentResumptionProjection> {
    let attempt_summary =
        task_attempt_summary_from_events(events, task_attempt_id, reader_actor_id)?;

    let Some(summary) = attempt_summary else {
        return Ok(AgentResumptionProjection {
            reader_actor_id: reader_actor_id.clone(),
            task_attempt_id: task_attempt_id.clone(),
            may_resume: false,
            state: AgentResumptionState::NoAttempt,
            latest_checkpoint: None,
            selected_input_request: None,
            selected_response: None,
            treated_as_operative: false,
            freshness: None,
            diagnostics: vec![TaskProjectionDiagnostic {
                code: "agent_resumption_no_task_attempt".to_owned(),
                message: format!(
                    "no TaskAttemptCaptured event for {} — failing closed",
                    task_attempt_id.as_str()
                ),
                event_id: None,
                reason: None,
            }],
        });
    };

    let latest_checkpoint = summary
        .latest_checkpoint
        .as_ref()
        .map(|cp| cp.checkpoint_id.clone());
    let latest_checkpoint_fingerprint = summary
        .latest_checkpoint
        .as_ref()
        .and_then(|cp| cp.checkpoint_fingerprint.clone());

    let input_request_records =
        collect_task_input_request_records(events, task_attempt_id, trust_set)?;
    let operative_input_request_records = input_request_records
        .iter()
        .filter(|record| record.view.mode == AssertionMode::Operative)
        .collect::<Vec<_>>();

    // Open (no representative response) input requests short-circuit to
    // Blocked. Pick the earliest by (occurred_at, event_id) as the selected
    // one so the diagnostic chain is deterministic.
    let open_input_request = operative_input_request_records
        .iter()
        .filter(|record| record.responses.is_empty())
        .min_by(|left, right| {
            envelope_chronological_order(&left.view.envelope, &right.view.envelope)
        })
        .cloned();

    if let Some(open) = open_input_request {
        return Ok(AgentResumptionProjection {
            reader_actor_id: reader_actor_id.clone(),
            task_attempt_id: task_attempt_id.clone(),
            may_resume: false,
            state: AgentResumptionState::Blocked,
            latest_checkpoint,
            selected_input_request: Some(open.view.clone()),
            selected_response: None,
            treated_as_operative: false,
            freshness: None,
            diagnostics: vec![TaskProjectionDiagnostic {
                code: "agent_resumption_open_task_input_request".to_owned(),
                message: format!(
                    "task input request {} is unresponded",
                    open.view.input_request_id.as_str()
                ),
                event_id: Some(open.view.envelope.event_id.clone()),
                reason: None,
            }],
        });
    }

    // Ambiguous responses block resumption; the projection refuses to pick a
    // winner.
    let ambiguous_input_request = operative_input_request_records
        .iter()
        .find(|record| record.responses.len() > 1)
        .copied();

    if let Some(ambiguous) = ambiguous_input_request {
        return Ok(AgentResumptionProjection {
            reader_actor_id: reader_actor_id.clone(),
            task_attempt_id: task_attempt_id.clone(),
            may_resume: false,
            state: AgentResumptionState::Ambiguous,
            latest_checkpoint,
            selected_input_request: Some(ambiguous.view.clone()),
            selected_response: None,
            treated_as_operative: false,
            freshness: None,
            diagnostics: vec![TaskProjectionDiagnostic {
                code: "agent_resumption_ambiguous_input_request_responses".to_owned(),
                message: format!(
                    "task input request {} has {} representative responses; projection refuses to pick a winner",
                    ambiguous.view.input_request_id.as_str(),
                    ambiguous.responses.len()
                ),
                event_id: Some(ambiguous.view.envelope.event_id.clone()),
                reason: None,
            }],
        });
    }

    // Evaluate each responded input request against the binding/freshness rules.
    // First failure wins so the projection surfaces it.
    let mut last_satisfied: Option<(
        TaskInputRequestView,
        AgentInputRequestResponsePolicyView,
        FreshnessBasis,
    )> = None;
    // ADR-0010: under `prefer`, an unresolved/ambiguous agent principal is an
    // advisory diagnostic with no operative effect; collected here and attached
    // to the projection on the satisfied (Ready) path.
    let mut principal_advisories: Vec<TaskProjectionDiagnostic> = Vec::new();
    for record in operative_input_request_records {
        let response = record
            .responses
            .first()
            .expect("non-empty after open / ambiguous branches");

        let freshness = freshness_for_task_target(
            &record.view,
            latest_checkpoint.as_ref(),
            latest_checkpoint_fingerprint.as_deref(),
            response.target_fingerprint.as_deref(),
        );
        let identity_binding =
            response_identity_is_binding(response.verification, response.ingested, binding_policy);
        // ADR-0010: binding' = binding AND principal_sufficient. Narrowing only.
        let principal_ok = principal_sufficient(
            &response.envelope.writer.actor_id,
            &response.envelope.occurred_at,
            delegation_map,
            principal_policy,
        );
        let binding = identity_binding && principal_ok;
        // `prefer` surfaces the same reason as advisory only.
        if principal_policy == PrincipalPolicy::Prefer
            && let Some(reason) = principal_block_reason(
                &response.envelope.writer.actor_id,
                &response.envelope.occurred_at,
                delegation_map,
            )
        {
            principal_advisories.push(TaskProjectionDiagnostic {
                code: "agent_resumption_response_principal_advisory".to_owned(),
                message: principal_block_message(reason).to_owned(),
                event_id: Some(response.envelope.event_id.clone()),
                reason: Some(reason.to_owned()),
            });
        }
        let outcome_allows = matches!(response.outcome, InputRequestResponseOutcome::Approved);
        let operative = response.envelope.assertion_mode == AssertionMode::Operative;

        let operative_reason = if binding && operative && outcome_allows {
            Some(
                "approved by a verified binding identity with envelope-level operative assertion mode"
                    .to_owned(),
            )
        } else {
            None
        };

        let response_view = AgentInputRequestResponsePolicyView {
            envelope: response.envelope.clone(),
            response_id: response.response_id.clone(),
            outcome: response.outcome,
            identity_treated_as_binding: identity_binding,
            fresh_for_target: freshness.is_fresh(),
            operative_reason,
            reason: response.reason.clone(),
            reason_artifact_path: response.reason_artifact_path.clone(),
            reason_byte_size: response.reason_byte_size,
            reason_content_hash: response.reason_content_hash.clone(),
            target_fingerprint: response.target_fingerprint.clone(),
        };

        let stale = matches!(
            freshness,
            FreshnessBasis::CheckpointStaleNewerExists
                | FreshnessBasis::CheckpointWithoutAttemptCheckpoint
                | FreshnessBasis::CheckpointFingerprintMismatch
        );

        if stale {
            let diagnostic = match freshness {
                FreshnessBasis::CheckpointFingerprintMismatch => TaskProjectionDiagnostic {
                    code: "agent_resumption_response_target_fingerprint_mismatch".to_owned(),
                    message: format!(
                        "task input request {} was responded to against a code state whose fingerprint disagrees with the latest checkpoint on this attempt",
                        record.view.input_request_id.as_str()
                    ),
                    event_id: Some(response.envelope.event_id.clone()),
                    reason: None,
                },
                _ => TaskProjectionDiagnostic {
                    code: "agent_resumption_response_targets_stale_checkpoint".to_owned(),
                    message: format!(
                        "task input request {} was responded to against an older checkpoint than the latest checkpoint on this attempt",
                        record.view.input_request_id.as_str()
                    ),
                    event_id: Some(response.envelope.event_id.clone()),
                    reason: None,
                },
            };
            return Ok(AgentResumptionProjection {
                reader_actor_id: reader_actor_id.clone(),
                task_attempt_id: task_attempt_id.clone(),
                may_resume: false,
                state: AgentResumptionState::Stale,
                latest_checkpoint,
                selected_input_request: Some(record.view.clone()),
                selected_response: Some(response_view),
                treated_as_operative: false,
                freshness: Some(freshness),
                diagnostics: vec![diagnostic],
            });
        }

        if !(binding && operative && outcome_allows) {
            let mut diagnostics = Vec::new();
            if !outcome_allows {
                diagnostics.push(TaskProjectionDiagnostic {
                    code: "agent_resumption_outcome_not_approved".to_owned(),
                    message: format!(
                        "response outcome is {:?}; only Approved permits resumption",
                        response.outcome
                    ),
                    event_id: Some(response.envelope.event_id.clone()),
                    reason: None,
                });
            }
            if !operative {
                diagnostics.push(TaskProjectionDiagnostic {
                    code: "agent_resumption_response_assertion_mode_not_operative".to_owned(),
                    message:
                        "response envelope assertion_mode is not Operative; advisory responses do not bind"
                            .to_owned(),
                    event_id: Some(response.envelope.event_id.clone()),
                    reason: None,
                });
            }
            if !identity_binding {
                let reason =
                    non_binding_reason(response.verification, response.ingested, binding_policy);
                diagnostics.push(TaskProjectionDiagnostic {
                    code: "agent_resumption_response_identity_not_binding".to_owned(),
                    message: non_binding_message(reason).to_owned(),
                    event_id: Some(response.envelope.event_id.clone()),
                    reason: Some(reason.to_owned()),
                });
            } else if !principal_ok {
                // ADR-0010, first-match-wins: ADR-0009's identity reason keeps
                // priority, so the principal reason is emitted only when the
                // identity itself was binding.
                let reason = principal_block_reason(
                    &response.envelope.writer.actor_id,
                    &response.envelope.occurred_at,
                    delegation_map,
                )
                .unwrap_or("principal_unresolvable");
                diagnostics.push(TaskProjectionDiagnostic {
                    code: "agent_resumption_response_principal_not_sufficient".to_owned(),
                    message: principal_block_message(reason).to_owned(),
                    event_id: Some(response.envelope.event_id.clone()),
                    reason: Some(reason.to_owned()),
                });
            }
            return Ok(AgentResumptionProjection {
                reader_actor_id: reader_actor_id.clone(),
                task_attempt_id: task_attempt_id.clone(),
                may_resume: false,
                state: AgentResumptionState::Blocked,
                latest_checkpoint,
                selected_input_request: Some(record.view.clone()),
                selected_response: Some(response_view),
                treated_as_operative: false,
                freshness: Some(freshness),
                diagnostics,
            });
        }

        // This input request is satisfied. Track the most recent satisfied
        // response for diagnostic completeness.
        match &last_satisfied {
            Some((_, prev_response_view, _))
                if envelope_chronological_order(
                    &prev_response_view.envelope,
                    &response_view.envelope,
                )
                .is_ge() => {}
            _ => {
                last_satisfied = Some((record.view.clone(), response_view, freshness));
            }
        }
    }

    // No open, ambiguous, stale, or non-binding input request found.
    let (selected_input_request, selected_response, freshness, treated_as_operative) =
        match last_satisfied {
            Some((view, response_view, freshness)) => {
                (Some(view), Some(response_view), Some(freshness), true)
            }
            None => (None, None, None, false),
        };

    Ok(AgentResumptionProjection {
        reader_actor_id: reader_actor_id.clone(),
        task_attempt_id: task_attempt_id.clone(),
        may_resume: true,
        state: AgentResumptionState::Ready,
        latest_checkpoint,
        selected_input_request,
        selected_response,
        treated_as_operative,
        freshness,
        diagnostics: principal_advisories,
    })
}

/// The bounded principal block reason for a response writer at `occurred_at`:
/// `principal_unresolvable` / `principal_ambiguous`, or `None` when the writer is
/// its own principal or resolves cleanly. Mirrors `principal_sufficient`'s
/// require-arm but yields the diagnostic reason.
fn principal_block_reason(
    writer_actor: &ActorId,
    occurred_at: &str,
    delegation_map: Option<&DelegationMap>,
) -> Option<&'static str> {
    let Some(map) = delegation_map else {
        // No map: a human is its own principal; an agent is unresolvable.
        return is_agent_actor_id(writer_actor.as_str()).then_some("principal_unresolvable");
    };
    match principal_resolution_for_writer(writer_actor, map, occurred_at) {
        // Non-agent writer (its own principal) or a clean resolution: no block.
        None | Some(PrincipalResolution::Resolved(_)) => None,
        Some(PrincipalResolution::None(_)) => Some("principal_unresolvable"),
        Some(PrincipalResolution::Ambiguous(_)) => Some("principal_ambiguous"),
    }
}

fn principal_block_message(reason: &str) -> &'static str {
    match reason {
        "principal_ambiguous" => {
            "the responder's agent identity resolves to more than one principal; the delegation map must disambiguate before this response binds"
        }
        _ => {
            "the responder's agent identity does not resolve to a responsible principal at the response time; require-resolvable-principal will not let it bind"
        }
    }
}

#[derive(Clone, Debug)]
struct TaskInputRequestRecord {
    view: TaskInputRequestView,
    responses: Vec<TaskInputRequestResponseRecord>,
}

#[derive(Clone, Debug)]
struct TaskInputRequestResponseRecord {
    envelope: TaskProjectionEventEnvelope,
    response_id: InputRequestResponseId,
    outcome: InputRequestResponseOutcome,
    reason: Option<String>,
    reason_artifact_path: Option<String>,
    reason_byte_size: Option<u64>,
    reason_content_hash: Option<String>,
    target_fingerprint: Option<String>,
    /// Binding evidence (ADR-0009), computed at collection where the full
    /// `ShoreEvent` is in hand; consumed by the two-arm predicate.
    verification: EventVerificationStatus,
    ingested: bool,
}

fn collect_task_input_request_records(
    events: &[ShoreEvent],
    task_attempt_id: &WorkObjectId,
    trust_set: &TrustSet,
) -> Result<Vec<TaskInputRequestRecord>> {
    let mut request_views: Vec<TaskInputRequestView> = Vec::new();
    // Collapse duplicate semantic response facts by
    // `input_request_response_id`: two events with the same response id
    // are retry duplicates, not distinct responses. This mirrors the
    // representative-selection convention in
    // `src/session/workflow/input_request/view.rs`. The map value carries
    // the owning `input_request_id` so the rollup below does not need a
    // second pass over `events`.
    let mut response_representatives: BTreeMap<
        InputRequestResponseId,
        (InputRequestId, TaskInputRequestResponseRecord),
    > = BTreeMap::new();

    for event in events {
        event.validate_schema_version()?;

        match event.event_type {
            EventType::InputRequestOpened => {
                if !envelope_targets_task_attempt(&event.target, task_attempt_id) {
                    continue;
                }
                let Some(subject) = event.target.subject.clone() else {
                    continue;
                };
                if !matches!(subject, TargetRef::Task(_)) {
                    continue;
                }
                let payload = decode_input_request_opened_payload(event.payload.clone())?;
                request_views.push(TaskInputRequestView {
                    input_request_id: payload.input_request_id,
                    envelope: TaskProjectionEventEnvelope::from_event(event),
                    target: subject,
                    payload_review_target: payload.target,
                    mode: event.assertion_mode,
                    reason_code: payload.reason_code,
                    title: payload.title,
                    body: payload.body,
                    body_artifact_path: payload.body_artifact_path,
                    body_byte_size: payload.body_byte_size,
                    body_content_hash: payload.body_content_hash,
                    target_fingerprint: payload.target_fingerprint,
                });
            }
            EventType::InputRequestResponded => {
                let payload: InputRequestRespondedPayload =
                    serde_json::from_value(event.payload.clone())?;
                let input_request_id = payload.input_request_id.clone();
                let response_id = payload.input_request_response_id.clone();
                let (verification, ingested) = response_binding_evidence(event, trust_set)?;
                let record = TaskInputRequestResponseRecord {
                    envelope: TaskProjectionEventEnvelope::from_event(event),
                    response_id: response_id.clone(),
                    outcome: payload.outcome,
                    reason: payload.reason,
                    reason_artifact_path: payload.reason_artifact_path,
                    reason_byte_size: payload.reason_byte_size,
                    reason_content_hash: payload.reason_content_hash,
                    target_fingerprint: payload.target_fingerprint,
                    verification,
                    ingested,
                };
                response_representatives
                    .entry(response_id)
                    .and_modify(|(_, existing)| {
                        if record.envelope.event_id.as_str() < existing.envelope.event_id.as_str() {
                            *existing = record.clone();
                        }
                    })
                    .or_insert((input_request_id, record));
            }
            _ => {}
        }
    }

    let mut responses_by_input_request: BTreeMap<
        InputRequestId,
        Vec<TaskInputRequestResponseRecord>,
    > = BTreeMap::new();
    for (input_request_id, response) in response_representatives.into_values() {
        responses_by_input_request
            .entry(input_request_id)
            .or_default()
            .push(response);
    }

    let mut records = Vec::with_capacity(request_views.len());
    for view in request_views {
        let responses = responses_by_input_request
            .remove(&view.input_request_id)
            .unwrap_or_default();
        records.push(TaskInputRequestRecord { view, responses });
    }
    Ok(records)
}

fn freshness_for_task_target(
    view: &TaskInputRequestView,
    latest_checkpoint: Option<&CheckpointId>,
    latest_checkpoint_fingerprint: Option<&str>,
    response_fingerprint: Option<&str>,
) -> FreshnessBasis {
    // When the writer recorded a fingerprint on both sides, opaque-string
    // equality is the freshness signal -- the responder's `target_fingerprint`
    // is compared to the latest checkpoint's `checkpoint_fingerprint`. The
    // identity-based fallback below handles every other shape (either side
    // `None`).
    if let (Some(latest_fp), Some(response_fp)) =
        (latest_checkpoint_fingerprint, response_fingerprint)
    {
        return if latest_fp == response_fp {
            FreshnessBasis::CheckpointFingerprintMatches
        } else {
            FreshnessBasis::CheckpointFingerprintMismatch
        };
    }

    match &view.target {
        TargetRef::Task(TaskTargetRef::TaskAttempt) => {
            FreshnessBasis::TaskAttemptNoFingerprintCheck
        }
        TargetRef::Task(TaskTargetRef::Checkpoint { checkpoint_id }) => match latest_checkpoint {
            Some(latest) if latest == checkpoint_id => FreshnessBasis::CheckpointMatchesLatest,
            Some(_) => FreshnessBasis::CheckpointStaleNewerExists,
            None => FreshnessBasis::CheckpointWithoutAttemptCheckpoint,
        },
        TargetRef::Review(_) => FreshnessBasis::TaskAttemptNoFingerprintCheck,
    }
}

/// ADR-0009: binding(event, policy) — true iff either arm holds. Neither arm
/// reads a field the writer asserted; the claimed actorId is reported in the
/// projection but is never the basis of the decision (ADR-0007 invariant).
/// Verification `Valid` already folds in allowed-signers authorization
/// (ADR-0004); binding consults no `EventVerificationPolicy` preset.
fn response_identity_is_binding(
    verification: EventVerificationStatus,
    ingested: bool,
    policy: ResumptionBindingPolicy,
) -> bool {
    // Arm (b): verified signer.
    if verification == EventVerificationStatus::Valid {
        return true;
    }
    // Arm (a): local possession. An invalid signature is affirmative
    // evidence of tampering and defeats this arm; Unsigned and any status
    // better are accepted.
    policy.permits_local_possession()
        && !ingested
        && verification != EventVerificationStatus::Invalid
}

/// ADR-0009 diagnostics: bounded reason naming the cheapest fix, first match
/// wins. Only defined on the non-binding domain: when verification is
/// `UntrustedKey` and arm (a) is available the response binds and no
/// diagnostic is emitted at all, so the plain status match is equivalent to
/// the ADR's "and arm (a) is unavailable" qualification.
fn non_binding_reason(
    verification: EventVerificationStatus,
    ingested: bool,
    _policy: ResumptionBindingPolicy,
) -> &'static str {
    match () {
        _ if verification == EventVerificationStatus::Invalid => "signature_invalid",
        _ if verification == EventVerificationStatus::UntrustedKey => "signer_not_authorized",
        _ if ingested => "ingested_unsigned",
        _ => "policy_excludes_local",
    }
}

/// Ambiguity-honest operator message per bounded reason: each states what the
/// projection knows, never a guess about who the responder "really" was.
fn non_binding_message(reason: &str) -> &'static str {
    match reason {
        "signature_invalid" => {
            "response signature is invalid; tampering or corruption — never binds"
        }
        "signer_not_authorized" => {
            "response signature verifies but the allowed-signers trust set does not authorize this signer for the claimed actor"
        }
        "ingested_unsigned" => {
            "response was ingested without a signature; the responder must sign for this response to bind"
        }
        _ => {
            "local unsigned response under verified-only; the store's policy requires a key to bind"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
    use crate::model::{
        ActorId, CheckpointId, ObservationId, SessionId, TargetRef, TaskTargetRef, WorkObjectId,
    };
    use crate::session::event::{
        AssertionMode, EventTarget, EventType, IngestProvenance, IngestVia, ShoreEvent, SourceRef,
        TaskAttemptCapturedPayload, TaskCheckpointCapturedPayload, TaskObservationRecordedPayload,
        Writer, WriterProducer,
    };
    use crate::session::event_signature_trust_set;
    use crate::session::projection::test_support::{
        checkpoint_event, reader_actor, task_attempt_event, task_input_request_event_with_target,
        user_response_event, writer_user,
    };

    fn observation_event(
        task_attempt_id: &WorkObjectId,
        session_id: &SessionId,
        checkpoint_id: Option<&CheckpointId>,
        source_id: &str,
        title: &str,
        occurred_at: &str,
    ) -> ShoreEvent {
        let observation_id = ObservationId::new(format!(
            "obs:sha256:{}",
            sha256_bytes_hex(source_id.as_bytes())
        ));
        let mut target = EventTarget::for_work_object(
            session_id.clone(),
            task_attempt_id.clone(),
            WorkObjectType::TaskAttempt,
        );
        target.subject = Some(match checkpoint_id {
            Some(checkpoint_id) => TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: checkpoint_id.clone(),
            }),
            None => TargetRef::Task(TaskTargetRef::TaskAttempt),
        });
        let payload = TaskObservationRecordedPayload {
            observation_id: observation_id.clone(),
            checkpoint_id: checkpoint_id.cloned(),
            title: title.to_owned(),
            body: None,
            body_artifact_path: None,
            body_byte_size: None,
            body_content_hash: None,
            source_speaker: None,
        };
        let idempotency_key = TaskObservationRecordedPayload::idempotency_key_for_work_object(
            task_attempt_id,
            WorkObjectType::TaskAttempt,
            observation_id.as_str(),
        );
        let mut event = ShoreEvent::new(
            EventType::TaskObservationRecorded,
            idempotency_key,
            target,
            Writer::shore_local("test"),
            payload,
            occurred_at,
        )
        .unwrap();
        event.source_ref = Some(SourceRef::new("claude_code", source_id));
        event.assertion_mode = AssertionMode::Advisory;
        event
    }

    #[test]
    fn task_attempt_summary_rolls_up_one_attempt() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let checkpoint_a = CheckpointId::new("checkpoint:sha256:cp-a");
        let checkpoint_b = CheckpointId::new("checkpoint:sha256:cp-b");

        let events = vec![
            task_attempt_event(
                &task_attempt_id,
                &session_id,
                "uuid-1",
                "2026-05-18T00:00:00Z",
            ),
            checkpoint_event(
                &task_attempt_id,
                &session_id,
                &checkpoint_a,
                "msg_1",
                vec!["tu_1".to_owned()],
                "2026-05-18T00:00:01Z",
            ),
            checkpoint_event(
                &task_attempt_id,
                &session_id,
                &checkpoint_b,
                "msg_2",
                vec!["tu_2".to_owned()],
                "2026-05-18T00:00:03Z",
            ),
            observation_event(
                &task_attempt_id,
                &session_id,
                Some(&checkpoint_a),
                "uuid-1#tool_result:tu_1",
                "tool_result: Bash",
                "2026-05-18T00:00:02Z",
            ),
            observation_event(
                &task_attempt_id,
                &session_id,
                Some(&checkpoint_b),
                "uuid-1#tool_result:tu_2",
                "tool_result: Read",
                "2026-05-18T00:00:04Z",
            ),
        ];

        let summary = task_attempt_summary_from_events(&events, &task_attempt_id, &reader_actor())
            .unwrap()
            .expect("attempt is present");

        assert_eq!(summary.reader_actor_id, reader_actor());
        assert_eq!(summary.task_attempt_id, task_attempt_id);
        assert_eq!(summary.project_path, "/repo");
        assert_eq!(summary.claude_session_uuid, "uuid-1");
        assert_eq!(summary.initial_prompt_hash, "sha256:prompt");
        assert_eq!(summary.predecessor, None);
        assert_eq!(summary.checkpoints.len(), 2);
        assert_eq!(
            summary
                .latest_checkpoint
                .as_ref()
                .map(|cp| cp.checkpoint_id.clone()),
            Some(checkpoint_b.clone())
        );

        let cp_a = summary
            .checkpoints
            .iter()
            .find(|cp| cp.checkpoint_id == checkpoint_a)
            .expect("checkpoint a present");
        assert_eq!(cp_a.observations.len(), 1);
        assert_eq!(cp_a.observations[0].title, "tool_result: Bash");

        let cp_b = summary
            .checkpoints
            .iter()
            .find(|cp| cp.checkpoint_id == checkpoint_b)
            .expect("checkpoint b present");
        assert_eq!(cp_b.observations.len(), 1);
        assert_eq!(cp_b.observations[0].title, "tool_result: Read");

        assert!(summary.observations_without_checkpoint.is_empty());
        assert!(summary.diagnostics.is_empty());
    }

    #[test]
    fn task_attempt_summary_ignores_other_task_attempts() {
        let attempt_a = WorkObjectId::new("task-attempt:sha256:a");
        let attempt_b = WorkObjectId::new("task-attempt:sha256:b");
        let session_a = SessionId::new("session:claude:uuid-a");
        let session_b = SessionId::new("session:claude:uuid-b");
        let checkpoint_a = CheckpointId::new("checkpoint:sha256:cp-a");
        let checkpoint_b = CheckpointId::new("checkpoint:sha256:cp-b");

        let events = vec![
            task_attempt_event(&attempt_a, &session_a, "uuid-a", "2026-05-18T00:00:00Z"),
            task_attempt_event(&attempt_b, &session_b, "uuid-b", "2026-05-18T00:00:00Z"),
            checkpoint_event(
                &attempt_a,
                &session_a,
                &checkpoint_a,
                "msg_a1",
                vec![],
                "2026-05-18T00:00:01Z",
            ),
            checkpoint_event(
                &attempt_b,
                &session_b,
                &checkpoint_b,
                "msg_b1",
                vec![],
                "2026-05-18T00:00:01Z",
            ),
            observation_event(
                &attempt_a,
                &session_a,
                Some(&checkpoint_a),
                "uuid-a#tool_result:1",
                "obs-a",
                "2026-05-18T00:00:02Z",
            ),
            observation_event(
                &attempt_b,
                &session_b,
                Some(&checkpoint_b),
                "uuid-b#tool_result:1",
                "obs-b",
                "2026-05-18T00:00:02Z",
            ),
        ];

        let summary = task_attempt_summary_from_events(&events, &attempt_a, &reader_actor())
            .unwrap()
            .expect("attempt a is present");

        assert_eq!(summary.task_attempt_id, attempt_a);
        assert_eq!(summary.checkpoints.len(), 1);
        assert_eq!(summary.checkpoints[0].checkpoint_id, checkpoint_a);
        assert_eq!(summary.checkpoints[0].observations.len(), 1);
        assert_eq!(summary.checkpoints[0].observations[0].title, "obs-a");

        for cp in &summary.checkpoints {
            assert_ne!(cp.checkpoint_id, checkpoint_b);
            for obs in &cp.observations {
                assert_ne!(obs.title, "obs-b");
            }
        }
    }

    #[test]
    fn task_attempt_summary_preserves_envelope_and_payload_fields() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let checkpoint = CheckpointId::new("checkpoint:sha256:cp");

        let attempt = task_attempt_event(
            &task_attempt_id,
            &session_id,
            "uuid-1",
            "2026-05-18T00:00:00Z",
        );
        let checkpoint_event_ev = checkpoint_event(
            &task_attempt_id,
            &session_id,
            &checkpoint,
            "msg_1",
            vec!["tu_1".to_owned()],
            "2026-05-18T00:00:01Z",
        );
        let observation = observation_event(
            &task_attempt_id,
            &session_id,
            Some(&checkpoint),
            "uuid-1#tool_result:tu_1",
            "tool_result: Bash",
            "2026-05-18T00:00:02Z",
        );

        let events = vec![
            attempt.clone(),
            checkpoint_event_ev.clone(),
            observation.clone(),
        ];
        let summary = task_attempt_summary_from_events(&events, &task_attempt_id, &reader_actor())
            .unwrap()
            .expect("attempt present");

        let env = &summary.attempt_event;
        assert_eq!(env.event_id, attempt.event_id);
        assert_eq!(env.event_type, EventType::TaskAttemptCaptured);
        assert_eq!(env.occurred_at, attempt.occurred_at);
        assert_eq!(env.payload_hash, attempt.payload_hash);
        assert_eq!(env.writer, attempt.writer);
        assert_eq!(env.assertion_mode, AssertionMode::Advisory);
        assert_eq!(env.source_ref, attempt.source_ref);
        assert_eq!(env.target, attempt.target);

        let cp = summary.checkpoints.first().expect("checkpoint present");
        assert_eq!(cp.envelope.event_id, checkpoint_event_ev.event_id);
        assert_eq!(cp.envelope.event_type, EventType::TaskCheckpointCaptured);
        assert_eq!(cp.envelope.payload_hash, checkpoint_event_ev.payload_hash);
        assert_eq!(cp.envelope.writer, checkpoint_event_ev.writer);
        assert_eq!(cp.envelope.source_ref, checkpoint_event_ev.source_ref);
        assert_eq!(cp.envelope.target, checkpoint_event_ev.target);
        assert_eq!(cp.assistant_message_id, "msg_1");
        assert_eq!(cp.tool_use_ids, vec!["tu_1".to_owned()]);

        let obs = cp.observations.first().expect("observation present");
        assert_eq!(obs.envelope.event_id, observation.event_id);
        assert_eq!(obs.envelope.event_type, EventType::TaskObservationRecorded);
        assert_eq!(obs.envelope.payload_hash, observation.payload_hash);
        assert_eq!(obs.envelope.writer, observation.writer);
        assert_eq!(obs.envelope.source_ref, observation.source_ref);
        assert_eq!(obs.envelope.target, observation.target);
        assert_eq!(obs.title, "tool_result: Bash");
        assert_eq!(obs.checkpoint_id.as_ref(), Some(&checkpoint));
        assert_eq!(obs.body, None);
        assert_eq!(obs.body_artifact_path, None);
        assert_eq!(obs.body_byte_size, None);
        assert_eq!(obs.body_content_hash, None);
    }

    #[test]
    fn task_attempt_summary_orders_latest_checkpoint_and_recent_observations() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let cp_early = CheckpointId::new("checkpoint:sha256:cp-early");
        let cp_late = CheckpointId::new("checkpoint:sha256:cp-late");

        // Feed events out of chronological order.
        let events = vec![
            observation_event(
                &task_attempt_id,
                &session_id,
                Some(&cp_late),
                "uuid-1#tool_result:later",
                "later observation",
                "2026-05-18T00:00:05Z",
            ),
            checkpoint_event(
                &task_attempt_id,
                &session_id,
                &cp_late,
                "msg_late",
                vec![],
                "2026-05-18T00:00:04Z",
            ),
            observation_event(
                &task_attempt_id,
                &session_id,
                Some(&cp_late),
                "uuid-1#tool_result:earlier-under-late",
                "earlier observation under late checkpoint",
                "2026-05-18T00:00:03Z",
            ),
            checkpoint_event(
                &task_attempt_id,
                &session_id,
                &cp_early,
                "msg_early",
                vec![],
                "2026-05-18T00:00:01Z",
            ),
            task_attempt_event(
                &task_attempt_id,
                &session_id,
                "uuid-1",
                "2026-05-18T00:00:00Z",
            ),
        ];

        let summary = task_attempt_summary_from_events(&events, &task_attempt_id, &reader_actor())
            .unwrap()
            .expect("attempt present");

        assert_eq!(
            summary
                .latest_checkpoint
                .as_ref()
                .map(|cp| cp.checkpoint_id.clone()),
            Some(cp_late.clone()),
            "latest_checkpoint is the highest occurred_at checkpoint"
        );

        let cp_late_summary = summary
            .checkpoints
            .iter()
            .find(|cp| cp.checkpoint_id == cp_late)
            .expect("late checkpoint present");
        let titles: Vec<&str> = cp_late_summary
            .observations
            .iter()
            .map(|obs| obs.title.as_str())
            .collect();
        assert_eq!(
            titles,
            vec![
                "later observation",
                "earlier observation under late checkpoint",
            ],
            "observations sort by occurred_at descending"
        );
    }

    #[test]
    fn task_attempt_summary_does_not_depend_on_adapter_intents() {
        // Build inputs as already-written ShoreEvents only. This is a documentation
        // pin: the projection lives downstream of the write seam.
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");

        let events = vec![task_attempt_event(
            &task_attempt_id,
            &session_id,
            "uuid-1",
            "2026-05-18T00:00:00Z",
        )];

        let summary = task_attempt_summary_from_events(&events, &task_attempt_id, &reader_actor())
            .unwrap()
            .expect("attempt present");

        assert_eq!(summary.task_attempt_id, task_attempt_id);
        assert!(summary.checkpoints.is_empty());
        assert!(summary.observations_without_checkpoint.is_empty());
        assert!(summary.latest_checkpoint.is_none());
    }

    #[test]
    fn task_attempt_summary_returns_none_when_attempt_not_present() {
        let other_attempt = WorkObjectId::new("task-attempt:sha256:other");
        let queried_attempt = WorkObjectId::new("task-attempt:sha256:queried");
        let session_id = SessionId::new("session:claude:uuid-other");

        let events = vec![task_attempt_event(
            &other_attempt,
            &session_id,
            "uuid-other",
            "2026-05-18T00:00:00Z",
        )];

        let summary =
            task_attempt_summary_from_events(&events, &queried_attempt, &reader_actor()).unwrap();
        assert!(summary.is_none());
    }

    // -- open_task_input_requests -------------------------------------------

    use crate::model::{
        InputRequestId, InputRequestResponseId, ReviewTargetRef, ReviewUnitId, TrackId,
    };
    use crate::session::event::{
        InputRequestOpenedPayload, InputRequestReasonCode, InputRequestRespondedPayload,
        InputRequestResponseOutcome,
    };

    #[allow(clippy::too_many_arguments)]
    fn task_input_request_event(
        task_attempt_id: &WorkObjectId,
        session_id: &SessionId,
        input_request_id: &InputRequestId,
        source_key: &str,
        occurred_at: &str,
        assertion_mode: AssertionMode,
        reason_code: InputRequestReasonCode,
        title: &str,
    ) -> ShoreEvent {
        let mut target = EventTarget::for_work_object(
            session_id.clone(),
            task_attempt_id.clone(),
            WorkObjectType::TaskAttempt,
        );
        target.subject = Some(TargetRef::Task(TaskTargetRef::TaskAttempt));
        // Current `InputRequestOpenedPayload.target` is review-shaped. The
        // task envelope is authoritative; this placeholder is preserved by the
        // projection only as diagnostic evidence.
        let payload = InputRequestOpenedPayload {
            input_request_id: input_request_id.clone(),
            target: ReviewTargetRef::ReviewUnit {
                review_unit_id: ReviewUnitId::new("review-unit:placeholder"),
            },
            reason_code,
            title: title.to_owned(),
            body: None,
            body_artifact_path: None,
            body_byte_size: None,
            body_content_hash: None,
            target_fingerprint: None,
        };
        let idempotency_key = InputRequestOpenedPayload::idempotency_key_for_work_object(
            task_attempt_id,
            WorkObjectType::TaskAttempt,
            source_key,
        );
        let mut event = ShoreEvent::new(
            EventType::InputRequestOpened,
            idempotency_key,
            target,
            Writer::shore_local("test"),
            payload,
            occurred_at,
        )
        .unwrap();
        event.source_ref = Some(SourceRef::new("claude_code", source_key));
        event.assertion_mode = assertion_mode;
        event
    }

    fn task_input_request_responded_event(
        task_attempt_id: &WorkObjectId,
        session_id: &SessionId,
        input_request_id: &InputRequestId,
        response_id: &InputRequestResponseId,
        occurred_at: &str,
    ) -> ShoreEvent {
        let target = EventTarget::for_work_object(
            session_id.clone(),
            task_attempt_id.clone(),
            WorkObjectType::TaskAttempt,
        );
        let payload = InputRequestRespondedPayload {
            input_request_response_id: response_id.clone(),
            input_request_id: input_request_id.clone(),
            outcome: InputRequestResponseOutcome::Approved,
            reason: None,
            reason_artifact_path: None,
            reason_byte_size: None,
            reason_content_hash: None,
            target_fingerprint: None,
        };
        let idempotency_key =
            InputRequestRespondedPayload::idempotency_key(input_request_id, response_id.as_str());
        ShoreEvent::new(
            EventType::InputRequestResponded,
            idempotency_key,
            target,
            Writer::shore_local("test"),
            payload,
            occurred_at,
        )
        .unwrap()
    }

    fn review_input_request_event(
        review_unit_id: &ReviewUnitId,
        track_id: &TrackId,
        input_request_id: &InputRequestId,
        source_key: &str,
        occurred_at: &str,
    ) -> ShoreEvent {
        let target = EventTarget {
            session_id: SessionId::new("session:review"),
            work_unit_id: None,
            work_object_id: None,
            work_object_type: None,
            review_unit_id: Some(review_unit_id.clone()),
            revision_id: None,
            snapshot_id: None,
            track_id: Some(track_id.clone()),
            subject: Some(TargetRef::Review(ReviewTargetRef::ReviewUnit {
                review_unit_id: review_unit_id.clone(),
            })),
        };
        let payload = InputRequestOpenedPayload {
            input_request_id: input_request_id.clone(),
            target: ReviewTargetRef::ReviewUnit {
                review_unit_id: review_unit_id.clone(),
            },
            reason_code: InputRequestReasonCode::ManualDecisionRequired,
            title: "review-domain".to_owned(),
            body: None,
            body_artifact_path: None,
            body_byte_size: None,
            body_content_hash: None,
            target_fingerprint: None,
        };
        let idempotency_key =
            InputRequestOpenedPayload::idempotency_key(review_unit_id, track_id, source_key);
        ShoreEvent::new(
            EventType::InputRequestOpened,
            idempotency_key,
            target,
            Writer::shore_local("test"),
            payload,
            occurred_at,
        )
        .unwrap()
        .with_assertion_mode(AssertionMode::Operative)
    }

    #[test]
    fn open_task_input_requests_returns_task_targeted_unresponded_requests() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let input_request_id = InputRequestId::new("input-request:sha256:1");

        let events = vec![task_input_request_event(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:1",
            "2026-05-18T00:00:00Z",
            AssertionMode::Operative,
            InputRequestReasonCode::ManualDecisionRequired,
            "Need a call",
        )];

        let projection =
            open_task_input_requests_from_events(&events, &task_attempt_id, &reader_actor())
                .unwrap();

        assert_eq!(projection.task_attempt_id, task_attempt_id);
        assert_eq!(projection.reader_actor_id, reader_actor());
        assert_eq!(projection.open_input_requests.len(), 1);
        let view = &projection.open_input_requests[0];
        assert_eq!(view.input_request_id, input_request_id);
        assert_eq!(
            view.target,
            TargetRef::Task(TaskTargetRef::TaskAttempt),
            "task target is read from the durable envelope, not the payload"
        );
        // The payload placeholder is preserved verbatim as evidence.
        assert_eq!(
            view.payload_review_target,
            ReviewTargetRef::ReviewUnit {
                review_unit_id: ReviewUnitId::new("review-unit:placeholder"),
            }
        );
    }

    #[test]
    fn open_task_input_requests_excludes_responded_input_request_ids() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let input_request_id = InputRequestId::new("input-request:sha256:1");
        let response_id = InputRequestResponseId::new("input-request-response:sha256:r1");

        let events = vec![
            task_input_request_event(
                &task_attempt_id,
                &session_id,
                &input_request_id,
                "source:1",
                "2026-05-18T00:00:00Z",
                AssertionMode::Operative,
                InputRequestReasonCode::ManualDecisionRequired,
                "Need a call",
            ),
            task_input_request_responded_event(
                &task_attempt_id,
                &session_id,
                &input_request_id,
                &response_id,
                "2026-05-18T00:00:01Z",
            ),
        ];

        let projection =
            open_task_input_requests_from_events(&events, &task_attempt_id, &reader_actor())
                .unwrap();

        assert!(
            projection.open_input_requests.is_empty(),
            "responded input request must not appear in open set; got {:?}",
            projection.open_input_requests
        );
    }

    #[test]
    fn open_task_input_requests_ignores_review_domain_requests() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let review_unit_id = ReviewUnitId::new("review-unit:sha256:u");
        let track_id = TrackId::new("agent:codex");
        let task_input_request_id = InputRequestId::new("input-request:sha256:task");
        let review_input_request_id = InputRequestId::new("input-request:sha256:review");

        let events = vec![
            task_input_request_event(
                &task_attempt_id,
                &SessionId::new("session:claude:uuid-1"),
                &task_input_request_id,
                "source:task",
                "2026-05-18T00:00:00Z",
                AssertionMode::Advisory,
                InputRequestReasonCode::FailedGate,
                "task-domain",
            ),
            review_input_request_event(
                &review_unit_id,
                &track_id,
                &review_input_request_id,
                "source:review",
                "2026-05-18T00:00:00Z",
            ),
        ];

        let projection =
            open_task_input_requests_from_events(&events, &task_attempt_id, &reader_actor())
                .unwrap();

        assert_eq!(projection.open_input_requests.len(), 1);
        assert_eq!(
            projection.open_input_requests[0].input_request_id,
            task_input_request_id
        );
        for view in &projection.open_input_requests {
            assert_ne!(view.input_request_id, review_input_request_id);
        }
    }

    #[test]
    fn open_task_input_requests_preserves_payload_target_mismatch_as_diagnostic() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let input_request_id = InputRequestId::new("input-request:sha256:1");

        let events = vec![task_input_request_event(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:1",
            "2026-05-18T00:00:00Z",
            AssertionMode::Operative,
            InputRequestReasonCode::ManualDecisionRequired,
            "Need a call",
        )];

        let projection =
            open_task_input_requests_from_events(&events, &task_attempt_id, &reader_actor())
                .unwrap();

        assert_eq!(projection.open_input_requests.len(), 1);
        let diag = projection
            .diagnostics
            .iter()
            .find(|d| d.code == "task_input_request_payload_target_is_review_shaped")
            .expect("payload target mismatch diagnostic is emitted");
        assert_eq!(
            diag.event_id,
            Some(projection.open_input_requests[0].envelope.event_id.clone())
        );
    }

    #[test]
    fn open_task_input_requests_preserves_envelope_policy_fields() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let input_request_id = InputRequestId::new("input-request:sha256:1");

        let request_event = task_input_request_event(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:1",
            "2026-05-18T00:00:00Z",
            AssertionMode::Operative,
            InputRequestReasonCode::ManualDecisionRequired,
            "Need a call",
        );

        let events = vec![request_event.clone()];

        let projection =
            open_task_input_requests_from_events(&events, &task_attempt_id, &reader_actor())
                .unwrap();

        let view = &projection.open_input_requests[0];
        assert_eq!(view.envelope.event_id, request_event.event_id);
        assert_eq!(view.envelope.event_type, EventType::InputRequestOpened);
        assert_eq!(view.envelope.occurred_at, request_event.occurred_at);
        assert_eq!(view.envelope.payload_hash, request_event.payload_hash);
        assert_eq!(view.envelope.writer, request_event.writer);
        assert_eq!(view.envelope.assertion_mode, AssertionMode::Operative);
        assert_eq!(view.envelope.source_ref, request_event.source_ref);
        assert_eq!(view.envelope.target, request_event.target);
        assert_eq!(view.mode, AssertionMode::Operative);
        assert_eq!(
            view.reason_code,
            InputRequestReasonCode::ManualDecisionRequired
        );
        assert_eq!(view.title, "Need a call");
        assert_eq!(view.body, None);
        assert_eq!(view.body_artifact_path, None);
        assert_eq!(view.body_byte_size, None);
        assert_eq!(view.body_content_hash, None);
    }

    #[test]
    fn open_task_input_requests_separates_multiple_open_requests() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let a = InputRequestId::new("input-request:sha256:a");
        let b = InputRequestId::new("input-request:sha256:b");

        let events = vec![
            task_input_request_event(
                &task_attempt_id,
                &session_id,
                &a,
                "source:a",
                "2026-05-18T00:00:00Z",
                AssertionMode::Advisory,
                InputRequestReasonCode::FailedGate,
                "first",
            ),
            task_input_request_event(
                &task_attempt_id,
                &session_id,
                &b,
                "source:b",
                "2026-05-18T00:00:01Z",
                AssertionMode::Operative,
                InputRequestReasonCode::ManualDecisionRequired,
                "second",
            ),
        ];

        let projection =
            open_task_input_requests_from_events(&events, &task_attempt_id, &reader_actor())
                .unwrap();

        let ids: Vec<InputRequestId> = projection
            .open_input_requests
            .iter()
            .map(|view| view.input_request_id.clone())
            .collect();
        assert!(ids.contains(&a), "first input request should be present");
        assert!(ids.contains(&b), "second input request should be present");
        assert_eq!(ids.len(), 2, "no collapse by target");
    }

    #[test]
    fn open_task_input_requests_derives_mode_from_envelope_assertion_mode() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let advisory_id = InputRequestId::new("input-request:sha256:adv");
        let operative_id = InputRequestId::new("input-request:sha256:op");

        let advisory = task_input_request_event(
            &task_attempt_id,
            &session_id,
            &advisory_id,
            "source:adv",
            "2026-05-18T00:00:00Z",
            AssertionMode::Advisory,
            InputRequestReasonCode::FailedGate,
            "heads up",
        );
        let operative = task_input_request_event(
            &task_attempt_id,
            &session_id,
            &operative_id,
            "source:op",
            "2026-05-18T00:00:01Z",
            AssertionMode::Operative,
            InputRequestReasonCode::ManualDecisionRequired,
            "needs decision",
        );

        let projection = open_task_input_requests_from_events(
            &[advisory, operative],
            &task_attempt_id,
            &reader_actor(),
        )
        .unwrap();

        assert_eq!(projection.open_input_requests.len(), 2);
        assert_eq!(
            projection.open_input_requests[0].mode,
            AssertionMode::Advisory
        );
        assert_eq!(
            projection.open_input_requests[1].mode,
            AssertionMode::Operative
        );
    }

    // -- agent_resumption --------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn task_input_request_event_with_target_and_assertion_mode(
        task_attempt_id: &WorkObjectId,
        session_id: &SessionId,
        input_request_id: &InputRequestId,
        source_key: &str,
        occurred_at: &str,
        subject: TargetRef,
        title: &str,
        assertion_mode: AssertionMode,
    ) -> ShoreEvent {
        let mut event = task_input_request_event_with_target(
            task_attempt_id,
            session_id,
            input_request_id,
            source_key,
            occurred_at,
            subject,
            title,
        );
        event.assertion_mode = assertion_mode;
        event
    }

    fn attempt_with_checkpoints(
        task_attempt_id: &WorkObjectId,
        session_id: &SessionId,
        checkpoints: &[(&CheckpointId, &str, &str)],
    ) -> Vec<ShoreEvent> {
        let mut events = vec![task_attempt_event(
            task_attempt_id,
            session_id,
            "uuid-1",
            "2026-05-18T00:00:00Z",
        )];
        for (checkpoint_id, message_id, occurred_at) in checkpoints {
            events.push(checkpoint_event(
                task_attempt_id,
                session_id,
                checkpoint_id,
                message_id,
                vec![],
                occurred_at,
            ));
        }
        events
    }

    #[test]
    fn agent_resumption_allows_resume_when_no_task_input_requests_exist() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let checkpoint = CheckpointId::new("checkpoint:sha256:cp");

        let events = attempt_with_checkpoints(
            &task_attempt_id,
            &session_id,
            &[(&checkpoint, "msg_1", "2026-05-18T00:00:01Z")],
        );

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();

        assert!(projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ready);
        assert_eq!(projection.latest_checkpoint, Some(checkpoint));
        assert!(projection.selected_input_request.is_none());
        assert!(projection.selected_response.is_none());
    }

    #[test]
    fn agent_resumption_ignores_open_advisory_task_input_request() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let input_request_id = InputRequestId::new("input-request:sha256:adv");
        let mut events = attempt_with_checkpoints(&task_attempt_id, &session_id, &[]);
        events.push(task_input_request_event_with_target_and_assertion_mode(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:adv",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::TaskAttempt),
            "heads up",
            AssertionMode::Advisory,
        ));

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();

        assert!(projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ready);
        assert!(projection.selected_input_request.is_none());
        assert!(projection.selected_response.is_none());
    }

    #[test]
    fn agent_resumption_ignores_ambiguous_advisory_task_input_request_responses() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let input_request_id = InputRequestId::new("input-request:sha256:adv");
        let r1 = InputRequestResponseId::new("input-request-response:sha256:r1");
        let r2 = InputRequestResponseId::new("input-request-response:sha256:r2");
        let mut events = attempt_with_checkpoints(&task_attempt_id, &session_id, &[]);
        events.push(task_input_request_event_with_target_and_assertion_mode(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:ambig-adv",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::TaskAttempt),
            "heads up",
            AssertionMode::Advisory,
        ));
        events.push(user_response_event(
            &input_request_id,
            &r1,
            InputRequestResponseOutcome::Approved,
            AssertionMode::Operative,
            "2026-05-18T00:00:03Z",
        ));
        events.push(user_response_event(
            &input_request_id,
            &r2,
            InputRequestResponseOutcome::Rejected,
            AssertionMode::Operative,
            "2026-05-18T00:00:04Z",
        ));

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();

        assert!(projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ready);
        assert!(projection.selected_input_request.is_none());
        assert_ne!(projection.state, AgentResumptionState::Ambiguous);
    }

    #[test]
    fn agent_resumption_ignores_stale_advisory_task_input_request_response() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let cp_a = CheckpointId::new("checkpoint:sha256:cp-a");
        let cp_b = CheckpointId::new("checkpoint:sha256:cp-b");
        let input_request_id = InputRequestId::new("input-request:sha256:adv");
        let response_id = InputRequestResponseId::new("input-request-response:sha256:r");

        let mut events = attempt_with_checkpoints(
            &task_attempt_id,
            &session_id,
            &[
                (&cp_a, "msg_a", "2026-05-18T00:00:01Z"),
                (&cp_b, "msg_b", "2026-05-18T00:00:05Z"),
            ],
        );
        events.push(task_input_request_event_with_target_and_assertion_mode(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:stale-adv",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: cp_a.clone(),
            }),
            "heads up",
            AssertionMode::Advisory,
        ));
        events.push(user_response_event(
            &input_request_id,
            &response_id,
            InputRequestResponseOutcome::Approved,
            AssertionMode::Operative,
            "2026-05-18T00:00:03Z",
        ));

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();

        assert!(projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ready);
        assert!(projection.selected_input_request.is_none());
        assert_ne!(projection.state, AgentResumptionState::Stale);
    }

    #[test]
    fn agent_resumption_still_blocks_for_open_operative_task_input_request() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let input_request_id = InputRequestId::new("input-request:sha256:op");
        let mut events = attempt_with_checkpoints(&task_attempt_id, &session_id, &[]);
        events.push(task_input_request_event_with_target_and_assertion_mode(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:op",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::TaskAttempt),
            "needs decision",
            AssertionMode::Operative,
        ));

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();

        assert!(!projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Blocked);
        let selected = projection
            .selected_input_request
            .as_ref()
            .expect("selected input request");
        assert_eq!(selected.input_request_id, input_request_id);
        assert_eq!(selected.mode, AssertionMode::Operative);
    }

    #[test]
    fn agent_resumption_pauses_for_open_task_input_request() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let checkpoint = CheckpointId::new("checkpoint:sha256:cp");
        let input_request_id = InputRequestId::new("input-request:sha256:1");

        let mut events = attempt_with_checkpoints(
            &task_attempt_id,
            &session_id,
            &[(&checkpoint, "msg_1", "2026-05-18T00:00:01Z")],
        );
        events.push(task_input_request_event_with_target(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:open",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::TaskAttempt),
            "open call",
        ));

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();

        assert!(!projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Blocked);
        let selected = projection
            .selected_input_request
            .as_ref()
            .expect("selected input request");
        assert_eq!(selected.input_request_id, input_request_id);
        assert!(projection.selected_response.is_none());
        assert!(
            projection
                .diagnostics
                .iter()
                .any(|d| d.code == "agent_resumption_open_task_input_request"),
            "diagnostic explains the open input request"
        );
    }

    /// The headline fixture: a fresh operative Approved response from the
    /// local claude_code user actor, unsigned and unstamped. The last event
    /// is the response, for tests that need to mutate it.
    fn approved_local_response_events() -> (Vec<ShoreEvent>, WorkObjectId) {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let checkpoint = CheckpointId::new("checkpoint:sha256:cp");
        let input_request_id = InputRequestId::new("input-request:sha256:1");
        let response_id = InputRequestResponseId::new("input-request-response:sha256:r");

        let mut events = attempt_with_checkpoints(
            &task_attempt_id,
            &session_id,
            &[(&checkpoint, "msg_1", "2026-05-18T00:00:01Z")],
        );
        events.push(task_input_request_event_with_target(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:approve",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: checkpoint.clone(),
            }),
            "needs approval",
        ));
        events.push(user_response_event(
            &input_request_id,
            &response_id,
            InputRequestResponseOutcome::Approved,
            AssertionMode::Operative,
            "2026-05-18T00:00:03Z",
        ));
        (events, task_attempt_id)
    }

    #[test]
    fn local_unsigned_operative_approved_response_binds_with_zero_configuration() {
        // The deny-all discharge (ADR-0009): the exact fixture that asserted
        // Blocked while the interim predicate was constant false now projects
        // Ready under the default policy — zero keys, zero configuration.
        let (events, task_attempt_id) = approved_local_response_events();

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();

        assert!(projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ready);
        let response_view = projection
            .selected_response
            .as_ref()
            .expect("selected response");
        assert!(response_view.identity_treated_as_binding);
        assert!(projection.treated_as_operative);
        assert!(
            projection
                .diagnostics
                .iter()
                .all(|d| d.code != "agent_resumption_response_identity_not_binding"),
            "no identity diagnostic for a binding response; got {:?}",
            projection.diagnostics
        );
    }

    #[test]
    fn verified_only_policy_blocks_local_unsigned_response() {
        // verified-only: nothing binds without a key — including the store's
        // own unsigned responses.
        let (events, task_attempt_id) = approved_local_response_events();

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::VerifiedOnly,
        )
        .unwrap();

        assert!(!projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Blocked);
        let response_view = projection
            .selected_response
            .as_ref()
            .expect("selected response");
        assert!(!response_view.identity_treated_as_binding);
    }

    #[test]
    fn ingested_unsigned_response_does_not_bind_even_under_default_policy() {
        let (mut events, task_attempt_id) = approved_local_response_events();
        let response = events.last_mut().expect("response event");
        response.ingest = Some(IngestProvenance {
            via: IngestVia::IngestEvents,
            received_at: "unix-ms:1".to_owned(),
        });

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();

        assert!(!projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Blocked);
        let response_view = projection
            .selected_response
            .as_ref()
            .expect("selected response");
        assert!(!response_view.identity_treated_as_binding);
    }

    #[test]
    fn invalid_signature_defeats_local_possession_arm() {
        // An invalid signature is affirmative evidence of tampering: a local
        // (unstamped) response carrying one never binds, even under the
        // default policy.
        let (mut events, task_attempt_id) = approved_local_response_events();
        let response = events.last_mut().expect("response event");
        response.signer = Some(
            crate::crypto::SignerId::parse(
                "did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd",
            )
            .unwrap(),
        );
        response.signature = Some(
            crate::session::event::EventSignature::new_ed25519_v1(
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==",
            )
            .unwrap(),
        );

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();

        assert!(!projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Blocked);
        let response_view = projection
            .selected_response
            .as_ref()
            .expect("selected response");
        assert!(!response_view.identity_treated_as_binding);
    }

    /// Signs a response event in place with a seeded Ed25519 key. The signer
    /// is not in any trust set, so verification yields `UntrustedKey` against
    /// `TrustSet::default()`.
    fn sign_event_with_seeded_key(event: &mut ShoreEvent) {
        crate::session::sign_event_if_requested(
            event,
            &crate::session::EventSigningOptions::sign_with(seeded_signer()),
        )
        .unwrap();
    }

    fn seeded_signer() -> crate::session::signing::test_support::DeterministicSigner {
        crate::session::signing::test_support::DeterministicSigner::from_seed([7u8; 32])
    }

    fn identity_not_binding_diagnostic(
        projection: &AgentResumptionProjection,
    ) -> &TaskProjectionDiagnostic {
        projection
            .diagnostics
            .iter()
            .find(|d| d.code == "agent_resumption_response_identity_not_binding")
            .expect("identity-not-binding diagnostic emitted")
    }

    /// Trust set authorizing the seeded signer for the fixture response actor.
    fn authorizing_trust_set() -> TrustSet {
        crate::session::signing::test_support::trust_for_actor(
            &ActorId::new("actor:claude_code:user"),
            &seeded_signer(),
        )
    }

    // The linked-read fixtures in workflow/ingest.rs (the linked_read_* and
    // authoring_worktree_* tests) extend this matrix through a real worktree
    // pair, `store link`, and the read seam — keep the two in step.
    #[test]
    fn binding_outcome_matrix_status_by_ingest_by_policy() {
        use EventVerificationStatus::{Invalid, Unsigned, UntrustedKey, Valid};
        use ResumptionBindingPolicy::{LocalAndVerified, VerifiedOnly};
        // (status, ingested, policy, binds, blocked reason)
        let cases: [(
            EventVerificationStatus,
            bool,
            ResumptionBindingPolicy,
            bool,
            Option<&str>,
        ); 16] = [
            (Valid, false, LocalAndVerified, true, None),
            (Valid, false, VerifiedOnly, true, None),
            (Valid, true, LocalAndVerified, true, None),
            (Valid, true, VerifiedOnly, true, None),
            (Unsigned, false, LocalAndVerified, true, None),
            (
                Unsigned,
                false,
                VerifiedOnly,
                false,
                Some("policy_excludes_local"),
            ),
            (
                Unsigned,
                true,
                LocalAndVerified,
                false,
                Some("ingested_unsigned"),
            ),
            (
                Unsigned,
                true,
                VerifiedOnly,
                false,
                Some("ingested_unsigned"),
            ),
            (UntrustedKey, false, LocalAndVerified, true, None),
            (
                UntrustedKey,
                false,
                VerifiedOnly,
                false,
                Some("signer_not_authorized"),
            ),
            (
                UntrustedKey,
                true,
                LocalAndVerified,
                false,
                Some("signer_not_authorized"),
            ),
            (
                UntrustedKey,
                true,
                VerifiedOnly,
                false,
                Some("signer_not_authorized"),
            ),
            (
                Invalid,
                false,
                LocalAndVerified,
                false,
                Some("signature_invalid"),
            ),
            (
                Invalid,
                false,
                VerifiedOnly,
                false,
                Some("signature_invalid"),
            ),
            (
                Invalid,
                true,
                LocalAndVerified,
                false,
                Some("signature_invalid"),
            ),
            (
                Invalid,
                true,
                VerifiedOnly,
                false,
                Some("signature_invalid"),
            ),
        ];

        for (status, ingested, policy, binds, reason) in cases {
            let (mut events, task_attempt_id) = approved_local_response_events();
            let response = events.last_mut().expect("response event");
            // Shape the response to produce the cell's verification status
            // under the cell's trust set, using the production signing
            // machinery so the fixture cannot drift from the verifier.
            let trust = match status {
                Valid => {
                    sign_event_with_seeded_key(response);
                    authorizing_trust_set()
                }
                UntrustedKey => {
                    sign_event_with_seeded_key(response);
                    TrustSet::default()
                }
                Unsigned => TrustSet::default(),
                Invalid => {
                    sign_event_with_seeded_key(response);
                    response.payload["tamperedAfterSigning"] = serde_json::json!(true);
                    response.payload_hash = sha256_json_prefixed(&response.payload).unwrap();
                    authorizing_trust_set()
                }
            };
            if ingested {
                response.ingest = Some(IngestProvenance {
                    via: IngestVia::IngestEvents,
                    received_at: "unix-ms:1760000000000".to_owned(),
                });
            }

            let projection = agent_resumption_from_events(
                &events,
                &task_attempt_id,
                &reader_actor(),
                &trust,
                policy,
            )
            .unwrap();

            let cell = format!("({status:?}, ingested: {ingested}, {policy:?})");
            let response_view = projection
                .selected_response
                .as_ref()
                .unwrap_or_else(|| panic!("selected response surfaced for {cell}"));
            if binds {
                assert!(projection.may_resume, "may_resume for {cell}");
                assert_eq!(projection.state, AgentResumptionState::Ready, "{cell}");
                assert!(response_view.identity_treated_as_binding, "{cell}");
            } else {
                assert!(!projection.may_resume, "no resume for {cell}");
                assert_eq!(projection.state, AgentResumptionState::Blocked, "{cell}");
                assert!(!response_view.identity_treated_as_binding, "{cell}");
                let diagnostic = identity_not_binding_diagnostic(&projection);
                assert_eq!(diagnostic.reason.as_deref(), reason, "{cell}");
            }
        }
    }

    #[test]
    fn binding_never_reads_claimed_actor_or_verification_policy_preset() {
        // Two unsigned local responses differing only in writer.actor_id
        // project identically: the claimed actor is reported, never decided
        // on. (There is no EventVerificationPolicy input to the projection at
        // all — pinned by the API shape itself.)
        let (events, task_attempt_id) = approved_local_response_events();
        let mut other_actor_events = events.clone();
        other_actor_events
            .last_mut()
            .expect("response event")
            .writer
            .actor_id = ActorId::new("actor:agent:someone-else");

        for events in [events, other_actor_events] {
            let projection = agent_resumption_from_events(
                &events,
                &task_attempt_id,
                &reader_actor(),
                &TrustSet::default(),
                ResumptionBindingPolicy::default(),
            )
            .unwrap();
            assert_eq!(projection.state, AgentResumptionState::Ready);
            assert!(projection.may_resume);
        }
    }

    // ADR-0010 principal-sufficiency composition tests.

    fn agent_response_events() -> (Vec<ShoreEvent>, WorkObjectId) {
        let (mut events, task_attempt_id) = approved_local_response_events();
        events.last_mut().expect("response event").writer.actor_id =
            ActorId::new("actor:agent:claude-code");
        (events, task_attempt_id)
    }

    fn delegates_for(records: serde_json::Value) -> DelegationMap {
        crate::session::delegation_map_from_value(serde_json::json!({
            "delegates": { "actor:agent:claude-code": records }
        }))
        .unwrap()
    }

    fn resolving_delegates() -> DelegationMap {
        // The response occurredAt is 2026-05-18T00:00:03Z.
        delegates_for(serde_json::json!([
            { "principal": "actor:git-email:kevin@swiber.dev",
              "validFrom": "2026-05-01T00:00:00Z", "validUntil": null }
        ]))
    }

    fn resumption_with_principal(
        events: &[ShoreEvent],
        task_attempt_id: &WorkObjectId,
        map: Option<&DelegationMap>,
        principal_policy: PrincipalPolicy,
    ) -> AgentResumptionProjection {
        agent_resumption_with_principal_policy(
            events,
            task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
            map,
            principal_policy,
        )
        .unwrap()
    }

    #[test]
    fn default_principal_policy_changes_no_binding_outcome() {
        let (events, task_attempt_id) = agent_response_events();
        let projection =
            resumption_with_principal(&events, &task_attempt_id, None, PrincipalPolicy::None);
        assert!(projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ready);
    }

    #[test]
    fn require_resolvable_principal_blocks_unresolved_agent_response() {
        let (events, task_attempt_id) = agent_response_events();
        let projection = resumption_with_principal(
            &events,
            &task_attempt_id,
            None,
            PrincipalPolicy::RequireResolvablePrincipal,
        );
        assert!(!projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Blocked);
        // The ADR-0009 identity itself was binding (arm a); only the principal
        // refinement blocks.
        assert!(
            projection
                .selected_response
                .as_ref()
                .expect("selected response")
                .identity_treated_as_binding
        );
        assert!(
            projection
                .diagnostics
                .iter()
                .any(|d| d.reason.as_deref() == Some("principal_unresolvable")),
            "diagnostics: {:?}",
            projection.diagnostics
        );
    }

    #[test]
    fn require_resolvable_principal_binds_resolved_local_agent_response() {
        let (events, task_attempt_id) = agent_response_events();
        let map = resolving_delegates();
        let projection = resumption_with_principal(
            &events,
            &task_attempt_id,
            Some(&map),
            PrincipalPolicy::RequireResolvablePrincipal,
        );
        assert!(
            projection.may_resume,
            "diagnostics: {:?}",
            projection.diagnostics
        );
        assert_eq!(projection.state, AgentResumptionState::Ready);
    }

    #[test]
    fn human_responses_are_unaffected_by_require_policy() {
        // The default response writer is a non-agent actor — its own principal.
        let (events, task_attempt_id) = approved_local_response_events();
        let projection = resumption_with_principal(
            &events,
            &task_attempt_id,
            None,
            PrincipalPolicy::RequireResolvablePrincipal,
        );
        assert!(projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ready);
    }

    #[test]
    fn principal_policy_never_widens_the_binding_predicate() {
        // Ingested unsigned: ADR-0009 refuses (arm (a) needs local possession).
        let (mut events, task_attempt_id) = agent_response_events();
        events.last_mut().expect("response event").ingest = Some(IngestProvenance {
            via: IngestVia::IngestEvents,
            received_at: "unix-ms:1760000000000".to_owned(),
        });
        let map = resolving_delegates();
        for policy in [
            PrincipalPolicy::None,
            PrincipalPolicy::Prefer,
            PrincipalPolicy::RequireResolvablePrincipal,
        ] {
            let projection =
                resumption_with_principal(&events, &task_attempt_id, Some(&map), policy);
            assert!(
                !projection.may_resume,
                "{policy:?} must never widen the binding predicate"
            );
        }
    }

    #[test]
    fn prefer_surfaces_diagnostics_without_operative_effect() {
        let (events, task_attempt_id) = agent_response_events();
        let projection =
            resumption_with_principal(&events, &task_attempt_id, None, PrincipalPolicy::Prefer);
        // Operative outcome identical to None.
        assert!(projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ready);
        // Plus an advisory principal diagnostic.
        assert!(
            projection
                .diagnostics
                .iter()
                .any(|d| d.reason.as_deref() == Some("principal_unresolvable")),
            "diagnostics: {:?}",
            projection.diagnostics
        );
    }

    #[test]
    fn ambiguous_principal_blocks_under_require_with_named_reason() {
        let (events, task_attempt_id) = agent_response_events();
        let map = delegates_for(serde_json::json!([
            { "principal": "actor:git-email:kevin@swiber.dev",
              "validFrom": "2026-05-01T00:00:00Z", "validUntil": null },
            { "principal": "actor:git-email:alice@example.com",
              "validFrom": "2026-05-01T00:00:00Z", "validUntil": null }
        ]));
        let projection = resumption_with_principal(
            &events,
            &task_attempt_id,
            Some(&map),
            PrincipalPolicy::RequireResolvablePrincipal,
        );
        assert!(!projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Blocked);
        assert!(
            projection
                .diagnostics
                .iter()
                .any(|d| d.reason.as_deref() == Some("principal_ambiguous")),
            "diagnostics: {:?}",
            projection.diagnostics
        );
    }

    #[test]
    fn non_binding_reason_vocabulary_first_match_wins() {
        use EventVerificationStatus::{Invalid, Unsigned, UntrustedKey};
        use ResumptionBindingPolicy::{LocalAndVerified, VerifiedOnly};
        let cases = [
            // (verification, ingested, policy,       reason)
            (Invalid, false, LocalAndVerified, "signature_invalid"),
            (Invalid, true, LocalAndVerified, "signature_invalid"),
            (Invalid, false, VerifiedOnly, "signature_invalid"),
            (Invalid, true, VerifiedOnly, "signature_invalid"),
            (
                UntrustedKey,
                true,
                LocalAndVerified,
                "signer_not_authorized",
            ),
            (UntrustedKey, false, VerifiedOnly, "signer_not_authorized"),
            (UntrustedKey, true, VerifiedOnly, "signer_not_authorized"),
            (Unsigned, true, LocalAndVerified, "ingested_unsigned"),
            (Unsigned, true, VerifiedOnly, "ingested_unsigned"),
            (Unsigned, false, VerifiedOnly, "policy_excludes_local"),
        ];
        for (verification, ingested, policy, expected) in cases {
            assert_eq!(
                non_binding_reason(verification, ingested, policy),
                expected,
                "({verification:?}, ingested: {ingested}, {policy:?})"
            );
        }
    }

    #[test]
    fn ingested_unsigned_response_diagnostic_names_ingested_unsigned() {
        let (mut events, task_attempt_id) = approved_local_response_events();
        events.last_mut().expect("response event").ingest = Some(IngestProvenance {
            via: IngestVia::IngestEvents,
            received_at: "unix-ms:1".to_owned(),
        });

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();

        assert_eq!(projection.state, AgentResumptionState::Blocked);
        let diagnostic = identity_not_binding_diagnostic(&projection);
        assert_eq!(diagnostic.reason.as_deref(), Some("ingested_unsigned"));
        // Ambiguity-honest: states what the projection knows (the responder
        // must sign for this to bind), never a guess about who responded.
        assert!(
            diagnostic.message.contains("sign"),
            "message names the cheapest fix; got {}",
            diagnostic.message
        );
    }

    #[test]
    fn invalid_signature_diagnostic_names_signature_invalid() {
        let (mut events, task_attempt_id) = approved_local_response_events();
        let response = events.last_mut().expect("response event");
        response.signer = Some(
            crate::crypto::SignerId::parse(
                "did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd",
            )
            .unwrap(),
        );
        response.signature = Some(
            crate::session::event::EventSignature::new_ed25519_v1(
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==",
            )
            .unwrap(),
        );

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();

        assert_eq!(projection.state, AgentResumptionState::Blocked);
        let diagnostic = identity_not_binding_diagnostic(&projection);
        assert_eq!(diagnostic.reason.as_deref(), Some("signature_invalid"));
        assert!(diagnostic.message.contains("invalid"));
    }

    #[test]
    fn unauthorized_signer_diagnostic_names_signer_not_authorized() {
        // A really-signed but unauthorized response: the signature verifies,
        // the empty trust set does not authorize the signer, and the ingest
        // stamp removes arm (a).
        let (mut events, task_attempt_id) = approved_local_response_events();
        let response = events.last_mut().expect("response event");
        sign_event_with_seeded_key(response);
        response.ingest = Some(IngestProvenance {
            via: IngestVia::IngestEvents,
            received_at: "unix-ms:1".to_owned(),
        });

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();

        assert_eq!(projection.state, AgentResumptionState::Blocked);
        let diagnostic = identity_not_binding_diagnostic(&projection);
        assert_eq!(diagnostic.reason.as_deref(), Some("signer_not_authorized"));
        assert!(diagnostic.message.contains("authorize"));
    }

    #[test]
    fn local_unsigned_under_verified_only_diagnostic_names_policy_excludes_local() {
        let (events, task_attempt_id) = approved_local_response_events();

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::VerifiedOnly,
        )
        .unwrap();

        assert_eq!(projection.state, AgentResumptionState::Blocked);
        let diagnostic = identity_not_binding_diagnostic(&projection);
        assert_eq!(diagnostic.reason.as_deref(), Some("policy_excludes_local"));
        assert!(diagnostic.message.contains("verified-only"));
    }

    #[test]
    fn binding_predicate_two_arm_truth_table() {
        use EventVerificationStatus::{Invalid, Unsigned, UntrustedKey, Valid};
        use ResumptionBindingPolicy::{LocalAndVerified, VerifiedOnly};
        let cases = [
            // (verification, ingested, policy,           binds)
            (Valid, false, LocalAndVerified, true), // arm (b)
            (Valid, true, LocalAndVerified, true),  // arm (b): stamp irrelevant
            (Valid, true, VerifiedOnly, true),      // arm (b): policy irrelevant
            (Valid, false, VerifiedOnly, true),     // arm (b)
            (Unsigned, false, LocalAndVerified, true), // arm (a): the local-first product
            (UntrustedKey, false, LocalAndVerified, true), // arm (a): != invalid suffices
            (Invalid, false, LocalAndVerified, false), // invalid defeats arm (a)
            (Unsigned, true, LocalAndVerified, false), // ingested unsigned never binds
            (UntrustedKey, true, LocalAndVerified, false), // ingested untrusted never binds
            (Invalid, true, LocalAndVerified, false),
            (Unsigned, false, VerifiedOnly, false), // policy excludes local
            (UntrustedKey, false, VerifiedOnly, false),
            (Invalid, false, VerifiedOnly, false),
            (Unsigned, true, VerifiedOnly, false),
            (UntrustedKey, true, VerifiedOnly, false),
            (Invalid, true, VerifiedOnly, false),
        ];
        for (verification, ingested, policy, expected) in cases {
            assert_eq!(
                response_identity_is_binding(verification, ingested, policy),
                expected,
                "({verification:?}, ingested: {ingested}, {policy:?})"
            );
        }
    }

    #[test]
    fn binding_predicate_ignores_source_speaker_payload_fact() {
        // A self-asserted sourceSpeaker: user payload fact must never drive
        // binding in either direction; payload facts are writer-asserted, not
        // verified identity (preserved non-input pin from plan 0059).
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let input_request_id = InputRequestId::new("input-request:sha256:1");
        let response_id = InputRequestResponseId::new("input-request-response:sha256:r");

        let mut events = attempt_with_checkpoints(&task_attempt_id, &session_id, &[]);
        events.push(task_input_request_event_with_target(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:speaker",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::TaskAttempt),
            "needs approval",
        ));
        let mut response = user_response_event(
            &input_request_id,
            &response_id,
            InputRequestResponseOutcome::Approved,
            AssertionMode::Operative,
            "2026-05-18T00:00:03Z",
        );
        response.payload["sourceSpeaker"] = serde_json::json!("user");
        events.push(response);

        // Local: binds via arm (a) — with or without the payload fact.
        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();
        assert_eq!(projection.state, AgentResumptionState::Ready);
        assert!(
            projection
                .selected_response
                .as_ref()
                .expect("selected response")
                .identity_treated_as_binding
        );

        // Ingested: does not bind — sourceSpeaker: "user" cannot rescue it.
        let response = events.last_mut().expect("response event");
        response.ingest = Some(IngestProvenance {
            via: IngestVia::IngestEvents,
            received_at: "unix-ms:1".to_owned(),
        });
        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();
        assert_eq!(projection.state, AgentResumptionState::Blocked);
        assert!(
            !projection
                .selected_response
                .as_ref()
                .expect("selected response")
                .identity_treated_as_binding
        );
    }

    #[test]
    fn resumption_binding_policy_default_is_local_and_verified() {
        assert_eq!(
            ResumptionBindingPolicy::default(),
            ResumptionBindingPolicy::LocalAndVerified
        );
        assert!(ResumptionBindingPolicy::LocalAndVerified.permits_local_possession());
        assert!(!ResumptionBindingPolicy::VerifiedOnly.permits_local_possession());
    }

    #[test]
    fn response_binding_evidence_reports_verification_and_ingest_presence() {
        let input_request_id = InputRequestId::new("input-request:sha256:1");
        let response_id = InputRequestResponseId::new("input-request-response:sha256:r");
        let unsigned = user_response_event(
            &input_request_id,
            &response_id,
            InputRequestResponseOutcome::Approved,
            AssertionMode::Operative,
            "2026-05-18T00:00:03Z",
        );

        // Unsigned local event -> (Unsigned, ingested: false)
        assert_eq!(
            response_binding_evidence(&unsigned, &TrustSet::default()).unwrap(),
            (EventVerificationStatus::Unsigned, false)
        );

        // Unsigned event with an ingest stamp -> (Unsigned, ingested: true)
        let mut stamped = unsigned.clone();
        stamped.ingest = Some(IngestProvenance {
            via: IngestVia::IngestEvents,
            received_at: "unix-ms:1".to_owned(),
        });
        assert_eq!(
            response_binding_evidence(&stamped, &TrustSet::default()).unwrap(),
            (EventVerificationStatus::Unsigned, true)
        );

        // Signed fixture event + authorizing trust set -> Valid.
        let signed: ShoreEvent = serde_json::from_str(include_str!(
            "../../../tests/fixtures/event_signatures/friendly-valid-event.json"
        ))
        .unwrap();
        let fixture_trust = event_signature_trust_set(
            serde_json::from_str(include_str!(
                "../../../tests/fixtures/event_signatures/did-key-ed25519.json"
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            response_binding_evidence(&signed, &fixture_trust)
                .unwrap()
                .0,
            EventVerificationStatus::Valid
        );

        // Signed event + empty trust set (non-did:key actor) -> UntrustedKey.
        assert_eq!(
            response_binding_evidence(&signed, &TrustSet::default())
                .unwrap()
                .0,
            EventVerificationStatus::UntrustedKey
        );

        // Tampered signed event -> Invalid.
        let mut tampered = signed.clone();
        tampered.payload["tamperedAfterSigning"] = serde_json::json!(true);
        tampered.payload_hash = sha256_json_prefixed(&tampered.payload).unwrap();
        assert_eq!(
            response_binding_evidence(&tampered, &fixture_trust)
                .unwrap()
                .0,
            EventVerificationStatus::Invalid
        );
    }

    #[test]
    fn agent_resumption_fails_closed_for_advisory_response() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let input_request_id = InputRequestId::new("input-request:sha256:1");
        let response_id = InputRequestResponseId::new("input-request-response:sha256:r");

        let mut events = attempt_with_checkpoints(&task_attempt_id, &session_id, &[]);
        events.push(task_input_request_event_with_target(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:adv",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::TaskAttempt),
            "needs approval",
        ));
        events.push(user_response_event(
            &input_request_id,
            &response_id,
            InputRequestResponseOutcome::Approved,
            AssertionMode::Advisory,
            "2026-05-18T00:00:03Z",
        ));

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();
        assert!(!projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Blocked);
        assert!(!projection.treated_as_operative);
    }

    #[test]
    fn agent_resumption_fails_closed_for_ambiguous_responses() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let input_request_id = InputRequestId::new("input-request:sha256:1");
        let r1 = InputRequestResponseId::new("input-request-response:sha256:r1");
        let r2 = InputRequestResponseId::new("input-request-response:sha256:r2");

        let mut events = attempt_with_checkpoints(&task_attempt_id, &session_id, &[]);
        events.push(task_input_request_event_with_target(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:ambig",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::TaskAttempt),
            "needs approval",
        ));
        events.push(user_response_event(
            &input_request_id,
            &r1,
            InputRequestResponseOutcome::Approved,
            AssertionMode::Operative,
            "2026-05-18T00:00:03Z",
        ));
        events.push(user_response_event(
            &input_request_id,
            &r2,
            InputRequestResponseOutcome::Rejected,
            AssertionMode::Operative,
            "2026-05-18T00:00:04Z",
        ));

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();

        assert!(!projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ambiguous);
        assert!(
            projection
                .diagnostics
                .iter()
                .any(|d| d.code == "agent_resumption_ambiguous_input_request_responses"),
            "diagnostic explains the ambiguity"
        );
        assert!(projection.selected_response.is_none());
    }

    #[test]
    fn agent_resumption_marks_checkpoint_response_stale_when_newer_checkpoint_exists() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let cp_a = CheckpointId::new("checkpoint:sha256:cp-a");
        let cp_b = CheckpointId::new("checkpoint:sha256:cp-b");
        let input_request_id = InputRequestId::new("input-request:sha256:1");
        let response_id = InputRequestResponseId::new("input-request-response:sha256:r");

        let mut events = attempt_with_checkpoints(
            &task_attempt_id,
            &session_id,
            &[
                (&cp_a, "msg_a", "2026-05-18T00:00:01Z"),
                (&cp_b, "msg_b", "2026-05-18T00:00:05Z"),
            ],
        );
        events.push(task_input_request_event_with_target(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:stale",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: cp_a.clone(),
            }),
            "needs approval at checkpoint a",
        ));
        events.push(user_response_event(
            &input_request_id,
            &response_id,
            InputRequestResponseOutcome::Approved,
            AssertionMode::Operative,
            "2026-05-18T00:00:03Z",
        ));

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();
        assert!(!projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Stale);
        assert!(!projection.treated_as_operative);
        assert_eq!(
            projection.freshness,
            Some(FreshnessBasis::CheckpointStaleNewerExists)
        );
    }

    // -- no-information-loss validation across the three sibling views ----

    #[test]
    fn task_projections_preserve_envelope_payload_and_semantic_ids_across_three_views() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let cp_a = CheckpointId::new("checkpoint:sha256:cp-a");
        let cp_b = CheckpointId::new("checkpoint:sha256:cp-b");
        let input_request_id = InputRequestId::new("input-request:sha256:1");
        let response_id = InputRequestResponseId::new("input-request-response:sha256:r");

        let attempt = task_attempt_event(
            &task_attempt_id,
            &session_id,
            "uuid-1",
            "2026-05-18T00:00:00Z",
        );
        let cp_a_event = checkpoint_event(
            &task_attempt_id,
            &session_id,
            &cp_a,
            "msg_a",
            vec!["tu_a".to_owned()],
            "2026-05-18T00:00:01Z",
        );
        let cp_b_event = checkpoint_event(
            &task_attempt_id,
            &session_id,
            &cp_b,
            "msg_b",
            vec!["tu_b".to_owned()],
            "2026-05-18T00:00:03Z",
        );
        let obs_a = observation_event(
            &task_attempt_id,
            &session_id,
            Some(&cp_a),
            "uuid-1#tool_result:tu_a",
            "tool_result: Bash",
            "2026-05-18T00:00:02Z",
        );
        let obs_b = observation_event(
            &task_attempt_id,
            &session_id,
            Some(&cp_b),
            "uuid-1#tool_result:tu_b",
            "tool_result: Read",
            "2026-05-18T00:00:04Z",
        );
        let request = task_input_request_event_with_target(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:approve",
            "2026-05-18T00:00:05Z",
            TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: cp_b.clone(),
            }),
            "needs approval",
        );
        let response = user_response_event_with_reason(
            &input_request_id,
            &response_id,
            InputRequestResponseOutcome::Approved,
            AssertionMode::Operative,
            "2026-05-18T00:00:06Z",
            Some("approved by reviewer".to_owned()),
            Some("artifacts/responses/r.txt".to_owned()),
            Some(19),
            Some("sha256:reason".to_owned()),
        );

        let events = vec![
            attempt.clone(),
            cp_a_event.clone(),
            cp_b_event.clone(),
            obs_a.clone(),
            obs_b.clone(),
            request.clone(),
            response.clone(),
        ];

        let summary = task_attempt_summary_from_events(&events, &task_attempt_id, &reader_actor())
            .unwrap()
            .expect("attempt present");
        let input_requests =
            open_task_input_requests_from_events(&events, &task_attempt_id, &reader_actor())
                .unwrap();
        let resumption = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();

        let task_event_ids: Vec<&str> = [
            attempt.event_id.as_str(),
            cp_a_event.event_id.as_str(),
            cp_b_event.event_id.as_str(),
            obs_a.event_id.as_str(),
            obs_b.event_id.as_str(),
        ]
        .to_vec();
        let mut surfaced_task_event_ids: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        surfaced_task_event_ids.insert(summary.attempt_event.event_id.as_str().to_owned());
        for cp in &summary.checkpoints {
            surfaced_task_event_ids.insert(cp.envelope.event_id.as_str().to_owned());
            for obs in &cp.observations {
                surfaced_task_event_ids.insert(obs.envelope.event_id.as_str().to_owned());
            }
        }
        for obs in &summary.observations_without_checkpoint {
            surfaced_task_event_ids.insert(obs.envelope.event_id.as_str().to_owned());
        }
        for task_id in &task_event_ids {
            assert!(
                surfaced_task_event_ids.contains(*task_id),
                "task event id {task_id} missing from task_attempt_summary"
            );
        }

        let selected_input_request = resumption
            .selected_input_request
            .as_ref()
            .expect("input request surfaced");
        assert_eq!(selected_input_request.envelope.event_id, request.event_id);
        let selected_response = resumption
            .selected_response
            .as_ref()
            .expect("response surfaced");
        assert_eq!(selected_response.envelope.event_id, response.event_id);

        assert_eq!(summary.task_attempt_id, task_attempt_id);
        let surfaced_checkpoint_ids: std::collections::BTreeSet<CheckpointId> = summary
            .checkpoints
            .iter()
            .map(|cp| cp.checkpoint_id.clone())
            .collect();
        assert!(surfaced_checkpoint_ids.contains(&cp_a));
        assert!(surfaced_checkpoint_ids.contains(&cp_b));

        let surfaced_observation_ids: std::collections::BTreeSet<ObservationId> = summary
            .checkpoints
            .iter()
            .flat_map(|cp| cp.observations.iter().map(|obs| obs.observation_id.clone()))
            .chain(
                summary
                    .observations_without_checkpoint
                    .iter()
                    .map(|obs| obs.observation_id.clone()),
            )
            .collect();
        let obs_a_payload: TaskObservationRecordedPayload =
            serde_json::from_value(obs_a.payload.clone()).unwrap();
        let obs_b_payload: TaskObservationRecordedPayload =
            serde_json::from_value(obs_b.payload.clone()).unwrap();
        assert!(surfaced_observation_ids.contains(&obs_a_payload.observation_id));
        assert!(surfaced_observation_ids.contains(&obs_b_payload.observation_id));

        assert_eq!(selected_input_request.input_request_id, input_request_id);
        assert_eq!(selected_response.response_id, response_id);

        let latest = summary
            .latest_checkpoint
            .as_ref()
            .expect("latest checkpoint surfaced");
        assert_eq!(latest.envelope.assertion_mode, cp_b_event.assertion_mode);
        assert_eq!(latest.envelope.source_ref, cp_b_event.source_ref);
        assert_eq!(latest.envelope.writer, cp_b_event.writer);
        assert_eq!(latest.envelope.target, cp_b_event.target);
        assert_eq!(latest.envelope.occurred_at, cp_b_event.occurred_at);
        assert_eq!(latest.envelope.payload_hash, cp_b_event.payload_hash);

        assert_eq!(
            selected_response.envelope.assertion_mode,
            AssertionMode::Operative
        );
        assert_eq!(selected_response.envelope.source_ref, response.source_ref);
        assert_eq!(selected_response.envelope.writer, response.writer);
        assert_eq!(selected_response.envelope.target, response.target);
        assert_eq!(
            selected_response.envelope.payload_hash,
            response.payload_hash
        );
        assert_eq!(selected_response.envelope.occurred_at, response.occurred_at);

        assert_eq!(summary.project_path, "/repo");
        assert_eq!(summary.claude_session_uuid, "uuid-1");
        assert_eq!(summary.initial_prompt_hash, "sha256:prompt");
        for cp in &summary.checkpoints {
            assert!(!cp.assistant_message_id.is_empty());
        }
        for cp in &summary.checkpoints {
            for obs in &cp.observations {
                assert!(!obs.title.is_empty());
            }
        }

        assert_eq!(selected_input_request.title, "needs approval");
        assert_eq!(selected_input_request.mode, AssertionMode::Operative);
        assert_eq!(
            selected_input_request.reason_code,
            InputRequestReasonCode::ManualDecisionRequired
        );

        // Response payload reason fields must survive into the policy view.
        assert_eq!(
            selected_response.reason.as_deref(),
            Some("approved by reviewer")
        );
        assert_eq!(
            selected_response.reason_artifact_path.as_deref(),
            Some("artifacts/responses/r.txt")
        );
        assert_eq!(selected_response.reason_byte_size, Some(19));
        assert_eq!(
            selected_response.reason_content_hash.as_deref(),
            Some("sha256:reason")
        );

        // The payload's review-shaped `target` field is preserved as
        // diagnostic evidence by `open_task_input_requests` when the
        // input request is still open. Re-run without the response to
        // confirm the diagnostic fires for unresponded task input requests.
        let events_without_response = vec![
            attempt.clone(),
            cp_a_event.clone(),
            cp_b_event.clone(),
            obs_a.clone(),
            obs_b.clone(),
            request.clone(),
        ];
        let input_requests_open_only = open_task_input_requests_from_events(
            &events_without_response,
            &task_attempt_id,
            &reader_actor(),
        )
        .unwrap();
        assert!(
            input_requests_open_only
                .diagnostics
                .iter()
                .any(|d| d.code == "task_input_request_payload_target_is_review_shaped"),
            "payload target mismatch must be visible as diagnostic for open input request"
        );

        // Once responded, the input request drops out of the open set.
        assert!(input_requests.open_input_requests.is_empty());

        // The local unsigned response binds via arm (a); envelope/payload
        // preservation above is the pin here.
        assert!(resumption.may_resume);
        assert_eq!(resumption.state, AgentResumptionState::Ready);
        assert!(resumption.treated_as_operative);
        assert_eq!(
            resumption.freshness,
            Some(FreshnessBasis::CheckpointMatchesLatest)
        );
    }

    #[test]
    fn agent_resumption_fails_closed_when_task_attempt_absent() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:missing");
        let projection = agent_resumption_from_events(
            &[],
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();
        assert!(!projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::NoAttempt);
        assert!(
            projection
                .diagnostics
                .iter()
                .any(|d| d.code == "agent_resumption_no_task_attempt"),
            "diagnostic explains the missing attempt"
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn user_response_event_with_reason(
        input_request_id: &InputRequestId,
        response_id: &InputRequestResponseId,
        outcome: InputRequestResponseOutcome,
        assertion_mode: AssertionMode,
        occurred_at: &str,
        reason: Option<String>,
        reason_artifact_path: Option<String>,
        reason_byte_size: Option<u64>,
        reason_content_hash: Option<String>,
    ) -> ShoreEvent {
        let target = EventTarget::for_work_object(
            SessionId::new("session:claude:uuid-1"),
            WorkObjectId::new("task-attempt:sha256:ta"),
            WorkObjectType::TaskAttempt,
        );
        let payload = InputRequestRespondedPayload {
            input_request_response_id: response_id.clone(),
            input_request_id: input_request_id.clone(),
            outcome,
            reason,
            reason_artifact_path,
            reason_byte_size,
            reason_content_hash,
            target_fingerprint: None,
        };
        let idempotency_key =
            InputRequestRespondedPayload::idempotency_key(input_request_id, response_id.as_str());
        let writer = Writer {
            actor_id: ActorId::new("actor:claude_code:user"),
            producer: WriterProducer {
                name: "claude_code".to_owned(),
                version: String::new(),
            },
        };
        let mut event = ShoreEvent::new(
            EventType::InputRequestResponded,
            idempotency_key,
            target,
            writer,
            payload,
            occurred_at,
        )
        .unwrap();
        event.assertion_mode = assertion_mode;
        event.source_ref = Some(SourceRef::new("claude_code", response_id.as_str()));
        event
    }

    #[test]
    fn task_projections_preserve_input_request_fingerprint_and_response_reason_fields() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let input_request_id = InputRequestId::new("input-request:sha256:1");
        let response_id = InputRequestResponseId::new("input-request-response:sha256:r");

        let mut events = attempt_with_checkpoints(&task_attempt_id, &session_id, &[]);
        events.push(task_input_request_event_with_target_and_fingerprint(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:no-loss",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::TaskAttempt),
            "needs approval",
            Some("sha256:request-state"),
        ));
        events.push(user_response_event_with_reason(
            &input_request_id,
            &response_id,
            InputRequestResponseOutcome::Approved,
            AssertionMode::Operative,
            "2026-05-18T00:00:03Z",
            Some("inlined justification".to_owned()),
            Some("artifacts/notes/reason.json".to_owned()),
            Some(128),
            Some("sha256:reason".to_owned()),
        ));

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();

        let selected = projection.selected_input_request.expect("request surfaced");
        assert_eq!(
            selected.target_fingerprint.as_deref(),
            Some("sha256:request-state")
        );

        let response = projection.selected_response.expect("response surfaced");
        assert_eq!(
            response.reason_artifact_path.as_deref(),
            Some("artifacts/notes/reason.json")
        );
        assert_eq!(response.reason_byte_size, Some(128));
        assert_eq!(
            response.reason_content_hash.as_deref(),
            Some("sha256:reason")
        );
    }

    #[test]
    fn agent_resumption_preserves_response_reason_payload_fields() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let input_request_id = InputRequestId::new("input-request:sha256:1");
        let response_id = InputRequestResponseId::new("input-request-response:sha256:r");

        let mut events = attempt_with_checkpoints(&task_attempt_id, &session_id, &[]);
        events.push(task_input_request_event_with_target(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:reason",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::TaskAttempt),
            "needs approval",
        ));
        events.push(user_response_event_with_reason(
            &input_request_id,
            &response_id,
            InputRequestResponseOutcome::Approved,
            AssertionMode::Operative,
            "2026-05-18T00:00:03Z",
            Some("inlined justification".to_owned()),
            Some("artifacts/responses/r.txt".to_owned()),
            Some(42),
            Some("sha256:reason-hash".to_owned()),
        ));

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();

        let view = projection
            .selected_response
            .as_ref()
            .expect("response surfaced");
        assert_eq!(view.reason.as_deref(), Some("inlined justification"));
        assert_eq!(
            view.reason_artifact_path.as_deref(),
            Some("artifacts/responses/r.txt"),
        );
        assert_eq!(view.reason_byte_size, Some(42));
        assert_eq!(
            view.reason_content_hash.as_deref(),
            Some("sha256:reason-hash"),
        );
    }

    #[test]
    fn agent_resumption_collapses_duplicate_response_facts_instead_of_ambiguous() {
        // Two `InputRequestResponded` events with the same
        // `input_request_response_id` are a retry duplicate, not two distinct
        // responses. The projection must collapse them and still treat the
        // input request as cleanly responded.
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let input_request_id = InputRequestId::new("input-request:sha256:1");
        let response_id = InputRequestResponseId::new("input-request-response:sha256:r");

        let mut events = attempt_with_checkpoints(&task_attempt_id, &session_id, &[]);
        events.push(task_input_request_event_with_target(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:dup",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::TaskAttempt),
            "needs approval",
        ));
        let first = user_response_event(
            &input_request_id,
            &response_id,
            InputRequestResponseOutcome::Approved,
            AssertionMode::Operative,
            "2026-05-18T00:00:03Z",
        );
        let retry = user_response_event(
            &input_request_id,
            &response_id,
            InputRequestResponseOutcome::Approved,
            AssertionMode::Operative,
            "2026-05-18T00:00:03Z",
        );
        // Confirm the duplicate construction would emit identical events
        // (same idempotency key, therefore same event_id) before asserting
        // the projection treats them as one.
        assert_eq!(first.event_id, retry.event_id);
        events.push(first);
        events.push(retry);

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();

        // The retry pair collapses to one representative response (not
        // Ambiguous); the local unsigned representative binds via arm (a).
        assert_ne!(projection.state, AgentResumptionState::Ambiguous);
        assert_eq!(projection.state, AgentResumptionState::Ready);
        assert!(
            projection.selected_response.is_some(),
            "collapsed representative response is surfaced"
        );
    }

    #[test]
    fn agent_resumption_collapses_distinct_event_ids_for_same_response_id() {
        // Same `input_request_response_id` but distinct envelope event ids
        // (e.g., a writer mistakenly emits the same semantic response with
        // two different idempotency keys). The projection still collapses by
        // response id and avoids a false Ambiguous classification.
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let input_request_id = InputRequestId::new("input-request:sha256:1");
        let response_id = InputRequestResponseId::new("input-request-response:sha256:r");

        let mut events = attempt_with_checkpoints(&task_attempt_id, &session_id, &[]);
        events.push(task_input_request_event_with_target(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:dup-ids",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::TaskAttempt),
            "needs approval",
        ));

        // Two semantically-equal response events with distinct event ids
        // (constructed by mutating the idempotency key directly so the
        // derived event_id differs).
        let mut first = user_response_event(
            &input_request_id,
            &response_id,
            InputRequestResponseOutcome::Approved,
            AssertionMode::Operative,
            "2026-05-18T00:00:03Z",
        );
        let mut second = user_response_event(
            &input_request_id,
            &response_id,
            InputRequestResponseOutcome::Approved,
            AssertionMode::Operative,
            "2026-05-18T00:00:04Z",
        );
        first.idempotency_key = "duplicate-a".to_owned();
        first.event_id = crate::model::EventId::new("evt:sha256:duplicate-a");
        second.idempotency_key = "duplicate-b".to_owned();
        second.event_id = crate::model::EventId::new("evt:sha256:duplicate-b");
        assert_ne!(first.event_id, second.event_id);

        events.push(first);
        events.push(second);

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();

        // Collapsed to one representative (not Ambiguous); the local unsigned
        // representative binds via arm (a).
        assert_ne!(projection.state, AgentResumptionState::Ambiguous);
        assert_eq!(projection.state, AgentResumptionState::Ready);
        let view = projection
            .selected_response
            .as_ref()
            .expect("response surfaced");
        // Representative selection is by lexicographically lowest event_id.
        assert_eq!(view.envelope.event_id.as_str(), "evt:sha256:duplicate-a");
    }

    // -- fingerprint-driven freshness ----------------------------------------

    // Realistic-looking opaque fingerprints. Equality is the only operation
    // the projection performs on these; the format mirrors `initial_prompt_hash`
    // so no string-ordering shortcut can substitute for `==`.
    const FP_A: &str = "sha256:000000000000000000000000000000000000000000000000000000000000000a";
    const FP_B: &str = "sha256:000000000000000000000000000000000000000000000000000000000000000b";
    const FP_C: &str = "sha256:000000000000000000000000000000000000000000000000000000000000000c";

    fn checkpoint_event_with_fingerprint(
        task_attempt_id: &WorkObjectId,
        session_id: &SessionId,
        checkpoint_id: &CheckpointId,
        assistant_message_id: &str,
        tool_use_ids: Vec<String>,
        occurred_at: &str,
        fingerprint: Option<&str>,
    ) -> ShoreEvent {
        let mut target = EventTarget::for_work_object(
            session_id.clone(),
            task_attempt_id.clone(),
            WorkObjectType::TaskAttempt,
        );
        target.subject = Some(TargetRef::Task(TaskTargetRef::Checkpoint {
            checkpoint_id: checkpoint_id.clone(),
        }));
        let payload = TaskCheckpointCapturedPayload {
            checkpoint_id: checkpoint_id.clone(),
            parent_task_attempt_id: task_attempt_id.clone(),
            assistant_message_id: assistant_message_id.to_owned(),
            tool_use_ids,
            checkpoint_fingerprint: fingerprint.map(str::to_owned),
            source_speaker: None,
        };
        let idempotency_key = TaskCheckpointCapturedPayload::idempotency_key_for_work_object(
            task_attempt_id,
            WorkObjectType::TaskAttempt,
            checkpoint_id.as_str(),
        );
        let mut event = ShoreEvent::new(
            EventType::TaskCheckpointCaptured,
            idempotency_key,
            target,
            Writer::shore_local("test"),
            payload,
            occurred_at,
        )
        .unwrap();
        event.source_ref = Some(SourceRef::new(
            "claude_code",
            format!("session:assistant:{assistant_message_id}"),
        ));
        event.assertion_mode = AssertionMode::Advisory;
        event
    }

    #[allow(clippy::too_many_arguments)]
    fn task_input_request_event_with_target_and_fingerprint(
        task_attempt_id: &WorkObjectId,
        session_id: &SessionId,
        input_request_id: &InputRequestId,
        source_key: &str,
        occurred_at: &str,
        subject: TargetRef,
        title: &str,
        target_fingerprint: Option<&str>,
    ) -> ShoreEvent {
        let mut target = EventTarget::for_work_object(
            session_id.clone(),
            task_attempt_id.clone(),
            WorkObjectType::TaskAttempt,
        );
        target.subject = Some(subject);
        let payload = InputRequestOpenedPayload {
            input_request_id: input_request_id.clone(),
            target: ReviewTargetRef::ReviewUnit {
                review_unit_id: ReviewUnitId::new("review-unit:placeholder"),
            },
            reason_code: InputRequestReasonCode::ManualDecisionRequired,
            title: title.to_owned(),
            body: None,
            body_artifact_path: None,
            body_byte_size: None,
            body_content_hash: None,
            target_fingerprint: target_fingerprint.map(str::to_owned),
        };
        let idempotency_key = InputRequestOpenedPayload::idempotency_key_for_work_object(
            task_attempt_id,
            WorkObjectType::TaskAttempt,
            source_key,
        );
        let mut event = ShoreEvent::new(
            EventType::InputRequestOpened,
            idempotency_key,
            target,
            Writer::shore_local("test"),
            payload,
            occurred_at,
        )
        .unwrap();
        event.source_ref = Some(SourceRef::new("claude_code", source_key));
        event.assertion_mode = AssertionMode::Operative;
        event
    }

    #[allow(clippy::too_many_arguments)]
    fn user_response_event_with_fingerprint(
        input_request_id: &InputRequestId,
        response_id: &InputRequestResponseId,
        outcome: InputRequestResponseOutcome,
        assertion_mode: AssertionMode,
        occurred_at: &str,
        target_fingerprint: Option<&str>,
    ) -> ShoreEvent {
        let target = EventTarget::for_work_object(
            SessionId::new("session:claude:uuid-1"),
            WorkObjectId::new("task-attempt:sha256:ta"),
            WorkObjectType::TaskAttempt,
        );
        let payload = InputRequestRespondedPayload {
            input_request_response_id: response_id.clone(),
            input_request_id: input_request_id.clone(),
            outcome,
            reason: None,
            reason_artifact_path: None,
            reason_byte_size: None,
            reason_content_hash: None,
            target_fingerprint: target_fingerprint.map(str::to_owned),
        };
        let idempotency_key =
            InputRequestRespondedPayload::idempotency_key(input_request_id, response_id.as_str());
        let writer = Writer {
            actor_id: ActorId::new("actor:claude_code:user"),
            producer: WriterProducer {
                name: "claude_code".to_owned(),
                version: String::new(),
            },
        };
        let mut event = ShoreEvent::new(
            EventType::InputRequestResponded,
            idempotency_key,
            target,
            writer,
            payload,
            occurred_at,
        )
        .unwrap();
        event.assertion_mode = assertion_mode;
        event.source_ref = Some(SourceRef::new("claude_code", response_id.as_str()));
        event
    }

    #[test]
    fn agent_resumption_marks_response_stale_when_target_fingerprint_diverges_from_latest_checkpoint()
     {
        // §4.5 Scenario A literal: requester saw F1 (checkpoint C_A); a later
        // checkpoint C_B with F2 exists; the responder acted on the F1 state.
        // The projection must flag the response stale on the fingerprint
        // disagreement -- not on identity alone.
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let cp_a = CheckpointId::new("checkpoint:sha256:cp-a");
        let cp_b = CheckpointId::new("checkpoint:sha256:cp-b");
        let input_request_id = InputRequestId::new("input-request:sha256:1");
        let response_id = InputRequestResponseId::new("input-request-response:sha256:r");

        let mut events = vec![task_attempt_event(
            &task_attempt_id,
            &session_id,
            "uuid-1",
            "2026-05-18T00:00:00Z",
        )];
        events.push(checkpoint_event_with_fingerprint(
            &task_attempt_id,
            &session_id,
            &cp_a,
            "msg_a",
            vec![],
            "2026-05-18T00:00:01Z",
            Some(FP_A),
        ));
        events.push(task_input_request_event_with_target_and_fingerprint(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:stale-fp",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: cp_a.clone(),
            }),
            "needs approval at F1",
            Some(FP_A),
        ));
        events.push(checkpoint_event_with_fingerprint(
            &task_attempt_id,
            &session_id,
            &cp_b,
            "msg_b",
            vec![],
            "2026-05-18T00:00:03Z",
            Some(FP_B),
        ));
        events.push(user_response_event_with_fingerprint(
            &input_request_id,
            &response_id,
            InputRequestResponseOutcome::Approved,
            AssertionMode::Operative,
            "2026-05-18T00:00:04Z",
            Some(FP_A),
        ));

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();

        assert!(!projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Stale);
        assert!(!projection.treated_as_operative);
        assert_eq!(
            projection.freshness,
            Some(FreshnessBasis::CheckpointFingerprintMismatch),
            "fingerprint disagreement is the stale signal, not just identity"
        );
        let response_view = projection
            .selected_response
            .as_ref()
            .expect("selected response surfaced");
        assert_eq!(
            response_view.target_fingerprint.as_deref(),
            Some(FP_A),
            "the responder's fingerprint is preserved on the policy view"
        );
        assert!(!response_view.fresh_for_target);
        assert!(
            projection
                .diagnostics
                .iter()
                .any(|d| { d.code == "agent_resumption_response_target_fingerprint_mismatch" }),
            "diagnostic names the fingerprint mismatch"
        );
    }

    #[test]
    fn agent_resumption_treats_response_fresh_when_target_fingerprint_matches_latest_checkpoint() {
        // §4.5 Scenario B (re-anchored): only one checkpoint, with fingerprint
        // F1; the responder acts on the F1 state. Both identity and fingerprint
        // agree, so the response is fresh.
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let cp_a = CheckpointId::new("checkpoint:sha256:cp-a");
        let input_request_id = InputRequestId::new("input-request:sha256:1");
        let response_id = InputRequestResponseId::new("input-request-response:sha256:r");

        let mut events = vec![task_attempt_event(
            &task_attempt_id,
            &session_id,
            "uuid-1",
            "2026-05-18T00:00:00Z",
        )];
        events.push(checkpoint_event_with_fingerprint(
            &task_attempt_id,
            &session_id,
            &cp_a,
            "msg_a",
            vec![],
            "2026-05-18T00:00:01Z",
            Some(FP_A),
        ));
        events.push(task_input_request_event_with_target_and_fingerprint(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:fresh-fp",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: cp_a.clone(),
            }),
            "needs approval at F1",
            Some(FP_A),
        ));
        events.push(user_response_event_with_fingerprint(
            &input_request_id,
            &response_id,
            InputRequestResponseOutcome::Approved,
            AssertionMode::Operative,
            "2026-05-18T00:00:03Z",
            Some(FP_A),
        ));

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();

        // Freshness pin: agreement on the opaque fingerprint is the signal;
        // the response is not classified stale.
        assert_ne!(projection.state, AgentResumptionState::Stale);
        assert_eq!(
            projection.freshness,
            Some(FreshnessBasis::CheckpointFingerprintMatches),
            "both fingerprints are Some and equal -- agreement on opaque fingerprint is the freshness signal"
        );
        let response_view = projection
            .selected_response
            .as_ref()
            .expect("selected response surfaced");
        assert!(response_view.fresh_for_target);
        assert_eq!(response_view.target_fingerprint.as_deref(), Some(FP_A));
    }

    #[test]
    fn agent_resumption_holds_ambiguous_when_two_distinct_response_facts_carry_different_target_fingerprints()
     {
        // Two distinct response facts (different `input_request_response_id`
        // values) on the same input request. The two also disagree on
        // `target_fingerprint`. The Ambiguous discipline must win before any
        // freshness check fires; introducing fingerprint comparison must not
        // accidentally let the projection pick one response and call the
        // other stale.
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let cp_a = CheckpointId::new("checkpoint:sha256:cp-a");
        let input_request_id = InputRequestId::new("input-request:sha256:1");
        let r1 = InputRequestResponseId::new("input-request-response:sha256:r1");
        let r2 = InputRequestResponseId::new("input-request-response:sha256:r2");

        let mut events = vec![task_attempt_event(
            &task_attempt_id,
            &session_id,
            "uuid-1",
            "2026-05-18T00:00:00Z",
        )];
        events.push(checkpoint_event_with_fingerprint(
            &task_attempt_id,
            &session_id,
            &cp_a,
            "msg_a",
            vec![],
            "2026-05-18T00:00:01Z",
            Some(FP_A),
        ));
        events.push(task_input_request_event_with_target_and_fingerprint(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:ambig-fp",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: cp_a.clone(),
            }),
            "needs approval",
            Some(FP_A),
        ));
        // Two distinct semantic responses: each carries its own
        // `target_fingerprint`. Both are by the same writer at Operative
        // mode -- the disagreement is on the fingerprint, not the writer.
        events.push(user_response_event_with_fingerprint(
            &input_request_id,
            &r1,
            InputRequestResponseOutcome::Approved,
            AssertionMode::Operative,
            "2026-05-18T00:00:03Z",
            Some(FP_B),
        ));
        events.push(user_response_event_with_fingerprint(
            &input_request_id,
            &r2,
            InputRequestResponseOutcome::Approved,
            AssertionMode::Operative,
            "2026-05-18T00:00:04Z",
            Some(FP_C),
        ));

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();

        assert!(!projection.may_resume);
        assert_eq!(
            projection.state,
            AgentResumptionState::Ambiguous,
            "Ambiguous wins before any freshness check fires"
        );
        assert!(projection.selected_response.is_none());
        assert!(
            projection
                .diagnostics
                .iter()
                .any(|d| d.code == "agent_resumption_ambiguous_input_request_responses")
        );
    }

    #[test]
    fn agent_resumption_treats_response_fresh_when_fingerprints_match_across_different_checkpoint_ids()
     {
        // Two checkpoints share a `checkpoint_fingerprint` (same code state
        // recorded under distinct identities -- a retry or a parallel-write
        // boundary). The responder targeted the older checkpoint id but
        // carried the shared fingerprint. The substrate-level rule is:
        // agreement on opaque fingerprint overrides the identity-based
        // staleness fallback. Without the fingerprint-equality short-circuit
        // the projection would mis-read this as `CheckpointStaleNewerExists`
        // and block resumption.
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let cp_older = CheckpointId::new("checkpoint:sha256:cp-older");
        let cp_latest = CheckpointId::new("checkpoint:sha256:cp-latest");
        let input_request_id = InputRequestId::new("input-request:sha256:1");
        let response_id = InputRequestResponseId::new("input-request-response:sha256:r");

        let mut events = vec![task_attempt_event(
            &task_attempt_id,
            &session_id,
            "uuid-1",
            "2026-05-18T00:00:00Z",
        )];
        events.push(checkpoint_event_with_fingerprint(
            &task_attempt_id,
            &session_id,
            &cp_older,
            "msg_older",
            vec![],
            "2026-05-18T00:00:01Z",
            Some(FP_A),
        ));
        events.push(task_input_request_event_with_target_and_fingerprint(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:fp-eq",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: cp_older.clone(),
            }),
            "needs approval at the shared code state",
            Some(FP_A),
        ));
        events.push(checkpoint_event_with_fingerprint(
            &task_attempt_id,
            &session_id,
            &cp_latest,
            "msg_latest",
            vec![],
            "2026-05-18T00:00:03Z",
            Some(FP_A),
        ));
        events.push(user_response_event_with_fingerprint(
            &input_request_id,
            &response_id,
            InputRequestResponseOutcome::Approved,
            AssertionMode::Operative,
            "2026-05-18T00:00:04Z",
            Some(FP_A),
        ));

        let projection = agent_resumption_from_events(
            &events,
            &task_attempt_id,
            &reader_actor(),
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        )
        .unwrap();

        assert_ne!(
            projection.state,
            AgentResumptionState::Stale,
            "fingerprint agreement overrides checkpoint-id staleness"
        );
        assert_eq!(
            projection.freshness,
            Some(FreshnessBasis::CheckpointFingerprintMatches),
            "fingerprint match must be the surfaced freshness basis"
        );
        let response_view = projection
            .selected_response
            .as_ref()
            .expect("selected response surfaced");
        assert!(response_view.fresh_for_target);
        assert_eq!(response_view.target_fingerprint.as_deref(), Some(FP_A));
    }

    #[test]
    fn task_projections_preserve_fingerprint_fields_through_summary_and_input_request_views() {
        // Codex P2 follow-up. The no-info-loss claim requires that the
        // payload-level fingerprint fields survive into the sibling projection
        // views, not just into the resumption policy view. Set every
        // fingerprint field to a distinct opaque string and confirm each
        // surfaces verbatim on its intended view.
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let cp_a = CheckpointId::new("checkpoint:sha256:cp-a");
        let input_request_id = InputRequestId::new("input-request:sha256:1");

        let attempt = task_attempt_event_with_base_snapshot_fingerprint(
            &task_attempt_id,
            &session_id,
            "uuid-1",
            "2026-05-18T00:00:00Z",
            Some(FP_A),
        );
        let checkpoint = checkpoint_event_with_fingerprint(
            &task_attempt_id,
            &session_id,
            &cp_a,
            "msg_a",
            vec![],
            "2026-05-18T00:00:01Z",
            Some(FP_B),
        );
        let request = task_input_request_event_with_target_and_fingerprint(
            &task_attempt_id,
            &session_id,
            &input_request_id,
            "source:p2",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: cp_a.clone(),
            }),
            "needs approval",
            Some(FP_C),
        );

        let events = vec![attempt, checkpoint, request];

        let summary = task_attempt_summary_from_events(&events, &task_attempt_id, &reader_actor())
            .unwrap()
            .expect("attempt present");
        assert_eq!(
            summary.base_snapshot_fingerprint.as_deref(),
            Some(FP_A),
            "base snapshot fingerprint must surface on the attempt summary"
        );
        let latest = summary
            .latest_checkpoint
            .as_ref()
            .expect("latest checkpoint surfaced");
        assert_eq!(
            latest.checkpoint_fingerprint.as_deref(),
            Some(FP_B),
            "checkpoint fingerprint must surface on the latest checkpoint summary"
        );

        let input_requests =
            open_task_input_requests_from_events(&events, &task_attempt_id, &reader_actor())
                .unwrap();
        let open = input_requests
            .open_input_requests
            .first()
            .expect("open input request surfaced");
        assert_eq!(
            open.target_fingerprint.as_deref(),
            Some(FP_C),
            "input request target fingerprint must surface on the open-input-request view"
        );
    }

    fn task_attempt_event_with_base_snapshot_fingerprint(
        task_attempt_id: &WorkObjectId,
        session_id: &SessionId,
        claude_session_uuid: &str,
        occurred_at: &str,
        base_snapshot_fingerprint: Option<&str>,
    ) -> ShoreEvent {
        let target = EventTarget::for_work_object(
            session_id.clone(),
            task_attempt_id.clone(),
            WorkObjectType::TaskAttempt,
        );
        let payload = TaskAttemptCapturedPayload {
            task_attempt_id: task_attempt_id.clone(),
            project_path: "/repo".to_owned(),
            claude_session_uuid: claude_session_uuid.to_owned(),
            initial_prompt_hash: "sha256:prompt".to_owned(),
            predecessor: None,
            base_snapshot_fingerprint: base_snapshot_fingerprint.map(str::to_owned),
            source_speaker: None,
        };
        let idempotency_key = TaskAttemptCapturedPayload::idempotency_key_for_work_object(
            task_attempt_id,
            WorkObjectType::TaskAttempt,
            claude_session_uuid,
        );
        let mut event = ShoreEvent::new(
            EventType::TaskAttemptCaptured,
            idempotency_key,
            target,
            writer_user(),
            payload,
            occurred_at,
        )
        .unwrap();
        event.source_ref = Some(SourceRef::new("claude_code", claude_session_uuid));
        event.assertion_mode = AssertionMode::Advisory;
        event
    }
}
