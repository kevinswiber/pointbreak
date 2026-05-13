use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::json;

use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::error::{Result, ShoreError};
use crate::model::{
    EventId, ObservationId, ReviewId, ReviewTargetRef, ReviewUnitId, RevisionId, Side, SnapshotId,
    TrackId,
};
use crate::session::body_artifact::{BodyArtifactOutcome, load_body_artifact, stage_body_artifact};
use crate::session::event::{
    EventTarget, EventType, ReviewObservationRecordedPayload, ReviewUnitCapturedPayload,
    ShoreEvent, Writer,
};
use crate::session::event_context::{current_timestamp, reviewer_from_git_config};
use crate::session::snapshot_artifact::read_snapshot_artifact;
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store_init::{ShoreStorePaths, prepare_shore_writer};
use crate::storage::{Durability, EventStore, EventWriteOutcome, LocalStorage};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedReviewUnit {
    pub review_id: ReviewId,
    pub review_unit_id: ReviewUnitId,
    pub revision_id: RevisionId,
    pub snapshot_id: SnapshotId,
}

struct ObservationEventRecord<'a> {
    event: &'a ShoreEvent,
    payload: ReviewObservationRecordedPayload,
    track_id: TrackId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationAddOptions {
    repo: PathBuf,
    review_unit_id: Option<ReviewUnitId>,
    track: Option<String>,
    title: Option<String>,
    body: Option<String>,
    target: ObservationTargetSelector,
    tags: Vec<String>,
    confidence: Option<String>,
    supersedes_observation_ids: Vec<ObservationId>,
    idempotency_key: Option<String>,
}

