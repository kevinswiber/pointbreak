//! Content-targeted artifact removal: the write workflow.
//!
//! `remove_content` takes an ergonomic selector, resolves it to a set of
//! content-addressed `content_hash`es from the event log + git reachability, and
//! emits **one `ArtifactRemoved` event per `content_hash`**. Removal is
//! content-targeted and remove-only: it never rewrites or tombstones an existing
//! event; the payload carries only the `content_hash`; the idempotency key is
//! non-overridable (`artifact_removed:<content_hash>`), so re-removing a hash
//! converges to the first-stored fact. One blob is shared by many review units
//! across sessions, so the workflow reports the units still referencing each
//! removed hash before acting, but there is no per-unit detach — removal targets
//! content.
//!
//! Physically reclaiming the bytes is the separate local `compact`/`gc` sweep;
//! there is no un-remove (re-capturing the same content re-materializes the blob
//! while the removal fact persists in the log).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::canonical_hash::sha256_bytes_hex;
use crate::error::{Result, ShoreError};
use crate::git::{git_rev_list_range, git_rev_parse_commit_oid};
use crate::model::{ActorId, ReviewUnitId, SessionId, SnapshotId};
use crate::session::body_artifact::note_body_content_hash_from_path;
use crate::session::event::{
    ArtifactRemovedPayload, EventTarget, EventType, ReviewUnitCapturedPayload, ShoreEvent,
};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store::resolution::{prepare_write_landing, resolve_write_store};
use crate::session::{
    ArtifactRemovalProjection, CommitGraphCondition, EventSigningOptions, EventStore,
    EventWriteOutcome, OrphanReason, ReviewUnitCommitRangeProjection, current_timestamp,
    enrich_liveness, referenced_artifacts, sign_event_if_requested, writer_from_options,
};
use crate::storage::{Durability, LocalStorage, RemoveOutcome};

/// Which content a removal targets. Every variant resolves to a set of
/// `content_hash`es before any event is emitted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RemoveSelector {
    /// A single snapshot's bound artifact content hash.
    Snapshot(SnapshotId),
    /// Every content hash a review unit references.
    ReviewUnit(ReviewUnitId),
    /// Content hashes of units anchored on the commit a ref resolves to.
    Ref(String),
    /// Content hashes of units anchored on a commit in the `<a>..<b>` range.
    Range(String),
    /// Content hashes of commit-anchored units whose current commits are all
    /// orphaned (no live ref reaches them).
    Orphans,
}

/// Inputs to [`remove_content`].
#[derive(Clone)]
pub struct RemoveOptions {
    repo: PathBuf,
    selector: RemoveSelector,
    signing: EventSigningOptions,
    actor_id: Option<ActorId>,
}

impl RemoveOptions {
    pub fn new(repo: impl Into<PathBuf>, selector: RemoveSelector) -> Self {
        Self {
            repo: repo.into(),
            selector,
            signing: EventSigningOptions::default(),
            actor_id: None,
        }
    }

    /// Attribute the emitted `artifact_removed` events to an explicit actor.
    /// `None` keeps the default env/Git resolution.
    pub fn with_actor_id(mut self, actor_id: ActorId) -> Self {
        self.actor_id = Some(actor_id);
        self
    }

    pub fn sign_with<S>(mut self, signer: S) -> Self
    where
        S: crate::crypto::EventSigner + Send + Sync + 'static,
    {
        self.signing = EventSigningOptions::sign_with(signer);
        self
    }

    pub fn sign_with_best_effort<S>(
        mut self,
        signer: S,
        skip_sink: crate::session::BestEffortSkipSink,
    ) -> Self
    where
        S: crate::crypto::EventSigner + Send + Sync + 'static,
    {
        self.signing = EventSigningOptions::sign_with_best_effort(signer, skip_sink);
        self
    }
}

/// One content hash a removal targeted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemovedContent {
    /// The normalized `sha256:<hex>` content hash.
    pub content_hash: String,
    /// `false` when the `artifact_removed` fact already existed (re-removal).
    pub created: bool,
    /// Other review units that still name this hash (the shared-content report).
    pub co_referencing_units: Vec<ReviewUnitId>,
}

/// The outcome of a [`remove_content`] call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoveResult {
    pub removed: Vec<RemovedContent>,
    pub events_created: usize,
    pub events_existing: usize,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

