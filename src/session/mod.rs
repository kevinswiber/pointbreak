mod body_artifact;
mod capture;
mod consume;
pub mod event;
mod event_context;
mod fingerprint;
mod import;
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
pub use event::{
    EventPayload, EventTarget, EventType, ReviewInitializedPayload, ReviewUnitCapturedPayload,
    RevisionPublishedPayload, ShoreEvent, SidecarObservedPayload, SidecarSource,
    SnapshotObservedPayload, Writer, WriterRole, WriterTool,
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
pub use publish::{
    PublishOptions, PublishResult, publish_worktree_review, read_events, rebuild_state,
};
pub(crate) use reload::reload_diagnostics_for_document;
pub use reload::{ReloadDiagnostic, ReloadDiagnosticCode, ReloadOutcome, reload_session};
pub use snapshot_artifact::{SnapshotArtifact, read_snapshot_artifact, write_snapshot_artifact};
pub use state::{ProjectionDiagnostic, SessionState};
pub(crate) use store_init::ensure_store_dirs;
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
