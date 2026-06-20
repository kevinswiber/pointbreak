use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::Result;
use crate::model::{
    ReviewEndpoint, ReviewUnitId, ReviewUnitSource, RevisionId, SessionId, SnapshotId,
};
use crate::session::event::{EventType, ReviewUnitCapturedPayload, ShoreEvent};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store::resolution::resolve_read_store;
use crate::session::workflow::association::normalize_ref;
use crate::session::workflow::commit_range_liveness::{CommitGraphCondition, enrich_liveness};
use crate::session::workflow::observation::{
    CurrentReviewUnitContext, review_unit_ids_in_worktree,
};
use crate::session::workflow::read_store::divergence_diagnostics;
use crate::session::{EventStore, ReviewUnitCommitRangeProjection, ReviewUnitCommitRangeView};

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
pub struct ReviewUnitListOptions {
    repo: PathBuf,
    ref_filter: Option<RefFilter>,
    orphan_visibility: OrphanVisibility,
    integration_ref: Option<String>,
    worktree_scope: Option<PathBuf>,
}

impl ReviewUnitListOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            ref_filter: None,
            orphan_visibility: OrphanVisibility::default(),
            integration_ref: None,
            worktree_scope: None,
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
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUnitListEntry {
    pub review_unit_id: ReviewUnitId,
    pub session_id: SessionId,
    pub captured_at: String,
    pub revision_id: RevisionId,
    pub snapshot_id: SnapshotId,
    pub source: ReviewUnitSource,
    pub base: ReviewEndpoint,
    pub target: ReviewEndpoint,
    pub snapshot_artifact_content_hash: String,
    /// Git-free commit-range lifecycle view for this unit (anchored/floating,
    /// current and withdrawn associations). Structural merge-status is attached
    /// separately in `merge_status`.
    pub commit_range: ReviewUnitCommitRangeView,
    /// Structural merge-status from git reachability: `merged | open | orphaned |
    /// unknown`. `unknown` covers floating units, disagreeing per-commit
    /// conditions, and a repo error (which degrades gracefully, never an error).
    pub merge_status: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUnitListResult {
    pub event_set_hash: String,
    pub event_count: usize,
    pub review_unit_count: usize,
    pub entries: Vec<ReviewUnitListEntry>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn list_review_units(options: ReviewUnitListOptions) -> Result<ReviewUnitListResult> {
    let read_store = resolve_read_store(&options.repo)?;
    let events = EventStore::open(read_store.store_dir()).list_events()?;
    let projection = ReviewUnitCommitRangeProjection::from_events(&events)?;
    let mut result = list_from_events(&events, &projection)?;

    if let Some(ref_filter) = &options.ref_filter {
        let matching = review_units_matching_ref(
            &projection,
            &ref_filter.name,
            ref_filter.mode,
            &options.repo,
        )?;
        result
            .entries
            .retain(|entry| matching.contains(&entry.review_unit_id));
        result.review_unit_count = result.entries.len();
    }

    if let Some(worktree) = &options.worktree_scope {
        let context = CurrentReviewUnitContext::for_repo(worktree)?;
        let in_scope = review_unit_ids_in_worktree(&events, &context)?;
        result
            .entries
            .retain(|entry| in_scope.contains(&entry.review_unit_id));
        result.review_unit_count = result.entries.len();
    }

    apply_orphan_visibility(&mut result, &options.repo, options.orphan_visibility);

    attach_merge_status(
        &mut result,
        &options.repo,
        options.integration_ref.as_deref(),
    );

    result
        .diagnostics
        .extend(divergence_diagnostics(&read_store));
    Ok(result)
}

/// Attach the structural merge-status to each surfaced entry. Run after the
/// visibility filter so hidden orphans are not classified twice. Reuses each
/// entry's `commit_range` view against git reachability.
fn attach_merge_status(
    result: &mut ReviewUnitListResult,
    repo: &Path,
    integration_ref: Option<&str>,
) {
    for entry in &mut result.entries {
        entry.merge_status =
            merge_status_for(&entry.commit_range, repo, integration_ref).to_owned();
    }
}

/// The domain-named merge-status for a unit's current commit set, from the landed
/// liveness engine's agreed headline. `unknown` covers no commit anchor,
/// disagreeing per-commit conditions, a projection diagnostic, or an unavailable
/// repo (graceful degradation — never an error).
fn merge_status_for(
    view: &ReviewUnitCommitRangeView,
    repo: &Path,
    integration_ref: Option<&str>,
) -> &'static str {
    match enrich_liveness(view, repo, integration_ref) {
        Ok(enrichment) => match enrichment.headline {
            Some(CommitGraphCondition::Merged) => "merged",
            Some(CommitGraphCondition::Live) => "open",
            Some(CommitGraphCondition::Orphaned { .. }) => "orphaned",
            None => "unknown",
        },
        Err(_) => "unknown",
    }
}

/// Apply the orphan-visibility filter over the already-built entries. Default
/// hides commit-anchored units whose every current commit is orphaned; `All`
/// shows everything; `OrphansOnly` keeps only the orphaned ones. Reuses each
/// entry's `commit_range` view (no event re-list) against git reachability.
fn apply_orphan_visibility(
    result: &mut ReviewUnitListResult,
    repo: &Path,
    visibility: OrphanVisibility,
) {
    match visibility {
        OrphanVisibility::All => {}
        OrphanVisibility::HideOrphans => {
            result
                .entries
                .retain(|entry| !is_hidden_orphan(&entry.commit_range, repo));
            result.review_unit_count = result.entries.len();
        }
        OrphanVisibility::OrphansOnly => {
            result
                .entries
                .retain(|entry| is_hidden_orphan(&entry.commit_range, repo));
            result.review_unit_count = result.entries.len();
        }
    }
}

/// Whether a unit is a hidden orphan: commit-anchored with **every** current
/// commit classified `Orphaned` (any reason) by the landed reachability engine.
/// Floating units (no current commits) are never orphaned. A repo-unavailable
/// git error degrades to "not a hidden orphan" — never hide what we cannot
/// classify, and never error (LB-9 graceful degradation).
fn is_hidden_orphan(view: &ReviewUnitCommitRangeView, repo: &Path) -> bool {
    if view.current_commits.is_empty() {
        return false;
    }
    match enrich_liveness(view, repo, None) {
        Ok(enrichment) => enrichment
            .per_commit
            .iter()
            .all(|commit| matches!(commit.condition, CommitGraphCondition::Orphaned { .. })),
        Err(_) => false,
    }
}

/// Convenience entry point for "which units are associated with this ref?".
/// Delegates to [`list_review_units`] with a `--ref` filter applied.
pub fn list_units_for_ref(
    repo: impl AsRef<Path>,
    ref_name: impl Into<String>,
    mode: RefFilterMode,
) -> Result<ReviewUnitListResult> {
    list_review_units(ReviewUnitListOptions::new(repo).with_ref_filter(ref_name, mode))
}

/// The review-unit ids matching a ref under the chosen mode. The name is
/// normalized to its full ref first. `Label` is fully offline (current ref
/// labels); `Liveness` joins `enrich_liveness` against the ref's tip and keeps
/// units with at least one reachable commit. Shared by `unit list` and history.
pub(crate) fn review_units_matching_ref(
    projection: &ReviewUnitCommitRangeProjection,
    name: &str,
    mode: RefFilterMode,
    repo: &Path,
) -> Result<BTreeSet<ReviewUnitId>> {
    let normalized_ref = normalize_ref(name);
    match mode {
        RefFilterMode::Label => Ok(projection
            .units_for_ref(&normalized_ref)
            .into_iter()
            .map(|view| view.review_unit_id.clone())
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
                    matching.insert(view.review_unit_id.clone());
                }
            }
            Ok(matching)
        }
    }
}

