mod body_artifact;
mod consume;
pub mod event;
mod fingerprint;
mod import;
mod publish;
mod reload;
pub mod state;
mod store_init;
mod verdict;

pub use consume::{
    Acknowledgement, CurrentVerdictView, ReviewArtifact, current_verdict_view,
    load_durable_notes_for_repo, load_or_rebuild_session_state, read_acknowledgements,
    read_review_artifacts,
};
pub use event::{
    EventPayload, EventTarget, EventType, ReviewInitializedPayload, RevisionPublishedPayload,
    ShoreEvent, SidecarObservedPayload, SidecarSource, SnapshotObservedPayload, Writer, WriterRole,
    WriterTool,
};
pub(crate) use fingerprint::worktree_fingerprint_for_files;
pub use fingerprint::{WorktreeFingerprint, capture_worktree_fingerprint};
pub use import::{ImportNotesOptions, ImportNotesResult, import_notes};
pub use publish::{
    PublishOptions, PublishResult, publish_worktree_review, read_events, rebuild_state,
};
pub(crate) use reload::reload_diagnostics_for_document;
pub use reload::{ReloadDiagnostic, ReloadDiagnosticCode, ReloadOutcome, reload_session};
pub use state::{ProjectionDiagnostic, SessionState};
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
