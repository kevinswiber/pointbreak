use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::RevisionProjectionRow;
use crate::model::{
    ActorId, DiffSnapshot, EventId, JournalId, ObjectId, ReviewEndpoint, RevisionId,
    RevisionSource, TrackId,
};
use crate::session::assessment::{AssessmentView, CurrentAssessmentView};
use crate::session::input_request::InputRequestView;
use crate::session::observation::ObservationView;
use crate::session::state::ProjectionDiagnostic;
use crate::session::workflow::ValidationCheckView;
use crate::session::{
    ActorAttributesMap, DelegationMap, EndorsementReadback, EventVerificationPolicy,
    EventVerificationStatus, PrincipalResolution, RemovalPolicy, RevisionCommitRangeView, TrustSet,
    principal_resolution_for_writer,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionShowOptions {
    pub(super) repo: PathBuf,
    pub(super) revision_id: Option<RevisionId>,
    pub(super) track: Option<String>,
    pub(super) include_body: bool,
    pub(super) verification_policy: Option<EventVerificationPolicy>,
    pub(super) trust_set: TrustSet,
    pub(super) removal_policy: RemovalPolicy,
    pub(super) actor_attributes: Option<ActorAttributesMap>,
    pub(super) delegation_map: Option<DelegationMap>,
    pub(super) read_for_display: bool,
    pub(super) exact: bool,
}

impl RevisionShowOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            revision_id: None,
            track: None,
            include_body: false,
            verification_policy: None,
            trust_set: TrustSet::default(),
            removal_policy: RemovalPolicy::default(),
            actor_attributes: None,
            delegation_map: None,
            read_for_display: false,
            exact: false,
        }
    }

    pub fn with_revision_id(mut self, revision_id: RevisionId) -> Self {
        self.revision_id = Some(revision_id);
        self
    }
    pub fn with_track(mut self, track: impl Into<String>) -> Self {
        self.track = Some(track.into());
        self
    }

    pub fn with_include_body(mut self, include_body: bool) -> Self {
        self.include_body = include_body;
        self
    }

    /// Supply the verification policy. Advisory (render-only): its presence enables
    /// the per-event `verificationStatus` readback; it never gates a write.
    pub fn with_verification_policy(mut self, policy: EventVerificationPolicy) -> Self {
        self.verification_policy = Some(policy);
        self
    }

    /// Supply the reader's trust set. Status and endorsement classification resolve
    /// against it (reader-relativity); the empty default reads every signer as
    /// `untrusted_key` / `unknown_endorser`.
    pub fn with_trust_set(mut self, trust_set: TrustSet) -> Self {
        self.trust_set = trust_set;
        self
    }

    /// Supply the render-time removal policy. A non-operative removal claim — for
    /// example an ingested, unverified one under the default `PossessionOrTrusted`
    /// policy — renders the bytes and a claim diagnostic instead of suppressing
    /// the snapshot. Render-only: it never gates the compact erasure sweep.
    pub fn with_removal_policy(mut self, removal_policy: RemovalPolicy) -> Self {
        self.removal_policy = removal_policy;
        self
    }

    /// Supply the reader's actor-attributes map. Sibling enrichment for endorsement
    /// readbacks (the endorser's attested kind/roles) — never a classifier input.
    pub fn with_actor_attributes(mut self, actor_attributes: Option<ActorAttributesMap>) -> Self {
        self.actor_attributes = actor_attributes;
        self
    }

    /// Supply the reader-side delegation map. With it set, `show` emits
    /// `principal_unresolvable` / `principal_ambiguous` diagnostics for
    /// agent-written events whose principal does not resolve; without it, no
    /// principal diagnostics are emitted (the zero-setup floor stays silent).
    pub fn with_delegation_map(mut self, delegation_map: DelegationMap) -> Self {
        self.delegation_map = Some(delegation_map);
        self
    }

    /// Read for a human-facing surface: skip a retired/unsupported event and
    /// surface it as a diagnostic instead of hard-failing the read. Off by
    /// default, so the relay and other strict callers keep the typed error.
    pub fn with_read_for_display(mut self, value: bool) -> Self {
        self.read_for_display = value;
        self
    }

    /// Resolve the supplied revision id **exactly** (no head resolution), so a
    /// superseded revision shows itself rather than forward-resolving to its
    /// thread's current head (which errors on a competing fork). Off by default,
    /// so the `--revision` CLI seed keeps its head-seed semantics; the inspector,
    /// which addresses specific revisions by id, opts in.
    pub fn with_exact(mut self, value: bool) -> Self {
        self.exact = value;
        self
    }
}

