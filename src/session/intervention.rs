use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::error::{Result, ShoreError};
use crate::model::{
    EventId, InterventionId, InterventionResolutionId, ObservationId, ReviewTargetRef,
    ReviewUnitId, Side, TrackId,
};
use crate::session::body_artifact::load_body_artifact;
use crate::session::event::{
    EventTarget, EventType, InterventionMode, InterventionReasonCode, InterventionRequestedPayload,
    InterventionResolutionOutcome, InterventionResolvedPayload, ReviewObservationRecordedPayload,
    ShoreEvent, Writer,
};
use crate::session::event_context::{current_timestamp, reviewer_from_git_config};
use crate::session::observation::{
    ObservationTargetSelector, ResolvedReviewUnit, required_title, resolve_observation_target,
    resolve_review_unit_for_observation, staged_body, target_matches_file, validated_track_id,
};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store_init::{ShoreStorePaths, prepare_shore_writer};
use crate::storage::{Durability, EventStore, EventWriteOutcome, LocalStorage};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterventionRequestOptions {
    repo: PathBuf,
    review_unit_id: Option<ReviewUnitId>,
    track: Option<String>,
    title: Option<String>,
    body: Option<String>,
    target: InterventionTargetSelector,
    mode: InterventionMode,
    reason_code: Option<InterventionReasonCode>,
    idempotency_key: Option<String>,
}

