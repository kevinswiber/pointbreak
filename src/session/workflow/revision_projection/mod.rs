use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::error::Result;
use crate::model::{DiffSnapshot, EventId, ReviewId, RevisionId};
use crate::session::assessment::{
    AssessmentProjectionOptions, AssessmentView, CurrentAssessmentView, project_assessments,
};
use crate::session::event::{EventType, ShoreEvent};
use crate::session::input_request::{
    InputRequestProjectionOptions, InputRequestStatusFilter, InputRequestView,
    project_input_requests,
};
use crate::session::observation::{
    CurrentRevisionContext, ObservationProjectionOptions, ObservationView, ResolvedRevision,
    RevisionScope, RevisionSelection, project_observations, resolve_revision, validated_track_id,
};
use crate::session::projection::cosignature::{
    CosignatureIndex, endorsement_readbacks, enrich_endorser_attributes,
};
use crate::session::projection::{
    ArtifactRemovalProjection, RemovalOperativeStatus, skipped_to_diagnostics,
};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store::backend::StoreBackend;
use crate::session::store::resolution::{ReadStore, resolve_read_store};
use crate::session::workflow::{
    ValidationCheckProjectionOptions, ValidationCheckView, annotate_validation_supersession,
    project_validation_checks,
};
use crate::session::{
    EventStore, RemovalPolicy, RevisionCommitRangeProjection, RevisionCommitRangeView,
    SupersessionView, TrustSet, verify_event_signature,
};

mod identity;
mod resolving;
mod rows;
mod search;
mod snapshot;
mod summary_cache;
mod validation_continuity;

use self::identity::principal_diagnostics;
pub use self::identity::{
    MemberReadback, RevisionProjectionIdentity, RevisionProjectionSummary, RevisionShowFilters,
    RevisionShowOptions, RevisionShowResult, SnapshotContentState,
};
use self::resolving::{enumerate_revision_identities, selected_revision_capture};
pub use self::rows::{RevisionProjectionRow, SnapshotOrder};
use self::rows::{
    build_assessment_rows, build_input_request_rows, build_observation_rows, build_snapshot_rows,
    build_validation_rows, renumber_projection_rows,
};
pub use self::search::{
    RevisionRecordInputs, RevisionSearchRecord, build_revision_search_record,
    current_assessment_includes_follow_up, stale_review_fact_count,
};
use self::snapshot::{
    SnapshotContent, load_bound_object_artifact_from_backend, resolve_snapshot_content,
};
pub use self::summary_cache::{SnapshotSummaryCache, SnapshotSummaryCounts};
pub use self::validation_continuity::{
    ValidationCheckDisposition, ValidationContinuitySummary, ValidationContinuityView,
    classify_validation_continuity,
};
use crate::session::projection::body_content::{BodyRemovalLens, body_content_diagnostics};

/// A removal is recorded for the bound snapshot content, but its bytes are still
/// stored: the suppression is reversible and a compact would reclaim them.
const SNAPSHOT_CONTENT_SUPPRESSED_PRESENT: &str = "snapshot_content_suppressed_present";
/// A removal is recorded for the bound snapshot content and its bytes have been
/// swept from the store.
const SNAPSHOT_CONTENT_PHYSICALLY_REMOVED: &str = "snapshot_content_physically_removed";
/// The bound snapshot artifact cannot be read. The capture remains a durable
/// revision record, but snapshot-derived rows and counts are unavailable.
const SNAPSHOT_CONTENT_UNAVAILABLE: &str = "snapshot_content_unavailable";
/// The bound content carries an unsigned removal claim that is not operative.
const REMOVAL_CLAIM_UNSIGNED: &str = "removal_claim_unsigned";
/// The bound content carries a removal signed by an untrusted key, not operative.
const REMOVAL_CLAIM_UNTRUSTED: &str = "removal_claim_untrusted";
/// The bound content carries a removal whose signature verifies invalid (the
/// integrity floor); not operative under any policy.
const REMOVAL_CLAIM_INVALID: &str = "removal_claim_invalid";
/// A removal targets a content hash that no event in this store references.
const SNAPSHOT_CONTENT_REMOVED_TARGET_MISSING: &str = "snapshot_content_removed_target_missing";
/// A capture re-binds a content hash that carries an operative removal.
const IDENTITY_REUSED_AFTER_REMOVAL: &str = "identity_reused_after_removal";

/// The store-wide read prefix shared by `show_revision` and the overview batch:
/// the single event-log read plus the projections that own their data (not the
/// borrowing `CosignatureIndex`). Each caller builds its own `cosig_index` from
/// `events` locally, so the borrow lives on the caller's stack rather than inside
/// this owned struct (which would be self-referential and would not compile).
struct StoreWideRead {
    events: Vec<ShoreEvent>,
    removal: ArtifactRemovalProjection,
    skip_diagnostics: Vec<ProjectionDiagnostic>,
}

/// Resolve and fold the event log once: the lenient/strict branch matching
/// `show_revision`, the skip diagnostics for the display path, and the
/// `ArtifactRemovalProjection`. The `CosignatureIndex` is deliberately *not*
/// built here — it borrows `events`, so each caller builds it locally.
fn load_store_wide_read(read_store: &ReadStore, read_for_display: bool) -> Result<StoreWideRead> {
    let span = tracing::debug_span!("shore.revisions.overviews.load_store_wide_read");
    let _guard = span.enter();

    let store = EventStore::from_backend(read_store.backend());
    let (events, skip_diagnostics) = {
        let span = tracing::debug_span!("shore.revisions.overviews.read_events");
        let _guard = span.enter();
        if read_for_display {
            let (events, skipped) = store.list_events_lenient()?;
            (events, skipped_to_diagnostics(skipped))
        } else {
            (store.list_events()?, Vec::new())
        }
    };
    let removal = {
        let span = tracing::debug_span!("shore.revisions.overviews.artifact_removal_projection");
        let _guard = span.enter();
        ArtifactRemovalProjection::from_events(&events)?
    };
    Ok(StoreWideRead {
        events,
        removal,
        skip_diagnostics,
    })
}

pub fn show_revision(options: RevisionShowOptions) -> Result<RevisionShowResult> {
    let read_store = resolve_read_store(&options.repo)?;
    let track_id = options
        .track
        .as_deref()
        .map(validated_track_id)
        .transpose()?;
    let StoreWideRead {
        events,
        removal,
        skip_diagnostics,
    } = load_store_wide_read(&read_store, options.read_for_display)?;
    // The `--revision` seed is a head seed (forward-resolving); an exact request
    // (the inspector addressing a specific revision by id, e.g. a superseded DAG
    // node) resolves the id directly instead.
    let selection = match (options.exact, options.revision_id.as_ref()) {
        (true, Some(id)) => RevisionSelection::Exact(id),
        _ => RevisionSelection::from_revision_seed(options.revision_id.as_ref()),
    };
    let resolved = resolve_revision(
        &events,
        selection,
        &CurrentRevisionContext::for_repo(&options.repo)?,
        RevisionScope::default(),
    )?;
    let revision = selected_revision_capture(&events, &resolved)?;
    // Built once from the shared read and shared downstream: the suppression
    // decision, the claim diagnostics, and the endorsement readback block all read
    // this one index (do not build a second). It borrows `events`, so it lives on
    // this stack frame, not inside `StoreWideRead`.
    let cosig_index = CosignatureIndex::build(&events)?;
    // The bound hash's operative status is decided once here so the same decision
    // drives both suppression and the claim diagnostics.
    let bound_status = removal.operative_status(
        &revision.object_artifact_content_hash,
        &options.trust_set,
        options.removal_policy,
        &cosig_index,
    )?;
    // The body twin of the bound-snapshot decision: one lens per read, shared by
    // every body-bearing projection below.
    let body_removal_lens = BodyRemovalLens::new(
        &removal,
        &options.trust_set,
        options.removal_policy,
        &cosig_index,
    );
    let snapshot_content = match resolve_snapshot_content(&options.repo, &revision, bound_status) {
        Ok(content) => content,
        Err(error) if options.read_for_display => SnapshotContent::Unavailable {
            content_hash: revision.object_artifact_content_hash.clone(),
            error: error.to_string(),
        },
        Err(error) => return Err(error),
    };
    let snapshot_content_state = SnapshotContentState::from(&snapshot_content);
    let (snapshot, removed_snapshot_content_hash, unavailable_snapshot) = match snapshot_content {
        SnapshotContent::Present(snapshot) => (snapshot, None, None),
        SnapshotContent::SuppressedPresent { content_hash }
        | SnapshotContent::PhysicallyRemoved { content_hash } => (
            DiffSnapshot::new(
                ReviewId::new(revision.journal_id.as_str()),
                revision.object_id.clone(),
                Vec::new(),
            ),
            Some(content_hash),
            None,
        ),
        SnapshotContent::Unavailable {
            content_hash,
            error,
        } => (
            DiffSnapshot::new(
                ReviewId::new(revision.journal_id.as_str()),
                revision.object_id.clone(),
                Vec::new(),
            ),
            None,
            Some((content_hash, error)),
        ),
    };
    let observations = project_observations(ObservationProjectionOptions {
        backend: read_store.backend(),
        events: &events,
        resolved: &resolved,
        track_filter: track_id.clone(),
        file_filter: None,
        tag_filters: &[],
        include_body: options.include_body,
        removal_lens: &body_removal_lens,
    })?;
    let input_requests = project_input_requests(InputRequestProjectionOptions {
        backend: read_store.backend(),
        events: &events,
        resolved: &resolved,
        track_filter: track_id.clone(),
        mode_filter: None,
        file_filter: None,
        status_filter: InputRequestStatusFilter::All,
        include_body: options.include_body,
        removal_lens: &body_removal_lens,
    })?;
    let (current_assessment, assessments) = project_assessments(AssessmentProjectionOptions {
        backend: Some(read_store.backend()),
        events: &events,
        resolved: &resolved,
        track_filter: track_id.clone(),
        include_summary: options.include_body,
        include_all: true,
        removal_lens: Some(&body_removal_lens),
    })?;
    let mut validation_checks = project_validation_checks(ValidationCheckProjectionOptions {
        backend: read_store.backend(),
        events: &events,
        revision_id: &resolved.revision_id,
        track_filter: track_id.clone(),
        status_filter: None,
        include_body: options.include_body,
        removal_lens: &body_removal_lens,
    })?;
    // Annotate each check with the successors of the revision it targets. An exact address can
    // resolve a superseded revision, so its checks carry the advisory staleness set; a head resolves
    // to itself and stays empty. Built from the events already read above — no second store read.
    let supersession = SupersessionView::from_events(&events)?;
    annotate_validation_supersession(&mut validation_checks, &supersession);
    let (snapshot_rows, mut summary) = if snapshot_content_state != SnapshotContentState::Present {
        // Removed or unavailable content has no snapshot rows; the explained
        // absence is carried by state and diagnostics below, not a misleading
        // empty-state row.
        (Vec::new(), RevisionProjectionSummary::default())
    } else {
        build_snapshot_rows(&snapshot, &revision.id)
    };
    let mut narrative_rows = Vec::new();
    let observation_rows = build_observation_rows(&observations);
    summary.observation_count = observations.len();
    narrative_rows.extend(observation_rows);
    let input_request_rows = build_input_request_rows(&input_requests);
    summary.input_request_count = input_requests.len();
    narrative_rows.extend(input_request_rows);
    let assessment_rows = build_assessment_rows(&assessments);
    summary.assessment_count = assessments.len();
    narrative_rows.extend(assessment_rows);
    let validation_rows = build_validation_rows(&validation_checks);
    summary.validation_check_count = validation_checks.len();
    narrative_rows.extend(validation_rows);
    summary.narrative_row_count = narrative_rows.len();
    summary.row_count = summary.narrative_row_count + summary.snapshot_remainder_row_count;
    let mut rows = narrative_rows;
    rows.extend(snapshot_rows);
    renumber_projection_rows(&mut rows);
    let state = SessionState::from_events(&events)?;
    let event_set_hash = state
        .event_set_hash
        .clone()
        .expect("SessionState::from_events sets event_set_hash");
    let mut diagnostics = state.diagnostics;
    diagnostics.extend(skip_diagnostics);
    if let Some(content_hash) = &removed_snapshot_content_hash {
        match snapshot_content_state {
            SnapshotContentState::SuppressedPresent => diagnostics.push(ProjectionDiagnostic {
                code: SNAPSHOT_CONTENT_SUPPRESSED_PRESENT.to_owned(),
                message: format!(
                    "snapshot content {content_hash} is suppressed by a recorded removal; the bytes \
                     are still stored and a compact would reclaim them"
                ),
            }),
            SnapshotContentState::PhysicallyRemoved => diagnostics.push(ProjectionDiagnostic {
                code: SNAPSHOT_CONTENT_PHYSICALLY_REMOVED.to_owned(),
                message: format!(
                    "snapshot content {content_hash} was removed and its bytes have been swept from \
                     the store"
                ),
            }),
            SnapshotContentState::Present | SnapshotContentState::Unavailable => {}
        }
    }
    if let Some((content_hash, error)) = &unavailable_snapshot {
        diagnostics.push(ProjectionDiagnostic {
            code: SNAPSHOT_CONTENT_UNAVAILABLE.to_owned(),
            message: format!("snapshot content {content_hash} is unavailable: {error}"),
        });
    }
    // The body twin of the snapshot block above: every body-bearing view's
    // (state, hash) pairs fold through the shared mapper. Later projections
    // chain their pairs here as they gain states.
    let body_states = observations
        .iter()
        .map(|o| (o.body_content_state, o.body_content_hash.as_deref()))
        .chain(
            assessments
                .iter()
                .map(|a| (a.summary_content_state, a.summary_content_hash.as_deref())),
        )
        .chain(
            input_requests
                .iter()
                .map(|r| (r.body_content_state, r.body_content_hash.as_deref())),
        )
        .chain(input_requests.iter().flat_map(|r| {
            r.responses.iter().map(|resp| {
                (
                    resp.reason_content_state,
                    resp.reason_content_hash.as_deref(),
                )
            })
        }))
        .chain(
            validation_checks
                .iter()
                .map(|v| (v.summary_content_state, v.summary_content_hash.as_deref())),
        );
    diagnostics.extend(body_content_diagnostics(body_states));

    // The bound hash's claim diagnostic, mapped from the operative status decided
    // once above (reused, not recomputed). A non-operative claim renders the bytes
    // and surfaces here instead of silently suppressing.
    let bound_hash = &revision.object_artifact_content_hash;
    match bound_status {
        RemovalOperativeStatus::ClaimUnsigned => diagnostics.push(ProjectionDiagnostic {
            code: REMOVAL_CLAIM_UNSIGNED.to_owned(),
            message: format!(
                "removal of snapshot content {bound_hash} is unsigned and not operative; ratify it \
                 with `pointbreak endorse` or extend trust"
            ),
        }),
        RemovalOperativeStatus::ClaimUntrusted => diagnostics.push(ProjectionDiagnostic {
            code: REMOVAL_CLAIM_UNTRUSTED.to_owned(),
            message: format!(
                "removal of snapshot content {bound_hash} is signed by an untrusted key and not \
                 operative; ratify it or extend trust"
            ),
        }),
        RemovalOperativeStatus::ClaimInvalid => diagnostics.push(ProjectionDiagnostic {
            code: REMOVAL_CLAIM_INVALID.to_owned(),
            message: format!(
                "removal of snapshot content {bound_hash} has an invalid signature (integrity \
                 floor) and is not operative; re-issue it cleanly"
            ),
        }),
        RemovalOperativeStatus::NoClaim
        | RemovalOperativeStatus::OperativePossession
        | RemovalOperativeStatus::OperativeTrusted => {}
    }

    // Projection-scope removal diagnostics, sharing the hoisted cosig index: a
    // removal of a never-referenced hash, and a capture re-binding an operatively
    // removed hash.
    for content_hash in removal.target_missing_diagnostics(&events)? {
        diagnostics.push(ProjectionDiagnostic {
            code: SNAPSHOT_CONTENT_REMOVED_TARGET_MISSING.to_owned(),
            message: format!(
                "removal targets content {content_hash}, which no event in this store references"
            ),
        });
    }
    for reuse in removal.identity_reuse_diagnostics(
        &events,
        &options.trust_set,
        options.removal_policy,
        &cosig_index,
    )? {
        diagnostics.push(ProjectionDiagnostic {
            code: IDENTITY_REUSED_AFTER_REMOVAL.to_owned(),
            message: format!(
                "revision {} re-binds removed snapshot content {} ({})",
                reuse.revision_id.as_str(),
                reuse.content_hash,
                reuse.kind.as_str()
            ),
        });
    }

    // Git-free commit-range lifecycle: fold the association events into the resolved
    // unit's view and surface its diagnostics. Liveness is layered by repo-holding
    // callers, never here.
    let commit_range = RevisionCommitRangeProjection::from_events(&events)?
        .unit(&resolved.revision_id)
        .cloned()
        .unwrap_or_else(|| RevisionCommitRangeView {
            revision_id: resolved.revision_id.clone(),
            anchored: false,
            current_commits: Vec::new(),
            current_refs: Vec::new(),
            withdrawn_commits: Vec::new(),
            withdrawn_refs: Vec::new(),
            diagnostics: Vec::new(),
        });
    diagnostics.extend(commit_range.diagnostics.clone());

    if let Some(map) = options.delegation_map.as_ref() {
        let members = observations
            .iter()
            .map(|view| (&view.writer.actor_id, view.created_at.as_str()))
            .chain(input_requests.iter().flat_map(|request| {
                std::iter::once((&request.writer.actor_id, request.created_at.as_str())).chain(
                    request
                        .responses
                        .iter()
                        .map(|response| (&response.writer.actor_id, response.created_at.as_str())),
                )
            }))
            .chain(
                assessments
                    .iter()
                    .map(|view| (&view.writer.actor_id, view.created_at.as_str())),
            )
            .chain(
                validation_checks
                    .iter()
                    .map(|view| (&view.writer.actor_id, view.created_at.as_str())),
            );
        diagnostics.extend(principal_diagnostics(members, map));
    }

    // Reader-relative readback, keyed by event id and computed once over the events
    // already in scope. Presence of a verification policy enables it; advisory render
    // only, never a gate. The document layer attaches it by event id.
    let mut member_readbacks: BTreeMap<EventId, MemberReadback> = BTreeMap::new();
    if options.verification_policy.is_some() {
        let by_id: HashMap<&str, &ShoreEvent> =
            events.iter().map(|e| (e.event_id.as_str(), e)).collect();
        // Reuses the `cosig_index` hoisted near the removal projection above.
        let mut record = |event_id: &EventId| -> Result<()> {
            if let Some(event) = by_id.get(event_id.as_str()) {
                let entry = member_readbacks.entry(event_id.clone()).or_default();
                entry.verification_status =
                    Some(verify_event_signature(event, &options.trust_set)?);
                // Trust-only classification, then sibling enrichment.
                let mut readbacks = endorsement_readbacks(
                    &cosig_index.cosignatures_for_target(event, &options.trust_set)?,
                );
                enrich_endorser_attributes(&mut readbacks, options.actor_attributes.as_ref());
                entry.endorsements = readbacks;
            }
            Ok(())
        };
        record(&revision.capture_event_id)?;
        for view in &observations {
            record(&view.event_id)?;
        }
        for request in &input_requests {
            record(&request.event_id)?;
            for response in &request.responses {
                record(&response.event_id)?;
            }
        }
        for view in &assessments {
            record(&view.event_id)?;
        }
        for view in &validation_checks {
            record(&view.event_id)?;
        }
    }

    Ok(RevisionShowResult {
        event_set_hash,
        event_count: events.len(),
        revision,
        snapshot,
        removed_snapshot_content_hash,
        snapshot_content_state,
        filters: RevisionShowFilters {
            revision_id: resolved.revision_id,
            track_id,
            include_body: options.include_body,
        },
        summary,
        current_assessment,
        observations,
        input_requests,
        assessments,
        validation_checks,
        rows,
        commit_range,
        member_readbacks,
        diagnostics,
    })
}

