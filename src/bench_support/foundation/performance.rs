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
    QualificationCandidateV1, QualificationCorpusManifestV1, QualificationFilesystemDispositionV1,
    QualificationInventoryV1, QualificationPlatformEnvironmentV1, QualificationProfile,
    QualificationRawSampleV1, QualificationRecordKindV1, SEGMENT_QUALIFICATION_PROFILE_ID_V1,
    SQLITE_QUALIFICATION_PROFILE_ID_V1, SegmentQualificationProfile, SqliteQualificationProfile,
    classify_qualification_filesystem, load_frozen_legacy_manifest_from_path,
    modeled_post_foundation_manifest, qualification_cargo_lock_sha256,
    qualification_filesystem_name, qualification_source_commit, synthetic_legacy_manifest,
};
use crate::canonical_hash::{canonical_json_bytes, sha256_bytes_hex};

pub const QUALIFICATION_PERFORMANCE_DIAGNOSTICS_SCHEMA_V1: &str =
    "pointbreak.qualification-performance-diagnostics.v1";
pub const QUALIFICATION_PERFORMANCE_DIAGNOSTIC_CONTRACT_SCHEMA_V1: &str =
    "pointbreak.qualification-performance-diagnostic-contract.v1";

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
            load_frozen_legacy_manifest_from_path(Some(path))
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
}