/// Resolve the selector to a content-hash set and emit one `ArtifactRemoved`
/// event per hash through the established write path (session-anchored,
/// content-addressed, lock-free).
pub fn remove_content(options: RemoveOptions) -> Result<RemoveResult> {
    let write_store = resolve_write_store(&options.repo)?;
    let worktree_root = write_store.worktree_root().to_path_buf();
    let store_dir = write_store.store_dir().to_path_buf();
    let storage = LocalStorage::new(&store_dir);
    prepare_write_landing(&write_store, &storage)?;
    let event_store = EventStore::open(&store_dir);

    // Resolve the selector to content hashes from the event log + git
    // reachability — entirely before any event is emitted.
    let events = event_store.list_events()?;
    let index = content_hash_unit_index(&events)?;
    let resolved = resolve_selector(&options.selector, &events, &options.repo, &index)?;

    // One ArtifactRemoved per resolved hash, session-anchored and idempotent.
    let session_id = SessionId::new("session:default");
    let writer = writer_from_options(&worktree_root, options.actor_id.as_ref());

    let mut removed = Vec::new();
    let mut events_created = 0;
    let mut events_existing = 0;
    for content_hash in &resolved.content_hashes {
        let mut event = ShoreEvent::new(
            EventType::ArtifactRemoved,
            ArtifactRemovedPayload::idempotency_key(content_hash),
            EventTarget::for_artifact_removal(session_id.clone()),
            writer.clone(),
            ArtifactRemovedPayload {
                content_hash: content_hash.clone(),
            },
            current_timestamp(),
        )?;
        sign_event_if_requested(&mut event, &options.signing)?;
        let created = match event_store.record_event_once(&event)? {
            EventWriteOutcome::Created => {
                events_created += 1;
                true
            }
            EventWriteOutcome::Existing | EventWriteOutcome::ExistingDivergentSignature => {
                events_existing += 1;
                false
            }
        };
        removed.push(RemovedContent {
            content_hash: content_hash.clone(),
            created,
            co_referencing_units: co_referencing_units(
                &index,
                content_hash,
                &resolved.targeted_units,
            ),
        });
    }

    // Regenerable projection rebuild from the full log (ArtifactRemoved does not
    // change SessionState; the rebuild keeps state.json fresh for concurrent
    // writers, never as the authority).
    let state = SessionState::from_events(&event_store.list_events()?)?;
    storage.write_json_atomic(
        &store_dir.join("state.json"),
        &state,
        Durability::Projection,
    )?;

    Ok(RemoveResult {
        removed,
        events_created,
        events_existing,
        diagnostics: state.diagnostics,
    })
}

/// What a selector resolved to: the content hashes to remove and the review
/// units the selector named (used to subtract the targeted units when reporting
/// the still-referencing units of each hash).
struct ResolvedSelection {
    content_hashes: BTreeSet<String>,
    targeted_units: BTreeSet<ReviewUnitId>,
}

/// Map every referenced `content_hash` to the review units that name it, built
/// once and consumed by both selector resolution and the co-referencing report.
/// Each event's content hashes come from [`referenced_artifacts`] (the canonical
/// normalized `sha256:` extraction); the naming unit is the event's review-unit
/// target.
fn content_hash_unit_index(
    events: &[ShoreEvent],
) -> Result<BTreeMap<String, BTreeSet<ReviewUnitId>>> {
    let mut index: BTreeMap<String, BTreeSet<ReviewUnitId>> = BTreeMap::new();
    for event in events {
        let Some(unit) = event.target.review_unit_id.clone() else {
            continue;
        };
        for artifact in referenced_artifacts(std::slice::from_ref(event))? {
            index
                .entry(artifact.content_hash().to_owned())
                .or_default()
                .insert(unit.clone());
        }
    }
    Ok(index)
}