impl ObservationAddOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_unit_id: None,
            track: None,
            title: None,
            body: None,
            target: ObservationTargetSelector::review_unit(),
            tags: Vec::new(),
            confidence: None,
            supersedes_observation_ids: Vec::new(),
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

    pub fn with_target(mut self, target: ObservationTargetSelector) -> Self {
        self.target = target;
        self
    }

    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    pub fn with_confidence(mut self, confidence: impl Into<String>) -> Self {
        self.confidence = Some(confidence.into());
        self
    }

    pub fn superseding(mut self, observation_id: ObservationId) -> Self {
        self.supersedes_observation_ids.push(observation_id);
        self
    }

    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationAddResult {
    pub review_unit_id: ReviewUnitId,
    pub observation_id: ObservationId,
    pub event_id: EventId,
    pub track_id: TrackId,
    pub target: ReviewTargetRef,
    pub body_content_hash: Option<String>,
    pub events_created: usize,
    pub events_existing: usize,
    pub events_created_by_type: BTreeMap<String, usize>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationListOptions {
    repo: PathBuf,
    review_unit_id: Option<ReviewUnitId>,
    track: Option<String>,
    file: Option<String>,
    include_body: bool,
}

impl ObservationListOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_unit_id: None,
            track: None,
            file: None,
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

    pub fn with_file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    pub fn with_include_body(mut self, include_body: bool) -> Self {
        self.include_body = include_body;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationListFilters {
    pub track_id: Option<TrackId>,
    pub file: Option<String>,
    pub include_body: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationListResult {
    pub review_unit_id: ReviewUnitId,
    pub filters: ObservationListFilters,
    pub observations: Vec<ObservationView>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationView {
    pub id: ObservationId,
    pub event_id: EventId,
    pub track_id: TrackId,
    pub target: ReviewTargetRef,
    pub title: String,
    pub body: Option<String>,
    pub tags: Vec<String>,
    pub confidence: Option<String>,
    pub status: ObservationStatus,
    pub supersedes: Vec<ObservationId>,
    pub body_content_hash: Option<String>,
    pub created_at: String,
    pub writer: Writer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationStatus {
    Active,
    Superseded,
}

pub fn record_observation(options: ObservationAddOptions) -> Result<ObservationAddResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let worktree_root = paths.worktree_root();
    let shore_dir = paths.shore_dir();
    let storage = LocalStorage::new(shore_dir);
    prepare_shore_writer(&paths, &storage)?;

    let event_store = EventStore::open(shore_dir);
    let events = event_store.list_events()?;
    let resolved = resolve_review_unit_for_observation(&events, options.review_unit_id.as_ref())?;
    let target = resolve_observation_target(worktree_root, &resolved, &options.target)?;
    let track_id = validated_track_id(
        options
            .track
            .as_deref()
            .ok_or_else(|| ShoreError::Message("track is required".to_owned()))?,
    )?;
    let title = required_title(options.title.as_deref())?;
    let writer = reviewer_from_git_config(worktree_root);
    let body_content_hash = options
        .body
        .as_ref()
        .map(|body| format!("sha256:{}", sha256_bytes_hex(body.as_bytes())));
    let (body, body_artifact_path, body_artifact_bytes, body_byte_size) =
        staged_body(options.body.as_deref())?;
    let observation_id = build_observation_id(ObservationIdMaterial {
        review_unit_id: &resolved.review_unit_id,
        track_id: &track_id,
        target: &target,
        title: &title,
        body_content_hash: body_content_hash.as_deref(),
        tags: &options.tags,
        confidence: options.confidence.as_deref(),
        supersedes_observation_ids: &options.supersedes_observation_ids,
        writer_actor_id: writer.actor_id.as_str(),
    })?;
    let source_key = options
        .idempotency_key
        .as_deref()
        .unwrap_or_else(|| observation_id.as_str());
    let idempotency_key = ReviewObservationRecordedPayload::idempotency_key(
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
        EventType::ReviewObservationRecorded,
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
        ReviewObservationRecordedPayload {
            observation_id: observation_id.clone(),
            target: target.clone(),
            title,
            body,
            body_artifact_path,
            body_byte_size,
            body_content_hash: body_content_hash.clone(),
            tags: options.tags,
            confidence: options.confidence,
            supersedes_observation_ids: options.supersedes_observation_ids,
        },
        current_timestamp(),
    )?;
    let event_id = event.event_id.clone();

    let mut events_created_by_type = BTreeMap::new();
    let (events_created, events_existing) = match event_store.record_event_once(&event)? {
        EventWriteOutcome::Created => {
            events_created_by_type.insert("review_observation_recorded".to_owned(), 1);
            (1, 0)
        }
        EventWriteOutcome::Existing => (0, 1),
    };

    let state = SessionState::from_events(&event_store.list_events()?)?;
    storage.write_json_atomic(&paths.state_path(), &state, Durability::Projection)?;

    Ok(ObservationAddResult {
        review_unit_id: resolved.review_unit_id,
        observation_id,
        event_id,
        track_id,
        target,
        body_content_hash,
        events_created,
        events_existing,
        events_created_by_type,
        diagnostics: state.diagnostics,
    })
}

pub fn list_observations(options: ObservationListOptions) -> Result<ObservationListResult> {
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
    let mut observation_records: BTreeMap<ObservationId, ObservationEventRecord<'_>> =
        BTreeMap::new();
    let mut superseded_ids = BTreeSet::new();

    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewObservationRecorded)
    {
        if event.target.review_unit_id.as_ref() != Some(&resolved.review_unit_id) {
            continue;
        }

        let payload: ReviewObservationRecordedPayload =
            serde_json::from_value(event.payload.clone())?;
        superseded_ids.extend(payload.supersedes_observation_ids.iter().cloned());

        let track_id =
            event.target.track_id.clone().ok_or_else(|| {
                ShoreError::Message("observation event missing track id".to_owned())
            })?;
        if track_filter
            .as_ref()
            .is_some_and(|filter| filter != &track_id)
        {
            continue;
        }
        if options
            .file
            .as_deref()
            .is_some_and(|file| !target_matches_file(&payload.target, file))
        {
            continue;
        }

        let observation_id = payload.observation_id.clone();
        let replace_record = observation_records
            .get(&observation_id)
            .is_none_or(|record| {
                // Event IDs are deterministic storage addresses, not causal order. Pick the
                // lowest one only as a stable representative for duplicate semantic facts.
                event.event_id.as_str() < record.event.event_id.as_str()
            });
        if replace_record {
            observation_records.insert(
                observation_id,
                ObservationEventRecord {
                    event,
                    payload,
                    track_id,
                },
            );
        }
    }

    let mut observations = Vec::new();
    for (_, record) in observation_records {
        let body = if options.include_body {
            observation_body(shore_dir, &record.payload)?
        } else {
            None
        };

        observations.push(ObservationView {
            id: record.payload.observation_id,
            event_id: record.event.event_id.clone(),
            track_id: record.track_id,
            target: record.payload.target,
            title: record.payload.title,
            body,
            tags: record.payload.tags,
            confidence: record.payload.confidence,
            status: ObservationStatus::Active,
            supersedes: record.payload.supersedes_observation_ids,
            body_content_hash: record.payload.body_content_hash,
            created_at: record.event.occurred_at.clone(),
            writer: record.event.writer.clone(),
        });
    }

    for observation in &mut observations {
        if superseded_ids.contains(&observation.id) {
            observation.status = ObservationStatus::Superseded;
        }
    }
    sort_observation_views(&mut observations);
    let diagnostics = SessionState::from_events(&events)?.diagnostics;

    Ok(ObservationListResult {
        review_unit_id: resolved.review_unit_id,
        filters: ObservationListFilters {
            track_id: track_filter,
            file: options.file,
            include_body: options.include_body,
        },
        observations,
        diagnostics,
    })
}

pub(crate) fn target_matches_file(target: &ReviewTargetRef, file: &str) -> bool {
    match target {
        ReviewTargetRef::File { file_path, .. } | ReviewTargetRef::Range { file_path, .. } => {
            file_path == file
        }
        ReviewTargetRef::ReviewUnit { .. }
        | ReviewTargetRef::Observation { .. }
        | ReviewTargetRef::Intervention { .. }
        | ReviewTargetRef::Disposition { .. }
        | ReviewTargetRef::Event { .. } => false,
    }
}

fn observation_body(
    shore_dir: &Path,
    payload: &ReviewObservationRecordedPayload,
) -> Result<Option<String>> {
    if payload.body.is_some() {
        return Ok(payload.body.clone());
    }
    match payload.body_artifact_path.as_deref() {
        Some(path) => load_body_artifact(shore_dir, path),
        None => Ok(None),
    }
}

fn sort_observation_views(observations: &mut [ObservationView]) {
    observations.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.event_id.as_str().cmp(right.event_id.as_str()))
    });
}