fn list_from_events(
    events: &[ShoreEvent],
    projection: &ReviewUnitCommitRangeProjection,
) -> Result<ReviewUnitListResult> {
    let state = SessionState::from_events(events)?;
    let event_set_hash = state
        .event_set_hash
        .clone()
        .expect("SessionState::from_events sets event_set_hash");

    let mut entries = events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewUnitCaptured)
        .map(|event| entry_from_event(event, projection))
        .collect::<Result<Vec<_>>>()?;

    entries.sort_by(|left, right| {
        left.captured_at.cmp(&right.captured_at).then_with(|| {
            left.review_unit_id
                .as_str()
                .cmp(right.review_unit_id.as_str())
        })
    });

    Ok(ReviewUnitListResult {
        event_set_hash,
        event_count: events.len(),
        review_unit_count: entries.len(),
        entries,
        diagnostics: state.diagnostics,
    })
}

fn entry_from_event(
    event: &ShoreEvent,
    projection: &ReviewUnitCommitRangeProjection,
) -> Result<ReviewUnitListEntry> {
    let payload: ReviewUnitCapturedPayload = serde_json::from_value(event.payload.clone())?;
    let commit_range = projection
        .unit(&payload.review_unit_id)
        .cloned()
        .unwrap_or_else(|| empty_view(payload.review_unit_id.clone()));
    Ok(ReviewUnitListEntry {
        review_unit_id: payload.review_unit_id,
        session_id: event.target.session_id.clone(),
        captured_at: event.occurred_at.clone(),
        revision_id: payload.revision_id,
        snapshot_id: payload.snapshot_id,
        source: payload.source,
        base: payload.base,
        target: payload.target,
        snapshot_artifact_content_hash: payload.snapshot_artifact_content_hash,
        commit_range,
        // Filled by `attach_merge_status` after the visibility filter.
        merge_status: String::new(),
    })
}

