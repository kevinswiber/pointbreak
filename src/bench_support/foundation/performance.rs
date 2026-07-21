use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::fault::{baseline_inventory, baseline_record_path, populate_profile, read_all_files};
use super::{
    QUALIFICATION_EXTERNAL_WORKLOAD_MANIFEST_SHA256_V2, QUALIFICATION_G0_MANIFEST_SHA256_V1,
    QUALIFICATION_G0_SCHEDULE_SHA256_V1, QUALIFICATION_G0_SPEC_SHA256_V1,
    QUALIFICATION_G1_MANIFEST_SHA256_V1, QUALIFICATION_G1_SCHEDULE_SHA256_V1,
    QUALIFICATION_G1_SPEC_SHA256_V1, QUALIFICATION_G2_MANIFEST_SHA256_V1,
    QUALIFICATION_G2_SCHEDULE_SHA256_V1, QUALIFICATION_G2_SPEC_SHA256_V1,
    QUALIFICATION_GENERATOR_SCHEMA_V1, QUALIFICATION_PUBLIC_SEED_HEX_V1, QualificationCandidateV1,
    QualificationCorpusManifestV1, QualificationFilesystemDispositionV1,
    QualificationGeneratedWorkloadV1, QualificationInventoryV1, QualificationKeyedReadClassV1,
    QualificationPlatformEnvironmentV1, QualificationProfile, QualificationRawSampleV1,
    QualificationRecordKindV1, SEGMENT_QUALIFICATION_PROFILE_ID_V1,
    SQLITE_QUALIFICATION_PROFILE_ID_V1, SegmentQualificationProfile, SqliteQualificationProfile,
    classify_qualification_filesystem, load_external_workload_v2_manifest_from_path,
    modeled_post_foundation_manifest, qualification_cargo_lock_sha256,
    qualification_filesystem_name, qualification_generated_manifest_v1,
    qualification_generator_spec_v1, qualification_operation_schedule_v1,
    qualification_source_commit, synthetic_legacy_manifest,
};
use crate::canonical_hash::{canonical_json_bytes, sha256_bytes_hex};

pub const QUALIFICATION_PERFORMANCE_DIAGNOSTICS_SCHEMA_V1: &str =
    "pointbreak.qualification-performance-diagnostics.v1";
pub const QUALIFICATION_PERFORMANCE_DIAGNOSTIC_CONTRACT_SCHEMA_V1: &str =
    "pointbreak.qualification-performance-diagnostic-contract.v1";
pub const QUALIFICATION_PERFORMANCE_CONTRACT_SCHEMA_V2: &str =
    "pointbreak.qualification-performance-contract.v2";
pub const QUALIFICATION_PERFORMANCE_EVIDENCE_SCHEMA_V2: &str =
    "pointbreak.qualification-performance-evidence.v2";
pub const QUALIFICATION_PERFORMANCE_EVALUATION_SCHEMA_V2: &str =
    "pointbreak.qualification-performance-evaluation.v2";
pub const QUALIFICATION_PERFORMANCE_PACKAGE_SCHEMA_V2: &str =
    "pointbreak.qualification-performance-package.v2";
pub const QUALIFICATION_PERFORMANCE_CONTRACT_PUBLICATION_SCHEMA_V2: &str =
    "pointbreak.qualification-performance-contract-publication.v2";
pub const QUALIFICATION_PERFORMANCE_CONTRACT_SHA256_V2: &str =
    "55c473f448c80ba26e5e0eeaf23ebdd7c7b3827954bfec78873d1fd839a54a36";
pub const QUALIFICATION_LOOSE_BASELINE_EVIDENCE_MODE_V1: &str = "--loose-baseline-evidence";
pub const QUALIFICATION_LOOSE_BASELINE_SMOKE_MODE_V1: &str = "--loose-baseline-smoke";
pub const QUALIFICATION_LOOSE_BASELINE_EVIDENCE_SCHEMA_V1: &str =
    "pointbreak.qualification-loose-baseline-evidence.v1";
pub const QUALIFICATION_LOOSE_BASELINE_SMOKE_SCHEMA_V1: &str =
    "pointbreak.qualification-loose-baseline-smoke.v1";
pub const QUALIFICATION_PROSPECTIVE_CONTRACT_PROPOSAL_SHAPE_SCHEMA_V1: &str =
    "pointbreak.qualification-prospective-contract-proposal-shape.v1";
pub const QUALIFICATION_PROSPECTIVE_CONTRACT_SCHEMA_V1: &str =
    "pointbreak.qualification-prospective-feasibility-contract.v1";
pub const QUALIFICATION_PROSPECTIVE_EVIDENCE_SCHEMA_V1: &str =
    "pointbreak.qualification-prospective-feasibility-evidence.v1";
pub const QUALIFICATION_PROSPECTIVE_EVALUATION_SCHEMA_V1: &str =
    "pointbreak.qualification-prospective-feasibility-evaluation.v1";
pub const QUALIFICATION_PROSPECTIVE_CONTRACT_PUBLICATION_SCHEMA_V1: &str =
    "pointbreak.qualification-prospective-feasibility-contract-publication.v1";
pub const QUALIFICATION_PROSPECTIVE_CONTRACT_PUBLICATION_MODE_V1: &str = "--prospective-contract";
pub const QUALIFICATION_PROSPECTIVE_CONTRACT_PROPOSAL_SHA256_V1: &str =
    "83446c8a40eb71fa4696ee5d71043c47beb8624fc97e2360b62337e489ad67e8";
pub const QUALIFICATION_PROSPECTIVE_CONTRACT_SHA256_V1: &str =
    "8e9fb5bffef230d97d3f4abc8a70c79958e4372668af8bde19b3aa815382857d";

const QUALIFICATION_LOOSE_BASELINE_WARMUP_ITERATIONS_V1: u32 = 3;
const QUALIFICATION_LOOSE_BASELINE_MEASURED_ITERATIONS_V1: u32 = 30;
const QUALIFICATION_LOOSE_BASELINE_INDEPENDENT_ROOTS_V1: u32 = 2;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationLooseBaselineOperationV1 {
    DurableAppend,
    StrictReplay,
    FreshProcessOpenRecovery,
    KeyedRead,
}

