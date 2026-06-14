use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::crypto::EventSigner;
use crate::error::Result;
use crate::git::{
    IngestOptions, capture_commit_range_diff_files, git_commit_tree_oid, git_rev_parse_commit_oid,
    ingest_tracked_diff_with_options,
};
use crate::model::{
    ActorId, DiffFile, DiffSnapshot, ReviewEndpoint, ReviewId, ReviewUnitId, ReviewUnitSource,
    RevisionId, SessionId, SnapshotId,
};
use crate::session::event::{EventTarget, EventType, ReviewUnitCapturedPayload, ShoreEvent};
use crate::session::fingerprint::{ResolvedCommitEndpoint, ReviewUnitFingerprint};
use crate::session::store::resolution::{StoreResolutionMode, resolve_store};
use crate::session::{
    EventSigningOptions, EventStore, EventWriteOutcome, ProjectionDiagnostic, SessionState,
    ShoreStorePaths, current_timestamp, prepare_shore_writer, sign_event_if_requested,
    writer_from_options,
};
use crate::storage::{Durability, LocalStorage};

const CLONE_LOCAL_CAPTURE_BATCH_ONLY_CODE: &str = "clone_local_capture_batch_only";

/// Commit-range capture input: a base rev and an optional target rev (defaults
/// to `HEAD`). Revs are resolved to commit OIDs at capture time; the spellings
/// are never stored, so equivalent spellings (`HEAD~1` vs the resolved OID)
/// capture the same ReviewUnit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitRangeSpec {
    base_rev: String,
    target_rev: Option<String>,
}

impl CommitRangeSpec {
    pub fn new(base_rev: impl Into<String>) -> Self {
        Self {
            base_rev: base_rev.into(),
            target_rev: None,
        }
    }

    pub fn with_target_rev(mut self, target_rev: impl Into<String>) -> Self {
        self.target_rev = Some(target_rev.into());
        self
    }
}

/// Which source adapter a capture lowers through. The default is the worktree
/// (`HEAD` -> working tree) adapter; `CommitRange` is the first explicit
/// non-worktree source (research 0004 carry-forward).
#[derive(Clone, Debug, Eq, PartialEq)]
enum CaptureSourceSpec {
    Worktree,
    CommitRange(CommitRangeSpec),
}

