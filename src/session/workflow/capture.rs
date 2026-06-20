use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::crypto::EventSigner;
use crate::error::Result;
use crate::git::{
    IngestOptions, capture_commit_range_diff_files, git_commit_tree_oid, git_head_oid,
    git_head_ref, git_rev_parse_commit_oid, ingest_tracked_diff_with_options,
};
use crate::model::{
    ActorId, DiffFile, DiffSnapshot, ReviewEndpoint, ReviewId, ReviewUnitId, ReviewUnitSource,
    RevisionId, SessionId, SnapshotId,
};
use crate::session::event::{EventTarget, EventType, ReviewUnitCapturedPayload, ShoreEvent};
use crate::session::fingerprint::{ResolvedCommitEndpoint, ReviewUnitFingerprint};
use crate::session::store::resolution::{prepare_write_landing, resolve_write_store};
use crate::session::{
    BestEffortSkipSink, EventSigningOptions, EventStore, EventWriteOutcome, ProjectionDiagnostic,
    SessionState, current_timestamp, sign_event_if_requested, writer_from_options,
};
use crate::storage::{Durability, LocalStorage};

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

    pub fn sign_with_best_effort<S>(mut self, signer: S, skip_sink: BestEffortSkipSink) -> Self
    where
        S: EventSigner + Send + Sync + 'static,
    {
        self.signing = EventSigningOptions::sign_with_best_effort(signer, skip_sink);
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

    // The write landing is mode-aware (INV-1): in a linked worktree the
    // artifact, event, and state.json all land in the clone-local store, so the
    // same worktree's reads (which already resolve it) see the capture in place.
    let write_store = resolve_write_store(&options.repo)?;
    let worktree_root = write_store.worktree_root().to_path_buf();
    let store_dir = write_store.store_dir().to_path_buf();
    let storage = LocalStorage::new(&store_dir);
    prepare_write_landing(&write_store, &storage)?;

    let PreparedCapture { files, fingerprint } = match &options.source {
        CaptureSourceSpec::Worktree => prepare_worktree_capture(&worktree_root, &options)?,
        CaptureSourceSpec::CommitRange(range) => {
            prepare_commit_range_capture(&worktree_root, range)?
        }
    };
    let review_id = ReviewId::new("review:default");
    let session_id = SessionId::new("session:default");
    let snapshot = DiffSnapshot::new(review_id, fingerprint.snapshot_id.clone(), files);
    let artifact = crate::session::snapshot_artifact::write_snapshot_artifact_to(
        &store_dir,
        &fingerprint,
        snapshot,
    )?;

    let event_store = EventStore::open(&store_dir);
    let mut recorder = CaptureRecorder::default();
    let writer = writer_from_options(&worktree_root, options.actor_id.as_ref());
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

    // Record the capture-time branch ref as a best-effort `ReviewUnitRefAssociated`
    // after the capture event, whenever the capture tips at the checked-out
    // branch: always for a worktree capture (its target is HEAD's working tree),
    // and for a commit-range capture only when its target endpoint is the current
    // HEAD. An arbitrary historical range (target != HEAD) records nothing — the
    // checked-out branch is not its provenance. Detached HEAD names no ref, so the
    // helper skips it. Any failure degrades to a diagnostic and never blocks capture.
    let auto_record_ref = match &options.source {
        CaptureSourceSpec::Worktree => true,
        CaptureSourceSpec::CommitRange(_) => matches!(
            &fingerprint.target,
            ReviewEndpoint::GitCommit { commit_oid, .. }
                if *commit_oid == git_head_oid(&worktree_root)?
        ),
    };
    let mut auto_record_diagnostics = Vec::new();
    if auto_record_ref
        && let Err(error) = auto_record_capture_ref_association(
            &worktree_root,
            &event_store,
            &mut recorder,
            &fingerprint,
            &session_id,
            &options,
        )
    {
        auto_record_diagnostics.push(ProjectionDiagnostic {
            code: "ref_association_auto_record_skipped".to_owned(),
            message: format!("capture-time ref association was not recorded: {error}"),
        });
    }

    let state = SessionState::from_events(&event_store.list_events()?)?;
    storage.write_json_atomic(
        &store_dir.join("state.json"),
        &state,
        Durability::Projection,
    )?;
    // Write-through (INV-1) lands the capture in the store reads already resolve,
    // so there is no longer a batch-only diagnostic telling the user to run
    // `shore store link` before their own capture is visible.
    let mut diagnostics = state.diagnostics;
    diagnostics.extend(auto_record_diagnostics);

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

/// Auto-record the capture-time branch ref as a `ReviewUnitRefAssociated`
/// (LB-11). Returns `Ok(())` and records nothing on a detached HEAD — a ref name
/// is never fabricated. Signs with the capture signer; the caller swallows any
/// error into a diagnostic so capture never fails on this.
fn auto_record_capture_ref_association(
    worktree_root: &Path,
    event_store: &EventStore,
    recorder: &mut CaptureRecorder,
    fingerprint: &ReviewUnitFingerprint,
    session_id: &SessionId,
    options: &CaptureOptions,
) -> Result<()> {
    let Some(ref_name) = git_head_ref(worktree_root)? else {
        return Ok(());
    };
    let head_oid = git_head_oid(worktree_root)?;
    let writer = writer_from_options(worktree_root, options.actor_id.as_ref());
    let mut event = super::association::build_ref_association_event(
        session_id,
        &fingerprint.review_unit_id,
        &fingerprint.revision_id,
        &fingerprint.snapshot_id,
        &ref_name,
        &head_oid,
        None,
        writer,
        current_timestamp(),
    )?;
    sign_event_if_requested(&mut event, &options.signing)?;
    // Route through the recorder so the capture's write-count envelope counts the
    // auto-recorded ref event (created or existing) and its event type.
    recorder.record(event_store, event)
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
        let event_type = event.event_type;
        match event_store.record_event_once(&event)? {
            EventWriteOutcome::Created => {
                self.events_created += 1;
                *self
                    .events_created_by_type
                    .entry(event_type.as_str().to_owned())
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

    use crate::canonical_hash::sha256_json_prefixed;
    use crate::git::git_common_dir;
    use crate::model::{
        CommitRangeCaptureMode, ReviewEndpoint, ReviewUnitLineageId, ReviewUnitSource, SnapshotId,
    };
    use crate::session::event::EventType;
    use crate::session::store::resolution::register_clone_local_store;
    use crate::session::{
        ArtifactKind, CaptureOptions, CaptureResult, CommitRangeSpec, EventStore,
        ImportArtifactOptions, ImportArtifactOutcome, LineageAttachOptions, ReviewUnitShowOptions,
        ShoreStorePaths, attach_review_unit_to_lineage, capture_review, capture_worktree_review,
        export_artifact, import_artifact, read_snapshot_artifact, referenced_artifacts,
        show_review_unit,
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
    fn worktree_capture_auto_records_ref_association() {
        let repo = committed_repo();
        repo.git(["branch", "-M", "feat/x"]);
        repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
        let head_oid = repo.rev_parse("HEAD");

        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        // The write-count envelope counts the auto-recorded ref event, not only the
        // capture event.
        assert_eq!(capture.events_created, 2);
        assert_eq!(capture.events_existing, 0);
        assert_eq!(capture.events_created_by_type["review_unit_captured"], 1);
        assert_eq!(
            capture.events_created_by_type["review_unit_ref_associated"],
            1
        );

        let events = EventStore::open(repo.path().join(".shore/data"))
            .list_events()
            .unwrap();
        let projection =
            crate::session::ReviewUnitCommitRangeProjection::from_events(&events).unwrap();
        let view = projection.unit(&capture.review_unit_id).unwrap();
        assert_eq!(view.current_refs.len(), 1);
        assert_eq!(view.current_refs[0].ref_name, "refs/heads/feat/x");
        assert_eq!(view.current_refs[0].head_oid, head_oid);
    }

    #[test]
    fn range_capture_tipping_at_head_records_capture_branch_ref() {
        let repo = committed_repo();
        repo.git(["branch", "-M", "feat/range"]);
        let head_oid = repo.rev_parse("HEAD");

        // A `--base <base>` range with no explicit target tips at HEAD, so it
        // records its capture-branch ref like a worktree capture does.
        let capture = capture_review(
            CaptureOptions::new(repo.path()).with_commit_range(CommitRangeSpec::new("HEAD~1")),
        )
        .unwrap();

        let events = EventStore::open(repo.path().join(".shore/data"))
            .list_events()
            .unwrap();
        let ref_count = events
            .iter()
            .filter(|event| event.event_type == EventType::ReviewUnitRefAssociated)
            .count();
        assert_eq!(
            ref_count, 1,
            "a HEAD-tipping range records its capture-branch ref"
        );

        let projection =
            crate::session::ReviewUnitCommitRangeProjection::from_events(&events).unwrap();
        let view = projection.unit(&capture.review_unit_id).unwrap();
        assert_eq!(view.current_refs.len(), 1);
        assert_eq!(view.current_refs[0].ref_name, "refs/heads/feat/range");
        assert_eq!(view.current_refs[0].head_oid, head_oid);
    }

    #[test]
    fn range_capture_not_tipping_at_head_records_nothing() {
        let repo = committed_repo();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
        repo.commit_all("third");

        // An explicit target older than HEAD: the range does not tip at the
        // checked-out branch, so it records no capture-branch ref.
        capture_review(
            CaptureOptions::new(repo.path())
                .with_commit_range(CommitRangeSpec::new("HEAD~2").with_target_rev("HEAD~1")),
        )
        .unwrap();

        let events = EventStore::open(repo.path().join(".shore/data"))
            .list_events()
            .unwrap();
        assert!(
            !events
                .iter()
                .any(|event| event.event_type == EventType::ReviewUnitRefAssociated),
            "an arbitrary range (target != HEAD) records no capture-branch ref"
        );
    }

    #[test]
    fn range_capture_on_detached_head_records_nothing() {
        let repo = committed_repo();
        repo.git(["checkout", "--detach"]);

        // The range still tips at HEAD, but a detached HEAD names no ref to record.
        capture_review(
            CaptureOptions::new(repo.path()).with_commit_range(CommitRangeSpec::new("HEAD~1")),
        )
        .unwrap();

        let events = EventStore::open(repo.path().join(".shore/data"))
            .list_events()
            .unwrap();
        assert!(
            !events
                .iter()
                .any(|event| event.event_type == EventType::ReviewUnitRefAssociated),
            "a detached HEAD names no ref to record, even when the range tips at HEAD"
        );
    }

    #[test]
    fn detached_head_capture_skips_ref_association_silently() {
        let repo = committed_repo();
        repo.git(["checkout", "--detach"]);
        repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");

        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let events = EventStore::open(repo.path().join(".shore/data"))
            .list_events()
            .unwrap();
        assert!(
            !events
                .iter()
                .any(|event| event.event_type == EventType::ReviewUnitRefAssociated),
            "a detached HEAD names no ref to associate"
        );
        assert!(
            !capture
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "ref_association_auto_record_skipped"),
            "a detached HEAD is a clean skip, not a failure"
        );
    }

    #[cfg(unix)]
    #[test]
    fn auto_record_failure_never_blocks_capture() {
        use std::os::unix::fs::PermissionsExt;

        let repo = committed_repo();
        repo.git(["branch", "-M", "main"]);
        repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
        let first = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        // A new branch at the same commit means the same review unit (so the
        // capture event is idempotent) but a new ref-association write. Make the
        // events dir read-only so only that write fails; reads still work.
        repo.git(["checkout", "-b", "other"]);
        let events_dir = repo.path().join(".shore/data/events");
        let original = fs::metadata(&events_dir).unwrap().permissions();
        fs::set_permissions(&events_dir, fs::Permissions::from_mode(0o555)).unwrap();

        let again = capture_worktree_review(CaptureOptions::new(repo.path()));

        // Restore writability so the tempdir can clean up regardless of outcome.
        fs::set_permissions(&events_dir, original).unwrap();

        let again = again.expect("capture still succeeds when the auto-record fails");
        assert_eq!(again.review_unit_id, first.review_unit_id);
        assert!(
            again
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "ref_association_auto_record_skipped"),
            "a failed auto-record degrades to a diagnostic, never an error"
        );
    }

    #[test]
    fn capture_review_from_commit_range_binds_snapshot_artifact() {
        let repo = committed_repo();

        let result = capture_review(
            CaptureOptions::new(repo.path()).with_commit_range(CommitRangeSpec::new("HEAD~1")),
        )
        .unwrap();
        let artifact = read_snapshot_artifact(repo.path(), &result.snapshot_id).unwrap();

        // The snapshot-scoped v2 artifact no longer carries source/base/target;
        // those live on the CaptureResult/event. The artifact binds via its
        // content hash (INV-3).
        assert!(matches!(
            result.source,
            ReviewUnitSource::GitCommitRange { .. }
        ));
        assert!(matches!(result.base, ReviewEndpoint::GitCommit { .. }));
        assert!(matches!(result.target, ReviewEndpoint::GitCommit { .. }));
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
        // idempotency keys and reports both existing events (capture + ref).
        assert_eq!(first.review_unit_id, second.review_unit_id);
        assert_eq!(first.snapshot_id, second.snapshot_id);
        assert_eq!(second.events_existing, 2);
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
        // The capture event plus the auto-recorded HEAD-tipping ref association.
        assert_eq!(second.events_existing, 2);
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
        // The capture event plus the auto-recorded HEAD-tipping ref association.
        assert_eq!(second.events_existing, 2);
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
        // The capture event plus the auto-recorded HEAD-tipping ref association.
        assert_eq!(second.events_existing, 2);
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

        let events = EventStore::open(repo.path().join(".shore/data"))
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

        let events = EventStore::open(repo.path().join(".shore/data"))
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

        assert!(repo.path().join(".shore/data/events").is_dir());
        assert!(repo.path().join(".shore/data/state.json").is_file());
        // The artifact binds via its content hash, not an embedded review_unit_id.
        assert_eq!(artifact.content_hash, result.snapshot_artifact_content_hash);
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

        let events = EventStore::open(repo.path().join(".shore/data"))
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

        let events = EventStore::open(repo.path().join(".shore/data"))
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
        let event_store = EventStore::open(repo.path().join(".shore/data"));
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
        let temp_path = repo
            .path()
            .join(".shore/data/events/.shore-write.inflight.tmp");

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

        assert!(repo.path().join(".shore/data/events").is_dir());
        assert!(
            result
                .review_unit_id
                .as_str()
                .starts_with("review-unit:sha256:")
        );
    }

    #[test]
    fn linked_capture_event_lands_in_clone_local_store() {
        let fixture = LinkedCapture::new();

        capture_review(CaptureOptions::new(&fixture.linked_path)).unwrap();

        // The event is in the CLONE-LOCAL store, not stranded in worktree-local.
        let clone_local = fixture.clone_local_store_dir();
        let captured: Vec<_> = EventStore::open(&clone_local)
            .list_events()
            .unwrap()
            .into_iter()
            .filter(|event| event.event_type == EventType::ReviewUnitCaptured)
            .collect();
        assert_eq!(captured.len(), 1);

        // Worktree-local .shore/data did NOT receive the capture event.
        let worktree_local = ShoreStorePaths::resolve(&fixture.linked_path).unwrap();
        let local: Vec<_> = EventStore::open(worktree_local.store_dir())
            .list_events()
            .unwrap_or_default()
            .into_iter()
            .filter(|event| event.event_type == EventType::ReviewUnitCaptured)
            .collect();
        assert!(local.is_empty());
    }

    #[test]
    fn linked_capture_does_not_emit_batch_only_diagnostic() {
        let fixture = LinkedCapture::new();

        let result = capture_review(CaptureOptions::new(&fixture.linked_path)).unwrap();

        assert!(
            !result
                .diagnostics
                .iter()
                .any(|d| d.code == "clone_local_capture_batch_only"),
            "write-through capture is no longer batch-only"
        );
    }

    #[test]
    fn unlinked_capture_still_lands_worktree_local() {
        // Escape hatch (INV-7): unchanged behavior.
        let repo = modified_repo();
        capture_review(CaptureOptions::new(repo.path())).unwrap();
        let store = ShoreStorePaths::resolve(repo.path()).unwrap();
        assert!(
            !EventStore::open(store.store_dir())
                .list_events()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn two_linked_worktrees_capture_same_range_into_shared_store() {
        // The #146 marquee repro: two linked worktrees capture the SAME commit
        // range into ONE shared clone-local store. Before the fix the second
        // capture errored `snapshot artifact conflict`; now both succeed, sharing
        // one byte-identical artifact and two distinct capture events.
        let fixture = SharedRangeCapture::new();

        let a = capture_review(
            CaptureOptions::new(&fixture.worktree_a)
                .with_commit_range(CommitRangeSpec::new("HEAD~1")),
        )
        .unwrap();
        let b = capture_review(
            CaptureOptions::new(&fixture.worktree_b)
                .with_commit_range(CommitRangeSpec::new("HEAD~1")),
        )
        .unwrap();

        assert_eq!(a.snapshot_id, b.snapshot_id);
        assert_ne!(a.review_unit_id, b.review_unit_id); // identity namespace unchanged
        assert_eq!(
            a.snapshot_artifact_content_hash,
            b.snapshot_artifact_content_hash
        );

        // One shared artifact, two capture events in the clone-local store.
        let clone_local = fixture.clone_local_store_dir();
        let captured: Vec<_> = EventStore::open(&clone_local)
            .list_events()
            .unwrap()
            .into_iter()
            .filter(|event| event.event_type == EventType::ReviewUnitCaptured)
            .collect();
        assert_eq!(captured.len(), 2);
        let snapshot_files = fs::read_dir(clone_local.join("artifacts/snapshots"))
            .unwrap()
            .count();
        assert_eq!(
            snapshot_files, 1,
            "the two captures dedup to one shared artifact"
        );

        // Both ReviewUnits resolve + render their snapshot through the binding.
        for review_unit_id in [&a.review_unit_id, &b.review_unit_id] {
            let shown = show_review_unit(
                ReviewUnitShowOptions::new(&fixture.worktree_a)
                    .with_review_unit_id(review_unit_id.clone()),
            )
            .unwrap();
            assert!(!shown.snapshot.files.is_empty());
        }
    }

    #[test]
    fn linked_capture_dedups_against_pre_existing_v1_and_both_units_render() {
        // The dual-read mixed case end to end: worktree A is captured by an "old"
        // binary (v1 artifact + v1-bound event), then worktree B captures the same
        // range with the v2 binary. B dedups against the untouched v1 artifact, A's
        // signed-shaped v1 binding stays valid, and both units render.
        let fixture = SharedRangeCapture::new();
        let a = capture_review(
            CaptureOptions::new(&fixture.worktree_a)
                .with_commit_range(CommitRangeSpec::new("HEAD~1")),
        )
        .unwrap();
        let clone_local = fixture.clone_local_store_dir();
        let v1_hash = downgrade_capture_to_v1(&clone_local, &a.snapshot_id, &a);
        assert_ne!(
            v1_hash, a.snapshot_artifact_content_hash,
            "the v1 body hash differs from the original v2 hash"
        );

        // Worktree B captures the same range: write dedups against the v1 body.
        let b = capture_review(
            CaptureOptions::new(&fixture.worktree_b)
                .with_commit_range(CommitRangeSpec::new("HEAD~1")),
        )
        .unwrap();
        assert_eq!(a.snapshot_id, b.snapshot_id);
        assert_ne!(a.review_unit_id, b.review_unit_id);
        // B bound to the on-disk (v1) hash it deduped against.
        assert_eq!(b.snapshot_artifact_content_hash, v1_hash);

        // The shared artifact is still the untouched v1 body, and both units render.
        let stored = read_snapshot_artifact(&fixture.worktree_a, &a.snapshot_id).unwrap();
        assert_eq!(stored.version, 1);
        let snapshot_files = fs::read_dir(clone_local.join("artifacts/snapshots"))
            .unwrap()
            .count();
        assert_eq!(snapshot_files, 1);
        for review_unit_id in [&a.review_unit_id, &b.review_unit_id] {
            let shown = show_review_unit(
                ReviewUnitShowOptions::new(&fixture.worktree_b)
                    .with_review_unit_id(review_unit_id.clone()),
            )
            .unwrap();
            assert!(!shown.snapshot.files.is_empty());
        }
    }

    #[test]
    fn independent_worktree_snapshot_artifacts_dedup_on_import() {
        // Two independent stores capture the same range (byte-identical v2
        // artifacts). Importing one's artifact into the other is a no-op Existing,
        // not a conflict (INV-5).
        let repo_a = committed_repo();
        let repo_b = clone_repo(&repo_a);
        let a = capture_review(
            CaptureOptions::new(repo_a.path()).with_commit_range(CommitRangeSpec::new("HEAD~1")),
        )
        .unwrap();
        let _b = capture_review(
            CaptureOptions::new(repo_b.path()).with_commit_range(CommitRangeSpec::new("HEAD~1")),
        )
        .unwrap();

        let events_a = EventStore::open(repo_a.path().join(".shore/data"))
            .list_events()
            .unwrap();
        let refs = referenced_artifacts(&events_a).unwrap();
        let snap_ref = refs
            .iter()
            .find(|artifact| artifact.kind() == ArtifactKind::Snapshot)
            .expect("snapshot artifact ref");
        let bytes = export_artifact(repo_a.path(), snap_ref).unwrap();
        let outcome = import_artifact(ImportArtifactOptions::new(
            repo_b.path(),
            snap_ref.clone(),
            bytes,
        ))
        .unwrap();
        assert_eq!(outcome.outcome, ImportArtifactOutcome::Existing);
        assert_eq!(a.snapshot_id, _b.snapshot_id);
    }

    /// A main clone plus two linked worktrees both detached at the same commit, so
    /// a `--base HEAD~1` capture from each resolves the identical commit range
    /// (same `snapshot_id`) under a distinct worktree root (distinct
    /// `review_unit_id`). Both share the clone-local `.git/shore` store.
    struct SharedRangeCapture {
        _main: TestRepo,
        _parent: tempfile::TempDir,
        worktree_a: std::path::PathBuf,
        worktree_b: std::path::PathBuf,
    }

    impl SharedRangeCapture {
        fn new() -> Self {
            let main = TestRepo::new();
            main.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
            main.commit_all("base");
            main.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
            main.commit_all("change"); // HEAD = change, HEAD~1 = base

            let parent = tempfile::tempdir().expect("create worktree parent");
            let worktree_a = parent.path().join("wt-a");
            let worktree_b = parent.path().join("wt-b");
            main.git([
                "worktree",
                "add",
                "--detach",
                worktree_a.to_str().unwrap(),
                "HEAD",
            ]);
            main.git([
                "worktree",
                "add",
                "--detach",
                worktree_b.to_str().unwrap(),
                "HEAD",
            ]);
            register_clone_local_store(&worktree_a).unwrap();
            register_clone_local_store(&worktree_b).unwrap();

            Self {
                _main: main,
                _parent: parent,
                worktree_a,
                worktree_b,
            }
        }

        fn clone_local_store_dir(&self) -> std::path::PathBuf {
            git_common_dir(&self.worktree_a).unwrap().join("shore")
        }
    }

    /// Rewrite a landed capture into a faithful v1 store: downgrade the snapshot
    /// artifact to a full v1 body (re-adding the identity/endpoint fields and a v1
    /// `contentHash`) and re-point its capture event's `snapshotArtifactContentHash`
    /// + `payloadHash` to the v1 hash, so the store looks as an old binary wrote it.
    fn downgrade_capture_to_v1(
        store_dir: &Path,
        snapshot_id: &SnapshotId,
        capture: &CaptureResult,
    ) -> String {
        let artifact_path = find_store_file(&store_dir.join("artifacts/snapshots"), |json| {
            json["snapshot"]["snapshot_id"] == snapshot_id.as_str()
        });
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&artifact_path).unwrap()).unwrap();
        let object = value.as_object_mut().unwrap();
        object.insert("version".to_owned(), serde_json::json!(1));
        object.insert(
            "reviewUnitId".to_owned(),
            serde_json::json!(capture.review_unit_id.as_str()),
        );
        object.insert(
            "source".to_owned(),
            serde_json::json!({ "kind": "git_working_tree" }),
        );
        object.insert(
            "base".to_owned(),
            serde_json::json!({ "kind": "git_working_tree", "worktreeRoot": "/legacy" }),
        );
        object.insert(
            "target".to_owned(),
            serde_json::json!({ "kind": "git_working_tree", "worktreeRoot": "/legacy" }),
        );
        object.remove("contentHash");
        let v1_hash = sha256_json_prefixed(&value).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("contentHash".to_owned(), serde_json::json!(v1_hash.clone()));
        fs::write(&artifact_path, serde_json::to_vec(&value).unwrap()).unwrap();

        let event_path = find_store_file(&store_dir.join("events"), |json| {
            json["eventType"] == "review_unit_captured"
                && json["payload"]["reviewUnitId"] == capture.review_unit_id.as_str()
        });
        let mut event: serde_json::Value =
            serde_json::from_slice(&fs::read(&event_path).unwrap()).unwrap();
        event["payload"]["snapshotArtifactContentHash"] = serde_json::json!(v1_hash);
        event["payloadHash"] = serde_json::json!(sha256_json_prefixed(&event["payload"]).unwrap());
        fs::write(&event_path, serde_json::to_vec(&event).unwrap()).unwrap();

        v1_hash
    }

    fn find_store_file(
        dir: &Path,
        matches: impl Fn(&serde_json::Value) -> bool,
    ) -> std::path::PathBuf {
        fs::read_dir(dir)
            .expect("read store directory")
            .map(|entry| entry.expect("read dir entry").path())
            .find(|path| {
                fs::read(path)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                    .is_some_and(|json| matches(&json))
            })
            .expect("matching store file")
    }

    fn clone_repo(source: &TestRepo) -> TestRepo {
        let root = tempfile::tempdir().expect("create clone temp directory");
        let status = Command::new("git")
            .args(["clone", "--quiet"])
            .arg(source.path())
            .arg(root.path())
            .status()
            .expect("run git clone");
        assert!(status.success(), "git clone failed");
        let clone = TestRepo { root };
        clone.git(["config", "user.name", "Shore Tests"]);
        clone.git(["config", "user.email", "shore-tests@example.com"]);
        clone.git(["config", "commit.gpgsign", "false"]);
        clone
    }

    /// A main repo plus a linked worktree carrying a tracked change ready to
    /// capture, registered against the clone-local store. Mirrors the
    /// `LinkedWorktreeFixture` in `resolution.rs` tests, specialized for capture.
    struct LinkedCapture {
        _main: TestRepo,
        _linked_parent: tempfile::TempDir,
        linked_path: std::path::PathBuf,
    }

    impl LinkedCapture {
        fn new() -> Self {
            let main = TestRepo::new();
            main.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
            main.commit_all("base");

            let linked_parent = tempfile::tempdir().expect("create linked worktree parent");
            let linked_path = linked_parent.path().join("linked");
            main.git([
                "worktree",
                "add",
                "-b",
                "linked",
                linked_path.to_str().unwrap(),
            ]);
            // A tracked change in the linked worktree gives capture something to record.
            fs::write(
                linked_path.join("src/lib.rs"),
                "pub fn value() -> u32 { 2 }\n",
            )
            .unwrap();
            register_clone_local_store(&linked_path).unwrap();

            Self {
                _main: main,
                _linked_parent: linked_parent,
                linked_path,
            }
        }

        fn clone_local_store_dir(&self) -> std::path::PathBuf {
            git_common_dir(&self.linked_path).unwrap().join("shore")
        }
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
