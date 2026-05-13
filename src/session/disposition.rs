use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::canonical_hash::{sha256_bytes_hex, sha256_json_hex, sha256_json_prefixed};
use crate::error::{Result, ShoreError};
use crate::model::{
    DispositionId, EventId, InterventionId, ObservationId, ReviewTargetRef, ReviewUnitId, Side,
    TrackId,
};
use crate::session::body_artifact::load_body_artifact;
use crate::session::event::{
    EventTarget, EventType, InterventionRequestedPayload, ReviewDisposition,
    ReviewDispositionRecordedPayload, ReviewObservationRecordedPayload, ShoreEvent,
};
use crate::session::event_context::{current_timestamp, reviewer_from_git_config};
use crate::session::observation::{
    ObservationTargetSelector, ResolvedReviewUnit, resolve_observation_target,
    resolve_review_unit_for_observation, staged_body, validated_track_id,
};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store_init::{ShoreStorePaths, prepare_shore_writer};
use crate::storage::{Durability, EventStore, EventWriteOutcome, LocalStorage};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispositionAddOptions {
    repo: PathBuf,
    review_unit_id: Option<ReviewUnitId>,
    track: Option<String>,
    disposition: Option<ReviewDisposition>,
    summary: Option<String>,
    target: DispositionTargetSelector,
    replaces_disposition_ids: Vec<DispositionId>,
    related_observation_ids: Vec<ObservationId>,
    related_intervention_ids: Vec<InterventionId>,
    overrides: Vec<DispositionOverrideSelector>,
    idempotency_key: Option<String>,
}

