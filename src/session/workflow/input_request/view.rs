use std::collections::BTreeMap;

use crate::error::{Result, ShoreError};
use crate::model::{EventId, InputRequestId, InputRequestResponseId, ReviewTargetRef, TrackId};
use crate::session::event::{
    AssertionMode, BodyContentType, EventType, InputRequestOpenedPayload, InputRequestReasonCode,
    InputRequestRespondedPayload, InputRequestResponseOutcome, ShoreEvent, Writer,
    decode_input_request_opened_payload,
};
use crate::session::observation::{ResolvedRevision, target_matches_file};
use crate::session::projection::body_content::{
    BodyContentState, BodyRemovalLens, resolve_body_content,
};
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
    /// The reader's removal lens: an operative removal over an externalized
    /// body or response reason renders an explained removed state.
    pub removal_lens: &'a BodyRemovalLens<'a>,
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
    pub body_content_type: BodyContentType,
    pub body_content_hash: Option<String>,
    pub body_content_state: BodyContentState,
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
    pub reason_content_type: BodyContentType,
    pub reason_content_hash: Option<String>,
    pub reason_content_state: BodyContentState,
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
        if event.subject_revision_id()?.as_ref() != Some(&options.resolved.revision_id) {
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

        // Status derives from the response count alone, so it gates BEFORE any
        // response-reason bytes resolve: a request this surface will not return
        // must never hydrate (or hard-error on) its responses.
        let input_request_id = record.payload.input_request_id.clone();
        let response_records = responses.get(&input_request_id);
        let status = status_for_response_count(response_records.map_or(0, |records| records.len()));
        if !options.status_filter.matches(status) {
            continue;
        }
        let responses = match response_records {
            Some(records) => response_views_from_records(
                options.backend,
                options.removal_lens,
                options.include_body,
                records,
            )?,
            None => Vec::new(),
        };
        let view = input_request_view_from_event(
            options.backend,
            options.removal_lens,
            event,
            record.payload,
            record.track_id,
            responses,
            options.include_body,
        )?;
        input_requests.push(view);
    }

    sort_input_request_views(&mut input_requests);
    Ok(input_requests)
}

pub(super) struct InputRequestOpenRecord<'a> {
    pub(super) event: &'a ShoreEvent,
    pub(super) payload: InputRequestOpenedPayload,
    pub(super) track_id: TrackId,
}

pub(super) struct InputRequestResponseRecord<'a> {
    pub(super) event: &'a ShoreEvent,
    pub(super) payload: InputRequestRespondedPayload,
}

pub(super) struct InputRequestProjectionRecords<'a> {
    pub(super) request_records: BTreeMap<InputRequestId, InputRequestOpenRecord<'a>>,
    pub(super) responses: BTreeMap<InputRequestId, Vec<InputRequestResponseRecord<'a>>>,
}

// A pure record pass: no store reads and no removal-lens consultation happen
// here. Response reasons resolve later, per request a surface actually
// returns (`response_views_from_records`), so a missing artifact on a request
// a caller filters out can never fail that caller's read.
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

    let mut responses: BTreeMap<InputRequestId, Vec<InputRequestResponseRecord<'a>>> =
        BTreeMap::new();
    for record in response_records.into_values() {
        responses
            .entry(record.payload.input_request_id.clone())
            .or_default()
            .push(record);
    }

    for response_records in responses.values_mut() {
        sort_response_records(response_records);
    }

    Ok(InputRequestProjectionRecords {
        request_records,
        responses,
    })
}

/// Build the response views for one request this surface will return,
/// resolving each reason's text and state through the shared body resolution
/// with the calling surface's include-body flag.
pub(super) fn response_views_from_records(
    backend: &StoreBackend,
    removal_lens: &BodyRemovalLens<'_>,
    include_body: bool,
    records: &[InputRequestResponseRecord<'_>],
) -> Result<Vec<InputRequestResponseView>> {
    records
        .iter()
        .map(|record| {
            let content = resolve_body_content(
                backend,
                removal_lens,
                include_body,
                record.payload.reason.clone(),
                record.payload.reason_artifact_path.as_deref(),
            )?;
            let reason_content_state = content.state();
            Ok(InputRequestResponseView {
                id: record.payload.input_request_response_id.clone(),
                event_id: record.event.event_id.clone(),
                outcome: record.payload.outcome,
                reason: content.into_text(),
                reason_content_type: record.payload.reason_content_type,
                reason_content_hash: record.payload.reason_content_hash.clone(),
                reason_content_state,
                created_at: record.event.occurred_at.clone(),
                writer: record.event.writer.clone(),
            })
        })
        .collect()
}

// Event IDs are deterministic storage addresses, not causal order. Pick the lowest one
// only as a stable representative for duplicate semantic facts.
fn should_replace_representative(current: Option<&ShoreEvent>, candidate: &ShoreEvent) -> bool {
    current.is_none_or(|existing| candidate.event_id.as_str() < existing.event_id.as_str())
}

pub(super) fn input_request_view_from_event(
    backend: &StoreBackend,
    removal_lens: &BodyRemovalLens<'_>,
    event: &ShoreEvent,
    payload: InputRequestOpenedPayload,
    track_id: TrackId,
    responses: Vec<InputRequestResponseView>,
    include_body: bool,
) -> Result<InputRequestView> {
    let content = resolve_body_content(
        backend,
        removal_lens,
        include_body,
        payload.body.clone(),
        payload.body_artifact_path.as_deref(),
    )?;
    let body_content_state = content.state();
    let body = content.into_text();
    let status = status_for_response_count(responses.len());

    Ok(InputRequestView {
        id: payload.input_request_id,
        event_id: event.event_id.clone(),
        track_id,
        target: payload.target,
        mode: event.assertion_mode,
        reason_code: payload.reason_code,
        title: payload.title,
        body,
        body_content_type: payload.body_content_type,
        body_content_hash: payload.body_content_hash,
        body_content_state,
        status,
        responses,
        created_at: event.occurred_at.clone(),
        writer: event.writer.clone(),
    })
}

fn status_for_response_count(count: usize) -> InputRequestStatus {
    match count {
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

fn sort_response_records(records: &mut [InputRequestResponseRecord<'_>]) {
    records.sort_by(|left, right| {
        left.event
            .occurred_at
            .cmp(&right.event.occurred_at)
            .then_with(|| {
                left.event
                    .event_id
                    .as_str()
                    .cmp(right.event.event_id.as_str())
            })
    });
}