impl InterventionRequestOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_unit_id: None,
            track: None,
            title: None,
            body: None,
            target: InterventionTargetSelector::review_unit(),
            mode: InterventionMode::Blocking,
            reason_code: None,
            idempotency_key: None,
        }
    }

    pub fn with_review_unit_id(mut self, id: ReviewUnitId) -> Self {
        self.review_unit_id = Some(id);
        self
    }

    pub fn with_track(mut self, track: impl Into<String>) -> Self {
        self.track = Some(track.into());
        self
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn with_body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn with_target(mut self, target: InterventionTargetSelector) -> Self {
        self.target = target;
        self
    }

    pub fn with_mode(mut self, mode: InterventionMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_reason_code(mut self, reason_code: InterventionReasonCode) -> Self {
        self.reason_code = Some(reason_code);
        self
    }

    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterventionRequestResult {
    pub review_unit_id: ReviewUnitId,
    pub intervention_id: InterventionId,
    pub event_id: EventId,
    pub track_id: TrackId,
    pub target: ReviewTargetRef,
    pub mode: InterventionMode,
    pub reason_code: InterventionReasonCode,
    pub body_content_hash: Option<String>,
    pub events_created: usize,
    pub events_existing: usize,
    pub events_created_by_type: BTreeMap<String, usize>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterventionListOptions {
    repo: PathBuf,
    review_unit_id: Option<ReviewUnitId>,
    track: Option<String>,
    mode: Option<InterventionMode>,
    file: Option<String>,
    status: InterventionStatusFilter,
    include_body: bool,
}

impl InterventionListOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_unit_id: None,
            track: None,
            mode: None,
            file: None,
            status: InterventionStatusFilter::Open,
            include_body: false,
        }
    }

    pub fn with_review_unit_id(mut self, id: ReviewUnitId) -> Self {
        self.review_unit_id = Some(id);
        self
    }

    pub fn with_track(mut self, track: impl Into<String>) -> Self {
        self.track = Some(track.into());
        self
    }

    pub fn with_mode(mut self, mode: InterventionMode) -> Self {
        self.mode = Some(mode);
        self
    }

    pub fn with_file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    pub fn with_status(mut self, status: InterventionStatusFilter) -> Self {
        self.status = status;
        self
    }

    pub fn with_include_body(mut self, include_body: bool) -> Self {
        self.include_body = include_body;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterventionFetchOptions {
    repo: PathBuf,
    intervention_id: InterventionId,
    include_body: bool,
}

impl InterventionFetchOptions {
    pub fn new(repo: impl AsRef<Path>, intervention_id: InterventionId) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            intervention_id,
            include_body: false,
        }
    }

    pub fn with_include_body(mut self, include_body: bool) -> Self {
        self.include_body = include_body;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterventionResolveOptions {
    repo: PathBuf,
    intervention_id: InterventionId,
    outcome: Option<InterventionResolutionOutcome>,
    reason: Option<String>,
    idempotency_key: Option<String>,
}

impl InterventionResolveOptions {
    pub fn new(repo: impl AsRef<Path>, intervention_id: InterventionId) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            intervention_id,
            outcome: None,
            reason: None,
            idempotency_key: None,
        }
    }

    pub fn with_outcome(mut self, outcome: InterventionResolutionOutcome) -> Self {
        self.outcome = Some(outcome);
        self
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterventionResolveResult {
    pub intervention_id: InterventionId,
    pub intervention_resolution_id: InterventionResolutionId,
    pub event_id: EventId,
    pub outcome: InterventionResolutionOutcome,
    pub reason_content_hash: Option<String>,
    pub events_created: usize,
    pub events_existing: usize,
    pub events_created_by_type: BTreeMap<String, usize>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterventionListFilters {
    pub track_id: Option<TrackId>,
    pub mode: Option<InterventionMode>,
    pub file: Option<String>,
    pub status: InterventionStatusFilter,
    pub include_body: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterventionListResult {
    pub review_unit_id: ReviewUnitId,
    pub filters: InterventionListFilters,
    pub interventions: Vec<InterventionView>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterventionFetchResult {
    pub intervention: InterventionView,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterventionView {
    pub id: InterventionId,
    pub event_id: EventId,
    pub track_id: TrackId,
    pub target: ReviewTargetRef,
    pub mode: InterventionMode,
    pub reason_code: InterventionReasonCode,
    pub title: String,
    pub body: Option<String>,
    pub body_content_hash: Option<String>,
    pub status: InterventionStatus,
    pub resolutions: Vec<InterventionResolutionView>,
    pub created_at: String,
    pub writer: Writer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterventionResolutionView {
    pub id: InterventionResolutionId,
    pub event_id: EventId,
    pub outcome: InterventionResolutionOutcome,
    pub reason: Option<String>,
    pub reason_content_hash: Option<String>,
    pub created_at: String,
    pub writer: Writer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterventionStatus {
    Open,
    Resolved,
    Ambiguous,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterventionStatusFilter {
    Open,
    Resolved,
    Ambiguous,
    All,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InterventionTargetSelector {
    ReviewUnit,
    File {
        path: String,
    },
    Range {
        path: String,
        side: Side,
        start_line: u32,
        end_line: Option<u32>,
    },
    Observation {
        observation_id: ObservationId,
    },
}

impl InterventionTargetSelector {
    pub fn review_unit() -> Self {
        Self::ReviewUnit
    }

    pub fn file(path: impl Into<String>) -> Self {
        Self::File { path: path.into() }
    }

    pub fn range(
        path: impl Into<String>,
        side: Side,
        start_line: u32,
        end_line: Option<u32>,
    ) -> Self {
        Self::Range {
            path: path.into(),
            side,
            start_line,
            end_line,
        }
    }

    pub fn observation(observation_id: ObservationId) -> Self {
        Self::Observation { observation_id }
    }
}

pub fn request_intervention(
    options: InterventionRequestOptions,
) -> Result<InterventionRequestResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let worktree_root = paths.worktree_root();
    let shore_dir = paths.shore_dir();
    let storage = LocalStorage::new(shore_dir);
    prepare_shore_writer(&paths, &storage)?;

    let event_store = EventStore::open(shore_dir);
    let events = event_store.list_events()?;
    let resolved = resolve_review_unit_for_observation(&events, options.review_unit_id.as_ref())?;
    let target = resolve_intervention_target(worktree_root, &events, &resolved, &options.target)?;
    let track_id = validated_track_id(
        options
            .track
            .as_deref()
            .ok_or_else(|| ShoreError::Message("track is required".to_owned()))?,
    )?;
    let title = required_title(options.title.as_deref())?;
    let reason_code = options
        .reason_code
        .ok_or_else(|| ShoreError::Message("reason code is required".to_owned()))?;
    let writer = reviewer_from_git_config(worktree_root);
    let body_content_hash = options
        .body
        .as_ref()
        .map(|body| format!("sha256:{}", sha256_bytes_hex(body.as_bytes())));
    let (body, body_artifact_path, body_artifact_bytes, body_byte_size) =
        staged_body(options.body.as_deref())?;
    let intervention_id = build_intervention_id(InterventionIdMaterial {
        review_unit_id: &resolved.review_unit_id,
        track_id: &track_id,
        target: &target,
        mode: options.mode,
        reason_code,
        title: &title,
        body_content_hash: body_content_hash.as_deref(),
        writer_actor_id: writer.actor_id.as_str(),
    })?;
    let source_key = options
        .idempotency_key
        .as_deref()
        .unwrap_or_else(|| intervention_id.as_str());
    let idempotency_key = InterventionRequestedPayload::idempotency_key(
        &resolved.review_unit_id,
        &track_id,
        source_key,
    );

    if !event_store.event_exists(&idempotency_key)?
        && let (Some(artifact_path), Some(bytes)) =
            (body_artifact_path.as_deref(), body_artifact_bytes.as_ref())
    {
        storage.write_bytes_atomic(Path::new(artifact_path), bytes, Durability::Durable)?;
    }

    let event = ShoreEvent::new(
        EventType::InterventionRequested,
        idempotency_key,
        EventTarget {
            review_id: resolved.review_id,
            work_unit_id: None,
            review_unit_id: Some(resolved.review_unit_id.clone()),
            revision_id: Some(resolved.revision_id),
            snapshot_id: Some(resolved.snapshot_id),
            track_id: Some(track_id.clone()),
            subject: Some(target.clone()),
        },
        writer,
        InterventionRequestedPayload {
            intervention_id: intervention_id.clone(),
            target: target.clone(),
            mode: options.mode,
            reason_code,
            title,
            body,
            body_artifact_path,
            body_byte_size,
            body_content_hash: body_content_hash.clone(),
        },
        current_timestamp(),
    )?;
    let event_id = event.event_id.clone();

    let mut events_created_by_type = BTreeMap::new();
    let (events_created, events_existing) = match event_store.record_event_once(&event)? {
        EventWriteOutcome::Created => {
            events_created_by_type.insert("intervention_requested".to_owned(), 1);
            (1, 0)
        }
        EventWriteOutcome::Existing => (0, 1),
    };

    let state = SessionState::from_events(&event_store.list_events()?)?;
    storage.write_json_atomic(&paths.state_path(), &state, Durability::Projection)?;

    Ok(InterventionRequestResult {
        review_unit_id: resolved.review_unit_id,
        intervention_id,
        event_id,
        track_id,
        target,
        mode: options.mode,
        reason_code,
        body_content_hash,
        events_created,
        events_existing,
        events_created_by_type,
        diagnostics: state.diagnostics,
    })
}

pub fn list_interventions(options: InterventionListOptions) -> Result<InterventionListResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let shore_dir = paths.shore_dir();
    let event_store = EventStore::open(shore_dir);
    let events = event_store.list_events()?;
    let resolved = resolve_review_unit_for_observation(&events, options.review_unit_id.as_ref())?;
    let track_filter = options
        .track
        .as_deref()
        .map(validated_track_id)
        .transpose()?;
    let resolutions = collect_resolution_views(&events)?;
    let mut interventions = Vec::new();

    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::InterventionRequested)
    {
        if event.target.review_unit_id.as_ref() != Some(&resolved.review_unit_id) {
            continue;
        }

        let payload: InterventionRequestedPayload = serde_json::from_value(event.payload.clone())?;
        let track_id =
            event.target.track_id.clone().ok_or_else(|| {
                ShoreError::Message("intervention event missing track id".to_owned())
            })?;
        if track_filter
            .as_ref()
            .is_some_and(|filter| filter != &track_id)
        {
            continue;
        }
        if options.mode.is_some_and(|mode| mode != payload.mode) {
            continue;
        }
        if options
            .file
            .as_deref()
            .is_some_and(|file| !target_matches_file(&payload.target, file))
        {
            continue;
        }

        let intervention_id = payload.intervention_id.clone();
        let view = intervention_view_from_event(
            shore_dir,
            event,
            payload,
            track_id,
            resolutions
                .get(&intervention_id)
                .cloned()
                .unwrap_or_default(),
            options.include_body,
        )?;
        if options.status.matches(view.status) {
            interventions.push(view);
        }
    }

    sort_intervention_views(&mut interventions);
    let diagnostics = SessionState::from_events(&events)?.diagnostics;

    Ok(InterventionListResult {
        review_unit_id: resolved.review_unit_id,
        filters: InterventionListFilters {
            track_id: track_filter,
            mode: options.mode,
            file: options.file,
            status: options.status,
            include_body: options.include_body,
        },
        interventions,
        diagnostics,
    })
}

pub fn fetch_intervention(options: InterventionFetchOptions) -> Result<InterventionFetchResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let shore_dir = paths.shore_dir();
    let events = EventStore::open(shore_dir).list_events()?;
    let resolutions = collect_resolution_views(&events)?;

    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::InterventionRequested)
    {
        let payload: InterventionRequestedPayload = serde_json::from_value(event.payload.clone())?;
        if payload.intervention_id != options.intervention_id {
            continue;
        }

        let track_id =
            event.target.track_id.clone().ok_or_else(|| {
                ShoreError::Message("intervention event missing track id".to_owned())
            })?;
        let view = intervention_view_from_event(
            shore_dir,
            event,
            payload,
            track_id,
            resolutions
                .get(&options.intervention_id)
                .cloned()
                .unwrap_or_default(),
            options.include_body,
        )?;
        let diagnostics = SessionState::from_events(&events)?.diagnostics;

        return Ok(InterventionFetchResult {
            intervention: view,
            diagnostics,
        });
    }

    Err(ShoreError::Message(format!(
        "unknown intervention: {}",
        options.intervention_id.as_str()
    )))
}

pub fn resolve_intervention(
    options: InterventionResolveOptions,
) -> Result<InterventionResolveResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let worktree_root = paths.worktree_root();
    let shore_dir = paths.shore_dir();
    let storage = LocalStorage::new(shore_dir);
    prepare_shore_writer(&paths, &storage)?;

    let event_store = EventStore::open(shore_dir);
    let events = event_store.list_events()?;
    let mut request_event_and_payload = None;
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::InterventionRequested)
    {
        let payload: InterventionRequestedPayload = serde_json::from_value(event.payload.clone())?;
        if payload.intervention_id == options.intervention_id {
            request_event_and_payload = Some((event, payload));
            break;
        }
    }
    let (request_event, request_payload) = request_event_and_payload.ok_or_else(|| {
        ShoreError::Message(format!(
            "unknown intervention: {}",
            options.intervention_id.as_str()
        ))
    })?;
    let outcome = options
        .outcome
        .ok_or_else(|| ShoreError::Message("outcome is required".to_owned()))?;
    let writer = reviewer_from_git_config(worktree_root);
    let reason_content_hash = options
        .reason
        .as_ref()
        .map(|reason| format!("sha256:{}", sha256_bytes_hex(reason.as_bytes())));
    let (reason, reason_artifact_path, reason_artifact_bytes, reason_byte_size) =
        staged_body(options.reason.as_deref())?;
    let intervention_resolution_id =
        build_intervention_resolution_id(InterventionResolutionIdMaterial {
            intervention_id: &request_payload.intervention_id,
            outcome,
            reason_content_hash: reason_content_hash.as_deref(),
            writer_actor_id: writer.actor_id.as_str(),
        })?;
    let source_key = options
        .idempotency_key
        .as_deref()
        .unwrap_or_else(|| intervention_resolution_id.as_str());
    let idempotency_key =
        InterventionResolvedPayload::idempotency_key(&request_payload.intervention_id, source_key);

    if !event_store.event_exists(&idempotency_key)?
        && let (Some(artifact_path), Some(bytes)) = (
            reason_artifact_path.as_deref(),
            reason_artifact_bytes.as_ref(),
        )
    {
        storage.write_bytes_atomic(Path::new(artifact_path), bytes, Durability::Durable)?;
    }

    let review_unit_id =
        request_event.target.review_unit_id.clone().ok_or_else(|| {
            ShoreError::Message("intervention event missing review unit".to_owned())
        })?;
    let event = ShoreEvent::new(
        EventType::InterventionResolved,
        idempotency_key,
        EventTarget {
            review_id: request_event.target.review_id.clone(),
            work_unit_id: None,
            review_unit_id: Some(review_unit_id.clone()),
            revision_id: request_event.target.revision_id.clone(),
            snapshot_id: request_event.target.snapshot_id.clone(),
            track_id: request_event.target.track_id.clone(),
            subject: Some(ReviewTargetRef::Intervention {
                review_unit_id,
                intervention_id: request_payload.intervention_id.clone(),
            }),
        },
        writer,
        InterventionResolvedPayload {
            intervention_resolution_id: intervention_resolution_id.clone(),
            intervention_id: request_payload.intervention_id.clone(),
            outcome,
            reason,
            reason_artifact_path,
            reason_byte_size,
            reason_content_hash: reason_content_hash.clone(),
        },
        current_timestamp(),
    )?;
    let event_id = event.event_id.clone();

    let mut events_created_by_type = BTreeMap::new();
    let (events_created, events_existing) = match event_store.record_event_once(&event)? {
        EventWriteOutcome::Created => {
            events_created_by_type.insert("intervention_resolved".to_owned(), 1);
            (1, 0)
        }
        EventWriteOutcome::Existing => (0, 1),
    };

    let state = SessionState::from_events(&event_store.list_events()?)?;
    storage.write_json_atomic(&paths.state_path(), &state, Durability::Projection)?;

    Ok(InterventionResolveResult {
        intervention_id: request_payload.intervention_id,
        intervention_resolution_id,
        event_id,
        outcome,
        reason_content_hash,
        events_created,
        events_existing,
        events_created_by_type,
        diagnostics: state.diagnostics,
    })
}

fn collect_resolution_views(
    events: &[ShoreEvent],
) -> Result<BTreeMap<InterventionId, Vec<InterventionResolutionView>>> {
    let mut resolutions: BTreeMap<InterventionId, Vec<InterventionResolutionView>> =
        BTreeMap::new();
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::InterventionResolved)
    {
        let payload: InterventionResolvedPayload = serde_json::from_value(event.payload.clone())?;
        resolutions
            .entry(payload.intervention_id)
            .or_default()
            .push(InterventionResolutionView {
                id: payload.intervention_resolution_id,
                event_id: event.event_id.clone(),
                outcome: payload.outcome,
                reason: payload.reason,
                reason_content_hash: payload.reason_content_hash,
                created_at: event.occurred_at.clone(),
                writer: event.writer.clone(),
            });
    }

    for resolution_views in resolutions.values_mut() {
        sort_resolution_views(resolution_views);
    }

    Ok(resolutions)
}

fn intervention_view_from_event(
    shore_dir: &Path,
    event: &ShoreEvent,
    payload: InterventionRequestedPayload,
    track_id: TrackId,
    resolutions: Vec<InterventionResolutionView>,
    include_body: bool,
) -> Result<InterventionView> {
    let body = if include_body {
        intervention_body(shore_dir, &payload)?
    } else {
        None
    };
    let status = status_for_resolutions(&resolutions);

    Ok(InterventionView {
        id: payload.intervention_id,
        event_id: event.event_id.clone(),
        track_id,
        target: payload.target,
        mode: payload.mode,
        reason_code: payload.reason_code,
        title: payload.title,
        body,
        body_content_hash: payload.body_content_hash,
        status,
        resolutions,
        created_at: event.occurred_at.clone(),
        writer: event.writer.clone(),
    })
}

fn intervention_body(
    shore_dir: &Path,
    payload: &InterventionRequestedPayload,
) -> Result<Option<String>> {
    if payload.body.is_some() {
        return Ok(payload.body.clone());
    }
    match payload.body_artifact_path.as_deref() {
        Some(path) => load_body_artifact(shore_dir, path),
        None => Ok(None),
    }
}

fn status_for_resolutions(resolutions: &[InterventionResolutionView]) -> InterventionStatus {
    match resolutions.len() {
        0 => InterventionStatus::Open,
        1 => InterventionStatus::Resolved,
        _ => InterventionStatus::Ambiguous,
    }
}

fn sort_intervention_views(interventions: &mut [InterventionView]) {
    interventions.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.event_id.as_str().cmp(right.event_id.as_str()))
    });
}

