use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    QualificationCorpusManifestV1, QualificationCreateOutcome, QualificationInventoryV1,
    QualificationProfile, QualificationRecordKindV1, QualificationScenarioOutcomeV1,
    SegmentDiagnosticStateV1, SegmentFailurePointV1, SegmentQualificationProfile,
    SqliteDiagnosticStateV1, SqliteQualificationProfile, modeled_post_foundation_manifest,
    qualification_filesystem_name, synthetic_legacy_manifest,
};
use crate::canonical_hash::{canonical_json_bytes, sha256_bytes_hex};

pub const QUALIFICATION_PLAN_SCHEMA_V1: &str = "pointbreak.qualification-plan.v1";
pub const QUALIFICATION_EVIDENCE_SCHEMA_V1: &str = "pointbreak.qualification-evidence.v1";
const BARRIER_SCHEMA_V1: &str = "pointbreak.qualification-process-barrier.v1";
const RUSQLITE_BUILD_VERSION: &str = "0.40.1";
const LIBSQLITE3_SYS_BUILD_VERSION: &str = "0.38.1";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationCandidateV1 {
    SqliteWal,
    BoundedSegments,
}

impl QualificationCandidateV1 {
    pub const ALL: [Self; 2] = [Self::SqliteWal, Self::BoundedSegments];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SqliteWal => "sqlite_wal",
            Self::BoundedSegments => "bounded_segments",
        }
    }

    pub fn build_id(self, cargo_lock_sha256: &str) -> String {
        match self {
            Self::SqliteWal => format!(
                "sqlite-wal:rusqlite-{RUSQLITE_BUILD_VERSION}:libsqlite3-sys-{LIBSQLITE3_SYS_BUILD_VERSION}:{cargo_lock_sha256}"
            ),
            Self::BoundedSegments => {
                format!(
                    "bounded-segments:pointbreak-{}:{cargo_lock_sha256}",
                    env!("CARGO_PKG_VERSION")
                )
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationWorkloadV1 {
    SyntheticLegacy,
    ModeledFoundation,
}

impl QualificationWorkloadV1 {
    pub const ALL: [Self; 2] = [Self::SyntheticLegacy, Self::ModeledFoundation];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SyntheticLegacy => "synthetic_legacy",
            Self::ModeledFoundation => "modeled_foundation",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationPlatformRequirementV1 {
    MultiprocessLocking,
    StableReaderWriter,
    CrashReopen,
    AmbiguousAcknowledgement,
    CorruptionDetection,
    MaintenanceRecovery,
    ConcurrentBackup,
    CopyOutRepair,
    AllocationFailure,
    RawPerformance,
    LongPath,
    OpenHandleReplacement,
    FilesystemPolicy,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationScenarioV1 {
    CreateOnceRace,
    StableReaderWriter,
    AcknowledgedWriteReopen,
    AmbiguousAcknowledgementRetry,
    CorruptionDetection,
    MaintenanceInterruption,
    BackupWriterOverlap,
    CopyOutRepair,
    AllocationFailure,
    Performance,
    LongPath,
    OpenHandleReplacement,
    FilesystemPolicy,
}

impl QualificationScenarioV1 {
    pub const ALL: [Self; 13] = [
        Self::CreateOnceRace,
        Self::StableReaderWriter,
        Self::AcknowledgedWriteReopen,
        Self::AmbiguousAcknowledgementRetry,
        Self::CorruptionDetection,
        Self::MaintenanceInterruption,
        Self::BackupWriterOverlap,
        Self::CopyOutRepair,
        Self::AllocationFailure,
        Self::Performance,
        Self::LongPath,
        Self::OpenHandleReplacement,
        Self::FilesystemPolicy,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::CreateOnceRace => "create_once_race",
            Self::StableReaderWriter => "stable_reader_writer",
            Self::AcknowledgedWriteReopen => "acknowledged_write_reopen",
            Self::AmbiguousAcknowledgementRetry => "ambiguous_acknowledgement_retry",
            Self::CorruptionDetection => "corruption_detection",
            Self::MaintenanceInterruption => "maintenance_interruption",
            Self::BackupWriterOverlap => "backup_writer_overlap",
            Self::CopyOutRepair => "copy_out_repair",
            Self::AllocationFailure => "allocation_failure",
            Self::Performance => "performance",
            Self::LongPath => "long_path",
            Self::OpenHandleReplacement => "open_handle_replacement",
            Self::FilesystemPolicy => "filesystem_policy",
        }
    }

    fn platform_requirement(self) -> QualificationPlatformRequirementV1 {
        match self {
            Self::CreateOnceRace => QualificationPlatformRequirementV1::MultiprocessLocking,
            Self::StableReaderWriter => QualificationPlatformRequirementV1::StableReaderWriter,
            Self::AcknowledgedWriteReopen => QualificationPlatformRequirementV1::CrashReopen,
            Self::AmbiguousAcknowledgementRetry => {
                QualificationPlatformRequirementV1::AmbiguousAcknowledgement
            }
            Self::CorruptionDetection => QualificationPlatformRequirementV1::CorruptionDetection,
            Self::MaintenanceInterruption => {
                QualificationPlatformRequirementV1::MaintenanceRecovery
            }
            Self::BackupWriterOverlap => QualificationPlatformRequirementV1::ConcurrentBackup,
            Self::CopyOutRepair => QualificationPlatformRequirementV1::CopyOutRepair,
            Self::AllocationFailure => QualificationPlatformRequirementV1::AllocationFailure,
            Self::Performance => QualificationPlatformRequirementV1::RawPerformance,
            Self::LongPath => QualificationPlatformRequirementV1::LongPath,
            Self::OpenHandleReplacement => {
                QualificationPlatformRequirementV1::OpenHandleReplacement
            }
            Self::FilesystemPolicy => QualificationPlatformRequirementV1::FilesystemPolicy,
        }
    }

    fn requires_process_overlap(self) -> bool {
        matches!(
            self,
            Self::CreateOnceRace | Self::StableReaderWriter | Self::BackupWriterOverlap
        )
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPlanEntryV1 {
    pub entry_id: String,
    pub candidate: QualificationCandidateV1,
    pub workload: QualificationWorkloadV1,
    pub scenario: QualificationScenarioV1,
    pub platform_requirement: QualificationPlatformRequirementV1,
    /// Stable row-identity entropy reserved for future seeded fault placement.
    /// V1 scenarios use fixed, named fault boundaries and do not randomize them.
    pub fault_seed: u64,
    /// The fixed V1 scenario boundary at which the fault is applied.
    pub kill_point: String,
    pub candidate_build_id: String,
}

impl QualificationPlanEntryV1 {
    pub fn identity(
        &self,
    ) -> (
        QualificationCandidateV1,
        QualificationWorkloadV1,
        QualificationScenarioV1,
        QualificationPlatformRequirementV1,
    ) {
        (
            self.candidate,
            self.workload,
            self.scenario,
            self.platform_requirement,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPlanV1 {
    pub schema: String,
    pub source_commit: String,
    pub cargo_lock_sha256: String,
    pub entries: Vec<QualificationPlanEntryV1>,
    pub plan_sha256: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct QualificationPlanPreimageV1<'a> {
    schema: &'a str,
    source_commit: &'a str,
    cargo_lock_sha256: &'a str,
    entries: &'a [QualificationPlanEntryV1],
}

impl QualificationPlanV1 {
    pub fn required(source_commit: &str, cargo_lock_sha256: &str) -> Self {
        let mut entries = Vec::new();
        for candidate in QualificationCandidateV1::ALL {
            for workload in QualificationWorkloadV1::ALL {
                for scenario in QualificationScenarioV1::ALL {
                    let identity = format!(
                        "{}:{}:{}:{}",
                        candidate.as_str(),
                        workload.as_str(),
                        scenario.as_str(),
                        source_commit
                    );
                    let digest = Sha256::digest(identity.as_bytes());
                    let fault_seed = u64::from_be_bytes(
                        digest[..8]
                            .try_into()
                            .expect("a SHA-256 prefix contains eight bytes"),
                    );
                    entries.push(QualificationPlanEntryV1 {
                        entry_id: format!(
                            "{}:{}:{}",
                            candidate.as_str(),
                            workload.as_str(),
                            scenario.as_str()
                        ),
                        candidate,
                        workload,
                        scenario,
                        platform_requirement: scenario.platform_requirement(),
                        fault_seed,
                        kill_point: scenario.as_str().to_owned(),
                        candidate_build_id: candidate.build_id(cargo_lock_sha256),
                    });
                }
            }
        }
        let preimage = QualificationPlanPreimageV1 {
            schema: QUALIFICATION_PLAN_SCHEMA_V1,
            source_commit,
            cargo_lock_sha256,
            entries: &entries,
        };
        let plan_sha256 = hash_canonical(&preimage).expect("qualification plan serializes");
        Self {
            schema: QUALIFICATION_PLAN_SCHEMA_V1.to_owned(),
            source_commit: source_commit.to_owned(),
            cargo_lock_sha256: cargo_lock_sha256.to_owned(),
            entries,
            plan_sha256,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != QUALIFICATION_PLAN_SCHEMA_V1 {
            return Err(format!(
                "unsupported qualification plan schema {}",
                self.schema
            ));
        }
        validate_hex(&self.source_commit, 40, "source commit")?;
        validate_hex(&self.cargo_lock_sha256, 64, "Cargo.lock SHA-256")?;
        let expected = Self::required(&self.source_commit, &self.cargo_lock_sha256);
        if self.entries != expected.entries {
            return Err("qualification plan is missing, duplicated, or reordered".to_owned());
        }
        if self.plan_sha256 != expected.plan_sha256 {
            return Err("qualification plan hash does not match its canonical preimage".to_owned());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationExecutionDispositionV1 {
    Executed,
    Skipped,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationFilesystemDispositionV1 {
    LocalProofEligible,
    AdvisoryOnly,
    Refused,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPlatformEnvironmentV1 {
    pub operating_system: String,
    pub architecture: String,
    pub filesystem: String,
    pub filesystem_disposition: QualificationFilesystemDispositionV1,
    pub allocation_method: String,
    pub rustc: String,
    pub build_source: String,
    pub build_describe: String,
    pub source_tree_dirty: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationRawSampleV1 {
    pub operation: String,
    pub iteration: u32,
    pub elapsed_nanos: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationBarrierParticipantEvidenceV1 {
    pub participant: String,
    pub process_id: u32,
    pub ready_unix_nanos: u64,
    pub completed_unix_nanos: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationProcessOverlapEvidenceV1 {
    pub release_unix_nanos: u64,
    pub participants: Vec<QualificationBarrierParticipantEvidenceV1>,
}

impl QualificationProcessOverlapEvidenceV1 {
    pub fn validate_overlap(&self) -> Result<(), String> {
        if self.participants.is_empty() {
            return Err("process-overlap evidence has no participants".to_owned());
        }
        let mut names = BTreeSet::new();
        for participant in &self.participants {
            if !names.insert(&participant.participant) {
                return Err("process-overlap evidence repeats a participant".to_owned());
            }
            if participant.process_id == 0
                || participant.ready_unix_nanos > self.release_unix_nanos
                || participant.completed_unix_nanos < self.release_unix_nanos
            {
                return Err("process barrier does not prove a real overlap interval".to_owned());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationScenarioEvidenceV1 {
    pub entry_id: String,
    pub candidate: QualificationCandidateV1,
    pub workload: QualificationWorkloadV1,
    pub scenario: QualificationScenarioV1,
    pub platform_requirement: QualificationPlatformRequirementV1,
    pub source_commit: String,
    pub candidate_build_id: String,
    pub workload_manifest_sha256: String,
    pub fault_seed: u64,
    pub kill_point: String,
    pub execution: QualificationExecutionDispositionV1,
    pub outcome: QualificationScenarioOutcomeV1,
    pub environment: QualificationPlatformEnvironmentV1,
    pub raw_samples: Vec<QualificationRawSampleV1>,
    pub inventory: Option<QualificationInventoryV1>,
    pub baseline_inventory: Option<QualificationInventoryV1>,
    pub process_overlap: Option<QualificationProcessOverlapEvidenceV1>,
    pub failure: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationEvidenceV1 {
    pub schema: String,
    pub plan_sha256: String,
    pub results: Vec<QualificationScenarioEvidenceV1>,
}

impl QualificationEvidenceV1 {
    pub fn validate(&self, plan: &QualificationPlanV1) -> Result<(), String> {
        plan.validate()?;
        if self.schema != QUALIFICATION_EVIDENCE_SCHEMA_V1 {
            return Err(format!(
                "unsupported qualification evidence schema {}",
                self.schema
            ));
        }
        if self.plan_sha256 != plan.plan_sha256 {
            return Err("evidence is bound to a different qualification plan".to_owned());
        }
        if self.results.len() != plan.entries.len() {
            return Err("qualification evidence is missing or has extra results".to_owned());
        }
        for (entry, result) in plan.entries.iter().zip(&self.results) {
            if result.entry_id != entry.entry_id
                || result.candidate != entry.candidate
                || result.workload != entry.workload
                || result.scenario != entry.scenario
                || result.platform_requirement != entry.platform_requirement
                || result.fault_seed != entry.fault_seed
                || result.kill_point != entry.kill_point
            {
                return Err(format!(
                    "result {} does not match its required matrix entry",
                    result.entry_id
                ));
            }
            if result.source_commit != plan.source_commit {
                return Err(format!(
                    "result {} uses a stale source commit",
                    result.entry_id
                ));
            }
            if result.candidate_build_id != entry.candidate_build_id {
                return Err(format!(
                    "result {} uses the wrong candidate dependency identity",
                    result.entry_id
                ));
            }
            validate_hex(
                &result.workload_manifest_sha256,
                64,
                "workload manifest SHA-256",
            )?;
            if result.environment.operating_system.is_empty()
                || result.environment.architecture.is_empty()
                || result.environment.filesystem.is_empty()
                || result.environment.allocation_method.is_empty()
                || result.environment.rustc.is_empty()
                || result.environment.build_source.is_empty()
                || result.environment.build_describe.is_empty()
            {
                return Err(format!(
                    "result {} omits environment metadata",
                    result.entry_id
                ));
            }
            if result.execution != QualificationExecutionDispositionV1::Executed
                && result.outcome == QualificationScenarioOutcomeV1::Passed
            {
                return Err(format!(
                    "result {} marks skipped or unsupported execution as pass",
                    result.entry_id
                ));
            }
            if result.environment.filesystem_disposition
                != QualificationFilesystemDispositionV1::LocalProofEligible
                && result.outcome == QualificationScenarioOutcomeV1::Passed
            {
                return Err(format!(
                    "result {} treats a nonlocal filesystem as platform proof",
                    result.entry_id
                ));
            }
            if result.raw_samples.is_empty()
                || result
                    .raw_samples
                    .iter()
                    .any(|sample| sample.operation.is_empty() || sample.elapsed_nanos == 0)
            {
                return Err(format!("result {} omits raw samples", result.entry_id));
            }
            match (&result.inventory, result.execution) {
                (Some(inventory), _) => validate_inventory(inventory, &result.entry_id)?,
                (None, QualificationExecutionDispositionV1::Executed) => {
                    return Err(format!(
                        "result {} omits its inventory sidecar",
                        result.entry_id
                    ));
                }
                (None, _) => {}
            }
            if result.scenario == QualificationScenarioV1::Performance
                && result.execution == QualificationExecutionDispositionV1::Executed
            {
                let baseline = result.baseline_inventory.as_ref().ok_or_else(|| {
                    format!(
                        "performance result {} omits its fresh baseline inventory",
                        result.entry_id
                    )
                })?;
                validate_inventory(baseline, &format!("{} baseline", result.entry_id))?;
                for operation in [
                    "candidate_durable_append",
                    "baseline_durable_append",
                    "candidate_replay",
                    "baseline_replay",
                    "candidate_keyed_read",
                    "baseline_keyed_read",
                    "candidate_open_recovery",
                    "baseline_open_recovery",
                    "candidate_backup",
                    "candidate_restore",
                ] {
                    if !result
                        .raw_samples
                        .iter()
                        .any(|sample| sample.operation == operation)
                    {
                        return Err(format!(
                            "performance result {} omits raw {operation} samples",
                            result.entry_id
                        ));
                    }
                }
            }
            if entry.scenario.requires_process_overlap()
                && result.outcome == QualificationScenarioOutcomeV1::Passed
            {
                result
                    .process_overlap
                    .as_ref()
                    .ok_or_else(|| {
                        format!("result {} omits process-overlap evidence", result.entry_id)
                    })?
                    .validate_overlap()?;
            }
            match result.outcome {
                QualificationScenarioOutcomeV1::Passed if result.failure.is_some() => {
                    return Err(format!(
                        "passing result {} carries a failure",
                        result.entry_id
                    ));
                }
                QualificationScenarioOutcomeV1::Failed if result.failure.is_none() => {
                    return Err(format!(
                        "failed result {} omits its failure",
                        result.entry_id
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn fixture_for_tests(plan: &QualificationPlanV1) -> Self {
        let inventory = QualificationInventoryV1 {
            carriers: vec!["fixture.pbrf".to_owned()],
            logical_bytes: 1,
            encoded_bytes: 1,
            allocated_bytes: 1,
            high_water_bytes: 1,
        };
        let overlap = QualificationProcessOverlapEvidenceV1 {
            release_unix_nanos: 2,
            participants: vec![QualificationBarrierParticipantEvidenceV1 {
                participant: "fixture".to_owned(),
                process_id: 1,
                ready_unix_nanos: 1,
                completed_unix_nanos: 3,
            }],
        };
        Self {
            schema: QUALIFICATION_EVIDENCE_SCHEMA_V1.to_owned(),
            plan_sha256: plan.plan_sha256.clone(),
            results: plan
                .entries
                .iter()
                .map(|entry| QualificationScenarioEvidenceV1 {
                    entry_id: entry.entry_id.clone(),
                    candidate: entry.candidate,
                    workload: entry.workload,
                    scenario: entry.scenario,
                    platform_requirement: entry.platform_requirement,
                    source_commit: plan.source_commit.clone(),
                    candidate_build_id: entry.candidate_build_id.clone(),
                    workload_manifest_sha256:
                        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                            .to_owned(),
                    fault_seed: entry.fault_seed,
                    kill_point: entry.kill_point.clone(),
                    execution: QualificationExecutionDispositionV1::Executed,
                    outcome: QualificationScenarioOutcomeV1::Passed,
                    environment: QualificationPlatformEnvironmentV1 {
                        operating_system: "test".to_owned(),
                        architecture: "test".to_owned(),
                        filesystem: "apfs".to_owned(),
                        filesystem_disposition:
                            QualificationFilesystemDispositionV1::LocalProofEligible,
                        allocation_method: "fixture".to_owned(),
                        rustc: "rustc test".to_owned(),
                        build_source: "git".to_owned(),
                        build_describe: "fixture".to_owned(),
                        source_tree_dirty: false,
                    },
                    raw_samples: if entry.scenario == QualificationScenarioV1::Performance {
                        [
                            "candidate_durable_append",
                            "baseline_durable_append",
                            "candidate_replay",
                            "baseline_replay",
                            "candidate_keyed_read",
                            "baseline_keyed_read",
                            "candidate_open_recovery",
                            "baseline_open_recovery",
                            "candidate_backup",
                            "candidate_restore",
                        ]
                        .into_iter()
                        .map(|operation| QualificationRawSampleV1 {
                            operation: operation.to_owned(),
                            iteration: 0,
                            elapsed_nanos: 1,
                        })
                        .collect()
                    } else {
                        vec![QualificationRawSampleV1 {
                            operation: "fixture".to_owned(),
                            iteration: 0,
                            elapsed_nanos: 1,
                        }]
                    },
                    inventory: Some(inventory.clone()),
                    baseline_inventory: (entry.scenario == QualificationScenarioV1::Performance)
                        .then(|| inventory.clone()),
                    process_overlap: entry
                        .scenario
                        .requires_process_overlap()
                        .then(|| overlap.clone()),
                    failure: None,
                })
                .collect(),
        }
    }
}

pub fn classify_qualification_filesystem(name: &str) -> QualificationFilesystemDispositionV1 {
    let normalized = name.trim().to_ascii_lowercase();
    if [
        "dropbox",
        "onedrive",
        "one drive",
        "icloud",
        "google drive",
        "syncthing",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        return QualificationFilesystemDispositionV1::AdvisoryOnly;
    }
    if ["nfs", "smb", "cifs", "afp", "webdav", "sshfs"]
        .iter()
        .any(|marker| normalized.contains(marker))
    {
        return QualificationFilesystemDispositionV1::Refused;
    }
    if [
        "apfs",
        "ext4",
        "xfs",
        "btrfs",
        "ntfs",
        "refs",
        "overlay",
        "overlayfs",
    ]
    .iter()
    .any(|local| normalized == *local)
    {
        return QualificationFilesystemDispositionV1::LocalProofEligible;
    }
    QualificationFilesystemDispositionV1::AdvisoryOnly
}

#[derive(Debug)]
pub struct QualificationProcessBarrierV1 {
    root: PathBuf,
    participants: Vec<String>,
}

impl QualificationProcessBarrierV1 {
    pub fn create(root: &Path, participants: &[&str]) -> Result<Self, String> {
        if participants.is_empty() {
            return Err("process barrier requires at least one participant".to_owned());
        }
        let participants = participants
            .iter()
            .map(|participant| validate_participant(participant).map(str::to_owned))
            .collect::<Result<Vec<_>, _>>()?;
        if participants.iter().collect::<BTreeSet<_>>().len() != participants.len() {
            return Err("process barrier participants must be unique".to_owned());
        }
        let root = root.to_path_buf();
        fs::create_dir(root.join("ready")).map_err(|error| error.to_string())?;
        fs::create_dir(root.join("completed")).map_err(|error| error.to_string())?;
        write_canonical_new(
            &root.join("barrier.json"),
            &BarrierDefinitionV1 {
                schema: BARRIER_SCHEMA_V1.to_owned(),
                participants: participants.clone(),
            },
        )?;
        Ok(Self { root, participants })
    }

    pub fn wait_until_ready(&self, timeout: Duration) -> Result<(), String> {
        wait_until(timeout, || {
            self.participants.iter().all(|participant| {
                self.root
                    .join("ready")
                    .join(format!("{participant}.json"))
                    .is_file()
            })
        })
    }

    pub fn release(&self) -> Result<u64, String> {
        self.wait_until_ready(Duration::from_secs(30))?;
        let release_unix_nanos = unix_nanos()?;
        write_canonical_atomic_new(
            &self.root.join("release.json"),
            &BarrierReleaseV1 {
                schema: BARRIER_SCHEMA_V1.to_owned(),
                release_unix_nanos,
            },
        )?;
        Ok(release_unix_nanos)
    }

    pub fn evidence(&self) -> Result<QualificationProcessOverlapEvidenceV1, String> {
        let release: BarrierReleaseV1 = read_json(&self.root.join("release.json"))?;
        let mut participants = Vec::with_capacity(self.participants.len());
        for participant in &self.participants {
            participants.push(read_json(
                &self
                    .root
                    .join("completed")
                    .join(format!("{participant}.json")),
            )?);
        }
        let evidence = QualificationProcessOverlapEvidenceV1 {
            release_unix_nanos: release.release_unix_nanos,
            participants,
        };
        evidence.validate_overlap()?;
        Ok(evidence)
    }
}

#[derive(Debug)]
pub struct QualificationProcessBarrierParticipantV1 {
    root: PathBuf,
    participant: String,
    process_id: u32,
    ready_unix_nanos: u64,
}

impl QualificationProcessBarrierParticipantV1 {
    pub fn join(root: impl AsRef<Path>, participant: &str) -> Result<Self, String> {
        let root = root.as_ref().to_path_buf();
        let participant = validate_participant(participant)?.to_owned();
        let definition: BarrierDefinitionV1 = read_json(&root.join("barrier.json"))?;
        if definition.schema != BARRIER_SCHEMA_V1 || !definition.participants.contains(&participant)
        {
            return Err(format!(
                "participant {participant} is not in this process barrier"
            ));
        }
        let process_id = std::process::id();
        let ready_unix_nanos = unix_nanos()?;
        write_canonical_new(
            &root.join("ready").join(format!("{participant}.json")),
            &BarrierReadyV1 {
                schema: BARRIER_SCHEMA_V1.to_owned(),
                participant: participant.clone(),
                process_id,
                ready_unix_nanos,
            },
        )?;
        Ok(Self {
            root,
            participant,
            process_id,
            ready_unix_nanos,
        })
    }

    pub fn wait_for_release(&self, timeout: Duration) -> Result<u64, String> {
        let path = self.root.join("release.json");
        wait_until(timeout, || path.is_file())?;
        let release: BarrierReleaseV1 = read_json(&path)?;
        Ok(release.release_unix_nanos)
    }

    pub fn complete(self) -> Result<(), String> {
        write_canonical_new(
            &self
                .root
                .join("completed")
                .join(format!("{}.json", self.participant)),
            &QualificationBarrierParticipantEvidenceV1 {
                participant: self.participant,
                process_id: self.process_id,
                ready_unix_nanos: self.ready_unix_nanos,
                completed_unix_nanos: unix_nanos()?,
            },
        )
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct BarrierDefinitionV1 {
    schema: String,
    participants: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct BarrierReadyV1 {
    schema: String,
    participant: String,
    process_id: u32,
    ready_unix_nanos: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct BarrierReleaseV1 {
    schema: String,
    release_unix_nanos: u64,
}

fn validate_inventory(inventory: &QualificationInventoryV1, entry_id: &str) -> Result<(), String> {
    if inventory.carriers.is_empty()
        || inventory.encoded_bytes == 0
        || inventory.high_water_bytes < inventory.allocated_bytes
    {
        return Err(format!(
            "result {entry_id} has an incomplete inventory sidecar"
        ));
    }
    if inventory
        .carriers
        .windows(2)
        .any(|window| window[0].as_bytes() >= window[1].as_bytes())
    {
        return Err(format!(
            "result {entry_id} has an invalid carrier inventory"
        ));
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

fn validate_participant(participant: &str) -> Result<&str, String> {
    if participant.is_empty()
        || !participant
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("process barrier participant contains unsupported characters".to_owned());
    }
    Ok(participant)
}

fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) -> Result<(), String> {
    let started = Instant::now();
    while !predicate() {
        if started.elapsed() >= timeout {
            return Err(format!(
                "timed out after {} ms waiting for process barrier",
                timeout.as_millis()
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

fn unix_nanos() -> Result<u64, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    u64::try_from(nanos).map_err(|_| "system time exceeds qualification timestamp range".to_owned())
}

fn hash_canonical(value: &impl Serialize) -> Result<String, String> {
    let value = serde_json::to_value(value).map_err(|error| error.to_string())?;
    let bytes = canonical_json_bytes(&value).map_err(|error| error.to_string())?;
    Ok(sha256_bytes_hex(&bytes))
}

fn write_canonical_new(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let value = serde_json::to_value(value).map_err(|error| error.to_string())?;
    let bytes = canonical_json_bytes(&value).map_err(|error| error.to_string())?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn write_canonical_atomic_new(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            format!(
                "qualification path has no UTF-8 file name: {}",
                path.display()
            )
        })?;
    let temporary = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
    write_canonical_new(&temporary, value)?;
    fs::rename(&temporary, path).map_err(|error| {
        format!(
            "failed to publish {} from {}: {error}",
            path.display(),
            temporary.display()
        )
    })
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))
}

#[derive(Clone, Debug)]
pub struct QualificationRunConfigurationV1 {
    pub executable: PathBuf,
    pub root: PathBuf,
    pub source_commit: String,
    pub cargo_lock_sha256: String,
    pub performance_samples: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationCompletenessReportV1 {
    pub schema: String,
    pub required_results: u64,
    pub recorded_results: u64,
    pub passed_results: u64,
    pub failed_results: u64,
    pub skipped_results: u64,
    pub unsupported_results: u64,
    pub matrix_complete: bool,
    pub all_results_passed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationPlatformReportV1 {
    pub schema: String,
    pub plan: QualificationPlanV1,
    pub evidence: QualificationEvidenceV1,
    pub completeness: QualificationCompletenessReportV1,
}

impl QualificationEvidenceV1 {
    pub fn completeness_report(
        &self,
        plan: &QualificationPlanV1,
    ) -> Result<QualificationCompletenessReportV1, String> {
        self.validate(plan)?;
        let passed_results = self
            .results
            .iter()
            .filter(|result| result.outcome == QualificationScenarioOutcomeV1::Passed)
            .count() as u64;
        let failed_results = self.results.len() as u64 - passed_results;
        let skipped_results = self
            .results
            .iter()
            .filter(|result| result.execution == QualificationExecutionDispositionV1::Skipped)
            .count() as u64;
        let unsupported_results = self
            .results
            .iter()
            .filter(|result| result.execution == QualificationExecutionDispositionV1::Unsupported)
            .count() as u64;
        let matrix_complete = self.results.len() == plan.entries.len();
        Ok(QualificationCompletenessReportV1 {
            schema: "pointbreak.qualification-completeness.v1".to_owned(),
            required_results: plan.entries.len() as u64,
            recorded_results: self.results.len() as u64,
            passed_results,
            failed_results,
            skipped_results,
            unsupported_results,
            matrix_complete,
            all_results_passed: matrix_complete && failed_results == 0,
        })
    }
}

pub fn qualification_source_commit() -> Result<String, String> {
    let commit = option_env!("POINTBREAK_BUILD_COMMIT")
        .unwrap_or_default()
        .trim();
    validate_hex(commit, 40, "build source commit")?;
    Ok(commit.to_owned())
}

pub fn qualification_cargo_lock_sha256() -> String {
    sha256_bytes_hex(include_bytes!("../../../Cargo.lock"))
}

pub fn run_qualification_platform_matrix(
    configuration: &QualificationRunConfigurationV1,
) -> Result<QualificationPlatformReportV1, String> {
    if configuration.performance_samples == 0 {
        return Err("qualification requires at least one performance sample".to_owned());
    }
    if !configuration.executable.is_file() {
        return Err(format!(
            "qualification executable does not exist: {}",
            configuration.executable.display()
        ));
    }
    let plan = QualificationPlanV1::required(
        &configuration.source_commit,
        &configuration.cargo_lock_sha256,
    );
    plan.validate()?;
    fs::create_dir(&configuration.root).map_err(|error| {
        format!(
            "qualification root must be a fresh path ({}): {error}",
            configuration.root.display()
        )
    })?;
    let filesystem = qualification_filesystem_name(&configuration.root);
    let filesystem_disposition = classify_qualification_filesystem(&filesystem);
    let environment = QualificationPlatformEnvironmentV1 {
        operating_system: std::env::consts::OS.to_owned(),
        architecture: std::env::consts::ARCH.to_owned(),
        filesystem,
        filesystem_disposition,
        allocation_method: native_allocation_method().to_owned(),
        rustc: rustc_version(),
        build_source: env!("POINTBREAK_BUILD_SOURCE").to_owned(),
        build_describe: env!("POINTBREAK_BUILD_DESCRIBE").to_owned(),
        source_tree_dirty: env!("POINTBREAK_BUILD_DIRTY") == "true",
    };
    let legacy = synthetic_legacy_manifest().map_err(|error| error.to_string())?;
    let modeled = modeled_post_foundation_manifest().map_err(|error| error.to_string())?;
    let mut results = Vec::with_capacity(plan.entries.len());
    for entry in &plan.entries {
        let workload = match entry.workload {
            QualificationWorkloadV1::SyntheticLegacy => &legacy,
            QualificationWorkloadV1::ModeledFoundation => &modeled,
        };
        results.push(run_qualification_entry(
            configuration,
            &plan,
            entry,
            workload,
            &environment,
        )?);
    }
    let evidence = QualificationEvidenceV1 {
        schema: QUALIFICATION_EVIDENCE_SCHEMA_V1.to_owned(),
        plan_sha256: plan.plan_sha256.clone(),
        results,
    };
    let completeness = evidence.completeness_report(&plan)?;
    Ok(QualificationPlatformReportV1 {
        schema: "pointbreak.qualification-platform-report.v1".to_owned(),
        plan,
        evidence,
        completeness,
    })
}

fn run_qualification_entry(
    configuration: &QualificationRunConfigurationV1,
    plan: &QualificationPlanV1,
    entry: &QualificationPlanEntryV1,
    workload: &QualificationCorpusManifestV1,
    environment: &QualificationPlatformEnvironmentV1,
) -> Result<QualificationScenarioEvidenceV1, String> {
    let entry_root = configuration
        .root
        .join("scenarios")
        .join(entry.entry_id.replace(':', "-"));
    fs::create_dir_all(&entry_root).map_err(|error| error.to_string())?;
    if environment.filesystem_disposition
        != QualificationFilesystemDispositionV1::LocalProofEligible
    {
        return Ok(QualificationScenarioEvidenceV1 {
            entry_id: entry.entry_id.clone(),
            candidate: entry.candidate,
            workload: entry.workload,
            scenario: entry.scenario,
            platform_requirement: entry.platform_requirement,
            source_commit: plan.source_commit.clone(),
            candidate_build_id: entry.candidate_build_id.clone(),
            workload_manifest_sha256: workload.manifest_sha256.clone(),
            fault_seed: entry.fault_seed,
            kill_point: entry.kill_point.clone(),
            execution: QualificationExecutionDispositionV1::Unsupported,
            outcome: QualificationScenarioOutcomeV1::Failed,
            environment: environment.clone(),
            raw_samples: vec![QualificationRawSampleV1 {
                operation: "filesystem_classification".to_owned(),
                iteration: 0,
                elapsed_nanos: 1,
            }],
            inventory: None,
            baseline_inventory: None,
            process_overlap: None,
            failure: Some(format!(
                "filesystem {} is {:?}; writable qualification was not attempted",
                environment.filesystem, environment.filesystem_disposition
            )),
        });
    }

    let candidate_root = entry_root.join("candidate");
    let profile = open_candidate(entry.candidate, &candidate_root)?;
    populate_profile(profile.as_profile(), workload)?;
    let initial_inventory = profile.as_profile().inventory()?;
    drop(profile);
    let started = Instant::now();
    let run = run_scenario(
        configuration,
        entry,
        workload,
        &entry_root,
        &candidate_root,
        initial_inventory.clone(),
    );
    let elapsed = elapsed_nanos(started);
    let (outcome, failure, artifacts) = match run {
        Ok(artifacts) if artifacts.gate_failure.is_some() => (
            QualificationScenarioOutcomeV1::Failed,
            artifacts.gate_failure.clone(),
            artifacts,
        ),
        Ok(artifacts) => (QualificationScenarioOutcomeV1::Passed, None, artifacts),
        Err(error) => (
            QualificationScenarioOutcomeV1::Failed,
            Some(error),
            ScenarioArtifactsV1 {
                raw_samples: vec![QualificationRawSampleV1 {
                    operation: entry.scenario.as_str().to_owned(),
                    iteration: 0,
                    elapsed_nanos: elapsed,
                }],
                inventory: Some(initial_inventory),
                baseline_inventory: None,
                process_overlap: None,
                gate_failure: None,
            },
        ),
    };
    let mut raw_samples = artifacts.raw_samples;
    if raw_samples.is_empty() {
        raw_samples.push(QualificationRawSampleV1 {
            operation: entry.scenario.as_str().to_owned(),
            iteration: 0,
            elapsed_nanos: elapsed,
        });
    }
    let inventory = artifacts
        .inventory
        .map(|inventory| {
            native_inventory(
                &candidate_root,
                inventory.logical_bytes,
                inventory.high_water_bytes,
            )
        })
        .transpose()?;
    Ok(QualificationScenarioEvidenceV1 {
        entry_id: entry.entry_id.clone(),
        candidate: entry.candidate,
        workload: entry.workload,
        scenario: entry.scenario,
        platform_requirement: entry.platform_requirement,
        source_commit: plan.source_commit.clone(),
        candidate_build_id: entry.candidate_build_id.clone(),
        workload_manifest_sha256: workload.manifest_sha256.clone(),
        fault_seed: entry.fault_seed,
        kill_point: entry.kill_point.clone(),
        execution: QualificationExecutionDispositionV1::Executed,
        outcome,
        environment: environment.clone(),
        raw_samples,
        inventory,
        baseline_inventory: artifacts.baseline_inventory,
        process_overlap: artifacts.process_overlap,
        failure,
    })
}

struct ScenarioArtifactsV1 {
    raw_samples: Vec<QualificationRawSampleV1>,
    inventory: Option<QualificationInventoryV1>,
    baseline_inventory: Option<QualificationInventoryV1>,
    process_overlap: Option<QualificationProcessOverlapEvidenceV1>,
    gate_failure: Option<String>,
}

fn run_scenario(
    configuration: &QualificationRunConfigurationV1,
    entry: &QualificationPlanEntryV1,
    workload: &QualificationCorpusManifestV1,
    entry_root: &Path,
    candidate_root: &Path,
    initial_inventory: QualificationInventoryV1,
) -> Result<ScenarioArtifactsV1, String> {
    let mut artifacts = match entry.scenario {
        QualificationScenarioV1::CreateOnceRace => run_create_once_race(
            &configuration.executable,
            entry.candidate,
            entry_root,
            candidate_root,
        )?,
        QualificationScenarioV1::StableReaderWriter => run_stable_reader_writer(
            &configuration.executable,
            entry.candidate,
            entry_root,
            candidate_root,
        )?,
        QualificationScenarioV1::AcknowledgedWriteReopen => run_kill_reopen(
            &configuration.executable,
            entry.candidate,
            entry_root,
            candidate_root,
            false,
        )?,
        QualificationScenarioV1::AmbiguousAcknowledgementRetry => run_kill_reopen(
            &configuration.executable,
            entry.candidate,
            entry_root,
            candidate_root,
            true,
        )?,
        QualificationScenarioV1::CorruptionDetection => {
            run_corruption_detection(entry.candidate, candidate_root, initial_inventory.clone())?
        }
        QualificationScenarioV1::MaintenanceInterruption => run_maintenance_interruption(
            &configuration.executable,
            entry.candidate,
            entry_root,
            candidate_root,
        )?,
        QualificationScenarioV1::BackupWriterOverlap => run_backup_writer_overlap(
            &configuration.executable,
            entry.candidate,
            entry_root,
            candidate_root,
        )?,
        QualificationScenarioV1::CopyOutRepair => {
            run_copy_out_repair(entry.candidate, entry_root, candidate_root, workload)?
        }
        QualificationScenarioV1::AllocationFailure => {
            run_allocation_failure(entry.candidate, candidate_root, initial_inventory.clone())?
        }
        QualificationScenarioV1::Performance => run_performance_samples(
            entry.candidate,
            entry_root,
            candidate_root,
            workload,
            configuration.performance_samples,
        )?,
        QualificationScenarioV1::LongPath => run_long_path(entry.candidate, entry_root, workload)?,
        QualificationScenarioV1::OpenHandleReplacement => {
            run_open_handle_replacement(entry.candidate, candidate_root)?
        }
        QualificationScenarioV1::FilesystemPolicy => {
            run_filesystem_policy(candidate_root, initial_inventory.clone())?
        }
    };
    if artifacts.inventory.is_none() {
        artifacts.inventory = Some(
            open_candidate(entry.candidate, candidate_root)?
                .as_profile()
                .inventory()?,
        );
    }
    Ok(artifacts)
}

enum CandidateProfileV1 {
    Sqlite(SqliteQualificationProfile),
    Segments(SegmentQualificationProfile),
}

impl CandidateProfileV1 {
    fn as_profile(&self) -> &dyn QualificationProfile {
        match self {
            Self::Sqlite(profile) => profile,
            Self::Segments(profile) => profile,
        }
    }
}

fn open_candidate(
    candidate: QualificationCandidateV1,
    root: &Path,
) -> Result<CandidateProfileV1, String> {
    match candidate {
        QualificationCandidateV1::SqliteWal => SqliteQualificationProfile::open(root)
            .map(CandidateProfileV1::Sqlite)
            .map_err(|error| error.to_string()),
        QualificationCandidateV1::BoundedSegments => SegmentQualificationProfile::open(root)
            .map(CandidateProfileV1::Segments)
            .map_err(|error| error.to_string()),
    }
}

fn populate_profile(
    profile: &dyn QualificationProfile,
    workload: &QualificationCorpusManifestV1,
) -> Result<(), String> {
    workload.validate().map_err(|error| error.to_string())?;
    for record in &workload.records {
        let outcome = match record.record_kind {
            QualificationRecordKindV1::LegacyEvent
            | QualificationRecordKindV1::GenerationProposal
            | QualificationRecordKindV1::RelationAttestation
            | QualificationRecordKindV1::FactPort => profile
                .journal()
                .create_once(&record.logical_key, &record.decoded_bytes)?,
            QualificationRecordKindV1::ObjectArtifact
            | QualificationRecordKindV1::NoteBody
            | QualificationRecordKindV1::RelationProof
            | QualificationRecordKindV1::DocumentManifest
            | QualificationRecordKindV1::DocumentBlob => profile.put_content_once(
                &record.logical_key,
                record.record_kind,
                &record.decoded_bytes,
            )?,
        };
        if outcome != QualificationCreateOutcome::Created {
            return Err(format!(
                "workload record {} was not created exactly once",
                record.logical_key
            ));
        }
    }
    profile.journal().integrity_check()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationChildRequestV1 {
    candidate: QualificationCandidateV1,
    candidate_root: PathBuf,
    barrier_root: Option<PathBuf>,
    participant: String,
    operation: QualificationChildOperationV1,
    result_path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum QualificationChildOperationV1 {
    CreateOnce {
        logical_key: String,
        value: Vec<u8>,
    },
    StableReader {
        initial_key: String,
        concurrent_key: String,
        concurrent_value: Vec<u8>,
    },
    AcknowledgedWriter {
        logical_key: String,
        value: Vec<u8>,
        committed_marker: PathBuf,
    },
    Backup {
        destination: PathBuf,
    },
    InterruptedMaintenance {
        committed_marker: PathBuf,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct QualificationChildResultV1 {
    outcome: String,
    message: Option<String>,
}

pub fn run_qualification_child(request_path: &Path) -> Result<(), String> {
    let request: QualificationChildRequestV1 = read_json(request_path)?;
    let profile = open_candidate(request.candidate, &request.candidate_root)?;
    let participant = request
        .barrier_root
        .as_ref()
        .map(|root| QualificationProcessBarrierParticipantV1::join(root, &request.participant))
        .transpose()?;
    if let Some(participant) = &participant {
        participant.wait_for_release(Duration::from_secs(30))?;
    }
    let result = match request.operation {
        QualificationChildOperationV1::CreateOnce { logical_key, value } => {
            match profile
                .as_profile()
                .journal()
                .create_once(&logical_key, &value)
            {
                Ok(QualificationCreateOutcome::Created) => QualificationChildResultV1 {
                    outcome: "created".to_owned(),
                    message: None,
                },
                Ok(QualificationCreateOutcome::AlreadyExists) => QualificationChildResultV1 {
                    outcome: "already_exists".to_owned(),
                    message: None,
                },
                Err(error) => QualificationChildResultV1 {
                    outcome: "conflict".to_owned(),
                    message: Some(error),
                },
            }
        }
        QualificationChildOperationV1::StableReader {
            initial_key,
            concurrent_key,
            concurrent_value,
        } => {
            if profile.as_profile().journal().read(&initial_key)?.is_none() {
                return Err(format!("stable reader omitted initial key {initial_key}"));
            }
            let started = Instant::now();
            loop {
                if profile
                    .as_profile()
                    .journal()
                    .read(&concurrent_key)?
                    .is_some_and(|entry| entry.decoded_bytes == concurrent_value)
                {
                    break QualificationChildResultV1 {
                        outcome: "observed".to_owned(),
                        message: None,
                    };
                }
                if started.elapsed() > Duration::from_secs(10) {
                    return Err(format!(
                        "stable reader did not observe concurrent key {concurrent_key}"
                    ));
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
        QualificationChildOperationV1::AcknowledgedWriter {
            logical_key,
            value,
            committed_marker,
        } => {
            let outcome = profile
                .as_profile()
                .journal()
                .create_once(&logical_key, &value)?;
            if outcome != QualificationCreateOutcome::Created {
                return Err("acknowledged writer did not create its record".to_owned());
            }
            write_marker(&committed_marker)?;
            thread::sleep(Duration::from_secs(30));
            QualificationChildResultV1 {
                outcome: "unexpected_resume".to_owned(),
                message: None,
            }
        }
        QualificationChildOperationV1::Backup { destination } => {
            profile.as_profile().backup_to(&destination)?;
            profile.as_profile().verify_restore(&destination)?;
            QualificationChildResultV1 {
                outcome: "backup_verified".to_owned(),
                message: None,
            }
        }
        QualificationChildOperationV1::InterruptedMaintenance { committed_marker } => {
            match profile {
                CandidateProfileV1::Sqlite(profile) => {
                    profile
                        .checkpoint_with_hook(|| {
                            write_marker(&committed_marker)
                                .expect("maintenance marker must publish before the parent kills");
                            thread::sleep(Duration::from_secs(30));
                        })
                        .map_err(|error| error.to_string())?;
                }
                CandidateProfileV1::Segments(profile) => {
                    let result = profile
                        .seal_active_with_failure(SegmentFailurePointV1::AfterGenerationPublish);
                    if result.is_ok() {
                        return Err("segment maintenance fault did not interrupt".to_owned());
                    }
                    write_marker(&committed_marker)?;
                    thread::sleep(Duration::from_secs(30));
                }
            }
            QualificationChildResultV1 {
                outcome: "unexpected_resume".to_owned(),
                message: None,
            }
        }
    };
    if let Some(participant) = participant {
        participant.complete()?;
    }
    write_canonical_new(&request.result_path, &result)
}

fn run_create_once_race(
    executable: &Path,
    candidate: QualificationCandidateV1,
    entry_root: &Path,
    candidate_root: &Path,
) -> Result<ScenarioArtifactsV1, String> {
    let barrier_root = entry_root.join("race-barrier");
    fs::create_dir(&barrier_root).map_err(|error| error.to_string())?;
    let names = ["writer0", "writer1", "writer2", "writer3"];
    let barrier = QualificationProcessBarrierV1::create(&barrier_root, &names)?;
    let mut children = Vec::new();
    let results_root = entry_root.join("child-results");
    fs::create_dir(&results_root).map_err(|error| error.to_string())?;
    for (index, participant) in names.iter().enumerate() {
        let result_path = results_root.join(format!("{participant}.json"));
        let value = if index < 2 { b"race-a" } else { b"race-b" };
        children.push(spawn_child(
            executable,
            entry_root,
            participant,
            QualificationChildRequestV1 {
                candidate,
                candidate_root: candidate_root.to_path_buf(),
                barrier_root: Some(barrier_root.clone()),
                participant: (*participant).to_owned(),
                operation: QualificationChildOperationV1::CreateOnce {
                    logical_key: "qualification/race".to_owned(),
                    value: value.to_vec(),
                },
                result_path,
            },
        )?);
    }
    barrier.wait_until_ready(Duration::from_secs(30))?;
    let started = Instant::now();
    barrier.release()?;
    wait_children(&mut children, Duration::from_secs(30))?;
    let overlap = barrier.evidence()?;
    let results = names
        .iter()
        .map(|name| {
            read_json::<QualificationChildResultV1>(&results_root.join(format!("{name}.json")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if results
        .iter()
        .filter(|result| result.outcome == "created")
        .count()
        != 1
    {
        return Err("create-once race did not produce exactly one winner".to_owned());
    }
    let profile = open_candidate(candidate, candidate_root)?;
    let winner = profile
        .as_profile()
        .journal()
        .read("qualification/race")?
        .ok_or_else(|| "create-once race winner is not durable".to_owned())?;
    if winner.decoded_bytes != b"race-a" && winner.decoded_bytes != b"race-b" {
        return Err("create-once race stored unexpected bytes".to_owned());
    }
    let expected_idempotent = 1;
    if results
        .iter()
        .filter(|result| result.outcome == "already_exists")
        .count()
        != expected_idempotent
        || results
            .iter()
            .filter(|result| result.outcome == "conflict")
            .count()
            != 2
    {
        return Err(
            "create-once race did not preserve winner/idempotent/conflict semantics".to_owned(),
        );
    }
    Ok(ScenarioArtifactsV1 {
        raw_samples: vec![sample("create_once_race", 0, elapsed_nanos(started))],
        inventory: Some(profile.as_profile().inventory()?),
        baseline_inventory: None,
        process_overlap: Some(overlap),
        gate_failure: None,
    })
}

fn run_stable_reader_writer(
    executable: &Path,
    candidate: QualificationCandidateV1,
    entry_root: &Path,
    candidate_root: &Path,
) -> Result<ScenarioArtifactsV1, String> {
    let initial_key = open_candidate(candidate, candidate_root)?
        .as_profile()
        .journal()
        .list()?
        .first()
        .ok_or_else(|| "workload has no journal record for stable-reader scenario".to_owned())?
        .logical_key
        .clone();
    let barrier_root = entry_root.join("reader-writer-barrier");
    fs::create_dir(&barrier_root).map_err(|error| error.to_string())?;
    let barrier = QualificationProcessBarrierV1::create(&barrier_root, &["reader"])?;
    let result_path = entry_root.join("reader-result.json");
    let mut child = spawn_child(
        executable,
        entry_root,
        "reader",
        QualificationChildRequestV1 {
            candidate,
            candidate_root: candidate_root.to_path_buf(),
            barrier_root: Some(barrier_root),
            participant: "reader".to_owned(),
            operation: QualificationChildOperationV1::StableReader {
                initial_key,
                concurrent_key: "qualification/concurrent-writer".to_owned(),
                concurrent_value: b"concurrent".to_vec(),
            },
            result_path: result_path.clone(),
        },
    )?;
    barrier.wait_until_ready(Duration::from_secs(30))?;
    let started = Instant::now();
    barrier.release()?;
    let writer = open_candidate(candidate, candidate_root)?;
    if writer
        .as_profile()
        .journal()
        .create_once("qualification/concurrent-writer", b"concurrent")?
        != QualificationCreateOutcome::Created
    {
        return Err("concurrent writer did not create its record".to_owned());
    }
    wait_child(&mut child, Duration::from_secs(30))?;
    let result: QualificationChildResultV1 = read_json(&result_path)?;
    if result.outcome != "observed" {
        return Err("stable reader did not observe the concurrent durable write".to_owned());
    }
    Ok(ScenarioArtifactsV1 {
        raw_samples: vec![sample("stable_reader_writer", 0, elapsed_nanos(started))],
        inventory: Some(writer.as_profile().inventory()?),
        baseline_inventory: None,
        process_overlap: Some(barrier.evidence()?),
        gate_failure: None,
    })
}

fn run_kill_reopen(
    executable: &Path,
    candidate: QualificationCandidateV1,
    entry_root: &Path,
    candidate_root: &Path,
    ambiguous: bool,
) -> Result<ScenarioArtifactsV1, String> {
    let key = if ambiguous {
        "qualification/ambiguous"
    } else {
        "qualification/acknowledged"
    };
    let marker = entry_root.join("committed.marker");
    let mut child = spawn_child(
        executable,
        entry_root,
        "kill-writer",
        QualificationChildRequestV1 {
            candidate,
            candidate_root: candidate_root.to_path_buf(),
            barrier_root: None,
            participant: "kill-writer".to_owned(),
            operation: QualificationChildOperationV1::AcknowledgedWriter {
                logical_key: key.to_owned(),
                value: b"durable".to_vec(),
                committed_marker: marker.clone(),
            },
            result_path: entry_root.join("unreachable-child-result.json"),
        },
    )?;
    wait_until(Duration::from_secs(30), || marker.is_file())?;
    let started = Instant::now();
    child.kill().map_err(|error| error.to_string())?;
    child.wait().map_err(|error| error.to_string())?;
    let profile = open_candidate(candidate, candidate_root)?;
    let durable = profile
        .as_profile()
        .journal()
        .read(key)?
        .is_some_and(|entry| entry.decoded_bytes == b"durable");
    if !durable {
        return Err(format!("killed writer lost durable key {key}"));
    }
    if ambiguous
        && profile
            .as_profile()
            .journal()
            .create_once(key, b"durable")?
            != QualificationCreateOutcome::AlreadyExists
    {
        return Err("ambiguous acknowledgement retry was not idempotent".to_owned());
    }
    Ok(ScenarioArtifactsV1 {
        raw_samples: vec![sample(
            if ambiguous {
                "ambiguous_retry"
            } else {
                "acknowledged_kill_reopen"
            },
            0,
            elapsed_nanos(started),
        )],
        inventory: Some(profile.as_profile().inventory()?),
        baseline_inventory: None,
        process_overlap: None,
        gate_failure: None,
    })
}

fn run_corruption_detection(
    candidate: QualificationCandidateV1,
    candidate_root: &Path,
    inventory: QualificationInventoryV1,
) -> Result<ScenarioArtifactsV1, String> {
    let carrier = match candidate {
        QualificationCandidateV1::SqliteWal => "journal.sqlite3".to_owned(),
        QualificationCandidateV1::BoundedSegments => inventory
            .carriers
            .iter()
            .find(|carrier| carrier.starts_with("active/"))
            .cloned()
            .ok_or_else(|| "segment inventory omitted its active carrier".to_owned())?,
    };
    let started = Instant::now();
    let path = candidate_root.join(carrier);
    let mut file = OpenOptions::new()
        .write(true)
        .open(&path)
        .map_err(|error| error.to_string())?;
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.write_all(b"!"))
        .and_then(|_| file.sync_all())
        .map_err(|error| error.to_string())?;
    let loud = match candidate {
        QualificationCandidateV1::SqliteWal => !matches!(
            SqliteQualificationProfile::diagnose_root(candidate_root),
            SqliteDiagnosticStateV1::Healthy
        ),
        QualificationCandidateV1::BoundedSegments => !matches!(
            SegmentQualificationProfile::diagnose_root(candidate_root),
            SegmentDiagnosticStateV1::Healthy
        ),
    };
    if !loud {
        return Err("candidate treated deliberate carrier corruption as healthy".to_owned());
    }
    Ok(ScenarioArtifactsV1 {
        raw_samples: vec![sample("corruption_diagnosis", 0, elapsed_nanos(started))],
        inventory: Some(inventory),
        baseline_inventory: None,
        process_overlap: None,
        gate_failure: None,
    })
}

fn run_maintenance_interruption(
    executable: &Path,
    candidate: QualificationCandidateV1,
    entry_root: &Path,
    candidate_root: &Path,
) -> Result<ScenarioArtifactsV1, String> {
    let marker = entry_root.join("maintenance.marker");
    let mut child = spawn_child(
        executable,
        entry_root,
        "maintenance",
        QualificationChildRequestV1 {
            candidate,
            candidate_root: candidate_root.to_path_buf(),
            barrier_root: None,
            participant: "maintenance".to_owned(),
            operation: QualificationChildOperationV1::InterruptedMaintenance {
                committed_marker: marker.clone(),
            },
            result_path: entry_root.join("unreachable-maintenance-result.json"),
        },
    )?;
    wait_until(Duration::from_secs(30), || marker.is_file())?;
    let started = Instant::now();
    child.kill().map_err(|error| error.to_string())?;
    child.wait().map_err(|error| error.to_string())?;
    let profile = open_candidate(candidate, candidate_root)?;
    match &profile {
        CandidateProfileV1::Sqlite(profile) => {
            profile
                .recover_interrupted_checkpoint()
                .map_err(|error| error.to_string())?;
            profile.checkpoint().map_err(|error| error.to_string())?;
        }
        CandidateProfileV1::Segments(profile) => {
            profile.seal_active().map_err(|error| error.to_string())?;
        }
    }
    profile.as_profile().journal().integrity_check()?;
    Ok(ScenarioArtifactsV1 {
        raw_samples: vec![sample(
            "maintenance_kill_recovery",
            0,
            elapsed_nanos(started),
        )],
        inventory: Some(profile.as_profile().inventory()?),
        baseline_inventory: None,
        process_overlap: None,
        gate_failure: None,
    })
}

fn run_backup_writer_overlap(
    executable: &Path,
    candidate: QualificationCandidateV1,
    entry_root: &Path,
    candidate_root: &Path,
) -> Result<ScenarioArtifactsV1, String> {
    let barrier_root = entry_root.join("backup-writer-barrier");
    fs::create_dir(&barrier_root).map_err(|error| error.to_string())?;
    let barrier = QualificationProcessBarrierV1::create(&barrier_root, &["backup"])?;
    let result_path = entry_root.join("backup-result.json");
    let mut child = spawn_child(
        executable,
        entry_root,
        "backup",
        QualificationChildRequestV1 {
            candidate,
            candidate_root: candidate_root.to_path_buf(),
            barrier_root: Some(barrier_root),
            participant: "backup".to_owned(),
            operation: QualificationChildOperationV1::Backup {
                destination: entry_root.join("backup"),
            },
            result_path: result_path.clone(),
        },
    )?;
    barrier.wait_until_ready(Duration::from_secs(30))?;
    let started = Instant::now();
    barrier.release()?;
    let writer = open_candidate(candidate, candidate_root)?;
    let concurrent_write = writer
        .as_profile()
        .journal()
        .create_once("qualification/backup-overlap", b"writer");
    wait_child(&mut child, Duration::from_secs(30))?;
    let result: QualificationChildResultV1 = read_json(&result_path)?;
    if result.outcome != "backup_verified" {
        return Err("overlapping backup did not verify".to_owned());
    }
    match concurrent_write {
        Ok(QualificationCreateOutcome::Created) => {}
        Ok(QualificationCreateOutcome::AlreadyExists) => {
            return Err("backup overlap writer unexpectedly found an existing record".to_owned());
        }
        Err(error)
            if candidate == QualificationCandidateV1::SqliteWal
                && error.contains("maintenance state is backing_up") =>
        {
            let retry = open_candidate(candidate, candidate_root)?;
            if retry
                .as_profile()
                .journal()
                .create_once("qualification/backup-overlap", b"writer")?
                != QualificationCreateOutcome::Created
            {
                return Err("SQLite writer did not recover after overlapping backup".to_owned());
            }
        }
        Err(error) => return Err(format!("overlapping writer failed unexpectedly: {error}")),
    }
    Ok(ScenarioArtifactsV1 {
        raw_samples: vec![sample("backup_writer_overlap", 0, elapsed_nanos(started))],
        inventory: Some(writer.as_profile().inventory()?),
        baseline_inventory: None,
        process_overlap: Some(barrier.evidence()?),
        gate_failure: None,
    })
}

fn run_copy_out_repair(
    candidate: QualificationCandidateV1,
    entry_root: &Path,
    candidate_root: &Path,
    workload: &QualificationCorpusManifestV1,
) -> Result<ScenarioArtifactsV1, String> {
    let started = Instant::now();
    let source = open_candidate(candidate, candidate_root)?;
    let destination_root = entry_root.join("repair");
    let destination = open_candidate(candidate, &destination_root)?;
    for entry in source.as_profile().journal().list()? {
        if destination
            .as_profile()
            .journal()
            .create_once(&entry.logical_key, &entry.decoded_bytes)?
            != QualificationCreateOutcome::Created
        {
            return Err("copy-out repair did not create a journal record".to_owned());
        }
    }
    for record in &workload.records {
        if matches!(
            record.record_kind,
            QualificationRecordKindV1::ObjectArtifact
                | QualificationRecordKindV1::NoteBody
                | QualificationRecordKindV1::RelationProof
                | QualificationRecordKindV1::DocumentManifest
                | QualificationRecordKindV1::DocumentBlob
        ) {
            let content = source
                .as_profile()
                .read_content(&record.logical_key)?
                .ok_or_else(|| format!("repair source omitted {}", record.logical_key))?;
            destination.as_profile().put_content_once(
                &record.logical_key,
                record.record_kind,
                &content.decoded_bytes,
            )?;
        }
    }
    if source.as_profile().journal().list()? != destination.as_profile().journal().list()? {
        return Err("copy-out repair changed journal records".to_owned());
    }
    source.as_profile().journal().integrity_check()?;
    destination.as_profile().journal().integrity_check()?;
    Ok(ScenarioArtifactsV1 {
        raw_samples: vec![sample("copy_out_repair", 0, elapsed_nanos(started))],
        inventory: Some(destination.as_profile().inventory()?),
        baseline_inventory: None,
        process_overlap: None,
        gate_failure: None,
    })
}

fn run_allocation_failure(
    candidate: QualificationCandidateV1,
    candidate_root: &Path,
    inventory: QualificationInventoryV1,
) -> Result<ScenarioArtifactsV1, String> {
    let logical_key = "qualification/allocation-failure";
    let started = Instant::now();
    match candidate {
        QualificationCandidateV1::SqliteWal => {
            let profile = SqliteQualificationProfile::open(candidate_root)
                .map_err(|error| error.to_string())?;
            profile
                .exercise_allocation_failure(logical_key)
                .map_err(|error| error.to_string())?;
            drop(profile);
            let reopened = open_candidate(candidate, candidate_root)?;
            if reopened.as_profile().journal().read(logical_key)?.is_none() {
                return Err("SQLite allocation-fault retry left no complete record".to_owned());
            }
            reopened.as_profile().journal().integrity_check()?;
        }
        QualificationCandidateV1::BoundedSegments => {
            let profile = SegmentQualificationProfile::open(candidate_root)
                .map_err(|error| error.to_string())?;
            if profile
                .create_once_with_failure(
                    logical_key,
                    b"short-write",
                    SegmentFailurePointV1::AfterRecordBytes,
                )
                .is_ok()
            {
                return Err("segment short-write fault did not interrupt allocation".to_owned());
            }
            drop(profile);
            let reopened = open_candidate(candidate, candidate_root)?;
            if reopened.as_profile().journal().read(logical_key)?.is_some() {
                return Err("segment allocation fault published a partial record".to_owned());
            }
            reopened.as_profile().journal().integrity_check()?;
            if reopened
                .as_profile()
                .journal()
                .create_once(logical_key, b"retry")?
                != QualificationCreateOutcome::Created
            {
                return Err("segment writer did not recover after the allocation fault".to_owned());
            }
        }
    }
    Ok(ScenarioArtifactsV1 {
        raw_samples: vec![sample(
            "allocation_failure_recovery",
            0,
            elapsed_nanos(started),
        )],
        inventory: Some(inventory),
        baseline_inventory: None,
        process_overlap: None,
        gate_failure: None,
    })
}

fn run_performance_samples(
    candidate: QualificationCandidateV1,
    entry_root: &Path,
    candidate_root: &Path,
    workload: &QualificationCorpusManifestV1,
    samples: u32,
) -> Result<ScenarioArtifactsV1, String> {
    let profile = open_candidate(candidate, candidate_root)?;
    let journal_key = profile
        .as_profile()
        .journal()
        .list()?
        .first()
        .ok_or_else(|| "performance workload has no journal record".to_owned())?
        .logical_key
        .clone();
    let baseline_root = entry_root.join("loose-baseline");
    fs::create_dir(&baseline_root).map_err(|error| error.to_string())?;
    for record in &workload.records {
        let path = baseline_record_path(&baseline_root, &record.logical_key, record.record_kind);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| error.to_string())?;
        file.write_all(&record.decoded_bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| error.to_string())?;
    }
    let baseline_key_path = workload
        .records
        .iter()
        .find(|record| record.logical_key == journal_key)
        .map(|record| baseline_record_path(&baseline_root, &record.logical_key, record.record_kind))
        .ok_or_else(|| "performance baseline omitted the selected journal key".to_owned())?;
    let mut raw_samples = Vec::new();
    for iteration in 0..samples {
        let candidate_key = format!("qualification/performance/{iteration:08}");
        let started = Instant::now();
        profile
            .as_profile()
            .journal()
            .create_once(&candidate_key, b"performance")?;
        raw_samples.push(sample(
            "candidate_durable_append",
            iteration,
            elapsed_nanos(started),
        ));

        let baseline_path = baseline_root
            .join("events")
            .join(format!("append-{iteration:08}.json"));
        let started = Instant::now();
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&baseline_path)
            .map_err(|error| error.to_string())?;
        file.write_all(b"performance")
            .and_then(|_| file.sync_all())
            .map_err(|error| error.to_string())?;
        raw_samples.push(sample(
            "baseline_durable_append",
            iteration,
            elapsed_nanos(started),
        ));

        let started = Instant::now();
        profile.as_profile().journal().list()?;
        raw_samples.push(sample(
            "candidate_replay",
            iteration,
            elapsed_nanos(started),
        ));

        let started = Instant::now();
        read_all_files(&baseline_root)?;
        raw_samples.push(sample("baseline_replay", iteration, elapsed_nanos(started)));

        let started = Instant::now();
        profile.as_profile().journal().read(&journal_key)?;
        raw_samples.push(sample(
            "candidate_keyed_read",
            iteration,
            elapsed_nanos(started),
        ));

        let started = Instant::now();
        let baseline_key_bytes = fs::read(&baseline_key_path).map_err(|error| error.to_string())?;
        std::hint::black_box(Sha256::digest(&baseline_key_bytes));
        raw_samples.push(sample(
            "baseline_keyed_read",
            iteration,
            elapsed_nanos(started),
        ));

        let started = Instant::now();
        let reopened = open_candidate(candidate, candidate_root)?;
        reopened.as_profile().journal().integrity_check()?;
        raw_samples.push(sample(
            "candidate_open_recovery",
            iteration,
            elapsed_nanos(started),
        ));

        let started = Instant::now();
        read_all_files(&baseline_root)?;
        raw_samples.push(sample(
            "baseline_open_recovery",
            iteration,
            elapsed_nanos(started),
        ));
    }
    let backup = entry_root.join("performance-backup");
    let started = Instant::now();
    profile.as_profile().backup_to(&backup)?;
    raw_samples.push(sample("candidate_backup", 0, elapsed_nanos(started)));
    let started = Instant::now();
    profile.as_profile().verify_restore(&backup)?;
    raw_samples.push(sample("candidate_restore", 0, elapsed_nanos(started)));
    let inventory = profile.as_profile().inventory()?;
    let baseline_logical = workload
        .records
        .iter()
        .try_fold(0_u64, |total, record| {
            total.checked_add(record.decoded_bytes.len() as u64)
        })
        .and_then(|total| total.checked_add(u64::from(samples) * b"performance".len() as u64))
        .ok_or_else(|| "baseline logical-byte total overflow".to_owned())?;
    if baseline_logical == 0 || inventory.logical_bytes == 0 {
        return Err("performance inventory omitted logical bytes".to_owned());
    }
    let gate_failure = (samples > 1)
        .then(|| performance_gate_failure(&raw_samples))
        .flatten();
    Ok(ScenarioArtifactsV1 {
        raw_samples,
        inventory: Some(inventory),
        baseline_inventory: Some(baseline_inventory(&baseline_root, baseline_logical)?),
        process_overlap: None,
        gate_failure,
    })
}

fn run_long_path(
    candidate: QualificationCandidateV1,
    entry_root: &Path,
    workload: &QualificationCorpusManifestV1,
) -> Result<ScenarioArtifactsV1, String> {
    let component = "qualification-long-path-component-0123456789abcdef";
    let mut root = entry_root.join("long-path");
    while root.as_os_str().to_string_lossy().len() < 300 {
        root = root.join(component);
    }
    let started = Instant::now();
    let profile = open_candidate(candidate, &root)?;
    populate_profile(profile.as_profile(), workload)?;
    profile.as_profile().journal().integrity_check()?;
    Ok(ScenarioArtifactsV1 {
        raw_samples: vec![sample("long_path", 0, elapsed_nanos(started))],
        inventory: Some(profile.as_profile().inventory()?),
        baseline_inventory: None,
        process_overlap: None,
        gate_failure: None,
    })
}

fn run_open_handle_replacement(
    candidate: QualificationCandidateV1,
    candidate_root: &Path,
) -> Result<ScenarioArtifactsV1, String> {
    let started = Instant::now();
    let inventory = match candidate {
        QualificationCandidateV1::SqliteWal => {
            let reader = SqliteQualificationProfile::open(candidate_root)
                .map_err(|error| error.to_string())?;
            let writer = SqliteQualificationProfile::open(candidate_root)
                .map_err(|error| error.to_string())?;
            writer
                .journal()
                .create_once("qualification/open-handle", b"sqlite")?;
            writer.checkpoint().map_err(|error| error.to_string())?;
            if reader
                .journal()
                .read("qualification/open-handle")?
                .is_none()
            {
                return Err(
                    "SQLite reader missed a write while its handle remained open".to_owned(),
                );
            }
            writer.inventory()?
        }
        QualificationCandidateV1::BoundedSegments => {
            let profile = SegmentQualificationProfile::open(candidate_root)
                .map_err(|error| error.to_string())?;
            profile.seal_active().map_err(|error| error.to_string())?;
            let pin = profile.pin_reader().map_err(|error| error.to_string())?;
            let generation = pin.generation();
            profile
                .journal()
                .create_once("qualification/open-handle", b"segments")?;
            profile.seal_active().map_err(|error| error.to_string())?;
            if profile.retire_generation(generation).is_ok() {
                return Err("segment generation retired while a reader handle was open".to_owned());
            }
            drop(pin);
            profile
                .retire_generation(generation)
                .map_err(|error| error.to_string())?;
            profile.inventory()?
        }
    };
    Ok(ScenarioArtifactsV1 {
        raw_samples: vec![sample("open_handle_replacement", 0, elapsed_nanos(started))],
        inventory: Some(inventory),
        baseline_inventory: None,
        process_overlap: None,
        gate_failure: None,
    })
}

fn run_filesystem_policy(
    candidate_root: &Path,
    inventory: QualificationInventoryV1,
) -> Result<ScenarioArtifactsV1, String> {
    let started = Instant::now();
    let actual = qualification_filesystem_name(candidate_root);
    if classify_qualification_filesystem(&actual)
        != QualificationFilesystemDispositionV1::LocalProofEligible
        || classify_qualification_filesystem("nfs") != QualificationFilesystemDispositionV1::Refused
        || classify_qualification_filesystem("OneDrive")
            != QualificationFilesystemDispositionV1::AdvisoryOnly
    {
        return Err("filesystem placement policy did not fail closed".to_owned());
    }
    Ok(ScenarioArtifactsV1 {
        raw_samples: vec![sample("filesystem_policy", 0, elapsed_nanos(started))],
        inventory: Some(inventory),
        baseline_inventory: None,
        process_overlap: None,
        gate_failure: None,
    })
}

fn spawn_child(
    executable: &Path,
    entry_root: &Path,
    label: &str,
    request: QualificationChildRequestV1,
) -> Result<Child, String> {
    let requests = entry_root.join("child-requests");
    fs::create_dir_all(&requests).map_err(|error| error.to_string())?;
    let request_path = requests.join(format!("{label}.json"));
    write_canonical_new(&request_path, &request)?;
    Command::new(executable)
        .arg("--qualification-child")
        .arg(&request_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to spawn qualification child {label}: {error}"))
}

fn wait_children(children: &mut [Child], timeout: Duration) -> Result<(), String> {
    for child in children {
        wait_child(child, timeout)?;
    }
    Ok(())
}

fn wait_child(child: &mut Child, timeout: Duration) -> Result<(), String> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            if status.success() {
                return Ok(());
            }
            let stderr = child
                .stderr
                .take()
                .and_then(|mut stderr| {
                    let mut message = String::new();
                    std::io::Read::read_to_string(&mut stderr, &mut message)
                        .ok()
                        .map(|_| message)
                })
                .unwrap_or_default();
            return Err(format!(
                "qualification child failed with {status}: {stderr}"
            ));
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "qualification child timed out after {} ms",
                timeout.as_millis()
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn write_marker(path: &Path) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(b"committed")
        .and_then(|_| file.sync_all())
        .map_err(|error| error.to_string())
}

fn baseline_record_path(
    root: &Path,
    logical_key: &str,
    record_kind: QualificationRecordKindV1,
) -> PathBuf {
    let directory = match record_kind {
        QualificationRecordKindV1::LegacyEvent
        | QualificationRecordKindV1::GenerationProposal
        | QualificationRecordKindV1::RelationAttestation
        | QualificationRecordKindV1::FactPort => "events",
        QualificationRecordKindV1::ObjectArtifact
        | QualificationRecordKindV1::NoteBody
        | QualificationRecordKindV1::RelationProof
        | QualificationRecordKindV1::DocumentManifest
        | QualificationRecordKindV1::DocumentBlob => "content",
    };
    root.join(directory)
        .join(format!("{}.json", sha256_bytes_hex(logical_key.as_bytes())))
}

fn read_all_files(root: &Path) -> Result<(), String> {
    for path in inventory_file_paths(root)? {
        let bytes = fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        std::hint::black_box(Sha256::digest(&bytes));
    }
    Ok(())
}

fn baseline_inventory(root: &Path, logical_bytes: u64) -> Result<QualificationInventoryV1, String> {
    native_inventory(root, logical_bytes, 0)
}

fn native_inventory(
    root: &Path,
    logical_bytes: u64,
    high_water_hint: u64,
) -> Result<QualificationInventoryV1, String> {
    let mut carriers = Vec::new();
    let mut encoded_bytes = 0_u64;
    let mut allocated_bytes = 0_u64;
    for path in inventory_file_paths(root)? {
        let metadata =
            fs::metadata(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        encoded_bytes = encoded_bytes
            .checked_add(metadata.len())
            .ok_or_else(|| "qualification encoded-byte inventory overflow".to_owned())?;
        allocated_bytes = allocated_bytes
            .checked_add(native_file_allocation(&path, &metadata)?)
            .ok_or_else(|| "qualification allocated-byte inventory overflow".to_owned())?;
        carriers.push(
            path.strip_prefix(root)
                .map_err(|error| error.to_string())?
                .to_string_lossy()
                .replace('\\', "/"),
        );
    }
    carriers.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    Ok(QualificationInventoryV1 {
        carriers,
        logical_bytes,
        encoded_bytes,
        allocated_bytes,
        high_water_bytes: high_water_hint.max(allocated_bytes),
    })
}

fn inventory_file_paths(root: &Path) -> Result<Vec<PathBuf>, String> {
    fn visit(root: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
        let mut entries = fs::read_dir(root)
            .map_err(|error| format!("{}: {error}", root.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let metadata = entry
                .metadata()
                .map_err(|error| format!("{}: {error}", path.display()))?;
            if metadata.is_dir() {
                visit(&path, paths)?;
            } else if metadata.is_file() {
                paths.push(path);
            }
        }
        Ok(())
    }

    let mut paths = Vec::new();
    visit(root, &mut paths)?;
    Ok(paths)
}

#[cfg(unix)]
fn native_file_allocation(_path: &Path, metadata: &fs::Metadata) -> Result<u64, String> {
    use std::os::unix::fs::MetadataExt;
    Ok(metadata.blocks().saturating_mul(512))
}

#[cfg(windows)]
fn native_file_allocation(path: &Path, _metadata: &fs::Metadata) -> Result<u64, String> {
    use std::os::windows::ffi::OsStrExt;

    unsafe extern "system" {
        fn GetCompressedFileSizeW(file_name: *const u16, file_size_high: *mut u32) -> u32;
    }

    let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
    wide.push(0);
    let mut high = 0_u32;
    // SAFETY: `wide` is NUL-terminated and remains alive for the call; `high`
    // is a valid writable u32 owned by this stack frame.
    let low = unsafe { GetCompressedFileSizeW(wide.as_ptr(), &mut high) };
    if low == u32::MAX {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error().is_some_and(|code| code != 0) {
            return Err(format!(
                "native allocation query failed for {}: {error}",
                path.display()
            ));
        }
    }
    Ok((u64::from(high) << 32) | u64::from(low))
}

#[cfg(not(any(unix, windows)))]
fn native_file_allocation(_path: &Path, metadata: &fs::Metadata) -> Result<u64, String> {
    Ok(metadata.len())
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

fn performance_gate_failure(samples: &[QualificationRawSampleV1]) -> Option<String> {
    let comparisons = [
        (
            "durable_append",
            "candidate_durable_append",
            "baseline_durable_append",
        ),
        ("replay", "candidate_replay", "baseline_replay"),
        ("keyed_read", "candidate_keyed_read", "baseline_keyed_read"),
        (
            "open_recovery",
            "candidate_open_recovery",
            "baseline_open_recovery",
        ),
    ];
    let mut failures = Vec::new();
    for (label, candidate, baseline) in comparisons {
        let candidate_p95 = sample_p95(samples, candidate)?;
        let baseline_p95 = sample_p95(samples, baseline)?;
        if u128::from(candidate_p95) * 100 > u128::from(baseline_p95) * 125 {
            failures.push(format!(
                "{label} p95 {candidate_p95}ns exceeds 125% of fresh loose baseline {baseline_p95}ns"
            ));
        }
    }
    (!failures.is_empty()).then(|| failures.join("; "))
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

fn sample(operation: &str, iteration: u32, elapsed_nanos: u64) -> QualificationRawSampleV1 {
    QualificationRawSampleV1 {
        operation: operation.to_owned(),
        iteration,
        elapsed_nanos: elapsed_nanos.max(1),
    }
}

fn elapsed_nanos(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos())
        .unwrap_or(u64::MAX)
        .max(1)
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::process::Command;

    use super::*;

    const SOURCE_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
    const LOCK_SHA256: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn locked_package_version<'a>(lockfile: &'a str, package: &str) -> Option<&'a str> {
        lockfile.split("[[package]]").find_map(|entry| {
            let name = entry.lines().find_map(|line| {
                line.strip_prefix("name = \"")
                    .and_then(|value| value.strip_suffix('"'))
            });
            if name != Some(package) {
                return None;
            }
            entry.lines().find_map(|line| {
                line.strip_prefix("version = \"")
                    .and_then(|value| value.strip_suffix('"'))
            })
        })
    }

    #[test]
    fn required_matrix_enumerates_each_candidate_workload_and_scenario_once() {
        let plan = QualificationPlanV1::required(SOURCE_COMMIT, LOCK_SHA256);
        let expected = QualificationCandidateV1::ALL.len()
            * QualificationWorkloadV1::ALL.len()
            * QualificationScenarioV1::ALL.len();
        let identities = plan
            .entries
            .iter()
            .map(QualificationPlanEntryV1::identity)
            .collect::<BTreeSet<_>>();

        assert_eq!(plan.entries.len(), expected);
        assert_eq!(identities.len(), expected);
        assert!(plan.validate().is_ok());
    }

    #[test]
    fn completeness_rejects_missing_stale_mismatched_and_incomplete_results() {
        let plan = QualificationPlanV1::required(SOURCE_COMMIT, LOCK_SHA256);
        let complete = QualificationEvidenceV1::fixture_for_tests(&plan);
        assert!(complete.validate(&plan).is_ok());

        let mut missing = complete.clone();
        missing.results.pop();
        assert!(missing.validate(&plan).is_err());

        let mut stale = complete.clone();
        stale.results[0].source_commit = "ffffffffffffffffffffffffffffffffffffffff".to_owned();
        assert!(stale.validate(&plan).is_err());

        let mut wrong_dependency = complete.clone();
        wrong_dependency.results[0].candidate_build_id = "wrong-dependency".to_owned();
        assert!(wrong_dependency.validate(&plan).is_err());

        let mut incomplete = complete.clone();
        incomplete.results[0].inventory = None;
        assert!(incomplete.validate(&plan).is_err());
    }

    #[test]
    fn skipped_or_unsupported_execution_cannot_masquerade_as_pass() {
        let plan = QualificationPlanV1::required(SOURCE_COMMIT, LOCK_SHA256);
        for disposition in [
            QualificationExecutionDispositionV1::Skipped,
            QualificationExecutionDispositionV1::Unsupported,
        ] {
            let mut evidence = QualificationEvidenceV1::fixture_for_tests(&plan);
            evidence.results[0].execution = disposition;
            assert!(evidence.validate(&plan).is_err());
        }
    }

    #[test]
    fn process_barrier_uses_a_distinct_process_and_records_real_overlap() {
        let root = tempfile::tempdir().expect("barrier root");
        let barrier = QualificationProcessBarrierV1::create(root.path(), &["reader"])
            .expect("create barrier");
        let mut child = Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "bench_support::foundation::fault::tests::process_barrier_child_entrypoint",
                "--nocapture",
            ])
            .env("POINTBREAK_QUALIFICATION_BARRIER_CHILD", root.path())
            .spawn()
            .expect("spawn barrier child");

        barrier
            .wait_until_ready(std::time::Duration::from_secs(10))
            .expect("child becomes ready");
        let release = barrier.release().expect("release child");
        let status = child.wait().expect("wait for child");
        assert!(status.success());

        let evidence = barrier.evidence().expect("barrier evidence");
        assert_eq!(evidence.participants.len(), 1);
        assert_ne!(evidence.participants[0].process_id, std::process::id());
        assert!(evidence.participants[0].ready_unix_nanos <= release);
        assert!(evidence.participants[0].completed_unix_nanos >= release);
        assert!(evidence.validate_overlap().is_ok());
    }

    #[test]
    fn process_barrier_child_entrypoint() {
        let Some(root) = std::env::var_os("POINTBREAK_QUALIFICATION_BARRIER_CHILD") else {
            return;
        };
        let participant =
            QualificationProcessBarrierParticipantV1::join(root, "reader").expect("join barrier");
        participant
            .wait_for_release(std::time::Duration::from_secs(10))
            .expect("wait for release");
        participant.complete().expect("complete barrier");
    }

    #[test]
    fn fault_seed_and_kill_point_are_reproducible() {
        let first = QualificationPlanV1::required(SOURCE_COMMIT, LOCK_SHA256);
        let second = QualificationPlanV1::required(SOURCE_COMMIT, LOCK_SHA256);

        assert_eq!(first.plan_sha256, second.plan_sha256);
        assert_eq!(first.entries, second.entries);
        assert!(first.entries.iter().all(|entry| entry.fault_seed != 0));
        assert!(
            first
                .entries
                .iter()
                .all(|entry| !entry.kill_point.is_empty())
        );
    }

    #[test]
    fn candidate_build_labels_match_the_locked_dependencies() {
        let lockfile = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock"));

        assert_eq!(
            locked_package_version(lockfile, "rusqlite"),
            Some(RUSQLITE_BUILD_VERSION)
        );
        assert_eq!(
            locked_package_version(lockfile, "libsqlite3-sys"),
            Some(LIBSQLITE3_SYS_BUILD_VERSION)
        );
        assert_eq!(
            locked_package_version(lockfile, "pointbreak"),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn performance_results_require_raw_samples_and_environment_metadata() {
        let plan = QualificationPlanV1::required(SOURCE_COMMIT, LOCK_SHA256);
        let mut evidence = QualificationEvidenceV1::fixture_for_tests(&plan);
        let performance = evidence
            .results
            .iter_mut()
            .find(|result| result.scenario == QualificationScenarioV1::Performance)
            .expect("performance result");

        performance.raw_samples.clear();
        assert!(evidence.validate(&plan).is_err());

        let mut failed = QualificationEvidenceV1::fixture_for_tests(&plan);
        let performance = failed
            .results
            .iter_mut()
            .find(|result| result.scenario == QualificationScenarioV1::Performance)
            .expect("performance result");
        performance.outcome = QualificationScenarioOutcomeV1::Failed;
        performance.failure = Some("threshold exceeded".to_owned());
        performance.baseline_inventory = None;
        assert!(failed.validate(&plan).is_err());

        let mut evidence = QualificationEvidenceV1::fixture_for_tests(&plan);
        evidence.results[0].environment.filesystem.clear();
        assert!(evidence.validate(&plan).is_err());
    }

    #[test]
    fn nonlocal_filesystems_never_count_as_local_platform_proof() {
        assert_eq!(
            classify_qualification_filesystem("apfs"),
            QualificationFilesystemDispositionV1::LocalProofEligible
        );
        assert_eq!(
            classify_qualification_filesystem("ntfs"),
            QualificationFilesystemDispositionV1::LocalProofEligible
        );
        assert_eq!(
            classify_qualification_filesystem("nfs"),
            QualificationFilesystemDispositionV1::Refused
        );
        assert_eq!(
            classify_qualification_filesystem("Dropbox"),
            QualificationFilesystemDispositionV1::AdvisoryOnly
        );
    }

    #[test]
    fn allocation_faults_publish_no_partial_record_and_allow_retry() {
        for candidate in QualificationCandidateV1::ALL {
            let root = tempfile::tempdir().expect("allocation-fault root");
            let candidate_root = root.path().join(candidate.as_str());
            let profile = open_candidate(candidate, &candidate_root).expect("open candidate");
            let inventory = profile
                .as_profile()
                .inventory()
                .expect("inventory candidate");
            drop(profile);

            let result = run_allocation_failure(candidate, &candidate_root, inventory)
                .expect("recover allocation fault");

            assert!(result.gate_failure.is_none());
            assert!(result.inventory.is_some());
        }
    }
}