/// Inputs for [`show_revision_overviews`], the single-pass batch behind the
/// inspector's `/api/revisions` overview cards. It mirrors the slice of
/// [`RevisionShowOptions`] the overview path actually reads: `trust_set` and
/// `removal_policy` drive `operative_status` (the snapshot suppression that sets
/// `file_count`/`row_count`), and `read_for_display` selects the lenient vs
/// strict read. The verification-policy / actor-attributes / delegation-map
/// inputs are deliberately absent — they feed only the per-event readback and the
/// principal diagnostics, neither of which a [`RevisionOverview`] carries.
#[derive(Clone, Debug)]
pub struct RevisionOverviewsOptions {
    pub(super) repo: PathBuf,
    pub(super) revisions: Vec<RevisionId>,
    pub(super) trust_set: TrustSet,
    pub(super) removal_policy: RemovalPolicy,
    pub(super) read_for_display: bool,
    pub(super) snapshot_summaries: Option<Arc<SnapshotSummaryCache>>,
}

impl RevisionOverviewsOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            revisions: Vec::new(),
            trust_set: TrustSet::default(),
            removal_policy: RemovalPolicy::default(),
            read_for_display: false,
            snapshot_summaries: None,
        }
    }

    /// The revisions to build overviews for. The caller supplies exactly the set
    /// it will surface — the inspector passes the `list_revisions` entry ids, which
    /// are a subset of every `WorkObjectProposed` revision after explicit filters
    /// and commit-OID grouping. Building only this set keeps the batch faithful to
    /// the per-revision path it replaces, and never resolves a
    /// snapshot for a revision that never reaches the wire.
    pub fn with_revisions(mut self, revisions: impl IntoIterator<Item = RevisionId>) -> Self {
        self.revisions = revisions.into_iter().collect();
        self
    }

    /// Supply the reader's trust set; it drives the operative-removal decision
    /// behind a revision's `file_count`/`row_count`.
    pub fn with_trust_set(mut self, trust_set: TrustSet) -> Self {
        self.trust_set = trust_set;
        self
    }

    /// Supply the render-time removal policy (default `PossessionOrTrusted`).
    pub fn with_removal_policy(mut self, removal_policy: RemovalPolicy) -> Self {
        self.removal_policy = removal_policy;
        self
    }

    /// Read for a human-facing surface: skip a retired/unsupported event and
    /// surface it as a diagnostic instead of hard-failing the read. The inspector
    /// overview path opts in, matching the singular `show_revision` it replaces.
    pub fn with_read_for_display(mut self, value: bool) -> Self {
        self.read_for_display = value;
        self
    }

    /// Share a snapshot summary cache across batch builds. Counts are keyed by
    /// the artifact content hash (immutable content), so a long-lived caller —
    /// the inspect server — decodes each snapshot artifact once per process,
    /// not once per rebuild (#426).
    pub fn with_snapshot_summary_cache(mut self, cache: Arc<SnapshotSummaryCache>) -> Self {
        self.snapshot_summaries = Some(cache);
        self
    }
}

/// The lean per-revision overview the inspector's `/api/revisions` cards read:
/// exactly the projection slice `revision_overview_document` consumes (the
/// summary counts, the current assessment, and the observation / input-request /
/// assessment / validation views that drive the attention counts
/// and latest-activity). It carries none of `show_revision`'s member readbacks,
/// rows or commit range. Diagnostics are scoped to failures that the batch can
/// isolate to this revision, most importantly an unavailable snapshot artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionOverview {
    pub summary: RevisionProjectionSummary,
    pub current_assessment: CurrentAssessmentView,
    pub observations: Vec<ObservationView>,
    pub input_requests: Vec<InputRequestView>,
    pub assessments: Vec<AssessmentView>,
    pub validation_checks: Vec<ValidationCheckView>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
    /// Advisory: the revisions that directly supersede *this* revision. Empty ⇒ head. Not serialized
    /// directly; the inspector derives its stale-fact count from it.
    pub superseded_by: BTreeSet<RevisionId>,
}

/// Batch the per-revision overview for every captured revision in one store-wide
/// pass. This replaces the inspector's N+1 — `show_revision` once per revision,
/// each re-reading and re-folding the whole event log — with a single
/// `list_events_lenient` plus one `ArtifactRemovalProjection`/`CosignatureIndex`,
/// then the same per-revision projections `show_revision` runs, minus the
/// member-readback verification loop and the `SessionState`/commit-range/
/// diagnostics assembly the overview never reads.
pub fn show_revision_overviews(
    options: RevisionOverviewsOptions,
) -> Result<BTreeMap<RevisionId, RevisionOverview>> {
    let span = tracing::debug_span!("shore.revisions.overviews");
    let _guard = span.enter();

    let read_store = {
        let span = tracing::debug_span!("shore.revisions.overviews.resolve_read_store");
        let _guard = span.enter();
        resolve_read_store(&options.repo)?
    };
    revision_overviews_from_store(
        &read_store,
        &options.revisions,
        &options.trust_set,
        options.removal_policy,
        options.read_for_display,
        options.snapshot_summaries.as_deref(),
    )
}

/// The store-injected core of [`show_revision_overviews`]: the single event-log
/// read happens here, so a test can drive it over an in-memory backend and prove
/// the log is read once for the whole batch (not `revision_count + 1` times). The
/// repo-path entry resolves the local backend and delegates here. Builds an
/// overview for each requested revision found in the log; a requested id with no
/// capture is skipped (the caller's lookup surfaces a genuine miss).
fn revision_overviews_from_store(
    read_store: &ReadStore,
    revisions: &[RevisionId],
    trust_set: &TrustSet,
    removal_policy: RemovalPolicy,
    read_for_display: bool,
    summary_cache: Option<&SnapshotSummaryCache>,
) -> Result<BTreeMap<RevisionId, RevisionOverview>> {
    let span = tracing::debug_span!(
        "shore.revisions.overviews.from_store",
        requested_revision_count = revisions.len()
    );
    let _guard = span.enter();

    // The single read for the whole batch, via the same prefix `show_revision`
    // uses. Store-wide skip diagnostics stay on the list projection rather than
    // being duplicated onto every overview, so they are discarded here;
    // revision-local snapshot failures are attached below. `cosig_index` borrows
    // `events`, so it is built locally here, on this stack frame.
    let StoreWideRead {
        events,
        removal,
        skip_diagnostics: _,
    } = load_store_wide_read(read_store, read_for_display)?;
    let cosig_index = {
        let span = tracing::debug_span!("shore.revisions.overviews.cosignature_index");
        let _guard = span.enter();
        CosignatureIndex::build(&events)?
    };
    // One supersession view for the whole batch, from the already-read events (single-read
    // preserved). Each overview reads its own direct superseders from it.
    let supersession = {
        let span = tracing::debug_span!("shore.revisions.overviews.supersession_view");
        let _guard = span.enter();
        SupersessionView::from_events(&events)?
    };
    // Index every captured identity by id once, then build only the requested
    // revisions. The requested set is the caller's listed entries, a subset of all
    // captures after explicit filters and grouping — building only it never resolves
    // a snapshot for a grouped-away revision the caller will not surface.
    let identities: BTreeMap<RevisionId, RevisionProjectionIdentity> = {
        let span = tracing::debug_span!("shore.revisions.overviews.enumerate_identities");
        let _guard = span.enter();
        enumerate_revision_identities(&events)?
            .into_iter()
            .map(|identity| (identity.id.clone(), identity))
            .collect()
    };

    let requested: Vec<&RevisionProjectionIdentity> = revisions
        .iter()
        .filter_map(|revision_id| identities.get(revision_id))
        .collect();

    // Snapshot seeds: decide the operative-removal outcome per revision, then
    // satisfy every present snapshot's counts from the shared cache or a
    // one-time (parallel) decode. This is the batch's dominant cost, so it runs
    // once per distinct content hash instead of once per revision per rebuild.
    let seeds = {
        let span = tracing::debug_span!("shore.revisions.overviews.snapshot_seeds");
        let _guard = span.enter();
        snapshot_summary_seeds(
            read_store.backend(),
            &requested,
            trust_set,
            removal_policy,
            &removal,
            &cosig_index,
            summary_cache,
        )?
    };

    // Bucket the review-fact events by their subject revision once, so each
    // per-revision projection folds its own events instead of re-scanning the
    // whole log per revision (the O(revisions × events) term).
    let fact_events = {
        let span = tracing::debug_span!("shore.revisions.overviews.bucket_fact_events");
        let _guard = span.enter();
        bucket_fact_events(&events, &requested)?
    };

    let mut overviews = BTreeMap::new();
    {
        let span = tracing::debug_span!("shore.revisions.overviews.build_requested");
        let _guard = span.enter();
        for identity in requested {
            let seed = seeds
                .get(&identity.id)
                .expect("every requested identity has a snapshot seed");
            let revision_facts = fact_events
                .get(&identity.id)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let overview = build_revision_overview(
                read_store.backend(),
                revision_facts,
                identity,
                seed,
                trust_set,
                removal_policy,
                &removal,
                &cosig_index,
                &supersession,
            )?;
            overviews.insert(identity.id.clone(), overview);
        }
    }
    Ok(overviews)
}

