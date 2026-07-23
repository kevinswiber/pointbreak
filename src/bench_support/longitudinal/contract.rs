use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::canonical_hash::{canonical_json_bytes, sha256_bytes_hex};

pub const LONGITUDINAL_WORKLOAD_SCHEMA_V1: &str = "pointbreak.longitudinal-workload.v1";
pub const LONGITUDINAL_PROTOCOL_SCHEMA_V1: &str = "pointbreak.longitudinal-q3.v1";
pub const LONGITUDINAL_PUBLIC_SEED_HEX_V1: &str =
    "f4da49601a212010bae444e6ca2de6c6bf28b5ec1b0a05bf42154a533ca513ff";
pub const LONGITUDINAL_CAPACITY_SCHEMA_V1: &str = "pointbreak.longitudinal-capacity-sentinel.v1";
pub const LONGITUDINAL_CAPACITY_RECEIPT_SCHEMA_V1: &str =
    "pointbreak.longitudinal-capacity-receipt.v1";
pub const LONGITUDINAL_CAPACITY_MEMORY_SCHEMA_V1: &str =
    "pointbreak.longitudinal-capacity-memory-receipt.v1";

pub const LONGITUDINAL_MATERIALIZATION_RECEIPT_SCHEMA_V1: &str =
    "pointbreak.longitudinal-materialization-receipt.v1";
pub const LONGITUDINAL_OPERATION_RECEIPT_SCHEMA_V1: &str =
    "pointbreak.longitudinal-operation-receipt.v1";
pub const LONGITUDINAL_COUNTER_RECEIPT_SCHEMA_V1: &str =
    "pointbreak.longitudinal-counter-receipt.v1";
pub const LONGITUDINAL_EVIDENCE_PACKAGE_SCHEMA_V1: &str =
    "pointbreak.longitudinal-evidence-package.v1";
pub const LONGITUDINAL_CAPACITY_MATERIALIZATION_RECEIPT_SCHEMA_V1: &str =
    "pointbreak.longitudinal-capacity-materialization-receipt.v1";
pub const LONGITUDINAL_CAPACITY_PACKAGE_SCHEMA_V1: &str =
    "pointbreak.longitudinal-capacity-package.v1";
pub const LONGITUDINAL_CONTRACT_PUBLICATION_SCHEMA_V1: &str =
    "pointbreak.longitudinal-contract-publication.v1";
pub const LONGITUDINAL_HELP_SCHEMA_V1: &str = "pointbreak.longitudinal-help.v1";
pub const LONGITUDINAL_CONTRACT_MODE_V1: &str = "--longitudinal-contract";
pub const LONGITUDINAL_HELP_MODE_V1: &str = "--longitudinal-help";
pub const LONGITUDINAL_RUNNER_CONTRACT_SHA256_V1: &str =
    "a2182c741530317f31532c62d4b0152b228e3d85aa177f812fdca433c265ed85";
pub const LONGITUDINAL_CAPACITY_CONTRACT_SHA256_V1: &str =
    "b888f40f415cc8a9d75b23c6ab135a9be33247ffff68f67c78287292c10a532d";
pub const LONGITUDINAL_CONTRACT_PUBLICATION_SHA256_V1: &str =
    "acb0a65963d7eff873c1cee4890cfa3e9e73ea900eab44121eb14c73c92c36e3";

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LongitudinalTierV1 {
    L1,
    L7,
    L25,
    L100,
}

