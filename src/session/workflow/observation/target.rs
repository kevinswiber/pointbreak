use std::path::Path;

use crate::error::{Result, ShoreError};
use crate::git::git_head_ref;
use crate::model::{
    ReviewEndpoint, ReviewTargetRef, ReviewUnitId, ReviewUnitLineageId, RevisionId, SessionId,
    Side, SnapshotId,
};
use crate::session::event::{
    EventType, ReviewUnitCapturedPayload, ReviewUnitRefAssociatedPayload, ShoreEvent,
};
use crate::session::projection::commit_range::review_unit_of;
use crate::session::projection::lineage::ReviewUnitLineageProjection;
use crate::session::snapshot_artifact::read_snapshot_artifact_for_write_validation;
use crate::session::store::fingerprint::normalized_worktree_root;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedReviewUnit {
    pub session_id: SessionId,
    pub review_unit_id: ReviewUnitId,
    pub revision_id: RevisionId,
    pub snapshot_id: SnapshotId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReviewUnitSelection<'a> {
    Current,
    Exact(&'a ReviewUnitId),
    LineageHead(&'a ReviewUnitLineageId),
}

impl<'a> ReviewUnitSelection<'a> {
    pub(crate) fn from_review_unit_or_lineage(
        review_unit_id: Option<&'a ReviewUnitId>,
        lineage_id: Option<&'a ReviewUnitLineageId>,
    ) -> Result<Self> {
        match (review_unit_id, lineage_id) {
            (Some(_), Some(_)) => Err(ShoreError::WorkflowInputInvalid {
                reason: "cannot combine --review-unit and --lineage".to_owned(),
            }),
            (Some(review_unit_id), None) => Ok(Self::Exact(review_unit_id)),
            (None, Some(lineage_id)) => Ok(Self::LineageHead(lineage_id)),
            (None, None) => Ok(Self::Current),
        }
    }
}

/// The caller's current git context for scoping [`ReviewUnitSelection::Current`].
/// `worktree_root` is the canonical root from [`normalized_worktree_root`];
/// `head_ref` is the full ref of HEAD (`refs/heads/...`), `None` on detached HEAD.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CurrentReviewUnitContext {
    pub worktree_root: String,
    pub head_ref: Option<String>,
}

impl CurrentReviewUnitContext {
    /// Resolve the context from a repo path: the canonical worktree root plus
    /// HEAD's full ref. Used by the read/observation workflows before selection.
    pub(crate) fn for_repo(repo: &Path) -> Result<Self> {
        Ok(Self {
            worktree_root: normalized_worktree_root(repo)?,
            head_ref: git_head_ref(repo)?,
        })
    }
}

/// How widely `Current` searches the (shared) store. The default scopes to the
/// caller's current worktree; the widening variants are the fixed vocabulary the
/// widening read selectors construct as they are wired onto the read surfaces.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(dead_code)] // widening variants are constructed by the read selectors that surface them
pub(crate) enum ReviewUnitScope {
    /// Default: only captures belonging to the caller's current worktree context.
    #[default]
    CurrentWorktree,
    /// Scope to a named worktree root: the named root rides in the context's
    /// `worktree_root` and is matched the same way as the current worktree.
    Worktree,
    /// Widen to the whole store (resolve against every captured unit).
    All,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationTargetSelector {
    pub file_path: Option<String>,
    pub side: Side,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
}

impl ObservationTargetSelector {
    pub fn review_unit() -> Self {
        Self {
            file_path: None,
            side: Side::New,
            start_line: None,
            end_line: None,
        }
    }

    pub fn file(path: impl Into<String>) -> Self {
        Self {
            file_path: Some(path.into()),
            side: Side::New,
            start_line: None,
            end_line: None,
        }
    }

    pub fn range(
        path: impl Into<String>,
        side: Side,
        start_line: u32,
        end_line: Option<u32>,
    ) -> Self {
        Self {
            file_path: Some(path.into()),
            side,
            start_line: Some(start_line),
            end_line,
        }
    }
}

pub(crate) fn resolve_review_unit(
    events: &[ShoreEvent],
    selection: ReviewUnitSelection<'_>,
    context: &CurrentReviewUnitContext,
    scope: ReviewUnitScope,
) -> Result<ResolvedReviewUnit> {
    if let ReviewUnitSelection::LineageHead(lineage_id) = selection {
        let projection = ReviewUnitLineageProjection::from_events(events)?;
        let lineage = projection.lineage(lineage_id).ok_or_else(|| {
            ShoreError::Message(format!(
                "unknown ReviewUnit lineage: {}",
                lineage_id.as_str()
            ))
        })?;
        if !lineage.diagnostics.is_empty() {
            return Err(ShoreError::Message(format!(
                "ReviewUnit lineage {} is malformed",
                lineage_id.as_str()
            )));
        }
        let head = lineage.head_review_unit_id.as_ref().ok_or_else(|| {
            ShoreError::Message(format!(
                "ReviewUnit lineage {} has no current head",
                lineage_id.as_str()
            ))
        })?;
        return resolve_review_unit(events, ReviewUnitSelection::Exact(head), context, scope);
    }

    // `Current` under a worktree scope (current or named) only considers captures
    // belonging to that worktree context; `All` considers the whole store. `Exact`
    // (handled inside the loop) is always context-independent.
    let scoped = matches!(
        scope,
        ReviewUnitScope::CurrentWorktree | ReviewUnitScope::Worktree
    );

    let mut captured = Vec::new();
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewUnitCaptured)
    {
        let payload: ReviewUnitCapturedPayload = serde_json::from_value(event.payload.clone())?;
        let resolved = ResolvedReviewUnit {
            session_id: event.target.session_id.clone(),
            review_unit_id: payload.review_unit_id.clone(),
            revision_id: payload.revision_id.clone(),
            snapshot_id: payload.snapshot_id.clone(),
        };
        if matches!(selection, ReviewUnitSelection::Exact(requested) if requested == &resolved.review_unit_id)
        {
            return Ok(resolved);
        }
        if matches!(selection, ReviewUnitSelection::Current)
            && scoped
            && !capture_matches_current_worktree(events, event, &payload, context)?
        {
            continue;
        }
        captured.push(resolved);
    }

    if let ReviewUnitSelection::Exact(requested) = selection {
        return Err(ShoreError::Message(format!(
            "unknown review unit: {}",
            requested.as_str()
        )));
    }

    match captured.as_slice() {
        [] => Err(ShoreError::Message("no captured review unit".to_owned())),
        [resolved] => Ok(resolved.clone()),
        _ => Err(ShoreError::Message(
            "multiple captured review units; pass --review-unit".to_owned(),
        )),
    }
}

/// Whether a capture belongs to the caller's current worktree context. A capture
/// matches when **any** of: a positive worktree-identity match (its `GitWorkingTree`
/// target equals the current worktree root, or a capture-time `ReviewUnitRefAssociated`
/// names the current branch); or (fail-open) it carries no locally-meaningful
/// worktree signal.
///
/// The fail-open net keeps two kinds of capture resolvable via bare `Current`: a
/// lone commit-range capture (no worktree path and no ref association) and an
/// ingested capture (its worktree path is the origin's, meaningless locally).
/// Scoping must never silently hide a capture it cannot classify. A
/// locally-authored worktree capture for a different root is a positive non-match
/// and does not fall through to fail-open.
fn capture_matches_current_worktree(
    events: &[ShoreEvent],
    capture_event: &ShoreEvent,
    payload: &ReviewUnitCapturedPayload,
    context: &CurrentReviewUnitContext,
) -> Result<bool> {
    if capture_has_worktree_identity_match(events, payload, context)? {
        return Ok(true);
    }

    let signal_less_range = matches!(payload.target, ReviewEndpoint::GitCommit { .. })
        && !capture_has_any_ref_association(events, &payload.review_unit_id)?;
    let is_ingested = capture_event.ingest.is_some();
    Ok(signal_less_range || is_ingested)
}

/// The positive worktree-identity half of [`capture_matches_current_worktree`]:
/// a capture matches iff its `GitWorkingTree` target equals the context's worktree
/// root, or it carries a capture-time `ReviewUnitRefAssociated` for the context's
/// branch. This is the strict identity match — without the fail-open net — that an
/// explicit worktree read selector scopes by.
pub(crate) fn capture_has_worktree_identity_match(
    events: &[ShoreEvent],
    payload: &ReviewUnitCapturedPayload,
    context: &CurrentReviewUnitContext,
) -> Result<bool> {
    if let ReviewEndpoint::GitWorkingTree { worktree_root } = &payload.target
        && worktree_root == &context.worktree_root
    {
        return Ok(true);
    }

    if let Some(head_ref) = context.head_ref.as_deref()
        && capture_has_ref_association(events, &payload.review_unit_id, head_ref)?
    {
        return Ok(true);
    }

    Ok(false)
}

/// The review-unit ids whose `ReviewUnitCaptured` event has a positive
/// worktree-identity match against `context`. Shared by the explicit worktree read
/// selector on the list surfaces (the strict identity match, no fail-open).
pub(crate) fn review_unit_ids_in_worktree(
    events: &[ShoreEvent],
    context: &CurrentReviewUnitContext,
) -> Result<std::collections::BTreeSet<ReviewUnitId>> {
    let mut ids = std::collections::BTreeSet::new();
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewUnitCaptured)
    {
        let payload: ReviewUnitCapturedPayload = serde_json::from_value(event.payload.clone())?;
        if capture_has_worktree_identity_match(events, &payload, context)? {
            ids.insert(payload.review_unit_id);
        }
    }
    Ok(ids)
}

