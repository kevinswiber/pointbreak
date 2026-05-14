pub mod event;
mod identity;
mod projection;
mod store;
mod workflow;

pub use event::{
    EventPayload, EventTarget, EventType, InterventionMode, InterventionReasonCode,
    InterventionRequestedPayload, InterventionResolutionOutcome, InterventionResolvedPayload,
    ReviewDisposition, ReviewDispositionRecordedPayload, ReviewInitializedPayload,
    ReviewObservationRecordedPayload, ReviewUnitCapturedPayload, ShoreEvent, SidecarSource, Writer,
    WriterRole, WriterTool,
};
pub(crate) use identity::{current_timestamp, reviewer_from_git_config, writer_from_git_config};
pub use projection::{
    ProjectionDiagnostic, SessionState, load_durable_notes_for_repo, load_or_rebuild_session_state,
    read_events, rebuild_state, state,
};
pub(crate) use store::{
    EventStore, EventWriteOutcome, ShoreStorePaths, prepare_shore_writer, sweep_stale_temp_files,
    worktree_fingerprint_for_files,
};
pub use store::{
    ReviewUnitFingerprint, SnapshotArtifact, WorktreeFingerprint, capture_worktree_fingerprint,
    compute_review_unit_fingerprint, ensure_shore_ignored, read_snapshot_artifact,
    shore_dir_for_repo, write_snapshot_artifact,
};
pub(in crate::session) use store::{body_artifact, fingerprint, snapshot_artifact, store_init};
pub(crate) use workflow::reload_diagnostics_for_document;
pub use workflow::{
    AdapterNoteView, CaptureOptions, CaptureResult, CurrentDispositionStatus,
    CurrentDispositionView, DispositionAddOptions, DispositionAddResult,
    DispositionOverrideSelector, DispositionRecordStatus, DispositionShowFilters,
    DispositionShowOptions, DispositionShowResult, DispositionTargetSelector, DispositionView,
    ImportNotesOptions, ImportNotesResult, InterventionFetchOptions, InterventionFetchResult,
    InterventionListFilters, InterventionListOptions, InterventionListResult,
    InterventionRequestOptions, InterventionRequestResult, InterventionResolutionView,
    InterventionResolveOptions, InterventionResolveResult, InterventionStatus,
    InterventionStatusFilter, InterventionTargetSelector, InterventionView, ObservationAddOptions,
    ObservationAddResult, ObservationListFilters, ObservationListOptions, ObservationListResult,
    ObservationStatus, ObservationTargetSelector, ObservationView, ReloadDiagnostic,
    ReloadDiagnosticCode, ReloadOutcome, ReviewHistoryEntry, ReviewHistoryFilters,
    ReviewHistoryOptions, ReviewHistoryResult, ReviewHistorySummary, ReviewUnitProjectionIdentity,
    ReviewUnitProjectionRow, ReviewUnitProjectionSummary, ReviewUnitShowFilters,
    ReviewUnitShowOptions, ReviewUnitShowResult, SnapshotOrder, capture_worktree_review,
    fetch_intervention, import_notes, list_interventions, list_observations, record_disposition,
    record_observation, reload_session, request_intervention, resolve_intervention, review_history,
    show_dispositions, show_review_unit,
};
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
