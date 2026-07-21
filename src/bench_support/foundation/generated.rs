use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    QUALIFICATION_CORPUS_SCHEMA_V1, QualificationCorpusManifestV1, QualificationRecordKindV1,
    QualificationRecordV1,
};
use crate::canonical_hash::{canonical_json_bytes, sha256_bytes_hex};

pub const QUALIFICATION_GENERATOR_SCHEMA_V1: &str =
    "pointbreak.qualification-workload-generator.v1";
pub const QUALIFICATION_GENERATED_SUMMARY_SCHEMA_V1: &str =
    "pointbreak.qualification-generated-workload-summary.v1";
pub const QUALIFICATION_GENERATED_SMOKE_SCHEMA_V1: &str =
    "pointbreak.qualification-generated-workload-smoke.v1";
pub const QUALIFICATION_LOGICAL_KEY_MAX_BYTES_V1: usize = 255;
pub const QUALIFICATION_RECORD_KIND_MINIMUM_V1: u64 = 1;
pub const QUALIFICATION_LIFECYCLE_MOTIF_MINIMUM_V1: u64 = 1;

pub const QUALIFICATION_PUBLIC_SEED_V1: [u8; 32] = [
    0xf4, 0xda, 0x49, 0x60, 0x1a, 0x21, 0x20, 0x10, 0xba, 0xe4, 0x44, 0xe6, 0xca, 0x2d, 0xe6, 0xc6,
    0xbf, 0x28, 0xb5, 0xec, 0x1b, 0x0a, 0x05, 0xbf, 0x42, 0x15, 0x4a, 0x53, 0x3c, 0xa5, 0x13, 0xff,
];
pub const QUALIFICATION_PUBLIC_SEED_HEX_V1: &str =
    "f4da49601a212010bae444e6ca2de6c6bf28b5ec1b0a05bf42154a533ca513ff";

pub const QUALIFICATION_G0_SPEC_SHA256_V1: &str =
    "5dd08fab4e371f90f9de401ea78c6e281d442627967a3a16db55f724eb32c928";
pub const QUALIFICATION_G0_MANIFEST_SHA256_V1: &str =
    "b35ebf4bd7bf09a40133e2066cce43cb901a07bf06d5b1caa0f4881bdad27595";
pub const QUALIFICATION_G0_SCHEDULE_SHA256_V1: &str =
    "8f2c69c54a1ea590d05c139cc5405a3e3081be1c9ca50278e3a5ec03df8f788b";
pub const QUALIFICATION_G1_SPEC_SHA256_V1: &str =
    "9a4b6c1ef8363866005d47860206f94f089a0ad0e2b0e89471dd7254098d368a";
pub const QUALIFICATION_G1_MANIFEST_SHA256_V1: &str =
    "f520817b751d672810bd8fbe842bb2983b5ff437cce1ad4db3341d79c9b4bf4f";
pub const QUALIFICATION_G1_SCHEDULE_SHA256_V1: &str =
    "a8a094aee8b4154d1c6d1c8c1dcf82f1bf2ecd12d22d4d8ffa4391960e1c0f58";
pub const QUALIFICATION_G2_SPEC_SHA256_V1: &str =
    "d19e86ed2ca9c0ccc03c1356d721216d3a8a9cba0c49ce19c29c3d52fc1a567c";
pub const QUALIFICATION_G2_MANIFEST_SHA256_V1: &str =
    "295240840539fbd500796d0cd125d3c1e5266cb61a9feba4aeab2a4d0c2c9158";
pub const QUALIFICATION_G2_SCHEDULE_SHA256_V1: &str =
    "e9f0e9e983873c5251b2ca401718e0ae2bfbde32a046c31b6ece7295c88199a9";

const GENERATED_RECORD_SIZE_PATTERN_V1: [usize; 8] =
    [512, 1_024, 2_048, 4_096, 8_192, 12_288, 16_384, 20_992];
const QUALIFICATION_APPEND_SCHEDULE_RECORDS_V1: usize = 30;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationGeneratedWorkloadV1 {
    G0,
    G1,
    G2,
}

impl QualificationGeneratedWorkloadV1 {
    pub const ALL: [Self; 3] = [Self::G0, Self::G1, Self::G2];

