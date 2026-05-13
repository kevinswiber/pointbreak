mod body_artifact;
mod capture;
mod consume;
mod disposition;
pub mod event;
mod event_context;
mod fingerprint;
mod import;
mod intervention;
mod observation;
mod publish;
mod reload;
mod snapshot_artifact;
pub mod state;
mod store_init;
mod verdict;

pub use capture::{CaptureOptions, CaptureResult, capture_worktree_review};
pub use consume::{
    Acknowledgement, CurrentVerdictView, ReviewArtifact, current_verdict_view,
    load_durable_notes_for_repo, load_or_rebuild_session_state, read_acknowledgements,
    read_review_artifacts,
};
pub use disposition::{
    CurrentDispositionStatus, CurrentDispositionView, DispositionAddOptions, DispositionAddResult,
    DispositionOverrideSelector, DispositionRecordStatus, DispositionShowFilters,
    DispositionShowOptions, DispositionShowResult, DispositionTargetSelector, DispositionView,
    record_disposition, show_dispositions,
};
pub use event::{
    EventPayload, EventTarget, EventType, InterventionMode, InterventionReasonCode,
    InterventionRequestedPayload, InterventionResolutionOutcome, InterventionResolvedPayload,
    ReviewDisposition, ReviewDispositionRecordedPayload, ReviewInitializedPayload,
    ReviewObservationRecordedPayload, ReviewUnitCapturedPayload, RevisionPublishedPayload,
    ShoreEvent, SidecarObservedPayload, SidecarSource, SnapshotObservedPayload, Writer, WriterRole,
    WriterTool,
};
pub(crate) use event_context::{
    current_timestamp, reviewer_from_git_config, writer_from_git_config,
};
pub(crate) use fingerprint::worktree_fingerprint_for_files;
pub use fingerprint::{
    ReviewUnitFingerprint, WorktreeFingerprint, capture_worktree_fingerprint,
    compute_review_unit_fingerprint,
};
pub use import::{ImportNotesOptions, ImportNotesResult, import_notes};
pub use intervention::{
    InterventionFetchOptions, InterventionFetchResult, InterventionListFilters,
    InterventionListOptions, InterventionListResult, InterventionRequestOptions,
    InterventionRequestResult, InterventionResolutionView, InterventionResolveOptions,
    InterventionResolveResult, InterventionStatus, InterventionStatusFilter,
    InterventionTargetSelector, InterventionView, fetch_intervention, list_interventions,
    request_intervention, resolve_intervention,
};
pub use observation::{
    ObservationAddOptions, ObservationAddResult, ObservationListFilters, ObservationListOptions,
    ObservationListResult, ObservationStatus, ObservationTargetSelector, ObservationView,
    list_observations, record_observation,
};
pub use publish::{
    PublishOptions, PublishResult, publish_worktree_review, read_events, rebuild_state,
};
pub(crate) use reload::reload_diagnostics_for_document;
pub use reload::{ReloadDiagnostic, ReloadDiagnosticCode, ReloadOutcome, reload_session};
pub use snapshot_artifact::{SnapshotArtifact, read_snapshot_artifact, write_snapshot_artifact};
pub use state::{ProjectionDiagnostic, SessionState};
pub(crate) use store_init::{ShoreStorePaths, prepare_shore_writer, sweep_stale_temp_files};
pub use store_init::{ensure_shore_ignored, shore_dir_for_repo};
pub use verdict::{
    AcknowledgeReviewOptions, AcknowledgeReviewResult, PublishVerdictOptions, PublishVerdictResult,
    acknowledge_review, publish_verdict,
};

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
