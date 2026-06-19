pub mod adapter;
pub mod event;
mod identity;
mod projection;
mod signing;
mod store;
mod workflow;

pub use event::{
    IngestProvenance, IngestVia, event_signature_pre_authentication_encoding, event_to_be_signed,
};
pub use identity::{
    ACTOR_ATTRIBUTES_LOCAL_REL_PATH, ACTOR_ATTRIBUTES_REL_PATH, ActorAttributes,
    ActorAttributesMap, ActorAttributesStageOutcome, ActorAttributesWriteRecord,
    DELEGATES_LOCAL_REL_PATH, DELEGATES_REL_PATH, DelegationMap, DelegationRecord,
    DelegationStageOutcome, DelegationWriteRecord, PrincipalResolution, PrincipalSource,
    PrincipalStatus, PrincipalView, UnresolvedReason, actor_attributes_from_value,
    delegation_map_from_value, format_rfc3339_utc_millis, is_agent_actor_id, now_rfc3339_utc,
    principal_display_label, principal_resolution_for_writer, principal_view_for,
    resolve_writer_actor_id, stage_actor_attributes, stage_delegation,
};
pub(crate) use identity::{
    current_timestamp, is_valid_actor_id, writer_from_git_config, writer_from_options,
};
pub use projection::cosignature::{
    EndorsementClassification, EndorsementReadback, EndorserAttributesView,
};
pub(crate) use projection::state;
pub use projection::{
    CommitEdgeSource, CurrentCommitAssociation, CurrentRefAssociation, ProjectionDiagnostic,
    ReviewUnitCommitRangeProjection, ReviewUnitCommitRangeView, SessionState,
    WithdrawnCommitAssociation, WithdrawnRefAssociation, load_durable_notes_for_repo, read_events,
    rebuild_state,
};
pub use signing::{
    ALLOWED_SIGNERS_REL_PATH, ArtifactAvailability, BestEffortSkipSink,
    COSIGNATURE_BINDING_MISMATCH_CODE, COSIGNATURE_INVALID_CODE, COSIGNATURE_TARGET_PENDING_CODE,
    COSIGNATURE_UNTRUSTED_SIGNER_CODE, CosignatureGateDecision, CosignatureVerification,
    EnrollmentDiff, EventSigningOptions, EventVerificationPolicy, EventVerificationView,
    IngestEventVerification, PrincipalPolicy, TrustSet, enroll_signer, event_signature_trust_set,
    gate_cosignature_for_store, principal_sufficient, stage_enrollment, trust_set_to_value,
    verification_view, verify_cosignature, verify_event_signature,
};
pub(crate) use signing::{sign_event_if_requested, verify_events_for_ingest};
#[cfg(test)]
pub(crate) use store::compute_review_unit_fingerprint;
pub(crate) use store::{
    EventStore, EventWriteOutcome, ReviewUnitFingerprint, ShoreStorePaths, sweep_stale_temp_files,
    worktree_fingerprint_for_files,
};
pub use store::{
    SnapshotArtifact, capture_worktree_fingerprint, ensure_local_actor_attributes_excluded,
    ensure_local_delegates_excluded, ensure_shore_storage_excluded, read_snapshot_artifact,
    store_dir_for_repo,
};
pub(in crate::session) use store::{body_artifact, fingerprint, snapshot_artifact, store_init};
pub(crate) use workflow::reload_diagnostics_for_document;
pub use workflow::{
    AdapterNoteView, ArtifactKind, ArtifactRef, AssessmentAddOptions, AssessmentAddResult,
    AssessmentRecordStatus, AssessmentShowFilters, AssessmentShowOptions, AssessmentShowResult,
    AssessmentTargetSelector, AssessmentView, CaptureOptions, CaptureResult, CommitRangeSpec,
    CurrentAssessmentStatus, CurrentAssessmentView, EventSignatureRecordOptions,
    EventSignatureRecordResult, ImportArtifactOptions, ImportArtifactOutcome, ImportArtifactResult,
    ImportEventOptions, ImportNotesOptions, ImportNotesResult, IngestEventsOptions,
    IngestEventsResult, InputRequestFetchOptions, InputRequestFetchResult, InputRequestListOptions,
    InputRequestListResult, InputRequestOpenOptions, InputRequestOpenResult,
    InputRequestRespondOptions, InputRequestRespondResult, InputRequestResponseView,
    InputRequestStatus, InputRequestStatusFilter, InputRequestTargetSelector, InputRequestView,
    LineageAttachOptions, LineageAttachResult, LineageListEntry, LineageListOptions,
    LineageListResult, LineageRoundView, LineageShowOptions, LineageShowResult, MemberReadback,
    MigrateStoreOptions, ObservationAddOptions, ObservationAddResult, ObservationListOptions,
    ObservationListResult, ObservationStatus, ObservationTargetSelector, ObservationView,
    ReloadDiagnostic, ReloadDiagnosticCode, ReloadOutcome, ReviewHistoryEntry,
    ReviewHistoryFilters, ReviewHistoryOptions, ReviewHistoryResult, ReviewUnitListEntry,
    ReviewUnitListOptions, ReviewUnitListResult, ReviewUnitProjectionIdentity,
    ReviewUnitProjectionRow, ReviewUnitProjectionSummary, ReviewUnitShowFilters,
    ReviewUnitShowOptions, ReviewUnitShowResult, SnapshotOrder, StoreLinkOptions, StoreLinkResult,
    StoreMigrateResult, StoreStatusArtifactInventory, StoreStatusInventory, StoreStatusOptions,
    StoreStatusResult, StoreStatusReviewUnitSnapshot, StoreStatusSensitivity,
    StoreStatusSensitivityFinding, ValidationAddOptions, ValidationAddResult,
    ValidationCheckProjectionOptions, ValidationCheckView, ValidationListFilters,
    ValidationListOptions, ValidationListResult, attach_review_unit_to_lineage, capture_review,
    capture_worktree_review, export_artifact, fetch_input_request, import_artifact, import_event,
    import_notes, ingest_events, link_clone_local_store, list_input_requests, list_lineages,
    list_observations, list_review_units, list_validation_checks, migrate_store,
    open_input_request, project_validation_checks, record_assessment, record_event_signature,
    record_observation, record_validation_check, referenced_artifacts, reload_session,
    respond_input_request, review_history, show_assessments, show_lineage, show_review_unit,
    store_status,
};
pub(in crate::session) use workflow::{assessment, input_request, observation};

pub use crate::crypto::EventVerificationStatus;

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