fn co_referencing_units(
    index: &BTreeMap<String, BTreeSet<ReviewUnitId>>,
    content_hash: &str,
    targeted_units: &BTreeSet<ReviewUnitId>,
) -> Vec<ReviewUnitId> {
    index
        .get(content_hash)
        .map(|units| {
            units
                .iter()
                .filter(|unit| !targeted_units.contains(*unit))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn resolve_selector(
    selector: &RemoveSelector,
    events: &[ShoreEvent],
    repo: &Path,
    index: &BTreeMap<String, BTreeSet<ReviewUnitId>>,
) -> Result<ResolvedSelection> {
    match selector {
        RemoveSelector::Snapshot(snapshot_id) => {
            let (unit, content_hash) = capture_bound_to_snapshot(events, snapshot_id)?;
            let mut content_hashes = BTreeSet::new();
            content_hashes.insert(content_hash);
            let mut targeted_units = BTreeSet::new();
            targeted_units.insert(unit);
            Ok(ResolvedSelection {
                content_hashes,
                targeted_units,
            })
        }
        RemoveSelector::ReviewUnit(review_unit_id) => {
            let mut targeted_units = BTreeSet::new();
            targeted_units.insert(review_unit_id.clone());
            Ok(selection_for_units(targeted_units, index))
        }
        RemoveSelector::Ref(reference) => {
            let oid = git_rev_parse_commit_oid(repo, reference)?;
            let oids = BTreeSet::from([oid]);
            let units = units_anchored_on_oids(events, &oids)?;
            Ok(selection_for_units(units, index))
        }
        RemoveSelector::Range(range) => {
            let oids: BTreeSet<String> = git_rev_list_range(repo, range)?.into_iter().collect();
            let units = units_anchored_on_oids(events, &oids)?;
            Ok(selection_for_units(units, index))
        }
        RemoveSelector::Orphans => {
            let units = orphaned_anchored_units(events, repo)?;
            Ok(selection_for_units(units, index))
        }
    }
}

/// The content hashes naming any of `targeted_units`, paired with those units.
fn selection_for_units(
    targeted_units: BTreeSet<ReviewUnitId>,
    index: &BTreeMap<String, BTreeSet<ReviewUnitId>>,
) -> ResolvedSelection {
    let content_hashes = index
        .iter()
        .filter(|(_, units)| units.iter().any(|unit| targeted_units.contains(unit)))
        .map(|(hash, _)| hash.clone())
        .collect();
    ResolvedSelection {
        content_hashes,
        targeted_units,
    }
}

/// The review unit and bound snapshot content hash of the capture that owns
/// `snapshot_id`.
fn capture_bound_to_snapshot(
    events: &[ShoreEvent],
    snapshot_id: &SnapshotId,
) -> Result<(ReviewUnitId, String)> {
    for event in events {
        if event.event_type != EventType::ReviewUnitCaptured {
            continue;
        }
        let payload: ReviewUnitCapturedPayload = serde_json::from_value(event.payload.clone())?;
        if &payload.snapshot_id == snapshot_id {
            return Ok((
                payload.review_unit_id,
                payload.snapshot_artifact_content_hash,
            ));
        }
    }
    Err(ShoreError::Message(format!(
        "unknown snapshot: {}",
        snapshot_id.as_str()
    )))
}

/// The units whose current commit set intersects `oids`.
fn units_anchored_on_oids(
    events: &[ShoreEvent],
    oids: &BTreeSet<String>,
) -> Result<BTreeSet<ReviewUnitId>> {
    let projection = ReviewUnitCommitRangeProjection::from_events(events)?;
    Ok(projection
        .units
        .into_iter()
        .filter(|(_, view)| {
            view.current_commits
                .iter()
                .any(|commit| oids.contains(&commit.commit_oid))
        })
        .map(|(id, _)| id)
        .collect())
}

/// The commit-anchored units whose every current commit is orphaned (no live ref
/// reaches it). A floating unit (no commit anchor) is never orphaned; a unit
/// whose reachability cannot be determined (a git error) degrades to unknown and
/// is skipped rather than removed.
fn orphaned_anchored_units(events: &[ShoreEvent], repo: &Path) -> Result<BTreeSet<ReviewUnitId>> {
    let projection = ReviewUnitCommitRangeProjection::from_events(events)?;
    let mut units = BTreeSet::new();
    for (id, view) in projection.units {
        if view.current_commits.is_empty() {
            continue;
        }
        let Ok(enrichment) = enrich_liveness(&view, repo, None) else {
            continue;
        };
        let all_orphaned = !enrichment.per_commit.is_empty()
            && enrichment
                .per_commit
                .iter()
                .all(|commit| is_orphaned(&commit.condition));
        if all_orphaned {
            units.insert(id);
        }
    }
    Ok(units)
}

/// Whether a commit is orphaned — either reason. A commit whose object was
/// reclaimed (`ObjectMissing`) is as orphaned as one no live ref reaches
/// (`Unreachable`); both mean the review's commit is gone from the live graph.
fn is_orphaned(condition: &CommitGraphCondition) -> bool {
    matches!(
        condition,
        CommitGraphCondition::Orphaned {
            reason: OrphanReason::ObjectMissing | OrphanReason::Unreachable
        }
    )
}

/// Inputs to [`compact_store`].
#[derive(Clone, Debug)]
pub struct CompactOptions {
    pub repo: PathBuf,
}

impl CompactOptions {
    pub fn new(repo: impl Into<PathBuf>) -> Self {
        Self { repo: repo.into() }
    }
}

/// What the sweep did to one on-disk blob. The public, binary-crate-visible
/// counterpart of the storage-layer removal outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SweepOutcome {
    Removed,
    Missing,
}

/// One blob the sweep attempted to reclaim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SweptBlob {
    /// The normalized `sha256:<hex>` content hash of the swept blob.
    pub content_hash: String,
    pub outcome: SweepOutcome,
}

/// The outcome of a [`compact_store`] sweep.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompactResult {
    /// The blobs the sweep attempted to delete, in deterministic order.
    pub swept: Vec<SweptBlob>,
    /// Best-effort sum of the on-disk byte sizes actually reclaimed.
    pub bytes_reclaimed: u64,
}

