//! Sibling task-domain projection over `ShoreEvent`s.
//!
//! Reads already-written task-domain events into a per-attempt summary.
//! `SessionState` remains review-domain; `review_history` filters task events
//! out unconditionally. This module is the sibling task entry point.

use std::collections::BTreeMap;

use crate::error::Result;
use crate::model::{
    ActorId, CheckpointId, EventId, InterventionId, InterventionResolutionId, ObservationId,
    ReviewTargetRef, TargetRef, TaskTargetRef, WorkObjectId, WorkObjectType,
};
use crate::session::event::{
    AssertionMode, EventTarget, EventType, InterventionMode, InterventionReasonCode,
    InterventionRequestedPayload, InterventionResolutionOutcome, InterventionResolvedPayload,
    ShoreEvent, SourceRef, TaskAttemptCapturedPayload, TaskCheckpointCapturedPayload,
    TaskObservationRecordedPayload, Writer,
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
    /// resolution's `target_fingerprint`.
    pub checkpoint_fingerprint: Option<String>,
    pub observations: Vec<TaskObservationSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskProjectionDiagnostic {
    pub code: String,
    pub message: String,
    pub event_id: Option<EventId>,
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
        base_snapshot_fingerprint: None,
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

/// One open task-targeted intervention. The envelope is authoritative; the
/// payload's review-shaped `target` is preserved so callers can detect the
/// current shape mismatch without losing data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskInterventionView {
    pub intervention_id: InterventionId,
    pub envelope: TaskProjectionEventEnvelope,
    pub target: TargetRef,
    pub payload_review_target: ReviewTargetRef,
    pub mode: InterventionMode,
    pub reason_code: InterventionReasonCode,
    pub title: String,
    pub body: Option<String>,
    pub body_artifact_path: Option<String>,
    pub body_byte_size: Option<u64>,
    pub body_content_hash: Option<String>,
    /// Opaque fingerprint the requester observed when raising the
    /// intervention, preserved verbatim from
    /// `InterventionRequestedPayload`.
    pub target_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskInterventionsProjection {
    pub reader_actor_id: ActorId,
    pub task_attempt_id: WorkObjectId,
    pub open_interventions: Vec<TaskInterventionView>,
    pub diagnostics: Vec<TaskProjectionDiagnostic>,
}

/// Project the unresolved `InterventionRequested` events whose durable envelope
/// targets the given `TaskAttempt`. Resolved interventions (matched by
/// `intervention_id`) are excluded.
#[allow(dead_code)]
pub(crate) fn open_task_interventions_from_events(
    events: &[ShoreEvent],
    task_attempt_id: &WorkObjectId,
    reader_actor_id: &ActorId,
) -> Result<TaskInterventionsProjection> {
    let mut requests: Vec<(
        TaskProjectionEventEnvelope,
        InterventionRequestedPayload,
        TargetRef,
    )> = Vec::new();
    let mut resolved: std::collections::BTreeSet<InterventionId> =
        std::collections::BTreeSet::new();

    for event in events {
        event.validate_schema_version()?;

        match event.event_type {
            EventType::InterventionRequested => {
                if !envelope_targets_task_attempt(&event.target, task_attempt_id) {
                    continue;
                }
                let Some(subject) = event.target.subject.clone() else {
                    continue;
                };
                if !matches!(subject, TargetRef::Task(_)) {
                    continue;
                }
                let payload: InterventionRequestedPayload =
                    serde_json::from_value(event.payload.clone())?;
                requests.push((
                    TaskProjectionEventEnvelope::from_event(event),
                    payload,
                    subject,
                ));
            }
            EventType::InterventionResolved => {
                let payload: InterventionResolvedPayload =
                    serde_json::from_value(event.payload.clone())?;
                resolved.insert(payload.intervention_id);
            }
            _ => {}
        }
    }

    let mut diagnostics: Vec<TaskProjectionDiagnostic> = Vec::new();
    let mut open_interventions: Vec<TaskInterventionView> = Vec::new();

    for (envelope, payload, subject) in requests {
        if resolved.contains(&payload.intervention_id) {
            continue;
        }

        // The payload's review-shaped `target` is preserved verbatim so callers
        // see the current shape mismatch instead of having it silently
        // promoted to the task target.
        diagnostics.push(TaskProjectionDiagnostic {
            code: "task_intervention_payload_target_is_review_shaped".to_owned(),
            message: format!(
                "intervention {} targets a TaskAttempt via the envelope but its \
                 payload `target` is still a ReviewTargetRef",
                payload.intervention_id.as_str()
            ),
            event_id: Some(envelope.event_id.clone()),
        });

        open_interventions.push(TaskInterventionView {
            intervention_id: payload.intervention_id,
            envelope,
            target: subject,
            payload_review_target: payload.target,
            mode: payload.mode,
            reason_code: payload.reason_code,
            title: payload.title,
            body: payload.body,
            body_artifact_path: payload.body_artifact_path,
            body_byte_size: payload.body_byte_size,
            body_content_hash: payload.body_content_hash,
            target_fingerprint: None,
        });
    }

    open_interventions
        .sort_by(|left, right| envelope_chronological_order(&left.envelope, &right.envelope));

    Ok(TaskInterventionsProjection {
        reader_actor_id: reader_actor_id.clone(),
        task_attempt_id: task_attempt_id.clone(),
        open_interventions,
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

/// Why a task-targeted intervention is considered fresh (or not) for the
/// resumption rule. The identity variants cover the checkpoint-by-identity
/// fallback; the fingerprint variant generalizes staleness to opaque-string
/// code-state fingerprints.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FreshnessBasis {
    /// No fingerprint check applies because the intervention targets the
    /// `TaskAttempt` itself rather than a specific checkpoint.
    TaskAttemptNoFingerprintCheck,
    /// Target checkpoint is the latest checkpoint on the attempt.
    CheckpointMatchesLatest,
    /// Target checkpoint is not the latest checkpoint on the attempt.
    CheckpointStaleNewerExists,
    /// Target was a checkpoint, but no checkpoint exists at all on the attempt.
    CheckpointWithoutAttemptCheckpoint,
    /// Resolution's `target_fingerprint` disagrees with the latest
    /// checkpoint's `checkpoint_fingerprint`; both are `Some`. Opaque-string
    /// `==` comparison only -- no domain semantics.
    CheckpointFingerprintMismatch,
    /// Resolution's `target_fingerprint` agrees with the latest checkpoint's
    /// `checkpoint_fingerprint`; both are `Some`. Fingerprint agreement
    /// overrides the identity-based staleness fallback so a resolver acting
    /// on the right code state under a stale checkpoint id stays fresh.
    CheckpointFingerprintMatches,
}

impl FreshnessBasis {
    fn is_fresh(self) -> bool {
        matches!(
            self,
            FreshnessBasis::TaskAttemptNoFingerprintCheck | FreshnessBasis::CheckpointMatchesLatest
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentResolutionPolicyView {
    pub envelope: TaskProjectionEventEnvelope,
    pub resolution_id: InterventionResolutionId,
    pub outcome: InterventionResolutionOutcome,
    pub writer_role_treated_as_binding: bool,
    pub fresh_for_target: bool,
    pub operative_reason: Option<String>,
    /// Free-text resolver justification carried verbatim from the
    /// `InterventionResolvedPayload`. Preserved here so the projection does
    /// not lose the resolver's stated reason.
    pub reason: Option<String>,
    pub reason_artifact_path: Option<String>,
    pub reason_byte_size: Option<u64>,
    pub reason_content_hash: Option<String>,
    /// Opaque fingerprint the resolver acted on, preserved verbatim from the
    /// `InterventionResolvedPayload`. The freshness rule compares this to the
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
    pub selected_intervention: Option<TaskInterventionView>,
    pub selected_resolution: Option<AgentResolutionPolicyView>,
    pub treated_as_operative: bool,
    pub freshness: Option<FreshnessBasis>,
    pub diagnostics: Vec<TaskProjectionDiagnostic>,
}

/// Read-side projection that answers: may this agent resume the named
/// `TaskAttempt`? The output is intentionally diagnostic-rich; the policy
/// never relies on a scheduler, lease, write gate, or global "current task".
#[allow(dead_code)]
pub(crate) fn agent_resumption_from_events(
    events: &[ShoreEvent],
    task_attempt_id: &WorkObjectId,
    reader_actor_id: &ActorId,
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
            selected_intervention: None,
            selected_resolution: None,
            treated_as_operative: false,
            freshness: None,
            diagnostics: vec![TaskProjectionDiagnostic {
                code: "agent_resumption_no_task_attempt".to_owned(),
                message: format!(
                    "no TaskAttemptCaptured event for {} — failing closed",
                    task_attempt_id.as_str()
                ),
                event_id: None,
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

    let intervention_records = collect_task_intervention_records(events, task_attempt_id)?;

    // Open (no representative resolution) interventions short-circuit to
    // Blocked. Pick the earliest by (occurred_at, event_id) as the selected
    // one so the diagnostic chain is deterministic.
    let open_intervention = intervention_records
        .iter()
        .filter(|record| record.resolutions.is_empty())
        .min_by(|left, right| {
            envelope_chronological_order(&left.view.envelope, &right.view.envelope)
        })
        .cloned();

    if let Some(open) = open_intervention {
        return Ok(AgentResumptionProjection {
            reader_actor_id: reader_actor_id.clone(),
            task_attempt_id: task_attempt_id.clone(),
            may_resume: false,
            state: AgentResumptionState::Blocked,
            latest_checkpoint,
            selected_intervention: Some(open.view.clone()),
            selected_resolution: None,
            treated_as_operative: false,
            freshness: None,
            diagnostics: vec![TaskProjectionDiagnostic {
                code: "agent_resumption_open_task_intervention".to_owned(),
                message: format!(
                    "task intervention {} is unresolved",
                    open.view.intervention_id.as_str()
                ),
                event_id: Some(open.view.envelope.event_id.clone()),
            }],
        });
    }

    // Ambiguous resolutions block resumption; the projection refuses to pick a
    // winner.
    let ambiguous_intervention = intervention_records
        .iter()
        .find(|record| record.resolutions.len() > 1)
        .cloned();

    if let Some(ambiguous) = ambiguous_intervention {
        return Ok(AgentResumptionProjection {
            reader_actor_id: reader_actor_id.clone(),
            task_attempt_id: task_attempt_id.clone(),
            may_resume: false,
            state: AgentResumptionState::Ambiguous,
            latest_checkpoint,
            selected_intervention: Some(ambiguous.view.clone()),
            selected_resolution: None,
            treated_as_operative: false,
            freshness: None,
            diagnostics: vec![TaskProjectionDiagnostic {
                code: "agent_resumption_ambiguous_resolutions".to_owned(),
                message: format!(
                    "task intervention {} has {} representative resolutions; projection refuses to pick a winner",
                    ambiguous.view.intervention_id.as_str(),
                    ambiguous.resolutions.len()
                ),
                event_id: Some(ambiguous.view.envelope.event_id.clone()),
            }],
        });
    }

    // Evaluate each resolved intervention against the binding/freshness rules.
    // First failure wins so the projection surfaces it.
    let mut last_satisfied: Option<(
        TaskInterventionView,
        AgentResolutionPolicyView,
        FreshnessBasis,
    )> = None;
    for record in &intervention_records {
        let resolution = record
            .resolutions
            .first()
            .expect("non-empty after open / ambiguous branches");

        let freshness = freshness_for_task_target(
            &record.view,
            latest_checkpoint.as_ref(),
            latest_checkpoint_fingerprint.as_deref(),
            resolution.target_fingerprint.as_deref(),
        );
        let writer_role_binding =
            resolution_writer_role_is_binding(&resolution.envelope.writer.role);
        let outcome_allows = matches!(resolution.outcome, InterventionResolutionOutcome::Approved);
        let operative = resolution.envelope.assertion_mode == AssertionMode::Operative;

        let operative_reason = if writer_role_binding && operative && outcome_allows {
            Some("approved by User writer with envelope-level operative assertion mode".to_owned())
        } else {
            None
        };

        let resolution_view = AgentResolutionPolicyView {
            envelope: resolution.envelope.clone(),
            resolution_id: resolution.resolution_id.clone(),
            outcome: resolution.outcome,
            writer_role_treated_as_binding: writer_role_binding,
            fresh_for_target: freshness.is_fresh(),
            operative_reason,
            reason: resolution.reason.clone(),
            reason_artifact_path: resolution.reason_artifact_path.clone(),
            reason_byte_size: resolution.reason_byte_size,
            reason_content_hash: resolution.reason_content_hash.clone(),
            target_fingerprint: resolution.target_fingerprint.clone(),
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
                    code: "agent_resumption_resolution_target_fingerprint_mismatch".to_owned(),
                    message: format!(
                        "task intervention {} resolved against a code state whose fingerprint disagrees with the latest checkpoint on this attempt",
                        record.view.intervention_id.as_str()
                    ),
                    event_id: Some(resolution.envelope.event_id.clone()),
                },
                _ => TaskProjectionDiagnostic {
                    code: "agent_resumption_resolution_targets_stale_checkpoint".to_owned(),
                    message: format!(
                        "task intervention {} resolved against an older checkpoint than the latest checkpoint on this attempt",
                        record.view.intervention_id.as_str()
                    ),
                    event_id: Some(resolution.envelope.event_id.clone()),
                },
            };
            return Ok(AgentResumptionProjection {
                reader_actor_id: reader_actor_id.clone(),
                task_attempt_id: task_attempt_id.clone(),
                may_resume: false,
                state: AgentResumptionState::Stale,
                latest_checkpoint,
                selected_intervention: Some(record.view.clone()),
                selected_resolution: Some(resolution_view),
                treated_as_operative: false,
                freshness: Some(freshness),
                diagnostics: vec![diagnostic],
            });
        }

        if !(writer_role_binding && operative && outcome_allows) {
            let mut diagnostics = Vec::new();
            if !outcome_allows {
                diagnostics.push(TaskProjectionDiagnostic {
                    code: "agent_resumption_outcome_not_approved".to_owned(),
                    message: format!(
                        "resolution outcome is {:?}; only Approved permits resumption",
                        resolution.outcome
                    ),
                    event_id: Some(resolution.envelope.event_id.clone()),
                });
            }
            if !operative {
                diagnostics.push(TaskProjectionDiagnostic {
                    code: "agent_resumption_resolution_assertion_mode_not_operative".to_owned(),
                    message:
                        "resolution envelope assertion_mode is not Operative; advisory resolutions do not bind"
                            .to_owned(),
                    event_id: Some(resolution.envelope.event_id.clone()),
                });
            }
            if !writer_role_binding {
                diagnostics.push(TaskProjectionDiagnostic {
                    code: "agent_resumption_resolution_writer_role_not_binding".to_owned(),
                    message: format!(
                        "resolution writer role {:?} does not bind; only User binds at this revision",
                        resolution.envelope.writer.role
                    ),
                    event_id: Some(resolution.envelope.event_id.clone()),
                });
            }
            return Ok(AgentResumptionProjection {
                reader_actor_id: reader_actor_id.clone(),
                task_attempt_id: task_attempt_id.clone(),
                may_resume: false,
                state: AgentResumptionState::Blocked,
                latest_checkpoint,
                selected_intervention: Some(record.view.clone()),
                selected_resolution: Some(resolution_view),
                treated_as_operative: false,
                freshness: Some(freshness),
                diagnostics,
            });
        }

        // This intervention is satisfied. Track the most recent satisfied
        // resolution for diagnostic completeness.
        match &last_satisfied {
            Some((_, prev_resolution_view, _))
                if envelope_chronological_order(
                    &prev_resolution_view.envelope,
                    &resolution_view.envelope,
                )
                .is_ge() => {}
            _ => {
                last_satisfied = Some((record.view.clone(), resolution_view, freshness));
            }
        }
    }

    // No open, ambiguous, stale, or non-binding intervention found.
    let (selected_intervention, selected_resolution, freshness, treated_as_operative) =
        match last_satisfied {
            Some((view, resolution_view, freshness)) => {
                (Some(view), Some(resolution_view), Some(freshness), true)
            }
            None => (None, None, None, false),
        };

    Ok(AgentResumptionProjection {
        reader_actor_id: reader_actor_id.clone(),
        task_attempt_id: task_attempt_id.clone(),
        may_resume: true,
        state: AgentResumptionState::Ready,
        latest_checkpoint,
        selected_intervention,
        selected_resolution,
        treated_as_operative,
        freshness,
        diagnostics: Vec::new(),
    })
}

#[derive(Clone, Debug)]
struct TaskInterventionRecord {
    view: TaskInterventionView,
    resolutions: Vec<TaskResolutionRecord>,
}

#[derive(Clone, Debug)]
struct TaskResolutionRecord {
    envelope: TaskProjectionEventEnvelope,
    resolution_id: InterventionResolutionId,
    outcome: InterventionResolutionOutcome,
    reason: Option<String>,
    reason_artifact_path: Option<String>,
    reason_byte_size: Option<u64>,
    reason_content_hash: Option<String>,
    target_fingerprint: Option<String>,
}

fn collect_task_intervention_records(
    events: &[ShoreEvent],
    task_attempt_id: &WorkObjectId,
) -> Result<Vec<TaskInterventionRecord>> {
    let mut request_views: Vec<TaskInterventionView> = Vec::new();
    // Collapse duplicate semantic resolution facts by
    // `intervention_resolution_id`: two events with the same resolution id
    // are retry duplicates, not distinct resolutions. This mirrors the
    // representative-selection convention in
    // `src/session/workflow/intervention/view.rs:256`. The map value carries
    // the owning `intervention_id` so the rollup below does not need a
    // second pass over `events`.
    let mut resolution_representatives: BTreeMap<
        InterventionResolutionId,
        (InterventionId, TaskResolutionRecord),
    > = BTreeMap::new();

    for event in events {
        event.validate_schema_version()?;

        match event.event_type {
            EventType::InterventionRequested => {
                if !envelope_targets_task_attempt(&event.target, task_attempt_id) {
                    continue;
                }
                let Some(subject) = event.target.subject.clone() else {
                    continue;
                };
                if !matches!(subject, TargetRef::Task(_)) {
                    continue;
                }
                let payload: InterventionRequestedPayload =
                    serde_json::from_value(event.payload.clone())?;
                request_views.push(TaskInterventionView {
                    intervention_id: payload.intervention_id,
                    envelope: TaskProjectionEventEnvelope::from_event(event),
                    target: subject,
                    payload_review_target: payload.target,
                    mode: payload.mode,
                    reason_code: payload.reason_code,
                    title: payload.title,
                    body: payload.body,
                    body_artifact_path: payload.body_artifact_path,
                    body_byte_size: payload.body_byte_size,
                    body_content_hash: payload.body_content_hash,
                    target_fingerprint: None,
                });
            }
            EventType::InterventionResolved => {
                let payload: InterventionResolvedPayload =
                    serde_json::from_value(event.payload.clone())?;
                let intervention_id = payload.intervention_id.clone();
                let resolution_id = payload.intervention_resolution_id.clone();
                let record = TaskResolutionRecord {
                    envelope: TaskProjectionEventEnvelope::from_event(event),
                    resolution_id: resolution_id.clone(),
                    outcome: payload.outcome,
                    reason: payload.reason,
                    reason_artifact_path: payload.reason_artifact_path,
                    reason_byte_size: payload.reason_byte_size,
                    reason_content_hash: payload.reason_content_hash,
                    target_fingerprint: payload.target_fingerprint,
                };
                resolution_representatives
                    .entry(resolution_id)
                    .and_modify(|(_, existing)| {
                        if record.envelope.event_id.as_str() < existing.envelope.event_id.as_str() {
                            *existing = record.clone();
                        }
                    })
                    .or_insert((intervention_id, record));
            }
            _ => {}
        }
    }

    let mut resolutions_by_intervention: BTreeMap<InterventionId, Vec<TaskResolutionRecord>> =
        BTreeMap::new();
    for (intervention_id, resolution) in resolution_representatives.into_values() {
        resolutions_by_intervention
            .entry(intervention_id)
            .or_default()
            .push(resolution);
    }

    let mut records = Vec::with_capacity(request_views.len());
    for view in request_views {
        let resolutions = resolutions_by_intervention
            .remove(&view.intervention_id)
            .unwrap_or_default();
        records.push(TaskInterventionRecord { view, resolutions });
    }
    Ok(records)
}

fn freshness_for_task_target(
    view: &TaskInterventionView,
    latest_checkpoint: Option<&CheckpointId>,
    latest_checkpoint_fingerprint: Option<&str>,
    resolution_fingerprint: Option<&str>,
) -> FreshnessBasis {
    // When the writer recorded a fingerprint on both sides, opaque-string
    // equality is the freshness signal -- the resolver's `target_fingerprint`
    // is compared to the latest checkpoint's `checkpoint_fingerprint`. The
    // identity-based fallback below handles every other shape.
    if let (Some(latest_fp), Some(resolution_fp)) =
        (latest_checkpoint_fingerprint, resolution_fingerprint)
        && latest_fp != resolution_fp
    {
        return FreshnessBasis::CheckpointFingerprintMismatch;
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

fn resolution_writer_role_is_binding(role: &crate::session::event::WriterRole) -> bool {
    matches!(role, crate::session::event::WriterRole::User)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical_hash::sha256_bytes_hex;
    use crate::model::{
        ActorId, CheckpointId, ObservationId, SessionId, TargetRef, TaskTargetRef, WorkObjectId,
    };
    use crate::session::event::{
        AssertionMode, EventTarget, EventType, ShoreEvent, SourceRef, TaskAttemptCapturedPayload,
        TaskCheckpointCapturedPayload, TaskObservationRecordedPayload, Writer, WriterRole,
        WriterTool,
    };

    fn writer_user() -> Writer {
        Writer {
            actor_id: ActorId::new("actor:claude_code:user"),
            role: WriterRole::User,
            tool: WriterTool {
                name: "claude_code".to_owned(),
                version: String::new(),
            },
        }
    }

    fn reader_actor() -> ActorId {
        ActorId::new("actor:shore:reader")
    }

    fn task_attempt_event(
        task_attempt_id: &WorkObjectId,
        session_id: &SessionId,
        claude_session_uuid: &str,
        occurred_at: &str,
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
            base_snapshot_fingerprint: None,
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

    fn checkpoint_event(
        task_attempt_id: &WorkObjectId,
        session_id: &SessionId,
        checkpoint_id: &CheckpointId,
        assistant_message_id: &str,
        tool_use_ids: Vec<String>,
        occurred_at: &str,
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
            checkpoint_fingerprint: None,
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
            Writer::shore_local_reviewer("test"),
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
            Writer::shore_local_reviewer("test"),
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

    // -- open_task_interventions -------------------------------------------

    use crate::model::{
        InterventionId, InterventionResolutionId, ReviewTargetRef, ReviewUnitId, TrackId,
    };
    use crate::session::event::{
        InterventionMode, InterventionReasonCode, InterventionRequestedPayload,
        InterventionResolutionOutcome, InterventionResolvedPayload,
    };

    #[allow(clippy::too_many_arguments)]
    fn task_intervention_event(
        task_attempt_id: &WorkObjectId,
        session_id: &SessionId,
        intervention_id: &InterventionId,
        source_key: &str,
        occurred_at: &str,
        mode: InterventionMode,
        reason_code: InterventionReasonCode,
        title: &str,
    ) -> ShoreEvent {
        let mut target = EventTarget::for_work_object(
            session_id.clone(),
            task_attempt_id.clone(),
            WorkObjectType::TaskAttempt,
        );
        target.subject = Some(TargetRef::Task(TaskTargetRef::TaskAttempt));
        // Current `InterventionRequestedPayload.target` is review-shaped. The
        // task envelope is authoritative; this placeholder is preserved by the
        // projection only as diagnostic evidence.
        let payload = InterventionRequestedPayload {
            intervention_id: intervention_id.clone(),
            target: ReviewTargetRef::ReviewUnit {
                review_unit_id: ReviewUnitId::new("review-unit:placeholder"),
            },
            mode,
            reason_code,
            title: title.to_owned(),
            body: None,
            body_artifact_path: None,
            body_byte_size: None,
            body_content_hash: None,
            target_fingerprint: None,
        };
        let idempotency_key = InterventionRequestedPayload::idempotency_key_for_work_object(
            task_attempt_id,
            WorkObjectType::TaskAttempt,
            source_key,
        );
        let mut event = ShoreEvent::new(
            EventType::InterventionRequested,
            idempotency_key,
            target,
            Writer::shore_local_reviewer("test"),
            payload,
            occurred_at,
        )
        .unwrap();
        event.source_ref = Some(SourceRef::new("claude_code", source_key));
        event.assertion_mode = AssertionMode::Advisory;
        event
    }

    fn task_intervention_resolved_event(
        task_attempt_id: &WorkObjectId,
        session_id: &SessionId,
        intervention_id: &InterventionId,
        resolution_id: &InterventionResolutionId,
        occurred_at: &str,
    ) -> ShoreEvent {
        let target = EventTarget::for_work_object(
            session_id.clone(),
            task_attempt_id.clone(),
            WorkObjectType::TaskAttempt,
        );
        let payload = InterventionResolvedPayload {
            intervention_resolution_id: resolution_id.clone(),
            intervention_id: intervention_id.clone(),
            outcome: InterventionResolutionOutcome::Approved,
            reason: None,
            reason_artifact_path: None,
            reason_byte_size: None,
            reason_content_hash: None,
            target_fingerprint: None,
        };
        let idempotency_key =
            InterventionResolvedPayload::idempotency_key(intervention_id, resolution_id.as_str());
        ShoreEvent::new(
            EventType::InterventionResolved,
            idempotency_key,
            target,
            Writer::shore_local_reviewer("test"),
            payload,
            occurred_at,
        )
        .unwrap()
    }

    fn review_intervention_event(
        review_unit_id: &ReviewUnitId,
        track_id: &TrackId,
        intervention_id: &InterventionId,
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
        let payload = InterventionRequestedPayload {
            intervention_id: intervention_id.clone(),
            target: ReviewTargetRef::ReviewUnit {
                review_unit_id: review_unit_id.clone(),
            },
            mode: InterventionMode::Blocking,
            reason_code: InterventionReasonCode::ManualDecisionRequired,
            title: "review-domain".to_owned(),
            body: None,
            body_artifact_path: None,
            body_byte_size: None,
            body_content_hash: None,
            target_fingerprint: None,
        };
        let idempotency_key =
            InterventionRequestedPayload::idempotency_key(review_unit_id, track_id, source_key);
        ShoreEvent::new(
            EventType::InterventionRequested,
            idempotency_key,
            target,
            Writer::shore_local_reviewer("test"),
            payload,
            occurred_at,
        )
        .unwrap()
    }

    #[test]
    fn open_task_interventions_returns_task_targeted_unresolved_requests() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let intervention_id = InterventionId::new("intervention:sha256:1");

        let events = vec![task_intervention_event(
            &task_attempt_id,
            &session_id,
            &intervention_id,
            "source:1",
            "2026-05-18T00:00:00Z",
            InterventionMode::Blocking,
            InterventionReasonCode::ManualDecisionRequired,
            "Need a call",
        )];

        let projection =
            open_task_interventions_from_events(&events, &task_attempt_id, &reader_actor())
                .unwrap();

        assert_eq!(projection.task_attempt_id, task_attempt_id);
        assert_eq!(projection.reader_actor_id, reader_actor());
        assert_eq!(projection.open_interventions.len(), 1);
        let view = &projection.open_interventions[0];
        assert_eq!(view.intervention_id, intervention_id);
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
    fn open_task_interventions_excludes_resolved_intervention_ids() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let intervention_id = InterventionId::new("intervention:sha256:1");
        let resolution_id = InterventionResolutionId::new("intervention-resolution:sha256:r1");

        let events = vec![
            task_intervention_event(
                &task_attempt_id,
                &session_id,
                &intervention_id,
                "source:1",
                "2026-05-18T00:00:00Z",
                InterventionMode::Blocking,
                InterventionReasonCode::ManualDecisionRequired,
                "Need a call",
            ),
            task_intervention_resolved_event(
                &task_attempt_id,
                &session_id,
                &intervention_id,
                &resolution_id,
                "2026-05-18T00:00:01Z",
            ),
        ];

        let projection =
            open_task_interventions_from_events(&events, &task_attempt_id, &reader_actor())
                .unwrap();

        assert!(
            projection.open_interventions.is_empty(),
            "resolved intervention must not appear in open set; got {:?}",
            projection.open_interventions
        );
    }

    #[test]
    fn open_task_interventions_ignores_review_domain_requests() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let review_unit_id = ReviewUnitId::new("review-unit:sha256:u");
        let track_id = TrackId::new("agent:codex");
        let task_intervention_id = InterventionId::new("intervention:sha256:task");
        let review_intervention_id = InterventionId::new("intervention:sha256:review");

        let events = vec![
            task_intervention_event(
                &task_attempt_id,
                &SessionId::new("session:claude:uuid-1"),
                &task_intervention_id,
                "source:task",
                "2026-05-18T00:00:00Z",
                InterventionMode::Advisory,
                InterventionReasonCode::FailedGate,
                "task-domain",
            ),
            review_intervention_event(
                &review_unit_id,
                &track_id,
                &review_intervention_id,
                "source:review",
                "2026-05-18T00:00:00Z",
            ),
        ];

        let projection =
            open_task_interventions_from_events(&events, &task_attempt_id, &reader_actor())
                .unwrap();

        assert_eq!(projection.open_interventions.len(), 1);
        assert_eq!(
            projection.open_interventions[0].intervention_id,
            task_intervention_id
        );
        for view in &projection.open_interventions {
            assert_ne!(view.intervention_id, review_intervention_id);
        }
    }

    #[test]
    fn open_task_interventions_preserves_payload_target_mismatch_as_diagnostic() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let intervention_id = InterventionId::new("intervention:sha256:1");

        let events = vec![task_intervention_event(
            &task_attempt_id,
            &session_id,
            &intervention_id,
            "source:1",
            "2026-05-18T00:00:00Z",
            InterventionMode::Blocking,
            InterventionReasonCode::ManualDecisionRequired,
            "Need a call",
        )];

        let projection =
            open_task_interventions_from_events(&events, &task_attempt_id, &reader_actor())
                .unwrap();

        assert_eq!(projection.open_interventions.len(), 1);
        let diag = projection
            .diagnostics
            .iter()
            .find(|d| d.code == "task_intervention_payload_target_is_review_shaped")
            .expect("payload target mismatch diagnostic is emitted");
        assert_eq!(
            diag.event_id,
            Some(projection.open_interventions[0].envelope.event_id.clone())
        );
    }

    #[test]
    fn open_task_interventions_preserves_envelope_policy_fields() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let intervention_id = InterventionId::new("intervention:sha256:1");

        let request_event = task_intervention_event(
            &task_attempt_id,
            &session_id,
            &intervention_id,
            "source:1",
            "2026-05-18T00:00:00Z",
            InterventionMode::Blocking,
            InterventionReasonCode::ManualDecisionRequired,
            "Need a call",
        );

        let events = vec![request_event.clone()];

        let projection =
            open_task_interventions_from_events(&events, &task_attempt_id, &reader_actor())
                .unwrap();

        let view = &projection.open_interventions[0];
        assert_eq!(view.envelope.event_id, request_event.event_id);
        assert_eq!(view.envelope.event_type, EventType::InterventionRequested);
        assert_eq!(view.envelope.occurred_at, request_event.occurred_at);
        assert_eq!(view.envelope.payload_hash, request_event.payload_hash);
        assert_eq!(view.envelope.writer, request_event.writer);
        assert_eq!(view.envelope.assertion_mode, AssertionMode::Advisory);
        assert_eq!(view.envelope.source_ref, request_event.source_ref);
        assert_eq!(view.envelope.target, request_event.target);
        assert_eq!(view.mode, InterventionMode::Blocking);
        assert_eq!(
            view.reason_code,
            InterventionReasonCode::ManualDecisionRequired
        );
        assert_eq!(view.title, "Need a call");
        assert_eq!(view.body, None);
        assert_eq!(view.body_artifact_path, None);
        assert_eq!(view.body_byte_size, None);
        assert_eq!(view.body_content_hash, None);
    }

    #[test]
    fn open_task_interventions_separates_multiple_open_requests() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let a = InterventionId::new("intervention:sha256:a");
        let b = InterventionId::new("intervention:sha256:b");

        let events = vec![
            task_intervention_event(
                &task_attempt_id,
                &session_id,
                &a,
                "source:a",
                "2026-05-18T00:00:00Z",
                InterventionMode::Advisory,
                InterventionReasonCode::FailedGate,
                "first",
            ),
            task_intervention_event(
                &task_attempt_id,
                &session_id,
                &b,
                "source:b",
                "2026-05-18T00:00:01Z",
                InterventionMode::Blocking,
                InterventionReasonCode::ManualDecisionRequired,
                "second",
            ),
        ];

        let projection =
            open_task_interventions_from_events(&events, &task_attempt_id, &reader_actor())
                .unwrap();

        let ids: Vec<InterventionId> = projection
            .open_interventions
            .iter()
            .map(|view| view.intervention_id.clone())
            .collect();
        assert!(ids.contains(&a), "first intervention should be present");
        assert!(ids.contains(&b), "second intervention should be present");
        assert_eq!(ids.len(), 2, "no collapse by target");
    }

    // -- agent_resumption --------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn task_intervention_event_with_target(
        task_attempt_id: &WorkObjectId,
        session_id: &SessionId,
        intervention_id: &InterventionId,
        source_key: &str,
        occurred_at: &str,
        subject: TargetRef,
        title: &str,
    ) -> ShoreEvent {
        let mut target = EventTarget::for_work_object(
            session_id.clone(),
            task_attempt_id.clone(),
            WorkObjectType::TaskAttempt,
        );
        target.subject = Some(subject);
        let payload = InterventionRequestedPayload {
            intervention_id: intervention_id.clone(),
            target: ReviewTargetRef::ReviewUnit {
                review_unit_id: ReviewUnitId::new("review-unit:placeholder"),
            },
            mode: InterventionMode::Blocking,
            reason_code: InterventionReasonCode::ManualDecisionRequired,
            title: title.to_owned(),
            body: None,
            body_artifact_path: None,
            body_byte_size: None,
            body_content_hash: None,
            target_fingerprint: None,
        };
        let idempotency_key = InterventionRequestedPayload::idempotency_key_for_work_object(
            task_attempt_id,
            WorkObjectType::TaskAttempt,
            source_key,
        );
        let mut event = ShoreEvent::new(
            EventType::InterventionRequested,
            idempotency_key,
            target,
            Writer::shore_local_reviewer("test"),
            payload,
            occurred_at,
        )
        .unwrap();
        event.source_ref = Some(SourceRef::new("claude_code", source_key));
        event.assertion_mode = AssertionMode::Advisory;
        event
    }

    fn user_resolution_event(
        intervention_id: &InterventionId,
        resolution_id: &InterventionResolutionId,
        outcome: InterventionResolutionOutcome,
        assertion_mode: AssertionMode,
        writer_role: WriterRole,
        occurred_at: &str,
    ) -> ShoreEvent {
        let target = EventTarget::for_work_object(
            SessionId::new("session:claude:uuid-1"),
            WorkObjectId::new("task-attempt:sha256:ta"),
            WorkObjectType::TaskAttempt,
        );
        let payload = InterventionResolvedPayload {
            intervention_resolution_id: resolution_id.clone(),
            intervention_id: intervention_id.clone(),
            outcome,
            reason: None,
            reason_artifact_path: None,
            reason_byte_size: None,
            reason_content_hash: None,
            target_fingerprint: None,
        };
        let idempotency_key =
            InterventionResolvedPayload::idempotency_key(intervention_id, resolution_id.as_str());
        let writer = Writer {
            actor_id: ActorId::new("actor:claude_code:user"),
            role: writer_role,
            tool: WriterTool {
                name: "claude_code".to_owned(),
                version: String::new(),
            },
        };
        let mut event = ShoreEvent::new(
            EventType::InterventionResolved,
            idempotency_key,
            target,
            writer,
            payload,
            occurred_at,
        )
        .unwrap();
        event.assertion_mode = assertion_mode;
        event.source_ref = Some(SourceRef::new("claude_code", resolution_id.as_str()));
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
    fn agent_resumption_allows_resume_when_no_task_interventions_exist() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let checkpoint = CheckpointId::new("checkpoint:sha256:cp");

        let events = attempt_with_checkpoints(
            &task_attempt_id,
            &session_id,
            &[(&checkpoint, "msg_1", "2026-05-18T00:00:01Z")],
        );

        let projection =
            agent_resumption_from_events(&events, &task_attempt_id, &reader_actor()).unwrap();

        assert!(projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ready);
        assert_eq!(projection.latest_checkpoint, Some(checkpoint));
        assert!(projection.selected_intervention.is_none());
        assert!(projection.selected_resolution.is_none());
    }

    #[test]
    fn agent_resumption_pauses_for_open_task_intervention() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let checkpoint = CheckpointId::new("checkpoint:sha256:cp");
        let intervention_id = InterventionId::new("intervention:sha256:1");

        let mut events = attempt_with_checkpoints(
            &task_attempt_id,
            &session_id,
            &[(&checkpoint, "msg_1", "2026-05-18T00:00:01Z")],
        );
        events.push(task_intervention_event_with_target(
            &task_attempt_id,
            &session_id,
            &intervention_id,
            "source:open",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::TaskAttempt),
            "open call",
        ));

        let projection =
            agent_resumption_from_events(&events, &task_attempt_id, &reader_actor()).unwrap();

        assert!(!projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Blocked);
        let selected = projection
            .selected_intervention
            .as_ref()
            .expect("selected intervention");
        assert_eq!(selected.intervention_id, intervention_id);
        assert!(projection.selected_resolution.is_none());
        assert!(
            projection
                .diagnostics
                .iter()
                .any(|d| d.code == "agent_resumption_open_task_intervention"),
            "diagnostic explains the open intervention"
        );
    }

    #[test]
    fn agent_resumption_allows_fresh_operative_user_approval() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let checkpoint = CheckpointId::new("checkpoint:sha256:cp");
        let intervention_id = InterventionId::new("intervention:sha256:1");
        let resolution_id = InterventionResolutionId::new("intervention-resolution:sha256:r");

        let mut events = attempt_with_checkpoints(
            &task_attempt_id,
            &session_id,
            &[(&checkpoint, "msg_1", "2026-05-18T00:00:01Z")],
        );
        events.push(task_intervention_event_with_target(
            &task_attempt_id,
            &session_id,
            &intervention_id,
            "source:approve",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: checkpoint.clone(),
            }),
            "needs approval",
        ));
        events.push(user_resolution_event(
            &intervention_id,
            &resolution_id,
            InterventionResolutionOutcome::Approved,
            AssertionMode::Operative,
            WriterRole::User,
            "2026-05-18T00:00:03Z",
        ));

        let projection =
            agent_resumption_from_events(&events, &task_attempt_id, &reader_actor()).unwrap();

        assert!(projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ready);
        assert!(projection.treated_as_operative);
        let resolution_view = projection
            .selected_resolution
            .as_ref()
            .expect("selected resolution");
        assert_eq!(resolution_view.resolution_id, resolution_id);
        assert_eq!(
            resolution_view.outcome,
            InterventionResolutionOutcome::Approved
        );
        assert!(resolution_view.writer_role_treated_as_binding);
        assert!(resolution_view.fresh_for_target);
        assert!(resolution_view.operative_reason.is_some());
        assert_eq!(
            projection.freshness,
            Some(FreshnessBasis::CheckpointMatchesLatest)
        );
    }

    #[test]
    fn agent_resumption_fails_closed_for_advisory_resolution() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let intervention_id = InterventionId::new("intervention:sha256:1");
        let resolution_id = InterventionResolutionId::new("intervention-resolution:sha256:r");

        let mut events = attempt_with_checkpoints(&task_attempt_id, &session_id, &[]);
        events.push(task_intervention_event_with_target(
            &task_attempt_id,
            &session_id,
            &intervention_id,
            "source:adv",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::TaskAttempt),
            "needs approval",
        ));
        events.push(user_resolution_event(
            &intervention_id,
            &resolution_id,
            InterventionResolutionOutcome::Approved,
            AssertionMode::Advisory,
            WriterRole::User,
            "2026-05-18T00:00:03Z",
        ));

        let projection =
            agent_resumption_from_events(&events, &task_attempt_id, &reader_actor()).unwrap();
        assert!(!projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Blocked);
        assert!(!projection.treated_as_operative);
    }

    #[test]
    fn agent_resumption_fails_closed_for_agent_written_resolution() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let intervention_id = InterventionId::new("intervention:sha256:1");
        let resolution_id = InterventionResolutionId::new("intervention-resolution:sha256:r");

        let mut events = attempt_with_checkpoints(&task_attempt_id, &session_id, &[]);
        events.push(task_intervention_event_with_target(
            &task_attempt_id,
            &session_id,
            &intervention_id,
            "source:agent",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::TaskAttempt),
            "needs approval",
        ));
        events.push(user_resolution_event(
            &intervention_id,
            &resolution_id,
            InterventionResolutionOutcome::Approved,
            AssertionMode::Operative,
            WriterRole::Agent,
            "2026-05-18T00:00:03Z",
        ));

        let projection =
            agent_resumption_from_events(&events, &task_attempt_id, &reader_actor()).unwrap();
        assert!(!projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Blocked);
        assert!(!projection.treated_as_operative);
    }

    #[test]
    fn agent_resumption_fails_closed_for_ambiguous_resolutions() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let intervention_id = InterventionId::new("intervention:sha256:1");
        let r1 = InterventionResolutionId::new("intervention-resolution:sha256:r1");
        let r2 = InterventionResolutionId::new("intervention-resolution:sha256:r2");

        let mut events = attempt_with_checkpoints(&task_attempt_id, &session_id, &[]);
        events.push(task_intervention_event_with_target(
            &task_attempt_id,
            &session_id,
            &intervention_id,
            "source:ambig",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::TaskAttempt),
            "needs approval",
        ));
        events.push(user_resolution_event(
            &intervention_id,
            &r1,
            InterventionResolutionOutcome::Approved,
            AssertionMode::Operative,
            WriterRole::User,
            "2026-05-18T00:00:03Z",
        ));
        events.push(user_resolution_event(
            &intervention_id,
            &r2,
            InterventionResolutionOutcome::Rejected,
            AssertionMode::Operative,
            WriterRole::User,
            "2026-05-18T00:00:04Z",
        ));

        let projection =
            agent_resumption_from_events(&events, &task_attempt_id, &reader_actor()).unwrap();

        assert!(!projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ambiguous);
        assert!(
            projection
                .diagnostics
                .iter()
                .any(|d| d.code == "agent_resumption_ambiguous_resolutions"),
            "diagnostic explains the ambiguity"
        );
        assert!(projection.selected_resolution.is_none());
    }

    #[test]
    fn agent_resumption_marks_checkpoint_resolution_stale_when_newer_checkpoint_exists() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let cp_a = CheckpointId::new("checkpoint:sha256:cp-a");
        let cp_b = CheckpointId::new("checkpoint:sha256:cp-b");
        let intervention_id = InterventionId::new("intervention:sha256:1");
        let resolution_id = InterventionResolutionId::new("intervention-resolution:sha256:r");

        let mut events = attempt_with_checkpoints(
            &task_attempt_id,
            &session_id,
            &[
                (&cp_a, "msg_a", "2026-05-18T00:00:01Z"),
                (&cp_b, "msg_b", "2026-05-18T00:00:05Z"),
            ],
        );
        events.push(task_intervention_event_with_target(
            &task_attempt_id,
            &session_id,
            &intervention_id,
            "source:stale",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: cp_a.clone(),
            }),
            "needs approval at checkpoint a",
        ));
        events.push(user_resolution_event(
            &intervention_id,
            &resolution_id,
            InterventionResolutionOutcome::Approved,
            AssertionMode::Operative,
            WriterRole::User,
            "2026-05-18T00:00:03Z",
        ));

        let projection =
            agent_resumption_from_events(&events, &task_attempt_id, &reader_actor()).unwrap();
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
        let intervention_id = InterventionId::new("intervention:sha256:1");
        let resolution_id = InterventionResolutionId::new("intervention-resolution:sha256:r");

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
        let request = task_intervention_event_with_target(
            &task_attempt_id,
            &session_id,
            &intervention_id,
            "source:approve",
            "2026-05-18T00:00:05Z",
            TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: cp_b.clone(),
            }),
            "needs approval",
        );
        let resolution = user_resolution_event_with_reason(
            &intervention_id,
            &resolution_id,
            InterventionResolutionOutcome::Approved,
            AssertionMode::Operative,
            WriterRole::User,
            "2026-05-18T00:00:06Z",
            Some("approved by reviewer".to_owned()),
            Some("artifacts/resolutions/r.txt".to_owned()),
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
            resolution.clone(),
        ];

        let summary = task_attempt_summary_from_events(&events, &task_attempt_id, &reader_actor())
            .unwrap()
            .expect("attempt present");
        let interventions =
            open_task_interventions_from_events(&events, &task_attempt_id, &reader_actor())
                .unwrap();
        let resumption =
            agent_resumption_from_events(&events, &task_attempt_id, &reader_actor()).unwrap();

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

        let selected_intervention = resumption
            .selected_intervention
            .as_ref()
            .expect("intervention surfaced");
        assert_eq!(selected_intervention.envelope.event_id, request.event_id);
        let selected_resolution = resumption
            .selected_resolution
            .as_ref()
            .expect("resolution surfaced");
        assert_eq!(selected_resolution.envelope.event_id, resolution.event_id);

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

        assert_eq!(selected_intervention.intervention_id, intervention_id);
        assert_eq!(selected_resolution.resolution_id, resolution_id);

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
            selected_resolution.envelope.assertion_mode,
            AssertionMode::Operative
        );
        assert_eq!(
            selected_resolution.envelope.source_ref,
            resolution.source_ref
        );
        assert_eq!(selected_resolution.envelope.writer, resolution.writer);
        assert_eq!(selected_resolution.envelope.target, resolution.target);
        assert_eq!(
            selected_resolution.envelope.payload_hash,
            resolution.payload_hash
        );
        assert_eq!(
            selected_resolution.envelope.occurred_at,
            resolution.occurred_at
        );

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

        assert_eq!(selected_intervention.title, "needs approval");
        assert_eq!(selected_intervention.mode, InterventionMode::Blocking);
        assert_eq!(
            selected_intervention.reason_code,
            InterventionReasonCode::ManualDecisionRequired
        );

        // Resolution payload reason fields must survive into the policy view.
        assert_eq!(
            selected_resolution.reason.as_deref(),
            Some("approved by reviewer")
        );
        assert_eq!(
            selected_resolution.reason_artifact_path.as_deref(),
            Some("artifacts/resolutions/r.txt")
        );
        assert_eq!(selected_resolution.reason_byte_size, Some(19));
        assert_eq!(
            selected_resolution.reason_content_hash.as_deref(),
            Some("sha256:reason")
        );

        // The payload's review-shaped `target` field is preserved as
        // diagnostic evidence by `open_task_interventions` when the
        // intervention is still open. Re-run without the resolution to
        // confirm the diagnostic fires for unresolved task interventions.
        let events_without_resolution = vec![
            attempt.clone(),
            cp_a_event.clone(),
            cp_b_event.clone(),
            obs_a.clone(),
            obs_b.clone(),
            request.clone(),
        ];
        let interventions_open_only = open_task_interventions_from_events(
            &events_without_resolution,
            &task_attempt_id,
            &reader_actor(),
        )
        .unwrap();
        assert!(
            interventions_open_only
                .diagnostics
                .iter()
                .any(|d| d.code == "task_intervention_payload_target_is_review_shaped"),
            "payload target mismatch must be visible as diagnostic for open intervention"
        );

        // Once resolved, the intervention drops out of the open set.
        assert!(interventions.open_interventions.is_empty());

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
        let projection =
            agent_resumption_from_events(&[], &task_attempt_id, &reader_actor()).unwrap();
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
    fn user_resolution_event_with_reason(
        intervention_id: &InterventionId,
        resolution_id: &InterventionResolutionId,
        outcome: InterventionResolutionOutcome,
        assertion_mode: AssertionMode,
        writer_role: WriterRole,
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
        let payload = InterventionResolvedPayload {
            intervention_resolution_id: resolution_id.clone(),
            intervention_id: intervention_id.clone(),
            outcome,
            reason,
            reason_artifact_path,
            reason_byte_size,
            reason_content_hash,
            target_fingerprint: None,
        };
        let idempotency_key =
            InterventionResolvedPayload::idempotency_key(intervention_id, resolution_id.as_str());
        let writer = Writer {
            actor_id: ActorId::new("actor:claude_code:user"),
            role: writer_role,
            tool: WriterTool {
                name: "claude_code".to_owned(),
                version: String::new(),
            },
        };
        let mut event = ShoreEvent::new(
            EventType::InterventionResolved,
            idempotency_key,
            target,
            writer,
            payload,
            occurred_at,
        )
        .unwrap();
        event.assertion_mode = assertion_mode;
        event.source_ref = Some(SourceRef::new("claude_code", resolution_id.as_str()));
        event
    }

    #[test]
    fn agent_resumption_preserves_resolution_reason_payload_fields() {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let intervention_id = InterventionId::new("intervention:sha256:1");
        let resolution_id = InterventionResolutionId::new("intervention-resolution:sha256:r");

        let mut events = attempt_with_checkpoints(&task_attempt_id, &session_id, &[]);
        events.push(task_intervention_event_with_target(
            &task_attempt_id,
            &session_id,
            &intervention_id,
            "source:reason",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::TaskAttempt),
            "needs approval",
        ));
        events.push(user_resolution_event_with_reason(
            &intervention_id,
            &resolution_id,
            InterventionResolutionOutcome::Approved,
            AssertionMode::Operative,
            WriterRole::User,
            "2026-05-18T00:00:03Z",
            Some("inlined justification".to_owned()),
            Some("artifacts/resolutions/r.txt".to_owned()),
            Some(42),
            Some("sha256:reason-hash".to_owned()),
        ));

        let projection =
            agent_resumption_from_events(&events, &task_attempt_id, &reader_actor()).unwrap();

        let view = projection
            .selected_resolution
            .as_ref()
            .expect("resolution surfaced");
        assert_eq!(view.reason.as_deref(), Some("inlined justification"));
        assert_eq!(
            view.reason_artifact_path.as_deref(),
            Some("artifacts/resolutions/r.txt"),
        );
        assert_eq!(view.reason_byte_size, Some(42));
        assert_eq!(
            view.reason_content_hash.as_deref(),
            Some("sha256:reason-hash"),
        );
    }

    #[test]
    fn agent_resumption_collapses_duplicate_resolution_facts_instead_of_ambiguous() {
        // Two `InterventionResolved` events with the same
        // `intervention_resolution_id` are a retry duplicate, not two distinct
        // resolutions. The projection must collapse them and still treat the
        // intervention as cleanly resolved.
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let intervention_id = InterventionId::new("intervention:sha256:1");
        let resolution_id = InterventionResolutionId::new("intervention-resolution:sha256:r");

        let mut events = attempt_with_checkpoints(&task_attempt_id, &session_id, &[]);
        events.push(task_intervention_event_with_target(
            &task_attempt_id,
            &session_id,
            &intervention_id,
            "source:dup",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::TaskAttempt),
            "needs approval",
        ));
        let first = user_resolution_event(
            &intervention_id,
            &resolution_id,
            InterventionResolutionOutcome::Approved,
            AssertionMode::Operative,
            WriterRole::User,
            "2026-05-18T00:00:03Z",
        );
        let retry = user_resolution_event(
            &intervention_id,
            &resolution_id,
            InterventionResolutionOutcome::Approved,
            AssertionMode::Operative,
            WriterRole::User,
            "2026-05-18T00:00:03Z",
        );
        // Confirm the duplicate construction would emit identical events
        // (same idempotency key, therefore same event_id) before asserting
        // the projection treats them as one.
        assert_eq!(first.event_id, retry.event_id);
        events.push(first);
        events.push(retry);

        let projection =
            agent_resumption_from_events(&events, &task_attempt_id, &reader_actor()).unwrap();

        assert!(projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ready);
        assert_ne!(projection.state, AgentResumptionState::Ambiguous);
    }

    #[test]
    fn agent_resumption_collapses_distinct_event_ids_for_same_resolution_id() {
        // Same `intervention_resolution_id` but distinct envelope event ids
        // (e.g., a writer mistakenly emits the same semantic resolution with
        // two different idempotency keys). The projection still collapses by
        // resolution id and avoids a false Ambiguous classification.
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let intervention_id = InterventionId::new("intervention:sha256:1");
        let resolution_id = InterventionResolutionId::new("intervention-resolution:sha256:r");

        let mut events = attempt_with_checkpoints(&task_attempt_id, &session_id, &[]);
        events.push(task_intervention_event_with_target(
            &task_attempt_id,
            &session_id,
            &intervention_id,
            "source:dup-ids",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::TaskAttempt),
            "needs approval",
        ));

        // Two semantically-equal resolution events with distinct event ids
        // (constructed by mutating the idempotency key directly so the
        // derived event_id differs).
        let mut first = user_resolution_event(
            &intervention_id,
            &resolution_id,
            InterventionResolutionOutcome::Approved,
            AssertionMode::Operative,
            WriterRole::User,
            "2026-05-18T00:00:03Z",
        );
        let mut second = user_resolution_event(
            &intervention_id,
            &resolution_id,
            InterventionResolutionOutcome::Approved,
            AssertionMode::Operative,
            WriterRole::User,
            "2026-05-18T00:00:04Z",
        );
        first.idempotency_key = "duplicate-a".to_owned();
        first.event_id = crate::model::EventId::new("evt:sha256:duplicate-a");
        second.idempotency_key = "duplicate-b".to_owned();
        second.event_id = crate::model::EventId::new("evt:sha256:duplicate-b");
        assert_ne!(first.event_id, second.event_id);

        events.push(first);
        events.push(second);

        let projection =
            agent_resumption_from_events(&events, &task_attempt_id, &reader_actor()).unwrap();

        assert!(projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ready);
        assert_ne!(projection.state, AgentResumptionState::Ambiguous);
        let view = projection
            .selected_resolution
            .as_ref()
            .expect("resolution surfaced");
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
            Writer::shore_local_reviewer("test"),
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
    fn task_intervention_event_with_target_and_fingerprint(
        task_attempt_id: &WorkObjectId,
        session_id: &SessionId,
        intervention_id: &InterventionId,
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
        let payload = InterventionRequestedPayload {
            intervention_id: intervention_id.clone(),
            target: ReviewTargetRef::ReviewUnit {
                review_unit_id: ReviewUnitId::new("review-unit:placeholder"),
            },
            mode: InterventionMode::Blocking,
            reason_code: InterventionReasonCode::ManualDecisionRequired,
            title: title.to_owned(),
            body: None,
            body_artifact_path: None,
            body_byte_size: None,
            body_content_hash: None,
            target_fingerprint: target_fingerprint.map(str::to_owned),
        };
        let idempotency_key = InterventionRequestedPayload::idempotency_key_for_work_object(
            task_attempt_id,
            WorkObjectType::TaskAttempt,
            source_key,
        );
        let mut event = ShoreEvent::new(
            EventType::InterventionRequested,
            idempotency_key,
            target,
            Writer::shore_local_reviewer("test"),
            payload,
            occurred_at,
        )
        .unwrap();
        event.source_ref = Some(SourceRef::new("claude_code", source_key));
        event.assertion_mode = AssertionMode::Advisory;
        event
    }

    #[allow(clippy::too_many_arguments)]
    fn user_resolution_event_with_fingerprint(
        intervention_id: &InterventionId,
        resolution_id: &InterventionResolutionId,
        outcome: InterventionResolutionOutcome,
        assertion_mode: AssertionMode,
        writer_role: WriterRole,
        occurred_at: &str,
        target_fingerprint: Option<&str>,
    ) -> ShoreEvent {
        let target = EventTarget::for_work_object(
            SessionId::new("session:claude:uuid-1"),
            WorkObjectId::new("task-attempt:sha256:ta"),
            WorkObjectType::TaskAttempt,
        );
        let payload = InterventionResolvedPayload {
            intervention_resolution_id: resolution_id.clone(),
            intervention_id: intervention_id.clone(),
            outcome,
            reason: None,
            reason_artifact_path: None,
            reason_byte_size: None,
            reason_content_hash: None,
            target_fingerprint: target_fingerprint.map(str::to_owned),
        };
        let idempotency_key =
            InterventionResolvedPayload::idempotency_key(intervention_id, resolution_id.as_str());
        let writer = Writer {
            actor_id: ActorId::new("actor:claude_code:user"),
            role: writer_role,
            tool: WriterTool {
                name: "claude_code".to_owned(),
                version: String::new(),
            },
        };
        let mut event = ShoreEvent::new(
            EventType::InterventionResolved,
            idempotency_key,
            target,
            writer,
            payload,
            occurred_at,
        )
        .unwrap();
        event.assertion_mode = assertion_mode;
        event.source_ref = Some(SourceRef::new("claude_code", resolution_id.as_str()));
        event
    }

    #[test]
    fn agent_resumption_marks_resolution_stale_when_target_fingerprint_diverges_from_latest_checkpoint()
     {
        // §4.5 Scenario A literal: requester saw F1 (checkpoint C_A); a later
        // checkpoint C_B with F2 exists; the resolver acted on the F1 state.
        // The projection must flag the resolution stale on the fingerprint
        // disagreement -- not on identity alone.
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let cp_a = CheckpointId::new("checkpoint:sha256:cp-a");
        let cp_b = CheckpointId::new("checkpoint:sha256:cp-b");
        let intervention_id = InterventionId::new("intervention:sha256:1");
        let resolution_id = InterventionResolutionId::new("intervention-resolution:sha256:r");

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
        events.push(task_intervention_event_with_target_and_fingerprint(
            &task_attempt_id,
            &session_id,
            &intervention_id,
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
        events.push(user_resolution_event_with_fingerprint(
            &intervention_id,
            &resolution_id,
            InterventionResolutionOutcome::Approved,
            AssertionMode::Operative,
            WriterRole::User,
            "2026-05-18T00:00:04Z",
            Some(FP_A),
        ));

        let projection =
            agent_resumption_from_events(&events, &task_attempt_id, &reader_actor()).unwrap();

        assert!(!projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Stale);
        assert!(!projection.treated_as_operative);
        assert_eq!(
            projection.freshness,
            Some(FreshnessBasis::CheckpointFingerprintMismatch),
            "fingerprint disagreement is the stale signal, not just identity"
        );
        let resolution_view = projection
            .selected_resolution
            .as_ref()
            .expect("selected resolution surfaced");
        assert_eq!(
            resolution_view.target_fingerprint.as_deref(),
            Some(FP_A),
            "the resolver's fingerprint is preserved on the policy view"
        );
        assert!(!resolution_view.fresh_for_target);
        assert!(
            projection
                .diagnostics
                .iter()
                .any(|d| { d.code == "agent_resumption_resolution_target_fingerprint_mismatch" }),
            "diagnostic names the fingerprint mismatch"
        );
    }

    #[test]
    fn agent_resumption_treats_resolution_fresh_when_target_fingerprint_matches_latest_checkpoint()
    {
        // §4.5 Scenario B (re-anchored): only one checkpoint, with fingerprint
        // F1; the resolver acts on the F1 state. Both identity and fingerprint
        // agree, so the resolution is fresh.
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let cp_a = CheckpointId::new("checkpoint:sha256:cp-a");
        let intervention_id = InterventionId::new("intervention:sha256:1");
        let resolution_id = InterventionResolutionId::new("intervention-resolution:sha256:r");

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
        events.push(task_intervention_event_with_target_and_fingerprint(
            &task_attempt_id,
            &session_id,
            &intervention_id,
            "source:fresh-fp",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: cp_a.clone(),
            }),
            "needs approval at F1",
            Some(FP_A),
        ));
        events.push(user_resolution_event_with_fingerprint(
            &intervention_id,
            &resolution_id,
            InterventionResolutionOutcome::Approved,
            AssertionMode::Operative,
            WriterRole::User,
            "2026-05-18T00:00:03Z",
            Some(FP_A),
        ));

        let projection =
            agent_resumption_from_events(&events, &task_attempt_id, &reader_actor()).unwrap();

        assert!(projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ready);
        assert!(projection.treated_as_operative);
        assert_eq!(
            projection.freshness,
            Some(FreshnessBasis::CheckpointMatchesLatest)
        );
        let resolution_view = projection
            .selected_resolution
            .as_ref()
            .expect("selected resolution surfaced");
        assert!(resolution_view.fresh_for_target);
        assert_eq!(resolution_view.target_fingerprint.as_deref(), Some(FP_A));
    }

    #[test]
    fn agent_resumption_holds_ambiguous_when_two_distinct_resolution_facts_carry_different_target_fingerprints()
     {
        // Two distinct resolution facts (different `intervention_resolution_id`
        // values) on the same intervention. The two also disagree on
        // `target_fingerprint`. The Ambiguous discipline must win before any
        // freshness check fires; introducing fingerprint comparison must not
        // accidentally let the projection pick one resolution and call the
        // other stale.
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let cp_a = CheckpointId::new("checkpoint:sha256:cp-a");
        let intervention_id = InterventionId::new("intervention:sha256:1");
        let r1 = InterventionResolutionId::new("intervention-resolution:sha256:r1");
        let r2 = InterventionResolutionId::new("intervention-resolution:sha256:r2");

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
        events.push(task_intervention_event_with_target_and_fingerprint(
            &task_attempt_id,
            &session_id,
            &intervention_id,
            "source:ambig-fp",
            "2026-05-18T00:00:02Z",
            TargetRef::Task(TaskTargetRef::Checkpoint {
                checkpoint_id: cp_a.clone(),
            }),
            "needs approval",
            Some(FP_A),
        ));
        // Two distinct semantic resolutions: each carries its own
        // `target_fingerprint`. Both are by the same User role at Operative
        // mode -- the disagreement is on the fingerprint, not the writer.
        events.push(user_resolution_event_with_fingerprint(
            &intervention_id,
            &r1,
            InterventionResolutionOutcome::Approved,
            AssertionMode::Operative,
            WriterRole::User,
            "2026-05-18T00:00:03Z",
            Some(FP_B),
        ));
        events.push(user_resolution_event_with_fingerprint(
            &intervention_id,
            &r2,
            InterventionResolutionOutcome::Approved,
            AssertionMode::Operative,
            WriterRole::User,
            "2026-05-18T00:00:04Z",
            Some(FP_C),
        ));

        let projection =
            agent_resumption_from_events(&events, &task_attempt_id, &reader_actor()).unwrap();

        assert!(!projection.may_resume);
        assert_eq!(
            projection.state,
            AgentResumptionState::Ambiguous,
            "Ambiguous wins before any freshness check fires"
        );
        assert!(projection.selected_resolution.is_none());
        assert!(
            projection
                .diagnostics
                .iter()
                .any(|d| d.code == "agent_resumption_ambiguous_resolutions")
        );
    }

    #[test]
    fn agent_resumption_treats_resolution_fresh_when_fingerprints_match_across_different_checkpoint_ids()
     {
        // Two checkpoints share a `checkpoint_fingerprint` (same code state
        // recorded under distinct identities -- a retry or a parallel-write
        // boundary). The resolver targeted the older checkpoint id but
        // carried the shared fingerprint. The substrate-level rule is:
        // agreement on opaque fingerprint overrides the identity-based
        // staleness fallback. Without the fingerprint-equality short-circuit
        // the projection would mis-read this as `CheckpointStaleNewerExists`
        // and block resumption.
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let cp_older = CheckpointId::new("checkpoint:sha256:cp-older");
        let cp_latest = CheckpointId::new("checkpoint:sha256:cp-latest");
        let intervention_id = InterventionId::new("intervention:sha256:1");
        let resolution_id = InterventionResolutionId::new("intervention-resolution:sha256:r");

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
        events.push(task_intervention_event_with_target_and_fingerprint(
            &task_attempt_id,
            &session_id,
            &intervention_id,
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
        events.push(user_resolution_event_with_fingerprint(
            &intervention_id,
            &resolution_id,
            InterventionResolutionOutcome::Approved,
            AssertionMode::Operative,
            WriterRole::User,
            "2026-05-18T00:00:04Z",
            Some(FP_A),
        ));

        let projection =
            agent_resumption_from_events(&events, &task_attempt_id, &reader_actor()).unwrap();

        assert_eq!(
            projection.state,
            AgentResumptionState::Ready,
            "fingerprint agreement overrides identity-based staleness"
        );
        assert!(projection.may_resume);
        assert!(projection.treated_as_operative);
        assert_eq!(
            projection.freshness,
            Some(FreshnessBasis::CheckpointFingerprintMatches),
            "fingerprint match must be the surfaced freshness basis"
        );
        let resolution_view = projection
            .selected_resolution
            .as_ref()
            .expect("selected resolution surfaced");
        assert!(resolution_view.fresh_for_target);
        assert_eq!(resolution_view.target_fingerprint.as_deref(), Some(FP_A));
    }

    #[test]
    fn task_projections_preserve_fingerprint_fields_through_summary_and_intervention_views() {
        // Codex P2 follow-up. The no-info-loss claim requires that the
        // payload-level fingerprint fields survive into the sibling projection
        // views, not just into the resumption policy view. Set every
        // fingerprint field to a distinct opaque string and confirm each
        // surfaces verbatim on its intended view.
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = SessionId::new("session:claude:uuid-1");
        let cp_a = CheckpointId::new("checkpoint:sha256:cp-a");
        let intervention_id = InterventionId::new("intervention:sha256:1");

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
        let request = task_intervention_event_with_target_and_fingerprint(
            &task_attempt_id,
            &session_id,
            &intervention_id,
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

        let interventions =
            open_task_interventions_from_events(&events, &task_attempt_id, &reader_actor())
                .unwrap();
        let open = interventions
            .open_interventions
            .first()
            .expect("open intervention surfaced");
        assert_eq!(
            open.target_fingerprint.as_deref(),
            Some(FP_C),
            "intervention target fingerprint must surface on the open-intervention view"
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