impl DispositionAddOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_unit_id: None,
            track: None,
            disposition: None,
            summary: None,
            target: DispositionTargetSelector::review_unit(),
            replaces_disposition_ids: Vec::new(),
            related_observation_ids: Vec::new(),
            related_intervention_ids: Vec::new(),
            overrides: Vec::new(),
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

    pub fn with_disposition(mut self, disposition: ReviewDisposition) -> Self {
        self.disposition = Some(disposition);
        self
    }

    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    pub fn with_target(mut self, target: DispositionTargetSelector) -> Self {
        self.target = target;
        self
    }

    pub fn replacing(mut self, disposition_id: DispositionId) -> Self {
        self.replaces_disposition_ids.push(disposition_id);
        self
    }

    pub fn related_observation(mut self, observation_id: ObservationId) -> Self {
        self.related_observation_ids.push(observation_id);
        self
    }

    pub fn related_intervention(mut self, intervention_id: InterventionId) -> Self {
        self.related_intervention_ids.push(intervention_id);
        self
    }

    pub fn overriding_observation(mut self, observation_id: ObservationId) -> Self {
        self.overrides
            .push(DispositionOverrideSelector::observation(observation_id));
        self
    }

    pub fn overriding_intervention(mut self, intervention_id: InterventionId) -> Self {
        self.overrides
            .push(DispositionOverrideSelector::intervention(intervention_id));
        self
    }

    pub fn overriding_disposition(mut self, disposition_id: DispositionId) -> Self {
        self.overrides
            .push(DispositionOverrideSelector::disposition(disposition_id));
        self
    }

    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispositionAddResult {
    pub review_unit_id: ReviewUnitId,
    pub disposition_id: DispositionId,
    pub event_id: EventId,
    pub track_id: TrackId,
    pub target: ReviewTargetRef,
    pub disposition: ReviewDisposition,
    pub summary_content_hash: Option<String>,
    pub events_created: usize,
    pub events_existing: usize,
    pub events_created_by_type: BTreeMap<String, usize>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispositionShowOptions {
    repo: PathBuf,
    review_unit_id: Option<ReviewUnitId>,
    track: Option<String>,
    include_summary: bool,
    include_all: bool,
}

impl DispositionShowOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_unit_id: None,
            track: None,
            include_summary: false,
            include_all: false,
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

    pub fn with_include_summary(mut self, include_summary: bool) -> Self {
        self.include_summary = include_summary;
        self
    }

    pub fn with_all(mut self, include_all: bool) -> Self {
        self.include_all = include_all;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispositionShowResult {
    pub review_unit_id: ReviewUnitId,
    pub filters: DispositionShowFilters,
    pub current: CurrentDispositionView,
    pub dispositions: Vec<DispositionView>,
    /// Diagnostics come from the full replayed event set, not only the filtered ReviewUnit.
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispositionShowFilters {
    pub track_id: Option<TrackId>,
    pub include_summary: bool,
    pub include_all: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CurrentDispositionView {
    pub status: CurrentDispositionStatus,
    pub dispositions: Vec<DispositionView>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CurrentDispositionStatus {
    None,
    Resolved,
    Ambiguous,
}

impl CurrentDispositionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Resolved => "resolved",
            Self::Ambiguous => "ambiguous",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispositionRecordStatus {
    Current,
    Replaced,
}

impl DispositionRecordStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Replaced => "replaced",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispositionView {
    pub id: DispositionId,
    pub event_id: EventId,
    pub track_id: TrackId,
    pub target: ReviewTargetRef,
    pub disposition: ReviewDisposition,
    pub summary: Option<String>,
    pub summary_content_hash: Option<String>,
    pub status: DispositionRecordStatus,
    pub replaces: Vec<DispositionId>,
    pub related_observations: Vec<ObservationId>,
    pub related_interventions: Vec<InterventionId>,
    pub overrides: Vec<ReviewTargetRef>,
    pub created_at: String,
    pub writer: crate::session::event::Writer,
}

pub fn record_disposition(options: DispositionAddOptions) -> Result<DispositionAddResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let worktree_root = paths.worktree_root();
    let shore_dir = paths.shore_dir();
    let storage = LocalStorage::new(shore_dir);
    prepare_shore_writer(&paths, &storage)?;

    let event_store = EventStore::open(shore_dir);
    let events = event_store.list_events()?;
    let resolved = resolve_review_unit_for_observation(&events, options.review_unit_id.as_ref())?;
    let target = resolve_disposition_target(worktree_root, &events, &resolved, &options.target)?;
    let track_id = validated_track_id(
        options
            .track
            .as_deref()
            .ok_or_else(|| ShoreError::Message("track is required".to_owned()))?,
    )?;
    let disposition = options
        .disposition
        .ok_or_else(|| ShoreError::Message("disposition is required".to_owned()))?;
    let relationships = resolve_disposition_relationships(
        &events,
        &resolved,
        &DispositionRelationships {
            replaces_disposition_ids: options.replaces_disposition_ids,
            related_observation_ids: options.related_observation_ids,
            related_intervention_ids: options.related_intervention_ids,
            overrides: options.overrides,
        },
        disposition,
        options.summary.as_deref(),
    )?;
    let writer = reviewer_from_git_config(worktree_root);
    let summary_content_hash = options
        .summary
        .as_ref()
        .map(|summary| format!("sha256:{}", sha256_bytes_hex(summary.as_bytes())));
    let (summary, summary_artifact_path, summary_artifact_bytes, summary_byte_size) =
        staged_body(options.summary.as_deref())?;
    let disposition_id = build_disposition_id(DispositionIdMaterial {
        review_unit_id: &resolved.review_unit_id,
        track_id: &track_id,
        target: &target.target,
        disposition,
        summary_content_hash: summary_content_hash.as_deref(),
        replaces_disposition_ids: &relationships.replaces_disposition_ids,
        related_observation_ids: &relationships.related_observation_ids,
        related_intervention_ids: &relationships.related_intervention_ids,
        overrides: &relationships.overrides,
        writer_actor_id: writer.actor_id.as_str(),
    })?;
    let source_key = options
        .idempotency_key
        .as_deref()
        .unwrap_or_else(|| disposition_id.as_str());
    let idempotency_key = ReviewDispositionRecordedPayload::idempotency_key(
        &resolved.review_unit_id,
        &track_id,
        source_key,
    );

    if !event_store.event_exists(&idempotency_key)?
        && let (Some(artifact_path), Some(bytes)) = (
            summary_artifact_path.as_deref(),
            summary_artifact_bytes.as_ref(),
        )
    {
        // Summary artifacts are content-addressed. A crash before the event commit can leave a
        // harmless orphan that a retry reuses or overwrites with the same bytes.
        storage.write_bytes_atomic(Path::new(artifact_path), bytes, Durability::Durable)?;
    }

    let event = ShoreEvent::new(
        EventType::ReviewDispositionRecorded,
        idempotency_key,
        EventTarget {
            review_id: resolved.review_id,
            work_unit_id: None,
            review_unit_id: Some(resolved.review_unit_id.clone()),
            revision_id: Some(resolved.revision_id),
            snapshot_id: Some(resolved.snapshot_id),
            track_id: Some(track_id.clone()),
            subject: Some(target.target.clone()),
        },
        writer,
        ReviewDispositionRecordedPayload {
            disposition_id: disposition_id.clone(),
            target: target.target.clone(),
            disposition,
            summary,
            summary_artifact_path,
            summary_byte_size,
            summary_content_hash: summary_content_hash.clone(),
            replaces_disposition_ids: relationships.replaces_disposition_ids,
            related_observation_ids: relationships.related_observation_ids,
            related_intervention_ids: relationships.related_intervention_ids,
            overrides: relationships.overrides,
        },
        current_timestamp(),
    )?;
    let event_id = event.event_id.clone();

    let mut events_created_by_type = BTreeMap::new();
    let (events_created, events_existing) = match event_store.record_event_once(&event)? {
        EventWriteOutcome::Created => {
            events_created_by_type.insert("review_disposition_recorded".to_owned(), 1);
            (1, 0)
        }
        EventWriteOutcome::Existing => (0, 1),
    };

    let state = SessionState::from_events(&event_store.list_events()?)?;
    storage.write_json_atomic(&paths.state_path(), &state, Durability::Projection)?;

    Ok(DispositionAddResult {
        review_unit_id: resolved.review_unit_id,
        disposition_id,
        event_id,
        track_id,
        target: target.target,
        disposition,
        summary_content_hash,
        events_created,
        events_existing,
        events_created_by_type,
        diagnostics: state.diagnostics,
    })
}

pub fn show_dispositions(options: DispositionShowOptions) -> Result<DispositionShowResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let shore_dir = paths.shore_dir();
    let events = EventStore::open(shore_dir).list_events()?;
    let resolved = resolve_review_unit_for_observation(&events, options.review_unit_id.as_ref())?;
    let track_filter = options
        .track
        .as_deref()
        .map(validated_track_id)
        .transpose()?;
    let records = collect_disposition_records(&events, &resolved)?;
    let replaced_ids = records
        .values()
        .flat_map(|record| record.payload.replaces_disposition_ids.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>();
    let mut all_views = Vec::new();

    for record in records.into_values() {
        if track_filter
            .as_ref()
            .is_some_and(|filter| filter != &record.track_id)
        {
            continue;
        }

        let view = disposition_view_from_event(
            shore_dir,
            record.event,
            record.payload,
            record.track_id,
            &replaced_ids,
            options.include_summary,
        )?;
        all_views.push(view);
    }

    sort_disposition_views(&mut all_views);
    let current_dispositions = all_views
        .iter()
        .filter(|view| view.status == DispositionRecordStatus::Current)
        .cloned()
        .collect::<Vec<_>>();
    let current_status = match current_dispositions.len() {
        0 => CurrentDispositionStatus::None,
        1 => CurrentDispositionStatus::Resolved,
        _ => CurrentDispositionStatus::Ambiguous,
    };
    let dispositions = if options.include_all {
        all_views
    } else {
        current_dispositions.clone()
    };
    // Reuse the state reducer for diagnostics so duplicate/corrupt-event policy stays
    // shared with state.json and other readers; row filtering is disposition-local.
    let diagnostics = SessionState::from_events(&events)?.diagnostics;

    Ok(DispositionShowResult {
        review_unit_id: resolved.review_unit_id,
        filters: DispositionShowFilters {
            track_id: track_filter,
            include_summary: options.include_summary,
            include_all: options.include_all,
        },
        current: CurrentDispositionView {
            status: current_status,
            dispositions: current_dispositions,
        },
        dispositions,
        diagnostics,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DispositionTargetSelector {
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
    Intervention {
        intervention_id: InterventionId,
    },
    Disposition {
        disposition_id: DispositionId,
    },
}

impl DispositionTargetSelector {
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

    pub fn intervention(intervention_id: InterventionId) -> Self {
        Self::Intervention { intervention_id }
    }

    pub fn disposition(disposition_id: DispositionId) -> Self {
        Self::Disposition { disposition_id }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DispositionRelationships {
    pub replaces_disposition_ids: Vec<DispositionId>,
    pub related_observation_ids: Vec<ObservationId>,
    pub related_intervention_ids: Vec<InterventionId>,
    pub overrides: Vec<DispositionOverrideSelector>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DispositionOverrideSelector {
    Observation { observation_id: ObservationId },
    Intervention { intervention_id: InterventionId },
    Disposition { disposition_id: DispositionId },
}

impl DispositionOverrideSelector {
    pub fn observation(observation_id: ObservationId) -> Self {
        Self::Observation { observation_id }
    }

    pub fn intervention(intervention_id: InterventionId) -> Self {
        Self::Intervention { intervention_id }
    }

    pub fn disposition(disposition_id: DispositionId) -> Self {
        Self::Disposition { disposition_id }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedDispositionTarget {
    pub target: ReviewTargetRef,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedDispositionRelationships {
    pub replaces_disposition_ids: Vec<DispositionId>,
    pub related_observation_ids: Vec<ObservationId>,
    pub related_intervention_ids: Vec<InterventionId>,
    pub overrides: Vec<ReviewTargetRef>,
}

pub(crate) fn resolve_disposition_target(
    repo: &Path,
    events: &[ShoreEvent],
    resolved: &ResolvedReviewUnit,
    selector: &DispositionTargetSelector,
) -> Result<ResolvedDispositionTarget> {
    let target = match selector {
        DispositionTargetSelector::ReviewUnit => {
            resolve_observation_target(repo, resolved, &ObservationTargetSelector::review_unit())?
        }
        DispositionTargetSelector::File { path } => {
            resolve_observation_target(repo, resolved, &ObservationTargetSelector::file(path))?
        }
        DispositionTargetSelector::Range {
            path,
            side,
            start_line,
            end_line,
        } => resolve_observation_target(
            repo,
            resolved,
            &ObservationTargetSelector::range(path, *side, *start_line, *end_line),
        )?,
        DispositionTargetSelector::Observation { observation_id } => {
            resolve_observation_ref(events, resolved, observation_id)?
        }
        DispositionTargetSelector::Intervention { intervention_id } => {
            resolve_intervention_ref(events, resolved, intervention_id)?
        }
        DispositionTargetSelector::Disposition { disposition_id } => {
            resolve_disposition_ref(events, resolved, disposition_id)?
        }
    };

    Ok(ResolvedDispositionTarget { target })
}

pub(crate) fn resolve_disposition_relationships(
    events: &[ShoreEvent],
    resolved: &ResolvedReviewUnit,
    relationships: &DispositionRelationships,
    disposition: ReviewDisposition,
    summary: Option<&str>,
) -> Result<ResolvedDispositionRelationships> {
    if disposition == ReviewDisposition::Overridden {
        if summary.is_none_or(|summary| summary.trim().is_empty()) {
            return Err(ShoreError::Message(
                "summary is required for overridden disposition".to_owned(),
            ));
        }
        if relationships.overrides.is_empty() {
            return Err(ShoreError::Message(
                "override reference is required for overridden disposition".to_owned(),
            ));
        }
    }

    for observation_id in &relationships.related_observation_ids {
        resolve_observation_ref(events, resolved, observation_id)?;
    }
    for intervention_id in &relationships.related_intervention_ids {
        resolve_intervention_ref(events, resolved, intervention_id)?;
    }
    for disposition_id in &relationships.replaces_disposition_ids {
        resolve_disposition_ref(events, resolved, disposition_id)?;
    }

    let mut overrides = Vec::with_capacity(relationships.overrides.len());
    for override_selector in &relationships.overrides {
        overrides.push(resolve_override_ref(events, resolved, override_selector)?);
    }

    Ok(ResolvedDispositionRelationships {
        replaces_disposition_ids: sorted_unique(relationships.replaces_disposition_ids.clone()),
        related_observation_ids: sorted_unique(relationships.related_observation_ids.clone()),
        related_intervention_ids: sorted_unique(relationships.related_intervention_ids.clone()),
        overrides: sorted_unique_targets(overrides)?,
    })
}

fn resolve_override_ref(
    events: &[ShoreEvent],
    resolved: &ResolvedReviewUnit,
    selector: &DispositionOverrideSelector,
) -> Result<ReviewTargetRef> {
    match selector {
        DispositionOverrideSelector::Observation { observation_id } => {
            resolve_observation_ref(events, resolved, observation_id)
        }
        DispositionOverrideSelector::Intervention { intervention_id } => {
            resolve_intervention_ref(events, resolved, intervention_id)
        }
        DispositionOverrideSelector::Disposition { disposition_id } => {
            resolve_disposition_ref(events, resolved, disposition_id)
        }
    }
}

fn resolve_observation_ref(
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

fn resolve_intervention_ref(
    events: &[ShoreEvent],
    resolved: &ResolvedReviewUnit,
    intervention_id: &InterventionId,
) -> Result<ReviewTargetRef> {
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::InterventionRequested)
    {
        if event.target.review_unit_id.as_ref() != Some(&resolved.review_unit_id) {
            continue;
        }

        let payload: InterventionRequestedPayload = serde_json::from_value(event.payload.clone())?;
        if &payload.intervention_id == intervention_id {
            return Ok(ReviewTargetRef::Intervention {
                review_unit_id: resolved.review_unit_id.clone(),
                intervention_id: intervention_id.clone(),
            });
        }
    }

    Err(ShoreError::Message(format!(
        "unknown intervention target: {}",
        intervention_id.as_str()
    )))
}

fn resolve_disposition_ref(
    events: &[ShoreEvent],
    resolved: &ResolvedReviewUnit,
    disposition_id: &DispositionId,
) -> Result<ReviewTargetRef> {
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewDispositionRecorded)
    {
        if event.target.review_unit_id.as_ref() != Some(&resolved.review_unit_id) {
            continue;
        }

        let payload: ReviewDispositionRecordedPayload =
            serde_json::from_value(event.payload.clone())?;
        if &payload.disposition_id == disposition_id {
            return Ok(ReviewTargetRef::Disposition {
                review_unit_id: resolved.review_unit_id.clone(),
                disposition_id: disposition_id.clone(),
            });
        }
    }

    Err(ShoreError::Message(format!(
        "unknown disposition target: {}",
        disposition_id.as_str()
    )))
}

struct DispositionEventRecord<'a> {
    event: &'a ShoreEvent,
    payload: ReviewDispositionRecordedPayload,
    track_id: TrackId,
}

fn collect_disposition_records<'a>(
    events: &'a [ShoreEvent],
    resolved: &ResolvedReviewUnit,
) -> Result<BTreeMap<DispositionId, DispositionEventRecord<'a>>> {
    let mut records: BTreeMap<DispositionId, DispositionEventRecord<'a>> = BTreeMap::new();
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewDispositionRecorded)
    {
        if event.target.review_unit_id.as_ref() != Some(&resolved.review_unit_id) {
            continue;
        }

        let payload: ReviewDispositionRecordedPayload =
            serde_json::from_value(event.payload.clone())?;
        let track_id =
            event.target.track_id.clone().ok_or_else(|| {
                ShoreError::Message("disposition event missing track id".to_owned())
            })?;
        let disposition_id = payload.disposition_id.clone();
        // Duplicate semantic events are reported by the state reducer diagnostics. This
        // read model keeps one stable representative so replacement/current projection
        // stays bounded even if duplicate payloads diverge.
        let replace_record = records.get(&disposition_id).is_none_or(|record| {
            // Event IDs are deterministic storage addresses, not causal order. Pick the
            // lowest one only as a stable representative for duplicate semantic facts.
            event.event_id.as_str() < record.event.event_id.as_str()
        });
        if replace_record {
            records.insert(
                disposition_id,
                DispositionEventRecord {
                    event,
                    payload,
                    track_id,
                },
            );
        }
    }

    Ok(records)
}

fn disposition_view_from_event(
    shore_dir: &Path,
    event: &ShoreEvent,
    payload: ReviewDispositionRecordedPayload,
    track_id: TrackId,
    replaced_ids: &std::collections::BTreeSet<DispositionId>,
    include_summary: bool,
) -> Result<DispositionView> {
    let summary = if include_summary {
        disposition_summary(shore_dir, &payload)?
    } else {
        None
    };
    let status = if replaced_ids.contains(&payload.disposition_id) {
        DispositionRecordStatus::Replaced
    } else {
        DispositionRecordStatus::Current
    };
    let replaces = sorted_unique(payload.replaces_disposition_ids);
    let related_observations = sorted_unique(payload.related_observation_ids);
    let related_interventions = sorted_unique(payload.related_intervention_ids);

    Ok(DispositionView {
        id: payload.disposition_id,
        event_id: event.event_id.clone(),
        track_id,
        target: payload.target,
        disposition: payload.disposition,
        summary,
        summary_content_hash: payload.summary_content_hash,
        status,
        replaces,
        related_observations,
        related_interventions,
        overrides: payload.overrides,
        created_at: event.occurred_at.clone(),
        writer: event.writer.clone(),
    })
}

fn disposition_summary(
    shore_dir: &Path,
    payload: &ReviewDispositionRecordedPayload,
) -> Result<Option<String>> {
    if payload.summary.is_some() {
        return Ok(payload.summary.clone());
    }
    match payload.summary_artifact_path.as_deref() {
        Some(path) => load_body_artifact(shore_dir, path),
        None => Ok(None),
    }
}

fn sort_disposition_views(dispositions: &mut [DispositionView]) {
    dispositions.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.event_id.as_str().cmp(right.event_id.as_str()))
    });
}

fn sorted_unique<T: Ord>(mut values: Vec<T>) -> Vec<T> {
    values.sort();
    values.dedup();
    values
}

fn sorted_unique_targets(targets: Vec<ReviewTargetRef>) -> Result<Vec<ReviewTargetRef>> {
    let mut keyed_targets = targets
        .into_iter()
        .map(|target| Ok((sha256_json_hex(&target)?, target)))
        .collect::<Result<Vec<_>>>()?;
    keyed_targets.sort_by(|(left, _), (right, _)| left.cmp(right));
    keyed_targets.dedup_by(|(left, _), (right, _)| left == right);
    Ok(keyed_targets
        .into_iter()
        .map(|(_, target)| target)
        .collect())
}

struct DispositionIdMaterial<'a> {
    review_unit_id: &'a ReviewUnitId,
    track_id: &'a TrackId,
    target: &'a ReviewTargetRef,
    disposition: ReviewDisposition,
    summary_content_hash: Option<&'a str>,
    replaces_disposition_ids: &'a [DispositionId],
    related_observation_ids: &'a [ObservationId],
    related_intervention_ids: &'a [InterventionId],
    overrides: &'a [ReviewTargetRef],
    writer_actor_id: &'a str,
}

fn build_disposition_id(material: DispositionIdMaterial<'_>) -> Result<DispositionId> {
    let mut replaces = material
        .replaces_disposition_ids
        .iter()
        .map(|disposition_id| disposition_id.as_str())
        .collect::<Vec<_>>();
    replaces.sort();
    let mut related_observations = material
        .related_observation_ids
        .iter()
        .map(|observation_id| observation_id.as_str())
        .collect::<Vec<_>>();
    related_observations.sort();
    let mut related_interventions = material
        .related_intervention_ids
        .iter()
        .map(|intervention_id| intervention_id.as_str())
        .collect::<Vec<_>>();
    related_interventions.sort();
    let mut overrides = material
        .overrides
        .iter()
        .map(sha256_json_hex)
        .collect::<Result<Vec<_>>>()?;
    // Hash each override target before sorting so the disposition ID is independent of
    // serde's struct-field declaration order for ReviewTargetRef variants.
    overrides.sort();

    let digest = sha256_json_prefixed(&json!({
        "reviewUnitId": material.review_unit_id.as_str(),
        "trackId": material.track_id.as_str(),
        "target": material.target,
        "disposition": material.disposition,
        "summaryContentHash": material.summary_content_hash,
        "replacesDispositionIds": replaces,
        "relatedObservationIds": related_observations,
        "relatedInterventionIds": related_interventions,
        "overrides": overrides,
        "writerActorId": material.writer_actor_id,
    }))?;
    Ok(DispositionId::new(format!("disp:{digest}")))
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::*;
    use crate::model::{
        DispositionId, InterventionId, ObservationId, ReviewId, ReviewTargetRef, ReviewUnitId,
        RevisionId, Side, SnapshotId, TrackId,
    };
    use crate::session::event::{
        EventTarget, EventType, ReviewDisposition, ReviewDispositionRecordedPayload, ShoreEvent,
        Writer,
    };
    use crate::session::intervention::{InterventionRequestOptions, InterventionTargetSelector};
    use crate::session::observation::{
        ObservationAddOptions, ObservationTargetSelector, resolve_review_unit_for_observation,
    };
    use crate::session::{
        CaptureOptions, InterventionMode, InterventionReasonCode, capture_worktree_review,
        record_observation, request_intervention,
    };
    use crate::storage::EventStore;

    #[test]
    fn resolves_current_review_unit_as_default_disposition_target() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let resolved = resolve_review_unit_for_observation(&events, None).unwrap();

        let target = resolve_disposition_target(
            repo.path(),
            &events,
            &resolved,
            &DispositionTargetSelector::review_unit(),
        )
        .unwrap();

        assert_eq!(
            target.target,
            ReviewTargetRef::ReviewUnit {
                review_unit_id: capture.review_unit_id
            }
        );
    }

    #[test]
    fn resolves_file_and_range_targets_against_captured_snapshot() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let resolved = resolve_review_unit_for_observation(&events, None).unwrap();

        let file = resolve_disposition_target(
            repo.path(),
            &events,
            &resolved,
            &DispositionTargetSelector::file("src/lib.rs"),
        )
        .unwrap();
        let range = resolve_disposition_target(
            repo.path(),
            &events,
            &resolved,
            &DispositionTargetSelector::range("src/lib.rs", Side::New, 2, Some(3)),
        )
        .unwrap();

        assert_eq!(
            file.target,
            ReviewTargetRef::File {
                review_unit_id: capture.review_unit_id.clone(),
                file_path: "src/lib.rs".to_owned()
            }
        );
        assert_eq!(
            range.target,
            ReviewTargetRef::Range {
                review_unit_id: capture.review_unit_id,
                file_path: "src/lib.rs".to_owned(),
                side: Side::New,
                start_line: 2,
                end_line: 3
            }
        );
    }

    #[test]
    fn resolves_observation_intervention_and_disposition_targets() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let observation = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Observation")
                .with_target(ObservationTargetSelector::file("src/lib.rs")),
        )
        .unwrap();
        let intervention = request_intervention(
            InterventionRequestOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InterventionReasonCode::ManualDecisionRequired)
                .with_mode(InterventionMode::Blocking)
                .with_target(InterventionTargetSelector::review_unit()),
        )
        .unwrap();
        let disposition_id = DispositionId::new("disp:sha256:existing");
        let mut events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        events.push(disposition_event(&capture.review_unit_id, &disposition_id));
        let resolved = resolve_review_unit_for_observation(&events, None).unwrap();

        let observation_target = resolve_disposition_target(
            repo.path(),
            &events,
            &resolved,
            &DispositionTargetSelector::observation(observation.observation_id.clone()),
        )
        .unwrap();
        let intervention_target = resolve_disposition_target(
            repo.path(),
            &events,
            &resolved,
            &DispositionTargetSelector::intervention(intervention.intervention_id.clone()),
        )
        .unwrap();
        let disposition_target = resolve_disposition_target(
            repo.path(),
            &events,
            &resolved,
            &DispositionTargetSelector::disposition(disposition_id.clone()),
        )
        .unwrap();

        assert_eq!(
            observation_target.target,
            ReviewTargetRef::Observation {
                review_unit_id: capture.review_unit_id.clone(),
                observation_id: observation.observation_id
            }
        );
        assert_eq!(
            intervention_target.target,
            ReviewTargetRef::Intervention {
                review_unit_id: capture.review_unit_id.clone(),
                intervention_id: intervention.intervention_id
            }
        );
        assert_eq!(
            disposition_target.target,
            ReviewTargetRef::Disposition {
                review_unit_id: capture.review_unit_id,
                disposition_id
            }
        );
    }

    #[test]
    fn rejects_unknown_related_observation_intervention_or_replacement() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let resolved = resolve_review_unit_for_observation(&events, None).unwrap();

        let missing_observation = resolve_disposition_relationships(
            &events,
            &resolved,
            &DispositionRelationships {
                related_observation_ids: vec![ObservationId::new("obs:sha256:missing")],
                ..DispositionRelationships::default()
            },
            ReviewDisposition::Accepted,
            Some("summary"),
        )
        .unwrap_err();
        let missing_intervention = resolve_disposition_relationships(
            &events,
            &resolved,
            &DispositionRelationships {
                related_intervention_ids: vec![InterventionId::new("intervention:sha256:missing")],
                ..DispositionRelationships::default()
            },
            ReviewDisposition::Accepted,
            Some("summary"),
        )
        .unwrap_err();
        let missing_replacement = resolve_disposition_relationships(
            &events,
            &resolved,
            &DispositionRelationships {
                replaces_disposition_ids: vec![DispositionId::new("disp:sha256:missing")],
                ..DispositionRelationships::default()
            },
            ReviewDisposition::Accepted,
            Some("summary"),
        )
        .unwrap_err();

        assert!(
            missing_observation
                .to_string()
                .contains("unknown observation")
        );
        assert!(
            missing_intervention
                .to_string()
                .contains("unknown intervention")
        );
        assert!(
            missing_replacement
                .to_string()
                .contains("unknown disposition")
        );
    }

    #[test]
    fn overridden_requires_summary_and_override_reference() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let resolved = resolve_review_unit_for_observation(&events, None).unwrap();

        let missing_summary = resolve_disposition_relationships(
            &events,
            &resolved,
            &DispositionRelationships {
                overrides: vec![DispositionOverrideSelector::observation(
                    ObservationId::new("obs:sha256:missing"),
                )],
                ..DispositionRelationships::default()
            },
            ReviewDisposition::Overridden,
            None,
        )
        .unwrap_err();
        let missing_override = resolve_disposition_relationships(
            &events,
            &resolved,
            &DispositionRelationships::default(),
            ReviewDisposition::Overridden,
            Some("manual override"),
        )
        .unwrap_err();

        assert!(missing_summary.to_string().contains("summary is required"));
        assert!(
            missing_override
                .to_string()
                .contains("override reference is required")
        );
    }

    #[test]
    fn record_disposition_writes_event_and_updates_state() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Accepted)
                .with_summary("Ship this"),
        )
        .unwrap();

        assert_eq!(result.review_unit_id, capture.review_unit_id);
        assert!(result.disposition_id.as_str().starts_with("disp:sha256:"));
        assert_eq!(result.track_id.as_str(), "human:kevin");
        assert_eq!(result.disposition, ReviewDisposition::Accepted);
        assert_eq!(
            result.events_created_by_type["review_disposition_recorded"],
            1
        );

        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let state = crate::session::SessionState::from_events(&events).unwrap();
        assert_eq!(state.disposition_count, 1);
    }

    #[test]
    fn record_disposition_is_idempotent_for_same_logical_input() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let options = DispositionAddOptions::new(repo.path())
            .with_track("human:kevin")
            .with_disposition(ReviewDisposition::Accepted)
            .with_summary("same summary");

        let first = record_disposition(options.clone()).unwrap();
        let second = record_disposition(options).unwrap();

        assert_eq!(first.disposition_id, second.disposition_id);
        assert_eq!(first.events_created, 1);
        assert_eq!(second.events_created, 0);
        assert_eq!(second.events_existing, 1);
    }

    #[test]
    fn explicit_same_idempotency_key_with_different_payload_conflicts() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Accepted)
                .with_summary("first")
                .with_idempotency_key("retry-key"),
        )
        .unwrap();
        let error = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::NeedsChanges)
                .with_summary("second")
                .with_idempotency_key("retry-key"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("event conflict"));
    }

    #[test]
    fn large_summary_is_stored_as_internal_body_artifact() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let summary = "x".repeat(crate::session::body_artifact::BODY_INLINE_LIMIT + 1);

        let result = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::AcceptedWithFollowUp)
                .with_summary(summary),
        )
        .unwrap();

        assert!(
            result
                .summary_content_hash
                .as_deref()
                .unwrap()
                .starts_with("sha256:")
        );
        assert!(
            !format!("{result:?}").contains("artifacts/notes/"),
            "workflow result must not expose internal artifact paths"
        );
    }

    #[test]
    fn replacement_records_new_disposition_with_replaces_link() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::NeedsChanges)
                .with_summary("Fix this"),
        )
        .unwrap();

        let replacement = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Accepted)
                .with_summary("Fixed")
                .replacing(first.disposition_id.clone()),
        )
        .unwrap();
        let payload = disposition_payload(&repo, &replacement.disposition_id);

        assert_eq!(payload.replaces_disposition_ids, vec![first.disposition_id]);
    }

    #[test]
    fn show_disposition_deduplicates_and_sorts_replaces() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::NeedsChanges)
                .with_summary("First"),
        )
        .unwrap();
        let second = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::NeedsClarification)
                .with_summary("Second"),
        )
        .unwrap();
        let replacement = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Accepted)
                .with_summary("Fixed")
                .replacing(second.disposition_id.clone())
                .replacing(first.disposition_id.clone())
                .replacing(first.disposition_id.clone()),
        )
        .unwrap();
        let mut expected = vec![first.disposition_id, second.disposition_id];
        expected.sort();

        let result =
            show_dispositions(DispositionShowOptions::new(repo.path()).with_all(true)).unwrap();
        let view = result
            .dispositions
            .iter()
            .find(|view| view.id == replacement.disposition_id)
            .expect("replacement disposition appears in all view");

        assert_eq!(view.replaces, expected);
    }

    #[test]
    fn override_references_are_metadata_not_replacement() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::NeedsChanges)
                .with_summary("Fix this"),
        )
        .unwrap();

        let override_result = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Overridden)
                .with_summary("Human override")
                .overriding_disposition(first.disposition_id.clone()),
        )
        .unwrap();
        let payload = disposition_payload(&repo, &override_result.disposition_id);

        assert!(payload.replaces_disposition_ids.is_empty());
        assert_eq!(
            payload.overrides,
            vec![ReviewTargetRef::Disposition {
                review_unit_id: override_result.review_unit_id,
                disposition_id: first.disposition_id
            }]
        );
    }

    #[test]
    fn override_order_does_not_change_disposition_id() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::NeedsChanges)
                .with_summary("First"),
        )
        .unwrap();
        let second = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::NeedsClarification)
                .with_summary("Second"),
        )
        .unwrap();

        let forward = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Overridden)
                .with_summary("Manual override")
                .overriding_disposition(first.disposition_id.clone())
                .overriding_disposition(second.disposition_id.clone()),
        )
        .unwrap();
        let reversed = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Overridden)
                .with_summary("Manual override")
                .overriding_disposition(second.disposition_id)
                .overriding_disposition(first.disposition_id),
        )
        .unwrap();

        assert_eq!(forward.disposition_id, reversed.disposition_id);
        assert_eq!(forward.events_created, 1);
        assert_eq!(reversed.events_created, 0);
    }

    #[test]
    fn show_disposition_reports_none_when_no_disposition_exists() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = show_dispositions(DispositionShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.current.status, CurrentDispositionStatus::None);
        assert!(result.current.dispositions.is_empty());
        assert!(result.dispositions.is_empty());
    }

    #[test]
    fn show_disposition_reports_one_unreplaced_current_disposition() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let disposition = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Accepted)
                .with_summary("Ship it"),
        )
        .unwrap();

        let result = show_dispositions(DispositionShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.current.status, CurrentDispositionStatus::Resolved);
        assert_eq!(result.current.dispositions.len(), 1);
        assert_eq!(
            result.current.dispositions[0].id,
            disposition.disposition_id
        );
        assert_eq!(
            result.dispositions[0].status,
            DispositionRecordStatus::Current
        );
    }

    #[test]
    fn show_disposition_reports_ambiguous_for_multiple_unreplaced_dispositions() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Accepted)
                .with_summary("Ship it"),
        )
        .unwrap();
        record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_disposition(ReviewDisposition::NeedsChanges)
                .with_summary("Needs one fix"),
        )
        .unwrap();

        let result = show_dispositions(DispositionShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.current.status, CurrentDispositionStatus::Ambiguous);
        assert_eq!(result.current.dispositions.len(), 2);
    }

    #[test]
    fn show_disposition_excludes_replaced_records_by_default_and_includes_with_all() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::NeedsChanges)
                .with_summary("Fix this"),
        )
        .unwrap();
        let replacement = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Accepted)
                .with_summary("Fixed")
                .replacing(first.disposition_id.clone()),
        )
        .unwrap();

        let current = show_dispositions(DispositionShowOptions::new(repo.path())).unwrap();
        let all =
            show_dispositions(DispositionShowOptions::new(repo.path()).with_all(true)).unwrap();

        assert_eq!(current.current.status, CurrentDispositionStatus::Resolved);
        assert_eq!(
            current
                .dispositions
                .iter()
                .map(|view| view.id.clone())
                .collect::<Vec<_>>(),
            vec![replacement.disposition_id.clone()]
        );
        assert_eq!(all.dispositions.len(), 2);
        assert!(
            all.dispositions
                .iter()
                .any(|view| view.id == first.disposition_id
                    && view.status == DispositionRecordStatus::Replaced)
        );
    }

    #[test]
    fn show_disposition_filters_by_track() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let human = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Accepted)
                .with_summary("Ship it"),
        )
        .unwrap();
        record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_disposition(ReviewDisposition::NeedsChanges)
                .with_summary("Needs one fix"),
        )
        .unwrap();

        let result =
            show_dispositions(DispositionShowOptions::new(repo.path()).with_track("human:kevin"))
                .unwrap();

        assert_eq!(result.current.status, CurrentDispositionStatus::Resolved);
        assert_eq!(result.dispositions.len(), 1);
        assert_eq!(result.dispositions[0].id, human.disposition_id);
        assert_eq!(
            result.filters.track_id.as_ref().unwrap().as_str(),
            "human:kevin"
        );
    }

    #[test]
    fn show_disposition_hydrates_summary_only_when_requested() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let summary = "x".repeat(crate::session::body_artifact::BODY_INLINE_LIMIT + 1);
        record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Accepted)
                .with_summary(summary.clone()),
        )
        .unwrap();

        let without_summary = show_dispositions(DispositionShowOptions::new(repo.path())).unwrap();
        let with_summary =
            show_dispositions(DispositionShowOptions::new(repo.path()).with_include_summary(true))
                .unwrap();

        assert!(without_summary.dispositions[0].summary.is_none());
        assert_eq!(
            with_summary.dispositions[0].summary.as_deref(),
            Some(summary.as_str())
        );
        assert!(!format!("{with_summary:?}").contains("artifacts/notes/"));
    }

    #[test]
    fn show_disposition_collapses_duplicate_semantic_events() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let options = DispositionAddOptions::new(repo.path())
            .with_track("human:kevin")
            .with_disposition(ReviewDisposition::Accepted)
            .with_summary("same summary");
        let first = record_disposition(options.clone().with_idempotency_key("retry-a")).unwrap();
        record_disposition(options.with_idempotency_key("retry-b")).unwrap();

        let result = show_dispositions(DispositionShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.dispositions.len(), 1);
        assert_eq!(result.dispositions[0].id, first.disposition_id);
        assert!(result.diagnostics.iter().any(|diagnostic| diagnostic.code
            == crate::session::state::DUPLICATE_SEMANTIC_DISPOSITION_EVENT_CODE));
    }

    #[test]
    fn show_disposition_sorts_by_created_at_then_event_id() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Accepted)
                .with_summary("First"),
        )
        .unwrap();
        let second = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_disposition(ReviewDisposition::NeedsChanges)
                .with_summary("Second"),
        )
        .unwrap();

        let result = show_dispositions(DispositionShowOptions::new(repo.path())).unwrap();

        assert_eq!(
            result
                .dispositions
                .iter()
                .map(|view| view.id.clone())
                .collect::<Vec<_>>(),
            vec![first.disposition_id, second.disposition_id]
        );
    }

    #[test]
    fn show_disposition_uses_replaces_not_overrides_for_current_projection() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::NeedsChanges)
                .with_summary("Fix this"),
        )
        .unwrap();
        record_disposition(
            DispositionAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_disposition(ReviewDisposition::Overridden)
                .with_summary("Manual override")
                .overriding_disposition(first.disposition_id),
        )
        .unwrap();

        let result = show_dispositions(DispositionShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.current.status, CurrentDispositionStatus::Ambiguous);
        assert_eq!(result.current.dispositions.len(), 2);
    }

    fn disposition_event(
        review_unit_id: &ReviewUnitId,
        disposition_id: &DispositionId,
    ) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewDispositionRecorded,
            ReviewDispositionRecordedPayload::idempotency_key(
                review_unit_id,
                &TrackId::new("human:kevin"),
                disposition_id.as_str(),
            ),
            EventTarget {
                review_id: ReviewId::new("review:default"),
                work_unit_id: None,
                review_unit_id: Some(review_unit_id.clone()),
                revision_id: Some(RevisionId::new("rev:git:sha256:one")),
                snapshot_id: Some(SnapshotId::new("snap:git:sha256:one")),
                track_id: Some(TrackId::new("human:kevin")),
                subject: Some(ReviewTargetRef::ReviewUnit {
                    review_unit_id: review_unit_id.clone(),
                }),
            },
            Writer::shore_local_reviewer("test"),
            ReviewDispositionRecordedPayload {
                disposition_id: disposition_id.clone(),
                target: ReviewTargetRef::ReviewUnit {
                    review_unit_id: review_unit_id.clone(),
                },
                disposition: ReviewDisposition::Accepted,
                summary: Some("Accepted".to_owned()),
                summary_artifact_path: None,
                summary_byte_size: Some(8),
                summary_content_hash: Some("sha256:accepted".to_owned()),
                replaces_disposition_ids: vec![],
                related_observation_ids: vec![],
                related_intervention_ids: vec![],
                overrides: vec![],
            },
            "2026-05-12T00:00:00Z",
        )
        .unwrap()
    }

    fn disposition_payload(
        repo: &TestRepo,
        disposition_id: &DispositionId,
    ) -> ReviewDispositionRecordedPayload {
        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        events
            .into_iter()
            .filter(|event| event.event_type == EventType::ReviewDispositionRecorded)
            .map(|event| serde_json::from_value(event.payload).unwrap())
            .find(|payload: &ReviewDispositionRecordedPayload| {
                &payload.disposition_id == disposition_id
            })
            .unwrap()
    }

    fn modified_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 {\n    1\n}\n");
        repo.git(&["add", "src/lib.rs"]);
        repo.git(&["commit", "-m", "base"]);
        repo.write("src/lib.rs", "pub fn value() -> u32 {\n    2\n}\n");
        repo
    }

    struct TestRepo {
        root: tempfile::TempDir,
    }

    impl TestRepo {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("create temp git repository directory");
            let repo = Self { root };

            repo.git(&["init"]);
            repo.git(&["config", "user.name", "Shore Tests"]);
            repo.git(&["config", "user.email", "shore-tests@example.com"]);
            repo.git(&["config", "commit.gpgsign", "false"]);

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

        fn git(&self, args: &[&str]) {
            let output = Command::new("git")
                .args(args)
                .current_dir(self.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
                args,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
