mod artifact_removal;
mod artifact_transfer;
pub(in crate::session) mod assessment;
mod association;
mod capture;
mod commit_range_liveness;
mod event_signature;
mod history;
mod ingest;
mod revision_list;
mod revision_projection;
mod store_family;
mod store_identity;
mod store_link;
mod store_migrate_common_dir;
mod store_status;
pub(in crate::session) mod util;
pub(in crate::session) mod validation;

pub(in crate::session) mod input_request;
pub(in crate::session) mod observation;

pub use artifact_removal::{
    CompactOptions, CompactResult, RemoveOptions, RemoveResult, RemoveSelector, RemovedContent,
    SkippedRemoval, SweepOutcome, SweptBlob, compact_store, remove_content,
};
pub use artifact_transfer::{
    ArtifactKind, ArtifactRef, ImportArtifactOptions, ImportArtifactOutcome, ImportArtifactResult,
    export_artifact, import_artifact, referenced_artifacts,
};
pub use assessment::{
    AssessmentAddOptions, AssessmentAddResult, AssessmentRecordStatus, AssessmentShowFilters,
    AssessmentShowOptions, AssessmentShowResult, AssessmentTargetSelector, AssessmentView,
    CurrentAssessmentStatus, CurrentAssessmentView, record_assessment, show_assessments,
};
pub use association::{
    AssociateCommitOptions, AssociateCommitResult, AssociateRefOptions, AssociateRefResult,
    AssociationAxis, ListAssociationsOptions, ListAssociationsResult, WithdrawCommitOptions,
    WithdrawCommitResult, WithdrawRefOptions, WithdrawRefResult, associate_commit, associate_ref,
    list_associations, withdraw_commit, withdraw_ref,
};
pub use capture::{
    CaptureDiffstat, CaptureOptions, CaptureResult, CommitRangeSpec, RootCommitSpec, StagedSpec,
    UnstagedSpec, WorktreeSpec, capture_review, capture_worktree_review, diffstat_from_files,
};
pub use commit_range_liveness::{
    CommitGraphCondition, CommitLiveness, LivenessEnrichment, OrphanReason, enrich_liveness,
};
pub use event_signature::{
    EventSignatureRecordOptions, EventSignatureRecordResult, record_event_signature,
};
pub use history::{
    BaseEntry, BaseHistoryProjection, BaseProjectionConfig, HistoryCursor, HistoryOrder,
    HistoryPage, HistoryQuery, HistoryWindow, QueriedHistory, QueryClause, ReviewHistoryEntry,
    ReviewHistoryFilters, ReviewHistoryOptions, ReviewHistoryResult, SearchRecord,
    apply_history_query, build_haystack, default_history_page_projection, history_base_projection,
    matches_query, parse_search_query, review_history,
};
pub use ingest::{
    ImportEventOptions, IngestEventsOptions, IngestEventsResult, import_event, ingest_events,
};
pub use input_request::{
    InputRequestFetchOptions, InputRequestFetchResult, InputRequestListOptions,
    InputRequestListResult, InputRequestOpenOptions, InputRequestOpenResult,
    InputRequestRespondOptions, InputRequestRespondResult, InputRequestResponseView,
    InputRequestStatus, InputRequestStatusFilter, InputRequestTargetSelector, InputRequestView,
    fetch_input_request, list_input_requests, open_input_request, respond_input_request,
};
pub use observation::{
    ObservationAddOptions, ObservationAddResult, ObservationListOptions, ObservationListResult,
    ObservationStatus, ObservationTargetSelector, ObservationView, list_observations,
    record_observation,
};
pub use revision_list::{
    OrphanVisibility, RefFilterMode, RevisionListEntry, RevisionListOptions, RevisionListResult,
    list_revisions, list_units_for_ref,
};
pub use revision_projection::{
    MemberReadback, RevisionOverview, RevisionOverviewsOptions, RevisionProjectionIdentity,
    RevisionProjectionRow, RevisionProjectionSummary, RevisionShowFilters, RevisionShowOptions,
    RevisionShowResult, SnapshotContentState, SnapshotOrder, show_revision,
    show_revision_overviews,
};
pub use store_family::{
    StoreForgetOptions, StoreForgetResult, StoreListEntry, StoreListResult, forget_family_store,
    list_family_stores,
};
pub use store_identity::{
    StoreFamily, StoreIdentity, StoreIdentityOptions, StorePlacement, store_identity,
};
pub use store_link::{
    StoreLinkOptions, StoreLinkPreview, StoreLinkResult, StoreUnlinkOptions, StoreUnlinkResult,
    link_store_to_family, preview_link_to_family, unlink_store_from_family,
};
pub use store_migrate_common_dir::{
    MigrateToCommonDirOptions, MigrateToCommonDirResult, migrate_store_to_common_dir,
};
pub use store_status::{
    StoreSensitivityPathGroup, StoreStatusArtifactInventory, StoreStatusInventory,
    StoreStatusOptions, StoreStatusResult, StoreStatusRevisionObject, StoreStatusSensitivity,
    StoreStatusSensitivityExcludeGlob, StoreStatusSensitivityFinding, explain_store_sensitivity,
    store_status,
};
pub use validation::{
    ValidationAddOptions, ValidationAddResult, ValidationCheckView, ValidationListFilters,
    ValidationListOptions, ValidationListResult, list_validation_checks, record_validation_check,
};
pub(crate) use validation::{
    ValidationCheckProjectionOptions, annotate_validation_supersession, project_validation_checks,
};