/// Physically delete the content-addressed blobs whose `content_hash` was marked
/// removed. A local, non-event maintenance sweep — it appends no event, rewrites
/// no `state.json`, and is fully re-derivable from the log. `gc` and `compact`
/// are the same operation; re-capturing the same content re-materializes a swept
/// blob (there is no un-remove — the removal fact persists in the log).
pub fn compact_store(options: CompactOptions) -> Result<CompactResult> {
    let write_store = resolve_write_store(&options.repo)?;
    let store_dir = write_store.store_dir().to_path_buf();
    let storage = LocalStorage::new(&store_dir);

    let events = EventStore::open(&store_dir).list_events()?;
    let removed = ArtifactRemovalProjection::from_events(&events)?;

    // A blob is swept iff its content hash is removed. A non-removed blob is never
    // enumerated into the sweep set, so a blob still named by a live, non-removed
    // event is protected by construction (the safety floor over the on-disk set).
    let mut swept = Vec::new();
    let mut bytes_reclaimed = 0u64;
    for blob in on_disk_blobs(&storage, &events)? {
        if !removed.is_removed(&blob.content_hash) {
            continue;
        }
        let byte_size = std::fs::metadata(store_dir.join(&blob.relative_path))
            .map(|meta| meta.len())
            .unwrap_or(0);
        let outcome = match storage.remove_file(&blob.relative_path)? {
            RemoveOutcome::Removed => {
                bytes_reclaimed += byte_size;
                SweepOutcome::Removed
            }
            RemoveOutcome::Missing => SweepOutcome::Missing,
        };
        swept.push(SweptBlob {
            content_hash: blob.content_hash,
            outcome,
        });
    }

    Ok(CompactResult {
        swept,
        bytes_reclaimed,
    })
}

/// A content-addressed blob on disk, paired with the `content_hash` it carries.
struct OnDiskBlob {
    /// Store-relative path under `artifacts/snapshots` or `artifacts/notes`.
    relative_path: String,
    /// The normalized `sha256:<hex>` content hash the blob carries.
    content_hash: String,
}

/// Enumerate every content-addressed blob under `artifacts/snapshots` and
/// `artifacts/notes`, mapping each on-disk file to its `content_hash`. A snapshot
/// file name is `sha256(snapshotId)` — *not* the content hash — so it is resolved
/// through the `ReviewUnitCaptured` payloads; a note file name stem *is* the
/// content-hash hex.
fn on_disk_blobs(storage: &LocalStorage, events: &[ShoreEvent]) -> Result<Vec<OnDiskBlob>> {
    let snapshot_hash_by_stem = snapshot_content_hash_by_file_stem(events)?;
    let mut blobs = Vec::new();

    for path in storage.list_dir(Path::new("artifacts/snapshots"))? {
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let stem = file_name.strip_suffix(".json").unwrap_or(file_name);
        if let Some(content_hash) = snapshot_hash_by_stem.get(stem) {
            blobs.push(OnDiskBlob {
                relative_path: format!("artifacts/snapshots/{file_name}"),
                content_hash: content_hash.clone(),
            });
        }
    }

    for path in storage.list_dir(Path::new("artifacts/notes"))? {
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let relative_path = format!("artifacts/notes/{file_name}");
        let content_hash = note_body_content_hash_from_path(&relative_path)?;
        blobs.push(OnDiskBlob {
            relative_path,
            content_hash,
        });
    }

    Ok(blobs)
}

