use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::crypto::EventSigner;
use crate::error::Result;
use crate::git::{
    IngestOptions, capture_commit_range_diff_files, git_commit_tree_oid, git_head_oid,
    git_head_ref, git_rev_parse_commit_oid, ingest_tracked_diff_with_options,
};
use crate::model::{
    ActorId, DiffFile, DiffSnapshot, EngagementId, EngagementType, JournalId, ObjectId,
    ReviewEndpoint, ReviewId, ReviewTargetRef, RevisionId, RevisionSource, TargetRef,
};
use crate::session::event::{
    EventTarget, EventType, Revision, ShoreEvent, WorkObjectProposal, WorkObjectProposedPayload,
};
use crate::session::fingerprint::{
    ResolvedCommitEndpoint, RevisionFingerprint, engagement_id_from_root, engagement_id_provisional,
};
use crate::session::store::resolution::{prepare_write_landing, resolve_write_store};
use crate::session::workflow::util::sorted_unique;
use crate::session::{
    BestEffortSkipSink, EventSigningOptions, EventStore, EventWriteOutcome, ProjectionDiagnostic,
    SessionState, current_timestamp, sign_event_if_requested, writer_from_options,
};
use crate::storage::{Durability, LocalStorage};

/// Commit-range capture input: a base rev and an optional target rev (defaults
/// to `HEAD`). Revs are resolved to commit OIDs at capture time; the spellings
/// are never stored, so equivalent spellings (`HEAD~1` vs the resolved OID)
/// capture the same Revision.
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
    supersedes: Vec<RevisionId>,
    signing: EventSigningOptions,
}

impl CaptureOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            source: CaptureSourceSpec::Worktree,
            excluded_helper_paths: Vec::new(),
            actor_id: None,
            supersedes: Vec::new(),
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

    /// Attribute the captured `revision_captured` event to an explicit
    /// actor, overriding the `SHORE_ACTOR_ID` env var and the local Git
    /// identity. A malformed id is ignored (falls back to env, then Git);
    /// `None` keeps the default resolution. The Revision id is derived from
    /// snapshot content, so the override changes attribution only, not identity.
    pub fn with_actor_id(mut self, actor_id: ActorId) -> Self {
        self.actor_id = Some(actor_id);
        self
    }

    /// Record this capture as superseding one or more earlier revisions (an
    /// evolution forward-pointer). The ids are sorted and deduped before hashing,
    /// so equivalent sets converge to one payload; an empty set (the default)
    /// leaves a root capture's payload unchanged. Supersession references a
    /// revision position, never its content object, and never gates the write — a
    /// not-yet-present target is resolved later by the read projections.
    pub fn with_supersedes(mut self, supersedes: Vec<RevisionId>) -> Self {
        self.supersedes = supersedes;
        self
    }

    /// Excludes an explicit command-helper path from the captured snapshot.
    ///
    /// This is intentionally narrow CLI plumbing for files such as `--log-file`.
    /// Other untracked agent/producer files remain part of the Revision unless the
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
    pub journal_id: JournalId,
    pub revision_id: RevisionId,
    pub object_id: ObjectId,
    pub engagement_id: EngagementId,
    pub source: RevisionSource,
    pub base: ReviewEndpoint,
    pub target: ReviewEndpoint,
    pub object_artifact_content_hash: String,
    pub events_created: usize,
    pub events_existing: usize,
    pub events_created_by_type: BTreeMap<String, usize>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

