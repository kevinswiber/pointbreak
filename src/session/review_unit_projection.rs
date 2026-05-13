use std::path::{Path, PathBuf};

use crate::error::{Result, ShoreError};
use crate::model::{
    DiffFile, DiffSnapshot, DispositionId, InterventionId, ObservationId, ReviewEndpoint, ReviewId,
    ReviewTargetRef, ReviewUnitId, ReviewUnitSource, RevisionId, RowId, SnapshotId, TrackId,
};
use crate::session::disposition::{
    CurrentDispositionStatus, CurrentDispositionView, DispositionView,
};
use crate::session::event::{EventType, ReviewUnitCapturedPayload, ShoreEvent};
use crate::session::intervention::InterventionView;
use crate::session::observation::{
    ObservationView, ResolvedReviewUnit, resolve_review_unit_for_observation, validated_track_id,
};
use crate::session::snapshot_artifact::read_snapshot_artifact;
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store_init::ShoreStorePaths;
use crate::storage::EventStore;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewUnitShowOptions {
    repo: PathBuf,
    review_unit_id: Option<ReviewUnitId>,
    track: Option<String>,
    include_body: bool,
}

impl ReviewUnitShowOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_unit_id: None,
            track: None,
            include_body: false,
        }
    }

    pub fn with_review_unit_id(mut self, review_unit_id: ReviewUnitId) -> Self {
        self.review_unit_id = Some(review_unit_id);
        self
    }

    pub fn with_track(mut self, track: impl Into<String>) -> Self {
        self.track = Some(track.into());
        self
    }

    pub fn with_include_body(mut self, include_body: bool) -> Self {
        self.include_body = include_body;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewUnitShowResult {
    pub event_set_hash: String,
    pub event_count: usize,
    pub review_unit: ReviewUnitProjectionIdentity,
    pub snapshot: DiffSnapshot,
    pub filters: ReviewUnitShowFilters,
    pub summary: ReviewUnitProjectionSummary,
    pub current_disposition: CurrentDispositionView,
    pub observations: Vec<ObservationView>,
    pub interventions: Vec<InterventionView>,
    pub dispositions: Vec<DispositionView>,
    pub adapter_notes: Vec<AdapterNoteView>,
    pub rows: Vec<ReviewUnitProjectionRow>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewUnitProjectionIdentity {
    pub id: ReviewUnitId,
    pub review_id: ReviewId,
    pub source: ReviewUnitSource,
    pub base: ReviewEndpoint,
    pub target: ReviewEndpoint,
    pub revision_id: RevisionId,
    pub snapshot_id: SnapshotId,
    pub snapshot_artifact_content_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewUnitShowFilters {
    pub review_unit_id: ReviewUnitId,
    pub track_id: Option<TrackId>,
    pub include_body: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReviewUnitProjectionSummary {
    pub file_count: usize,
    pub row_count: usize,
    pub narrative_row_count: usize,
    pub snapshot_row_count: usize,
    pub snapshot_remainder_row_count: usize,
    pub observation_count: usize,
    pub intervention_count: usize,
    pub disposition_count: usize,
    pub adapter_note_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterNoteView {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewUnitProjectionRow {
    pub id: RowId,
    pub kind: ReviewUnitProjectionRowKind,
    pub projection_phase: ProjectionPhase,
    pub projection_order: usize,
    pub snapshot_order: Option<SnapshotOrder>,
    pub coverage: ProjectionCoverage,
    pub target: Option<ReviewTargetRef>,
    pub file_path: Option<String>,
    pub old_path: Option<String>,
    pub related_observation_ids: Vec<ObservationId>,
    pub related_intervention_ids: Vec<InterventionId>,
    pub related_disposition_ids: Vec<DispositionId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewUnitProjectionRowKind {
    FileHeader,
    Metadata,
    HunkHeader,
    Diff,
    EmptyState,
}

impl ReviewUnitProjectionRowKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FileHeader => "file_header",
            Self::Metadata => "metadata",
            Self::HunkHeader => "hunk_header",
            Self::Diff => "diff",
            Self::EmptyState => "empty_state",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionPhase {
    SnapshotRemainder,
}

impl ProjectionPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SnapshotRemainder => "snapshot_remainder",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionCoverage {
    Context,
    Unreviewed,
}

impl ProjectionCoverage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Context => "context",
            Self::Unreviewed => "unreviewed",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotOrder {
    pub file_index: usize,
    pub metadata_index: Option<usize>,
    pub hunk_index: Option<usize>,
    pub row_index: Option<usize>,
}

pub fn show_review_unit(options: ReviewUnitShowOptions) -> Result<ReviewUnitShowResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let track_id = options
        .track
        .as_deref()
        .map(validated_track_id)
        .transpose()?;
    let events = EventStore::open(paths.shore_dir()).list_events()?;
    let resolved = resolve_review_unit_for_observation(&events, options.review_unit_id.as_ref())?;
    let review_unit = selected_review_unit_capture(&events, &resolved)?;
    let snapshot = load_bound_snapshot_artifact(paths.worktree_root(), &review_unit)?;
    let (rows, summary) = build_snapshot_rows(&snapshot, &review_unit.id);
    let state = SessionState::from_events(&events)?;
    let event_set_hash = state
        .event_set_hash
        .clone()
        .expect("SessionState::from_events sets event_set_hash");

    Ok(ReviewUnitShowResult {
        event_set_hash,
        event_count: events.len(),
        review_unit,
        snapshot,
        filters: ReviewUnitShowFilters {
            review_unit_id: resolved.review_unit_id,
            track_id,
            include_body: options.include_body,
        },
        summary,
        current_disposition: CurrentDispositionView {
            status: CurrentDispositionStatus::None,
            dispositions: Vec::new(),
        },
        observations: Vec::new(),
        interventions: Vec::new(),
        dispositions: Vec::new(),
        adapter_notes: Vec::new(),
        rows,
        diagnostics: state.diagnostics,
    })
}

fn load_bound_snapshot_artifact(
    repo: &Path,
    review_unit: &ReviewUnitProjectionIdentity,
) -> Result<DiffSnapshot> {
    let artifact = read_snapshot_artifact(repo, &review_unit.snapshot_id)?;
    if artifact.review_unit_id != review_unit.id
        || artifact.source != review_unit.source
        || artifact.base != review_unit.base
        || artifact.target != review_unit.target
        || artifact.snapshot.snapshot_id != review_unit.snapshot_id
    {
        return Err(ShoreError::Message(format!(
            "snapshot artifact metadata mismatch for {}",
            review_unit.id.as_str()
        )));
    }
    if artifact.content_hash != review_unit.snapshot_artifact_content_hash {
        return Err(ShoreError::Message(format!(
            "snapshot artifact content hash mismatch for {}",
            review_unit.id.as_str()
        )));
    }

    Ok(artifact.snapshot)
}

fn build_snapshot_rows(
    snapshot: &DiffSnapshot,
    review_unit_id: &ReviewUnitId,
) -> (Vec<ReviewUnitProjectionRow>, ReviewUnitProjectionSummary) {
    let mut rows = Vec::new();

    if snapshot.files.is_empty() {
        rows.push(snapshot_row(
            rows.len(),
            ReviewUnitProjectionRowKind::EmptyState,
            None,
            ProjectionCoverage::Context,
            None,
            None,
            None,
        ));
    }

    for (file_index, file) in snapshot.files.iter().enumerate() {
        let file_path = snapshot_file_path(file);
        let old_path = file.old_path.clone();
        let file_target = file_path.as_ref().map(|file_path| ReviewTargetRef::File {
            review_unit_id: review_unit_id.clone(),
            file_path: file_path.clone(),
        });
        rows.push(snapshot_row(
            rows.len(),
            ReviewUnitProjectionRowKind::FileHeader,
            Some(SnapshotOrder {
                file_index,
                metadata_index: None,
                hunk_index: None,
                row_index: None,
            }),
            ProjectionCoverage::Unreviewed,
            file_target,
            file_path.clone(),
            old_path.clone(),
        ));

        for (metadata_index, _) in file.metadata_rows.iter().enumerate() {
            rows.push(snapshot_row(
                rows.len(),
                ReviewUnitProjectionRowKind::Metadata,
                Some(SnapshotOrder {
                    file_index,
                    metadata_index: Some(metadata_index),
                    hunk_index: None,
                    row_index: None,
                }),
                ProjectionCoverage::Unreviewed,
                None,
                file_path.clone(),
                old_path.clone(),
            ));
        }

        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            rows.push(snapshot_row(
                rows.len(),
                ReviewUnitProjectionRowKind::HunkHeader,
                Some(SnapshotOrder {
                    file_index,
                    metadata_index: None,
                    hunk_index: Some(hunk_index),
                    row_index: None,
                }),
                ProjectionCoverage::Unreviewed,
                None,
                file_path.clone(),
                old_path.clone(),
            ));

            for (row_index, _) in hunk.rows.iter().enumerate() {
                rows.push(snapshot_row(
                    rows.len(),
                    ReviewUnitProjectionRowKind::Diff,
                    Some(SnapshotOrder {
                        file_index,
                        metadata_index: None,
                        hunk_index: Some(hunk_index),
                        row_index: Some(row_index),
                    }),
                    ProjectionCoverage::Unreviewed,
                    None,
                    file_path.clone(),
                    old_path.clone(),
                ));
            }
        }
    }

    let summary = ReviewUnitProjectionSummary {
        file_count: snapshot.files.len(),
        row_count: rows.len(),
        snapshot_row_count: rows.len(),
        snapshot_remainder_row_count: rows.len(),
        ..ReviewUnitProjectionSummary::default()
    };

    (rows, summary)
}

fn snapshot_row(
    projection_order: usize,
    kind: ReviewUnitProjectionRowKind,
    snapshot_order: Option<SnapshotOrder>,
    coverage: ProjectionCoverage,
    target: Option<ReviewTargetRef>,
    file_path: Option<String>,
    old_path: Option<String>,
) -> ReviewUnitProjectionRow {
    ReviewUnitProjectionRow {
        id: RowId::new(format!("row:{projection_order:06}")),
        kind,
        projection_phase: ProjectionPhase::SnapshotRemainder,
        projection_order,
        snapshot_order,
        coverage,
        target,
        file_path,
        old_path,
        related_observation_ids: Vec::new(),
        related_intervention_ids: Vec::new(),
        related_disposition_ids: Vec::new(),
    }
}

fn snapshot_file_path(file: &DiffFile) -> Option<String> {
    file.new_path.clone().or_else(|| file.old_path.clone())
}

fn selected_review_unit_capture(
    events: &[ShoreEvent],
    resolved: &ResolvedReviewUnit,
) -> Result<ReviewUnitProjectionIdentity> {
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewUnitCaptured)
    {
        let payload: ReviewUnitCapturedPayload = serde_json::from_value(event.payload.clone())?;
        if payload.review_unit_id == resolved.review_unit_id {
            return Ok(ReviewUnitProjectionIdentity {
                id: payload.review_unit_id,
                review_id: event.target.review_id.clone(),
                source: payload.source,
                base: payload.base,
                target: payload.target,
                revision_id: payload.revision_id,
                snapshot_id: payload.snapshot_id,
                snapshot_artifact_content_hash: payload.snapshot_artifact_content_hash,
            });
        }
    }

    Err(ShoreError::Message(format!(
        "captured review unit event missing for {}",
        resolved.review_unit_id.as_str()
    )))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::*;
    use crate::canonical_hash::sha256_json_prefixed;
    use crate::model::{DiffSnapshot, ReviewId, ReviewUnitId, SnapshotId};
    use crate::session::{CaptureOptions, capture_worktree_review};

    #[test]
    fn show_review_unit_errors_when_no_review_unit_is_captured() {
        let repo = modified_repo();

        let error = show_review_unit(ReviewUnitShowOptions::new(repo.path()))
            .expect_err("no captured ReviewUnit should fail");

        assert!(error.to_string().contains("no captured review unit"));
    }

    #[test]
    fn show_review_unit_resolves_single_current_review_unit_and_freshness() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = show_review_unit(ReviewUnitShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.review_unit.id, capture.review_unit_id);
        assert_eq!(result.review_unit.revision_id, capture.revision_id);
        assert_eq!(result.review_unit.snapshot_id, capture.snapshot_id);
        assert_eq!(result.filters.review_unit_id, capture.review_unit_id);
        assert_eq!(result.event_count, 1);
        assert!(result.event_set_hash.starts_with("sha256:"));
    }

    #[test]
    fn show_review_unit_requires_explicit_id_when_current_is_ambiguous() {
        let repo = modified_repo();
        let first = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
        let second = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let error = show_review_unit(ReviewUnitShowOptions::new(repo.path()))
            .expect_err("multiple captures should be ambiguous");
        assert!(error.to_string().contains("multiple captured review units"));

        let explicit = show_review_unit(
            ReviewUnitShowOptions::new(repo.path())
                .with_review_unit_id(first.review_unit_id.clone()),
        )
        .unwrap();

        assert_ne!(first.review_unit_id, second.review_unit_id);
        assert_eq!(explicit.review_unit.id, first.review_unit_id);
        assert_eq!(explicit.event_count, 2);
    }

    #[test]
    fn show_review_unit_uses_captured_snapshot_after_worktree_drift() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 99 }\n");

        let result = show_review_unit(ReviewUnitShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.review_unit.id, capture.review_unit_id);
        assert_eq!(
            result.snapshot.files[0].new_path.as_deref(),
            Some("src/lib.rs")
        );
        assert!(format!("{:?}", result.snapshot).contains("2"));
        assert!(!format!("{:?}", result.snapshot).contains("99"));
    }

    #[test]
    fn show_review_unit_rejects_snapshot_artifact_hash_mismatch() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        tamper_snapshot_artifact_target(repo.path(), &capture.snapshot_id, "/other/repo");

        let error = show_review_unit(ReviewUnitShowOptions::new(repo.path()))
            .expect_err("tampered artifact should fail");

        assert!(error.to_string().contains("content hash"));
    }

    #[test]
    fn show_review_unit_rejects_event_artifact_binding_mismatch() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        rewrite_capture_event_snapshot_artifact_hash(
            repo.path(),
            &capture.review_unit_id,
            "sha256:bad",
        );

        let error = show_review_unit(ReviewUnitShowOptions::new(repo.path()))
            .expect_err("event/artifact mismatch should fail");

        assert!(error.to_string().contains("snapshot artifact content hash"));
    }

    #[test]
    fn show_review_unit_emits_snapshot_rows_in_captured_order() {
        let repo = multi_file_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = show_review_unit(ReviewUnitShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.rows[0].kind.as_str(), "file_header");
        assert_eq!(
            result.rows[0].projection_phase.as_str(),
            "snapshot_remainder"
        );
        assert_eq!(result.rows[0].coverage.as_str(), "unreviewed");
        assert_eq!(result.rows[0].projection_order, 0);
        assert_eq!(
            result.rows[0].snapshot_order.as_ref().unwrap().file_index,
            0
        );
        assert!(result.rows.iter().any(|row| row.kind.as_str() == "diff"));
    }

    #[test]
    fn show_review_unit_emits_empty_state_row_for_empty_snapshot() {
        let (rows, summary) = build_snapshot_rows(
            &DiffSnapshot::new(
                ReviewId::new("review:empty"),
                SnapshotId::new("snap:empty"),
                Vec::new(),
            ),
            &ReviewUnitId::new("review-unit:empty"),
        );

        assert_eq!(summary.file_count, 0);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind.as_str(), "empty_state");
    }

    #[test]
    fn show_review_unit_rows_do_not_expose_storage_paths() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = show_review_unit(ReviewUnitShowOptions::new(repo.path())).unwrap();
        let debug = format!("{result:?}");

        assert!(!debug.contains("artifacts/snapshots"));
        assert!(!debug.contains(".shore/events"));
    }

    fn modified_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        repo
    }

    fn multi_file_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.write("src/other.rs", "pub fn other() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        repo.write("src/other.rs", "pub fn other() -> u32 { 2 }\n");
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

        fn write(&self, path: impl AsRef<Path>, contents: impl AsRef<[u8]>) {
            let path = self.root.path().join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directories");
            }
            fs::write(path, contents).expect("write test repository file");
        }

        fn commit_all(&self, message: &str) {
            self.git(["add", "--all"]);
            self.git(["commit", "-m", message]);
        }

        fn git<I, S>(&self, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<OsStr>,
        {
            let args = args
                .into_iter()
                .map(|arg| arg.as_ref().to_owned())
                .collect::<Vec<_>>();
            let output = Command::new("git")
                .args(&args)
                .current_dir(self.root.path())
                .output()
                .unwrap_or_else(|error| panic!("run git {:?}: {error}", args));

            assert!(
                output.status.success(),
                "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
                args,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    fn tamper_snapshot_artifact_target(repo: &Path, snapshot_id: &SnapshotId, target_root: &str) {
        let path = snapshot_artifact_path(repo, snapshot_id);
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("read snapshot artifact"))
                .expect("parse snapshot artifact json");

        assert_eq!(json["snapshot"]["snapshot_id"], snapshot_id.as_str());
        json["target"]["worktreeRoot"] = target_root.into();

        fs::write(
            &path,
            serde_json::to_vec_pretty(&json).expect("serialize tampered snapshot artifact"),
        )
        .expect("write tampered snapshot artifact");
    }

    fn rewrite_capture_event_snapshot_artifact_hash(
        repo: &Path,
        review_unit_id: &ReviewUnitId,
        hash: &str,
    ) {
        let path = capture_event_path(repo, review_unit_id);
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("read capture event"))
                .expect("parse capture event json");

        json["payload"]["snapshotArtifactContentHash"] = hash.into();
        json["payloadHash"] = sha256_json_prefixed(&json["payload"])
            .expect("hash rewritten capture event payload")
            .into();

        fs::write(
            &path,
            serde_json::to_vec_pretty(&json).expect("serialize rewritten capture event"),
        )
        .expect("write rewritten capture event");
    }

    fn snapshot_artifact_path(repo: &Path, snapshot_id: &SnapshotId) -> PathBuf {
        fs::read_dir(repo.join(".shore/artifacts/snapshots"))
            .expect("read snapshot artifacts directory")
            .map(|entry| entry.expect("read snapshot artifact dir entry").path())
            .find(|path| {
                let Ok(bytes) = fs::read(path) else {
                    return false;
                };
                let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                    return false;
                };
                json["snapshot"]["snapshot_id"] == snapshot_id.as_str()
            })
            .expect("find snapshot artifact")
    }

    fn capture_event_path(repo: &Path, review_unit_id: &ReviewUnitId) -> PathBuf {
        fs::read_dir(repo.join(".shore/events"))
            .expect("read events directory")
            .map(|entry| entry.expect("read event dir entry").path())
            .find(|path| {
                let Ok(bytes) = fs::read(path) else {
                    return false;
                };
                let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                    return false;
                };
                json["eventType"] == "review_unit_captured"
                    && json["payload"]["reviewUnitId"] == review_unit_id.as_str()
            })
            .expect("find capture event")
    }
}