/// Map each captured snapshot's on-disk file stem (`sha256(snapshotId)`) to its
/// artifact content hash, the join that lets the sweep recognize a snapshot blob.
fn snapshot_content_hash_by_file_stem(events: &[ShoreEvent]) -> Result<BTreeMap<String, String>> {
    let mut by_stem = BTreeMap::new();
    for event in events {
        if event.event_type != EventType::ReviewUnitCaptured {
            continue;
        }
        let payload: ReviewUnitCapturedPayload = serde_json::from_value(event.payload.clone())?;
        let stem = sha256_bytes_hex(payload.snapshot_id.as_str().as_bytes());
        by_stem.insert(stem, payload.snapshot_artifact_content_hash);
    }
    Ok(by_stem)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;
    use std::process::Command;

    use super::*;
    use crate::session::event::{
        EventTarget, EventType, ReviewUnitCapturedPayload, ShoreEvent, Writer, WriterProducer,
    };
    use crate::session::{
        ArtifactRemovalProjection, CaptureOptions, CommitRangeSpec, EventStore,
        ObservationAddOptions, capture_review, capture_worktree_review, record_observation,
    };

    /// The store a workflow actually lands in for `repo` — the shared common-dir
    /// store by default. Reads that follow a workflow resolve here, not the raw
    /// worktree-local `.shore/data`.
    fn resolved_store_dir(repo: &Path) -> std::path::PathBuf {
        crate::git::git_common_dir(repo).unwrap().join("shore")
    }

    struct TestRepo {
        root: tempfile::TempDir,
    }

    impl TestRepo {
        fn init() -> Self {
            let repo = Self {
                root: tempfile::tempdir().unwrap(),
            };
            repo.git(["init"]);
            repo.git(["config", "user.name", "Shore Tests"]);
            repo.git(["config", "user.email", "shore-tests@example.com"]);
            repo.git(["config", "commit.gpgsign", "false"]);
            repo
        }

        fn path(&self) -> &Path {
            self.root.path()
        }

        fn commit(&self, contents: &str, message: &str) -> String {
            std::fs::write(self.path().join("src.txt"), contents).unwrap();
            self.git(["add", "--all"]);
            self.git(["commit", "-m", message]);
            self.rev_parse("HEAD")
        }

        fn rev_parse(&self, rev: &str) -> String {
            let output = Command::new("git")
                .args(["rev-parse", "--verify", rev])
                .current_dir(self.path())
                .output()
                .unwrap();
            assert!(output.status.success());
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        }

        fn git<I, S>(&self, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<OsStr>,
        {
            let status = Command::new("git")
                .args(
                    args.into_iter()
                        .map(|a| a.as_ref().to_owned())
                        .collect::<Vec<_>>(),
                )
                .current_dir(self.path())
                .status()
                .unwrap();
            assert!(status.success());
        }
    }

    /// A worktree capture, leaving a working-tree change first so there is a diff.
    fn capture_worktree(repo: &TestRepo) -> crate::session::CaptureResult {
        repo.commit("base\n", "base");
        std::fs::write(repo.path().join("src.txt"), "changed\n").unwrap();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap()
    }

    fn list_events(repo: &TestRepo) -> Vec<ShoreEvent> {
        let store_dir = resolved_store_dir(repo.path());
        EventStore::open(store_dir).list_events().unwrap()
    }

    fn removed_set(repo: &TestRepo) -> ArtifactRemovalProjection {
        ArtifactRemovalProjection::from_events(&list_events(repo)).unwrap()
    }

    /// Count the blob files under a store subdir (e.g. `artifacts/snapshots`).
    fn blob_count(repo: &TestRepo, subdir: &str) -> usize {
        let dir = resolved_store_dir(repo.path()).join(subdir);
        match std::fs::read_dir(&dir) {
            Ok(entries) => entries.filter_map(std::result::Result::ok).count(),
            Err(_) => 0,
        }
    }

    /// Record a fabricated sibling `ReviewUnitCaptured` event that binds the same
    /// snapshot content hash under a different review unit — the cross-worktree
    /// coexistence case (one shared blob, two capture events). Returns the
    /// sibling unit id.
    fn fabricate_sibling_capture(
        repo: &TestRepo,
        original: &crate::session::CaptureResult,
    ) -> ReviewUnitId {
        let sibling_unit =
            ReviewUnitId::new(format!("{}-sibling", original.review_unit_id.as_str()));
        let sibling_snapshot =
            SnapshotId::new(format!("{}-sibling", original.snapshot_id.as_str()));
        let event = ShoreEvent::new(
            EventType::ReviewUnitCaptured,
            format!("review_unit_captured:{}", sibling_unit.as_str()),
            EventTarget::for_review_unit(
                original.session_id.clone(),
                sibling_unit.clone(),
                original.revision_id.clone(),
                sibling_snapshot.clone(),
            ),
            Writer {
                actor_id: ActorId::new("actor:sibling"),
                producer: WriterProducer {
                    name: "shore".to_owned(),
                    version: "test".to_owned(),
                },
            },
            ReviewUnitCapturedPayload {
                review_unit_id: sibling_unit.clone(),
                source: original.source.clone(),
                base: original.base.clone(),
                target: original.target.clone(),
                revision_id: original.revision_id.clone(),
                snapshot_id: sibling_snapshot,
                snapshot_artifact_content_hash: original.snapshot_artifact_content_hash.clone(),
            },
            "2026-06-19T00:00:00Z",
        )
        .unwrap();
        let store_dir = resolved_store_dir(repo.path());
        EventStore::open(store_dir)
            .record_event_once(&event)
            .unwrap();
        sibling_unit
    }

    #[test]
    fn remove_by_snapshot_resolves_bound_content_hash() {
        let repo = TestRepo::init();
        let capture = capture_worktree(&repo);

        let result = remove_content(RemoveOptions::new(
            repo.path(),
            RemoveSelector::Snapshot(capture.snapshot_id.clone()),
        ))
        .unwrap();

        assert_eq!(result.removed.len(), 1);
        assert_eq!(
            result.removed[0].content_hash,
            capture.snapshot_artifact_content_hash
        );
        assert!(result.removed[0].created);
        assert_eq!(result.events_created, 1);
        assert!(removed_set(&repo).is_removed(&capture.snapshot_artifact_content_hash));
    }

    #[test]
    fn remove_by_review_unit_resolves_all_referenced_hashes() {
        let repo = TestRepo::init();
        let capture = capture_worktree(&repo);
        let big_body = "x".repeat(5000);
        let observation = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_review_unit_id(capture.review_unit_id.clone())
                .with_track("agent:tester")
                .with_title("a large observation")
                .with_body(big_body),
        )
        .unwrap();
        let body_hash = observation
            .body_content_hash
            .expect("a >4096-byte body is stored as a note artifact");

        let result = remove_content(RemoveOptions::new(
            repo.path(),
            RemoveSelector::ReviewUnit(capture.review_unit_id.clone()),
        ))
        .unwrap();

        let hashes: Vec<&str> = result
            .removed
            .iter()
            .map(|r| r.content_hash.as_str())
            .collect();
        assert!(hashes.contains(&capture.snapshot_artifact_content_hash.as_str()));
        assert!(hashes.contains(&body_hash.as_str()));
        assert_eq!(result.events_created, 2);
        let removed = removed_set(&repo);
        assert!(removed.is_removed(&capture.snapshot_artifact_content_hash));
        assert!(removed.is_removed(&body_hash));
    }

    #[test]
    fn remove_by_review_unit_reports_co_referencing_units() {
        let repo = TestRepo::init();
        let capture = capture_worktree(&repo);
        let sibling = fabricate_sibling_capture(&repo, &capture);

        let result = remove_content(RemoveOptions::new(
            repo.path(),
            RemoveSelector::ReviewUnit(capture.review_unit_id.clone()),
        ))
        .unwrap();

        let entry = result
            .removed
            .iter()
            .find(|r| r.content_hash == capture.snapshot_artifact_content_hash)
            .expect("the shared snapshot hash is removed");
        assert!(
            entry.co_referencing_units.contains(&sibling),
            "the sibling unit still names the shared blob: {:?}",
            entry.co_referencing_units
        );
    }

    #[test]
    fn remove_by_orphans_resolves_unreachable_anchored_units() {
        let repo = TestRepo::init();
        let base = repo.commit("base\n", "base");
        let reachable = repo.commit("reachable\n", "reachable");
        // A unit anchored on a commit that stays reachable from the default branch.
        let reachable_capture = capture_review(CaptureOptions::new(repo.path()).with_commit_range(
            CommitRangeSpec::new(base.clone()).with_target_rev(reachable.clone()),
        ))
        .unwrap();

        // A unit anchored on a commit that will become unreachable.
        repo.git(["checkout", "-b", "doomed"]);
        let doomed = repo.commit("doomed\n", "doomed");
        let doomed_capture = capture_review(
            CaptureOptions::new(repo.path())
                .with_commit_range(CommitRangeSpec::new(reachable.clone()).with_target_rev(doomed)),
        )
        .unwrap();
        // Detach onto the reachable commit (which the default branch still points
        // at) so the doomed branch can be deleted — branch-name-agnostic, since the
        // initial branch may be `main` or `master` depending on the git default.
        repo.git(["checkout", &reachable]);
        repo.git(["branch", "-D", "doomed"]);

        let result =
            remove_content(RemoveOptions::new(repo.path(), RemoveSelector::Orphans)).unwrap();

        let hashes: Vec<&str> = result
            .removed
            .iter()
            .map(|r| r.content_hash.as_str())
            .collect();
        assert!(
            hashes.contains(&doomed_capture.snapshot_artifact_content_hash.as_str()),
            "the orphaned unit's hash is resolved"
        );
        assert!(
            !hashes.contains(&reachable_capture.snapshot_artifact_content_hash.as_str()),
            "a reachable unit's hash is not resolved"
        );
    }

    #[test]
    fn remove_by_ref_and_range_resolve_anchored_hashes() {
        let repo = TestRepo::init();
        let base = repo.commit("base\n", "base");
        let mid = repo.commit("mid\n", "mid");
        let tip = repo.commit("tip\n", "tip");

        let mid_unit = capture_review(
            CaptureOptions::new(repo.path())
                .with_commit_range(CommitRangeSpec::new(base.clone()).with_target_rev(mid.clone())),
        )
        .unwrap();
        let tip_unit = capture_review(
            CaptureOptions::new(repo.path())
                .with_commit_range(CommitRangeSpec::new(mid.clone()).with_target_rev(tip.clone())),
        )
        .unwrap();

        // `--ref` resolves only the unit anchored on the named commit.
        let by_ref = remove_content(RemoveOptions::new(
            repo.path(),
            RemoveSelector::Ref(mid.clone()),
        ))
        .unwrap();
        let ref_hashes: Vec<&str> = by_ref
            .removed
            .iter()
            .map(|r| r.content_hash.as_str())
            .collect();
        assert!(ref_hashes.contains(&mid_unit.snapshot_artifact_content_hash.as_str()));
        assert!(!ref_hashes.contains(&tip_unit.snapshot_artifact_content_hash.as_str()));

        // `--range mid..tip` resolves only the unit anchored on tip.
        let by_range = remove_content(RemoveOptions::new(
            repo.path(),
            RemoveSelector::Range(format!("{mid}..{tip}")),
        ))
        .unwrap();
        let range_hashes: Vec<&str> = by_range
            .removed
            .iter()
            .map(|r| r.content_hash.as_str())
            .collect();
        assert!(range_hashes.contains(&tip_unit.snapshot_artifact_content_hash.as_str()));
    }

    #[test]
    fn remove_by_range_rejects_a_non_range_argument() {
        let repo = TestRepo::init();
        repo.commit("base\n", "base");

        // A bare rev is not a range; removal must refuse it rather than silently
        // resolving every unit anchored on a commit reachable from that rev.
        let error = remove_content(RemoveOptions::new(
            repo.path(),
            RemoveSelector::Range("HEAD".to_owned()),
        ))
        .unwrap_err();
        assert!(
            error.to_string().contains(".."),
            "error names the expected range form: {error}"
        );
    }

    #[test]
    fn re_removing_a_hash_dedups_to_existing() {
        let repo = TestRepo::init();
        let capture = capture_worktree(&repo);
        let selector = RemoveSelector::Snapshot(capture.snapshot_id.clone());

        let first = remove_content(RemoveOptions::new(repo.path(), selector.clone())).unwrap();
        assert_eq!(first.events_created, 1);

        let second = remove_content(RemoveOptions::new(repo.path(), selector)).unwrap();
        assert_eq!(second.events_created, 0);
        assert_eq!(second.events_existing, 1);
        assert!(!second.removed[0].created);

        let stored = list_events(&repo)
            .into_iter()
            .filter(|event| event.event_type == EventType::ArtifactRemoved)
            .count();
        assert_eq!(stored, 1, "the idempotency key collapses the re-removal");
    }

    #[test]
    fn remove_payload_carries_only_content_hash() {
        let repo = TestRepo::init();
        let capture = capture_worktree(&repo);

        remove_content(RemoveOptions::new(
            repo.path(),
            RemoveSelector::Snapshot(capture.snapshot_id.clone()),
        ))
        .unwrap();

        let event = list_events(&repo)
            .into_iter()
            .find(|event| event.event_type == EventType::ArtifactRemoved)
            .expect("an artifact_removed event was recorded");
        let keys: Vec<&str> = event
            .payload
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, vec!["contentHash"]);
        // The envelope is session-only: no review unit, no snapshot binding.
        assert!(event.target.review_unit_id.is_none());
        assert!(event.target.snapshot_id.is_none());
    }

    #[test]
    fn remove_then_compact_physically_deletes_blob() {
        let repo = TestRepo::init();
        let capture = capture_worktree(&repo);
        assert_eq!(blob_count(&repo, "artifacts/snapshots"), 1);

        remove_content(RemoveOptions::new(
            repo.path(),
            RemoveSelector::Snapshot(capture.snapshot_id.clone()),
        ))
        .unwrap();
        let result = compact_store(CompactOptions::new(repo.path())).unwrap();

        // The snapshot blob is gone; the sweep reports it removed with bytes back.
        assert_eq!(blob_count(&repo, "artifacts/snapshots"), 0);
        let swept = result
            .swept
            .iter()
            .find(|blob| blob.content_hash == capture.snapshot_artifact_content_hash)
            .expect("the removed snapshot blob is swept");
        assert_eq!(swept.outcome, SweepOutcome::Removed);
        assert!(result.bytes_reclaimed > 0);

        // The event log and projection are never swept.
        assert!(
            list_events(&repo)
                .iter()
                .any(|event| event.event_type == EventType::ReviewUnitCaptured)
        );
        assert!(resolved_store_dir(repo.path()).join("state.json").exists());
    }

    #[test]
    fn compact_emits_no_event() {
        let repo = TestRepo::init();
        let capture = capture_worktree(&repo);
        remove_content(RemoveOptions::new(
            repo.path(),
            RemoveSelector::Snapshot(capture.snapshot_id.clone()),
        ))
        .unwrap();

        let before = list_events(&repo).len();
        compact_store(CompactOptions::new(repo.path())).unwrap();
        let after = list_events(&repo).len();
        assert_eq!(before, after, "the sweep appends no event");
    }

    #[test]
    fn compact_never_sweeps_a_referenced_non_removed_blob() {
        let repo = TestRepo::init();
        let base = repo.commit("base\n", "base");
        let kept = repo.commit("kept\n", "kept");
        let removed = repo.commit("removed\n", "removed");
        let kept_unit =
            capture_review(CaptureOptions::new(repo.path()).with_commit_range(
                CommitRangeSpec::new(base.clone()).with_target_rev(kept.clone()),
            ))
            .unwrap();
        let removed_unit = capture_review(
            CaptureOptions::new(repo.path())
                .with_commit_range(CommitRangeSpec::new(kept).with_target_rev(removed)),
        )
        .unwrap();
        assert_eq!(blob_count(&repo, "artifacts/snapshots"), 2);

        remove_content(RemoveOptions::new(
            repo.path(),
            RemoveSelector::Snapshot(removed_unit.snapshot_id.clone()),
        ))
        .unwrap();
        compact_store(CompactOptions::new(repo.path())).unwrap();

        // Only the removed blob is gone; the live, non-removed one survives.
        assert_eq!(blob_count(&repo, "artifacts/snapshots"), 1);
        assert!(!removed_set(&repo).is_removed(&kept_unit.snapshot_artifact_content_hash));
    }

    #[test]
    fn compact_is_idempotent() {
        let repo = TestRepo::init();
        let capture = capture_worktree(&repo);
        remove_content(RemoveOptions::new(
            repo.path(),
            RemoveSelector::Snapshot(capture.snapshot_id.clone()),
        ))
        .unwrap();

        compact_store(CompactOptions::new(repo.path())).unwrap();
        let second = compact_store(CompactOptions::new(repo.path())).unwrap();
        assert!(
            second
                .swept
                .iter()
                .all(|blob| blob.outcome == SweepOutcome::Missing),
            "a second sweep finds every removed blob already gone: {:?}",
            second.swept
        );
        assert_eq!(second.bytes_reclaimed, 0);
    }

    #[test]
    fn compact_sweeps_removed_note_body_blob() {
        let repo = TestRepo::init();
        let capture = capture_worktree(&repo);
        let big_body = "y".repeat(5000);
        let observation = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_review_unit_id(capture.review_unit_id.clone())
                .with_track("agent:tester")
                .with_title("a large observation")
                .with_body(big_body),
        )
        .unwrap();
        let body_hash = observation
            .body_content_hash
            .expect("a >4096-byte body is stored as a note artifact");
        assert_eq!(blob_count(&repo, "artifacts/notes"), 1);

        remove_content(RemoveOptions::new(
            repo.path(),
            RemoveSelector::ReviewUnit(capture.review_unit_id.clone()),
        ))
        .unwrap();
        let result = compact_store(CompactOptions::new(repo.path())).unwrap();

        assert_eq!(blob_count(&repo, "artifacts/notes"), 0);
        assert!(
            result
                .swept
                .iter()
                .any(|blob| blob.content_hash == body_hash
                    && blob.outcome == SweepOutcome::Removed)
        );
    }
}
