pub mod adapter;
pub mod event;
mod identity;
mod projection;
mod store;
mod workflow;

pub(crate) use identity::{current_timestamp, reviewer_from_git_config, writer_from_git_config};
pub(crate) use projection::state;
pub use projection::{
    ProjectionDiagnostic, SessionState, load_durable_notes_for_repo, read_events, rebuild_state,
};
pub(crate) use store::{
    EventStore, EventWriteOutcome, ReviewUnitFingerprint, ShoreStorePaths, prepare_shore_writer,
    sweep_stale_temp_files, worktree_fingerprint_for_files,
};
pub(in crate::session) use store::{body_artifact, fingerprint, snapshot_artifact, store_init};
pub use store::{capture_worktree_fingerprint, ensure_shore_ignored, shore_dir_for_repo};
#[cfg(test)]
pub(crate) use store::{compute_review_unit_fingerprint, read_snapshot_artifact};
pub use workflow::{
    AdapterNoteView, AssessmentAddOptions, AssessmentAddResult, AssessmentRecordStatus,
    AssessmentShowFilters, AssessmentShowOptions, AssessmentShowResult, AssessmentView,
    CaptureOptions, CaptureResult, CurrentAssessmentStatus, CurrentAssessmentView,
    CurrentDispositionStatus, CurrentDispositionView, DispositionAddOptions, DispositionAddResult,
    DispositionShowFilters, DispositionShowOptions, DispositionShowResult,
    DispositionTargetSelector, DispositionView, ImportNotesOptions, ImportNotesResult,
    InterventionFetchOptions, InterventionFetchResult, InterventionListOptions,
    InterventionListResult, InterventionRequestOptions, InterventionRequestResult,
    InterventionResolutionView, InterventionResolveOptions, InterventionResolveResult,
    InterventionStatusFilter, InterventionTargetSelector, InterventionView, ObservationAddOptions,
    ObservationAddResult, ObservationListOptions, ObservationListResult, ObservationStatus,
    ObservationTargetSelector, ObservationView, ReloadDiagnosticCode, ReviewHistoryEntry,
    ReviewHistoryFilters, ReviewHistoryOptions, ReviewHistoryResult, ReviewUnitListEntry,
    ReviewUnitListOptions, ReviewUnitListResult, ReviewUnitProjectionIdentity,
    ReviewUnitProjectionRow, ReviewUnitProjectionSummary, ReviewUnitShowFilters,
    ReviewUnitShowOptions, ReviewUnitShowResult, SnapshotOrder, StateChangeOptions,
    capture_worktree_review, fetch_intervention, import_notes, list_interventions,
    list_observations, list_review_units, record_assessment, record_deferred_state_change,
    record_disposition, record_observation, record_overridden_state_change,
    record_split_out_state_change, record_superseded_state_change, reload_session,
    request_intervention, resolve_intervention, review_history, show_assessments,
    show_dispositions, show_review_unit,
};
#[cfg(test)]
pub(crate) use workflow::{InterventionStatus, ReloadOutcome};
pub(crate) use workflow::{ReloadDiagnostic, reload_diagnostics_for_document};
pub(in crate::session) use workflow::{disposition, intervention, observation};

#[cfg(test)]
mod tests {
    #[test]
    fn reload_session_is_reachable_from_session_namespace() {
        fn _smoke() -> crate::error::Result<crate::session::ReloadOutcome> {
            let repo = std::path::Path::new(".");
            crate::session::reload_session(repo, || crate::dump::DumpDocument::from_repo(repo))
        }
    }
}