pub(crate) fn required_title(title: Option<&str>) -> Result<String> {
    let title = title.unwrap_or_default().trim();
    if title.is_empty() {
        return Err(ShoreError::Message("title is required".to_owned()));
    }
    Ok(title.to_owned())
}

pub(crate) type StagedBody = (Option<String>, Option<String>, Option<Vec<u8>>, Option<u64>);

pub(crate) fn staged_body(body: Option<&str>) -> Result<StagedBody> {
    match body {
        Some(body) => match stage_body_artifact(body.as_bytes())? {
            BodyArtifactOutcome::Inline { byte_size } => {
                Ok((Some(body.to_owned()), None, None, Some(byte_size)))
            }
            BodyArtifactOutcome::Artifact {
                relative_path,
                byte_size,
                body_envelope,
            } => Ok((
                None,
                Some(relative_path),
                Some(body_envelope.to_json_bytes()?),
                Some(byte_size),
            )),
        },
        None => Ok((None, None, None, None)),
    }
}

struct ObservationIdMaterial<'a> {
    review_unit_id: &'a ReviewUnitId,
    track_id: &'a TrackId,
    target: &'a ReviewTargetRef,
    title: &'a str,
    body_content_hash: Option<&'a str>,
    tags: &'a [String],
    confidence: Option<&'a str>,
    supersedes_observation_ids: &'a [ObservationId],
    writer_actor_id: &'a str,
}

