use std::path::{Path, PathBuf};

use super::{AdapterNoteView, ReviewUnitProjectionRow};
use crate::model::{
    ActorId, DiffSnapshot, ReviewEndpoint, ReviewUnitId, ReviewUnitLineageId, ReviewUnitSource,
    RevisionId, SessionId, SnapshotId, TrackId,
};
use crate::session::assessment::{AssessmentView, CurrentAssessmentView};
use crate::session::input_request::InputRequestView;
use crate::session::observation::ObservationView;
use crate::session::state::ProjectionDiagnostic;
use crate::session::workflow::ValidationCheckView;
use crate::session::{DelegationMap, PrincipalResolution, principal_resolution_for_writer};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewUnitShowOptions {
    pub(super) repo: PathBuf,
    pub(super) review_unit_id: Option<ReviewUnitId>,
    pub(super) lineage_id: Option<ReviewUnitLineageId>,
    pub(super) track: Option<String>,
    pub(super) include_body: bool,
    pub(super) delegation_map: Option<DelegationMap>,
}

impl ReviewUnitShowOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_unit_id: None,
            lineage_id: None,
            track: None,
            include_body: false,
            delegation_map: None,
        }
    }

    pub fn with_review_unit_id(mut self, review_unit_id: ReviewUnitId) -> Self {
        self.review_unit_id = Some(review_unit_id);
        self
    }

    pub fn with_lineage_id(mut self, lineage_id: ReviewUnitLineageId) -> Self {
        self.lineage_id = Some(lineage_id);
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

    /// Supply the reader-side delegation map. With it set, `show` emits
    /// `principal_unresolvable` / `principal_ambiguous` diagnostics for
    /// agent-written events whose principal does not resolve; without it, no
    /// principal diagnostics are emitted (the zero-setup floor stays silent).
    pub fn with_delegation_map(mut self, delegation_map: DelegationMap) -> Self {
        self.delegation_map = Some(delegation_map);
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewUnitShowResult {
    pub event_set_hash: String,
    pub event_count: usize,
    pub review_unit: ReviewUnitProjectionIdentity,
    pub snapshot: DiffSnapshot,
    pub filters: ReviewUnitShowFilters,
    pub summary: ReviewUnitProjectionSummary,
    pub current_assessment: CurrentAssessmentView,
    pub observations: Vec<ObservationView>,
    pub input_requests: Vec<InputRequestView>,
    pub assessments: Vec<AssessmentView>,
    pub validation_checks: Vec<ValidationCheckView>,
    pub adapter_notes: Vec<AdapterNoteView>,
    pub rows: Vec<ReviewUnitProjectionRow>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewUnitProjectionIdentity {
    pub id: ReviewUnitId,
    pub session_id: SessionId,
    pub source: ReviewUnitSource,
    pub base: ReviewEndpoint,
    pub target: ReviewEndpoint,
    pub revision_id: RevisionId,
    pub snapshot_id: SnapshotId,
    pub snapshot_artifact_content_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewUnitShowFilters {
    pub review_unit_id: ReviewUnitId,
    pub track_id: Option<TrackId>,
    pub include_body: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReviewUnitProjectionSummary {
    pub file_count: usize,
    pub row_count: usize,
    pub narrative_row_count: usize,
    pub snapshot_row_count: usize,
    pub snapshot_remainder_row_count: usize,
    pub observation_count: usize,
    pub input_request_count: usize,
    pub assessment_count: usize,
    pub validation_check_count: usize,
    pub adapter_note_count: usize,
}