/// Build `principal_unresolvable` / `principal_ambiguous` diagnostics for the
/// agent-written members of a unit. Non-agent writers are skipped (they are
/// their own principal); resolved agents are silent. Surface, never block
/// (ADR-0003).
pub(super) fn principal_diagnostics<'a>(
    members: impl Iterator<Item = (&'a ActorId, &'a str)>,
    map: &DelegationMap,
) -> Vec<ProjectionDiagnostic> {
    let mut diagnostics = Vec::new();
    for (writer_actor, occurred_at) in members {
        let agent = writer_actor.as_str();
        match principal_resolution_for_writer(writer_actor, map, occurred_at) {
            Some(PrincipalResolution::None(reason)) => diagnostics.push(ProjectionDiagnostic {
                code: "principal_unresolvable".to_owned(),
                message: format!(
                    "agent {agent} has no resolvable principal at {occurred_at} ({})",
                    reason.as_str()
                ),
            }),
            Some(PrincipalResolution::Ambiguous(principals)) => {
                let candidates = principals
                    .iter()
                    .map(ActorId::as_str)
                    .collect::<Vec<_>>()
                    .join(", ");
                diagnostics.push(ProjectionDiagnostic {
                    code: "principal_ambiguous".to_owned(),
                    message: format!(
                        "agent {agent} resolves to multiple principals at {occurred_at}: {candidates}"
                    ),
                });
            }
            // Resolved agents and non-agent writers are silent.
            Some(PrincipalResolution::Resolved(_)) | None => {}
        }
    }
    diagnostics
}

/// Reader-relative readback for one event, attached to unit-show documents by
/// event id. `verification_status` is the per-event signature ladder under the
/// reader's trust set; `endorsements` are its endorsement readbacks (with sibling
/// enrichment). Both render only when a verification policy is set.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MemberReadback {
    pub verification_status: Option<EventVerificationStatus>,
    pub endorsements: Vec<EndorsementReadback>,
}

/// The resolved read state of a revision's bound snapshot content. `Present`
/// renders bytes; the two removed states distinguish a recorded removal whose
/// blob is still on disk (`SuppressedPresent`, reversible until a compact
/// reclaims the bytes) from one whose blob has been swept (`PhysicallyRemoved`),
/// so the read surface never overstates what has happened.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SnapshotContentState {
    #[default]
    Present,
    SuppressedPresent,
    PhysicallyRemoved,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionShowResult {
    pub event_set_hash: String,
    pub event_count: usize,
    pub revision: RevisionProjectionIdentity,
    pub snapshot: DiffSnapshot,
    /// Set when the bound object artifact has a recorded `ArtifactRemoved`
    /// fact: `snapshot` is empty and this carries the removed content hash.
    /// `None` for a present snapshot. Whether the bytes are still stored is
    /// reported by `snapshot_content_state`.
    pub removed_snapshot_content_hash: Option<String>,
    /// The resolved read state of the bound snapshot content (present, suppressed
    /// but still stored, or physically swept). A library-API addition; not
    /// serialized into the `pointbreak.review-revision` document (the diagnostics carry
    /// the public surface).
    pub snapshot_content_state: SnapshotContentState,
    pub filters: RevisionShowFilters,
    pub summary: RevisionProjectionSummary,
    pub current_assessment: CurrentAssessmentView,
    pub observations: Vec<ObservationView>,
    pub input_requests: Vec<InputRequestView>,
    pub assessments: Vec<AssessmentView>,
    pub validation_checks: Vec<ValidationCheckView>,
    pub rows: Vec<RevisionProjectionRow>,
    /// Commit-range lifecycle view (floating/anchored, current and withdrawn
    /// commit/ref associations) derived git-free from the event set. Liveness
    /// (merged/live/orphaned) is layered separately by callers that hold a repo.
    pub commit_range: RevisionCommitRangeView,
    /// Reader-relative readback keyed by event id, covering the capture event and
    /// every narrative member. Attached at the document layer; empty when no
    /// verification policy is set.
    pub member_readbacks: BTreeMap<EventId, MemberReadback>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

impl RevisionShowResult {
    /// Whether the bound object artifact's content was removed (an
    /// `ArtifactRemoved` fact exists for its content hash). When true, `snapshot`
    /// is empty and `removed_snapshot_content_hash` names the removed blob.
    pub fn snapshot_is_removed(&self) -> bool {
        self.removed_snapshot_content_hash.is_some()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionProjectionIdentity {
    pub id: RevisionId,
    pub journal_id: JournalId,
    pub source: RevisionSource,
    pub base: ReviewEndpoint,
    pub target: ReviewEndpoint,
    pub revision_id: RevisionId,
    pub object_id: ObjectId,
    pub object_artifact_content_hash: String,
    /// The capture event's id, so the document layer can key the readback side
    /// table for the revision identity (the capture has no `eventId` of its own).
    pub capture_event_id: EventId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionShowFilters {
    pub revision_id: RevisionId,
    pub track_id: Option<TrackId>,
    pub include_body: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RevisionProjectionSummary {
    pub file_count: usize,
    pub row_count: usize,
    pub narrative_row_count: usize,
    pub snapshot_row_count: usize,
    pub snapshot_remainder_row_count: usize,
    pub observation_count: usize,
    pub input_request_count: usize,
    pub assessment_count: usize,
    pub validation_check_count: usize,
}