fn build_observation_id(material: ObservationIdMaterial<'_>) -> Result<ObservationId> {
    let mut tags = material.tags.to_vec();
    tags.sort();
    let mut supersedes = material
        .supersedes_observation_ids
        .iter()
        .map(|observation_id| observation_id.as_str())
        .collect::<Vec<_>>();
    supersedes.sort();
    let digest = sha256_json_prefixed(&json!({
        "reviewUnitId": material.review_unit_id.as_str(),
        "trackId": material.track_id.as_str(),
        "target": material.target,
        "title": material.title,
        "bodyContentHash": material.body_content_hash,
        "tags": tags,
        "confidence": material.confidence,
        "supersedesObservationIds": supersedes,
        "writerActorId": material.writer_actor_id,
    }))?;
    Ok(ObservationId::new(format!("obs:{digest}")))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationTargetSelector {
    pub file_path: Option<String>,
    pub side: Side,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
}

impl ObservationTargetSelector {
    pub fn review_unit() -> Self {
        Self {
            file_path: None,
            side: Side::New,
            start_line: None,
            end_line: None,
        }
    }

    pub fn file(path: impl Into<String>) -> Self {
        Self {
            file_path: Some(path.into()),
            side: Side::New,
            start_line: None,
            end_line: None,
        }
    }

    pub fn range(
        path: impl Into<String>,
        side: Side,
        start_line: u32,
        end_line: Option<u32>,
    ) -> Self {
        Self {
            file_path: Some(path.into()),
            side,
            start_line: Some(start_line),
            end_line,
        }
    }
}

pub(crate) fn resolve_review_unit_for_observation(
    events: &[ShoreEvent],
    requested: Option<&ReviewUnitId>,
) -> Result<ResolvedReviewUnit> {
    let mut captured = Vec::new();
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewUnitCaptured)
    {
        let payload: ReviewUnitCapturedPayload = serde_json::from_value(event.payload.clone())?;
        let resolved = ResolvedReviewUnit {
            review_id: event.target.review_id.clone(),
            review_unit_id: payload.review_unit_id,
            revision_id: payload.revision_id,
            snapshot_id: payload.snapshot_id,
        };
        if requested.is_some_and(|requested| requested == &resolved.review_unit_id) {
            return Ok(resolved);
        }
        captured.push(resolved);
    }

    if let Some(requested) = requested {
        return Err(ShoreError::Message(format!(
            "unknown review unit: {}",
            requested.as_str()
        )));
    }

    match captured.as_slice() {
        [] => Err(ShoreError::Message("no captured review unit".to_owned())),
        [resolved] => Ok(resolved.clone()),
        _ => Err(ShoreError::Message(
            "multiple captured review units; pass --review-unit".to_owned(),
        )),
    }
}

pub(crate) fn resolve_observation_target(
    repo: &Path,
    resolved: &ResolvedReviewUnit,
    selector: &ObservationTargetSelector,
) -> Result<ReviewTargetRef> {
    let Some(file_path) = selector.file_path.as_deref() else {
        if selector.start_line.is_some() || selector.end_line.is_some() {
            return Err(ShoreError::Message(
                "file is required when selecting observation lines".to_owned(),
            ));
        }
        return Ok(ReviewTargetRef::ReviewUnit {
            review_unit_id: resolved.review_unit_id.clone(),
        });
    };

    let artifact = read_snapshot_artifact(repo, &resolved.snapshot_id)?;
    if !artifact.snapshot.files.iter().any(|file| {
        file.new_path.as_deref() == Some(file_path) || file.old_path.as_deref() == Some(file_path)
    }) {
        return Err(ShoreError::Message(format!(
            "file target is not present in captured snapshot: {file_path}"
        )));
    }

    match selector.start_line {
        Some(start_line) => {
            if start_line == 0 {
                return Err(ShoreError::Message(
                    "start line must be greater than zero".to_owned(),
                ));
            }
            let end_line = selector.end_line.unwrap_or(start_line);
            if end_line < start_line {
                return Err(ShoreError::Message(
                    "end line must be greater than or equal to start line".to_owned(),
                ));
            }
            Ok(ReviewTargetRef::Range {
                review_unit_id: resolved.review_unit_id.clone(),
                file_path: file_path.to_owned(),
                side: selector.side,
                start_line,
                end_line,
            })
        }
        None => {
            if selector.end_line.is_some() {
                return Err(ShoreError::Message(
                    "start line is required when end line is supplied".to_owned(),
                ));
            }
            Ok(ReviewTargetRef::File {
                review_unit_id: resolved.review_unit_id.clone(),
                file_path: file_path.to_owned(),
            })
        }
    }
}

pub(crate) fn validated_track_id(value: &str) -> Result<TrackId> {
    let value = value.trim();
    if value.is_empty() {
        return Err(invalid_track_id("track id cannot be empty"));
    }
    if value.len() > 128 {
        return Err(invalid_track_id("track id must be 128 bytes or fewer"));
    }
    if matches!(value, "all" | "none" | "null" | "default" | "*") {
        return Err(invalid_track_id("track id is reserved"));
    }
    if value.starts_with("system:") || value.starts_with("import:") {
        return Err(invalid_track_id("track namespace is reserved"));
    }
    if !value.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b':')
    }) {
        return Err(invalid_track_id(
            "track id may only contain lowercase ASCII letters, digits, '-' and ':'",
        ));
    }

    Ok(TrackId::new(value.to_owned()))
}