fn sort_resolution_views(resolutions: &mut [InterventionResolutionView]) {
    resolutions.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.event_id.as_str().cmp(right.event_id.as_str()))
    });
}

impl InterventionStatusFilter {
    fn matches(self, status: InterventionStatus) -> bool {
        match self {
            Self::Open => status == InterventionStatus::Open,
            Self::Resolved => status == InterventionStatus::Resolved,
            Self::Ambiguous => status == InterventionStatus::Ambiguous,
            Self::All => true,
        }
    }
}

fn resolve_intervention_target(
    repo: &Path,
    events: &[ShoreEvent],
    resolved: &ResolvedReviewUnit,
    selector: &InterventionTargetSelector,
) -> Result<ReviewTargetRef> {
    match selector {
        InterventionTargetSelector::ReviewUnit => {
            resolve_observation_target(repo, resolved, &ObservationTargetSelector::review_unit())
        }
        InterventionTargetSelector::File { path } => {
            resolve_observation_target(repo, resolved, &ObservationTargetSelector::file(path))
        }
        InterventionTargetSelector::Range {
            path,
            side,
            start_line,
            end_line,
        } => resolve_observation_target(
            repo,
            resolved,
            &ObservationTargetSelector::range(path, *side, *start_line, *end_line),
        ),
        InterventionTargetSelector::Observation { observation_id } => {
            resolve_native_observation_target(events, resolved, observation_id)
        }
    }
}