impl LongitudinalTierV1 {
    pub const ALL: [Self; 4] = [Self::L1, Self::L7, Self::L25, Self::L100];
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
pub enum LongitudinalCapacityProfileV1 {
    #[serde(rename = "L100-O10K")]
    L100O10K,
    #[serde(rename = "C262")]
    C262,
    #[serde(rename = "C524")]
    C524,
}

impl LongitudinalCapacityProfileV1 {
    pub const ALL: [Self; 3] = [Self::L100O10K, Self::C262, Self::C524];
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(tag = "kind", content = "profile")]
pub enum LongitudinalCapacitySubjectV1 {
    #[serde(rename = "V1L100")]
    V1L100,
    #[serde(rename = "companion")]
    Companion(LongitudinalCapacityProfileV1),
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LongitudinalCapacityProbeV1 {
    CarrierKey,
    SemanticId,
    ChronologicalHead,
    ChronologicalMiddle,
    ChronologicalTail,
    ObjectDetail,
    AppendDelta,
}

impl LongitudinalCapacityProbeV1 {
    pub const ALL: [Self; 7] = [
        Self::CarrierKey,
        Self::SemanticId,
        Self::ChronologicalHead,
        Self::ChronologicalMiddle,
        Self::ChronologicalTail,
        Self::ObjectDetail,
        Self::AppendDelta,
    ];
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LongitudinalCapacityClassificationV1 {
    Bounded,
    HistoryOrCardinalityProportional,
    UnsupportedNoCurrentSurface,
    ValidFailure,
    NotRunByGate,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LongitudinalLaneV1 {
    ReleaseUninstrumented,
    DebugUninstrumented,
    AttributionCounts,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LongitudinalOperationV1 {
    ColdHead,
    WarmHead,
    WinAdjacent,
    WinDeep,
    WinTail,
    AtDeep,
    SearchStructured,
    SearchBody,
    SearchMiss,
    FilterFacet,
    Revisions,
    Threads,
    AttentionAll,
    AttentionOne,
    DetailActive,
    SnapshotActive,
    DetailRemoved,
    FreshNoChange,
    NewCountZero,
    AppendOne,
    PostOne,
    AppendBurst,
    PostBurst,
    Restart,
    AuditExact,
    ExportExact,
    MigrateFresh,
    RebuildFull,
}

impl LongitudinalOperationV1 {
    pub const ALL: [Self; 28] = [
        Self::ColdHead,
        Self::WarmHead,
        Self::WinAdjacent,
        Self::WinDeep,
        Self::WinTail,
        Self::AtDeep,
        Self::SearchStructured,
        Self::SearchBody,
        Self::SearchMiss,
        Self::FilterFacet,
        Self::Revisions,
        Self::Threads,
        Self::AttentionAll,
        Self::AttentionOne,
        Self::DetailActive,
        Self::SnapshotActive,
        Self::DetailRemoved,
        Self::FreshNoChange,
        Self::NewCountZero,
        Self::AppendOne,
        Self::PostOne,
        Self::AppendBurst,
        Self::PostBurst,
        Self::Restart,
        Self::AuditExact,
        Self::ExportExact,
        Self::MigrateFresh,
        Self::RebuildFull,
    ];
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LongitudinalInteractionClassV1 {
    ColdHead,
    WarmWindow,
    SearchFilterFacet,
    FullRevisionThreadAttention,
    RevisionDetail,
    SnapshotDetail,
    FreshnessNewCount,
    AppendOne,
    PostOne,
    AppendBurst,
    PostBurst,
    Restart,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LongitudinalMemoryCheckpointV1 {
    Empty,
    WarmHead,
    Search,
    DetailPeak,
    PostDetail,
    PostAppend,
    PostBurst,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LongitudinalCapacityMemoryCheckpointV1 {
    ClosedRootReopenIdle,
    PostCarrierKey,
    PostSemanticId,
    PostChronologicalHead,
    PostChronologicalMiddle,
    PostChronologicalTail,
    PostObjectDetailL100O10K,
    PostContentionQuiescence,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LongitudinalCounterFieldV1 {
    DirectoryEntriesWalked,
    CarrierOpens,
    CarrierBytesRead,
    EventDecodes,
    EventValidations,
    EventFolds,
    ChronologicalSortItems,
    BodyArtifactReads,
    BodyBytesRead,
    ObjectArtifactReads,
    ObjectBytesRead,
    ProjectionRebuilds,
    StateRebuilds,
    ResponseBytes,
}

impl LongitudinalCounterFieldV1 {
    pub const ALL: [Self; 14] = [
        Self::DirectoryEntriesWalked,
        Self::CarrierOpens,
        Self::CarrierBytesRead,
        Self::EventDecodes,
        Self::EventValidations,
        Self::EventFolds,
        Self::ChronologicalSortItems,
        Self::BodyArtifactReads,
        Self::BodyBytesRead,
        Self::ObjectArtifactReads,
        Self::ObjectBytesRead,
        Self::ProjectionRebuilds,
        Self::StateRebuilds,
        Self::ResponseBytes,
    ];
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LongitudinalCapacityOwnershipFieldV1 {
    RetainedDecodedEvents,
    RetainedHydratedHistoryEntries,
    RetainedHydratedBodyBytes,
    RetainedSearchRecordStrings,
    RetainedSearchRecordFieldBytes,
    RetainedSerializedResponseCacheBytes,
    RetainedSnapshotHighlightEntries,
    RetainedSnapshotHighlightBytes,
}

impl LongitudinalCapacityOwnershipFieldV1 {
    pub const ALL: [Self; 8] = [
        Self::RetainedDecodedEvents,
        Self::RetainedHydratedHistoryEntries,
        Self::RetainedHydratedBodyBytes,
        Self::RetainedSearchRecordStrings,
        Self::RetainedSearchRecordFieldBytes,
        Self::RetainedSerializedResponseCacheBytes,
        Self::RetainedSnapshotHighlightEntries,
        Self::RetainedSnapshotHighlightBytes,
    ];
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LongitudinalContentKindV1 {
    ExternalBody,
    ObjectArtifact,
    ValidationLog,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalTierRequirementV1 {
    pub tier: LongitudinalTierV1,
    pub event_count: u64,
    pub block_count: u64,
    pub journal_count: u64,
    pub revision_count: u64,
    pub task_attempt_count: u64,
    pub body_fact_count: u64,
    pub external_body_count: u64,
    pub object_artifact_count: u64,
    pub validation_log_count: u64,
    pub removed_content_count: u64,
    pub present_content_count: u64,
    pub decoded_body_bytes: u64,
    pub decoded_object_target_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalEventFamilyRequirementV1 {
    pub event_type: String,
    pub per_block: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalEventFamilyCountV1 {
    pub event_type: String,
    pub count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalRunMatrixEntryV1 {
    pub lane: LongitudinalLaneV1,
    pub roots_per_tier: u8,
    pub records_timing: bool,
    pub records_cpu: bool,
    pub records_rss_heap: bool,
    pub performance_comparison_admissible: bool,
    pub counters_admissible: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalSamplePlanV1 {
    pub process_cold_per_release_root: u16,
    pub untimed_warm_requests: u16,
    pub excluded_warmups: u16,
    pub warm_samples_per_release_root: u16,
    pub append_one_samples_per_release_root: u16,
    pub append_burst_samples_per_release_root: u16,
    pub restart_samples_per_release_root: u16,
    pub maintenance_samples_per_release_root: u16,
    pub debug_cold_samples: u16,
    pub debug_warm_samples: u16,
    pub debug_write_restart_maintenance_samples: u16,
    pub counting_operations: Vec<LongitudinalOperationV1>,
    pub nearest_rank_percentile: u8,
    pub retain_all_samples: bool,
    pub outlier_removal_allowed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalInteractionEnvelopeV1 {
    pub class: LongitudinalInteractionClassV1,
    pub l1_wall_millis: u64,
    pub l7_wall_millis: u64,
    pub l25_wall_millis: u64,
    pub l100_wall_millis: u64,
    pub l100_cpu_millis: u64,
    pub complexity_intent: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalMemoryEnvelopeV1 {
    pub l100_steady_rss_mib: u64,
    pub l100_peak_rss_mib: u64,
    pub l100_retained_heap_mib: u64,
    pub l7_to_l100_steady_rss_bytes_per_event: u64,
    pub l7_to_l100_retained_heap_bytes_per_event: u64,
    pub selected_allocation_release_basis_points: u16,
    pub checkpoints: Vec<LongitudinalMemoryCheckpointV1>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalWriteEnvelopeV1 {
    pub steady_amplification_basis_points: u32,
    pub high_water_amplification_basis_points: u32,
    pub append_one_fixed_overhead_bytes: u64,
    pub append_burst_fixed_overhead_bytes: u64,
    pub old_authoritative_rewrite_allowed: bool,
    pub history_proportional_derived_rewrite_allowed: bool,
    pub full_state_rebuild_allowed: bool,
    pub burst_checkpoint_publications: u8,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalMaintenanceEnvelopeV1 {
    pub rebuild_wall_seconds: u64,
    pub rebuild_cpu_seconds: u64,
    pub rebuild_high_water_basis_points: u16,
    pub audit_wall_seconds: u64,
    pub export_wall_seconds: u64,
    pub import_wall_seconds: u64,
    pub migration_wall_seconds: u64,
    pub repair_wall_seconds: u64,
    pub free_space_source_basis_points: u16,
    pub free_space_fixed_bytes: u64,
    pub destination_estimate_required: bool,
    pub temporary_estimate_required: bool,
    pub take_maximum_of_source_and_destination_formulas: bool,
    pub interruption_percentages: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LongitudinalCounterEnvelopeClassV1 {
    NoChangeFreshness,
    FixedHistorySearchFilterWindow,
    RevisionsThreadsAttention,
    RevisionDetail,
    AppendOneMaintenance,
    AppendBurstMaintenance,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalCounterEnvelopeV1 {
    pub class: LongitudinalCounterEnvelopeClassV1,
    pub directory_entries_walked_max: u64,
    pub carrier_opens_max: u64,
    pub event_decodes_max: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_artifact_reads_max: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_artifact_reads_max: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_folds_max: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chronological_sort_items_max: Option<u64>,
    pub full_projection_rebuilds_max: u8,
    pub state_rebuilds_max: u8,
    pub selected_content_only: bool,
    pub output_sensitive_fold_sort: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalRunnerContractV1 {
    pub schema: String,
    pub protocol: String,
    pub public_seed_hex: String,
    pub block_event_count: u64,
    pub history_page_size: u64,
    pub body_size_pattern: Vec<u64>,
    pub object_size_multiplier: u64,
    pub append_batch_size: u64,
    pub tiers: Vec<LongitudinalTierRequirementV1>,
    pub event_families: Vec<LongitudinalEventFamilyRequirementV1>,
    pub operation_schedule: Vec<LongitudinalOperationV1>,
    pub run_matrix: Vec<LongitudinalRunMatrixEntryV1>,
    pub samples: LongitudinalSamplePlanV1,
    pub interaction_envelopes: Vec<LongitudinalInteractionEnvelopeV1>,
    pub memory_envelope: LongitudinalMemoryEnvelopeV1,
    pub counter_fields: Vec<LongitudinalCounterFieldV1>,
    pub counter_envelopes: Vec<LongitudinalCounterEnvelopeV1>,
    pub fixed_output_l100_to_l25_max_ratio_basis_points: u16,
    pub write_envelope: LongitudinalWriteEnvelopeV1,
    pub maintenance_envelope: LongitudinalMaintenanceEnvelopeV1,
    pub public_inputs_only: bool,
    pub lane_pooling_allowed: bool,
    pub contract_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalCapacityProfileRequirementV1 {
    pub profile: LongitudinalCapacityProfileV1,
    pub required: bool,
    pub event_count: u64,
    pub revision_count: u64,
    pub object_artifact_count: u64,
    pub task_attempt_count: u64,
    pub body_fact_count: u64,
    pub external_body_count: u64,
    pub validation_log_count: u64,
    pub removed_content_count: u64,
    pub present_content_count: u64,
    pub decoded_body_bytes: u64,
    pub decoded_object_target_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalCapacitySamplePlanV1 {
    pub process_cold_samples: u16,
    pub excluded_warmups: u16,
    pub warm_samples: u16,
    pub report_median: bool,
    pub report_maximum: bool,
    pub report_p95: bool,
    pub latency_threshold: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalContentionContractV1 {
    pub profile: LongitudinalCapacityProfileV1,
    pub writer_processes: u8,
    pub reader_processes: u8,
    pub unique_attempts_per_writer: u8,
    pub shared_attempts_per_writer: u8,
    pub expected_created: u8,
    pub expected_existing: u8,
    pub reader_cycles: u8,
    pub wall_guard_seconds: u64,
    pub rerun_allowed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalC524GateContractV1 {
    pub event_growth_basis_points: u32,
    pub maximum_pre_admission_ratio_basis_points: u32,
    pub maximum_post_admission_ratio_basis_points: u32,
    pub free_space_high_water_basis_points: u32,
    pub free_space_fixed_bytes: u64,
    pub identity_required: bool,
    pub base_rows_required: bool,
    pub counting_rows_required: bool,
    pub memory_receipts_required: bool,
    pub package_verification_required: bool,
    pub host_pressure_must_be_clear: bool,
    pub manual_override_allowed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalNonCompensationContractV1 {
    pub maximum_ratio_basis_points: u16,
    pub direct_whole_history_is_proportional: bool,
    pub wall_time_can_rescue: bool,
    pub unsupported_can_pass: bool,
    pub allowed_exhaustive_operations: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalCapacityContractV1 {
    pub schema: String,
    pub receipt_schema: String,
    pub memory_schema: String,
    pub public_seed_hex: String,
    pub derivation_domain: String,
    pub profiles: Vec<LongitudinalCapacityProfileRequirementV1>,
    pub subjects: Vec<LongitudinalCapacitySubjectV1>,
    pub probes: Vec<LongitudinalCapacityProbeV1>,
    pub samples: LongitudinalCapacitySamplePlanV1,
    pub memory_checkpoints: Vec<LongitudinalCapacityMemoryCheckpointV1>,
    pub ownership_fields: Vec<LongitudinalCapacityOwnershipFieldV1>,
    pub contention: LongitudinalContentionContractV1,
    pub c524_gate: LongitudinalC524GateContractV1,
    pub non_compensation: LongitudinalNonCompensationContractV1,
    pub v1_pooling_allowed: bool,
    pub contract_sha256: String,
}

impl LongitudinalRunnerContractV1 {
    pub fn canonical_sha256(&self) -> Result<String, LongitudinalContractError> {
        canonical_sha256_without(&self.contract_sha256, |contract_sha256| {
            let mut preimage = self.clone();
            preimage.contract_sha256 = contract_sha256;
            preimage
        })
    }

    pub fn validate(&self) -> Result<(), LongitudinalContractError> {
        if self.schema != LONGITUDINAL_WORKLOAD_SCHEMA_V1
            || self.protocol != LONGITUDINAL_PROTOCOL_SCHEMA_V1
            || self.public_seed_hex != LONGITUDINAL_PUBLIC_SEED_HEX_V1
        {
            return Err(LongitudinalContractError::UnsupportedContract);
        }
        require_exact_sequence(
            self.tiers.iter().map(|entry| entry.tier),
            LongitudinalTierV1::ALL,
            "workload tiers",
        )?;
        require_exact_sequence(
            self.operation_schedule.iter().copied(),
            LongitudinalOperationV1::ALL,
            "operation schedule",
        )?;
        require_exact_sequence(
            self.counter_fields.iter().copied(),
            LongitudinalCounterFieldV1::ALL,
            "counter fields",
        )?;
        validate_bound_hash(
            &self.contract_sha256,
            LONGITUDINAL_RUNNER_CONTRACT_SHA256_V1,
            "longitudinal runner contract",
        )?;
        if self.canonical_sha256()? != LONGITUDINAL_RUNNER_CONTRACT_SHA256_V1 {
            return Err(LongitudinalContractError::ContractDrift {
                field: "longitudinal runner contract",
            });
        }
        Ok(())
    }
}

impl LongitudinalCapacityContractV1 {
    pub fn canonical_sha256(&self) -> Result<String, LongitudinalContractError> {
        canonical_sha256_without(&self.contract_sha256, |contract_sha256| {
            let mut preimage = self.clone();
            preimage.contract_sha256 = contract_sha256;
            preimage
        })
    }

    pub fn validate(&self) -> Result<(), LongitudinalContractError> {
        if self.schema != LONGITUDINAL_CAPACITY_SCHEMA_V1
            || self.receipt_schema != LONGITUDINAL_CAPACITY_RECEIPT_SCHEMA_V1
            || self.memory_schema != LONGITUDINAL_CAPACITY_MEMORY_SCHEMA_V1
            || self.public_seed_hex != LONGITUDINAL_PUBLIC_SEED_HEX_V1
            || self.derivation_domain != LONGITUDINAL_CAPACITY_SCHEMA_V1
        {
            return Err(LongitudinalContractError::UnsupportedContract);
        }
        require_exact_sequence(
            self.profiles.iter().map(|entry| entry.profile),
            LongitudinalCapacityProfileV1::ALL,
            "capacity profiles",
        )?;
        require_exact_sequence(
            self.probes.iter().copied(),
            LongitudinalCapacityProbeV1::ALL,
            "capacity probes",
        )?;
        require_exact_sequence(
            self.ownership_fields.iter().copied(),
            LongitudinalCapacityOwnershipFieldV1::ALL,
            "capacity ownership fields",
        )?;
        validate_bound_hash(
            &self.contract_sha256,
            LONGITUDINAL_CAPACITY_CONTRACT_SHA256_V1,
            "longitudinal capacity contract",
        )?;
        if self.canonical_sha256()? != LONGITUDINAL_CAPACITY_CONTRACT_SHA256_V1 {
            return Err(LongitudinalContractError::ContractDrift {
                field: "longitudinal capacity contract",
            });
        }
        Ok(())
    }
}

pub fn longitudinal_runner_contract_v1() -> LongitudinalRunnerContractV1 {
    let body_sizes = vec![512, 1_024, 2_048, 4_096, 8_192, 12_288, 16_384, 20_992];
    let mut contract = LongitudinalRunnerContractV1 {
        schema: LONGITUDINAL_WORKLOAD_SCHEMA_V1.to_owned(),
        protocol: LONGITUDINAL_PROTOCOL_SCHEMA_V1.to_owned(),
        public_seed_hex: LONGITUDINAL_PUBLIC_SEED_HEX_V1.to_owned(),
        block_event_count: 256,
        history_page_size: 100,
        body_size_pattern: body_sizes,
        object_size_multiplier: 8,
        append_batch_size: 30,
        tiers: vec![
            tier_requirement(LongitudinalTierV1::L1, 4),
            tier_requirement(LongitudinalTierV1::L7, 28),
            tier_requirement(LongitudinalTierV1::L25, 100),
            tier_requirement(LongitudinalTierV1::L100, 400),
        ],
        event_families: vec![
            event_family("review_initialized", 1),
            event_family("work_object_proposed", 16),
            event_family("review_observation_recorded", 64),
            event_family("review_assessment_recorded", 24),
            event_family("input_request_opened", 24),
            event_family("input_request_responded", 16),
            event_family("revision_ref_associated", 12),
            event_family("revision_ref_withdrawn", 4),
            event_family("revision_commit_associated", 16),
            event_family("revision_commit_withdrawn", 4),
            event_family("validation_check_recorded", 40),
            event_family("task_checkpoint_captured", 12),
            event_family("task_observation_recorded", 12),
            event_family("event_signature_recorded", 8),
            event_family("artifact_removed", 3),
            event_family("review_note_imported", 0),
        ],
        operation_schedule: LongitudinalOperationV1::ALL.to_vec(),
        run_matrix: vec![
            LongitudinalRunMatrixEntryV1 {
                lane: LongitudinalLaneV1::ReleaseUninstrumented,
                roots_per_tier: 2,
                records_timing: true,
                records_cpu: true,
                records_rss_heap: true,
                performance_comparison_admissible: true,
                counters_admissible: false,
            },
            LongitudinalRunMatrixEntryV1 {
                lane: LongitudinalLaneV1::DebugUninstrumented,
                roots_per_tier: 1,
                records_timing: true,
                records_cpu: true,
                records_rss_heap: true,
                performance_comparison_admissible: false,
                counters_admissible: false,
            },
            LongitudinalRunMatrixEntryV1 {
                lane: LongitudinalLaneV1::AttributionCounts,
                roots_per_tier: 1,
                records_timing: false,
                records_cpu: false,
                records_rss_heap: false,
                performance_comparison_admissible: false,
                counters_admissible: true,
            },
        ],
        samples: LongitudinalSamplePlanV1 {
            process_cold_per_release_root: 10,
            untimed_warm_requests: 1,
            excluded_warmups: 3,
            warm_samples_per_release_root: 30,
            append_one_samples_per_release_root: 30,
            append_burst_samples_per_release_root: 10,
            restart_samples_per_release_root: 10,
            maintenance_samples_per_release_root: 3,
            debug_cold_samples: 1,
            debug_warm_samples: 3,
            debug_write_restart_maintenance_samples: 1,
            counting_operations: vec![
                LongitudinalOperationV1::ColdHead,
                LongitudinalOperationV1::WarmHead,
                LongitudinalOperationV1::WinDeep,
                LongitudinalOperationV1::SearchBody,
                LongitudinalOperationV1::Revisions,
                LongitudinalOperationV1::Threads,
                LongitudinalOperationV1::AttentionAll,
                LongitudinalOperationV1::DetailActive,
                LongitudinalOperationV1::FreshNoChange,
                LongitudinalOperationV1::AppendOne,
                LongitudinalOperationV1::AppendBurst,
                LongitudinalOperationV1::Restart,
                LongitudinalOperationV1::AuditExact,
                LongitudinalOperationV1::ExportExact,
                LongitudinalOperationV1::MigrateFresh,
                LongitudinalOperationV1::RebuildFull,
            ],
            nearest_rank_percentile: 95,
            retain_all_samples: true,
            outlier_removal_allowed: false,
        },
        interaction_envelopes: interaction_envelopes(),
        memory_envelope: LongitudinalMemoryEnvelopeV1 {
            l100_steady_rss_mib: 96,
            l100_peak_rss_mib: 128,
            l100_retained_heap_mib: 64,
            l7_to_l100_steady_rss_bytes_per_event: 512,
            l7_to_l100_retained_heap_bytes_per_event: 384,
            selected_allocation_release_basis_points: 9_500,
            checkpoints: vec![
                LongitudinalMemoryCheckpointV1::Empty,
                LongitudinalMemoryCheckpointV1::WarmHead,
                LongitudinalMemoryCheckpointV1::Search,
                LongitudinalMemoryCheckpointV1::DetailPeak,
                LongitudinalMemoryCheckpointV1::PostDetail,
                LongitudinalMemoryCheckpointV1::PostAppend,
                LongitudinalMemoryCheckpointV1::PostBurst,
            ],
        },
        counter_fields: LongitudinalCounterFieldV1::ALL.to_vec(),
        counter_envelopes: counter_envelopes(),
        fixed_output_l100_to_l25_max_ratio_basis_points: 20_000,
        write_envelope: LongitudinalWriteEnvelopeV1 {
            steady_amplification_basis_points: 80_000,
            high_water_amplification_basis_points: 120_000,
            append_one_fixed_overhead_bytes: 256 * 1024,
            append_burst_fixed_overhead_bytes: MIB,
            old_authoritative_rewrite_allowed: false,
            history_proportional_derived_rewrite_allowed: false,
            full_state_rebuild_allowed: false,
            burst_checkpoint_publications: 1,
        },
        maintenance_envelope: LongitudinalMaintenanceEnvelopeV1 {
            rebuild_wall_seconds: 60,
            rebuild_cpu_seconds: 45,
            rebuild_high_water_basis_points: 15_000,
            audit_wall_seconds: 60,
            export_wall_seconds: 120,
            import_wall_seconds: 180,
            migration_wall_seconds: 15 * 60,
            repair_wall_seconds: 15 * 60,
            free_space_source_basis_points: 12_000,
            free_space_fixed_bytes: 256 * MIB,
            destination_estimate_required: true,
            temporary_estimate_required: true,
            take_maximum_of_source_and_destination_formulas: true,
            interruption_percentages: vec![10, 50, 90],
        },
        public_inputs_only: true,
        lane_pooling_allowed: false,
        contract_sha256: String::new(),
    };
    contract.contract_sha256 = contract
        .canonical_sha256()
        .expect("the frozen longitudinal runner contract is canonical");
    contract
}

pub fn longitudinal_capacity_contract_v1() -> LongitudinalCapacityContractV1 {
    let mut contract = LongitudinalCapacityContractV1 {
        schema: LONGITUDINAL_CAPACITY_SCHEMA_V1.to_owned(),
        receipt_schema: LONGITUDINAL_CAPACITY_RECEIPT_SCHEMA_V1.to_owned(),
        memory_schema: LONGITUDINAL_CAPACITY_MEMORY_SCHEMA_V1.to_owned(),
        public_seed_hex: LONGITUDINAL_PUBLIC_SEED_HEX_V1.to_owned(),
        derivation_domain: LONGITUDINAL_CAPACITY_SCHEMA_V1.to_owned(),
        profiles: vec![
            LongitudinalCapacityProfileRequirementV1 {
                profile: LongitudinalCapacityProfileV1::L100O10K,
                required: true,
                event_count: 102_400,
                revision_count: 10_000,
                object_artifact_count: 10_000,
                task_attempt_count: 400,
                body_fact_count: 65_800,
                external_body_count: 32_900,
                validation_log_count: 1_000,
                removed_content_count: 300,
                present_content_count: 43_600,
                decoded_body_bytes: 539_033_600,
                decoded_object_target_bytes: 655_360_000,
            },
            LongitudinalCapacityProfileRequirementV1 {
                profile: LongitudinalCapacityProfileV1::C262,
                required: true,
                event_count: 262_144,
                revision_count: 12_288,
                object_artifact_count: 12_288,
                task_attempt_count: 4_096,
                body_fact_count: 184_320,
                external_body_count: 92_160,
                validation_log_count: 5_120,
                removed_content_count: 3_072,
                present_content_count: 106_496,
                decoded_body_bytes: 1_509_949_440,
                decoded_object_target_bytes: 805_306_368,
            },
            LongitudinalCapacityProfileRequirementV1 {
                profile: LongitudinalCapacityProfileV1::C524,
                required: false,
                event_count: 524_288,
                revision_count: 24_576,
                object_artifact_count: 24_576,
                task_attempt_count: 8_192,
                body_fact_count: 368_640,
                external_body_count: 184_320,
                validation_log_count: 10_240,
                removed_content_count: 6_144,
                present_content_count: 212_992,
                decoded_body_bytes: 3_019_898_880,
                decoded_object_target_bytes: 1_610_612_736,
            },
        ],
        subjects: vec![
            LongitudinalCapacitySubjectV1::V1L100,
            LongitudinalCapacitySubjectV1::Companion(LongitudinalCapacityProfileV1::L100O10K),
            LongitudinalCapacitySubjectV1::Companion(LongitudinalCapacityProfileV1::C262),
            LongitudinalCapacitySubjectV1::Companion(LongitudinalCapacityProfileV1::C524),
        ],
        probes: LongitudinalCapacityProbeV1::ALL.to_vec(),
        samples: LongitudinalCapacitySamplePlanV1 {
            process_cold_samples: 1,
            excluded_warmups: 1,
            warm_samples: 5,
            report_median: true,
            report_maximum: true,
            report_p95: false,
            latency_threshold: None,
        },
        memory_checkpoints: vec![
            LongitudinalCapacityMemoryCheckpointV1::ClosedRootReopenIdle,
            LongitudinalCapacityMemoryCheckpointV1::PostCarrierKey,
            LongitudinalCapacityMemoryCheckpointV1::PostSemanticId,
            LongitudinalCapacityMemoryCheckpointV1::PostChronologicalHead,
            LongitudinalCapacityMemoryCheckpointV1::PostChronologicalMiddle,
            LongitudinalCapacityMemoryCheckpointV1::PostChronologicalTail,
            LongitudinalCapacityMemoryCheckpointV1::PostObjectDetailL100O10K,
            LongitudinalCapacityMemoryCheckpointV1::PostContentionQuiescence,
        ],
        ownership_fields: LongitudinalCapacityOwnershipFieldV1::ALL.to_vec(),
        contention: LongitudinalContentionContractV1 {
            profile: LongitudinalCapacityProfileV1::L100O10K,
            writer_processes: 2,
            reader_processes: 1,
            unique_attempts_per_writer: 4,
            shared_attempts_per_writer: 2,
            expected_created: 10,
            expected_existing: 2,
            reader_cycles: 20,
            wall_guard_seconds: 15 * 60,
            rerun_allowed: false,
        },
        c524_gate: LongitudinalC524GateContractV1 {
            event_growth_basis_points: 25_600,
            maximum_pre_admission_ratio_basis_points: 32_000,
            maximum_post_admission_ratio_basis_points: 25_000,
            free_space_high_water_basis_points: 25_000,
            free_space_fixed_bytes: 16 * GIB,
            identity_required: true,
            base_rows_required: true,
            counting_rows_required: true,
            memory_receipts_required: true,
            package_verification_required: true,
            host_pressure_must_be_clear: true,
            manual_override_allowed: false,
        },
        non_compensation: LongitudinalNonCompensationContractV1 {
            maximum_ratio_basis_points: 12_500,
            direct_whole_history_is_proportional: true,
            wall_time_can_rescue: false,
            unsupported_can_pass: false,
            allowed_exhaustive_operations: [
                "integrity_audit",
                "logical_export_import_verification",
                "migration",
                "copy_out_repair",
                "backup_restore_verification",
                "removal_compaction_reconciliation",
                "deliberate_rebuild",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        },
        v1_pooling_allowed: false,
        contract_sha256: String::new(),
    };
    contract.contract_sha256 = contract
        .canonical_sha256()
        .expect("the frozen longitudinal capacity contract is canonical");
    contract
}

fn tier_requirement(tier: LongitudinalTierV1, blocks: u64) -> LongitudinalTierRequirementV1 {
    LongitudinalTierRequirementV1 {
        tier,
        event_count: blocks * 256,
        block_count: blocks,
        journal_count: blocks,
        revision_count: blocks * 12,
        task_attempt_count: blocks * 4,
        body_fact_count: blocks * 180,
        external_body_count: blocks * 90,
        object_artifact_count: blocks * 12,
        validation_log_count: blocks * 5,
        removed_content_count: blocks * 3,
        present_content_count: blocks * 104,
        decoded_body_bytes: blocks * 1_474_560,
        decoded_object_target_bytes: blocks * 786_432,
    }
}

fn event_family(event_type: &str, per_block: u64) -> LongitudinalEventFamilyRequirementV1 {
    LongitudinalEventFamilyRequirementV1 {
        event_type: event_type.to_owned(),
        per_block,
    }
}

fn interaction_envelopes() -> Vec<LongitudinalInteractionEnvelopeV1> {
    use LongitudinalInteractionClassV1 as Class;
    [
        (
            Class::ColdHead,
            500,
            750,
            1_000,
            2_000,
            1_500,
            "product target; exact fresh-process validation remains visible",
        ),
        (
            Class::WarmWindow,
            75,
            100,
            125,
            150,
            100,
            "window plus bounded delta, never all events",
        ),
        (
            Class::SearchFilterFacet,
            100,
            150,
            200,
            250,
            200,
            "query result plus derived lookup, never body-wide scan",
        ),
        (
            Class::FullRevisionThreadAttention,
            200,
            350,
            750,
            1_500,
            1_000,
            "response rows or edges plus bounded delta",
        ),
        (
            Class::RevisionDetail,
            100,
            125,
            175,
            250,
            175,
            "selected revision facts plus selected content",
        ),
        (
            Class::SnapshotDetail,
            150,
            200,
            350,
            500,
            350,
            "selected snapshot bytes and rows",
        ),
        (
            Class::FreshnessNewCount,
            25,
            30,
            40,
            50,
            25,
            "constant marker or query delta; no full journal",
        ),
        (
            Class::AppendOne,
            100,
            125,
            175,
            250,
            200,
            "new fact plus bounded target state; no full replay",
        ),
        (
            Class::PostOne,
            200,
            250,
            350,
            500,
            400,
            "bounded incremental maintenance",
        ),
        (
            Class::AppendBurst,
            750,
            1_000,
            1_500,
            3_000,
            2_000,
            "thirty events plus bounded affected state",
        ),
        (
            Class::PostBurst,
            350,
            500,
            750,
            1_000,
            750,
            "one coalesced incremental refresh",
        ),
        (
            Class::Restart,
            750,
            1_000,
            1_500,
            3_000,
            2_500,
            "exact recovery receipt remains mandatory",
        ),
    ]
    .into_iter()
    .map(
        |(class, l1, l7, l25, l100, cpu, intent)| LongitudinalInteractionEnvelopeV1 {
            class,
            l1_wall_millis: l1,
            l7_wall_millis: l7,
            l25_wall_millis: l25,
            l100_wall_millis: l100,
            l100_cpu_millis: cpu,
            complexity_intent: intent.to_owned(),
        },
    )
    .collect()
}

fn counter_envelopes() -> Vec<LongitudinalCounterEnvelopeV1> {
    use LongitudinalCounterEnvelopeClassV1 as Class;
    vec![
        LongitudinalCounterEnvelopeV1 {
            class: Class::NoChangeFreshness,
            directory_entries_walked_max: 4,
            carrier_opens_max: 0,
            event_decodes_max: 0,
            body_artifact_reads_max: Some(0),
            object_artifact_reads_max: Some(0),
            event_folds_max: Some(0),
            chronological_sort_items_max: Some(0),
            full_projection_rebuilds_max: 0,
            state_rebuilds_max: 0,
            selected_content_only: true,
            output_sensitive_fold_sort: false,
        },
        LongitudinalCounterEnvelopeV1 {
            class: Class::FixedHistorySearchFilterWindow,
            directory_entries_walked_max: 8,
            carrier_opens_max: 512,
            event_decodes_max: 256,
            body_artifact_reads_max: Some(100),
            object_artifact_reads_max: Some(1),
            event_folds_max: Some(2_048),
            chronological_sort_items_max: Some(512),
            full_projection_rebuilds_max: 0,
            state_rebuilds_max: 0,
            selected_content_only: true,
            output_sensitive_fold_sort: false,
        },
        LongitudinalCounterEnvelopeV1 {
            class: Class::RevisionsThreadsAttention,
            directory_entries_walked_max: 8,
            carrier_opens_max: 2_048,
            event_decodes_max: 1_024,
            body_artifact_reads_max: Some(0),
            object_artifact_reads_max: Some(0),
            event_folds_max: None,
            chronological_sort_items_max: None,
            full_projection_rebuilds_max: 0,
            state_rebuilds_max: 0,
            selected_content_only: true,
            output_sensitive_fold_sort: true,
        },
        LongitudinalCounterEnvelopeV1 {
            class: Class::RevisionDetail,
            directory_entries_walked_max: 8,
            carrier_opens_max: 512,
            event_decodes_max: 256,
            body_artifact_reads_max: None,
            object_artifact_reads_max: None,
            event_folds_max: Some(2_048),
            chronological_sort_items_max: None,
            full_projection_rebuilds_max: 0,
            state_rebuilds_max: 0,
            selected_content_only: true,
            output_sensitive_fold_sort: false,
        },
        LongitudinalCounterEnvelopeV1 {
            class: Class::AppendOneMaintenance,
            directory_entries_walked_max: 16,
            carrier_opens_max: 512,
            event_decodes_max: 256,
            body_artifact_reads_max: None,
            object_artifact_reads_max: None,
            event_folds_max: Some(2_048),
            chronological_sort_items_max: None,
            full_projection_rebuilds_max: 0,
            state_rebuilds_max: 0,
            selected_content_only: true,
            output_sensitive_fold_sort: false,
        },
        LongitudinalCounterEnvelopeV1 {
            class: Class::AppendBurstMaintenance,
            directory_entries_walked_max: 32,
            carrier_opens_max: 2_048,
            event_decodes_max: 1_024,
            body_artifact_reads_max: None,
            object_artifact_reads_max: None,
            event_folds_max: Some(8_192),
            chronological_sort_items_max: None,
            full_projection_rebuilds_max: 0,
            state_rebuilds_max: 0,
            selected_content_only: true,
            output_sensitive_fold_sort: false,
        },
    ]
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalExecutionIdentityV1 {
    pub source_commit: String,
    pub source_tree: String,
    pub cargo_lock_sha256: String,
    pub runner_sha256: String,
    pub build_profile: String,
    pub operating_system: String,
    pub architecture: String,
    pub filesystem: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_commit: Option<String>,
}

impl LongitudinalExecutionIdentityV1 {
    pub fn canonical_sha256(&self) -> Result<String, LongitudinalContractError> {
        canonical_sha256(self)
    }

    pub fn validate(&self) -> Result<(), LongitudinalContractError> {
        validate_hex(&self.source_commit, 40, "source commit")?;
        validate_hex(&self.source_tree, 40, "source tree")?;
        validate_hex(&self.cargo_lock_sha256, 64, "Cargo.lock SHA-256")?;
        validate_hex(&self.runner_sha256, 64, "runner SHA-256")?;
        if let Some(parent) = &self.parent_commit {
            validate_hex(parent, 40, "parent commit")?;
        }
        require_nonempty(&self.build_profile, "build profile")?;
        require_nonempty(&self.operating_system, "operating system")?;
        require_nonempty(&self.architecture, "architecture")?;
        require_nonempty(&self.filesystem, "filesystem")
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalEventIdentityV1 {
    pub event_id: String,
    pub canonical_decoded_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalInventoryEntryV1 {
    pub logical_key: String,
    pub kind: LongitudinalContentKindV1,
    pub raw_sha256: String,
    pub decoded_sha256: String,
    pub raw_bytes: u64,
    pub decoded_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalEventCarrierV1 {
    pub event_id: String,
    pub logical_key_sha256: String,
    pub raw_sha256: String,
    pub raw_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalExpectedSemanticReceiptV1 {
    pub operation: LongitudinalOperationV1,
    pub semantic_receipt_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalWorkloadManifestV1 {
    pub schema: String,
    pub protocol: String,
    pub contract_sha256: String,
    pub execution: LongitudinalExecutionIdentityV1,
    pub public_seed_hex: String,
    pub tier: LongitudinalTierV1,
    pub event_count: u64,
    pub revision_count: u64,
    pub by_type: Vec<LongitudinalEventFamilyCountV1>,
    pub ordered_events: Vec<LongitudinalEventIdentityV1>,
    pub event_carriers: Vec<LongitudinalEventCarrierV1>,
    pub content_inventory: Vec<LongitudinalInventoryEntryV1>,
    pub removed_content_sha256: Vec<String>,
    pub expected_semantic_receipts: Vec<LongitudinalExpectedSemanticReceiptV1>,
    pub schedule: Vec<LongitudinalOperationV1>,
    pub schedule_sha256: String,
    pub manifest_sha256: String,
}

impl LongitudinalWorkloadManifestV1 {
    pub fn canonical_sha256(&self) -> Result<String, LongitudinalContractError> {
        canonical_sha256_without(&self.manifest_sha256, |manifest_sha256| {
            let mut preimage = self.clone();
            preimage.manifest_sha256 = manifest_sha256;
            preimage
        })
    }

    pub fn validate(&self) -> Result<(), LongitudinalContractError> {
        let contract = longitudinal_runner_contract_v1();
        if self.schema != contract.schema
            || self.protocol != contract.protocol
            || self.contract_sha256 != contract.contract_sha256
            || self.public_seed_hex != contract.public_seed_hex
        {
            return Err(LongitudinalContractError::UnsupportedContract);
        }
        self.execution.validate()?;
        let tier = contract
            .tiers
            .iter()
            .find(|candidate| candidate.tier == self.tier)
            .ok_or(LongitudinalContractError::UnsupportedTier)?;
        require_count(self.event_count, tier.event_count, "manifest event count")?;
        require_count(
            self.revision_count,
            tier.revision_count,
            "manifest revision count",
        )?;
        require_len(
            self.ordered_events.len(),
            tier.event_count,
            "ordered event identities",
        )?;
        require_len(
            self.event_carriers.len(),
            tier.event_count,
            "event carrier inventory",
        )?;
        require_len(
            self.content_inventory.len(),
            tier.present_content_count,
            "content inventory",
        )?;
        require_len(
            self.removed_content_sha256.len(),
            tier.removed_content_count,
            "removed-content set",
        )?;
        require_exact_sequence(
            self.schedule.iter().copied(),
            LongitudinalOperationV1::ALL,
            "manifest schedule",
        )?;
        require_exact_sequence(
            self.expected_semantic_receipts
                .iter()
                .map(|receipt| receipt.operation),
            LongitudinalOperationV1::ALL,
            "expected semantic receipts",
        )?;
        if self.by_type != scaled_event_families(&contract.event_families, tier.block_count) {
            return Err(LongitudinalContractError::ContractDrift {
                field: "event-family counts",
            });
        }
        require_unique(
            self.ordered_events
                .iter()
                .map(|event| event.event_id.as_str()),
            "ordered event ids",
        )?;
        require_unique(
            self.event_carriers
                .iter()
                .map(|carrier| carrier.event_id.as_str()),
            "event carrier ids",
        )?;
        require_unique(
            self.content_inventory
                .iter()
                .map(|entry| entry.logical_key.as_str()),
            "content logical keys",
        )?;
        require_unique(
            self.removed_content_sha256.iter().map(String::as_str),
            "removed-content hashes",
        )?;
        for event in &self.ordered_events {
            validate_hex(
                &event.canonical_decoded_sha256,
                64,
                "canonical event SHA-256",
            )?;
        }
        for carrier in &self.event_carriers {
            validate_hex(&carrier.logical_key_sha256, 64, "event key SHA-256")?;
            validate_hex(&carrier.raw_sha256, 64, "event carrier SHA-256")?;
        }
        for content in &self.content_inventory {
            validate_hex(&content.raw_sha256, 64, "content raw SHA-256")?;
            validate_hex(&content.decoded_sha256, 64, "content decoded SHA-256")?;
        }
        for hash in &self.removed_content_sha256 {
            validate_hex(hash, 64, "removed-content SHA-256")?;
        }
        for receipt in &self.expected_semantic_receipts {
            validate_hex(
                &receipt.semantic_receipt_sha256,
                64,
                "semantic receipt SHA-256",
            )?;
        }
        validate_bound_hash(
            &self.schedule_sha256,
            &canonical_sha256(&self.schedule)?,
            "operation schedule",
        )?;
        validate_bound_hash(
            &self.manifest_sha256,
            &self.canonical_sha256()?,
            "workload manifest",
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalStrictSemanticReceiptV1 {
    pub event_set_sha256: String,
    pub ordered_journal_sha256: String,
    pub state_sha256: String,
    pub projection_sha256: String,
    pub content_inventory_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalMaterializationReceiptV1 {
    pub schema: String,
    pub root_identity: String,
    pub manifest: LongitudinalWorkloadManifestV1,
    pub strict: LongitudinalStrictSemanticReceiptV1,
    pub materialization_sha256: String,
}

impl LongitudinalMaterializationReceiptV1 {
    pub fn canonical_sha256(&self) -> Result<String, LongitudinalContractError> {
        canonical_sha256_without(&self.materialization_sha256, |materialization_sha256| {
            let mut preimage = self.clone();
            preimage.materialization_sha256 = materialization_sha256;
            preimage
        })
    }

    fn pair_identity_sha256(&self) -> Result<String, LongitudinalContractError> {
        let mut preimage = self.clone();
        preimage.root_identity.clear();
        preimage.materialization_sha256.clear();
        canonical_sha256(&preimage)
    }

    pub fn validate(&self) -> Result<(), LongitudinalContractError> {
        if self.schema != LONGITUDINAL_MATERIALIZATION_RECEIPT_SCHEMA_V1 {
            return Err(LongitudinalContractError::UnsupportedContract);
        }
        validate_hex(&self.root_identity, 64, "root identity")?;
        self.manifest.validate()?;
        validate_strict_receipt(&self.strict)?;
        validate_bound_hash(
            &self.materialization_sha256,
            &self.canonical_sha256()?,
            "materialization receipt",
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalSemanticFactsV1 {
    pub ordered_ids: Vec<String>,
    pub fact_ids: Vec<String>,
    pub diagnostics: Vec<String>,
    pub event_count: u64,
    pub event_set_sha256: String,
    pub content_state_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalOperationMetricsV1 {
    pub wall_nanos: u64,
    pub process_user_cpu_nanos: u64,
    pub process_system_cpu_nanos: u64,
    pub baseline_rss_bytes: u64,
    pub peak_rss_bytes: u64,
    pub steady_rss_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalOperationReceiptV1 {
    pub schema: String,
    pub run_identity: String,
    pub root_identity: String,
    pub tier: LongitudinalTierV1,
    pub manifest_sha256: String,
    pub schedule_sha256: String,
    pub execution_identity_sha256: String,
    pub lane: LongitudinalLaneV1,
    pub operation: LongitudinalOperationV1,
    pub sample_ordinal: u32,
    pub pre_event_count: u64,
    pub post_event_count: u64,
    pub pre_event_set_sha256: String,
    pub post_event_set_sha256: String,
    pub response_bytes: u64,
    pub response_canonical_sha256: String,
    pub semantic_facts: LongitudinalSemanticFactsV1,
    pub semantic_receipt_sha256: String,
    pub metrics: LongitudinalOperationMetricsV1,
    pub receipt_sha256: String,
}

impl LongitudinalOperationReceiptV1 {
    pub fn canonical_sha256(&self) -> Result<String, LongitudinalContractError> {
        canonical_sha256_without(&self.receipt_sha256, |receipt_sha256| {
            let mut preimage = self.clone();
            preimage.receipt_sha256 = receipt_sha256;
            preimage
        })
    }

    pub fn validate(&self) -> Result<(), LongitudinalContractError> {
        if self.schema != LONGITUDINAL_OPERATION_RECEIPT_SCHEMA_V1
            || self.lane == LongitudinalLaneV1::AttributionCounts
        {
            return Err(LongitudinalContractError::MixedLane);
        }
        for (hash, name) in [
            (&self.run_identity, "run identity"),
            (&self.root_identity, "root identity"),
            (&self.manifest_sha256, "manifest SHA-256"),
            (&self.schedule_sha256, "schedule SHA-256"),
            (
                &self.execution_identity_sha256,
                "execution identity SHA-256",
            ),
            (&self.pre_event_set_sha256, "pre-event-set SHA-256"),
            (&self.post_event_set_sha256, "post-event-set SHA-256"),
            (&self.response_canonical_sha256, "response SHA-256"),
            (&self.semantic_receipt_sha256, "semantic receipt SHA-256"),
        ] {
            validate_hex(hash, 64, name)?;
        }
        validate_bound_hash(
            &self.receipt_sha256,
            &self.canonical_sha256()?,
            "operation receipt",
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalCountersV1 {
    pub directory_entries_walked: u64,
    pub carrier_opens: u64,
    pub carrier_bytes_read: u64,
    pub event_decodes: u64,
    pub event_validations: u64,
    pub event_folds: u64,
    pub chronological_sort_items: u64,
    pub body_artifact_reads: u64,
    pub body_bytes_read: u64,
    pub object_artifact_reads: u64,
    pub object_bytes_read: u64,
    pub projection_rebuilds: u64,
    pub state_rebuilds: u64,
    pub response_bytes: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalCapacityOwnershipV1 {
    pub retained_decoded_events: u64,
    pub retained_hydrated_history_entries: u64,
    pub retained_hydrated_body_bytes: u64,
    pub retained_search_record_strings: u64,
    pub retained_search_record_field_bytes: u64,
    pub retained_serialized_response_cache_bytes: u64,
    pub retained_snapshot_highlight_entries: u64,
    pub retained_snapshot_highlight_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalCounterReceiptV1 {
    pub schema: String,
    pub run_identity: String,
    pub root_identity: String,
    pub operation: String,
    pub phase: String,
    pub base_execution_identity_sha256: String,
    pub derivative_execution_identity_sha256: String,
    pub manifest_sha256: String,
    pub schedule_sha256: String,
    pub success: bool,
    pub semantic_result_sha256: String,
    pub counters: LongitudinalCountersV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capacity_ownership: Option<LongitudinalCapacityOwnershipV1>,
    pub receipt_sha256: String,
}

impl LongitudinalCounterReceiptV1 {
    pub fn canonical_sha256(&self) -> Result<String, LongitudinalContractError> {
        canonical_sha256_without(&self.receipt_sha256, |receipt_sha256| {
            let mut preimage = self.clone();
            preimage.receipt_sha256 = receipt_sha256;
            preimage
        })
    }

    pub fn validate(&self) -> Result<(), LongitudinalContractError> {
        if self.schema != LONGITUDINAL_COUNTER_RECEIPT_SCHEMA_V1 {
            return Err(LongitudinalContractError::UnsupportedContract);
        }
        for (hash, name) in [
            (&self.run_identity, "run identity"),
            (&self.root_identity, "root identity"),
            (
                &self.base_execution_identity_sha256,
                "base execution identity",
            ),
            (
                &self.derivative_execution_identity_sha256,
                "derivative execution identity",
            ),
            (&self.manifest_sha256, "manifest SHA-256"),
            (&self.schedule_sha256, "schedule SHA-256"),
            (&self.semantic_result_sha256, "semantic result SHA-256"),
        ] {
            validate_hex(hash, 64, name)?;
        }
        require_nonempty(&self.operation, "counter operation")?;
        require_nonempty(&self.phase, "counter phase")?;
        validate_bound_hash(
            &self.receipt_sha256,
            &self.canonical_sha256()?,
            "counter receipt",
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalCapacityManifestV1 {
    pub schema: String,
    pub contract_sha256: String,
    pub execution: LongitudinalExecutionIdentityV1,
    pub public_seed_hex: String,
    pub subject: LongitudinalCapacitySubjectV1,
    pub event_count: u64,
    pub revision_count: u64,
    pub object_artifact_count: u64,
    pub task_attempt_count: u64,
    pub body_fact_count: u64,
    pub external_body_count: u64,
    pub decoded_body_bytes: u64,
    pub decoded_object_target_bytes: u64,
    pub ordered_events: Vec<LongitudinalEventIdentityV1>,
    pub event_carriers: Vec<LongitudinalEventCarrierV1>,
    pub content_inventory: Vec<LongitudinalInventoryEntryV1>,
    pub removed_content_sha256: Vec<String>,
    pub probe_schedule: Vec<LongitudinalCapacityProbeV1>,
    pub schedule_sha256: String,
    pub manifest_sha256: String,
}

impl LongitudinalCapacityManifestV1 {
    pub fn canonical_sha256(&self) -> Result<String, LongitudinalContractError> {
        canonical_sha256_without(&self.manifest_sha256, |manifest_sha256| {
            let mut preimage = self.clone();
            preimage.manifest_sha256 = manifest_sha256;
            preimage
        })
    }

    pub fn validate(&self) -> Result<(), LongitudinalContractError> {
        let contract = longitudinal_capacity_contract_v1();
        if self.schema != contract.schema
            || self.contract_sha256 != contract.contract_sha256
            || self.public_seed_hex != contract.public_seed_hex
        {
            return Err(LongitudinalContractError::UnsupportedContract);
        }
        self.execution.validate()?;
        let requirement = match self.subject {
            LongitudinalCapacitySubjectV1::V1L100 => {
                let tier = longitudinal_runner_contract_v1()
                    .tiers
                    .into_iter()
                    .find(|candidate| candidate.tier == LongitudinalTierV1::L100)
                    .ok_or(LongitudinalContractError::UnsupportedTier)?;
                LongitudinalCapacityManifestRequirementV1 {
                    event_count: tier.event_count,
                    revision_count: tier.revision_count,
                    object_artifact_count: tier.object_artifact_count,
                    task_attempt_count: tier.task_attempt_count,
                    body_fact_count: tier.body_fact_count,
                    external_body_count: tier.external_body_count,
                    present_content_count: tier.present_content_count,
                    removed_content_count: tier.removed_content_count,
                    decoded_body_bytes: tier.decoded_body_bytes,
                    decoded_object_target_bytes: tier.decoded_object_target_bytes,
                }
            }
            LongitudinalCapacitySubjectV1::Companion(profile) => {
                let profile = contract
                    .profiles
                    .iter()
                    .find(|candidate| candidate.profile == profile)
                    .ok_or(LongitudinalContractError::UnsupportedProfile)?;
                LongitudinalCapacityManifestRequirementV1 {
                    event_count: profile.event_count,
                    revision_count: profile.revision_count,
                    object_artifact_count: profile.object_artifact_count,
                    task_attempt_count: profile.task_attempt_count,
                    body_fact_count: profile.body_fact_count,
                    external_body_count: profile.external_body_count,
                    present_content_count: profile.present_content_count,
                    removed_content_count: profile.removed_content_count,
                    decoded_body_bytes: profile.decoded_body_bytes,
                    decoded_object_target_bytes: profile.decoded_object_target_bytes,
                }
            }
        };
        require_count(
            self.event_count,
            requirement.event_count,
            "capacity event count",
        )?;
        require_count(
            self.revision_count,
            requirement.revision_count,
            "capacity revision count",
        )?;
        require_count(
            self.object_artifact_count,
            requirement.object_artifact_count,
            "capacity object count",
        )?;
        require_count(
            self.task_attempt_count,
            requirement.task_attempt_count,
            "capacity task-attempt count",
        )?;
        require_count(
            self.body_fact_count,
            requirement.body_fact_count,
            "capacity body-fact count",
        )?;
        require_count(
            self.external_body_count,
            requirement.external_body_count,
            "capacity external-body count",
        )?;
        require_count(
            self.decoded_body_bytes,
            requirement.decoded_body_bytes,
            "capacity decoded body bytes",
        )?;
        require_count(
            self.decoded_object_target_bytes,
            requirement.decoded_object_target_bytes,
            "capacity decoded object bytes",
        )?;
        require_len(
            self.content_inventory.len(),
            requirement.present_content_count,
            "capacity content inventory",
        )?;
        require_len(
            self.removed_content_sha256.len(),
            requirement.removed_content_count,
            "capacity removed-content set",
        )?;
        require_len(
            self.ordered_events.len(),
            requirement.event_count,
            "capacity ordered events",
        )?;
        require_len(
            self.event_carriers.len(),
            requirement.event_count,
            "capacity event carriers",
        )?;
        require_exact_sequence(
            self.probe_schedule.iter().copied(),
            LongitudinalCapacityProbeV1::ALL,
            "capacity probe schedule",
        )?;
        require_unique(
            self.ordered_events
                .iter()
                .map(|event| event.event_id.as_str()),
            "capacity ordered event ids",
        )?;
        require_unique(
            self.event_carriers
                .iter()
                .map(|event| event.event_id.as_str()),
            "capacity event carrier ids",
        )?;
        require_unique(
            self.content_inventory
                .iter()
                .map(|content| content.logical_key.as_str()),
            "capacity content logical keys",
        )?;
        for event in &self.ordered_events {
            validate_hex(
                &event.canonical_decoded_sha256,
                64,
                "capacity event SHA-256",
            )?;
        }
        for carrier in &self.event_carriers {
            validate_hex(&carrier.logical_key_sha256, 64, "capacity key SHA-256")?;
            validate_hex(&carrier.raw_sha256, 64, "capacity carrier SHA-256")?;
        }
        for content in &self.content_inventory {
            validate_hex(&content.raw_sha256, 64, "capacity content raw SHA-256")?;
            validate_hex(
                &content.decoded_sha256,
                64,
                "capacity content decoded SHA-256",
            )?;
        }
        for hash in &self.removed_content_sha256 {
            validate_hex(hash, 64, "capacity removed-content SHA-256")?;
        }
        validate_bound_hash(
            &self.schedule_sha256,
            &canonical_sha256(&self.probe_schedule)?,
            "capacity schedule",
        )?;
        validate_bound_hash(
            &self.manifest_sha256,
            &self.canonical_sha256()?,
            "capacity manifest",
        )
    }
}

struct LongitudinalCapacityManifestRequirementV1 {
    event_count: u64,
    revision_count: u64,
    object_artifact_count: u64,
    task_attempt_count: u64,
    body_fact_count: u64,
    external_body_count: u64,
    present_content_count: u64,
    removed_content_count: u64,
    decoded_body_bytes: u64,
    decoded_object_target_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalCapacityMaterializationReceiptV1 {
    pub schema: String,
    pub root_identity: String,
    pub manifest: LongitudinalCapacityManifestV1,
    pub strict: LongitudinalStrictSemanticReceiptV1,
    pub materialization_sha256: String,
}

impl LongitudinalCapacityMaterializationReceiptV1 {
    pub fn canonical_sha256(&self) -> Result<String, LongitudinalContractError> {
        canonical_sha256_without(&self.materialization_sha256, |materialization_sha256| {
            let mut preimage = self.clone();
            preimage.materialization_sha256 = materialization_sha256;
            preimage
        })
    }

    fn pair_identity_sha256(&self) -> Result<String, LongitudinalContractError> {
        let mut preimage = self.clone();
        preimage.root_identity.clear();
        preimage.materialization_sha256.clear();
        canonical_sha256(&preimage)
    }

    pub fn validate(&self) -> Result<(), LongitudinalContractError> {
        if self.schema != LONGITUDINAL_CAPACITY_MATERIALIZATION_RECEIPT_SCHEMA_V1 {
            return Err(LongitudinalContractError::UnsupportedContract);
        }
        validate_hex(&self.root_identity, 64, "capacity root identity")?;
        self.manifest.validate()?;
        validate_strict_receipt(&self.strict)?;
        validate_bound_hash(
            &self.materialization_sha256,
            &self.canonical_sha256()?,
            "capacity materialization receipt",
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalGenerationResourceReceiptV1 {
    pub subject: LongitudinalCapacitySubjectV1,
    pub wall_nanos: u64,
    pub process_cpu_nanos: u64,
    pub encoded_bytes: u64,
    pub allocated_bytes: u64,
    pub high_water_bytes: u64,
    pub receipt_sha256: String,
}

impl LongitudinalGenerationResourceReceiptV1 {
    pub fn canonical_sha256(&self) -> Result<String, LongitudinalContractError> {
        canonical_sha256_without(&self.receipt_sha256, |receipt_sha256| {
            let mut preimage = self.clone();
            preimage.receipt_sha256 = receipt_sha256;
            preimage
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalCapacityReceiptV1 {
    pub schema: String,
    pub subject: LongitudinalCapacitySubjectV1,
    pub probe: LongitudinalCapacityProbeV1,
    pub execution_identity_sha256: String,
    pub manifest_sha256: String,
    pub sample_ordinal: u16,
    pub cold: bool,
    pub output_count: u64,
    pub selected_bytes: u64,
    pub semantic_receipt_sha256: String,
    pub classification: LongitudinalCapacityClassificationV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<LongitudinalOperationMetricsV1>,
    pub receipt_sha256: String,
}

impl LongitudinalCapacityReceiptV1 {
    pub fn canonical_sha256(&self) -> Result<String, LongitudinalContractError> {
        canonical_sha256_without(&self.receipt_sha256, |receipt_sha256| {
            let mut preimage = self.clone();
            preimage.receipt_sha256 = receipt_sha256;
            preimage
        })
    }

    pub fn validate(&self) -> Result<(), LongitudinalContractError> {
        if self.schema != LONGITUDINAL_CAPACITY_RECEIPT_SCHEMA_V1 {
            return Err(LongitudinalContractError::UnsupportedContract);
        }
        validate_hex(
            &self.execution_identity_sha256,
            64,
            "capacity execution identity",
        )?;
        validate_hex(&self.manifest_sha256, 64, "capacity manifest SHA-256")?;
        validate_hex(
            &self.semantic_receipt_sha256,
            64,
            "capacity semantic receipt",
        )?;
        validate_bound_hash(
            &self.receipt_sha256,
            &self.canonical_sha256()?,
            "capacity receipt",
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalCapacityMemoryReceiptV1 {
    pub schema: String,
    pub subject: LongitudinalCapacitySubjectV1,
    pub checkpoint: LongitudinalCapacityMemoryCheckpointV1,
    pub execution_identity_sha256: String,
    pub absolute_rss_bytes: u64,
    pub adjusted_rss_bytes: u64,
    pub vmmap_sha256: String,
    pub ownership: LongitudinalCapacityOwnershipV1,
    pub unknown_ownership: Vec<String>,
    pub receipt_sha256: String,
}

impl LongitudinalCapacityMemoryReceiptV1 {
    pub fn canonical_sha256(&self) -> Result<String, LongitudinalContractError> {
        canonical_sha256_without(&self.receipt_sha256, |receipt_sha256| {
            let mut preimage = self.clone();
            preimage.receipt_sha256 = receipt_sha256;
            preimage
        })
    }

    pub fn validate(&self) -> Result<(), LongitudinalContractError> {
        if self.schema != LONGITUDINAL_CAPACITY_MEMORY_SCHEMA_V1 {
            return Err(LongitudinalContractError::UnsupportedContract);
        }
        validate_hex(
            &self.execution_identity_sha256,
            64,
            "capacity memory execution identity",
        )?;
        validate_hex(&self.vmmap_sha256, 64, "vmmap SHA-256")?;
        validate_bound_hash(
            &self.receipt_sha256,
            &self.canonical_sha256()?,
            "capacity memory receipt",
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalContentionReceiptV1 {
    pub schema: String,
    pub manifest_sha256: String,
    pub writer_attempts: u8,
    pub created: u8,
    pub existing: u8,
    pub conflicts: u8,
    pub reader_cycles: u8,
    pub final_event_count: u64,
    pub strict_replay_sha256: String,
    pub timed_out: bool,
    pub semantic_failure: bool,
    pub receipt_sha256: String,
}

impl LongitudinalContentionReceiptV1 {
    pub fn canonical_sha256(&self) -> Result<String, LongitudinalContractError> {
        canonical_sha256_without(&self.receipt_sha256, |receipt_sha256| {
            let mut preimage = self.clone();
            preimage.receipt_sha256 = receipt_sha256;
            preimage
        })
    }

    pub fn validate(&self) -> Result<(), LongitudinalContractError> {
        if self.schema != "pointbreak.longitudinal-contention-receipt.v1" {
            return Err(LongitudinalContractError::UnsupportedContract);
        }
        validate_hex(&self.manifest_sha256, 64, "contention manifest SHA-256")?;
        validate_hex(&self.strict_replay_sha256, 64, "contention replay SHA-256")?;
        validate_bound_hash(
            &self.receipt_sha256,
            &self.canonical_sha256()?,
            "contention receipt",
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalC524GateInputsV1 {
    pub v1_l100_generation: LongitudinalGenerationResourceReceiptV1,
    pub c262_generation: LongitudinalGenerationResourceReceiptV1,
    pub c262_identity_complete: bool,
    pub c262_base_complete: bool,
    pub c262_counting_complete: bool,
    pub c262_memory_complete: bool,
    pub c262_package_complete: bool,
    pub host_pressure_clear: bool,
    pub free_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalC524GateReceiptV1 {
    pub schema: String,
    pub inputs_sha256: String,
    pub admitted: bool,
    pub reasons: Vec<String>,
    pub receipt_sha256: String,
}

impl LongitudinalC524GateReceiptV1 {
    pub fn canonical_sha256(&self) -> Result<String, LongitudinalContractError> {
        canonical_sha256_without(&self.receipt_sha256, |receipt_sha256| {
            let mut preimage = self.clone();
            preimage.receipt_sha256 = receipt_sha256;
            preimage
        })
    }

    pub fn validate_against(
        &self,
        inputs: &LongitudinalC524GateInputsV1,
    ) -> Result<(), LongitudinalContractError> {
        if self.schema != "pointbreak.longitudinal-c524-gate-receipt.v1" {
            return Err(LongitudinalContractError::UnsupportedContract);
        }
        let expected_inputs = canonical_sha256(inputs)?;
        validate_bound_hash(&self.inputs_sha256, &expected_inputs, "C524 gate inputs")?;
        let expected = evaluate_c524_gate(inputs)?;
        if self.admitted != expected.0 || self.reasons != expected.1 {
            return Err(LongitudinalContractError::ContractDrift {
                field: "C524 gate decision",
            });
        }
        validate_bound_hash(
            &self.receipt_sha256,
            &self.canonical_sha256()?,
            "C524 gate receipt",
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalComplexityCountsV1 {
    pub carrier_opens: u64,
    pub event_decodes: u64,
    pub event_validations: u64,
    pub event_folds: u64,
    pub chronological_sort_items: u64,
    pub rebuilds: u64,
    pub retained_records: u64,
    pub retained_bytes: u64,
}

pub fn classify_longitudinal_capacity_work_v1(
    baseline: &LongitudinalComplexityCountsV1,
    candidate: &LongitudinalComplexityCountsV1,
    output_grew: bool,
    direct_whole_history_work_or_retention: bool,
    surface_supported: bool,
) -> LongitudinalCapacityClassificationV1 {
    if !surface_supported {
        return LongitudinalCapacityClassificationV1::UnsupportedNoCurrentSurface;
    }
    if direct_whole_history_work_or_retention {
        return LongitudinalCapacityClassificationV1::HistoryOrCardinalityProportional;
    }
    if !output_grew
        && baseline
            .as_array()
            .into_iter()
            .zip(candidate.as_array())
            .any(|(baseline, candidate)| exceeds_ratio(baseline, candidate, 12_500))
    {
        return LongitudinalCapacityClassificationV1::HistoryOrCardinalityProportional;
    }
    LongitudinalCapacityClassificationV1::Bounded
}

impl LongitudinalComplexityCountsV1 {
    fn as_array(&self) -> [u64; 8] {
        [
            self.carrier_opens,
            self.event_decodes,
            self.event_validations,
            self.event_folds,
            self.chronological_sort_items,
            self.rebuilds,
            self.retained_records,
            self.retained_bytes,
        ]
    }
}

fn evaluate_c524_gate(
    inputs: &LongitudinalC524GateInputsV1,
) -> Result<(bool, Vec<String>), LongitudinalContractError> {
    let gate = longitudinal_capacity_contract_v1().c524_gate;
    validate_generation_resource(&inputs.v1_l100_generation)?;
    validate_generation_resource(&inputs.c262_generation)?;
    if inputs.v1_l100_generation.subject != LongitudinalCapacitySubjectV1::V1L100
        || inputs.c262_generation.subject
            != LongitudinalCapacitySubjectV1::Companion(LongitudinalCapacityProfileV1::C262)
    {
        return Err(LongitudinalContractError::ContractDrift {
            field: "C524 generation subjects",
        });
    }
    let mut reasons = Vec::new();
    for (complete, reason) in [
        (inputs.c262_identity_complete, "c262_identity_incomplete"),
        (inputs.c262_base_complete, "c262_base_incomplete"),
        (inputs.c262_counting_complete, "c262_counting_incomplete"),
        (inputs.c262_memory_complete, "c262_memory_incomplete"),
        (inputs.c262_package_complete, "c262_package_incomplete"),
        (inputs.host_pressure_clear, "host_pressure_not_clear"),
    ] {
        if !complete {
            reasons.push(reason.to_owned());
        }
    }
    for (baseline, candidate, reason) in [
        (
            inputs.v1_l100_generation.wall_nanos,
            inputs.c262_generation.wall_nanos,
            "generation_wall_ratio_exceeded",
        ),
        (
            inputs.v1_l100_generation.process_cpu_nanos,
            inputs.c262_generation.process_cpu_nanos,
            "generation_cpu_ratio_exceeded",
        ),
        (
            inputs.v1_l100_generation.encoded_bytes,
            inputs.c262_generation.encoded_bytes,
            "encoded_byte_ratio_exceeded",
        ),
        (
            inputs.v1_l100_generation.allocated_bytes,
            inputs.c262_generation.allocated_bytes,
            "allocated_byte_ratio_exceeded",
        ),
        (
            inputs.v1_l100_generation.high_water_bytes,
            inputs.c262_generation.high_water_bytes,
            "high_water_ratio_exceeded",
        ),
    ] {
        if exceeds_ratio(
            baseline,
            candidate,
            gate.maximum_pre_admission_ratio_basis_points,
        ) {
            reasons.push(reason.to_owned());
        }
    }
    let scaled_high_water = u128::from(inputs.c262_generation.high_water_bytes)
        .checked_mul(u128::from(gate.free_space_high_water_basis_points))
        .ok_or(LongitudinalContractError::ContractDrift {
            field: "C524 free-space arithmetic",
        })?
        .div_ceil(10_000);
    let required_free = scaled_high_water
        .checked_add(u128::from(gate.free_space_fixed_bytes))
        .ok_or(LongitudinalContractError::ContractDrift {
            field: "C524 free-space arithmetic",
        })?;
    if u128::from(inputs.free_bytes) < required_free {
        reasons.push("insufficient_free_space".to_owned());
    }
    Ok((reasons.is_empty(), reasons))
}

fn validate_generation_resource(
    receipt: &LongitudinalGenerationResourceReceiptV1,
) -> Result<(), LongitudinalContractError> {
    validate_bound_hash(
        &receipt.receipt_sha256,
        &receipt.canonical_sha256()?,
        "generation resource receipt",
    )
}

fn exceeds_ratio(baseline: u64, candidate: u64, maximum_basis_points: u32) -> bool {
    if baseline == 0 {
        return candidate > 0;
    }
    u128::from(candidate) * 10_000 > u128::from(baseline) * u128::from(maximum_basis_points)
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalRawFileV1 {
    pub relative_path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalFailureReceiptV1 {
    pub reason_code: String,
    pub detail_sha256: String,
    pub immutable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalEvidencePackageV1 {
    pub schema: String,
    pub contract_sha256: String,
    pub base_execution: LongitudinalExecutionIdentityV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derivative_execution: Option<LongitudinalExecutionIdentityV1>,
    pub materializations: Vec<LongitudinalMaterializationReceiptV1>,
    pub operations: Vec<LongitudinalOperationReceiptV1>,
    pub counters: Vec<LongitudinalCounterReceiptV1>,
    pub raw_inventory: Vec<LongitudinalRawFileV1>,
    pub failures: Vec<LongitudinalFailureReceiptV1>,
    pub compensation_applied: bool,
    pub package_sha256: String,
}

impl LongitudinalEvidencePackageV1 {
    pub fn canonical_sha256(&self) -> Result<String, LongitudinalContractError> {
        canonical_sha256_without(&self.package_sha256, |package_sha256| {
            let mut preimage = self.clone();
            preimage.package_sha256 = package_sha256;
            preimage
        })
    }

    pub fn validate(&self) -> Result<(), LongitudinalContractError> {
        let contract = longitudinal_runner_contract_v1();
        if self.schema != LONGITUDINAL_EVIDENCE_PACKAGE_SCHEMA_V1
            || self.contract_sha256 != contract.contract_sha256
        {
            return Err(LongitudinalContractError::UnsupportedContract);
        }
        self.base_execution.validate()?;
        if self.base_execution.parent_commit.is_some() {
            return Err(LongitudinalContractError::MixedRevision);
        }
        let base_hash = self.base_execution.canonical_sha256()?;
        let derivative_hash = match &self.derivative_execution {
            Some(derivative) => {
                derivative.validate()?;
                if derivative.parent_commit.as_deref()
                    != Some(self.base_execution.source_commit.as_str())
                {
                    return Err(LongitudinalContractError::MixedRevision);
                }
                Some(derivative.canonical_sha256()?)
            }
            None => None,
        };
        if self.raw_inventory.is_empty() {
            return Err(LongitudinalContractError::MissingRawInventory);
        }
        if self.compensation_applied {
            return Err(LongitudinalContractError::CompensationForbidden);
        }
        require_unique(
            self.raw_inventory
                .iter()
                .map(|file| file.relative_path.as_str()),
            "raw inventory paths",
        )?;
        for file in &self.raw_inventory {
            validate_relative_path(&file.relative_path)?;
            validate_hex(&file.sha256, 64, "raw file SHA-256")?;
        }
        for receipt in &self.materializations {
            receipt.validate()?;
            if receipt.manifest.execution != self.base_execution {
                return Err(LongitudinalContractError::MixedRevision);
            }
        }
        let mut sample_keys = BTreeSet::new();
        for receipt in &self.operations {
            receipt.validate()?;
            if receipt.execution_identity_sha256 != base_hash {
                return Err(LongitudinalContractError::MixedRevision);
            }
            if !sample_keys.insert((
                receipt.run_identity.clone(),
                receipt.operation,
                receipt.sample_ordinal,
            )) {
                return Err(LongitudinalContractError::DuplicateRow {
                    field: "operation samples",
                });
            }
        }
        let mut counter_keys = BTreeSet::new();
        for receipt in &self.counters {
            receipt.validate()?;
            if receipt.base_execution_identity_sha256 != base_hash
                || derivative_hash.as_deref()
                    != Some(receipt.derivative_execution_identity_sha256.as_str())
            {
                return Err(LongitudinalContractError::MixedRevision);
            }
            if !counter_keys.insert((
                receipt.run_identity.clone(),
                receipt.operation.clone(),
                receipt.phase.clone(),
            )) {
                return Err(LongitudinalContractError::DuplicateRow {
                    field: "counter receipts",
                });
            }
        }
        for failure in &self.failures {
            if !failure.immutable {
                return Err(LongitudinalContractError::MutableFailure);
            }
            require_nonempty(&failure.reason_code, "failure reason")?;
            validate_hex(&failure.detail_sha256, 64, "failure detail SHA-256")?;
        }
        validate_bound_hash(
            &self.package_sha256,
            &self.canonical_sha256()?,
            "longitudinal evidence package",
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalCapacityPackageV1 {
    pub schema: String,
    pub contract_sha256: String,
    pub linked_v1_package_sha256: String,
    pub base_execution: LongitudinalExecutionIdentityV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derivative_execution: Option<LongitudinalExecutionIdentityV1>,
    pub materializations: Vec<LongitudinalCapacityMaterializationReceiptV1>,
    pub generation_resources: Vec<LongitudinalGenerationResourceReceiptV1>,
    pub probe_receipts: Vec<LongitudinalCapacityReceiptV1>,
    pub counter_receipts: Vec<LongitudinalCounterReceiptV1>,
    pub memory_receipts: Vec<LongitudinalCapacityMemoryReceiptV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contention: Option<LongitudinalContentionReceiptV1>,
    pub c524_gate_inputs: LongitudinalC524GateInputsV1,
    pub c524_gate: LongitudinalC524GateReceiptV1,
    pub raw_inventory: Vec<LongitudinalRawFileV1>,
    pub failures: Vec<LongitudinalFailureReceiptV1>,
    pub compensation_applied: bool,
    pub package_sha256: String,
}

impl LongitudinalCapacityPackageV1 {
    pub fn canonical_sha256(&self) -> Result<String, LongitudinalContractError> {
        canonical_sha256_without(&self.package_sha256, |package_sha256| {
            let mut preimage = self.clone();
            preimage.package_sha256 = package_sha256;
            preimage
        })
    }

    pub fn validate(&self) -> Result<(), LongitudinalContractError> {
        let contract = longitudinal_capacity_contract_v1();
        if self.schema != LONGITUDINAL_CAPACITY_PACKAGE_SCHEMA_V1
            || self.contract_sha256 != contract.contract_sha256
        {
            return Err(LongitudinalContractError::UnsupportedContract);
        }
        validate_hex(
            &self.linked_v1_package_sha256,
            64,
            "linked v1 package SHA-256",
        )?;
        self.base_execution.validate()?;
        if self.base_execution.parent_commit.is_some() {
            return Err(LongitudinalContractError::MixedRevision);
        }
        if self.raw_inventory.is_empty() {
            return Err(LongitudinalContractError::MissingRawInventory);
        }
        if self.compensation_applied {
            return Err(LongitudinalContractError::CompensationForbidden);
        }
        let base_hash = self.base_execution.canonical_sha256()?;
        let derivative_hash = match &self.derivative_execution {
            Some(derivative) => {
                derivative.validate()?;
                if derivative.parent_commit.as_deref()
                    != Some(self.base_execution.source_commit.as_str())
                {
                    return Err(LongitudinalContractError::MixedRevision);
                }
                Some(derivative.canonical_sha256()?)
            }
            None => None,
        };
        require_unique(
            self.raw_inventory
                .iter()
                .map(|file| file.relative_path.as_str()),
            "capacity raw inventory paths",
        )?;
        for file in &self.raw_inventory {
            validate_relative_path(&file.relative_path)?;
            validate_hex(&file.sha256, 64, "capacity raw file SHA-256")?;
        }
        for receipt in &self.materializations {
            receipt.validate()?;
            if receipt.manifest.execution != self.base_execution {
                return Err(LongitudinalContractError::MixedRevision);
            }
        }
        let mut generation_subjects = BTreeSet::new();
        for receipt in &self.generation_resources {
            validate_generation_resource(receipt)?;
            if !generation_subjects.insert(receipt.subject) {
                return Err(LongitudinalContractError::DuplicateRow {
                    field: "generation resource subjects",
                });
            }
        }
        let mut probe_keys = BTreeSet::new();
        for receipt in &self.probe_receipts {
            receipt.validate()?;
            if receipt.execution_identity_sha256 != base_hash {
                return Err(LongitudinalContractError::MixedRevision);
            }
            if !probe_keys.insert((receipt.subject, receipt.probe, receipt.sample_ordinal)) {
                return Err(LongitudinalContractError::DuplicateRow {
                    field: "capacity probe samples",
                });
            }
        }
        for receipt in &self.counter_receipts {
            receipt.validate()?;
            if receipt.base_execution_identity_sha256 != base_hash
                || derivative_hash.as_deref()
                    != Some(receipt.derivative_execution_identity_sha256.as_str())
            {
                return Err(LongitudinalContractError::MixedRevision);
            }
        }
        let mut memory_keys = BTreeSet::new();
        for receipt in &self.memory_receipts {
            receipt.validate()?;
            if receipt.execution_identity_sha256 != base_hash {
                return Err(LongitudinalContractError::MixedRevision);
            }
            if !memory_keys.insert((receipt.subject, receipt.checkpoint)) {
                return Err(LongitudinalContractError::DuplicateRow {
                    field: "capacity memory receipts",
                });
            }
        }
        if let Some(contention) = &self.contention {
            contention.validate()?;
        }
        if !self
            .generation_resources
            .contains(&self.c524_gate_inputs.v1_l100_generation)
            || !self
                .generation_resources
                .contains(&self.c524_gate_inputs.c262_generation)
            || self.c524_gate_inputs.c262_identity_complete
                != has_capacity_materialization_pair(
                    &self.materializations,
                    LongitudinalCapacitySubjectV1::Companion(LongitudinalCapacityProfileV1::C262),
                )
        {
            return Err(LongitudinalContractError::ContractDrift {
                field: "C524 package gate inputs",
            });
        }
        self.c524_gate.validate_against(&self.c524_gate_inputs)?;
        for failure in &self.failures {
            if !failure.immutable {
                return Err(LongitudinalContractError::MutableFailure);
            }
        }
        validate_bound_hash(
            &self.package_sha256,
            &self.canonical_sha256()?,
            "longitudinal capacity package",
        )
    }
}

fn has_capacity_materialization_pair(
    receipts: &[LongitudinalCapacityMaterializationReceiptV1],
    subject: LongitudinalCapacitySubjectV1,
) -> bool {
    let mut matching = receipts
        .iter()
        .filter(|receipt| receipt.manifest.subject == subject);
    let (Some(left), Some(right)) = (matching.next(), matching.next()) else {
        return false;
    };
    matching.next().is_none() && verify_longitudinal_capacity_pair_v1(left, right).is_ok()
}

pub fn verify_longitudinal_materialization_pair_v1(
    left: &LongitudinalMaterializationReceiptV1,
    right: &LongitudinalMaterializationReceiptV1,
) -> Result<(), LongitudinalContractError> {
    left.validate()?;
    right.validate()?;
    if left.root_identity == right.root_identity {
        return Err(LongitudinalContractError::RootIdentityNotDistinct);
    }
    if left.pair_identity_sha256()? != right.pair_identity_sha256()? {
        return Err(LongitudinalContractError::PairMismatch);
    }
    Ok(())
}

pub fn verify_longitudinal_capacity_pair_v1(
    left: &LongitudinalCapacityMaterializationReceiptV1,
    right: &LongitudinalCapacityMaterializationReceiptV1,
) -> Result<(), LongitudinalContractError> {
    left.validate()?;
    right.validate()?;
    if left.root_identity == right.root_identity {
        return Err(LongitudinalContractError::RootIdentityNotDistinct);
    }
    if left.pair_identity_sha256()? != right.pair_identity_sha256()? {
        return Err(LongitudinalContractError::PairMismatch);
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalContractPublicationV1 {
    pub schema: String,
    pub mode: String,
    pub workload: LongitudinalRunnerContractV1,
    pub capacity: LongitudinalCapacityContractV1,
    pub publication_sha256: String,
}

impl LongitudinalContractPublicationV1 {
    pub fn canonical_sha256(&self) -> Result<String, LongitudinalContractError> {
        canonical_sha256_without(&self.publication_sha256, |publication_sha256| {
            let mut preimage = self.clone();
            preimage.publication_sha256 = publication_sha256;
            preimage
        })
    }

    pub fn validate(&self) -> Result<(), LongitudinalContractError> {
        if self.schema != LONGITUDINAL_CONTRACT_PUBLICATION_SCHEMA_V1
            || self.mode != "non_timing_contract_publication"
        {
            return Err(LongitudinalContractError::UnsupportedContract);
        }
        self.workload.validate()?;
        self.capacity.validate()?;
        validate_bound_hash(
            &self.publication_sha256,
            LONGITUDINAL_CONTRACT_PUBLICATION_SHA256_V1,
            "longitudinal contract publication",
        )?;
        if self.canonical_sha256()? != LONGITUDINAL_CONTRACT_PUBLICATION_SHA256_V1 {
            return Err(LongitudinalContractError::ContractDrift {
                field: "longitudinal contract publication",
            });
        }
        Ok(())
    }
}

pub fn longitudinal_contract_publication_v1() -> LongitudinalContractPublicationV1 {
    let mut publication = LongitudinalContractPublicationV1 {
        schema: LONGITUDINAL_CONTRACT_PUBLICATION_SCHEMA_V1.to_owned(),
        mode: "non_timing_contract_publication".to_owned(),
        workload: longitudinal_runner_contract_v1(),
        capacity: longitudinal_capacity_contract_v1(),
        publication_sha256: String::new(),
    };
    publication.publication_sha256 = publication
        .canonical_sha256()
        .expect("the frozen longitudinal publication is canonical");
    publication
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalHelpModeV1 {
    pub flag: String,
    pub purpose: String,
    pub constructs_roots: bool,
    pub runs_timing: bool,
    pub reads_private_inputs: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LongitudinalHelpV1 {
    pub schema: String,
    pub modes: Vec<LongitudinalHelpModeV1>,
    pub later_materialization_available: bool,
    pub later_evidence_available: bool,
    pub private_inputs_allowed: bool,
}

pub fn longitudinal_help_v1() -> LongitudinalHelpV1 {
    LongitudinalHelpV1 {
        schema: LONGITUDINAL_HELP_SCHEMA_V1.to_owned(),
        modes: vec![
            LongitudinalHelpModeV1 {
                flag: LONGITUDINAL_CONTRACT_MODE_V1.to_owned(),
                purpose: "print the frozen workload and capacity contracts".to_owned(),
                constructs_roots: false,
                runs_timing: false,
                reads_private_inputs: false,
            },
            LongitudinalHelpModeV1 {
                flag: LONGITUDINAL_HELP_MODE_V1.to_owned(),
                purpose: "describe the available non-timing longitudinal modes".to_owned(),
                constructs_roots: false,
                runs_timing: false,
                reads_private_inputs: false,
            },
        ],
        later_materialization_available: false,
        later_evidence_available: false,
        private_inputs_allowed: false,
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum LongitudinalContractError {
    #[error("unsupported longitudinal contract identity")]
    UnsupportedContract,
    #[error("unsupported longitudinal tier")]
    UnsupportedTier,
    #[error("unsupported longitudinal capacity profile")]
    UnsupportedProfile,
    #[error("{field} is empty")]
    EmptyField { field: &'static str },
    #[error("{field} must be {length} lowercase hexadecimal characters")]
    InvalidHex { field: &'static str, length: usize },
    #[error("{field} hash mismatch")]
    HashMismatch { field: &'static str },
    #[error("{field} count mismatch: expected {expected}, got {actual}")]
    CountMismatch {
        field: &'static str,
        expected: u64,
        actual: u64,
    },
    #[error("duplicate {field}")]
    DuplicateRow { field: &'static str },
    #[error("{field} drifted from the frozen contract")]
    ContractDrift { field: &'static str },
    #[error("materialization roots must be distinct")]
    RootIdentityNotDistinct,
    #[error("materialization pair identity differs")]
    PairMismatch,
    #[error("uninstrumented and attribution lanes were mixed")]
    MixedLane,
    #[error("evidence contains mixed source revisions")]
    MixedRevision,
    #[error("raw evidence inventory is absent")]
    MissingRawInventory,
    #[error("performance or complexity compensation is forbidden")]
    CompensationForbidden,
    #[error("a retained failure was marked mutable")]
    MutableFailure,
    #[error("raw inventory path must be relative and normalized")]
    InvalidRelativePath,
    #[error("canonical JSON failed: {message}")]
    CanonicalJson { message: String },
}

impl LongitudinalContractError {
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::UnsupportedContract => "unsupported_contract",
            Self::UnsupportedTier => "unsupported_tier",
            Self::UnsupportedProfile => "unsupported_profile",
            Self::EmptyField { .. } => "empty_field",
            Self::InvalidHex { .. } => "invalid_hex",
            Self::HashMismatch { .. } => "hash_mismatch",
            Self::CountMismatch { .. } => "count_mismatch",
            Self::DuplicateRow { .. } => "duplicate_row",
            Self::ContractDrift { .. } => "contract_drift",
            Self::RootIdentityNotDistinct => "root_identity_not_distinct",
            Self::PairMismatch => "pair_mismatch",
            Self::MixedLane => "mixed_lane",
            Self::MixedRevision => "mixed_revision",
            Self::MissingRawInventory => "missing_raw_inventory",
            Self::CompensationForbidden => "compensation_forbidden",
            Self::MutableFailure => "mutable_failure",
            Self::InvalidRelativePath => "invalid_relative_path",
            Self::CanonicalJson { .. } => "canonical_json",
        }
    }
}

fn canonical_sha256<T: Serialize>(value: &T) -> Result<String, LongitudinalContractError> {
    let value =
        serde_json::to_value(value).map_err(|error| LongitudinalContractError::CanonicalJson {
            message: error.to_string(),
        })?;
    let bytes =
        canonical_json_bytes(&value).map_err(|error| LongitudinalContractError::CanonicalJson {
            message: error.to_string(),
        })?;
    Ok(sha256_bytes_hex(&bytes))
}

fn canonical_sha256_without<T: Serialize>(
    _current: &str,
    preimage: impl FnOnce(String) -> T,
) -> Result<String, LongitudinalContractError> {
    canonical_sha256(&preimage(String::new()))
}

fn validate_bound_hash(
    provided: &str,
    expected: &str,
    field: &'static str,
) -> Result<(), LongitudinalContractError> {
    validate_hex(provided, 64, field)?;
    if provided != expected {
        return Err(LongitudinalContractError::HashMismatch { field });
    }
    Ok(())
}

fn validate_hex(
    value: &str,
    length: usize,
    field: &'static str,
) -> Result<(), LongitudinalContractError> {
    if value.len() != length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(LongitudinalContractError::InvalidHex { field, length });
    }
    Ok(())
}

fn require_nonempty(value: &str, field: &'static str) -> Result<(), LongitudinalContractError> {
    if value.trim().is_empty() {
        return Err(LongitudinalContractError::EmptyField { field });
    }
    Ok(())
}

fn require_count(
    actual: u64,
    expected: u64,
    field: &'static str,
) -> Result<(), LongitudinalContractError> {
    if actual != expected {
        return Err(LongitudinalContractError::CountMismatch {
            field,
            expected,
            actual,
        });
    }
    Ok(())
}

fn require_len(
    actual: usize,
    expected: u64,
    field: &'static str,
) -> Result<(), LongitudinalContractError> {
    require_count(
        u64::try_from(actual).map_err(|_| LongitudinalContractError::CountMismatch {
            field,
            expected,
            actual: u64::MAX,
        })?,
        expected,
        field,
    )
}

fn require_unique<'a>(
    values: impl IntoIterator<Item = &'a str>,
    field: &'static str,
) -> Result<(), LongitudinalContractError> {
    let mut seen = BTreeSet::new();
    if values.into_iter().any(|value| !seen.insert(value)) {
        return Err(LongitudinalContractError::DuplicateRow { field });
    }
    Ok(())
}

fn require_exact_sequence<T, const N: usize>(
    actual: impl IntoIterator<Item = T>,
    expected: [T; N],
    field: &'static str,
) -> Result<(), LongitudinalContractError>
where
    T: Eq,
{
    if !actual.into_iter().eq(expected) {
        return Err(LongitudinalContractError::ContractDrift { field });
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<(), LongitudinalContractError> {
    let path = std::path::Path::new(path);
    if path.is_absolute()
        || path.as_os_str().is_empty()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(LongitudinalContractError::InvalidRelativePath);
    }
    Ok(())
}

fn scaled_event_families(
    families: &[LongitudinalEventFamilyRequirementV1],
    blocks: u64,
) -> Vec<LongitudinalEventFamilyCountV1> {
    families
        .iter()
        .map(|family| LongitudinalEventFamilyCountV1 {
            event_type: family.event_type.clone(),
            count: family.per_block * blocks,
        })
        .collect()
}

fn validate_strict_receipt(
    receipt: &LongitudinalStrictSemanticReceiptV1,
) -> Result<(), LongitudinalContractError> {
    for (hash, field) in [
        (&receipt.event_set_sha256, "event-set SHA-256"),
        (&receipt.ordered_journal_sha256, "ordered-journal SHA-256"),
        (&receipt.state_sha256, "state SHA-256"),
        (&receipt.projection_sha256, "projection SHA-256"),
        (
            &receipt.content_inventory_sha256,
            "content-inventory SHA-256",
        ),
    ] {
        validate_hex(hash, 64, field)?;
    }
    Ok(())
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    fn digest(label: &str) -> String {
        sha256_bytes_hex(label.as_bytes())
    }

    fn execution() -> LongitudinalExecutionIdentityV1 {
        LongitudinalExecutionIdentityV1 {
            source_commit: "1".repeat(40),
            source_tree: "2".repeat(40),
            cargo_lock_sha256: "3".repeat(64),
            runner_sha256: "4".repeat(64),
            build_profile: "release".to_owned(),
            operating_system: "macos".to_owned(),
            architecture: "aarch64".to_owned(),
            filesystem: "apfs".to_owned(),
            parent_commit: None,
        }
    }

    fn materialization(root_label: &str) -> LongitudinalMaterializationReceiptV1 {
        let contract = longitudinal_runner_contract_v1();
        let tier = contract
            .tiers
            .iter()
            .find(|tier| tier.tier == LongitudinalTierV1::L1)
            .expect("L1 contract");
        let ordered_events = (0..tier.event_count)
            .map(|ordinal| LongitudinalEventIdentityV1 {
                event_id: format!("evt:{ordinal:04}"),
                canonical_decoded_sha256: digest(&format!("event-decoded-{ordinal}")),
            })
            .collect::<Vec<_>>();
        let event_carriers = ordered_events
            .iter()
            .enumerate()
            .map(|(ordinal, event)| LongitudinalEventCarrierV1 {
                event_id: event.event_id.clone(),
                logical_key_sha256: digest(&format!("event-key-{ordinal}")),
                raw_sha256: digest(&format!("event-raw-{ordinal}")),
                raw_bytes: 128,
            })
            .collect::<Vec<_>>();
        let content_inventory = (0..tier.present_content_count)
            .map(|ordinal| LongitudinalInventoryEntryV1 {
                logical_key: format!("content:{ordinal:04}"),
                kind: match ordinal % 3 {
                    0 => LongitudinalContentKindV1::ExternalBody,
                    1 => LongitudinalContentKindV1::ObjectArtifact,
                    _ => LongitudinalContentKindV1::ValidationLog,
                },
                raw_sha256: digest(&format!("content-raw-{ordinal}")),
                decoded_sha256: digest(&format!("content-decoded-{ordinal}")),
                raw_bytes: 256,
                decoded_bytes: 512,
            })
            .collect::<Vec<_>>();
        let schedule = LongitudinalOperationV1::ALL.to_vec();
        let mut manifest = LongitudinalWorkloadManifestV1 {
            schema: contract.schema.clone(),
            protocol: contract.protocol.clone(),
            contract_sha256: contract.contract_sha256.clone(),
            execution: execution(),
            public_seed_hex: contract.public_seed_hex.clone(),
            tier: tier.tier,
            event_count: tier.event_count,
            revision_count: tier.revision_count,
            by_type: scaled_event_families(&contract.event_families, tier.block_count),
            ordered_events,
            event_carriers,
            content_inventory,
            removed_content_sha256: (0..tier.removed_content_count)
                .map(|ordinal| digest(&format!("removed-{ordinal}")))
                .collect(),
            expected_semantic_receipts: LongitudinalOperationV1::ALL
                .into_iter()
                .map(|operation| LongitudinalExpectedSemanticReceiptV1 {
                    operation,
                    semantic_receipt_sha256: digest(&format!("semantic-{operation:?}")),
                })
                .collect(),
            schedule_sha256: canonical_sha256(&schedule).expect("schedule hash"),
            schedule,
            manifest_sha256: String::new(),
        };
        manifest.manifest_sha256 = manifest.canonical_sha256().expect("manifest hash");
        let mut receipt = LongitudinalMaterializationReceiptV1 {
            schema: LONGITUDINAL_MATERIALIZATION_RECEIPT_SCHEMA_V1.to_owned(),
            root_identity: digest(root_label),
            manifest,
            strict: LongitudinalStrictSemanticReceiptV1 {
                event_set_sha256: digest("event-set"),
                ordered_journal_sha256: digest("ordered-journal"),
                state_sha256: digest("state"),
                projection_sha256: digest("projection"),
                content_inventory_sha256: digest("content-inventory"),
            },
            materialization_sha256: String::new(),
        };
        receipt.materialization_sha256 = receipt.canonical_sha256().expect("materialization hash");
        receipt
    }

    fn rebind(receipt: &mut LongitudinalMaterializationReceiptV1) {
        receipt.manifest.manifest_sha256 =
            receipt.manifest.canonical_sha256().expect("manifest hash");
        receipt.materialization_sha256 = receipt.canonical_sha256().expect("materialization hash");
    }

    fn generation(
        subject: LongitudinalCapacitySubjectV1,
        scalar: u64,
    ) -> LongitudinalGenerationResourceReceiptV1 {
        let mut receipt = LongitudinalGenerationResourceReceiptV1 {
            subject,
            wall_nanos: scalar,
            process_cpu_nanos: scalar,
            encoded_bytes: scalar,
            allocated_bytes: scalar,
            high_water_bytes: scalar,
            receipt_sha256: String::new(),
        };
        receipt.receipt_sha256 = receipt.canonical_sha256().expect("generation hash");
        receipt
    }

    fn capacity_manifest_header() -> LongitudinalCapacityManifestV1 {
        let contract = longitudinal_capacity_contract_v1();
        let profile = contract
            .profiles
            .iter()
            .find(|profile| profile.profile == LongitudinalCapacityProfileV1::C262)
            .expect("C262 contract");
        let probe_schedule = LongitudinalCapacityProbeV1::ALL.to_vec();
        LongitudinalCapacityManifestV1 {
            schema: contract.schema,
            contract_sha256: contract.contract_sha256,
            execution: execution(),
            public_seed_hex: contract.public_seed_hex,
            subject: LongitudinalCapacitySubjectV1::Companion(LongitudinalCapacityProfileV1::C262),
            event_count: profile.event_count,
            revision_count: profile.revision_count,
            object_artifact_count: profile.object_artifact_count,
            task_attempt_count: profile.task_attempt_count,
            body_fact_count: profile.body_fact_count,
            external_body_count: profile.external_body_count,
            decoded_body_bytes: profile.decoded_body_bytes,
            decoded_object_target_bytes: profile.decoded_object_target_bytes,
            ordered_events: Vec::new(),
            event_carriers: Vec::new(),
            content_inventory: Vec::new(),
            removed_content_sha256: Vec::new(),
            schedule_sha256: canonical_sha256(&probe_schedule).expect("probe schedule hash"),
            probe_schedule,
            manifest_sha256: String::new(),
        }
    }

    fn operation_receipt(
        lane: LongitudinalLaneV1,
        execution_identity_sha256: String,
        sample_ordinal: u32,
    ) -> LongitudinalOperationReceiptV1 {
        let mut receipt = LongitudinalOperationReceiptV1 {
            schema: LONGITUDINAL_OPERATION_RECEIPT_SCHEMA_V1.to_owned(),
            run_identity: digest("operation-run"),
            root_identity: digest("operation-root"),
            tier: LongitudinalTierV1::L1,
            manifest_sha256: digest("operation-manifest"),
            schedule_sha256: digest("operation-schedule"),
            execution_identity_sha256,
            lane,
            operation: LongitudinalOperationV1::WarmHead,
            sample_ordinal,
            pre_event_count: 1_024,
            post_event_count: 1_024,
            pre_event_set_sha256: digest("pre-set"),
            post_event_set_sha256: digest("post-set"),
            response_bytes: 1,
            response_canonical_sha256: digest("response"),
            semantic_facts: LongitudinalSemanticFactsV1 {
                ordered_ids: vec!["evt:0000".to_owned()],
                fact_ids: Vec::new(),
                diagnostics: Vec::new(),
                event_count: 1_024,
                event_set_sha256: digest("semantic-event-set"),
                content_state_sha256: digest("semantic-content-state"),
            },
            semantic_receipt_sha256: digest("operation-semantic"),
            metrics: LongitudinalOperationMetricsV1 {
                wall_nanos: 1,
                process_user_cpu_nanos: 1,
                process_system_cpu_nanos: 1,
                baseline_rss_bytes: 1,
                peak_rss_bytes: 1,
                steady_rss_bytes: 1,
            },
            receipt_sha256: String::new(),
        };
        receipt.receipt_sha256 = receipt.canonical_sha256().expect("operation hash");
        receipt
    }

    fn empty_package() -> LongitudinalEvidencePackageV1 {
        let contract = longitudinal_runner_contract_v1();
        let mut package = LongitudinalEvidencePackageV1 {
            schema: LONGITUDINAL_EVIDENCE_PACKAGE_SCHEMA_V1.to_owned(),
            contract_sha256: contract.contract_sha256,
            base_execution: execution(),
            derivative_execution: None,
            materializations: Vec::new(),
            operations: Vec::new(),
            counters: Vec::new(),
            raw_inventory: vec![LongitudinalRawFileV1 {
                relative_path: "raw/sample.json".to_owned(),
                sha256: digest("sample"),
                bytes: 1,
            }],
            failures: Vec::new(),
            compensation_applied: false,
            package_sha256: String::new(),
        };
        package.package_sha256 = package.canonical_sha256().expect("package hash");
        package
    }

    #[test]
    fn longitudinal_contract_freezes_workload_matrix_samples_and_envelopes() {
        let contract = longitudinal_runner_contract_v1();
        contract.validate().expect("frozen contract validates");

        assert_eq!(contract.block_event_count, 256);
        assert_eq!(contract.history_page_size, 100);
        assert_eq!(contract.append_batch_size, 30);
        assert_eq!(contract.operation_schedule, LongitudinalOperationV1::ALL);
        assert_eq!(contract.samples.process_cold_per_release_root, 10);
        assert_eq!(contract.samples.excluded_warmups, 3);
        assert_eq!(contract.samples.warm_samples_per_release_root, 30);
        assert_eq!(contract.samples.append_one_samples_per_release_root, 30);
        assert_eq!(contract.samples.append_burst_samples_per_release_root, 10);
        assert_eq!(contract.samples.maintenance_samples_per_release_root, 3);
        assert_eq!(contract.samples.nearest_rank_percentile, 95);
        assert!(contract.samples.retain_all_samples);
        assert!(!contract.samples.outlier_removal_allowed);
        assert_eq!(contract.run_matrix[0].roots_per_tier, 2);
        assert!(contract.run_matrix[1].records_timing);
        assert!(!contract.run_matrix[1].performance_comparison_admissible);
        assert!(contract.run_matrix[2].counters_admissible);
        assert!(!contract.run_matrix[2].records_timing);
        assert_eq!(contract.interaction_envelopes.len(), 12);
        assert_eq!(contract.counter_envelopes.len(), 6);
        assert_eq!(contract.memory_envelope.checkpoints.len(), 7);
    }

    #[test]
    fn longitudinal_capacity_manifest_binds_frozen_counts_and_content_cardinality() {
        let mut manifest = capacity_manifest_header();
        manifest.body_fact_count -= 1;
        assert_eq!(
            manifest.validate(),
            Err(LongitudinalContractError::CountMismatch {
                field: "capacity body-fact count",
                expected: 184_320,
                actual: 184_319,
            })
        );

        manifest = capacity_manifest_header();
        manifest.decoded_body_bytes -= 1;
        assert_eq!(
            manifest.validate(),
            Err(LongitudinalContractError::CountMismatch {
                field: "capacity decoded body bytes",
                expected: 1_509_949_440,
                actual: 1_509_949_439,
            })
        );

        manifest = capacity_manifest_header();
        assert_eq!(
            manifest.validate(),
            Err(LongitudinalContractError::CountMismatch {
                field: "capacity content inventory",
                expected: 106_496,
                actual: 0,
            })
        );
    }

    #[test]
    fn longitudinal_counters_have_exact_stable_fields_and_no_metric_slots() {
        let value = serde_json::to_value(LongitudinalCountersV1::default()).expect("counter JSON");
        assert_eq!(
            value
                .as_object()
                .expect("counter object")
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec![
                "bodyArtifactReads",
                "bodyBytesRead",
                "carrierBytesRead",
                "carrierOpens",
                "chronologicalSortItems",
                "directoryEntriesWalked",
                "eventDecodes",
                "eventFolds",
                "eventValidations",
                "objectArtifactReads",
                "objectBytesRead",
                "projectionRebuilds",
                "responseBytes",
                "stateRebuilds",
            ]
        );

        let mut receipt = LongitudinalCounterReceiptV1 {
            schema: LONGITUDINAL_COUNTER_RECEIPT_SCHEMA_V1.to_owned(),
            run_identity: digest("run"),
            root_identity: digest("root"),
            operation: "WARM_HEAD".to_owned(),
            phase: "warm".to_owned(),
            base_execution_identity_sha256: digest("base"),
            derivative_execution_identity_sha256: digest("derivative"),
            manifest_sha256: digest("manifest"),
            schedule_sha256: digest("schedule"),
            success: true,
            semantic_result_sha256: digest("semantic"),
            counters: LongitudinalCountersV1::default(),
            capacity_ownership: None,
            receipt_sha256: String::new(),
        };
        receipt.receipt_sha256 = receipt.canonical_sha256().expect("counter hash");
        receipt.validate().expect("counter receipt validates");
        let mut value = serde_json::to_value(receipt).expect("receipt JSON");
        value
            .as_object_mut()
            .expect("receipt object")
            .insert("wallNanos".to_owned(), 1_u64.into());
        assert!(serde_json::from_value::<LongitudinalCounterReceiptV1>(value).is_err());
    }

    #[test]
    fn longitudinal_canonical_hashes_ignore_only_their_own_slot() {
        let contract = longitudinal_runner_contract_v1();
        let canonical = contract.canonical_sha256().expect("contract hash");
        let mut own_hash_changed = contract.clone();
        own_hash_changed.contract_sha256 = "f".repeat(64);
        assert_eq!(
            own_hash_changed.canonical_sha256().expect("contract hash"),
            canonical
        );

        let mut governed_field_changed = contract;
        governed_field_changed.history_page_size += 1;
        assert_ne!(
            governed_field_changed
                .canonical_sha256()
                .expect("contract hash"),
            canonical
        );

        let capacity = longitudinal_capacity_contract_v1();
        let capacity_hash = capacity.canonical_sha256().expect("capacity hash");
        let mut capacity_changed = capacity;
        capacity_changed.contention.reader_cycles += 1;
        assert_ne!(
            capacity_changed.canonical_sha256().expect("capacity hash"),
            capacity_hash
        );
    }

    #[test]
    fn longitudinal_pair_verifier_accepts_distinct_identical_roots() {
        let left = materialization("root-left");
        let mut right = left.clone();
        right.root_identity = digest("root-right");
        right.materialization_sha256 = right.canonical_sha256().expect("right hash");

        verify_longitudinal_materialization_pair_v1(&left, &right)
            .expect("the two roots have identical canonical materialization");
    }

    #[test]
    fn longitudinal_pair_verifier_rejects_carrier_order_and_receipt_drift() {
        let left = materialization("root-left");

        let mut carrier_drift = left.clone();
        carrier_drift.root_identity = digest("root-carrier-drift");
        carrier_drift.manifest.event_carriers[0].raw_sha256 = digest("one-byte-drift");
        rebind(&mut carrier_drift);
        assert_eq!(
            verify_longitudinal_materialization_pair_v1(&left, &carrier_drift),
            Err(LongitudinalContractError::PairMismatch)
        );

        let mut order_drift = left.clone();
        order_drift.root_identity = digest("root-order-drift");
        order_drift.manifest.ordered_events.swap(0, 1);
        rebind(&mut order_drift);
        assert_eq!(
            verify_longitudinal_materialization_pair_v1(&left, &order_drift),
            Err(LongitudinalContractError::PairMismatch)
        );

        let mut receipt_drift = left.clone();
        receipt_drift.root_identity = digest("root-receipt-drift");
        receipt_drift.strict.state_sha256 = digest("state-drift");
        rebind(&mut receipt_drift);
        assert_eq!(
            verify_longitudinal_materialization_pair_v1(&left, &receipt_drift),
            Err(LongitudinalContractError::PairMismatch)
        );

        let mut content_drift = left.clone();
        content_drift.root_identity = digest("root-content-drift");
        content_drift.manifest.content_inventory[0].raw_sha256 = digest("content-byte-drift");
        rebind(&mut content_drift);
        assert_eq!(
            verify_longitudinal_materialization_pair_v1(&left, &content_drift),
            Err(LongitudinalContractError::PairMismatch)
        );

        let mut source_drift = left.clone();
        source_drift.root_identity = digest("root-source-drift");
        source_drift.manifest.execution.runner_sha256 = digest("other-runner");
        rebind(&mut source_drift);
        assert_eq!(
            verify_longitudinal_materialization_pair_v1(&left, &source_drift),
            Err(LongitudinalContractError::PairMismatch)
        );
    }

    #[test]
    fn longitudinal_manifest_rejects_missing_duplicate_and_schedule_rows() {
        let receipt = materialization("root");

        let mut missing = receipt.manifest.clone();
        missing.ordered_events.pop();
        missing.manifest_sha256 = missing.canonical_sha256().expect("missing hash");
        assert!(matches!(
            missing.validate(),
            Err(LongitudinalContractError::CountMismatch { .. })
        ));

        let mut duplicate = receipt.manifest.clone();
        duplicate.ordered_events[1].event_id = duplicate.ordered_events[0].event_id.clone();
        duplicate.manifest_sha256 = duplicate.canonical_sha256().expect("duplicate hash");
        assert_eq!(
            duplicate.validate(),
            Err(LongitudinalContractError::DuplicateRow {
                field: "ordered event ids"
            })
        );

        let mut schedule = receipt.manifest.clone();
        schedule.schedule.pop();
        schedule.schedule_sha256 = canonical_sha256(&schedule.schedule).expect("schedule hash");
        schedule.manifest_sha256 = schedule.canonical_sha256().expect("manifest hash");
        assert_eq!(
            schedule.validate(),
            Err(LongitudinalContractError::ContractDrift {
                field: "manifest schedule"
            })
        );
    }

    #[test]
    fn longitudinal_non_compensation_classifies_shape_before_wall_time() {
        let baseline = LongitudinalComplexityCountsV1 {
            carrier_opens: 100,
            ..LongitudinalComplexityCountsV1::default()
        };
        let at_limit = LongitudinalComplexityCountsV1 {
            carrier_opens: 125,
            ..LongitudinalComplexityCountsV1::default()
        };
        let above_limit = LongitudinalComplexityCountsV1 {
            carrier_opens: 126,
            ..LongitudinalComplexityCountsV1::default()
        };
        assert_eq!(
            classify_longitudinal_capacity_work_v1(&baseline, &at_limit, false, false, true),
            LongitudinalCapacityClassificationV1::Bounded
        );
        assert_eq!(
            classify_longitudinal_capacity_work_v1(&baseline, &above_limit, false, false, true),
            LongitudinalCapacityClassificationV1::HistoryOrCardinalityProportional
        );
        assert_eq!(
            classify_longitudinal_capacity_work_v1(&baseline, &baseline, false, true, true),
            LongitudinalCapacityClassificationV1::HistoryOrCardinalityProportional
        );
        assert_eq!(
            classify_longitudinal_capacity_work_v1(&baseline, &baseline, false, false, false),
            LongitudinalCapacityClassificationV1::UnsupportedNoCurrentSurface
        );
    }

    #[test]
    fn longitudinal_c524_gate_is_typed_and_has_no_manual_override() {
        let baseline = generation(LongitudinalCapacitySubjectV1::V1L100, 100);
        let c262 = generation(
            LongitudinalCapacitySubjectV1::Companion(LongitudinalCapacityProfileV1::C262),
            256,
        );
        let inputs = LongitudinalC524GateInputsV1 {
            v1_l100_generation: baseline,
            c262_generation: c262,
            c262_identity_complete: true,
            c262_base_complete: true,
            c262_counting_complete: true,
            c262_memory_complete: true,
            c262_package_complete: true,
            host_pressure_clear: true,
            free_bytes: 17 * GIB,
        };
        let (admitted, reasons) = evaluate_c524_gate(&inputs).expect("gate evaluation");
        assert!(admitted);
        assert!(reasons.is_empty());

        let mut receipt = LongitudinalC524GateReceiptV1 {
            schema: "pointbreak.longitudinal-c524-gate-receipt.v1".to_owned(),
            inputs_sha256: canonical_sha256(&inputs).expect("input hash"),
            admitted,
            reasons,
            receipt_sha256: String::new(),
        };
        receipt.receipt_sha256 = receipt.canonical_sha256().expect("gate hash");
        receipt
            .validate_against(&inputs)
            .expect("gate receipt is bound to its inputs");

        let mut pressure = inputs;
        pressure.host_pressure_clear = false;
        assert_eq!(
            evaluate_c524_gate(&pressure).expect("pressure gate").1,
            vec!["host_pressure_not_clear"]
        );
        assert!(
            !longitudinal_capacity_contract_v1()
                .c524_gate
                .manual_override_allowed
        );
    }

    #[test]
    fn longitudinal_package_rejects_missing_inventory_compensation_and_mutable_failure() {
        let mut package = empty_package();
        package.raw_inventory.clear();
        package.package_sha256 = package.canonical_sha256().expect("package hash");
        assert_eq!(
            package.validate(),
            Err(LongitudinalContractError::MissingRawInventory)
        );

        package = empty_package();
        package.compensation_applied = true;
        package.package_sha256 = package.canonical_sha256().expect("package hash");
        assert_eq!(
            package.validate(),
            Err(LongitudinalContractError::CompensationForbidden)
        );

        package.compensation_applied = false;
        package.failures.push(LongitudinalFailureReceiptV1 {
            reason_code: "valid_failure".to_owned(),
            detail_sha256: digest("failure"),
            immutable: false,
        });
        package.package_sha256 = package.canonical_sha256().expect("package hash");
        assert_eq!(
            package.validate(),
            Err(LongitudinalContractError::MutableFailure)
        );
    }

    #[test]
    fn longitudinal_package_rejects_mixed_revisions_duplicate_samples_and_lane_pooling() {
        let mut package = empty_package();
        let base_hash = package
            .base_execution
            .canonical_sha256()
            .expect("base identity hash");
        let sample = operation_receipt(LongitudinalLaneV1::ReleaseUninstrumented, base_hash, 0);
        package.operations = vec![sample.clone(), sample];
        package.package_sha256 = package.canonical_sha256().expect("package hash");
        assert_eq!(
            package.validate(),
            Err(LongitudinalContractError::DuplicateRow {
                field: "operation samples"
            })
        );

        let mut mixed = empty_package();
        mixed.operations.push(operation_receipt(
            LongitudinalLaneV1::ReleaseUninstrumented,
            digest("other-execution"),
            0,
        ));
        mixed.package_sha256 = mixed.canonical_sha256().expect("package hash");
        assert_eq!(
            mixed.validate(),
            Err(LongitudinalContractError::MixedRevision)
        );

        let attribution_timing = operation_receipt(
            LongitudinalLaneV1::AttributionCounts,
            digest("execution"),
            0,
        );
        assert_eq!(
            attribution_timing.validate(),
            Err(LongitudinalContractError::MixedLane)
        );

        let mut serialized = serde_json::to_value(empty_package()).expect("package JSON");
        serialized
            .as_object_mut()
            .expect("package object")
            .insert("capacityProbeReceipts".to_owned(), serde_json::json!([]));
        assert!(serde_json::from_value::<LongitudinalEvidencePackageV1>(serialized).is_err());
    }

    #[test]
    fn longitudinal_manifests_cannot_serialize_root_paths() {
        let mut serialized =
            serde_json::to_value(materialization("root").manifest).expect("manifest JSON");
        serialized
            .as_object_mut()
            .expect("manifest object")
            .insert("rootPath".to_owned(), "/tmp/root".into());
        assert!(serde_json::from_value::<LongitudinalWorkloadManifestV1>(serialized).is_err());
    }

    #[test]
    fn longitudinal_contract_fixtures_and_publication_are_byte_stable() {
        let workload = longitudinal_runner_contract_v1();
        let capacity = longitudinal_capacity_contract_v1();
        let workload_fixture = include_str!(
            "../../../tests/fixtures/store-foundation/longitudinal-runner-contract-v1.json"
        );
        let capacity_fixture = include_str!(
            "../../../tests/fixtures/store-foundation/longitudinal-capacity-sentinel-v1.json"
        );
        let expected_workload = format!(
            "{}\n",
            serde_json::to_string_pretty(&workload).expect("workload fixture JSON")
        );
        let expected_capacity = format!(
            "{}\n",
            serde_json::to_string_pretty(&capacity).expect("capacity fixture JSON")
        );

        assert_eq!(normalize_lf(workload_fixture), expected_workload);
        assert_eq!(normalize_lf(capacity_fixture), expected_capacity);
        assert_eq!(
            normalize_lf(&expected_workload.replace('\n', "\r\n")),
            expected_workload
        );
        assert_eq!(
            normalize_lf(&expected_capacity.replace('\n', "\r\n")),
            expected_capacity
        );

        let attributes = include_str!("../../../.gitattributes");
        assert!(attributes.contains(
            "tests/fixtures/store-foundation/longitudinal-runner-contract-v1.json text eol=lf"
        ));
        assert!(attributes.contains(
            "tests/fixtures/store-foundation/longitudinal-capacity-sentinel-v1.json text eol=lf"
        ));

        let publication = longitudinal_contract_publication_v1();
        publication.validate().expect("publication validates");
        let bytes = serde_json::to_string(&publication).expect("publication JSON");
        assert!(!bytes.contains('\n'));
        assert!(!bytes.contains('\r'));
        assert_eq!(
            publication.publication_sha256,
            LONGITUDINAL_CONTRACT_PUBLICATION_SHA256_V1
        );
    }

    fn normalize_lf(value: &str) -> String {
        value.replace("\r\n", "\n")
    }
}
