//! JSON payload builders for the inspector server.
//!
//! Each builder reuses a public `shoreline::session` projection so the
//! inspector reads the store through the same validated path as the
//! corresponding `shore review` command, rather than parsing raw `.shore/data/`
//! files. Errors are stringified so the server can surface them to the UI as
//! a JSON `error` body instead of crashing a connection thread.

use std::path::Path;

use serde::Serialize;
use shoreline::documents::{lineage_show_document, unit_show_document};
use shoreline::model::{ReviewEndpoint, ReviewUnitId, ReviewUnitLineageId, SnapshotId};
use shoreline::session::{
    EventVerificationPolicy, LineageListEntry, LineageListOptions, LineageShowOptions,
    LineageShowResult, LivenessEnrichment, ProjectionDiagnostic, ReviewHistoryEntry,
    ReviewHistoryOptions, ReviewUnitListEntry, ReviewUnitListOptions, ReviewUnitShowOptions,
    enrich_liveness, list_lineages, list_review_units, read_snapshot_artifact, review_history,
    show_lineage, show_review_unit,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryPayload {
    schema: &'static str,
    event_set_hash: String,
    event_count: usize,
    history_count: usize,
    entries: Vec<ReviewHistoryEntry>,
    diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UnitsPayload {
    schema: &'static str,
    event_set_hash: String,
    event_count: usize,
    review_unit_count: usize,
    entries: Vec<UnitEntryDocument>,
    diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LineagesPayload {
    schema: &'static str,
    event_set_hash: String,
    event_count: usize,
    lineage_count: usize,
    entries: Vec<LineageEntryDocument>,
    diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LineageEntryDocument {
    lineage_id: String,
    head_review_unit_id: Option<String>,
    round_count: usize,
    rounds: Vec<LineageRoundEntryDocument>,
    diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LineageRoundEntryDocument {
    lineage_id: String,
    round_id: String,
    review_unit_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    predecessor_review_unit_id: Option<String>,
    round_index: Option<usize>,
    is_head: bool,
}

/// One `/api/units` entry: the full `ReviewUnitListEntry` flattened verbatim,
/// plus an additive, path-private `targetDisplay`. `#[serde(flatten)]` keeps
/// every existing field byte-present and unchanged.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UnitEntryDocument {
    #[serde(flatten)]
    entry: ReviewUnitListEntry,
    target_display: TargetDisplay,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FreshnessPayload {
    schema: &'static str,
    event_set_hash: String,
    event_count: usize,
    diagnostic_count: usize,
}

/// The literal floor label shown when no worktree basename can be derived.
const WORKING_TREE_FLOOR: &str = "working tree";
/// The floor label for a commit target whose OID is empty/unreadable. Distinct
/// from the worktree floor: a commit target is never a "working tree".
const GIT_COMMIT_FLOOR: &str = "git commit";
/// Length of the git-style short commit OID used for head labels (git's default).
const SHORT_OID_LEN: usize = 7;

/// Path-private display view-model for a ReviewUnit target.
///
/// Derived at read time from fields already present in a captured unit. The raw
/// worktree path never enters this block: only the final path component (a
/// basename) and a short commit OID are exposed, so the inspector can show a
/// meaningful worktree/head label without leaking absolute paths into its JSON.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TargetDisplay {
    /// `"working_tree"` for a Git working-tree target; `"git_commit"` for a
    /// commit target (e.g. a commit-range capture).
    kind: &'static str,
    /// For a working-tree target, the worktree-root basename (or the
    /// `"working tree"` floor). For a commit target, the short target OID (or
    /// the `"git commit"` floor).
    label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    head: Option<HeadDisplay>,
    /// Always true: this block is built from path-free fields and never carries
    /// the raw worktree path.
    path_private: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HeadDisplay {
    commit_oid_short: String,
    /// Capture-time head label. The baseline label is the short commit OID; a
    /// branch label is a deferred follow-up.
    label: String,
    /// Reserved for a deferred live-branch probe; rendered as current/live,
    /// never as capture-time provenance.
    #[serde(skip_serializing_if = "Option::is_none")]
    live_branch: Option<String>,
}

/// Derive the path-private [`TargetDisplay`] for a captured unit from its target
/// and base endpoints.
///
/// Pure: reads only captured fields, never the filesystem, and never rewrites
/// identity.
fn derive_target_display(target: &ReviewEndpoint, base: &ReviewEndpoint) -> TargetDisplay {
    let (kind, label) = match target {
        ReviewEndpoint::GitWorkingTree { worktree_root } => {
            ("working_tree", basename_label(worktree_root))
        }
        ReviewEndpoint::GitCommit { commit_oid, .. } => (
            "git_commit",
            short_oid(commit_oid).unwrap_or_else(|| GIT_COMMIT_FLOOR.to_owned()),
        ),
    };
    TargetDisplay {
        kind,
        label,
        head: head_display(base),
        // The raw worktree path is never copied into this block.
        path_private: true,
    }
}

/// Final non-empty path component of a worktree root, or the `"working tree"`
/// floor when the path is empty, the filesystem root, or not representable.
fn basename_label(worktree_root: &str) -> String {
    Path::new(worktree_root)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| WORKING_TREE_FLOOR.to_owned())
}

/// Git-style short commit OID: the first [`SHORT_OID_LEN`] characters, or the
/// whole oid when shorter. Returns `None` for an empty oid.
fn short_oid(commit_oid: &str) -> Option<String> {
    if commit_oid.is_empty() {
        return None;
    }
    Some(commit_oid.chars().take(SHORT_OID_LEN).collect())
}

/// Head label for a base endpoint: a short OID for a Git commit, else `None`.
fn head_display(base: &ReviewEndpoint) -> Option<HeadDisplay> {
    match base {
        ReviewEndpoint::GitCommit { commit_oid, .. } => {
            let short = short_oid(commit_oid)?;
            Some(HeadDisplay {
                label: short.clone(),
                commit_oid_short: short,
                live_branch: None,
            })
        }
        ReviewEndpoint::GitWorkingTree { .. } => None,
    }
}

/// Insert a derived `targetDisplay` into the `reviewUnit` object of a serialized
/// `/api/unit` document, leaving every existing field (including the verbatim
/// `target`) in place. A no-op if `reviewUnit` is not an object.
fn splice_target_display(
    document: &mut serde_json::Value,
    target_display: TargetDisplay,
) -> Result<(), String> {
    let value = serde_json::to_value(target_display).map_err(|error| error.to_string())?;
    if let Some(review_unit) = document
        .get_mut("reviewUnit")
        .and_then(|ru| ru.as_object_mut())
    {
        review_unit.insert("targetDisplay".to_owned(), value);
    }
    Ok(())
}

/// Wrap each list entry with its derived, path-private `targetDisplay`, leaving
/// every existing field on the entry untouched.
fn to_unit_entry_documents(entries: Vec<ReviewUnitListEntry>) -> Vec<UnitEntryDocument> {
    entries
        .into_iter()
        .map(|entry| {
            let target_display = derive_target_display(&entry.target, &entry.base);
            UnitEntryDocument {
                entry,
                target_display,
            }
        })
        .collect()
}

impl From<LineageShowResult> for LineageEntryDocument {
    fn from(result: LineageShowResult) -> Self {
        let round_count = result.rounds.len();
        Self {
            lineage_id: result.lineage_id.as_str().to_owned(),
            head_review_unit_id: result
                .head_review_unit_id
                .map(|review_unit_id| review_unit_id.as_str().to_owned()),
            round_count,
            rounds: result
                .rounds
                .into_iter()
                .map(LineageRoundEntryDocument::from)
                .collect(),
            diagnostics: result.diagnostics,
        }
    }
}

impl From<shoreline::session::LineageRoundView> for LineageRoundEntryDocument {
    fn from(round: shoreline::session::LineageRoundView) -> Self {
        Self {
            lineage_id: round.lineage_id.as_str().to_owned(),
            round_id: round.round_id.as_str().to_owned(),
            review_unit_id: round.review_unit_id.as_str().to_owned(),
            predecessor_review_unit_id: round
                .predecessor_review_unit_id
                .map(|review_unit_id| review_unit_id.as_str().to_owned()),
            round_index: round.round_index,
            is_head: round.is_head,
        }
    }
}

/// Full chronological event timeline with hydrated bodies.
pub(super) fn history_json(repo: &Path) -> Result<String, String> {
    let mut options = ReviewHistoryOptions::new(repo)
        .with_include_body(true)
        .with_verification_policy(EventVerificationPolicy::advisory())
        .with_trust_set(crate::cli::review::common::discover_trust_set(repo))
        .with_actor_attributes(crate::cli::review::common::discover_actor_attributes(repo));
    if let Some(map) = crate::cli::review::common::discover_delegation_map(repo) {
        options = options.with_delegation_map(map);
    }
    let result = review_history(options).map_err(|error| error.to_string())?;
    let history_count = result.history_count();
    let payload = HistoryPayload {
        schema: "shore.inspect-history",
        event_set_hash: result.event_set_hash,
        event_count: result.event_count,
        history_count,
        entries: result.entries,
        diagnostics: result.diagnostics,
    };
    serde_json::to_string(&payload).map_err(|error| error.to_string())
}

/// Captured ReviewUnits with their base/target/snapshot identity.
pub(super) fn units_json(repo: &Path) -> Result<String, String> {
    let result =
        list_review_units(ReviewUnitListOptions::new(repo)).map_err(|error| error.to_string())?;
    let payload = UnitsPayload {
        schema: "shore.inspect-units",
        event_set_hash: result.event_set_hash,
        event_count: result.event_count,
        review_unit_count: result.review_unit_count,
        entries: to_unit_entry_documents(result.entries),
        diagnostics: result.diagnostics,
    };
    serde_json::to_string(&payload).map_err(|error| error.to_string())
}

/// ReviewUnit lineages summarized for the inspector sidebar/list view.
pub(super) fn lineages_json(repo: &Path) -> Result<String, String> {
    let result = list_lineages(LineageListOptions::new(repo)).map_err(|error| error.to_string())?;
    let entries = result
        .entries
        .into_iter()
        .map(|entry: LineageListEntry| {
            show_lineage(LineageShowOptions::new(repo, entry.lineage_id))
                .map(LineageEntryDocument::from)
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let payload = LineagesPayload {
        schema: "shore.inspect-lineages",
        event_set_hash: result.event_set_hash,
        event_count: result.event_count,
        lineage_count: result.lineage_count,
        entries,
        diagnostics: result.diagnostics,
    };
    serde_json::to_string(&payload).map_err(|error| error.to_string())
}

/// The read-only lineage projection for one ReviewUnit lineage.
pub(super) fn lineage_json(repo: &Path, lineage_id: &str) -> Result<String, String> {
    if lineage_id.is_empty() {
        return Err("missing lineage id".to_owned());
    }
    let result = show_lineage(LineageShowOptions::new(
        repo,
        ReviewUnitLineageId::new(lineage_id.to_owned()),
    ))
    .map_err(|error| {
        tracing::debug!(error = %error, lineage = lineage_id, "inspect_lineage_read_failed");
        format!("lineage not found or unreadable: {lineage_id}")
    })?;
    let document = lineage_show_document(result);
    serde_json::to_string(&document).map_err(|error| error.to_string())
}

/// The captured diff snapshot for one ReviewUnit, by snapshot id.
///
/// Reads the immutable snapshot artifact through the validated read path
/// (`read_snapshot_artifact` recomputes and checks the content hash), so the
/// inspector renders exactly the frozen diff that was reviewed.
///
/// The wire shape redacts the hash-baked `target.worktreeRoot` after
/// validation: a linked inspector serves snapshots captured in sibling
/// worktrees, and their raw absolute paths must not reach other readers. The
/// stored artifact is untouched, so `contentHashScope: "stored-artifact"`
/// records that `contentHash` covers the stored bytes (including the redacted
/// field) — consumers re-validate by fetching the artifact, not by hashing
/// this wire JSON.
pub(super) fn snapshot_json(repo: &Path, snapshot_id: &str) -> Result<String, String> {
    if snapshot_id.is_empty() {
        return Err("missing snapshot id".to_owned());
    }
    let artifact = read_snapshot_artifact(repo, &SnapshotId::new(snapshot_id.to_owned())).map_err(
        |error| {
            // Keep the full error (which may include the internal artifact path)
            // in the server trace, but return a path-free message to the client.
            tracing::debug!(error = %error, snapshot = snapshot_id, "inspect_snapshot_read_failed");
            format!("snapshot not found or unreadable: {snapshot_id}")
        },
    )?;
    let mut wire = serde_json::to_value(&artifact).map_err(|error| error.to_string())?;
    if let Some(object) = wire.as_object_mut() {
        // Snapshot-scoped wire: identity/endpoints live on /api/unit (from the
        // projection), never on the shared snapshot artifact. The v2 body already
        // omits these; the removals also keep a dual-read v1 artifact path-private
        // here, so the endpoint is forward- and backward-compatible.
        for key in ["reviewUnitId", "source", "base", "target"] {
            object.remove(key);
        }
    }
    serde_json::to_string(&wire).map_err(|error| error.to_string())
}

/// The full composite projection for one ReviewUnit.
///
/// Reuses the exact `shore.review-unit` document the `shore review unit show`
/// command builds (`unit_show_document`), so the inspector renders the same
/// authoritative composite — current-assessment status, duplicate-collapsed
/// facts, supersession, adapter notes, and projection rows — rather than
/// re-deriving it client-side.
pub(super) fn unit_json(repo: &Path, review_unit_id: &str) -> Result<String, String> {
    if review_unit_id.is_empty() {
        return Err("missing review unit id".to_owned());
    }
    let mut show_options = ReviewUnitShowOptions::new(repo)
        .with_review_unit_id(ReviewUnitId::new(review_unit_id.to_owned()))
        .with_include_body(true)
        .with_verification_policy(EventVerificationPolicy::advisory())
        .with_trust_set(crate::cli::review::common::discover_trust_set(repo))
        .with_actor_attributes(crate::cli::review::common::discover_actor_attributes(repo));
    if let Some(map) = crate::cli::review::common::discover_delegation_map(repo) {
        show_options = show_options.with_delegation_map(map);
    }
    let result = show_review_unit(show_options).map_err(|error| {
        tracing::debug!(error = %error, review_unit = review_unit_id, "inspect_unit_read_failed");
        format!("review unit not found or unreadable: {review_unit_id}")
    })?;
    // Thread the typed endpoints and the commit-range view out before
    // `unit_show_document` consumes `result`, then splice the additive
    // `targetDisplay` into the serialized document.
    let target_display =
        derive_target_display(&result.review_unit.target, &result.review_unit.base);
    let head_oid = match &result.review_unit.base {
        ReviewEndpoint::GitCommit { commit_oid, .. } => Some(commit_oid.clone()),
        ReviewEndpoint::GitWorkingTree { .. } => None,
    };
    let commit_range = result.commit_range.clone();
    let document = unit_show_document(result);
    let mut value = serde_json::to_value(&document).map_err(|error| error.to_string())?;
    splice_target_display(&mut value, target_display)?;

    // Current/live enrichment is best-effort: a missing or unreadable repo leaves
    // `liveBranch` omitted ("reachability unknown"), never an error.
    if let Some(head_oid) = head_oid
        && let Ok(enrichment) = enrich_liveness(&commit_range, repo, None)
        && let Some(live_branch) = resolve_head_live_branch(&enrichment, &head_oid)
    {
        set_head_live_branch(&mut value, live_branch);
    }

    serde_json::to_string(&value).map_err(|error| error.to_string())
}

/// The branch a unit's head commit currently lives on, for the head display.
/// Prefers the displayed head commit's own liveness; when the head is not among
/// the unit's current commits (a commit-range base differs from its target),
/// falls back to the unit's single unambiguous live branch.
fn resolve_head_live_branch(enrichment: &LivenessEnrichment, head_oid: &str) -> Option<String> {
    if let Some(commit) = enrichment
        .per_commit
        .iter()
        .find(|commit| commit.commit_oid == head_oid)
    {
        return commit.live_branch.clone();
    }
    let mut labels = enrichment
        .per_commit
        .iter()
        .filter_map(|commit| commit.live_branch.clone());
    let first = labels.next()?;
    labels.all(|label| label == first).then_some(first)
}

/// Insert `liveBranch` into the spliced `reviewUnit.targetDisplay.head` object.
/// A no-op if the head block is absent (e.g. a working-tree base with no head).
fn set_head_live_branch(document: &mut serde_json::Value, live_branch: String) {
    if let Some(head) = document
        .get_mut("reviewUnit")
        .and_then(|review_unit| review_unit.get_mut("targetDisplay"))
        .and_then(|target_display| target_display.get_mut("head"))
        .and_then(|head| head.as_object_mut())
    {
        head.insert("liveBranch".to_owned(), live_branch.into());
    }
}

/// Cheap freshness probe for client-side auto-refresh polling.
///
/// Computes `eventSetHash` from the live event set (without hydrating bodies)
/// so the UI can detect store changes and re-fetch only when something moved.
pub(super) fn freshness_json(repo: &Path) -> Result<String, String> {
    let result = review_history(ReviewHistoryOptions::new(repo).with_include_body(false))
        .map_err(|error| error.to_string())?;
    let payload = FreshnessPayload {
        schema: "shore.inspect-freshness",
        event_set_hash: result.event_set_hash,
        event_count: result.event_count,
        diagnostic_count: result.diagnostics.len(),
    };
    serde_json::to_string(&payload).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use shoreline::model::{
        ReviewEndpoint, ReviewUnitSource, RevisionId, SessionId, WorktreeCaptureMode,
    };
    use shoreline::session::event::ReviewUnitCapturedPayload;

    use super::*;

    fn working_tree(root: &str) -> ReviewEndpoint {
        ReviewEndpoint::GitWorkingTree {
            worktree_root: root.to_owned(),
        }
    }

    fn commit(oid: &str) -> ReviewEndpoint {
        ReviewEndpoint::GitCommit {
            commit_oid: oid.to_owned(),
            tree_oid: "tree-oid".to_owned(),
        }
    }

    fn captured_repo() -> (tempfile::TempDir, String) {
        let root = tempfile::tempdir().expect("create temp repo");
        let path = root.path();
        git(path, &["init"]);
        git(path, &["config", "user.name", "Shore Tests"]);
        git(path, &["config", "user.email", "shore-tests@example.com"]);
        git(path, &["config", "commit.gpgsign", "false"]);
        std::fs::write(path.join("src.txt"), "base\n").unwrap();
        git(path, &["add", "--all"]);
        git(path, &["commit", "-m", "base"]);
        std::fs::write(path.join("src.txt"), "changed\n").unwrap();
        let result = shoreline::session::capture_worktree_review(
            shoreline::session::CaptureOptions::new(path),
        )
        .expect("capture worktree review");
        (root, result.snapshot_id.as_str().to_owned())
    }

    fn git(cwd: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap_or_else(|error| panic!("run git {args:?}: {error}"));
        assert!(
            output.status.success(),
            "git {args:?} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// The shared common-dir store a clone resolves by default
    /// (`<git-common-dir>/shore`). A non-ephemeral worktree reads and writes here,
    /// so a post-capture store path resolves here, not the worktree-local
    /// `.shore/data`.
    fn common_dir_store(repo: &Path) -> std::path::PathBuf {
        let output = std::process::Command::new("git")
            .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
            .current_dir(repo)
            .output()
            .expect("run git rev-parse --git-common-dir");
        assert!(output.status.success(), "git rev-parse --git-common-dir");
        let common_dir = String::from_utf8(output.stdout)
            .expect("git-common-dir is utf-8")
            .trim()
            .to_owned();
        Path::new(&common_dir).join("shore")
    }

    fn stored_snapshot_artifact_path(repo: &Path) -> std::path::PathBuf {
        let snapshots_dir = common_dir_store(repo).join("artifacts/snapshots");
        let mut entries: Vec<_> = std::fs::read_dir(&snapshots_dir)
            .expect("snapshot artifacts dir exists")
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(entries.len(), 1, "exactly one stored snapshot artifact");
        entries.remove(0)
    }

    #[test]
    fn snapshot_json_serves_snapshot_scoped_wire() {
        let (repo, snapshot_id) = captured_repo();

        let wire: serde_json::Value =
            serde_json::from_str(&snapshot_json(repo.path(), &snapshot_id).unwrap()).unwrap();

        // Snapshot-scoped wire: content hash + frozen diff only. Identity and
        // endpoints live on /api/unit (from the projection), never here — so the
        // worktree root is simply absent (nothing to redact).
        assert!(wire["contentHash"].is_string());
        assert!(wire.get("reviewUnitId").is_none());
        assert!(wire.get("source").is_none());
        assert!(wire.get("base").is_none());
        assert!(wire.get("target").is_none());
        assert!(wire.get("worktreeRootRedacted").is_none());
        assert!(wire.get("contentHashScope").is_none());
        assert!(wire.get("targetDisplay").is_none());
    }

    #[test]
    fn snapshot_json_rejects_tampered_artifact_before_wire_shaping() {
        let (repo, snapshot_id) = captured_repo();
        let artifact_path = stored_snapshot_artifact_path(repo.path());
        let mut json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&artifact_path).unwrap()).unwrap();
        // Tamper a field that is inside the content hash for both v1 and v2 (the
        // snapshot rows). `DiffFile` is snake_case, unlike the camelCase wrapper.
        json["snapshot"]["files"][0]["new_path"] = serde_json::json!("/evil");
        std::fs::write(&artifact_path, serde_json::to_vec(&json).unwrap()).unwrap();

        let error = snapshot_json(repo.path(), &snapshot_id)
            .expect_err("tampered artifact is rejected before wire shaping");

        assert!(error.contains("snapshot not found or unreadable"));
        assert!(!error.contains("/evil"));
    }

    #[test]
    fn derives_basename_label_and_short_head_from_captured_fields() {
        let target = working_tree("/Users/x/worktrees/boardwalk/plan-0006");
        let base = commit("545b0eb81463aaaaaaaaaaaaaaaaaaaaaaaaaaaa");

        let display = derive_target_display(&target, &base);

        assert_eq!(display.kind, "working_tree");
        assert_eq!(display.label, "plan-0006");
        let head = display
            .head
            .as_ref()
            .expect("head derived from base commit");
        assert_eq!(head.commit_oid_short, "545b0eb");
        assert_eq!(head.label, "545b0eb");
        assert!(head.live_branch.is_none());
        assert!(display.path_private);
    }

    #[test]
    fn floors_empty_or_root_worktree_root_to_working_tree() {
        assert_eq!(
            derive_target_display(&working_tree("/"), &commit("abc1234")).label,
            "working tree"
        );
        assert_eq!(
            derive_target_display(&working_tree(""), &commit("abc1234")).label,
            "working tree"
        );
    }

    #[test]
    fn empty_commit_oid_yields_no_head() {
        let display = derive_target_display(&working_tree("/repo/wt"), &commit(""));
        assert!(display.head.is_none());
    }

    #[test]
    fn commit_target_displays_short_target_oid_label() {
        let display = derive_target_display(
            &commit("9fceb02d0ae598e95dc970b74767f19372d61af8"),
            &commit("abc1234def"),
        );

        assert_eq!(display.kind, "git_commit");
        assert_eq!(display.label, "9fceb02");
        assert_eq!(display.head.unwrap().commit_oid_short, "abc1234");
        assert!(display.path_private);
    }

    #[test]
    fn commit_target_with_empty_oid_floors_to_kind_label() {
        let display = derive_target_display(&commit(""), &commit("abc1234def"));

        assert_eq!(display.kind, "git_commit");
        assert_eq!(display.label, "git commit");
        assert_ne!(display.label, "working tree");
    }

    #[test]
    fn serialized_block_is_camel_case_and_path_private() {
        let display = derive_target_display(
            &working_tree("/Users/x/worktrees/boardwalk/plan-0006"),
            &commit("545b0eb81463aaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        );
        let json = serde_json::to_string(&display).unwrap();

        assert!(json.contains("\"pathPrivate\":true"));
        assert!(json.contains("\"commitOidShort\":\"545b0eb\""));
        assert!(json.contains("\"label\":\"plan-0006\""));
        // No raw absolute path and no worktreeRoot key leak into the display block.
        assert!(!json.contains("/Users"));
        assert!(!json.contains("worktreeRoot"));
    }

    fn entry(worktree: &str, commit: &str) -> ReviewUnitListEntry {
        ReviewUnitListEntry {
            review_unit_id: ReviewUnitId::new("review-unit:sha256:abc"),
            session_id: SessionId::new("session:default"),
            captured_at: "2026-05-13T10:00:00Z".to_owned(),
            revision_id: RevisionId::new("rev:sha256:abc"),
            snapshot_id: SnapshotId::new("snap:sha256:abc"),
            source: ReviewUnitSource::GitWorktree {
                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                include_untracked: true,
            },
            base: ReviewEndpoint::GitCommit {
                commit_oid: commit.to_owned(),
                tree_oid: "tree-oid".to_owned(),
            },
            target: ReviewEndpoint::GitWorkingTree {
                worktree_root: worktree.to_owned(),
            },
            snapshot_artifact_content_hash: "sha256:artifact:abc".to_owned(),
            commit_range: shoreline::session::ReviewUnitCommitRangeView {
                review_unit_id: ReviewUnitId::new("review-unit:sha256:abc"),
                anchored: false,
                current_commits: Vec::new(),
                current_refs: Vec::new(),
                withdrawn_commits: Vec::new(),
                withdrawn_refs: Vec::new(),
                diagnostics: Vec::new(),
            },
            merge_status: "unknown".to_owned(),
            grouped_review_unit_ids: vec![ReviewUnitId::new("review-unit:sha256:abc")],
        }
    }

    #[test]
    fn units_document_splices_target_display_additively() {
        let entries = vec![entry(
            "/Users/x/worktrees/boardwalk/plan-0006",
            "545b0eb81463aaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )];

        let docs = to_unit_entry_documents(entries);
        let json = serde_json::to_value(&docs[0]).unwrap();

        // The derived, path-private targetDisplay is spliced in...
        assert_eq!(json["targetDisplay"]["label"], "plan-0006");
        assert_eq!(json["targetDisplay"]["head"]["commitOidShort"], "545b0eb");
        assert_eq!(json["targetDisplay"]["pathPrivate"], true);

        // ...and every prior field is still byte-present and unchanged (additive).
        assert_eq!(
            json["target"]["worktreeRoot"],
            "/Users/x/worktrees/boardwalk/plan-0006"
        );
        assert_eq!(json["target"]["kind"], "git_working_tree");
        assert_eq!(
            json["base"]["commitOid"],
            "545b0eb81463aaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert!(json["source"].is_object());
        assert_eq!(json["snapshotArtifactContentHash"], "sha256:artifact:abc");
        assert_eq!(json["reviewUnitId"], "review-unit:sha256:abc");
        assert_eq!(json["capturedAt"], "2026-05-13T10:00:00Z");
        assert_eq!(json["revisionId"], "rev:sha256:abc");
        assert_eq!(json["snapshotId"], "snap:sha256:abc");
    }

    #[test]
    fn splice_target_display_adds_block_without_dropping_target_fields() {
        // Mirrors the /api/unit document shape: reviewUnit carries the verbatim target.
        let mut document = serde_json::json!({
            "reviewUnit": {
                "id": "review-unit:sha256:abc",
                "target": {
                    "kind": "git_working_tree",
                    "worktreeRoot": "/Users/x/worktrees/boardwalk/plan-0006"
                },
                "base": { "kind": "git_commit", "commitOid": "545b0eb81463", "treeOid": "t" }
            }
        });
        let display = derive_target_display(
            &working_tree("/Users/x/worktrees/boardwalk/plan-0006"),
            &commit("545b0eb81463aaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        );

        splice_target_display(&mut document, display).unwrap();

        assert_eq!(
            document["reviewUnit"]["targetDisplay"]["label"],
            "plan-0006"
        );
        assert_eq!(
            document["reviewUnit"]["targetDisplay"]["head"]["commitOidShort"],
            "545b0eb"
        );
        // Additive: the raw target endpoint is untouched.
        assert_eq!(
            document["reviewUnit"]["target"]["worktreeRoot"],
            "/Users/x/worktrees/boardwalk/plan-0006"
        );
        assert_eq!(document["reviewUnit"]["target"]["kind"], "git_working_tree");
    }

    #[test]
    fn legacy_worktree_root_payload_derives_basename_without_touching_identity() {
        // A payload that only ever carried `worktreeRoot`. Deriving the display
        // must be a pure read: it must not rewrite the ReviewUnit identity and
        // must not leak the raw path into the derived block.
        let review_unit_id = ReviewUnitId::new("review-unit:sha256:legacy");
        let payload = ReviewUnitCapturedPayload {
            review_unit_id: review_unit_id.clone(),
            source: ReviewUnitSource::GitWorktree {
                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                include_untracked: true,
            },
            base: ReviewEndpoint::GitCommit {
                commit_oid: "0123456789abcdef0123456789abcdef01234567".to_owned(),
                tree_oid: "tree-oid".to_owned(),
            },
            target: ReviewEndpoint::GitWorkingTree {
                worktree_root: "/repo/legacy-wt".to_owned(),
            },
            revision_id: RevisionId::new("rev:sha256:legacy"),
            snapshot_id: SnapshotId::new("snap:sha256:legacy"),
            snapshot_artifact_content_hash: "sha256:artifact:legacy".to_owned(),
        };

        let display = derive_target_display(&payload.target, &payload.base);
        let json = serde_json::to_string(&display).unwrap();

        assert_eq!(display.label, "legacy-wt");
        assert!(display.path_private);
        assert_eq!(display.head.as_ref().unwrap().commit_oid_short, "0123456");
        // No raw path leaks into the derived block.
        assert!(!json.contains("/repo"));
        // Derivation never rewrote identity (no event/file written).
        assert_eq!(payload.review_unit_id, review_unit_id);
    }

    fn captured_commit_range_repo() -> (tempfile::TempDir, String, String) {
        let root = tempfile::tempdir().expect("create temp repo");
        let path = root.path();
        git(path, &["init"]);
        git(path, &["config", "user.name", "Shore Tests"]);
        git(path, &["config", "user.email", "shore-tests@example.com"]);
        git(path, &["config", "commit.gpgsign", "false"]);
        std::fs::write(path.join("src.txt"), "base\n").unwrap();
        git(path, &["add", "--all"]);
        git(path, &["commit", "-m", "base"]);
        std::fs::write(path.join("src.txt"), "next\n").unwrap();
        git(path, &["add", "--all"]);
        git(path, &["commit", "-m", "next"]);

        let result = shoreline::session::capture_review(
            shoreline::session::CaptureOptions::new(path).with_commit_range(
                shoreline::session::CommitRangeSpec::new("HEAD~1").with_target_rev("HEAD"),
            ),
        )
        .expect("capture commit range review");
        let branch = current_branch(path);
        (root, result.review_unit_id.as_str().to_owned(), branch)
    }

    fn current_branch(repo: &Path) -> String {
        let output = std::process::Command::new("git")
            .args(["symbolic-ref", "--short", "HEAD"])
            .current_dir(repo)
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    #[test]
    fn unit_json_populates_live_branch_for_anchored_commit_on_a_branch() {
        let (repo, review_unit_id, branch) = captured_commit_range_repo();

        let value: serde_json::Value =
            serde_json::from_str(&unit_json(repo.path(), &review_unit_id).unwrap()).unwrap();

        assert_eq!(
            value["reviewUnit"]["targetDisplay"]["head"]["liveBranch"],
            serde_json::json!(branch),
            "the anchored target commit is the branch tip → live on that branch"
        );
    }

    #[test]
    fn unit_json_omits_live_branch_for_floating_worktree_capture() {
        let root = tempfile::tempdir().expect("create temp repo");
        let path = root.path();
        git(path, &["init"]);
        git(path, &["config", "user.name", "Shore Tests"]);
        git(path, &["config", "user.email", "shore-tests@example.com"]);
        git(path, &["config", "commit.gpgsign", "false"]);
        std::fs::write(path.join("src.txt"), "base\n").unwrap();
        git(path, &["add", "--all"]);
        git(path, &["commit", "-m", "base"]);
        std::fs::write(path.join("src.txt"), "changed\n").unwrap();
        let capture = shoreline::session::capture_worktree_review(
            shoreline::session::CaptureOptions::new(path),
        )
        .expect("capture worktree review");

        let value: serde_json::Value =
            serde_json::from_str(&unit_json(path, capture.review_unit_id.as_str()).unwrap())
                .unwrap();

        assert!(
            value["reviewUnit"]["targetDisplay"]["head"]["liveBranch"].is_null(),
            "a floating worktree capture has no current commit → liveBranch omitted"
        );
    }

    #[test]
    fn unit_json_omits_live_branch_when_commit_objects_are_unavailable() {
        let (repo, review_unit_id, _branch) = captured_commit_range_repo();

        // A second repo that serves the same store but whose object database does
        // not hold the captured commits (the linked-inspector case). The store
        // still reads; reachability cannot resolve, so liveBranch is omitted.
        let elsewhere = tempfile::tempdir().expect("create separate repo");
        git(elsewhere.path(), &["init"]);
        git(elsewhere.path(), &["config", "user.name", "Shore Tests"]);
        git(
            elsewhere.path(),
            &["config", "user.email", "shore-tests@example.com"],
        );
        git(elsewhere.path(), &["config", "commit.gpgsign", "false"]);
        copy_dir_all(
            &common_dir_store(repo.path()),
            &common_dir_store(elsewhere.path()),
        );

        let value: serde_json::Value =
            serde_json::from_str(&unit_json(elsewhere.path(), &review_unit_id).unwrap()).unwrap();

        assert!(
            value["reviewUnit"]["targetDisplay"]["head"]["liveBranch"].is_null(),
            "commit objects absent → reachability unknown → liveBranch omitted, request still 200s"
        );
    }

    fn copy_dir_all(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir_all(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    #[test]
    fn resolve_head_live_branch_prefers_head_then_falls_back_to_single_unambiguous() {
        use shoreline::session::{CommitGraphCondition, CommitLiveness, LivenessEnrichment};

        // Head commit itself is among the current commits → use its own branch.
        let matched = LivenessEnrichment {
            per_commit: vec![CommitLiveness {
                commit_oid: "headoid".to_owned(),
                condition: CommitGraphCondition::Live,
                live_branch: Some("main".to_owned()),
            }],
            headline: Some(CommitGraphCondition::Live),
        };
        assert_eq!(
            resolve_head_live_branch(&matched, "headoid").as_deref(),
            Some("main")
        );

        // Head not among current commits (commit-range base != target) → fall back
        // to the unit's single live branch.
        assert_eq!(
            resolve_head_live_branch(&matched, "baseoid").as_deref(),
            Some("main")
        );

        // Two current commits on different branches → ambiguous → None.
        let ambiguous = LivenessEnrichment {
            per_commit: vec![
                CommitLiveness {
                    commit_oid: "a".to_owned(),
                    condition: CommitGraphCondition::Live,
                    live_branch: Some("main".to_owned()),
                },
                CommitLiveness {
                    commit_oid: "b".to_owned(),
                    condition: CommitGraphCondition::Live,
                    live_branch: Some("feature".to_owned()),
                },
            ],
            headline: None,
        };
        assert_eq!(resolve_head_live_branch(&ambiguous, "baseoid"), None);
    }
}
