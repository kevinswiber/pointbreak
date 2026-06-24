use std::collections::BTreeMap;

use crate::error::{Result, ShoreError};
use crate::model::{EventId, InputRequestId, InputRequestResponseId, ReviewTargetRef, TrackId};
use crate::session::body_artifact::load_body_artifact;
use crate::session::event::{
    AssertionMode, EventType, InputRequestOpenedPayload, InputRequestReasonCode,
    InputRequestRespondedPayload, InputRequestResponseOutcome, ShoreEvent, Writer,
    decode_input_request_opened_payload,
};
use crate::session::observation::{ResolvedRevision, target_matches_file};
use crate::session::store::backend::StoreBackend;

pub(crate) struct InputRequestProjectionOptions<'a> {
    pub backend: &'a StoreBackend,
    pub events: &'a [ShoreEvent],
    pub resolved: &'a ResolvedRevision,
    pub track_filter: Option<TrackId>,
    pub mode_filter: Option<AssertionMode>,
    pub file_filter: Option<&'a str>,
    pub status_filter: InputRequestStatusFilter,
    pub include_body: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputRequestView {
    pub id: InputRequestId,
    pub event_id: EventId,
    pub track_id: TrackId,
    pub target: ReviewTargetRef,
    pub mode: AssertionMode,
    pub reason_code: InputRequestReasonCode,
    pub title: String,
    pub body: Option<String>,
    pub body_content_hash: Option<String>,
    pub status: InputRequestStatus,
    pub responses: Vec<InputRequestResponseView>,
    pub created_at: String,
    pub writer: Writer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputRequestResponseView {
    pub id: InputRequestResponseId,
    pub event_id: EventId,
    pub outcome: InputRequestResponseOutcome,
    pub reason: Option<String>,
    pub reason_content_hash: Option<String>,
    pub created_at: String,
    pub writer: Writer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputRequestStatus {
    Open,
    Responded,
    Ambiguous,
}

impl InputRequestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Responded => "responded",
            Self::Ambiguous => "ambiguous",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputRequestStatusFilter {
    Open,
    Responded,
    Ambiguous,
    All,
}

impl InputRequestStatusFilter {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Responded => "responded",
            Self::Ambiguous => "ambiguous",
            Self::All => "all",
        }
    }

    pub(super) fn matches(self, status: InputRequestStatus) -> bool {
        match self {
            Self::Open => status == InputRequestStatus::Open,
            Self::Responded => status == InputRequestStatus::Responded,
            Self::Ambiguous => status == InputRequestStatus::Ambiguous,
            Self::All => true,
        }
    }
}

pub(crate) fn project_input_requests(
    options: InputRequestProjectionOptions<'_>,
) -> Result<Vec<InputRequestView>> {
    let InputRequestProjectionRecords {
        request_records,
        responses,
    } = collect_input_request_projection_records(options.events)?;
    let mut input_requests = Vec::new();

    for record in request_records.into_values() {
        let event = record.event;
        if crate::model::subject_revision_id(&event.target.subject)
            != Some(&options.resolved.revision_id)
        {
            continue;
        }

        if options
            .track_filter
            .as_ref()
            .is_some_and(|filter| filter != &record.track_id)
        {
            continue;
        }
        if options
            .mode_filter
            .is_some_and(|mode| mode != event.assertion_mode)
        {
            continue;
        }
        if options
            .file_filter
            .is_some_and(|file| !target_matches_file(&record.payload.target, file))
        {
            continue;
        }

        let input_request_id = record.payload.input_request_id.clone();
        let responses = responses
            .get(&input_request_id)
            .cloned()
            .unwrap_or_default();
        let view = input_request_view_from_event(
            options.backend,
            event,
            record.payload,
            record.track_id,
            responses,
            options.include_body,
        )?;
        if options.status_filter.matches(view.status) {
            input_requests.push(view);
        }
    }

    sort_input_request_views(&mut input_requests);
    Ok(input_requests)
}

pub(super) struct InputRequestOpenRecord<'a> {
    pub(super) event: &'a ShoreEvent,
    pub(super) payload: InputRequestOpenedPayload,
    pub(super) track_id: TrackId,
}

struct InputRequestResponseRecord<'a> {
    event: &'a ShoreEvent,
    payload: InputRequestRespondedPayload,
}

pub(super) struct InputRequestProjectionRecords<'a> {
    pub(super) request_records: BTreeMap<InputRequestId, InputRequestOpenRecord<'a>>,
    pub(super) responses: BTreeMap<InputRequestId, Vec<InputRequestResponseView>>,
}

