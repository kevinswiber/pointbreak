//! Shared task-domain event builders for in-crate unit tests, used by the
//! task projection suite and the seam-level end-to-end resumption tests.

use crate::canonical_hash::sha256_bytes_hex;
use crate::model::{
    ActorId, CheckpointId, EngagementId, InputRequestId, InputRequestResponseId, JournalId,
    ReviewTargetRef, RevisionId, TargetRef, TaskTargetRef, WorkObjectId, WorkObjectType,
};
use crate::session::event::{
    AssertionMode, EventTarget, EventType, InputRequestOpenedPayload, InputRequestReasonCode,
    InputRequestRespondedPayload, InputRequestResponseOutcome, ShoreEvent, SourceRef,
    TaskCheckpointCapturedPayload, WorkObjectProposal, WorkObjectProposedPayload, Writer,
    WriterProducer,
};

fn task_attempt_subject() -> TargetRef {
    TargetRef::Task(TaskTargetRef::TaskAttempt {
        task_attempt_id: WorkObjectId::new("task-attempt:sha256:ta"),
    })
}

fn task_engagement_id(task_attempt_id: &WorkObjectId) -> EngagementId {
    EngagementId::new(format!(
        "engagement:sha256:{}",
        sha256_bytes_hex(task_attempt_id.as_str().as_bytes())
    ))
}

pub(crate) fn writer_user() -> Writer {
    Writer {
        actor_id: ActorId::new("actor:claude_code:user"),
        producer: WriterProducer {
            name: "claude_code".to_owned(),
            version: String::new(),
        },
    }
}

pub(crate) fn reader_actor() -> ActorId {
    ActorId::new("actor:shore:reader")
}

pub(crate) fn task_attempt_event(
    task_attempt_id: &WorkObjectId,
    session_id: &JournalId,
    claude_session_uuid: &str,
    occurred_at: &str,
) -> ShoreEvent {
    let target =
        EventTarget::for_subject(session_id.clone(), task_attempt_subject(), None).unwrap();
    let payload = WorkObjectProposedPayload {
        engagement_id: task_engagement_id(task_attempt_id),
        work_object: WorkObjectProposal::TaskAttempt {
            task_attempt_id: task_attempt_id.clone(),
            project_path: "/repo".to_owned(),
            claude_session_uuid: claude_session_uuid.to_owned(),
            initial_prompt_hash: "sha256:prompt".to_owned(),
            predecessor: None,
            base_state_fingerprint: None,
            source_speaker: None,
        },
    };
    let idempotency_key = format!("work_object_proposed:{}", task_attempt_id.as_str());
    let mut event = ShoreEvent::new(
        EventType::WorkObjectProposed,
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

pub(crate) fn checkpoint_event(
    task_attempt_id: &WorkObjectId,
    session_id: &JournalId,
    checkpoint_id: &CheckpointId,
    assistant_message_id: &str,
    tool_use_ids: Vec<String>,
    occurred_at: &str,
) -> ShoreEvent {
    let target = EventTarget::for_subject(
        session_id.clone(),
        TargetRef::Task(TaskTargetRef::Checkpoint {
            checkpoint_id: checkpoint_id.clone(),
        }),
        None,
    )
    .unwrap();
    let payload = TaskCheckpointCapturedPayload {
        checkpoint_id: checkpoint_id.clone(),
        parent_task_attempt_id: task_attempt_id.clone(),
        assistant_message_id: assistant_message_id.to_owned(),
        tool_use_ids,
        checkpoint_fingerprint: None,
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
pub(crate) fn task_input_request_event_with_target(
    task_attempt_id: &WorkObjectId,
    session_id: &JournalId,
    input_request_id: &InputRequestId,
    source_key: &str,
    occurred_at: &str,
    subject: TargetRef,
    title: &str,
) -> ShoreEvent {
    // The payload task_target mirrors the envelope's task subject (attempt or
    // checkpoint) — captured before `subject` is moved into the constructor.
    let task_target = match &subject {
        TargetRef::Task(task) => Some(task.clone()),
        _ => None,
    };
    let target = EventTarget::for_subject(session_id.clone(), subject, None).unwrap();
    let payload = InputRequestOpenedPayload {
        input_request_id: input_request_id.clone(),
        target: ReviewTargetRef::Revision {
            revision_id: RevisionId::new("review-unit:placeholder"),
        },
        task_target,
        reason_code: InputRequestReasonCode::ManualDecisionRequired,
        title: title.to_owned(),
        body: None,
        body_content_type: Default::default(),
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
    event.assertion_mode = AssertionMode::Operative;
    event
}

pub(crate) fn user_response_event(
    input_request_id: &InputRequestId,
    response_id: &InputRequestResponseId,
    outcome: InputRequestResponseOutcome,
    assertion_mode: AssertionMode,
    occurred_at: &str,
) -> ShoreEvent {
    let target = EventTarget::for_subject(
        JournalId::new("journal:claude:uuid-1"),
        task_attempt_subject(),
        None,
    )
    .unwrap();
    let payload = InputRequestRespondedPayload {
        input_request_response_id: response_id.clone(),
        input_request_id: input_request_id.clone(),
        revision_id: None,
        task_target: Some(TaskTargetRef::TaskAttempt {
            task_attempt_id: WorkObjectId::new("task-attempt:sha256:ta"),
        }),
        outcome,
        reason: None,
        reason_content_type: Default::default(),
        reason_artifact_path: None,
        reason_byte_size: None,
        reason_content_hash: None,
        target_fingerprint: None,
    };
    let idempotency_key =
        InputRequestRespondedPayload::idempotency_key(input_request_id, response_id.as_str());
    let mut event = ShoreEvent::new(
        EventType::InputRequestResponded,
        idempotency_key,
        target,
        writer_user(),
        payload,
        occurred_at,
    )
    .unwrap();
    event.assertion_mode = assertion_mode;
    event.source_ref = Some(SourceRef::new("claude_code", response_id.as_str()));
    event
}
