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
    ActorAttributes, ActorAttributesMap, ActorAttributesStageOutcome, ActorAttributesWriteRecord,
    DelegationMap, DelegationRecord, DelegationStageOutcome, DelegationWriteRecord,
    PrincipalResolution, PrincipalSource, PrincipalStatus, PrincipalView, UnresolvedReason,
    actor_attributes_from_value, compare_event_instants, delegation_map_from_value,
    format_rfc3339_utc_millis, is_agent_actor_id, is_valid_actor_id, now_rfc3339_utc,
    parse_event_instant, principal_display_label, principal_resolution_for_writer,
    principal_view_for, resolve_writer_actor_id, stage_actor_attributes, stage_delegation,
};
pub(crate) use identity::{current_timestamp, writer_from_options};
pub use projection::cosignature::{
    EndorsementClassification, EndorsementReadback, EndorserAttributesView,
};
pub(crate) use projection::state;
pub use projection::{
    ArtifactRemovalProjection, BodyContentState, CommitEdgeSource, CommitOidGroupingProjection,
    CurrentCommitAssociation, CurrentRefAssociation, EngagementGrouping, EngagementLifecycle,
    EngagementView, LivenessScope, LivenessToken, ProjectionDiagnostic, RemovalClaim,
    RemovalOperativeStatus, RevisionClassificationFacet, RevisionCommitRangeProjection,
    RevisionCommitRangeView, RevisionsByBase, SessionState, StoreIdIndex, SupersessionView,
    WithdrawnCommitAssociation, WithdrawnRefAssociation, read_events, read_events_for_display,
    rebuild_state, revision_supersession_classification, store_id_index,
};
pub use sensitivity_vocabulary::{SensitivityKind, SensitivityPolicyOutcome, SensitivitySeverity};
pub use signing::{
    ArtifactAvailability, BestEffortSkipSink, COSIGNATURE_BINDING_MISMATCH_CODE,
    COSIGNATURE_INVALID_CODE, COSIGNATURE_TARGET_PENDING_CODE, COSIGNATURE_UNTRUSTED_SIGNER_CODE,
    CosignatureGateDecision, CosignatureVerification, EnrollmentDiff, EventSigningOptions,
    EventVerificationPolicy, EventVerificationView, IngestEventVerification, PrincipalPolicy,
    RemovalPolicy, TrustSet, allowed_signers_path_for_repo, enroll_signer,
    event_signature_trust_set, gate_cosignature_for_store, principal_sufficient, stage_enrollment,
    trust_set_to_value, verification_view, verify_cosignature, verify_event_signature,
};
pub(crate) use signing::{sign_event_if_requested, verify_events_for_ingest};
#[cfg(test)]
pub(crate) use store::compute_revision_fingerprint;
pub(crate) use store::{
    EventStore, RepositoryPaths, RevisionFingerprint, SkippedEvent, sweep_stale_temp_files,
    worktree_fingerprint_for_files,
};
pub use store::{
    EventWriteOutcome, ObjectArtifact, StoreMode, StoreModeOutcome, StoreModeSource, StorePaths,
    capture_worktree_fingerprint, ensure_pointbreak_gitignore, event_log_head_marker,
    family_link_advisory, read_bound_object_artifact, read_object_artifact,
    resolve_store_mode_for_repo, set_store_mode_for_repo, store_dir_for_repo, store_paths_for_repo,
};
pub(in crate::session) use store::{body_artifact, fingerprint, object_artifact, store_init};
pub use workflow::{
    ArtifactKind, ArtifactRef, AssessmentAddOptions, AssessmentAddResult, AssessmentRecordStatus,
    AssessmentShowFilters, AssessmentShowOptions, AssessmentShowResult, AssessmentTargetSelector,
    AssessmentView, AssociateCommitOptions, AssociateCommitResult, AssociateRefOptions,
    AssociateRefResult, AssociationAxis, AttentionAssessmentRecord, AttentionDetail,
    AttentionFreshness, AttentionFreshnessState, AttentionItem, AttentionListOptions,
    AttentionListResult, AttentionProjection, AttentionTier, BaseEntry, BaseHistoryProjection,
    BaseProjectionConfig, CaptureDiffstat, CaptureOptions, CaptureResult, CommitGraphCondition,
    CommitLiveness, CommitRangeSpec, CompactOptions, CompactResult, CurrentAssessmentStatus,
    CurrentAssessmentView, DistinctValues, EVENT_QUERY_FIELDS, EventRecordExtras,
    EventSignatureRecordOptions, EventSignatureRecordResult, HistoryCursor, HistoryOrder,
    HistoryPage, HistoryQuery, ImportArtifactOptions, ImportArtifactOutcome, ImportArtifactResult,
    ImportEventOptions, IngestEventsOptions, IngestEventsResult, InputRequestFetchOptions,
    InputRequestFetchResult, InputRequestListOptions, InputRequestListResult,
    InputRequestOpenOptions, InputRequestOpenResult, InputRequestRespondOptions,
    InputRequestRespondResult, InputRequestResponseView, InputRequestStatus,
    InputRequestStatusFilter, InputRequestTargetSelector, InputRequestView, KNOWN_QUERY_KEYS,
    ListAssociationsOptions, ListAssociationsResult, LivenessEnrichment, MemberReadback,
    MigrateToCommonDirOptions, MigrateToCommonDirResult, ObservationAddOptions,
    ObservationAddResult, ObservationListOptions, ObservationListResult, ObservationStatus,
    ObservationTargetSelector, ObservationView, ParsedQuery, QueriedHistory, QueryClause,
    QueryDiagnostic, QueryDiagnosticCode, QuerySurface, RANGE_ANCHOR_FIELD, REF_REWRITTEN_CODE,
    REVISION_ATTENTION_VALUES, REVISION_QUERY_FIELDS, RefContinuity, RefContinuityReport,
    RefContinuityView, RefFilterMode, RemoveOptions, RemoveResult, RemoveSelector, RemovedContent,
    Retention, ReviewHistoryEntry, ReviewHistoryFilters, ReviewHistoryOptions, ReviewHistoryResult,
    ReviewHistorySummary, RevisionListEntry, RevisionListOptions, RevisionListResult,
    RevisionOverview, RevisionOverviewsOptions, RevisionProjectionIdentity, RevisionProjectionRow,
    RevisionProjectionSummary, RevisionRecordInputs, RevisionSearchRecord, RevisionShowFilters,
    RevisionShowOptions, RevisionShowResult, RootCommitSpec, SearchRecord, SkippedRemoval,
    SnapshotContentState, SnapshotOrder, SnapshotSummaryCache, SnapshotSummaryCounts, StagedSpec,
    StoreFamily, StoreForgetOptions, StoreForgetResult, StoreIdentity, StoreIdentityOptions,
    StoreLinkOptions, StoreLinkPreview, StoreLinkResult, StoreListEntry, StoreListResult,
    StorePlacement, StoreSensitivityPathGroup, StoreStatusArtifactInventory, StoreStatusInventory,
    StoreStatusOptions, StoreStatusResult, StoreStatusRevisionObject, StoreStatusSensitivity,
    StoreStatusSensitivityExcludeGlob, StoreStatusSensitivityFinding, StoreUnlinkOptions,
    StoreUnlinkResult, SweepOutcome, SweptBlob, UnreachableVisibility, UnstagedSpec,
    ValidationAddOptions, ValidationAddResult, ValidationCheckDisposition, ValidationCheckView,
    ValidationContinuitySummary, ValidationContinuityView, ValidationListFilters,
    ValidationListOptions, ValidationListResult, WithdrawCommitOptions, WithdrawCommitResult,
    WithdrawRefOptions, WithdrawRefResult, WorktreeSpec, apply_history_query, associate_commit,
    associate_ref, build_haystack, build_revision_search_record, capture_review,
    capture_worktree_review, classify_validation_continuity, commit_graph_stamp, compact_store,
    count_new_since, current_assessment_includes_follow_up, default_history_page_projection,
    diagnose_ref_continuity, diffstat_from_files, effective_integration_ref, enrich_liveness,
    explain_store_sensitivity, export_artifact, fetch_input_request, forget_family_store,
    history_base_projection, import_artifact, import_event, ingest_events, link_store_to_family,
    list_associations, list_attention, list_family_stores, list_input_requests, list_observations,
    list_revisions, list_units_for_ref, list_validation_checks, matches_query,
    migrate_store_to_common_dir, open_input_request, parse_search_query, parse_search_query_for,
    preview_link_to_family, record_assessment, record_event_signature, record_observation,
    record_validation_check, referenced_artifacts, remove_content, resolve_default_integration_ref,
    respond_input_request, review_history, show_assessments, show_revision,
    show_revision_overviews, stale_review_fact_count, store_identity, store_status,
    unlink_store_from_family, withdraw_commit, withdraw_ref,
};
pub(in crate::session) use workflow::{assessment, input_request, observation};

pub use crate::crypto::EventVerificationStatus;