/// Canonical capture entry point. Dispatches on the options' source spec to a
/// source adapter (worktree by default, or a commit range), then runs the
/// shared tail: write the object artifact, record the idempotent
/// `revision_captured` event, rebuild projection state, and surface the
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
    let journal_id = JournalId::new("journal:default");
    let snapshot = DiffSnapshot::new(review_id, fingerprint.object_id.clone(), files);
    let artifact = crate::session::object_artifact::write_object_artifact_to(
        write_store.backend(),
        &fingerprint,
        snapshot,
    )?;

    let event_store = EventStore::from_backend(write_store.backend());
    let mut recorder = CaptureRecorder::default();
    let writer = writer_from_options(&worktree_root, options.actor_id.as_ref());
    let occurred_at = current_timestamp();
    // Canonicalize the supersession set so equivalent inputs converge to one
    // payload, then derive the engagement grouping hint over the existing log: a
    // root (empty supersedes) seeds from its own revision; a present predecessor
    // lends its engagement; an all-dangling set takes a deterministic provisional
    // id. The hint never gates the write — the read projections own grouping.
    let supersedes = sorted_unique(options.supersedes.clone());
    let engagement_id = derive_engagement_id(
        &event_store.list_events()?,
        &fingerprint.revision_id,
        &supersedes,
    )?;
    // The generative move is an advisory proposal of a revision over a
    // content-only object, with its derived engagement grouping hint. The
    // subject addresses the revision through the checked review-domain
    // constructor, so a review engagement can never mint a non-review subject.
    let subject = TargetRef::Review(ReviewTargetRef::Revision {
        revision_id: fingerprint.revision_id.clone(),
    });
    let target = EventTarget::for_generative_move(
        journal_id.clone(),
        EngagementType::Review,
        subject,
        None,
    )?;
    let mut event = ShoreEvent::new(
        EventType::WorkObjectProposed,
        work_object_proposed_idempotency_key(&fingerprint.revision_id),
        target,
        writer,
        WorkObjectProposedPayload {
            engagement_id: engagement_id.clone(),
            work_object: WorkObjectProposal::Revision {
                revision: Revision {
                    id: fingerprint.revision_id.clone(),
                    object_id: fingerprint.object_id.clone(),
                    git_provenance: Some(fingerprint.git_provenance()),
                },
                object_artifact_content_hash: artifact.content_hash.clone(),
                supersedes,
            },
        },
        occurred_at,
    )?;
    sign_event_if_requested(&mut event, &options.signing)?;
    recorder.record(&event_store, event)?;

    // Record the capture-time branch ref as a best-effort `RevisionRefAssociated`
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
            &journal_id,
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
        journal_id,
        revision_id: fingerprint.revision_id,
        object_id: fingerprint.object_id,
        engagement_id,
        source: fingerprint.source,
        base: fingerprint.base,
        target: fingerprint.target,
        object_artifact_content_hash: artifact.content_hash,
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

