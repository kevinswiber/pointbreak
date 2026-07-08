use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::Result;
use crate::model::{ObjectId, ReviewEndpoint, RevisionId, RevisionSource};
use crate::session::event::{EventType, ShoreEvent, WorkObjectProposal, WorkObjectProposedPayload};
use crate::session::projection::skipped_to_diagnostics;
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store::resolution::resolve_read_store;
use crate::session::workflow::association::normalize_ref;
use crate::session::workflow::commit_range_liveness::{
    CommitGraphCondition, LivenessBatch, enrich_liveness,
};
use crate::session::workflow::observation::{CurrentRevisionContext, revision_ids_in_worktree};
use crate::session::{
    CommitOidGroupingProjection, EventStore, RevisionCommitRangeProjection, RevisionCommitRangeView,
};

/// How a `--ref` read filter matches: by the recorded label (offline, answerable
/// even after the branch is deleted) or by reachability from the ref's live tip.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RefFilterMode {
    #[default]
    Label,
    Liveness,
}

/// Which units the list surfaces with respect to commit-reachability. A unit is
/// "orphaned" when it is commit-anchored and every current commit is unreachable
/// from any live ref; floating (commit-free) units are never orphaned.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OrphanVisibility {
    /// Default: hide commit-anchored units whose every current commit is orphaned.
    #[default]
    HideOrphans,
    /// Show everything (hidden + visible).
    All,
    /// Show only the orphaned units.
    OrphansOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RefFilter {
    name: String,
    mode: RefFilterMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionListOptions {
    repo: PathBuf,
    ref_filter: Option<RefFilter>,
    orphan_visibility: OrphanVisibility,
    integration_ref: Option<String>,
    worktree_scope: Option<PathBuf>,
    read_for_display: bool,
}

impl RevisionListOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            ref_filter: None,
            orphan_visibility: OrphanVisibility::default(),
            integration_ref: None,
            worktree_scope: None,
            read_for_display: false,
        }
    }

    /// Filter to units associated with `name`; the name is normalized to its full
    /// ref before matching the stored `ref_name`.
    pub fn with_ref_filter(mut self, name: impl Into<String>, mode: RefFilterMode) -> Self {
        self.ref_filter = Some(RefFilter {
            name: name.into(),
            mode,
        });
        self
    }

    /// Choose which units the list surfaces with respect to commit-reachability.
    pub fn with_orphan_visibility(mut self, visibility: OrphanVisibility) -> Self {
        self.orphan_visibility = visibility;
        self
    }

    /// Reachability target for the "merged" merge-status: a unit is merged only
    /// when an ancestor of this ref. Defaults to broad reachability (any live tip).
    pub fn with_integration_ref(mut self, integration_ref: impl Into<String>) -> Self {
        self.integration_ref = Some(integration_ref.into());
        self
    }

    /// Scope the listing to captures belonging to the worktree rooted at `path`
    /// (its canonical root + HEAD), via the shared worktree-identity match.
    pub fn with_worktree_scope(mut self, path: impl AsRef<Path>) -> Self {
        self.worktree_scope = Some(path.as_ref().to_path_buf());
        self
    }

    /// Read for a human-facing surface: skip a retired/unsupported event and
    /// surface it as a diagnostic instead of hard-failing the read. Off by
    /// default, so the relay and other strict callers keep the typed error.
    pub fn with_read_for_display(mut self, value: bool) -> Self {
        self.read_for_display = value;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionListEntry {
    pub captured_at: String,
    pub revision_id: RevisionId,
    pub object_id: ObjectId,
    pub source: RevisionSource,
    pub base: ReviewEndpoint,
    pub target: ReviewEndpoint,
    pub object_artifact_content_hash: String,
    /// Git-free commit-range lifecycle view for this unit (anchored/floating,
    /// current and withdrawn associations). Structural merge-status is attached
    /// separately in `merge_status`.
    pub commit_range: RevisionCommitRangeView,
    /// Structural merge-status from git reachability: `merged | open | orphaned |
    /// unknown`. `unknown` covers floating units, disagreeing per-commit
    /// conditions, and a repo error (which degrades gracefully, never an error).
    pub merge_status: String,
    /// The review units this entry stands for. Singleton (`[revision_id]`) for an
    /// ungrouped unit; for a unit whose current commit OID is shared by sibling
    /// captures (e.g. the same range captured in two worktrees, which mint distinct
    /// ids), this lists every member. The representative `revision_id` is the
    /// lexicographically smallest member, so the choice is deterministic and re-ID-free.
    pub grouped_revision_ids: Vec<RevisionId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionListResult {
    pub event_set_hash: String,
    pub event_count: usize,
    pub revision_count: usize,
    pub entries: Vec<RevisionListEntry>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn list_revisions(options: RevisionListOptions) -> Result<RevisionListResult> {
    let span = tracing::debug_span!("shore.revisions.list");
    let _guard = span.enter();

    let read_store = {
        let span = tracing::debug_span!("shore.revisions.list.resolve_read_store");
        let _guard = span.enter();
        resolve_read_store(&options.repo)?
    };
    let store = EventStore::from_backend(read_store.backend());
    let (events, skip_diagnostics) = {
        let span = tracing::debug_span!("shore.revisions.list.read_events");
        let _guard = span.enter();
        if options.read_for_display {
            let (events, skipped) = store.list_events_lenient()?;
            (events, skipped_to_diagnostics(skipped))
        } else {
            (store.list_events()?, Vec::new())
        }
    };
    let projection = {
        let span = tracing::debug_span!("shore.revisions.list.commit_range_projection");
        let _guard = span.enter();
        RevisionCommitRangeProjection::from_events(&events)?
    };
    let mut result = {
        let span = tracing::debug_span!("shore.revisions.list.from_events");
        let _guard = span.enter();
        list_from_events(&events, &projection)?
    };
    result.diagnostics.extend(skip_diagnostics);

    if let Some(ref_filter) = &options.ref_filter {
        let span = tracing::debug_span!("shore.revisions.list.ref_filter");
        let _guard = span.enter();
        let matching = revisions_matching_ref(
            &projection,
            &ref_filter.name,
            ref_filter.mode,
            &options.repo,
        )?;
        result
            .entries
            .retain(|entry| matching.contains(&entry.revision_id));
        result.revision_count = result.entries.len();
    }

    if let Some(worktree) = &options.worktree_scope {
        let span = tracing::debug_span!("shore.revisions.list.worktree_scope");
        let _guard = span.enter();
        let context = CurrentRevisionContext::for_repo(worktree)?;
        let in_scope = revision_ids_in_worktree(&events, &context)?;
        result
            .entries
            .retain(|entry| in_scope.contains(&entry.revision_id));
        result.revision_count = result.entries.len();
    }

    // Resolve the live tips and reachability ONCE for the whole list. The
    // orphan-visibility filter and the merge-status attach each classify every
    // entry's commits against this shared set by in-memory membership, instead of a
    // git ancestry probe per (commit, tip) pair per entry. A git failure here
    // degrades every entry to "reachability unknown" — the same graceful fallback
    // the per-entry path applies — so it is carried as `Option`, never propagated.
    let liveness = {
        let span = tracing::debug_span!("shore.revisions.list.liveness_batch");
        let _guard = span.enter();
        LivenessBatch::build(&options.repo, options.integration_ref.as_deref()).ok()
    };

    {
        let span = tracing::debug_span!("shore.revisions.list.orphan_visibility");
        let _guard = span.enter();
        apply_orphan_visibility(
            &mut result,
            &options.repo,
            options.orphan_visibility,
            liveness.as_ref(),
        );
    }

    // Canonical read-surface order: build entries → `--ref` retain → `--worktree`
    // identity retain → orphan-visibility retain → grouping → recompute count →
    // merge-status attach → divergence diagnostics. Grouping runs after every retain
    // (a `--ref`/`--worktree`/orphan filter that drops a member shrinks its group; a
    // group whose only surviving member matched still surfaces via that member), and
    // before merge-status, which is attached over the grouped entries.
    let grouping = {
        let span = tracing::debug_span!("shore.revisions.list.grouping_projection");
        let _guard = span.enter();
        CommitOidGroupingProjection::from_events(&events)?
    };
    result.entries = {
        let span = tracing::debug_span!("shore.revisions.list.group_entries");
        let _guard = span.enter();
        group_entries(result.entries, &grouping)
    };
    result.revision_count = result.entries.len();

    {
        let span = tracing::debug_span!("shore.revisions.list.attach_merge_status");
        let _guard = span.enter();
        attach_merge_status(&mut result, &options.repo, liveness.as_ref());
    }

    Ok(result)
}

/// Attach the structural merge-status to each surfaced entry. Run after the
/// visibility filter so hidden orphans are not classified twice. Reuses each
/// entry's `commit_range` view against the shared reachability batch.
fn attach_merge_status(
    result: &mut RevisionListResult,
    repo: &Path,
    liveness: Option<&LivenessBatch>,
) {
    for entry in &mut result.entries {
        entry.merge_status = merge_status_for(&entry.commit_range, repo, liveness).to_owned();
    }
}

/// The domain-named merge-status for a unit's current commit set, from the landed
/// liveness engine's agreed headline. `unknown` covers no commit anchor,
/// disagreeing per-commit conditions, a projection diagnostic, or an unavailable
/// repo (graceful degradation — never an error).
fn merge_status_for(
    view: &RevisionCommitRangeView,
    repo: &Path,
    liveness: Option<&LivenessBatch>,
) -> &'static str {
    match liveness.map(|batch| batch.enrich_merge(repo, view)) {
        Some(Ok(enrichment)) => match enrichment.headline {
            Some(CommitGraphCondition::Merged) => "merged",
            Some(CommitGraphCondition::Live) => "open",
            Some(CommitGraphCondition::Orphaned { .. }) => "orphaned",
            None => "unknown",
        },
        Some(Err(_)) | None => "unknown",
    }
}

/// Apply the orphan-visibility filter over the already-built entries. Default
/// hides commit-anchored units whose every current commit is orphaned; `All`
/// shows everything; `OrphansOnly` keeps only the orphaned ones. Reuses each
/// entry's `commit_range` view (no event re-list) against git reachability.
fn apply_orphan_visibility(
    result: &mut RevisionListResult,
    repo: &Path,
    visibility: OrphanVisibility,
    liveness: Option<&LivenessBatch>,
) {
    match visibility {
        OrphanVisibility::All => {}
        OrphanVisibility::HideOrphans => {
            result
                .entries
                .retain(|entry| !is_hidden_orphan(&entry.commit_range, repo, liveness));
            result.revision_count = result.entries.len();
        }
        OrphanVisibility::OrphansOnly => {
            result
                .entries
                .retain(|entry| is_hidden_orphan(&entry.commit_range, repo, liveness));
            result.revision_count = result.entries.len();
        }
    }
}

/// Whether a unit is a hidden orphan: commit-anchored with **every** current
/// commit classified `Orphaned` (any reason) by the shared reachability batch.
/// Floating units (no current commits) are never orphaned. A repo-unavailable
/// git error degrades to "not a hidden orphan" — never hide what we cannot
/// classify, and never error (graceful degradation).
fn is_hidden_orphan(
    view: &RevisionCommitRangeView,
    repo: &Path,
    liveness: Option<&LivenessBatch>,
) -> bool {
    if view.current_commits.is_empty() {
        return false;
    }
    match liveness.map(|batch| batch.enrich_broad(repo, view)) {
        Some(Ok(enrichment)) => enrichment
            .per_commit
            .iter()
            .all(|commit| matches!(commit.condition, CommitGraphCondition::Orphaned { .. })),
        Some(Err(_)) | None => false,
    }
}

/// Collapse capture entries that share a current commit OID into one entry per
/// group. Two worktree captures of the same range mint distinct revision_ids
/// (the identity fold is per-worktree, in `fingerprint.rs` — deliberately NOT
/// changed), but converge on a shared OID; this presents them as one row exposing
/// both ids. Floating captures and captures whose OID no sibling shares pass through
/// unchanged (singleton member set). No re-identification: the representative is just
/// the smallest member id, chosen for a deterministic row.
///
/// The representative entry's scalar fields (`object_artifact_content_hash`,
/// `target`, …) come from the smallest-id member. Same-range captures already share
/// one content-addressed object artifact (the body is decoupled from the identity
/// fields), so the artifact hash is identical across members — collapsing is honest,
/// not lossy. That honesty depends on the members sharing one pathspec scope:
/// differently scoped captures of the same range are distinct review units with
/// (in general) different content objects, so the union is partitioned by the
/// recorded `source.pathspecs` and never merges across scopes.
fn group_entries(
    entries: Vec<RevisionListEntry>,
    grouping: &CommitOidGroupingProjection,
) -> Vec<RevisionListEntry> {
    let by_id: BTreeMap<RevisionId, RevisionListEntry> = entries
        .into_iter()
        .map(|entry| (entry.revision_id.clone(), entry))
        .collect();

    let mut grouped: Vec<RevisionListEntry> = Vec::new();
    for members in connected_components(&by_id, grouping) {
        // `members` is a non-empty ordered set; the representative is the smallest id,
        // so the row is stable across runs.
        let representative = members
            .iter()
            .next()
            .expect("a component has at least one member")
            .clone();
        let mut entry = by_id
            .get(&representative)
            .expect("representative is a known entry")
            .clone();
        entry.grouped_revision_ids = members.into_iter().collect();
        debug_assert!(
            entry.grouped_revision_ids.contains(&entry.revision_id),
            "the member set always contains the representative id"
        );
        grouped.push(entry);
    }

    grouped.sort_by(|left, right| {
        left.captured_at
            .cmp(&right.captured_at)
            .then_with(|| left.revision_id.as_str().cmp(right.revision_id.as_str()))
    });
    grouped
}

/// Partition the known entry ids into connected components over the "shares any
/// current commit OID within one pathspec scope" relation. Each entry seeds its
/// own component; for every grouping bucket (`commit_oid → member ids`) that
/// names two or more known entries with an equal recorded `source.pathspecs`
/// set, those entries' components are unioned. Differently scoped members of a
/// bucket never union — a path-scoped capture is its own review unit. A unit
/// with multiple current OIDs chains its buckets into one component (transitive
/// closure). Ids the grouping names that are not entries in this view (filtered
/// out upstream) are ignored — a group whose only surviving member matched
/// collapses to a singleton.
fn connected_components(
    by_id: &BTreeMap<RevisionId, RevisionListEntry>,
    grouping: &CommitOidGroupingProjection,
) -> Vec<BTreeSet<RevisionId>> {
    // id → component index, seeded one-per-entry.
    let mut component_of: BTreeMap<RevisionId, usize> = by_id
        .keys()
        .cloned()
        .enumerate()
        .map(|(index, id)| (id, index))
        .collect();

    for members in grouping.groups.values() {
        let mut by_scope: BTreeMap<&[String], Vec<RevisionId>> = BTreeMap::new();
        for id in members {
            if let Some(entry) = by_id.get(id) {
                by_scope
                    .entry(source_pathspecs(&entry.source))
                    .or_default()
                    .push(id.clone());
            }
        }
        for known in by_scope.into_values() {
            let mut known = known.into_iter();
            if let Some(first) = known.next() {
                let target = component_of[&first];
                for other in known {
                    let source = component_of[&other];
                    if source != target {
                        for value in component_of.values_mut() {
                            if *value == source {
                                *value = target;
                            }
                        }
                    }
                }
            }
        }
    }

    let mut buckets: BTreeMap<usize, BTreeSet<RevisionId>> = BTreeMap::new();
    for (id, index) in component_of {
        buckets.entry(index).or_default().insert(id);
    }
    buckets.into_values().collect()
}

/// The recorded capture pathspec scope of a listed source; empty means the
/// whole repository (an unscoped capture).
fn source_pathspecs(source: &RevisionSource) -> &[String] {
    match source {
        RevisionSource::GitWorktree { pathspecs, .. }
        | RevisionSource::GitCommitRange { pathspecs, .. }
        | RevisionSource::GitRootCommit { pathspecs, .. } => pathspecs,
    }
}

/// Convenience entry point for "which units are associated with this ref?".
/// Delegates to [`list_revisions`] with a `--ref` filter applied.
pub fn list_units_for_ref(
    repo: impl AsRef<Path>,
    ref_name: impl Into<String>,
    mode: RefFilterMode,
) -> Result<RevisionListResult> {
    list_revisions(RevisionListOptions::new(repo).with_ref_filter(ref_name, mode))
}

/// The review-unit ids matching a ref under the chosen mode. The name is
/// normalized to its full ref first. `Label` is fully offline (current ref
/// labels); `Liveness` joins `enrich_liveness` against the ref's tip and keeps
/// units with at least one reachable commit. Shared by `unit list` and history.
pub(crate) fn revisions_matching_ref(
    projection: &RevisionCommitRangeProjection,
    name: &str,
    mode: RefFilterMode,
    repo: &Path,
) -> Result<BTreeSet<RevisionId>> {
    let normalized_ref = normalize_ref(name);
    match mode {
        RefFilterMode::Label => Ok(projection
            .units_for_ref(&normalized_ref)
            .into_iter()
            .map(|view| view.revision_id.clone())
            .collect()),
        RefFilterMode::Liveness => {
            let mut matching = BTreeSet::new();
            for view in projection.units.values() {
                let enrichment = enrich_liveness(view, repo, Some(&normalized_ref))?;
                if enrichment.per_commit.iter().any(|commit| {
                    matches!(
                        commit.condition,
                        CommitGraphCondition::Merged | CommitGraphCondition::Live
                    )
                }) {
                    matching.insert(view.revision_id.clone());
                }
            }
            Ok(matching)
        }
    }
}

fn list_from_events(
    events: &[ShoreEvent],
    projection: &RevisionCommitRangeProjection,
) -> Result<RevisionListResult> {
    let state = SessionState::from_events(events)?;
    let event_set_hash = state
        .event_set_hash
        .clone()
        .expect("SessionState::from_events sets event_set_hash");

    let mut entries = events
        .iter()
        .filter(|event| event.event_type == EventType::WorkObjectProposed)
        .filter_map(|event| entry_from_event(event, projection).transpose())
        .collect::<Result<Vec<_>>>()?;

    entries.sort_by(|left, right| {
        left.captured_at
            .cmp(&right.captured_at)
            .then_with(|| left.revision_id.as_str().cmp(right.revision_id.as_str()))
    });

    Ok(RevisionListResult {
        event_set_hash,
        event_count: events.len(),
        revision_count: entries.len(),
        entries,
        diagnostics: state.diagnostics,
    })
}

fn entry_from_event(
    event: &ShoreEvent,
    projection: &RevisionCommitRangeProjection,
) -> Result<Option<RevisionListEntry>> {
    let payload: WorkObjectProposedPayload = serde_json::from_value(event.payload.clone())?;
    let WorkObjectProposal::Revision {
        revision,
        object_artifact_content_hash,
        ..
    } = payload.work_object
    else {
        // A generative move proposing a task attempt is not a review revision;
        // the review listing skips task-domain proposals rather than failing.
        return Ok(None);
    };
    let provenance = revision.git_provenance.ok_or_else(|| {
        crate::error::ShoreError::Message(
            "review unit listing requires git provenance for a captured revision".to_owned(),
        )
    })?;
    let commit_range = projection
        .unit(&revision.id)
        .cloned()
        .unwrap_or_else(|| empty_view(revision.id.clone()));
    let revision_id = revision.id;
    Ok(Some(RevisionListEntry {
        captured_at: event.occurred_at.clone(),
        revision_id: revision_id.clone(),
        object_id: revision.object_id,
        source: provenance.source,
        base: provenance.base,
        target: provenance.target,
        object_artifact_content_hash,
        commit_range,
        // Filled by `attach_merge_status` after the visibility filter.
        merge_status: String::new(),
        // Every entry starts standing only for itself; the grouping pass rewrites this
        // for entries whose current commit OID is shared by sibling captures.
        grouped_revision_ids: vec![revision_id],
    }))
}

fn empty_view(revision_id: RevisionId) -> RevisionCommitRangeView {
    RevisionCommitRangeView {
        revision_id,
        anchored: false,
        current_commits: Vec::new(),
        current_refs: Vec::new(),
        withdrawn_commits: Vec::new(),
        withdrawn_refs: Vec::new(),
        diagnostics: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        EngagementId, JournalId, ReviewEndpoint, RevisionSource, TargetRef, TaskTargetRef,
        WorkObjectId, WorktreeCaptureMode,
    };
    use crate::session::event::{EventTarget, GitProvenance, Revision, Writer};

    #[test]
    fn empty_event_set_returns_no_entries() {
        let result = list_from_events(
            &[],
            &RevisionCommitRangeProjection::from_events(&[]).unwrap(),
        )
        .unwrap();

        assert_eq!(result.event_count, 0);
        assert_eq!(result.revision_count, 0);
        assert!(result.entries.is_empty());
        assert!(result.event_set_hash.starts_with("sha256:"));
    }

    #[test]
    fn list_revisions_strict_by_default_lenient_when_opted_in() {
        let repo = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(repo.path())
            .output()
            .unwrap();

        // A retired-type event in the resolved store: the probe rejects it.
        let events_dir = resolve_read_store(repo.path())
            .unwrap()
            .store_dir()
            .join("events");
        std::fs::create_dir_all(&events_dir).unwrap();
        std::fs::write(
            events_dir.join(format!("{}.json", "a".repeat(64))),
            br#"{"eventType":"review_disposition_recorded"}"#,
        )
        .unwrap();

        // Default (the relay/strict case): a retired event hard-fails the read.
        assert!(list_revisions(RevisionListOptions::new(repo.path())).is_err());

        // Opted in (CLI/inspector): the retired event is skipped and surfaced.
        let result =
            list_revisions(RevisionListOptions::new(repo.path()).with_read_for_display(true))
                .unwrap();
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "unsupported_event_type")
        );
    }

    #[test]
    fn includes_only_revision_captured_events() {
        let capture = captured_event("a", "2026-05-13T10:00:00Z");
        let events = [capture];
        let projection = RevisionCommitRangeProjection::from_events(&events).unwrap();
        let result = list_from_events(&events, &projection).unwrap();

        assert_eq!(result.event_count, 1);
        assert_eq!(result.revision_count, 1);
        assert_eq!(
            result.entries[0].revision_id.as_str(),
            "review-unit:sha256:a"
        );
        assert_eq!(result.entries[0].captured_at, "2026-05-13T10:00:00Z");
        assert_eq!(
            result.entries[0].object_artifact_content_hash,
            "sha256:artifact:a"
        );
    }

    #[test]
    fn sorts_entries_by_captured_at_then_revision_id() {
        let later = captured_event("z-later", "2026-05-13T10:00:05Z");
        let tie_b = captured_event("b-tie", "2026-05-13T10:00:01Z");
        let tie_a = captured_event("a-tie", "2026-05-13T10:00:01Z");

        let events = [later, tie_b, tie_a];
        let projection = RevisionCommitRangeProjection::from_events(&events).unwrap();
        let result = list_from_events(&events, &projection).unwrap();

        let order: Vec<&str> = result
            .entries
            .iter()
            .map(|entry| entry.revision_id.as_str())
            .collect();
        assert_eq!(
            order,
            vec![
                "review-unit:sha256:a-tie",
                "review-unit:sha256:b-tie",
                "review-unit:sha256:z-later",
            ]
        );
    }

    #[test]
    fn entry_serializes_with_camel_case_and_no_internal_paths() {
        let events = [captured_event("one", "2026-05-13T10:00:00Z")];
        let projection = RevisionCommitRangeProjection::from_events(&events).unwrap();
        let result = list_from_events(&events, &projection).unwrap();
        let json = serde_json::to_string(&result.entries[0]).unwrap();

        assert!(json.contains("revisionId"));
        assert!(!json.contains("reviewUnitId"));
        assert!(json.contains("capturedAt"));
        assert!(json.contains("objectArtifactContentHash"));
        assert!(!json.contains("artifacts/"));
        assert!(!json.contains("statePath"));
        assert!(!json.contains("payloadHash"));
    }

    fn captured_event(suffix: &str, occurred_at: &str) -> ShoreEvent {
        // A real capture stamps the envelope subject and the payload revision with
        // one minted id; the listing reads the revision from the payload, so both
        // carry the same id here.
        let revision_id = RevisionId::new(format!("review-unit:sha256:{suffix}"));
        let object_id = ObjectId::new(format!("obj:sha256:{suffix}"));
        let payload = WorkObjectProposedPayload {
            engagement_id: EngagementId::new(format!(
                "engagement:sha256:{}",
                crate::canonical_hash::sha256_bytes_hex((revision_id.clone()).as_str().as_bytes())
            )),
            work_object: WorkObjectProposal::Revision {
                revision: Revision {
                    id: revision_id.clone(),
                    object_id: object_id.clone(),
                    git_provenance: Some(GitProvenance {
                        source: RevisionSource::GitWorktree {
                            mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                            include_untracked: true,
                            pathspecs: Vec::new(),
                        },
                        base: ReviewEndpoint::GitCommit {
                            commit_oid: format!("base:{suffix}"),
                            tree_oid: format!("base-tree:{suffix}"),
                        },
                        target: ReviewEndpoint::GitWorkingTree {
                            worktree_root: "/repo".to_owned(),
                        },
                    }),
                },
                object_artifact_content_hash: format!("sha256:artifact:{suffix}"),
                supersedes: vec![],
            },
        };
        ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("capture:{suffix}"),
            EventTarget::for_revision(JournalId::new("journal:default"), revision_id, None)
                .unwrap(),
            Writer::shore_local("test"),
            payload,
            occurred_at,
        )
        .unwrap()
    }

    /// A generative move that proposes a task attempt rather than a review
    /// revision: it carries a Task subject and the TaskAttempt payload arm. A
    /// review listing must skip it, not fail to decode it as a revision.
    fn task_attempt_event(suffix: &str, occurred_at: &str) -> ShoreEvent {
        let payload = WorkObjectProposedPayload {
            engagement_id: EngagementId::new(format!("engagement:sha256:{suffix}")),
            work_object: WorkObjectProposal::TaskAttempt {
                task_attempt_id: WorkObjectId::new(format!("task-attempt:sha256:{suffix}")),
                project_path: "/repo".to_owned(),
                claude_session_uuid: format!("uuid-{suffix}"),
                initial_prompt_hash: format!("sha256:prompt:{suffix}"),
                predecessor: None,
                base_state_fingerprint: None,
                source_speaker: None,
            },
        };
        ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("task-capture:{suffix}"),
            EventTarget::for_subject(
                JournalId::new("journal:default"),
                TargetRef::Task(TaskTargetRef::TaskAttempt {
                    task_attempt_id: WorkObjectId::new(format!("task-attempt:sha256:{suffix}")),
                }),
                None,
            )
            .unwrap(),
            Writer::shore_local("test"),
            payload,
            occurred_at,
        )
        .unwrap()
    }

    #[test]
    fn skips_task_attempt_proposals_in_a_mixed_store() {
        // One event type now carries both review and task generative moves. The
        // review listing surfaces only the review revisions and never errors on a
        // task-attempt proposal sharing the store.
        let review = captured_event("rev", "2026-05-13T10:00:00Z");
        let task = task_attempt_event("task", "2026-05-13T10:00:01Z");
        let events = [review, task];
        let projection = RevisionCommitRangeProjection::from_events(&events).unwrap();

        let result = list_from_events(&events, &projection).unwrap();

        assert_eq!(result.revision_count, 1);
        assert_eq!(
            result.entries[0].revision_id.as_str(),
            "review-unit:sha256:rev"
        );
    }

    use std::path::Path;
    use std::process::Command;

    use tempfile::TempDir;

    use crate::git::{Ancestry, git_is_ancestor};
    use crate::model::{CommitAssociationId, CommitRangeCaptureMode, ReviewTargetRef};
    use crate::session::event::RevisionCommitAssociatedPayload;

    /// A repo whose `main` is `base → mid → tip`, plus a dangling commit (child of
    /// tip whose branch was deleted) so it exists but no live ref reaches it.
    struct OrphanRepo {
        root: TempDir,
    }

    impl OrphanRepo {
        fn new() -> Self {
            let repo = Self {
                root: TempDir::new().unwrap(),
            };
            repo.git(["init"]);
            repo.git(["config", "user.name", "Shore Tests"]);
            repo.git(["config", "user.email", "shore-tests@example.com"]);
            repo.git(["config", "commit.gpgsign", "false"]);
            repo.commit("base", "base\n");
            repo.git(["branch", "-M", "main"]);
            repo.commit("mid", "mid\n");
            repo.commit("tip", "tip\n");
            repo.git(["checkout", "-b", "tmp"]);
            repo.commit("dangling", "dangling\n");
            repo.git(["checkout", "main"]);
            repo.git(["branch", "-D", "tmp"]);
            // A second live branch `other` forking at mid (base → mid → feat1 →
            // feat2), so a commit can be merged into one branch but not main.
            repo.git(["checkout", "-b", "other", "main~1"]);
            repo.commit("feat1", "feat1\n");
            repo.commit("feat2", "feat2\n");
            repo.git(["checkout", "main"]);
            repo
        }

        fn path(&self) -> &Path {
            self.root.path()
        }

        fn commit(&self, message: &str, contents: &str) {
            std::fs::write(self.path().join("file.txt"), contents).unwrap();
            self.git(["add", "--all"]);
            self.git(["commit", "-m", message]);
        }

        fn oid(&self, rev: &str) -> String {
            let output = Command::new("git")
                .args(["rev-parse", "--verify", rev])
                .current_dir(self.path())
                .output()
                .unwrap();
            assert!(output.status.success(), "git rev-parse {rev} failed");
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        }

        /// The OID of the dangling commit (child of tip, branch deleted), found by
        /// scanning the reflog for the unreachable child of `main`.
        fn dangling_oid(&self) -> String {
            let output = Command::new("git")
                .args(["log", "-g", "--format=%H"])
                .current_dir(self.path())
                .output()
                .unwrap();
            let reflog = String::from_utf8(output.stdout).unwrap();
            let tip = self.oid("main");
            reflog
                .lines()
                .map(str::to_owned)
                .find(|oid| {
                    *oid != tip
                        && git_is_ancestor(self.path(), oid, &tip).unwrap() == Ancestry::NotAncestor
                        && git_is_ancestor(self.path(), &tip, oid).unwrap() == Ancestry::Ancestor
                })
                .expect("a dangling child of tip is in the reflog")
        }

        fn git<I, S>(&self, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<std::ffi::OsStr>,
        {
            let status = Command::new("git")
                .args(args)
                .current_dir(self.path())
                .status()
                .unwrap();
            assert!(status.success());
        }
    }

    /// A commit-range capture anchored to `commit_oid` (a `GitCommit` target, which
    /// seeds the unit's `current_commits`).
    fn range_captured_event(suffix: &str, occurred_at: &str, commit_oid: &str) -> ShoreEvent {
        // One minted id stamps both the envelope subject and the payload revision.
        let revision_id = RevisionId::new(format!("review-unit:sha256:{suffix}"));
        let object_id = ObjectId::new(format!("obj:sha256:{suffix}"));
        let payload = WorkObjectProposedPayload {
            engagement_id: EngagementId::new(format!(
                "engagement:sha256:{}",
                crate::canonical_hash::sha256_bytes_hex((revision_id.clone()).as_str().as_bytes())
            )),
            work_object: WorkObjectProposal::Revision {
                revision: Revision {
                    id: revision_id.clone(),
                    object_id: object_id.clone(),
                    git_provenance: Some(GitProvenance {
                        source: RevisionSource::GitCommitRange {
                            mode: CommitRangeCaptureMode::BaseTreeToTargetTree,
                            pathspecs: Vec::new(),
                        },
                        base: ReviewEndpoint::GitCommit {
                            commit_oid: format!("base:{suffix}"),
                            tree_oid: format!("base-tree:{suffix}"),
                        },
                        target: ReviewEndpoint::GitCommit {
                            commit_oid: commit_oid.to_owned(),
                            tree_oid: format!("{commit_oid}-tree"),
                        },
                    }),
                },
                object_artifact_content_hash: format!("sha256:artifact:{suffix}"),
                supersedes: vec![],
            },
        };
        ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("capture:{suffix}"),
            EventTarget::for_revision(JournalId::new("journal:default"), revision_id, None)
                .unwrap(),
            Writer::shore_local("test"),
            payload,
            occurred_at,
        )
        .unwrap()
    }

    /// Adds a second current commit to an existing unit via a commit association.
    fn commit_associated_event(suffix: &str, commit_oid: &str) -> ShoreEvent {
        let revision_id = RevisionId::new(format!("review-unit:sha256:{suffix}"));
        let payload = RevisionCommitAssociatedPayload {
            commit_association_id: CommitAssociationId::new(format!(
                "commit-association:sha256:{suffix}:{commit_oid}"
            )),
            target: ReviewTargetRef::Revision {
                revision_id: revision_id.clone(),
            },
            commit: ReviewEndpoint::GitCommit {
                commit_oid: commit_oid.to_owned(),
                tree_oid: format!("{commit_oid}-tree"),
            },
        };
        ShoreEvent::new(
            EventType::RevisionCommitAssociated,
            RevisionCommitAssociatedPayload::idempotency_key(&revision_id, commit_oid),
            EventTarget::for_revision(JournalId::new("journal:default"), revision_id, None)
                .unwrap(),
            Writer::shore_local("test"),
            payload,
            "2026-05-13T10:00:09Z",
        )
        .unwrap()
    }

    fn listed(events: &[ShoreEvent], visibility: OrphanVisibility, repo: &Path) -> Vec<String> {
        let projection = RevisionCommitRangeProjection::from_events(events).unwrap();
        let mut result = list_from_events(events, &projection).unwrap();
        let liveness = LivenessBatch::build(repo, None).ok();
        apply_orphan_visibility(&mut result, repo, visibility, liveness.as_ref());
        assert_eq!(result.revision_count, result.entries.len());
        result
            .entries
            .iter()
            .map(|entry| entry.revision_id.as_str().to_owned())
            .collect()
    }

    #[test]
    fn orphan_capture_is_hidden_by_default() {
        let repo = OrphanRepo::new();
        let dangling = repo.dangling_oid();
        let tip = repo.oid("main");
        let events = [
            range_captured_event("orph", "2026-05-13T10:00:00Z", &dangling),
            captured_event("float", "2026-05-13T10:00:01Z"),
            range_captured_event("live", "2026-05-13T10:00:02Z", &tip),
        ];

        let ids = listed(&events, OrphanVisibility::HideOrphans, repo.path());

        assert!(ids.contains(&"review-unit:sha256:float".to_owned()));
        assert!(ids.contains(&"review-unit:sha256:live".to_owned()));
        assert!(!ids.contains(&"review-unit:sha256:orph".to_owned()));
    }

    #[test]
    fn orphan_capture_is_shown_with_all() {
        let repo = OrphanRepo::new();
        let dangling = repo.dangling_oid();
        let tip = repo.oid("main");
        let events = [
            range_captured_event("orph", "2026-05-13T10:00:00Z", &dangling),
            captured_event("float", "2026-05-13T10:00:01Z"),
            range_captured_event("live", "2026-05-13T10:00:02Z", &tip),
        ];

        let ids = listed(&events, OrphanVisibility::All, repo.path());

        assert!(ids.contains(&"review-unit:sha256:orph".to_owned()));
        assert!(ids.contains(&"review-unit:sha256:float".to_owned()));
        assert!(ids.contains(&"review-unit:sha256:live".to_owned()));
    }

    #[test]
    fn orphans_flag_shows_only_orphaned() {
        let repo = OrphanRepo::new();
        let dangling = repo.dangling_oid();
        let tip = repo.oid("main");
        let events = [
            range_captured_event("orph", "2026-05-13T10:00:00Z", &dangling),
            captured_event("float", "2026-05-13T10:00:01Z"),
            range_captured_event("live", "2026-05-13T10:00:02Z", &tip),
        ];

        let ids = listed(&events, OrphanVisibility::OrphansOnly, repo.path());

        assert_eq!(ids, vec!["review-unit:sha256:orph".to_owned()]);
    }

    #[test]
    fn floating_capture_is_never_hidden() {
        let repo = OrphanRepo::new();
        let events = [captured_event("float", "2026-05-13T10:00:00Z")];

        let default_ids = listed(&events, OrphanVisibility::HideOrphans, repo.path());
        assert!(default_ids.contains(&"review-unit:sha256:float".to_owned()));

        let orphan_ids = listed(&events, OrphanVisibility::OrphansOnly, repo.path());
        assert!(orphan_ids.is_empty());
    }

    #[test]
    fn live_capture_is_never_hidden() {
        let repo = OrphanRepo::new();
        let tip = repo.oid("main");
        let events = [range_captured_event("live", "2026-05-13T10:00:00Z", &tip)];

        let default_ids = listed(&events, OrphanVisibility::HideOrphans, repo.path());
        assert!(default_ids.contains(&"review-unit:sha256:live".to_owned()));

        let orphan_ids = listed(&events, OrphanVisibility::OrphansOnly, repo.path());
        assert!(orphan_ids.is_empty());
    }

    #[test]
    fn gone_commit_is_hidden_and_degrades_without_error() {
        let repo = OrphanRepo::new();
        let missing = "0".repeat(repo.oid("main").len());
        let events = [range_captured_event(
            "gone",
            "2026-05-13T10:00:00Z",
            &missing,
        )];

        // A gc'd (object-missing) commit classifies Orphaned and is hidden by
        // default; enrich_liveness returns Ok, so the list never errors.
        let default_ids = listed(&events, OrphanVisibility::HideOrphans, repo.path());
        assert!(default_ids.is_empty());

        let orphan_ids = listed(&events, OrphanVisibility::OrphansOnly, repo.path());
        assert_eq!(orphan_ids, vec!["review-unit:sha256:gone".to_owned()]);
    }

    #[test]
    fn partial_orphan_is_not_hidden() {
        let repo = OrphanRepo::new();
        let mid = repo.oid("main~1");
        let dangling = repo.dangling_oid();
        // One current commit Merged (mid), one Orphaned (dangling) → not every
        // current commit is orphaned, so the unit is not hidden.
        let events = [
            range_captured_event("mix", "2026-05-13T10:00:00Z", &mid),
            commit_associated_event("mix", &dangling),
        ];

        let default_ids = listed(&events, OrphanVisibility::HideOrphans, repo.path());
        assert!(default_ids.contains(&"review-unit:sha256:mix".to_owned()));
    }

    /// Surface every entry (so orphaned units are present too) and attach
    /// merge-status, returning `(revision_id, merge_status)` pairs.
    fn merge_statuses(
        events: &[ShoreEvent],
        repo: &Path,
        integration_ref: Option<&str>,
    ) -> Vec<(String, String)> {
        let projection = RevisionCommitRangeProjection::from_events(events).unwrap();
        let mut result = list_from_events(events, &projection).unwrap();
        let liveness = LivenessBatch::build(repo, integration_ref).ok();
        apply_orphan_visibility(&mut result, repo, OrphanVisibility::All, liveness.as_ref());
        attach_merge_status(&mut result, repo, liveness.as_ref());
        result
            .entries
            .iter()
            .map(|entry| {
                (
                    entry.revision_id.as_str().to_owned(),
                    entry.merge_status.clone(),
                )
            })
            .collect()
    }

    #[test]
    fn list_entries_carry_merge_status() {
        let repo = OrphanRepo::new();
        let mid = repo.oid("main~1");
        let tip = repo.oid("main");
        let dangling = repo.dangling_oid();
        let events = [
            range_captured_event("merged", "2026-05-13T10:00:00Z", &mid),
            range_captured_event("open", "2026-05-13T10:00:01Z", &tip),
            range_captured_event("orphan", "2026-05-13T10:00:02Z", &dangling),
            captured_event("float", "2026-05-13T10:00:03Z"),
        ];

        let statuses = merge_statuses(&events, repo.path(), None);
        let status_of = |id: &str| {
            statuses
                .iter()
                .find(|(unit, _)| unit == id)
                .map(|(_, status)| status.as_str())
                .unwrap()
        };

        assert_eq!(status_of("review-unit:sha256:merged"), "merged");
        assert_eq!(status_of("review-unit:sha256:open"), "open");
        assert_eq!(status_of("review-unit:sha256:orphan"), "orphaned");
        assert_eq!(status_of("review-unit:sha256:float"), "unknown");
    }

    #[test]
    fn integration_ref_narrows_merged() {
        let repo = OrphanRepo::new();
        // feat1 is merged into `other` (a live tip) but is not an ancestor of main.
        let feat1 = repo.oid("other~1");
        let events = [range_captured_event("c", "2026-05-13T10:00:00Z", &feat1)];

        let broad = merge_statuses(&events, repo.path(), None);
        assert_eq!(broad[0].1, "merged");

        let narrow = merge_statuses(&events, repo.path(), Some("refs/heads/main"));
        assert_eq!(narrow[0].1, "orphaned");
    }

    #[test]
    fn repo_unavailable_merge_status_is_unknown_not_error() {
        let non_repo = TempDir::new().unwrap();
        let events = [range_captured_event(
            "c",
            "2026-05-13T10:00:00Z",
            &"a".repeat(40),
        )];

        // enrich_liveness errors against a non-repo path; the status degrades to
        // "unknown" and the list does not error.
        let statuses = merge_statuses(&events, non_repo.path(), None);
        assert_eq!(
            statuses,
            vec![("review-unit:sha256:c".to_owned(), "unknown".to_owned())]
        );
    }

    #[test]
    fn merge_status_serializes_camel_case() {
        let repo = OrphanRepo::new();
        let tip = repo.oid("main");
        let events = [range_captured_event("c", "2026-05-13T10:00:00Z", &tip)];
        let projection = RevisionCommitRangeProjection::from_events(&events).unwrap();
        let mut result = list_from_events(&events, &projection).unwrap();
        let liveness = LivenessBatch::build(repo.path(), None).ok();
        attach_merge_status(&mut result, repo.path(), liveness.as_ref());

        let json = serde_json::to_string(&result.entries[0]).unwrap();
        assert!(json.contains("\"mergeStatus\""));
        assert!(json.contains("\"open\""));
    }

    /// A worktree capture (floating until a commit is associated) for an explicit id,
    /// so tests can mint the distinct ids two worktrees would produce for one range.
    fn worktree_capture_for(unit: &RevisionId, occurred_at: &str) -> ShoreEvent {
        // The envelope subject and the payload revision carry the same minted id, so
        // the listing keys this capture by `unit` (its associations target `unit` too).
        let revision_id = unit.clone();
        let object_id = ObjectId::new(format!("obj:{}", unit.as_str()));
        let payload = WorkObjectProposedPayload {
            engagement_id: EngagementId::new(format!(
                "engagement:sha256:{}",
                crate::canonical_hash::sha256_bytes_hex((revision_id.clone()).as_str().as_bytes())
            )),
            work_object: WorkObjectProposal::Revision {
                revision: Revision {
                    id: revision_id.clone(),
                    object_id: object_id.clone(),
                    git_provenance: Some(GitProvenance {
                        source: RevisionSource::GitWorktree {
                            mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                            include_untracked: true,
                            pathspecs: Vec::new(),
                        },
                        base: ReviewEndpoint::GitCommit {
                            commit_oid: format!("base:{}", unit.as_str()),
                            tree_oid: format!("base-tree:{}", unit.as_str()),
                        },
                        target: ReviewEndpoint::GitWorkingTree {
                            worktree_root: "/repo".to_owned(),
                        },
                    }),
                },
                object_artifact_content_hash: format!("sha256:artifact:{}", unit.as_str()),
                supersedes: vec![],
            },
        };
        ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("capture:{}", unit.as_str()),
            EventTarget::for_revision(JournalId::new("journal:default"), unit.clone(), None)
                .unwrap(),
            Writer::shore_local("test"),
            payload,
            occurred_at,
        )
        .unwrap()
    }

    /// Associate `commit_oid` onto an existing unit (adds it to the unit's current set).
    fn commit_associated_for(unit: &RevisionId, commit_oid: &str) -> ShoreEvent {
        let payload = RevisionCommitAssociatedPayload {
            commit_association_id: CommitAssociationId::new(format!(
                "commit-association:sha256:{}:{commit_oid}",
                unit.as_str()
            )),
            target: ReviewTargetRef::Revision {
                revision_id: unit.clone(),
            },
            commit: ReviewEndpoint::GitCommit {
                commit_oid: commit_oid.to_owned(),
                tree_oid: format!("{commit_oid}-tree"),
            },
        };
        ShoreEvent::new(
            EventType::RevisionCommitAssociated,
            RevisionCommitAssociatedPayload::idempotency_key(unit, commit_oid),
            EventTarget::for_revision(JournalId::new("journal:default"), unit.clone(), None)
                .unwrap(),
            Writer::shore_local("test"),
            payload,
            "2026-06-19T00:00:09Z",
        )
        .unwrap()
    }

    /// A commit-range capture anchored to `commit_oid` with a recorded pathspec
    /// scope. Distinct suffixes model distinct scoped captures of one range.
    fn scoped_range_captured_event(
        suffix: &str,
        occurred_at: &str,
        commit_oid: &str,
        pathspecs: &[&str],
    ) -> ShoreEvent {
        let revision_id = RevisionId::new(format!("review-unit:sha256:{suffix}"));
        let object_id = ObjectId::new(format!("obj:sha256:{suffix}"));
        let payload = WorkObjectProposedPayload {
            engagement_id: EngagementId::new(format!(
                "engagement:sha256:{}",
                crate::canonical_hash::sha256_bytes_hex((revision_id.clone()).as_str().as_bytes())
            )),
            work_object: WorkObjectProposal::Revision {
                revision: Revision {
                    id: revision_id.clone(),
                    object_id: object_id.clone(),
                    git_provenance: Some(GitProvenance {
                        source: RevisionSource::GitCommitRange {
                            mode: CommitRangeCaptureMode::BaseTreeToTargetTree,
                            pathspecs: pathspecs.iter().map(|s| (*s).to_owned()).collect(),
                        },
                        base: ReviewEndpoint::GitCommit {
                            commit_oid: "base:shared".to_owned(),
                            tree_oid: "base-tree:shared".to_owned(),
                        },
                        target: ReviewEndpoint::GitCommit {
                            commit_oid: commit_oid.to_owned(),
                            tree_oid: format!("{commit_oid}-tree"),
                        },
                    }),
                },
                object_artifact_content_hash: format!("sha256:artifact:{suffix}"),
                supersedes: vec![],
            },
        };
        ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("capture:{suffix}"),
            EventTarget::for_revision(JournalId::new("journal:default"), revision_id, None)
                .unwrap(),
            Writer::shore_local("test"),
            payload,
            occurred_at,
        )
        .unwrap()
    }

    #[test]
    fn scoped_range_captures_with_different_pathspecs_stay_separate_entries() {
        // Two path-scoped captures of the SAME range are different review units
        // with different content objects; collapsing them would hide one unit and
        // surface only one member's source.pathspecs. They must not group even
        // though their current commit sets share the target OID.
        let events = [
            scoped_range_captured_event("scope-a", "2026-06-19T00:00:00Z", "shared", &["a"]),
            scoped_range_captured_event("scope-b", "2026-06-19T00:00:01Z", "shared", &["b"]),
        ];
        let projection = RevisionCommitRangeProjection::from_events(&events).unwrap();
        let grouping = CommitOidGroupingProjection::from_events(&events).unwrap();
        let base = list_from_events(&events, &projection).unwrap();

        let grouped = group_entries(base.entries, &grouping);

        assert_eq!(
            grouped.len(),
            2,
            "differently scoped captures of one range stay separate entries"
        );
        let mut scopes: Vec<Vec<String>> = grouped
            .iter()
            .map(|entry| match &entry.source {
                RevisionSource::GitCommitRange { pathspecs, .. } => pathspecs.clone(),
                RevisionSource::GitWorktree { pathspecs, .. } => pathspecs.clone(),
                RevisionSource::GitRootCommit { pathspecs, .. } => pathspecs.clone(),
            })
            .collect();
        scopes.sort();
        assert_eq!(scopes, vec![vec!["a".to_owned()], vec!["b".to_owned()]]);
        for entry in &grouped {
            assert_eq!(entry.grouped_revision_ids, vec![entry.revision_id.clone()]);
        }
    }

    #[test]
    fn scoped_captures_with_equal_pathspecs_still_group_on_a_shared_oid() {
        // The cross-worktree convergence story is unchanged within one scope:
        // equal pathspec sets sharing an OID still collapse to one entry.
        let events = [
            scoped_range_captured_event("scope-a1", "2026-06-19T00:00:00Z", "shared", &["a"]),
            scoped_range_captured_event("scope-a2", "2026-06-19T00:00:01Z", "shared", &["a"]),
        ];
        let projection = RevisionCommitRangeProjection::from_events(&events).unwrap();
        let grouping = CommitOidGroupingProjection::from_events(&events).unwrap();
        let base = list_from_events(&events, &projection).unwrap();

        let grouped = group_entries(base.entries, &grouping);

        assert_eq!(grouped.len(), 1, "equal scopes still collapse");
        assert_eq!(grouped[0].grouped_revision_ids.len(), 2);
    }

    #[test]
    fn cross_worktree_same_range_captures_present_as_one_grouped_entry() {
        // Two captures (distinct revision_ids, as two worktrees would mint) whose
        // current commit sets both contain the same OID collapse into ONE list entry
        // that exposes BOTH ids in its grouped-member set. One shared artifact, two
        // capture events — no re-ID.
        let unit_a = RevisionId::new("review-unit:sha256:a");
        let unit_b = RevisionId::new("review-unit:sha256:b");
        let events = [
            worktree_capture_for(&unit_a, "2026-06-19T00:00:00Z"),
            commit_associated_for(&unit_a, "shared"),
            worktree_capture_for(&unit_b, "2026-06-19T00:00:01Z"),
            commit_associated_for(&unit_b, "shared"),
        ];
        let projection = RevisionCommitRangeProjection::from_events(&events).unwrap();
        let grouping = CommitOidGroupingProjection::from_events(&events).unwrap();
        let base = list_from_events(&events, &projection).unwrap();

        let grouped = group_entries(base.entries, &grouping);

        assert_eq!(
            grouped.len(),
            1,
            "two same-range captures collapse to one entry"
        );
        let members = &grouped[0].grouped_revision_ids;
        assert_eq!(members.len(), 2);
        assert!(members.contains(&unit_a));
        assert!(members.contains(&unit_b));
    }

    #[test]
    fn ungrouped_units_are_unaffected() {
        // Two captures on DIFFERENT oids (and one floating) each stay their own entry,
        // with a single-member grouped set (the entry's own id).
        let unit_a = RevisionId::new("review-unit:sha256:a");
        let unit_b = RevisionId::new("review-unit:sha256:b");
        let unit_floating = RevisionId::new("review-unit:sha256:f");
        let events = [
            worktree_capture_for(&unit_a, "2026-06-19T00:00:00Z"),
            commit_associated_for(&unit_a, "oidA"),
            worktree_capture_for(&unit_b, "2026-06-19T00:00:01Z"),
            commit_associated_for(&unit_b, "oidB"),
            worktree_capture_for(&unit_floating, "2026-06-19T00:00:02Z"),
        ];
        let projection = RevisionCommitRangeProjection::from_events(&events).unwrap();
        let grouping = CommitOidGroupingProjection::from_events(&events).unwrap();
        let base = list_from_events(&events, &projection).unwrap();

        let grouped = group_entries(base.entries, &grouping);

        assert_eq!(
            grouped.len(),
            3,
            "no two share an OID; all three stay separate"
        );
        for entry in &grouped {
            assert_eq!(
                entry.grouped_revision_ids,
                vec![entry.revision_id.clone()],
                "an ungrouped entry's member set is just its own id"
            );
        }
    }
}
