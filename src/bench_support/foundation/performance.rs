use std::collections::BTreeSet;
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
    QUALIFICATION_EXTERNAL_WORKLOAD_MANIFEST_SHA256_V2, QualificationCandidateV1,
    QualificationCorpusManifestV1, QualificationFilesystemDispositionV1, QualificationInventoryV1,
    QualificationPlatformEnvironmentV1, QualificationProfile, QualificationRawSampleV1,
    QualificationRecordKindV1, SEGMENT_QUALIFICATION_PROFILE_ID_V1,
    SQLITE_QUALIFICATION_PROFILE_ID_V1, SegmentQualificationProfile, SqliteQualificationProfile,
    classify_qualification_filesystem, load_external_workload_v2_manifest_from_path,
    modeled_post_foundation_manifest, qualification_cargo_lock_sha256,
    qualification_filesystem_name, qualification_source_commit, synthetic_legacy_manifest,
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
pub const QUALIFICATION_PERFORMANCE_CONTRACT_PUBLICATION_SCHEMA_V2: &str =
    "pointbreak.qualification-performance-contract-publication.v2";
pub const QUALIFICATION_PERFORMANCE_CONTRACT_SHA256_V2: &str =
    "55c473f448c80ba26e5e0eeaf23ebdd7c7b3827954bfec78873d1fd839a54a36";

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
        assert_eq!(
            evaluate_qualification_performance_h8_v1(&equal_boundary),
            Ok(None)
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
}