/// Auto-record the capture-time branch ref as a ref association.
/// Returns `Ok(())` and records nothing on a detached HEAD — a ref name
/// is never fabricated. Signs with the capture signer; the caller swallows any
/// error into a diagnostic so capture never fails on this.
fn auto_record_capture_ref_association(
    worktree_root: &Path,
    event_store: &EventStore,
    recorder: &mut CaptureRecorder,
    fingerprint: &RevisionFingerprint,
    journal_id: &JournalId,
    options: &CaptureOptions,
) -> Result<()> {
    let Some(ref_name) = git_head_ref(worktree_root)? else {
        return Ok(());
    };
    let head_oid = git_head_oid(worktree_root)?;
    let writer = writer_from_options(worktree_root, options.actor_id.as_ref());
    let mut event = super::association::build_ref_association_event(
        journal_id,
        &fingerprint.revision_id,
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
    fingerprint: RevisionFingerprint,
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
        crate::session::fingerprint::revision_fingerprint_for_files(worktree_root, &files)?;
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
        capture_commit_range_diff_files(worktree_root, &base.commit_oid, &target.commit_oid, &[])?;
    let fingerprint = crate::session::fingerprint::commit_range_revision_fingerprint_for_files(
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

fn work_object_proposed_idempotency_key(revision_id: &RevisionId) -> String {
    format!("work_object_proposed:{}", revision_id.as_str())
}

/// Derive the engagement grouping hint for a generative move over the existing
/// log. A root (empty `supersedes`) seeds from its own revision; otherwise the
/// first present predecessor (the lowest superseded revision id, since the set is
/// sorted) lends its engagement so a whole thread shares one grouping; an
/// all-dangling set takes a deterministic provisional id. The hint never gates
/// the write — a dangling or cross-engagement target is reconciled later by the
/// read projections.
fn derive_engagement_id(
    events: &[ShoreEvent],
    revision_id: &RevisionId,
    supersedes: &[RevisionId],
) -> Result<EngagementId> {
    if supersedes.is_empty() {
        return Ok(engagement_id_from_root(revision_id));
    }
    let hints = revision_engagement_hints(events)?;
    for predecessor in supersedes {
        if let Some(engagement_id) = hints.get(predecessor) {
            return Ok(engagement_id.clone());
        }
    }
    Ok(engagement_id_provisional(supersedes))
}

/// Map each captured revision to its stored engagement hint, discriminating the
/// generative arm: only a review-domain revision carries an engagement to
/// inherit; a task-attempt proposal in a mixed log is skipped, never decoded as
/// a revision.
fn revision_engagement_hints(events: &[ShoreEvent]) -> Result<BTreeMap<RevisionId, EngagementId>> {
    let mut hints = BTreeMap::new();
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::WorkObjectProposed)
    {
        let payload: WorkObjectProposedPayload = serde_json::from_value(event.payload.clone())?;
        if let WorkObjectProposal::Revision { revision, .. } = payload.work_object {
            hints.insert(revision.id, payload.engagement_id);
        }
    }
    Ok(hints)
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

    use crate::git::git_common_dir;
    use crate::model::{CommitRangeCaptureMode, ReviewEndpoint, RevisionSource};
    use crate::session::event::EventType;
    use crate::session::store::content::ContentArtifacts;
    use crate::session::{
        ArtifactKind, CaptureOptions, CommitRangeSpec, EventStore, ImportArtifactOptions,
        ImportArtifactOutcome, RevisionShowOptions, ShoreStorePaths, capture_review,
        capture_worktree_review, export_artifact, import_artifact, read_object_artifact,
        referenced_artifacts, show_revision,
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
            RevisionSource::GitCommitRange {
                mode: CommitRangeCaptureMode::BaseTreeToTargetTree,
                ..
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
        assert!(result.revision_id.as_str().starts_with("rev:sha256:"));
        assert_eq!(result.events_created_by_type["work_object_proposed"], 1);
    }

    #[test]
    fn native_capture_mints_a_journal_prefixed_container_id() {
        // The dominant container value in the real review stores comes from the
        // native capture path; it mints the `journal:`-prefixed default container.
        let repo = committed_repo();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
        let result = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        assert!(
            result.journal_id.as_str().starts_with("journal:"),
            "container id must carry the journal: prefix, got {}",
            result.journal_id.as_str()
        );
    }

    #[test]
    fn capture_with_supersedes_inherits_the_engagement_and_supersedes_the_predecessor() {
        let repo = committed_repo();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
        let root = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        repo.write("src/lib.rs", "pub fn value() -> u32 { 4 }\n");
        let successor = capture_worktree_review(
            CaptureOptions::new(repo.path())
                // A duplicated predecessor is deduped before hashing.
                .with_supersedes(vec![root.revision_id.clone(), root.revision_id.clone()]),
        )
        .unwrap();

        // A present predecessor lends its engagement, so the thread shares one
        // grouping; the successor's own revision id is distinct.
        assert_eq!(successor.engagement_id, root.engagement_id);
        assert_ne!(successor.revision_id, root.revision_id);

        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();

        // The stored generative move carries the deduped supersession pointer.
        let stored = events
            .iter()
            .find_map(|event| {
                let payload: crate::session::event::WorkObjectProposedPayload =
                    serde_json::from_value(event.payload.clone()).ok()?;
                match payload.work_object {
                    crate::session::event::WorkObjectProposal::Revision {
                        revision,
                        supersedes,
                        ..
                    } if revision.id == successor.revision_id => Some(supersedes),
                    _ => None,
                }
            })
            .expect("successor generative move present");
        assert_eq!(stored, vec![root.revision_id.clone()]);

        // The supersession projection resolves the predecessor as superseded and
        // the successor as the lone head.
        let view = crate::session::SupersessionView::from_events(&events).unwrap();
        assert_eq!(
            view.heads,
            [successor.revision_id.clone()].into_iter().collect()
        );
        assert_eq!(
            view.superseded,
            [root.revision_id.clone()].into_iter().collect()
        );
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
        assert_eq!(capture.events_created_by_type["work_object_proposed"], 1);
        assert_eq!(capture.events_created_by_type["revision_ref_associated"], 1);

        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let projection =
            crate::session::RevisionCommitRangeProjection::from_events(&events).unwrap();
        let view = projection.unit(&capture.revision_id).unwrap();
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

        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let ref_count = events
            .iter()
            .filter(|event| event.event_type == EventType::RevisionRefAssociated)
            .count();
        assert_eq!(
            ref_count, 1,
            "a HEAD-tipping range records its capture-branch ref"
        );

        let projection =
            crate::session::RevisionCommitRangeProjection::from_events(&events).unwrap();
        let view = projection.unit(&capture.revision_id).unwrap();
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

        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        assert!(
            !events
                .iter()
                .any(|event| event.event_type == EventType::RevisionRefAssociated),
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

        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        assert!(
            !events
                .iter()
                .any(|event| event.event_type == EventType::RevisionRefAssociated),
            "a detached HEAD names no ref to record, even when the range tips at HEAD"
        );
    }

    #[test]
    fn detached_head_capture_skips_ref_association_silently() {
        let repo = committed_repo();
        repo.git(["checkout", "--detach"]);
        repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");

        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        assert!(
            !events
                .iter()
                .any(|event| event.event_type == EventType::RevisionRefAssociated),
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
        let events_dir = resolved_store_dir(repo.path()).join("events");
        let original = fs::metadata(&events_dir).unwrap().permissions();
        fs::set_permissions(&events_dir, fs::Permissions::from_mode(0o555)).unwrap();

        let again = capture_worktree_review(CaptureOptions::new(repo.path()));

        // Restore writability so the tempdir can clean up regardless of outcome.
        fs::set_permissions(&events_dir, original).unwrap();

        let again = again.expect("capture still succeeds when the auto-record fails");
        assert_eq!(again.revision_id, first.revision_id);
        assert!(
            again
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "ref_association_auto_record_skipped"),
            "a failed auto-record degrades to a diagnostic, never an error"
        );
    }

    #[test]
    fn capture_review_from_commit_range_binds_object_artifact() {
        let repo = committed_repo();

        let result = capture_review(
            CaptureOptions::new(repo.path()).with_commit_range(CommitRangeSpec::new("HEAD~1")),
        )
        .unwrap();
        let artifact = read_object_artifact(repo.path(), &result.object_id).unwrap();

        // The object-scoped v2 artifact no longer carries source/base/target;
        // those live on the CaptureResult/event. The artifact binds via its
        // content hash (INV-3).
        assert!(matches!(
            result.source,
            RevisionSource::GitCommitRange { .. }
        ));
        assert!(matches!(result.base, ReviewEndpoint::GitCommit { .. }));
        assert!(matches!(result.target, ReviewEndpoint::GitCommit { .. }));
        assert_eq!(artifact.content_hash, result.object_artifact_content_hash);
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
        assert_eq!(first.revision_id, second.revision_id);
        assert_eq!(first.object_id, second.object_id);
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

        assert_eq!(first.revision_id, second.revision_id);
        assert_eq!(first.object_id, second.object_id);
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

        assert_eq!(first.revision_id, second.revision_id);
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
        let artifact = read_object_artifact(repo.path(), &result.object_id).unwrap();

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
        let artifact = read_object_artifact(repo.path(), &first.object_id).unwrap();
        assert!(artifact.snapshot.files.is_empty());

        let second = capture_review(
            CaptureOptions::new(repo.path()).with_commit_range(CommitRangeSpec::new("HEAD")),
        )
        .unwrap();
        assert_eq!(first.revision_id, second.revision_id);
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
        // tree and range-capture it: the content is identical, so the content-only
        // object id converges (one shared artifact, no conflict), while the differing
        // provenance (worktree path vs commit pair) mints two distinct revisions.
        let worktree = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        repo.commit_all("change");
        let range = capture_review(
            CaptureOptions::new(repo.path()).with_commit_range(CommitRangeSpec::new("HEAD~1")),
        )
        .unwrap();

        assert_eq!(worktree.object_id, range.object_id);
        assert_ne!(worktree.revision_id, range.revision_id);
        // The shared artifact is independently readable under either capture: the
        // converged object id never conflicted on write.
        read_object_artifact(repo.path(), &worktree.object_id).unwrap();
        read_object_artifact(repo.path(), &range.object_id).unwrap();

        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let captured = events
            .iter()
            .filter(|event| event.event_type == EventType::WorkObjectProposed)
            .count();
        assert_eq!(captured, 2);
    }

    #[test]
    fn capture_worktree_review_writes_event_artifact_and_state() {
        let repo = modified_repo();

        let result = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let artifact = read_object_artifact(repo.path(), &result.object_id).unwrap();

        assert!(resolved_store_dir(repo.path()).join("events").is_dir());
        assert!(resolved_store_dir(repo.path()).join("state.json").is_file());
        // The artifact binds via its content hash, not an embedded revision_id.
        assert_eq!(artifact.content_hash, result.object_artifact_content_hash);
        assert!(result.revision_id.as_str().starts_with("rev:sha256:"));
        assert_eq!(result.events_created_by_type["work_object_proposed"], 1);
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

        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let event = events
            .iter()
            .find(|event| event.event_type == EventType::WorkObjectProposed)
            .unwrap();

        // Attribution changes; the Revision id is derived from snapshot content, not the writer.
        assert_eq!(event.writer.actor_id.as_str(), "actor:agent:capturer");
        assert!(result.revision_id.as_str().starts_with("rev:sha256:"));
    }

    #[test]
    fn capture_worktree_review_without_actor_id_uses_git_identity() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let event = events
            .iter()
            .find(|event| event.event_type == EventType::WorkObjectProposed)
            .unwrap();
        assert_eq!(
            event.writer.actor_id.as_str(),
            "actor:git-email:shore-tests@example.com"
        );
    }

    #[test]
    fn capture_worktree_review_binds_event_to_object_artifact_hash() {
        let repo = modified_repo();

        let result = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let artifact = read_object_artifact(repo.path(), &result.object_id).unwrap();
        let event_store = EventStore::open(resolved_store_dir(repo.path()));
        let events = event_store.list_events().unwrap();
        let event = events
            .iter()
            .find(|event| event.event_type == EventType::WorkObjectProposed)
            .unwrap();

        assert_eq!(result.object_artifact_content_hash, artifact.content_hash);
        assert_eq!(
            event.payload["workObject"]["objectArtifactContentHash"],
            artifact.content_hash
        );
    }

    #[test]
    fn capture_worktree_review_preserves_fresh_shore_temp_files() {
        let repo = modified_repo();
        let temp_path = resolved_store_dir(repo.path()).join("events/.shore-write.inflight.tmp");

        fs::create_dir_all(temp_path.parent().unwrap()).unwrap();
        fs::write(&temp_path, b"in flight").unwrap();

        let result = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        assert_eq!(result.events_created_by_type["work_object_proposed"], 1);
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

        assert!(resolved_store_dir(repo.path()).join("events").is_dir());
        assert!(result.revision_id.as_str().starts_with("rev:sha256:"));
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
            .filter(|event| event.event_type == EventType::WorkObjectProposed)
            .collect();
        assert_eq!(captured.len(), 1);

        // Worktree-local .shore/data did NOT receive the capture event.
        let worktree_local = ShoreStorePaths::resolve(&fixture.linked_path).unwrap();
        let local: Vec<_> = EventStore::open(worktree_local.store_dir())
            .list_events()
            .unwrap_or_default()
            .into_iter()
            .filter(|event| event.event_type == EventType::WorkObjectProposed)
            .collect();
        assert!(local.is_empty());
    }

    #[test]
    fn default_capture_lands_in_shared_common_dir_store() {
        // The shared-store default: a capture in a plain (non-ephemeral) worktree
        // lands in the common-dir store, not the worktree-local .shore/data.
        let repo = modified_repo();
        capture_review(CaptureOptions::new(repo.path())).unwrap();

        let common_dir = git_common_dir(repo.path()).unwrap().join("shore");
        assert!(
            !EventStore::open(&common_dir)
                .list_events()
                .unwrap()
                .is_empty()
        );
        // The worktree-local .shore/data is NOT the resolved store anymore.
        let worktree_local = ShoreStorePaths::resolve(repo.path()).unwrap();
        let local = EventStore::open(worktree_local.store_dir())
            .list_events()
            .unwrap_or_default();
        assert!(local.is_empty());
    }

    #[test]
    fn two_linked_worktrees_capture_same_range_into_shared_store() {
        // Two linked worktrees capture the SAME commit range into ONE shared
        // clone-local store. A commit-range capture's provenance is the commit
        // pair with no working-tree path, so identical content under identical
        // provenance converges to one revision: the second capture dedups against
        // the first, leaving one capture event and one shared artifact. Before the
        // artifact-sharing fix the second capture errored `object artifact
        // conflict`; now it is a clean no-op convergence.
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

        assert_eq!(a.object_id, b.object_id);
        assert_eq!(a.revision_id, b.revision_id);
        assert_eq!(
            a.object_artifact_content_hash,
            b.object_artifact_content_hash
        );

        // One shared artifact and one converged capture event in the clone-local
        // store: the second capture deduped against the first.
        let clone_local = fixture.clone_local_store_dir();
        let captured: Vec<_> = EventStore::open(&clone_local)
            .list_events()
            .unwrap()
            .into_iter()
            .filter(|event| event.event_type == EventType::WorkObjectProposed)
            .collect();
        assert_eq!(captured.len(), 1);
        let snapshot_files = ContentArtifacts::local(&clone_local)
            .list_refs("artifacts/objects")
            .unwrap()
            .len();
        assert_eq!(
            snapshot_files, 1,
            "the two captures dedup to one shared artifact"
        );

        // The converged Revision resolves + renders its snapshot through the binding.
        for revision_id in [&a.revision_id, &b.revision_id] {
            let shown = show_revision(
                RevisionShowOptions::new(&fixture.worktree_a).with_revision_id(revision_id.clone()),
            )
            .unwrap();
            assert!(!shown.snapshot.files.is_empty());
        }
    }

    #[test]
    fn independent_worktree_object_artifacts_dedup_on_import() {
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

        let events_a = EventStore::open(resolved_store_dir(repo_a.path()))
            .list_events()
            .unwrap();
        let refs = referenced_artifacts(&events_a).unwrap();
        let snap_ref = refs
            .iter()
            .find(|artifact| artifact.kind() == ArtifactKind::Object)
            .expect("object artifact ref");
        let bytes = export_artifact(repo_a.path(), snap_ref).unwrap();
        let outcome = import_artifact(ImportArtifactOptions::new(
            repo_b.path(),
            snap_ref.clone(),
            bytes,
        ))
        .unwrap();
        assert_eq!(outcome.outcome, ImportArtifactOutcome::Existing);
        assert_eq!(a.object_id, _b.object_id);
    }

    /// A main clone plus two linked worktrees both detached at the same commit, so
    /// a `--base HEAD~1` capture from each resolves the identical commit range. A
    /// commit-range capture's provenance is the commit pair (no working-tree path),
    /// so both captures converge to one `object_id` and one `revision_id`. Both
    /// share the clone-local `.git/shore` store.
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

    /// The store a workflow actually lands in for `repo` — the shared common-dir
    /// store by default. Reads that follow a capture/observation/etc. resolve
    /// here, never the raw worktree-local `.shore/data`.
    fn resolved_store_dir(repo: &Path) -> std::path::PathBuf {
        git_common_dir(repo).unwrap().join("shore")
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
