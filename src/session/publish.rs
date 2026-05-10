use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::canonical_hash::sha256_bytes_hex;
use crate::error::{Result, ShoreError};
use crate::git::{capture_worktree_diff_files, git_worktree_root};
use crate::model::{ActorId, DiffFile, ReviewId, RevisionId, SnapshotId, WorkUnitId};
use crate::session::{
    EventTarget, EventType, ProjectionDiagnostic, ReviewInitializedPayload,
    RevisionPublishedPayload, SessionState, ShoreEvent, SidecarObservedPayload, SidecarSource,
    SnapshotObservedPayload, Writer, WriterRole, WriterTool, ensure_shore_ignored,
    worktree_fingerprint_for_files,
};
use crate::sidecar::{
    DiagnosticLevel, ParsedReviewNotes, ReviewNotesDiagnosticCode, parse_hunk_agent_context,
    parse_review_notes_sidecar, read_legacy_hunk_agent_context_file,
    read_review_notes_sidecar_file,
};
use crate::storage::{Durability, EventStore, EventWriteOutcome, LocalStorage, TempSweepAge};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishOptions {
    repo: PathBuf,
    review_notes: Option<PathBuf>,
    legacy_hunk_agent_context: Option<PathBuf>,
}

impl PublishOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_notes: None,
            legacy_hunk_agent_context: None,
        }
    }

    /// Preflight-validates a native review-notes sidecar during publish.
    ///
    /// Task 4.1 only checks that the file can be read. Recording the corresponding
    /// `sidecar_observed` event is task 4.2's responsibility.
    pub fn with_review_notes(mut self, path: impl AsRef<Path>) -> Self {
        self.review_notes = Some(path.as_ref().to_path_buf());
        self
    }

    /// Preflight-validates a legacy Hunk `agent-context.json` sidecar during publish.
    ///
    /// Task 4.1 only checks that the file can be read. Recording the corresponding
    /// `sidecar_observed` event is task 4.2's responsibility.
    pub fn with_legacy_hunk_agent_context(mut self, path: impl AsRef<Path>) -> Self {
        self.legacy_hunk_agent_context = Some(path.as_ref().to_path_buf());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishResult {
    pub review_id: ReviewId,
    pub work_unit_id: WorkUnitId,
    pub revision_id: RevisionId,
    pub snapshot_id: SnapshotId,
    pub events_created: usize,
    pub events_existing: usize,
    pub events_created_by_type: BTreeMap<String, usize>,
    pub state_path: PathBuf,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

/// Publishes the current worktree review state into the repo-local `.shore/` directory.
///
/// This V1 path is intended for a single publisher at a time. Concurrent calls against the same
/// `.shore/` directory are not coordinated yet; lock or lease semantics are deliberately deferred.
pub fn publish_worktree_review(options: PublishOptions) -> Result<PublishResult> {
    let span =
        tracing::info_span!("session.publish_worktree_review", repo = %options.repo.display());
    let _entered = span.enter();

    let sidecar_observations = preflight_sidecar_inputs(&options)?;

    let worktree_root = git_worktree_root(&options.repo)?;
    let shore_dir = worktree_root.join(".shore");
    let storage = LocalStorage::new(&shore_dir);
    storage.sweep_temp_files(&shore_dir, TempSweepAge::zero())?;
    ensure_store_dirs(&shore_dir)?;
    ensure_shore_ignored(&worktree_root)?;

    let files = filter_explicit_sidecar_files(
        capture_worktree_diff_files(&worktree_root)?,
        &worktree_root,
        &sidecar_observations,
    );
    let fingerprint = worktree_fingerprint_for_files(&worktree_root, &files)?;
    let event_store = EventStore::open(&shore_dir);
    let existing_state = SessionState::from_events(&event_store.list_events()?)?;

    let review_id = ReviewId::new("review:default");
    let work_unit_id = WorkUnitId::new("work:default");
    let target = EventTarget::new(review_id.clone(), work_unit_id.clone());
    let writer = writer_from_git_config(&worktree_root);
    let occurred_at = current_timestamp();
    let supersedes_revision_ids = supersedes_revision_ids(
        existing_state.current_revision_id.as_ref(),
        &fingerprint.revision_id,
    );

    write_revision_artifact(
        &storage,
        &shore_dir,
        &work_unit_id,
        &fingerprint.revision_id,
        &supersedes_revision_ids,
    )?;
    write_snapshot_artifact(
        &storage,
        &shore_dir,
        &fingerprint.snapshot_id,
        &fingerprint.revision_id,
        files.len(),
        files.iter().map(|file| file.hunks.len()).sum(),
    )?;

    let mut recorder = PublishRecorder::default();
    recorder.record(
        &event_store,
        ShoreEvent::new(
            EventType::ReviewInitialized,
            format!(
                "review_initialized:{}:{}",
                review_id.as_str(),
                work_unit_id.as_str()
            ),
            target.clone(),
            writer.clone(),
            ReviewInitializedPayload {},
            occurred_at.clone(),
        )?,
    )?;
    recorder.record(
        &event_store,
        ShoreEvent::new(
            EventType::RevisionPublished,
            format!(
                "revision_published:explicit:{}:{}",
                work_unit_id.as_str(),
                fingerprint.revision_id.as_str()
            ),
            target.clone(),
            writer.clone(),
            RevisionPublishedPayload {
                revision_id: fingerprint.revision_id.clone(),
                supersedes_revision_ids,
            },
            occurred_at.clone(),
        )?,
    )?;
    recorder.record(
        &event_store,
        ShoreEvent::new(
            EventType::SnapshotObserved,
            format!(
                "snapshot_observed:{}:{}",
                work_unit_id.as_str(),
                fingerprint.snapshot_id.as_str()
            ),
            target.clone(),
            writer.clone(),
            SnapshotObservedPayload {
                snapshot_id: fingerprint.snapshot_id.clone(),
                revision_id: fingerprint.revision_id.clone(),
            },
            occurred_at.clone(),
        )?,
    )?;
    for observation in sidecar_observations {
        recorder.record(
            &event_store,
            ShoreEvent::new(
                EventType::SidecarObserved,
                format!(
                    "sidecar_observed:{}:{}:{}",
                    event_type_sidecar_source_key(observation.source),
                    observation.path,
                    observation.content_hash
                ),
                target.clone(),
                writer.clone(),
                observation.into_payload(),
                occurred_at.clone(),
            )?,
        )?;
    }

    let state = SessionState::from_events(&event_store.list_events()?)?;
    let state_path = shore_dir.join("state.json");
    storage.write_json_atomic(&state_path, &state, Durability::Projection)?;

    Ok(PublishResult {
        review_id,
        work_unit_id,
        revision_id: fingerprint.revision_id,
        snapshot_id: fingerprint.snapshot_id,
        events_created: recorder.events_created,
        events_existing: recorder.events_existing,
        events_created_by_type: recorder.events_created_by_type,
        state_path,
        diagnostics: state.diagnostics,
    })
}

pub fn rebuild_state(repo: impl AsRef<Path>) -> Result<SessionState> {
    let worktree_root = git_worktree_root(repo.as_ref())?;
    let shore_dir = worktree_root.join(".shore");
    let storage = LocalStorage::new(&shore_dir);
    storage.sweep_temp_files(&shore_dir, TempSweepAge::zero())?;

    let span = tracing::info_span!("session.rebuild_state", repo = %worktree_root.display());
    let _entered = span.enter();

    let event_store = EventStore::open(&shore_dir);
    let state = SessionState::from_events(&event_store.list_events()?)?;
    storage.write_json_atomic(
        &shore_dir.join("state.json"),
        &state,
        Durability::Projection,
    )?;
    Ok(state)
}

pub fn read_events(repo: impl AsRef<Path>) -> Result<Vec<ShoreEvent>> {
    let worktree_root = git_worktree_root(repo.as_ref())?;
    EventStore::open(worktree_root.join(".shore")).list_events()
}

#[derive(Default)]
struct PublishRecorder {
    events_created: usize,
    events_existing: usize,
    events_created_by_type: BTreeMap<String, usize>,
}

impl PublishRecorder {
    fn record(&mut self, event_store: &EventStore, event: ShoreEvent) -> Result<()> {
        match event_store.record_event_once(&event)? {
            EventWriteOutcome::Created => {
                self.events_created += 1;
                *self
                    .events_created_by_type
                    .entry(event_type_key(event.event_type).to_owned())
                    .or_default() += 1;
            }
            EventWriteOutcome::Existing => {
                self.events_existing += 1;
            }
        }
        Ok(())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RevisionArtifact<'a> {
    schema: &'static str,
    version: u32,
    work_unit_id: &'a WorkUnitId,
    revision_id: &'a RevisionId,
    supersedes_revision_ids: &'a [RevisionId],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotArtifact<'a> {
    schema: &'static str,
    version: u32,
    snapshot_id: &'a SnapshotId,
    revision_id: &'a RevisionId,
    file_count: usize,
    hunk_count: usize,
}

struct SidecarObservation {
    source: SidecarSource,
    path: String,
    byte_size: usize,
    content_hash: String,
    schema: Option<String>,
    imported_schema: Option<String>,
    version: Option<u32>,
    diagnostic_count: usize,
    diagnostic_levels: BTreeMap<String, usize>,
}

impl SidecarObservation {
    fn into_payload(self) -> SidecarObservedPayload {
        SidecarObservedPayload {
            source: self.source,
            path: self.path,
            byte_size: self.byte_size,
            content_hash: self.content_hash,
            schema: self.schema,
            imported_schema: self.imported_schema,
            version: self.version,
            diagnostic_count: self.diagnostic_count,
            diagnostic_levels: self.diagnostic_levels,
        }
    }
}

fn preflight_sidecar_inputs(options: &PublishOptions) -> Result<Vec<SidecarObservation>> {
    if options.review_notes.is_some() && options.legacy_hunk_agent_context.is_some() {
        return Err(ShoreError::Message(
            "only one review-notes input can be supplied".to_owned(),
        ));
    }

    if let Some(path) = &options.review_notes {
        return Ok(vec![observe_native_review_notes(path)?]);
    }

    if let Some(path) = &options.legacy_hunk_agent_context {
        return Ok(vec![observe_legacy_hunk_agent_context(path)?]);
    }

    Ok(Vec::new())
}

fn observe_native_review_notes(path: &Path) -> Result<SidecarObservation> {
    let input = read_review_notes_sidecar_file(path)?;
    let parsed = parse_review_notes_sidecar(&input.text)?;
    Ok(sidecar_observation(
        SidecarSource::ReviewNotes,
        path,
        &input.bytes,
        parsed,
        None,
    ))
}

fn observe_legacy_hunk_agent_context(path: &Path) -> Result<SidecarObservation> {
    let input = read_legacy_hunk_agent_context_file(path)?;
    let parsed = parse_hunk_agent_context(&input.text)?;
    Ok(sidecar_observation(
        SidecarSource::LegacyHunkAgentContext,
        path,
        &input.bytes,
        parsed,
        Some("shore.agent-context".to_owned()),
    ))
}

fn sidecar_observation(
    source: SidecarSource,
    path: &Path,
    bytes: &[u8],
    parsed: ParsedReviewNotes,
    imported_schema: Option<String>,
) -> SidecarObservation {
    let version = if parsed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == ReviewNotesDiagnosticCode::MissingVersion)
    {
        None
    } else {
        Some(parsed.sidecar.version)
    };

    SidecarObservation {
        source,
        path: path.to_string_lossy().into_owned(),
        byte_size: bytes.len(),
        content_hash: format!("sha256:{}", sha256_bytes_hex(bytes)),
        schema: parsed.sidecar.schema,
        imported_schema,
        version,
        diagnostic_count: parsed.diagnostics.len(),
        diagnostic_levels: diagnostic_levels(&parsed.diagnostics),
    }
}

fn diagnostic_levels(
    diagnostics: &[crate::sidecar::ReviewNotesDiagnostic],
) -> BTreeMap<String, usize> {
    let mut levels = BTreeMap::new();
    levels.insert(
        diagnostic_level_key(&DiagnosticLevel::Warning).to_owned(),
        0,
    );
    for diagnostic in diagnostics {
        *levels
            .entry(diagnostic_level_key(&diagnostic.level).to_owned())
            .or_default() += 1;
    }
    levels
}

fn diagnostic_level_key(level: &DiagnosticLevel) -> &'static str {
    match level {
        DiagnosticLevel::Warning => "warning",
    }
}

fn filter_explicit_sidecar_files(
    files: Vec<DiffFile>,
    worktree_root: &Path,
    observations: &[SidecarObservation],
) -> Vec<DiffFile> {
    let excluded = observations
        .iter()
        .filter_map(|observation| worktree_relative_path(worktree_root, &observation.path))
        .collect::<BTreeSet<_>>();
    if excluded.is_empty() {
        return files;
    }

    files
        .into_iter()
        .filter(|file| {
            !file
                .new_path
                .as_ref()
                .is_some_and(|path| excluded.contains(path))
                && !file
                    .old_path
                    .as_ref()
                    .is_some_and(|path| excluded.contains(path))
        })
        .collect()
}

fn worktree_relative_path(worktree_root: &Path, path: &str) -> Option<String> {
    let path = Path::new(path);
    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        worktree_root.join(path)
    };
    let canonical_path = absolute_path.canonicalize().ok()?;
    let canonical_root = worktree_root
        .canonicalize()
        .unwrap_or_else(|_| worktree_root.to_path_buf());
    canonical_path
        .strip_prefix(canonical_root)
        .ok()
        .map(|path| {
            path.to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/")
        })
}

fn ensure_store_dirs(shore_dir: &Path) -> Result<()> {
    for dir in [
        shore_dir.join("events"),
        shore_dir.join("artifacts/revisions"),
        shore_dir.join("artifacts/snapshots"),
    ] {
        fs::create_dir_all(&dir)
            .map_err(|error| ShoreError::Message(format!("create {}: {error}", dir.display())))?;
    }
    Ok(())
}

fn write_revision_artifact(
    storage: &LocalStorage,
    shore_dir: &Path,
    work_unit_id: &WorkUnitId,
    revision_id: &RevisionId,
    supersedes_revision_ids: &[RevisionId],
) -> Result<()> {
    let artifact = RevisionArtifact {
        schema: "shore.revision-artifact",
        version: 1,
        work_unit_id,
        revision_id,
        supersedes_revision_ids,
    };
    storage.write_json_atomic(
        &shore_dir
            .join("artifacts/revisions")
            .join(format!("{}.json", artifact_file_stem(revision_id.as_str()))),
        &artifact,
        Durability::Durable,
    )
}

fn write_snapshot_artifact(
    storage: &LocalStorage,
    shore_dir: &Path,
    snapshot_id: &SnapshotId,
    revision_id: &RevisionId,
    file_count: usize,
    hunk_count: usize,
) -> Result<()> {
    let artifact = SnapshotArtifact {
        schema: "shore.snapshot-artifact",
        version: 1,
        snapshot_id,
        revision_id,
        file_count,
        hunk_count,
    };
    storage.write_json_atomic(
        &shore_dir
            .join("artifacts/snapshots")
            .join(format!("{}.json", artifact_file_stem(snapshot_id.as_str()))),
        &artifact,
        Durability::Durable,
    )
}

fn supersedes_revision_ids(
    current_revision_id: Option<&RevisionId>,
    revision_id: &RevisionId,
) -> Vec<RevisionId> {
    match current_revision_id {
        Some(current) if current != revision_id => vec![current.clone()],
        _ => Vec::new(),
    }
}

fn writer_from_git_config(repo: &Path) -> Writer {
    let actor_id = git_config_value(repo, "user.email")
        .map(|email| ActorId::new(format!("actor:git-email:{email}")))
        .or_else(|| {
            git_config_value(repo, "user.name")
                .map(|name| ActorId::new(format!("actor:git-name:{name}")))
        })
        .unwrap_or_else(|| ActorId::new("actor:local"));

    Writer {
        actor_id,
        role: WriterRole::Author,
        tool: WriterTool {
            name: "shore".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
    }
}

fn git_config_value(repo: &Path, key: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["config", "--get", key])
        .current_dir(repo)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn current_timestamp() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("unix-ms:{millis}")
}

fn artifact_file_stem(id: &str) -> String {
    sha256_bytes_hex(id.as_bytes())
}

fn event_type_key(event_type: EventType) -> &'static str {
    match event_type {
        EventType::ReviewInitialized => "review_initialized",
        EventType::RevisionPublished => "revision_published",
        EventType::SnapshotObserved => "snapshot_observed",
        EventType::SidecarObserved => "sidecar_observed",
    }
}

fn event_type_sidecar_source_key(source: SidecarSource) -> &'static str {
    match source {
        SidecarSource::ReviewNotes => "review_notes",
        SidecarSource::LegacyHunkAgentContext => "legacy_hunk_agent_context",
    }
}