fn empty_view(review_unit_id: ReviewUnitId) -> ReviewUnitCommitRangeView {
    ReviewUnitCommitRangeView {
        review_unit_id,
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
    use crate::model::{ReviewEndpoint, ReviewUnitSource, WorktreeCaptureMode};
    use crate::session::event::{EventTarget, Writer};

    #[test]
    fn empty_event_set_returns_no_entries() {
        let result = list_from_events(
            &[],
            &ReviewUnitCommitRangeProjection::from_events(&[]).unwrap(),
        )
        .unwrap();

        assert_eq!(result.event_count, 0);
        assert_eq!(result.review_unit_count, 0);
        assert!(result.entries.is_empty());
        assert!(result.event_set_hash.starts_with("sha256:"));
    }

    #[test]
    fn includes_only_review_unit_captured_events() {
        let capture = captured_event("a", "2026-05-13T10:00:00Z");
        let events = [capture];
        let projection = ReviewUnitCommitRangeProjection::from_events(&events).unwrap();
        let result = list_from_events(&events, &projection).unwrap();

        assert_eq!(result.event_count, 1);
        assert_eq!(result.review_unit_count, 1);
        assert_eq!(
            result.entries[0].review_unit_id.as_str(),
            "review-unit:sha256:a"
        );
        assert_eq!(result.entries[0].captured_at, "2026-05-13T10:00:00Z");
        assert_eq!(
            result.entries[0].snapshot_artifact_content_hash,
            "sha256:artifact:a"
        );
    }

    #[test]
    fn sorts_entries_by_captured_at_then_review_unit_id() {
        let later = captured_event("z-later", "2026-05-13T10:00:05Z");
        let tie_b = captured_event("b-tie", "2026-05-13T10:00:01Z");
        let tie_a = captured_event("a-tie", "2026-05-13T10:00:01Z");

        let events = [later, tie_b, tie_a];
        let projection = ReviewUnitCommitRangeProjection::from_events(&events).unwrap();
        let result = list_from_events(&events, &projection).unwrap();

        let order: Vec<&str> = result
            .entries
            .iter()
            .map(|entry| entry.review_unit_id.as_str())
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
        let projection = ReviewUnitCommitRangeProjection::from_events(&events).unwrap();
        let result = list_from_events(&events, &projection).unwrap();
        let json = serde_json::to_string(&result.entries[0]).unwrap();

        assert!(json.contains("reviewUnitId"));
        assert!(json.contains("capturedAt"));
        assert!(json.contains("snapshotArtifactContentHash"));
        assert!(!json.contains("artifacts/"));
        assert!(!json.contains("statePath"));
        assert!(!json.contains("payloadHash"));
    }

    fn captured_event(suffix: &str, occurred_at: &str) -> ShoreEvent {
        let review_unit_id = ReviewUnitId::new(format!("review-unit:sha256:{suffix}"));
        let revision_id = RevisionId::new(format!("rev:sha256:{suffix}"));
        let snapshot_id = SnapshotId::new(format!("snap:sha256:{suffix}"));
        let payload = ReviewUnitCapturedPayload {
            review_unit_id: review_unit_id.clone(),
            source: ReviewUnitSource::GitWorktree {
                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                include_untracked: true,
            },
            base: ReviewEndpoint::GitCommit {
                commit_oid: format!("base:{suffix}"),
                tree_oid: format!("base-tree:{suffix}"),
            },
            target: ReviewEndpoint::GitWorkingTree {
                worktree_root: "/repo".to_owned(),
            },
            revision_id: revision_id.clone(),
            snapshot_id: snapshot_id.clone(),
            snapshot_artifact_content_hash: format!("sha256:artifact:{suffix}"),
        };
        ShoreEvent::new(
            EventType::ReviewUnitCaptured,
            format!("capture:{suffix}"),
            EventTarget::for_review_unit(
                SessionId::new("session:default"),
                review_unit_id,
                revision_id,
                snapshot_id,
            ),
            Writer::shore_local("test"),
            payload,
            occurred_at,
        )
        .unwrap()
    }

    use std::path::Path;
    use std::process::Command;

    use tempfile::TempDir;

    use crate::git::{Ancestry, git_is_ancestor};
    use crate::model::{CommitAssociationId, CommitRangeCaptureMode, ReviewTargetRef};
    use crate::session::event::ReviewUnitCommitAssociatedPayload;

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
        let review_unit_id = ReviewUnitId::new(format!("review-unit:sha256:{suffix}"));
        let revision_id = RevisionId::new(format!("rev:sha256:{suffix}"));
        let snapshot_id = SnapshotId::new(format!("snap:sha256:{suffix}"));
        let payload = ReviewUnitCapturedPayload {
            review_unit_id: review_unit_id.clone(),
            source: ReviewUnitSource::GitCommitRange {
                mode: CommitRangeCaptureMode::BaseTreeToTargetTree,
            },
            base: ReviewEndpoint::GitCommit {
                commit_oid: format!("base:{suffix}"),
                tree_oid: format!("base-tree:{suffix}"),
            },
            target: ReviewEndpoint::GitCommit {
                commit_oid: commit_oid.to_owned(),
                tree_oid: format!("{commit_oid}-tree"),
            },
            revision_id: revision_id.clone(),
            snapshot_id: snapshot_id.clone(),
            snapshot_artifact_content_hash: format!("sha256:artifact:{suffix}"),
        };
        ShoreEvent::new(
            EventType::ReviewUnitCaptured,
            format!("capture:{suffix}"),
            EventTarget::for_review_unit(
                SessionId::new("session:default"),
                review_unit_id,
                revision_id,
                snapshot_id,
            ),
            Writer::shore_local("test"),
            payload,
            occurred_at,
        )
        .unwrap()
    }

    /// Adds a second current commit to an existing unit via a commit association.
    fn commit_associated_event(suffix: &str, commit_oid: &str) -> ShoreEvent {
        let review_unit_id = ReviewUnitId::new(format!("review-unit:sha256:{suffix}"));
        let payload = ReviewUnitCommitAssociatedPayload {
            commit_association_id: CommitAssociationId::new(format!(
                "commit-association:sha256:{suffix}:{commit_oid}"
            )),
            target: ReviewTargetRef::ReviewUnit {
                review_unit_id: review_unit_id.clone(),
            },
            commit: ReviewEndpoint::GitCommit {
                commit_oid: commit_oid.to_owned(),
                tree_oid: format!("{commit_oid}-tree"),
            },
        };
        ShoreEvent::new(
            EventType::ReviewUnitCommitAssociated,
            ReviewUnitCommitAssociatedPayload::idempotency_key(&review_unit_id, commit_oid),
            EventTarget::for_review_unit(
                SessionId::new("session:default"),
                review_unit_id,
                RevisionId::new(format!("rev:sha256:{suffix}")),
                SnapshotId::new(format!("snap:sha256:{suffix}")),
            ),
            Writer::shore_local("test"),
            payload,
            "2026-05-13T10:00:09Z",
        )
        .unwrap()
    }

    fn listed(events: &[ShoreEvent], visibility: OrphanVisibility, repo: &Path) -> Vec<String> {
        let projection = ReviewUnitCommitRangeProjection::from_events(events).unwrap();
        let mut result = list_from_events(events, &projection).unwrap();
        apply_orphan_visibility(&mut result, repo, visibility);
        assert_eq!(result.review_unit_count, result.entries.len());
        result
            .entries
            .iter()
            .map(|entry| entry.review_unit_id.as_str().to_owned())
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
    /// merge-status, returning `(review_unit_id, merge_status)` pairs.
    fn merge_statuses(
        events: &[ShoreEvent],
        repo: &Path,
        integration_ref: Option<&str>,
    ) -> Vec<(String, String)> {
        let projection = ReviewUnitCommitRangeProjection::from_events(events).unwrap();
        let mut result = list_from_events(events, &projection).unwrap();
        apply_orphan_visibility(&mut result, repo, OrphanVisibility::All);
        attach_merge_status(&mut result, repo, integration_ref);
        result
            .entries
            .iter()
            .map(|entry| {
                (
                    entry.review_unit_id.as_str().to_owned(),
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
        let projection = ReviewUnitCommitRangeProjection::from_events(&events).unwrap();
        let mut result = list_from_events(&events, &projection).unwrap();
        attach_merge_status(&mut result, repo.path(), None);

        let json = serde_json::to_string(&result.entries[0]).unwrap();
        assert!(json.contains("\"mergeStatus\""));
        assert!(json.contains("\"open\""));
    }
}
