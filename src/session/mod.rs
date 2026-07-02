pub mod adapter;
pub mod event;
mod identity;
mod projection;
mod sensitivity_vocabulary;
mod signing;
mod store;
mod workflow;

pub use event::{
    BodyContentType, IngestProvenance, IngestVia, event_signature_pre_authentication_encoding,
    event_to_be_signed,
};
pub use identity::{
    ACTOR_ATTRIBUTES_LOCAL_REL_PATH, ACTOR_ATTRIBUTES_REL_PATH, ActorAttributes,
    ActorAttributesMap, ActorAttributesStageOutcome, ActorAttributesWriteRecord,
    DELEGATES_LOCAL_REL_PATH, DELEGATES_REL_PATH, DelegationMap, DelegationRecord,
    DelegationStageOutcome, DelegationWriteRecord, PrincipalResolution, PrincipalSource,
    PrincipalStatus, PrincipalView, UnresolvedReason, actor_attributes_from_value,
    delegation_map_from_value, format_rfc3339_utc_millis, is_agent_actor_id, is_valid_actor_id,
    now_rfc3339_utc, principal_display_label, principal_resolution_for_writer, principal_view_for,
    resolve_writer_actor_id, stage_actor_attributes, stage_delegation,
};
pub(crate) use identity::{current_timestamp, writer_from_git_config, writer_from_options};
pub use projection::cosignature::{
    EndorsementClassification, EndorsementReadback, EndorserAttributesView,
};
pub(crate) use projection::state;
pub use projection::{
    ArtifactRemovalProjection, CommitEdgeSource, CommitOidGroupingProjection,
    CurrentCommitAssociation, CurrentRefAssociation, EngagementGrouping, EngagementLifecycle,
    EngagementView, LivenessScope, LivenessToken, ProjectionDiagnostic, RemovalClaim,
    RemovalOperativeStatus, RevisionCommitRangeProjection, RevisionCommitRangeView,
    RevisionsByBase, SessionState, SupersessionView, WithdrawnCommitAssociation,
    WithdrawnRefAssociation, load_durable_notes_for_repo, read_events, read_events_for_display,
    rebuild_state,
};
pub use sensitivity_vocabulary::{SensitivityKind, SensitivityPolicyOutcome, SensitivitySeverity};
pub use signing::{
    ALLOWED_SIGNERS_REL_PATH, ArtifactAvailability, BestEffortSkipSink,
    COSIGNATURE_BINDING_MISMATCH_CODE, COSIGNATURE_INVALID_CODE, COSIGNATURE_TARGET_PENDING_CODE,
    COSIGNATURE_UNTRUSTED_SIGNER_CODE, CosignatureGateDecision, CosignatureVerification,
    EnrollmentDiff, EventSigningOptions, EventVerificationPolicy, EventVerificationView,
    IngestEventVerification, PrincipalPolicy, RemovalPolicy, TrustSet, enroll_signer,
    event_signature_trust_set, gate_cosignature_for_store, principal_sufficient, stage_enrollment,
    trust_set_to_value, verification_view, verify_cosignature, verify_event_signature,
};
pub(crate) use signing::{sign_event_if_requested, verify_events_for_ingest};
#[cfg(test)]
pub(crate) use store::compute_revision_fingerprint;
pub(crate) use store::{
    EventStore, RevisionFingerprint, ShoreStorePaths, SkippedEvent, sweep_stale_temp_files,
    worktree_fingerprint_for_files,
};
pub use store::{
    EventWriteOutcome, ObjectArtifact, StoreMode, StoreModeOutcome, StoreModeSource,
    capture_worktree_fingerprint, ensure_shore_gitignore, event_log_head_marker,
    read_bound_object_artifact, read_object_artifact, resolve_store_mode_for_repo,
    set_store_mode_for_repo, store_dir_for_repo,
};
pub(in crate::session) use store::{body_artifact, fingerprint, object_artifact, store_init};
pub(crate) use workflow::reload_diagnostics_for_document;
pub use workflow::{
    AdapterNoteView, ArtifactKind, ArtifactRef, AssessmentAddOptions, AssessmentAddResult,
    AssessmentRecordStatus, AssessmentShowFilters, AssessmentShowOptions, AssessmentShowResult,
    AssessmentTargetSelector, AssessmentView, AssociateCommitOptions, AssociateCommitResult,
    AssociateRefOptions, AssociateRefResult, AssociationAxis, BaseEntry, BaseHistoryProjection,
    BaseProjectionConfig, CaptureOptions, CaptureResult, CommitGraphCondition, CommitLiveness,
    CommitRangeSpec, CompactOptions, CompactResult, CurrentAssessmentStatus, CurrentAssessmentView,
    EventSignatureRecordOptions, EventSignatureRecordResult, HistoryCursor, HistoryOrder,
    HistoryPage, HistoryQuery, HistoryWindow, ImportArtifactOptions, ImportArtifactOutcome,
    ImportArtifactResult, ImportEventOptions, ImportNotesOptions, ImportNotesResult,
    IngestEventsOptions, IngestEventsResult, InputRequestFetchOptions, InputRequestFetchResult,
    InputRequestListOptions, InputRequestListResult, InputRequestOpenOptions,
    InputRequestOpenResult, InputRequestRespondOptions, InputRequestRespondResult,
    InputRequestResponseView, InputRequestStatus, InputRequestStatusFilter,
    InputRequestTargetSelector, InputRequestView, ListAssociationsOptions, ListAssociationsResult,
    LivenessEnrichment, MemberReadback, MigrateStoreOptions, MigrateToCommonDirOptions,
    MigrateToCommonDirResult, ObservationAddOptions, ObservationAddResult, ObservationListOptions,
    ObservationListResult, ObservationStatus, ObservationTargetSelector, ObservationView,
    OrphanReason, OrphanVisibility, QueriedHistory, QueryClause, RefFilterMode, ReloadDiagnostic,
    ReloadDiagnosticCode, ReloadOutcome, RemoveOptions, RemoveResult, RemoveSelector,
    RemovedContent, ReviewHistoryEntry, ReviewHistoryFilters, ReviewHistoryOptions,
    ReviewHistoryResult, RevisionListEntry, RevisionListOptions, RevisionListResult,
    RevisionOverview, RevisionOverviewsOptions, RevisionProjectionIdentity, RevisionProjectionRow,
    RevisionProjectionSummary, RevisionShowFilters, RevisionShowOptions, RevisionShowResult,
    SearchRecord, SkippedRemoval, SnapshotContentState, SnapshotOrder, StoreMigrateResult,
    StoreStatusArtifactInventory, StoreStatusInventory, StoreStatusOptions, StoreStatusResult,
    StoreStatusRevisionObject, StoreStatusSensitivity, StoreStatusSensitivityFinding, SweepOutcome,
    SweptBlob, ValidationAddOptions, ValidationAddResult, ValidationCheckView,
    ValidationListFilters, ValidationListOptions, ValidationListResult, WithdrawCommitOptions,
    WithdrawCommitResult, WithdrawRefOptions, WithdrawRefResult, apply_history_query,
    associate_commit, associate_ref, build_haystack, capture_review, capture_worktree_review,
    compact_store, enrich_liveness, export_artifact, fetch_input_request, history_base_projection,
    import_artifact, import_event, import_notes, ingest_events, list_associations,
    list_input_requests, list_observations, list_revisions, list_units_for_ref,
    list_validation_checks, matches_query, migrate_store, migrate_store_to_common_dir,
    open_input_request, parse_search_query, record_assessment, record_event_signature,
    record_observation, record_validation_check, referenced_artifacts, reload_session,
    remove_content, respond_input_request, review_history, show_assessments, show_revision,
    show_revision_overviews, store_status, withdraw_commit, withdraw_ref,
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
