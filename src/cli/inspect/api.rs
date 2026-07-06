//! JSON payload builders for the inspector server.
//!
//! Each builder reuses a public `shoreline::session` projection so the
//! inspector reads the store through the same validated path as the
//! corresponding `shore` review read command, rather than parsing raw `.shore/data/`
//! files. Errors are stringified so the server can surface them to the UI as
//! a JSON `error` body instead of crashing a connection thread.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::RwLock;

use mmdflux::graph::{Direction, Edge, Graph, Node};
use mmdflux::layout::{LaidOutGraph, LayoutOptions, layout_graph};
use serde::Serialize;
use shoreline::documents::revision_show_document;
use shoreline::highlight::{emphasis_file, highlight_file};
use shoreline::model::{ObjectId, ReviewEndpoint, RevisionId, RevisionSource, ValidationStatus};
use shoreline::session::event::ReviewAssessment;
use shoreline::session::{
    AssessmentRecordStatus, AssessmentView, BaseProjectionConfig, CurrentAssessmentStatus,
    EventVerificationPolicy, HistoryPage, HistoryQuery, InputRequestStatus, LivenessEnrichment,
    ObservationStatus, ObservationView, ProjectionDiagnostic, ReviewHistoryEntry,
    RevisionCommitRangeView, RevisionListEntry, RevisionListOptions, RevisionOverview,
    RevisionOverviewsOptions, RevisionProjectionSummary, RevisionShowOptions, RevisionShowResult,
    SessionState, SupersessionView, apply_history_query, enrich_liveness, event_log_head_marker,
    history_base_projection, list_revisions, read_bound_object_artifact, read_events_for_display,
    read_object_artifact, show_revision, show_revision_overviews,
};

use super::server::HighlightCache;
use super::wire::WireObjectArtifact;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryPayload {
    schema: &'static str,
    event_set_hash: String,
    event_count: usize,
    history_count: usize,
    entries: Vec<ReviewHistoryEntry>,
    /// Per-event-type counts under the active `q`/`track`/`object` query but
    /// EXCLUDING the `type` page filter — the timeline's type-toggle numbers
    /// (INV-3). Additive, always present.
    facets: BTreeMap<String, usize>,
    /// The full filtered size (post-`type`), so the virtualized scrollbar is sized
    /// without transferring every row. Additive, always present.
    match_count: usize,
    /// The returned window's start index in the filtered set, for the sparse
    /// virtual list. Additive, always present.
    offset: usize,
    /// The located index of an `at=<eventId>` request within the filtered set, so
    /// reveal / deep-link can fetch-to-reveal an off-page target. Present only for
    /// an `at` request.
    #[serde(skip_serializing_if = "Option::is_none")]
    match_index: Option<usize>,
    diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RevisionsPayload {
    schema: &'static str,
    event_set_hash: String,
    event_count: usize,
    revision_count: usize,
    entries: Vec<RevisionEntryDocument>,
    diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadsPayload {
    schema: &'static str,
    event_set_hash: String,
    event_count: usize,
    thread_count: usize,
    threads: Vec<ThreadDocument>,
    /// Forward supersession edges (revision -> the revisions it directly
    /// supersedes), so the inspector can render the DAG and a supersedes chip on
    /// the capture row. Only revisions that supersede something appear.
    supersedes: BTreeMap<String, Vec<String>>,
    /// Reverse supersession edges (revision -> the revisions that directly
    /// supersede it), so a fact on a superseded revision can name *all* of its
    /// superseding successors. Only superseded revisions appear.
    superseded_by: BTreeMap<String, Vec<String>>,
    /// Per-revision supersession classification (head / superseded / isolated +
    /// its direct superseders/predecessors), so the client reads a field instead
    /// of re-deriving head/superseded status from the edge maps every render. An
    /// advisory readback: a fork classifies both competing heads as `head`, no
    /// winner.
    revision_classification: BTreeMap<String, RevisionClassification>,
    diagnostics: Vec<ProjectionDiagnostic>,
}

/// The supersession standing of one revision, derived from the projection.
/// `state` is `head` (a current head that supersedes at least one predecessor),
/// `superseded` (named by at least one successor), or `isolated` (a lone root —
/// a current head with no incident edges either way).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RevisionClassification {
    state: &'static str,
    superseded_by: Vec<String>,
    supersedes: Vec<String>,
}

/// One supersession thread (the connected component of the supersession graph —
/// the engagement, labeled domain-side). Fork-tolerant: `heads` carries every
/// competing head, never a null head.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadDocument {
    revisions: Vec<String>,
    heads: Vec<String>,
    superseded: Vec<String>,
    /// `true` when the thread has more than one current head (a fork).
    competing: bool,
    /// The server-laid graph geometry for this thread (node centers + routed
    /// edge polylines + bounds), so the client is a thin SVG painter. Additive.
    laid_out: ThreadLayout,
}

/// Server-computed supersession-DAG geometry for one thread, normalized to a
/// `(0,0)` top-left so the client paints into `viewBox="0 0 w h"` with no
/// clipping. Topology is the contract; exact pixels are the pinned engine's.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadLayout {
    nodes: Vec<LaidOutNodeWire>,
    edges: Vec<LaidOutEdgeWire>,
    bounds: LayoutBounds,
}

/// One laid-out revision node: `x,y` is the box CENTER (not a corner), `w,h` its
/// size; `isHead`/`isSuperseded` come from the projection, never from the engine.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LaidOutNodeWire {
    id: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    is_head: bool,
    is_superseded: bool,
}

/// One routed supersession edge: `from` SUPERSEDES `to`; `path` is the routed
/// polyline. The client orients the arrowhead by the from/to node centers, so the
/// engine's cycle-removal `is_backward` flag is intentionally not surfaced here.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LaidOutEdgeWire {
    from: String,
    to: String,
    path: Vec<[f64; 2]>,
    /// The fact relation this edge encodes (`"replaces"` / `"supersedes"`); the
    /// tagged-edge model that lets a later fact edge kind slot in. Revision edges
    /// carry `None`, so the `/api/threads` wire is byte-identical.
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LayoutBounds {
    w: f64,
    h: f64,
}

/// Inspector-private, additive fact-level supersession graphs for one revision,
/// spliced into `/api/revisions/{id}`. Fork-gated: a sub-field is present only
/// when that fact type forks, and the whole struct is omitted when neither does,
/// so a non-forked revision's wire is byte-identical. Never added to the shared
/// `shore.review-revision` document.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FactSupersessionDocument {
    #[serde(skip_serializing_if = "Option::is_none")]
    assessments: Option<FactGraphDocument>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observations: Option<FactGraphDocument>,
}

impl FactSupersessionDocument {
    fn is_empty(&self) -> bool {
        self.assessments.is_none() && self.observations.is_none()
    }
}

/// One fact type's laid-out supersession graph. `laidOut` is the exact revision
/// `ThreadLayout` wire; its edges additionally carry `kind`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FactGraphDocument {
    laid_out: ThreadLayout,
}