impl QualificationLooseBaselineOperationV1 {
    pub const ALL: [Self; 4] = [
        Self::DurableAppend,
        Self::StrictReplay,
        Self::FreshProcessOpenRecovery,
        Self::KeyedRead,
    ];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationLooseBaselineMeasurementScopeV1 {
    Diagnostic,
    Baseline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationLooseBaselineSampleRetentionV1 {
    Raw,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationLooseBaselineRunControlsV1 {
    pub warmup_iterations: u32,
    pub measured_iterations: u32,
    pub independent_roots: u32,
    pub sample_retention: QualificationLooseBaselineSampleRetentionV1,
    pub outlier_policy: QualificationPerformanceOutlierPolicyV2,
}

impl QualificationLooseBaselineRunControlsV1 {
    pub fn fixed() -> Self {
        Self {
            warmup_iterations: QUALIFICATION_LOOSE_BASELINE_WARMUP_ITERATIONS_V1,
            measured_iterations: QUALIFICATION_LOOSE_BASELINE_MEASURED_ITERATIONS_V1,
            independent_roots: QUALIFICATION_LOOSE_BASELINE_INDEPENDENT_ROOTS_V1,
            sample_retention: QualificationLooseBaselineSampleRetentionV1::Raw,
            outlier_policy: QualificationPerformanceOutlierPolicyV2::RetainAll,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationLooseBaselineReceiptKindV1 {
    DurableAppendVisibleExact,
    StrictReplayExact,
    FreshProcessOpenExact,
    KeyedReadPresentExact,
    KeyedReadAbsentExact,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationLooseBaselineSemanticReceiptV1 {
    pub kind: QualificationLooseBaselineReceiptKindV1,
    pub record_count: u64,
    pub logical_byte_count: u64,
    pub aggregate_receipt_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationLooseBaselineSampleV1 {
    pub operation: QualificationLooseBaselineOperationV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_class: Option<QualificationKeyedReadClassV1>,
    pub iteration: u32,
    pub elapsed_nanos: u64,
    pub receipt: QualificationLooseBaselineSemanticReceiptV1,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationLooseBaselineInventoryV1 {
    pub carrier_count: u64,
    pub carrier_set_sha256: String,
    pub logical_bytes: u64,
    pub encoded_bytes: u64,
    pub allocated_bytes: u64,
    pub high_water_bytes: u64,
}

impl From<QualificationPerformanceInventoryV2> for QualificationLooseBaselineInventoryV1 {
    fn from(inventory: QualificationPerformanceInventoryV2) -> Self {
        Self {
            carrier_count: inventory.carrier_count,
            carrier_set_sha256: inventory.carrier_set_sha256,
            logical_bytes: inventory.logical_bytes,
            encoded_bytes: inventory.encoded_bytes,
            allocated_bytes: inventory.allocated_bytes,
            high_water_bytes: inventory.high_water_bytes,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationLooseBaselineAllocationSnapshotV1 {
    pub scope: QualificationPerformanceAllocationScopeV2,
    pub state: QualificationPerformanceInventoryStateV1,
    pub inventory: QualificationLooseBaselineInventoryV1,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationLooseBaselinePlatformV1 {
    pub operating_system: String,
    pub operating_system_version: String,
    pub architecture: String,
    pub cpu: String,
    pub filesystem: String,
    pub allocation_api: String,
    pub rustc: String,
    pub build_source: String,
    pub build_describe: String,
    pub source_tree_clean: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationLooseBaselineRunV1 {
    pub run_index: u32,
    pub run_identity: String,
    pub workload: QualificationGeneratedWorkloadV1,
    pub measurement_scope: QualificationLooseBaselineMeasurementScopeV1,
    pub generator_spec_sha256: String,
    pub manifest_sha256: String,
    pub schedule_sha256: String,
    pub controls: QualificationLooseBaselineRunControlsV1,
    pub samples: Vec<QualificationLooseBaselineSampleV1>,
    pub allocations: Vec<QualificationLooseBaselineAllocationSnapshotV1>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationLooseBaselineEvidenceV1 {
    pub schema: String,
    pub source_commit: String,
    pub cargo_lock_sha256: String,
    pub generator_schema: String,
    pub public_seed_hex: String,
    pub platform: QualificationLooseBaselinePlatformV1,
    pub runs: Vec<QualificationLooseBaselineRunV1>,
    pub evidence_sha256: String,
}

impl QualificationLooseBaselineEvidenceV1 {
    pub fn canonical_sha256(&self) -> Result<String, String> {
        let mut preimage = self.clone();
        preimage.evidence_sha256.clear();
        let value = serde_json::to_value(preimage).map_err(|error| error.to_string())?;
        canonical_json_bytes(&value)
            .map(|bytes| sha256_bytes_hex(&bytes))
            .map_err(|error| error.to_string())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != QUALIFICATION_LOOSE_BASELINE_EVIDENCE_SCHEMA_V1
            || self.source_commit != qualification_source_commit()?
            || self.cargo_lock_sha256 != qualification_cargo_lock_sha256()
            || self.generator_schema != QUALIFICATION_GENERATOR_SCHEMA_V1
            || self.public_seed_hex != QUALIFICATION_PUBLIC_SEED_HEX_V1
        {
            return Err("loose baseline evidence identity is missing or stale".to_owned());
        }
        validate_loose_baseline_platform_v1(&self.platform)?;
        if self.evidence_sha256 != self.canonical_sha256()? {
            return Err("loose baseline evidence hash does not match its preimage".to_owned());
        }
        let mut run_keys = BTreeSet::new();
        for run in &self.runs {
            validate_loose_baseline_run_v1(run, &self.platform)?;
            if !run_keys.insert((run.workload, run.run_index)) {
                return Err("loose baseline evidence contains a duplicate run".to_owned());
            }
        }
        let expected = QualificationGeneratedWorkloadV1::ALL
            .into_iter()
            .flat_map(|workload| {
                (1..=QUALIFICATION_LOOSE_BASELINE_INDEPENDENT_ROOTS_V1)
                    .map(move |run_index| (workload, run_index))
            })
            .collect::<BTreeSet<_>>();
        if run_keys != expected {
            return Err(
                "loose baseline evidence is missing a workload or independent root".to_owned(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationProspectiveContractDecisionFieldV1 {
    OperationAbsoluteP95Ceilings,
    OperationRelativeP95Allowances,
    OperationSmallBaselineGuardBands,
    AbsoluteRelativeCombinationFormula,
    P0M0FixedOverheadCap,
    P0M0PeakHeadroomCap,
    FirstRequiredPublicCrossoverTier,
    EventAllocationSavingsG1G2,
    CompleteProfileAllocationSavingsG1G2,
    AllocationStates,
    HighWaterAmplificationBudget,
    MaintenanceDurationBudget,
    P0Role,
    M0Role,
    G0Role,
    G1Role,
    G2Role,
    WorkloadManifestIdentities,
    PublicSeed,
    GeneratorCalibrationVersion,
    OperationSchedule,
    OperationTimedWindowDefinition,
    NativePlatformRoles,
    FilesystemRules,
    AllocationRules,
    KeyedReadClassTreatment,
    ExternalSnapshotAuthority,
    ProvenanceSchema,
    PrivacySchema,
    CausalEarlyStops,
}

impl QualificationProspectiveContractDecisionFieldV1 {
    pub const ALL: [Self; 30] = [
        Self::OperationAbsoluteP95Ceilings,
        Self::OperationRelativeP95Allowances,
        Self::OperationSmallBaselineGuardBands,
        Self::AbsoluteRelativeCombinationFormula,
        Self::P0M0FixedOverheadCap,
        Self::P0M0PeakHeadroomCap,
        Self::FirstRequiredPublicCrossoverTier,
        Self::EventAllocationSavingsG1G2,
        Self::CompleteProfileAllocationSavingsG1G2,
        Self::AllocationStates,
        Self::HighWaterAmplificationBudget,
        Self::MaintenanceDurationBudget,
        Self::P0Role,
        Self::M0Role,
        Self::G0Role,
        Self::G1Role,
        Self::G2Role,
        Self::WorkloadManifestIdentities,
        Self::PublicSeed,
        Self::GeneratorCalibrationVersion,
        Self::OperationSchedule,
        Self::OperationTimedWindowDefinition,
        Self::NativePlatformRoles,
        Self::FilesystemRules,
        Self::AllocationRules,
        Self::KeyedReadClassTreatment,
        Self::ExternalSnapshotAuthority,
        Self::ProvenanceSchema,
        Self::PrivacySchema,
        Self::CausalEarlyStops,
    ];
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationProspectiveContractProposalShapeV1 {
    pub schema: String,
    pub decision_fields: Vec<QualificationProspectiveContractDecisionFieldV1>,
}

impl QualificationProspectiveContractProposalShapeV1 {
    pub fn complete() -> Self {
        Self {
            schema: QUALIFICATION_PROSPECTIVE_CONTRACT_PROPOSAL_SHAPE_SCHEMA_V1.to_owned(),
            decision_fields: QualificationProspectiveContractDecisionFieldV1::ALL.to_vec(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != QUALIFICATION_PROSPECTIVE_CONTRACT_PROPOSAL_SHAPE_SCHEMA_V1
            || self.decision_fields != QualificationProspectiveContractDecisionFieldV1::ALL
        {
            return Err("prospective contract proposal shape is incomplete".to_owned());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationLooseBaselineSmokeReceiptV1 {
    pub operation: QualificationLooseBaselineOperationV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_class: Option<QualificationKeyedReadClassV1>,
    pub receipt: QualificationLooseBaselineSemanticReceiptV1,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationLooseBaselineSmokeV1 {
    pub schema: String,
    pub mode: String,
    pub generator_schema: String,
    pub public_seed_hex: String,
    pub workload: QualificationGeneratedWorkloadV1,
    pub generator_spec_sha256: String,
    pub manifest_sha256: String,
    pub schedule_sha256: String,
    pub receipts: Vec<QualificationLooseBaselineSmokeReceiptV1>,
    pub allocations: Vec<QualificationLooseBaselineAllocationSnapshotV1>,
    pub proposal_shape: QualificationProspectiveContractProposalShapeV1,
}

impl QualificationLooseBaselineSmokeV1 {
    pub fn validate(&self) -> Result<(), String> {
        let (spec_sha256, manifest_sha256, schedule_sha256) =
            frozen_generated_identities_v1(QualificationGeneratedWorkloadV1::G0);
        if self.schema != QUALIFICATION_LOOSE_BASELINE_SMOKE_SCHEMA_V1
            || self.mode != "non_timing_validation"
            || self.generator_schema != QUALIFICATION_GENERATOR_SCHEMA_V1
            || self.public_seed_hex != QUALIFICATION_PUBLIC_SEED_HEX_V1
            || self.workload != QualificationGeneratedWorkloadV1::G0
            || self.generator_spec_sha256 != spec_sha256
            || self.manifest_sha256 != manifest_sha256
            || self.schedule_sha256 != schedule_sha256
        {
            return Err("loose baseline smoke identity is incomplete".to_owned());
        }
        validate_loose_baseline_smoke_receipts_v1(&self.receipts)?;
        validate_loose_baseline_allocations_v1(&self.allocations)?;
        self.proposal_shape.validate()
    }
}

#[derive(Clone, Debug)]
pub struct QualificationLooseBaselineEvidenceConfigurationV1 {
    pub executable: PathBuf,
    pub root: PathBuf,
    pub source_commit: String,
    pub cargo_lock_sha256: String,
    pub quiesced_host: bool,
}

#[derive(Clone, Debug)]
pub struct QualificationLooseBaselineSmokeConfigurationV1 {
    pub executable: PathBuf,
    pub root: PathBuf,
}

fn frozen_generated_identities_v1(
    workload: QualificationGeneratedWorkloadV1,
) -> (&'static str, &'static str, &'static str) {
    match workload {
        QualificationGeneratedWorkloadV1::G0 => (
            QUALIFICATION_G0_SPEC_SHA256_V1,
            QUALIFICATION_G0_MANIFEST_SHA256_V1,
            QUALIFICATION_G0_SCHEDULE_SHA256_V1,
        ),
        QualificationGeneratedWorkloadV1::G1 => (
            QUALIFICATION_G1_SPEC_SHA256_V1,
            QUALIFICATION_G1_MANIFEST_SHA256_V1,
            QUALIFICATION_G1_SCHEDULE_SHA256_V1,
        ),
        QualificationGeneratedWorkloadV1::G2 => (
            QUALIFICATION_G2_SPEC_SHA256_V1,
            QUALIFICATION_G2_MANIFEST_SHA256_V1,
            QUALIFICATION_G2_SCHEDULE_SHA256_V1,
        ),
    }
}

fn loose_baseline_measurement_scope_v1(
    workload: QualificationGeneratedWorkloadV1,
) -> QualificationLooseBaselineMeasurementScopeV1 {
    match workload {
        QualificationGeneratedWorkloadV1::G0 => {
            QualificationLooseBaselineMeasurementScopeV1::Diagnostic
        }
        QualificationGeneratedWorkloadV1::G1 | QualificationGeneratedWorkloadV1::G2 => {
            QualificationLooseBaselineMeasurementScopeV1::Baseline
        }
    }
}

fn loose_baseline_workload_label_v1(workload: QualificationGeneratedWorkloadV1) -> &'static str {
    match workload {
        QualificationGeneratedWorkloadV1::G0 => "g0",
        QualificationGeneratedWorkloadV1::G1 => "g1",
        QualificationGeneratedWorkloadV1::G2 => "g2",
    }
}

fn qualification_keyed_read_classes_v1() -> [QualificationKeyedReadClassV1; 4] {
    [
        QualificationKeyedReadClassV1::Oldest,
        QualificationKeyedReadClassV1::Middle,
        QualificationKeyedReadClassV1::Newest,
        QualificationKeyedReadClassV1::Absent,
    ]
}

fn validate_loose_baseline_platform_v1(
    platform: &QualificationLooseBaselinePlatformV1,
) -> Result<(), String> {
    let supported = matches!(
        (
            platform.operating_system.as_str(),
            platform.filesystem.as_str()
        ),
        ("macos", "apfs") | ("linux", "ext4") | ("windows", "ntfs")
    );
    if !supported
        || platform.operating_system != std::env::consts::OS
        || platform.architecture != std::env::consts::ARCH
        || platform.allocation_api != final_native_allocation_method()
        || classify_qualification_filesystem(&platform.filesystem)
            != QualificationFilesystemDispositionV1::LocalProofEligible
        || !concrete_platform_value_v1(&platform.operating_system_version)
        || !concrete_platform_value_v1(&platform.cpu)
        || !concrete_platform_value_v1(&platform.rustc)
        || platform.build_source.trim().is_empty()
        || platform.build_describe.trim().is_empty()
        || !platform.source_tree_clean
    {
        return Err("loose baseline platform provenance is incomplete".to_owned());
    }
    Ok(())
}

fn concrete_platform_value_v1(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty() && value != "-" && !value.eq_ignore_ascii_case("unavailable")
}

fn validate_loose_baseline_run_v1(
    run: &QualificationLooseBaselineRunV1,
    platform: &QualificationLooseBaselinePlatformV1,
) -> Result<(), String> {
    let (spec_sha256, manifest_sha256, schedule_sha256) =
        frozen_generated_identities_v1(run.workload);
    let expected_run_identity = format!(
        "{}-{}-{}-independent-{}",
        platform.operating_system,
        platform.architecture,
        loose_baseline_workload_label_v1(run.workload),
        run.run_index,
    );
    if !(1..=QUALIFICATION_LOOSE_BASELINE_INDEPENDENT_ROOTS_V1).contains(&run.run_index)
        || run.run_identity != expected_run_identity
        || run.measurement_scope != loose_baseline_measurement_scope_v1(run.workload)
        || run.generator_spec_sha256 != spec_sha256
        || run.manifest_sha256 != manifest_sha256
        || run.schedule_sha256 != schedule_sha256
        || run.controls != QualificationLooseBaselineRunControlsV1::fixed()
    {
        return Err("loose baseline run identity or controls are incomplete".to_owned());
    }

    let mut sample_keys = BTreeSet::new();
    for sample in &run.samples {
        if sample.iteration >= QUALIFICATION_LOOSE_BASELINE_MEASURED_ITERATIONS_V1
            || sample.elapsed_nanos == 0
        {
            return Err("loose baseline raw sample is invalid".to_owned());
        }
        validate_loose_baseline_receipt_v1(sample.operation, sample.read_class, &sample.receipt)?;
        if !sample_keys.insert((sample.operation, sample.read_class, sample.iteration)) {
            return Err("loose baseline run contains a duplicate sample".to_owned());
        }
    }
    let expected_samples = (0..QUALIFICATION_LOOSE_BASELINE_MEASURED_ITERATIONS_V1)
        .flat_map(|iteration| {
            [
                (
                    QualificationLooseBaselineOperationV1::DurableAppend,
                    None,
                    iteration,
                ),
                (
                    QualificationLooseBaselineOperationV1::StrictReplay,
                    None,
                    iteration,
                ),
                (
                    QualificationLooseBaselineOperationV1::FreshProcessOpenRecovery,
                    None,
                    iteration,
                ),
            ]
            .into_iter()
            .chain(
                qualification_keyed_read_classes_v1()
                    .into_iter()
                    .map(move |read_class| {
                        (
                            QualificationLooseBaselineOperationV1::KeyedRead,
                            Some(read_class),
                            iteration,
                        )
                    }),
            )
        })
        .collect::<BTreeSet<_>>();
    if sample_keys != expected_samples {
        return Err("loose baseline run is missing an operation or read class".to_owned());
    }
    validate_loose_baseline_allocations_v1(&run.allocations)
}

fn validate_loose_baseline_receipt_v1(
    operation: QualificationLooseBaselineOperationV1,
    read_class: Option<QualificationKeyedReadClassV1>,
    receipt: &QualificationLooseBaselineSemanticReceiptV1,
) -> Result<(), String> {
    let expected_kind = match (operation, read_class) {
        (QualificationLooseBaselineOperationV1::DurableAppend, None) => {
            QualificationLooseBaselineReceiptKindV1::DurableAppendVisibleExact
        }
        (QualificationLooseBaselineOperationV1::StrictReplay, None) => {
            QualificationLooseBaselineReceiptKindV1::StrictReplayExact
        }
        (QualificationLooseBaselineOperationV1::FreshProcessOpenRecovery, None) => {
            QualificationLooseBaselineReceiptKindV1::FreshProcessOpenExact
        }
        (
            QualificationLooseBaselineOperationV1::KeyedRead,
            Some(QualificationKeyedReadClassV1::Absent),
        ) => QualificationLooseBaselineReceiptKindV1::KeyedReadAbsentExact,
        (QualificationLooseBaselineOperationV1::KeyedRead, Some(_)) => {
            QualificationLooseBaselineReceiptKindV1::KeyedReadPresentExact
        }
        _ => return Err("loose baseline receipt operation shape is invalid".to_owned()),
    };
    let absent = receipt.kind == QualificationLooseBaselineReceiptKindV1::KeyedReadAbsentExact;
    if receipt.kind != expected_kind
        || (absent && (receipt.record_count != 0 || receipt.logical_byte_count != 0))
        || (!absent && (receipt.record_count == 0 || receipt.logical_byte_count == 0))
    {
        return Err("loose baseline semantic receipt is incomplete".to_owned());
    }
    validate_hex(
        &receipt.aggregate_receipt_sha256,
        64,
        "loose baseline aggregate receipt SHA-256",
    )
}

fn validate_loose_baseline_allocations_v1(
    allocations: &[QualificationLooseBaselineAllocationSnapshotV1],
) -> Result<(), String> {
    let mut keys = BTreeSet::new();
    for allocation in allocations {
        if allocation.inventory.carrier_count == 0
            || allocation.inventory.logical_bytes == 0
            || allocation.inventory.encoded_bytes == 0
            || allocation.inventory.allocated_bytes == 0
            || allocation.inventory.high_water_bytes < allocation.inventory.allocated_bytes
            || !keys.insert((allocation.scope, allocation.state))
        {
            return Err("loose baseline allocation inventory is incomplete".to_owned());
        }
        validate_hex(
            &allocation.inventory.carrier_set_sha256,
            64,
            "loose baseline carrier-set SHA-256",
        )?;
    }
    let expected = [
        QualificationPerformanceAllocationScopeV2::Event,
        QualificationPerformanceAllocationScopeV2::CompleteProfile,
    ]
    .into_iter()
    .flat_map(|scope| {
        QualificationPerformanceInventoryStateV1::ALL
            .into_iter()
            .map(move |state| (scope, state))
    })
    .collect::<BTreeSet<_>>();
    if keys != expected {
        return Err("loose baseline allocation scope or state is missing".to_owned());
    }
    Ok(())
}

fn validate_loose_baseline_smoke_receipts_v1(
    receipts: &[QualificationLooseBaselineSmokeReceiptV1],
) -> Result<(), String> {
    let mut keys = BTreeSet::new();
    for receipt in receipts {
        validate_loose_baseline_receipt_v1(
            receipt.operation,
            receipt.read_class,
            &receipt.receipt,
        )?;
        if !keys.insert((receipt.operation, receipt.read_class)) {
            return Err("loose baseline smoke contains a duplicate receipt".to_owned());
        }
    }
    let expected = [
        (QualificationLooseBaselineOperationV1::DurableAppend, None),
        (QualificationLooseBaselineOperationV1::StrictReplay, None),
        (
            QualificationLooseBaselineOperationV1::FreshProcessOpenRecovery,
            None,
        ),
    ]
    .into_iter()
    .chain(
        qualification_keyed_read_classes_v1()
            .into_iter()
            .map(|read_class| {
                (
                    QualificationLooseBaselineOperationV1::KeyedRead,
                    Some(read_class),
                )
            }),
    )
    .collect::<BTreeSet<_>>();
    if keys != expected {
        return Err("loose baseline smoke is missing an operation or read class".to_owned());
    }
    Ok(())
}

#[cfg(test)]
impl QualificationLooseBaselineEvidenceV1 {
    fn fixture_for_tests() -> Self {
        let filesystem = match std::env::consts::OS {
            "macos" => "apfs",
            "linux" => "ext4",
            "windows" => "ntfs",
            operating_system => panic!("unsupported test operating system {operating_system}"),
        };
        let platform = QualificationLooseBaselinePlatformV1 {
            operating_system: std::env::consts::OS.to_owned(),
            operating_system_version: "fixture-os-version".to_owned(),
            architecture: std::env::consts::ARCH.to_owned(),
            cpu: "fixture-cpu".to_owned(),
            filesystem: filesystem.to_owned(),
            allocation_api: final_native_allocation_method().to_owned(),
            rustc: format!("rustc fixture host: {}", std::env::consts::ARCH),
            build_source: "git".to_owned(),
            build_describe: "fixture".to_owned(),
            source_tree_clean: true,
        };
        let runs = QualificationGeneratedWorkloadV1::ALL
            .into_iter()
            .flat_map(|workload| {
                let platform = platform.clone();
                (1..=QUALIFICATION_LOOSE_BASELINE_INDEPENDENT_ROOTS_V1).map(move |run_index| {
                    let mut samples = Vec::new();
                    for iteration in 0..QUALIFICATION_LOOSE_BASELINE_MEASURED_ITERATIONS_V1 {
                        for (operation, read_class) in [
                            (QualificationLooseBaselineOperationV1::DurableAppend, None),
                            (QualificationLooseBaselineOperationV1::StrictReplay, None),
                            (
                                QualificationLooseBaselineOperationV1::FreshProcessOpenRecovery,
                                None,
                            ),
                        ]
                        .into_iter()
                        .chain(qualification_keyed_read_classes_v1().into_iter().map(
                            |read_class| {
                                (
                                    QualificationLooseBaselineOperationV1::KeyedRead,
                                    Some(read_class),
                                )
                            },
                        )) {
                            let kind = match (operation, read_class) {
                                (QualificationLooseBaselineOperationV1::DurableAppend, None) => {
                                    QualificationLooseBaselineReceiptKindV1::DurableAppendVisibleExact
                                }
                                (QualificationLooseBaselineOperationV1::StrictReplay, None) => {
                                    QualificationLooseBaselineReceiptKindV1::StrictReplayExact
                                }
                                (
                                    QualificationLooseBaselineOperationV1::FreshProcessOpenRecovery,
                                    None,
                                ) => QualificationLooseBaselineReceiptKindV1::FreshProcessOpenExact,
                                (
                                    QualificationLooseBaselineOperationV1::KeyedRead,
                                    Some(QualificationKeyedReadClassV1::Absent),
                                ) => QualificationLooseBaselineReceiptKindV1::KeyedReadAbsentExact,
                                (QualificationLooseBaselineOperationV1::KeyedRead, Some(_)) => {
                                    QualificationLooseBaselineReceiptKindV1::KeyedReadPresentExact
                                }
                                _ => unreachable!(),
                            };
                            let absent = kind
                                == QualificationLooseBaselineReceiptKindV1::KeyedReadAbsentExact;
                            let receipt_seed = format!(
                                "{workload:?}-{run_index}-{iteration}-{operation:?}-{read_class:?}"
                            );
                            samples.push(QualificationLooseBaselineSampleV1 {
                                operation,
                                read_class,
                                iteration,
                                elapsed_nanos: 1,
                                receipt: QualificationLooseBaselineSemanticReceiptV1 {
                                    kind,
                                    record_count: u64::from(!absent),
                                    logical_byte_count: u64::from(!absent),
                                    aggregate_receipt_sha256: sha256_bytes_hex(
                                        receipt_seed.as_bytes(),
                                    ),
                                },
                            });
                        }
                    }
                    let allocations = [
                        QualificationPerformanceAllocationScopeV2::Event,
                        QualificationPerformanceAllocationScopeV2::CompleteProfile,
                    ]
                    .into_iter()
                    .flat_map(|scope| {
                        QualificationPerformanceInventoryStateV1::ALL.into_iter().map(
                            move |state| QualificationLooseBaselineAllocationSnapshotV1 {
                                scope,
                                state,
                                inventory: QualificationLooseBaselineInventoryV1 {
                                    carrier_count: 1,
                                    carrier_set_sha256: sha256_bytes_hex(
                                        format!("{workload:?}-{run_index}-{scope:?}-{state:?}")
                                            .as_bytes(),
                                    ),
                                    logical_bytes: 1,
                                    encoded_bytes: 1,
                                    allocated_bytes: 1,
                                    high_water_bytes: 1,
                                },
                            },
                        )
                    })
                    .collect();
                    let (generator_spec_sha256, manifest_sha256, schedule_sha256) =
                        frozen_generated_identities_v1(workload);
                    QualificationLooseBaselineRunV1 {
                        run_index,
                        run_identity: format!(
                            "{}-{}-{}-independent-{run_index}",
                            platform.operating_system,
                            platform.architecture,
                            loose_baseline_workload_label_v1(workload),
                        ),
                        workload,
                        measurement_scope: loose_baseline_measurement_scope_v1(workload),
                        generator_spec_sha256: generator_spec_sha256.to_owned(),
                        manifest_sha256: manifest_sha256.to_owned(),
                        schedule_sha256: schedule_sha256.to_owned(),
                        controls: QualificationLooseBaselineRunControlsV1::fixed(),
                        samples,
                        allocations,
                    }
                })
            })
            .collect();
        let mut evidence = Self {
            schema: QUALIFICATION_LOOSE_BASELINE_EVIDENCE_SCHEMA_V1.to_owned(),
            source_commit: qualification_source_commit().expect("test build source commit"),
            cargo_lock_sha256: qualification_cargo_lock_sha256(),
            generator_schema: QUALIFICATION_GENERATOR_SCHEMA_V1.to_owned(),
            public_seed_hex: QUALIFICATION_PUBLIC_SEED_HEX_V1.to_owned(),
            platform,
            runs,
            evidence_sha256: String::new(),
        };
        evidence.evidence_sha256 = evidence
            .canonical_sha256()
            .expect("canonical loose baseline evidence");
        evidence
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationPerformanceOperationV1 {
    DurableAppend,
    StrictReplay,
    KeyedRead,
    OpenRecovery,
}

impl QualificationPerformanceOperationV1 {
    pub const ALL: [Self; 4] = [
        Self::DurableAppend,
        Self::StrictReplay,
        Self::KeyedRead,
        Self::OpenRecovery,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::DurableAppend => "durable_append",
            Self::StrictReplay => "strict_replay",
            Self::KeyedRead => "keyed_read",
            Self::OpenRecovery => "open_recovery",
        }
    }

    fn legacy_sample_names(self) -> (&'static str, &'static str) {
        match self {
            Self::DurableAppend => ("candidate_durable_append", "baseline_durable_append"),
            Self::StrictReplay => ("candidate_replay", "baseline_replay"),
            Self::KeyedRead => ("candidate_keyed_read", "baseline_keyed_read"),
            Self::OpenRecovery => ("candidate_open_recovery", "baseline_open_recovery"),
        }
    }

    fn legacy_failure_label(self) -> &'static str {
        match self {
            Self::StrictReplay => "replay",
            operation => operation.as_str(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationPerformanceRoleV1 {
    LooseBaseline,
    SqliteWal,
    BoundedSegments,
}

impl QualificationPerformanceRoleV1 {
    pub const CANDIDATES: [Self; 2] = [Self::SqliteWal, Self::BoundedSegments];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::LooseBaseline => "loose_baseline",
            Self::SqliteWal => "sqlite_wal",
            Self::BoundedSegments => "bounded_segments",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationPerformancePairOrderV1 {
    CandidateThenBaseline,
    BaselineThenCandidate,
    Alternating,
}

#[derive(Clone, Debug)]
pub struct QualificationPerformanceDiagnosticConfigurationV1 {
    pub executable: PathBuf,
    pub root: PathBuf,
    pub source_commit: String,
    pub cargo_lock_sha256: String,
    pub warmup_samples: u32,
    pub measured_samples: u32,
    pub pair_order: QualificationPerformancePairOrderV1,
    pub external_corpus_root: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct QualificationPerformanceCampaignConfigurationV2 {
    pub executable: PathBuf,
    pub root: PathBuf,
    pub source_commit: String,
    pub cargo_lock_sha256: String,
    pub external_corpus_root: Option<PathBuf>,
    pub quiesced_host: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct QualificationPerformanceOpenRequestV2 {
    role: QualificationPerformanceRoleV1,
    root: PathBuf,
    result_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct QualificationPerformanceSemanticReceiptV2 {
    record_count: u64,
    receipt_sha256: String,
}

#[derive(Clone, Debug)]
pub(super) struct QualificationPerformanceOperationRequestV1<'a> {
    pub operation: QualificationPerformanceOperationV1,
    pub role: QualificationPerformanceRoleV1,
    pub iteration: u32,
    pub pair_order: u8,
    pub logical_key: &'a str,
    pub decoded_bytes: &'a [u8],
}

pub(super) trait QualificationPerformanceProbe {
    fn run_profiled_operation(
        &self,
        request: &QualificationPerformanceOperationRequestV1<'_>,
    ) -> Result<QualificationPerformanceDiagnosticSampleV1, String>;
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPerformanceDiagnosticSampleV1 {
    pub operation: QualificationPerformanceOperationV1,
    pub role: QualificationPerformanceRoleV1,
    pub iteration: u32,
    pub pair_order: u8,
    pub total_elapsed_nanos: u64,
    pub stages: Vec<QualificationPerformanceStageSampleV1>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPerformanceStageSampleV1 {
    pub stage: String,
    pub elapsed_nanos: u64,
}

#[derive(Debug)]
pub(super) struct QualificationPerformanceStageRecorder {
    started: Instant,
    stages: Vec<QualificationPerformanceStageSampleV1>,
}

impl Default for QualificationPerformanceStageRecorder {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            stages: Vec::new(),
        }
    }
}

impl QualificationPerformanceStageRecorder {
    pub(super) fn measure<T, E>(
        &mut self,
        stage: &str,
        operation: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E> {
        let started = Instant::now();
        let result = operation();
        self.stages.push(QualificationPerformanceStageSampleV1 {
            stage: stage.to_owned(),
            elapsed_nanos: elapsed_nanos(started),
        });
        result
    }

    pub(super) fn elapsed_nanos(&self) -> u64 {
        elapsed_nanos(self.started)
    }

    pub(super) fn finish(
        self,
        total_elapsed_nanos: u64,
    ) -> Result<Vec<QualificationPerformanceStageSampleV1>, String> {
        if self.stages.is_empty()
            || self
                .stages
                .iter()
                .any(|stage| stage.stage.trim().is_empty())
            || self
                .stages
                .iter()
                .try_fold(0_u64, |total, stage| total.checked_add(stage.elapsed_nanos))
                .is_none_or(|stages| stages > total_elapsed_nanos)
        {
            return Err("profiled operation produced invalid timing stages".to_owned());
        }
        Ok(self.stages)
    }
}

#[derive(Debug)]
pub(super) struct LooseQualificationPerformanceProbe {
    root: PathBuf,
    logical_bytes: AtomicU64,
    high_water_bytes: AtomicU64,
}

impl LooseQualificationPerformanceProbe {
    pub(super) fn create(
        root: PathBuf,
        workload: &super::QualificationCorpusManifestV1,
    ) -> Result<Self, String> {
        fs::create_dir(&root).map_err(|_| "loose baseline root creation failed".to_owned())?;
        let mut logical_bytes = 0_u64;
        for record in &workload.records {
            let path = baseline_record_path(&root, &record.logical_key, record.record_kind);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|_| "loose baseline directory creation failed".to_owned())?;
            }
            write_new_synced(&path, &record.decoded_bytes)?;
            logical_bytes = logical_bytes
                .checked_add(record.decoded_bytes.len() as u64)
                .ok_or_else(|| "loose baseline logical-byte total overflow".to_owned())?;
        }
        let probe = Self {
            root,
            logical_bytes: AtomicU64::new(logical_bytes),
            high_water_bytes: AtomicU64::new(0),
        };
        probe.inventory()?;
        Ok(probe)
    }

    pub(super) fn inventory(&self) -> Result<QualificationInventoryV1, String> {
        let mut inventory =
            baseline_inventory(&self.root, self.logical_bytes.load(Ordering::Relaxed))?;
        let high_water = self
            .high_water_bytes
            .fetch_max(inventory.allocated_bytes, Ordering::Relaxed)
            .max(inventory.allocated_bytes);
        inventory.high_water_bytes = high_water;
        Ok(inventory)
    }

    fn verify_read(
        &self,
        request: &QualificationPerformanceOperationRequestV1<'_>,
    ) -> Result<(), String> {
        let path = baseline_record_path(
            &self.root,
            request.logical_key,
            QualificationRecordKindV1::LegacyEvent,
        );
        let bytes = fs::read(path).map_err(|_| "loose keyed read failed".to_owned())?;
        if bytes != request.decoded_bytes {
            return Err("loose keyed read returned different decoded bytes".to_owned());
        }
        std::hint::black_box(sha256_bytes_hex(&bytes));
        Ok(())
    }

    pub(super) fn legacy_durable_append(
        &self,
        path: &Path,
        decoded_bytes: &[u8],
    ) -> Result<(), String> {
        write_new_synced(path, decoded_bytes)
    }

    pub(super) fn record_legacy_append(&self, decoded_bytes: &[u8]) {
        self.logical_bytes
            .fetch_add(decoded_bytes.len() as u64, Ordering::Relaxed);
    }

    pub(super) fn legacy_replay(&self) -> Result<(), String> {
        read_all_files(&self.root)
    }

    pub(super) fn legacy_keyed_read(&self, path: &Path) -> Result<(), String> {
        let bytes = fs::read(path).map_err(|error| error.to_string())?;
        std::hint::black_box(Sha256::digest(&bytes));
        Ok(())
    }

    pub(super) fn legacy_open_recovery(&self) -> Result<(), String> {
        read_all_files(&self.root)
    }

    fn run_operation(
        &self,
        request: &QualificationPerformanceOperationRequestV1<'_>,
        mut recorder: Option<&mut QualificationPerformanceStageRecorder>,
    ) -> Result<(), String> {
        match request.operation {
            QualificationPerformanceOperationV1::DurableAppend => {
                measure_string_stage(&mut recorder, "file_create_write_sync", || {
                    let path = baseline_record_path(
                        &self.root,
                        request.logical_key,
                        QualificationRecordKindV1::LegacyEvent,
                    );
                    write_new_synced(&path, request.decoded_bytes)
                })?;
                self.logical_bytes
                    .fetch_add(request.decoded_bytes.len() as u64, Ordering::Relaxed);
            }
            QualificationPerformanceOperationV1::StrictReplay => {
                measure_string_stage(&mut recorder, "enumerate_read_hash", || {
                    read_all_files(&self.root.join("events"))
                        .map_err(|_| "loose strict replay failed".to_owned())
                })?;
            }
            QualificationPerformanceOperationV1::KeyedRead => {
                measure_string_stage(&mut recorder, "file_read_hash", || {
                    self.verify_read(request)
                })?;
            }
            QualificationPerformanceOperationV1::OpenRecovery => {
                measure_string_stage(&mut recorder, "reopen_traversal", || {
                    read_all_files(&self.root)
                        .map_err(|_| "loose reopen validation failed".to_owned())
                })?;
            }
        }
        Ok(())
    }
}

impl QualificationPerformanceProbe for LooseQualificationPerformanceProbe {
    fn run_profiled_operation(
        &self,
        request: &QualificationPerformanceOperationRequestV1<'_>,
    ) -> Result<QualificationPerformanceDiagnosticSampleV1, String> {
        if request.role != QualificationPerformanceRoleV1::LooseBaseline {
            return Err("loose probe received a candidate request".to_owned());
        }
        let mut recorder = QualificationPerformanceStageRecorder::default();
        self.run_operation(request, Some(&mut recorder))?;
        let total_elapsed_nanos = recorder.elapsed_nanos();
        let stages = recorder.finish(total_elapsed_nanos)?;
        self.inventory()?;
        Ok(QualificationPerformanceDiagnosticSampleV1 {
            operation: request.operation,
            role: request.role,
            iteration: request.iteration,
            pair_order: request.pair_order,
            total_elapsed_nanos,
            stages,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationPerformanceInventoryStateV1 {
    Steady,
    Reopened,
    HighWater,
}

impl QualificationPerformanceInventoryStateV1 {
    pub const ALL: [Self; 3] = [Self::Steady, Self::Reopened, Self::HighWater];
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationPerformanceWorkloadV2 {
    ExternalCorpus,
    ModeledFoundation,
    PublicSmoke,
}

impl QualificationPerformanceWorkloadV2 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExternalCorpus => "external_corpus",
            Self::ModeledFoundation => "modeled_foundation",
            Self::PublicSmoke => "public_smoke",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationPerformanceAllocationScopeV2 {
    Event,
    CompleteProfile,
}

impl QualificationPerformanceAllocationScopeV2 {
    pub const ALL: [Self; 2] = [Self::Event, Self::CompleteProfile];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationPerformanceConfidenceMethodV2 {
    IndependentRuns,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationPerformanceOutlierPolicyV2 {
    RetainAll,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationPerformanceCachePolicyV2 {
    OsWarm,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPerformancePlatformRequirementV2 {
    pub operating_system: String,
    pub filesystem: String,
    pub allocation_method: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPerformanceWorkloadRequirementV2 {
    pub workload: QualificationPerformanceWorkloadV2,
    pub manifest_sha256: String,
    pub quantitative: bool,
    pub platforms: Vec<QualificationPerformancePlatformRequirementV2>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPerformanceProtocolV2 {
    pub baseline_representation: String,
    pub carrier_accounting: String,
    pub evidence_inventory: String,
    pub append_payload: String,
    pub append_state: String,
    pub durable_append_acknowledgement: String,
    pub strict_replay_receipt: String,
    pub keyed_read_receipt: String,
    pub open_recovery_receipt: String,
    pub timing_ratio: String,
    pub percentiles: Vec<u32>,
    pub report_range: bool,
    pub standard_deviation: String,
    pub missing_evidence: String,
    pub required_controls: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPerformanceContractV2 {
    pub schema: String,
    pub operations: Vec<QualificationPerformanceOperationV1>,
    pub candidates: Vec<QualificationPerformanceRoleV1>,
    pub workloads: Vec<QualificationPerformanceWorkloadRequirementV2>,
    pub protocol: QualificationPerformanceProtocolV2,
    pub inventory_states: Vec<QualificationPerformanceInventoryStateV1>,
    pub allocation_scopes: Vec<QualificationPerformanceAllocationScopeV2>,
    pub warmup_samples: u32,
    pub measured_samples: u32,
    pub independent_runs: u32,
    pub pair_order: QualificationPerformancePairOrderV1,
    pub confidence_method: QualificationPerformanceConfidenceMethodV2,
    pub outlier_policy: QualificationPerformanceOutlierPolicyV2,
    pub cache_policy: QualificationPerformanceCachePolicyV2,
    pub ceiling_percent: u32,
    pub allocation_must_be_strictly_lower: bool,
}

impl QualificationPerformanceContractV2 {
    pub fn frozen() -> Self {
        let native_platforms = vec![
            QualificationPerformancePlatformRequirementV2 {
                operating_system: "macos".to_owned(),
                filesystem: "apfs".to_owned(),
                allocation_method: "stat_blocks_512".to_owned(),
            },
            QualificationPerformancePlatformRequirementV2 {
                operating_system: "linux".to_owned(),
                filesystem: "ext4".to_owned(),
                allocation_method: "stat_blocks_512".to_owned(),
            },
            QualificationPerformancePlatformRequirementV2 {
                operating_system: "windows".to_owned(),
                filesystem: "ntfs".to_owned(),
                allocation_method: "file_standard_info_allocation_size".to_owned(),
            },
        ];
        Self {
            schema: QUALIFICATION_PERFORMANCE_CONTRACT_SCHEMA_V2.to_owned(),
            operations: QualificationPerformanceOperationV1::ALL.to_vec(),
            candidates: QualificationPerformanceRoleV1::CANDIDATES.to_vec(),
            workloads: vec![
                QualificationPerformanceWorkloadRequirementV2 {
                    workload: QualificationPerformanceWorkloadV2::ExternalCorpus,
                    manifest_sha256: QUALIFICATION_EXTERNAL_WORKLOAD_MANIFEST_SHA256_V2.to_owned(),
                    quantitative: true,
                    platforms: vec![native_platforms[0].clone()],
                },
                QualificationPerformanceWorkloadRequirementV2 {
                    workload: QualificationPerformanceWorkloadV2::ModeledFoundation,
                    manifest_sha256:
                        "5d7ea2f2a8398722e2dcc853ef2c4ebe1976a02fd1585a190c9c6b86e132da7d"
                            .to_owned(),
                    quantitative: true,
                    platforms: native_platforms.clone(),
                },
                QualificationPerformanceWorkloadRequirementV2 {
                    workload: QualificationPerformanceWorkloadV2::PublicSmoke,
                    manifest_sha256:
                        "03cfda81e2ea988ec119b942530022b345d08b1261a6f198f87fdade2a4d1b01"
                            .to_owned(),
                    quantitative: false,
                    platforms: native_platforms,
                },
            ],
            protocol: QualificationPerformanceProtocolV2 {
                baseline_representation: "fresh_current_profile_loose".to_owned(),
                carrier_accounting: "all_profile_owned_and_complete_content".to_owned(),
                evidence_inventory: "carrier_count_hash_and_totals".to_owned(),
                append_payload: "manifest_representative_size_deterministic_valid_records"
                    .to_owned(),
                append_state: "monotonic_depth_matched".to_owned(),
                durable_append_acknowledgement: "normal_durable_boundary_and_fresh_reader_receipt"
                    .to_owned(),
                strict_replay_receipt: "exact_order_count_and_hash".to_owned(),
                keyed_read_receipt: "manifest_selected_exact_record".to_owned(),
                open_recovery_receipt: "fresh_process_exact_visible_event_set".to_owned(),
                timing_ratio: "adjacent_candidate_to_baseline".to_owned(),
                percentiles: vec![50, 95],
                report_range: true,
                standard_deviation: "population".to_owned(),
                missing_evidence: "unknown".to_owned(),
                required_controls: vec![
                    "fresh_roots".to_owned(),
                    "quiesced_host".to_owned(),
                    "native_execution".to_owned(),
                    "equivalent_decoded_bytes".to_owned(),
                    "monotonic_append_state".to_owned(),
                    "durable_acknowledgement".to_owned(),
                    "semantic_validation".to_owned(),
                    "open_recovery_fresh_process".to_owned(),
                ],
            },
            inventory_states: QualificationPerformanceInventoryStateV1::ALL.to_vec(),
            allocation_scopes: vec![
                QualificationPerformanceAllocationScopeV2::Event,
                QualificationPerformanceAllocationScopeV2::CompleteProfile,
            ],
            warmup_samples: 3,
            measured_samples: 30,
            independent_runs: 2,
            pair_order: QualificationPerformancePairOrderV1::Alternating,
            confidence_method: QualificationPerformanceConfidenceMethodV2::IndependentRuns,
            outlier_policy: QualificationPerformanceOutlierPolicyV2::RetainAll,
            cache_policy: QualificationPerformanceCachePolicyV2::OsWarm,
            ceiling_percent: 125,
            allocation_must_be_strictly_lower: true,
        }
    }

    pub fn canonical_sha256(&self) -> Result<String, String> {
        let value = serde_json::to_value(self).map_err(|error| error.to_string())?;
        canonical_json_bytes(&value)
            .map(|bytes| sha256_bytes_hex(&bytes))
            .map_err(|error| error.to_string())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self != &Self::frozen() {
            return Err("unsupported performance contract".to_owned());
        }
        if self.canonical_sha256()? != QUALIFICATION_PERFORMANCE_CONTRACT_SHA256_V2 {
            return Err("performance contract hash does not match the frozen identity".to_owned());
        }
        for workload in &self.workloads {
            validate_hex(
                &workload.manifest_sha256,
                64,
                "performance workload manifest SHA-256",
            )?;
        }
        Ok(())
    }

    pub fn decision_table_markdown(&self) -> String {
        let mut table = vec![
            "| Workload | Platforms | Verdict | Protocol |".to_owned(),
            "| --- | --- | --- | --- |".to_owned(),
        ];
        for workload in &self.workloads {
            let platforms = workload
                .platforms
                .iter()
                .map(|platform| format!("{}/{}", platform.operating_system, platform.filesystem))
                .collect::<Vec<_>>()
                .join(", ");
            let verdict = if workload.quantitative {
                format!(
                    "paired p95 <= {}%; event and complete allocation strictly lower",
                    self.ceiling_percent
                )
            } else {
                "protocol and semantic receipt; timing diagnostic only".to_owned()
            };
            table.push(format!(
                "| `{}` | {} | {} | {} warm-up, {} measured, {} independent runs, alternating pairs |",
                workload.workload.as_str(),
                platforms,
                verdict,
                self.warmup_samples,
                self.measured_samples,
                self.independent_runs,
            ));
        }
        table.join("\n")
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPerformanceContractPublicationV2 {
    pub schema: String,
    pub contract: QualificationPerformanceContractV2,
    pub contract_sha256: String,
    pub decision_table_markdown: String,
}

pub fn qualification_performance_contract_v2_publication()
-> QualificationPerformanceContractPublicationV2 {
    let contract = QualificationPerformanceContractV2::frozen();
    QualificationPerformanceContractPublicationV2 {
        schema: QUALIFICATION_PERFORMANCE_CONTRACT_PUBLICATION_SCHEMA_V2.to_owned(),
        contract_sha256: contract
            .canonical_sha256()
            .expect("the frozen performance contract is canonical"),
        decision_table_markdown: contract.decision_table_markdown(),
        contract,
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationProspectivePlatformV1 {
    MacosApfs,
    LinuxExt4,
    WindowsNtfs,
}

impl QualificationProspectivePlatformV1 {
    pub const ALL: [Self; 3] = [Self::MacosApfs, Self::LinuxExt4, Self::WindowsNtfs];

    fn operating_system(self) -> &'static str {
        match self {
            Self::MacosApfs => "macos",
            Self::LinuxExt4 => "linux",
            Self::WindowsNtfs => "windows",
        }
    }

    fn filesystem(self) -> &'static str {
        match self {
            Self::MacosApfs => "apfs",
            Self::LinuxExt4 => "ext4",
            Self::WindowsNtfs => "ntfs",
        }
    }

    fn allocation_api(self) -> &'static str {
        match self {
            Self::MacosApfs | Self::LinuxExt4 => "stat_blocks_512",
            Self::WindowsNtfs => "file_standard_info_allocation_size",
        }
    }

    fn evidence_role(self) -> &'static str {
        match self {
            Self::MacosApfs => "required_quantitative",
            Self::LinuxExt4 => "required_quantitative_non_container",
            Self::WindowsNtfs => "required_quantitative_native",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::MacosApfs => "macOS/APFS",
            Self::LinuxExt4 => "Linux/ext4 (non-container)",
            Self::WindowsNtfs => "Windows/NTFS (native)",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationProspectiveWorkloadV1 {
    P0,
    M0,
    G0,
    G1,
    G2,
}

impl QualificationProspectiveWorkloadV1 {
    pub const ALL: [Self; 5] = [Self::P0, Self::M0, Self::G0, Self::G1, Self::G2];

    pub fn timing_required(self) -> bool {
        matches!(self, Self::G0 | Self::G1 | Self::G2)
    }

    pub fn savings_required(self) -> bool {
        matches!(self, Self::G1 | Self::G2)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationProspectiveWorkloadRoleV1 {
    SemanticProvenancePrivacySmoke,
    CapabilityFixedCostSentinel,
    DiagnosticCausalEarlyStop,
    RequiredMediumFirstCrossover,
    RequiredRepresentativePublic,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationProspectivePlatformRequirementV1 {
    pub platform: QualificationProspectivePlatformV1,
    pub operating_system: String,
    pub filesystem: String,
    pub allocation_api: String,
    pub evidence_role: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationProspectiveWorkloadRequirementV1 {
    pub workload: QualificationProspectiveWorkloadV1,
    pub role: QualificationProspectiveWorkloadRoleV1,
    pub manifest_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generator_spec_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_schedule_sha256: Option<String>,
    pub timing_required: bool,
    pub savings_required: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationProspectiveRunControlsV1 {
    pub warmup_iterations: u32,
    pub measured_iterations: u32,
    pub independent_runs: u32,
    pub p95_estimator: String,
    pub p95_rank: u32,
    pub sample_retention: String,
    pub outlier_policy: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationProspectiveTimingThresholdV1 {
    pub operation: QualificationLooseBaselineOperationV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_class: Option<QualificationKeyedReadClassV1>,
    pub absolute_ceiling_nanos: u64,
    pub relative_numerator: u32,
    pub relative_denominator: u32,
    pub small_baseline_guard_band_nanos: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationProspectiveAllocationPolicyV1 {
    pub event_fixed_overhead_bytes: u64,
    pub complete_profile_fixed_overhead_bytes: u64,
    pub event_peak_headroom_bytes: u64,
    pub complete_profile_peak_headroom_bytes: u64,
    pub first_required_crossover_workload: QualificationProspectiveWorkloadV1,
    pub event_savings_percent: u32,
    pub complete_profile_savings_percent: u32,
    pub savings_workloads: Vec<QualificationProspectiveWorkloadV1>,
    pub states: Vec<QualificationPerformanceInventoryStateV1>,
    pub high_water_numerator: u32,
    pub high_water_denominator: u32,
    pub carrier_accounting: String,
    pub virtual_address_reservation_excluded: bool,
}

impl QualificationProspectiveAllocationPolicyV1 {
    pub fn fixed_overhead_cap(&self, scope: QualificationPerformanceAllocationScopeV2) -> u64 {
        match scope {
            QualificationPerformanceAllocationScopeV2::Event => self.event_fixed_overhead_bytes,
            QualificationPerformanceAllocationScopeV2::CompleteProfile => {
                self.complete_profile_fixed_overhead_bytes
            }
        }
    }

    pub fn peak_headroom_cap(&self, scope: QualificationPerformanceAllocationScopeV2) -> u64 {
        match scope {
            QualificationPerformanceAllocationScopeV2::Event => self.event_peak_headroom_bytes,
            QualificationPerformanceAllocationScopeV2::CompleteProfile => {
                self.complete_profile_peak_headroom_bytes
            }
        }
    }

    fn savings_percent(&self, scope: QualificationPerformanceAllocationScopeV2) -> u32 {
        match scope {
            QualificationPerformanceAllocationScopeV2::Event => self.event_savings_percent,
            QualificationPerformanceAllocationScopeV2::CompleteProfile => {
                self.complete_profile_savings_percent
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationProspectiveMaintenancePolicyV1 {
    pub foreground_p95_max_nanos: u64,
    pub g1_total_max_nanos: u64,
    pub g2_total_max_nanos: u64,
    pub not_applicable_requires_mechanism_proof: bool,
}

impl QualificationProspectiveMaintenancePolicyV1 {
    pub fn total_max_nanos(&self, workload: QualificationProspectiveWorkloadV1) -> u64 {
        match workload {
            QualificationProspectiveWorkloadV1::G1 => self.g1_total_max_nanos,
            QualificationProspectiveWorkloadV1::G2 => self.g2_total_max_nanos,
            _ => 0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationProspectiveBaselineAuthorityV1 {
    pub platform: QualificationProspectivePlatformV1,
    pub evidence_sha256: String,
    pub file_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationProspectiveDerivationV1 {
    pub pointbreak_commit: String,
    pub pointbreak_tree: String,
    pub cargo_lock_sha256: String,
    pub generator_schema: String,
    pub generator_landing_commit: String,
    pub public_seed_hex: String,
    pub calibration_version: String,
    pub allowed_inputs: Vec<String>,
    pub candidate_measurements_used: bool,
    pub historical_candidate_results_used: bool,
    pub private_calibration_used: bool,
    pub private_corpus_used: bool,
    pub owner_approval_required_before_compilation: bool,
    pub baseline_authorities: Vec<QualificationProspectiveBaselineAuthorityV1>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationProspectiveContractV1 {
    pub schema: String,
    pub approved_proposal_sha256: String,
    pub derivation: QualificationProspectiveDerivationV1,
    pub run_controls: QualificationProspectiveRunControlsV1,
    pub operations: Vec<QualificationLooseBaselineOperationV1>,
    pub keyed_read_classes: Vec<QualificationKeyedReadClassV1>,
    pub timing_thresholds: Vec<QualificationProspectiveTimingThresholdV1>,
    pub timing_combination_formula: String,
    pub platforms: Vec<QualificationProspectivePlatformRequirementV1>,
    pub workloads: Vec<QualificationProspectiveWorkloadRequirementV1>,
    pub allocation: QualificationProspectiveAllocationPolicyV1,
    pub maintenance: QualificationProspectiveMaintenancePolicyV1,
    pub timed_window_definition: BTreeMap<String, String>,
    pub external_snapshot_authority: String,
    pub filesystem_proof_eligible: Vec<String>,
    pub filesystem_refused: Vec<String>,
    pub filesystem_advisory_only: Vec<String>,
    pub provenance_required_fields: Vec<String>,
    pub provenance_rejected_conditions: Vec<String>,
    pub privacy_allowed_fields: Vec<String>,
    pub privacy_forbidden_fields: Vec<String>,
    pub causal_early_stops: Vec<String>,
    pub prospective_feasibility_only: bool,
    pub h1_h10_selection_authorized: bool,
    pub production_storage_authorized: bool,
    pub migration_authorized: bool,
}

impl QualificationProspectiveContractV1 {
    pub fn frozen() -> Self {
        let timing_thresholds = [
            (
                QualificationLooseBaselineOperationV1::DurableAppend,
                None,
                50_000_000,
                5_000_000,
            ),
            (
                QualificationLooseBaselineOperationV1::StrictReplay,
                None,
                500_000_000,
                10_000_000,
            ),
            (
                QualificationLooseBaselineOperationV1::FreshProcessOpenRecovery,
                None,
                750_000_000,
                25_000_000,
            ),
        ]
        .into_iter()
        .map(
            |(operation, read_class, absolute_ceiling_nanos, guard_band_nanos)| {
                QualificationProspectiveTimingThresholdV1 {
                    operation,
                    read_class,
                    absolute_ceiling_nanos,
                    relative_numerator: 125,
                    relative_denominator: 100,
                    small_baseline_guard_band_nanos: guard_band_nanos,
                }
            },
        )
        .chain(
            qualification_keyed_read_classes_v1()
                .into_iter()
                .map(|read_class| QualificationProspectiveTimingThresholdV1 {
                    operation: QualificationLooseBaselineOperationV1::KeyedRead,
                    read_class: Some(read_class),
                    absolute_ceiling_nanos: 5_000_000,
                    relative_numerator: 125,
                    relative_denominator: 100,
                    small_baseline_guard_band_nanos: 1_000_000,
                }),
        )
        .collect();
        let platforms = QualificationProspectivePlatformV1::ALL
            .into_iter()
            .map(|platform| QualificationProspectivePlatformRequirementV1 {
                platform,
                operating_system: platform.operating_system().to_owned(),
                filesystem: platform.filesystem().to_owned(),
                allocation_api: platform.allocation_api().to_owned(),
                evidence_role: platform.evidence_role().to_owned(),
            })
            .collect();
        let workloads = vec![
            QualificationProspectiveWorkloadRequirementV1 {
                workload: QualificationProspectiveWorkloadV1::P0,
                role: QualificationProspectiveWorkloadRoleV1::SemanticProvenancePrivacySmoke,
                manifest_sha256: "03cfda81e2ea988ec119b942530022b345d08b1261a6f198f87fdade2a4d1b01"
                    .to_owned(),
                generator_spec_sha256: None,
                operation_schedule_sha256: None,
                timing_required: false,
                savings_required: false,
            },
            QualificationProspectiveWorkloadRequirementV1 {
                workload: QualificationProspectiveWorkloadV1::M0,
                role: QualificationProspectiveWorkloadRoleV1::CapabilityFixedCostSentinel,
                manifest_sha256: "5d7ea2f2a8398722e2dcc853ef2c4ebe1976a02fd1585a190c9c6b86e132da7d"
                    .to_owned(),
                generator_spec_sha256: None,
                operation_schedule_sha256: None,
                timing_required: false,
                savings_required: false,
            },
            QualificationProspectiveWorkloadRequirementV1 {
                workload: QualificationProspectiveWorkloadV1::G0,
                role: QualificationProspectiveWorkloadRoleV1::DiagnosticCausalEarlyStop,
                manifest_sha256: QUALIFICATION_G0_MANIFEST_SHA256_V1.to_owned(),
                generator_spec_sha256: Some(QUALIFICATION_G0_SPEC_SHA256_V1.to_owned()),
                operation_schedule_sha256: Some(QUALIFICATION_G0_SCHEDULE_SHA256_V1.to_owned()),
                timing_required: true,
                savings_required: false,
            },
            QualificationProspectiveWorkloadRequirementV1 {
                workload: QualificationProspectiveWorkloadV1::G1,
                role: QualificationProspectiveWorkloadRoleV1::RequiredMediumFirstCrossover,
                manifest_sha256: QUALIFICATION_G1_MANIFEST_SHA256_V1.to_owned(),
                generator_spec_sha256: Some(QUALIFICATION_G1_SPEC_SHA256_V1.to_owned()),
                operation_schedule_sha256: Some(QUALIFICATION_G1_SCHEDULE_SHA256_V1.to_owned()),
                timing_required: true,
                savings_required: true,
            },
            QualificationProspectiveWorkloadRequirementV1 {
                workload: QualificationProspectiveWorkloadV1::G2,
                role: QualificationProspectiveWorkloadRoleV1::RequiredRepresentativePublic,
                manifest_sha256: QUALIFICATION_G2_MANIFEST_SHA256_V1.to_owned(),
                generator_spec_sha256: Some(QUALIFICATION_G2_SPEC_SHA256_V1.to_owned()),
                operation_schedule_sha256: Some(QUALIFICATION_G2_SCHEDULE_SHA256_V1.to_owned()),
                timing_required: true,
                savings_required: true,
            },
        ];
        let timed_window_definition = BTreeMap::from([
            (
                "durable_append".to_owned(),
                "begin before append; end only after the normal durable acknowledgement, semantic receipt, and fresh-reader visibility proof".to_owned(),
            ),
            (
                "strict_replay".to_owned(),
                "begin before strict ordered replay; end after exact count, byte total, order, and aggregate receipt verification".to_owned(),
            ),
            (
                "fresh_process_open_recovery".to_owned(),
                "include child-process startup, open/recovery, exact visible-event receipt, teardown, and result transfer".to_owned(),
            ),
            (
                "keyed_read".to_owned(),
                "begin before the independent oldest/middle/newest/absent lookup; end after the exact present-or-absent semantic receipt".to_owned(),
            ),
        ]);
        Self {
            schema: QUALIFICATION_PROSPECTIVE_CONTRACT_SCHEMA_V1.to_owned(),
            approved_proposal_sha256:
                QUALIFICATION_PROSPECTIVE_CONTRACT_PROPOSAL_SHA256_V1.to_owned(),
            derivation: QualificationProspectiveDerivationV1 {
                pointbreak_commit: "5155d1459330b111ba5eac8a4abcdc57e4107d7f".to_owned(),
                pointbreak_tree: "47182acd6c54b261618dd7e0dc6bd25713dffbd5".to_owned(),
                cargo_lock_sha256:
                    "a2ca8ebbe2d95af8ce58bee9f6b95e67f63451f75874716d9d112fbfe502976b"
                        .to_owned(),
                generator_schema: QUALIFICATION_GENERATOR_SCHEMA_V1.to_owned(),
                generator_landing_commit: "8e4894fb93a0b184f5af7340fd5b4e91751743fe"
                    .to_owned(),
                public_seed_hex: QUALIFICATION_PUBLIC_SEED_HEX_V1.to_owned(),
                calibration_version: "public_stress_envelope_v1".to_owned(),
                allowed_inputs: [
                    "current_product_tolerance",
                    "admitted_public_loose_baseline_evidence",
                    "migration_and_permanent_support_cost",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect(),
                candidate_measurements_used: false,
                historical_candidate_results_used: false,
                private_calibration_used: false,
                private_corpus_used: false,
                owner_approval_required_before_compilation: true,
                baseline_authorities: vec![
                    QualificationProspectiveBaselineAuthorityV1 {
                        platform: QualificationProspectivePlatformV1::MacosApfs,
                        evidence_sha256: "4030be89a544dfe143fdd058bb413e76e6bdc66f6815201671e460c0eac8df5a".to_owned(),
                        file_sha256: "6ac32f94d4f0379ca0fe94edbe1c669b724bce064177aade4b6467e54ce54b65".to_owned(),
                    },
                    QualificationProspectiveBaselineAuthorityV1 {
                        platform: QualificationProspectivePlatformV1::LinuxExt4,
                        evidence_sha256: "75d06dbcffc100ea9f4d6d082f9229860961e20f6d6634be94675d0407b8e9ed".to_owned(),
                        file_sha256: "134584e827618111c12929f20fe7d662d37dff42218ae0437642c282824baea8".to_owned(),
                    },
                    QualificationProspectiveBaselineAuthorityV1 {
                        platform: QualificationProspectivePlatformV1::WindowsNtfs,
                        evidence_sha256: "03c7115df99fa5b3ac9098e7902557bcd479f624602dde2c8fc590b02ff532b1".to_owned(),
                        file_sha256: "5a9f9abd8a6af6ad2c7b706a2a426d17c7b70aad6b87ddde06c8dba6a90e42bc".to_owned(),
                    },
                ],
            },
            run_controls: QualificationProspectiveRunControlsV1 {
                warmup_iterations: 3,
                measured_iterations: 30,
                independent_runs: 2,
                p95_estimator: "ascending_nearest_rank".to_owned(),
                p95_rank: 29,
                sample_retention: "raw".to_owned(),
                outlier_policy: "retain_all".to_owned(),
            },
            operations: QualificationLooseBaselineOperationV1::ALL.to_vec(),
            keyed_read_classes: qualification_keyed_read_classes_v1().to_vec(),
            timing_thresholds,
            timing_combination_formula: "candidate_p95_ns <= absolute_ceiling_ns AND candidate_p95_ns <= max(ceil(loose_p95_ns * relative_numerator / relative_denominator), loose_p95_ns + guard_band_ns); each platform, workload, independent run, operation, and keyed-read class gates separately with no pooling or offset".to_owned(),
            platforms,
            workloads,
            allocation: QualificationProspectiveAllocationPolicyV1 {
                event_fixed_overhead_bytes: 1_048_576,
                complete_profile_fixed_overhead_bytes: 2_097_152,
                event_peak_headroom_bytes: 1_048_576,
                complete_profile_peak_headroom_bytes: 2_097_152,
                first_required_crossover_workload: QualificationProspectiveWorkloadV1::G1,
                event_savings_percent: 25,
                complete_profile_savings_percent: 10,
                savings_workloads: vec![
                    QualificationProspectiveWorkloadV1::G1,
                    QualificationProspectiveWorkloadV1::G2,
                ],
                states: QualificationPerformanceInventoryStateV1::ALL.to_vec(),
                high_water_numerator: 150,
                high_water_denominator: 100,
                carrier_accounting: "all_profile_owned_event_content_index_log_manifest_metadata_old_generation_and_temporary_carriers".to_owned(),
                virtual_address_reservation_excluded: true,
            },
            maintenance: QualificationProspectiveMaintenancePolicyV1 {
                foreground_p95_max_nanos: 250_000_000,
                g1_total_max_nanos: 5_000_000_000,
                g2_total_max_nanos: 30_000_000_000,
                not_applicable_requires_mechanism_proof: true,
            },
            timed_window_definition,
            external_snapshot_authority: "owner_local_sanitized_corroboration_and_adoption_veto_after_all_public_gates; never a public row, never transported, never pooled, and never able to turn a public failure into a pass".to_owned(),
            filesystem_proof_eligible: ["apfs", "ext4", "ntfs"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            filesystem_refused: ["nfs", "smb", "cifs", "sshfs", "fuse", "overlay"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            filesystem_advisory_only: vec!["cloud_synced_or_unknown".to_owned()],
            provenance_required_fields: [
                "source_commit",
                "source_tree",
                "cargo_lock_sha256",
                "contract_sha256",
                "approved_proposal_sha256",
                "generator_schema",
                "public_seed_hex",
                "generator_spec_sha256",
                "manifest_sha256",
                "operation_schedule_sha256",
                "platform",
                "filesystem",
                "allocation_api",
                "controls",
                "run_index",
                "semantic_receipt_sha256",
                "baseline_evidence_sha256",
                "evidence_sha256",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            provenance_rejected_conditions: [
                "missing",
                "stale",
                "duplicate",
                "mixed_revision",
                "wrong_hash",
                "noncanonical",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            privacy_allowed_fields: [
                "aggregate_receipts",
                "carrier_set_hashes",
                "counts",
                "byte_totals",
                "timings",
                "allocation",
                "toolchain_identity",
                "platform_identity",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            privacy_forbidden_fields: [
                "paths",
                "environment_values",
                "payloads",
                "logical_keys",
                "record_level_hashes",
                "error_text",
                "private_corpus_material",
                "candidate_results_in_publication",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            causal_early_stops: [
                "stop_on_semantic_receipt_mismatch_or_inexact_create_once",
                "stop_on_acknowledged_loss_ambiguous_retry_failure_or_fresh_process_visibility_failure",
                "stop_on_missing_stale_mixed_or_noncanonical_provenance",
                "stop_on_unsupported_native_platform_filesystem_or_allocation_api",
                "stop_on_unknown_or_incomplete_carrier_inventory",
                "stop_after_g0_if_any_operation_exceeds_its_absolute_product_ceiling",
                "stop_after_g1_if_event_or_complete_profile_fails_crossover_or_savings_in_any_state",
                "stop_after_g1_if_high_water_amplification_or_maintenance_duration_exceeds_budget",
                "stop_on_superlinear_retained_growth_unbounded_reader_retention_or_unbounded_maintenance",
                "stop_on_private_data_path_key_payload_error_or_candidate_result_leakage",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            prospective_feasibility_only: true,
            h1_h10_selection_authorized: false,
            production_storage_authorized: false,
            migration_authorized: false,
        }
    }

    pub fn canonical_sha256(&self) -> Result<String, String> {
        let value = serde_json::to_value(self).map_err(|error| error.to_string())?;
        canonical_json_bytes(&value)
            .map(|bytes| sha256_bytes_hex(&bytes))
            .map_err(|error| error.to_string())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self != &Self::frozen() {
            return Err("unsupported prospective feasibility contract".to_owned());
        }
        if self.canonical_sha256()? != QUALIFICATION_PROSPECTIVE_CONTRACT_SHA256_V1 {
            return Err("prospective feasibility contract hash is not frozen".to_owned());
        }
        for hash in self
            .derivation
            .baseline_authorities
            .iter()
            .flat_map(|authority| [&authority.evidence_sha256, &authority.file_sha256])
            .chain(self.workloads.iter().flat_map(|workload| {
                std::iter::once(&workload.manifest_sha256)
                    .chain(workload.generator_spec_sha256.iter())
                    .chain(workload.operation_schedule_sha256.iter())
            }))
        {
            validate_hex(hash, 64, "prospective contract SHA-256")?;
        }
        Ok(())
    }

    pub fn decision_table_markdown(&self) -> String {
        let mut rows = vec![
            "| Decision | Required value |".to_owned(),
            "| --- | --- |".to_owned(),
            format!(
                "| Native platforms | {} |",
                self.platforms
                    .iter()
                    .map(|platform| platform.platform.display_name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            "| Timing | absolute ceiling and relative 125% or operation guard band; every operation/read class/run stands alone |".to_owned(),
            format!(
                "| G1/G2 event allocation | at least {}% savings in steady, reopened, and high-water states |",
                self.allocation.event_savings_percent
            ),
            format!(
                "| G1/G2 complete profile | at least {}% savings in steady, reopened, and high-water states |",
                self.allocation.complete_profile_savings_percent
            ),
            "| First crossover | G1; candidate allocation must be strictly below paired loose allocation |".to_owned(),
            format!(
                "| High water | at most {}/{} of candidate steady allocation and still satisfies savings |",
                self.allocation.high_water_numerator, self.allocation.high_water_denominator
            ),
            format!(
                "| Maintenance | foreground p95 <= {} ms; G1 total <= {} s; G2 total <= {} s |",
                self.maintenance.foreground_p95_max_nanos / 1_000_000,
                self.maintenance.g1_total_max_nanos / 1_000_000_000,
                self.maintenance.g2_total_max_nanos / 1_000_000_000,
            ),
            "| Authority | public G1/G2 gates decide feasibility; owner-local external corroboration can veto adoption but cannot rescue a failure |".to_owned(),
            "| Meaning | prospective feasibility only; no H1-H10 selection, production storage, or migration authorization |".to_owned(),
        ];
        rows.push(format!(
            "| Contract | `{}` from approved proposal `{}` |",
            self.canonical_sha256()
                .expect("the frozen prospective contract is canonical"),
            self.approved_proposal_sha256
        ));
        rows.join("\n")
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationProspectiveContractPublicationV1 {
    pub schema: String,
    pub mode: String,
    pub contract: QualificationProspectiveContractV1,
    pub contract_sha256: String,
    pub decision_table_markdown: String,
}

pub fn qualification_prospective_contract_v1_publication()
-> QualificationProspectiveContractPublicationV1 {
    let contract = QualificationProspectiveContractV1::frozen();
    QualificationProspectiveContractPublicationV1 {
        schema: QUALIFICATION_PROSPECTIVE_CONTRACT_PUBLICATION_SCHEMA_V1.to_owned(),
        mode: "non_timing_contract_publication".to_owned(),
        contract_sha256: contract
            .canonical_sha256()
            .expect("the frozen prospective contract is canonical"),
        decision_table_markdown: contract.decision_table_markdown(),
        contract,
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationProspectiveCriterionStatusV1 {
    Passed,
    Failed,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationProspectiveCriterionKindV1 {
    Protocol,
    Timing,
    DiagnosticAllocation,
    SmallStoreFixedOverhead,
    SmallStorePeakHeadroom,
    AllocationSavings,
    FirstCrossover,
    HighWaterAmplification,
    MaintenanceForeground,
    MaintenanceTotal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationProspectiveExternalCorroborationV1 {
    Satisfied,
    Vetoed,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationProspectiveEvidenceControlsV1 {
    pub fresh_root: bool,
    pub native_execution: bool,
    pub quiesced_host: bool,
    pub semantic_receipt_verified: bool,
    pub durable_acknowledgement_verified: bool,
    pub fresh_process_visibility_verified: bool,
    pub carrier_inventory_complete: bool,
}

impl QualificationProspectiveEvidenceControlsV1 {
    fn all_satisfied(&self) -> bool {
        self.fresh_root
            && self.native_execution
            && self.quiesced_host
            && self.semantic_receipt_verified
            && self.durable_acknowledgement_verified
            && self.fresh_process_visibility_verified
            && self.carrier_inventory_complete
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationProspectiveTimingEvidenceV1 {
    pub operation: QualificationLooseBaselineOperationV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_class: Option<QualificationKeyedReadClassV1>,
    pub candidate_samples_nanos: Vec<u64>,
    pub baseline_samples_nanos: Vec<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationProspectiveAllocationEvidenceV1 {
    pub scope: QualificationPerformanceAllocationScopeV2,
    pub state: QualificationPerformanceInventoryStateV1,
    pub candidate_logical_bytes: u64,
    pub candidate_allocated_bytes: u64,
    pub baseline_allocated_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationProspectiveMaintenanceEvidenceV1 {
    pub required: bool,
    pub foreground_samples_nanos: Vec<u64>,
    pub total_nanos: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_applicable_mechanism_proof_sha256: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationProspectiveRunEvidenceV1 {
    pub platform: QualificationProspectivePlatformV1,
    pub workload: QualificationProspectiveWorkloadV1,
    pub run_index: u32,
    pub manifest_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generator_spec_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_schedule_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_evidence_sha256: Option<String>,
    pub allocation_api: String,
    pub controls: QualificationProspectiveEvidenceControlsV1,
    pub semantic_receipt_sha256: String,
    pub timing: Vec<QualificationProspectiveTimingEvidenceV1>,
    pub allocations: Vec<QualificationProspectiveAllocationEvidenceV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maintenance: Option<QualificationProspectiveMaintenanceEvidenceV1>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationProspectiveEvidenceV1 {
    pub schema: String,
    pub contract_schema: String,
    pub contract_sha256: String,
    pub approved_proposal_sha256: String,
    pub source_commit: String,
    pub source_tree: String,
    pub cargo_lock_sha256: String,
    pub generator_schema: String,
    pub public_seed_hex: String,
    pub profile_id: String,
    pub runs: Vec<QualificationProspectiveRunEvidenceV1>,
    pub evidence_sha256: String,
}

impl QualificationProspectiveEvidenceV1 {
    pub fn canonical_sha256(&self) -> Result<String, String> {
        let mut preimage = self.clone();
        preimage.evidence_sha256.clear();
        let value = serde_json::to_value(preimage).map_err(|error| error.to_string())?;
        canonical_json_bytes(&value)
            .map(|bytes| sha256_bytes_hex(&bytes))
            .map_err(|error| error.to_string())
    }

    pub fn validate(&self) -> Result<(), String> {
        let contract = QualificationProspectiveContractV1::frozen();
        contract.validate()?;
        if self.schema != QUALIFICATION_PROSPECTIVE_EVIDENCE_SCHEMA_V1
            || self.contract_schema != contract.schema
            || self.contract_sha256 != contract.canonical_sha256()?
            || self.approved_proposal_sha256 != contract.approved_proposal_sha256
        {
            return Err("prospective evidence uses a different contract".to_owned());
        }
        if self.source_commit != contract.derivation.pointbreak_commit
            || self.source_tree != contract.derivation.pointbreak_tree
            || self.cargo_lock_sha256 != contract.derivation.cargo_lock_sha256
            || self.generator_schema != contract.derivation.generator_schema
            || self.public_seed_hex != contract.derivation.public_seed_hex
        {
            return Err("prospective evidence uses stale derivation identities".to_owned());
        }
        if self.profile_id.trim().is_empty() {
            return Err("prospective evidence profile identity is missing".to_owned());
        }
        validate_hex(&self.evidence_sha256, 64, "prospective evidence SHA-256")?;
        if self.evidence_sha256 != self.canonical_sha256()? {
            return Err("prospective evidence hash does not match its preimage".to_owned());
        }

        let mut run_keys = BTreeSet::new();
        for run in &self.runs {
            validate_prospective_run_v1(run, &contract)?;
            if !run_keys.insert((run.platform, run.workload, run.run_index)) {
                return Err("prospective evidence contains a duplicate run".to_owned());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationProspectiveCriterionV1 {
    pub kind: QualificationProspectiveCriterionKindV1,
    pub platform: QualificationProspectivePlatformV1,
    pub workload: QualificationProspectiveWorkloadV1,
    pub run_index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<QualificationLooseBaselineOperationV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_class: Option<QualificationKeyedReadClassV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allocation_scope: Option<QualificationPerformanceAllocationScopeV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inventory_state: Option<QualificationPerformanceInventoryStateV1>,
    pub status: QualificationProspectiveCriterionStatusV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_value: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_value: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationProspectiveEvaluationV1 {
    pub schema: String,
    pub contract_sha256: String,
    pub evidence_sha256: String,
    pub status: QualificationProspectiveCriterionStatusV1,
    pub eligible: bool,
    pub criteria: Vec<QualificationProspectiveCriterionV1>,
}

pub fn prospective_timing_limit_nanos_v1(
    threshold: &QualificationProspectiveTimingThresholdV1,
    baseline_p95_nanos: u64,
) -> u64 {
    let relative = u128::from(baseline_p95_nanos)
        .saturating_mul(u128::from(threshold.relative_numerator))
        .div_ceil(u128::from(threshold.relative_denominator));
    let guard = baseline_p95_nanos.saturating_add(threshold.small_baseline_guard_band_nanos);
    let dynamic = relative.max(u128::from(guard)).min(u128::from(u64::MAX)) as u64;
    threshold.absolute_ceiling_nanos.min(dynamic)
}

pub fn prospective_timing_status_v1(
    threshold: &QualificationProspectiveTimingThresholdV1,
    baseline_p95_nanos: u64,
    candidate_p95_nanos: u64,
) -> QualificationProspectiveCriterionStatusV1 {
    prospective_at_or_below_status_v1(
        candidate_p95_nanos,
        prospective_timing_limit_nanos_v1(threshold, baseline_p95_nanos),
    )
}

pub fn prospective_at_or_below_status_v1(
    candidate: u64,
    limit: u64,
) -> QualificationProspectiveCriterionStatusV1 {
    if candidate <= limit {
        QualificationProspectiveCriterionStatusV1::Passed
    } else {
        QualificationProspectiveCriterionStatusV1::Failed
    }
}

pub fn prospective_savings_status_v1(
    candidate_bytes: u64,
    baseline_bytes: u64,
    savings_percent: u32,
) -> QualificationProspectiveCriterionStatusV1 {
    if baseline_bytes == 0 {
        return QualificationProspectiveCriterionStatusV1::Unknown;
    }
    if candidate_bytes <= prospective_savings_limit_bytes_v1(baseline_bytes, savings_percent) {
        QualificationProspectiveCriterionStatusV1::Passed
    } else {
        QualificationProspectiveCriterionStatusV1::Failed
    }
}

fn prospective_savings_limit_bytes_v1(baseline_bytes: u64, savings_percent: u32) -> u64 {
    let limit = u128::from(baseline_bytes)
        .saturating_mul(u128::from(100_u32.saturating_sub(savings_percent)))
        / 100;
    limit.min(u128::from(u64::MAX)) as u64
}

fn prospective_ceiling_ratio_v1(value: u64, numerator: u32, denominator: u32) -> u64 {
    let ceiling = u128::from(value)
        .saturating_mul(u128::from(numerator))
        .div_ceil(u128::from(denominator));
    ceiling.min(u128::from(u64::MAX)) as u64
}

pub fn prospective_crossover_status_v1(
    candidate_bytes: u64,
    baseline_bytes: u64,
) -> QualificationProspectiveCriterionStatusV1 {
    if baseline_bytes == 0 {
        QualificationProspectiveCriterionStatusV1::Unknown
    } else if candidate_bytes < baseline_bytes {
        QualificationProspectiveCriterionStatusV1::Passed
    } else {
        QualificationProspectiveCriterionStatusV1::Failed
    }
}

pub fn prospective_nearest_rank_p95_v1(samples: &[u64]) -> Option<u64> {
    if samples.is_empty() {
        return None;
    }
    let mut ordered = samples.to_vec();
    ordered.sort_unstable();
    let rank = ordered.len().saturating_mul(95).div_ceil(100).max(1);
    ordered.get(rank - 1).copied()
}

pub fn apply_prospective_external_corroboration_v1(
    public_status: QualificationProspectiveCriterionStatusV1,
    corroboration: QualificationProspectiveExternalCorroborationV1,
) -> QualificationProspectiveCriterionStatusV1 {
    match (public_status, corroboration) {
        (
            QualificationProspectiveCriterionStatusV1::Passed,
            QualificationProspectiveExternalCorroborationV1::Vetoed,
        ) => QualificationProspectiveCriterionStatusV1::Failed,
        (status, _) => status,
    }
}

fn validate_prospective_run_v1(
    run: &QualificationProspectiveRunEvidenceV1,
    contract: &QualificationProspectiveContractV1,
) -> Result<(), String> {
    let measured_iterations = contract.run_controls.measured_iterations as usize;
    let platform = contract
        .platforms
        .iter()
        .find(|requirement| requirement.platform == run.platform)
        .ok_or_else(|| "prospective run uses an unsupported platform".to_owned())?;
    let workload = contract
        .workloads
        .iter()
        .find(|requirement| requirement.workload == run.workload)
        .ok_or_else(|| "prospective run uses an unsupported workload".to_owned())?;
    if !(1..=contract.run_controls.independent_runs).contains(&run.run_index)
        || run.manifest_sha256 != workload.manifest_sha256
        || run.generator_spec_sha256 != workload.generator_spec_sha256
        || run.operation_schedule_sha256 != workload.operation_schedule_sha256
        || run.allocation_api != platform.allocation_api
    {
        return Err("prospective run provenance does not match the contract".to_owned());
    }
    let expected_baseline = contract
        .derivation
        .baseline_authorities
        .iter()
        .find(|authority| authority.platform == run.platform)
        .map(|authority| authority.evidence_sha256.as_str())
        .ok_or_else(|| "prospective run has no baseline authority".to_owned())?;
    if run.baseline_evidence_sha256.as_deref() != Some(expected_baseline) {
        return Err("prospective run baseline identity does not match the contract".to_owned());
    }
    validate_hex(
        &run.semantic_receipt_sha256,
        64,
        "prospective semantic receipt SHA-256",
    )?;
    let mut timing_keys = BTreeSet::new();
    for row in &run.timing {
        if !timing_keys.insert((row.operation, row.read_class)) {
            return Err("prospective run contains a duplicate timing row".to_owned());
        }
        let valid_axis = contract.timing_thresholds.iter().any(|threshold| {
            threshold.operation == row.operation && threshold.read_class == row.read_class
        });
        if !valid_axis {
            return Err("prospective run contains an unsupported timing row".to_owned());
        }
        if row.candidate_samples_nanos.len() > measured_iterations
            || row.baseline_samples_nanos.len() > measured_iterations
        {
            return Err("prospective run contains too many timing samples".to_owned());
        }
    }
    let mut allocation_keys = BTreeSet::new();
    for row in &run.allocations {
        if !allocation_keys.insert((row.scope, row.state)) {
            return Err("prospective run contains a duplicate allocation row".to_owned());
        }
        if row.candidate_logical_bytes == 0
            || row.candidate_allocated_bytes == 0
            || row.baseline_allocated_bytes == 0
        {
            return Err("prospective run contains an incomplete allocation row".to_owned());
        }
    }
    if let Some(maintenance) = &run.maintenance {
        if maintenance.required {
            if maintenance.not_applicable_mechanism_proof_sha256.is_some()
                || maintenance.foreground_samples_nanos.len() > measured_iterations
            {
                return Err("prospective maintenance evidence is inconsistent".to_owned());
            }
        } else {
            let proof = maintenance
                .not_applicable_mechanism_proof_sha256
                .as_deref()
                .ok_or_else(|| "maintenance N/A requires mechanism proof".to_owned())?;
            validate_hex(proof, 64, "maintenance N/A mechanism proof SHA-256")?;
            if !maintenance.foreground_samples_nanos.is_empty() || maintenance.total_nanos != 0 {
                return Err("maintenance N/A contains measurements".to_owned());
            }
        }
    }
    Ok(())
}

fn prospective_criterion_v1(
    kind: QualificationProspectiveCriterionKindV1,
    run: &QualificationProspectiveRunEvidenceV1,
    status: QualificationProspectiveCriterionStatusV1,
    message: impl Into<String>,
) -> QualificationProspectiveCriterionV1 {
    QualificationProspectiveCriterionV1 {
        kind,
        platform: run.platform,
        workload: run.workload,
        run_index: run.run_index,
        operation: None,
        read_class: None,
        allocation_scope: None,
        inventory_state: None,
        status,
        candidate_value: None,
        baseline_value: None,
        limit: None,
        message: message.into(),
    }
}

fn prospective_missing_criterion_v1(
    platform: QualificationProspectivePlatformV1,
    workload: QualificationProspectiveWorkloadV1,
    run_index: u32,
) -> QualificationProspectiveCriterionV1 {
    QualificationProspectiveCriterionV1 {
        kind: QualificationProspectiveCriterionKindV1::Protocol,
        platform,
        workload,
        run_index,
        operation: None,
        read_class: None,
        allocation_scope: None,
        inventory_state: None,
        status: QualificationProspectiveCriterionStatusV1::Unknown,
        candidate_value: None,
        baseline_value: None,
        limit: None,
        message: "required prospective evidence run is missing".to_owned(),
    }
}

pub fn evaluate_qualification_prospective_v1(
    evidence: &QualificationProspectiveEvidenceV1,
) -> Result<QualificationProspectiveEvaluationV1, String> {
    evidence.validate()?;
    let contract = QualificationProspectiveContractV1::frozen();
    let mut criteria = Vec::new();
    for platform in QualificationProspectivePlatformV1::ALL {
        for workload in QualificationProspectiveWorkloadV1::ALL {
            for run_index in 1..=contract.run_controls.independent_runs {
                let Some(run) = evidence.runs.iter().find(|run| {
                    (run.platform, run.workload, run.run_index) == (platform, workload, run_index)
                }) else {
                    criteria.push(prospective_missing_criterion_v1(
                        platform, workload, run_index,
                    ));
                    continue;
                };
                criteria.push(prospective_criterion_v1(
                    QualificationProspectiveCriterionKindV1::Protocol,
                    run,
                    if run.controls.all_satisfied() {
                        QualificationProspectiveCriterionStatusV1::Passed
                    } else {
                        QualificationProspectiveCriterionStatusV1::Failed
                    },
                    "native run controls, provenance, and semantic receipt",
                ));
                evaluate_prospective_timing_v1(run, &contract, &mut criteria);
                evaluate_prospective_allocations_v1(run, &contract, &mut criteria);
                evaluate_prospective_maintenance_v1(run, &contract, &mut criteria);
            }
        }
    }
    let status = aggregate_prospective_status_v1(criteria.iter().map(|row| row.status));
    Ok(QualificationProspectiveEvaluationV1 {
        schema: QUALIFICATION_PROSPECTIVE_EVALUATION_SCHEMA_V1.to_owned(),
        contract_sha256: evidence.contract_sha256.clone(),
        evidence_sha256: evidence.evidence_sha256.clone(),
        status,
        eligible: status == QualificationProspectiveCriterionStatusV1::Passed,
        criteria,
    })
}

fn evaluate_prospective_timing_v1(
    run: &QualificationProspectiveRunEvidenceV1,
    contract: &QualificationProspectiveContractV1,
    criteria: &mut Vec<QualificationProspectiveCriterionV1>,
) {
    if !run.workload.timing_required() {
        return;
    }
    let measured_iterations = contract.run_controls.measured_iterations as usize;
    for threshold in &contract.timing_thresholds {
        let row = run.timing.iter().find(|row| {
            row.operation == threshold.operation && row.read_class == threshold.read_class
        });
        let complete_row = row.filter(|row| {
            row.candidate_samples_nanos.len() == measured_iterations
                && row.baseline_samples_nanos.len() == measured_iterations
        });
        let p95 = complete_row.and_then(|row| {
            Some((
                prospective_nearest_rank_p95_v1(&row.candidate_samples_nanos)?,
                prospective_nearest_rank_p95_v1(&row.baseline_samples_nanos)?,
            ))
        });
        let (status, candidate, baseline, limit, message) = if let Some((candidate, baseline)) = p95
        {
            let limit = if run.workload == QualificationProspectiveWorkloadV1::G0 {
                threshold.absolute_ceiling_nanos
            } else {
                prospective_timing_limit_nanos_v1(threshold, baseline)
            };
            let message = if run.workload == QualificationProspectiveWorkloadV1::G0 {
                "G0 candidate p95 must satisfy the absolute causal-stop ceiling"
            } else {
                "candidate p95 must satisfy both the absolute and dynamic ceiling"
            };
            (
                prospective_at_or_below_status_v1(candidate, limit),
                Some(candidate),
                Some(baseline),
                Some(limit),
                message,
            )
        } else {
            (
                QualificationProspectiveCriterionStatusV1::Unknown,
                None,
                None,
                None,
                "required timing samples are missing or incomplete",
            )
        };
        let mut criterion = prospective_criterion_v1(
            QualificationProspectiveCriterionKindV1::Timing,
            run,
            status,
            message,
        );
        criterion.operation = Some(threshold.operation);
        criterion.read_class = threshold.read_class;
        criterion.candidate_value = candidate;
        criterion.baseline_value = baseline;
        criterion.limit = limit;
        criteria.push(criterion);
    }
}

fn evaluate_prospective_allocations_v1(
    run: &QualificationProspectiveRunEvidenceV1,
    contract: &QualificationProspectiveContractV1,
    criteria: &mut Vec<QualificationProspectiveCriterionV1>,
) {
    for scope in QualificationPerformanceAllocationScopeV2::ALL {
        let steady = run.allocations.iter().find(|row| {
            row.scope == scope && row.state == QualificationPerformanceInventoryStateV1::Steady
        });
        for state in QualificationPerformanceInventoryStateV1::ALL {
            let row = run
                .allocations
                .iter()
                .find(|row| row.scope == scope && row.state == state);
            let mut push = |kind, status, candidate, baseline, limit, message| {
                let mut criterion = prospective_criterion_v1(kind, run, status, message);
                criterion.allocation_scope = Some(scope);
                criterion.inventory_state = Some(state);
                criterion.candidate_value = candidate;
                criterion.baseline_value = baseline;
                criterion.limit = limit;
                criteria.push(criterion);
            };
            match run.workload {
                QualificationProspectiveWorkloadV1::P0 | QualificationProspectiveWorkloadV1::M0 => {
                    let (kind, cap, baseline) =
                        if state == QualificationPerformanceInventoryStateV1::HighWater {
                            (
                                QualificationProspectiveCriterionKindV1::SmallStorePeakHeadroom,
                                contract.allocation.peak_headroom_cap(scope),
                                steady.map(|row| row.candidate_allocated_bytes),
                            )
                        } else {
                            (
                                QualificationProspectiveCriterionKindV1::SmallStoreFixedOverhead,
                                contract.allocation.fixed_overhead_cap(scope),
                                row.map(|row| row.candidate_logical_bytes),
                            )
                        };
                    match (row, baseline) {
                        (Some(row), Some(baseline)) => {
                            let limit = baseline.saturating_add(cap);
                            push(
                                kind,
                                prospective_at_or_below_status_v1(
                                    row.candidate_allocated_bytes,
                                    limit,
                                ),
                                Some(row.candidate_allocated_bytes),
                                Some(baseline),
                                Some(limit),
                                "small-store allocation must remain within fixed headroom",
                            );
                        }
                        _ => push(
                            kind,
                            QualificationProspectiveCriterionStatusV1::Unknown,
                            None,
                            baseline,
                            None,
                            "required small-store allocation row is missing",
                        ),
                    }
                }
                QualificationProspectiveWorkloadV1::G0 => match row {
                    Some(row) => push(
                        QualificationProspectiveCriterionKindV1::DiagnosticAllocation,
                        QualificationProspectiveCriterionStatusV1::Passed,
                        Some(row.candidate_allocated_bytes),
                        Some(row.baseline_allocated_bytes),
                        None,
                        "G0 allocation is diagnostic but must be present",
                    ),
                    None => push(
                        QualificationProspectiveCriterionKindV1::DiagnosticAllocation,
                        QualificationProspectiveCriterionStatusV1::Unknown,
                        None,
                        None,
                        None,
                        "required G0 diagnostic allocation row is missing",
                    ),
                },
                QualificationProspectiveWorkloadV1::G1 | QualificationProspectiveWorkloadV1::G2 => {
                    let savings_percent = contract.allocation.savings_percent(scope);
                    match row {
                        Some(row) => {
                            push(
                                QualificationProspectiveCriterionKindV1::AllocationSavings,
                                prospective_savings_status_v1(
                                    row.candidate_allocated_bytes,
                                    row.baseline_allocated_bytes,
                                    savings_percent,
                                ),
                                Some(row.candidate_allocated_bytes),
                                Some(row.baseline_allocated_bytes),
                                Some(prospective_savings_limit_bytes_v1(
                                    row.baseline_allocated_bytes,
                                    savings_percent,
                                )),
                                "candidate allocation must satisfy the scope savings floor",
                            );
                            if run.workload == contract.allocation.first_required_crossover_workload
                            {
                                push(
                                    QualificationProspectiveCriterionKindV1::FirstCrossover,
                                    prospective_crossover_status_v1(
                                        row.candidate_allocated_bytes,
                                        row.baseline_allocated_bytes,
                                    ),
                                    Some(row.candidate_allocated_bytes),
                                    Some(row.baseline_allocated_bytes),
                                    None,
                                    "G1 candidate allocation must be strictly below loose",
                                );
                            }
                            if state == QualificationPerformanceInventoryStateV1::HighWater {
                                if let Some(steady) = steady {
                                    let limit = prospective_ceiling_ratio_v1(
                                        steady.candidate_allocated_bytes,
                                        contract.allocation.high_water_numerator,
                                        contract.allocation.high_water_denominator,
                                    );
                                    push(
                                        QualificationProspectiveCriterionKindV1::HighWaterAmplification,
                                        prospective_at_or_below_status_v1(
                                            row.candidate_allocated_bytes,
                                            limit,
                                        ),
                                        Some(row.candidate_allocated_bytes),
                                        Some(steady.candidate_allocated_bytes),
                                        Some(limit),
                                        "high-water allocation must remain within steady-state amplification",
                                    );
                                } else {
                                    push(
                                        QualificationProspectiveCriterionKindV1::HighWaterAmplification,
                                        QualificationProspectiveCriterionStatusV1::Unknown,
                                        Some(row.candidate_allocated_bytes),
                                        None,
                                        None,
                                        "steady allocation is missing for high-water comparison",
                                    );
                                }
                            }
                        }
                        None => push(
                            QualificationProspectiveCriterionKindV1::AllocationSavings,
                            QualificationProspectiveCriterionStatusV1::Unknown,
                            None,
                            None,
                            None,
                            "required quantitative allocation row is missing",
                        ),
                    }
                }
            }
        }
    }
}

fn evaluate_prospective_maintenance_v1(
    run: &QualificationProspectiveRunEvidenceV1,
    contract: &QualificationProspectiveContractV1,
    criteria: &mut Vec<QualificationProspectiveCriterionV1>,
) {
    if !matches!(
        run.workload,
        QualificationProspectiveWorkloadV1::G1 | QualificationProspectiveWorkloadV1::G2
    ) {
        return;
    }
    let Some(maintenance) = &run.maintenance else {
        for kind in [
            QualificationProspectiveCriterionKindV1::MaintenanceForeground,
            QualificationProspectiveCriterionKindV1::MaintenanceTotal,
        ] {
            criteria.push(prospective_criterion_v1(
                kind,
                run,
                QualificationProspectiveCriterionStatusV1::Unknown,
                "required maintenance evidence is missing",
            ));
        }
        return;
    };
    if !maintenance.required {
        for kind in [
            QualificationProspectiveCriterionKindV1::MaintenanceForeground,
            QualificationProspectiveCriterionKindV1::MaintenanceTotal,
        ] {
            criteria.push(prospective_criterion_v1(
                kind,
                run,
                QualificationProspectiveCriterionStatusV1::Passed,
                "maintenance is inapplicable and a mechanism proof is present",
            ));
        }
        return;
    }
    let foreground =
        prospective_nearest_rank_p95_v1(&maintenance.foreground_samples_nanos).filter(|_| {
            maintenance.foreground_samples_nanos.len()
                == contract.run_controls.measured_iterations as usize
        });
    let mut foreground_criterion = prospective_criterion_v1(
        QualificationProspectiveCriterionKindV1::MaintenanceForeground,
        run,
        foreground.map_or(
            QualificationProspectiveCriterionStatusV1::Unknown,
            |value| {
                prospective_at_or_below_status_v1(
                    value,
                    contract.maintenance.foreground_p95_max_nanos,
                )
            },
        ),
        "maintenance foreground p95 must remain within budget",
    );
    foreground_criterion.candidate_value = foreground;
    foreground_criterion.limit = Some(contract.maintenance.foreground_p95_max_nanos);
    criteria.push(foreground_criterion);

    let total_limit = contract.maintenance.total_max_nanos(run.workload);
    let mut total_criterion = prospective_criterion_v1(
        QualificationProspectiveCriterionKindV1::MaintenanceTotal,
        run,
        prospective_at_or_below_status_v1(maintenance.total_nanos, total_limit),
        "maintenance completion time must remain within the workload budget",
    );
    total_criterion.candidate_value = Some(maintenance.total_nanos);
    total_criterion.limit = Some(total_limit);
    criteria.push(total_criterion);
}

fn aggregate_prospective_status_v1(
    statuses: impl IntoIterator<Item = QualificationProspectiveCriterionStatusV1>,
) -> QualificationProspectiveCriterionStatusV1 {
    let mut aggregate = QualificationProspectiveCriterionStatusV1::Passed;
    for status in statuses {
        match status {
            QualificationProspectiveCriterionStatusV1::Failed => {
                return QualificationProspectiveCriterionStatusV1::Failed;
            }
            QualificationProspectiveCriterionStatusV1::Unknown => {
                aggregate = QualificationProspectiveCriterionStatusV1::Unknown;
            }
            QualificationProspectiveCriterionStatusV1::Passed => {}
        }
    }
    aggregate
}

#[cfg(test)]
impl QualificationProspectiveEvidenceV1 {
    fn fixture_for_tests() -> Self {
        let contract = QualificationProspectiveContractV1::frozen();
        let mut runs = Vec::new();
        for platform in QualificationProspectivePlatformV1::ALL {
            let platform_requirement = contract
                .platforms
                .iter()
                .find(|requirement| requirement.platform == platform)
                .expect("fixture platform");
            let baseline_evidence_sha256 = contract
                .derivation
                .baseline_authorities
                .iter()
                .find(|authority| authority.platform == platform)
                .expect("fixture baseline")
                .evidence_sha256
                .clone();
            for workload in QualificationProspectiveWorkloadV1::ALL {
                let workload_requirement = contract
                    .workloads
                    .iter()
                    .find(|requirement| requirement.workload == workload)
                    .expect("fixture workload");
                for run_index in 1..=contract.run_controls.independent_runs {
                    let timing = if workload.timing_required() {
                        contract
                            .timing_thresholds
                            .iter()
                            .map(|threshold| QualificationProspectiveTimingEvidenceV1 {
                                operation: threshold.operation,
                                read_class: threshold.read_class,
                                candidate_samples_nanos: vec![10_000; 30],
                                baseline_samples_nanos: vec![10_000; 30],
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    let mut allocations = Vec::new();
                    for scope in QualificationPerformanceAllocationScopeV2::ALL {
                        for state in QualificationPerformanceInventoryStateV1::ALL {
                            let (logical, candidate, baseline) = match workload {
                                QualificationProspectiveWorkloadV1::P0
                                | QualificationProspectiveWorkloadV1::M0 => {
                                    let logical = 1_000_000;
                                    let steady =
                                        logical + contract.allocation.fixed_overhead_cap(scope);
                                    let candidate = if state
                                        == QualificationPerformanceInventoryStateV1::HighWater
                                    {
                                        steady + contract.allocation.peak_headroom_cap(scope)
                                    } else {
                                        steady
                                    };
                                    (logical, candidate, candidate + 1_000_000)
                                }
                                QualificationProspectiveWorkloadV1::G0 => (8_000, 8_000, 10_000),
                                QualificationProspectiveWorkloadV1::G1
                                | QualificationProspectiveWorkloadV1::G2 => {
                                    let candidate = match scope {
                                        QualificationPerformanceAllocationScopeV2::Event => 7_000,
                                        QualificationPerformanceAllocationScopeV2::CompleteProfile => {
                                            8_500
                                        }
                                    };
                                    (candidate, candidate, 10_000)
                                }
                            };
                            allocations.push(QualificationProspectiveAllocationEvidenceV1 {
                                scope,
                                state,
                                candidate_logical_bytes: logical,
                                candidate_allocated_bytes: candidate,
                                baseline_allocated_bytes: baseline,
                            });
                        }
                    }
                    let maintenance = workload.savings_required().then(|| {
                        QualificationProspectiveMaintenanceEvidenceV1 {
                            required: true,
                            foreground_samples_nanos: vec![100_000_000; 30],
                            total_nanos: 1_000_000_000,
                            not_applicable_mechanism_proof_sha256: None,
                        }
                    });
                    runs.push(QualificationProspectiveRunEvidenceV1 {
                        platform,
                        workload,
                        run_index,
                        manifest_sha256: workload_requirement.manifest_sha256.clone(),
                        generator_spec_sha256: workload_requirement.generator_spec_sha256.clone(),
                        operation_schedule_sha256: workload_requirement
                            .operation_schedule_sha256
                            .clone(),
                        baseline_evidence_sha256: Some(baseline_evidence_sha256.clone()),
                        allocation_api: platform_requirement.allocation_api.clone(),
                        controls: QualificationProspectiveEvidenceControlsV1 {
                            fresh_root: true,
                            native_execution: true,
                            quiesced_host: true,
                            semantic_receipt_verified: true,
                            durable_acknowledgement_verified: true,
                            fresh_process_visibility_verified: true,
                            carrier_inventory_complete: true,
                        },
                        semantic_receipt_sha256:
                            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                                .to_owned(),
                        timing,
                        allocations,
                        maintenance,
                    });
                }
            }
        }
        let mut evidence = Self {
            schema: QUALIFICATION_PROSPECTIVE_EVIDENCE_SCHEMA_V1.to_owned(),
            contract_schema: contract.schema.clone(),
            contract_sha256: contract.canonical_sha256().expect("fixture contract hash"),
            approved_proposal_sha256: contract.approved_proposal_sha256.clone(),
            source_commit: contract.derivation.pointbreak_commit.clone(),
            source_tree: contract.derivation.pointbreak_tree.clone(),
            cargo_lock_sha256: contract.derivation.cargo_lock_sha256.clone(),
            generator_schema: contract.derivation.generator_schema.clone(),
            public_seed_hex: contract.derivation.public_seed_hex.clone(),
            profile_id: "prospective-fixture".to_owned(),
            runs,
            evidence_sha256: String::new(),
        };
        evidence.evidence_sha256 = evidence.canonical_sha256().expect("fixture evidence hash");
        evidence
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPerformanceRunControlsV2 {
    pub fresh_roots: bool,
    pub quiesced_host: bool,
    pub native_execution: bool,
    pub equivalent_decoded_bytes: bool,
    pub monotonic_append_state: bool,
    pub durable_acknowledgement: bool,
    pub semantic_validation: bool,
    pub open_recovery_fresh_process: bool,
}

impl QualificationPerformanceRunControlsV2 {
    fn all_satisfied(&self) -> bool {
        self.fresh_roots
            && self.quiesced_host
            && self.native_execution
            && self.equivalent_decoded_bytes
            && self.monotonic_append_state
            && self.durable_acknowledgement
            && self.semantic_validation
            && self.open_recovery_fresh_process
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPerformanceAllocationSnapshotV2 {
    pub role: QualificationPerformanceRoleV1,
    pub scope: QualificationPerformanceAllocationScopeV2,
    pub state: QualificationPerformanceInventoryStateV1,
    pub inventory: QualificationPerformanceInventoryV2,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPerformanceInventoryV2 {
    pub carrier_count: u64,
    pub carrier_set_sha256: String,
    pub logical_bytes: u64,
    pub encoded_bytes: u64,
    pub allocated_bytes: u64,
    pub high_water_bytes: u64,
}

impl QualificationPerformanceInventoryV2 {
    pub fn from_inventory(inventory: &QualificationInventoryV1) -> Result<Self, String> {
        if inventory.carriers.is_empty()
            || inventory.carriers.iter().any(|carrier| carrier.is_empty())
            || inventory
                .carriers
                .windows(2)
                .any(|carriers| carriers[0].as_bytes() >= carriers[1].as_bytes())
            || inventory.logical_bytes == 0
            || inventory.encoded_bytes == 0
            || inventory.allocated_bytes == 0
            || inventory.high_water_bytes < inventory.allocated_bytes
        {
            return Err("performance inventory is incomplete".to_owned());
        }
        let carrier_set =
            serde_json::to_value(&inventory.carriers).map_err(|error| error.to_string())?;
        let carrier_set = canonical_json_bytes(&carrier_set).map_err(|error| error.to_string())?;
        Ok(Self {
            carrier_count: inventory.carriers.len() as u64,
            carrier_set_sha256: sha256_bytes_hex(&carrier_set),
            logical_bytes: inventory.logical_bytes,
            encoded_bytes: inventory.encoded_bytes,
            allocated_bytes: inventory.allocated_bytes,
            high_water_bytes: inventory.high_water_bytes,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPerformanceRunV2 {
    pub run_index: u32,
    pub workload: QualificationPerformanceWorkloadV2,
    pub workload_manifest_sha256: String,
    pub candidate: QualificationPerformanceRoleV1,
    pub candidate_build_id: String,
    pub physical_profile_id: String,
    pub environment: QualificationPlatformEnvironmentV1,
    pub operating_system_version: String,
    pub cpu: String,
    pub target_architecture: String,
    pub run_identity: String,
    pub warmup_samples: u32,
    pub measured_samples: u32,
    pub pair_order: QualificationPerformancePairOrderV1,
    pub confidence_method: QualificationPerformanceConfidenceMethodV2,
    pub outlier_policy: QualificationPerformanceOutlierPolicyV2,
    pub cache_policy: QualificationPerformanceCachePolicyV2,
    pub controls: QualificationPerformanceRunControlsV2,
    pub semantic_receipt_sha256: String,
    pub samples: Vec<QualificationPerformanceDiagnosticSampleV1>,
    pub allocations: Vec<QualificationPerformanceAllocationSnapshotV2>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPerformanceEvidenceV2 {
    pub schema: String,
    pub contract_schema: String,
    pub contract_sha256: String,
    pub source_commit: String,
    pub cargo_lock_sha256: String,
    pub runs: Vec<QualificationPerformanceRunV2>,
    pub evidence_sha256: String,
}

impl QualificationPerformanceEvidenceV2 {
    pub fn canonical_sha256(&self) -> Result<String, String> {
        let mut preimage = self.clone();
        preimage.evidence_sha256.clear();
        let value = serde_json::to_value(preimage).map_err(|error| error.to_string())?;
        canonical_json_bytes(&value)
            .map(|bytes| sha256_bytes_hex(&bytes))
            .map_err(|error| error.to_string())
    }

    pub fn validate(&self) -> Result<(), String> {
        let contract = QualificationPerformanceContractV2::frozen();
        contract.validate()?;
        if self.schema != QUALIFICATION_PERFORMANCE_EVIDENCE_SCHEMA_V2
            || self.contract_schema != contract.schema
            || self.contract_sha256 != contract.canonical_sha256()?
        {
            return Err("performance evidence uses a different contract".to_owned());
        }
        if self.source_commit != qualification_source_commit()? {
            return Err("performance evidence source commit is stale".to_owned());
        }
        if self.cargo_lock_sha256 != qualification_cargo_lock_sha256() {
            return Err("performance evidence Cargo.lock hash is stale".to_owned());
        }
        if self.evidence_sha256 != self.canonical_sha256()? {
            return Err("performance evidence hash does not match its preimage".to_owned());
        }
        let mut run_keys = BTreeSet::new();
        let mut run_identities = BTreeSet::new();
        for run in &self.runs {
            validate_performance_run_v2(run, &contract, &self.cargo_lock_sha256)?;
            let key = (
                run.candidate,
                run.workload,
                run.environment.operating_system.as_str(),
                run.environment.filesystem.as_str(),
                run.run_index,
            );
            if !run_keys.insert(key) {
                return Err("performance evidence contains a duplicate run".to_owned());
            }
            if !run_identities.insert(run.run_identity.as_str()) {
                return Err("performance evidence contains a duplicate run identity".to_owned());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationPerformanceCriterionKindV2 {
    Protocol,
    Timing,
    Allocation,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationPerformanceCriterionStatusV2 {
    Passed,
    Failed,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPerformanceTimingSummaryV2 {
    pub p50_ratio_millionths: u64,
    pub p95_ratio_millionths: u64,
    pub minimum_ratio_millionths: u64,
    pub maximum_ratio_millionths: u64,
    pub population_standard_deviation_millionths: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPerformanceCriterionV2 {
    pub kind: QualificationPerformanceCriterionKindV2,
    pub workload: QualificationPerformanceWorkloadV2,
    pub platform: QualificationPerformancePlatformRequirementV2,
    pub run_index: u32,
    pub operation: Option<QualificationPerformanceOperationV1>,
    pub allocation_scope: Option<QualificationPerformanceAllocationScopeV2>,
    pub inventory_state: Option<QualificationPerformanceInventoryStateV1>,
    pub status: QualificationPerformanceCriterionStatusV2,
    pub timing: Option<QualificationPerformanceTimingSummaryV2>,
    pub candidate_bytes: Option<u64>,
    pub baseline_bytes: Option<u64>,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPerformanceCandidateEvaluationV2 {
    pub candidate: QualificationPerformanceRoleV1,
    pub status: QualificationPerformanceCriterionStatusV2,
    pub criteria: Vec<QualificationPerformanceCriterionV2>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPerformanceEvaluationV2 {
    pub schema: String,
    pub contract_sha256: String,
    pub source_commit: String,
    pub cargo_lock_sha256: String,
    pub candidates: Vec<QualificationPerformanceCandidateEvaluationV2>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPerformancePackageV2 {
    pub schema: String,
    pub evidence: QualificationPerformanceEvidenceV2,
    pub evaluation: QualificationPerformanceEvaluationV2,
    pub package_sha256: String,
}

impl QualificationPerformancePackageV2 {
    pub fn assemble(shards: &[QualificationPerformanceEvidenceV2]) -> Result<Self, String> {
        let first = shards
            .first()
            .ok_or_else(|| "performance package has no evidence shards".to_owned())?;
        first.validate()?;
        let mut evidence = QualificationPerformanceEvidenceV2 {
            schema: first.schema.clone(),
            contract_schema: first.contract_schema.clone(),
            contract_sha256: first.contract_sha256.clone(),
            source_commit: first.source_commit.clone(),
            cargo_lock_sha256: first.cargo_lock_sha256.clone(),
            runs: Vec::new(),
            evidence_sha256: String::new(),
        };
        for shard in shards {
            shard.validate()?;
            if shard.schema != evidence.schema
                || shard.contract_schema != evidence.contract_schema
                || shard.contract_sha256 != evidence.contract_sha256
                || shard.source_commit != evidence.source_commit
                || shard.cargo_lock_sha256 != evidence.cargo_lock_sha256
            {
                return Err("performance evidence shards use different identities".to_owned());
            }
            evidence.runs.extend(shard.runs.iter().cloned());
        }
        evidence.evidence_sha256 = evidence.canonical_sha256()?;
        evidence.validate()?;
        let evaluation = evaluate_qualification_performance_v2(&evidence)?;
        if evaluation.candidates.iter().any(|candidate| {
            candidate.criteria.iter().any(|criterion| {
                criterion.status == QualificationPerformanceCriterionStatusV2::Unknown
            })
        }) {
            return Err("performance package is incomplete".to_owned());
        }
        let mut package = Self {
            schema: QUALIFICATION_PERFORMANCE_PACKAGE_SCHEMA_V2.to_owned(),
            evidence,
            evaluation,
            package_sha256: String::new(),
        };
        package.package_sha256 = package.canonical_sha256()?;
        package.validate()?;
        Ok(package)
    }

    pub fn canonical_sha256(&self) -> Result<String, String> {
        let mut preimage = self.clone();
        preimage.package_sha256.clear();
        let value = serde_json::to_value(preimage).map_err(|error| error.to_string())?;
        canonical_json_bytes(&value)
            .map(|bytes| sha256_bytes_hex(&bytes))
            .map_err(|error| error.to_string())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != QUALIFICATION_PERFORMANCE_PACKAGE_SCHEMA_V2 {
            return Err("performance package uses an unsupported schema".to_owned());
        }
        self.evidence.validate()?;
        let expected = evaluate_qualification_performance_v2(&self.evidence)?;
        if self.evaluation != expected {
            return Err("performance package evaluation does not match its evidence".to_owned());
        }
        if self.evaluation.candidates.iter().any(|candidate| {
            candidate.criteria.iter().any(|criterion| {
                criterion.status == QualificationPerformanceCriterionStatusV2::Unknown
            })
        }) {
            return Err("performance package is incomplete".to_owned());
        }
        if self.package_sha256 != self.canonical_sha256()? {
            return Err("performance package hash does not match its preimage".to_owned());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct QualificationPerformanceAppendRecordV2 {
    logical_key: String,
    decoded_bytes: Vec<u8>,
}

fn generate_qualification_append_record(
    series: &str,
    iteration: u32,
    decoded_len: usize,
) -> Result<QualificationPerformanceAppendRecordV2, String> {
    if series.is_empty()
        || !series
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err("performance append series is invalid".to_owned());
    }
    let prefix = format!(r#"{{"i":"{series}-{iteration:08}","p":""#);
    let suffix = b"\"}";
    let minimum = prefix.len().saturating_add(suffix.len());
    if decoded_len < minimum {
        return Err("performance append payload size is too small".to_owned());
    }
    let mut decoded_bytes = prefix.into_bytes();
    decoded_bytes.extend(std::iter::repeat_n(b'x', decoded_len - minimum));
    decoded_bytes.extend_from_slice(suffix);
    serde_json::from_slice::<serde_json::Value>(&decoded_bytes)
        .map_err(|_| "performance append payload is invalid JSON".to_owned())?;
    let digest = sha256_bytes_hex(&decoded_bytes);
    Ok(QualificationPerformanceAppendRecordV2 {
        logical_key: format!("qualification/append/{digest}"),
        decoded_bytes,
    })
}

fn scoped_native_inventory(
    root: &Path,
    scope: QualificationPerformanceAllocationScopeV2,
    logical_bytes: u64,
    high_water_bytes: u64,
) -> Result<QualificationPerformanceInventoryV2, String> {
    fn visit(
        root: &Path,
        directory: &Path,
        scope: QualificationPerformanceAllocationScopeV2,
        paths: &mut Vec<PathBuf>,
    ) -> Result<(), String> {
        let mut entries = fs::read_dir(directory)
            .map_err(|error| format!("{}: {error}", directory.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let relative = path.strip_prefix(root).map_err(|error| error.to_string())?;
            if scope == QualificationPerformanceAllocationScopeV2::Event
                && relative
                    .components()
                    .next()
                    .is_some_and(|component| component.as_os_str() == "content")
            {
                continue;
            }
            let file_type = entry
                .file_type()
                .map_err(|error| format!("{}: {error}", path.display()))?;
            if file_type.is_symlink() {
                return Err("performance inventory rejects symbolic-link carriers".to_owned());
            }
            if file_type.is_dir() {
                visit(root, &path, scope, paths)?;
            } else if file_type.is_file() {
                paths.push(path);
            } else {
                return Err("performance inventory rejects non-file carriers".to_owned());
            }
        }
        Ok(())
    }

    let mut paths = Vec::new();
    visit(root, root, scope, &mut paths)?;
    let mut carriers = Vec::with_capacity(paths.len());
    let mut encoded_bytes = 0_u64;
    let mut allocated_bytes = 0_u64;
    for path in paths {
        let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
        encoded_bytes = encoded_bytes
            .checked_add(metadata.len())
            .ok_or_else(|| "performance encoded-byte inventory overflow".to_owned())?;
        allocated_bytes = allocated_bytes
            .checked_add(super::fault::native_file_allocation(&path, &metadata)?)
            .ok_or_else(|| "performance allocated-byte inventory overflow".to_owned())?;
        carriers.push(
            path.strip_prefix(root)
                .map_err(|error| error.to_string())?
                .to_string_lossy()
                .replace('\\', "/"),
        );
    }
    carriers.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    QualificationPerformanceInventoryV2::from_inventory(&QualificationInventoryV1 {
        carriers,
        logical_bytes,
        encoded_bytes,
        allocated_bytes,
        high_water_bytes: high_water_bytes.max(allocated_bytes),
    })
}

const QUALIFICATION_LOOSE_BASELINE_OPEN_REQUEST_SCHEMA_V1: &str =
    "pointbreak.qualification-loose-baseline-open-request.v1";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QualificationLooseBaselineOpenRequestV1 {
    schema: String,
    root: PathBuf,
    result_path: PathBuf,
}

#[derive(Clone, Debug)]
struct LooseBaselineEventAggregateV1 {
    record_count: u64,
    logical_byte_count: u64,
    receipt_sha256: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LooseBaselineReceiptPreimageV1<'a> {
    kind: QualificationLooseBaselineReceiptKindV1,
    record_count: u64,
    logical_byte_count: u64,
    witness_sha256: &'a str,
}

type LooseBaselineInventoryMapV1 =
    BTreeMap<QualificationPerformanceAllocationScopeV2, QualificationLooseBaselineInventoryV1>;

pub fn run_qualification_loose_baseline_evidence_v1(
    configuration: &QualificationLooseBaselineEvidenceConfigurationV1,
) -> Result<QualificationLooseBaselineEvidenceV1, String> {
    validate_loose_baseline_evidence_configuration_v1(configuration)?;
    fs::create_dir(&configuration.root)
        .map_err(|_| "loose baseline evidence root creation failed".to_owned())?;
    let platform = loose_baseline_platform_v1(&configuration.root);
    validate_loose_baseline_platform_v1(&platform)?;

    let mut runs = Vec::new();
    for workload in QualificationGeneratedWorkloadV1::ALL {
        let (manifest, schedule) = generated_loose_baseline_inputs_v1(workload)?;
        for run_index in 1..=QUALIFICATION_LOOSE_BASELINE_INDEPENDENT_ROOTS_V1 {
            let case_root = configuration.root.join(format!(
                "{}-independent-{run_index}",
                loose_baseline_workload_label_v1(workload),
            ));
            fs::create_dir(&case_root)
                .map_err(|_| "loose baseline case root creation failed".to_owned())?;
            runs.push(run_loose_baseline_case_v1(
                &configuration.executable,
                workload,
                &manifest,
                &schedule,
                run_index,
                &case_root,
                &platform,
            )?);
        }
    }

    let mut evidence = QualificationLooseBaselineEvidenceV1 {
        schema: QUALIFICATION_LOOSE_BASELINE_EVIDENCE_SCHEMA_V1.to_owned(),
        source_commit: configuration.source_commit.clone(),
        cargo_lock_sha256: configuration.cargo_lock_sha256.clone(),
        generator_schema: QUALIFICATION_GENERATOR_SCHEMA_V1.to_owned(),
        public_seed_hex: QUALIFICATION_PUBLIC_SEED_HEX_V1.to_owned(),
        platform,
        runs,
        evidence_sha256: String::new(),
    };
    evidence.evidence_sha256 = evidence.canonical_sha256()?;
    evidence.validate()?;
    Ok(evidence)
}

pub fn run_qualification_loose_baseline_smoke_v1(
    configuration: &QualificationLooseBaselineSmokeConfigurationV1,
) -> Result<QualificationLooseBaselineSmokeV1, String> {
    validate_loose_baseline_smoke_configuration_v1(configuration)?;
    fs::create_dir(&configuration.root)
        .map_err(|_| "loose baseline smoke root creation failed".to_owned())?;
    let workload = QualificationGeneratedWorkloadV1::G0;
    let (manifest, schedule) = generated_loose_baseline_inputs_v1(workload)?;
    let measurement_root = configuration.root.join("measurement");
    let probe = LooseQualificationPerformanceProbe::create(measurement_root.clone(), &manifest)?;
    let samples = execute_loose_baseline_iteration_v1(
        &configuration.executable,
        &probe,
        workload,
        &manifest,
        &schedule,
        0,
        &configuration.root,
        "smoke",
    )?;
    let (event_logical_bytes, complete_logical_bytes) =
        loose_baseline_logical_bytes_v1(&manifest, &schedule, 1)?;
    let mut high_water = BTreeMap::new();
    let current = capture_loose_baseline_inventories_v1(
        &measurement_root,
        event_logical_bytes,
        complete_logical_bytes,
        &mut high_water,
    )?;
    let allocations = loose_baseline_allocation_snapshots_v1(&current, &current, &high_water)?;
    let (generator_spec_sha256, manifest_sha256, schedule_sha256) =
        frozen_generated_identities_v1(workload);
    let report = QualificationLooseBaselineSmokeV1 {
        schema: QUALIFICATION_LOOSE_BASELINE_SMOKE_SCHEMA_V1.to_owned(),
        mode: "non_timing_validation".to_owned(),
        generator_schema: QUALIFICATION_GENERATOR_SCHEMA_V1.to_owned(),
        public_seed_hex: QUALIFICATION_PUBLIC_SEED_HEX_V1.to_owned(),
        workload,
        generator_spec_sha256: generator_spec_sha256.to_owned(),
        manifest_sha256: manifest_sha256.to_owned(),
        schedule_sha256: schedule_sha256.to_owned(),
        receipts: samples
            .into_iter()
            .map(|sample| QualificationLooseBaselineSmokeReceiptV1 {
                operation: sample.operation,
                read_class: sample.read_class,
                receipt: sample.receipt,
            })
            .collect(),
        allocations,
        proposal_shape: QualificationProspectiveContractProposalShapeV1::complete(),
    };
    report.validate()?;
    Ok(report)
}

pub fn run_qualification_loose_baseline_open_child_v1(request_path: &Path) -> Result<(), String> {
    let bytes = fs::read(request_path)
        .map_err(|_| "loose baseline open request could not be read".to_owned())?;
    let request: QualificationLooseBaselineOpenRequestV1 = serde_json::from_slice(&bytes)
        .map_err(|_| "loose baseline open request is invalid".to_owned())?;
    if request.schema != QUALIFICATION_LOOSE_BASELINE_OPEN_REQUEST_SCHEMA_V1
        || !request.root.is_dir()
        || request.result_path.exists()
        || request
            .result_path
            .parent()
            .is_none_or(|parent| !parent.is_dir())
    {
        return Err("loose baseline open request is invalid".to_owned());
    }
    let aggregate = loose_baseline_event_aggregate_v1(&request.root)?;
    let receipt = loose_baseline_receipt_v1(
        QualificationLooseBaselineReceiptKindV1::FreshProcessOpenExact,
        aggregate.record_count,
        aggregate.logical_byte_count,
        &aggregate.receipt_sha256,
    )?;
    write_json_new_synced(&request.result_path, &receipt)
}

fn validate_loose_baseline_evidence_configuration_v1(
    configuration: &QualificationLooseBaselineEvidenceConfigurationV1,
) -> Result<(), String> {
    if std::env::var_os("POINTBREAK_QUALIFICATION_CORPUS").is_some()
        || !configuration.executable.is_file()
        || configuration.root.exists()
        || configuration
            .root
            .parent()
            .is_none_or(|parent| !parent.is_dir())
        || configuration.source_commit != qualification_source_commit()?
        || configuration.cargo_lock_sha256 != qualification_cargo_lock_sha256()
        || !configuration.quiesced_host
        || env!("POINTBREAK_BUILD_DIRTY") == "true"
    {
        return Err("loose baseline evidence configuration is not proof-eligible".to_owned());
    }
    let parent = configuration
        .root
        .parent()
        .ok_or_else(|| "loose baseline evidence root has no parent".to_owned())?;
    let filesystem = qualification_filesystem_name(parent);
    let supported = matches!(
        (std::env::consts::OS, filesystem.as_str()),
        ("macos", "apfs") | ("linux", "ext4") | ("windows", "ntfs")
    );
    if !supported
        || classify_qualification_filesystem(&filesystem)
            != QualificationFilesystemDispositionV1::LocalProofEligible
    {
        return Err("loose baseline evidence requires a supported native filesystem".to_owned());
    }
    Ok(())
}

fn validate_loose_baseline_smoke_configuration_v1(
    configuration: &QualificationLooseBaselineSmokeConfigurationV1,
) -> Result<(), String> {
    if std::env::var_os("POINTBREAK_QUALIFICATION_CORPUS").is_some()
        || !configuration.executable.is_file()
        || configuration.root.exists()
        || configuration
            .root
            .parent()
            .is_none_or(|parent| !parent.is_dir())
    {
        return Err("loose baseline smoke configuration is invalid".to_owned());
    }
    Ok(())
}

fn loose_baseline_platform_v1(root: &Path) -> QualificationLooseBaselinePlatformV1 {
    QualificationLooseBaselinePlatformV1 {
        operating_system: std::env::consts::OS.to_owned(),
        operating_system_version: native_operating_system_version(),
        architecture: std::env::consts::ARCH.to_owned(),
        cpu: native_cpu_description(),
        filesystem: qualification_filesystem_name(root),
        allocation_api: final_native_allocation_method().to_owned(),
        rustc: rustc_verbose_version(),
        build_source: env!("POINTBREAK_BUILD_SOURCE").to_owned(),
        build_describe: env!("POINTBREAK_BUILD_DESCRIBE").to_owned(),
        source_tree_clean: env!("POINTBREAK_BUILD_DIRTY") != "true",
    }
}

fn generated_loose_baseline_inputs_v1(
    workload: QualificationGeneratedWorkloadV1,
) -> Result<
    (
        QualificationCorpusManifestV1,
        super::QualificationOperationScheduleV1,
    ),
    String,
> {
    let spec = qualification_generator_spec_v1(workload);
    let manifest = qualification_generated_manifest_v1(&spec).map_err(|error| error.to_string())?;
    let schedule = qualification_operation_schedule_v1(&spec).map_err(|error| error.to_string())?;
    let (spec_sha256, manifest_sha256, schedule_sha256) = frozen_generated_identities_v1(workload);
    let actual_spec_sha256 = {
        let value = serde_json::to_value(&spec).map_err(|error| error.to_string())?;
        let bytes = canonical_json_bytes(&value).map_err(|error| error.to_string())?;
        sha256_bytes_hex(&bytes)
    };
    if actual_spec_sha256 != spec_sha256
        || manifest.manifest_sha256 != manifest_sha256
        || schedule.schedule_sha256 != schedule_sha256
    {
        return Err("loose baseline generated workload identity has drifted".to_owned());
    }
    Ok((manifest, schedule))
}

#[allow(clippy::too_many_arguments)]
fn run_loose_baseline_case_v1(
    executable: &Path,
    workload: QualificationGeneratedWorkloadV1,
    manifest: &QualificationCorpusManifestV1,
    schedule: &super::QualificationOperationScheduleV1,
    run_index: u32,
    case_root: &Path,
    platform: &QualificationLooseBaselinePlatformV1,
) -> Result<QualificationLooseBaselineRunV1, String> {
    let warmup_root = case_root.join("warmup");
    let warmup = LooseQualificationPerformanceProbe::create(warmup_root.clone(), manifest)?;
    for iteration in 0..QUALIFICATION_LOOSE_BASELINE_WARMUP_ITERATIONS_V1 {
        execute_loose_baseline_iteration_v1(
            executable, &warmup, workload, manifest, schedule, iteration, case_root, "warmup",
        )?;
    }
    drop(warmup);
    fs::remove_dir_all(&warmup_root)
        .map_err(|_| "loose baseline warm-up root cleanup failed".to_owned())?;

    let measurement_root = case_root.join("measurement");
    let probe = LooseQualificationPerformanceProbe::create(measurement_root.clone(), manifest)?;
    let (event_logical_base, complete_logical_base) =
        loose_baseline_logical_bytes_v1(manifest, schedule, 0)?;
    let mut high_water = BTreeMap::new();
    capture_loose_baseline_inventories_v1(
        &measurement_root,
        event_logical_base,
        complete_logical_base,
        &mut high_water,
    )?;
    let mut samples = Vec::new();
    for iteration in 0..QUALIFICATION_LOOSE_BASELINE_MEASURED_ITERATIONS_V1 {
        samples.extend(execute_loose_baseline_iteration_v1(
            executable, &probe, workload, manifest, schedule, iteration, case_root, "measured",
        )?);
        let (event_logical_bytes, complete_logical_bytes) =
            loose_baseline_logical_bytes_v1(manifest, schedule, iteration + 1)?;
        capture_loose_baseline_inventories_v1(
            &measurement_root,
            event_logical_bytes,
            complete_logical_bytes,
            &mut high_water,
        )?;
    }
    let (event_logical_bytes, complete_logical_bytes) = loose_baseline_logical_bytes_v1(
        manifest,
        schedule,
        QUALIFICATION_LOOSE_BASELINE_MEASURED_ITERATIONS_V1,
    )?;
    let steady = capture_loose_baseline_inventories_v1(
        &measurement_root,
        event_logical_bytes,
        complete_logical_bytes,
        &mut high_water,
    )?;
    drop(probe);
    spawn_loose_baseline_open_receipt_v1(executable, &measurement_root, case_root, "final-reopen")?;
    let reopened = capture_loose_baseline_inventories_v1(
        &measurement_root,
        event_logical_bytes,
        complete_logical_bytes,
        &mut high_water,
    )?;
    let allocations = loose_baseline_allocation_snapshots_v1(&steady, &reopened, &high_water)?;
    let (generator_spec_sha256, manifest_sha256, schedule_sha256) =
        frozen_generated_identities_v1(workload);
    Ok(QualificationLooseBaselineRunV1 {
        run_index,
        run_identity: format!(
            "{}-{}-{}-independent-{run_index}",
            platform.operating_system,
            platform.architecture,
            loose_baseline_workload_label_v1(workload),
        ),
        workload,
        measurement_scope: loose_baseline_measurement_scope_v1(workload),
        generator_spec_sha256: generator_spec_sha256.to_owned(),
        manifest_sha256: manifest_sha256.to_owned(),
        schedule_sha256: schedule_sha256.to_owned(),
        controls: QualificationLooseBaselineRunControlsV1::fixed(),
        samples,
        allocations,
    })
}

#[allow(clippy::too_many_arguments)]
fn execute_loose_baseline_iteration_v1(
    executable: &Path,
    probe: &LooseQualificationPerformanceProbe,
    workload: QualificationGeneratedWorkloadV1,
    manifest: &QualificationCorpusManifestV1,
    schedule: &super::QualificationOperationScheduleV1,
    iteration: u32,
    control_root: &Path,
    series: &str,
) -> Result<Vec<QualificationLooseBaselineSampleV1>, String> {
    let append_index = *schedule
        .append_record_indices
        .get(iteration as usize)
        .ok_or_else(|| "loose baseline append schedule is incomplete".to_owned())?;
    let append_record = manifest
        .records
        .get(append_index as usize)
        .ok_or_else(|| "loose baseline append record is missing".to_owned())?;
    let append_key = format!(
        "qualification/loose-append/{}/{series}-{iteration:08}",
        loose_baseline_workload_label_v1(workload),
    );
    let append_path = baseline_record_path(
        &probe.root,
        &append_key,
        QualificationRecordKindV1::LegacyEvent,
    );
    let started = Instant::now();
    write_new_synced(&append_path, &append_record.decoded_bytes)?;
    probe.record_legacy_append(&append_record.decoded_bytes);
    let stored = fs::read(&append_path)
        .map_err(|_| "loose baseline durable append readback failed".to_owned())?;
    if stored != append_record.decoded_bytes {
        return Err("loose baseline durable append readback differed".to_owned());
    }
    let append_elapsed = elapsed_nanos(started);
    let aggregate = loose_baseline_event_aggregate_v1(&probe.root)?;
    let mut samples = vec![QualificationLooseBaselineSampleV1 {
        operation: QualificationLooseBaselineOperationV1::DurableAppend,
        read_class: None,
        iteration,
        elapsed_nanos: append_elapsed,
        receipt: loose_baseline_receipt_v1(
            QualificationLooseBaselineReceiptKindV1::DurableAppendVisibleExact,
            aggregate.record_count,
            aggregate.logical_byte_count,
            &aggregate.receipt_sha256,
        )?,
    }];

    let started = Instant::now();
    read_all_files(&probe.root.join("events"))
        .map_err(|_| "loose baseline strict replay failed".to_owned())?;
    let replay_elapsed = elapsed_nanos(started);
    let replay_aggregate = loose_baseline_event_aggregate_v1(&probe.root)?;
    samples.push(QualificationLooseBaselineSampleV1 {
        operation: QualificationLooseBaselineOperationV1::StrictReplay,
        read_class: None,
        iteration,
        elapsed_nanos: replay_elapsed,
        receipt: loose_baseline_receipt_v1(
            QualificationLooseBaselineReceiptKindV1::StrictReplayExact,
            replay_aggregate.record_count,
            replay_aggregate.logical_byte_count,
            &replay_aggregate.receipt_sha256,
        )?,
    });

    let (open_elapsed, open_receipt) = spawn_loose_baseline_open_receipt_v1(
        executable,
        &probe.root,
        control_root,
        &format!("{series}-{iteration}"),
    )?;
    samples.push(QualificationLooseBaselineSampleV1 {
        operation: QualificationLooseBaselineOperationV1::FreshProcessOpenRecovery,
        read_class: None,
        iteration,
        elapsed_nanos: open_elapsed,
        receipt: open_receipt,
    });

    for scheduled_read in &schedule.keyed_reads {
        let started = Instant::now();
        let (kind, record_count, logical_byte_count, witness_sha256) =
            if scheduled_read.class == QualificationKeyedReadClassV1::Absent {
                let event_path = baseline_record_path(
                    &probe.root,
                    &scheduled_read.logical_key,
                    QualificationRecordKindV1::LegacyEvent,
                );
                let content_path = baseline_record_path(
                    &probe.root,
                    &scheduled_read.logical_key,
                    QualificationRecordKindV1::ObjectArtifact,
                );
                if event_path.exists() || content_path.exists() {
                    return Err("loose baseline absent keyed read found a record".to_owned());
                }
                (
                    QualificationLooseBaselineReceiptKindV1::KeyedReadAbsentExact,
                    0,
                    0,
                    sha256_bytes_hex(b"loose-baseline-keyed-read-absent-v1"),
                )
            } else {
                let expected = manifest
                    .records
                    .iter()
                    .find(|record| record.logical_key == scheduled_read.logical_key)
                    .ok_or_else(|| "loose baseline keyed read target is missing".to_owned())?;
                let path = baseline_record_path(
                    &probe.root,
                    &scheduled_read.logical_key,
                    expected.record_kind,
                );
                let bytes =
                    fs::read(path).map_err(|_| "loose baseline keyed read failed".to_owned())?;
                if bytes != expected.decoded_bytes {
                    return Err("loose baseline keyed read returned different bytes".to_owned());
                }
                std::hint::black_box(Sha256::digest(&bytes));
                (
                    QualificationLooseBaselineReceiptKindV1::KeyedReadPresentExact,
                    1,
                    bytes.len() as u64,
                    sha256_bytes_hex(&bytes),
                )
            };
        samples.push(QualificationLooseBaselineSampleV1 {
            operation: QualificationLooseBaselineOperationV1::KeyedRead,
            read_class: Some(scheduled_read.class),
            iteration,
            elapsed_nanos: elapsed_nanos(started),
            receipt: loose_baseline_receipt_v1(
                kind,
                record_count,
                logical_byte_count,
                &witness_sha256,
            )?,
        });
    }
    Ok(samples)
}

fn spawn_loose_baseline_open_receipt_v1(
    executable: &Path,
    root: &Path,
    control_root: &Path,
    label: &str,
) -> Result<(u64, QualificationLooseBaselineSemanticReceiptV1), String> {
    let control = control_root.join("loose-open-controls");
    fs::create_dir_all(&control)
        .map_err(|_| "loose baseline open control creation failed".to_owned())?;
    let request_path = control.join(format!("{label}-request.json"));
    let result_path = control.join(format!("{label}-result.json"));
    let request = QualificationLooseBaselineOpenRequestV1 {
        schema: QUALIFICATION_LOOSE_BASELINE_OPEN_REQUEST_SCHEMA_V1.to_owned(),
        root: root.to_path_buf(),
        result_path: result_path.clone(),
    };
    write_json_new_synced(&request_path, &request)?;
    let started = Instant::now();
    let output = Command::new(executable)
        .arg("--loose-baseline-open-child")
        .arg(&request_path)
        .output()
        .map_err(|_| "loose baseline open child could not start".to_owned())?;
    let elapsed = elapsed_nanos(started);
    if !output.status.success() {
        return Err("loose baseline open child failed".to_owned());
    }
    let bytes = fs::read(&result_path)
        .map_err(|_| "loose baseline open result could not be read".to_owned())?;
    let receipt: QualificationLooseBaselineSemanticReceiptV1 = serde_json::from_slice(&bytes)
        .map_err(|_| "loose baseline open result is invalid".to_owned())?;
    validate_loose_baseline_receipt_v1(
        QualificationLooseBaselineOperationV1::FreshProcessOpenRecovery,
        None,
        &receipt,
    )?;
    fs::remove_file(&request_path)
        .and_then(|_| fs::remove_file(&result_path))
        .map_err(|_| "loose baseline open control cleanup failed".to_owned())?;
    Ok((elapsed, receipt))
}

fn loose_baseline_event_aggregate_v1(root: &Path) -> Result<LooseBaselineEventAggregateV1, String> {
    let events = root.join("events");
    let mut records = Vec::new();
    let mut logical_byte_count = 0_u64;
    let entries = fs::read_dir(&events)
        .map_err(|_| "loose baseline event directory could not be read".to_owned())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "loose baseline event directory is invalid".to_owned())?;
    for entry in entries {
        let path = entry.path();
        if !entry
            .file_type()
            .map_err(|_| "loose baseline event carrier type could not be read".to_owned())?
            .is_file()
        {
            return Err("loose baseline event carrier is not a file".to_owned());
        }
        let key_hash = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| "loose baseline event carrier name is invalid".to_owned())?
            .to_owned();
        validate_hex(&key_hash, 64, "loose baseline event key SHA-256")?;
        let bytes = fs::read(&path)
            .map_err(|_| "loose baseline event carrier could not be read".to_owned())?;
        logical_byte_count = logical_byte_count
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| "loose baseline receipt byte total overflow".to_owned())?;
        records.push((key_hash, sha256_bytes_hex(&bytes)));
    }
    if records.is_empty() {
        return Err("loose baseline event receipt is empty".to_owned());
    }
    records.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    let value = serde_json::to_value(&records).map_err(|error| error.to_string())?;
    let bytes = canonical_json_bytes(&value).map_err(|error| error.to_string())?;
    Ok(LooseBaselineEventAggregateV1 {
        record_count: records.len() as u64,
        logical_byte_count,
        receipt_sha256: sha256_bytes_hex(&bytes),
    })
}

fn loose_baseline_receipt_v1(
    kind: QualificationLooseBaselineReceiptKindV1,
    record_count: u64,
    logical_byte_count: u64,
    witness_sha256: &str,
) -> Result<QualificationLooseBaselineSemanticReceiptV1, String> {
    validate_hex(witness_sha256, 64, "loose baseline receipt witness SHA-256")?;
    let preimage = LooseBaselineReceiptPreimageV1 {
        kind,
        record_count,
        logical_byte_count,
        witness_sha256,
    };
    let value = serde_json::to_value(preimage).map_err(|error| error.to_string())?;
    let bytes = canonical_json_bytes(&value).map_err(|error| error.to_string())?;
    Ok(QualificationLooseBaselineSemanticReceiptV1 {
        kind,
        record_count,
        logical_byte_count,
        aggregate_receipt_sha256: sha256_bytes_hex(&bytes),
    })
}

fn loose_baseline_logical_bytes_v1(
    manifest: &QualificationCorpusManifestV1,
    schedule: &super::QualificationOperationScheduleV1,
    appended_iterations: u32,
) -> Result<(u64, u64), String> {
    let event_base = manifest
        .records
        .iter()
        .filter(|record| is_journal_record(record.record_kind))
        .try_fold(0_u64, |total, record| {
            total.checked_add(record.decoded_bytes.len() as u64)
        })
        .ok_or_else(|| "loose baseline event logical-byte total overflow".to_owned())?;
    let complete_base = manifest
        .records
        .iter()
        .try_fold(0_u64, |total, record| {
            total.checked_add(record.decoded_bytes.len() as u64)
        })
        .ok_or_else(|| "loose baseline complete logical-byte total overflow".to_owned())?;
    let appended = schedule
        .append_record_indices
        .iter()
        .take(appended_iterations as usize)
        .try_fold(0_u64, |total, index| {
            manifest
                .records
                .get(*index as usize)
                .and_then(|record| total.checked_add(record.decoded_bytes.len() as u64))
        })
        .ok_or_else(|| "loose baseline append logical-byte total is invalid".to_owned())?;
    Ok((event_base + appended, complete_base + appended))
}

fn capture_loose_baseline_inventories_v1(
    root: &Path,
    event_logical_bytes: u64,
    complete_logical_bytes: u64,
    high_water: &mut BTreeMap<QualificationPerformanceAllocationScopeV2, u64>,
) -> Result<LooseBaselineInventoryMapV1, String> {
    let mut inventories = BTreeMap::new();
    for scope in [
        QualificationPerformanceAllocationScopeV2::Event,
        QualificationPerformanceAllocationScopeV2::CompleteProfile,
    ] {
        let logical_bytes = match scope {
            QualificationPerformanceAllocationScopeV2::Event => event_logical_bytes,
            QualificationPerformanceAllocationScopeV2::CompleteProfile => complete_logical_bytes,
        };
        let mut inventory: QualificationLooseBaselineInventoryV1 =
            scoped_native_inventory(root, scope, logical_bytes, 0)?.into();
        let observed = high_water.entry(scope).or_default();
        *observed = (*observed).max(inventory.allocated_bytes);
        inventory.high_water_bytes = *observed;
        inventories.insert(scope, inventory);
    }
    Ok(inventories)
}

fn loose_baseline_allocation_snapshots_v1(
    steady: &LooseBaselineInventoryMapV1,
    reopened: &LooseBaselineInventoryMapV1,
    high_water: &BTreeMap<QualificationPerformanceAllocationScopeV2, u64>,
) -> Result<Vec<QualificationLooseBaselineAllocationSnapshotV1>, String> {
    let mut snapshots = Vec::new();
    for (state, source) in [
        (QualificationPerformanceInventoryStateV1::Steady, steady),
        (QualificationPerformanceInventoryStateV1::Reopened, reopened),
        (QualificationPerformanceInventoryStateV1::HighWater, steady),
    ] {
        for scope in [
            QualificationPerformanceAllocationScopeV2::Event,
            QualificationPerformanceAllocationScopeV2::CompleteProfile,
        ] {
            let mut inventory = source
                .get(&scope)
                .cloned()
                .ok_or_else(|| "loose baseline allocation inventory is missing".to_owned())?;
            inventory.high_water_bytes = *high_water
                .get(&scope)
                .ok_or_else(|| "loose baseline allocation high-water is missing".to_owned())?;
            snapshots.push(QualificationLooseBaselineAllocationSnapshotV1 {
                scope,
                state,
                inventory,
            });
        }
    }
    Ok(snapshots)
}

pub fn evaluate_qualification_performance_v2(
    evidence: &QualificationPerformanceEvidenceV2,
) -> Result<QualificationPerformanceEvaluationV2, String> {
    evidence.validate()?;
    let contract = QualificationPerformanceContractV2::frozen();
    let mut candidates = Vec::new();
    for candidate in &contract.candidates {
        let mut criteria = Vec::new();
        for workload in &contract.workloads {
            for platform in &workload.platforms {
                for run_index in 1..=contract.independent_runs {
                    let run = evidence.runs.iter().find(|run| {
                        run.candidate == *candidate
                            && run.workload == workload.workload
                            && run.environment.operating_system == platform.operating_system
                            && run.environment.filesystem == platform.filesystem
                            && run.run_index == run_index
                    });
                    let Some(run) = run else {
                        criteria.push(QualificationPerformanceCriterionV2 {
                            kind: QualificationPerformanceCriterionKindV2::Protocol,
                            workload: workload.workload,
                            platform: platform.clone(),
                            run_index,
                            operation: None,
                            allocation_scope: None,
                            inventory_state: None,
                            status: QualificationPerformanceCriterionStatusV2::Unknown,
                            timing: None,
                            candidate_bytes: None,
                            baseline_bytes: None,
                            message: "required performance run is missing".to_owned(),
                        });
                        continue;
                    };
                    criteria.push(QualificationPerformanceCriterionV2 {
                        kind: QualificationPerformanceCriterionKindV2::Protocol,
                        workload: workload.workload,
                        platform: platform.clone(),
                        run_index,
                        operation: None,
                        allocation_scope: None,
                        inventory_state: None,
                        status: QualificationPerformanceCriterionStatusV2::Passed,
                        timing: None,
                        candidate_bytes: None,
                        baseline_bytes: None,
                        message: "run protocol and semantic receipt are complete".to_owned(),
                    });
                    for operation in &contract.operations {
                        let pairs = timing_pairs(run, *candidate, *operation)?;
                        let summary = timing_summary(&pairs);
                        let p95 = ratio_at_rank(&pairs, 95);
                        let threshold_passed = u128::from(p95.0) * 100
                            <= u128::from(p95.1) * u128::from(contract.ceiling_percent);
                        let status = if !workload.quantitative || threshold_passed {
                            QualificationPerformanceCriterionStatusV2::Passed
                        } else {
                            QualificationPerformanceCriterionStatusV2::Failed
                        };
                        criteria.push(QualificationPerformanceCriterionV2 {
                            kind: QualificationPerformanceCriterionKindV2::Timing,
                            workload: workload.workload,
                            platform: platform.clone(),
                            run_index,
                            operation: Some(*operation),
                            allocation_scope: None,
                            inventory_state: None,
                            status,
                            timing: Some(summary),
                            candidate_bytes: None,
                            baseline_bytes: None,
                            message: if workload.quantitative {
                                format!(
                                    "paired p95 must be at or below {}%",
                                    contract.ceiling_percent
                                )
                            } else {
                                "diagnostic timing summary; no quantitative threshold".to_owned()
                            },
                        });
                    }
                    for scope in &contract.allocation_scopes {
                        for state in &contract.inventory_states {
                            let candidate_inventory =
                                allocation_inventory(run, *candidate, *scope, *state)?;
                            let baseline_inventory = allocation_inventory(
                                run,
                                QualificationPerformanceRoleV1::LooseBaseline,
                                *scope,
                                *state,
                            )?;
                            let candidate_bytes = allocation_bytes(candidate_inventory, *state);
                            let baseline_bytes = allocation_bytes(baseline_inventory, *state);
                            let status =
                                if !workload.quantitative || candidate_bytes < baseline_bytes {
                                    QualificationPerformanceCriterionStatusV2::Passed
                                } else {
                                    QualificationPerformanceCriterionStatusV2::Failed
                                };
                            criteria.push(QualificationPerformanceCriterionV2 {
                                kind: QualificationPerformanceCriterionKindV2::Allocation,
                                workload: workload.workload,
                                platform: platform.clone(),
                                run_index,
                                operation: None,
                                allocation_scope: Some(*scope),
                                inventory_state: Some(*state),
                                status,
                                timing: None,
                                candidate_bytes: Some(candidate_bytes),
                                baseline_bytes: Some(baseline_bytes),
                                message: if workload.quantitative {
                                    "candidate allocation must be strictly lower".to_owned()
                                } else {
                                    "diagnostic allocation summary; no quantitative threshold"
                                        .to_owned()
                                },
                            });
                        }
                    }
                }
            }
        }
        let status = aggregate_criterion_status(criteria.iter().map(|criterion| criterion.status));
        candidates.push(QualificationPerformanceCandidateEvaluationV2 {
            candidate: *candidate,
            status,
            criteria,
        });
    }
    Ok(QualificationPerformanceEvaluationV2 {
        schema: QUALIFICATION_PERFORMANCE_EVALUATION_SCHEMA_V2.to_owned(),
        contract_sha256: contract.canonical_sha256()?,
        source_commit: evidence.source_commit.clone(),
        cargo_lock_sha256: evidence.cargo_lock_sha256.clone(),
        candidates,
    })
}

fn validate_performance_run_v2(
    run: &QualificationPerformanceRunV2,
    contract: &QualificationPerformanceContractV2,
    cargo_lock_sha256: &str,
) -> Result<(), String> {
    let workload = contract
        .workloads
        .iter()
        .find(|requirement| requirement.workload == run.workload)
        .ok_or_else(|| "performance run uses an unsupported workload".to_owned())?;
    if run.workload_manifest_sha256 != workload.manifest_sha256 {
        return Err("performance run uses a different workload manifest".to_owned());
    }
    let platform = workload
        .platforms
        .iter()
        .find(|requirement| {
            requirement.operating_system == run.environment.operating_system
                && requirement.filesystem == run.environment.filesystem
        })
        .ok_or_else(|| "performance run uses an unsupported platform".to_owned())?;
    if run.environment.allocation_method != platform.allocation_method
        || run.environment.filesystem_disposition
            != QualificationFilesystemDispositionV1::LocalProofEligible
        || run.environment.architecture.trim().is_empty()
        || !run.environment.rustc.contains("host:")
        || run.environment.build_source.trim().is_empty()
        || run.environment.build_describe.trim().is_empty()
        || run.environment.source_tree_dirty
        || run.operating_system_version.trim().is_empty()
        || run.cpu.trim().is_empty()
        || run.target_architecture.trim().is_empty()
        || run.target_architecture != run.environment.architecture
        || run.run_identity.trim().is_empty()
    {
        return Err("performance run environment is not proof-eligible".to_owned());
    }
    if !(1..=contract.independent_runs).contains(&run.run_index) {
        return Err("performance run index is outside the contract".to_owned());
    }
    let (candidate, physical_profile_id) = match run.candidate {
        QualificationPerformanceRoleV1::SqliteWal => (
            QualificationCandidateV1::SqliteWal,
            SQLITE_QUALIFICATION_PROFILE_ID_V1,
        ),
        QualificationPerformanceRoleV1::BoundedSegments => (
            QualificationCandidateV1::BoundedSegments,
            SEGMENT_QUALIFICATION_PROFILE_ID_V1,
        ),
        QualificationPerformanceRoleV1::LooseBaseline => {
            return Err("loose baseline cannot be evaluated as a candidate".to_owned());
        }
    };
    if run.candidate_build_id != candidate.build_id(cargo_lock_sha256)
        || run.physical_profile_id != physical_profile_id
    {
        return Err("performance run candidate identity is stale".to_owned());
    }
    if run.warmup_samples != contract.warmup_samples
        || run.measured_samples != contract.measured_samples
        || run.pair_order != contract.pair_order
        || run.confidence_method != contract.confidence_method
        || run.outlier_policy != contract.outlier_policy
        || run.cache_policy != contract.cache_policy
        || !run.controls.all_satisfied()
    {
        return Err("performance run protocol is incomplete".to_owned());
    }
    validate_hex(
        &run.semantic_receipt_sha256,
        64,
        "performance semantic receipt SHA-256",
    )?;

    let expected_sample_count = contract
        .operations
        .len()
        .checked_mul(contract.measured_samples as usize)
        .and_then(|count| count.checked_mul(2))
        .ok_or_else(|| "performance sample count overflow".to_owned())?;
    if run.samples.len() != expected_sample_count {
        return Err("performance run has the wrong measured sample count".to_owned());
    }
    let mut sample_keys = BTreeSet::new();
    for sample in &run.samples {
        if !contract.operations.contains(&sample.operation)
            || sample.iteration >= contract.measured_samples
            || sample.pair_order > 1
            || ![run.candidate, QualificationPerformanceRoleV1::LooseBaseline]
                .contains(&sample.role)
            || sample.total_elapsed_nanos == 0
            || sample.stages.is_empty()
            || sample
                .stages
                .iter()
                .any(|stage| stage.stage.trim().is_empty())
        {
            return Err("performance run contains an invalid measured sample".to_owned());
        }
        let stage_total = sample
            .stages
            .iter()
            .try_fold(0_u64, |total, stage| total.checked_add(stage.elapsed_nanos))
            .ok_or_else(|| "performance stage duration overflow".to_owned())?;
        if stage_total > sample.total_elapsed_nanos {
            return Err("performance stages exceed the measured total".to_owned());
        }
        let expected_roles = paired_roles(contract.pair_order, run.candidate, sample.iteration);
        if sample.role != expected_roles[sample.pair_order as usize]
            || !sample_keys.insert((
                sample.operation,
                sample.iteration,
                sample.pair_order,
                sample.role,
            ))
        {
            return Err("performance sample pairing is invalid".to_owned());
        }
    }

    let expected_allocation_count = contract
        .allocation_scopes
        .len()
        .checked_mul(contract.inventory_states.len())
        .and_then(|count| count.checked_mul(2))
        .ok_or_else(|| "performance allocation count overflow".to_owned())?;
    if run.allocations.len() != expected_allocation_count {
        return Err("performance run has the wrong allocation snapshot count".to_owned());
    }
    let mut allocation_keys = BTreeSet::new();
    for allocation in &run.allocations {
        if !contract.allocation_scopes.contains(&allocation.scope)
            || !contract.inventory_states.contains(&allocation.state)
            || ![run.candidate, QualificationPerformanceRoleV1::LooseBaseline]
                .contains(&allocation.role)
            || allocation.inventory.carrier_count == 0
            || allocation.inventory.logical_bytes == 0
            || allocation.inventory.encoded_bytes == 0
            || allocation.inventory.allocated_bytes == 0
            || allocation.inventory.high_water_bytes < allocation.inventory.allocated_bytes
            || !allocation_keys.insert((allocation.role, allocation.scope, allocation.state))
        {
            return Err("performance allocation snapshot is invalid".to_owned());
        }
        validate_hex(
            &allocation.inventory.carrier_set_sha256,
            64,
            "performance carrier-set SHA-256",
        )?;
    }
    Ok(())
}

fn timing_pairs(
    run: &QualificationPerformanceRunV2,
    candidate: QualificationPerformanceRoleV1,
    operation: QualificationPerformanceOperationV1,
) -> Result<Vec<(u64, u64)>, String> {
    let mut pairs = Vec::with_capacity(run.measured_samples as usize);
    for iteration in 0..run.measured_samples {
        let candidate_nanos = run
            .samples
            .iter()
            .find(|sample| {
                sample.operation == operation
                    && sample.iteration == iteration
                    && sample.role == candidate
            })
            .map(|sample| sample.total_elapsed_nanos)
            .ok_or_else(|| "candidate timing sample is missing".to_owned())?;
        let baseline_nanos = run
            .samples
            .iter()
            .find(|sample| {
                sample.operation == operation
                    && sample.iteration == iteration
                    && sample.role == QualificationPerformanceRoleV1::LooseBaseline
            })
            .map(|sample| sample.total_elapsed_nanos)
            .ok_or_else(|| "baseline timing sample is missing".to_owned())?;
        pairs.push((candidate_nanos, baseline_nanos));
    }
    Ok(pairs)
}

fn sorted_timing_pairs(pairs: &[(u64, u64)]) -> Vec<(u64, u64)> {
    let mut sorted = pairs.to_vec();
    sorted.sort_unstable_by(|left, right| {
        (u128::from(left.0) * u128::from(right.1)).cmp(&(u128::from(right.0) * u128::from(left.1)))
    });
    sorted
}

fn ratio_at_rank(pairs: &[(u64, u64)], percentile: usize) -> (u64, u64) {
    let sorted = sorted_timing_pairs(pairs);
    let rank = sorted.len().saturating_mul(percentile).div_ceil(100).max(1);
    sorted[rank - 1]
}

fn ratio_millionths(pair: (u64, u64)) -> u64 {
    let scaled = u128::from(pair.0) * 1_000_000;
    u64::try_from((scaled + u128::from(pair.1 / 2)) / u128::from(pair.1)).unwrap_or(u64::MAX)
}

fn timing_summary(pairs: &[(u64, u64)]) -> QualificationPerformanceTimingSummaryV2 {
    let sorted = sorted_timing_pairs(pairs);
    let ratios = sorted
        .iter()
        .copied()
        .map(ratio_millionths)
        .collect::<Vec<_>>();
    let mean = ratios.iter().map(|value| *value as f64).sum::<f64>() / ratios.len() as f64;
    let variance = ratios
        .iter()
        .map(|value| {
            let difference = *value as f64 - mean;
            difference * difference
        })
        .sum::<f64>()
        / ratios.len() as f64;
    QualificationPerformanceTimingSummaryV2 {
        p50_ratio_millionths: ratio_millionths(ratio_at_rank(&sorted, 50)),
        p95_ratio_millionths: ratio_millionths(ratio_at_rank(&sorted, 95)),
        minimum_ratio_millionths: *ratios.first().expect("validated timing pairs"),
        maximum_ratio_millionths: *ratios.last().expect("validated timing pairs"),
        population_standard_deviation_millionths: variance.sqrt().round() as u64,
    }
}

fn allocation_inventory(
    run: &QualificationPerformanceRunV2,
    role: QualificationPerformanceRoleV1,
    scope: QualificationPerformanceAllocationScopeV2,
    state: QualificationPerformanceInventoryStateV1,
) -> Result<&QualificationPerformanceInventoryV2, String> {
    run.allocations
        .iter()
        .find(|allocation| {
            allocation.role == role && allocation.scope == scope && allocation.state == state
        })
        .map(|allocation| &allocation.inventory)
        .ok_or_else(|| "required allocation snapshot is missing".to_owned())
}

fn allocation_bytes(
    inventory: &QualificationPerformanceInventoryV2,
    state: QualificationPerformanceInventoryStateV1,
) -> u64 {
    match state {
        QualificationPerformanceInventoryStateV1::Steady
        | QualificationPerformanceInventoryStateV1::Reopened => inventory.allocated_bytes,
        QualificationPerformanceInventoryStateV1::HighWater => inventory.high_water_bytes,
    }
}

fn aggregate_criterion_status(
    statuses: impl Iterator<Item = QualificationPerformanceCriterionStatusV2>,
) -> QualificationPerformanceCriterionStatusV2 {
    let mut aggregate = QualificationPerformanceCriterionStatusV2::Passed;
    for status in statuses {
        match status {
            QualificationPerformanceCriterionStatusV2::Failed => {
                return QualificationPerformanceCriterionStatusV2::Failed;
            }
            QualificationPerformanceCriterionStatusV2::Unknown => {
                aggregate = QualificationPerformanceCriterionStatusV2::Unknown;
            }
            QualificationPerformanceCriterionStatusV2::Passed => {}
        }
    }
    aggregate
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPerformanceInventorySnapshotV1 {
    pub role: QualificationPerformanceRoleV1,
    pub state: QualificationPerformanceInventoryStateV1,
    pub inventory: QualificationInventoryV1,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPerformanceDiagnosticCaseV1 {
    pub candidate: QualificationPerformanceRoleV1,
    pub candidate_build_id: String,
    pub physical_profile_id: String,
    pub workload_manifest_sha256: String,
    pub samples: Vec<QualificationPerformanceDiagnosticSampleV1>,
    pub inventories: Vec<QualificationPerformanceInventorySnapshotV1>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPerformanceDiagnosticsReportV1 {
    pub schema: String,
    pub contract_schema: String,
    pub contract_sha256: String,
    pub source_commit: String,
    pub cargo_lock_sha256: String,
    pub environment: QualificationPlatformEnvironmentV1,
    pub warmup_samples: u32,
    pub measured_samples: u32,
    pub pair_order: QualificationPerformancePairOrderV1,
    pub cases: Vec<QualificationPerformanceDiagnosticCaseV1>,
    pub report_sha256: String,
}

impl QualificationPerformanceDiagnosticsReportV1 {
    pub fn canonical_sha256(&self) -> Result<String, String> {
        let mut preimage = self.clone();
        preimage.report_sha256.clear();
        let value = serde_json::to_value(preimage).map_err(|error| error.to_string())?;
        canonical_json_bytes(&value)
            .map(|bytes| sha256_bytes_hex(&bytes))
            .map_err(|error| error.to_string())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != QUALIFICATION_PERFORMANCE_DIAGNOSTICS_SCHEMA_V1 {
            return Err("unsupported performance diagnostics schema".to_owned());
        }
        if self.contract_schema != QUALIFICATION_PERFORMANCE_DIAGNOSTIC_CONTRACT_SCHEMA_V1
            || self.contract_sha256 != diagnostic_contract_sha256()
        {
            return Err("performance diagnostics use a different contract".to_owned());
        }
        validate_hex(&self.source_commit, 40, "source commit")?;
        validate_hex(&self.cargo_lock_sha256, 64, "Cargo.lock SHA-256")?;
        if self.warmup_samples == 0 || self.measured_samples == 0 || self.cases.is_empty() {
            return Err("performance diagnostics are incomplete".to_owned());
        }
        if self.environment.filesystem_disposition
            != QualificationFilesystemDispositionV1::LocalProofEligible
            || self.environment.operating_system.is_empty()
            || self.environment.architecture.is_empty()
            || self.environment.filesystem.is_empty()
            || self.environment.allocation_method.is_empty()
            || self.environment.rustc.is_empty()
            || self.environment.build_source.is_empty()
            || self.environment.build_describe.is_empty()
        {
            return Err("performance diagnostics require a local proof filesystem".to_owned());
        }
        for case in &self.cases {
            if case.candidate == QualificationPerformanceRoleV1::LooseBaseline
                || case.candidate_build_id.is_empty()
                || case.physical_profile_id.is_empty()
            {
                return Err(
                    "performance diagnostic case has incomplete candidate identity".to_owned(),
                );
            }
            validate_hex(
                &case.workload_manifest_sha256,
                64,
                "workload manifest SHA-256",
            )?;
            let expected_samples = usize::try_from(self.measured_samples)
                .map_err(|_| "measured sample count exceeds this platform".to_owned())?
                .saturating_mul(QualificationPerformanceOperationV1::ALL.len())
                .saturating_mul(2);
            if case.samples.len() != expected_samples {
                return Err("performance diagnostic case has incomplete samples".to_owned());
            }
            for sample in &case.samples {
                if !matches!(sample.role, QualificationPerformanceRoleV1::LooseBaseline)
                    && sample.role != case.candidate
                    || sample.pair_order > 1
                    || sample.total_elapsed_nanos == 0
                    || sample
                        .stages
                        .iter()
                        .any(|stage| stage.stage.trim().is_empty() || stage.elapsed_nanos == 0)
                    || sample
                        .stages
                        .iter()
                        .try_fold(0_u64, |total, stage| total.checked_add(stage.elapsed_nanos))
                        .is_none_or(|stages| stages > sample.total_elapsed_nanos)
                {
                    return Err(
                        "performance diagnostic sample has invalid timing stages".to_owned()
                    );
                }
            }
            for operation in QualificationPerformanceOperationV1::ALL {
                for role in [
                    case.candidate,
                    QualificationPerformanceRoleV1::LooseBaseline,
                ] {
                    let count = case
                        .samples
                        .iter()
                        .filter(|sample| sample.operation == operation && sample.role == role)
                        .count();
                    if count != self.measured_samples as usize {
                        return Err(
                            "performance diagnostic case is missing an operation role".to_owned()
                        );
                    }
                }
            }
            let inventories = case
                .inventories
                .iter()
                .map(|snapshot| (snapshot.role, snapshot.state))
                .collect::<BTreeSet<_>>();
            let required_inventories = [
                case.candidate,
                QualificationPerformanceRoleV1::LooseBaseline,
            ]
            .into_iter()
            .flat_map(|role| {
                [
                    QualificationPerformanceInventoryStateV1::Steady,
                    QualificationPerformanceInventoryStateV1::Reopened,
                    QualificationPerformanceInventoryStateV1::HighWater,
                ]
                .into_iter()
                .map(move |state| (role, state))
            })
            .collect::<BTreeSet<_>>();
            if inventories != required_inventories
                || case.inventories.iter().any(|snapshot| {
                    snapshot.inventory.carriers.is_empty()
                        || snapshot.inventory.encoded_bytes == 0
                        || snapshot.inventory.high_water_bytes < snapshot.inventory.allocated_bytes
                })
            {
                return Err("performance diagnostic case has incomplete inventory".to_owned());
            }
        }
        let case_keys = self
            .cases
            .iter()
            .map(|case| (case.workload_manifest_sha256.as_str(), case.candidate))
            .collect::<BTreeSet<_>>();
        let workloads = self
            .cases
            .iter()
            .map(|case| case.workload_manifest_sha256.as_str())
            .collect::<BTreeSet<_>>();
        if case_keys.len() != self.cases.len()
            || workloads.iter().any(|workload| {
                QualificationPerformanceRoleV1::CANDIDATES
                    .iter()
                    .any(|candidate| !case_keys.contains(&(*workload, *candidate)))
            })
        {
            return Err("performance diagnostics have an incomplete candidate matrix".to_owned());
        }
        if self.report_sha256 != self.canonical_sha256()? {
            return Err(
                "performance diagnostic report hash does not match its preimage".to_owned(),
            );
        }
        Ok(())
    }
}

pub fn diagnostic_contract_sha256() -> String {
    let contract = serde_json::json!({
        "schema": QUALIFICATION_PERFORMANCE_DIAGNOSTIC_CONTRACT_SCHEMA_V1,
        "operations": QualificationPerformanceOperationV1::ALL,
        "roles": [
            QualificationPerformanceRoleV1::LooseBaseline,
            QualificationPerformanceRoleV1::SqliteWal,
            QualificationPerformanceRoleV1::BoundedSegments,
        ],
        "inventoryStates": [
            QualificationPerformanceInventoryStateV1::Steady,
            QualificationPerformanceInventoryStateV1::Reopened,
            QualificationPerformanceInventoryStateV1::HighWater,
        ],
        "gating": false,
    });
    sha256_bytes_hex(&canonical_json_bytes(&contract).expect("static contract is canonical"))
}

pub fn validate_diagnostic_configuration(
    configuration: &QualificationPerformanceDiagnosticConfigurationV1,
) -> Result<(), String> {
    if configuration.warmup_samples == 0 || configuration.measured_samples == 0 {
        return Err("performance diagnostics require warm-up and measured samples".to_owned());
    }
    if !configuration.executable.is_file() {
        return Err("performance diagnostics executable does not exist".to_owned());
    }
    if configuration
        .root
        .try_exists()
        .map_err(|error| error.to_string())?
    {
        return Err("performance diagnostics root must be a fresh path".to_owned());
    }
    if configuration.source_commit != qualification_source_commit()? {
        return Err("performance diagnostics source commit is stale".to_owned());
    }
    if configuration.cargo_lock_sha256 != qualification_cargo_lock_sha256() {
        return Err("performance diagnostics Cargo.lock hash is stale".to_owned());
    }
    let parent = configuration
        .root
        .parent()
        .ok_or_else(|| "performance diagnostics root has no parent".to_owned())?;
    if !parent.is_dir() {
        return Err("performance diagnostics root parent does not exist".to_owned());
    }
    let filesystem = qualification_filesystem_name(parent);
    if classify_qualification_filesystem(&filesystem)
        != QualificationFilesystemDispositionV1::LocalProofEligible
    {
        return Err(format!(
            "performance diagnostics require a local proof filesystem, found {filesystem}"
        ));
    }
    Ok(())
}

pub fn run_qualification_performance_campaign_v2(
    configuration: &QualificationPerformanceCampaignConfigurationV2,
) -> Result<QualificationPerformanceEvidenceV2, String> {
    validate_campaign_configuration(configuration)?;
    let contract = QualificationPerformanceContractV2::frozen();
    let operating_system = std::env::consts::OS.to_owned();
    let workloads = qualification_campaign_workloads(
        &contract,
        &operating_system,
        configuration.external_corpus_root.as_deref(),
    )?;
    fs::create_dir(&configuration.root)
        .map_err(|_| "performance campaign root creation failed".to_owned())?;
    let filesystem = qualification_filesystem_name(&configuration.root);
    let environment = QualificationPlatformEnvironmentV1 {
        operating_system: operating_system.clone(),
        architecture: std::env::consts::ARCH.to_owned(),
        filesystem: filesystem.clone(),
        filesystem_disposition: classify_qualification_filesystem(&filesystem),
        allocation_method: final_native_allocation_method().to_owned(),
        rustc: rustc_verbose_version(),
        build_source: env!("POINTBREAK_BUILD_SOURCE").to_owned(),
        build_describe: env!("POINTBREAK_BUILD_DESCRIBE").to_owned(),
        source_tree_dirty: env!("POINTBREAK_BUILD_DIRTY") == "true",
    };
    let operating_system_version = native_operating_system_version();
    let cpu = native_cpu_description();
    let mut runs = Vec::new();
    for (workload_kind, workload) in &workloads {
        for candidate in QualificationPerformanceRoleV1::CANDIDATES {
            for run_index in 1..=contract.independent_runs {
                let case_root = configuration.root.join(format!(
                    "{}-{}-run-{run_index}",
                    candidate.as_str(),
                    workload_kind.as_str(),
                ));
                fs::create_dir(&case_root)
                    .map_err(|_| "performance campaign case creation failed".to_owned())?;
                runs.push(run_campaign_case(
                    configuration,
                    &contract,
                    candidate,
                    *workload_kind,
                    workload,
                    run_index,
                    &case_root,
                    &environment,
                    &operating_system_version,
                    &cpu,
                )?);
            }
        }
    }
    let mut evidence = QualificationPerformanceEvidenceV2 {
        schema: QUALIFICATION_PERFORMANCE_EVIDENCE_SCHEMA_V2.to_owned(),
        contract_schema: contract.schema.clone(),
        contract_sha256: contract.canonical_sha256()?,
        source_commit: configuration.source_commit.clone(),
        cargo_lock_sha256: configuration.cargo_lock_sha256.clone(),
        runs,
        evidence_sha256: String::new(),
    };
    evidence.evidence_sha256 = evidence.canonical_sha256()?;
    evidence.validate()?;
    Ok(evidence)
}

pub fn run_qualification_performance_open_child(request_path: &Path) -> Result<(), String> {
    let bytes = fs::read(request_path)
        .map_err(|_| "performance open request could not be read".to_owned())?;
    let request: QualificationPerformanceOpenRequestV2 = serde_json::from_slice(&bytes)
        .map_err(|_| "performance open request is invalid".to_owned())?;
    if !request.root.is_dir()
        || request.result_path.exists()
        || request
            .result_path
            .parent()
            .is_none_or(|parent| !parent.is_dir())
    {
        return Err("performance open request paths are invalid".to_owned());
    }
    let receipt = if request.role == QualificationPerformanceRoleV1::LooseBaseline {
        loose_semantic_receipt(&request.root)?
    } else {
        DiagnosticCandidateProfile::open(request.role, &request.root)?.semantic_receipt()?
    };
    write_json_new_synced(&request.result_path, &receipt)
}

fn validate_campaign_configuration(
    configuration: &QualificationPerformanceCampaignConfigurationV2,
) -> Result<(), String> {
    let contract = QualificationPerformanceContractV2::frozen();
    contract.validate()?;
    if !configuration.executable.is_file()
        || configuration.root.exists()
        || configuration
            .root
            .parent()
            .is_none_or(|parent| !parent.is_dir())
        || configuration.source_commit != qualification_source_commit()?
        || configuration.cargo_lock_sha256 != qualification_cargo_lock_sha256()
        || !configuration.quiesced_host
        || env!("POINTBREAK_BUILD_DIRTY") == "true"
    {
        return Err("performance campaign configuration is not proof-eligible".to_owned());
    }
    let parent = configuration
        .root
        .parent()
        .ok_or_else(|| "performance campaign root has no parent".to_owned())?;
    let filesystem = qualification_filesystem_name(parent);
    if classify_qualification_filesystem(&filesystem)
        != QualificationFilesystemDispositionV1::LocalProofEligible
    {
        return Err("performance campaign requires a local proof filesystem".to_owned());
    }
    if !qualification_performance_platform_is_supported(
        &contract,
        std::env::consts::OS,
        &filesystem,
        final_native_allocation_method(),
    ) {
        return Err("performance campaign platform is outside the frozen contract".to_owned());
    }
    match std::env::consts::OS {
        "macos" if configuration.external_corpus_root.is_none() => {
            Err("performance campaign requires the external workload on macOS".to_owned())
        }
        "macos" => Ok(()),
        "linux" | "windows" if configuration.external_corpus_root.is_some() => {
            Err("performance campaign accepts the external workload only on macOS".to_owned())
        }
        "linux" | "windows" => Ok(()),
        _ => Err("performance campaign platform is unsupported".to_owned()),
    }
}

fn qualification_performance_platform_is_supported(
    contract: &QualificationPerformanceContractV2,
    operating_system: &str,
    filesystem: &str,
    allocation_method: &str,
) -> bool {
    contract.workloads.iter().any(|workload| {
        workload.platforms.iter().any(|platform| {
            platform.operating_system == operating_system
                && platform.filesystem == filesystem
                && platform.allocation_method == allocation_method
        })
    })
}

fn qualification_campaign_workloads(
    contract: &QualificationPerformanceContractV2,
    operating_system: &str,
    external_corpus_root: Option<&Path>,
) -> Result<
    Vec<(
        QualificationPerformanceWorkloadV2,
        QualificationCorpusManifestV1,
    )>,
    String,
> {
    let public = synthetic_legacy_manifest()
        .map_err(|_| "public performance workload is invalid".to_owned())?;
    let modeled = modeled_post_foundation_manifest()
        .map_err(|_| "modeled performance workload is invalid".to_owned())?;
    let mut workloads = Vec::new();
    for requirement in &contract.workloads {
        if !requirement
            .platforms
            .iter()
            .any(|platform| platform.operating_system == operating_system)
        {
            continue;
        }
        let manifest = match requirement.workload {
            QualificationPerformanceWorkloadV2::ExternalCorpus => {
                load_external_workload_v2_manifest_from_path(external_corpus_root).map_err(
                    |_| "external performance workload is invalid or has drifted".to_owned(),
                )?
            }
            QualificationPerformanceWorkloadV2::ModeledFoundation => modeled.clone(),
            QualificationPerformanceWorkloadV2::PublicSmoke => public.clone(),
        };
        if manifest.manifest_sha256 != requirement.manifest_sha256 {
            return Err("performance workload manifest does not match the contract".to_owned());
        }
        workloads.push((requirement.workload, manifest));
    }
    Ok(workloads)
}

#[allow(clippy::too_many_arguments)]
fn run_campaign_case(
    configuration: &QualificationPerformanceCampaignConfigurationV2,
    contract: &QualificationPerformanceContractV2,
    candidate_role: QualificationPerformanceRoleV1,
    workload_kind: QualificationPerformanceWorkloadV2,
    workload: &QualificationCorpusManifestV1,
    run_index: u32,
    case_root: &Path,
    environment: &QualificationPlatformEnvironmentV1,
    operating_system_version: &str,
    cpu: &str,
) -> Result<QualificationPerformanceRunV2, String> {
    let selected = workload
        .records
        .iter()
        .find(|record| is_journal_record(record.record_kind))
        .ok_or_else(|| "performance workload has no journal record".to_owned())?;
    let mut journal_lengths = workload
        .records
        .iter()
        .filter(|record| is_journal_record(record.record_kind))
        .map(|record| record.decoded_bytes.len())
        .collect::<Vec<_>>();
    journal_lengths.sort_unstable();
    let representative_len = journal_lengths[journal_lengths.len() / 2];

    let warmup_root = case_root.join("warmup");
    fs::create_dir(&warmup_root)
        .map_err(|_| "performance warm-up root creation failed".to_owned())?;
    let warmup_candidate_root = warmup_root.join("candidate");
    let warmup_candidate =
        DiagnosticCandidateProfile::open(candidate_role, &warmup_candidate_root)?;
    populate_profile(warmup_candidate.as_profile(), workload)
        .map_err(|_| "performance warm-up candidate population failed".to_owned())?;
    let warmup_loose =
        LooseQualificationPerformanceProbe::create(warmup_root.join("loose"), workload)?;
    for iteration in 0..contract.warmup_samples {
        run_campaign_iteration(
            configuration,
            &warmup_candidate,
            &warmup_candidate_root,
            &warmup_loose,
            candidate_role,
            selected,
            representative_len,
            iteration,
            "warmup",
            &warmup_root,
        )?;
    }
    drop(warmup_loose);
    drop(warmup_candidate);
    fs::remove_dir_all(&warmup_root)
        .map_err(|_| "performance warm-up roots could not be discarded".to_owned())?;

    let candidate_root = case_root.join("candidate");
    let loose_root = case_root.join("loose");
    let candidate = DiagnosticCandidateProfile::open(candidate_role, &candidate_root)?;
    populate_profile(candidate.as_profile(), workload)
        .map_err(|_| "performance candidate population failed".to_owned())?;
    let loose = LooseQualificationPerformanceProbe::create(loose_root.clone(), workload)?;
    let event_logical_base = workload
        .records
        .iter()
        .filter(|record| is_journal_record(record.record_kind))
        .try_fold(0_u64, |total, record| {
            total.checked_add(record.decoded_bytes.len() as u64)
        })
        .ok_or_else(|| "performance event logical-byte total overflow".to_owned())?;
    let complete_logical_base = workload
        .records
        .iter()
        .try_fold(0_u64, |total, record| {
            total.checked_add(record.decoded_bytes.len() as u64)
        })
        .ok_or_else(|| "performance complete logical-byte total overflow".to_owned())?;
    let mut high_water = BTreeMap::new();
    capture_campaign_inventories(
        candidate_role,
        &candidate_root,
        &loose_root,
        event_logical_base,
        complete_logical_base,
        &mut high_water,
    )?;
    let mut samples = Vec::new();
    let mut appended_bytes = 0_u64;
    for iteration in 0..contract.measured_samples {
        samples.extend(run_campaign_iteration(
            configuration,
            &candidate,
            &candidate_root,
            &loose,
            candidate_role,
            selected,
            representative_len,
            iteration,
            "measured",
            case_root,
        )?);
        appended_bytes = appended_bytes
            .checked_add(representative_len as u64)
            .ok_or_else(|| "performance append logical-byte total overflow".to_owned())?;
        capture_campaign_inventories(
            candidate_role,
            &candidate_root,
            &loose_root,
            event_logical_base + appended_bytes,
            complete_logical_base + appended_bytes,
            &mut high_water,
        )?;
    }
    let event_logical = event_logical_base + appended_bytes;
    let complete_logical = complete_logical_base + appended_bytes;
    let steady = capture_campaign_inventories(
        candidate_role,
        &candidate_root,
        &loose_root,
        event_logical,
        complete_logical,
        &mut high_water,
    )?;
    candidate.maintenance_boundary()?;
    let high_water_current = capture_campaign_inventories(
        candidate_role,
        &candidate_root,
        &loose_root,
        event_logical,
        complete_logical,
        &mut high_water,
    )?;
    drop(candidate);
    let candidate_receipt = spawn_open_receipt(
        configuration,
        candidate_role,
        &candidate_root,
        case_root,
        "final-candidate",
        0,
        0,
    )?
    .1;
    let loose_receipt = spawn_open_receipt(
        configuration,
        QualificationPerformanceRoleV1::LooseBaseline,
        &loose_root,
        case_root,
        "final-loose",
        0,
        0,
    )?
    .1;
    if candidate_receipt != loose_receipt {
        return Err("performance final semantic receipts differ".to_owned());
    }
    let reopened_candidate = DiagnosticCandidateProfile::open(candidate_role, &candidate_root)?;
    if reopened_candidate.semantic_receipt()? != candidate_receipt {
        return Err("performance reopened candidate receipt differs".to_owned());
    }
    let reopened = capture_campaign_inventories(
        candidate_role,
        &candidate_root,
        &loose_root,
        event_logical,
        complete_logical,
        &mut high_water,
    )?;
    let allocations = campaign_allocation_snapshots(
        candidate_role,
        &steady,
        &reopened,
        &high_water_current,
        &high_water,
    )?;
    let qualification_candidate = match candidate_role {
        QualificationPerformanceRoleV1::SqliteWal => QualificationCandidateV1::SqliteWal,
        QualificationPerformanceRoleV1::BoundedSegments => {
            QualificationCandidateV1::BoundedSegments
        }
        QualificationPerformanceRoleV1::LooseBaseline => unreachable!(),
    };
    let physical_profile_id = match candidate_role {
        QualificationPerformanceRoleV1::SqliteWal => SQLITE_QUALIFICATION_PROFILE_ID_V1,
        QualificationPerformanceRoleV1::BoundedSegments => SEGMENT_QUALIFICATION_PROFILE_ID_V1,
        QualificationPerformanceRoleV1::LooseBaseline => unreachable!(),
    };
    Ok(QualificationPerformanceRunV2 {
        run_index,
        workload: workload_kind,
        workload_manifest_sha256: workload.manifest_sha256.clone(),
        candidate: candidate_role,
        candidate_build_id: qualification_candidate.build_id(&configuration.cargo_lock_sha256),
        physical_profile_id: physical_profile_id.to_owned(),
        environment: environment.clone(),
        operating_system_version: operating_system_version.to_owned(),
        cpu: cpu.to_owned(),
        target_architecture: environment.architecture.clone(),
        run_identity: format!(
            "{}-{}-{}-{}-run-{run_index}",
            environment.operating_system,
            environment.architecture,
            workload_kind.as_str(),
            candidate_role.as_str(),
        ),
        warmup_samples: contract.warmup_samples,
        measured_samples: contract.measured_samples,
        pair_order: contract.pair_order,
        confidence_method: contract.confidence_method,
        outlier_policy: contract.outlier_policy,
        cache_policy: contract.cache_policy,
        controls: QualificationPerformanceRunControlsV2 {
            fresh_roots: true,
            quiesced_host: configuration.quiesced_host,
            native_execution: true,
            equivalent_decoded_bytes: true,
            monotonic_append_state: true,
            durable_acknowledgement: true,
            semantic_validation: true,
            open_recovery_fresh_process: true,
        },
        semantic_receipt_sha256: candidate_receipt.receipt_sha256,
        samples,
        allocations,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_campaign_iteration(
    configuration: &QualificationPerformanceCampaignConfigurationV2,
    candidate: &DiagnosticCandidateProfile,
    candidate_profile_root: &Path,
    loose: &LooseQualificationPerformanceProbe,
    candidate_role: QualificationPerformanceRoleV1,
    selected: &super::QualificationRecordV1,
    representative_len: usize,
    iteration: u32,
    series: &str,
    control_root: &Path,
) -> Result<Vec<QualificationPerformanceDiagnosticSampleV1>, String> {
    let append = generate_qualification_append_record(series, iteration, representative_len)?;
    let mut samples = Vec::with_capacity(QualificationPerformanceOperationV1::ALL.len() * 2);
    for operation in QualificationPerformanceOperationV1::ALL {
        let (logical_key, decoded_bytes) =
            if operation == QualificationPerformanceOperationV1::DurableAppend {
                (append.logical_key.as_str(), append.decoded_bytes.as_slice())
            } else {
                (
                    selected.logical_key.as_str(),
                    selected.decoded_bytes.as_slice(),
                )
            };
        let mut receipts = Vec::new();
        for (pair_order, role) in paired_roles(
            QualificationPerformancePairOrderV1::Alternating,
            candidate_role,
            iteration,
        )
        .into_iter()
        .enumerate()
        {
            let request = QualificationPerformanceOperationRequestV1 {
                operation,
                role,
                iteration,
                pair_order: pair_order as u8,
                logical_key,
                decoded_bytes,
            };
            let (sample, receipt) = match operation {
                QualificationPerformanceOperationV1::StrictReplay => {
                    let (sample, receipt) = campaign_replay_sample(candidate, loose, &request)?;
                    (sample, Some(receipt))
                }
                QualificationPerformanceOperationV1::OpenRecovery => {
                    let (sample, receipt) = spawn_open_receipt(
                        configuration,
                        role,
                        if role == QualificationPerformanceRoleV1::LooseBaseline {
                            &loose.root
                        } else {
                            candidate_profile_root
                        },
                        control_root,
                        &format!("{series}-{iteration}-{pair_order}"),
                        iteration,
                        pair_order as u8,
                    )?;
                    (sample, Some(receipt))
                }
                _ if role == QualificationPerformanceRoleV1::LooseBaseline => {
                    (loose.run_profiled_operation(&request)?, None)
                }
                _ => (candidate.as_probe().run_profiled_operation(&request)?, None),
            };
            if let Some(receipt) = receipt {
                receipts.push((role, receipt));
            }
            samples.push(sample);
        }
        if matches!(
            operation,
            QualificationPerformanceOperationV1::StrictReplay
                | QualificationPerformanceOperationV1::OpenRecovery
        ) {
            let candidate_receipt = receipts
                .iter()
                .find(|(role, _)| *role == candidate_role)
                .map(|(_, receipt)| receipt);
            let loose_receipt = receipts
                .iter()
                .find(|(role, _)| *role == QualificationPerformanceRoleV1::LooseBaseline)
                .map(|(_, receipt)| receipt);
            if candidate_receipt.is_none() || candidate_receipt != loose_receipt {
                return Err("performance semantic receipts differ".to_owned());
            }
        }
        if operation == QualificationPerformanceOperationV1::DurableAppend {
            let candidate_bytes = candidate
                .as_profile()
                .journal()
                .read(logical_key)
                .map_err(|_| "performance candidate append verification failed".to_owned())?
                .ok_or_else(|| "performance candidate append is missing".to_owned())?
                .decoded_bytes;
            if candidate_bytes != decoded_bytes {
                return Err("performance candidate append returned different bytes".to_owned());
            }
            let baseline_request = QualificationPerformanceOperationRequestV1 {
                operation,
                role: QualificationPerformanceRoleV1::LooseBaseline,
                iteration,
                pair_order: 0,
                logical_key,
                decoded_bytes,
            };
            loose.verify_read(&baseline_request)?;
        }
    }
    Ok(samples)
}

fn campaign_replay_sample(
    candidate: &DiagnosticCandidateProfile,
    loose: &LooseQualificationPerformanceProbe,
    request: &QualificationPerformanceOperationRequestV1<'_>,
) -> Result<
    (
        QualificationPerformanceDiagnosticSampleV1,
        QualificationPerformanceSemanticReceiptV2,
    ),
    String,
> {
    let started = Instant::now();
    let receipt = if request.role == QualificationPerformanceRoleV1::LooseBaseline {
        loose_semantic_receipt(&loose.root)?
    } else {
        candidate.semantic_receipt()?
    };
    let elapsed = elapsed_nanos(started);
    Ok((
        QualificationPerformanceDiagnosticSampleV1 {
            operation: request.operation,
            role: request.role,
            iteration: request.iteration,
            pair_order: request.pair_order,
            total_elapsed_nanos: elapsed,
            stages: vec![QualificationPerformanceStageSampleV1 {
                stage: "replay_receipt".to_owned(),
                elapsed_nanos: elapsed,
            }],
        },
        receipt,
    ))
}

fn spawn_open_receipt(
    configuration: &QualificationPerformanceCampaignConfigurationV2,
    role: QualificationPerformanceRoleV1,
    root: &Path,
    control_root: &Path,
    label: &str,
    iteration: u32,
    pair_order: u8,
) -> Result<
    (
        QualificationPerformanceDiagnosticSampleV1,
        QualificationPerformanceSemanticReceiptV2,
    ),
    String,
> {
    let control = control_root.join("open-controls");
    fs::create_dir_all(&control)
        .map_err(|_| "performance open control directory creation failed".to_owned())?;
    let role_label = role.as_str();
    let request_path = control.join(format!("{label}-{role_label}-request.json"));
    let result_path = control.join(format!("{label}-{role_label}-result.json"));
    let request = QualificationPerformanceOpenRequestV2 {
        role,
        root: root.to_path_buf(),
        result_path: result_path.clone(),
    };
    write_json_new_synced(&request_path, &request)?;
    let started = Instant::now();
    let output = Command::new(&configuration.executable)
        .arg("--qualification-performance-open-child")
        .arg(&request_path)
        .output()
        .map_err(|_| "performance open child could not start".to_owned())?;
    let elapsed = elapsed_nanos(started);
    if !output.status.success() {
        return Err("performance open child failed".to_owned());
    }
    let result_bytes = fs::read(&result_path)
        .map_err(|_| "performance open result could not be read".to_owned())?;
    let receipt: QualificationPerformanceSemanticReceiptV2 = serde_json::from_slice(&result_bytes)
        .map_err(|_| "performance open result is invalid".to_owned())?;
    validate_hex(
        &receipt.receipt_sha256,
        64,
        "performance semantic receipt SHA-256",
    )?;
    fs::remove_file(&request_path)
        .and_then(|_| fs::remove_file(&result_path))
        .map_err(|_| "performance open control cleanup failed".to_owned())?;
    Ok((
        QualificationPerformanceDiagnosticSampleV1 {
            operation: QualificationPerformanceOperationV1::OpenRecovery,
            role,
            iteration,
            pair_order,
            total_elapsed_nanos: elapsed,
            stages: vec![QualificationPerformanceStageSampleV1 {
                stage: "fresh_process_open_receipt".to_owned(),
                elapsed_nanos: elapsed,
            }],
        },
        receipt,
    ))
}

fn capture_campaign_inventories(
    candidate_role: QualificationPerformanceRoleV1,
    candidate_root: &Path,
    loose_root: &Path,
    event_logical_bytes: u64,
    complete_logical_bytes: u64,
    high_water: &mut BTreeMap<
        (
            QualificationPerformanceRoleV1,
            QualificationPerformanceAllocationScopeV2,
        ),
        u64,
    >,
) -> Result<
    BTreeMap<
        (
            QualificationPerformanceRoleV1,
            QualificationPerformanceAllocationScopeV2,
        ),
        QualificationPerformanceInventoryV2,
    >,
    String,
> {
    let mut inventories = BTreeMap::new();
    for (role, root) in [
        (candidate_role, candidate_root),
        (QualificationPerformanceRoleV1::LooseBaseline, loose_root),
    ] {
        for scope in [
            QualificationPerformanceAllocationScopeV2::Event,
            QualificationPerformanceAllocationScopeV2::CompleteProfile,
        ] {
            let logical_bytes = match scope {
                QualificationPerformanceAllocationScopeV2::Event => event_logical_bytes,
                QualificationPerformanceAllocationScopeV2::CompleteProfile => {
                    complete_logical_bytes
                }
            };
            let mut inventory = scoped_native_inventory(root, scope, logical_bytes, 0)?;
            let observed = high_water.entry((role, scope)).or_default();
            *observed = (*observed).max(inventory.allocated_bytes);
            inventory.high_water_bytes = *observed;
            inventories.insert((role, scope), inventory);
        }
    }
    Ok(inventories)
}

fn campaign_allocation_snapshots(
    candidate_role: QualificationPerformanceRoleV1,
    steady: &BTreeMap<
        (
            QualificationPerformanceRoleV1,
            QualificationPerformanceAllocationScopeV2,
        ),
        QualificationPerformanceInventoryV2,
    >,
    reopened: &BTreeMap<
        (
            QualificationPerformanceRoleV1,
            QualificationPerformanceAllocationScopeV2,
        ),
        QualificationPerformanceInventoryV2,
    >,
    high_water_current: &BTreeMap<
        (
            QualificationPerformanceRoleV1,
            QualificationPerformanceAllocationScopeV2,
        ),
        QualificationPerformanceInventoryV2,
    >,
    high_water: &BTreeMap<
        (
            QualificationPerformanceRoleV1,
            QualificationPerformanceAllocationScopeV2,
        ),
        u64,
    >,
) -> Result<Vec<QualificationPerformanceAllocationSnapshotV2>, String> {
    let mut snapshots = Vec::new();
    for (state, source) in [
        (QualificationPerformanceInventoryStateV1::Steady, steady),
        (QualificationPerformanceInventoryStateV1::Reopened, reopened),
        (
            QualificationPerformanceInventoryStateV1::HighWater,
            high_water_current,
        ),
    ] {
        for scope in [
            QualificationPerformanceAllocationScopeV2::Event,
            QualificationPerformanceAllocationScopeV2::CompleteProfile,
        ] {
            for role in [
                candidate_role,
                QualificationPerformanceRoleV1::LooseBaseline,
            ] {
                let mut inventory = source
                    .get(&(role, scope))
                    .cloned()
                    .ok_or_else(|| "performance allocation snapshot is missing".to_owned())?;
                inventory.high_water_bytes = *high_water
                    .get(&(role, scope))
                    .ok_or_else(|| "performance allocation high-water is missing".to_owned())?;
                snapshots.push(QualificationPerformanceAllocationSnapshotV2 {
                    role,
                    scope,
                    state,
                    inventory,
                });
            }
        }
    }
    Ok(snapshots)
}

fn qualification_journal_receipt(
    entries: Vec<super::QualificationEntry>,
) -> Result<QualificationPerformanceSemanticReceiptV2, String> {
    let mut records = entries
        .into_iter()
        .map(|entry| {
            let actual = sha256_bytes_hex(&entry.decoded_bytes);
            if actual != entry.decoded_sha256 {
                return Err("performance journal receipt found a decoded hash mismatch".to_owned());
            }
            Ok((sha256_bytes_hex(entry.logical_key.as_bytes()), actual))
        })
        .collect::<Result<Vec<_>, String>>()?;
    records.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    let value = serde_json::to_value(&records).map_err(|error| error.to_string())?;
    let bytes = canonical_json_bytes(&value).map_err(|error| error.to_string())?;
    Ok(QualificationPerformanceSemanticReceiptV2 {
        record_count: records.len() as u64,
        receipt_sha256: sha256_bytes_hex(&bytes),
    })
}

fn loose_semantic_receipt(
    root: &Path,
) -> Result<QualificationPerformanceSemanticReceiptV2, String> {
    let events = root.join("events");
    let mut records = Vec::new();
    let entries = fs::read_dir(&events)
        .map_err(|_| "performance loose event directory could not be read".to_owned())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "performance loose event directory is invalid".to_owned())?;
    for entry in entries {
        let path = entry.path();
        if !entry
            .file_type()
            .map_err(|_| "performance loose carrier type could not be read".to_owned())?
            .is_file()
        {
            return Err("performance loose event carrier is not a file".to_owned());
        }
        let key_hash = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| "performance loose event carrier name is invalid".to_owned())?
            .to_owned();
        validate_hex(&key_hash, 64, "performance loose event key SHA-256")?;
        let bytes = fs::read(&path)
            .map_err(|_| "performance loose event carrier could not be read".to_owned())?;
        records.push((key_hash, sha256_bytes_hex(&bytes)));
    }
    records.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    let value = serde_json::to_value(&records).map_err(|error| error.to_string())?;
    let bytes = canonical_json_bytes(&value).map_err(|error| error.to_string())?;
    Ok(QualificationPerformanceSemanticReceiptV2 {
        record_count: records.len() as u64,
        receipt_sha256: sha256_bytes_hex(&bytes),
    })
}

fn is_journal_record(kind: QualificationRecordKindV1) -> bool {
    matches!(
        kind,
        QualificationRecordKindV1::LegacyEvent
            | QualificationRecordKindV1::GenerationProposal
            | QualificationRecordKindV1::RelationAttestation
            | QualificationRecordKindV1::FactPort
    )
}

fn write_json_new_synced(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let bytes =
        serde_json::to_vec(value).map_err(|_| "performance control JSON failed".to_owned())?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "performance control file creation failed".to_owned())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "performance control file write failed".to_owned())
}

pub fn run_qualification_performance_diagnostics(
    configuration: &QualificationPerformanceDiagnosticConfigurationV1,
) -> Result<QualificationPerformanceDiagnosticsReportV1, String> {
    validate_diagnostic_configuration(configuration)?;
    let mut workloads = vec![
        synthetic_legacy_manifest()
            .map_err(|_| "synthetic diagnostic workload is invalid".to_owned())?,
        modeled_post_foundation_manifest()
            .map_err(|_| "modeled diagnostic workload is invalid".to_owned())?,
    ];
    if let Some(path) = configuration.external_corpus_root.as_deref() {
        workloads.push(
            load_external_workload_v2_manifest_from_path(Some(path))
                .map_err(|_| "external diagnostic workload is invalid or has drifted".to_owned())?,
        );
    }

    fs::create_dir(&configuration.root)
        .map_err(|_| "performance diagnostics root creation failed".to_owned())?;
    let filesystem = qualification_filesystem_name(&configuration.root);
    let environment = QualificationPlatformEnvironmentV1 {
        operating_system: std::env::consts::OS.to_owned(),
        architecture: std::env::consts::ARCH.to_owned(),
        filesystem: filesystem.clone(),
        filesystem_disposition: classify_qualification_filesystem(&filesystem),
        allocation_method: native_allocation_method().to_owned(),
        rustc: rustc_version(),
        build_source: env!("POINTBREAK_BUILD_SOURCE").to_owned(),
        build_describe: env!("POINTBREAK_BUILD_DESCRIBE").to_owned(),
        source_tree_dirty: env!("POINTBREAK_BUILD_DIRTY") == "true",
    };
    let mut cases = Vec::new();
    for workload in &workloads {
        for candidate in QualificationPerformanceRoleV1::CANDIDATES {
            let case_root = configuration.root.join(format!(
                "{}-{}",
                candidate.as_str(),
                &workload.manifest_sha256[..16]
            ));
            fs::create_dir(&case_root)
                .map_err(|_| "performance diagnostic case root creation failed".to_owned())?;
            cases.push(run_diagnostic_case(
                configuration,
                candidate,
                workload,
                &case_root,
            )?);
        }
    }
    let mut report = QualificationPerformanceDiagnosticsReportV1 {
        schema: QUALIFICATION_PERFORMANCE_DIAGNOSTICS_SCHEMA_V1.to_owned(),
        contract_schema: QUALIFICATION_PERFORMANCE_DIAGNOSTIC_CONTRACT_SCHEMA_V1.to_owned(),
        contract_sha256: diagnostic_contract_sha256(),
        source_commit: configuration.source_commit.clone(),
        cargo_lock_sha256: configuration.cargo_lock_sha256.clone(),
        environment,
        warmup_samples: configuration.warmup_samples,
        measured_samples: configuration.measured_samples,
        pair_order: configuration.pair_order,
        cases,
        report_sha256: String::new(),
    };
    report.report_sha256 = report.canonical_sha256()?;
    report.validate()?;
    Ok(report)
}

enum DiagnosticCandidateProfile {
    Sqlite {
        profile: SqliteQualificationProfile,
        #[cfg(test)]
        root: PathBuf,
    },
    Segments {
        profile: SegmentQualificationProfile,
        #[cfg(test)]
        root: PathBuf,
    },
}

impl DiagnosticCandidateProfile {
    fn open(role: QualificationPerformanceRoleV1, root: &Path) -> Result<Self, String> {
        match role {
            QualificationPerformanceRoleV1::SqliteWal => SqliteQualificationProfile::open(root)
                .map(|profile| Self::Sqlite {
                    profile,
                    #[cfg(test)]
                    root: root.to_path_buf(),
                })
                .map_err(|_| "SQLite diagnostic profile open failed".to_owned()),
            QualificationPerformanceRoleV1::BoundedSegments => {
                SegmentQualificationProfile::open(root)
                    .map(|profile| Self::Segments {
                        profile,
                        #[cfg(test)]
                        root: root.to_path_buf(),
                    })
                    .map_err(|_| "segment diagnostic profile open failed".to_owned())
            }
            QualificationPerformanceRoleV1::LooseBaseline => {
                Err("loose baseline is not a candidate profile".to_owned())
            }
        }
    }

    fn as_profile(&self) -> &dyn QualificationProfile {
        match self {
            Self::Sqlite { profile, .. } => profile,
            Self::Segments { profile, .. } => profile,
        }
    }

    fn as_probe(&self) -> &dyn QualificationPerformanceProbe {
        match self {
            Self::Sqlite { profile, .. } => profile,
            Self::Segments { profile, .. } => profile,
        }
    }

    fn semantic_receipt(&self) -> Result<QualificationPerformanceSemanticReceiptV2, String> {
        qualification_journal_receipt(self.as_profile().journal().list()?)
    }

    fn maintenance_boundary(&self) -> Result<(), String> {
        match self {
            Self::Sqlite { profile, .. } => profile
                .checkpoint()
                .map(|_| ())
                .map_err(|error| error.to_string()),
            Self::Segments { profile, .. } => profile
                .seal_active()
                .map(|_| ())
                .map_err(|error| error.to_string()),
        }
    }

    #[cfg(test)]
    fn run_normal_operation(
        &self,
        request: &QualificationPerformanceOperationRequestV1<'_>,
    ) -> Result<(), String> {
        match request.operation {
            QualificationPerformanceOperationV1::DurableAppend => {
                if self
                    .as_profile()
                    .journal()
                    .create_once(request.logical_key, request.decoded_bytes)?
                    != super::QualificationCreateOutcome::Created
                {
                    return Err("normal append did not create a fresh record".to_owned());
                }
            }
            QualificationPerformanceOperationV1::StrictReplay => {
                std::hint::black_box(self.as_profile().journal().list()?);
            }
            QualificationPerformanceOperationV1::KeyedRead => {
                let entry = self
                    .as_profile()
                    .journal()
                    .read(request.logical_key)?
                    .ok_or_else(|| "normal keyed read omitted a record".to_owned())?;
                if entry.decoded_bytes != request.decoded_bytes {
                    return Err("normal keyed read returned different bytes".to_owned());
                }
            }
            QualificationPerformanceOperationV1::OpenRecovery => match self {
                Self::Sqlite { root, .. } => {
                    let reopened = SqliteQualificationProfile::open(root)
                        .map_err(|_| "normal SQLite reopen failed".to_owned())?;
                    reopened.journal().integrity_check()?;
                }
                Self::Segments { root, .. } => {
                    let reopened = SegmentQualificationProfile::open(root)
                        .map_err(|_| "normal segment reopen failed".to_owned())?;
                    reopened.journal().integrity_check()?;
                }
            },
        }
        Ok(())
    }
}

fn run_diagnostic_case(
    configuration: &QualificationPerformanceDiagnosticConfigurationV1,
    role: QualificationPerformanceRoleV1,
    workload: &QualificationCorpusManifestV1,
    root: &Path,
) -> Result<QualificationPerformanceDiagnosticCaseV1, String> {
    let candidate_root = root.join("candidate");
    let loose_root = root.join("loose");
    let candidate = DiagnosticCandidateProfile::open(role, &candidate_root)?;
    populate_profile(candidate.as_profile(), workload)
        .map_err(|_| "diagnostic candidate population failed".to_owned())?;
    let loose = LooseQualificationPerformanceProbe::create(loose_root, workload)?;
    let selected = workload
        .records
        .iter()
        .find(|record| {
            matches!(
                record.record_kind,
                QualificationRecordKindV1::LegacyEvent
                    | QualificationRecordKindV1::GenerationProposal
                    | QualificationRecordKindV1::RelationAttestation
                    | QualificationRecordKindV1::FactPort
            )
        })
        .ok_or_else(|| "diagnostic workload has no journal record".to_owned())?;

    for iteration in 0..configuration.warmup_samples {
        run_diagnostic_iteration(
            &candidate,
            &loose,
            role,
            selected,
            configuration.pair_order,
            iteration,
            "warmup",
        )?;
    }
    let mut samples = Vec::new();
    for iteration in 0..configuration.measured_samples {
        samples.extend(run_diagnostic_iteration(
            &candidate,
            &loose,
            role,
            selected,
            configuration.pair_order,
            iteration,
            "measured",
        )?);
    }

    let steady_candidate = candidate.as_profile().inventory()?;
    let steady_loose = loose.inventory()?;
    let reopened = DiagnosticCandidateProfile::open(role, &candidate_root)?;
    let reopened_candidate = reopened.as_profile().inventory()?;
    let reopened_loose = loose.inventory()?;
    let mut high_water_candidate = reopened_candidate.clone();
    high_water_candidate.high_water_bytes = high_water_candidate
        .high_water_bytes
        .max(steady_candidate.high_water_bytes);
    let mut high_water_loose = reopened_loose.clone();
    high_water_loose.high_water_bytes = high_water_loose
        .high_water_bytes
        .max(steady_loose.high_water_bytes);
    let inventories = [
        (
            role,
            QualificationPerformanceInventoryStateV1::Steady,
            steady_candidate,
        ),
        (
            QualificationPerformanceRoleV1::LooseBaseline,
            QualificationPerformanceInventoryStateV1::Steady,
            steady_loose,
        ),
        (
            role,
            QualificationPerformanceInventoryStateV1::Reopened,
            reopened_candidate,
        ),
        (
            QualificationPerformanceRoleV1::LooseBaseline,
            QualificationPerformanceInventoryStateV1::Reopened,
            reopened_loose,
        ),
        (
            role,
            QualificationPerformanceInventoryStateV1::HighWater,
            high_water_candidate,
        ),
        (
            QualificationPerformanceRoleV1::LooseBaseline,
            QualificationPerformanceInventoryStateV1::HighWater,
            high_water_loose,
        ),
    ]
    .into_iter()
    .map(
        |(role, state, inventory)| QualificationPerformanceInventorySnapshotV1 {
            role,
            state,
            inventory,
        },
    )
    .collect();

    let candidate_identity = match role {
        QualificationPerformanceRoleV1::SqliteWal => QualificationCandidateV1::SqliteWal,
        QualificationPerformanceRoleV1::BoundedSegments => {
            QualificationCandidateV1::BoundedSegments
        }
        QualificationPerformanceRoleV1::LooseBaseline => unreachable!(),
    };
    let physical_profile_id = match role {
        QualificationPerformanceRoleV1::SqliteWal => SQLITE_QUALIFICATION_PROFILE_ID_V1,
        QualificationPerformanceRoleV1::BoundedSegments => SEGMENT_QUALIFICATION_PROFILE_ID_V1,
        QualificationPerformanceRoleV1::LooseBaseline => unreachable!(),
    };
    Ok(QualificationPerformanceDiagnosticCaseV1 {
        candidate: role,
        candidate_build_id: candidate_identity.build_id(&configuration.cargo_lock_sha256),
        physical_profile_id: physical_profile_id.to_owned(),
        workload_manifest_sha256: workload.manifest_sha256.clone(),
        samples,
        inventories,
    })
}

fn run_diagnostic_iteration(
    candidate: &DiagnosticCandidateProfile,
    loose: &LooseQualificationPerformanceProbe,
    candidate_role: QualificationPerformanceRoleV1,
    selected: &super::QualificationRecordV1,
    order: QualificationPerformancePairOrderV1,
    iteration: u32,
    series: &str,
) -> Result<Vec<QualificationPerformanceDiagnosticSampleV1>, String> {
    let mut samples = Vec::with_capacity(QualificationPerformanceOperationV1::ALL.len() * 2);
    for operation in QualificationPerformanceOperationV1::ALL {
        let append_key = format!("diagnostics/{series}/{iteration:08}");
        let logical_key = if operation == QualificationPerformanceOperationV1::DurableAppend {
            append_key.as_str()
        } else {
            selected.logical_key.as_str()
        };
        let pair_order = match paired_roles(order, candidate_role, iteration)[0] {
            QualificationPerformanceRoleV1::LooseBaseline => 1,
            _ => 0,
        };
        let candidate_request = QualificationPerformanceOperationRequestV1 {
            operation,
            role: candidate_role,
            iteration,
            pair_order,
            logical_key,
            decoded_bytes: &selected.decoded_bytes,
        };
        let baseline_request = QualificationPerformanceOperationRequestV1 {
            role: QualificationPerformanceRoleV1::LooseBaseline,
            ..candidate_request.clone()
        };
        validate_equivalent_pair(&candidate_request, &baseline_request)?;
        for role in paired_roles(order, candidate_role, iteration) {
            let sample = if role == QualificationPerformanceRoleV1::LooseBaseline {
                loose.run_profiled_operation(&baseline_request)
            } else {
                candidate
                    .as_probe()
                    .run_profiled_operation(&candidate_request)
            }?;
            samples.push(sample);
        }
        if operation == QualificationPerformanceOperationV1::DurableAppend {
            let candidate_bytes = candidate
                .as_profile()
                .journal()
                .read(logical_key)
                .map_err(|_| "candidate append verification failed".to_owned())?
                .ok_or_else(|| "candidate append verification omitted a record".to_owned())?
                .decoded_bytes;
            if candidate_bytes != selected.decoded_bytes {
                return Err("candidate append verification returned different bytes".to_owned());
            }
            loose.verify_read(&baseline_request)?;
        }
    }
    Ok(samples)
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "loose baseline file creation failed".to_owned())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "loose baseline durable write failed".to_owned())
}

fn measure_string_stage<T>(
    recorder: &mut Option<&mut QualificationPerformanceStageRecorder>,
    stage: &str,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    match recorder.as_deref_mut() {
        Some(recorder) => recorder.measure(stage, operation),
        None => operation(),
    }
}

fn rustc_version() -> String {
    Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|version| version.trim().to_owned())
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| "unavailable".to_owned())
}

fn rustc_verbose_version() -> String {
    command_stdout("rustc", &["-vV"]).unwrap_or_else(|| "unavailable\nhost: unavailable".to_owned())
}

fn command_stdout(program: &str, arguments: &[&str]) -> Option<String> {
    Command::new(program)
        .args(arguments)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(target_os = "macos")]
fn native_operating_system_version() -> String {
    command_stdout("sw_vers", &["-productVersion"]).unwrap_or_else(|| "unavailable".to_owned())
}

#[cfg(target_os = "linux")]
fn native_operating_system_version() -> String {
    command_stdout("uname", &["-sr"]).unwrap_or_else(|| "unavailable".to_owned())
}

#[cfg(target_os = "windows")]
fn native_operating_system_version() -> String {
    command_stdout("cmd", &["/C", "ver"]).unwrap_or_else(|| "unavailable".to_owned())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn native_operating_system_version() -> String {
    "unavailable".to_owned()
}

#[cfg(target_os = "macos")]
fn native_cpu_description() -> String {
    command_stdout("sysctl", &["-n", "machdep.cpu.brand_string"])
        .or_else(|| command_stdout("sysctl", &["-n", "hw.model"]))
        .unwrap_or_else(|| "unavailable".to_owned())
}

#[cfg(target_os = "linux")]
fn native_cpu_description() -> String {
    let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let device_tree_model =
        fs::read_to_string("/sys/firmware/devicetree/base/model").unwrap_or_default();
    let dmi_product_name =
        fs::read_to_string("/sys/devices/virtual/dmi/id/product_name").unwrap_or_default();
    linux_cpu_description_from_sources_v1(
        &cpuinfo,
        &[device_tree_model.as_str(), dmi_product_name.as_str()],
    )
    .unwrap_or_else(|| "unavailable".to_owned())
}

#[cfg(any(target_os = "linux", test))]
fn linux_cpu_description_from_sources_v1(
    cpuinfo: &str,
    fallback_hardware_models: &[&str],
) -> Option<String> {
    cpuinfo
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(key, _)| matches!(key.trim(), "model name" | "Model"))
                .and_then(|(_, value)| normalized_hardware_description_v1(value))
        })
        .or_else(|| {
            fallback_hardware_models
                .iter()
                .find_map(|value| normalized_hardware_description_v1(value))
        })
}

#[cfg(any(target_os = "linux", test))]
fn normalized_hardware_description_v1(value: &str) -> Option<String> {
    let value =
        value.trim_matches(|character: char| character.is_whitespace() || character == '\0');
    concrete_platform_value_v1(value).then(|| value.to_owned())
}

#[cfg(target_os = "windows")]
fn native_cpu_description() -> String {
    std::env::var("PROCESSOR_IDENTIFIER")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "unavailable".to_owned())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn native_cpu_description() -> String {
    "unavailable".to_owned()
}

#[cfg(unix)]
fn final_native_allocation_method() -> &'static str {
    "stat_blocks_512"
}

#[cfg(windows)]
fn final_native_allocation_method() -> &'static str {
    "file_standard_info_allocation_size"
}

#[cfg(not(any(unix, windows)))]
fn final_native_allocation_method() -> &'static str {
    "logical_length_fallback"
}

#[cfg(unix)]
fn native_allocation_method() -> &'static str {
    "stat_blocks_512"
}

#[cfg(windows)]
fn native_allocation_method() -> &'static str {
    "get_compressed_file_size_w"
}

#[cfg(not(any(unix, windows)))]
fn native_allocation_method() -> &'static str {
    "logical_length_fallback"
}

pub fn evaluate_qualification_performance_h8_v1(
    samples: &[QualificationRawSampleV1],
) -> Result<Option<String>, String> {
    let mut failures = Vec::new();
    for operation in QualificationPerformanceOperationV1::ALL {
        let (candidate, baseline) = operation.legacy_sample_names();
        let candidate_p95 = sample_p95(samples, candidate)
            .ok_or_else(|| format!("H8 v1 is missing required {candidate} samples"))?;
        let baseline_p95 = sample_p95(samples, baseline)
            .ok_or_else(|| format!("H8 v1 is missing required {baseline} samples"))?;
        if u128::from(candidate_p95) * 100 > u128::from(baseline_p95) * 125 {
            failures.push(format!(
                "{} p95 {candidate_p95}ns exceeds 125% of fresh loose baseline {baseline_p95}ns",
                operation.legacy_failure_label()
            ));
        }
    }
    Ok((!failures.is_empty()).then(|| failures.join("; ")))
}

fn sample_p95(samples: &[QualificationRawSampleV1], operation: &str) -> Option<u64> {
    let mut values = samples
        .iter()
        .filter(|sample| sample.operation == operation)
        .map(|sample| sample.elapsed_nanos)
        .collect::<Vec<_>>();
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let rank = values.len().saturating_mul(95).div_ceil(100).max(1);
    values.get(rank - 1).copied()
}

pub(super) fn paired_roles(
    order: QualificationPerformancePairOrderV1,
    candidate: QualificationPerformanceRoleV1,
    iteration: u32,
) -> [QualificationPerformanceRoleV1; 2] {
    let candidate_first = match order {
        QualificationPerformancePairOrderV1::CandidateThenBaseline => true,
        QualificationPerformancePairOrderV1::BaselineThenCandidate => false,
        QualificationPerformancePairOrderV1::Alternating => iteration.is_multiple_of(2),
    };
    if candidate_first {
        [candidate, QualificationPerformanceRoleV1::LooseBaseline]
    } else {
        [QualificationPerformanceRoleV1::LooseBaseline, candidate]
    }
}

pub(super) fn validate_equivalent_pair(
    candidate: &QualificationPerformanceOperationRequestV1<'_>,
    baseline: &QualificationPerformanceOperationRequestV1<'_>,
) -> Result<(), String> {
    if candidate.operation != baseline.operation
        || candidate.iteration != baseline.iteration
        || candidate.pair_order != baseline.pair_order
        || candidate.logical_key != baseline.logical_key
        || candidate.decoded_bytes != baseline.decoded_bytes
        || candidate.role == QualificationPerformanceRoleV1::LooseBaseline
        || baseline.role != QualificationPerformanceRoleV1::LooseBaseline
    {
        return Err("paired performance operations are not equivalent".to_owned());
    }
    Ok(())
}

fn validate_hex(value: &str, length: usize, label: &str) -> Result<(), String> {
    if value.len() != length || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "{label} must be exactly {length} hexadecimal characters"
        ));
    }
    Ok(())
}

fn elapsed_nanos(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos())
        .unwrap_or(u64::MAX)
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench_support::foundation::{
        QualificationFilesystemDispositionV1, QualificationInventoryV1,
        QualificationPlatformEnvironmentV1, QualificationRawSampleV1,
    };

    const SOURCE_COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const LOCK_SHA256: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn raw(operation: &str, values: &[u64]) -> Vec<QualificationRawSampleV1> {
        values
            .iter()
            .enumerate()
            .map(|(iteration, elapsed_nanos)| QualificationRawSampleV1 {
                operation: operation.to_owned(),
                iteration: iteration as u32,
                elapsed_nanos: *elapsed_nanos,
            })
            .collect()
    }

    fn complete_h8_samples(
        candidate_append: &[u64],
        baseline_append: &[u64],
    ) -> Vec<QualificationRawSampleV1> {
        let mut samples = Vec::new();
        samples.extend(raw("candidate_durable_append", candidate_append));
        samples.extend(raw("baseline_durable_append", baseline_append));
        for (candidate, baseline) in [
            ("candidate_replay", "baseline_replay"),
            ("candidate_keyed_read", "baseline_keyed_read"),
            ("candidate_open_recovery", "baseline_open_recovery"),
        ] {
            samples.extend(raw(candidate, &[100, 100, 100, 100, 100]));
            samples.extend(raw(baseline, &[100, 100, 100, 100, 100]));
        }
        samples
    }

    fn inventory() -> QualificationInventoryV1 {
        QualificationInventoryV1 {
            carriers: vec!["carrier".to_owned()],
            logical_bytes: 1,
            encoded_bytes: 1,
            allocated_bytes: 1,
            high_water_bytes: 1,
        }
    }

    fn environment() -> QualificationPlatformEnvironmentV1 {
        QualificationPlatformEnvironmentV1 {
            operating_system: "test".to_owned(),
            architecture: "test".to_owned(),
            filesystem: "apfs".to_owned(),
            filesystem_disposition: QualificationFilesystemDispositionV1::LocalProofEligible,
            allocation_method: "fixture".to_owned(),
            rustc: "rustc test".to_owned(),
            build_source: "git".to_owned(),
            build_describe: "fixture".to_owned(),
            source_tree_dirty: false,
        }
    }

    #[test]
    fn h8_v1_preserves_the_125_percent_boundary_and_fails_closed() {
        let equal_boundary = complete_h8_samples(&[125; 5], &[100; 5]);
        let equal_verdict = evaluate_qualification_performance_h8_v1(&equal_boundary);
        assert_eq!(equal_verdict, Ok(None));

        for (value, expected_sha256) in [
            (
                serde_json::to_value(&equal_boundary).expect("H8 v1 evidence value"),
                "4fa036f296ac8726339b1e7708dd91f19ef26058ae7fae039c2d41d05ef9e12d",
            ),
            (
                serde_json::to_value(&equal_verdict).expect("H8 v1 verdict value"),
                "d68bd6e1a809e2f79779af352f22efcd54a285cfb02c752ca6a67feb434cd59c",
            ),
        ] {
            let bytes = canonical_json_bytes(&value).expect("canonical H8 v1 bytes");
            assert_eq!(sha256_bytes_hex(&bytes), expected_sha256);
        }
        assert_eq!(
            diagnostic_contract_sha256(),
            "a2bc02fd4d2d0072ee5dc6ef3d21ad460cfcf317a18d87f59a76549b08326e0b"
        );

        let above_boundary = complete_h8_samples(&[126; 5], &[100; 5]);
        assert_eq!(
            evaluate_qualification_performance_h8_v1(&above_boundary),
            Ok(Some(
                "durable_append p95 126ns exceeds 125% of fresh loose baseline 100ns".to_owned()
            ))
        );

        let incomplete = raw("candidate_durable_append", &[100; 5]);
        assert!(evaluate_qualification_performance_h8_v1(&incomplete).is_err());
    }

    #[test]
    fn frozen_native_h8_rows_preserve_their_verdicts() {
        let rows: &[(&str, [[u64; 2]; 4], &str)] = &[
            (
                "macos-sqlite-small",
                [
                    [8_287_708, 5_256_958],
                    [29_917, 173_125],
                    [10_708, 14_000],
                    [1_659_209, 153_875],
                ],
                "durable_append p95 8287708ns exceeds 125% of fresh loose baseline 5256958ns; open_recovery p95 1659209ns exceeds 125% of fresh loose baseline 153875ns",
            ),
            (
                "macos-sqlite-modeled",
                [
                    [10_354_917, 9_362_750],
                    [81_167, 726_084],
                    [17_083, 16_584],
                    [2_030_458, 651_000],
                ],
                "open_recovery p95 2030458ns exceeds 125% of fresh loose baseline 651000ns",
            ),
            (
                "macos-segments-small",
                [
                    [31_818_375, 5_454_458],
                    [334_208, 899_959],
                    [266_416, 74_625],
                    [2_814_333, 626_167],
                ],
                "durable_append p95 31818375ns exceeds 125% of fresh loose baseline 5454458ns; keyed_read p95 266416ns exceeds 125% of fresh loose baseline 74625ns; open_recovery p95 2814333ns exceeds 125% of fresh loose baseline 626167ns",
            ),
            (
                "macos-segments-modeled",
                [
                    [25_242_208, 5_128_917],
                    [646_834, 2_317_125],
                    [453_000, 49_375],
                    [1_940_834, 1_153_084],
                ],
                "durable_append p95 25242208ns exceeds 125% of fresh loose baseline 5128917ns; keyed_read p95 453000ns exceeds 125% of fresh loose baseline 49375ns; open_recovery p95 1940834ns exceeds 125% of fresh loose baseline 1153084ns",
            ),
            (
                "linux-sqlite-small",
                [
                    [1_023_958, 1_782_584],
                    [36_791, 106_125],
                    [49_125, 3_542],
                    [11_167_708, 32_459],
                ],
                "keyed_read p95 49125ns exceeds 125% of fresh loose baseline 3542ns; open_recovery p95 11167708ns exceeds 125% of fresh loose baseline 32459ns",
            ),
            (
                "linux-sqlite-modeled",
                [
                    [949_917, 1_866_375],
                    [68_083, 94_625],
                    [13_750, 3_250],
                    [11_404_375, 78_333],
                ],
                "keyed_read p95 13750ns exceeds 125% of fresh loose baseline 3250ns; open_recovery p95 11404375ns exceeds 125% of fresh loose baseline 78333ns",
            ),
            (
                "linux-segments-small",
                [
                    [4_720_291, 453_834],
                    [42_625, 37_625],
                    [27_334, 2_500],
                    [677_917, 34_500],
                ],
                "durable_append p95 4720291ns exceeds 125% of fresh loose baseline 453834ns; keyed_read p95 27334ns exceeds 125% of fresh loose baseline 2500ns; open_recovery p95 677917ns exceeds 125% of fresh loose baseline 34500ns",
            ),
            (
                "linux-segments-modeled",
                [
                    [4_762_958, 1_826_250],
                    [86_500, 92_958],
                    [75_416, 2_750],
                    [630_791, 70_833],
                ],
                "durable_append p95 4762958ns exceeds 125% of fresh loose baseline 1826250ns; keyed_read p95 75416ns exceeds 125% of fresh loose baseline 2750ns; open_recovery p95 630791ns exceeds 125% of fresh loose baseline 70833ns",
            ),
            (
                "windows-sqlite-small",
                [
                    [7_259_125, 7_535_417],
                    [173_958, 2_939_208],
                    [414_625, 420_208],
                    [45_601_500, 1_035_583],
                ],
                "open_recovery p95 45601500ns exceeds 125% of fresh loose baseline 1035583ns",
            ),
            (
                "windows-sqlite-modeled",
                [
                    [13_515_542, 7_252_041],
                    [533_875, 4_632_125],
                    [92_666, 141_166],
                    [53_205_958, 2_990_042],
                ],
                "durable_append p95 13515542ns exceeds 125% of fresh loose baseline 7252041ns; open_recovery p95 53205958ns exceeds 125% of fresh loose baseline 2990042ns",
            ),
            (
                "windows-segments-small",
                [
                    [74_651_167, 15_579_667],
                    [945_042, 3_927_250],
                    [1_839_292, 122_375],
                    [4_854_458, 951_791],
                ],
                "durable_append p95 74651167ns exceeds 125% of fresh loose baseline 15579667ns; keyed_read p95 1839292ns exceeds 125% of fresh loose baseline 122375ns; open_recovery p95 4854458ns exceeds 125% of fresh loose baseline 951791ns",
            ),
            (
                "windows-segments-modeled",
                [
                    [12_241_625, 652_000],
                    [621_375, 2_106_375],
                    [221_083, 64_625],
                    [4_688_708, 1_263_459],
                ],
                "durable_append p95 12241625ns exceeds 125% of fresh loose baseline 652000ns; keyed_read p95 221083ns exceeds 125% of fresh loose baseline 64625ns; open_recovery p95 4688708ns exceeds 125% of fresh loose baseline 1263459ns",
            ),
        ];

        for (name, pairs, expected) in rows {
            let mut samples = Vec::new();
            for (operation, [candidate, baseline]) in QualificationPerformanceOperationV1::ALL
                .into_iter()
                .zip(pairs)
            {
                let (candidate_name, baseline_name) = operation.legacy_sample_names();
                samples.extend(raw(candidate_name, &[*candidate; 5]));
                samples.extend(raw(baseline_name, &[*baseline; 5]));
            }
            assert_eq!(
                evaluate_qualification_performance_h8_v1(&samples),
                Ok(Some((*expected).to_owned())),
                "{name}"
            );
        }
    }

    #[test]
    fn diagnostic_report_is_provenance_complete_and_request_data_is_not_serialized() {
        let mut report = QualificationPerformanceDiagnosticsReportV1 {
            schema: QUALIFICATION_PERFORMANCE_DIAGNOSTICS_SCHEMA_V1.to_owned(),
            contract_schema: QUALIFICATION_PERFORMANCE_DIAGNOSTIC_CONTRACT_SCHEMA_V1.to_owned(),
            contract_sha256: diagnostic_contract_sha256(),
            source_commit: SOURCE_COMMIT.to_owned(),
            cargo_lock_sha256: LOCK_SHA256.to_owned(),
            environment: environment(),
            warmup_samples: 1,
            measured_samples: 1,
            pair_order: QualificationPerformancePairOrderV1::Alternating,
            cases: vec![QualificationPerformanceDiagnosticCaseV1 {
                candidate: QualificationPerformanceRoleV1::SqliteWal,
                candidate_build_id: "sqlite-build".to_owned(),
                physical_profile_id: "sqlite-profile".to_owned(),
                workload_manifest_sha256: LOCK_SHA256.to_owned(),
                samples: QualificationPerformanceOperationV1::ALL
                    .into_iter()
                    .flat_map(|operation| {
                        [
                            QualificationPerformanceRoleV1::SqliteWal,
                            QualificationPerformanceRoleV1::LooseBaseline,
                        ]
                        .into_iter()
                        .map(move |role| {
                            QualificationPerformanceDiagnosticSampleV1 {
                                operation,
                                role,
                                iteration: 0,
                                pair_order: 0,
                                total_elapsed_nanos: 2,
                                stages: vec![QualificationPerformanceStageSampleV1 {
                                    stage: "durable_work".to_owned(),
                                    elapsed_nanos: 1,
                                }],
                            }
                        })
                    })
                    .collect(),
                inventories: [
                    QualificationPerformanceRoleV1::SqliteWal,
                    QualificationPerformanceRoleV1::LooseBaseline,
                ]
                .into_iter()
                .flat_map(|role| {
                    [
                        QualificationPerformanceInventoryStateV1::Steady,
                        QualificationPerformanceInventoryStateV1::Reopened,
                        QualificationPerformanceInventoryStateV1::HighWater,
                    ]
                    .into_iter()
                    .map(move |state| {
                        QualificationPerformanceInventorySnapshotV1 {
                            role,
                            state,
                            inventory: inventory(),
                        }
                    })
                })
                .collect(),
            }],
            report_sha256: String::new(),
        };
        let mut segments = report.cases[0].clone();
        segments.candidate = QualificationPerformanceRoleV1::BoundedSegments;
        segments.candidate_build_id = "segment-build".to_owned();
        segments.physical_profile_id = "segment-profile".to_owned();
        for sample in &mut segments.samples {
            if sample.role == QualificationPerformanceRoleV1::SqliteWal {
                sample.role = QualificationPerformanceRoleV1::BoundedSegments;
            }
        }
        for snapshot in &mut segments.inventories {
            if snapshot.role == QualificationPerformanceRoleV1::SqliteWal {
                snapshot.role = QualificationPerformanceRoleV1::BoundedSegments;
            }
        }
        report.cases.push(segments);
        report.report_sha256 = report.canonical_sha256().expect("report hash");
        let serialized = serde_json::to_string(&report).expect("diagnostic JSON");

        assert!(report.validate().is_ok());
        assert!(!serialized.contains("logicalKey"));
        assert!(!serialized.contains("decodedBytes"));
        assert!(!serialized.contains("externalCorpus"));
    }

    #[test]
    fn pair_order_is_deterministic_and_mismatched_requests_fail_before_execution() {
        assert_eq!(
            paired_roles(
                QualificationPerformancePairOrderV1::Alternating,
                QualificationPerformanceRoleV1::SqliteWal,
                0,
            ),
            [
                QualificationPerformanceRoleV1::SqliteWal,
                QualificationPerformanceRoleV1::LooseBaseline,
            ]
        );
        assert_eq!(
            paired_roles(
                QualificationPerformancePairOrderV1::Alternating,
                QualificationPerformanceRoleV1::SqliteWal,
                1,
            ),
            [
                QualificationPerformanceRoleV1::LooseBaseline,
                QualificationPerformanceRoleV1::SqliteWal,
            ]
        );

        let candidate = QualificationPerformanceOperationRequestV1 {
            operation: QualificationPerformanceOperationV1::DurableAppend,
            role: QualificationPerformanceRoleV1::SqliteWal,
            iteration: 0,
            pair_order: 0,
            logical_key: "same-key",
            decoded_bytes: b"same-bytes",
        };
        let baseline = QualificationPerformanceOperationRequestV1 {
            decoded_bytes: b"different-bytes",
            role: QualificationPerformanceRoleV1::LooseBaseline,
            ..candidate.clone()
        };
        assert!(validate_equivalent_pair(&candidate, &baseline).is_err());
    }

    #[test]
    fn invalid_configuration_is_rejected_before_creating_the_output_root() {
        let parent = tempfile::tempdir().expect("configuration parent");
        let root = parent.path().join("diagnostics");
        let mut configuration = QualificationPerformanceDiagnosticConfigurationV1 {
            executable: std::env::current_exe().expect("test executable"),
            root: root.clone(),
            source_commit: crate::bench_support::foundation::qualification_source_commit()
                .expect("build commit"),
            cargo_lock_sha256: crate::bench_support::foundation::qualification_cargo_lock_sha256(),
            warmup_samples: 0,
            measured_samples: 1,
            pair_order: QualificationPerformancePairOrderV1::Alternating,
            external_corpus_root: None,
        };

        assert!(validate_diagnostic_configuration(&configuration).is_err());
        assert!(!root.exists());

        configuration.warmup_samples = 1;
        configuration.source_commit = SOURCE_COMMIT.to_owned();
        assert!(validate_diagnostic_configuration(&configuration).is_err());
        assert!(!root.exists());

        configuration.source_commit =
            crate::bench_support::foundation::qualification_source_commit().expect("build commit");
        std::fs::create_dir(&root).expect("pre-existing root");
        assert!(validate_diagnostic_configuration(&configuration).is_err());
    }

    #[test]
    fn stage_samples_are_positive_non_overlapping_and_sanitized() {
        let mut recorder = QualificationPerformanceStageRecorder::default();
        let secret = "request-secret-marker";
        let value = recorder
            .measure("semantic_validation", || Ok::<_, String>(secret.len()))
            .expect("profiled work");
        let total = recorder.elapsed_nanos();
        let stages = recorder.finish(total).expect("valid stages");
        let serialized = serde_json::to_string(&stages).expect("stage JSON");

        assert_eq!(value, secret.len());
        assert!(stages.iter().all(|stage| stage.elapsed_nanos > 0));
        assert!(stages.iter().map(|stage| stage.elapsed_nanos).sum::<u64>() <= total);
        assert!(!serialized.contains(secret));
    }

    #[test]
    fn normal_and_profiled_operations_leave_equivalent_state() {
        let workload = synthetic_legacy_manifest().expect("synthetic workload");
        let selected = workload
            .records
            .iter()
            .find(|record| record.record_kind == QualificationRecordKindV1::LegacyEvent)
            .expect("journal record");
        let roots = tempfile::tempdir().expect("equivalence roots");

        for role in QualificationPerformanceRoleV1::CANDIDATES {
            let normal = DiagnosticCandidateProfile::open(
                role,
                &roots.path().join(format!("{}-normal", role.as_str())),
            )
            .expect("normal candidate");
            let profiled = DiagnosticCandidateProfile::open(
                role,
                &roots.path().join(format!("{}-profiled", role.as_str())),
            )
            .expect("profiled candidate");
            populate_profile(normal.as_profile(), &workload).expect("normal population");
            populate_profile(profiled.as_profile(), &workload).expect("profiled population");

            for operation in QualificationPerformanceOperationV1::ALL {
                let append_key = format!("equivalence/{}/append", role.as_str());
                let request = QualificationPerformanceOperationRequestV1 {
                    operation,
                    role,
                    iteration: 0,
                    pair_order: 0,
                    logical_key: if operation == QualificationPerformanceOperationV1::DurableAppend
                    {
                        &append_key
                    } else {
                        &selected.logical_key
                    },
                    decoded_bytes: &selected.decoded_bytes,
                };
                normal
                    .run_normal_operation(&request)
                    .expect("normal operation");
                profiled
                    .as_probe()
                    .run_profiled_operation(&request)
                    .expect("profiled operation");

                assert_eq!(
                    normal.as_profile().journal().list().expect("normal list"),
                    profiled
                        .as_profile()
                        .journal()
                        .list()
                        .expect("profiled list")
                );
                assert_eq!(
                    normal
                        .as_profile()
                        .journal()
                        .head_marker()
                        .expect("normal head"),
                    profiled
                        .as_profile()
                        .journal()
                        .head_marker()
                        .expect("profiled head")
                );
                let normal_inventory = normal.as_profile().inventory().expect("normal inventory");
                let profiled_inventory = profiled
                    .as_profile()
                    .inventory()
                    .expect("profiled inventory");
                assert_eq!(normal_inventory.carriers, profiled_inventory.carriers);
                assert_eq!(
                    normal_inventory.logical_bytes,
                    profiled_inventory.logical_bytes
                );
                assert_eq!(
                    normal_inventory.encoded_bytes,
                    profiled_inventory.encoded_bytes
                );
                // Sparse-file native allocation can differ between otherwise identical roots,
                // especially while unrelated filesystem-heavy tests run in parallel. Preserve
                // both complete observations, but compare the deterministic inventory state.
                for inventory in [normal_inventory, profiled_inventory] {
                    assert!(inventory.high_water_bytes >= inventory.allocated_bytes);
                    assert!(inventory.high_water_bytes >= inventory.encoded_bytes);
                }
            }

            let marker = b"unique-secret-marker";
            let conflict_key = format!("equivalence/{}/append", role.as_str());
            let conflicting = QualificationPerformanceOperationRequestV1 {
                operation: QualificationPerformanceOperationV1::DurableAppend,
                role,
                iteration: 1,
                pair_order: 0,
                logical_key: &conflict_key,
                decoded_bytes: marker,
            };
            let head_before = profiled
                .as_profile()
                .journal()
                .head_marker()
                .expect("head before failure");
            let error = profiled
                .as_probe()
                .run_profiled_operation(&conflicting)
                .expect_err("conflicting profiled operation");
            assert!(!error.contains("equivalence/"));
            assert!(!error.contains("unique-secret-marker"));
            assert_eq!(
                profiled
                    .as_profile()
                    .journal()
                    .head_marker()
                    .expect("head after failure"),
                head_before
            );
        }

        let normal_loose = LooseQualificationPerformanceProbe::create(
            roots.path().join("loose-normal"),
            &workload,
        )
        .expect("normal loose");
        let profiled_loose = LooseQualificationPerformanceProbe::create(
            roots.path().join("loose-profiled"),
            &workload,
        )
        .expect("profiled loose");
        for operation in QualificationPerformanceOperationV1::ALL {
            let append_key = "equivalence/loose/append";
            let request = QualificationPerformanceOperationRequestV1 {
                operation,
                role: QualificationPerformanceRoleV1::LooseBaseline,
                iteration: 0,
                pair_order: 0,
                logical_key: if operation == QualificationPerformanceOperationV1::DurableAppend {
                    append_key
                } else {
                    &selected.logical_key
                },
                decoded_bytes: &selected.decoded_bytes,
            };
            normal_loose
                .run_operation(&request, None)
                .expect("normal loose operation");
            profiled_loose
                .run_profiled_operation(&request)
                .expect("profiled loose operation");
            assert_eq!(
                normal_loose.inventory().expect("normal loose inventory"),
                profiled_loose
                    .inventory()
                    .expect("profiled loose inventory")
            );
        }
    }

    #[test]
    fn public_diagnostic_run_is_complete_non_gating_and_alternates_pairs() {
        let parent = tempfile::tempdir().expect("diagnostic parent");
        let configuration = QualificationPerformanceDiagnosticConfigurationV1 {
            executable: std::env::current_exe().expect("test executable"),
            root: parent.path().join("diagnostics"),
            source_commit: crate::bench_support::foundation::qualification_source_commit()
                .expect("build commit"),
            cargo_lock_sha256: crate::bench_support::foundation::qualification_cargo_lock_sha256(),
            warmup_samples: 1,
            measured_samples: 2,
            pair_order: QualificationPerformancePairOrderV1::Alternating,
            external_corpus_root: None,
        };

        let report = run_qualification_performance_diagnostics(&configuration)
            .expect("complete public diagnostics");

        assert_eq!(report.cases.len(), 4);
        assert!(report.validate().is_ok());
        assert!(report.cases.iter().all(|case| case.samples.len() == 16));
        assert!(report.cases.iter().all(|case| {
            case.samples.iter().any(|sample| sample.pair_order == 0)
                && case.samples.iter().any(|sample| sample.pair_order == 1)
        }));
    }

    fn v2_inventory(allocated_bytes: u64) -> QualificationPerformanceInventoryV2 {
        QualificationPerformanceInventoryV2 {
            carrier_count: 1,
            carrier_set_sha256: LOCK_SHA256.to_owned(),
            logical_bytes: 1,
            encoded_bytes: 1,
            allocated_bytes,
            high_water_bytes: allocated_bytes,
        }
    }

    fn complete_v2_evidence(
        candidate_nanos: u64,
        baseline_nanos: u64,
        candidate_allocated: u64,
        baseline_allocated: u64,
    ) -> QualificationPerformanceEvidenceV2 {
        let contract = QualificationPerformanceContractV2::frozen();
        let source_commit = crate::bench_support::foundation::qualification_source_commit()
            .expect("build source commit");
        let cargo_lock_sha256 = crate::bench_support::foundation::qualification_cargo_lock_sha256();
        let mut runs = Vec::new();

        for candidate in QualificationPerformanceRoleV1::CANDIDATES {
            let qualification_candidate = match candidate {
                QualificationPerformanceRoleV1::SqliteWal => QualificationCandidateV1::SqliteWal,
                QualificationPerformanceRoleV1::BoundedSegments => {
                    QualificationCandidateV1::BoundedSegments
                }
                QualificationPerformanceRoleV1::LooseBaseline => unreachable!(),
            };
            let physical_profile_id = match candidate {
                QualificationPerformanceRoleV1::SqliteWal => SQLITE_QUALIFICATION_PROFILE_ID_V1,
                QualificationPerformanceRoleV1::BoundedSegments => {
                    SEGMENT_QUALIFICATION_PROFILE_ID_V1
                }
                QualificationPerformanceRoleV1::LooseBaseline => unreachable!(),
            };

            for workload in &contract.workloads {
                for platform in &workload.platforms {
                    for run_index in 1..=contract.independent_runs {
                        let mut samples = Vec::new();
                        for operation in QualificationPerformanceOperationV1::ALL {
                            for iteration in 0..contract.measured_samples {
                                for (pair_order, role) in paired_roles(
                                    QualificationPerformancePairOrderV1::Alternating,
                                    candidate,
                                    iteration,
                                )
                                .into_iter()
                                .enumerate()
                                {
                                    let elapsed_nanos =
                                        if role == QualificationPerformanceRoleV1::LooseBaseline {
                                            baseline_nanos
                                        } else {
                                            candidate_nanos
                                        };
                                    samples.push(QualificationPerformanceDiagnosticSampleV1 {
                                        operation,
                                        role,
                                        iteration,
                                        pair_order: pair_order as u8,
                                        total_elapsed_nanos: elapsed_nanos,
                                        stages: vec![QualificationPerformanceStageSampleV1 {
                                            stage: "complete_operation".to_owned(),
                                            elapsed_nanos,
                                        }],
                                    });
                                }
                            }
                        }

                        let allocations = [
                            QualificationPerformanceAllocationScopeV2::Event,
                            QualificationPerformanceAllocationScopeV2::CompleteProfile,
                        ]
                        .into_iter()
                        .flat_map(|scope| {
                            QualificationPerformanceInventoryStateV1::ALL
                                .into_iter()
                                .flat_map(move |state| {
                                    [
                                        QualificationPerformanceAllocationSnapshotV2 {
                                            role: candidate,
                                            scope,
                                            state,
                                            inventory: v2_inventory(candidate_allocated),
                                        },
                                        QualificationPerformanceAllocationSnapshotV2 {
                                            role: QualificationPerformanceRoleV1::LooseBaseline,
                                            scope,
                                            state,
                                            inventory: v2_inventory(baseline_allocated),
                                        },
                                    ]
                                })
                        })
                        .collect();

                        runs.push(QualificationPerformanceRunV2 {
                            run_index,
                            workload: workload.workload,
                            workload_manifest_sha256: workload.manifest_sha256.clone(),
                            candidate,
                            candidate_build_id: qualification_candidate
                                .build_id(&cargo_lock_sha256),
                            physical_profile_id: physical_profile_id.to_owned(),
                            environment: QualificationPlatformEnvironmentV1 {
                                operating_system: platform.operating_system.clone(),
                                architecture: "native-test-architecture".to_owned(),
                                filesystem: platform.filesystem.clone(),
                                filesystem_disposition:
                                    QualificationFilesystemDispositionV1::LocalProofEligible,
                                allocation_method: platform.allocation_method.clone(),
                                rustc: "rustc 1.97.1\nhost: native-test-architecture".to_owned(),
                                build_source: "git".to_owned(),
                                build_describe: "fixture".to_owned(),
                                source_tree_dirty: false,
                            },
                            operating_system_version: "fixture-os-version".to_owned(),
                            cpu: "fixture-cpu".to_owned(),
                            target_architecture: "native-test-architecture".to_owned(),
                            run_identity: format!(
                                "{}-{}-{}-{}",
                                candidate.as_str(),
                                workload.workload.as_str(),
                                platform.operating_system,
                                run_index
                            ),
                            warmup_samples: contract.warmup_samples,
                            measured_samples: contract.measured_samples,
                            pair_order: contract.pair_order,
                            confidence_method:
                                QualificationPerformanceConfidenceMethodV2::IndependentRuns,
                            outlier_policy: QualificationPerformanceOutlierPolicyV2::RetainAll,
                            cache_policy: QualificationPerformanceCachePolicyV2::OsWarm,
                            controls: QualificationPerformanceRunControlsV2 {
                                fresh_roots: true,
                                quiesced_host: true,
                                native_execution: true,
                                equivalent_decoded_bytes: true,
                                monotonic_append_state: true,
                                durable_acknowledgement: true,
                                semantic_validation: true,
                                open_recovery_fresh_process: true,
                            },
                            semantic_receipt_sha256: LOCK_SHA256.to_owned(),
                            samples,
                            allocations,
                        });
                    }
                }
            }
        }

        let mut evidence = QualificationPerformanceEvidenceV2 {
            schema: QUALIFICATION_PERFORMANCE_EVIDENCE_SCHEMA_V2.to_owned(),
            contract_schema: contract.schema.clone(),
            contract_sha256: contract.canonical_sha256().expect("contract hash"),
            source_commit,
            cargo_lock_sha256,
            runs,
            evidence_sha256: String::new(),
        };
        evidence.evidence_sha256 = evidence.canonical_sha256().expect("evidence hash");
        evidence
    }

    fn rehash_v2(evidence: &mut QualificationPerformanceEvidenceV2) {
        evidence.evidence_sha256 = evidence.canonical_sha256().expect("evidence hash");
    }

    #[test]
    fn historical_h8_v2_normalized_evidence_evaluation_and_publication_bytes_are_frozen() {
        let evidence = complete_v2_evidence(100, 100, 99, 100);
        let evaluation = evaluate_qualification_performance_v2(&evidence).expect("evaluation");
        let publication = qualification_performance_contract_v2_publication();

        assert_eq!(
            evidence.evidence_sha256,
            evidence.canonical_sha256().expect("evidence hash")
        );
        // Build and dependency identities are provenance, so freeze the
        // historical H8 v2 payloads after replacing only those values.
        let mut normalized_evidence = evidence.clone();
        normalized_evidence.source_commit = SOURCE_COMMIT.to_owned();
        normalized_evidence.cargo_lock_sha256 = LOCK_SHA256.to_owned();
        for run in &mut normalized_evidence.runs {
            run.candidate_build_id = "historical-h8-v2-candidate-build".to_owned();
        }
        normalized_evidence.evidence_sha256.clear();
        let mut normalized_evaluation = evaluation.clone();
        normalized_evaluation.source_commit = SOURCE_COMMIT.to_owned();
        normalized_evaluation.cargo_lock_sha256 = LOCK_SHA256.to_owned();

        for (value, expected_sha256) in [
            (
                serde_json::to_value(&normalized_evidence).expect("evidence value"),
                "59d4a276670d75d865454b12cf3603af9a1848e626598f11ae8c73d66df33f42",
            ),
            (
                serde_json::to_value(&normalized_evaluation).expect("evaluation value"),
                "6542d24b288cb5f4b15e85e582e31a728681c087132f0546e7ca8d9aba63fcfe",
            ),
            (
                serde_json::to_value(&publication).expect("publication value"),
                "d96840d658bb83d8c4c0117d24e04fd6f4dd1e54829d6186e32f2c9c7fffbc45",
            ),
        ] {
            let bytes = canonical_json_bytes(&value).expect("canonical bytes");
            assert_eq!(sha256_bytes_hex(&bytes), expected_sha256);
        }
        let stdout = format!(
            "{}\n",
            serde_json::to_string(&publication).expect("publication JSON")
        );
        assert_eq!(
            sha256_bytes_hex(stdout.as_bytes()),
            "b369b0e913e3f320de1ce8c84566b2adbede80492702346a5262b57f15536695"
        );
        assert!(evaluation.candidates.iter().all(|candidate| {
            candidate.status == QualificationPerformanceCriterionStatusV2::Passed
        }));
    }

    #[test]
    fn final_append_records_are_valid_exact_sized_and_content_addressed() {
        let first =
            generate_qualification_append_record("measured", 0, 512).expect("first append record");
        let second =
            generate_qualification_append_record("measured", 1, 512).expect("second append record");

        assert_eq!(first.decoded_bytes.len(), 512);
        assert_eq!(second.decoded_bytes.len(), 512);
        assert_ne!(first.decoded_bytes, second.decoded_bytes);
        assert_ne!(first.logical_key, second.logical_key);
        assert!(serde_json::from_slice::<serde_json::Value>(&first.decoded_bytes).is_ok());
        assert!(
            first
                .logical_key
                .ends_with(&sha256_bytes_hex(&first.decoded_bytes))
        );
        assert!(generate_qualification_append_record("measured", 0, 8).is_err());
    }

    #[test]
    fn final_native_inventory_separates_event_and_complete_carriers() {
        let root = tempfile::tempdir().expect("inventory root");
        std::fs::create_dir(root.path().join("events")).expect("events directory");
        std::fs::create_dir(root.path().join("content")).expect("content directory");
        write_new_synced(&root.path().join("events/event.json"), b"event").expect("event carrier");
        write_new_synced(&root.path().join("content/object.json"), b"content")
            .expect("content carrier");
        write_new_synced(&root.path().join("profile.json"), b"profile").expect("profile carrier");

        let event = scoped_native_inventory(
            root.path(),
            QualificationPerformanceAllocationScopeV2::Event,
            5,
            0,
        )
        .expect("event inventory");
        let complete = scoped_native_inventory(
            root.path(),
            QualificationPerformanceAllocationScopeV2::CompleteProfile,
            12,
            0,
        )
        .expect("complete inventory");

        assert_eq!(event.carrier_count, 2);
        assert_eq!(complete.carrier_count, 3);
        assert_eq!(event.logical_bytes, 5);
        assert_eq!(complete.logical_bytes, 12);
        assert!(event.allocated_bytes < complete.allocated_bytes);
        assert_ne!(event.carrier_set_sha256, complete.carrier_set_sha256);
    }

    #[test]
    fn final_candidate_and_loose_semantic_receipts_match() {
        let workload = modeled_post_foundation_manifest().expect("modeled workload");
        let roots = tempfile::tempdir().expect("receipt roots");

        for candidate_role in QualificationPerformanceRoleV1::CANDIDATES {
            let candidate = DiagnosticCandidateProfile::open(
                candidate_role,
                &roots
                    .path()
                    .join(format!("{}-candidate", candidate_role.as_str())),
            )
            .expect("candidate profile");
            populate_profile(candidate.as_profile(), &workload).expect("candidate population");
            let loose_root = roots
                .path()
                .join(format!("{}-loose", candidate_role.as_str()));
            let _loose = LooseQualificationPerformanceProbe::create(loose_root.clone(), &workload)
                .expect("loose population");

            assert_eq!(
                candidate.semantic_receipt().expect("candidate receipt"),
                loose_semantic_receipt(&loose_root).expect("loose receipt")
            );
        }
    }

    #[test]
    fn final_performance_package_requires_every_exact_platform_shard() {
        let complete = complete_v2_evidence(100, 100, 99, 100);
        let mut shards = Vec::new();
        for operating_system in ["macos", "linux", "windows"] {
            let mut shard = complete.clone();
            shard
                .runs
                .retain(|run| run.environment.operating_system == operating_system);
            rehash_v2(&mut shard);
            shards.push(shard);
        }

        let package = QualificationPerformancePackageV2::assemble(&shards)
            .expect("complete platform package");
        assert!(package.validate().is_ok());
        assert!(package.evaluation.candidates.iter().all(|candidate| {
            candidate.criteria.iter().all(|criterion| {
                criterion.status != QualificationPerformanceCriterionStatusV2::Unknown
            })
        }));

        let error = QualificationPerformancePackageV2::assemble(&shards[..2])
            .expect_err("missing Windows shard");
        assert!(error.contains("incomplete"));

        let mut duplicate = shards.clone();
        duplicate.push(shards[0].clone());
        assert!(QualificationPerformancePackageV2::assemble(&duplicate).is_err());
    }

    #[test]
    fn final_campaign_admits_only_exact_frozen_platform_rows() {
        let contract = QualificationPerformanceContractV2::frozen();

        assert!(qualification_performance_platform_is_supported(
            &contract,
            "linux",
            "ext4",
            "stat_blocks_512"
        ));
        assert!(!qualification_performance_platform_is_supported(
            &contract,
            "linux",
            "ext2/ext3",
            "stat_blocks_512"
        ));
        assert!(!qualification_performance_platform_is_supported(
            &contract,
            "linux",
            "xfs",
            "stat_blocks_512"
        ));
    }

    #[test]
    fn h8_v2_contract_is_exact_hashable_and_generates_its_human_table() {
        let contract = QualificationPerformanceContractV2::frozen();

        assert_eq!(
            contract.schema,
            QUALIFICATION_PERFORMANCE_CONTRACT_SCHEMA_V2
        );
        assert_eq!(contract.warmup_samples, 3);
        assert_eq!(contract.measured_samples, 30);
        assert_eq!(contract.independent_runs, 2);
        assert_eq!(
            contract.pair_order,
            QualificationPerformancePairOrderV1::Alternating
        );
        assert_eq!(contract.ceiling_percent, 125);
        assert_eq!(
            contract.protocol.timing_ratio,
            "adjacent_candidate_to_baseline"
        );
        assert_eq!(contract.protocol.percentiles, [50, 95]);
        assert_eq!(contract.protocol.missing_evidence, "unknown");
        assert_eq!(contract.protocol.required_controls.len(), 8);
        assert_eq!(
            contract.canonical_sha256().expect("contract hash"),
            QUALIFICATION_PERFORMANCE_CONTRACT_SHA256_V2
        );
        assert_eq!(
            contract
                .workloads
                .iter()
                .find(|workload| {
                    workload.workload == QualificationPerformanceWorkloadV2::ExternalCorpus
                })
                .expect("external workload")
                .manifest_sha256,
            "f53ed03dbad9668f3819563dd1d7002f5cef8e6bbe07e7a89a51ae0c86a4f181"
        );
        assert!(contract.validate().is_ok());
        assert_eq!(
            contract.canonical_sha256().expect("first hash"),
            contract.canonical_sha256().expect("second hash")
        );

        let table = contract.decision_table_markdown();
        assert!(table.contains("30"));
        assert!(table.contains("125%"));
        assert!(table.contains("ext4"));
        assert!(!table.contains("49648e94"));
    }

    #[test]
    fn h8_v2_rejects_wrong_identity_profile_workload_and_platform_before_scoring() {
        let pristine = complete_v2_evidence(100, 100, 99, 100);
        let mut fixtures = Vec::new();

        let mut wrong_contract = pristine.clone();
        wrong_contract.contract_sha256 = LOCK_SHA256.to_owned();
        rehash_v2(&mut wrong_contract);
        fixtures.push(wrong_contract);

        let mut wrong_source = pristine.clone();
        wrong_source.source_commit = SOURCE_COMMIT.to_owned();
        rehash_v2(&mut wrong_source);
        fixtures.push(wrong_source);

        let mut wrong_lock = pristine.clone();
        wrong_lock.cargo_lock_sha256 = LOCK_SHA256.to_owned();
        rehash_v2(&mut wrong_lock);
        fixtures.push(wrong_lock);

        let mut wrong_profile = pristine.clone();
        wrong_profile.runs[0].physical_profile_id = "different-profile".to_owned();
        rehash_v2(&mut wrong_profile);
        fixtures.push(wrong_profile);

        let mut wrong_workload = pristine.clone();
        wrong_workload.runs[0].workload_manifest_sha256 = LOCK_SHA256.to_owned();
        rehash_v2(&mut wrong_workload);
        fixtures.push(wrong_workload);

        let mut wrong_platform = pristine;
        wrong_platform.runs[0].environment.filesystem = "overlayfs".to_owned();
        rehash_v2(&mut wrong_platform);
        fixtures.push(wrong_platform);

        for fixture in fixtures {
            assert!(evaluate_qualification_performance_v2(&fixture).is_err());
        }
    }

    #[test]
    fn h8_v2_timing_uses_paired_nearest_rank_p95_at_the_125_percent_boundary() {
        let boundary = complete_v2_evidence(125, 100, 99, 100);
        let boundary_result =
            evaluate_qualification_performance_v2(&boundary).expect("valid boundary evidence");
        assert!(boundary_result.candidates.iter().all(|candidate| {
            candidate.status == QualificationPerformanceCriterionStatusV2::Passed
        }));

        let above = complete_v2_evidence(126, 100, 99, 100);
        let above_result =
            evaluate_qualification_performance_v2(&above).expect("valid above-boundary evidence");
        assert!(above_result.candidates.iter().all(|candidate| {
            candidate.status == QualificationPerformanceCriterionStatusV2::Failed
                && candidate.criteria.iter().any(|criterion| {
                    criterion.kind == QualificationPerformanceCriterionKindV2::Timing
                        && criterion.status == QualificationPerformanceCriterionStatusV2::Failed
                })
        }));
    }

    #[test]
    fn h8_v2_reports_nearest_rank_range_and_population_deviation_without_outlier_removal() {
        let mut evidence = complete_v2_evidence(100, 100, 99, 100);
        let run = evidence
            .runs
            .iter_mut()
            .find(|run| {
                run.workload == QualificationPerformanceWorkloadV2::ExternalCorpus
                    && run.candidate == QualificationPerformanceRoleV1::SqliteWal
                    && run.run_index == 1
            })
            .expect("required run");
        for sample in run.samples.iter_mut().filter(|sample| {
            sample.operation == QualificationPerformanceOperationV1::DurableAppend
                && sample.role == QualificationPerformanceRoleV1::SqliteWal
        }) {
            sample.total_elapsed_nanos = u64::from(sample.iteration + 1);
            sample.stages[0].elapsed_nanos = sample.total_elapsed_nanos;
        }
        rehash_v2(&mut evidence);

        let result = evaluate_qualification_performance_v2(&evidence).expect("valid evidence");
        let timing = result
            .candidates
            .iter()
            .find(|candidate| candidate.candidate == QualificationPerformanceRoleV1::SqliteWal)
            .and_then(|candidate| {
                candidate.criteria.iter().find(|criterion| {
                    criterion.workload == QualificationPerformanceWorkloadV2::ExternalCorpus
                        && criterion.run_index == 1
                        && criterion.operation
                            == Some(QualificationPerformanceOperationV1::DurableAppend)
                })
            })
            .and_then(|criterion| criterion.timing.as_ref())
            .expect("timing summary");

        assert_eq!(timing.minimum_ratio_millionths, 10_000);
        assert_eq!(timing.p50_ratio_millionths, 150_000);
        assert_eq!(timing.p95_ratio_millionths, 290_000);
        assert_eq!(timing.maximum_ratio_millionths, 300_000);
        assert_eq!(timing.population_standard_deviation_millionths, 86_554);
    }

    #[test]
    fn h8_v2_one_failed_timing_prevents_eligibility_when_every_other_timing_passes() {
        let mut evidence = complete_v2_evidence(100, 100, 99, 100);
        let run = evidence
            .runs
            .iter_mut()
            .find(|run| {
                run.workload == QualificationPerformanceWorkloadV2::ExternalCorpus
                    && run.candidate == QualificationPerformanceRoleV1::SqliteWal
                    && run.run_index == 1
            })
            .expect("required run");
        for sample in run.samples.iter_mut().filter(|sample| {
            sample.operation == QualificationPerformanceOperationV1::DurableAppend
                && sample.role == QualificationPerformanceRoleV1::SqliteWal
                && sample.iteration >= 28
        }) {
            sample.total_elapsed_nanos = 126;
            sample.stages[0].elapsed_nanos = 126;
        }
        rehash_v2(&mut evidence);

        let result = evaluate_qualification_performance_v2(&evidence).expect("valid evidence");
        let sqlite = result
            .candidates
            .iter()
            .find(|candidate| candidate.candidate == QualificationPerformanceRoleV1::SqliteWal)
            .expect("SQLite result");
        assert_eq!(
            sqlite.status,
            QualificationPerformanceCriterionStatusV2::Failed
        );
        assert_eq!(
            sqlite
                .criteria
                .iter()
                .filter(|criterion| {
                    criterion.kind == QualificationPerformanceCriterionKindV2::Timing
                        && criterion.status == QualificationPerformanceCriterionStatusV2::Failed
                })
                .count(),
            1
        );
    }

    #[test]
    fn h8_v2_public_rows_publish_timing_and_allocation_diagnostics_without_gating() {
        let evidence = complete_v2_evidence(1_000, 1, 1_000, 1);
        let result = evaluate_qualification_performance_v2(&evidence).expect("valid evidence");

        for candidate in &result.candidates {
            let public = candidate
                .criteria
                .iter()
                .filter(|criterion| {
                    criterion.workload == QualificationPerformanceWorkloadV2::PublicSmoke
                })
                .collect::<Vec<_>>();
            assert!(public.iter().any(|criterion| {
                criterion.kind == QualificationPerformanceCriterionKindV2::Timing
                    && criterion.timing.is_some()
                    && criterion.status == QualificationPerformanceCriterionStatusV2::Passed
            }));
            assert!(public.iter().any(|criterion| {
                criterion.kind == QualificationPerformanceCriterionKindV2::Allocation
                    && criterion.candidate_bytes.is_some()
                    && criterion.status == QualificationPerformanceCriterionStatusV2::Passed
            }));
            assert!(candidate.criteria.iter().any(|criterion| {
                criterion.workload != QualificationPerformanceWorkloadV2::PublicSmoke
                    && criterion.status == QualificationPerformanceCriterionStatusV2::Failed
            }));
        }
    }

    #[test]
    fn h8_v2_serialized_inventory_contains_only_sanitized_counts_hashes_and_totals() {
        let inventory = QualificationInventoryV1 {
            carriers: vec!["events/private-path-sentinel.json".to_owned()],
            logical_bytes: 10,
            encoded_bytes: 20,
            allocated_bytes: 4096,
            high_water_bytes: 4096,
        };
        let sanitized = QualificationPerformanceInventoryV2::from_inventory(&inventory)
            .expect("sanitized inventory");
        let serialized = serde_json::to_string(&sanitized).expect("inventory JSON");

        assert_eq!(sanitized.carrier_count, 1);
        assert!(!serialized.contains("private-path-sentinel"));
        assert!(!serialized.contains("events/"));
        assert_eq!(sanitized.carrier_set_sha256.len(), 64);
    }

    #[test]
    fn h8_v2_checks_both_allocation_scopes_in_every_required_state() {
        for scope in [
            QualificationPerformanceAllocationScopeV2::Event,
            QualificationPerformanceAllocationScopeV2::CompleteProfile,
        ] {
            for state in QualificationPerformanceInventoryStateV1::ALL {
                let mut evidence = complete_v2_evidence(100, 100, 99, 100);
                let run = evidence
                    .runs
                    .iter_mut()
                    .find(|run| {
                        run.workload == QualificationPerformanceWorkloadV2::ExternalCorpus
                            && run.candidate == QualificationPerformanceRoleV1::SqliteWal
                            && run.run_index == 1
                    })
                    .expect("required run");
                let allocation = run
                    .allocations
                    .iter_mut()
                    .find(|allocation| {
                        allocation.role == QualificationPerformanceRoleV1::SqliteWal
                            && allocation.scope == scope
                            && allocation.state == state
                    })
                    .expect("required allocation");
                allocation.inventory.allocated_bytes = 100;
                allocation.inventory.high_water_bytes = 100;
                rehash_v2(&mut evidence);

                let result = evaluate_qualification_performance_v2(&evidence)
                    .expect("valid allocation evidence");
                let sqlite = result
                    .candidates
                    .iter()
                    .find(|candidate| {
                        candidate.candidate == QualificationPerformanceRoleV1::SqliteWal
                    })
                    .expect("SQLite result");
                assert_eq!(
                    sqlite.status,
                    QualificationPerformanceCriterionStatusV2::Failed
                );
                assert!(sqlite.criteria.iter().any(|criterion| {
                    criterion.kind == QualificationPerformanceCriterionKindV2::Allocation
                        && criterion.allocation_scope == Some(scope)
                        && criterion.inventory_state == Some(state)
                        && criterion.status == QualificationPerformanceCriterionStatusV2::Failed
                }));
            }
        }
    }

    #[test]
    fn h8_v2_missing_required_external_run_is_unknown_and_prevents_eligibility() {
        let mut evidence = complete_v2_evidence(100, 100, 99, 100);
        evidence.runs.retain(|run| {
            !(run.workload == QualificationPerformanceWorkloadV2::ExternalCorpus
                && run.candidate == QualificationPerformanceRoleV1::SqliteWal
                && run.run_index == 1)
        });
        rehash_v2(&mut evidence);

        let result =
            evaluate_qualification_performance_v2(&evidence).expect("valid partial package");
        let sqlite = result
            .candidates
            .iter()
            .find(|candidate| candidate.candidate == QualificationPerformanceRoleV1::SqliteWal)
            .expect("SQLite result");
        assert_eq!(
            sqlite.status,
            QualificationPerformanceCriterionStatusV2::Unknown
        );
        assert!(sqlite.criteria.iter().any(|criterion| {
            criterion.workload == QualificationPerformanceWorkloadV2::ExternalCorpus
                && criterion.run_index == 1
                && criterion.status == QualificationPerformanceCriterionStatusV2::Unknown
        }));
    }

    #[test]
    fn h8_v2_rejects_incomplete_protocol_fields_and_noncanonical_evidence() {
        let mut incomplete = complete_v2_evidence(100, 100, 99, 100);
        incomplete.runs[0].controls.durable_acknowledgement = false;
        rehash_v2(&mut incomplete);
        assert!(evaluate_qualification_performance_v2(&incomplete).is_err());

        let mut noncanonical = complete_v2_evidence(100, 100, 99, 100);
        noncanonical.evidence_sha256 = LOCK_SHA256.to_owned();
        assert!(evaluate_qualification_performance_v2(&noncanonical).is_err());
    }

    #[test]
    fn h8_v1_reports_remain_historical_and_cannot_parse_as_v2() {
        let historical = serde_json::json!({
            "schema": QUALIFICATION_PERFORMANCE_DIAGNOSTICS_SCHEMA_V1,
            "contractSchema": QUALIFICATION_PERFORMANCE_DIAGNOSTIC_CONTRACT_SCHEMA_V1,
            "contractSha256": diagnostic_contract_sha256(),
            "cases": [],
        });

        assert!(serde_json::from_value::<QualificationPerformanceEvidenceV2>(historical).is_err());
    }

    #[test]
    fn loose_baseline_evidence_rejects_candidate_authority_and_has_no_verdict() {
        let evidence = QualificationLooseBaselineEvidenceV1::fixture_for_tests();
        evidence.validate().expect("complete loose-only evidence");

        let serialized = serde_json::to_string(&evidence).expect("loose evidence JSON");
        for forbidden in [
            "candidate",
            "candidateRole",
            "comparison",
            "threshold",
            "ceiling",
            "verdict",
            "eligible",
            "passed",
            "failed",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "loose evidence serialized forbidden authority {forbidden}"
            );
        }

        for (field, value) in [
            ("candidate", serde_json::json!("engine")),
            ("candidateRole", serde_json::json!("replacement")),
            ("comparison", serde_json::json!({"baseline": 1})),
            ("threshold", serde_json::json!(125)),
            ("verdict", serde_json::json!("passed")),
        ] {
            let mut document = serde_json::to_value(&evidence).expect("loose evidence value");
            document
                .as_object_mut()
                .expect("evidence object")
                .insert(field.to_owned(), value);
            assert!(
                serde_json::from_value::<QualificationLooseBaselineEvidenceV1>(document).is_err(),
                "loose evidence admitted {field}"
            );
        }

        let mut nested = serde_json::to_value(&evidence).expect("loose evidence value");
        nested["runs"][0]["allocations"][0]["inventory"]["candidate"] = serde_json::json!("engine");
        assert!(
            serde_json::from_value::<QualificationLooseBaselineEvidenceV1>(nested).is_err(),
            "loose inventory admitted candidate identity"
        );
    }

    #[test]
    fn loose_baseline_evidence_requires_every_identity_operation_and_read_class() {
        let complete = QualificationLooseBaselineEvidenceV1::fixture_for_tests();
        complete.validate().expect("complete loose evidence");

        let mut stale_source = complete.clone();
        stale_source.source_commit = SOURCE_COMMIT.to_owned();
        stale_source.evidence_sha256 = stale_source.canonical_sha256().expect("canonical evidence");
        assert!(stale_source.validate().is_err());

        let mut missing_schedule = complete.clone();
        missing_schedule.runs[0].schedule_sha256.clear();
        missing_schedule.evidence_sha256 = missing_schedule
            .canonical_sha256()
            .expect("canonical evidence");
        assert!(missing_schedule.validate().is_err());

        let mut missing_operation = complete.clone();
        missing_operation.runs[0].samples.retain(|sample| {
            sample.operation != QualificationLooseBaselineOperationV1::StrictReplay
        });
        missing_operation.evidence_sha256 = missing_operation
            .canonical_sha256()
            .expect("canonical evidence");
        assert!(missing_operation.validate().is_err());

        let mut missing_read_class = complete;
        missing_read_class.runs[0]
            .samples
            .retain(|sample| sample.read_class != Some(QualificationKeyedReadClassV1::Middle));
        missing_read_class.evidence_sha256 = missing_read_class
            .canonical_sha256()
            .expect("canonical evidence");
        assert!(missing_read_class.validate().is_err());
    }

    #[test]
    fn loose_baseline_evidence_rejects_unavailable_platform_provenance() {
        for missing_cpu in ["unavailable", "-"] {
            let mut evidence = QualificationLooseBaselineEvidenceV1::fixture_for_tests();
            evidence.platform.cpu = missing_cpu.to_owned();
            evidence.evidence_sha256 = evidence.canonical_sha256().expect("canonical evidence");

            assert!(evidence.validate().is_err());
        }
    }

    #[test]
    fn linux_cpu_provenance_falls_back_to_a_hardware_model() {
        let cpuinfo = "processor: 0\nCPU implementer: 0x41\nCPU architecture: 8\n";

        assert_eq!(
            linux_cpu_description_from_sources_v1(
                cpuinfo,
                &["", "Parallels ARM Virtual Machine\n"]
            ),
            Some("Parallels ARM Virtual Machine".to_owned())
        );
        assert_eq!(
            linux_cpu_description_from_sources_v1(cpuinfo, &["unavailable", "-"]),
            None
        );
    }

    #[test]
    fn loose_baseline_evidence_requires_fixed_controls_receipts_and_allocation_states() {
        let complete = QualificationLooseBaselineEvidenceV1::fixture_for_tests();

        let mut wrong_controls = complete.clone();
        wrong_controls.runs[0].controls.measured_iterations -= 1;
        wrong_controls.evidence_sha256 = wrong_controls
            .canonical_sha256()
            .expect("canonical evidence");
        assert!(wrong_controls.validate().is_err());

        let mut missing_receipt = complete.clone();
        missing_receipt.runs[0].samples[0]
            .receipt
            .aggregate_receipt_sha256
            .clear();
        missing_receipt.evidence_sha256 = missing_receipt
            .canonical_sha256()
            .expect("canonical evidence");
        assert!(missing_receipt.validate().is_err());

        let mut missing_scope = complete.clone();
        missing_scope.runs[0].allocations.retain(|allocation| {
            allocation.scope != QualificationPerformanceAllocationScopeV2::Event
        });
        missing_scope.evidence_sha256 = missing_scope
            .canonical_sha256()
            .expect("canonical evidence");
        assert!(missing_scope.validate().is_err());

        let mut missing_state = complete;
        missing_state.runs[0].allocations.retain(|allocation| {
            allocation.state != QualificationPerformanceInventoryStateV1::Reopened
        });
        missing_state.evidence_sha256 = missing_state
            .canonical_sha256()
            .expect("canonical evidence");
        assert!(missing_state.validate().is_err());
    }

    #[test]
    fn loose_baseline_serialization_excludes_private_and_record_level_material() {
        let evidence = QualificationLooseBaselineEvidenceV1::fixture_for_tests();
        let serialized = serde_json::to_string(&evidence).expect("loose evidence JSON");

        for forbidden in [
            "/Users/",
            "\\\\",
            "POINTBREAK_",
            "environmentValue",
            "root",
            "path",
            "payload",
            "decodedBytes",
            "logicalKey",
            "recordHash",
            "recordSha256",
            "error",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "loose evidence serialized private material {forbidden}"
            );
        }
    }

    #[test]
    fn loose_baseline_keyed_read_receipts_bind_exact_outcomes_without_exposing_record_hashes() {
        let first = loose_baseline_receipt_v1(
            QualificationLooseBaselineReceiptKindV1::KeyedReadPresentExact,
            1,
            512,
            &sha256_bytes_hex(b"first exact value"),
        )
        .expect("first keyed-read receipt");
        let second = loose_baseline_receipt_v1(
            QualificationLooseBaselineReceiptKindV1::KeyedReadPresentExact,
            1,
            512,
            &sha256_bytes_hex(b"second exact value"),
        )
        .expect("second keyed-read receipt");

        assert_ne!(
            first.aggregate_receipt_sha256,
            second.aggregate_receipt_sha256
        );
        let serialized = serde_json::to_string(&first).expect("keyed-read receipt JSON");
        assert!(!serialized.contains(&sha256_bytes_hex(b"first exact value")));
    }

    #[test]
    fn prospective_contract_proposal_shape_requires_every_decision_surface_field() {
        let complete = QualificationProspectiveContractProposalShapeV1::complete();
        complete.validate().expect("complete proposal shape");
        assert!(complete.decision_fields.contains(
            &QualificationProspectiveContractDecisionFieldV1::OperationTimedWindowDefinition
        ));
        assert_eq!(complete.decision_fields.len(), 30);
        assert_eq!(
            complete.decision_fields,
            QualificationProspectiveContractDecisionFieldV1::ALL
        );

        let mut missing = complete.clone();
        missing.decision_fields.pop();
        assert!(missing.validate().is_err());

        let mut duplicate = complete;
        duplicate.decision_fields[0] = duplicate.decision_fields[1];
        assert!(duplicate.validate().is_err());
    }

    #[test]
    fn loose_baseline_cli_modes_and_schema_identities_are_frozen() {
        assert_eq!(
            QUALIFICATION_LOOSE_BASELINE_EVIDENCE_MODE_V1,
            "--loose-baseline-evidence"
        );
        assert_eq!(
            QUALIFICATION_LOOSE_BASELINE_SMOKE_MODE_V1,
            "--loose-baseline-smoke"
        );
        assert_eq!(
            QUALIFICATION_LOOSE_BASELINE_EVIDENCE_SCHEMA_V1,
            "pointbreak.qualification-loose-baseline-evidence.v1"
        );
        assert_eq!(
            QUALIFICATION_LOOSE_BASELINE_SMOKE_SCHEMA_V1,
            "pointbreak.qualification-loose-baseline-smoke.v1"
        );
        assert_eq!(
            QUALIFICATION_PROSPECTIVE_CONTRACT_PROPOSAL_SHAPE_SCHEMA_V1,
            "pointbreak.qualification-prospective-contract-proposal-shape.v1"
        );
    }

    #[test]
    fn prospective_contract_v1_compiles_the_exact_approved_docket() {
        let contract = QualificationProspectiveContractV1::frozen();

        assert_eq!(
            contract.schema,
            "pointbreak.qualification-prospective-feasibility-contract.v1"
        );
        assert_eq!(
            contract.approved_proposal_sha256,
            "83446c8a40eb71fa4696ee5d71043c47beb8624fc97e2360b62337e489ad67e8"
        );
        assert_eq!(contract.run_controls.warmup_iterations, 3);
        assert_eq!(contract.run_controls.measured_iterations, 30);
        assert_eq!(contract.run_controls.independent_runs, 2);
        assert_eq!(contract.run_controls.p95_rank, 29);
        assert_eq!(contract.timing_thresholds.len(), 7);
        assert_eq!(contract.platforms.len(), 3);
        assert_eq!(contract.workloads.len(), 5);
        assert_eq!(contract.allocation.event_savings_percent, 25);
        assert_eq!(contract.allocation.complete_profile_savings_percent, 10);
        assert_eq!(contract.allocation.high_water_numerator, 150);
        assert_eq!(contract.allocation.high_water_denominator, 100);
        assert_eq!(contract.maintenance.foreground_p95_max_nanos, 250_000_000);
        assert_eq!(contract.maintenance.g1_total_max_nanos, 5_000_000_000);
        assert_eq!(contract.maintenance.g2_total_max_nanos, 30_000_000_000);
        assert_eq!(
            contract.derivation.generator_landing_commit,
            "8e4894fb93a0b184f5af7340fd5b4e91751743fe"
        );
        assert!(!contract.derivation.candidate_measurements_used);
        assert!(!contract.derivation.historical_candidate_results_used);
        assert!(!contract.derivation.private_corpus_used);
        assert_eq!(contract.filesystem_proof_eligible, ["apfs", "ext4", "ntfs"]);
        assert!(
            contract
                .timing_combination_formula
                .contains("ceil(loose_p95_ns")
        );
        assert_eq!(
            contract.canonical_sha256().expect("contract hash"),
            QUALIFICATION_PROSPECTIVE_CONTRACT_SHA256_V1
        );
        contract.validate().expect("frozen prospective contract");

        let publication = qualification_prospective_contract_v1_publication();
        assert_eq!(publication.contract, contract);
        assert_eq!(
            publication.contract_sha256,
            QUALIFICATION_PROSPECTIVE_CONTRACT_SHA256_V1
        );
        assert!(publication.decision_table_markdown.contains("G1"));
        assert!(publication.decision_table_markdown.contains("25%"));
        assert!(publication.decision_table_markdown.contains("APFS"));
    }

    #[test]
    fn prospective_contract_v1_timing_boundaries_bind_absolute_relative_and_guard_rules() {
        let contract = QualificationProspectiveContractV1::frozen();

        for threshold in &contract.timing_thresholds {
            let small_baseline = 1_000;
            let small_limit = prospective_timing_limit_nanos_v1(threshold, small_baseline);
            assert_eq!(
                small_limit,
                small_baseline + threshold.small_baseline_guard_band_nanos
            );
            assert_eq!(
                prospective_timing_status_v1(threshold, small_baseline, small_limit),
                QualificationProspectiveCriterionStatusV1::Passed
            );
            assert_eq!(
                prospective_timing_status_v1(threshold, small_baseline, small_limit + 1),
                QualificationProspectiveCriterionStatusV1::Failed
            );

            let large_baseline = threshold.small_baseline_guard_band_nanos.saturating_mul(5);
            let relative_limit = large_baseline
                .saturating_mul(u64::from(threshold.relative_numerator))
                .div_ceil(u64::from(threshold.relative_denominator));
            assert!(relative_limit >= large_baseline + threshold.small_baseline_guard_band_nanos);
            assert_eq!(
                prospective_timing_limit_nanos_v1(threshold, large_baseline),
                threshold.absolute_ceiling_nanos.min(relative_limit)
            );

            let ceiling_baseline = threshold.absolute_ceiling_nanos;
            assert_eq!(
                prospective_timing_status_v1(
                    threshold,
                    ceiling_baseline,
                    threshold.absolute_ceiling_nanos,
                ),
                QualificationProspectiveCriterionStatusV1::Passed
            );
            assert_eq!(
                prospective_timing_status_v1(
                    threshold,
                    ceiling_baseline,
                    threshold.absolute_ceiling_nanos + 1,
                ),
                QualificationProspectiveCriterionStatusV1::Failed
            );
        }
    }

    #[test]
    fn prospective_contract_v1_allocation_and_maintenance_boundaries_are_exact() {
        let contract = QualificationProspectiveContractV1::frozen();

        for cap in [
            contract.allocation.event_fixed_overhead_bytes,
            contract.allocation.complete_profile_fixed_overhead_bytes,
            contract.allocation.event_peak_headroom_bytes,
            contract.allocation.complete_profile_peak_headroom_bytes,
        ] {
            assert_eq!(
                prospective_at_or_below_status_v1(cap, cap),
                QualificationProspectiveCriterionStatusV1::Passed
            );
            assert_eq!(
                prospective_at_or_below_status_v1(cap + 1, cap),
                QualificationProspectiveCriterionStatusV1::Failed
            );
        }

        for savings_percent in [
            contract.allocation.event_savings_percent,
            contract.allocation.complete_profile_savings_percent,
        ] {
            let baseline_bytes = 1_000;
            let largest_passing = baseline_bytes * u64::from(100 - savings_percent) / 100;
            assert_eq!(
                prospective_savings_status_v1(largest_passing, baseline_bytes, savings_percent,),
                QualificationProspectiveCriterionStatusV1::Passed
            );
            assert_eq!(
                prospective_savings_status_v1(largest_passing + 1, baseline_bytes, savings_percent,),
                QualificationProspectiveCriterionStatusV1::Failed
            );
        }

        assert_eq!(
            prospective_crossover_status_v1(999, 1_000),
            QualificationProspectiveCriterionStatusV1::Passed
        );
        assert_eq!(
            prospective_crossover_status_v1(1_000, 1_000),
            QualificationProspectiveCriterionStatusV1::Failed
        );

        let high_water_limit = 1_001_u64
            .saturating_mul(u64::from(contract.allocation.high_water_numerator))
            .div_ceil(u64::from(contract.allocation.high_water_denominator));
        assert_eq!(
            prospective_at_or_below_status_v1(high_water_limit, high_water_limit),
            QualificationProspectiveCriterionStatusV1::Passed
        );
        assert_eq!(
            prospective_at_or_below_status_v1(high_water_limit + 1, high_water_limit),
            QualificationProspectiveCriterionStatusV1::Failed
        );

        for limit in [
            contract.maintenance.foreground_p95_max_nanos,
            contract.maintenance.g1_total_max_nanos,
            contract.maintenance.g2_total_max_nanos,
        ] {
            assert_eq!(
                prospective_at_or_below_status_v1(limit, limit),
                QualificationProspectiveCriterionStatusV1::Passed
            );
            assert_eq!(
                prospective_at_or_below_status_v1(limit + 1, limit),
                QualificationProspectiveCriterionStatusV1::Failed
            );
        }
    }

    #[test]
    fn prospective_contract_v1_missing_duplicate_stale_and_wrong_hash_evidence_fail_closed() {
        let complete = QualificationProspectiveEvidenceV1::fixture_for_tests();
        let complete_evaluation =
            evaluate_qualification_prospective_v1(&complete).expect("complete evaluation");
        assert_eq!(
            complete_evaluation.status,
            QualificationProspectiveCriterionStatusV1::Passed
        );
        assert!(complete_evaluation.eligible);

        for run in complete.runs.clone() {
            let mut missing = complete.clone();
            missing.runs.retain(|candidate| {
                (candidate.platform, candidate.workload, candidate.run_index)
                    != (run.platform, run.workload, run.run_index)
            });
            missing.evidence_sha256 = missing.canonical_sha256().expect("missing evidence hash");
            let evaluation =
                evaluate_qualification_prospective_v1(&missing).expect("partial evidence");
            assert_eq!(
                evaluation.status,
                QualificationProspectiveCriterionStatusV1::Unknown
            );
            assert!(!evaluation.eligible);
            assert!(evaluation.criteria.iter().any(|criterion| {
                criterion.platform == run.platform
                    && criterion.workload == run.workload
                    && criterion.run_index == run.run_index
                    && criterion.status == QualificationProspectiveCriterionStatusV1::Unknown
            }));
        }

        let mut duplicate = complete.clone();
        duplicate.runs.push(duplicate.runs[0].clone());
        duplicate.evidence_sha256 = duplicate
            .canonical_sha256()
            .expect("duplicate evidence hash");
        assert!(evaluate_qualification_prospective_v1(&duplicate).is_err());

        let mut missing_timing = complete.clone();
        missing_timing
            .runs
            .iter_mut()
            .find(|run| run.workload == QualificationProspectiveWorkloadV1::G1)
            .expect("G1 run")
            .timing
            .pop();
        missing_timing.evidence_sha256 = missing_timing
            .canonical_sha256()
            .expect("missing-timing evidence hash");
        assert_eq!(
            evaluate_qualification_prospective_v1(&missing_timing)
                .expect("missing timing evaluates")
                .status,
            QualificationProspectiveCriterionStatusV1::Unknown
        );

        let mut missing_allocation = complete.clone();
        missing_allocation.runs[0].allocations.pop();
        missing_allocation.evidence_sha256 = missing_allocation
            .canonical_sha256()
            .expect("missing-allocation evidence hash");
        assert_eq!(
            evaluate_qualification_prospective_v1(&missing_allocation)
                .expect("missing allocation evaluates")
                .status,
            QualificationProspectiveCriterionStatusV1::Unknown
        );

        let mut duplicate_timing = complete.clone();
        let timing_row = duplicate_timing
            .runs
            .iter()
            .find(|run| run.workload == QualificationProspectiveWorkloadV1::G1)
            .expect("G1 run")
            .timing[0]
            .clone();
        duplicate_timing
            .runs
            .iter_mut()
            .find(|run| run.workload == QualificationProspectiveWorkloadV1::G1)
            .expect("G1 run")
            .timing
            .push(timing_row);
        duplicate_timing.evidence_sha256 = duplicate_timing
            .canonical_sha256()
            .expect("duplicate-timing evidence hash");
        assert!(evaluate_qualification_prospective_v1(&duplicate_timing).is_err());

        let mut stale = complete.clone();
        stale.source_commit = SOURCE_COMMIT.to_owned();
        stale.evidence_sha256 = stale.canonical_sha256().expect("stale evidence hash");
        assert!(evaluate_qualification_prospective_v1(&stale).is_err());

        let mut wrong_contract = complete.clone();
        wrong_contract.contract_sha256 = LOCK_SHA256.to_owned();
        wrong_contract.evidence_sha256 = wrong_contract
            .canonical_sha256()
            .expect("wrong-contract evidence hash");
        assert!(evaluate_qualification_prospective_v1(&wrong_contract).is_err());

        let mut wrong_hash = complete;
        wrong_hash.evidence_sha256 = LOCK_SHA256.to_owned();
        assert!(evaluate_qualification_prospective_v1(&wrong_hash).is_err());
    }

    #[test]
    fn prospective_contract_v1_every_timing_read_and_allocation_axis_gates_independently() {
        let complete = QualificationProspectiveEvidenceV1::fixture_for_tests();
        let contract = QualificationProspectiveContractV1::frozen();

        for run in complete
            .runs
            .iter()
            .filter(|run| run.workload.timing_required())
        {
            for threshold in &contract.timing_thresholds {
                let mut failed = complete.clone();
                let row = failed
                    .runs
                    .iter_mut()
                    .find(|candidate| {
                        (candidate.platform, candidate.workload, candidate.run_index)
                            == (run.platform, run.workload, run.run_index)
                    })
                    .and_then(|candidate| {
                        candidate.timing.iter_mut().find(|row| {
                            row.operation == threshold.operation
                                && row.read_class == threshold.read_class
                        })
                    })
                    .expect("timing row");
                let baseline_p95 = prospective_nearest_rank_p95_v1(&row.baseline_samples_nanos)
                    .expect("baseline p95");
                let failed_value = if run.workload == QualificationProspectiveWorkloadV1::G0 {
                    threshold.absolute_ceiling_nanos + 1
                } else {
                    prospective_timing_limit_nanos_v1(threshold, baseline_p95) + 1
                };
                row.candidate_samples_nanos.fill(failed_value);
                failed.evidence_sha256 = failed.canonical_sha256().expect("failed evidence hash");
                let evaluation =
                    evaluate_qualification_prospective_v1(&failed).expect("timing evaluation");
                assert_eq!(
                    evaluation.status,
                    QualificationProspectiveCriterionStatusV1::Failed
                );
                assert!(evaluation.criteria.iter().any(|criterion| {
                    criterion.platform == run.platform
                        && criterion.workload == run.workload
                        && criterion.run_index == run.run_index
                        && criterion.operation == Some(threshold.operation)
                        && criterion.read_class == threshold.read_class
                        && criterion.status == QualificationProspectiveCriterionStatusV1::Failed
                }));
            }
        }

        for run in complete
            .runs
            .iter()
            .filter(|run| run.workload.savings_required())
        {
            for scope in QualificationPerformanceAllocationScopeV2::ALL {
                for state in QualificationPerformanceInventoryStateV1::ALL {
                    let mut failed = complete.clone();
                    let row = failed
                        .runs
                        .iter_mut()
                        .find(|candidate| {
                            (candidate.platform, candidate.workload, candidate.run_index)
                                == (run.platform, run.workload, run.run_index)
                        })
                        .and_then(|candidate| {
                            candidate
                                .allocations
                                .iter_mut()
                                .find(|row| row.scope == scope && row.state == state)
                        })
                        .expect("allocation row");
                    row.candidate_allocated_bytes = row.baseline_allocated_bytes;
                    failed.evidence_sha256 =
                        failed.canonical_sha256().expect("failed evidence hash");
                    let evaluation = evaluate_qualification_prospective_v1(&failed)
                        .expect("allocation evaluation");
                    assert_eq!(
                        evaluation.status,
                        QualificationProspectiveCriterionStatusV1::Failed
                    );
                    assert!(evaluation.criteria.iter().any(|criterion| {
                        criterion.platform == run.platform
                            && criterion.workload == run.workload
                            && criterion.run_index == run.run_index
                            && criterion.allocation_scope == Some(scope)
                            && criterion.inventory_state == Some(state)
                            && criterion.status == QualificationProspectiveCriterionStatusV1::Failed
                    }));
                }
            }
        }
    }

    #[test]
    fn prospective_contract_v1_small_store_high_water_and_maintenance_rows_are_distinct() {
        let complete = QualificationProspectiveEvidenceV1::fixture_for_tests();
        let contract = QualificationProspectiveContractV1::frozen();

        for workload in [
            QualificationProspectiveWorkloadV1::P0,
            QualificationProspectiveWorkloadV1::M0,
        ] {
            for scope in QualificationPerformanceAllocationScopeV2::ALL {
                let mut fixed_overhead = complete.clone();
                let row = fixed_overhead
                    .runs
                    .iter_mut()
                    .find(|run| run.workload == workload)
                    .and_then(|run| {
                        run.allocations.iter_mut().find(|row| {
                            row.scope == scope
                                && row.state == QualificationPerformanceInventoryStateV1::Steady
                        })
                    })
                    .expect("small-store steady row");
                let cap = contract.allocation.fixed_overhead_cap(scope);
                row.candidate_allocated_bytes = row.candidate_logical_bytes + cap + 1;
                fixed_overhead.evidence_sha256 = fixed_overhead
                    .canonical_sha256()
                    .expect("fixed-overhead evidence hash");
                let evaluation = evaluate_qualification_prospective_v1(&fixed_overhead)
                    .expect("fixed-overhead evaluation");
                assert!(evaluation.criteria.iter().any(|criterion| {
                    criterion.kind
                        == QualificationProspectiveCriterionKindV1::SmallStoreFixedOverhead
                        && criterion.workload == workload
                        && criterion.allocation_scope == Some(scope)
                        && criterion.status == QualificationProspectiveCriterionStatusV1::Failed
                }));

                let mut peak = complete.clone();
                let run = peak
                    .runs
                    .iter_mut()
                    .find(|run| run.workload == workload)
                    .expect("small-store run");
                let steady = run
                    .allocations
                    .iter()
                    .find(|row| {
                        row.scope == scope
                            && row.state == QualificationPerformanceInventoryStateV1::Steady
                    })
                    .expect("small-store steady row")
                    .candidate_allocated_bytes;
                let high_water = run
                    .allocations
                    .iter_mut()
                    .find(|row| {
                        row.scope == scope
                            && row.state == QualificationPerformanceInventoryStateV1::HighWater
                    })
                    .expect("small-store high-water row");
                high_water.candidate_allocated_bytes =
                    steady + contract.allocation.peak_headroom_cap(scope) + 1;
                peak.evidence_sha256 = peak.canonical_sha256().expect("peak evidence hash");
                let evaluation =
                    evaluate_qualification_prospective_v1(&peak).expect("peak evaluation");
                assert!(evaluation.criteria.iter().any(|criterion| {
                    criterion.kind
                        == QualificationProspectiveCriterionKindV1::SmallStorePeakHeadroom
                        && criterion.workload == workload
                        && criterion.allocation_scope == Some(scope)
                        && criterion.status == QualificationProspectiveCriterionStatusV1::Failed
                }));
            }
        }

        for workload in [
            QualificationProspectiveWorkloadV1::G1,
            QualificationProspectiveWorkloadV1::G2,
        ] {
            let mut high_water = complete.clone();
            let run = high_water
                .runs
                .iter_mut()
                .find(|run| run.workload == workload)
                .expect("quantitative run");
            let steady = run
                .allocations
                .iter()
                .find(|row| {
                    row.scope == QualificationPerformanceAllocationScopeV2::Event
                        && row.state == QualificationPerformanceInventoryStateV1::Steady
                })
                .expect("steady row")
                .candidate_allocated_bytes;
            let row = run
                .allocations
                .iter_mut()
                .find(|row| {
                    row.scope == QualificationPerformanceAllocationScopeV2::Event
                        && row.state == QualificationPerformanceInventoryStateV1::HighWater
                })
                .expect("high-water row");
            row.candidate_allocated_bytes = steady
                .saturating_mul(u64::from(contract.allocation.high_water_numerator))
                .div_ceil(u64::from(contract.allocation.high_water_denominator))
                + 1;
            high_water.evidence_sha256 = high_water
                .canonical_sha256()
                .expect("high-water evidence hash");
            let evaluation =
                evaluate_qualification_prospective_v1(&high_water).expect("high-water evaluation");
            assert!(evaluation.criteria.iter().any(|criterion| {
                criterion.kind == QualificationProspectiveCriterionKindV1::HighWaterAmplification
                    && criterion.workload == workload
                    && criterion.status == QualificationProspectiveCriterionStatusV1::Failed
            }));

            let mut maintenance = complete.clone();
            let run = maintenance
                .runs
                .iter_mut()
                .find(|run| run.workload == workload)
                .expect("maintenance run");
            let maintenance_row = run.maintenance.as_mut().expect("maintenance row");
            maintenance_row.total_nanos = contract.maintenance.total_max_nanos(workload) + 1;
            maintenance.evidence_sha256 = maintenance
                .canonical_sha256()
                .expect("maintenance evidence hash");
            let evaluation = evaluate_qualification_prospective_v1(&maintenance)
                .expect("maintenance evaluation");
            assert!(evaluation.criteria.iter().any(|criterion| {
                criterion.kind == QualificationProspectiveCriterionKindV1::MaintenanceTotal
                    && criterion.workload == workload
                    && criterion.status == QualificationProspectiveCriterionStatusV1::Failed
            }));
        }

        let mut not_applicable = complete.clone();
        for run in not_applicable
            .runs
            .iter_mut()
            .filter(|run| run.workload.savings_required())
        {
            run.maintenance = Some(QualificationProspectiveMaintenanceEvidenceV1 {
                required: false,
                foreground_samples_nanos: Vec::new(),
                total_nanos: 0,
                not_applicable_mechanism_proof_sha256: Some(
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned(),
                ),
            });
        }
        not_applicable.evidence_sha256 = not_applicable
            .canonical_sha256()
            .expect("N/A evidence hash");
        assert_eq!(
            evaluate_qualification_prospective_v1(&not_applicable)
                .expect("mechanism-proven N/A evaluates")
                .status,
            QualificationProspectiveCriterionStatusV1::Passed
        );

        not_applicable.runs[6]
            .maintenance
            .as_mut()
            .expect("maintenance row")
            .not_applicable_mechanism_proof_sha256 = None;
        not_applicable.evidence_sha256 = not_applicable
            .canonical_sha256()
            .expect("invalid N/A evidence hash");
        assert!(evaluate_qualification_prospective_v1(&not_applicable).is_err());
    }

    #[test]
    fn prospective_contract_v1_external_corroboration_is_veto_only_and_never_public() {
        assert_eq!(
            apply_prospective_external_corroboration_v1(
                QualificationProspectiveCriterionStatusV1::Failed,
                QualificationProspectiveExternalCorroborationV1::Satisfied,
            ),
            QualificationProspectiveCriterionStatusV1::Failed
        );
        assert_eq!(
            apply_prospective_external_corroboration_v1(
                QualificationProspectiveCriterionStatusV1::Passed,
                QualificationProspectiveExternalCorroborationV1::Vetoed,
            ),
            QualificationProspectiveCriterionStatusV1::Failed
        );
        assert_eq!(
            apply_prospective_external_corroboration_v1(
                QualificationProspectiveCriterionStatusV1::Passed,
                QualificationProspectiveExternalCorroborationV1::Satisfied,
            ),
            QualificationProspectiveCriterionStatusV1::Passed
        );

        let publication = qualification_prospective_contract_v1_publication();
        let serialized = serde_json::to_string(&publication).expect("publication JSON");
        for forbidden in [
            "candidateObservation",
            "candidateP95",
            "baselineP95",
            "profileId",
            "externalCorroborationEvidence",
        ] {
            assert!(!serialized.contains(forbidden), "published {forbidden}");
        }
        assert_eq!(
            QUALIFICATION_PROSPECTIVE_CONTRACT_PUBLICATION_MODE_V1,
            "--prospective-contract"
        );
    }
}
