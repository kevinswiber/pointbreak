//! Shared task-domain event builders for in-crate unit tests, used by the
//! task projection suite and the seam-level end-to-end resumption tests.

use crate::model::{
    ActorId, CheckpointId, InputRequestId, InputRequestResponseId, ReviewTargetRef, ReviewUnitId,
    SessionId, TargetRef, WorkObjectId, WorkObjectType,
};
use crate::session::event::{
    AssertionMode, EventTarget, EventType, InputRequestOpenedPayload, InputRequestReasonCode,
    InputRequestRespondedPayload, InputRequestResponseOutcome, ShoreEvent, SourceRef,
    TaskAttemptCapturedPayload, TaskCheckpointCapturedPayload, Writer, WriterProducer,
};

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

pub(crate) fn checkpoint_event(
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
    target.subject = Some(TargetRef::Task(crate::model::TaskTargetRef::Checkpoint {
        checkpoint_id: checkpoint_id.clone(),
    }));
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
    session_id: &SessionId,
    input_request_id: &InputRequestId,
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