fn resolve_native_observation_target(
    events: &[ShoreEvent],
    resolved: &ResolvedReviewUnit,
    observation_id: &ObservationId,
) -> Result<ReviewTargetRef> {
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewObservationRecorded)
    {
        if event.target.review_unit_id.as_ref() != Some(&resolved.review_unit_id) {
            continue;
        }

        let payload: ReviewObservationRecordedPayload =
            serde_json::from_value(event.payload.clone())?;
        if &payload.observation_id == observation_id {
            return Ok(ReviewTargetRef::Observation {
                review_unit_id: resolved.review_unit_id.clone(),
                observation_id: observation_id.clone(),
            });
        }
    }

    Err(ShoreError::Message(format!(
        "unknown observation target: {}",
        observation_id.as_str()
    )))
}

struct InterventionIdMaterial<'a> {
    review_unit_id: &'a ReviewUnitId,
    track_id: &'a TrackId,
    target: &'a ReviewTargetRef,
    mode: InterventionMode,
    reason_code: InterventionReasonCode,
    title: &'a str,
    body_content_hash: Option<&'a str>,
    writer_actor_id: &'a str,
}

fn build_intervention_id(material: InterventionIdMaterial<'_>) -> Result<InterventionId> {
    let digest = sha256_json_prefixed(&json!({
        "reviewUnitId": material.review_unit_id.as_str(),
        "trackId": material.track_id.as_str(),
        "target": material.target,
        "mode": material.mode,
        "reasonCode": material.reason_code,
        "title": material.title,
        "bodyContentHash": material.body_content_hash,
        "writerActorId": material.writer_actor_id,
    }))?;
    Ok(InterventionId::new(format!("intervention:{digest}")))
}

