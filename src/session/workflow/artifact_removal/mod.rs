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
use crate::model::{ActorId, JournalId, ObjectId, RevisionId, id_prefix};
use crate::session::body_artifact::{
    note_body_content_hash_from_path, validate_note_body_artifact_bytes,
};
use crate::session::event::{
    ArtifactRemovedPayload, EventTarget, EventType, ShoreEvent, WorkObjectProposal,
    WorkObjectProposedPayload,
};
use crate::session::object_artifact::decode_and_validate_object_artifact;
use crate::session::projection::cosignature::CosignatureIndex;
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store::content::ContentArtifacts;
use crate::session::store::resolution::{prepare_write_landing, resolve_write_store};
use crate::session::{
    ArtifactRemovalProjection, CommitGraphCondition, EventSigningOptions, EventStore,
    EventWriteOutcome, OrphanReason, RemovalOperativeStatus, RemovalPolicy,
    RevisionCommitRangeProjection, TrustSet, current_timestamp, enrich_liveness,
    referenced_artifacts, sign_event_if_requested, writer_from_options,
};
use crate::storage::{Durability, LocalStorage, RemoveOutcome};

/// Which content a removal targets. Every variant resolves to a set of
/// `content_hash`es before any event is emitted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RemoveSelector {
    /// A single snapshot's bound artifact content hash.
    Snapshot(ObjectId),
    /// Every content hash a review unit references.
    Revision(RevisionId),
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
    pub co_referencing_units: Vec<RevisionId>,
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
    let event_store = EventStore::from_backend(write_store.backend());

    // Resolve the selector to content hashes from the event log + git
    // reachability — entirely before any event is emitted.
    let events = event_store.list_events()?;
    let index = content_hash_unit_index(&events)?;
    let resolved = resolve_selector(&options.selector, &events, &options.repo, &index)?;

    // One ArtifactRemoved per resolved hash, session-anchored and idempotent.
    let session_id = JournalId::new(format!("{}:default", id_prefix::JOURNAL));
    let writer = writer_from_options(&worktree_root, options.actor_id.as_ref());

    let mut removed = Vec::new();
    let mut events_created = 0;
    let mut events_existing = 0;
    for content_hash in &resolved.content_hashes {
        let mut event = ShoreEvent::new(
            EventType::ArtifactRemoved,
            ArtifactRemovedPayload::idempotency_key(content_hash),
            EventTarget::for_journal(session_id.clone()),
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
    targeted_units: BTreeSet<RevisionId>,
}

/// Map every referenced `content_hash` to the review units that name it, built
/// once and consumed by both selector resolution and the co-referencing report.
/// Each event's content hashes come from [`referenced_artifacts`] (the canonical
/// normalized `sha256:` extraction); the naming unit is the event's review-unit
/// target.
fn content_hash_unit_index(
    events: &[ShoreEvent],
) -> Result<BTreeMap<String, BTreeSet<RevisionId>>> {
    let mut index: BTreeMap<String, BTreeSet<RevisionId>> = BTreeMap::new();
    for event in events {
        let Some(unit) = event.subject_revision_id()? else {
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
    index: &BTreeMap<String, BTreeSet<RevisionId>>,
    content_hash: &str,
    targeted_units: &BTreeSet<RevisionId>,
) -> Vec<RevisionId> {
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
    index: &BTreeMap<String, BTreeSet<RevisionId>>,
) -> Result<ResolvedSelection> {
    match selector {
        RemoveSelector::Snapshot(object_id) => {
            let (unit, content_hash) = capture_bound_to_snapshot(events, object_id)?;
            let mut content_hashes = BTreeSet::new();
            content_hashes.insert(content_hash);
            let mut targeted_units = BTreeSet::new();
            targeted_units.insert(unit);
            Ok(ResolvedSelection {
                content_hashes,
                targeted_units,
            })
        }
        RemoveSelector::Revision(revision_id) => {
            let mut targeted_units = BTreeSet::new();
            targeted_units.insert(revision_id.clone());
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
    targeted_units: BTreeSet<RevisionId>,
    index: &BTreeMap<String, BTreeSet<RevisionId>>,
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
/// `object_id`.
fn capture_bound_to_snapshot(
    events: &[ShoreEvent],
    object_id: &ObjectId,
) -> Result<(RevisionId, String)> {
    for event in events {
        if event.event_type != EventType::WorkObjectProposed {
            continue;
        }
        let payload: WorkObjectProposedPayload = serde_json::from_value(event.payload.clone())?;
        let WorkObjectProposal::Revision {
            revision,
            object_artifact_content_hash,
            ..
        } = payload.work_object
        else {
            continue;
        };
        if &revision.object_id == object_id {
            return Ok((revision.id, object_artifact_content_hash));
        }
    }
    Err(ShoreError::Message(format!(
        "unknown snapshot: {}",
        object_id.as_str()
    )))
}

/// The units whose current commit set intersects `oids`.
fn units_anchored_on_oids(
    events: &[ShoreEvent],
    oids: &BTreeSet<String>,
) -> Result<BTreeSet<RevisionId>> {
    let projection = RevisionCommitRangeProjection::from_events(events)?;
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
fn orphaned_anchored_units(events: &[ShoreEvent], repo: &Path) -> Result<BTreeSet<RevisionId>> {
    let projection = RevisionCommitRangeProjection::from_events(events)?;
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
    /// The reader's trust set, so the fixed erase-eligibility rule can lift a
    /// relayed removal via a trusted signer or endorsement. The empty default
    /// erases only locally-authored (possession) removals.
    pub trust_set: TrustSet,
    /// Preview only: enumerate the eligible and skipped sets and delete nothing.
    pub dry_run: bool,
}

impl CompactOptions {
    pub fn new(repo: impl Into<PathBuf>) -> Self {
        Self {
            repo: repo.into(),
            trust_set: TrustSet::default(),
            dry_run: false,
        }
    }

    pub fn with_trust_set(mut self, trust_set: TrustSet) -> Self {
        self.trust_set = trust_set;
        self
    }

    pub fn with_dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }
}

/// A removal whose blob was NOT erased because it is not erase-eligible, paired
/// with the reason: the integrity floor (`ClaimInvalid`), or an ingested
/// untrusted/unsigned claim without a trusted endorsement (`ClaimUntrusted` /
/// `ClaimUnsigned`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkippedRemoval {
    /// The normalized `sha256:<hex>` content hash that was withheld from erasure.
    pub content_hash: String,
    pub reason: RemovalOperativeStatus,
}

/// What the sweep did to one on-disk blob. The public, binary-crate-visible
/// counterpart of the storage-layer removal outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SweepOutcome {
    Removed,
    Missing,
    /// The blob was erase-eligible but its on-disk bytes no longer hash to the
    /// content hash the payload→file join claims, so the sweep refused to delete
    /// it. The bytes survive; the drift is surfaced as a `compact_hash_mismatch`
    /// diagnostic.
    HashMismatchSkipped,
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
    /// The erase-eligible blobs the sweep deleted (or, under `dry_run`, would
    /// delete), in deterministic order.
    pub swept: Vec<SweptBlob>,
    /// Best-effort sum of the on-disk byte sizes actually reclaimed. Always 0
    /// under `dry_run`.
    pub bytes_reclaimed: u64,
    /// True when this was a preview that deleted nothing.
    pub dry_run: bool,
    /// Removals that were withheld from erasure because they are not
    /// erase-eligible, each with its reason. Never deleted.
    pub skipped_ineligible: Vec<SkippedRemoval>,
}

/// Physically delete the content-addressed blobs whose `content_hash` was marked
/// removed. A local, non-event maintenance sweep — it appends no event, rewrites
/// no `state.json`, and is fully re-derivable from the log. `gc` and `compact`
/// are the same operation; re-capturing the same content re-materializes a swept
/// blob (there is no un-remove — the removal fact persists in the log).
pub fn compact_store(options: CompactOptions) -> Result<CompactResult> {
    let write_store = resolve_write_store(&options.repo)?;
    let content = ContentArtifacts::from_backend(write_store.backend());

    let events = EventStore::from_backend(write_store.backend()).list_events()?;
    let removal = ArtifactRemovalProjection::from_events(&events)?;
    let cosig = CosignatureIndex::build(&events)?;

    // Two floors protect the on-disk set. First: a non-removed blob is never
    // enumerated into the sweep, so a blob still named by a live, non-removed
    // event is safe by construction. Second: of the removed blobs, only the
    // erase-eligible ones are deleted — the fixed `PossessionOrTrusted` rule,
    // which never reads a render preset, so a laxer reader can never widen what
    // is irreversibly erased. An ineligible removal is reported, never deleted.
    let mut swept = Vec::new();
    let mut bytes_reclaimed = 0u64;
    let mut skipped_ineligible = Vec::new();
    for blob in on_disk_blobs(&content, &events)? {
        if !removal.is_removed(&blob.content_hash) {
            continue;
        }
        if !removal.is_erase_eligible(&blob.content_hash, &options.trust_set, &cosig)? {
            let reason = removal.operative_status(
                &blob.content_hash,
                &options.trust_set,
                RemovalPolicy::PossessionOrTrusted,
                &cosig,
            )?;
            skipped_ineligible.push(SkippedRemoval {
                content_hash: blob.content_hash,
                reason,
            });
            continue;
        }
        // A third floor, on the irreversible delete path: re-read the blob and
        // re-derive its content hash, refusing to delete bytes that have drifted
        // from the hash the payload→file join claims (the analog of refusing to
        // delete a chunk whose hash no longer verifies). The read's length feeds
        // the byte accounting, so the delete path reads each file exactly once,
        // and the same check runs under `dry_run` so a preview is honest about
        // what it would skip.
        let bytes = match content.get_if_exists(&blob.relative_path)? {
            Some(bytes) => bytes,
            None => {
                // Already gone (e.g. a prior sweep): nothing to reclaim.
                swept.push(SweptBlob {
                    content_hash: blob.content_hash,
                    outcome: SweepOutcome::Missing,
                });
                continue;
            }
        };
        if !blob_content_matches_claim(&blob, &bytes) {
            swept.push(SweptBlob {
                content_hash: blob.content_hash,
                outcome: SweepOutcome::HashMismatchSkipped,
            });
            continue;
        }
        if options.dry_run {
            // Would-remove: list it, delete nothing, reclaim nothing.
            swept.push(SweptBlob {
                content_hash: blob.content_hash,
                outcome: SweepOutcome::Removed,
            });
            continue;
        }
        let byte_size = bytes.len() as u64;
        let outcome = match content.remove(&blob.relative_path)? {
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
        dry_run: options.dry_run,
        skipped_ineligible,
    })
}

/// Re-derive an erase-eligible blob's content hash from its on-disk bytes and
/// confirm it still equals the `content_hash` the payload→file join claims.
/// Objects must decode to a self-consistent v2 artifact whose hash matches; note
/// bodies must hash to their locator. Identity is over decoded content, not raw
/// storage bytes, so a cosmetic re-encoding that decodes identically still
/// matches. Any decode/parse failure counts as a mismatch — the sweep never
/// deletes bytes it cannot prove still match their claimed identity.
fn blob_content_matches_claim(blob: &OnDiskBlob, bytes: &[u8]) -> bool {
    if blob.relative_path.starts_with("artifacts/objects/") {
        matches!(
            decode_and_validate_object_artifact(bytes),
            Ok(artifact) if artifact.content_hash == blob.content_hash
        )
    } else {
        validate_note_body_artifact_bytes(&blob.relative_path, &blob.content_hash, bytes).is_ok()
    }
}

/// A content-addressed blob on disk, paired with the `content_hash` it carries.
struct OnDiskBlob {
    /// Store-relative path under `artifacts/objects` or `artifacts/notes`.
    relative_path: String,
    /// The normalized `sha256:<hex>` content hash the blob carries.
    content_hash: String,
}

/// Enumerate every content-addressed blob under `artifacts/objects` and
/// `artifacts/notes`, mapping each on-disk file to its `content_hash`. New object
/// artifact filenames are the content-hash hex; legacy object-id-keyed filenames
/// are still resolved through the `WorkObjectProposed` payloads.
fn on_disk_blobs(content: &ContentArtifacts, events: &[ShoreEvent]) -> Result<Vec<OnDiskBlob>> {
    let object_hashes = object_content_hashes(events)?;
    let legacy_snapshot_hash_by_stem = snapshot_content_hash_by_file_stem(events)?;
    let mut blobs = Vec::new();

    for relative_path in content.list_refs("artifacts/objects")? {
        let file_name = ref_file_name(&relative_path);
        let stem = file_name.strip_suffix(".json").unwrap_or(file_name);
        let stem_hash = normalized_sha256_stem(stem);
        let content_hash = match stem_hash {
            Some(stem_hash) if object_hashes.contains(&stem_hash) => Some(stem_hash),
            _ => legacy_snapshot_hash_by_stem
                .get(stem)
                .cloned()
                .or_else(|| normalized_sha256_stem(stem)),
        };
        if let Some(content_hash) = content_hash {
            blobs.push(OnDiskBlob {
                relative_path: relative_path.clone(),
                content_hash,
            });
        }
    }

    for relative_path in content.list_refs("artifacts/notes")? {
        let content_hash = note_body_content_hash_from_path(&relative_path)?;
        blobs.push(OnDiskBlob {
            relative_path,
            content_hash,
        });
    }

    Ok(blobs)
}

/// The trailing file-name segment of a store-relative content ref.
fn ref_file_name(content_ref: &str) -> &str {
    content_ref.rsplit('/').next().unwrap_or(content_ref)
}

fn normalized_sha256_stem(stem: &str) -> Option<String> {
    (stem.len() == 64 && stem.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| format!("sha256:{stem}"))
}

fn object_content_hashes(events: &[ShoreEvent]) -> Result<std::collections::BTreeSet<String>> {
    let mut hashes = std::collections::BTreeSet::new();
    for event in events {
        if event.event_type != EventType::WorkObjectProposed {
            continue;
        }
        let payload: WorkObjectProposedPayload = serde_json::from_value(event.payload.clone())?;
        let WorkObjectProposal::Revision {
            object_artifact_content_hash,
            ..
        } = payload.work_object
        else {
            continue;
        };
        hashes.insert(object_artifact_content_hash);
    }
    Ok(hashes)
}

/// Map each legacy captured snapshot's on-disk file stem (`sha256(objectId)`) to
/// its artifact content hash, the join that lets the sweep recognize old object
/// blobs that predate content-hash filenames.
fn snapshot_content_hash_by_file_stem(events: &[ShoreEvent]) -> Result<BTreeMap<String, String>> {
    let mut by_stem = BTreeMap::new();
    for event in events {
        if event.event_type != EventType::WorkObjectProposed {
            continue;
        }
        let payload: WorkObjectProposedPayload = serde_json::from_value(event.payload.clone())?;
        let WorkObjectProposal::Revision {
            revision,
            object_artifact_content_hash,
            ..
        } = payload.work_object
        else {
            continue;
        };
        let stem = sha256_bytes_hex(revision.object_id.as_str().as_bytes());
        by_stem.insert(stem, object_artifact_content_hash);
    }
    Ok(by_stem)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;
    use std::process::Command;

    use super::*;
    use crate::crypto::{EventSignatureBytes, SignerId};
    use crate::model::{EngagementId, JournalId};
    use crate::session::event::{
        ArtifactRemovedPayload, EventSignature, EventTarget, EventType, GitProvenance,
        IngestProvenance, IngestVia, Revision, ShoreEvent, WorkObjectProposal,
        WorkObjectProposedPayload, Writer, WriterProducer,
    };
    use crate::session::{
        ArtifactRemovalProjection, CaptureOptions, CommitRangeSpec, EventStore,
        ObservationAddOptions, RemovalOperativeStatus, capture_review, capture_worktree_review,
        record_observation,
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

    /// Count the blob files under a store subdir (e.g. `artifacts/objects`).
    fn blob_count(repo: &TestRepo, subdir: &str) -> usize {
        let dir = resolved_store_dir(repo.path()).join(subdir);
        match std::fs::read_dir(&dir) {
            Ok(entries) => entries.filter_map(std::result::Result::ok).count(),
            Err(_) => 0,
        }
    }

    /// Record a fabricated sibling `WorkObjectProposed` event that binds the same
    /// snapshot content hash under a different review unit — the cross-worktree
    /// coexistence case (one shared blob, two capture events). Returns the
    /// sibling unit id.
    fn fabricate_sibling_capture(
        repo: &TestRepo,
        original: &crate::session::CaptureResult,
    ) -> RevisionId {
        let sibling_unit = RevisionId::new(format!("{}-sibling", original.revision_id.as_str()));
        let sibling_snapshot = ObjectId::new(format!("{}-sibling", original.object_id.as_str()));
        let event = ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("work_object_proposed:{}", sibling_unit.as_str()),
            EventTarget::for_revision(original.journal_id.clone(), sibling_unit.clone(), None)
                .unwrap(),
            Writer {
                actor_id: ActorId::new("actor:sibling"),
                producer: WriterProducer {
                    name: "shore".to_owned(),
                    version: "test".to_owned(),
                },
            },
            WorkObjectProposedPayload {
                engagement_id: EngagementId::new(format!(
                    "engagement:sha256:{}",
                    crate::canonical_hash::sha256_bytes_hex(
                        (original.revision_id.clone()).as_str().as_bytes()
                    )
                )),
                work_object: WorkObjectProposal::Revision {
                    revision: Revision {
                        // The payload revision must match the envelope subject
                        // (`sibling_unit`): the content-hash→unit index now derives
                        // the unit from this payload, not the envelope.
                        id: sibling_unit.clone(),
                        object_id: sibling_snapshot,
                        git_provenance: Some(GitProvenance {
                            source: original.source.clone(),
                            base: original.base.clone(),
                            target: original.target.clone(),
                        }),
                    },
                    object_artifact_content_hash: original.object_artifact_content_hash.clone(),
                    supersedes: vec![],
                },
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
            RemoveSelector::Snapshot(capture.object_id.clone()),
        ))
        .unwrap();

        assert_eq!(result.removed.len(), 1);
        assert_eq!(
            result.removed[0].content_hash,
            capture.object_artifact_content_hash
        );
        assert!(result.removed[0].created);
        assert_eq!(result.events_created, 1);
        assert!(removed_set(&repo).is_removed(&capture.object_artifact_content_hash));
    }

    #[test]
    fn remove_by_revision_resolves_all_referenced_hashes() {
        let repo = TestRepo::init();
        let capture = capture_worktree(&repo);
        let big_body = "x".repeat(5000);
        let observation = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_revision_id(capture.revision_id.clone())
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
            RemoveSelector::Revision(capture.revision_id.clone()),
        ))
        .unwrap();

        let hashes: Vec<&str> = result
            .removed
            .iter()
            .map(|r| r.content_hash.as_str())
            .collect();
        assert!(hashes.contains(&capture.object_artifact_content_hash.as_str()));
        assert!(hashes.contains(&body_hash.as_str()));
        assert_eq!(result.events_created, 2);
        let removed = removed_set(&repo);
        assert!(removed.is_removed(&capture.object_artifact_content_hash));
        assert!(removed.is_removed(&body_hash));
    }

    #[test]
    fn remove_by_revision_reports_co_referencing_units() {
        let repo = TestRepo::init();
        let capture = capture_worktree(&repo);
        let sibling = fabricate_sibling_capture(&repo, &capture);

        let result = remove_content(RemoveOptions::new(
            repo.path(),
            RemoveSelector::Revision(capture.revision_id.clone()),
        ))
        .unwrap();

        let entry = result
            .removed
            .iter()
            .find(|r| r.content_hash == capture.object_artifact_content_hash)
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
            hashes.contains(&doomed_capture.object_artifact_content_hash.as_str()),
            "the orphaned unit's hash is resolved"
        );
        assert!(
            !hashes.contains(&reachable_capture.object_artifact_content_hash.as_str()),
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
        assert!(ref_hashes.contains(&mid_unit.object_artifact_content_hash.as_str()));
        assert!(!ref_hashes.contains(&tip_unit.object_artifact_content_hash.as_str()));

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
        assert!(range_hashes.contains(&tip_unit.object_artifact_content_hash.as_str()));
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
        let selector = RemoveSelector::Snapshot(capture.object_id.clone());

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
            RemoveSelector::Snapshot(capture.object_id.clone()),
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
        // The envelope is journal-only: no review unit, no snapshot binding — it
        // rides the subject-less journal carrier.
        assert!(crate::model::subject_revision_id(&event.reconstruct_subject().unwrap()).is_none());
        assert!(matches!(
            event.reconstruct_subject().unwrap(),
            crate::model::TargetRef::Journal
        ));
    }

    #[test]
    fn remove_then_compact_physically_deletes_blob() {
        let repo = TestRepo::init();
        let capture = capture_worktree(&repo);
        assert_eq!(blob_count(&repo, "artifacts/objects"), 1);

        remove_content(RemoveOptions::new(
            repo.path(),
            RemoveSelector::Snapshot(capture.object_id.clone()),
        ))
        .unwrap();
        let result = compact_store(CompactOptions::new(repo.path())).unwrap();

        // The snapshot blob is gone; the sweep reports it removed with bytes back.
        assert_eq!(blob_count(&repo, "artifacts/objects"), 0);
        let swept = result
            .swept
            .iter()
            .find(|blob| blob.content_hash == capture.object_artifact_content_hash)
            .expect("the removed snapshot blob is swept");
        assert_eq!(swept.outcome, SweepOutcome::Removed);
        assert!(result.bytes_reclaimed > 0);

        // The event log and projection are never swept.
        assert!(
            list_events(&repo)
                .iter()
                .any(|event| event.event_type == EventType::WorkObjectProposed)
        );
        assert!(resolved_store_dir(repo.path()).join("state.json").exists());
    }

    #[test]
    fn compact_skips_a_blob_whose_bytes_no_longer_match_its_hash() {
        let repo = TestRepo::init();
        let capture = capture_worktree(&repo);
        assert_eq!(blob_count(&repo, "artifacts/objects"), 1);

        remove_content(RemoveOptions::new(
            repo.path(),
            RemoveSelector::Snapshot(capture.object_id.clone()),
        ))
        .unwrap();

        // Tamper the erase-eligible blob's bytes on disk so they no longer hash
        // to the content hash the payload→file join claims.
        let objects_dir = resolved_store_dir(repo.path()).join("artifacts/objects");
        let blob_path = std::fs::read_dir(&objects_dir)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .next()
            .expect("one object artifact file on disk");
        std::fs::write(&blob_path, b"tampered: no longer a valid object artifact").unwrap();

        let result = compact_store(CompactOptions::new(repo.path())).unwrap();

        // The file SURVIVES: the sweep refuses to delete bytes that have drifted
        // from their claimed hash, and reports the drift instead of deleting.
        assert_eq!(blob_count(&repo, "artifacts/objects"), 1);
        let swept = result
            .swept
            .iter()
            .find(|blob| blob.content_hash == capture.object_artifact_content_hash)
            .expect("the drifted blob is reported");
        assert_eq!(swept.outcome, SweepOutcome::HashMismatchSkipped);
        assert_eq!(result.bytes_reclaimed, 0);
    }

    #[test]
    fn compact_emits_no_event() {
        let repo = TestRepo::init();
        let capture = capture_worktree(&repo);
        remove_content(RemoveOptions::new(
            repo.path(),
            RemoveSelector::Snapshot(capture.object_id.clone()),
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
        assert_eq!(blob_count(&repo, "artifacts/objects"), 2);

        remove_content(RemoveOptions::new(
            repo.path(),
            RemoveSelector::Snapshot(removed_unit.object_id.clone()),
        ))
        .unwrap();
        compact_store(CompactOptions::new(repo.path())).unwrap();

        // Only the removed blob is gone; the live, non-removed one survives.
        assert_eq!(blob_count(&repo, "artifacts/objects"), 1);
        assert!(!removed_set(&repo).is_removed(&kept_unit.object_artifact_content_hash));
    }

    #[test]
    fn compact_is_idempotent() {
        let repo = TestRepo::init();
        let capture = capture_worktree(&repo);
        remove_content(RemoveOptions::new(
            repo.path(),
            RemoveSelector::Snapshot(capture.object_id.clone()),
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
                .with_revision_id(capture.revision_id.clone())
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
            RemoveSelector::Revision(capture.revision_id.clone()),
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

    #[test]
    fn compact_skips_a_note_body_blob_whose_bytes_no_longer_match_its_hash() {
        let repo = TestRepo::init();
        let capture = capture_worktree(&repo);
        let observation = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_revision_id(capture.revision_id.clone())
                .with_track("agent:tester")
                .with_title("a large observation")
                .with_body("y".repeat(5000)),
        )
        .unwrap();
        let body_hash = observation
            .body_content_hash
            .expect("a >4096-byte body is stored as a note artifact");

        remove_content(RemoveOptions::new(
            repo.path(),
            RemoveSelector::Revision(capture.revision_id.clone()),
        ))
        .unwrap();

        // Tamper the note body blob so it no longer decodes to a body that hashes
        // to its locator — the note-path arm of the re-hash floor.
        let notes_dir = resolved_store_dir(repo.path()).join("artifacts/notes");
        let blob_path = std::fs::read_dir(&notes_dir)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .next()
            .expect("one note body artifact file on disk");
        std::fs::write(
            &blob_path,
            b"tampered: no longer a valid note body artifact",
        )
        .unwrap();

        let result = compact_store(CompactOptions::new(repo.path())).unwrap();

        // The note body survives and is reported as a hash mismatch.
        assert_eq!(blob_count(&repo, "artifacts/notes"), 1);
        assert!(
            result
                .swept
                .iter()
                .any(|blob| blob.content_hash == body_hash
                    && blob.outcome == SweepOutcome::HashMismatchSkipped)
        );
    }

    fn removal_event_for(content_hash: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ArtifactRemoved,
            ArtifactRemovedPayload::idempotency_key(content_hash),
            EventTarget::for_journal(JournalId::new("journal:default")),
            Writer::shore_local("test"),
            ArtifactRemovedPayload {
                content_hash: content_hash.to_owned(),
            },
            "2026-06-19T00:00:00Z",
        )
        .unwrap()
    }

    /// Record an `ArtifactRemoved` that arrived through a foreign-event seam
    /// (`ingest = Some`, unsigned): a non-operative claim with no possession arm.
    fn record_ingested_removal(repo: &TestRepo, content_hash: &str) {
        let mut event = removal_event_for(content_hash);
        event.ingest = Some(IngestProvenance {
            via: IngestVia::IngestEvents,
            received_at: "2026-06-19T01:00:00Z".to_owned(),
        });
        EventStore::open(resolved_store_dir(repo.path()))
            .record_event_once(&event)
            .unwrap();
    }

    /// Record a locally-authored (`ingest = None`) removal whose inline signature
    /// verifies invalid — the integrity floor, which possession can never lift.
    fn record_invalid_possessed_removal(repo: &TestRepo, content_hash: &str) {
        let mut event = removal_event_for(content_hash);
        event.signer = Some(SignerId::from_ed25519_public_key([93u8; 32]));
        event.signature = Some(EventSignature::ed25519_v1(EventSignatureBytes::from_bytes(
            &[0u8; 64],
        )));
        EventStore::open(resolved_store_dir(repo.path()))
            .record_event_once(&event)
            .unwrap();
    }

    #[test]
    fn compact_skips_an_ingested_unsigned_removal() {
        let repo = TestRepo::init();
        let capture = capture_worktree(&repo);
        assert_eq!(blob_count(&repo, "artifacts/objects"), 1);
        record_ingested_removal(&repo, &capture.object_artifact_content_hash);

        let result = compact_store(CompactOptions::new(repo.path())).unwrap();

        // The blob survives; it is reported skipped with its claim reason.
        assert_eq!(blob_count(&repo, "artifacts/objects"), 1);
        assert!(result.swept.is_empty());
        assert!(result.skipped_ineligible.iter().any(|skipped| {
            skipped.content_hash == capture.object_artifact_content_hash
                && skipped.reason == RemovalOperativeStatus::ClaimUnsigned
        }));
    }

    #[test]
    fn compact_never_erases_invalid_floor_even_possessed() {
        let repo = TestRepo::init();
        let capture = capture_worktree(&repo);
        record_invalid_possessed_removal(&repo, &capture.object_artifact_content_hash);

        let result = compact_store(CompactOptions::new(repo.path())).unwrap();

        assert_eq!(
            blob_count(&repo, "artifacts/objects"),
            1,
            "the invalid floor is never erased, even with possession"
        );
        assert!(
            result
                .skipped_ineligible
                .iter()
                .any(|skipped| skipped.reason == RemovalOperativeStatus::ClaimInvalid)
        );
    }

    #[test]
    fn compact_dry_run_deletes_nothing() {
        let repo = TestRepo::init();
        let capture = capture_worktree(&repo);
        remove_content(RemoveOptions::new(
            repo.path(),
            RemoveSelector::Snapshot(capture.object_id.clone()),
        ))
        .unwrap();

        let result = compact_store(CompactOptions::new(repo.path()).with_dry_run(true)).unwrap();

        assert_eq!(
            blob_count(&repo, "artifacts/objects"),
            1,
            "a dry run deletes nothing"
        );
        assert!(result.dry_run);
        assert_eq!(result.bytes_reclaimed, 0);
        // The would-remove blob is still listed.
        assert!(result.swept.iter().any(|blob| {
            blob.content_hash == capture.object_artifact_content_hash
                && blob.outcome == SweepOutcome::Removed
        }));
    }

    #[test]
    fn compact_re_erases_recaptured_blob() {
        let repo = TestRepo::init();
        let capture = capture_worktree(&repo);
        remove_content(RemoveOptions::new(
            repo.path(),
            RemoveSelector::Snapshot(capture.object_id.clone()),
        ))
        .unwrap();
        compact_store(CompactOptions::new(repo.path())).unwrap();
        assert_eq!(blob_count(&repo, "artifacts/objects"), 0);

        // Re-capturing the same content re-materializes the blob while the removal
        // fact persists in the log.
        let recaptured = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        assert_eq!(
            recaptured.object_artifact_content_hash,
            capture.object_artifact_content_hash
        );
        assert_eq!(blob_count(&repo, "artifacts/objects"), 1);

        let result = compact_store(CompactOptions::new(repo.path())).unwrap();

        assert_eq!(
            blob_count(&repo, "artifacts/objects"),
            0,
            "the re-captured blob is re-erased on the next compact"
        );
        assert!(result.swept.iter().any(|blob| {
            blob.content_hash == capture.object_artifact_content_hash
                && blob.outcome == SweepOutcome::Removed
        }));
    }
}