pub(super) fn collect_input_request_projection_records<'a>(
    events: &'a [ShoreEvent],
) -> Result<InputRequestProjectionRecords<'a>> {
    let mut request_records: BTreeMap<InputRequestId, InputRequestOpenRecord<'a>> = BTreeMap::new();
    let mut response_records: BTreeMap<InputRequestResponseId, InputRequestResponseRecord<'a>> =
        BTreeMap::new();

    for event in events {
        match event.event_type {
            EventType::InputRequestOpened => {
                let payload = decode_input_request_opened_payload(event.payload.clone())?;
                let track_id = event.target.track_id.clone().ok_or_else(|| {
                    ShoreError::Message("input request event missing track id".to_owned())
                })?;
                let input_request_id = payload.input_request_id.clone();
                if should_replace_representative(
                    request_records
                        .get(&input_request_id)
                        .map(|record| record.event),
                    event,
                ) {
                    request_records.insert(
                        input_request_id,
                        InputRequestOpenRecord {
                            event,
                            payload,
                            track_id,
                        },
                    );
                }
            }
            EventType::InputRequestResponded => {
                let payload: InputRequestRespondedPayload =
                    serde_json::from_value(event.payload.clone())?;
                let response_id = payload.input_request_response_id.clone();
                if should_replace_representative(
                    response_records
                        .get(&response_id)
                        .map(|record| record.event),
                    event,
                ) {
                    response_records
                        .insert(response_id, InputRequestResponseRecord { event, payload });
                }
            }
            _ => {}
        }
    }

    let mut responses: BTreeMap<InputRequestId, Vec<InputRequestResponseView>> = BTreeMap::new();
    for record in response_records.into_values() {
        let event = record.event;
        let payload = record.payload;
        responses
            .entry(payload.input_request_id)
            .or_default()
            .push(InputRequestResponseView {
                id: payload.input_request_response_id,
                event_id: event.event_id.clone(),
                outcome: payload.outcome,
                reason: payload.reason,
                reason_content_hash: payload.reason_content_hash,
                created_at: event.occurred_at.clone(),
                writer: event.writer.clone(),
            });
    }

    for response_views in responses.values_mut() {
        sort_response_views(response_views);
    }

    Ok(InputRequestProjectionRecords {
        request_records,
        responses,
    })
}

// Event IDs are deterministic storage addresses, not causal order. Pick the lowest one
// only as a stable representative for duplicate semantic facts.
fn should_replace_representative(current: Option<&ShoreEvent>, candidate: &ShoreEvent) -> bool {
    current.is_none_or(|existing| candidate.event_id.as_str() < existing.event_id.as_str())
}

pub(super) fn input_request_view_from_event(
    backend: &StoreBackend,
    event: &ShoreEvent,
    payload: InputRequestOpenedPayload,
    track_id: TrackId,
    responses: Vec<InputRequestResponseView>,
    include_body: bool,
) -> Result<InputRequestView> {
    let body = if include_body {
        input_request_body(backend, &payload)?
    } else {
        None
    };
    let status = status_for_responses(&responses);

    Ok(InputRequestView {
        id: payload.input_request_id,
        event_id: event.event_id.clone(),
        track_id,
        target: payload.target,
        mode: event.assertion_mode,
        reason_code: payload.reason_code,
        title: payload.title,
        body,
        body_content_hash: payload.body_content_hash,
        status,
        responses,
        created_at: event.occurred_at.clone(),
        writer: event.writer.clone(),
    })
}

fn input_request_body(
    backend: &StoreBackend,
    payload: &InputRequestOpenedPayload,
) -> Result<Option<String>> {
    if payload.body.is_some() {
        return Ok(payload.body.clone());
    }
    match payload.body_artifact_path.as_deref() {
        Some(path) => load_body_artifact(backend, path),
        None => Ok(None),
    }
}

fn status_for_responses(responses: &[InputRequestResponseView]) -> InputRequestStatus {
    match responses.len() {
        0 => InputRequestStatus::Open,
        1 => InputRequestStatus::Responded,
        _ => InputRequestStatus::Ambiguous,
    }
}

pub(super) fn sort_input_request_views(input_requests: &mut [InputRequestView]) {
    input_requests.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.event_id.as_str().cmp(right.event_id.as_str()))
    });
}

fn sort_response_views(responses: &mut [InputRequestResponseView]) {
    responses.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.event_id.as_str().cmp(right.event_id.as_str()))
    });
}
