use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::git::{IngestOptions, ingest_tracked_diff_with_options};
use crate::model::{
    DiffSnapshot, ReviewEndpoint, ReviewId, ReviewUnitId, ReviewUnitSource, RevisionId, SessionId,
    SnapshotId,
};
use crate::session::event::{EventTarget, EventType, ReviewUnitCapturedPayload, ShoreEvent};
use crate::session::store::resolution::{StoreResolutionMode, resolve_store};
use crate::session::{
    EventStore, EventWriteOutcome, ProjectionDiagnostic, SessionState, ShoreStorePaths,
    current_timestamp, prepare_shore_writer, writer_from_git_config,
};
use crate::storage::{Durability, LocalStorage};

const CLONE_LOCAL_CAPTURE_BATCH_ONLY_CODE: &str = "clone_local_capture_batch_only";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureOptions {
    repo: PathBuf,
    excluded_helper_paths: Vec<PathBuf>,
}

impl CaptureOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            excluded_helper_paths: Vec::new(),
        }
    }

    /// Excludes an explicit command-helper path from the captured snapshot.
    ///
    /// This is intentionally narrow CLI plumbing for files such as `--log-file`.
    /// Other untracked agent/tool files remain part of the ReviewUnit unless the
    /// caller chooses to exclude them.
    pub fn with_excluded_helper_path(mut self, path: impl AsRef<Path>) -> Self {
        self.excluded_helper_paths.push(path.as_ref().to_path_buf());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureResult {
    pub session_id: SessionId,
    pub review_unit_id: ReviewUnitId,
    pub revision_id: RevisionId,
    pub snapshot_id: SnapshotId,
    pub source: ReviewUnitSource,
    pub base: ReviewEndpoint,
    pub target: ReviewEndpoint,
    pub snapshot_artifact_content_hash: String,
    pub events_created: usize,
    pub events_existing: usize,
    pub events_created_by_type: BTreeMap<String, usize>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn capture_worktree_review(options: CaptureOptions) -> Result<CaptureResult> {
    let span =
        tracing::info_span!("session.capture_worktree_review", repo = %options.repo.display());
    let _entered = span.enter();

    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let worktree_root = paths.worktree_root();
    let store_resolution = resolve_store(worktree_root)?;
    let shore_dir = paths.shore_dir();
    let storage = LocalStorage::new(shore_dir);
    prepare_shore_writer(&paths, &storage)?;

    let snapshot =
        ingest_tracked_diff_with_options(worktree_root, capture_ingest_options(&options))?;
    let files = snapshot.files;
    let fingerprint =
        crate::session::fingerprint::review_unit_fingerprint_for_files(worktree_root, &files)?;
    let review_id = ReviewId::new("review:default");
    let session_id = SessionId::new("session:default");
    let snapshot = DiffSnapshot::new(review_id, fingerprint.snapshot_id.clone(), files);
    let artifact = crate::session::snapshot_artifact::write_snapshot_artifact(
        worktree_root,
        &fingerprint,
        snapshot,
    )?;

    let event_store = EventStore::open(shore_dir);
    let mut recorder = CaptureRecorder::default();
    let writer = writer_from_git_config(worktree_root);
    let occurred_at = current_timestamp();
    recorder.record(
        &event_store,
        ShoreEvent::new(
            EventType::ReviewUnitCaptured,
            review_unit_captured_idempotency_key(&fingerprint.review_unit_id),
            EventTarget::for_review_unit(
                session_id.clone(),
                fingerprint.review_unit_id.clone(),
                fingerprint.revision_id.clone(),
                fingerprint.snapshot_id.clone(),
            ),
            writer,
            ReviewUnitCapturedPayload {
                review_unit_id: fingerprint.review_unit_id.clone(),
                source: fingerprint.source.clone(),
                base: fingerprint.base.clone(),
                target: fingerprint.target.clone(),
                revision_id: fingerprint.revision_id.clone(),
                snapshot_id: fingerprint.snapshot_id.clone(),
                snapshot_artifact_content_hash: artifact.content_hash.clone(),
            },
            occurred_at,
        )?,
    )?;

    let state = SessionState::from_events(&event_store.list_events()?)?;
    storage.write_json_atomic(&paths.state_path(), &state, Durability::Projection)?;
    let mut diagnostics = state.diagnostics;
    if store_resolution.mode == StoreResolutionMode::CloneLocal {
        diagnostics.push(ProjectionDiagnostic {
            code: CLONE_LOCAL_CAPTURE_BATCH_ONLY_CODE.to_owned(),
            message:
                "review capture writes local facts; run shore store link to copy them to the linked clone-local store"
                    .to_owned(),
        });
    }

    Ok(CaptureResult {
        session_id,
        review_unit_id: fingerprint.review_unit_id,
        revision_id: fingerprint.revision_id,
        snapshot_id: fingerprint.snapshot_id,
        source: fingerprint.source,
        base: fingerprint.base,
        target: fingerprint.target,
        snapshot_artifact_content_hash: artifact.content_hash,
        events_created: recorder.events_created,
        events_existing: recorder.events_existing,
        events_created_by_type: recorder.events_created_by_type,
        diagnostics,
    })
}

fn capture_ingest_options(options: &CaptureOptions) -> IngestOptions {
    options
        .excluded_helper_paths
        .iter()
        .fold(IngestOptions::new(), |options, path| {
            options.exclude_helper_path(path)
        })
}

fn review_unit_captured_idempotency_key(review_unit_id: &ReviewUnitId) -> String {
    format!("review_unit_captured:{}", review_unit_id.as_str())
}

#[derive(Default)]
struct CaptureRecorder {
    events_created: usize,
    events_existing: usize,
    events_created_by_type: BTreeMap<String, usize>,
}

impl CaptureRecorder {
    fn record(&mut self, event_store: &EventStore, event: ShoreEvent) -> Result<()> {
        match event_store.record_event_once(&event)? {
            EventWriteOutcome::Created => {
                self.events_created += 1;
                *self
                    .events_created_by_type
                    .entry("review_unit_captured".to_owned())
                    .or_default() += 1;
            }
            EventWriteOutcome::Existing => {
                self.events_existing += 1;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use crate::session::event::EventType;
    use crate::session::{
        CaptureOptions, EventStore, capture_worktree_review, read_snapshot_artifact,
    };

    #[test]
    fn capture_worktree_review_writes_event_artifact_and_state() {
        let repo = modified_repo();

        let result = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let artifact = read_snapshot_artifact(repo.path(), &result.snapshot_id).unwrap();

        assert!(repo.path().join(".shore/events").is_dir());
        assert!(repo.path().join(".shore/state.json").is_file());
        assert_eq!(artifact.review_unit_id, result.review_unit_id);
        assert!(
            result
                .review_unit_id
                .as_str()
                .starts_with("review-unit:sha256:")
        );
        assert_eq!(result.events_created_by_type["review_unit_captured"], 1);
        assert!(
            !result
                .events_created_by_type
                .contains_key("review_initialized")
        );
    }

    #[test]
    fn capture_worktree_review_binds_event_to_snapshot_artifact_hash() {
        let repo = modified_repo();

        let result = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let artifact = read_snapshot_artifact(repo.path(), &result.snapshot_id).unwrap();
        let event_store = EventStore::open(repo.path().join(".shore"));
        let events = event_store.list_events().unwrap();
        let event = events
            .iter()
            .find(|event| event.event_type == EventType::ReviewUnitCaptured)
            .unwrap();

        assert_eq!(result.snapshot_artifact_content_hash, artifact.content_hash);
        assert_eq!(
            event.payload["snapshotArtifactContentHash"],
            artifact.content_hash
        );
    }

    #[test]
    fn capture_worktree_review_preserves_fresh_shore_temp_files() {
        let repo = modified_repo();
        let temp_path = repo.path().join(".shore/events/.shore-write.inflight.tmp");

        fs::create_dir_all(temp_path.parent().unwrap()).unwrap();
        fs::write(&temp_path, b"in flight").unwrap();

        let result = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        assert_eq!(result.events_created_by_type["review_unit_captured"], 1);
        assert_eq!(
            fs::read(&temp_path).unwrap(),
            b"in flight",
            "capture startup must not remove fresh temp files from another in-flight writer"
        );
    }

    #[test]
    fn capture_from_subdir_uses_worktree_root() {
        let repo = modified_repo();
        let subdir = repo.path().join("src");

        let result = capture_worktree_review(CaptureOptions::new(&subdir)).unwrap();

        assert!(repo.path().join(".shore/events").is_dir());
        assert!(
            result
                .review_unit_id
                .as_str()
                .starts_with("review-unit:sha256:")
        );
    }

    fn modified_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
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
                "git {:?} failed\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
                args,
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
