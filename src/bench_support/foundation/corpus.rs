use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{
    QualificationContractError, QualificationCorpusManifestV1, QualificationRecordKindV1,
    QualificationRecordV1,
};
use crate::canonical_hash::canonical_json_bytes;

const EXTERNAL_CORPUS_VARIABLE: &str = "POINTBREAK_QUALIFICATION_CORPUS";
const EXTERNAL_SOURCE_LABEL: &str = "external-frozen-legacy";
const EXTERNAL_WORKLOAD_SOURCE_LABEL_V2: &str = "external-performance-workload-v2";
const SYNTHETIC_LEGACY_SOURCE_LABEL: &str = "synthetic-legacy-shape";
const MODELED_SOURCE_LABEL: &str = "modeled-foundation-workload";

pub const QUALIFICATION_EXTERNAL_WORKLOAD_MANIFEST_SHA256_V2: &str =
    "f53ed03dbad9668f3819563dd1d7002f5cef8e6bbe07e7a89a51ae0c86a4f181";

const SYNTHETIC_LEGACY_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/store-foundation/legacy-shape/records.json"
));
const MODELED_WORKLOAD_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/store-foundation/modeled-workload/records.json"
));

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct QualificationCorpusSummaryV1 {
    pub schema: String,
    pub source: String,
    pub manifest_sha256: String,
    pub record_count: u64,
    pub decoded_bytes: u64,
    pub by_kind: Vec<QualificationRecordSummaryV1>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct QualificationRecordSummaryV1 {
    pub record_kind: QualificationRecordKindV1,
    pub record_count: u64,
    pub decoded_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SnapshotDriftReportV1 {
    pub expected: QualificationSnapshotTotalsV1,
    pub actual: QualificationSnapshotTotalsV1,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct QualificationSnapshotTotalsV1 {
    pub logical_records: u64,
    pub logical_decoded_bytes: u64,
    pub store_metadata_records: u64,
    pub store_metadata_decoded_bytes: u64,
    pub total_records: u64,
    pub total_decoded_bytes: u64,
    pub by_kind: Vec<QualificationRecordSummaryV1>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct StoreMetadataTotals {
    records: u64,
    decoded_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum QualificationCorpusError {
    #[error("{EXTERNAL_CORPUS_VARIABLE} must name an explicitly supplied external corpus")]
    ExternalPathRequired,
    #[error("the external qualification corpus path is invalid: {reason}")]
    InvalidExternalPath { reason: String },
    #[error("source-tree paths cannot be used as an external qualification corpus: {reason}")]
    SourceTreePathRejected { reason: String },
    #[error("the external qualification corpus layout is invalid: {reason}")]
    InvalidExternalLayout { reason: String },
    #[error("failed to {operation} the external qualification corpus: {message}")]
    Io {
        operation: &'static str,
        message: String,
    },
    #[error("qualification fixture is invalid: {message}")]
    InvalidFixture { message: String },
    #[error(transparent)]
    Contract(#[from] QualificationContractError),
    #[error("the external corpus does not match the frozen legacy snapshot")]
    SnapshotDrift(Box<SnapshotDriftReportV1>),
    #[error("the external corpus manifest hash is {actual}, expected {expected}")]
    ManifestHashMismatch { expected: String, actual: String },
}

#[derive(Deserialize)]
struct FixtureRecordV1 {
    logical_key: String,
    record_kind: QualificationRecordKindV1,
    decoded: serde_json::Value,
}

impl QualificationCorpusSummaryV1 {
    pub fn from_manifest(manifest: &QualificationCorpusManifestV1) -> Self {
        let totals =
            QualificationSnapshotTotalsV1::from_manifest(manifest, StoreMetadataTotals::default());
        Self {
            schema: manifest.schema.clone(),
            source: manifest.source.clone(),
            manifest_sha256: manifest.manifest_sha256.clone(),
            record_count: totals.logical_records,
            decoded_bytes: totals.logical_decoded_bytes,
            by_kind: totals.by_kind,
        }
    }
}

impl QualificationSnapshotTotalsV1 {
    fn from_manifest(
        manifest: &QualificationCorpusManifestV1,
        store_metadata: StoreMetadataTotals,
    ) -> Self {
        let mut by_kind = BTreeMap::<QualificationRecordKindV1, (u64, u64)>::new();
        let mut logical_decoded_bytes = 0_u64;
        for record in &manifest.records {
            let record_bytes = record.decoded_bytes.len() as u64;
            logical_decoded_bytes += record_bytes;
            let totals = by_kind.entry(record.record_kind).or_default();
            totals.0 += 1;
            totals.1 += record_bytes;
        }
        let logical_records = manifest.records.len() as u64;
        Self {
            logical_records,
            logical_decoded_bytes,
            store_metadata_records: store_metadata.records,
            store_metadata_decoded_bytes: store_metadata.decoded_bytes,
            total_records: logical_records + store_metadata.records,
            total_decoded_bytes: logical_decoded_bytes + store_metadata.decoded_bytes,
            by_kind: by_kind
                .into_iter()
                .map(
                    |(record_kind, (record_count, decoded_bytes))| QualificationRecordSummaryV1 {
                        record_kind,
                        record_count,
                        decoded_bytes,
                    },
                )
                .collect(),
        }
    }

    pub fn frozen_legacy() -> Self {
        Self {
            logical_records: 6_433,
            logical_decoded_bytes: 57_040_114,
            store_metadata_records: 4,
            store_metadata_decoded_bytes: 1_568,
            total_records: 6_437,
            total_decoded_bytes: 57_041_682,
            by_kind: vec![
                QualificationRecordSummaryV1 {
                    record_kind: QualificationRecordKindV1::LegacyEvent,
                    record_count: 6_131,
                    decoded_bytes: 10_934_633,
                },
                QualificationRecordSummaryV1 {
                    record_kind: QualificationRecordKindV1::ObjectArtifact,
                    record_count: 301,
                    decoded_bytes: 45_884_082,
                },
                QualificationRecordSummaryV1 {
                    record_kind: QualificationRecordKindV1::NoteBody,
                    record_count: 1,
                    decoded_bytes: 221_399,
                },
            ],
        }
    }

    pub fn external_v2() -> Self {
        Self {
            logical_records: 6_702,
            logical_decoded_bytes: 58_210_604,
            store_metadata_records: 4,
            store_metadata_decoded_bytes: 1_568,
            total_records: 6_706,
            total_decoded_bytes: 58_212_172,
            by_kind: vec![
                QualificationRecordSummaryV1 {
                    record_kind: QualificationRecordKindV1::LegacyEvent,
                    record_count: 6_392,
                    decoded_bytes: 11_404_022,
                },
                QualificationRecordSummaryV1 {
                    record_kind: QualificationRecordKindV1::ObjectArtifact,
                    record_count: 309,
                    decoded_bytes: 46_585_183,
                },
                QualificationRecordSummaryV1 {
                    record_kind: QualificationRecordKindV1::NoteBody,
                    record_count: 1,
                    decoded_bytes: 221_399,
                },
            ],
        }
    }
}

pub fn synthetic_legacy_manifest() -> Result<QualificationCorpusManifestV1, QualificationCorpusError>
{
    fixture_manifest(SYNTHETIC_LEGACY_SOURCE_LABEL, SYNTHETIC_LEGACY_FIXTURE)
}

pub fn modeled_post_foundation_manifest()
-> Result<QualificationCorpusManifestV1, QualificationCorpusError> {
    fixture_manifest(MODELED_SOURCE_LABEL, MODELED_WORKLOAD_FIXTURE)
}

pub fn load_frozen_legacy_manifest_from_env()
-> Result<QualificationCorpusManifestV1, QualificationCorpusError> {
    let path = env::var_os(EXTERNAL_CORPUS_VARIABLE).map(PathBuf::from);
    load_frozen_legacy_manifest_from_path(path.as_deref())
}

pub fn load_frozen_legacy_manifest_from_path(
    path: Option<&Path>,
) -> Result<QualificationCorpusManifestV1, QualificationCorpusError> {
    let path = path.ok_or(QualificationCorpusError::ExternalPathRequired)?;
    let root = validate_external_root(path)?;
    let manifest = load_external_legacy_manifest_from_root(&root)?;
    let store_metadata = collect_store_metadata_totals(&root)?;
    validate_frozen_legacy_snapshot(&manifest, store_metadata)?;
    Ok(manifest)
}

pub fn load_external_workload_v2_manifest_from_env()
-> Result<QualificationCorpusManifestV1, QualificationCorpusError> {
    let path = env::var_os(EXTERNAL_CORPUS_VARIABLE).map(PathBuf::from);
    load_external_workload_v2_manifest_from_path(path.as_deref())
}

pub fn load_external_workload_v2_manifest_from_path(
    path: Option<&Path>,
) -> Result<QualificationCorpusManifestV1, QualificationCorpusError> {
    let path = path.ok_or(QualificationCorpusError::ExternalPathRequired)?;
    let root = validate_external_root(path)?;
    let manifest = load_external_manifest_from_root(&root, EXTERNAL_WORKLOAD_SOURCE_LABEL_V2)?;
    let store_metadata = collect_store_metadata_totals(&root)?;
    validate_external_workload_v2_snapshot(&manifest, store_metadata)?;
    Ok(manifest)
}

pub fn load_external_legacy_manifest(
    path: impl AsRef<Path>,
) -> Result<QualificationCorpusManifestV1, QualificationCorpusError> {
    let root = validate_external_root(path.as_ref())?;
    load_external_legacy_manifest_from_root(&root)
}

fn load_external_legacy_manifest_from_root(
    root: &Path,
) -> Result<QualificationCorpusManifestV1, QualificationCorpusError> {
    load_external_manifest_from_root(root, EXTERNAL_SOURCE_LABEL)
}

fn load_external_manifest_from_root(
    root: &Path,
    source: &str,
) -> Result<QualificationCorpusManifestV1, QualificationCorpusError> {
    let mut records = Vec::new();
    collect_records(
        root,
        "events",
        QualificationRecordKindV1::LegacyEvent,
        &mut records,
    )?;
    collect_records(
        root,
        "artifacts/objects",
        QualificationRecordKindV1::ObjectArtifact,
        &mut records,
    )?;
    collect_records(
        root,
        "artifacts/notes",
        QualificationRecordKindV1::NoteBody,
        &mut records,
    )?;
    records.sort_by(|left, right| left.logical_key.cmp(&right.logical_key));

    let manifest = QualificationCorpusManifestV1::new(source, records)?;
    manifest.validate()?;
    Ok(manifest)
}

fn validate_frozen_legacy_snapshot(
    manifest: &QualificationCorpusManifestV1,
    store_metadata: StoreMetadataTotals,
) -> Result<(), QualificationCorpusError> {
    let expected = QualificationSnapshotTotalsV1::frozen_legacy();
    let actual = QualificationSnapshotTotalsV1::from_manifest(manifest, store_metadata);
    if actual != expected {
        return Err(QualificationCorpusError::SnapshotDrift(Box::new(
            SnapshotDriftReportV1 { expected, actual },
        )));
    }
    Ok(())
}

fn validate_external_workload_v2_snapshot(
    manifest: &QualificationCorpusManifestV1,
    store_metadata: StoreMetadataTotals,
) -> Result<(), QualificationCorpusError> {
    let expected = QualificationSnapshotTotalsV1::external_v2();
    let actual = QualificationSnapshotTotalsV1::from_manifest(manifest, store_metadata);
    if actual != expected {
        return Err(QualificationCorpusError::SnapshotDrift(Box::new(
            SnapshotDriftReportV1 { expected, actual },
        )));
    }
    if manifest.manifest_sha256 != QUALIFICATION_EXTERNAL_WORKLOAD_MANIFEST_SHA256_V2 {
        return Err(QualificationCorpusError::ManifestHashMismatch {
            expected: QUALIFICATION_EXTERNAL_WORKLOAD_MANIFEST_SHA256_V2.to_owned(),
            actual: manifest.manifest_sha256.clone(),
        });
    }
    Ok(())
}

fn fixture_manifest(
    source: &str,
    fixture: &str,
) -> Result<QualificationCorpusManifestV1, QualificationCorpusError> {
    let fixture_records =
        serde_json::from_str::<Vec<FixtureRecordV1>>(fixture).map_err(|error| {
            QualificationCorpusError::InvalidFixture {
                message: error.to_string(),
            }
        })?;
    let mut records = fixture_records
        .into_iter()
        .map(|record| {
            let decoded_bytes = canonical_json_bytes(&record.decoded).map_err(|error| {
                QualificationCorpusError::InvalidFixture {
                    message: error.to_string(),
                }
            })?;
            Ok(QualificationRecordV1::new(
                record.logical_key,
                record.record_kind,
                decoded_bytes,
            ))
        })
        .collect::<Result<Vec<_>, QualificationCorpusError>>()?;
    records.sort_by(|left, right| left.logical_key.cmp(&right.logical_key));

    let manifest = QualificationCorpusManifestV1::new(source, records)?;
    manifest.validate()?;
    Ok(manifest)
}

fn validate_external_root(path: &Path) -> Result<PathBuf, QualificationCorpusError> {
    let supplied_metadata =
        path.symlink_metadata()
            .map_err(|error| QualificationCorpusError::InvalidExternalPath {
                reason: error.to_string(),
            })?;
    if supplied_metadata.file_type().is_symlink() {
        return Err(QualificationCorpusError::InvalidExternalPath {
            reason: "the supplied root cannot be a symbolic link".to_owned(),
        });
    }
    let root =
        path.canonicalize()
            .map_err(|error| QualificationCorpusError::InvalidExternalPath {
                reason: error.to_string(),
            })?;
    if !root.is_dir() {
        return Err(QualificationCorpusError::InvalidExternalPath {
            reason: "expected a directory".to_owned(),
        });
    }

    let source_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .canonicalize()
        .map_err(|error| QualificationCorpusError::InvalidExternalPath {
            reason: format!("could not resolve source root: {error}"),
        })?;
    if root.starts_with(source_root) {
        return Err(QualificationCorpusError::SourceTreePathRejected {
            reason: "use a separately supplied corpus copy".to_owned(),
        });
    }
    if root
        .ancestors()
        .any(|ancestor| ancestor.join(".git").exists())
    {
        return Err(QualificationCorpusError::SourceTreePathRejected {
            reason: "the external corpus cannot be inside a Git worktree".to_owned(),
        });
    }
    Ok(root)
}

fn collect_records(
    root: &Path,
    relative_dir: &str,
    record_kind: QualificationRecordKindV1,
    records: &mut Vec<QualificationRecordV1>,
) -> Result<(), QualificationCorpusError> {
    let directory = root.join(relative_dir);
    if !directory.is_dir() {
        return Err(QualificationCorpusError::InvalidExternalLayout {
            reason: format!("missing {relative_dir}/"),
        });
    }
    collect_directory(root, &directory, record_kind, records)
}

fn collect_store_metadata_totals(
    root: &Path,
) -> Result<StoreMetadataTotals, QualificationCorpusError> {
    let mut totals = StoreMetadataTotals::default();
    collect_store_metadata_directory(root, root, &mut totals)?;
    Ok(totals)
}

fn collect_store_metadata_directory(
    root: &Path,
    directory: &Path,
    totals: &mut StoreMetadataTotals,
) -> Result<(), QualificationCorpusError> {
    for entry in sorted_directory_entries(directory)? {
        let path = entry.path();
        let key = logical_key(root, &path)?;
        let file_type = entry
            .file_type()
            .map_err(|error| QualificationCorpusError::Io {
                operation: "inspect",
                message: error.to_string(),
            })?;
        if file_type.is_symlink() {
            return Err(QualificationCorpusError::InvalidExternalLayout {
                reason: "symbolic links are not allowed".to_owned(),
            });
        }
        if file_type.is_dir() {
            collect_store_metadata_directory(root, &path, totals)?;
        } else if file_type.is_file() && !is_logical_record_key(&key) {
            let metadata = entry
                .metadata()
                .map_err(|error| QualificationCorpusError::Io {
                    operation: "inspect",
                    message: error.to_string(),
                })?;
            totals.records += 1;
            totals.decoded_bytes += metadata.len();
        }
    }
    Ok(())
}

fn is_logical_record_key(key: &str) -> bool {
    ["events/", "artifacts/objects/", "artifacts/notes/"]
        .iter()
        .any(|prefix| key.starts_with(prefix))
}

fn collect_directory(
    root: &Path,
    directory: &Path,
    record_kind: QualificationRecordKindV1,
    records: &mut Vec<QualificationRecordV1>,
) -> Result<(), QualificationCorpusError> {
    for entry in sorted_directory_entries(directory)? {
        let path = entry.path();
        let key = logical_key(root, &path)?;
        let file_type = entry
            .file_type()
            .map_err(|error| QualificationCorpusError::Io {
                operation: "inspect",
                message: error.to_string(),
            })?;
        if file_type.is_symlink() {
            return Err(QualificationCorpusError::InvalidExternalLayout {
                reason: "symbolic links are not allowed".to_owned(),
            });
        }
        if file_type.is_dir() {
            collect_directory(root, &path, record_kind, records)?;
        } else if file_type.is_file() {
            let decoded_bytes =
                std::fs::read(&path).map_err(|error| QualificationCorpusError::Io {
                    operation: "read",
                    message: error.to_string(),
                })?;
            records.push(QualificationRecordV1::new(key, record_kind, decoded_bytes));
        } else {
            return Err(QualificationCorpusError::InvalidExternalLayout {
                reason: "the corpus contains an unsupported entry".to_owned(),
            });
        }
    }
    Ok(())
}

fn sorted_directory_entries(
    directory: &Path,
) -> Result<Vec<std::fs::DirEntry>, QualificationCorpusError> {
    let mut entries = std::fs::read_dir(directory)
        .map_err(|error| QualificationCorpusError::Io {
            operation: "list",
            message: error.to_string(),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| QualificationCorpusError::Io {
            operation: "list",
            message: error.to_string(),
        })?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    Ok(entries)
}

fn logical_key(root: &Path, path: &Path) -> Result<String, QualificationCorpusError> {
    let relative =
        path.strip_prefix(root)
            .map_err(|_| QualificationCorpusError::InvalidExternalLayout {
                reason: "record escaped the external corpus root".to_owned(),
            })?;
    let components = relative
        .components()
        .map(|component| {
            component.as_os_str().to_str().ok_or_else(|| {
                QualificationCorpusError::InvalidExternalLayout {
                    reason: "record names must be UTF-8".to_owned(),
                }
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(components.join("/"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::Value;

    use super::*;
    use crate::bench_support::foundation::QualificationRecordKindV1;

    #[test]
    fn frozen_legacy_shape_is_deterministic() {
        let first = synthetic_legacy_manifest().expect("legacy fixture");
        let second = synthetic_legacy_manifest().expect("legacy fixture");

        assert_eq!(first, second);
        assert_eq!(
            first.manifest_sha256,
            "03cfda81e2ea988ec119b942530022b345d08b1261a6f198f87fdade2a4d1b01"
        );
        assert_eq!(first.records.len(), 3);
        assert!(first.validate().is_ok());
    }

    #[test]
    fn modeled_workload_covers_the_required_relations_and_artifacts() {
        let manifest = modeled_post_foundation_manifest().expect("modeled fixture");
        assert_eq!(
            manifest.manifest_sha256,
            "5d7ea2f2a8398722e2dcc853ef2c4ebe1976a02fd1585a190c9c6b86e132da7d"
        );
        let decoded = manifest
            .records
            .iter()
            .map(|record| {
                serde_json::from_slice::<Value>(&record.decoded_bytes)
                    .expect("fixture records contain JSON")
            })
            .collect::<Vec<_>>();

        for status in [
            "verified",
            "asserted",
            "unverified",
            "indeterminate",
            "refuted",
        ] {
            assert!(decoded.iter().any(|value| value["proof_status"] == status));
        }
        for relation in [
            "context_only",
            "reanchored_as",
            "carried_open_as",
            "resolved_by",
        ] {
            assert!(decoded.iter().any(|value| value["relation"] == relation));
        }

        assert!(decoded.iter().any(|value| value["generation"] == "root"));
        assert!(
            decoded
                .iter()
                .any(|value| value["generation"] == "replacement")
        );
        assert!(decoded.iter().any(|value| value["continuation"] == true));

        for kind in [
            QualificationRecordKindV1::RelationProof,
            QualificationRecordKindV1::DocumentManifest,
            QualificationRecordKindV1::DocumentBlob,
        ] {
            assert!(
                manifest
                    .records
                    .iter()
                    .any(|record| record.record_kind == kind)
            );
        }
        let artifact_sizes = manifest
            .records
            .iter()
            .filter(|record| record.record_kind == QualificationRecordKindV1::ObjectArtifact)
            .map(|record| record.decoded_bytes.len())
            .collect::<Vec<_>>();
        assert!(
            artifact_sizes.windows(2).all(|sizes| sizes[0] < sizes[1]),
            "the modeled workload grows artifacts across multiple rounds"
        );
    }

    #[test]
    fn relation_proof_details_live_in_relation_proof_content() {
        let manifest = modeled_post_foundation_manifest().expect("modeled fixture");
        let proof = manifest
            .records
            .iter()
            .find(|record| record.record_kind == QualificationRecordKindV1::RelationProof)
            .expect("relation proof fixture");
        let proof: Value = serde_json::from_slice(&proof.decoded_bytes).expect("proof JSON");

        assert_eq!(proof["content"]["proof_kind"], "commit_association");
        assert_eq!(proof["content"]["verified"], true);

        for event in manifest.records.iter().filter(|record| {
            matches!(
                record.record_kind,
                QualificationRecordKindV1::LegacyEvent
                    | QualificationRecordKindV1::GenerationProposal
                    | QualificationRecordKindV1::RelationAttestation
                    | QualificationRecordKindV1::FactPort
            )
        }) {
            let event: Value = serde_json::from_slice(&event.decoded_bytes).expect("event JSON");
            assert!(event.get("proof_details").is_none());
        }
    }

    #[test]
    fn external_loader_requires_an_explicit_path_and_rejects_source_tree_bytes() {
        assert_eq!(
            load_frozen_legacy_manifest_from_path(None),
            Err(QualificationCorpusError::ExternalPathRequired)
        );

        let source_fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/store-foundation/legacy-shape");
        assert!(matches!(
            load_frozen_legacy_manifest_from_path(Some(&source_fixture)),
            Err(QualificationCorpusError::SourceTreePathRejected { .. })
        ));
        assert_eq!(
            load_external_workload_v2_manifest_from_path(None),
            Err(QualificationCorpusError::ExternalPathRequired)
        );
        assert!(matches!(
            load_external_workload_v2_manifest_from_path(Some(&source_fixture)),
            Err(QualificationCorpusError::SourceTreePathRejected { .. })
        ));
    }

    #[test]
    fn external_summary_contains_no_private_bytes_or_path() {
        let root = tempfile::tempdir().expect("external temp corpus");
        let events = root.path().join("events");
        let objects = root.path().join("artifacts/objects");
        let notes = root.path().join("artifacts/notes");
        fs::create_dir_all(&events).unwrap();
        fs::create_dir_all(&objects).unwrap();
        fs::create_dir_all(&notes).unwrap();
        fs::write(events.join("one.json"), b"private-event-sentinel").unwrap();
        fs::write(objects.join("one.json"), b"private-object-sentinel").unwrap();
        fs::write(notes.join("one.md"), b"private-note-sentinel").unwrap();

        let manifest = load_external_legacy_manifest(root.path()).expect("external manifest");
        let summary = QualificationCorpusSummaryV1::from_manifest(&manifest);
        let serialized = serde_json::to_string(&summary).expect("summary JSON");

        assert!(!serialized.contains("private-event-sentinel"));
        assert!(!serialized.contains("private-object-sentinel"));
        assert!(!serialized.contains("private-note-sentinel"));
        assert!(!serialized.contains(root.path().to_string_lossy().as_ref()));
        assert_eq!(summary.record_count, 3);
    }

    #[test]
    fn pinned_snapshot_mismatch_is_structured() {
        let manifest = synthetic_legacy_manifest().expect("legacy fixture");
        let error = validate_frozen_legacy_snapshot(&manifest, StoreMetadataTotals::default())
            .expect_err("small fixture drifts");

        let QualificationCorpusError::SnapshotDrift(report) = error else {
            panic!("expected a structured snapshot-drift report")
        };
        assert_eq!(report.expected.total_records, 6_437);
        assert_eq!(report.expected.total_decoded_bytes, 57_041_682);
        assert_eq!(report.expected.store_metadata_records, 4);
        assert_eq!(report.actual.store_metadata_records, 0);
    }

    #[test]
    fn external_v2_snapshot_identity_is_separate_from_the_historical_snapshot() {
        let historical = QualificationSnapshotTotalsV1::frozen_legacy();
        let external_v2 = QualificationSnapshotTotalsV1::external_v2();

        assert_eq!(external_v2.total_records, 6_706);
        assert_eq!(external_v2.total_decoded_bytes, 58_212_172);
        assert_eq!(external_v2.logical_records, 6_702);
        assert_eq!(external_v2.logical_decoded_bytes, 58_210_604);
        assert_ne!(external_v2, historical);
        assert_eq!(QUALIFICATION_EXTERNAL_WORKLOAD_MANIFEST_SHA256_V2.len(), 64);
    }
}