    fn id(self) -> &'static str {
        match self {
            Self::G0 => "g0",
            Self::G1 => "g1",
            Self::G2 => "g2",
        }
    }

    fn canonical_scale(self) -> (u64, u64, u32) {
        match self {
            Self::G0 => (128, 1_048_576, 4),
            Self::G1 => (1_024, 8_388_608, 8),
            Self::G2 => (8_192, 67_108_864, 8),
        }
    }

    fn source_label(self) -> String {
        format!("generated-public-{}-v1", self.id())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct QualificationGeneratorSpecV1 {
    pub schema: String,
    pub public_seed: [u8; 32],
    pub workload: QualificationGeneratedWorkloadV1,
    pub record_count: u64,
    pub decoded_bytes: u64,
    pub cohorts: u32,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationKeyedReadClassV1 {
    Oldest,
    Middle,
    Newest,
    Absent,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct QualificationScheduledReadV1 {
    pub class: QualificationKeyedReadClassV1,
    pub logical_key: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct QualificationOperationScheduleV1 {
    pub keyed_reads: Vec<QualificationScheduledReadV1>,
    pub append_record_indices: Vec<u64>,
    pub schedule_sha256: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationKeyShapeV1 {
    DigestUniform,
    CommonPrefix,
    CohortOrdered,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationLifecycleMotifV1 {
    RootOnly,
    RootToReplacement,
    Continuation,
    ForkedReplacement,
    CarriedOpen,
    Resolved,
    RemovableContentPresent,
    RemovedContentAbsent,
    RestoredFromBackup,
}

impl QualificationLifecycleMotifV1 {
    const ALL: [Self; 9] = [
        Self::RootOnly,
        Self::RootToReplacement,
        Self::Continuation,
        Self::ForkedReplacement,
        Self::CarriedOpen,
        Self::Resolved,
        Self::RemovableContentPresent,
        Self::RemovedContentAbsent,
        Self::RestoredFromBackup,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::RootOnly => "root_only",
            Self::RootToReplacement => "root_to_replacement",
            Self::Continuation => "continuation",
            Self::ForkedReplacement => "forked_replacement",
            Self::CarriedOpen => "carried_open",
            Self::Resolved => "resolved",
            Self::RemovableContentPresent => "removable_content_present",
            Self::RemovedContentAbsent => "removed_content_absent",
            Self::RestoredFromBackup => "restored_from_backup",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationPayloadClassV1 {
    LowCompressibility,
    MediumCompressibility,
    HighCompressibility,
}

impl QualificationPayloadClassV1 {
    fn name(self) -> &'static str {
        match self {
            Self::LowCompressibility => "low",
            Self::MediumCompressibility => "medium",
            Self::HighCompressibility => "high",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct QualificationGeneratedRecordKindCountV1 {
    pub record_kind: QualificationRecordKindV1,
    pub record_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct QualificationGeneratedKeyShapeCountV1 {
    pub key_shape: QualificationKeyShapeV1,
    pub record_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct QualificationGeneratedLifecycleCountV1 {
    pub lifecycle_motif: QualificationLifecycleMotifV1,
    pub record_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct QualificationGeneratedPayloadClassCountV1 {
    pub payload_class: QualificationPayloadClassV1,
    pub record_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct QualificationGeneratedWorkloadSummaryV1 {
    pub schema: String,
    pub generator_schema: String,
    pub generator_spec_sha256: String,
    pub public_seed_hex: String,
    pub workload: QualificationGeneratedWorkloadV1,
    pub record_count: u64,
    pub decoded_bytes: u64,
    pub cohorts: u32,
    pub manifest_sha256: String,
    pub schedule_sha256: String,
    pub by_kind: Vec<QualificationGeneratedRecordKindCountV1>,
    pub key_shapes: Vec<QualificationGeneratedKeyShapeCountV1>,
    pub lifecycle_motifs: Vec<QualificationGeneratedLifecycleCountV1>,
    pub payload_classes: Vec<QualificationGeneratedPayloadClassCountV1>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct QualificationGeneratedWorkloadSmokeV1 {
    pub schema: String,
    pub mode: String,
    pub generator_schema: String,
    pub public_seed_hex: String,
    pub workloads: Vec<QualificationGeneratedWorkloadSummaryV1>,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum QualificationGeneratorError {
    #[error("unsupported generated workload schema {schema}")]
    UnsupportedSchema { schema: String },
    #[error("{workload:?} requires {expected} records, found {actual}")]
    RecordCountMismatch {
        workload: QualificationGeneratedWorkloadV1,
        expected: u64,
        actual: u64,
    },
    #[error("{workload:?} requires {expected} decoded bytes, found {actual}")]
    DecodedByteTotalMismatch {
        workload: QualificationGeneratedWorkloadV1,
        expected: u64,
        actual: u64,
    },
    #[error("generated workloads require between 3 and 32 cohorts, found {cohorts}")]
    InvalidCohortCount { cohorts: u32 },
    #[error("generated workload logical key is not portable public ASCII")]
    InvalidLogicalKey,
    #[error("generated logical key appears more than once")]
    DuplicateLogicalKey,
    #[error("generated manifest logical keys are not sorted")]
    UnsortedLogicalKeys,
    #[error("generated record decoded hash does not match its bytes")]
    DecodedHashMismatch,
    #[error("generated manifest source does not match the workload specification")]
    SourceMismatch,
    #[error("generated manifest schema does not match the corpus contract")]
    ManifestSchemaMismatch,
    #[error("generated manifest hash does not match its canonical bytes")]
    ManifestHashMismatch,
    #[error("generated operation schedule is invalid: {reason}")]
    InvalidOperationSchedule { reason: String },
    #[error("generated workload does not match its frozen public identity")]
    FrozenIdentityMismatch,
    #[error("generated workload could not be canonicalized: {message}")]
    Canonicalization { message: String },
}

#[derive(Clone, Debug)]
struct GeneratedRecordPlanV1 {
    ordinal: u64,
    logical_key: String,
    record_kind: QualificationRecordKindV1,
    cohort: u32,
    key_shape: QualificationKeyShapeV1,
    lifecycle_motif: QualificationLifecycleMotifV1,
    payload_class: QualificationPayloadClassV1,
    decoded_bytes: usize,
}

#[derive(Debug)]
pub struct QualificationGeneratedRecordStreamV1 {
    spec: QualificationGeneratorSpecV1,
    plans: std::vec::IntoIter<GeneratedRecordPlanV1>,
    retained_plan_bytes: usize,
}

impl QualificationGeneratedRecordStreamV1 {
    pub fn retained_payload_bytes(&self) -> usize {
        0
    }

    pub fn retained_plan_bytes(&self) -> usize {
        self.retained_plan_bytes
    }
}

impl Iterator for QualificationGeneratedRecordStreamV1 {
    type Item = Result<QualificationRecordV1, QualificationGeneratorError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.plans
            .next()
            .map(|plan| generated_record_v1(&self.spec, &plan))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.plans.size_hint()
    }
}

impl ExactSizeIterator for QualificationGeneratedRecordStreamV1 {}

#[derive(Serialize)]
struct ScheduleHashPreimageV1<'a> {
    keyed_reads: &'a [QualificationScheduledReadV1],
    append_record_indices: &'a [u64],
}

pub fn qualification_generator_spec_v1(
    workload: QualificationGeneratedWorkloadV1,
) -> QualificationGeneratorSpecV1 {
    let (record_count, decoded_bytes, cohorts) = workload.canonical_scale();
    QualificationGeneratorSpecV1 {
        schema: QUALIFICATION_GENERATOR_SCHEMA_V1.to_owned(),
        public_seed: QUALIFICATION_PUBLIC_SEED_V1,
        workload,
        record_count,
        decoded_bytes,
        cohorts,
    }
}

pub fn qualification_generated_records_v1(
    spec: &QualificationGeneratorSpecV1,
) -> Result<QualificationGeneratedRecordStreamV1, QualificationGeneratorError> {
    validate_generator_spec_v1(spec)?;
    let plans = generated_record_plans_v1(spec)?;
    let retained_plan_bytes = plans
        .iter()
        .map(|plan| std::mem::size_of::<GeneratedRecordPlanV1>() + plan.logical_key.capacity())
        .sum();
    Ok(QualificationGeneratedRecordStreamV1 {
        spec: spec.clone(),
        plans: plans.into_iter(),
        retained_plan_bytes,
    })
}

pub fn qualification_generated_manifest_v1(
    spec: &QualificationGeneratorSpecV1,
) -> Result<QualificationCorpusManifestV1, QualificationGeneratorError> {
    let records = qualification_generated_records_v1(spec)?.collect::<Result<Vec<_>, _>>()?;
    let manifest_sha256 = generated_manifest_sha256_v1(spec, records.iter())?;
    Ok(QualificationCorpusManifestV1 {
        schema: QUALIFICATION_CORPUS_SCHEMA_V1.to_owned(),
        source: spec.workload.source_label(),
        records,
        manifest_sha256,
    })
}

pub fn qualification_generated_workload_summary_v1(
    spec: &QualificationGeneratorSpecV1,
) -> Result<QualificationGeneratedWorkloadSummaryV1, QualificationGeneratorError> {
    validate_generator_spec_v1(spec)?;
    let records = qualification_generated_records_v1(spec)?;
    let mut manifest_hasher = GeneratedManifestHasherV1::new(spec)?;
    let mut record_count = 0_u64;
    let mut decoded_bytes = 0_u64;
    let mut by_kind = BTreeMap::new();
    let mut key_shapes = BTreeMap::new();
    let mut lifecycle_motifs = BTreeMap::new();
    let mut payload_classes = BTreeMap::new();

    for record in records {
        let record = record?;
        let value: serde_json::Value =
            serde_json::from_slice(&record.decoded_bytes).map_err(|error| {
                QualificationGeneratorError::Canonicalization {
                    message: error.to_string(),
                }
            })?;
        let key_shape = qualification_key_shape_v1(&record.logical_key)
            .ok_or(QualificationGeneratorError::InvalidLogicalKey)?;
        let lifecycle_motif =
            lifecycle_motif_from_name_v1(value["lifecycle_motif"].as_str().ok_or_else(|| {
                QualificationGeneratorError::Canonicalization {
                    message: "generated lifecycle motif is missing".to_owned(),
                }
            })?)?;
        let payload_class =
            payload_class_from_name_v1(value["compressibility"].as_str().ok_or_else(|| {
                QualificationGeneratorError::Canonicalization {
                    message: "generated compressibility class is missing".to_owned(),
                }
            })?)?;

        record_count += 1;
        decoded_bytes += record.decoded_bytes.len() as u64;
        *by_kind.entry(record.record_kind).or_insert(0_u64) += 1;
        *key_shapes.entry(key_shape).or_insert(0_u64) += 1;
        *lifecycle_motifs.entry(lifecycle_motif).or_insert(0_u64) += 1;
        *payload_classes.entry(payload_class).or_insert(0_u64) += 1;
        manifest_hasher.record(&record)?;
    }
    let manifest_sha256 = manifest_hasher.finish(spec)?;
    if record_count != spec.record_count {
        return Err(QualificationGeneratorError::RecordCountMismatch {
            workload: spec.workload,
            expected: spec.record_count,
            actual: record_count,
        });
    }
    if decoded_bytes != spec.decoded_bytes {
        return Err(QualificationGeneratorError::DecodedByteTotalMismatch {
            workload: spec.workload,
            expected: spec.decoded_bytes,
            actual: decoded_bytes,
        });
    }

    let schedule = qualification_operation_schedule_v1(spec)?;
    Ok(QualificationGeneratedWorkloadSummaryV1 {
        schema: QUALIFICATION_GENERATED_SUMMARY_SCHEMA_V1.to_owned(),
        generator_schema: spec.schema.clone(),
        generator_spec_sha256: generator_spec_sha256_v1(spec)?,
        public_seed_hex: hex_lower(&spec.public_seed),
        workload: spec.workload,
        record_count,
        decoded_bytes,
        cohorts: spec.cohorts,
        manifest_sha256,
        schedule_sha256: schedule.schedule_sha256,
        by_kind: by_kind
            .into_iter()
            .map(
                |(record_kind, record_count)| QualificationGeneratedRecordKindCountV1 {
                    record_kind,
                    record_count,
                },
            )
            .collect(),
        key_shapes: key_shapes
            .into_iter()
            .map(
                |(key_shape, record_count)| QualificationGeneratedKeyShapeCountV1 {
                    key_shape,
                    record_count,
                },
            )
            .collect(),
        lifecycle_motifs: lifecycle_motifs
            .into_iter()
            .map(
                |(lifecycle_motif, record_count)| QualificationGeneratedLifecycleCountV1 {
                    lifecycle_motif,
                    record_count,
                },
            )
            .collect(),
        payload_classes: payload_classes
            .into_iter()
            .map(
                |(payload_class, record_count)| QualificationGeneratedPayloadClassCountV1 {
                    payload_class,
                    record_count,
                },
            )
            .collect(),
    })
}

pub fn qualification_generated_workload_smoke_v1()
-> Result<QualificationGeneratedWorkloadSmokeV1, QualificationGeneratorError> {
    let workloads = QualificationGeneratedWorkloadV1::ALL
        .into_iter()
        .map(qualification_generator_spec_v1)
        .map(|spec| {
            let summary = qualification_generated_workload_summary_v1(&spec)?;
            validate_frozen_identity_v1(&summary)?;
            Ok(summary)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(QualificationGeneratedWorkloadSmokeV1 {
        schema: QUALIFICATION_GENERATED_SMOKE_SCHEMA_V1.to_owned(),
        mode: "non_timing_regeneration".to_owned(),
        generator_schema: QUALIFICATION_GENERATOR_SCHEMA_V1.to_owned(),
        public_seed_hex: QUALIFICATION_PUBLIC_SEED_HEX_V1.to_owned(),
        workloads,
    })
}

pub fn qualification_operation_schedule_v1(
    spec: &QualificationGeneratorSpecV1,
) -> Result<QualificationOperationScheduleV1, QualificationGeneratorError> {
    validate_generator_spec_v1(spec)?;
    let plans = generated_record_plans_v1(spec)?;
    let read_classes = [
        (QualificationKeyedReadClassV1::Oldest, 0, "read-oldest"),
        (
            QualificationKeyedReadClassV1::Middle,
            spec.cohorts / 2,
            "read-middle",
        ),
        (
            QualificationKeyedReadClassV1::Newest,
            spec.cohorts - 1,
            "read-newest",
        ),
    ];
    let mut keyed_reads = Vec::with_capacity(4);
    for (class, cohort, domain) in read_classes {
        let selected = plans
            .iter()
            .filter(|plan| plan.cohort == cohort)
            .min_by_key(|plan| domain_digest_v1(spec, domain, plan.ordinal))
            .ok_or_else(|| QualificationGeneratorError::InvalidOperationSchedule {
                reason: format!("cohort {cohort} has no records"),
            })?;
        keyed_reads.push(QualificationScheduledReadV1 {
            class,
            logical_key: selected.logical_key.clone(),
        });
    }
    keyed_reads.push(QualificationScheduledReadV1 {
        class: QualificationKeyedReadClassV1::Absent,
        logical_key: format!(
            "records/absent/{}.json",
            hex_lower(&domain_digest_v1(spec, "read-absent", 0))
        ),
    });

    let mut append_rank = plans
        .iter()
        .enumerate()
        .map(|(manifest_index, plan)| {
            (
                domain_digest_v1(spec, "append-order", plan.ordinal),
                manifest_index as u64,
            )
        })
        .collect::<Vec<_>>();
    append_rank.sort();
    let append_record_indices = append_rank
        .into_iter()
        .take(QUALIFICATION_APPEND_SCHEDULE_RECORDS_V1)
        .map(|(_, manifest_index)| manifest_index)
        .collect::<Vec<_>>();
    let schedule_sha256 = schedule_sha256_v1(&keyed_reads, &append_record_indices)?;
    Ok(QualificationOperationScheduleV1 {
        keyed_reads,
        append_record_indices,
        schedule_sha256,
    })
}

pub fn validate_qualification_generated_manifest_v1(
    spec: &QualificationGeneratorSpecV1,
    manifest: &QualificationCorpusManifestV1,
) -> Result<(), QualificationGeneratorError> {
    validate_generator_spec_v1(spec)?;
    if manifest.schema != QUALIFICATION_CORPUS_SCHEMA_V1 {
        return Err(QualificationGeneratorError::ManifestSchemaMismatch);
    }
    if manifest.source != spec.workload.source_label() {
        return Err(QualificationGeneratorError::SourceMismatch);
    }
    if manifest.records.len() as u64 != spec.record_count {
        return Err(QualificationGeneratorError::RecordCountMismatch {
            workload: spec.workload,
            expected: spec.record_count,
            actual: manifest.records.len() as u64,
        });
    }

    let mut decoded_bytes = 0_u64;
    let mut previous: Option<&str> = None;
    for record in &manifest.records {
        validate_logical_key_v1(&record.logical_key)?;
        if let Some(previous) = previous {
            if previous == record.logical_key {
                return Err(QualificationGeneratorError::DuplicateLogicalKey);
            }
            if previous > record.logical_key.as_str() {
                return Err(QualificationGeneratorError::UnsortedLogicalKeys);
            }
        }
        if sha256_bytes_hex(&record.decoded_bytes) != record.decoded_sha256 {
            return Err(QualificationGeneratorError::DecodedHashMismatch);
        }
        decoded_bytes += record.decoded_bytes.len() as u64;
        previous = Some(&record.logical_key);
    }
    if decoded_bytes != spec.decoded_bytes {
        return Err(QualificationGeneratorError::DecodedByteTotalMismatch {
            workload: spec.workload,
            expected: spec.decoded_bytes,
            actual: decoded_bytes,
        });
    }
    let actual_hash = generated_manifest_sha256_v1(spec, manifest.records.iter())?;
    if actual_hash != manifest.manifest_sha256 {
        return Err(QualificationGeneratorError::ManifestHashMismatch);
    }
    let expected_hash = qualification_generated_workload_summary_v1(spec)?.manifest_sha256;
    if actual_hash != expected_hash {
        return Err(QualificationGeneratorError::ManifestHashMismatch);
    }
    Ok(())
}

pub fn validate_qualification_operation_schedule_v1(
    spec: &QualificationGeneratorSpecV1,
    manifest: &QualificationCorpusManifestV1,
    schedule: &QualificationOperationScheduleV1,
) -> Result<(), QualificationGeneratorError> {
    validate_qualification_generated_manifest_v1(spec, manifest)?;
    let expected_classes = [
        QualificationKeyedReadClassV1::Oldest,
        QualificationKeyedReadClassV1::Middle,
        QualificationKeyedReadClassV1::Newest,
        QualificationKeyedReadClassV1::Absent,
    ];
    if schedule
        .keyed_reads
        .iter()
        .map(|read| read.class)
        .ne(expected_classes)
    {
        return Err(QualificationGeneratorError::InvalidOperationSchedule {
            reason: "keyed reads must name oldest, middle, newest, and absent once".to_owned(),
        });
    }
    let read_keys = schedule
        .keyed_reads
        .iter()
        .map(|read| read.logical_key.as_str())
        .collect::<BTreeSet<_>>();
    if read_keys.len() != 4 {
        return Err(QualificationGeneratorError::InvalidOperationSchedule {
            reason: "keyed read keys must be distinct".to_owned(),
        });
    }
    let manifest_keys = manifest
        .records
        .iter()
        .map(|record| record.logical_key.as_str())
        .collect::<BTreeSet<_>>();
    if !schedule.keyed_reads[..3]
        .iter()
        .all(|read| manifest_keys.contains(read.logical_key.as_str()))
        || manifest_keys.contains(schedule.keyed_reads[3].logical_key.as_str())
    {
        return Err(QualificationGeneratorError::InvalidOperationSchedule {
            reason: "existing read keys must exist and the absent read key must not".to_owned(),
        });
    }
    if schedule.append_record_indices.len() != QUALIFICATION_APPEND_SCHEDULE_RECORDS_V1
        || schedule
            .append_record_indices
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len()
            != schedule.append_record_indices.len()
        || schedule
            .append_record_indices
            .iter()
            .any(|index| *index >= spec.record_count)
    {
        return Err(QualificationGeneratorError::InvalidOperationSchedule {
            reason: "append indices must be unique and in range".to_owned(),
        });
    }
    let actual_hash = schedule_sha256_v1(&schedule.keyed_reads, &schedule.append_record_indices)?;
    if actual_hash != schedule.schedule_sha256 {
        return Err(QualificationGeneratorError::InvalidOperationSchedule {
            reason: "schedule hash does not match canonical schedule bytes".to_owned(),
        });
    }
    if schedule != &qualification_operation_schedule_v1(spec)? {
        return Err(QualificationGeneratorError::InvalidOperationSchedule {
            reason: "schedule does not match the canonical workload schedule".to_owned(),
        });
    }
    Ok(())
}

pub fn qualification_key_shape_v1(logical_key: &str) -> Option<QualificationKeyShapeV1> {
    if logical_key.starts_with("records/u/") {
        Some(QualificationKeyShapeV1::DigestUniform)
    } else if logical_key.starts_with("records/p/qualification-common-prefix-v1/") {
        Some(QualificationKeyShapeV1::CommonPrefix)
    } else if logical_key.starts_with("records/c/cohort-") {
        Some(QualificationKeyShapeV1::CohortOrdered)
    } else {
        None
    }
}

fn validate_generator_spec_v1(
    spec: &QualificationGeneratorSpecV1,
) -> Result<(), QualificationGeneratorError> {
    if spec.schema != QUALIFICATION_GENERATOR_SCHEMA_V1 {
        return Err(QualificationGeneratorError::UnsupportedSchema {
            schema: spec.schema.clone(),
        });
    }
    let (record_count, decoded_bytes, _) = spec.workload.canonical_scale();
    if spec.record_count != record_count {
        return Err(QualificationGeneratorError::RecordCountMismatch {
            workload: spec.workload,
            expected: record_count,
            actual: spec.record_count,
        });
    }
    if spec.decoded_bytes != decoded_bytes {
        return Err(QualificationGeneratorError::DecodedByteTotalMismatch {
            workload: spec.workload,
            expected: decoded_bytes,
            actual: spec.decoded_bytes,
        });
    }
    if !(3..=32).contains(&spec.cohorts) {
        return Err(QualificationGeneratorError::InvalidCohortCount {
            cohorts: spec.cohorts,
        });
    }
    Ok(())
}

fn validate_frozen_identity_v1(
    summary: &QualificationGeneratedWorkloadSummaryV1,
) -> Result<(), QualificationGeneratorError> {
    let expected = match summary.workload {
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
    };
    if summary.generator_spec_sha256 != expected.0
        || summary.manifest_sha256 != expected.1
        || summary.schedule_sha256 != expected.2
    {
        return Err(QualificationGeneratorError::FrozenIdentityMismatch);
    }
    Ok(())
}

fn generated_record_plans_v1(
    spec: &QualificationGeneratorSpecV1,
) -> Result<Vec<GeneratedRecordPlanV1>, QualificationGeneratorError> {
    let mut plans = Vec::with_capacity(spec.record_count as usize);
    for ordinal in 0..spec.record_count {
        let key_shape = if ordinal < spec.record_count / 2 {
            QualificationKeyShapeV1::DigestUniform
        } else if ordinal < spec.record_count * 3 / 4 {
            QualificationKeyShapeV1::CommonPrefix
        } else {
            QualificationKeyShapeV1::CohortOrdered
        };
        let cohort = (ordinal * u64::from(spec.cohorts) / spec.record_count) as u32;
        let logical_key = generated_logical_key_v1(spec, ordinal, cohort, key_shape);
        validate_logical_key_v1(&logical_key)?;
        plans.push(GeneratedRecordPlanV1 {
            ordinal,
            logical_key,
            record_kind: record_kind_for_ordinal_v1(ordinal),
            cohort,
            key_shape,
            lifecycle_motif: QualificationLifecycleMotifV1::ALL
                [ordinal as usize % QualificationLifecycleMotifV1::ALL.len()],
            payload_class: match ordinal % 3 {
                0 => QualificationPayloadClassV1::LowCompressibility,
                1 => QualificationPayloadClassV1::MediumCompressibility,
                _ => QualificationPayloadClassV1::HighCompressibility,
            },
            decoded_bytes: GENERATED_RECORD_SIZE_PATTERN_V1
                [ordinal as usize % GENERATED_RECORD_SIZE_PATTERN_V1.len()],
        });
    }
    plans.sort_by(|left, right| left.logical_key.cmp(&right.logical_key));
    for pair in plans.windows(2) {
        if pair[0].logical_key == pair[1].logical_key {
            return Err(QualificationGeneratorError::DuplicateLogicalKey);
        }
    }
    Ok(plans)
}

fn generated_logical_key_v1(
    spec: &QualificationGeneratorSpecV1,
    ordinal: u64,
    cohort: u32,
    key_shape: QualificationKeyShapeV1,
) -> String {
    let digest = hex_lower(&domain_digest_v1(spec, "logical-key", ordinal));
    match key_shape {
        QualificationKeyShapeV1::DigestUniform => {
            format!("records/u/{digest}-{ordinal:08}.json")
        }
        QualificationKeyShapeV1::CommonPrefix => format!(
            "records/p/qualification-common-prefix-v1/{ordinal:08}-{}.json",
            &digest[..16]
        ),
        QualificationKeyShapeV1::CohortOrdered => {
            format!("records/c/cohort-{cohort:02}/{:08}.json", ordinal)
        }
    }
}

fn generated_record_v1(
    spec: &QualificationGeneratorSpecV1,
    plan: &GeneratedRecordPlanV1,
) -> Result<QualificationRecordV1, QualificationGeneratorError> {
    let prefix = format!(
        "{{\"cohort\":{},\"compressibility\":\"{}\",\"generator\":\"{}\",\"key_shape\":\"{}\",\"lifecycle_motif\":\"{}\",\"ordinal\":{},\"padding\":\"",
        plan.cohort,
        plan.payload_class.name(),
        QUALIFICATION_GENERATOR_SCHEMA_V1,
        key_shape_name_v1(plan.key_shape),
        plan.lifecycle_motif.name(),
        plan.ordinal,
    );
    let suffix = format!(
        "\",\"record_kind\":\"{}\",\"workload\":\"{}\"}}",
        record_kind_name_v1(plan.record_kind),
        spec.workload.id()
    );
    let envelope_bytes = prefix.len() + suffix.len();
    if envelope_bytes > plan.decoded_bytes {
        return Err(QualificationGeneratorError::Canonicalization {
            message: "generated record envelope exceeds its decoded size bin".to_owned(),
        });
    }
    let padding_bytes = plan.decoded_bytes - envelope_bytes;
    let mut decoded_bytes = Vec::with_capacity(plan.decoded_bytes);
    decoded_bytes.extend_from_slice(prefix.as_bytes());
    append_padding_v1(
        spec,
        plan.ordinal,
        plan.payload_class,
        padding_bytes,
        &mut decoded_bytes,
    );
    decoded_bytes.extend_from_slice(suffix.as_bytes());
    debug_assert_eq!(decoded_bytes.len(), plan.decoded_bytes);
    Ok(QualificationRecordV1::new(
        plan.logical_key.clone(),
        plan.record_kind,
        decoded_bytes,
    ))
}

fn append_padding_v1(
    spec: &QualificationGeneratorSpecV1,
    ordinal: u64,
    payload_class: QualificationPayloadClassV1,
    length: usize,
    output: &mut Vec<u8>,
) {
    match payload_class {
        QualificationPayloadClassV1::HighCompressibility => {
            output.resize(output.len() + length, b'a');
        }
        QualificationPayloadClassV1::MediumCompressibility => {
            const BLOCK: &[u8] = b"pointbreak-public-workload-medium-padding-v1-";
            for index in 0..length {
                output.push(BLOCK[(index + ordinal as usize) % BLOCK.len()]);
            }
        }
        QualificationPayloadClassV1::LowCompressibility => {
            let mut remaining = length;
            let mut counter = 0_u64;
            while remaining > 0 {
                let digest = domain_digest_v1(
                    spec,
                    "payload-padding",
                    ordinal.wrapping_mul(1_000_000).wrapping_add(counter),
                );
                let encoded = hex_lower(&digest);
                let take = remaining.min(encoded.len());
                output.extend_from_slice(&encoded.as_bytes()[..take]);
                remaining -= take;
                counter += 1;
            }
        }
    }
}

fn record_kind_for_ordinal_v1(ordinal: u64) -> QualificationRecordKindV1 {
    const KINDS: [QualificationRecordKindV1; 9] = [
        QualificationRecordKindV1::LegacyEvent,
        QualificationRecordKindV1::GenerationProposal,
        QualificationRecordKindV1::RelationAttestation,
        QualificationRecordKindV1::FactPort,
        QualificationRecordKindV1::ObjectArtifact,
        QualificationRecordKindV1::NoteBody,
        QualificationRecordKindV1::RelationProof,
        QualificationRecordKindV1::DocumentManifest,
        QualificationRecordKindV1::DocumentBlob,
    ];
    KINDS[ordinal as usize % KINDS.len()]
}

fn record_kind_name_v1(record_kind: QualificationRecordKindV1) -> &'static str {
    match record_kind {
        QualificationRecordKindV1::LegacyEvent => "legacy_event",
        QualificationRecordKindV1::GenerationProposal => "generation_proposal",
        QualificationRecordKindV1::RelationAttestation => "relation_attestation",
        QualificationRecordKindV1::FactPort => "fact_port",
        QualificationRecordKindV1::ObjectArtifact => "object_artifact",
        QualificationRecordKindV1::NoteBody => "note_body",
        QualificationRecordKindV1::RelationProof => "relation_proof",
        QualificationRecordKindV1::DocumentManifest => "document_manifest",
        QualificationRecordKindV1::DocumentBlob => "document_blob",
    }
}

fn key_shape_name_v1(key_shape: QualificationKeyShapeV1) -> &'static str {
    match key_shape {
        QualificationKeyShapeV1::DigestUniform => "digest_uniform",
        QualificationKeyShapeV1::CommonPrefix => "common_prefix",
        QualificationKeyShapeV1::CohortOrdered => "cohort_ordered",
    }
}

fn lifecycle_motif_from_name_v1(
    name: &str,
) -> Result<QualificationLifecycleMotifV1, QualificationGeneratorError> {
    QualificationLifecycleMotifV1::ALL
        .into_iter()
        .find(|motif| motif.name() == name)
        .ok_or_else(|| QualificationGeneratorError::Canonicalization {
            message: "generated lifecycle motif is unknown".to_owned(),
        })
}

fn payload_class_from_name_v1(
    name: &str,
) -> Result<QualificationPayloadClassV1, QualificationGeneratorError> {
    [
        QualificationPayloadClassV1::LowCompressibility,
        QualificationPayloadClassV1::MediumCompressibility,
        QualificationPayloadClassV1::HighCompressibility,
    ]
    .into_iter()
    .find(|payload_class| payload_class.name() == name)
    .ok_or_else(|| QualificationGeneratorError::Canonicalization {
        message: "generated compressibility class is unknown".to_owned(),
    })
}

fn validate_logical_key_v1(logical_key: &str) -> Result<(), QualificationGeneratorError> {
    if logical_key.is_empty()
        || logical_key.len() > QUALIFICATION_LOGICAL_KEY_MAX_BYTES_V1
        || !logical_key.is_ascii()
        || logical_key != logical_key.to_ascii_lowercase()
        || logical_key.starts_with('/')
        || logical_key.ends_with('/')
        || logical_key.contains(['\\', '\r', '\n', ':'])
        || logical_key
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(QualificationGeneratorError::InvalidLogicalKey);
    }
    Ok(())
}

fn generator_spec_sha256_v1(
    spec: &QualificationGeneratorSpecV1,
) -> Result<String, QualificationGeneratorError> {
    let value = serde_json::to_value(spec).map_err(|error| {
        QualificationGeneratorError::Canonicalization {
            message: error.to_string(),
        }
    })?;
    let bytes = canonical_json_bytes(&value).map_err(|error| {
        QualificationGeneratorError::Canonicalization {
            message: error.to_string(),
        }
    })?;
    Ok(sha256_bytes_hex(&bytes))
}

fn schedule_sha256_v1(
    keyed_reads: &[QualificationScheduledReadV1],
    append_record_indices: &[u64],
) -> Result<String, QualificationGeneratorError> {
    let preimage = ScheduleHashPreimageV1 {
        keyed_reads,
        append_record_indices,
    };
    let value = serde_json::to_value(preimage).map_err(|error| {
        QualificationGeneratorError::Canonicalization {
            message: error.to_string(),
        }
    })?;
    let bytes = canonical_json_bytes(&value).map_err(|error| {
        QualificationGeneratorError::Canonicalization {
            message: error.to_string(),
        }
    })?;
    Ok(sha256_bytes_hex(&bytes))
}

fn generated_manifest_sha256_v1<'a>(
    spec: &QualificationGeneratorSpecV1,
    records: impl IntoIterator<Item = &'a QualificationRecordV1>,
) -> Result<String, QualificationGeneratorError> {
    let mut hasher = GeneratedManifestHasherV1::new(spec)?;
    for record in records {
        hasher.record(record)?;
    }
    hasher.finish(spec)
}

struct GeneratedManifestHasherV1 {
    writer: Sha256WriterV1,
    first: bool,
}

impl GeneratedManifestHasherV1 {
    fn new(spec: &QualificationGeneratorSpecV1) -> Result<Self, QualificationGeneratorError> {
        validate_generator_spec_v1(spec)?;
        let mut writer = Sha256WriterV1(Sha256::new());
        writer
            .write_all(b"{\"records\":[")
            .map_err(canonicalization_io_error)?;
        Ok(Self {
            writer,
            first: true,
        })
    }

    fn record(
        &mut self,
        record: &QualificationRecordV1,
    ) -> Result<(), QualificationGeneratorError> {
        if !self.first {
            self.writer
                .write_all(b",")
                .map_err(canonicalization_io_error)?;
        }
        self.first = false;
        self.writer
            .write_all(b"{\"decoded_bytes\":")
            .map_err(canonicalization_io_error)?;
        serde_json::to_writer(&mut self.writer, &record.decoded_bytes).map_err(|error| {
            QualificationGeneratorError::Canonicalization {
                message: error.to_string(),
            }
        })?;
        self.writer
            .write_all(b",\"decoded_sha256\":")
            .map_err(canonicalization_io_error)?;
        serde_json::to_writer(&mut self.writer, &record.decoded_sha256).map_err(|error| {
            QualificationGeneratorError::Canonicalization {
                message: error.to_string(),
            }
        })?;
        self.writer
            .write_all(b",\"logical_key\":")
            .map_err(canonicalization_io_error)?;
        serde_json::to_writer(&mut self.writer, &record.logical_key).map_err(|error| {
            QualificationGeneratorError::Canonicalization {
                message: error.to_string(),
            }
        })?;
        self.writer
            .write_all(b",\"record_kind\":")
            .map_err(canonicalization_io_error)?;
        serde_json::to_writer(&mut self.writer, &record.record_kind).map_err(|error| {
            QualificationGeneratorError::Canonicalization {
                message: error.to_string(),
            }
        })?;
        self.writer
            .write_all(b"}")
            .map_err(canonicalization_io_error)?;
        Ok(())
    }

    fn finish(
        mut self,
        spec: &QualificationGeneratorSpecV1,
    ) -> Result<String, QualificationGeneratorError> {
        self.writer
            .write_all(b"],\"schema\":")
            .map_err(canonicalization_io_error)?;
        serde_json::to_writer(&mut self.writer, QUALIFICATION_CORPUS_SCHEMA_V1).map_err(
            |error| QualificationGeneratorError::Canonicalization {
                message: error.to_string(),
            },
        )?;
        self.writer
            .write_all(b",\"source\":")
            .map_err(canonicalization_io_error)?;
        serde_json::to_writer(&mut self.writer, &spec.workload.source_label()).map_err(
            |error| QualificationGeneratorError::Canonicalization {
                message: error.to_string(),
            },
        )?;
        self.writer
            .write_all(b"}")
            .map_err(canonicalization_io_error)?;
        Ok(hex_lower(&self.writer.0.finalize()))
    }
}

struct Sha256WriterV1(Sha256);

impl Write for Sha256WriterV1 {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.update(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn canonicalization_io_error(error: std::io::Error) -> QualificationGeneratorError {
    QualificationGeneratorError::Canonicalization {
        message: error.to_string(),
    }
}

fn domain_digest_v1(spec: &QualificationGeneratorSpecV1, domain: &str, counter: u64) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(spec.public_seed);
    hasher.update(spec.schema.as_bytes());
    hasher.update(spec.workload.id().as_bytes());
    hasher.update(domain.as_bytes());
    hasher.update(counter.to_be_bytes());
    hasher.finalize().into()
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use serde_json::Value;

    use super::*;
    use crate::bench_support::foundation::{QualificationCorpusManifestV1, QualificationRecordV1};

    #[test]
    fn generated_workloads_match_exact_totals_and_frozen_identities() {
        let expected = [
            (
                QualificationGeneratedWorkloadV1::G0,
                128,
                1_048_576,
                QUALIFICATION_G0_SPEC_SHA256_V1,
                QUALIFICATION_G0_MANIFEST_SHA256_V1,
                QUALIFICATION_G0_SCHEDULE_SHA256_V1,
            ),
            (
                QualificationGeneratedWorkloadV1::G1,
                1_024,
                8_388_608,
                QUALIFICATION_G1_SPEC_SHA256_V1,
                QUALIFICATION_G1_MANIFEST_SHA256_V1,
                QUALIFICATION_G1_SCHEDULE_SHA256_V1,
            ),
            (
                QualificationGeneratedWorkloadV1::G2,
                8_192,
                67_108_864,
                QUALIFICATION_G2_SPEC_SHA256_V1,
                QUALIFICATION_G2_MANIFEST_SHA256_V1,
                QUALIFICATION_G2_SCHEDULE_SHA256_V1,
            ),
        ];

        for (workload, expected_records, expected_bytes, spec_hash, manifest_hash, schedule_hash) in
            expected
        {
            let spec = qualification_generator_spec_v1(workload);
            let mut record_count = 0_u64;
            let mut decoded_bytes = 0_u64;
            for record in qualification_generated_records_v1(&spec).expect("valid stream") {
                let record = record.expect("valid generated record");
                record_count += 1;
                decoded_bytes += record.decoded_bytes.len() as u64;
            }
            let summary = qualification_generated_workload_summary_v1(&spec)
                .expect("valid generated summary");

            assert_eq!(record_count, expected_records);
            assert_eq!(decoded_bytes, expected_bytes);
            assert_eq!(summary.record_count, expected_records);
            assert_eq!(summary.decoded_bytes, expected_bytes);
            assert_eq!(summary.generator_spec_sha256, spec_hash);
            assert_eq!(summary.manifest_sha256, manifest_hash);
            assert_eq!(summary.schedule_sha256, schedule_hash);
        }
    }

    #[test]
    fn canonical_identity_fixture_matches_the_public_specification() {
        let fixture = serde_json::from_str::<Value>(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/store-foundation/generated-workloads-v1.json"
        )))
        .expect("canonical identity fixture");
        let workloads = QualificationGeneratedWorkloadV1::ALL
            .into_iter()
            .map(|workload| {
                let spec = qualification_generator_spec_v1(workload);
                let (spec_hash, manifest_hash, schedule_hash) = match workload {
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
                };
                serde_json::json!({
                    "workload": workload,
                    "recordCount": spec.record_count,
                    "decodedBytes": spec.decoded_bytes,
                    "cohorts": spec.cohorts,
                    "generatorSpecSha256": spec_hash,
                    "manifestSha256": manifest_hash,
                    "scheduleSha256": schedule_hash,
                })
            })
            .collect::<Vec<_>>();
        let expected = serde_json::json!({
            "schema": "pointbreak.qualification-generated-workload-identities.v1",
            "generatorSchema": QUALIFICATION_GENERATOR_SCHEMA_V1,
            "publicSeedHex": QUALIFICATION_PUBLIC_SEED_HEX_V1,
            "workloads": workloads,
        });

        assert_eq!(fixture, expected);
    }

    #[test]
    fn streaming_manifest_hash_matches_the_existing_canonical_manifest_contract() {
        let spec = qualification_generator_spec_v1(QualificationGeneratedWorkloadV1::G0);
        let generated = qualification_generated_manifest_v1(&spec).expect("generated G0 manifest");
        let contract =
            QualificationCorpusManifestV1::new(generated.source.clone(), generated.records.clone())
                .expect("existing canonical manifest");

        assert_eq!(generated.manifest_sha256, contract.manifest_sha256);
    }

    #[test]
    fn seed_and_spec_drift_change_manifest_and_schedule_identities() {
        let spec = qualification_generator_spec_v1(QualificationGeneratedWorkloadV1::G0);
        let canonical =
            qualification_generated_workload_summary_v1(&spec).expect("canonical summary");

        let mut changed_seed = spec.clone();
        changed_seed.public_seed[0] ^= 1;
        let changed_seed = qualification_generated_workload_summary_v1(&changed_seed)
            .expect("changed public seed remains generatable");
        assert_ne!(
            canonical.generator_spec_sha256,
            changed_seed.generator_spec_sha256
        );
        assert_ne!(canonical.manifest_sha256, changed_seed.manifest_sha256);
        assert_ne!(canonical.schedule_sha256, changed_seed.schedule_sha256);

        let mut changed_spec = spec;
        changed_spec.cohorts += 1;
        let changed_spec = qualification_generated_workload_summary_v1(&changed_spec)
            .expect("changed public cohort spec remains generatable");
        assert_ne!(
            canonical.generator_spec_sha256,
            changed_spec.generator_spec_sha256
        );
        assert_ne!(canonical.manifest_sha256, changed_spec.manifest_sha256);
        assert_ne!(canonical.schedule_sha256, changed_spec.schedule_sha256);
    }

    #[test]
    fn streaming_g2_matches_collected_bytes_without_buffering_payloads() {
        let spec = qualification_generator_spec_v1(QualificationGeneratedWorkloadV1::G2);
        let collected = qualification_generated_manifest_v1(&spec).expect("collected G2 manifest");
        let mut streamed = qualification_generated_records_v1(&spec).expect("streamed G2 records");

        assert_eq!(streamed.retained_payload_bytes(), 0);
        assert!(streamed.retained_plan_bytes() < spec.decoded_bytes as usize / 8);
        for collected_record in &collected.records {
            let streamed_record = streamed
                .next()
                .expect("matching streamed record")
                .expect("valid streamed record");
            assert_eq!(&streamed_record, collected_record);
            assert_eq!(streamed.retained_payload_bytes(), 0);
        }
        assert!(streamed.next().is_none());

        let streaming_summary =
            qualification_generated_workload_summary_v1(&spec).expect("streaming identity summary");
        assert_eq!(streaming_summary.manifest_sha256, collected.manifest_sha256);
        assert_eq!(
            streaming_summary.record_count,
            collected.records.len() as u64
        );
    }

    #[test]
    fn generated_workload_declares_all_record_kinds_and_lifecycle_motifs() {
        let spec = qualification_generator_spec_v1(QualificationGeneratedWorkloadV1::G0);
        let summary =
            qualification_generated_workload_summary_v1(&spec).expect("generated summary");

        assert_eq!(summary.by_kind.len(), 9);
        assert!(
            summary
                .by_kind
                .iter()
                .all(|count| count.record_count >= QUALIFICATION_RECORD_KIND_MINIMUM_V1)
        );
        assert_eq!(summary.lifecycle_motifs.len(), 9);
        assert!(
            summary
                .lifecycle_motifs
                .iter()
                .all(|count| { count.record_count >= QUALIFICATION_LIFECYCLE_MOTIF_MINIMUM_V1 })
        );

        let manifest = qualification_generated_manifest_v1(&spec).expect("generated manifest");
        let motifs = manifest
            .records
            .iter()
            .map(|record| {
                serde_json::from_slice::<Value>(&record.decoded_bytes)
                    .expect("generated records are JSON")["lifecycle_motif"]
                    .as_str()
                    .expect("generated lifecycle motif")
                    .to_owned()
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(motifs.len(), 9);
    }

    #[test]
    fn generated_keys_use_the_frozen_shape_weights_and_public_bounds() {
        for workload in QualificationGeneratedWorkloadV1::ALL {
            let spec = qualification_generator_spec_v1(workload);
            let records = qualification_generated_records_v1(&spec).expect("generated records");
            let mut shapes = BTreeMap::new();
            for record in records {
                let record = record.expect("valid record");
                assert!(record.logical_key.is_ascii());
                assert_eq!(record.logical_key, record.logical_key.to_ascii_lowercase());
                assert!(record.logical_key.len() <= QUALIFICATION_LOGICAL_KEY_MAX_BYTES_V1);
                assert!(!record.logical_key.contains(['\\', '\r', '\n', ':']));
                *shapes
                    .entry(
                        qualification_key_shape_v1(&record.logical_key).expect("known key shape"),
                    )
                    .or_insert(0_u64) += 1;
            }

            assert_eq!(
                shapes[&QualificationKeyShapeV1::DigestUniform],
                spec.record_count / 2
            );
            assert_eq!(
                shapes[&QualificationKeyShapeV1::CommonPrefix],
                spec.record_count / 4
            );
            assert_eq!(
                shapes[&QualificationKeyShapeV1::CohortOrdered],
                spec.record_count / 4
            );
        }
    }

    #[test]
    fn keyed_read_schedule_names_four_distinct_independent_classes() {
        let spec = qualification_generator_spec_v1(QualificationGeneratedWorkloadV1::G0);
        let manifest = qualification_generated_manifest_v1(&spec).expect("generated manifest");
        let schedule = qualification_operation_schedule_v1(&spec).expect("generated schedule");
        let manifest_keys = manifest
            .records
            .iter()
            .map(|record| record.logical_key.as_str())
            .collect::<BTreeSet<_>>();

        assert_eq!(
            schedule
                .keyed_reads
                .iter()
                .map(|read| read.class)
                .collect::<Vec<_>>(),
            vec![
                QualificationKeyedReadClassV1::Oldest,
                QualificationKeyedReadClassV1::Middle,
                QualificationKeyedReadClassV1::Newest,
                QualificationKeyedReadClassV1::Absent,
            ]
        );
        assert_eq!(
            schedule
                .keyed_reads
                .iter()
                .map(|read| read.logical_key.as_str())
                .collect::<BTreeSet<_>>()
                .len(),
            4
        );
        assert!(
            schedule.keyed_reads[..3]
                .iter()
                .all(|read| manifest_keys.contains(read.logical_key.as_str()))
        );
        assert!(!manifest_keys.contains(schedule.keyed_reads[3].logical_key.as_str()));
        let selected_cohorts = schedule.keyed_reads[..3]
            .iter()
            .map(|read| {
                let record = manifest
                    .records
                    .iter()
                    .find(|record| record.logical_key == read.logical_key)
                    .expect("scheduled record");
                serde_json::from_slice::<Value>(&record.decoded_bytes)
                    .expect("scheduled record JSON")["cohort"]
                    .as_u64()
                    .expect("scheduled cohort")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            selected_cohorts,
            vec![0, u64::from(spec.cohorts / 2), u64::from(spec.cohorts - 1)]
        );
        let cohort_width = spec.record_count / u64::from(spec.cohorts);
        let selected_ordinals = schedule.keyed_reads[..3]
            .iter()
            .map(|read| {
                let record = manifest
                    .records
                    .iter()
                    .find(|record| record.logical_key == read.logical_key)
                    .expect("scheduled record");
                serde_json::from_slice::<Value>(&record.decoded_bytes)
                    .expect("scheduled record JSON")["ordinal"]
                    .as_u64()
                    .expect("scheduled ordinal")
            })
            .collect::<Vec<_>>();
        for (ordinal, cohort) in selected_ordinals.into_iter().zip([0, 2, 3]) {
            assert!(
                (cohort * cohort_width..(cohort + 1) * cohort_width).contains(&ordinal),
                "scheduled ordinal {ordinal} must belong to logical-age cohort {cohort}"
            );
        }
        assert_eq!(schedule.append_record_indices.len(), 30);
        assert_eq!(
            schedule
                .append_record_indices
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len(),
            schedule.append_record_indices.len()
        );
        assert!(
            schedule
                .append_record_indices
                .iter()
                .all(|index| *index < spec.record_count)
        );
        validate_qualification_operation_schedule_v1(&spec, &manifest, &schedule)
            .expect("valid generated schedule");
    }

    #[test]
    fn generated_manifest_validation_fails_closed() {
        let spec = qualification_generator_spec_v1(QualificationGeneratedWorkloadV1::G0);
        let manifest = qualification_generated_manifest_v1(&spec).expect("generated manifest");

        let mut reordered = manifest.clone();
        reordered.records.swap(0, 1);
        assert!(matches!(
            validate_qualification_generated_manifest_v1(&spec, &reordered),
            Err(QualificationGeneratorError::UnsortedLogicalKeys)
        ));

        let mut duplicate = manifest.clone();
        duplicate.records[1] = duplicate.records[0].clone();
        assert!(matches!(
            validate_qualification_generated_manifest_v1(&spec, &duplicate),
            Err(QualificationGeneratorError::DuplicateLogicalKey)
        ));

        let mut wrong_count = manifest.clone();
        wrong_count.records.pop();
        assert!(matches!(
            validate_qualification_generated_manifest_v1(&spec, &wrong_count),
            Err(QualificationGeneratorError::RecordCountMismatch { .. })
        ));

        let mut wrong_bytes = manifest.clone();
        let last = wrong_bytes.records.last_mut().expect("last record");
        last.decoded_bytes.pop();
        *last = QualificationRecordV1::new(
            last.logical_key.clone(),
            last.record_kind,
            last.decoded_bytes.clone(),
        );
        assert!(matches!(
            validate_qualification_generated_manifest_v1(&spec, &wrong_bytes),
            Err(QualificationGeneratorError::DecodedByteTotalMismatch { .. })
        ));

        let mut unknown_schema = spec.clone();
        unknown_schema.schema = "pointbreak.qualification-workload-generator.v2".to_owned();
        assert!(matches!(
            qualification_generated_manifest_v1(&unknown_schema),
            Err(QualificationGeneratorError::UnsupportedSchema { .. })
        ));

        for invalid_key in [
            "records\\windows.json",
            "records/line\nbreak.json",
            "c:/record.json",
        ] {
            let mut invalid = manifest.clone();
            invalid.records[0].logical_key = invalid_key.to_owned();
            assert!(matches!(
                validate_qualification_generated_manifest_v1(&spec, &invalid),
                Err(QualificationGeneratorError::InvalidLogicalKey)
            ));
        }

        let _: QualificationCorpusManifestV1 = manifest;
    }

    #[test]
    fn generated_public_documents_serialize_without_private_or_external_fields() {
        let spec = qualification_generator_spec_v1(QualificationGeneratedWorkloadV1::G0);
        let summary =
            qualification_generated_workload_summary_v1(&spec).expect("generated summary");
        let report = qualification_generated_workload_smoke_v1().expect("generated smoke report");
        let serialized = serde_json::to_string(&(spec, summary, report)).expect("public JSON");

        for forbidden in [
            "POINTBREAK_QUALIFICATION_CORPUS",
            "/Users/",
            "\\\\",
            "private",
            "external_corpus",
            "logical_key",
            "decoded_sha256",
            "record_hash",
            "source_path",
        ] {
            assert!(!serialized.contains(forbidden), "serialized {forbidden}");
        }
    }

    #[test]
    fn generated_workload_smoke_is_regeneration_only_and_non_timing() {
        let report = qualification_generated_workload_smoke_v1().expect("generated smoke report");
        let value = serde_json::to_value(report).expect("smoke JSON");

        assert_eq!(
            value["schema"],
            "pointbreak.qualification-generated-workload-smoke.v1"
        );
        assert_eq!(value["mode"], "non_timing_regeneration");
        assert_eq!(value["workloads"].as_array().map(Vec::len), Some(3));
        for forbidden_field in [
            "timing_samples",
            "duration",
            "candidate",
            "root",
            "path",
            "external_corpus",
        ] {
            assert!(value.get(forbidden_field).is_none());
        }
    }
}
