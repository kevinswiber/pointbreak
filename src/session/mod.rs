pub mod adapter;
pub mod event;
mod identity;
mod projection;
mod store;
mod workflow;

pub(crate) use identity::{
    current_timestamp, is_valid_actor_id, reviewer_from_options, writer_from_git_config,
    writer_from_options,
};
pub(crate) use projection::state;
pub use projection::{
    ProjectionDiagnostic, SessionState, load_durable_notes_for_repo, read_events, rebuild_state,
};
#[cfg(test)]
pub(crate) use store::compute_review_unit_fingerprint;
pub(crate) use store::{
    EventStore, EventWriteOutcome, ReviewUnitFingerprint, ShoreStorePaths, prepare_shore_writer,
    sweep_stale_temp_files, worktree_fingerprint_for_files,
};
pub use store::{
    SnapshotArtifact, capture_worktree_fingerprint, ensure_shore_storage_excluded,
    read_snapshot_artifact, shore_dir_for_repo,
};
pub(in crate::session) use store::{body_artifact, fingerprint, snapshot_artifact, store_init};
pub(crate) use workflow::reload_diagnostics_for_document;
pub use workflow::{
    AdapterNoteView, ArtifactKind, ArtifactRef, AssessmentAddOptions, AssessmentAddResult,
    AssessmentRecordStatus, AssessmentShowFilters, AssessmentShowOptions, AssessmentShowResult,
    AssessmentTargetSelector, AssessmentView, CaptureOptions, CaptureResult,
    CurrentAssessmentStatus, CurrentAssessmentView, ImportArtifactOptions, ImportArtifactOutcome,
    ImportArtifactResult, ImportEventOptions, ImportNotesOptions, ImportNotesResult,
    IngestEventsOptions, IngestEventsResult, InputRequestFetchOptions, InputRequestFetchResult,
    InputRequestListOptions, InputRequestListResult, InputRequestOpenOptions,
    InputRequestOpenResult, InputRequestRespondOptions, InputRequestRespondResult,
    InputRequestResponseView, InputRequestStatus, InputRequestStatusFilter,
    InputRequestTargetSelector, InputRequestView, ObservationAddOptions, ObservationAddResult,
    ObservationListOptions, ObservationListResult, ObservationStatus, ObservationTargetSelector,
    ObservationView, ReloadDiagnostic, ReloadDiagnosticCode, ReloadOutcome, ReviewHistoryEntry,
    ReviewHistoryFilters, ReviewHistoryOptions, ReviewHistoryResult, ReviewUnitListEntry,
    ReviewUnitListOptions, ReviewUnitListResult, ReviewUnitProjectionIdentity,
    ReviewUnitProjectionRow, ReviewUnitProjectionSummary, ReviewUnitShowFilters,
    ReviewUnitShowOptions, ReviewUnitShowResult, SnapshotOrder, StoreLinkOptions, StoreLinkResult,
    StoreStatusArtifactInventory, StoreStatusInventory, StoreStatusOptions, StoreStatusResult,
    StoreStatusReviewUnitSnapshot, StoreStatusSensitivity, StoreStatusSensitivityFinding,
    capture_worktree_review, export_artifact, fetch_input_request, import_artifact, import_event,
    import_notes, ingest_events, link_clone_local_store, list_input_requests, list_observations,
    list_review_units, open_input_request, record_assessment, record_observation,
    referenced_artifacts, reload_session, respond_input_request, review_history, show_assessments,
    show_review_unit, store_status,
};
pub(in crate::session) use workflow::{assessment, input_request, observation};

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