/// One `/api/revisions` entry: the shared `RevisionListEntry` re-keyed into the
/// inspector's snapshot vocabulary (`snapshotId`, `snapshotContentHash`), plus
/// the additive, path-private `targetDisplay` and the server-computed
/// `overview`. The shared `shore.review-revision-list` document is untouched;
/// this is the inspector-private wire shape. Fields are listed explicitly (no
/// flatten) so a new shared field forces a naming decision on this surface.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RevisionEntryDocument {
    captured_at: String,
    revision_id: RevisionId,
    snapshot_id: ObjectId,
    source: RevisionSource,
    base: ReviewEndpoint,
    target: ReviewEndpoint,
    snapshot_content_hash: String,
    commit_range: RevisionCommitRangeView,
    merge_status: String,
    grouped_revision_ids: Vec<RevisionId>,
    target_display: TargetDisplay,
    overview: RevisionOverviewDocument,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RevisionOverviewDocument {
    current_assessment: RevisionOverviewAssessmentDocument,
    attention: RevisionAttentionDocument,
    counts: RevisionOverviewCounts,
    latest_activity: Option<RevisionLatestActivityDocument>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RevisionOverviewAssessmentDocument {
    status: &'static str,
    assessment: Option<ReviewAssessment>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RevisionAttentionDocument {
    unassessed: bool,
    accepted_with_follow_up: bool,
    open_input_request_count: usize,
    failed_validation_count: usize,
    errored_validation_count: usize,
    stale_fact_count: usize,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RevisionOverviewCounts {
    files: usize,
    rows: usize,
    observations: usize,
    input_requests: usize,
    assessments: usize,
    validation_checks: usize,
    adapter_notes: usize,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RevisionLatestActivityDocument {
    kind: &'static str,
    title: String,
    at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FreshnessPayload {
    schema: &'static str,
    /// The event-log head marker (the event count), the cheap change key the
    /// client polls. Sourced from `event_log_head_marker` — no full read, no
    /// event-set hash. The authoritative `eventSetHash` confirm stamp stays on the
    /// full-read endpoints (`/api/history`, `/api/revisions`).
    event_count: u64,
}

/// The literal floor label shown when no worktree basename can be derived.
const WORKING_TREE_FLOOR: &str = "working tree";
/// The floor label for a commit target whose OID is empty/unreadable. Distinct
/// from the worktree floor: a commit target is never a "working tree".
const GIT_COMMIT_FLOOR: &str = "git commit";
/// Length of the git-style short commit OID used for head labels (git's default).
const SHORT_OID_LEN: usize = 7;

/// Path-private display view-model for a Revision target.
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

/// Insert a derived `targetDisplay` into the `revision` object of a serialized
/// `/api/revision` document, leaving every existing field (including the verbatim
/// `target`) in place. A no-op if `revision` is not an object.
fn splice_target_display(
    document: &mut serde_json::Value,
    target_display: TargetDisplay,
) -> Result<(), String> {
    let value = serde_json::to_value(target_display).map_err(|error| error.to_string())?;
    if let Some(revision) = document
        .get_mut("revision")
        .and_then(|ru| ru.as_object_mut())
    {
        revision.insert("targetDisplay".to_owned(), value);
    }
    Ok(())
}

/// Re-key each shared list entry into the inspector's snapshot vocabulary and
/// attach the derived, additive display fields.
fn to_unit_entry_documents(
    entries: Vec<RevisionListEntry>,
    mut overviews: BTreeMap<String, RevisionOverviewDocument>,
) -> Result<Vec<RevisionEntryDocument>, String> {
    entries
        .into_iter()
        .map(|entry| {
            // Exhaustive destructure: a new shared field fails compilation here
            // and forces a deliberate naming decision on the inspector wire.
            let RevisionListEntry {
                captured_at,
                revision_id,
                object_id,
                source,
                base,
                target,
                object_artifact_content_hash,
                commit_range,
                merge_status,
                grouped_revision_ids,
            } = entry;
            let target_display = derive_target_display(&target, &base);
            let overview = overviews.remove(revision_id.as_str()).ok_or_else(|| {
                format!("missing overview for revision: {}", revision_id.as_str())
            })?;
            Ok(RevisionEntryDocument {
                captured_at,
                revision_id,
                snapshot_id: object_id,
                source,
                base,
                target,
                snapshot_content_hash: object_artifact_content_hash,
                commit_range,
                merge_status,
                grouped_revision_ids,
                target_display,
                overview,
            })
        })
        .collect()
}

/// Server-side overview seam for `/api/revisions`. One store-wide pass
/// (`show_revision_overviews`) builds every revision's overview, replacing the
/// per-revision N+1 (`show_revision` once per revision, each re-reading and
/// re-folding the whole event log). The client-facing JSON contract is unchanged;
/// an `eventSetHash`-keyed projection cache can still layer on top (#255).
fn revision_overviews(
    repo: &Path,
    entries: &[RevisionListEntry],
) -> Result<BTreeMap<String, RevisionOverviewDocument>, String> {
    // Build overviews for exactly the listed entries (orphan-hidden and grouped-away
    // captures are not among them). The overview slice reads no member readbacks or
    // principal diagnostics, so the verification-policy / actor-attributes /
    // delegation-map inputs are dropped; the trust set is retained — it drives the
    // operative-removal decision behind file_count/row_count.
    let overviews = show_revision_overviews(
        RevisionOverviewsOptions::new(repo)
            .with_revisions(entries.iter().map(|entry| entry.revision_id.clone()))
            .with_read_for_display(true)
            .with_trust_set(crate::cli::common::discover_trust_set(repo)),
    )
    .map_err(|error| {
        tracing::debug!(error = %error, "inspect_unit_overviews_failed");
        format!("revision overviews not available: {error}")
    })?;

    let mut documents = BTreeMap::new();
    for entry in entries {
        let revision_id = entry.revision_id.as_str().to_owned();
        let overview = overviews
            .get(&entry.revision_id)
            .ok_or_else(|| format!("revision overview not available: {revision_id}"))?;
        documents.insert(
            revision_id,
            revision_overview_document(overview, &entry.captured_at),
        );
    }

    Ok(documents)
}

/// Advisory count of a revision's review facts that target a now-superseded revision. Non-zero only
/// when the revision itself is superseded; sums the four review-fact families (observations, input
/// requests, assessments, validation checks). Adapter notes are excluded (ingestion provenance, not a
/// review assertion). Never gates — it feeds an attention badge only.
fn stale_review_fact_count(
    superseded_by: &BTreeSet<RevisionId>,
    summary: &RevisionProjectionSummary,
) -> usize {
    if superseded_by.is_empty() {
        0
    } else {
        summary.observation_count
            + summary.input_request_count
            + summary.assessment_count
            + summary.validation_check_count
    }
}

fn revision_overview_document(
    result: &RevisionOverview,
    captured_at: &str,
) -> RevisionOverviewDocument {
    let summary = &result.summary;
    RevisionOverviewDocument {
        current_assessment: overview_current_assessment(&result.current_assessment.status),
        attention: RevisionAttentionDocument {
            unassessed: result.current_assessment.status == CurrentAssessmentStatus::Unassessed,
            accepted_with_follow_up: current_assessment_includes_follow_up(
                &result.current_assessment.status,
            ),
            open_input_request_count: result
                .input_requests
                .iter()
                .filter(|request| request.status == InputRequestStatus::Open)
                .count(),
            failed_validation_count: result
                .validation_checks
                .iter()
                .filter(|check| check.status == ValidationStatus::Failed)
                .count(),
            errored_validation_count: result
                .validation_checks
                .iter()
                .filter(|check| check.status == ValidationStatus::Errored)
                .count(),
            stale_fact_count: stale_review_fact_count(&result.superseded_by, summary),
        },
        counts: RevisionOverviewCounts {
            files: summary.file_count,
            rows: summary.row_count,
            observations: summary.observation_count,
            input_requests: summary.input_request_count,
            assessments: summary.assessment_count,
            validation_checks: summary.validation_check_count,
            adapter_notes: summary.adapter_note_count,
        },
        latest_activity: latest_revision_activity(result, captured_at),
    }
}

fn overview_current_assessment(
    status: &CurrentAssessmentStatus,
) -> RevisionOverviewAssessmentDocument {
    RevisionOverviewAssessmentDocument {
        status: status.as_str(),
        assessment: match status {
            CurrentAssessmentStatus::Resolved(assessment) => Some(*assessment),
            CurrentAssessmentStatus::Unassessed | CurrentAssessmentStatus::Ambiguous(_) => None,
        },
    }
}

fn current_assessment_includes_follow_up(status: &CurrentAssessmentStatus) -> bool {
    match status {
        CurrentAssessmentStatus::Resolved(ReviewAssessment::AcceptedWithFollowUp) => true,
        CurrentAssessmentStatus::Ambiguous(assessments) => {
            assessments.contains(&ReviewAssessment::AcceptedWithFollowUp)
        }
        CurrentAssessmentStatus::Unassessed | CurrentAssessmentStatus::Resolved(_) => false,
    }
}

fn latest_revision_activity(
    result: &RevisionOverview,
    captured_at: &str,
) -> Option<RevisionLatestActivityDocument> {
    let mut latest = Some(RevisionLatestActivityDocument {
        kind: "revision",
        title: "Revision captured".to_owned(),
        at: captured_at.to_owned(),
    });

    for observation in &result.observations {
        set_latest_activity(
            &mut latest,
            "observation",
            observation.title.clone(),
            observation.created_at.clone(),
        );
    }
    for request in &result.input_requests {
        set_latest_activity(
            &mut latest,
            "input_request",
            request.title.clone(),
            request.created_at.clone(),
        );
        for response in &request.responses {
            set_latest_activity(
                &mut latest,
                "input_request",
                "Input request response".to_owned(),
                response.created_at.clone(),
            );
        }
    }
    for assessment in &result.assessments {
        let title = assessment
            .summary
            .clone()
            .unwrap_or_else(|| format!("Assessment: {}", assessment_label(assessment.assessment)));
        set_latest_activity(
            &mut latest,
            "assessment",
            title,
            assessment.created_at.clone(),
        );
    }
    for check in &result.validation_checks {
        let at = check
            .completed_at
            .clone()
            .unwrap_or_else(|| check.created_at.clone());
        set_latest_activity(&mut latest, "validation", check.check_name.clone(), at);
    }
    for note in &result.adapter_notes {
        if let Some(created_at) = &note.created_at {
            set_latest_activity(
                &mut latest,
                "adapter_note",
                note.title.clone(),
                created_at.clone(),
            );
        }
    }

    latest
}

fn set_latest_activity(
    latest: &mut Option<RevisionLatestActivityDocument>,
    kind: &'static str,
    title: String,
    at: String,
) {
    if latest
        .as_ref()
        .is_none_or(|current| at.as_str() > current.at.as_str())
    {
        *latest = Some(RevisionLatestActivityDocument { kind, title, at });
    }
}

fn assessment_label(assessment: ReviewAssessment) -> &'static str {
    match assessment {
        ReviewAssessment::Accepted => "accepted",
        ReviewAssessment::AcceptedWithFollowUp => "accepted with follow-up",
        ReviewAssessment::NeedsChanges => "needs changes",
        ReviewAssessment::NeedsClarification => "needs clarification",
    }
}

/// The advisory verification + reader enrichment config for the base projection.
/// The library cannot reach these `pub(crate)` `discover_*` helpers (INV-8), so
/// the binary builds the config here and passes it in — the single call site
/// (reused by the projection cache in a later phase).
fn inspect_base_config(repo: &Path) -> BaseProjectionConfig {
    BaseProjectionConfig {
        verification_policy: Some(EventVerificationPolicy::advisory()),
        trust_set: crate::cli::common::discover_trust_set(repo),
        actor_attributes: crate::cli::common::discover_actor_attributes(repo),
        delegation_map: crate::cli::common::discover_delegation_map(repo),
        removal_policy: shoreline::session::RemovalPolicy::default(),
    }
}

/// The server-side query surface for `/api/history`: reuse the cached
/// body-hydrated base projection (rebuilt only when the store's head marker
/// moves — #255 / INV-5), then run the pure `apply_history_query` (filter →
/// facets → order → window). `query`/`page` carry the parsed params; the
/// `at` › `offset` precedence lives inside `apply_history_query`.
pub(super) fn history_json(
    repo: &Path,
    cache: &super::cache::HistoryProjectionCache,
    query: &HistoryQuery,
    page: &HistoryPage,
) -> Result<String, String> {
    // The cheap monotonic head marker is the per-request change detector (no
    // event-byte decode — plan 0090); the cached base's `eventSetHash` is the
    // served stamp. Build-once/serve-many across all queries for one store version.
    let marker = event_log_head_marker(repo).map_err(|error| error.to_string())?;
    let config = inspect_base_config(repo);
    let base = cache.get_or_build(marker, || {
        history_base_projection(repo, &config).map_err(|error| error.to_string())
    })?;
    let out = apply_history_query(&base, query, page);
    let payload = HistoryPayload {
        schema: "shore.inspect-history",
        event_set_hash: out.event_set_hash,
        event_count: out.event_count,
        history_count: out.entries.len(),
        entries: out.entries,
        facets: out.facets,
        match_count: out.match_count,
        offset: out.offset,
        match_index: out.match_index,
        diagnostics: out.diagnostics,
    };
    serde_json::to_string(&payload).map_err(|error| error.to_string())
}

/// Captured Revisions with their base/target/snapshot identity.
pub(super) fn revisions_json(repo: &Path) -> Result<String, String> {
    let result = list_revisions(RevisionListOptions::new(repo).with_read_for_display(true))
        .map_err(|error| error.to_string())?;
    let overviews = revision_overviews(repo, &result.entries)?;
    let payload = RevisionsPayload {
        schema: "shore.inspect-revisions",
        event_set_hash: result.event_set_hash,
        event_count: result.event_count,
        revision_count: result.revision_count,
        entries: to_unit_entry_documents(result.entries, overviews)?,
        diagnostics: result.diagnostics,
    };
    serde_json::to_string(&payload).map_err(|error| error.to_string())
}

/// The supersession-DAG threads (the connected components of the supersession
/// graph, labeled domain-side), each with its competing heads and superseded
/// revisions. Fork-tolerant: never a null head, never a "malformed" error.
pub(super) fn threads_json(repo: &Path) -> Result<String, String> {
    let (events, display_diagnostics) =
        read_events_for_display(repo).map_err(|error| error.to_string())?;
    let state = SessionState::from_events(&events).map_err(|error| error.to_string())?;
    let view = SupersessionView::from_events(&events).map_err(|error| error.to_string())?;

    let threads = view
        .components
        .iter()
        .map(|component| {
            let heads: Vec<String> = component
                .intersection(&view.heads)
                .map(|revision| revision.as_str().to_owned())
                .collect();
            let superseded: Vec<String> = component
                .intersection(&view.superseded)
                .map(|revision| revision.as_str().to_owned())
                .collect();
            Ok(ThreadDocument {
                revisions: component
                    .iter()
                    .map(|revision| revision.as_str().to_owned())
                    .collect(),
                competing: heads.len() > 1,
                heads,
                superseded,
                laid_out: thread_layout(component, &view)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    // Build everything that borrows `view` before moving `view.diagnostics` out.
    let supersedes = revision_edge_map(&view.supersedes);
    let superseded_by = revision_edge_map(&view.superseded_by);
    let classification = revision_classification(&view);

    let mut diagnostics = view.diagnostics;
    diagnostics.extend(display_diagnostics);
    let payload = ThreadsPayload {
        schema: "shore.inspect-threads",
        event_set_hash: state.event_set_hash.unwrap_or_default(),
        event_count: state.event_count,
        thread_count: threads.len(),
        threads,
        supersedes,
        superseded_by,
        revision_classification: classification,
        diagnostics,
    };
    serde_json::to_string(&payload).map_err(|error| error.to_string())
}

/// The short display form the client paints inside a DAG node (`shortId`): the
/// segment after the last `:`, capped at 12 chars. Used only to size the layout
/// boxes to the painted text; the node id is unaffected.
fn short_node_label(id: &str) -> String {
    let tail = id.rsplit(':').next().unwrap_or("");
    tail.chars().take(12).collect()
}

/// One node fed to `layout_supersession_graph`: an opaque id, the short label used
/// to SIZE the box (never for identity), and the projection-derived head/superseded
/// standing. Node-id-agnostic — shared by the revision and fact adapters.
struct SupersessionLayoutNode {
    id: String,
    label: String,
    is_head: bool,
    is_superseded: bool,
}

/// One directed supersession edge fed to the layout: `from` supersedes `to`, tagged
/// with its relation kind (the tagged-edge model).
struct SupersessionLayoutEdge {
    from: String,
    to: String,
    kind: Option<&'static str>,
}

/// Lay out a supersession graph server-side via `mmdflux`, node-id-agnostic. One
/// node per input (insertion order = input order, so callers control column
/// stability), one edge `from -> to` (from supersedes to). Normalizes to a (0,0)
/// origin over the content bounding box, inset by `NODE_STROKE_MARGIN`. The engine
/// never sees the id's meaning; the label only sizes the box.
fn layout_supersession_graph(
    nodes: &[SupersessionLayoutNode],
    edges: &[SupersessionLayoutEdge],
) -> Result<ThreadLayout, String> {
    let mut graph = Graph::new(Direction::TopDown);
    for node in nodes {
        // Size the box to the SHORT form the client actually paints, not the full
        // id — otherwise mmdflux measures the long id and the boxes (and the whole
        // graph) blow up, so the painted short text scales tiny. The node id still
        // round-trips verbatim; the label only drives sizing.
        graph.add_node(Node::new(node.id.as_str()).with_label(node.label.clone()));
    }
    for edge in edges {
        graph.add_edge(Edge::new(edge.from.as_str(), edge.to.as_str()));
    }
    let laid =
        layout_graph(&graph, &LayoutOptions::default()).map_err(|error| error.to_string())?;

    let kinds: std::collections::HashMap<(&str, &str), Option<&'static str>> = edges
        .iter()
        .map(|e| ((e.from.as_str(), e.to.as_str()), e.kind))
        .collect();
    let is_head = |id: &str| nodes.iter().any(|n| n.id.as_str() == id && n.is_head);
    let is_superseded = |id: &str| nodes.iter().any(|n| n.id.as_str() == id && n.is_superseded);

    // mmdflux's bounds are extents not guaranteed to start at the origin, so
    // shift to a (0,0) top-left over the real content and emit the content
    // extent (max - min), not laid.width/height directly. Inset by a margin so a
    // node stroke (drawn CENTERED on the box edge, up to ~half a stroke-width
    // outside the box) is not clipped at the viewBox edge.
    let (min_x, min_y, max_x, max_y) = content_bounds(&laid);
    let (origin_x, origin_y) = (min_x - NODE_STROKE_MARGIN, min_y - NODE_STROKE_MARGIN);
    Ok(ThreadLayout {
        nodes: laid
            .nodes
            .iter()
            .map(|n| LaidOutNodeWire {
                id: n.id.clone(),
                x: n.center.x - origin_x,
                y: n.center.y - origin_y,
                w: n.width,
                h: n.height,
                is_head: is_head(&n.id),
                is_superseded: is_superseded(&n.id),
            })
            .collect(),
        edges: laid
            .edges
            .iter()
            .map(|e| LaidOutEdgeWire {
                from: e.from.clone(),
                to: e.to.clone(),
                path: e
                    .points
                    .iter()
                    .map(|p| [p.x - origin_x, p.y - origin_y])
                    .collect(),
                kind: kinds
                    .get(&(e.from.as_str(), e.to.as_str()))
                    .copied()
                    .flatten(),
            })
            .collect(),
        bounds: LayoutBounds {
            w: (max_x - min_x) + 2.0 * NODE_STROKE_MARGIN,
            h: (max_y - min_y) + 2.0 * NODE_STROKE_MARGIN,
        },
    })
}

/// Lay out one revision supersession thread by adapting it onto the generic
/// `layout_supersession_graph` core. The graph is `TopDown`, one node per revision
/// (iterated over the `BTreeSet` component for stable columns), one edge `B -> A`
/// for each `A` that `B` supersedes (so `from` supersedes `to`). The engine ranks
/// in-degree-0 heads as equal rank-0 peers, so competing heads stay equal by
/// construction. `isHead`/`isSuperseded` come from the projection, not the engine.
/// Revision edges carry no `kind`, so the `/api/threads` wire is byte-identical.
fn thread_layout(
    component: &BTreeSet<RevisionId>,
    view: &SupersessionView,
) -> Result<ThreadLayout, String> {
    let nodes: Vec<SupersessionLayoutNode> = component
        .iter()
        .map(|revision| SupersessionLayoutNode {
            id: revision.as_str().to_owned(),
            label: short_node_label(revision.as_str()),
            is_head: view.heads.contains(revision),
            is_superseded: view.superseded.contains(revision),
        })
        .collect();
    let mut edges = Vec::new();
    for revision in component {
        if let Some(targets) = view.supersedes.get(revision) {
            for target in targets {
                if component.contains(target) {
                    edges.push(SupersessionLayoutEdge {
                        from: revision.as_str().to_owned(),
                        to: target.as_str().to_owned(),
                        kind: None,
                    });
                }
            }
        }
    }
    layout_supersession_graph(&nodes, &edges)
}

/// Padding (user units) added around the content on every side so a node box's
/// centered stroke — widest on a selected/focused node — is never clipped at the
/// viewBox edge. Comfortably covers the largest stroke the client paints.
const NODE_STROKE_MARGIN: f64 = 4.0;

/// The content bounding box over node boxes (center +/- half-size) and routed
/// edge points. Falls back to the engine's own extent for an empty graph.
fn content_bounds(laid: &LaidOutGraph) -> (f64, f64, f64, f64) {
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for n in &laid.nodes {
        min_x = min_x.min(n.center.x - n.width / 2.0);
        max_x = max_x.max(n.center.x + n.width / 2.0);
        min_y = min_y.min(n.center.y - n.height / 2.0);
        max_y = max_y.max(n.center.y + n.height / 2.0);
    }
    for e in &laid.edges {
        for p in &e.points {
            min_x = min_x.min(p.x);
            max_x = max_x.max(p.x);
            min_y = min_y.min(p.y);
            max_y = max_y.max(p.y);
        }
    }
    if min_x.is_finite() {
        (min_x, min_y, max_x, max_y)
    } else {
        (0.0, 0.0, laid.width, laid.height)
    }
}

/// Classify every known revision (head / superseded / isolated) with its direct
/// superseders and predecessors, from the supersession projection. Iterates the
/// components so isolated roots (no edges) are still classified. Deterministic:
/// built from `BTreeMap`/`BTreeSet`, keyed into a `BTreeMap<String, _>`.
fn revision_classification(view: &SupersessionView) -> BTreeMap<String, RevisionClassification> {
    let mut map = BTreeMap::new();
    for component in &view.components {
        for revision in component {
            let superseded_by = view
                .superseded_by
                .get(revision)
                .map(|s| s.iter().map(|r| r.as_str().to_owned()).collect())
                .unwrap_or_default();
            let supersedes = view
                .supersedes
                .get(revision)
                .map(|s| s.iter().map(|r| r.as_str().to_owned()).collect())
                .unwrap_or_default();
            let state = if view.superseded.contains(revision) {
                "superseded"
            } else if view.supersedes.contains_key(revision) {
                "head" // a current head that supersedes at least one predecessor
            } else {
                "isolated" // a lone root: a current head with no incident edges
            };
            map.insert(
                revision.as_str().to_owned(),
                RevisionClassification {
                    state,
                    superseded_by,
                    supersedes,
                },
            );
        }
    }
    map
}

/// Flatten the projection's `RevisionId -> {RevisionId}` adjacency into a wire
/// map of id strings, dropping any empty entry so only revisions that actually
/// carry an edge appear.
fn revision_edge_map(
    edges: &std::collections::BTreeMap<RevisionId, std::collections::BTreeSet<RevisionId>>,
) -> BTreeMap<String, Vec<String>> {
    edges
        .iter()
        .filter(|(_, targets)| !targets.is_empty())
        .map(|(revision, targets)| {
            (
                revision.as_str().to_owned(),
                targets.iter().map(|t| t.as_str().to_owned()).collect(),
            )
        })
        .collect()
}

/// The captured diff snapshot for one Revision, by snapshot id and optional bound
/// object artifact content hash.
///
/// Reads the immutable object artifact through the validated read path
/// (`read_object_artifact` recomputes and checks the content hash), so the
/// inspector renders exactly the frozen diff that was reviewed.
///
/// The wire shape redacts the hash-baked `target.worktreeRoot` after
/// validation: a linked inspector serves snapshots captured in sibling
/// worktrees, and their raw absolute paths must not reach other readers. The
/// stored artifact is untouched, so `contentHashScope: "stored-artifact"`
/// records that `contentHash` covers the stored bytes (including the redacted
/// field) — consumers re-validate by fetching the artifact, not by hashing
/// this wire JSON.
pub(super) fn snapshot_json(
    repo: &Path,
    snapshot_id: &str,
    content_hash: Option<&str>,
    cache: Option<&RwLock<HighlightCache>>,
) -> Result<String, String> {
    if snapshot_id.is_empty() {
        return Err("missing snapshot id".to_owned());
    }
    let object_id = ObjectId::new(snapshot_id.to_owned());
    let artifact = match content_hash {
        Some(content_hash) => read_bound_object_artifact(repo, &object_id, content_hash),
        None => read_object_artifact(repo, &object_id),
    }
    .map_err(|error| {
        // Keep the full error (which may include the internal artifact path)
        // in the server trace, but return a path-free message to the client.
        tracing::debug!(
            error = %error,
            snapshot = snapshot_id,
            content_hash,
            "inspect_snapshot_read_failed"
        );
        format!("snapshot not found or unreadable: {snapshot_id}")
    })?;
    // Key the cache on the VALIDATED artifact's content hash, never the raw request param: the read
    // above already decoded and hash-checked the artifact, so a cache hit cannot bypass that check.
    let cache_key = artifact.content_hash.clone();
    if let Some(cache) = cache
        && let Some(cached) = cache.read().ok().and_then(|cache| cache.get(&cache_key))
    {
        return Ok(cached);
    }
    // Build the enriched wire DTO from the validated artifact: it mirrors the stored serialized
    // shape (identity/endpoints already absent on the v2 body) and additively carries per-row syntax
    // tokens and intraline emphasis. The stored bytes are untouched. Round-trip through
    // `serde_json::Value` so the served key order is unchanged from before this DTO existed (an
    // unhighlighted, unemphasized row is byte-identical).
    let wire = WireObjectArtifact::from_artifact(&artifact, highlight_file, emphasis_file);
    let value = serde_json::to_value(&wire).map_err(|error| error.to_string())?;
    let body = serde_json::to_string(&value).map_err(|error| error.to_string())?;
    // Content-addressed: the body is immutable for this hash, so caching needs no invalidation.
    if let Some(cache) = cache
        && let Ok(mut cache) = cache.write()
    {
        cache.put(&cache_key, body.clone());
    }
    Ok(body)
}

/// The full composite projection for one Revision.
///
/// Build the fork-gated fact graphs for one revision from its show result. A
/// layout error degrades that fact type to `None` (advisory, best-effort — never
/// fails the composite page).
fn fact_supersession_document(result: &RevisionShowResult) -> FactSupersessionDocument {
    let assessments = matches!(
        result.current_assessment.status,
        CurrentAssessmentStatus::Ambiguous(_)
    )
    .then(|| assessment_fact_graph(&result.assessments))
    .flatten();

    let observations = result
        .observations
        .iter()
        .any(|o| o.status == ObservationStatus::Superseded)
        .then(|| observation_fact_graph(&result.observations))
        .flatten();

    FactSupersessionDocument {
        assessments,
        observations,
    }
}

fn assessment_fact_graph(assessments: &[AssessmentView]) -> Option<FactGraphDocument> {
    let ids: std::collections::BTreeSet<&str> = assessments.iter().map(|a| a.id.as_str()).collect();
    let nodes: Vec<SupersessionLayoutNode> = assessments
        .iter()
        .map(|a| SupersessionLayoutNode {
            id: a.id.as_str().to_owned(),
            label: short_node_label(a.id.as_str()),
            is_head: a.status == AssessmentRecordStatus::Current,
            is_superseded: a.status == AssessmentRecordStatus::Replaced,
        })
        .collect();
    let mut edges = Vec::new();
    for a in assessments {
        for replaced in &a.replaces {
            if ids.contains(replaced.as_str()) {
                edges.push(SupersessionLayoutEdge {
                    from: a.id.as_str().to_owned(),
                    to: replaced.as_str().to_owned(),
                    kind: Some("replaces"),
                });
            }
        }
    }
    layout_or_log(&nodes, &edges, "assessment")
}

fn observation_fact_graph(observations: &[ObservationView]) -> Option<FactGraphDocument> {
    let ids: std::collections::BTreeSet<&str> =
        observations.iter().map(|o| o.id.as_str()).collect();
    let nodes: Vec<SupersessionLayoutNode> = observations
        .iter()
        .map(|o| SupersessionLayoutNode {
            id: o.id.as_str().to_owned(),
            label: short_node_label(o.id.as_str()),
            is_head: o.status == ObservationStatus::Active,
            is_superseded: o.status == ObservationStatus::Superseded,
        })
        .collect();
    let mut edges = Vec::new();
    for o in observations {
        for superseded in &o.supersedes {
            if ids.contains(superseded.as_str()) {
                edges.push(SupersessionLayoutEdge {
                    from: o.id.as_str().to_owned(),
                    to: superseded.as_str().to_owned(),
                    kind: Some("supersedes"),
                });
            }
        }
    }
    layout_or_log(&nodes, &edges, "observation")
}

fn layout_or_log(
    nodes: &[SupersessionLayoutNode],
    edges: &[SupersessionLayoutEdge],
    kind: &str,
) -> Option<FactGraphDocument> {
    match layout_supersession_graph(nodes, edges) {
        Ok(laid_out) => Some(FactGraphDocument { laid_out }),
        Err(error) => {
            tracing::debug!(error = %error, fact_kind = kind, "fact_supersession_layout_failed");
            None
        }
    }
}

/// Splice the inspector-private `factSupersession` field into the composite value.
/// A no-op (omits the key) when neither fact type forks, so the wire stays
/// byte-identical for the common case.
fn splice_fact_supersession(
    value: &mut serde_json::Value,
    doc: FactSupersessionDocument,
) -> Result<(), String> {
    if doc.is_empty() {
        return Ok(());
    }
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "factSupersession".to_owned(),
            serde_json::to_value(&doc).map_err(|error| error.to_string())?,
        );
    }
    Ok(())
}

/// Reuses the exact `shore.review-revision` document the `shore revision show`
/// command builds (`revision_show_document`), so the inspector renders the same
/// authoritative composite — current-assessment status, duplicate-collapsed
/// facts, supersession, adapter notes, and projection rows — rather than
/// re-deriving it client-side.
pub(super) fn revision_json(repo: &Path, revision_id: &str) -> Result<String, String> {
    if revision_id.is_empty() {
        return Err("missing revision id".to_owned());
    }
    let mut show_options = RevisionShowOptions::new(repo)
        .with_revision_id(RevisionId::new(revision_id.to_owned()))
        // The inspector addresses a specific revision by id (e.g. a superseded
        // DAG node), so resolve it exactly rather than forward-resolving to a
        // thread head (which errors on a competing fork).
        .with_exact(true)
        .with_include_body(true)
        .with_read_for_display(true)
        .with_verification_policy(EventVerificationPolicy::advisory())
        .with_trust_set(crate::cli::common::discover_trust_set(repo))
        .with_actor_attributes(crate::cli::common::discover_actor_attributes(repo));
    if let Some(map) = crate::cli::common::discover_delegation_map(repo) {
        show_options = show_options.with_delegation_map(map);
    }
    let result = show_revision(show_options).map_err(|error| {
        tracing::debug!(error = %error, revision = revision_id, "inspect_unit_read_failed");
        format!("revision not found or unreadable: {revision_id}")
    })?;
    // Thread the typed endpoints and the commit-range view out before
    // `revision_show_document` consumes `result`, then splice the additive
    // `targetDisplay` into the serialized document.
    let target_display = derive_target_display(&result.revision.target, &result.revision.base);
    let head_oid = match &result.revision.base {
        ReviewEndpoint::GitCommit { commit_oid, .. } => Some(commit_oid.clone()),
        ReviewEndpoint::GitWorkingTree { .. } => None,
    };
    let commit_range = result.commit_range.clone();
    // Build the inspector-private fact graphs while `result` is still live; it is
    // moved into `revision_show_document` on the next line.
    let fact_supersession = fact_supersession_document(&result);
    let document = revision_show_document(result);
    let mut value = serde_json::to_value(&document).map_err(|error| error.to_string())?;
    splice_target_display(&mut value, target_display)?;
    splice_fact_supersession(&mut value, fact_supersession)?;

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

/// Insert `liveBranch` into the spliced `revision.targetDisplay.head` object.
/// A no-op if the head block is absent (e.g. a working-tree base with no head).
fn set_head_live_branch(document: &mut serde_json::Value, live_branch: String) {
    if let Some(head) = document
        .get_mut("revision")
        .and_then(|revision| revision.get_mut("targetDisplay"))
        .and_then(|target_display| target_display.get_mut("head"))
        .and_then(|head| head.as_object_mut())
    {
        head.insert("liveBranch".to_owned(), live_branch.into());
    }
}

/// Cheap freshness probe for client-side auto-refresh polling.
///
/// Returns the event-log head marker (the event count) as `eventCount`, computed
/// without reading or decoding any event bytes. The client compares it across
/// polls and re-fetches only when it moves — replacing the old per-poll full read
/// and event-set-hash recompute. The event-set hash remains the authoritative
/// confirm stamp on the full-read endpoints (`/api/history`, `/api/revisions`).
pub(super) fn freshness_json(repo: &Path) -> Result<String, String> {
    let event_count = event_log_head_marker(repo).map_err(|error| error.to_string())?;
    let payload = FreshnessPayload {
        schema: "shore.inspect-freshness",
        event_count,
    };
    serde_json::to_string(&payload).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use shoreline::model::{
        EngagementId, ObjectId, ReviewEndpoint, RevisionId, RevisionSource, WorktreeCaptureMode,
    };
    use shoreline::session::event::{
        GitProvenance, Revision, WorkObjectProposal, WorkObjectProposedPayload,
    };

    use super::*;

    #[test]
    fn stale_review_fact_count_sums_review_facts_only_when_superseded() {
        use shoreline::session::RevisionProjectionSummary;

        let summary = RevisionProjectionSummary {
            observation_count: 2,
            input_request_count: 1,
            assessment_count: 1,
            validation_check_count: 3,
            adapter_note_count: 5, // excluded — not a review fact
            ..Default::default()
        };

        // Superseded ⇒ the four review families (2 + 1 + 1 + 3 = 7); adapter notes excluded.
        let superseded: BTreeSet<RevisionId> = [RevisionId::new("rev:sha256:successor")]
            .into_iter()
            .collect();
        assert_eq!(stale_review_fact_count(&superseded, &summary), 7);

        // Head (empty superseders) ⇒ zero, regardless of fact counts.
        assert_eq!(stale_review_fact_count(&BTreeSet::new(), &summary), 0);
    }

    #[test]
    fn revision_classification_marks_head_superseded_and_isolated() {
        use shoreline::session::SupersessionView;
        // A <- B (B supersedes A), plus an isolated root Z.
        let view = SupersessionView::from_edges([
            (RevisionId::new("A"), vec![]),
            (RevisionId::new("B"), vec![RevisionId::new("A")]),
            (RevisionId::new("Z"), vec![]),
        ]);

        let map = revision_classification(&view);

        assert_eq!(map["B"].state, "head");
        assert_eq!(map["A"].state, "superseded");
        assert_eq!(map["A"].superseded_by, vec!["B".to_owned()]);
        assert_eq!(map["B"].supersedes, vec!["A".to_owned()]);
        assert_eq!(map["Z"].state, "isolated");
    }

    #[test]
    fn thread_layout_lays_out_a_fork_as_equal_peers() {
        use shoreline::session::SupersessionView;
        // A superseded by two heads B and C: heads {B, C}, superseded {A}.
        let view = SupersessionView::from_edges([
            (RevisionId::new("A"), vec![]),
            (RevisionId::new("B"), vec![RevisionId::new("A")]),
            (RevisionId::new("C"), vec![RevisionId::new("A")]),
        ]);
        let component: std::collections::BTreeSet<RevisionId> =
            ["A", "B", "C"].into_iter().map(RevisionId::new).collect();

        let laid = thread_layout(&component, &view).expect("layout");

        // Topology, never pixels.
        assert_eq!(laid.nodes.len(), 3);
        let head_ids: Vec<&str> = laid
            .nodes
            .iter()
            .filter(|n| n.is_head)
            .map(|n| n.id.as_str())
            .collect();
        assert_eq!(head_ids.len(), 2, "two equal-peer heads: {head_ids:?}");
        assert!(head_ids.contains(&"B") && head_ids.contains(&"C"));
        let superseded: Vec<&str> = laid
            .nodes
            .iter()
            .filter(|n| n.is_superseded)
            .map(|n| n.id.as_str())
            .collect();
        assert_eq!(superseded, vec!["A"]);

        // Each edge B->A / C->A: `from` supersedes `to`, every edge points at A.
        assert_eq!(laid.edges.len(), 2);
        for e in &laid.edges {
            assert_eq!(e.to, "A");
            assert_ne!(e.from, "A");
            assert!(e.path.len() >= 2, "a routed polyline has >=2 points");
        }

        // Normalized to a (0,0) origin: every node box fits the emitted bounds.
        assert!(laid.bounds.w > 0.0 && laid.bounds.h > 0.0);
        for n in &laid.nodes {
            assert!(n.x - n.w / 2.0 >= -0.5 && n.x + n.w / 2.0 <= laid.bounds.w + 0.5);
            assert!(n.y - n.h / 2.0 >= -0.5 && n.y + n.h / 2.0 <= laid.bounds.h + 0.5);
        }

        // Competing heads share a rank (equal y); no node sits above them.
        let head_ys: Vec<f64> = laid
            .nodes
            .iter()
            .filter(|n| n.is_head)
            .map(|n| n.y)
            .collect();
        assert!((head_ys[0] - head_ys[1]).abs() < 1.0, "heads share a rank");
        let min_head_y = head_ys.iter().cloned().fold(f64::INFINITY, f64::min);
        for n in &laid.nodes {
            if !n.is_head {
                assert!(n.y >= min_head_y, "no non-head node above a head");
            }
        }
    }

    #[test]
    fn layout_core_lays_out_tagged_edges_over_opaque_ids() {
        // A fact-shaped fork: node "a1" (replaced), replaced by two current heads "a2", "a3".
        let nodes = vec![
            SupersessionLayoutNode {
                id: "a1".into(),
                label: "a1".into(),
                is_head: false,
                is_superseded: true,
            },
            SupersessionLayoutNode {
                id: "a2".into(),
                label: "a2".into(),
                is_head: true,
                is_superseded: false,
            },
            SupersessionLayoutNode {
                id: "a3".into(),
                label: "a3".into(),
                is_head: true,
                is_superseded: false,
            },
        ];
        let edges = vec![
            SupersessionLayoutEdge {
                from: "a2".into(),
                to: "a1".into(),
                kind: Some("replaces"),
            },
            SupersessionLayoutEdge {
                from: "a3".into(),
                to: "a1".into(),
                kind: Some("replaces"),
            },
        ];

        let laid = layout_supersession_graph(&nodes, &edges).expect("layout");

        // Topology, never pixels.
        assert_eq!(laid.nodes.len(), 3);
        assert_eq!(laid.edges.len(), 2);
        for e in &laid.edges {
            assert_eq!(e.to, "a1");
            assert_ne!(e.from, "a1");
            assert_eq!(
                e.kind,
                Some("replaces"),
                "the edge kind rides through the layout"
            );
            assert!(e.path.len() >= 2);
        }
        let heads: Vec<&str> = laid
            .nodes
            .iter()
            .filter(|n| n.is_head)
            .map(|n| n.id.as_str())
            .collect();
        assert_eq!(heads.len(), 2);
        assert!(laid.bounds.w > 0.0 && laid.bounds.h > 0.0);
    }

    #[test]
    fn revision_thread_edges_omit_the_kind_field_on_the_wire() {
        use shoreline::session::SupersessionView;
        // A <- B (B supersedes A): the revision adapter must pass kind = None so the
        // /api/threads wire is byte-identical to before this field existed.
        let view = SupersessionView::from_edges([
            (RevisionId::new("A"), vec![]),
            (RevisionId::new("B"), vec![RevisionId::new("A")]),
        ]);
        let component: std::collections::BTreeSet<RevisionId> =
            ["A", "B"].into_iter().map(RevisionId::new).collect();

        let laid = thread_layout(&component, &view).expect("layout");
        let json = serde_json::to_value(&laid.edges[0]).expect("serialize edge");

        assert!(
            json.get("kind").is_none(),
            "revision edge must omit `kind`: {json}"
        );
        assert_eq!(laid.edges[0].from, "B");
        assert_eq!(laid.edges[0].to, "A");
    }

    #[test]
    fn thread_layout_degenerate_single_node_has_no_edges() {
        use shoreline::session::SupersessionView;
        let view = SupersessionView::from_edges([(RevisionId::new("solo"), vec![])]);
        let component: std::collections::BTreeSet<RevisionId> =
            std::iter::once(RevisionId::new("solo")).collect();

        let laid = thread_layout(&component, &view).expect("layout");
        assert_eq!(laid.nodes.len(), 1);
        assert_eq!(laid.edges.len(), 0);
        assert!(laid.nodes[0].is_head);
    }

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

    fn captured_repo() -> (tempfile::TempDir, String, String) {
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
        (
            root,
            result.object_id.as_str().to_owned(),
            result.object_artifact_content_hash,
        )
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

    /// Capture a worktree review of a single file changed from `base` to `changed`,
    /// returning the captured object id and content hash.
    fn captured_repo_with(
        file: &str,
        base: &str,
        changed: &str,
    ) -> (tempfile::TempDir, String, String) {
        let root = tempfile::tempdir().expect("create temp repo");
        let path = root.path();
        git(path, &["init"]);
        git(path, &["config", "user.name", "Shore Tests"]);
        git(path, &["config", "user.email", "shore-tests@example.com"]);
        git(path, &["config", "commit.gpgsign", "false"]);
        if let Some(parent) = Path::new(file).parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(path.join(parent)).unwrap();
        }
        std::fs::write(path.join(file), base).unwrap();
        git(path, &["add", "--all"]);
        git(path, &["commit", "-m", "base"]);
        std::fs::write(path.join(file), changed).unwrap();
        let result = shoreline::session::capture_worktree_review(
            shoreline::session::CaptureOptions::new(path),
        )
        .expect("capture worktree review");
        (
            root,
            result.object_id.as_str().to_owned(),
            result.object_artifact_content_hash,
        )
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

    fn stored_object_artifact_path(repo: &Path) -> std::path::PathBuf {
        let snapshots_dir = common_dir_store(repo).join("artifacts/objects");
        let mut entries: Vec<_> = std::fs::read_dir(&snapshots_dir)
            .expect("object artifacts dir exists")
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(entries.len(), 1, "exactly one stored object artifact");
        entries.remove(0)
    }

    #[test]
    fn threads_json_advertises_domain_named_schema() {
        let (repo, _, _) = captured_repo();
        // The threads payload advertises its domain-named schema tag, not the
        // substrate "objects" term.
        let payload: serde_json::Value =
            serde_json::from_str(&threads_json(repo.path()).unwrap()).unwrap();
        assert_eq!(payload["schema"], "shore.inspect-threads");
    }

    #[test]
    fn snapshot_json_serves_snapshot_scoped_wire() {
        let (repo, snapshot_id, _) = captured_repo();

        let wire: serde_json::Value =
            serde_json::from_str(&snapshot_json(repo.path(), &snapshot_id, None, None).unwrap())
                .unwrap();

        // Object-scoped wire: content hash + frozen diff only. Identity and
        // endpoints live on /api/revisions/{id} (from the projection), never here — so
        // the worktree root is simply absent (nothing to redact).
        assert!(wire["contentHash"].is_string());
        assert!(wire.get("revisionId").is_none());
        assert!(wire.get("source").is_none());
        assert!(wire.get("base").is_none());
        assert!(wire.get("target").is_none());
        assert!(wire.get("worktreeRootRedacted").is_none());
        assert!(wire.get("contentHashScope").is_none());
        assert!(wire.get("targetDisplay").is_none());
    }

    #[test]
    fn snapshot_json_includes_tokens_for_known_language() {
        let (repo, object_id, content_hash) = captured_repo_with(
            "src/lib.rs",
            "pub fn value() -> u32 { 1 }\n",
            "pub fn value() -> u32 { 2 }\n",
        );
        let json: serde_json::Value = serde_json::from_str(
            &snapshot_json(repo.path(), &object_id, Some(&content_hash), None).unwrap(),
        )
        .unwrap();
        let row = &json["snapshot"]["files"][0]["hunks"][0]["rows"][0];
        let tokens = row["tokens"]
            .as_array()
            .expect("highlighted row has tokens");
        assert!(!tokens.is_empty());
        assert!(
            tokens.iter().any(|t| t["kind"] == "keyword"),
            "a .rs row carries a keyword token"
        );
    }

    #[test]
    fn snapshot_json_is_byte_stable_across_two_calls() {
        let (repo, object_id, content_hash) = captured_repo_with(
            "src/lib.rs",
            "pub fn value() -> u32 { 1 }\n",
            "pub fn value() -> u32 { 2 }\n",
        );
        let cache = std::sync::RwLock::new(super::super::server::HighlightCache::new(8));
        let first =
            snapshot_json(repo.path(), &object_id, Some(&content_hash), Some(&cache)).unwrap();
        // The second call hits the content-hash cache and must return identical bytes.
        let second =
            snapshot_json(repo.path(), &object_id, Some(&content_hash), Some(&cache)).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn snapshot_body_includes_emphasis_and_is_cached() {
        // A single-word edit (`1` → `2`) on an otherwise-identical line: the shared content keeps the
        // width-ratio distance well under the guard, so the changed digit is emphasized.
        let (repo, object_id, content_hash) = captured_repo_with(
            "src/lib.rs",
            "pub fn value() -> u32 { 1 }\n",
            "pub fn value() -> u32 { 2 }\n",
        );
        let cache = std::sync::RwLock::new(super::super::server::HighlightCache::new(8));
        let first =
            snapshot_json(repo.path(), &object_id, Some(&content_hash), Some(&cache)).unwrap();
        assert!(
            first.contains("\"emphasis\""),
            "the rendered body carries intraline emphasis on the changed row"
        );
        // The second call hits the content-hash cache (unchanged key) and returns identical bytes,
        // so emphasis rides the existing cache with no key change or invalidation.
        let second =
            snapshot_json(repo.path(), &object_id, Some(&content_hash), Some(&cache)).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn snapshot_json_omits_tokens_for_unknown_language() {
        let (repo, object_id, content_hash) =
            captured_repo_with("notes.xyzzy", "alpha\n", "beta\n");
        let json: serde_json::Value = serde_json::from_str(
            &snapshot_json(repo.path(), &object_id, Some(&content_hash), None).unwrap(),
        )
        .unwrap();
        let row = &json["snapshot"]["files"][0]["hunks"][0]["rows"][0];
        // wire byte-identical to today for unhighlightable files: no `tokens` key.
        assert!(row.get("tokens").is_none());
    }

    #[test]
    fn snapshot_json_can_read_rebased_recapture_by_bound_hash() {
        let root = tempfile::tempdir().expect("create temp repo");
        let path = root.path();
        git(path, &["init"]);
        git(path, &["config", "user.name", "Shore Tests"]);
        git(path, &["config", "user.email", "shore-tests@example.com"]);
        git(path, &["config", "commit.gpgsign", "false"]);
        std::fs::create_dir_all(path.join("src")).unwrap();
        let base = (1..=12)
            .map(|line| format!("preamble {line}\n"))
            .chain(["pub fn value() -> u32 { 1 }\n".to_owned()])
            .chain((1..=6).map(|line| format!("trailer {line}\n")))
            .collect::<String>();
        std::fs::write(path.join("src/lib.rs"), &base).unwrap();
        git(path, &["add", "--all"]);
        git(path, &["commit", "-m", "base"]);

        let reviewed = base.replace("pub fn value() -> u32 { 1 }", "pub fn value() -> u32 { 2 }");
        std::fs::write(path.join("src/lib.rs"), reviewed).unwrap();
        let first = shoreline::session::capture_worktree_review(
            shoreline::session::CaptureOptions::new(path),
        )
        .expect("capture first revision");

        git(path, &["checkout", "--", "src/lib.rs"]);
        let rebased_base = format!("inserted upstream line\n{base}");
        std::fs::write(path.join("src/lib.rs"), &rebased_base).unwrap();
        git(path, &["add", "--all"]);
        git(path, &["commit", "-m", "upstream insert"]);
        let rebased_reviewed =
            rebased_base.replace("pub fn value() -> u32 { 1 }", "pub fn value() -> u32 { 2 }");
        std::fs::write(path.join("src/lib.rs"), rebased_reviewed).unwrap();
        let second = shoreline::session::capture_worktree_review(
            shoreline::session::CaptureOptions::new(path)
                .with_supersedes(vec![first.revision_id.clone()]),
        )
        .expect("capture rebased successor");

        assert_eq!(first.object_id, second.object_id);
        assert_ne!(
            first.object_artifact_content_hash,
            second.object_artifact_content_hash
        );

        let first_wire: serde_json::Value = serde_json::from_str(
            &snapshot_json(
                path,
                first.object_id.as_str(),
                Some(&first.object_artifact_content_hash),
                None,
            )
            .unwrap(),
        )
        .unwrap();
        let second_wire: serde_json::Value = serde_json::from_str(
            &snapshot_json(
                path,
                second.object_id.as_str(),
                Some(&second.object_artifact_content_hash),
                None,
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            first_wire["contentHash"],
            first.object_artifact_content_hash.as_str()
        );
        assert_eq!(
            second_wire["contentHash"],
            second.object_artifact_content_hash.as_str()
        );
    }

    #[test]
    fn snapshot_json_rejects_tampered_artifact_before_wire_shaping() {
        let (repo, snapshot_id, _) = captured_repo();
        let artifact_path = stored_object_artifact_path(repo.path());
        let mut json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&artifact_path).unwrap()).unwrap();
        // Tamper a field that is inside the content hash for both v1 and v2 (the
        // snapshot rows). `DiffFile` is snake_case, unlike the camelCase wrapper.
        json["snapshot"]["files"][0]["new_path"] = serde_json::json!("/evil");
        std::fs::write(&artifact_path, serde_json::to_vec(&json).unwrap()).unwrap();

        let error = snapshot_json(repo.path(), &snapshot_id, None, None)
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

    fn entry(worktree: &str, commit: &str) -> RevisionListEntry {
        RevisionListEntry {
            captured_at: "2026-05-13T10:00:00Z".to_owned(),
            revision_id: RevisionId::new("rev:sha256:abc"),
            object_id: ObjectId::new("snap:sha256:abc"),
            source: RevisionSource::GitWorktree {
                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                include_untracked: true,
                pathspecs: Vec::new(),
            },
            base: ReviewEndpoint::GitCommit {
                commit_oid: commit.to_owned(),
                tree_oid: "tree-oid".to_owned(),
            },
            target: ReviewEndpoint::GitWorkingTree {
                worktree_root: worktree.to_owned(),
            },
            object_artifact_content_hash: "sha256:artifact:abc".to_owned(),
            commit_range: shoreline::session::RevisionCommitRangeView {
                revision_id: RevisionId::new("review-unit:sha256:abc"),
                anchored: false,
                current_commits: Vec::new(),
                current_refs: Vec::new(),
                withdrawn_commits: Vec::new(),
                withdrawn_refs: Vec::new(),
                diagnostics: Vec::new(),
            },
            merge_status: "unknown".to_owned(),
            grouped_revision_ids: vec![RevisionId::new("review-unit:sha256:abc")],
        }
    }

    fn overview() -> RevisionOverviewDocument {
        RevisionOverviewDocument {
            current_assessment: RevisionOverviewAssessmentDocument {
                status: "resolved",
                assessment: Some(ReviewAssessment::Accepted),
            },
            attention: RevisionAttentionDocument {
                unassessed: false,
                accepted_with_follow_up: false,
                open_input_request_count: 0,
                failed_validation_count: 0,
                errored_validation_count: 0,
                stale_fact_count: 0,
            },
            counts: RevisionOverviewCounts {
                files: 1,
                rows: 1,
                observations: 0,
                input_requests: 0,
                assessments: 1,
                validation_checks: 0,
                adapter_notes: 0,
            },
            latest_activity: None,
        }
    }

    #[test]
    fn units_document_splices_target_display_additively() {
        let entries = vec![entry(
            "/Users/x/worktrees/boardwalk/plan-0006",
            "545b0eb81463aaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )];
        let overviews = BTreeMap::from([("rev:sha256:abc".to_owned(), overview())]);

        let docs = to_unit_entry_documents(entries, overviews).unwrap();
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
        assert_eq!(json["snapshotContentHash"], "sha256:artifact:abc");
        assert!(
            json.get("reviewUnitId").is_none(),
            "the redundant reviewUnitId alias is dropped"
        );
        assert_eq!(json["capturedAt"], "2026-05-13T10:00:00Z");
        assert_eq!(json["revisionId"], "rev:sha256:abc");
        assert_eq!(json["snapshotId"], "snap:sha256:abc");
        assert!(
            json.get("objectId").is_none() && json.get("objectArtifactContentHash").is_none(),
            "the inspector entry re-keys the shared fields into snapshot vocabulary"
        );
    }

    #[test]
    fn splice_target_display_adds_block_without_dropping_target_fields() {
        // Mirrors the /api/revision document shape: revision carries the verbatim target.
        let mut document = serde_json::json!({
            "revision": {
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

        assert_eq!(document["revision"]["targetDisplay"]["label"], "plan-0006");
        assert_eq!(
            document["revision"]["targetDisplay"]["head"]["commitOidShort"],
            "545b0eb"
        );
        // Additive: the raw target endpoint is untouched.
        assert_eq!(
            document["revision"]["target"]["worktreeRoot"],
            "/Users/x/worktrees/boardwalk/plan-0006"
        );
        assert_eq!(document["revision"]["target"]["kind"], "git_working_tree");
    }

    #[test]
    fn legacy_worktree_root_payload_derives_basename_without_touching_identity() {
        // A payload that only ever carried `worktreeRoot`. Deriving the display
        // must be a pure read: it must not rewrite the Revision identity and
        // must not leak the raw path into the derived block.
        let revision_id = RevisionId::new("rev:sha256:legacy");
        let payload = WorkObjectProposedPayload {
            engagement_id: EngagementId::new("engagement:sha256:legacy"),
            work_object: WorkObjectProposal::Revision {
                revision: Revision {
                    id: revision_id.clone(),
                    object_id: ObjectId::new("obj:sha256:legacy"),
                    git_provenance: Some(GitProvenance {
                        source: RevisionSource::GitWorktree {
                            mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                            include_untracked: true,
                            pathspecs: Vec::new(),
                        },
                        base: ReviewEndpoint::GitCommit {
                            commit_oid: "0123456789abcdef0123456789abcdef01234567".to_owned(),
                            tree_oid: "tree-oid".to_owned(),
                        },
                        target: ReviewEndpoint::GitWorkingTree {
                            worktree_root: "/repo/legacy-wt".to_owned(),
                        },
                    }),
                },
                object_artifact_content_hash: "sha256:artifact:legacy".to_owned(),
                supersedes: vec![],
            },
        };

        let WorkObjectProposal::Revision { revision, .. } = payload.work_object else {
            unreachable!("constructed a revision proposal");
        };
        let provenance = revision.git_provenance.as_ref().unwrap();
        let display = derive_target_display(&provenance.target, &provenance.base);
        let json = serde_json::to_string(&display).unwrap();

        assert_eq!(display.label, "legacy-wt");
        assert!(display.path_private);
        assert_eq!(display.head.as_ref().unwrap().commit_oid_short, "0123456");
        // No raw path leaks into the derived block.
        assert!(!json.contains("/repo"));
        // Derivation never rewrote identity (no event/file written).
        assert_eq!(revision.id, revision_id);
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
        (root, result.revision_id.as_str().to_owned(), branch)
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
    fn revision_json_populates_live_branch_for_anchored_commit_on_a_branch() {
        let (repo, revision_id, branch) = captured_commit_range_repo();

        let value: serde_json::Value =
            serde_json::from_str(&revision_json(repo.path(), &revision_id).unwrap()).unwrap();

        assert_eq!(
            value["revision"]["targetDisplay"]["head"]["liveBranch"],
            serde_json::json!(branch),
            "the anchored target commit is the branch tip → live on that branch"
        );
    }

    #[test]
    fn revision_json_omits_live_branch_for_floating_worktree_capture() {
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
            serde_json::from_str(&revision_json(path, capture.revision_id.as_str()).unwrap())
                .unwrap();

        assert!(
            value["revision"]["targetDisplay"]["head"]["liveBranch"].is_null(),
            "a floating worktree capture has no current commit → liveBranch omitted"
        );
    }

    #[test]
    fn revision_json_omits_live_branch_when_commit_objects_are_unavailable() {
        let (repo, revision_id, _branch) = captured_commit_range_repo();

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
            serde_json::from_str(&revision_json(elsewhere.path(), &revision_id).unwrap()).unwrap();

        assert!(
            value["revision"]["targetDisplay"]["head"]["liveBranch"].is_null(),
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