struct InterventionResolutionIdMaterial<'a> {
    intervention_id: &'a InterventionId,
    outcome: InterventionResolutionOutcome,
    reason_content_hash: Option<&'a str>,
    writer_actor_id: &'a str,
}

fn build_intervention_resolution_id(
    material: InterventionResolutionIdMaterial<'_>,
) -> Result<InterventionResolutionId> {
    let digest = sha256_json_prefixed(&json!({
        "interventionId": material.intervention_id.as_str(),
        "outcome": material.outcome,
        "reasonContentHash": material.reason_content_hash,
        "writerActorId": material.writer_actor_id,
    }))?;
    Ok(InterventionResolutionId::new(format!(
        "intervention-resolution:{digest}"
    )))
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::*;
    use crate::model::{ObservationId, ReviewTargetRef, Side};
    use crate::session::{
        CaptureOptions, ObservationAddOptions, SessionState, capture_worktree_review,
        record_observation,
    };
    use crate::storage::EventStore;

    #[test]
    fn request_intervention_writes_event_and_updates_state() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired)
                .with_mode(InterventionMode::Blocking)
                .with_target(InterventionTargetSelector::review_unit()),
        )
        .unwrap();

        assert_eq!(result.review_unit_id, capture.review_unit_id);
        assert!(
            result
                .intervention_id
                .as_str()
                .starts_with("intervention:sha256:")
        );
        assert_eq!(result.mode, InterventionMode::Blocking);
        assert_eq!(
            result.reason_code,
            InterventionReasonCode::ManualDecisionRequired
        );
        assert_eq!(result.track_id.as_str(), "human:kevin");
        assert_eq!(result.events_created, 1);
        assert_eq!(result.events_existing, 0);
        assert_eq!(result.events_created_by_type["intervention_requested"], 1);

        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let state = SessionState::from_events(&events).unwrap();
        assert_eq!(state.intervention_count, 1);
        assert_eq!(state.open_intervention_count, 1);
        assert_eq!(state.open_blocking_intervention_count, 1);
    }

    #[test]
    fn request_intervention_is_idempotent_for_same_logical_input() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let options = InterventionRequestOptions::new(repo.path())
            .with_track("agent:codex")
            .with_title("Pause")
            .with_reason_code(InterventionReasonCode::UnsafeAction)
            .with_body("same body");

        let first = request_intervention(options.clone()).unwrap();
        let second = request_intervention(options).unwrap();

        assert_eq!(first.intervention_id, second.intervention_id);
        assert_eq!(first.events_created, 1);
        assert_eq!(second.events_created, 0);
        assert_eq!(second.events_existing, 1);
    }

    #[test]
    fn request_intervention_requires_a_captured_review_unit() {
        let repo = modified_repo();

        let error = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired),
        )
        .unwrap_err();

        assert!(error.to_string().contains("no captured review unit"));
    }

    #[test]
    fn request_intervention_requires_explicit_review_unit_when_current_is_ambiguous() {
        let repo = modified_repo();
        let first = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        repo.write("src/lib.rs", "pub fn value() -> u32 {\n    3\n}\n");
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let ambiguous = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired),
        )
        .unwrap_err();
        assert!(
            ambiguous
                .to_string()
                .contains("multiple captured review units")
        );

        let explicit = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_review_unit_id(first.review_unit_id.clone())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired),
        )
        .unwrap();
        assert_eq!(explicit.review_unit_id, first.review_unit_id);
    }

    #[test]
    fn request_intervention_rejects_invalid_track() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let error = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("system:shore")
                .with_title("Need approval")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired),
        )
        .unwrap_err();

        assert!(error.to_string().contains("track namespace is reserved"));
    }

    #[test]
    fn request_intervention_rejects_file_target_absent_from_snapshot() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let error = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired)
                .with_target(InterventionTargetSelector::file("missing.rs")),
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("not present in captured snapshot")
        );
    }

    #[test]
    fn request_intervention_supports_file_range_targets() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired)
                .with_target(InterventionTargetSelector::range(
                    "src/lib.rs",
                    Side::New,
                    2,
                    Some(2),
                )),
        )
        .unwrap();

        assert_eq!(result.review_unit_id, capture.review_unit_id);
        assert!(matches!(
            result.target,
            ReviewTargetRef::Range { start_line: 2, .. }
        ));
    }

    #[test]
    fn request_intervention_validates_native_observation_target() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let observation = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Observation"),
        )
        .unwrap();

        let result = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired)
                .with_target(InterventionTargetSelector::observation(
                    observation.observation_id.clone(),
                )),
        )
        .unwrap();

        assert_eq!(
            result.target,
            ReviewTargetRef::Observation {
                review_unit_id: capture.review_unit_id,
                observation_id: observation.observation_id,
            }
        );
    }

    #[test]
    fn request_intervention_rejects_unknown_observation_target() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let error = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired)
                .with_target(InterventionTargetSelector::observation(ObservationId::new(
                    "obs:sha256:missing",
                ))),
        )
        .unwrap_err();

        assert!(error.to_string().contains("unknown observation target"));
    }

    #[test]
    fn large_intervention_body_is_stored_as_internal_body_artifact() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let body = "x".repeat(super::super::body_artifact::BODY_INLINE_LIMIT + 1);

        let result = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired)
                .with_body(body),
        )
        .unwrap();

        assert!(
            result
                .body_content_hash
                .as_deref()
                .unwrap()
                .starts_with("sha256:")
        );
        assert!(
            !format!("{result:?}").contains("artifacts/notes/"),
            "workflow result must not expose internal artifact paths"
        );

        let artifacts = std::fs::read_dir(repo.path().join(".shore/artifacts/notes"))
            .unwrap()
            .collect::<Vec<_>>();
        assert_eq!(artifacts.len(), 1);
    }

    #[test]
    fn explicit_same_idempotency_key_with_different_payload_conflicts() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("First")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired)
                .with_idempotency_key("retry-key"),
        )
        .unwrap();
        let error = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Second")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired)
                .with_idempotency_key("retry-key"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("event conflict"));
    }

    #[test]
    fn list_interventions_defaults_to_open_interventions() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = request_intervention(open_request(repo.path(), "First")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = request_intervention(open_request(repo.path(), "Second")).unwrap();

        let result = list_interventions(InterventionListOptions::new(repo.path())).unwrap();

        let ids = result
            .interventions
            .iter()
            .map(|view| view.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![first.intervention_id, second.intervention_id]);
    }

    #[test]
    fn fetch_intervention_hydrates_body_by_id() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = request_intervention(
            open_request(repo.path(), "Need details").with_body("full request body"),
        )
        .unwrap();

        let result = fetch_intervention(
            InterventionFetchOptions::new(repo.path(), requested.intervention_id.clone())
                .with_include_body(true),
        )
        .unwrap();

        assert_eq!(result.intervention.id, requested.intervention_id);
        assert_eq!(
            result.intervention.body.as_deref(),
            Some("full request body")
        );
    }

    #[test]
    fn list_interventions_all_includes_resolved_and_ambiguous_statuses() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let open = request_intervention(open_request(repo.path(), "Open")).unwrap();
        let resolved = request_intervention(open_request(repo.path(), "Resolved")).unwrap();
        let ambiguous = request_intervention(open_request(repo.path(), "Ambiguous")).unwrap();
        write_resolution_event(
            repo.path(),
            &resolved,
            "approved",
            InterventionResolutionOutcome::Approved,
        );
        write_resolution_event(
            repo.path(),
            &ambiguous,
            "approved",
            InterventionResolutionOutcome::Approved,
        );
        write_resolution_event(
            repo.path(),
            &ambiguous,
            "rejected",
            InterventionResolutionOutcome::Rejected,
        );

        let result = list_interventions(
            InterventionListOptions::new(repo.path()).with_status(InterventionStatusFilter::All),
        )
        .unwrap();

        assert_status(&result, &open.intervention_id, InterventionStatus::Open);
        assert_status(
            &result,
            &resolved.intervention_id,
            InterventionStatus::Resolved,
        );
        assert_status(
            &result,
            &ambiguous.intervention_id,
            InterventionStatus::Ambiguous,
        );
    }

    #[test]
    fn list_interventions_filters_by_track_mode_and_file() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let matching = request_intervention(
            open_request(repo.path(), "Match")
                .with_track("agent:codex")
                .with_mode(InterventionMode::Advisory)
                .with_target(InterventionTargetSelector::file("src/lib.rs")),
        )
        .unwrap();
        request_intervention(
            open_request(repo.path(), "Wrong track")
                .with_track("agent:claude")
                .with_mode(InterventionMode::Advisory)
                .with_target(InterventionTargetSelector::file("src/lib.rs")),
        )
        .unwrap();
        request_intervention(
            open_request(repo.path(), "Wrong mode")
                .with_track("agent:codex")
                .with_mode(InterventionMode::Blocking)
                .with_target(InterventionTargetSelector::file("src/lib.rs")),
        )
        .unwrap();

        let result = list_interventions(
            InterventionListOptions::new(repo.path())
                .with_track("agent:codex")
                .with_mode(InterventionMode::Advisory)
                .with_file("src/lib.rs"),
        )
        .unwrap();

        assert_eq!(result.interventions.len(), 1);
        assert_eq!(result.interventions[0].id, matching.intervention_id);
    }

    #[test]
    fn list_interventions_include_body_hydrates_artifact_backed_bodies() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let body = "large ".repeat(1000);
        let requested =
            request_intervention(open_request(repo.path(), "Large body").with_body(body.clone()))
                .unwrap();

        let result =
            list_interventions(InterventionListOptions::new(repo.path()).with_include_body(true))
                .unwrap();

        let view = result
            .interventions
            .iter()
            .find(|view| view.id == requested.intervention_id)
            .unwrap();
        assert_eq!(view.body.as_deref(), Some(body.as_str()));
        assert!(
            !format!("{result:?}").contains("artifacts/notes/"),
            "list result must not expose internal artifact paths"
        );
    }

    #[test]
    fn fetch_intervention_rejects_unknown_id() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let error = fetch_intervention(InterventionFetchOptions::new(
            repo.path(),
            InterventionId::new("intervention:sha256:missing"),
        ))
        .unwrap_err();

        assert!(error.to_string().contains("unknown intervention"));
    }

    #[test]
    fn resolved_intervention_view_includes_resolution_details() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = request_intervention(open_request(repo.path(), "Resolved")).unwrap();
        let resolution_event_id = write_resolution_event(
            repo.path(),
            &requested,
            "approved",
            InterventionResolutionOutcome::Approved,
        );

        let result = fetch_intervention(InterventionFetchOptions::new(
            repo.path(),
            requested.intervention_id.clone(),
        ))
        .unwrap();

        assert_eq!(result.intervention.status, InterventionStatus::Resolved);
        assert_eq!(result.intervention.resolutions.len(), 1);
        assert_eq!(
            result.intervention.resolutions[0].event_id,
            resolution_event_id
        );
        assert_eq!(
            result.intervention.resolutions[0].outcome,
            InterventionResolutionOutcome::Approved
        );
    }

    #[test]
    fn multiple_resolution_events_make_intervention_ambiguous() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = request_intervention(open_request(repo.path(), "Ambiguous")).unwrap();
        write_resolution_event(
            repo.path(),
            &requested,
            "approved",
            InterventionResolutionOutcome::Approved,
        );
        write_resolution_event(
            repo.path(),
            &requested,
            "rejected",
            InterventionResolutionOutcome::Rejected,
        );

        let result = fetch_intervention(InterventionFetchOptions::new(
            repo.path(),
            requested.intervention_id,
        ))
        .unwrap();

        assert_eq!(result.intervention.status, InterventionStatus::Ambiguous);
        assert_eq!(result.intervention.resolutions.len(), 2);
    }

    #[test]
    fn sort_intervention_views_uses_created_at_then_event_id() {
        let mut views = vec![
            intervention_view_for_sort("intervention:sha256:b", "evt:sha256:b", "unix-ms:2"),
            intervention_view_for_sort("intervention:sha256:c", "evt:sha256:c", "unix-ms:1"),
            intervention_view_for_sort("intervention:sha256:a", "evt:sha256:a", "unix-ms:1"),
        ];

        sort_intervention_views(&mut views);

        assert_eq!(
            views
                .iter()
                .map(|view| view.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "intervention:sha256:a",
                "intervention:sha256:c",
                "intervention:sha256:b"
            ]
        );
    }

    #[test]
    fn resolve_intervention_records_resolution_and_closes_open_count() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = request_intervention(open_request(repo.path(), "Need approval")).unwrap();

        let resolved = resolve_intervention(
            InterventionResolveOptions::new(repo.path(), requested.intervention_id.clone())
                .with_outcome(InterventionResolutionOutcome::Approved)
                .with_reason("approved locally"),
        )
        .unwrap();

        assert!(
            resolved
                .intervention_resolution_id
                .as_str()
                .starts_with("intervention-resolution:sha256:")
        );
        assert_eq!(resolved.outcome, InterventionResolutionOutcome::Approved);
        assert_eq!(resolved.events_created_by_type["intervention_resolved"], 1);

        let state = SessionState::from_events(
            &EventStore::open(repo.path().join(".shore"))
                .list_events()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(state.intervention_count, 1);
        assert_eq!(state.open_intervention_count, 0);
        assert_eq!(state.open_blocking_intervention_count, 0);
    }

    #[test]
    fn resolving_unknown_intervention_fails() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let error = resolve_intervention(
            InterventionResolveOptions::new(
                repo.path(),
                InterventionId::new("intervention:sha256:missing"),
            )
            .with_outcome(InterventionResolutionOutcome::Dismissed),
        )
        .unwrap_err();

        assert!(error.to_string().contains("unknown intervention"));
    }

    #[test]
    fn resolve_intervention_is_idempotent_for_same_logical_input() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = request_intervention(open_request(repo.path(), "Need approval")).unwrap();
        let options =
            InterventionResolveOptions::new(repo.path(), requested.intervention_id.clone())
                .with_outcome(InterventionResolutionOutcome::Approved)
                .with_reason("approved locally");

        let first = resolve_intervention(options.clone()).unwrap();
        let second = resolve_intervention(options).unwrap();

        assert_eq!(
            first.intervention_resolution_id,
            second.intervention_resolution_id
        );
        assert_eq!(first.events_created, 1);
        assert_eq!(second.events_created, 0);
        assert_eq!(second.events_existing, 1);
    }

    #[test]
    fn explicit_same_resolution_idempotency_key_with_different_payload_conflicts() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = request_intervention(open_request(repo.path(), "Need approval")).unwrap();

        resolve_intervention(
            InterventionResolveOptions::new(repo.path(), requested.intervention_id.clone())
                .with_outcome(InterventionResolutionOutcome::Approved)
                .with_idempotency_key("retry-key"),
        )
        .unwrap();
        let error = resolve_intervention(
            InterventionResolveOptions::new(repo.path(), requested.intervention_id.clone())
                .with_outcome(InterventionResolutionOutcome::Rejected)
                .with_idempotency_key("retry-key"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("event conflict"));
    }

    #[test]
    fn resolving_twice_with_different_outcomes_is_append_only_and_ambiguous() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = request_intervention(open_request(repo.path(), "Need approval")).unwrap();

        resolve_intervention(
            InterventionResolveOptions::new(repo.path(), requested.intervention_id.clone())
                .with_outcome(InterventionResolutionOutcome::Approved),
        )
        .unwrap();
        resolve_intervention(
            InterventionResolveOptions::new(repo.path(), requested.intervention_id.clone())
                .with_outcome(InterventionResolutionOutcome::Rejected),
        )
        .unwrap();

        let result = fetch_intervention(InterventionFetchOptions::new(
            repo.path(),
            requested.intervention_id,
        ))
        .unwrap();

        assert_eq!(result.intervention.status, InterventionStatus::Ambiguous);
        assert_eq!(result.intervention.resolutions.len(), 2);
    }

    #[test]
    fn large_resolution_reason_is_stored_as_internal_body_artifact() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let requested = request_intervention(open_request(repo.path(), "Need approval")).unwrap();
        let reason = "resolved ".repeat(1000);

        let result = resolve_intervention(
            InterventionResolveOptions::new(repo.path(), requested.intervention_id)
                .with_outcome(InterventionResolutionOutcome::Approved)
                .with_reason(reason),
        )
        .unwrap();

        assert!(
            result
                .reason_content_hash
                .as_deref()
                .unwrap()
                .starts_with("sha256:")
        );
        assert!(
            !format!("{result:?}").contains("artifacts/notes/"),
            "resolve result must not expose internal artifact paths"
        );
    }

    fn modified_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 {\n    1\n}\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 {\n    2\n}\n");
        repo
    }

    fn open_request(repo: &Path, title: &str) -> InterventionRequestOptions {
        InterventionRequestOptions::new(repo)
            .with_track("human:kevin")
            .with_title(title)
            .with_reason_code(InterventionReasonCode::ManualDecisionRequired)
    }

    fn write_resolution_event(
        repo: &Path,
        requested: &InterventionRequestResult,
        source_key: &str,
        outcome: InterventionResolutionOutcome,
    ) -> EventId {
        let resolution_id =
            InterventionResolutionId::new(format!("intervention-resolution:sha256:{}", source_key));
        let payload = InterventionResolvedPayload {
            intervention_resolution_id: resolution_id,
            intervention_id: requested.intervention_id.clone(),
            outcome,
            reason: Some("resolved".to_owned()),
            reason_artifact_path: None,
            reason_byte_size: Some(8),
            reason_content_hash: Some("sha256:resolved".to_owned()),
        };
        let event = ShoreEvent::new(
            EventType::InterventionResolved,
            InterventionResolvedPayload::idempotency_key(&requested.intervention_id, source_key),
            EventTarget {
                review_id: crate::model::ReviewId::new("review:default"),
                work_unit_id: None,
                review_unit_id: Some(requested.review_unit_id.clone()),
                revision_id: None,
                snapshot_id: None,
                track_id: Some(requested.track_id.clone()),
                subject: Some(ReviewTargetRef::Intervention {
                    review_unit_id: requested.review_unit_id.clone(),
                    intervention_id: requested.intervention_id.clone(),
                }),
            },
            Writer::shore_local_reviewer("test"),
            payload,
            current_timestamp(),
        )
        .unwrap();
        let event_id = event.event_id.clone();
        EventStore::open(repo.join(".shore"))
            .record_event_once(&event)
            .unwrap();
        event_id
    }

    fn assert_status(
        result: &InterventionListResult,
        id: &InterventionId,
        status: InterventionStatus,
    ) {
        let view = result
            .interventions
            .iter()
            .find(|view| &view.id == id)
            .unwrap();
        assert_eq!(view.status, status);
    }

    fn intervention_view_for_sort(
        intervention_id: &str,
        event_id: &str,
        created_at: &str,
    ) -> InterventionView {
        let review_unit_id = ReviewUnitId::new("review-unit:sha256:one");
        InterventionView {
            id: InterventionId::new(intervention_id),
            event_id: EventId::new(event_id),
            track_id: TrackId::new("agent:codex"),
            target: ReviewTargetRef::ReviewUnit { review_unit_id },
            mode: InterventionMode::Blocking,
            reason_code: InterventionReasonCode::ManualDecisionRequired,
            title: "sort".to_owned(),
            body: None,
            body_content_hash: None,
            status: InterventionStatus::Open,
            resolutions: vec![],
            created_at: created_at.to_owned(),
            writer: Writer::shore_local_reviewer("test"),
        }
    }

    struct TestRepo {
        root: tempfile::TempDir,
    }

    impl TestRepo {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("create temp git repository directory");
            let repo = Self { root };

            repo.git(["init"]);
            repo.git(["config", "user.name", "Shore Tests"]);
            repo.git(["config", "user.email", "shore-tests@example.com"]);
            repo.git(["config", "commit.gpgsign", "false"]);

            repo
        }

        fn path(&self) -> &Path {
            self.root.path()
        }

        fn write(&self, path: &str, contents: &str) {
            let path = self.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }

        fn commit_all(&self, message: &str) {
            self.git(["add", "."]);
            self.git(["commit", "-m", message]);
        }

        fn git<I, S>(&self, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<std::ffi::OsStr>,
        {
            let output = Command::new("git")
                .args(args)
                .current_dir(self.path())
                .output()
                .expect("run git command");
            assert!(
                output.status.success(),
                "git failed\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