fn invalid_track_id(message: &str) -> ShoreError {
    ShoreError::Message(message.to_owned())
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::*;
    use crate::model::{
        EventId, ReviewEndpoint, ReviewId, ReviewTargetRef, ReviewUnitId, ReviewUnitSource,
        RevisionId, Side, SnapshotId, WorktreeCaptureMode,
    };
    use crate::session::{
        CaptureOptions, CaptureResult, EventTarget, EventType, ReviewUnitCapturedPayload,
        SessionState, ShoreEvent, Writer, capture_worktree_review,
    };
    use crate::storage::EventStore;

    #[test]
    fn track_policy_accepts_lowercase_local_and_namespaced_ids() {
        assert_eq!(validated_track_id("codex").unwrap().as_str(), "codex");
        assert_eq!(
            validated_track_id("agent:codex").unwrap().as_str(),
            "agent:codex"
        );
        assert_eq!(
            validated_track_id("human:kevin").unwrap().as_str(),
            "human:kevin"
        );
    }

    #[test]
    fn track_policy_rejects_reserved_or_unsafe_ids() {
        for bad in [
            "",
            "All",
            "all",
            "*",
            "none",
            "null",
            "default",
            "agent/codex",
            "agent codex",
            "system:shore",
            "import:hunk",
        ] {
            assert!(validated_track_id(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn track_policy_rejects_overlong_ids() {
        let too_long = "a".repeat(129);

        assert!(validated_track_id(&too_long).is_err());
    }

    #[test]
    fn resolves_single_current_review_unit_when_not_explicit() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let event_store = EventStore::open(repo.path().join(".shore"));
        let events = event_store.list_events().unwrap();

        let resolved = resolve_review_unit_for_observation(&events, None).unwrap();

        assert_eq!(resolved.review_unit_id, capture.review_unit_id);
        assert_eq!(resolved.revision_id, capture.revision_id);
        assert_eq!(resolved.snapshot_id, capture.snapshot_id);
    }

    #[test]
    fn resolving_current_review_unit_errors_when_none_captured() {
        let events = Vec::new();

        let error = resolve_review_unit_for_observation(&events, None).unwrap_err();

        assert!(error.to_string().contains("no captured review unit"));
    }

    #[test]
    fn resolving_current_review_unit_errors_when_ambiguous() {
        let events = vec![
            review_unit_captured_event_with_ids("review-unit:sha256:one", "rev:one", "snap:one"),
            review_unit_captured_event_with_ids("review-unit:sha256:two", "rev:two", "snap:two"),
        ];

        let error = resolve_review_unit_for_observation(&events, None).unwrap_err();

        assert!(error.to_string().contains("multiple captured review units"));
    }

    #[test]
    fn explicit_unknown_review_unit_is_rejected() {
        let events = vec![review_unit_captured_event_with_ids(
            "review-unit:sha256:known",
            "rev:one",
            "snap:one",
        )];

        let error = resolve_review_unit_for_observation(
            &events,
            Some(&ReviewUnitId::new("review-unit:sha256:missing")),
        )
        .unwrap_err();

        assert!(error.to_string().contains("unknown review unit"));
    }

    #[test]
    fn target_selector_builds_review_wide_file_and_range_refs() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let resolved = resolved_from_capture(&capture);

        let review_wide = resolve_observation_target(
            repo.path(),
            &resolved,
            &ObservationTargetSelector::review_unit(),
        )
        .unwrap();
        let file = resolve_observation_target(
            repo.path(),
            &resolved,
            &ObservationTargetSelector::file("src/lib.rs"),
        )
        .unwrap();
        let range = resolve_observation_target(
            repo.path(),
            &resolved,
            &ObservationTargetSelector::range("src/lib.rs", Side::New, 2, Some(3)),
        )
        .unwrap();

        assert!(matches!(review_wide, ReviewTargetRef::ReviewUnit { .. }));
        assert!(matches!(file, ReviewTargetRef::File { .. }));
        assert!(matches!(
            range,
            ReviewTargetRef::Range {
                start_line: 2,
                end_line: 3,
                ..
            }
        ));
    }

    #[test]
    fn target_selector_rejects_file_not_in_captured_snapshot() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let resolved = resolved_from_capture(&capture);

        let error = resolve_observation_target(
            repo.path(),
            &resolved,
            &ObservationTargetSelector::file("missing.rs"),
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("not present in captured snapshot")
        );
    }

    #[test]
    fn target_selector_rejects_invalid_range_shape() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let resolved = resolved_from_capture(&capture);

        let zero = resolve_observation_target(
            repo.path(),
            &resolved,
            &ObservationTargetSelector::range("src/lib.rs", Side::New, 0, Some(1)),
        )
        .unwrap_err();
        let reversed = resolve_observation_target(
            repo.path(),
            &resolved,
            &ObservationTargetSelector::range("src/lib.rs", Side::New, 3, Some(2)),
        )
        .unwrap_err();

        assert!(zero.to_string().contains("start line"));
        assert!(reversed.to_string().contains("end line"));
    }

    #[test]
    fn record_observation_writes_event_and_updates_state() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Check return value")
                .with_target(ObservationTargetSelector::file("src/lib.rs")),
        )
        .unwrap();

        assert_eq!(result.review_unit_id, capture.review_unit_id);
        assert!(result.observation_id.as_str().starts_with("obs:sha256:"));
        assert_eq!(result.track_id.as_str(), "agent:codex");
        assert_eq!(result.events_created, 1);
        assert_eq!(result.events_existing, 0);
        assert_eq!(
            result.events_created_by_type["review_observation_recorded"],
            1
        );
        assert!(result.body_content_hash.is_none());

        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let state = SessionState::from_events(&events).unwrap();
        assert_eq!(state.observation_count, 1);
    }

    #[test]
    fn record_observation_is_idempotent_for_same_logical_input() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let options = ObservationAddOptions::new(repo.path())
            .with_track("agent:codex")
            .with_title("Same finding")
            .with_body("same body")
            .with_target(ObservationTargetSelector::review_unit());

        let first = record_observation(options.clone()).unwrap();
        let second = record_observation(options).unwrap();

        assert_eq!(first.observation_id, second.observation_id);
        assert_eq!(first.events_created, 1);
        assert_eq!(second.events_created, 0);
        assert_eq!(second.events_existing, 1);
    }

    #[test]
    fn explicit_same_idempotency_key_with_different_payload_conflicts() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("First")
                .with_idempotency_key("retry-key"),
        )
        .unwrap();
        let error = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Second")
                .with_idempotency_key("retry-key"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("event conflict"));
    }

    #[test]
    fn large_observation_body_is_stored_as_internal_body_artifact() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let body = "x".repeat(super::super::body_artifact::BODY_INLINE_LIMIT + 1);

        let result = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Large body")
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
    fn correction_records_new_observation_with_supersedes_link() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let original = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Original"),
        )
        .unwrap();
        let correction = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Correction")
                .superseding(original.observation_id.clone()),
        )
        .unwrap();

        assert_ne!(original.observation_id, correction.observation_id);

        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let correction_event = events
            .iter()
            .find(|event| event.event_id == correction.event_id)
            .unwrap();
        assert_eq!(
            correction_event.payload["supersedesObservationIds"][0],
            original.observation_id.as_str()
        );
    }

    #[test]
    fn list_observations_returns_observations_for_current_review_unit() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("First"),
        )
        .unwrap();
        let second = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:claude")
                .with_title("Second"),
        )
        .unwrap();

        let result = list_observations(ObservationListOptions::new(repo.path())).unwrap();

        assert_eq!(result.review_unit_id, capture.review_unit_id);
        let mut actual_ids = result
            .observations
            .iter()
            .map(|observation| observation.id.as_str().to_owned())
            .collect::<Vec<_>>();
        actual_ids.sort();
        let mut expected_ids = vec![
            first.observation_id.as_str().to_owned(),
            second.observation_id.as_str().to_owned(),
        ];
        expected_ids.sort();
        assert_eq!(actual_ids, expected_ids);
    }

    #[test]
    fn list_observations_collapses_duplicate_semantic_events() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let first = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Same finding")
                .with_body("same body")
                .with_idempotency_key("retry-a"),
        )
        .unwrap();
        let second = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Same finding")
                .with_body("same body")
                .with_idempotency_key("retry-b"),
        )
        .unwrap();

        let result =
            list_observations(ObservationListOptions::new(repo.path()).with_include_body(true))
                .unwrap();

        assert_eq!(first.observation_id, second.observation_id);
        assert_eq!(first.events_created, 1);
        assert_eq!(second.events_created, 1);
        assert_eq!(result.observations.len(), 1);
        assert_eq!(result.observations[0].id, first.observation_id);
        assert_eq!(result.observations[0].body.as_deref(), Some("same body"));
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == crate::session::state::DUPLICATE_SEMANTIC_OBSERVATION_EVENT_CODE
        }));
    }

    #[test]
    fn list_observations_uses_worktree_shore_dir_from_subdirectory() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let added = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("subdir read"),
        )
        .unwrap();

        let result = list_observations(ObservationListOptions::new(repo.path().join("src")))
            .expect("observations load from subdirectory");

        assert_eq!(result.observations[0].id, added.observation_id);
    }

    #[test]
    fn list_observations_filters_by_track_and_file() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("File")
                .with_target(ObservationTargetSelector::file("src/lib.rs")),
        )
        .unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:claude")
                .with_title("Review wide"),
        )
        .unwrap();

        let result = list_observations(
            ObservationListOptions::new(repo.path())
                .with_track("agent:codex")
                .with_file("src/lib.rs"),
        )
        .unwrap();

        assert_eq!(result.observations.len(), 1);
        assert_eq!(result.observations[0].track_id.as_str(), "agent:codex");
    }

    #[test]
    fn list_observations_omits_body_by_default_and_hydrates_when_requested() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Body")
                .with_body("large ".repeat(1000)),
        )
        .unwrap();

        let without_body = list_observations(ObservationListOptions::new(repo.path())).unwrap();
        let with_body =
            list_observations(ObservationListOptions::new(repo.path()).with_include_body(true))
                .unwrap();

        assert!(without_body.observations[0].body.is_none());
        assert!(
            with_body.observations[0]
                .body
                .as_deref()
                .unwrap()
                .starts_with("large ")
        );
        assert!(
            !format!("{with_body:?}").contains("artifacts/notes/"),
            "list result must not expose internal artifact paths"
        );
    }

    #[test]
    fn list_observations_marks_superseded_observations() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let original = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Original"),
        )
        .unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Correction")
                .superseding(original.observation_id.clone()),
        )
        .unwrap();

        let result = list_observations(ObservationListOptions::new(repo.path())).unwrap();
        let original_view = result
            .observations
            .iter()
            .find(|observation| observation.id == original.observation_id)
            .unwrap();

        assert_eq!(original_view.status, ObservationStatus::Superseded);
    }

    #[test]
    fn list_observations_sorts_by_occurred_at_then_event_id() {
        let mut observations = vec![
            observation_view_for_sort("obs:sha256:b", "evt:sha256:b", "unix-ms:2"),
            observation_view_for_sort("obs:sha256:c", "evt:sha256:c", "unix-ms:1"),
            observation_view_for_sort("obs:sha256:a", "evt:sha256:a", "unix-ms:1"),
        ];

        sort_observation_views(&mut observations);

        assert_eq!(
            observations
                .iter()
                .map(|observation| observation.id.as_str())
                .collect::<Vec<_>>(),
            vec!["obs:sha256:a", "obs:sha256:c", "obs:sha256:b"]
        );
    }

    fn resolved_from_capture(capture: &CaptureResult) -> ResolvedReviewUnit {
        ResolvedReviewUnit {
            review_id: capture.review_id.clone(),
            review_unit_id: capture.review_unit_id.clone(),
            revision_id: capture.revision_id.clone(),
            snapshot_id: capture.snapshot_id.clone(),
        }
    }

    fn review_unit_captured_event_with_ids(
        review_unit_id: &str,
        revision_id: &str,
        snapshot_id: &str,
    ) -> ShoreEvent {
        let review_unit_id = ReviewUnitId::new(review_unit_id);
        let revision_id = RevisionId::new(revision_id);
        let snapshot_id = SnapshotId::new(snapshot_id);
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
        .unwrap()
    }

    fn observation_view_for_sort(
        observation_id: &str,
        event_id: &str,
        created_at: &str,
    ) -> ObservationView {
        let review_unit_id = ReviewUnitId::new("review-unit:sha256:one");
        ObservationView {
            id: crate::model::ObservationId::new(observation_id),
            event_id: EventId::new(event_id),
            track_id: TrackId::new("agent:codex"),
            target: ReviewTargetRef::ReviewUnit { review_unit_id },
            title: "sort".to_owned(),
            body: None,
            tags: vec![],
            confidence: None,
            status: ObservationStatus::Active,
            supersedes: vec![],
            body_content_hash: None,
            created_at: created_at.to_owned(),
            writer: Writer::shore_local_reviewer("test"),
        }
    }

    fn modified_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 {\n    1\n}\n");
        repo.commit_all("base");
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