/// The snapshot-derived half of one revision's overview, decided before the
/// per-revision assembly: the present content's counts (from the shared cache or
/// a one-time decode), the operative-removal outcome, or an isolated read
/// failure. Removed and unavailable snapshots both seed empty snapshot counts;
/// only the latter carries a diagnostic.
#[derive(Clone)]
enum SnapshotSeed {
    Present(SnapshotSummaryCounts),
    Removed,
    Unavailable(String),
}

/// Resolve every requested revision's snapshot counts: operative removals seed
/// `Removed`; present content resolves from the shared cache when warm, and the
/// remaining misses decode once per distinct content hash (in parallel) and
/// backfill the cache.
fn snapshot_summary_seeds(
    backend: &StoreBackend,
    requested: &[&RevisionProjectionIdentity],
    trust_set: &TrustSet,
    removal_policy: RemovalPolicy,
    removal: &ArtifactRemovalProjection,
    cosig_index: &CosignatureIndex<'_>,
    summary_cache: Option<&SnapshotSummaryCache>,
) -> Result<BTreeMap<RevisionId, SnapshotSeed>> {
    let mut seeds = BTreeMap::new();
    let mut misses: Vec<&RevisionProjectionIdentity> = Vec::new();
    let mut miss_hashes: BTreeSet<&str> = BTreeSet::new();
    for identity in requested {
        let status = {
            let span = tracing::debug_span!("shore.revisions.overviews.operative_status");
            let _guard = span.enter();
            removal.operative_status(
                &identity.object_artifact_content_hash,
                trust_set,
                removal_policy,
                cosig_index,
            )?
        };
        if matches!(
            status,
            RemovalOperativeStatus::OperativePossession | RemovalOperativeStatus::OperativeTrusted
        ) {
            seeds.insert(identity.id.clone(), SnapshotSeed::Removed);
            continue;
        }
        if let Some(counts) =
            summary_cache.and_then(|cache| cache.get(&identity.object_artifact_content_hash))
        {
            seeds.insert(identity.id.clone(), SnapshotSeed::Present(counts));
            continue;
        }
        // Decode work is deduped by content hash: two revisions binding the
        // same content share one decode.
        if miss_hashes.insert(identity.object_artifact_content_hash.as_str()) {
            misses.push(identity);
        }
    }

    let decoded = decode_snapshot_counts(backend, &misses);
    let decoded_by_hash: BTreeMap<&str, SnapshotSeed> = misses
        .iter()
        .zip(decoded)
        .map(|(identity, result)| {
            let seed = match result {
                Ok(counts) => SnapshotSeed::Present(counts),
                Err(error) => SnapshotSeed::Unavailable(error.to_string()),
            };
            (identity.object_artifact_content_hash.as_str(), seed)
        })
        .collect();
    if let Some(cache) = summary_cache {
        for (content_hash, seed) in &decoded_by_hash {
            if let SnapshotSeed::Present(counts) = seed {
                cache.insert((*content_hash).to_owned(), *counts);
            }
        }
    }
    for identity in requested {
        if seeds.contains_key(&identity.id) {
            continue;
        }
        let seed = decoded_by_hash
            .get(identity.object_artifact_content_hash.as_str())
            .cloned()
            .expect("every non-removed requested revision was decoded or cache-seeded");
        seeds.insert(identity.id.clone(), seed);
    }
    Ok(seeds)
}

/// Decode each missed artifact once and count its snapshot rows. The decode —
/// a full JSON parse plus a canonical-hash revalidation per artifact — is the
/// batch's dominant cost and each miss is independent, so misses run on scoped
/// threads. Results keep input order, including failures, so one unreadable
/// artifact can be attached to its revision without failing the whole batch.
fn decode_snapshot_counts(
    backend: &StoreBackend,
    misses: &[&RevisionProjectionIdentity],
) -> Vec<Result<SnapshotSummaryCounts>> {
    let worker_count = std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1)
        .min(misses.len());
    if worker_count <= 1 {
        return misses
            .iter()
            .map(|identity| decode_one_snapshot_counts(backend, identity))
            .collect();
    }

    let next = AtomicUsize::new(0);
    let mut indexed: Vec<(usize, Result<SnapshotSummaryCounts>)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..worker_count)
            .map(|_| {
                scope.spawn(|| {
                    let mut local = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(identity) = misses.get(index) else {
                            break;
                        };
                        local.push((index, decode_one_snapshot_counts(backend, identity)));
                    }
                    local
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| {
                worker
                    .join()
                    .expect("snapshot decode worker does not panic")
            })
            .collect()
    });
    indexed.sort_unstable_by_key(|(index, _)| *index);
    indexed.into_iter().map(|(_, counts)| counts).collect()
}

fn decode_one_snapshot_counts(
    backend: &StoreBackend,
    identity: &RevisionProjectionIdentity,
) -> Result<SnapshotSummaryCounts> {
    let span = tracing::debug_span!("shore.revisions.overviews.resolve_snapshot_content");
    let _guard = span.enter();
    let snapshot = load_bound_object_artifact_from_backend(backend, identity)?;
    // The rows are built only to count them, exactly as the summary recompute
    // in `show_revision` does, so the counts cannot drift from the row builder.
    let (_rows, summary) = build_snapshot_rows(&snapshot, &identity.id);
    Ok(SnapshotSummaryCounts {
        file_count: summary.file_count,
        snapshot_row_count: summary.snapshot_row_count,
    })
}

/// The event types the four overview fact projections read. Bucketing must
/// cover exactly this set: a type outside it never reaches an overview
/// projection, and every one of these reconstructs a review-revision subject
/// (an input-request response carries its request's revision in the payload).
const OVERVIEW_FACT_EVENT_TYPES: [EventType; 5] = [
    EventType::ReviewObservationRecorded,
    EventType::InputRequestOpened,
    EventType::InputRequestResponded,
    EventType::ReviewAssessmentRecorded,
    EventType::ValidationCheckRecorded,
];

/// One pass over the log: group the requested revisions' review-fact events by
/// their subject revision, so each per-revision projection receives only its
/// own events. Subject reconstruction errors propagate exactly as they would
/// from the projections themselves (same types, same probe).
fn bucket_fact_events(
    events: &[ShoreEvent],
    requested: &[&RevisionProjectionIdentity],
) -> Result<BTreeMap<RevisionId, Vec<ShoreEvent>>> {
    let requested_ids: BTreeSet<&RevisionId> =
        requested.iter().map(|identity| &identity.id).collect();
    let mut buckets: BTreeMap<RevisionId, Vec<ShoreEvent>> = BTreeMap::new();
    for event in events {
        if !OVERVIEW_FACT_EVENT_TYPES.contains(&event.event_type) {
            continue;
        }
        let Some(revision_id) = event.subject_revision_id()? else {
            continue;
        };
        if !requested_ids.contains(&revision_id) {
            continue;
        }
        buckets.entry(revision_id).or_default().push(event.clone());
    }
    Ok(buckets)
}