impl CaptureSourceSpec {
    /// Stable label for the capture tracing span.
    fn label(&self) -> &'static str {
        match self {
            Self::Worktree => "worktree",
            Self::CommitRange(_) => "commit_range",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureOptions {
    repo: PathBuf,
    source: CaptureSourceSpec,
    excluded_helper_paths: Vec<PathBuf>,
    actor_id: Option<ActorId>,
    signing: EventSigningOptions,
}

impl CaptureOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            source: CaptureSourceSpec::Worktree,
            excluded_helper_paths: Vec::new(),
            actor_id: None,
            signing: EventSigningOptions::default(),
        }
    }

    /// Capture the tree diff of a commit range instead of the `HEAD` ->
    /// working-tree diff. The working tree and untracked files are not read, and
    /// helper-path exclusion does not apply (a range capture is a faithful tree
    /// diff). The default (no call) keeps today's worktree capture behavior.
    pub fn with_commit_range(mut self, range: CommitRangeSpec) -> Self {
        self.source = CaptureSourceSpec::CommitRange(range);
        self
    }

    /// Attribute the captured `review_unit_captured` event to an explicit
    /// actor, overriding the `SHORE_ACTOR_ID` env var and the local Git
    /// identity. A malformed id is ignored (falls back to env, then Git);
    /// `None` keeps the default resolution. The ReviewUnit id is derived from
    /// snapshot content, so the override changes attribution only, not identity.
    pub fn with_actor_id(mut self, actor_id: ActorId) -> Self {
        self.actor_id = Some(actor_id);
        self
    }

    /// Excludes an explicit command-helper path from the captured snapshot.
    ///
    /// This is intentionally narrow CLI plumbing for files such as `--log-file`.
    /// Other untracked agent/producer files remain part of the ReviewUnit unless the
    /// caller chooses to exclude them.
    pub fn with_excluded_helper_path(mut self, path: impl AsRef<Path>) -> Self {
        self.excluded_helper_paths.push(path.as_ref().to_path_buf());
        self
    }

    pub fn sign_with<S>(mut self, signer: S) -> Self
    where
        S: EventSigner + Send + Sync + 'static,
    {
        self.signing = EventSigningOptions::sign_with(signer);
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

/// Canonical capture entry point. Dispatches on the options' source spec to a
/// source adapter (worktree by default, or a commit range), then runs the
/// shared tail: write the snapshot artifact, record the idempotent
/// `review_unit_captured` event, rebuild projection state, and surface the
/// clone-local diagnostic.
pub fn capture_review(options: CaptureOptions) -> Result<CaptureResult> {
    let span = tracing::info_span!(
        "session.capture_review",
        repo = %options.repo.display(),
        source = options.source.label(),
    );
    let _entered = span.enter();

    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let worktree_root = paths.worktree_root();
    let store_resolution = resolve_store(worktree_root)?;
    let shore_dir = paths.shore_dir();
    let storage = LocalStorage::new(shore_dir);
    prepare_shore_writer(&paths, &storage)?;

    let PreparedCapture { files, fingerprint } = match &options.source {
        CaptureSourceSpec::Worktree => prepare_worktree_capture(worktree_root, &options)?,
        CaptureSourceSpec::CommitRange(range) => {
            prepare_commit_range_capture(worktree_root, range)?
        }
    };
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
    let writer = writer_from_options(worktree_root, options.actor_id.as_ref());
    let occurred_at = current_timestamp();
    let mut event = ShoreEvent::new(
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
    )?;
    sign_event_if_requested(&mut event, &options.signing)?;
    recorder.record(&event_store, event)?;

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

/// Worktree-source convenience entry point. [`capture_review`] is the canonical
/// entry; this alias keeps existing library consumers (e.g. the shoreline-relay
/// bridge) compiling. It honors whatever source spec the options carry; the spec
/// defaults to the worktree adapter, so plain delegation preserves behavior.
pub fn capture_worktree_review(options: CaptureOptions) -> Result<CaptureResult> {
    capture_review(options)
}

/// The row inventory plus resolved identity an adapter hands to the shared tail.
struct PreparedCapture {
    files: Vec<DiffFile>,
    fingerprint: ReviewUnitFingerprint,
}

/// Worktree adapter: ingest the `HEAD` -> working-tree diff (helper-path
/// exclusion applies here only) and fingerprint it as a worktree review unit.
fn prepare_worktree_capture(
    worktree_root: &Path,
    options: &CaptureOptions,
) -> Result<PreparedCapture> {
    let snapshot =
        ingest_tracked_diff_with_options(worktree_root, capture_ingest_options(options))?;
    let files = snapshot.files;
    let fingerprint =
        crate::session::fingerprint::review_unit_fingerprint_for_files(worktree_root, &files)?;
    Ok(PreparedCapture { files, fingerprint })
}

/// Commit-range adapter: resolve the base/target revs to commit endpoints,
/// diff the two trees (no working-tree, index, or untracked involvement; no
/// helper-path exclusion), and fingerprint it as a commit-range review unit.
fn prepare_commit_range_capture(
    worktree_root: &Path,
    range: &CommitRangeSpec,
) -> Result<PreparedCapture> {
    let base = resolve_commit_endpoint(worktree_root, &range.base_rev)?;
    let target_rev = range.target_rev.as_deref().unwrap_or("HEAD");
    let target = resolve_commit_endpoint(worktree_root, target_rev)?;
    let files =
        capture_commit_range_diff_files(worktree_root, &base.commit_oid, &target.commit_oid)?;
    let fingerprint = crate::session::fingerprint::commit_range_review_unit_fingerprint_for_files(
        worktree_root,
        &base,
        &target,
        &files,
    )?;
    Ok(PreparedCapture { files, fingerprint })
}

/// Resolve a user rev to a commit endpoint (commit OID + tree OID). The
/// underlying git helper's error already names the rev and says "commit", so a
/// failed `--base`/`--target` surfaces honestly to the CLI without coupling the
/// library to flag spellings.
fn resolve_commit_endpoint(repo: &Path, rev: &str) -> Result<ResolvedCommitEndpoint> {
    let commit_oid = git_rev_parse_commit_oid(repo, rev)?;
    let tree_oid = git_commit_tree_oid(repo, &commit_oid)?;
    Ok(ResolvedCommitEndpoint {
        commit_oid,
        tree_oid,
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
            EventWriteOutcome::Existing | EventWriteOutcome::ExistingDivergentSignature => {
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

    use crate::model::{
        CommitRangeCaptureMode, ReviewEndpoint, ReviewUnitLineageId, ReviewUnitSource,
    };
    use crate::session::event::EventType;
    use crate::session::{
        CaptureOptions, CommitRangeSpec, EventStore, LineageAttachOptions,
        attach_review_unit_to_lineage, capture_review, capture_worktree_review,
        read_snapshot_artifact,
    };

    #[test]
    fn capture_review_from_commit_range_records_commit_pair_endpoints() {
        let repo = committed_repo();
        let head_oid = repo.rev_parse("HEAD");

        let result = capture_review(
            CaptureOptions::new(repo.path()).with_commit_range(CommitRangeSpec::new("HEAD~1")),
        )
        .unwrap();

        assert!(matches!(
            result.source,
            ReviewUnitSource::GitCommitRange {
                mode: CommitRangeCaptureMode::BaseTreeToTargetTree
            }
        ));
        match &result.base {
            ReviewEndpoint::GitCommit { .. } => {}
            other => panic!("unexpected base endpoint: {other:?}"),
        }
        match &result.target {
            ReviewEndpoint::GitCommit { commit_oid, .. } => {
                // Target defaulted to HEAD.
                assert_eq!(commit_oid.as_str(), head_oid.as_str());
            }
            other => panic!("unexpected target endpoint: {other:?}"),
        }
        assert!(
            result
                .review_unit_id
                .as_str()
                .starts_with("review-unit:sha256:")
        );
        assert_eq!(result.events_created_by_type["review_unit_captured"], 1);
    }

    #[test]
    fn capture_review_from_commit_range_binds_snapshot_artifact() {
        let repo = committed_repo();

        let result = capture_review(
            CaptureOptions::new(repo.path()).with_commit_range(CommitRangeSpec::new("HEAD~1")),
        )
        .unwrap();
        let artifact = read_snapshot_artifact(repo.path(), &result.snapshot_id).unwrap();

        assert_eq!(artifact.source, result.source);
        assert_eq!(artifact.base, result.base);
        assert_eq!(artifact.target, result.target);
        assert_eq!(artifact.content_hash, result.snapshot_artifact_content_hash);
        assert!(
            artifact
                .snapshot
                .files
                .iter()
                .any(|file| file.new_path.as_deref() == Some("src/lib.rs"))
        );
    }

    #[test]
    fn capture_review_rejects_unresolvable_base_rev() {
        let repo = committed_repo();

        let error = capture_review(
            CaptureOptions::new(repo.path()).with_commit_range(CommitRangeSpec::new("no-such-rev")),
        )
        .unwrap_err();

        let message = error.to_string();
        assert!(message.contains("no-such-rev"), "message: {message}");
        assert!(message.contains("commit"), "message: {message}");
    }

    #[test]
    fn capture_review_with_worktree_source_matches_capture_worktree_review() {
        let repo = modified_repo();

        let first = capture_review(CaptureOptions::new(repo.path())).unwrap();
        let second = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        // Default spec is the worktree adapter; the second capture hits the same
        // idempotency key and reports the existing event.
        assert_eq!(first.review_unit_id, second.review_unit_id);
        assert_eq!(first.snapshot_id, second.snapshot_id);
        assert_eq!(second.events_existing, 1);
    }

    #[test]
    fn range_recapture_is_idempotent_with_existing_diagnostics() {
        let repo = committed_repo();

        let first = capture_review(
            CaptureOptions::new(repo.path()).with_commit_range(CommitRangeSpec::new("HEAD~1")),
        )
        .unwrap();
        let second = capture_review(
            CaptureOptions::new(repo.path()).with_commit_range(CommitRangeSpec::new("HEAD~1")),
        )
        .unwrap();

        assert_eq!(first.review_unit_id, second.review_unit_id);
        assert_eq!(first.snapshot_id, second.snapshot_id);
        assert_eq!(second.events_created, 0);
        assert_eq!(second.events_existing, 1);
    }

    #[test]
    fn range_recapture_via_equivalent_spelling_is_idempotent() {
        let repo = committed_repo();
        let base_oid = repo.rev_parse("HEAD~1");

        let first = capture_review(
            CaptureOptions::new(repo.path()).with_commit_range(CommitRangeSpec::new("HEAD~1")),
        )
        .unwrap();
        // The resolved OID spelling must capture the same unit: spellings are not
        // stored, so they cannot fork identity.
        let second = capture_review(
            CaptureOptions::new(repo.path()).with_commit_range(CommitRangeSpec::new(base_oid)),
        )
        .unwrap();

        assert_eq!(first.review_unit_id, second.review_unit_id);
        assert_eq!(second.events_existing, 1);
    }

    #[test]
    fn range_capture_excludes_dirty_worktree_and_untracked_files() {
        let repo = committed_repo();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 999 }\n");
        repo.write("untracked.txt", "untracked\n");

        let result = capture_review(
            CaptureOptions::new(repo.path()).with_commit_range(CommitRangeSpec::new("HEAD~1")),
        )
        .unwrap();
        let artifact = read_snapshot_artifact(repo.path(), &result.snapshot_id).unwrap();

        let paths: Vec<&str> = artifact
            .snapshot
            .files
            .iter()
            .filter_map(|file| file.new_path.as_deref())
            .collect();
        assert_eq!(paths, vec!["src/lib.rs"]);
        assert!(!paths.contains(&"untracked.txt"));
        assert!(artifact.snapshot.files.iter().all(|file| !file.synthetic));
    }

    #[test]
    fn range_capture_of_identical_trees_captures_empty_snapshot() {
        let repo = committed_repo();

        let first = capture_review(
            CaptureOptions::new(repo.path()).with_commit_range(CommitRangeSpec::new("HEAD")),
        )
        .unwrap();
        let artifact = read_snapshot_artifact(repo.path(), &first.snapshot_id).unwrap();
        assert!(artifact.snapshot.files.is_empty());

        let second = capture_review(
            CaptureOptions::new(repo.path()).with_commit_range(CommitRangeSpec::new("HEAD")),
        )
        .unwrap();
        assert_eq!(first.review_unit_id, second.review_unit_id);
        assert_eq!(second.events_existing, 1);
    }

    #[test]
    fn staged_worktree_capture_then_range_capture_do_not_conflict() {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        repo.git(["add", "--all"]);

        // Capture the fully-staged change as a worktree unit, then commit the same
        // tree and range-capture it: the snapshot ids must differ so the artifact
        // layer never conflicts (the collision the tree-pair snapshot hash prevents).
        let worktree = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        repo.commit_all("change");
        let range = capture_review(
            CaptureOptions::new(repo.path()).with_commit_range(CommitRangeSpec::new("HEAD~1")),
        )
        .unwrap();

        assert_ne!(worktree.snapshot_id, range.snapshot_id);
        assert_ne!(worktree.review_unit_id, range.review_unit_id);
        // Both artifacts are independently readable: no conflict overwrote either.
        read_snapshot_artifact(repo.path(), &worktree.snapshot_id).unwrap();
        read_snapshot_artifact(repo.path(), &range.snapshot_id).unwrap();

        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let captured = events
            .iter()
            .filter(|event| event.event_type == EventType::ReviewUnitCaptured)
            .count();
        assert_eq!(captured, 2);
    }

    #[test]
    fn range_capture_attaches_to_lineage_with_commit_range_basis() {
        let repo = committed_repo();
        let result = capture_review(
            CaptureOptions::new(repo.path()).with_commit_range(CommitRangeSpec::new("HEAD~1")),
        )
        .unwrap();

        let lineage_id = ReviewUnitLineageId::new("review-unit-lineage:random:test");
        attach_review_unit_to_lineage(
            LineageAttachOptions::new(repo.path(), lineage_id)
                .with_review_unit_id(result.review_unit_id.clone()),
        )
        .unwrap();

        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let declared = events
            .iter()
            .find(|event| event.event_type == EventType::ReviewUnitLineageDeclared)
            .unwrap();
        assert_eq!(
            declared.payload["basis"]["source"]["kind"],
            "git_commit_range"
        );
        assert_eq!(declared.payload["basis"]["base"]["kind"], "git_commit");
    }

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
    fn capture_worktree_review_with_actor_id_attributes_override_as_author() {
        use crate::model::ActorId;

        let repo = modified_repo();
        let result = capture_worktree_review(
            CaptureOptions::new(repo.path()).with_actor_id(ActorId::new("actor:agent:capturer")),
        )
        .unwrap();

        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let event = events
            .iter()
            .find(|event| event.event_type == EventType::ReviewUnitCaptured)
            .unwrap();

        // Attribution changes; the ReviewUnit id is derived from snapshot content, not the writer.
        assert_eq!(event.writer.actor_id.as_str(), "actor:agent:capturer");
        assert!(
            result
                .review_unit_id
                .as_str()
                .starts_with("review-unit:sha256:")
        );
    }

    #[test]
    fn capture_worktree_review_without_actor_id_uses_git_identity() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let events = EventStore::open(repo.path().join(".shore"))
            .list_events()
            .unwrap();
        let event = events
            .iter()
            .find(|event| event.event_type == EventType::ReviewUnitCaptured)
            .unwrap();
        assert_eq!(
            event.writer.actor_id.as_str(),
            "actor:git-email:shore-tests@example.com"
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

    fn committed_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        repo.commit_all("change");
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

        fn rev_parse(&self, rev: &str) -> String {
            let output = Command::new("git")
                .args(["rev-parse", rev])
                .current_dir(self.root.path())
                .output()
                .expect("run git rev-parse");
            assert!(output.status.success(), "git rev-parse {rev} failed");
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
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