/// Whether the event set carries a `ReviewUnitRefAssociated` for `review_unit_id`
/// naming `head_ref` (the full ref, matching `git_head_ref`'s spelling).
fn capture_has_ref_association(
    events: &[ShoreEvent],
    review_unit_id: &ReviewUnitId,
    head_ref: &str,
) -> Result<bool> {
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewUnitRefAssociated)
    {
        let payload: ReviewUnitRefAssociatedPayload =
            serde_json::from_value(event.payload.clone())?;
        if review_unit_of(&payload.target).as_ref() == Some(review_unit_id)
            && payload.ref_name == head_ref
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether the event set carries any `ReviewUnitRefAssociated` for
/// `review_unit_id`, regardless of which ref it names.
fn capture_has_any_ref_association(
    events: &[ShoreEvent],
    review_unit_id: &ReviewUnitId,
) -> Result<bool> {
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::ReviewUnitRefAssociated)
    {
        let payload: ReviewUnitRefAssociatedPayload =
            serde_json::from_value(event.payload.clone())?;
        if review_unit_of(&payload.target).as_ref() == Some(review_unit_id) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn resolve_observation_target(
    repo: &Path,
    resolved: &ResolvedReviewUnit,
    selector: &ObservationTargetSelector,
) -> Result<ReviewTargetRef> {
    let Some(file_path) = selector.file_path.as_deref() else {
        if selector.start_line.is_some() || selector.end_line.is_some() {
            return Err(ShoreError::WorkflowInputInvalid {
                reason: "file is required when selecting observation lines".to_owned(),
            });
        }
        return Ok(ReviewTargetRef::ReviewUnit {
            review_unit_id: resolved.review_unit_id.clone(),
        });
    };

    let artifact = read_snapshot_artifact_for_write_validation(repo, &resolved.snapshot_id)?;
    if !artifact.snapshot.files.iter().any(|file| {
        file.new_path.as_deref() == Some(file_path) || file.old_path.as_deref() == Some(file_path)
    }) {
        return Err(ShoreError::Message(format!(
            "file target is not present in captured snapshot: {file_path}"
        )));
    }

    match selector.start_line {
        Some(start_line) => {
            if start_line == 0 {
                return Err(ShoreError::WorkflowInputInvalid {
                    reason: "start line must be greater than zero".to_owned(),
                });
            }
            let end_line = selector.end_line.unwrap_or(start_line);
            if end_line < start_line {
                return Err(ShoreError::WorkflowInputInvalid {
                    reason: "end line must be greater than or equal to start line".to_owned(),
                });
            }
            Ok(ReviewTargetRef::Range {
                review_unit_id: resolved.review_unit_id.clone(),
                file_path: file_path.to_owned(),
                side: selector.side,
                start_line,
                end_line,
            })
        }
        None => {
            if selector.end_line.is_some() {
                return Err(ShoreError::WorkflowInputInvalid {
                    reason: "start line is required when end line is supplied".to_owned(),
                });
            }
            Ok(ReviewTargetRef::File {
                review_unit_id: resolved.review_unit_id.clone(),
                file_path: file_path.to_owned(),
            })
        }
    }
}

#[cfg(test)]
mod scope_tests {
    use std::process::Command;

    use super::*;
    use crate::model::{
        CommitRangeCaptureMode, ReviewEndpoint, ReviewUnitId, ReviewUnitSource, RevisionId,
        SessionId, SnapshotId, WorktreeCaptureMode,
    };
    use crate::session::event::{
        EventTarget, IngestProvenance, IngestVia, ReviewUnitCapturedPayload, Writer,
    };

    fn capture_event(suffix: &str, source: ReviewUnitSource, target: ReviewEndpoint) -> ShoreEvent {
        let review_unit_id = ReviewUnitId::new(format!("review-unit:sha256:{suffix}"));
        let revision_id = RevisionId::new(format!("rev:{suffix}"));
        let snapshot_id = SnapshotId::new(format!("snap:{suffix}"));
        ShoreEvent::new(
            EventType::ReviewUnitCaptured,
            format!("review_unit_captured:{}", review_unit_id.as_str()),
            EventTarget::for_review_unit(
                SessionId::new("session:default"),
                review_unit_id.clone(),
                revision_id.clone(),
                snapshot_id.clone(),
            ),
            Writer::shore_local("test"),
            ReviewUnitCapturedPayload {
                review_unit_id,
                source,
                base: ReviewEndpoint::GitCommit {
                    commit_oid: "base".to_owned(),
                    tree_oid: "base-tree".to_owned(),
                },
                target,
                revision_id,
                snapshot_id,
                snapshot_artifact_content_hash: "sha256:artifact".to_owned(),
            },
            "2026-05-12T00:00:00Z",
        )
        .unwrap()
    }

    /// A worktree capture: `GitWorkingTree` target carrying the worktree root.
    fn worktree_capture(suffix: &str, worktree_root: &str) -> ShoreEvent {
        capture_event(
            suffix,
            ReviewUnitSource::GitWorktree {
                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                include_untracked: true,
            },
            ReviewEndpoint::GitWorkingTree {
                worktree_root: worktree_root.to_owned(),
            },
        )
    }

    /// A commit-range capture: `GitCommit` target, no worktree root.
    fn range_capture(suffix: &str) -> ShoreEvent {
        capture_event(
            suffix,
            ReviewUnitSource::GitCommitRange {
                mode: CommitRangeCaptureMode::BaseTreeToTargetTree,
            },
            ReviewEndpoint::GitCommit {
                commit_oid: format!("oid:{suffix}"),
                tree_oid: format!("tree:{suffix}"),
            },
        )
    }

    /// An ingested copy of a worktree capture: it keeps the origin's worktree root
    /// and carries an `ingest` provenance marker (the dest stamps this on import).
    fn ingested_worktree_capture(suffix: &str, origin_root: &str) -> ShoreEvent {
        let mut event = worktree_capture(suffix, origin_root);
        event.ingest = Some(IngestProvenance {
            via: IngestVia::IngestEvents,
            received_at: "unix-ms:1760000000000".to_owned(),
        });
        event
    }

    /// A capture-time branch ref association for a given unit + full ref.
    fn ref_association(unit_suffix: &str, ref_name: &str) -> ShoreEvent {
        crate::session::workflow::association::build_ref_association_event(
            &SessionId::new("session:default"),
            &ReviewUnitId::new(format!("review-unit:sha256:{unit_suffix}")),
            &RevisionId::new(format!("rev:{unit_suffix}")),
            &SnapshotId::new(format!("snap:{unit_suffix}")),
            ref_name,
            &"0".repeat(40),
            None,
            Writer::shore_local("test"),
            "2026-05-12T00:00:00Z",
        )
        .unwrap()
    }

    fn ctx(worktree_root: &str, head_ref: Option<&str>) -> CurrentReviewUnitContext {
        CurrentReviewUnitContext {
            worktree_root: worktree_root.to_owned(),
            head_ref: head_ref.map(str::to_owned),
        }
    }

    fn current(
        events: &[ShoreEvent],
        context: &CurrentReviewUnitContext,
        scope: ReviewUnitScope,
    ) -> Result<ResolvedReviewUnit> {
        resolve_review_unit(events, ReviewUnitSelection::Current, context, scope)
    }

    #[test]
    fn current_resolves_this_worktrees_capture_with_sibling_captures_present() {
        let events = [
            worktree_capture("a", "/wt/a"),
            worktree_capture("b", "/wt/b"),
        ];

        let resolved_a = current(
            &events,
            &ctx("/wt/a", None),
            ReviewUnitScope::CurrentWorktree,
        )
        .expect("worktree a resolves its own capture");
        assert_eq!(resolved_a.review_unit_id.as_str(), "review-unit:sha256:a");

        let resolved_b = current(
            &events,
            &ctx("/wt/b", None),
            ReviewUnitScope::CurrentWorktree,
        )
        .expect("worktree b resolves its own capture");
        assert_eq!(resolved_b.review_unit_id.as_str(), "review-unit:sha256:b");
    }

    #[test]
    fn current_two_own_worktree_captures_is_selection_error() {
        let events = [
            worktree_capture("a", "/wt/a"),
            worktree_capture("a2", "/wt/a"),
        ];

        let error = current(
            &events,
            &ctx("/wt/a", None),
            ReviewUnitScope::CurrentWorktree,
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("multiple captured review units; pass --review-unit")
        );
    }

    #[test]
    fn current_matches_capture_by_capture_time_ref_when_target_path_differs() {
        // A range capture (no worktree path) that carries a capture-time ref for
        // the current branch, with a context whose worktree root differs.
        let events = [
            range_capture("r"),
            ref_association("r", "refs/heads/feat/x"),
        ];

        let resolved = current(
            &events,
            &ctx("/some/other/root", Some("refs/heads/feat/x")),
            ReviewUnitScope::CurrentWorktree,
        )
        .expect("ref association scopes the range capture into the current branch context");
        assert_eq!(resolved.review_unit_id.as_str(), "review-unit:sha256:r");
    }

    #[test]
    fn current_no_match_in_worktree_is_no_captured_review_unit_error() {
        // Both captures carry a worktree-path signal that does not match the
        // context, so the fail-open clause does not apply to either.
        let events = [
            worktree_capture("b", "/wt/b"),
            worktree_capture("c", "/wt/c"),
        ];

        let error = current(
            &events,
            &ctx("/wt/a", None),
            ReviewUnitScope::CurrentWorktree,
        )
        .unwrap_err();

        assert!(error.to_string().contains("no captured review unit"));
    }

    #[test]
    fn exact_resolves_regardless_of_worktree_context() {
        // `Exact` widens by construction: it resolves a sibling worktree's unit
        // even from a non-matching context. (LineageHead threads the same context
        // through its recursion; the lineage suite covers that path.)
        let events = [
            worktree_capture("a", "/wt/a"),
            worktree_capture("b", "/wt/b"),
        ];
        let requested = ReviewUnitId::new("review-unit:sha256:b");

        let resolved = resolve_review_unit(
            &events,
            ReviewUnitSelection::Exact(&requested),
            &ctx("/wt/a", None),
            ReviewUnitScope::CurrentWorktree,
        )
        .expect("exact selection is context-independent");
        assert_eq!(resolved.review_unit_id.as_str(), "review-unit:sha256:b");
    }

    #[test]
    fn widen_to_all_resolves_single_cross_worktree_capture_but_still_errors_on_two() {
        let one = [worktree_capture("b", "/wt/b")];
        let resolved = current(&one, &ctx("/wt/a", None), ReviewUnitScope::All)
            .expect("widening to the whole store resolves a single capture");
        assert_eq!(resolved.review_unit_id.as_str(), "review-unit:sha256:b");

        let two = [
            worktree_capture("b", "/wt/b"),
            worktree_capture("c", "/wt/c"),
        ];
        let error = current(&two, &ctx("/wt/a", None), ReviewUnitScope::All).unwrap_err();
        assert!(error.to_string().contains("multiple captured review units"));
    }

    #[test]
    fn normalized_worktree_root_is_pub_crate() {
        // Compile-level proof that the function is reachable from a sibling module.
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            assert!(
                Command::new("git")
                    .args(args)
                    .current_dir(dir.path())
                    .status()
                    .unwrap()
                    .success()
            );
        };
        run(&["init"]);

        let root = crate::session::store::fingerprint::normalized_worktree_root(dir.path())
            .expect("normalized worktree root resolves for an initialized repo");
        assert!(!root.is_empty());
    }

    #[test]
    fn lone_signal_less_range_capture_resolves_via_fail_open() {
        // A range capture with neither a worktree path nor any ref association.
        // It must keep resolving via bare `Current` (the no-regression guarantee).
        let events = [range_capture("r")];

        let resolved = current(
            &events,
            &ctx("/wt/a", Some("refs/heads/main")),
            ReviewUnitScope::CurrentWorktree,
        )
        .expect("a signal-less range capture fails open to the current worktree");
        assert_eq!(resolved.review_unit_id.as_str(), "review-unit:sha256:r");
    }

    #[test]
    fn signal_less_range_capture_plus_worktree_capture_is_ambiguous() {
        // A matching worktree capture and a signal-less range capture both land in
        // scope (worktree-path match + fail-open), so bare `Current` stays
        // ambiguous rather than silently picking one.
        let events = [worktree_capture("w", "/wt/a"), range_capture("r")];

        let error = current(
            &events,
            &ctx("/wt/a", None),
            ReviewUnitScope::CurrentWorktree,
        )
        .unwrap_err();
        assert!(error.to_string().contains("multiple captured review units"));
    }

    #[test]
    fn ingested_capture_resolves_via_fail_open_in_dest_worktree() {
        // The capture's only worktree path is the origin's; in the destination
        // worktree it resolves via the ingest fail-open clause, not hidden.
        let events = [ingested_worktree_capture("i", "/origin/wt")];

        let resolved = current(
            &events,
            &ctx("/dest/wt", None),
            ReviewUnitScope::CurrentWorktree,
        )
        .expect("an ingested capture fails open into the destination worktree");
        assert_eq!(resolved.review_unit_id.as_str(), "review-unit:sha256:i");
    }

    #[test]
    fn ingested_capture_plus_own_worktree_capture_is_ambiguous() {
        // A local capture matches by worktree path; an ingested foreign capture
        // fails open. Both in scope → the explicit-selection error, not a silent
        // pick.
        let events = [
            worktree_capture("local", "/dest/wt"),
            ingested_worktree_capture("foreign", "/origin/wt"),
        ];

        let error = current(
            &events,
            &ctx("/dest/wt", None),
            ReviewUnitScope::CurrentWorktree,
        )
        .unwrap_err();
        assert!(error.to_string().contains("multiple captured review units"));
    }

    #[test]
    fn worktree_ids_select_identity_matches_without_fail_open() {
        // A matching worktree capture, a sibling-root worktree capture, and a
        // signal-less range capture. The explicit worktree selector is the strict
        // identity match — only the matching worktree path is selected; the range
        // capture's fail-open does not widen it.
        let events = [
            worktree_capture("here", "/wt/a"),
            worktree_capture("there", "/wt/b"),
            range_capture("floaty"),
        ];

        let ids = review_unit_ids_in_worktree(&events, &ctx("/wt/a", None)).unwrap();

        assert!(ids.contains(&ReviewUnitId::new("review-unit:sha256:here")));
        assert!(!ids.contains(&ReviewUnitId::new("review-unit:sha256:there")));
        assert!(!ids.contains(&ReviewUnitId::new("review-unit:sha256:floaty")));
    }
}