/// The per-revision tail, the same one `show_revision` runs for the single
/// revision it resolves: run the same five projections over the revision's own
/// bucketed events and recompute the summary the same way, seeding the
/// snapshot-derived counts from the precomputed [`SnapshotSeed`]. It omits the
/// member-readback loop and the `SessionState`/commit-range/diagnostics
/// assembly — none of which a [`RevisionOverview`] carries.
#[allow(clippy::too_many_arguments)]
fn build_revision_overview(
    backend: &StoreBackend,
    fact_events: &[ShoreEvent],
    revision: &RevisionProjectionIdentity,
    seed: &SnapshotSeed,
    trust_set: &TrustSet,
    removal_policy: RemovalPolicy,
    removal: &ArtifactRemovalProjection,
    cosig_index: &CosignatureIndex<'_>,
    supersession: &SupersessionView,
) -> Result<RevisionOverview> {
    let span = tracing::debug_span!("shore.revisions.overviews.build_one");
    let _guard = span.enter();

    let resolved = ResolvedRevision {
        journal_id: revision.journal_id.clone(),
        revision_id: revision.revision_id.clone(),
        object_id: revision.object_id.clone(),
        object_artifact_content_hash: revision.object_artifact_content_hash.clone(),
    };
    let body_removal_lens = BodyRemovalLens::new(removal, trust_set, removal_policy, cosig_index);
    // The overview reads only counts, titles, statuses, and timestamps — never a
    // hydrated body — so `include_body` is false on every projection, matching the
    // current overview path (which never sets `with_include_body`).
    let observations = {
        let span = tracing::debug_span!("shore.revisions.overviews.project_observations");
        let _guard = span.enter();
        project_observations(ObservationProjectionOptions {
            backend,
            events: fact_events,
            resolved: &resolved,
            track_filter: None,
            file_filter: None,
            tag_filters: &[],
            include_body: false,
            removal_lens: &body_removal_lens,
        })?
    };
    let input_requests = {
        let span = tracing::debug_span!("shore.revisions.overviews.project_input_requests");
        let _guard = span.enter();
        project_input_requests(InputRequestProjectionOptions {
            backend,
            events: fact_events,
            resolved: &resolved,
            track_filter: None,
            mode_filter: None,
            file_filter: None,
            status_filter: InputRequestStatusFilter::All,
            include_body: false,
            removal_lens: &body_removal_lens,
        })?
    };
    let (current_assessment, assessments) = {
        let span = tracing::debug_span!("shore.revisions.overviews.project_assessments");
        let _guard = span.enter();
        project_assessments(AssessmentProjectionOptions {
            backend: Some(backend),
            events: fact_events,
            resolved: &resolved,
            track_filter: None,
            include_summary: false,
            include_all: true,
            removal_lens: Some(&body_removal_lens),
        })?
    };
    let validation_checks = {
        let span = tracing::debug_span!("shore.revisions.overviews.project_validation_checks");
        let _guard = span.enter();
        project_validation_checks(ValidationCheckProjectionOptions {
            backend,
            events: fact_events,
            revision_id: &resolved.revision_id,
            track_filter: None,
            status_filter: None,
            include_body: false,
            removal_lens: &body_removal_lens,
        })?
    };

    // Recompute the summary exactly as `show_revision` does: the snapshot counts
    // seed `file_count` + `snapshot_remainder_row_count`, the narrative builders
    // seed `narrative_row_count`, and `row_count` is their sum. An operative
    // removal seeds the same default (empty) summary the resolve path produced.
    let mut summary = match seed {
        SnapshotSeed::Present(counts) => RevisionProjectionSummary {
            file_count: counts.file_count,
            row_count: counts.snapshot_row_count,
            snapshot_row_count: counts.snapshot_row_count,
            snapshot_remainder_row_count: counts.snapshot_row_count,
            ..RevisionProjectionSummary::default()
        },
        SnapshotSeed::Removed | SnapshotSeed::Unavailable(_) => {
            RevisionProjectionSummary::default()
        }
    };
    {
        let span = tracing::debug_span!("shore.revisions.overviews.build_narrative_counts");
        let _guard = span.enter();
        let mut narrative_rows = Vec::new();
        narrative_rows.extend(build_observation_rows(&observations));
        summary.observation_count = observations.len();
        narrative_rows.extend(build_input_request_rows(&input_requests));
        summary.input_request_count = input_requests.len();
        narrative_rows.extend(build_assessment_rows(&assessments));
        summary.assessment_count = assessments.len();
        narrative_rows.extend(build_validation_rows(&validation_checks));
        summary.validation_check_count = validation_checks.len();
        summary.narrative_row_count = narrative_rows.len();
        summary.row_count = summary.narrative_row_count + summary.snapshot_remainder_row_count;
    }

    let diagnostics = match seed {
        SnapshotSeed::Unavailable(error) => vec![ProjectionDiagnostic {
            code: SNAPSHOT_CONTENT_UNAVAILABLE.to_owned(),
            message: format!(
                "snapshot content {} for revision {} is unavailable: {error}",
                revision.object_artifact_content_hash,
                revision.id.as_str()
            ),
        }],
        SnapshotSeed::Present(_) | SnapshotSeed::Removed => Vec::new(),
    };

    Ok(RevisionOverview {
        summary,
        current_assessment,
        observations,
        input_requests,
        assessments,
        validation_checks,
        diagnostics,
        superseded_by: supersession.stale_by_superseding_revision(&resolved.revision_id),
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;

    use super::rows::RevisionProjectionRowKind;
    use super::*;
    use crate::canonical_hash::sha256_json_prefixed;
    use crate::crypto::{EventSignatureBytes, EventSigner};
    use crate::model::{
        DiffSnapshot, EngagementId, JournalId, ObjectId, ReviewEndpoint, ReviewId, RevisionId,
        RevisionSource, ValidationCheckId, ValidationStatus, ValidationTarget, ValidationTrigger,
        WorktreeCaptureMode,
    };
    use crate::session::event::{
        ArtifactRemovedPayload, EventSignature, EventTarget, EventToBeSigned, EventType,
        GitProvenance, IngestProvenance, IngestVia, InputRequestReasonCode,
        InputRequestResponseOutcome, ReviewAssessment, Revision, ShoreEvent,
        ValidationCheckRecordedPayload, WorkObjectProposal, WorkObjectProposedPayload, Writer,
        event_signature_pre_authentication_encoding,
    };
    use crate::session::signing::test_support::DeterministicSigner;
    use crate::session::store::backend::{InMemoryStore, StoreBackend};
    use crate::session::{
        AssessmentAddOptions, AssessmentShowOptions, BodyContentState, CaptureOptions,
        CaptureResult, CurrentAssessmentStatus, EventStore, InputRequestFetchOptions,
        InputRequestListOptions, InputRequestOpenOptions, InputRequestRespondOptions,
        InputRequestStatus, InputRequestStatusFilter, ObservationAddOptions,
        ObservationListOptions, ObservationTargetSelector, RemovalPolicy, RevisionListOptions,
        RootCommitSpec, capture_review, capture_worktree_review, fetch_input_request,
        list_input_requests, list_observations, list_revisions, open_input_request,
        record_assessment, record_observation, respond_input_request, show_assessments,
    };

    // ---- Overview batch: golden-equality + single-read invariants ----

    /// The batch overview for every captured revision must equal the overview
    /// derived from today's per-revision `show_revision` path — the byte-exactness
    /// oracle that lets the N+1 fix refactor safely (complements the wire-parity
    /// gate). The fixture spans several revisions with observations, an input
    /// request + response, an assessment, passed + failed validation checks, and
    /// one operatively-removed snapshot (so the `operative_status` → empty-rows
    /// path is exercised).
    #[test]
    fn show_revision_overviews_matches_per_revision_show_revision() {
        let repo = build_multi_revision_fixture();
        let trust = TrustSet::default();
        let policy = RemovalPolicy::default();

        let ids = list_revision_ids(repo.path());
        let expected: BTreeMap<RevisionId, RevisionOverview> = ids
            .iter()
            .map(|id| {
                let overview = overview_from_show_revision(repo.path(), id, &trust, policy);
                (id.clone(), overview)
            })
            .collect();

        let actual = show_revision_overviews(
            RevisionOverviewsOptions::new(repo.path())
                .with_revisions(ids)
                .with_trust_set(trust.clone())
                .with_removal_policy(policy)
                .with_read_for_display(true),
        )
        .unwrap();

        assert_eq!(actual, expected);
        // The fixture genuinely spanned multiple revisions and the removed-snapshot
        // path, so the equality above is not vacuous.
        assert_eq!(expected.len(), 3);
        assert!(
            expected
                .values()
                .any(|overview| overview.summary.file_count == 0),
            "the operatively-removed revision contributes an empty snapshot"
        );
    }

    /// The N+1 is gone: the batch reads the event log exactly once for the whole
    /// batch, not `revision_count + 1` times (the old per-revision loop). Driven
    /// over an injected in-memory backend whose journal counts its listings; the
    /// revisions carry operative removals so the snapshot path short-circuits to a
    /// stat and never reads an event byte the bare store lacks.
    #[test]
    fn revision_overviews_from_store_reads_the_log_once() {
        // A real git repo backs `resolve_read_store(repo)` in the snapshot path; the
        // events themselves come from the injected in-memory store.
        let repo = modified_repo();
        let (read_store, store, revision_ids) =
            in_memory_read_store_with_removed_revisions(repo.path(), 5);

        let overviews = revision_overviews_from_store(
            &read_store,
            &revision_ids,
            &TrustSet::default(),
            RemovalPolicy::default(),
            true,
            None,
        )
        .unwrap();

        assert_eq!(overviews.len(), 5);
        assert_eq!(
            store.list_event_entries_call_count(),
            1,
            "the batch reads the log once, not revision_count + 1"
        );
    }

    /// A shared summary cache makes rebuilds skip the snapshot artifact decode
    /// entirely: after a first build fills it, the bound blob can vanish from
    /// disk and a cache-bearing rebuild still serves the same counts. A cold
    /// build isolates the missing artifact to the affected overview — proving
    /// the hit really skipped the read rather than re-deriving the counts.
    #[test]
    fn shared_summary_cache_serves_snapshot_counts_without_rereading_artifacts() {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        let captured = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let cache = Arc::new(SnapshotSummaryCache::new());
        let cached_options = || {
            RevisionOverviewsOptions::new(repo.path())
                .with_revisions(vec![captured.revision_id.clone()])
                .with_read_for_display(true)
                .with_snapshot_summary_cache(Arc::clone(&cache))
        };

        let first = show_revision_overviews(cached_options()).unwrap();
        let summary = first[&captured.revision_id].summary.clone();
        assert!(
            summary.file_count > 0,
            "the capture produced a non-empty snapshot"
        );

        let blob = crate::session::object_artifact::object_artifact_path_for_hash(
            &resolved_store_dir(repo.path()),
            &captured.object_artifact_content_hash,
        );
        fs::remove_file(blob).expect("delete bound snapshot blob");

        let rebuilt = show_revision_overviews(cached_options()).unwrap();
        assert_eq!(rebuilt[&captured.revision_id].summary, summary);

        let cold = show_revision_overviews(
            RevisionOverviewsOptions::new(repo.path())
                .with_revisions(vec![captured.revision_id.clone()])
                .with_read_for_display(true),
        )
        .expect("one unavailable snapshot does not fail the overview batch");
        let unavailable = &cold[&captured.revision_id];
        assert_eq!(unavailable.summary.file_count, 0);
        assert_eq!(unavailable.summary.snapshot_row_count, 0);
        assert_eq!(unavailable.diagnostics.len(), 1);
        assert_eq!(
            unavailable.diagnostics[0].code,
            SNAPSHOT_CONTENT_UNAVAILABLE
        );
        assert!(
            unavailable.diagnostics[0]
                .message
                .contains(&captured.object_artifact_content_hash)
        );
    }

    #[test]
    fn show_revision_exact_annotates_a_superseded_revisions_checks() {
        use crate::session::{ValidationAddOptions, record_validation_check};

        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");

        // Root revision + a validation check recorded against it.
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        let root = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_validation_check(
            ValidationAddOptions::new(repo.path())
                .with_revision_id(root.revision_id.clone())
                .with_track("agent:codex")
                .with_check_name("cargo test")
                .with_status(ValidationStatus::Passed),
        )
        .unwrap();

        // A successor supersedes the root.
        repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
        let successor = capture_worktree_review(
            CaptureOptions::new(repo.path()).with_supersedes(vec![root.revision_id.clone()]),
        )
        .unwrap();

        // Exact selection projects the SUPERSEDED root's own checks (a head seed would resolve
        // away to the successor).
        let result = show_revision(
            RevisionShowOptions::new(repo.path())
                .with_revision_id(root.revision_id.clone())
                .with_exact(true)
                .with_read_for_display(true),
        )
        .unwrap();

        assert_eq!(result.validation_checks.len(), 1);
        assert_eq!(
            result.validation_checks[0].superseded_by_revisions,
            [successor.revision_id.clone()].into_iter().collect(),
        );

        // The head resolves to itself and its checks stay current (empty annotation).
        let head = show_revision(
            RevisionShowOptions::new(repo.path())
                .with_revision_id(successor.revision_id.clone())
                .with_exact(true)
                .with_read_for_display(true),
        )
        .unwrap();
        assert!(
            head.validation_checks
                .iter()
                .all(|check| check.superseded_by_revisions.is_empty())
        );
    }

    #[test]
    fn capture_review_root_revision_show_preserves_source_and_tree_base() {
        let repo = TestRepo::new();
        repo.write("README.md", "hello\n");
        repo.commit_all("initial");
        let capture = capture_review(
            CaptureOptions::new(repo.path()).with_root_commit(RootCommitSpec::new()),
        )
        .unwrap();

        let shown = show_revision(
            RevisionShowOptions::new(repo.path()).with_revision_id(capture.revision_id),
        )
        .unwrap();

        assert!(matches!(
            shown.revision.source,
            RevisionSource::GitRootCommit { .. }
        ));
        assert!(matches!(
            shown.revision.base,
            ReviewEndpoint::GitTree { .. }
        ));
        assert!(matches!(
            shown.revision.target,
            ReviewEndpoint::GitCommit { .. }
        ));
    }

    #[test]
    fn superseded_revision_overview_names_its_superseders() {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");

        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        let root = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
        let successor = capture_worktree_review(
            CaptureOptions::new(repo.path()).with_supersedes(vec![root.revision_id.clone()]),
        )
        .unwrap();

        let overviews = show_revision_overviews(
            RevisionOverviewsOptions::new(repo.path())
                .with_revisions(vec![
                    root.revision_id.clone(),
                    successor.revision_id.clone(),
                ])
                .with_read_for_display(true),
        )
        .unwrap();

        assert_eq!(
            overviews[&root.revision_id].superseded_by,
            [successor.revision_id.clone()].into_iter().collect(),
        );
        assert!(overviews[&successor.revision_id].superseded_by.is_empty());
    }

    fn build_multi_revision_fixture() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");

        // Revision A: an observation, an open input request, an accepted assessment.
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        let a = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_revision_id(a.revision_id.clone())
                .with_track("agent:codex")
                .with_title("A observation"),
        )
        .unwrap();
        open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_revision_id(a.revision_id.clone())
                .with_track("agent:codex")
                .with_title("A decision")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired),
        )
        .unwrap();
        record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_revision_id(a.revision_id.clone())
                .with_track("human:kevin")
                .with_assessment(ReviewAssessment::Accepted)
                .with_summary("ship A"),
        )
        .unwrap();

        // Revision B: an input request + response, a passed and a failed check.
        repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
        let b = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let request = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_revision_id(b.revision_id.clone())
                .with_track("agent:codex")
                .with_title("B decision")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired),
        )
        .unwrap();
        respond_input_request(
            InputRequestRespondOptions::new(repo.path(), request.input_request_id.clone())
                .with_outcome(InputRequestResponseOutcome::Approved)
                .with_reason("ok"),
        )
        .unwrap();
        record_validation_with_status(
            repo.path(),
            &b,
            "validation:sha256:b-pass",
            ValidationStatus::Passed,
        );
        record_validation_with_status(
            repo.path(),
            &b,
            "validation:sha256:b-fail",
            ValidationStatus::Failed,
        );

        // Revision C: an operatively-removed snapshot (operative_status → empty rows),
        // plus a removed-and-swept externalized observation body (the body twin).
        repo.write("src/lib.rs", "pub fn value() -> u32 { 4 }\n");
        let c = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let removed_body = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_revision_id(c.revision_id.clone())
                .with_track("agent:codex")
                .with_title("C removed body")
                .with_body("x".repeat(5000)),
        )
        .unwrap();
        let removed_body_hash = removed_body
            .body_content_hash
            .expect("a >4096-byte body is stored as a note artifact");
        record_artifact_removed(repo.path(), &c.object_artifact_content_hash);
        record_artifact_removed(repo.path(), &removed_body_hash);
        delete_note_body_blob(repo.path(), &removed_body_hash);

        repo
    }

    /// Derive an overview from today's singular `show_revision` path, the same way
    /// the inspector's overview path does, so the batch can be checked against it.
    fn overview_from_show_revision(
        repo: &Path,
        id: &RevisionId,
        trust: &TrustSet,
        policy: RemovalPolicy,
    ) -> RevisionOverview {
        let result = show_revision(
            RevisionShowOptions::new(repo)
                .with_revision_id(id.clone())
                .with_exact(true)
                .with_read_for_display(true)
                .with_trust_set(trust.clone())
                .with_removal_policy(policy),
        )
        .unwrap();
        RevisionOverview {
            summary: result.summary,
            current_assessment: result.current_assessment,
            observations: result.observations,
            input_requests: result.input_requests,
            assessments: result.assessments,
            validation_checks: result.validation_checks,
            diagnostics: Vec::new(),
            superseded_by: BTreeSet::new(),
        }
    }

    fn list_revision_ids(repo: &Path) -> Vec<RevisionId> {
        list_revisions(RevisionListOptions::new(repo).with_read_for_display(true))
            .unwrap()
            .entries
            .into_iter()
            .map(|entry| entry.revision_id)
            .collect()
    }

    fn record_validation_with_status(
        repo: &Path,
        capture: &CaptureResult,
        validation_check_id: &str,
        status: ValidationStatus,
    ) {
        let exit_code = Some(if matches!(status, ValidationStatus::Passed) {
            0
        } else {
            1
        });
        let target = EventTarget::for_revision(
            JournalId::new("journal:default"),
            capture.revision_id.clone(),
            Some(crate::model::TrackId::new("agent:codex")),
        )
        .unwrap();
        let event = ShoreEvent::new(
            EventType::ValidationCheckRecorded,
            format!("validation_check_recorded:{validation_check_id}"),
            target,
            Writer::shore_local("0.1.0"),
            ValidationCheckRecordedPayload {
                validation_check_id: ValidationCheckId::new(validation_check_id),
                target: ValidationTarget::Revision {
                    revision_id: capture.revision_id.clone(),
                },
                check_name: "cargo test".to_owned(),
                command: None,
                status,
                exit_code,
                trigger: ValidationTrigger::Manual,
                source_fingerprint: None,
                summary: Some("ran".to_owned()),
                summary_content_type: Default::default(),
                summary_artifact_path: None,
                summary_byte_size: Some(3),
                summary_content_hash: Some("sha256:summary".to_owned()),
                started_at: None,
                completed_at: Some("2026-05-10T00:00:00Z".to_owned()),
                log_artifact_content_hashes: Vec::new(),
            },
            "2026-05-10T00:00:00Z",
        )
        .unwrap();
        EventStore::open(resolved_store_dir(repo))
            .record_event_once(&event)
            .unwrap();
    }

    /// Seed an injection-only in-memory store with `n` captured revisions, each
    /// carrying an operative (locally-authored, unsigned) `ArtifactRemoved` so the
    /// overview snapshot path short-circuits to a stat — no artifact bytes are
    /// read from a store that does not hold them. Returns the read store, the
    /// shared `InMemoryStore` whose journal counts its listings, and the seeded
    /// revision ids (the set to request overviews for).
    fn in_memory_read_store_with_removed_revisions(
        repo: &Path,
        n: usize,
    ) -> (ReadStore, Arc<InMemoryStore>, Vec<RevisionId>) {
        let store = InMemoryStore::new();
        let backend = StoreBackend::Memory(Arc::clone(&store));
        let event_store = EventStore::from_backend(&backend);
        let journal_id = JournalId::new("journal:synthetic");
        let mut revision_ids = Vec::new();
        for index in 0..n {
            let revision_id = RevisionId::new(format!("review-unit:sha256:r{index:02}"));
            let object_id = ObjectId::new(format!("obj:sha256:o{index:02}"));
            let content_hash = format!("sha256:{:064x}", index + 1);
            event_store
                .record_event_once(&synthetic_revision_proposed_event(
                    &journal_id,
                    &revision_id,
                    &object_id,
                    &content_hash,
                    &format!("2026-05-10T00:00:{index:02}Z"),
                ))
                .unwrap();
            event_store
                .record_event_once(&synthetic_removal_event(
                    &content_hash,
                    &format!("2026-05-10T01:00:{index:02}Z"),
                ))
                .unwrap();
            revision_ids.push(revision_id);
        }
        let read_store = ReadStore::for_test(resolved_store_dir(repo), backend);
        (read_store, store, revision_ids)
    }

    fn synthetic_revision_proposed_event(
        journal_id: &JournalId,
        revision_id: &RevisionId,
        object_id: &ObjectId,
        content_hash: &str,
        occurred_at: &str,
    ) -> ShoreEvent {
        ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("work_object_proposed:{}", revision_id.as_str()),
            EventTarget::for_revision(journal_id.clone(), revision_id.clone(), None).unwrap(),
            Writer::shore_local("0.1.0"),
            WorkObjectProposedPayload {
                engagement_id: EngagementId::new(format!("engagement:{}", revision_id.as_str())),
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
                                commit_oid: "0".repeat(40),
                                tree_oid: "tree".to_owned(),
                            },
                            target: ReviewEndpoint::GitWorkingTree {
                                worktree_root: "/synthetic/worktree".to_owned(),
                            },
                        }),
                    },
                    object_artifact_content_hash: content_hash.to_owned(),
                    supersedes: vec![],
                },
            },
            occurred_at,
        )
        .unwrap()
    }

    fn synthetic_removal_event(content_hash: &str, occurred_at: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ArtifactRemoved,
            ArtifactRemovedPayload::idempotency_key(content_hash),
            EventTarget::for_journal(JournalId::new("journal:synthetic")),
            Writer::shore_local("0.1.0"),
            ArtifactRemovedPayload {
                content_hash: content_hash.to_owned(),
            },
            occurred_at,
        )
        .unwrap()
    }

    #[test]
    fn show_revision_errors_when_no_revision_is_captured() {
        let repo = modified_repo();

        let error = show_revision(RevisionShowOptions::new(repo.path()))
            .expect_err("no captured Revision should fail");

        assert!(error.to_string().contains("no captured revision"));
    }

    #[test]
    fn show_revision_resolves_single_current_revision_and_freshness() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.revision.id, capture.revision_id);
        assert_eq!(result.revision.revision_id, capture.revision_id);
        assert_eq!(result.revision.object_id, capture.object_id);
        assert_eq!(result.filters.revision_id, capture.revision_id);
        // Capture event plus the auto-recorded capture-time ref association.
        assert_eq!(result.event_count, 2);
        assert!(result.event_set_hash.starts_with("sha256:"));
    }

    #[test]
    fn show_revision_strict_by_default_lenient_when_opted_in() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        // A retired-type event in the store: the probe rejects it before decode.
        let events_dir = resolve_read_store(repo.path())
            .unwrap()
            .store_dir()
            .join("events");
        fs::write(
            events_dir.join(format!("{}.json", "a".repeat(64))),
            br#"{"eventType":"review_disposition_recorded"}"#,
        )
        .unwrap();

        // Default (the relay/strict case): a retired event hard-fails the read.
        assert!(show_revision(RevisionShowOptions::new(repo.path())).is_err());

        // Opted in (CLI/inspector): the retired event is skipped and surfaced.
        let result =
            show_revision(RevisionShowOptions::new(repo.path()).with_read_for_display(true))
                .unwrap();
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "unsupported_event_type")
        );
    }

    #[test]
    fn show_revision_includes_validation_checks() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_validation_event(repo.path(), &capture, "validation:sha256:one");

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.validation_checks.len(), 1);
        assert_eq!(result.summary.validation_check_count, 1);
        let row = result
            .rows
            .iter()
            .find(|row| row.kind == RevisionProjectionRowKind::ValidationEvidence)
            .expect("validation evidence row");
        assert_eq!(
            row.related_validation_check_ids,
            vec![result.validation_checks[0].id.clone()]
        );
        let first_snapshot_remainder = result
            .rows
            .iter()
            .position(|row| row.projection_phase.as_str() == "snapshot_remainder")
            .expect("snapshot remainder starts");
        let validation_row = result
            .rows
            .iter()
            .position(|row| row.kind == RevisionProjectionRowKind::ValidationEvidence)
            .unwrap();
        assert!(validation_row < first_snapshot_remainder);
    }

    #[test]
    fn non_validation_rows_have_empty_related_validation_check_ids() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_revision_id(capture.revision_id)
                .with_track("agent:codex")
                .with_title("Observation"),
        )
        .unwrap();

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert!(result.rows.iter().all(|row| {
            row.kind == RevisionProjectionRowKind::ValidationEvidence
                || row.related_validation_check_ids.is_empty()
        }));
    }

    fn capture_with_agent_observation() -> (TestRepo, RevisionId) {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_revision_id(capture.revision_id.clone())
                .with_track("agent:claude-code")
                .with_actor_id(crate::model::ActorId::new("actor:agent:claude-code"))
                .with_title("Agent observation"),
        )
        .unwrap();
        (repo, capture.revision_id)
    }

    #[test]
    fn unit_show_emits_diagnostic_for_unresolvable_agent_principal() {
        let (repo, revision_id) = capture_with_agent_observation();
        // A map that does not know this agent → no_delegation_record.
        let map = crate::session::delegation_map_from_value(serde_json::json!({
            "delegates": {}
        }))
        .unwrap();

        let result = show_revision(
            RevisionShowOptions::new(repo.path())
                .with_revision_id(revision_id)
                .with_delegation_map(map),
        )
        .unwrap();

        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "principal_unresolvable")
            .expect("an unresolvable agent principal emits a diagnostic");
        assert!(diagnostic.message.contains("actor:agent:claude-code"));
        assert!(diagnostic.message.contains("no_delegation_record"));
    }

    #[test]
    fn unit_show_emits_diagnostic_for_ambiguous_principal() {
        let (repo, revision_id) = capture_with_agent_observation();
        // Two overlapping open windows with distinct principals → ambiguous.
        let map = crate::session::delegation_map_from_value(serde_json::json!({
            "delegates": {
                "actor:agent:claude-code": [
                    { "principal": "actor:git-email:kevin@swiber.dev",
                      "validFrom": "2020-01-01T00:00:00Z", "validUntil": null },
                    { "principal": "actor:git-email:alice@example.com",
                      "validFrom": "2020-01-01T00:00:00Z", "validUntil": null }
                ]
            }
        }))
        .unwrap();

        let result = show_revision(
            RevisionShowOptions::new(repo.path())
                .with_revision_id(revision_id)
                .with_delegation_map(map),
        )
        .unwrap();

        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "principal_ambiguous")
            .expect("an ambiguous agent principal emits a diagnostic");
        assert!(
            diagnostic
                .message
                .contains("actor:git-email:kevin@swiber.dev")
        );
        assert!(
            diagnostic
                .message
                .contains("actor:git-email:alice@example.com")
        );
    }

    #[test]
    fn unit_show_without_map_emits_no_principal_diagnostics() {
        let (repo, revision_id) = capture_with_agent_observation();
        let result =
            show_revision(RevisionShowOptions::new(repo.path()).with_revision_id(revision_id))
                .unwrap();
        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.code.starts_with("principal_")),
            "no map supplied → no principal diagnostics"
        );
    }

    #[test]
    fn show_revision_requires_explicit_id_when_current_is_ambiguous() {
        let repo = modified_repo();
        let first = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
        let second = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let error = show_revision(RevisionShowOptions::new(repo.path()))
            .expect_err("multiple captures should be ambiguous");
        assert!(error.to_string().contains("multiple captured revisions"));

        let explicit = show_revision(
            RevisionShowOptions::new(repo.path()).with_revision_id(first.revision_id.clone()),
        )
        .unwrap();

        assert_ne!(first.revision_id, second.revision_id);
        assert_eq!(explicit.revision.id, first.revision_id);
        // Two worktree captures, each with its auto-recorded ref association.
        assert_eq!(explicit.event_count, 4);
    }

    #[test]
    fn show_revision_uses_captured_snapshot_after_worktree_drift() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 99 }\n");

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.revision.id, capture.revision_id);
        assert_eq!(
            result.snapshot.files[0].new_path.as_deref(),
            Some("src/lib.rs")
        );
        assert!(format!("{:?}", result.snapshot).contains("2"));
        assert!(!format!("{:?}", result.snapshot).contains("99"));
    }

    #[test]
    fn show_revision_rejects_object_artifact_hash_mismatch() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        tamper_object_artifact_snapshot_field(repo.path(), &capture.object_id);

        let error = show_revision(RevisionShowOptions::new(repo.path()))
            .expect_err("tampered artifact should fail");

        assert!(error.to_string().contains("content hash"));
    }

    #[test]
    fn show_revision_rejects_event_artifact_binding_mismatch() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let bad_hash = format!("sha256:{}", "0".repeat(64));
        rewrite_capture_event_object_artifact_hash(repo.path(), &capture.revision_id, &bad_hash);
        let original_path = object_artifact_path(repo.path(), &capture.object_id);
        let bad_path = crate::session::object_artifact::object_artifact_path_for_hash(
            &resolved_store_dir(repo.path()),
            &bad_hash,
        );
        fs::copy(original_path, bad_path).expect("stage mismatched object artifact");

        let error = show_revision(RevisionShowOptions::new(repo.path()))
            .expect_err("event/artifact mismatch should fail");

        assert!(error.to_string().contains("object artifact content hash"));
    }

    #[test]
    fn show_revision_emits_snapshot_rows_in_captured_order() {
        let repo = multi_file_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.rows[0].kind.as_str(), "file_header");
        assert_eq!(
            result.rows[0].projection_phase.as_str(),
            "snapshot_remainder"
        );
        assert_eq!(result.rows[0].coverage.as_str(), "unreviewed");
        assert_eq!(result.rows[0].projection_order, 0);
        assert_eq!(
            result.rows[0].snapshot_order.as_ref().unwrap().file_index,
            0
        );
        assert!(result.rows.iter().any(|row| row.kind.as_str() == "diff"));
    }

    #[test]
    fn show_revision_emits_empty_state_row_for_empty_snapshot() {
        let (rows, summary) = build_snapshot_rows(
            &DiffSnapshot::new(
                ReviewId::new("review:empty"),
                ObjectId::new("snap:empty"),
                Vec::new(),
            ),
            &RevisionId::new("review-unit:empty"),
        );

        assert_eq!(summary.file_count, 0);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind.as_str(), "empty_state");
    }

    #[test]
    fn show_revision_rows_do_not_expose_storage_paths() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();
        let debug = format!("{result:?}");

        assert!(!debug.contains("artifacts/objects"));
        assert!(!debug.contains(".pointbreak/data/events"));
    }

    #[test]
    fn show_revision_includes_active_observations() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Check this")
                .with_body("Observation body"),
        )
        .unwrap();

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert_eq!(result.observations.len(), 1);
        assert_eq!(result.observations[0].title, "Check this");
        assert_eq!(result.observations[0].body, None);
        assert_eq!(result.summary.observation_count, 1);
        assert!(
            result
                .rows
                .iter()
                .any(|row| row.kind.as_str() == "observation")
        );
    }

    #[test]
    fn show_revision_hydrates_observation_bodies_when_requested() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Body")
                .with_body("Observation body"),
        )
        .unwrap();

        let result =
            show_revision(RevisionShowOptions::new(repo.path()).with_include_body(true)).unwrap();

        assert_eq!(
            result.observations[0].body.as_deref(),
            Some("Observation body")
        );
        assert!(!format!("{result:?}").contains("artifacts/notes/"));
    }

    #[test]
    fn show_revision_observations_match_list_semantics_for_duplicates_and_supersession() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        add_duplicate_observations_with_distinct_idempotency_keys(&repo);
        add_superseding_observation(&repo);

        let unit = show_revision(RevisionShowOptions::new(repo.path())).unwrap();
        let list = list_observations(ObservationListOptions::new(repo.path())).unwrap();

        assert_eq!(unit.observations, list.observations);
        assert_eq!(unit.diagnostics, list.diagnostics);
    }

    #[test]
    fn show_revision_includes_open_and_responded_input_requests() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let request = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Need decision")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired),
        )
        .unwrap();
        respond_input_request(
            InputRequestRespondOptions::new(repo.path(), request.input_request_id.clone())
                .with_outcome(InputRequestResponseOutcome::Approved)
                .with_reason("ok"),
        )
        .unwrap();

        let unit = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert_eq!(unit.input_requests.len(), 1);
        assert_eq!(unit.input_requests[0].id, request.input_request_id);
        assert_eq!(unit.input_requests[0].status, InputRequestStatus::Responded);
        assert_eq!(unit.summary.input_request_count, 1);
        assert!(
            unit.rows
                .iter()
                .any(|row| row.kind.as_str() == "input_request")
        );
    }

    #[test]
    fn show_revision_input_requests_match_list_semantics() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        add_duplicate_input_requests(&repo);
        add_ambiguous_input_request_responses(&repo);

        let unit = show_revision(RevisionShowOptions::new(repo.path())).unwrap();
        let list = list_input_requests(
            InputRequestListOptions::new(repo.path()).with_status(InputRequestStatusFilter::All),
        )
        .unwrap();

        assert_eq!(unit.input_requests, list.input_requests);
        assert_eq!(unit.diagnostics, list.diagnostics);
    }

    #[test]
    fn show_revision_includes_current_assessment() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let assessment = record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_assessment(ReviewAssessment::Accepted)
                .with_summary("ship it"),
        )
        .unwrap();

        let unit = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert_eq!(
            unit.current_assessment.status,
            CurrentAssessmentStatus::Resolved(ReviewAssessment::Accepted)
        );
        assert_eq!(unit.assessments.len(), 1);
        assert_eq!(unit.assessments[0].id, assessment.assessment_id);
        assert_eq!(unit.summary.assessment_count, 1);
        assert!(
            unit.rows
                .iter()
                .any(|row| row.kind.as_str() == "assessment")
        );
    }

    #[test]
    fn show_revision_assessments_match_show_semantics() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        add_replaced_and_duplicate_assessments(&repo);

        let unit =
            show_revision(RevisionShowOptions::new(repo.path()).with_include_body(true)).unwrap();
        let show = show_assessments(
            AssessmentShowOptions::new(repo.path())
                .with_include_summary(true)
                .with_all(true),
        )
        .unwrap();

        assert_eq!(unit.current_assessment, show.current);
        assert_eq!(unit.assessments, show.assessments);
        assert_eq!(unit.diagnostics, show.diagnostics);
    }

    #[test]
    fn show_revision_places_reviewed_material_before_snapshot_remainder() {
        let repo = multi_hunk_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Important")
                .with_target(ObservationTargetSelector::file("src/lib.rs")),
        )
        .unwrap();

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        let first_snapshot_remainder = result
            .rows
            .iter()
            .position(|row| row.projection_phase.as_str() == "snapshot_remainder")
            .unwrap();
        let observation_row = result
            .rows
            .iter()
            .position(|row| row.kind.as_str() == "observation")
            .unwrap();

        assert!(observation_row < first_snapshot_remainder);
        assert_eq!(result.summary.narrative_row_count, first_snapshot_remainder);
        assert!(result.summary.snapshot_remainder_row_count > 0);
    }

    #[test]
    fn show_revision_keeps_unreviewed_snapshot_rows_complete() {
        let repo = multi_file_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Review wide"),
        )
        .unwrap();

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        let snapshot_row_count = result
            .rows
            .iter()
            .filter(|row| row.snapshot_order.is_some())
            .count();
        assert_eq!(snapshot_row_count, result.summary.snapshot_row_count);
        assert!(
            result
                .rows
                .iter()
                .any(|row| row.coverage.as_str() == "unreviewed")
        );
    }

    #[test]
    fn show_revision_track_filter_narrows_narrative_without_mutating_snapshot_remainder() {
        let repo = multi_file_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        add_observation(&repo, "agent:codex", "Codex");
        add_observation(&repo, "agent:claude", "Claude");

        let all = show_revision(RevisionShowOptions::new(repo.path())).unwrap();
        let codex =
            show_revision(RevisionShowOptions::new(repo.path()).with_track("agent:codex")).unwrap();

        assert!(all.summary.narrative_row_count > codex.summary.narrative_row_count);
        assert_eq!(
            all.summary.snapshot_remainder_row_count,
            codex.summary.snapshot_remainder_row_count
        );
        assert!(
            codex
                .observations
                .iter()
                .all(|obs| obs.track_id.as_str() == "agent:codex")
        );
    }

    #[test]
    fn show_revision_surfaces_floating_then_anchored() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let floating = show_revision(RevisionShowOptions::new(repo.path())).unwrap();
        assert!(!floating.commit_range.anchored);
        assert!(floating.commit_range.current_commits.is_empty());

        record_commit_association(repo.path(), &capture, "oidA");

        let anchored = show_revision(RevisionShowOptions::new(repo.path())).unwrap();
        assert!(anchored.commit_range.anchored);
        assert_eq!(anchored.commit_range.current_commits.len(), 1);
        assert_eq!(anchored.commit_range.current_commits[0].commit_oid, "oidA");
    }

    #[test]
    fn show_revision_extends_diagnostics_with_commit_range_diagnostics() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        // Two association edges sharing one tree under different commit OIDs — the
        // fold's content-equivalent-rewrite diagnostic. (Divergence is an
        // enrichment-level verdict needing real ancestry, so the fold seam is
        // exercised through the rewrite code.)
        record_commit_association_with_tree(repo.path(), &capture, "oidA", "sharedtree");
        record_commit_association_with_tree(repo.path(), &capture, "oidB", "sharedtree");

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "rewritten_commit_association")
        );
    }

    fn record_commit_association(repo: &Path, capture: &CaptureResult, commit_oid: &str) {
        record_commit_association_with_tree(
            repo,
            capture,
            commit_oid,
            &format!("{commit_oid}-tree"),
        );
    }

    fn record_commit_association_with_tree(
        repo: &Path,
        capture: &CaptureResult,
        commit_oid: &str,
        tree_oid: &str,
    ) {
        use crate::session::event::{RevisionCommitAssociatedPayload, build_commit_association_id};
        let commit_association_id =
            build_commit_association_id(&capture.revision_id, commit_oid).unwrap();
        let target = EventTarget::for_revision(
            crate::model::JournalId::new("journal:default"),
            capture.revision_id.clone(),
            Some(crate::model::TrackId::new("agent:codex")),
        )
        .unwrap();
        let event = ShoreEvent::new(
            EventType::RevisionCommitAssociated,
            RevisionCommitAssociatedPayload::idempotency_key(&capture.revision_id, commit_oid),
            target,
            Writer::shore_local("0.1.0"),
            RevisionCommitAssociatedPayload {
                commit_association_id,
                target: crate::model::ReviewTargetRef::Revision {
                    revision_id: capture.revision_id.clone(),
                },
                commit: crate::model::ReviewEndpoint::GitCommit {
                    commit_oid: commit_oid.to_owned(),
                    tree_oid: tree_oid.to_owned(),
                },
            },
            "2026-05-10T00:00:00Z",
        )
        .unwrap();

        EventStore::open(resolved_store_dir(repo))
            .record_event_once(&event)
            .unwrap();
    }

    fn modified_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        repo
    }

    fn record_validation_event(repo: &Path, capture: &CaptureResult, validation_check_id: &str) {
        let target = EventTarget::for_revision(
            crate::model::JournalId::new("journal:default"),
            capture.revision_id.clone(),
            Some(crate::model::TrackId::new("agent:codex")),
        )
        .unwrap();
        let event = ShoreEvent::new(
            EventType::ValidationCheckRecorded,
            format!("validation_check_recorded:{validation_check_id}"),
            target,
            Writer::shore_local("0.1.0"),
            ValidationCheckRecordedPayload {
                validation_check_id: ValidationCheckId::new(validation_check_id),
                target: ValidationTarget::Revision {
                    revision_id: capture.revision_id.clone(),
                },
                check_name: "cargo test".to_owned(),
                command: None,
                status: ValidationStatus::Passed,
                exit_code: Some(0),
                trigger: ValidationTrigger::Manual,
                source_fingerprint: None,
                summary: Some("tests passed".to_owned()),
                summary_content_type: Default::default(),
                summary_artifact_path: None,
                summary_byte_size: Some(12),
                summary_content_hash: Some("sha256:summary".to_owned()),
                started_at: None,
                completed_at: Some("2026-05-10T00:00:00Z".to_owned()),
                log_artifact_content_hashes: Vec::new(),
            },
            "2026-05-10T00:00:00Z",
        )
        .unwrap();

        EventStore::open(resolved_store_dir(repo))
            .record_event_once(&event)
            .unwrap();
    }

    fn multi_file_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.write("src/other.rs", "pub fn other() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        repo.write("src/other.rs", "pub fn other() -> u32 { 2 }\n");
        repo
    }

    fn multi_hunk_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write(
            "src/lib.rs",
            (1..=30)
                .map(|line| format!("pub fn value_{line}() -> u32 {{ {line} }}\n"))
                .collect::<String>(),
        );
        repo.commit_all("base");
        repo.write(
            "src/lib.rs",
            (1..=30)
                .map(|line| {
                    let value = if line == 2 || line == 28 {
                        line + 100
                    } else {
                        line
                    };
                    format!("pub fn value_{line}() -> u32 {{ {value} }}\n")
                })
                .collect::<String>(),
        );
        repo
    }

    fn add_observation(repo: &TestRepo, track: &str, title: &str) {
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track(track)
                .with_title(title),
        )
        .unwrap();
    }

    fn add_duplicate_observations_with_distinct_idempotency_keys(repo: &TestRepo) {
        let first = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Same finding")
                .with_body("same body")
                .with_idempotency_key("retry-a"),
        )
        .unwrap();
        let second = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Same finding")
                .with_body("same body")
                .with_idempotency_key("retry-b"),
        )
        .unwrap();

        assert_eq!(first.observation_id, second.observation_id);
    }

    fn add_superseding_observation(repo: &TestRepo) {
        let original = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Original"),
        )
        .unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Correction")
                .superseding(original.observation_id),
        )
        .unwrap();
    }

    fn add_duplicate_input_requests(repo: &TestRepo) {
        let first = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Same decision")
                .with_body("same body")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired)
                .with_idempotency_key("input-request-retry-a"),
        )
        .unwrap();
        let second = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Same decision")
                .with_body("same body")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired)
                .with_idempotency_key("input-request-retry-b"),
        )
        .unwrap();

        assert_eq!(first.input_request_id, second.input_request_id);
    }

    fn add_ambiguous_input_request_responses(repo: &TestRepo) {
        let request = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("agent:codex")
                .with_title("Ambiguous")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired),
        )
        .unwrap();
        respond_input_request(
            InputRequestRespondOptions::new(repo.path(), request.input_request_id.clone())
                .with_outcome(InputRequestResponseOutcome::Approved),
        )
        .unwrap();
        respond_input_request(
            InputRequestRespondOptions::new(repo.path(), request.input_request_id)
                .with_outcome(InputRequestResponseOutcome::Rejected),
        )
        .unwrap();
    }

    fn add_replaced_and_duplicate_assessments(repo: &TestRepo) {
        let duplicate_options = AssessmentAddOptions::new(repo.path())
            .with_track("human:kevin")
            .with_assessment(ReviewAssessment::NeedsClarification)
            .with_summary("same summary");
        let first = record_assessment(
            duplicate_options
                .clone()
                .with_idempotency_key("assessment-retry-a"),
        )
        .unwrap();
        let second =
            record_assessment(duplicate_options.with_idempotency_key("assessment-retry-b"))
                .unwrap();

        assert_eq!(first.assessment_id, second.assessment_id);

        record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_track("human:kevin")
                .with_assessment(ReviewAssessment::AcceptedWithFollowUp)
                .with_summary("replacement")
                .replacing(first.assessment_id),
        )
        .unwrap();
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
                "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
                args,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    /// The store a workflow actually lands in for `repo` — the shared common-dir
    /// store by default. Reads that follow a workflow resolve here, not the raw
    /// worktree-local `.pointbreak/data`.
    fn resolved_store_dir(repo: &Path) -> std::path::PathBuf {
        crate::git::git_common_dir(repo).unwrap().join("pointbreak")
    }

    fn tamper_object_artifact_snapshot_field(repo: &Path, object_id: &ObjectId) {
        let path = object_artifact_path(repo, object_id);
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("read object artifact"))
                .expect("parse object artifact json");

        assert_eq!(json["snapshot"]["object_id"], object_id.as_str());
        // Perturb a field inside the v2 content hash without re-stamping it.
        // `DiffFile` is snake_case, unlike the camelCase artifact wrapper.
        json["snapshot"]["files"][0]["new_path"] = "/evil".into();

        fs::write(
            &path,
            serde_json::to_vec_pretty(&json).expect("serialize tampered object artifact"),
        )
        .expect("write tampered object artifact");
    }

    fn rewrite_capture_event_object_artifact_hash(
        repo: &Path,
        revision_id: &RevisionId,
        hash: &str,
    ) {
        let path = capture_event_path(repo, revision_id);
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("read capture event"))
                .expect("parse capture event json");

        json["payload"]["workObject"]["objectArtifactContentHash"] = hash.into();
        json["payloadHash"] = sha256_json_prefixed(&json["payload"])
            .expect("hash rewritten capture event payload")
            .into();

        fs::write(
            &path,
            serde_json::to_vec_pretty(&json).expect("serialize rewritten capture event"),
        )
        .expect("write rewritten capture event");
    }

    fn record_artifact_removed(repo: &Path, content_hash: &str) {
        let event = ShoreEvent::new(
            EventType::ArtifactRemoved,
            ArtifactRemovedPayload::idempotency_key(content_hash),
            EventTarget::for_journal(crate::model::JournalId::new("journal:default")),
            Writer::shore_local("0.1.0"),
            ArtifactRemovedPayload {
                content_hash: content_hash.to_owned(),
            },
            "2026-05-10T00:00:00Z",
        )
        .unwrap();
        EventStore::open(resolved_store_dir(repo))
            .record_event_once(&event)
            .unwrap();
    }

    /// Record an `ArtifactRemoved` that arrived through a foreign-event seam
    /// (`ingest = Some`): it has no local-possession arm, so the default policy
    /// reads it as a non-operative claim.
    fn record_ingested_artifact_removed(repo: &Path, content_hash: &str) {
        let mut event = ShoreEvent::new(
            EventType::ArtifactRemoved,
            ArtifactRemovedPayload::idempotency_key(content_hash),
            EventTarget::for_journal(crate::model::JournalId::new("journal:default")),
            Writer::shore_local("0.1.0"),
            ArtifactRemovedPayload {
                content_hash: content_hash.to_owned(),
            },
            "2026-05-10T00:00:00Z",
        )
        .unwrap();
        event.ingest = Some(IngestProvenance {
            via: IngestVia::IngestEvents,
            received_at: "2026-05-10T01:00:00Z".to_owned(),
        });
        EventStore::open(resolved_store_dir(repo))
            .record_event_once(&event)
            .unwrap();
    }

    /// Record an `ArtifactRemoved` carrying an inline signature, optionally
    /// ingested and optionally tampered. `validate_event` does not verify
    /// signatures, so the read-time reader-relative check classifies it: a valid
    /// signature under the empty default trust reads `UntrustedKey`, a tampered
    /// one reads `Invalid`.
    fn record_signed_artifact_removed(repo: &Path, content_hash: &str, ingest: bool, tamper: bool) {
        let signer = DeterministicSigner::from_seed([91u8; 32]);
        let mut event = ShoreEvent::new(
            EventType::ArtifactRemoved,
            ArtifactRemovedPayload::idempotency_key(content_hash),
            EventTarget::for_journal(crate::model::JournalId::new("journal:default")),
            Writer::shore_local("0.1.0"),
            ArtifactRemovedPayload {
                content_hash: content_hash.to_owned(),
            },
            "2026-05-10T00:00:00Z",
        )
        .unwrap();
        let tbs = EventToBeSigned::from_event(&event, signer.signer_id()).unwrap();
        let pae = event_signature_pre_authentication_encoding(&tbs).unwrap();
        let sig = signer.sign_event_message(&pae).unwrap();
        event.signer = Some(signer.signer_id().clone());
        event.signature = Some(if tamper {
            EventSignature::ed25519_v1(EventSignatureBytes::from_bytes(&[0u8; 64]))
        } else {
            EventSignature::ed25519_v1(sig)
        });
        if ingest {
            event.ingest = Some(IngestProvenance {
                via: IngestVia::IngestEvents,
                received_at: "2026-05-10T01:00:00Z".to_owned(),
            });
        }
        EventStore::open(resolved_store_dir(repo))
            .record_event_once(&event)
            .unwrap();
    }

    /// Fabricate a second, distinct-revision capture binding the SAME content
    /// hash as `capture` (the content/object identity-reuse case).
    fn fabricate_distinct_sibling_capture(repo: &Path, capture: &CaptureResult) {
        let sibling_unit = RevisionId::new(format!("{}-reuse", capture.revision_id.as_str()));
        let event = ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("work_object_proposed:{}", sibling_unit.as_str()),
            EventTarget::for_revision(capture.journal_id.clone(), sibling_unit.clone(), None)
                .unwrap(),
            Writer::shore_local("0.1.0"),
            WorkObjectProposedPayload {
                engagement_id: EngagementId::new("engagement:sha256:reuse"),
                work_object: WorkObjectProposal::Revision {
                    revision: Revision {
                        id: sibling_unit,
                        object_id: capture.object_id.clone(),
                        git_provenance: None,
                    },
                    object_artifact_content_hash: capture.object_artifact_content_hash.clone(),
                    supersedes: vec![],
                },
            },
            "2026-05-10T00:00:00Z",
        )
        .unwrap();
        EventStore::open(resolved_store_dir(repo))
            .record_event_once(&event)
            .unwrap();
    }

    fn delete_snapshot_blob(repo: &Path, object_id: &ObjectId) {
        let path = object_artifact_path(repo, object_id);
        fs::remove_file(path).expect("delete snapshot blob");
    }

    /// Twin of `delete_snapshot_blob` for note-body artifacts: unlink the
    /// `artifacts/notes/<hex>.json` blob behind a normalized `sha256:` hash.
    fn delete_note_body_blob(repo: &Path, body_content_hash: &str) {
        let hex = body_content_hash
            .strip_prefix("sha256:")
            .expect("normalized body content hash");
        let path = resolved_store_dir(repo)
            .join("artifacts")
            .join("notes")
            .join(format!("{hex}.json"));
        fs::remove_file(path).expect("delete note body blob");
    }

    /// Capture plus one externalized (> 4096-byte) observation body; returns
    /// the repo and the body's normalized content hash (the removal key).
    fn revision_with_externalized_observation_body() -> (TestRepo, String) {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let observation = record_observation(
            ObservationAddOptions::new(repo.path())
                .with_revision_id(capture.revision_id.clone())
                .with_track("agent:tester")
                .with_title("a large observation")
                .with_body("x".repeat(5000)),
        )
        .unwrap();
        let hash = observation
            .body_content_hash
            .expect("a >4096-byte body is stored as a note artifact");
        (repo, hash)
    }

    #[test]
    fn removed_and_swept_observation_body_renders_physically_removed_not_a_hard_error() {
        let (repo, body_hash) = revision_with_externalized_observation_body();
        record_artifact_removed(repo.path(), &body_hash);
        delete_note_body_blob(repo.path(), &body_hash);

        let result = show_revision(RevisionShowOptions::new(repo.path()).with_include_body(true))
            .expect("swept observation body must not hard-error");

        let observation = &result.observations[0];
        assert_eq!(observation.body, None);
        assert_eq!(
            observation.body_content_state,
            BodyContentState::PhysicallyRemoved
        );
        assert!(
            observation.body_content_hash.is_some(),
            "hash survives removal"
        );
    }

    #[test]
    fn removed_unswept_observation_body_is_suppressed_present() {
        let (repo, body_hash) = revision_with_externalized_observation_body();
        record_artifact_removed(repo.path(), &body_hash);

        let result = show_revision(RevisionShowOptions::new(repo.path()).with_include_body(true))
            .expect("suppressed body renders without a hard error");

        let observation = &result.observations[0];
        assert_eq!(observation.body, None);
        assert_eq!(
            observation.body_content_state,
            BodyContentState::SuppressedPresent
        );
        assert!(observation.body_content_hash.is_some());
    }

    #[test]
    fn ingested_unsigned_body_removal_renders_present_under_default_policy() {
        let (repo, body_hash) = revision_with_externalized_observation_body();
        record_ingested_artifact_removed(repo.path(), &body_hash);

        let result = show_revision(RevisionShowOptions::new(repo.path()).with_include_body(true))
            .expect("non-operative claim renders the bytes");

        let observation = &result.observations[0];
        assert_eq!(observation.body_content_state, BodyContentState::Present);
        assert_eq!(observation.body.as_deref(), Some("x".repeat(5000).as_str()));
    }

    #[test]
    fn possessed_body_removal_renders_present_under_trusted_strict() {
        let (repo, body_hash) = revision_with_externalized_observation_body();
        record_artifact_removed(repo.path(), &body_hash);

        let result = show_revision(
            RevisionShowOptions::new(repo.path())
                .with_include_body(true)
                .with_removal_policy(RemovalPolicy::TrustedStrict),
        )
        .expect("possession arm dropped under trusted-strict");

        let observation = &result.observations[0];
        assert_eq!(observation.body_content_state, BodyContentState::Present);
        assert!(observation.body.is_some());
    }

    #[test]
    fn truly_missing_unremoved_body_still_errors() {
        let (repo, body_hash) = revision_with_externalized_observation_body();
        delete_note_body_blob(repo.path(), &body_hash);

        let err = show_revision(RevisionShowOptions::new(repo.path()).with_include_body(true))
            .expect_err("absent bytes without an operative removal keep the hard error");

        assert!(err.to_string().contains("import referenced artifacts"));
    }

    #[test]
    fn removed_body_diagnostics_surface_on_show_revision() {
        let (repo, body_hash) = revision_with_externalized_observation_body();
        record_artifact_removed(repo.path(), &body_hash);
        delete_note_body_blob(repo.path(), &body_hash);

        // Default options: the state (and so the diagnostic) surfaces even when
        // bodies are not hydrated.
        let result = show_revision(RevisionShowOptions::new(repo.path())).expect("renders");

        assert!(result.diagnostics.iter().any(|d| {
            d.code == "body_content_physically_removed" && d.message.contains(&body_hash)
        }));
    }

    #[test]
    fn suppressed_present_body_diagnostic_does_not_claim_bytes_are_gone() {
        let (repo, body_hash) = revision_with_externalized_observation_body();
        record_artifact_removed(repo.path(), &body_hash);

        let result = show_revision(RevisionShowOptions::new(repo.path())).expect("renders");

        let diagnostic = result
            .diagnostics
            .iter()
            .find(|d| d.code == "body_content_suppressed_present")
            .expect("suppressed-present body diagnostic");
        assert!(diagnostic.message.contains(&body_hash));
        assert!(diagnostic.message.contains("still stored"));
        assert!(!diagnostic.message.contains("swept"));
    }

    /// Capture plus one externalized (> 4096-byte) assessment summary; returns
    /// the repo and the summary's normalized content hash (the removal key).
    fn revision_with_externalized_assessment_summary() -> (TestRepo, String) {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let assessment = record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_revision_id(capture.revision_id.clone())
                .with_track("human:kevin")
                .with_assessment(ReviewAssessment::Accepted)
                .with_summary("s".repeat(5000)),
        )
        .unwrap();
        let hash = assessment
            .summary_content_hash
            .expect("a >4096-byte summary is stored as a note artifact");
        (repo, hash)
    }

    #[test]
    fn removed_and_swept_assessment_summary_renders_physically_removed() {
        let (repo, summary_hash) = revision_with_externalized_assessment_summary();
        record_artifact_removed(repo.path(), &summary_hash);
        delete_note_body_blob(repo.path(), &summary_hash);

        let result = show_revision(RevisionShowOptions::new(repo.path()).with_include_body(true))
            .expect("swept assessment summary must not hard-error");

        let record = &result.current_assessment.records[0];
        assert_eq!(record.summary, None);
        assert_eq!(
            record.summary_content_state,
            BodyContentState::PhysicallyRemoved
        );
        assert!(record.summary_content_hash.is_some());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "body_content_physically_removed"
                    && d.message.contains(&summary_hash))
        );
    }

    #[test]
    fn removed_unswept_assessment_summary_is_suppressed_present() {
        let (repo, summary_hash) = revision_with_externalized_assessment_summary();
        record_artifact_removed(repo.path(), &summary_hash);

        let result = show_revision(RevisionShowOptions::new(repo.path()).with_include_body(true))
            .expect("suppressed assessment summary renders");

        let record = &result.current_assessment.records[0];
        assert_eq!(record.summary, None);
        assert_eq!(
            record.summary_content_state,
            BodyContentState::SuppressedPresent
        );
    }

    #[test]
    fn missing_unremoved_assessment_summary_still_errors() {
        let (repo, summary_hash) = revision_with_externalized_assessment_summary();
        delete_note_body_blob(repo.path(), &summary_hash);

        let err = show_revision(RevisionShowOptions::new(repo.path()).with_include_body(true))
            .expect_err("absent summary bytes without an operative removal keep the hard error");

        assert!(err.to_string().contains("import referenced artifacts"));
    }

    /// INV: without a lens (status-only projections, e.g. the engagement
    /// lifecycle) no state resolution happens — an operative removal over the
    /// summary hash still reads `Present` because nothing consults it.
    #[test]
    fn status_only_assessment_projection_with_no_lens_stays_present() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let assessment = record_assessment(
            AssessmentAddOptions::new(repo.path())
                .with_revision_id(capture.revision_id.clone())
                .with_track("human:kevin")
                .with_assessment(ReviewAssessment::Accepted)
                .with_summary("s".repeat(5000)),
        )
        .unwrap();
        let hash = assessment
            .summary_content_hash
            .expect("a >4096-byte summary is stored as a note artifact");
        record_artifact_removed(repo.path(), &hash);

        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        let resolved = ResolvedRevision {
            journal_id: capture.journal_id.clone(),
            revision_id: capture.revision_id.clone(),
            object_id: capture.object_id.clone(),
            object_artifact_content_hash: capture.object_artifact_content_hash.clone(),
        };
        let (current, _) = project_assessments(AssessmentProjectionOptions {
            backend: None,
            events: &events,
            resolved: &resolved,
            track_filter: None,
            include_summary: false,
            include_all: true,
            removal_lens: None,
        })
        .unwrap();

        let record = &current.records[0];
        assert_eq!(record.summary, None);
        assert_eq!(record.summary_content_state, BodyContentState::Present);
    }

    /// Capture plus an input request with an externalized (> 4096-byte) body
    /// and one response with an externalized reason; returns the repo and the
    /// two normalized content hashes (the removal keys).
    fn revision_with_externalized_input_request() -> (TestRepo, String, String) {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let request = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_revision_id(capture.revision_id.clone())
                .with_track("agent:codex")
                .with_title("a large request")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired)
                .with_body("b".repeat(5000)),
        )
        .unwrap();
        let body_hash = request
            .body_content_hash
            .expect("a >4096-byte body is stored as a note artifact");
        let response = respond_input_request(
            InputRequestRespondOptions::new(repo.path(), request.input_request_id.clone())
                .with_outcome(InputRequestResponseOutcome::Approved)
                .with_reason("r".repeat(5000)),
        )
        .unwrap();
        let reason_hash = response
            .reason_content_hash
            .expect("a >4096-byte reason is stored as a note artifact");
        (repo, body_hash, reason_hash)
    }

    #[test]
    fn removed_and_swept_input_request_body_renders_physically_removed() {
        let (repo, body_hash, _reason_hash) = revision_with_externalized_input_request();
        record_artifact_removed(repo.path(), &body_hash);
        delete_note_body_blob(repo.path(), &body_hash);

        let result = show_revision(RevisionShowOptions::new(repo.path()).with_include_body(true))
            .expect("swept input-request body must not hard-error");

        let request = &result.input_requests[0];
        assert_eq!(request.body, None);
        assert_eq!(
            request.body_content_state,
            BodyContentState::PhysicallyRemoved
        );
        assert!(request.body_content_hash.is_some());
    }

    #[test]
    fn removed_unswept_input_request_body_is_suppressed_present() {
        let (repo, body_hash, _reason_hash) = revision_with_externalized_input_request();
        record_artifact_removed(repo.path(), &body_hash);

        let result = show_revision(RevisionShowOptions::new(repo.path()).with_include_body(true))
            .expect("suppressed input-request body renders");

        let request = &result.input_requests[0];
        assert_eq!(request.body, None);
        assert_eq!(
            request.body_content_state,
            BodyContentState::SuppressedPresent
        );
    }

    #[test]
    fn missing_unremoved_input_request_body_still_errors() {
        let (repo, body_hash, _reason_hash) = revision_with_externalized_input_request();
        delete_note_body_blob(repo.path(), &body_hash);

        let err = show_revision(RevisionShowOptions::new(repo.path()).with_include_body(true))
            .expect_err("absent request-body bytes without an operative removal keep the error");

        assert!(err.to_string().contains("import referenced artifacts"));
    }

    /// Response reasons hydrate through the shared body resolution like every
    /// other note-shaped body, so a removed reason renders the explained
    /// state: `reason` is absent, the state and the diagnostic explain the
    /// removal, and the read never hard-errors.
    #[test]
    fn removed_response_reason_renders_explained_state() {
        let (repo, _body_hash, reason_hash) = revision_with_externalized_input_request();
        record_artifact_removed(repo.path(), &reason_hash);
        delete_note_body_blob(repo.path(), &reason_hash);

        let result = show_revision(RevisionShowOptions::new(repo.path()).with_include_body(true))
            .expect("swept response reason must not hard-error");

        let response = &result.input_requests[0].responses[0];
        assert_eq!(response.reason, None);
        assert_eq!(
            response.reason_content_state,
            BodyContentState::PhysicallyRemoved
        );
        assert!(result.diagnostics.iter().any(
            |d| d.code == "body_content_physically_removed" && d.message.contains(&reason_hash)
        ));
    }

    #[test]
    fn list_observations_surfaces_removed_body_diagnostics() {
        let (repo, body_hash) = revision_with_externalized_observation_body();
        record_artifact_removed(repo.path(), &body_hash);
        delete_note_body_blob(repo.path(), &body_hash);

        let result =
            list_observations(ObservationListOptions::new(repo.path()).with_include_body(true))
                .expect("swept body must not hard-error the list");

        assert_eq!(
            result.observations[0].body_content_state,
            BodyContentState::PhysicallyRemoved
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "body_content_physically_removed"
                    && d.message.contains(&body_hash))
        );
    }

    #[test]
    fn list_observations_honors_removal_policy_option() {
        let (repo, body_hash) = revision_with_externalized_observation_body();
        record_artifact_removed(repo.path(), &body_hash);

        let result = list_observations(
            ObservationListOptions::new(repo.path())
                .with_include_body(true)
                .with_removal_policy(RemovalPolicy::Advisory),
        )
        .expect("advisory policy renders the bytes");

        assert_eq!(
            result.observations[0].body_content_state,
            BodyContentState::Present
        );
        assert!(result.observations[0].body.is_some());
        assert!(
            !result
                .diagnostics
                .iter()
                .any(|d| d.code.starts_with("body_content_"))
        );
    }

    #[test]
    fn list_input_requests_surfaces_removed_body_diagnostics() {
        let (repo, body_hash, reason_hash) = revision_with_externalized_input_request();
        record_artifact_removed(repo.path(), &body_hash);
        record_artifact_removed(repo.path(), &reason_hash);
        delete_note_body_blob(repo.path(), &body_hash);

        let result = list_input_requests(
            InputRequestListOptions::new(repo.path())
                .with_include_body(true)
                .with_status(InputRequestStatusFilter::All),
        )
        .expect("swept body must not hard-error the list");

        assert_eq!(
            result.input_requests[0].body_content_state,
            BodyContentState::PhysicallyRemoved
        );
        assert_eq!(
            result.input_requests[0].responses[0].reason_content_state,
            BodyContentState::SuppressedPresent
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "body_content_physically_removed")
        );
        assert!(result.diagnostics.iter().any(
            |d| d.code == "body_content_suppressed_present" && d.message.contains(&reason_hash)
        ));
    }

    #[test]
    fn fetch_input_request_surfaces_removed_body_diagnostics() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let request = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_revision_id(capture.revision_id.clone())
                .with_track("agent:codex")
                .with_title("a large request")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired)
                .with_body("b".repeat(5000)),
        )
        .unwrap();
        let body_hash = request.body_content_hash.expect("externalized body");
        record_artifact_removed(repo.path(), &body_hash);
        delete_note_body_blob(repo.path(), &body_hash);

        let result = fetch_input_request(
            InputRequestFetchOptions::new(repo.path(), request.input_request_id.clone())
                .with_include_body(true),
        )
        .expect("swept body must not hard-error the fetch");

        assert_eq!(
            result.input_request.body_content_state,
            BodyContentState::PhysicallyRemoved
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "body_content_physically_removed")
        );
    }

    #[test]
    fn show_assessments_surfaces_removed_summary_diagnostics() {
        let (repo, summary_hash) = revision_with_externalized_assessment_summary();
        record_artifact_removed(repo.path(), &summary_hash);
        delete_note_body_blob(repo.path(), &summary_hash);

        let result = show_assessments(
            AssessmentShowOptions::new(repo.path())
                .with_include_summary(true)
                .with_all(true),
        )
        .expect("swept summary must not hard-error the show");

        assert_eq!(
            result.assessments[0].summary_content_state,
            BodyContentState::PhysicallyRemoved
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "body_content_physically_removed")
        );
    }

    #[test]
    fn list_validation_checks_surfaces_removed_summary_diagnostics() {
        use crate::session::{
            ValidationAddOptions, ValidationListOptions, list_validation_checks,
            record_validation_check,
        };

        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let check = record_validation_check(
            ValidationAddOptions::new(repo.path())
                .with_revision_id(capture.revision_id.clone())
                .with_track("agent:codex")
                .with_check_name("cargo test")
                .with_status(ValidationStatus::Passed)
                .with_summary("v".repeat(5000)),
        )
        .unwrap();
        let summary_hash = check.summary_content_hash.expect("externalized summary");
        record_artifact_removed(repo.path(), &summary_hash);
        delete_note_body_blob(repo.path(), &summary_hash);

        let result =
            list_validation_checks(ValidationListOptions::new(repo.path()).with_include_body(true))
                .expect("swept summary must not hard-error the list");

        assert_eq!(
            result.validation_checks[0].summary_content_state,
            BodyContentState::PhysicallyRemoved
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "body_content_physically_removed")
        );
    }

    #[test]
    fn ingested_unsigned_removal_emits_removal_claim_unsigned() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_ingested_artifact_removed(repo.path(), &capture.object_artifact_content_hash);

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "removal_claim_unsigned")
        );
        // The non-operative claim renders the bytes rather than suppressing them.
        assert!(!result.snapshot.files.is_empty());
    }

    #[test]
    fn ingested_untrusted_removal_emits_removal_claim_untrusted() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_signed_artifact_removed(
            repo.path(),
            &capture.object_artifact_content_hash,
            true,
            false,
        );

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "removal_claim_untrusted")
        );
    }

    #[test]
    fn invalid_signature_removal_emits_removal_claim_invalid() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        // Locally authored (possessed) but the inline signature is tampered: the
        // integrity floor classifies it invalid even with possession.
        record_signed_artifact_removed(
            repo.path(),
            &capture.object_artifact_content_hash,
            false,
            true,
        );

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "removal_claim_invalid")
        );
    }

    #[test]
    fn removal_of_unreferenced_hash_emits_removed_target_missing() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        // A possessed removal over a hash no event references.
        record_artifact_removed(repo.path(), "sha256:ghost-unreferenced");

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert!(result.diagnostics.iter().any(|d| {
            d.code == "snapshot_content_removed_target_missing"
                && d.message.contains("sha256:ghost-unreferenced")
        }));
    }

    #[test]
    fn second_capture_over_removed_hash_emits_identity_reused_after_removal() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        // A distinct capture re-binds the same content hash, then the hash is
        // removed (possessed → operative).
        fabricate_distinct_sibling_capture(repo.path(), &capture);
        record_artifact_removed(repo.path(), &capture.object_artifact_content_hash);

        let result = show_revision(
            RevisionShowOptions::new(repo.path()).with_revision_id(capture.revision_id.clone()),
        )
        .unwrap();

        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "identity_reused_after_removal")
        );
    }

    #[test]
    fn ingested_unsigned_removal_renders_present_under_default_policy() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_ingested_artifact_removed(repo.path(), &capture.object_artifact_content_hash);

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        // An ingested, unverified removal is non-operative under the default
        // policy: the snapshot renders rather than being suppressed.
        assert_eq!(result.snapshot_content_state, SnapshotContentState::Present);
        assert!(!result.snapshot_is_removed());
        assert!(!result.snapshot.files.is_empty());
    }

    #[test]
    fn possessed_unsigned_removal_still_suppresses_under_default() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        // Locally authored (ingest = None): the zero-key local possession floor.
        record_artifact_removed(repo.path(), &capture.object_artifact_content_hash);

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert_eq!(
            result.snapshot_content_state,
            SnapshotContentState::SuppressedPresent
        );
        assert!(result.snapshot_is_removed());
    }

    #[test]
    fn possessed_removal_renders_present_under_trusted_strict() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_artifact_removed(repo.path(), &capture.object_artifact_content_hash);

        let result = show_revision(
            RevisionShowOptions::new(repo.path()).with_removal_policy(RemovalPolicy::TrustedStrict),
        )
        .unwrap();

        // TrustedStrict drops the possession arm; the possessed unsigned removal
        // no longer suppresses.
        assert_eq!(result.snapshot_content_state, SnapshotContentState::Present);
        assert!(!result.snapshot_is_removed());
    }

    #[test]
    fn removed_and_swept_snapshot_renders_physically_removed_not_a_hard_error() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_artifact_removed(repo.path(), &capture.object_artifact_content_hash);
        delete_snapshot_blob(repo.path(), &capture.object_id);

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert!(result.snapshot_is_removed());
        assert_eq!(
            result.removed_snapshot_content_hash.as_deref(),
            Some(capture.object_artifact_content_hash.as_str())
        );
        assert!(result.snapshot.files.is_empty());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "snapshot_content_physically_removed"),
            "expected a physically-removed diagnostic, got {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn result_carries_snapshot_content_state() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        // A present capture resolves to Present and is not removed.
        let present = show_revision(RevisionShowOptions::new(repo.path())).unwrap();
        assert_eq!(
            present.snapshot_content_state,
            SnapshotContentState::Present
        );
        assert!(!present.snapshot_is_removed());

        // A removal with the blob still on disk resolves to SuppressedPresent.
        record_artifact_removed(repo.path(), &capture.object_artifact_content_hash);
        let suppressed = show_revision(RevisionShowOptions::new(repo.path())).unwrap();
        assert_eq!(
            suppressed.snapshot_content_state,
            SnapshotContentState::SuppressedPresent
        );
        assert!(suppressed.snapshot_is_removed());

        // Once the blob is swept, it resolves to PhysicallyRemoved.
        delete_snapshot_blob(repo.path(), &capture.object_id);
        let removed = show_revision(RevisionShowOptions::new(repo.path())).unwrap();
        assert_eq!(
            removed.snapshot_content_state,
            SnapshotContentState::PhysicallyRemoved
        );
        assert!(removed.snapshot_is_removed());
    }

    #[test]
    fn suppressed_present_diagnostic_does_not_claim_bytes_are_gone() {
        // A removal is recorded but the blob is NOT swept (no compact): the
        // diagnostic must report suppression without claiming the bytes are gone.
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_artifact_removed(repo.path(), &capture.object_artifact_content_hash);

        let result = show_revision(RevisionShowOptions::new(repo.path())).unwrap();

        assert!(result.snapshot_is_removed());
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "snapshot_content_suppressed_present")
            .expect("expected a suppressed-present diagnostic");
        assert!(
            !diagnostic.message.contains("no longer stored"),
            "the suppressed-present message must not claim the bytes are gone: {}",
            diagnostic.message
        );
    }

    #[test]
    fn truly_missing_unremoved_snapshot_degrades_only_for_display_reads() {
        let repo = modified_repo();
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        // Delete the blob WITHOUT a removal fact: not-yet-synced, not removed.
        delete_snapshot_blob(repo.path(), &capture.object_id);

        let err = show_revision(RevisionShowOptions::new(repo.path())).unwrap_err();
        assert!(
            err.to_string().contains("import referenced artifacts"),
            "expected the hard missing-artifact error, got: {err}"
        );

        let result =
            show_revision(RevisionShowOptions::new(repo.path()).with_read_for_display(true))
                .expect("a display read preserves the incomplete revision");
        assert_eq!(
            result.snapshot_content_state,
            SnapshotContentState::Unavailable
        );
        assert!(!result.snapshot_is_removed());
        assert!(result.snapshot.files.is_empty());
        assert_eq!(result.summary.file_count, 0);
        assert!(result.rows.is_empty());
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == SNAPSHOT_CONTENT_UNAVAILABLE
                && diagnostic
                    .message
                    .contains(&capture.object_artifact_content_hash)
        }));
    }

    fn object_artifact_path(repo: &Path, object_id: &ObjectId) -> PathBuf {
        fs::read_dir(resolved_store_dir(repo).join("artifacts/objects"))
            .expect("read object artifacts directory")
            .map(|entry| entry.expect("read object artifact dir entry").path())
            .find(|path| {
                let Ok(bytes) = fs::read(path) else {
                    return false;
                };
                let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                    return false;
                };
                json["snapshot"]["object_id"] == object_id.as_str()
            })
            .expect("find object artifact")
    }

    fn capture_event_path(repo: &Path, revision_id: &RevisionId) -> PathBuf {
        fs::read_dir(resolved_store_dir(repo).join("events"))
            .expect("read events directory")
            .map(|entry| entry.expect("read event dir entry").path())
            .find(|path| {
                let Ok(bytes) = fs::read(path) else {
                    return false;
                };
                let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                    return false;
                };
                json["eventType"] == "t:02"
                    && json["payload"]["workObject"]["revision"]["id"] == revision_id.as_str()
            })
            .expect("find capture event")
    }
}
