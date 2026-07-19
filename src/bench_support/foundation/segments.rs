use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::codec::logical_key_digest;
use super::{
    IndependentContentStoreV1, LogicalCapabilityEpochV1, NeverCancelled, PhysicalRecordKindV1,
    PhysicalRecordV1, PhysicalStoreHeaderV1, QualificationCorpusManifestV1,
    QualificationCreateOutcome, QualificationEntry, QualificationInventoryV1, QualificationJournal,
    QualificationPerformanceDiagnosticSampleV1, QualificationPerformanceOperationRequestV1,
    QualificationPerformanceOperationV1, QualificationPerformanceProbe,
    QualificationPerformanceStageRecorder, QualificationProfile, QualificationProfileDescriptorV1,
    QualificationRecordKindV1, publish_completed_backup, verify_completed_backup,
};
use crate::canonical_hash::{canonical_json_bytes, sha256_bytes_hex};

pub const SEGMENT_QUALIFICATION_PROFILE_ID_V1: &str = "pointbreak.bounded-segments-pbrf.v1";
pub const SEGMENT_SIZE_CANDIDATES_V1: [u64; 3] = [256 * 1024, 1024 * 1024, 4 * 1024 * 1024];
pub const DEFAULT_SEGMENT_BYTES_V1: u64 = 1024 * 1024;
pub const SEGMENT_FRAME_HEADER_LEN_V1: usize = 96;
pub const SEGMENT_FRAME_FOOTER_LEN_V1: usize = 64;
pub const SEGMENT_HEAD_FILE_V1: &str = "segment-head-v1.json";
pub const SEGMENT_INDEX_FILE_V1: &str = "segment-index-v1.json";

const PROFILE_FILE: &str = "profile.pbst";
const METADATA_FILE: &str = "segment-profile-v1.json";
const LOCK_FILE: &str = "segment.lock";
const CONTENT_DIRECTORY: &str = "content";
const ACTIVE_DIRECTORY: &str = "active";
const GENERATION_DIRECTORY: &str = "generations";
const PIN_DIRECTORY: &str = "pins";
const GENERATION_MANIFEST_FILE: &str = "manifest-v1.json";
const GENERATION_INDEX_FILE: &str = "index-v1.json";
const SEGMENT_PROFILE_ID: u32 = 2;
const FRAME_MAGIC: &[u8; 4] = b"PBSF";
const FOOTER_MAGIC: &[u8; 4] = b"PBSC";
const FRAME_VERSION: u16 = 1;
const MAX_LOGICAL_KEY_BYTES: usize = 4096;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SegmentFailurePointV1 {
    AfterRecordBytes,
    AfterCommitFooter,
    AfterTailSync,
    AfterHeadPublish,
    AfterIndexPublish,
    AfterSealedGenerationSync,
    AfterGenerationPublish,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SegmentRecoveryStateV1 {
    Healthy,
    DiscardedUncommittedSuffix { bytes: u64 },
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SegmentDiagnosticStateV1 {
    Healthy,
    StructuralCorruption { message: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SegmentInventoryEvidenceV1 {
    pub active_file: String,
    pub active_committed_bytes: u64,
    pub active_slack_bytes: u64,
    pub current_generation_file: Option<String>,
    pub retained_generations: u64,
    pub retired_generation_bytes: u64,
    pub index_bytes: u64,
    pub head_bytes: u64,
    pub high_water_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SegmentMeasurementV1 {
    pub segment_bytes: u64,
    pub inventory: QualificationInventoryV1,
    pub physical: SegmentInventoryEvidenceV1,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SegmentWorkloadEvidenceV1 {
    pub schema: String,
    pub physical_profile_id: String,
    pub manifest_sha256: String,
    pub records: u64,
    pub journal_records: u64,
    pub content_records: u64,
    pub decoded_bytes: u64,
    pub selected_segment_bytes: u64,
    pub measurements: Vec<SegmentMeasurementV1>,
}

#[derive(Debug, thiserror::Error)]
pub enum SegmentQualificationError {
    #[error("segment profile I/O failed at {path}: {message}", path = .path.display())]
    Io { path: PathBuf, message: String },
    #[error("invalid segment profile: {message}")]
    InvalidProfile { message: String },
    #[error("segment journal conflict for logical key {logical_key}")]
    Conflict { logical_key: String },
    #[error("segment journal corruption: {message}")]
    Corruption { message: String },
    #[error("injected segment failure after {point:?}")]
    InjectedFailure { point: SegmentFailurePointV1 },
    #[error("generation {generation} is pinned by an active reader")]
    PinnedGeneration { generation: u64 },
    #[error("generation {generation} is the current generation")]
    CurrentGeneration { generation: u64 },
    #[error("segment inventory overflow")]
    InventoryOverflow,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SegmentMetadataV1 {
    schema: String,
    descriptor: QualificationProfileDescriptorV1,
    segment_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SegmentHeadV1 {
    schema: String,
    active_file: String,
    committed_active_bytes: u64,
    head_marker: u64,
    next_sequence: u64,
    current_generation: Option<u64>,
    next_generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
struct SegmentIndexEntryV1 {
    logical_key: String,
    decoded_sha256: String,
    sequence: u64,
    carrier: String,
    offset: u64,
    frame_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
struct SegmentIndexV1 {
    schema: String,
    head_marker: u64,
    entries: Vec<SegmentIndexEntryV1>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SealedCarrierV1 {
    relative_path: String,
    encoded_bytes: u64,
    encoded_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SealedGenerationManifestV1 {
    schema: String,
    generation: u64,
    head_marker: u64,
    carriers: Vec<SealedCarrierV1>,
}

#[derive(Debug)]
struct ScannedFrame {
    entry: QualificationEntry,
    sequence: u64,
    carrier: String,
    offset: u64,
    frame_bytes: u64,
}

#[derive(Debug)]
pub struct SegmentQualificationJournal {
    root: PathBuf,
    segment_bytes: u64,
}

pub struct SegmentQualificationProfile {
    root: PathBuf,
    descriptor: QualificationProfileDescriptorV1,
    header: PhysicalStoreHeaderV1,
    journal: SegmentQualificationJournal,
    content: IndependentContentStoreV1,
    recovery_state: SegmentRecoveryStateV1,
}

impl std::fmt::Debug for SegmentQualificationProfile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SegmentQualificationProfile")
            .field("root", &self.root)
            .field("descriptor", &self.descriptor)
            .field("recovery_state", &self.recovery_state)
            .finish_non_exhaustive()
    }
}

impl SegmentQualificationProfile {
    pub fn open(root: &Path) -> Result<Self, SegmentQualificationError> {
        Self::open_internal(root, None)
    }

    pub fn open_with_segment_bytes(
        root: &Path,
        segment_bytes: u64,
    ) -> Result<Self, SegmentQualificationError> {
        validate_segment_bytes(segment_bytes)?;
        Self::open_internal(root, Some(segment_bytes))
    }

    fn open_internal(
        root: &Path,
        requested_segment_bytes: Option<u64>,
    ) -> Result<Self, SegmentQualificationError> {
        fs::create_dir_all(root).map_err(|error| io_error(root, error))?;
        let root = root.canonicalize().map_err(|error| io_error(root, error))?;
        let _lock = SegmentLock::acquire(&root)?;
        let descriptor = segment_descriptor();
        let profile_path = root.join(PROFILE_FILE);
        let metadata_path = root.join(METADATA_FILE);
        let head_path = root.join(SEGMENT_HEAD_FILE_V1);
        let bootstrap_exists = profile_path
            .try_exists()
            .map_err(|error| io_error(&profile_path, error))?;
        let metadata_exists = metadata_path
            .try_exists()
            .map_err(|error| io_error(&metadata_path, error))?;
        let head_exists = head_path
            .try_exists()
            .map_err(|error| io_error(&head_path, error))?;
        if !(bootstrap_exists == metadata_exists && metadata_exists == head_exists) {
            return Err(SegmentQualificationError::InvalidProfile {
                message: "profile bootstrap, metadata, and head must be created together"
                    .to_owned(),
            });
        }

        let (header, segment_bytes) = if bootstrap_exists {
            let header = read_profile_header(&profile_path)?;
            let metadata: SegmentMetadataV1 = read_json(&metadata_path)?;
            validate_metadata(&metadata, &descriptor)?;
            if let Some(requested) = requested_segment_bytes
                && requested != metadata.segment_bytes
            {
                return Err(SegmentQualificationError::InvalidProfile {
                    message: format!(
                        "profile segment size {} differs from requested {requested}",
                        metadata.segment_bytes
                    ),
                });
            }
            (header, metadata.segment_bytes)
        } else {
            let segment_bytes = requested_segment_bytes.unwrap_or(DEFAULT_SEGMENT_BYTES_V1);
            validate_segment_bytes(segment_bytes)?;
            let mut store_uuid = [0_u8; 16];
            getrandom::fill(&mut store_uuid).map_err(|error| {
                SegmentQualificationError::InvalidProfile {
                    message: format!("store UUID generation failed: {error}"),
                }
            })?;
            let header = PhysicalStoreHeaderV1::new(SEGMENT_PROFILE_ID, store_uuid);
            write_new_synced(&profile_path, &header.encode())?;
            write_json_new(
                &metadata_path,
                &SegmentMetadataV1 {
                    schema: "pointbreak.segment-profile.v1".to_owned(),
                    descriptor: descriptor.clone(),
                    segment_bytes,
                },
            )?;
            fs::create_dir_all(root.join(ACTIVE_DIRECTORY))
                .map_err(|error| io_error(&root.join(ACTIVE_DIRECTORY), error))?;
            fs::create_dir_all(root.join(GENERATION_DIRECTORY))
                .map_err(|error| io_error(&root.join(GENERATION_DIRECTORY), error))?;
            fs::create_dir_all(root.join(PIN_DIRECTORY))
                .map_err(|error| io_error(&root.join(PIN_DIRECTORY), error))?;
            let active_file = active_relative_path(0);
            create_preallocated(&root.join(&active_file), segment_bytes)?;
            let head = SegmentHeadV1 {
                schema: "pointbreak.segment-head.v1".to_owned(),
                active_file,
                committed_active_bytes: 0,
                head_marker: 0,
                next_sequence: 1,
                current_generation: None,
                next_generation: 1,
            };
            write_json_new(&head_path, &head)?;
            write_index(&root, &head, &[])?;
            sync_directory(&root)?;
            (header, segment_bytes)
        };

        fs::create_dir_all(root.join(ACTIVE_DIRECTORY))
            .map_err(|error| io_error(&root.join(ACTIVE_DIRECTORY), error))?;
        fs::create_dir_all(root.join(GENERATION_DIRECTORY))
            .map_err(|error| io_error(&root.join(GENERATION_DIRECTORY), error))?;
        fs::create_dir_all(root.join(PIN_DIRECTORY))
            .map_err(|error| io_error(&root.join(PIN_DIRECTORY), error))?;
        let head = read_head(&root)?;
        validate_head(&head, segment_bytes)?;
        cleanup_unpublished_generations(&root, &head)?;
        cleanup_unreferenced_active_files(&root, &head)?;
        validate_retained_generations(&root, segment_bytes)?;
        let recovery_state = recover_active_tail(&root, &head, segment_bytes)?;
        let frames = scan_visible(&root, &head, segment_bytes)?;
        validate_visible_frames(&head, &frames)?;
        ensure_index(&root, &head, &frames)?;
        let content =
            IndependentContentStoreV1::open(&root.join(CONTENT_DIRECTORY)).map_err(|error| {
                SegmentQualificationError::InvalidProfile {
                    message: error.to_string(),
                }
            })?;
        Ok(Self {
            root: root.clone(),
            descriptor,
            header,
            journal: SegmentQualificationJournal {
                root,
                segment_bytes,
            },
            content,
            recovery_state,
        })
    }

    pub fn diagnose_root(root: &Path) -> SegmentDiagnosticStateV1 {
        match validate_root_read_only(root, None) {
            Ok(()) => SegmentDiagnosticStateV1::Healthy,
            Err(error) => SegmentDiagnosticStateV1::StructuralCorruption {
                message: error.to_string(),
            },
        }
    }

    pub fn recovery_state(&self) -> SegmentRecoveryStateV1 {
        self.recovery_state.clone()
    }

    pub fn create_once_with_failure(
        &self,
        logical_key: &str,
        decoded_bytes: &[u8],
        point: SegmentFailurePointV1,
    ) -> Result<QualificationCreateOutcome, SegmentQualificationError> {
        self.journal
            .create_once_typed(logical_key, decoded_bytes, Some(point))
    }

    pub fn seal_active(&self) -> Result<u64, SegmentQualificationError> {
        self.seal_active_internal(None)
    }

    pub fn seal_active_with_failure(
        &self,
        point: SegmentFailurePointV1,
    ) -> Result<u64, SegmentQualificationError> {
        self.seal_active_internal(Some(point))
    }

    fn seal_active_internal(
        &self,
        failure: Option<SegmentFailurePointV1>,
    ) -> Result<u64, SegmentQualificationError> {
        let _lock = SegmentLock::acquire(&self.root)?;
        seal_active_locked(&self.root, self.journal.segment_bytes, failure)
    }

    pub fn pin_reader(&self) -> Result<SegmentReaderPin, SegmentQualificationError> {
        let _lock = SegmentLock::acquire(&self.root)?;
        let head = read_head(&self.root)?;
        let generation =
            head.current_generation
                .ok_or_else(|| SegmentQualificationError::InvalidProfile {
                    message: "cannot pin a profile without a sealed generation".to_owned(),
                })?;
        let segment_path = generation_segment_path(&self.root, generation);
        let handle = File::open(&segment_path).map_err(|error| io_error(&segment_path, error))?;
        let pin_path = self.root.join(PIN_DIRECTORY).join(format!(
            "{:016x}-{:016x}.pin",
            generation,
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        write_new_synced(&pin_path, generation.to_string().as_bytes())?;
        let pin_handle = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&pin_path)
            .map_err(|error| io_error(&pin_path, error))?;
        pin_handle
            .lock()
            .map_err(|error| io_error(&pin_path, error))?;
        sync_directory(&self.root.join(PIN_DIRECTORY))?;
        Ok(SegmentReaderPin {
            generation,
            pin_path,
            _handle: handle,
            pin_handle: Some(pin_handle),
        })
    }

    pub fn retire_generation(&self, generation: u64) -> Result<(), SegmentQualificationError> {
        let _lock = SegmentLock::acquire(&self.root)?;
        let head = read_head(&self.root)?;
        if head.current_generation == Some(generation) {
            return Err(SegmentQualificationError::CurrentGeneration { generation });
        }
        if generation_is_pinned(&self.root, generation)? {
            return Err(SegmentQualificationError::PinnedGeneration { generation });
        }
        let directory = generation_directory_path(&self.root, generation);
        fs::remove_dir_all(&directory).map_err(|error| io_error(&directory, error))?;
        sync_directory(&self.root.join(GENERATION_DIRECTORY))
    }

    pub fn segment_inventory_evidence(
        &self,
    ) -> Result<SegmentInventoryEvidenceV1, SegmentQualificationError> {
        let _lock = SegmentLock::acquire(&self.root)?;
        segment_inventory(&self.root, self.journal.segment_bytes)
    }

    fn populate_backup(&self, destination: &Path) -> Result<(), SegmentQualificationError> {
        let _lock = SegmentLock::acquire(&self.root)?;
        let head = read_head(&self.root)?;
        fs::create_dir_all(destination.join(GENERATION_DIRECTORY))
            .map_err(|error| io_error(&destination.join(GENERATION_DIRECTORY), error))?;
        for relative in [
            PROFILE_FILE,
            METADATA_FILE,
            SEGMENT_HEAD_FILE_V1,
            SEGMENT_INDEX_FILE_V1,
        ] {
            copy_file_synced(&self.root.join(relative), &destination.join(relative))?;
        }
        copy_file_synced(
            &self.root.join(&head.active_file),
            &destination.join(&head.active_file),
        )?;
        if let Some(generation) = head.current_generation {
            copy_tree(
                &generation_directory_path(&self.root, generation),
                &generation_directory_path(destination, generation),
            )?;
        }
        if self
            .content
            .root()
            .try_exists()
            .map_err(|error| io_error(self.content.root(), error))?
        {
            copy_tree(self.content.root(), &destination.join(CONTENT_DIRECTORY))?;
        }
        sync_directory(destination)
    }
}

impl QualificationJournal for SegmentQualificationJournal {
    fn create_once(
        &self,
        logical_key: &str,
        decoded_bytes: &[u8],
    ) -> Result<QualificationCreateOutcome, String> {
        self.create_once_typed(logical_key, decoded_bytes, None)
            .map_err(|error| error.to_string())
    }

    fn read(&self, logical_key: &str) -> Result<Option<QualificationEntry>, String> {
        let _lock = SegmentLock::acquire(&self.root).map_err(|error| error.to_string())?;
        let head = read_head(&self.root).map_err(|error| error.to_string())?;
        let frames = scan_visible(&self.root, &head, self.segment_bytes)
            .map_err(|error| error.to_string())?;
        Ok(frames
            .into_iter()
            .find(|frame| frame.entry.logical_key == logical_key)
            .map(|frame| frame.entry))
    }

    fn list(&self) -> Result<Vec<QualificationEntry>, String> {
        let _lock = SegmentLock::acquire(&self.root).map_err(|error| error.to_string())?;
        let head = read_head(&self.root).map_err(|error| error.to_string())?;
        let mut entries = scan_visible(&self.root, &head, self.segment_bytes)
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|frame| frame.entry)
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            left.logical_key
                .as_bytes()
                .cmp(right.logical_key.as_bytes())
        });
        Ok(entries)
    }

    fn head_marker(&self) -> Result<u64, String> {
        let _lock = SegmentLock::acquire(&self.root).map_err(|error| error.to_string())?;
        read_head(&self.root)
            .map(|head| head.head_marker)
            .map_err(|error| error.to_string())
    }

    fn integrity_check(&self) -> Result<(), String> {
        let _lock = SegmentLock::acquire(&self.root).map_err(|error| error.to_string())?;
        let head = read_head(&self.root).map_err(|error| error.to_string())?;
        validate_retained_generations(&self.root, self.segment_bytes)
            .map_err(|error| error.to_string())?;
        let frames = scan_visible(&self.root, &head, self.segment_bytes)
            .map_err(|error| error.to_string())?;
        validate_visible_frames(&head, &frames).map_err(|error| error.to_string())
    }
}

impl SegmentQualificationJournal {
    fn create_once_typed(
        &self,
        logical_key: &str,
        decoded_bytes: &[u8],
        failure: Option<SegmentFailurePointV1>,
    ) -> Result<QualificationCreateOutcome, SegmentQualificationError> {
        self.create_once_typed_profiled(logical_key, decoded_bytes, failure, None)
    }

    fn create_once_typed_profiled(
        &self,
        logical_key: &str,
        decoded_bytes: &[u8],
        failure: Option<SegmentFailurePointV1>,
        mut recorder: Option<&mut QualificationPerformanceStageRecorder>,
    ) -> Result<QualificationCreateOutcome, SegmentQualificationError> {
        measure_profile_stage(&mut recorder, "validate_request", || {
            if logical_key.is_empty() || logical_key.len() > MAX_LOGICAL_KEY_BYTES {
                return Err(SegmentQualificationError::InvalidProfile {
                    message: "journal key must contain between 1 and 4096 bytes".to_owned(),
                });
            }
            Ok(())
        })?;
        let (_lock, mut head, frames) =
            measure_profile_stage(&mut recorder, "lock_head_visible_scan", || {
                let lock = SegmentLock::acquire(&self.root)?;
                let head = read_head(&self.root)?;
                let frames = scan_visible(&self.root, &head, self.segment_bytes)?;
                Ok((lock, head, frames))
            })?;
        if let Some(existing) = frames
            .iter()
            .find(|frame| frame.entry.logical_key == logical_key)
        {
            return if existing.entry.decoded_bytes == decoded_bytes {
                Ok(QualificationCreateOutcome::AlreadyExists)
            } else {
                Err(SegmentQualificationError::Conflict {
                    logical_key: logical_key.to_owned(),
                })
            };
        }
        let (frame, frame_bytes) =
            measure_profile_stage(&mut recorder, "frame_encode_and_rollover", || {
                let frame = encode_frame(logical_key, decoded_bytes, head.next_sequence)?;
                let frame_bytes = frame.len() as u64;
                if frame_bytes > self.segment_bytes {
                    return Err(SegmentQualificationError::InvalidProfile {
                        message: format!(
                            "encoded frame requires {frame_bytes} bytes, exceeding segment size {}",
                            self.segment_bytes
                        ),
                    });
                }
                if head.committed_active_bytes + frame_bytes > self.segment_bytes {
                    seal_active_locked(&self.root, self.segment_bytes, None)?;
                    head = read_head(&self.root)?;
                }
                Ok((frame, frame_bytes))
            })?;
        measure_profile_stage(&mut recorder, "tail_write_and_sync", || {
            let active_path = self.root.join(&head.active_file);
            let body_len = frame.len() - SEGMENT_FRAME_FOOTER_LEN_V1;
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&active_path)
                .map_err(|error| io_error(&active_path, error))?;
            file.seek(SeekFrom::Start(head.committed_active_bytes))
                .and_then(|_| file.write_all(&frame[..body_len]))
                .map_err(|error| io_error(&active_path, error))?;
            inject_if(failure, SegmentFailurePointV1::AfterRecordBytes)?;
            file.write_all(&frame[body_len..])
                .map_err(|error| io_error(&active_path, error))?;
            inject_if(failure, SegmentFailurePointV1::AfterCommitFooter)?;
            file.sync_all()
                .map_err(|error| io_error(&active_path, error))?;
            inject_if(failure, SegmentFailurePointV1::AfterTailSync)
        })?;

        head.committed_active_bytes += frame_bytes;
        head.head_marker += 1;
        head.next_sequence += 1;
        measure_profile_stage(&mut recorder, "head_publication", || {
            write_json_atomic(&self.root.join(SEGMENT_HEAD_FILE_V1), &head)?;
            sync_directory(&self.root)?;
            inject_if(failure, SegmentFailurePointV1::AfterHeadPublish)
        })?;
        measure_profile_stage(&mut recorder, "index_scan_and_publication", || {
            let frames = scan_visible(&self.root, &head, self.segment_bytes)?;
            write_index(&self.root, &head, &frames)?;
            inject_if(failure, SegmentFailurePointV1::AfterIndexPublish)
        })?;
        Ok(QualificationCreateOutcome::Created)
    }
}

fn measure_profile_stage<T>(
    recorder: &mut Option<&mut QualificationPerformanceStageRecorder>,
    stage: &str,
    operation: impl FnOnce() -> Result<T, SegmentQualificationError>,
) -> Result<T, SegmentQualificationError> {
    match recorder.as_deref_mut() {
        Some(recorder) => recorder.measure(stage, operation),
        None => operation(),
    }
}

impl QualificationProfile for SegmentQualificationProfile {
    fn descriptor(&self) -> Result<QualificationProfileDescriptorV1, String> {
        Ok(self.descriptor.clone())
    }

    fn journal(&self) -> &dyn QualificationJournal {
        &self.journal
    }

    fn put_content_once(
        &self,
        content_key: &str,
        record_kind: QualificationRecordKindV1,
        decoded_bytes: &[u8],
    ) -> Result<QualificationCreateOutcome, String> {
        let _lock = SegmentLock::acquire(&self.root).map_err(|error| error.to_string())?;
        self.content
            .put_once(content_key, record_kind, decoded_bytes)
            .map_err(|error| error.to_string())
    }

    fn read_content(&self, content_key: &str) -> Result<Option<QualificationEntry>, String> {
        self.content
            .read(content_key)
            .map_err(|error| error.to_string())
    }

    fn remove_content(&self, content_key: &str) -> Result<bool, String> {
        let _lock = SegmentLock::acquire(&self.root).map_err(|error| error.to_string())?;
        self.content
            .remove(content_key)
            .map_err(|error| error.to_string())
    }

    fn backup_to(&self, destination: &Path) -> Result<(), String> {
        publish_completed_backup(destination, &self.descriptor, |root| {
            self.populate_backup(root)
                .map_err(|error| error.to_string())
        })
        .map(|_| ())
        .map_err(|error| error.to_string())
    }

    fn verify_restore(&self, restored_root: &Path) -> Result<(), String> {
        verify_completed_backup(restored_root, &self.descriptor)
            .map_err(|error| error.to_string())?;
        validate_root_read_only(restored_root, Some(&self.header))
            .map_err(|error| error.to_string())
    }

    fn inventory(&self) -> Result<QualificationInventoryV1, String> {
        let _lock = SegmentLock::acquire(&self.root).map_err(|error| error.to_string())?;
        let head = read_head(&self.root).map_err(|error| error.to_string())?;
        let frames = scan_visible(&self.root, &head, self.journal.segment_bytes)
            .map_err(|error| error.to_string())?;
        let content = self
            .content
            .inventory()
            .map_err(|error| error.to_string())?;
        let files = collect_files(&self.root).map_err(|error| error.to_string())?;
        let encoded_bytes = files
            .iter()
            .try_fold(0_u64, |total, file| total.checked_add(file.1))
            .ok_or_else(|| "segment profile encoded-byte inventory overflow".to_owned())?;
        let mut carriers = files.into_iter().map(|file| file.0).collect::<Vec<_>>();
        carriers.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        let journal_logical = frames
            .iter()
            .try_fold(0_u64, |total, frame| {
                total.checked_add(frame.entry.decoded_bytes.len() as u64)
            })
            .ok_or_else(|| "segment profile logical-byte inventory overflow".to_owned())?;
        Ok(QualificationInventoryV1 {
            carriers,
            logical_bytes: journal_logical
                .checked_add(content.logical_bytes)
                .ok_or_else(|| "segment profile logical-byte inventory overflow".to_owned())?,
            encoded_bytes,
            allocated_bytes: encoded_bytes,
            high_water_bytes: encoded_bytes,
        })
    }
}

impl QualificationPerformanceProbe for SegmentQualificationProfile {
    fn run_profiled_operation(
        &self,
        request: &QualificationPerformanceOperationRequestV1<'_>,
    ) -> Result<QualificationPerformanceDiagnosticSampleV1, String> {
        let mut recorder = QualificationPerformanceStageRecorder::default();
        match request.operation {
            QualificationPerformanceOperationV1::DurableAppend => {
                let outcome = self
                    .journal
                    .create_once_typed_profiled(
                        request.logical_key,
                        request.decoded_bytes,
                        None,
                        Some(&mut recorder),
                    )
                    .map_err(|_| "segment profiled append failed".to_owned())?;
                if outcome != QualificationCreateOutcome::Created {
                    return Err("segment profiled append did not create a fresh record".to_owned());
                }
            }
            QualificationPerformanceOperationV1::StrictReplay => {
                let entries = recorder
                    .measure("lock_head_scan_decode", || self.journal.list())
                    .map_err(|_| "segment profiled replay failed".to_owned())?;
                std::hint::black_box(entries);
            }
            QualificationPerformanceOperationV1::KeyedRead => {
                let entry = recorder
                    .measure("lock_head_scan_lookup", || {
                        self.journal.read(request.logical_key)
                    })
                    .map_err(|_| "segment profiled keyed read failed".to_owned())?
                    .ok_or_else(|| "segment profiled keyed read omitted a record".to_owned())?;
                if entry.decoded_bytes != request.decoded_bytes {
                    return Err("segment profiled keyed read returned different bytes".to_owned());
                }
            }
            QualificationPerformanceOperationV1::OpenRecovery => {
                let reopened = recorder.measure("open_cleanup_recovery_scan", || {
                    SegmentQualificationProfile::open(&self.root)
                        .map_err(|_| "segment profiled reopen failed".to_owned())
                })?;
                recorder.measure("retained_and_visible_integrity", || {
                    reopened
                        .journal()
                        .integrity_check()
                        .map_err(|_| "segment profiled integrity validation failed".to_owned())
                })?;
            }
        }
        let total_elapsed_nanos = recorder.elapsed_nanos();
        let stages = recorder.finish(total_elapsed_nanos)?;
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

pub struct SegmentReaderPin {
    generation: u64,
    pin_path: PathBuf,
    _handle: File,
    pin_handle: Option<File>,
}

impl SegmentReaderPin {
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

impl std::fmt::Debug for SegmentReaderPin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SegmentReaderPin")
            .field("generation", &self.generation)
            .field("pin_path", &self.pin_path)
            .finish_non_exhaustive()
    }
}

impl Drop for SegmentReaderPin {
    fn drop(&mut self) {
        drop(self.pin_handle.take());
        let _ = fs::remove_file(&self.pin_path);
    }
}

#[derive(Debug)]
struct SegmentLock {
    _file: File,
}

impl SegmentLock {
    fn acquire(root: &Path) -> Result<Self, SegmentQualificationError> {
        let path = root.join(LOCK_FILE);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| io_error(&path, error))?;
        file.lock().map_err(|error| io_error(&path, error))?;
        Ok(Self { _file: file })
    }
}

pub fn run_segment_workload(
    root: &Path,
    manifest: &QualificationCorpusManifestV1,
) -> Result<SegmentWorkloadEvidenceV1, String> {
    manifest.validate().map_err(|error| error.to_string())?;
    let mut measurements = Vec::new();
    let mut selected_counts = None;
    for segment_bytes in SEGMENT_SIZE_CANDIDATES_V1 {
        let candidate_root = root.join(format!("segment-{segment_bytes}"));
        let profile =
            SegmentQualificationProfile::open_with_segment_bytes(&candidate_root, segment_bytes)
                .map_err(|error| error.to_string())?;
        let mut journal_records = 0_u64;
        let mut content_records = 0_u64;
        let mut decoded_bytes = 0_u64;
        for record in &manifest.records {
            let outcome = match record.record_kind {
                QualificationRecordKindV1::LegacyEvent
                | QualificationRecordKindV1::GenerationProposal
                | QualificationRecordKindV1::RelationAttestation
                | QualificationRecordKindV1::FactPort => {
                    journal_records += 1;
                    profile
                        .journal()
                        .create_once(&record.logical_key, &record.decoded_bytes)?
                }
                QualificationRecordKindV1::ObjectArtifact
                | QualificationRecordKindV1::NoteBody
                | QualificationRecordKindV1::RelationProof
                | QualificationRecordKindV1::DocumentManifest
                | QualificationRecordKindV1::DocumentBlob => {
                    content_records += 1;
                    profile.put_content_once(
                        &record.logical_key,
                        record.record_kind,
                        &record.decoded_bytes,
                    )?
                }
            };
            if outcome != QualificationCreateOutcome::Created {
                return Err(format!(
                    "fresh segment workload record {} did not return Created",
                    record.logical_key
                ));
            }
            decoded_bytes = decoded_bytes
                .checked_add(record.decoded_bytes.len() as u64)
                .ok_or_else(|| "segment workload decoded-byte total overflow".to_owned())?;
        }
        profile.journal().integrity_check()?;
        profile.seal_active().map_err(|error| error.to_string())?;
        let inventory = profile.inventory()?;
        let physical = profile
            .segment_inventory_evidence()
            .map_err(|error| error.to_string())?;
        if segment_bytes == DEFAULT_SEGMENT_BYTES_V1 {
            selected_counts = Some((journal_records, content_records, decoded_bytes));
        }
        measurements.push(SegmentMeasurementV1 {
            segment_bytes,
            inventory,
            physical,
        });
    }
    let (journal_records, content_records, decoded_bytes) =
        selected_counts.ok_or_else(|| "selected segment size was not measured".to_owned())?;
    Ok(SegmentWorkloadEvidenceV1 {
        schema: "pointbreak.segment-workload-evidence.v1".to_owned(),
        physical_profile_id: SEGMENT_QUALIFICATION_PROFILE_ID_V1.to_owned(),
        manifest_sha256: manifest.manifest_sha256.clone(),
        records: manifest.records.len() as u64,
        journal_records,
        content_records,
        decoded_bytes,
        selected_segment_bytes: DEFAULT_SEGMENT_BYTES_V1,
        measurements,
    })
}

fn seal_active_locked(
    root: &Path,
    segment_bytes: u64,
    failure: Option<SegmentFailurePointV1>,
) -> Result<u64, SegmentQualificationError> {
    let head = read_head(root)?;
    let frames = scan_visible(root, &head, segment_bytes)?;
    cleanup_unpublished_generations(root, &head)?;
    let generation = head.next_generation;
    let generation_dir = generation_directory_path(root, generation);
    fs::create_dir(&generation_dir).map_err(|error| io_error(&generation_dir, error))?;
    let mut carrier_paths = Vec::new();
    let mut segment_number = 0_u64;
    let mut segment_file = None;
    let mut segment_path = PathBuf::new();
    let mut segment_used = 0_u64;
    let mut generation_index = Vec::new();
    for source in &frames {
        let frame = encode_frame(
            &source.entry.logical_key,
            &source.entry.decoded_bytes,
            source.sequence,
        )?;
        let frame_bytes = frame.len() as u64;
        if frame_bytes > segment_bytes {
            return Err(SegmentQualificationError::InvalidProfile {
                message: format!("sealed frame requires {frame_bytes} bytes"),
            });
        }
        if segment_file.is_none() || segment_used + frame_bytes > segment_bytes {
            if let Some(file) = segment_file.take() {
                sync_file(file, &segment_path)?;
                carrier_paths.push(segment_path.clone());
            }
            segment_path = generation_dir.join(sealed_segment_name(segment_number));
            segment_number += 1;
            segment_used = 0;
            segment_file = Some(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&segment_path)
                    .map_err(|error| io_error(&segment_path, error))?,
            );
        }
        segment_file
            .as_mut()
            .expect("sealed segment file")
            .write_all(&frame)
            .map_err(|error| io_error(&segment_path, error))?;
        let relative = relative_path_string(root, &segment_path)?;
        generation_index.push(SegmentIndexEntryV1 {
            logical_key: source.entry.logical_key.clone(),
            decoded_sha256: source.entry.decoded_sha256.clone(),
            sequence: source.sequence,
            carrier: relative,
            offset: segment_used,
            frame_bytes,
        });
        segment_used += frame_bytes;
    }
    if let Some(file) = segment_file.take() {
        sync_file(file, &segment_path)?;
        carrier_paths.push(segment_path);
    }
    if carrier_paths.is_empty() {
        let path = generation_dir.join(sealed_segment_name(0));
        write_new_synced(&path, &[])?;
        carrier_paths.push(path);
    }
    generation_index.sort_by(|left, right| {
        left.logical_key
            .as_bytes()
            .cmp(right.logical_key.as_bytes())
    });
    write_json_new(
        &generation_dir.join(GENERATION_INDEX_FILE),
        &SegmentIndexV1 {
            schema: "pointbreak.segment-index.v1".to_owned(),
            head_marker: head.head_marker,
            entries: generation_index,
        },
    )?;
    let carriers = carrier_paths
        .iter()
        .map(|path| {
            let bytes = fs::read(path).map_err(|error| io_error(path, error))?;
            Ok(SealedCarrierV1 {
                relative_path: relative_path_string(&generation_dir, path)?,
                encoded_bytes: bytes.len() as u64,
                encoded_sha256: sha256_bytes_hex(&bytes),
            })
        })
        .collect::<Result<Vec<_>, SegmentQualificationError>>()?;
    write_json_new(
        &generation_dir.join(GENERATION_MANIFEST_FILE),
        &SealedGenerationManifestV1 {
            schema: "pointbreak.sealed-generation.v1".to_owned(),
            generation,
            head_marker: head.head_marker,
            carriers,
        },
    )?;
    sync_directory(&generation_dir)?;
    sync_directory(&root.join(GENERATION_DIRECTORY))?;
    inject_if(failure, SegmentFailurePointV1::AfterSealedGenerationSync)?;

    let new_active = active_relative_path(generation);
    create_preallocated(&root.join(&new_active), segment_bytes)?;
    let published = SegmentHeadV1 {
        schema: head.schema.clone(),
        active_file: new_active,
        committed_active_bytes: 0,
        head_marker: head.head_marker,
        next_sequence: head.next_sequence,
        current_generation: Some(generation),
        next_generation: generation + 1,
    };
    write_json_atomic(&root.join(SEGMENT_HEAD_FILE_V1), &published)?;
    sync_directory(root)?;
    inject_if(failure, SegmentFailurePointV1::AfterGenerationPublish)?;
    let published_frames = scan_visible(root, &published, segment_bytes)?;
    write_index(root, &published, &published_frames)?;
    if head.active_file != published.active_file {
        let old_active = root.join(&head.active_file);
        if old_active
            .try_exists()
            .map_err(|error| io_error(&old_active, error))?
        {
            fs::remove_file(&old_active).map_err(|error| io_error(&old_active, error))?;
        }
    }
    Ok(generation)
}

fn encode_frame(
    logical_key: &str,
    decoded_bytes: &[u8],
    sequence: u64,
) -> Result<Vec<u8>, SegmentQualificationError> {
    let pbrf = PhysicalRecordV1::encode(
        logical_key,
        QualificationRecordKindV1::LegacyEvent,
        decoded_bytes,
        &NeverCancelled,
    )
    .map_err(|error| SegmentQualificationError::InvalidProfile {
        message: error.to_string(),
    })?;
    let key = logical_key.as_bytes();
    let frame_bytes =
        SEGMENT_FRAME_HEADER_LEN_V1 + key.len() + pbrf.len() + SEGMENT_FRAME_FOOTER_LEN_V1;
    let mut header = [0_u8; SEGMENT_FRAME_HEADER_LEN_V1];
    header[0..4].copy_from_slice(FRAME_MAGIC);
    header[4..6].copy_from_slice(&FRAME_VERSION.to_le_bytes());
    header[6..8].copy_from_slice(&(SEGMENT_FRAME_HEADER_LEN_V1 as u16).to_le_bytes());
    header[8..16].copy_from_slice(&sequence.to_le_bytes());
    header[16..20].copy_from_slice(&(key.len() as u32).to_le_bytes());
    header[24..32].copy_from_slice(&(pbrf.len() as u64).to_le_bytes());
    header[32..64].copy_from_slice(&logical_key_digest(logical_key));
    let header_hash = Sha256::digest(&header[..64]);
    header[64..96].copy_from_slice(&header_hash);
    let mut body = Vec::with_capacity(frame_bytes);
    body.extend_from_slice(&header);
    body.extend_from_slice(key);
    body.extend_from_slice(&pbrf);
    let body_hash = Sha256::digest(&body);
    let mut footer = [0_u8; SEGMENT_FRAME_FOOTER_LEN_V1];
    footer[0..4].copy_from_slice(FOOTER_MAGIC);
    footer[4..6].copy_from_slice(&FRAME_VERSION.to_le_bytes());
    footer[6..8].copy_from_slice(&(SEGMENT_FRAME_FOOTER_LEN_V1 as u16).to_le_bytes());
    footer[8..16].copy_from_slice(&sequence.to_le_bytes());
    footer[16..24].copy_from_slice(&(frame_bytes as u64).to_le_bytes());
    footer[24..56].copy_from_slice(&body_hash);
    body.extend_from_slice(&footer);
    Ok(body)
}

fn scan_visible(
    root: &Path,
    head: &SegmentHeadV1,
    segment_bytes: u64,
) -> Result<Vec<ScannedFrame>, SegmentQualificationError> {
    let mut frames = Vec::new();
    if let Some(generation) = head.current_generation {
        frames.extend(scan_generation(root, generation, segment_bytes)?);
    }
    frames.extend(scan_segment(
        root,
        &root.join(&head.active_file),
        head.committed_active_bytes,
    )?);
    Ok(frames)
}

fn scan_generation(
    root: &Path,
    generation: u64,
    segment_bytes: u64,
) -> Result<Vec<ScannedFrame>, SegmentQualificationError> {
    let directory = generation_directory_path(root, generation);
    let manifest: SealedGenerationManifestV1 =
        read_json(&directory.join(GENERATION_MANIFEST_FILE))?;
    if manifest.schema != "pointbreak.sealed-generation.v1" || manifest.generation != generation {
        return Err(SegmentQualificationError::Corruption {
            message: format!("invalid manifest for generation {generation}"),
        });
    }
    let mut paths = sealed_segment_paths(&directory)?;
    if paths.is_empty() {
        return Err(SegmentQualificationError::Corruption {
            message: format!("generation {generation} contains no sealed segments"),
        });
    }
    let actual = paths
        .iter()
        .map(|path| {
            let bytes = fs::read(path).map_err(|error| io_error(path, error))?;
            if bytes.len() as u64 > segment_bytes {
                return Err(SegmentQualificationError::Corruption {
                    message: format!("sealed segment {} exceeds configured bound", path.display()),
                });
            }
            Ok(SealedCarrierV1 {
                relative_path: relative_path_string(&directory, path)?,
                encoded_bytes: bytes.len() as u64,
                encoded_sha256: sha256_bytes_hex(&bytes),
            })
        })
        .collect::<Result<Vec<_>, SegmentQualificationError>>()?;
    if actual.len() != manifest.carriers.len()
        || actual.iter().zip(&manifest.carriers).any(|(left, right)| {
            left.relative_path != right.relative_path
                || left.encoded_bytes != right.encoded_bytes
                || left.encoded_sha256 != right.encoded_sha256
        })
    {
        return Err(SegmentQualificationError::Corruption {
            message: format!("sealed generation {generation} carrier manifest mismatch"),
        });
    }
    let mut frames = Vec::new();
    for path in paths.drain(..) {
        let len = fs::metadata(&path)
            .map_err(|error| io_error(&path, error))?
            .len();
        frames.extend(scan_segment(root, &path, len)?);
    }
    Ok(frames)
}

fn scan_segment(
    root: &Path,
    path: &Path,
    committed_bytes: u64,
) -> Result<Vec<ScannedFrame>, SegmentQualificationError> {
    let mut file = File::open(path).map_err(|error| io_error(path, error))?;
    let file_len = file
        .metadata()
        .map_err(|error| io_error(path, error))?
        .len();
    if file_len < committed_bytes {
        return Err(SegmentQualificationError::Corruption {
            message: format!(
                "carrier {} has {file_len} bytes below committed length {committed_bytes}",
                path.display()
            ),
        });
    }
    let mut bytes = vec![0_u8; committed_bytes as usize];
    file.read_exact(&mut bytes)
        .map_err(|error| io_error(path, error))?;
    let carrier = relative_path_string(root, path)?;
    let mut offset = 0_usize;
    let mut frames = Vec::new();
    while offset < bytes.len() {
        let remaining = &bytes[offset..];
        if remaining.len() < SEGMENT_FRAME_HEADER_LEN_V1 + SEGMENT_FRAME_FOOTER_LEN_V1 {
            return Err(frame_corruption(path, offset, "truncated frame"));
        }
        let header = &remaining[..SEGMENT_FRAME_HEADER_LEN_V1];
        if &header[0..4] != FRAME_MAGIC
            || read_u16(header, 4)? != FRAME_VERSION
            || read_u16(header, 6)? as usize != SEGMENT_FRAME_HEADER_LEN_V1
            || header[20..24].iter().any(|byte| *byte != 0)
            || Sha256::digest(&header[..64]).as_slice() != &header[64..96]
        {
            return Err(frame_corruption(path, offset, "invalid frame header"));
        }
        let sequence = read_u64(header, 8)?;
        let key_len = read_u32(header, 16)? as usize;
        let pbrf_len = read_u64(header, 24)? as usize;
        if key_len == 0 || key_len > MAX_LOGICAL_KEY_BYTES {
            return Err(frame_corruption(path, offset, "invalid logical-key length"));
        }
        let frame_len = SEGMENT_FRAME_HEADER_LEN_V1
            .checked_add(key_len)
            .and_then(|value| value.checked_add(pbrf_len))
            .and_then(|value| value.checked_add(SEGMENT_FRAME_FOOTER_LEN_V1))
            .ok_or_else(|| frame_corruption(path, offset, "frame length overflow"))?;
        if frame_len > remaining.len() {
            return Err(frame_corruption(path, offset, "truncated committed frame"));
        }
        let key_start = SEGMENT_FRAME_HEADER_LEN_V1;
        let pbrf_start = key_start + key_len;
        let footer_start = pbrf_start + pbrf_len;
        let key = std::str::from_utf8(&remaining[key_start..pbrf_start]).map_err(|error| {
            frame_corruption(path, offset, &format!("logical key is not UTF-8: {error}"))
        })?;
        if logical_key_digest(key).as_slice() != &header[32..64] {
            return Err(frame_corruption(
                path,
                offset,
                "logical-key digest mismatch",
            ));
        }
        let footer = &remaining[footer_start..frame_len];
        if &footer[0..4] != FOOTER_MAGIC
            || read_u16(footer, 4)? != FRAME_VERSION
            || read_u16(footer, 6)? as usize != SEGMENT_FRAME_FOOTER_LEN_V1
            || read_u64(footer, 8)? != sequence
            || read_u64(footer, 16)? != frame_len as u64
            || footer[56..64].iter().any(|byte| *byte != 0)
            || Sha256::digest(&remaining[..footer_start]).as_slice() != &footer[24..56]
        {
            return Err(frame_corruption(path, offset, "invalid commit footer"));
        }
        let decoded =
            PhysicalRecordV1::decode(&remaining[pbrf_start..footer_start], &NeverCancelled)
                .map_err(|error| frame_corruption(path, offset, &error.to_string()))?;
        if decoded.record_kind != PhysicalRecordKindV1::Event
            || decoded.logical_key_digest != logical_key_digest(key)
        {
            return Err(frame_corruption(path, offset, "PBRF identity mismatch"));
        }
        frames.push(ScannedFrame {
            entry: QualificationEntry {
                logical_key: key.to_owned(),
                decoded_sha256: sha256_bytes_hex(&decoded.decoded_bytes),
                decoded_bytes: decoded.decoded_bytes,
            },
            sequence,
            carrier: carrier.clone(),
            offset: offset as u64,
            frame_bytes: frame_len as u64,
        });
        offset += frame_len;
    }
    Ok(frames)
}

fn validate_visible_frames(
    head: &SegmentHeadV1,
    frames: &[ScannedFrame],
) -> Result<(), SegmentQualificationError> {
    if frames.len() as u64 != head.head_marker {
        return Err(SegmentQualificationError::Corruption {
            message: format!(
                "head marker {} differs from visible record count {}",
                head.head_marker,
                frames.len()
            ),
        });
    }
    let mut keys = BTreeSet::new();
    let mut sequences = BTreeSet::new();
    for frame in frames {
        if !keys.insert(frame.entry.logical_key.clone()) {
            return Err(SegmentQualificationError::Corruption {
                message: format!("duplicate logical key {}", frame.entry.logical_key),
            });
        }
        if !sequences.insert(frame.sequence) {
            return Err(SegmentQualificationError::Corruption {
                message: format!("duplicate append sequence {}", frame.sequence),
            });
        }
    }
    if frames.iter().map(|frame| frame.sequence).max().unwrap_or(0) + 1 != head.next_sequence {
        return Err(SegmentQualificationError::Corruption {
            message: "next append sequence does not follow the visible prefix".to_owned(),
        });
    }
    Ok(())
}

fn recover_active_tail(
    root: &Path,
    head: &SegmentHeadV1,
    segment_bytes: u64,
) -> Result<SegmentRecoveryStateV1, SegmentQualificationError> {
    let path = root.join(&head.active_file);
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|error| io_error(&path, error))?;
    let len = file
        .metadata()
        .map_err(|error| io_error(&path, error))?
        .len();
    if len < head.committed_active_bytes {
        return Err(SegmentQualificationError::Corruption {
            message: format!(
                "active tail has {len} bytes below committed length {}",
                head.committed_active_bytes
            ),
        });
    }
    if len > segment_bytes {
        return Err(SegmentQualificationError::Corruption {
            message: format!("active tail has {len} bytes above segment bound {segment_bytes}"),
        });
    }
    let suffix_len = len.saturating_sub(head.committed_active_bytes);
    file.seek(SeekFrom::Start(head.committed_active_bytes))
        .map_err(|error| io_error(&path, error))?;
    let mut suffix = vec![0_u8; suffix_len as usize];
    file.read_exact(&mut suffix)
        .map_err(|error| io_error(&path, error))?;
    let dirty = suffix.iter().filter(|byte| **byte != 0).count() as u64;
    if dirty > 0 {
        file.seek(SeekFrom::Start(head.committed_active_bytes))
            .map_err(|error| io_error(&path, error))?;
        let zeros = vec![0_u8; suffix_len as usize];
        file.write_all(&zeros)
            .and_then(|_| file.sync_all())
            .map_err(|error| io_error(&path, error))?;
        Ok(SegmentRecoveryStateV1::DiscardedUncommittedSuffix { bytes: dirty })
    } else {
        Ok(SegmentRecoveryStateV1::Healthy)
    }
}

fn ensure_index(
    root: &Path,
    head: &SegmentHeadV1,
    frames: &[ScannedFrame],
) -> Result<(), SegmentQualificationError> {
    let expected = build_index(head, frames);
    let path = root.join(SEGMENT_INDEX_FILE_V1);
    let actual = read_json::<SegmentIndexV1>(&path).ok();
    if actual.as_ref() != Some(&expected) {
        write_json_atomic(&path, &expected)?;
    }
    Ok(())
}

fn write_index(
    root: &Path,
    head: &SegmentHeadV1,
    frames: &[ScannedFrame],
) -> Result<(), SegmentQualificationError> {
    let path = root.join(SEGMENT_INDEX_FILE_V1);
    if path.try_exists().map_err(|error| io_error(&path, error))? {
        write_json_atomic(&path, &build_index(head, frames))
    } else {
        write_json_new(&path, &build_index(head, frames))
    }
}

fn build_index(head: &SegmentHeadV1, frames: &[ScannedFrame]) -> SegmentIndexV1 {
    let mut entries = frames
        .iter()
        .map(|frame| SegmentIndexEntryV1 {
            logical_key: frame.entry.logical_key.clone(),
            decoded_sha256: frame.entry.decoded_sha256.clone(),
            sequence: frame.sequence,
            carrier: frame.carrier.clone(),
            offset: frame.offset,
            frame_bytes: frame.frame_bytes,
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        left.logical_key
            .as_bytes()
            .cmp(right.logical_key.as_bytes())
    });
    SegmentIndexV1 {
        schema: "pointbreak.segment-index.v1".to_owned(),
        head_marker: head.head_marker,
        entries,
    }
}

fn segment_inventory(
    root: &Path,
    segment_bytes: u64,
) -> Result<SegmentInventoryEvidenceV1, SegmentQualificationError> {
    let head = read_head(root)?;
    let generations = generation_numbers(root)?;
    let current_generation_file = head.current_generation.map(generation_relative_path);
    let retired_generation_bytes = generations
        .iter()
        .filter(|generation| Some(**generation) != head.current_generation)
        .try_fold(0_u64, |total, generation| {
            directory_encoded_bytes(&generation_directory_path(root, *generation))?
                .checked_add(total)
                .ok_or(SegmentQualificationError::InventoryOverflow)
        })?;
    let files = collect_files(root)?;
    let high_water_bytes = files.iter().try_fold(0_u64, |total, file| {
        total
            .checked_add(file.1)
            .ok_or(SegmentQualificationError::InventoryOverflow)
    })?;
    Ok(SegmentInventoryEvidenceV1 {
        active_file: head.active_file,
        active_committed_bytes: head.committed_active_bytes,
        active_slack_bytes: segment_bytes.saturating_sub(head.committed_active_bytes),
        current_generation_file,
        retained_generations: generations.len() as u64,
        retired_generation_bytes,
        index_bytes: fs::metadata(root.join(SEGMENT_INDEX_FILE_V1))
            .map_err(|error| io_error(&root.join(SEGMENT_INDEX_FILE_V1), error))?
            .len(),
        head_bytes: fs::metadata(root.join(SEGMENT_HEAD_FILE_V1))
            .map_err(|error| io_error(&root.join(SEGMENT_HEAD_FILE_V1), error))?
            .len(),
        high_water_bytes,
    })
}

fn validate_root_read_only(
    root: &Path,
    expected_header: Option<&PhysicalStoreHeaderV1>,
) -> Result<(), SegmentQualificationError> {
    let header = read_profile_header(&root.join(PROFILE_FILE))?;
    if let Some(expected) = expected_header
        && &header != expected
    {
        return Err(SegmentQualificationError::InvalidProfile {
            message: "restored PBST header differs from source".to_owned(),
        });
    }
    let descriptor = segment_descriptor();
    let metadata: SegmentMetadataV1 = read_json(&root.join(METADATA_FILE))?;
    validate_metadata(&metadata, &descriptor)?;
    let head = read_head(root)?;
    validate_head(&head, metadata.segment_bytes)?;
    validate_retained_generations(root, metadata.segment_bytes)?;
    let frames = scan_visible(root, &head, metadata.segment_bytes)?;
    validate_visible_frames(&head, &frames)?;
    let index: SegmentIndexV1 = read_json(&root.join(SEGMENT_INDEX_FILE_V1))?;
    if index != build_index(&head, &frames) {
        return Err(SegmentQualificationError::Corruption {
            message: "restored segment index is stale or invalid".to_owned(),
        });
    }
    let content_root = root.join(CONTENT_DIRECTORY);
    if content_root
        .try_exists()
        .map_err(|error| io_error(&content_root, error))?
    {
        IndependentContentStoreV1::open(&content_root)
            .map_err(|error| SegmentQualificationError::InvalidProfile {
                message: error.to_string(),
            })?
            .list()
            .map_err(|error| SegmentQualificationError::InvalidProfile {
                message: error.to_string(),
            })?;
    }
    Ok(())
}

fn validate_retained_generations(
    root: &Path,
    segment_bytes: u64,
) -> Result<(), SegmentQualificationError> {
    for generation in generation_numbers(root)? {
        scan_generation(root, generation, segment_bytes)?;
    }
    Ok(())
}

fn cleanup_unpublished_generations(
    root: &Path,
    head: &SegmentHeadV1,
) -> Result<(), SegmentQualificationError> {
    for generation in generation_numbers(root)? {
        if generation >= head.next_generation {
            let path = generation_directory_path(root, generation);
            fs::remove_dir_all(&path).map_err(|error| io_error(&path, error))?;
        }
    }
    Ok(())
}

fn cleanup_unreferenced_active_files(
    root: &Path,
    head: &SegmentHeadV1,
) -> Result<(), SegmentQualificationError> {
    let directory = root.join(ACTIVE_DIRECTORY);
    for entry in fs::read_dir(&directory).map_err(|error| io_error(&directory, error))? {
        let entry = entry.map_err(|error| io_error(&directory, error))?;
        let relative = relative_path_string(root, &entry.path())?;
        if relative != head.active_file {
            fs::remove_file(entry.path()).map_err(|error| io_error(&entry.path(), error))?;
        }
    }
    Ok(())
}

fn generation_is_pinned(root: &Path, generation: u64) -> Result<bool, SegmentQualificationError> {
    let directory = root.join(PIN_DIRECTORY);
    for entry in fs::read_dir(&directory).map_err(|error| io_error(&directory, error))? {
        let entry = entry.map_err(|error| io_error(&directory, error))?;
        let path = entry.path();
        if read_pin_generation(&path)? != Some(generation) {
            continue;
        }
        let file = match OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(io_error(&path, error)),
        };
        match file.try_lock() {
            Ok(()) => {
                drop(file);
                if let Err(error) = fs::remove_file(&path)
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    return Err(io_error(&path, error));
                }
            }
            Err(std::fs::TryLockError::WouldBlock) => return Ok(true),
            Err(std::fs::TryLockError::Error(error)) => return Err(io_error(&path, error)),
        }
    }
    Ok(false)
}

fn read_pin_generation(path: &Path) -> Result<Option<u64>, SegmentQualificationError> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(value.trim().parse::<u64>().ok()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error(path, error)),
    }
}

fn generation_numbers(root: &Path) -> Result<Vec<u64>, SegmentQualificationError> {
    let directory = root.join(GENERATION_DIRECTORY);
    let mut generations = Vec::new();
    for entry in fs::read_dir(&directory).map_err(|error| io_error(&directory, error))? {
        let entry = entry.map_err(|error| io_error(&directory, error))?;
        if !entry
            .file_type()
            .map_err(|error| io_error(&entry.path(), error))?
            .is_dir()
        {
            return Err(SegmentQualificationError::Corruption {
                message: format!("unexpected generation carrier {}", entry.path().display()),
            });
        }
        let name =
            entry
                .file_name()
                .into_string()
                .map_err(|_| SegmentQualificationError::Corruption {
                    message: "generation directory name is not UTF-8".to_owned(),
                })?;
        let generation =
            name.parse::<u64>()
                .map_err(|_| SegmentQualificationError::Corruption {
                    message: format!("invalid generation directory {name}"),
                })?;
        generations.push(generation);
    }
    generations.sort_unstable();
    Ok(generations)
}

fn sealed_segment_paths(directory: &Path) -> Result<Vec<PathBuf>, SegmentQualificationError> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory).map_err(|error| io_error(directory, error))? {
        let entry = entry.map_err(|error| io_error(directory, error))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("events-") && name.ends_with(".seg") {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}

fn segment_descriptor() -> QualificationProfileDescriptorV1 {
    QualificationProfileDescriptorV1 {
        physical_profile_id: SEGMENT_QUALIFICATION_PROFILE_ID_V1.to_owned(),
        logical_capabilities: LogicalCapabilityEpochV1::foundation(),
    }
}

fn validate_metadata(
    metadata: &SegmentMetadataV1,
    descriptor: &QualificationProfileDescriptorV1,
) -> Result<(), SegmentQualificationError> {
    if metadata.schema != "pointbreak.segment-profile.v1" || &metadata.descriptor != descriptor {
        return Err(SegmentQualificationError::InvalidProfile {
            message: "segment metadata does not match the supported profile".to_owned(),
        });
    }
    validate_segment_bytes(metadata.segment_bytes)
}

fn validate_segment_bytes(segment_bytes: u64) -> Result<(), SegmentQualificationError> {
    if SEGMENT_SIZE_CANDIDATES_V1.contains(&segment_bytes) {
        Ok(())
    } else {
        Err(SegmentQualificationError::InvalidProfile {
            message: format!("unsupported segment size {segment_bytes}"),
        })
    }
}

fn validate_head(
    head: &SegmentHeadV1,
    segment_bytes: u64,
) -> Result<(), SegmentQualificationError> {
    if head.schema != "pointbreak.segment-head.v1"
        || !head.active_file.starts_with("active/")
        || head.committed_active_bytes > segment_bytes
        || head.next_sequence != head.head_marker + 1
        || head.next_generation == 0
    {
        return Err(SegmentQualificationError::InvalidProfile {
            message: "segment head contains invalid coordinates".to_owned(),
        });
    }
    Ok(())
}

fn read_profile_header(path: &Path) -> Result<PhysicalStoreHeaderV1, SegmentQualificationError> {
    let bytes = fs::read(path).map_err(|error| io_error(path, error))?;
    let header = PhysicalStoreHeaderV1::decode(&bytes).map_err(|error| {
        SegmentQualificationError::InvalidProfile {
            message: error.to_string(),
        }
    })?;
    if header.profile_id != SEGMENT_PROFILE_ID {
        return Err(SegmentQualificationError::InvalidProfile {
            message: format!(
                "PBST profile id {} is not the segment profile",
                header.profile_id
            ),
        });
    }
    Ok(header)
}

fn read_head(root: &Path) -> Result<SegmentHeadV1, SegmentQualificationError> {
    read_json(&root.join(SEGMENT_HEAD_FILE_V1))
}

#[cfg(test)]
fn read_head_for_test(root: &Path) -> SegmentHeadV1 {
    read_head(root).expect("segment head")
}

fn active_relative_path(coordinate: u64) -> String {
    format!("{ACTIVE_DIRECTORY}/{coordinate:016}.seg")
}

fn generation_directory_path(root: &Path, generation: u64) -> PathBuf {
    root.join(GENERATION_DIRECTORY)
        .join(format!("{generation:016}"))
}

pub fn generation_relative_path(generation: u64) -> String {
    format!(
        "{GENERATION_DIRECTORY}/{generation:016}/{}",
        sealed_segment_name(0)
    )
}

pub fn generation_segment_path(root: &Path, generation: u64) -> PathBuf {
    root.join(generation_relative_path(generation))
}

fn sealed_segment_name(number: u64) -> String {
    format!("events-{number:06}.seg")
}

fn inject_if(
    actual: Option<SegmentFailurePointV1>,
    expected: SegmentFailurePointV1,
) -> Result<(), SegmentQualificationError> {
    if actual == Some(expected) {
        Err(SegmentQualificationError::InjectedFailure { point: expected })
    } else {
        Ok(())
    }
}

fn frame_corruption(path: &Path, offset: usize, message: &str) -> SegmentQualificationError {
    SegmentQualificationError::Corruption {
        message: format!("{} at byte {offset}: {message}", path.display()),
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, SegmentQualificationError> {
    let value =
        bytes
            .get(offset..offset + 2)
            .ok_or_else(|| SegmentQualificationError::Corruption {
                message: "truncated u16".to_owned(),
            })?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, SegmentQualificationError> {
    let value =
        bytes
            .get(offset..offset + 4)
            .ok_or_else(|| SegmentQualificationError::Corruption {
                message: "truncated u32".to_owned(),
            })?;
    Ok(u32::from_le_bytes(value.try_into().expect("four bytes")))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, SegmentQualificationError> {
    let value =
        bytes
            .get(offset..offset + 8)
            .ok_or_else(|| SegmentQualificationError::Corruption {
                message: "truncated u64".to_owned(),
            })?;
    Ok(u64::from_le_bytes(value.try_into().expect("eight bytes")))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, SegmentQualificationError> {
    let bytes = fs::read(path).map_err(|error| io_error(path, error))?;
    serde_json::from_slice(&bytes).map_err(|error| SegmentQualificationError::Corruption {
        message: format!("invalid JSON at {}: {error}", path.display()),
    })
}

fn json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, SegmentQualificationError> {
    let value =
        serde_json::to_value(value).map_err(|error| SegmentQualificationError::InvalidProfile {
            message: error.to_string(),
        })?;
    canonical_json_bytes(&value).map_err(|error| SegmentQualificationError::InvalidProfile {
        message: error.to_string(),
    })
}

fn write_json_new<T: Serialize>(path: &Path, value: &T) -> Result<(), SegmentQualificationError> {
    write_new_synced(path, &json_bytes(value)?)
}

fn write_json_atomic<T: Serialize>(
    path: &Path,
    value: &T,
) -> Result<(), SegmentQualificationError> {
    let parent = path
        .parent()
        .ok_or_else(|| SegmentQualificationError::InvalidProfile {
            message: format!("{} has no parent", path.display()),
        })?;
    fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("segment"),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    write_new_synced(&temporary, &json_bytes(value)?)?;
    fs::rename(&temporary, path).map_err(|error| io_error(path, error))?;
    sync_directory(parent)
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), SegmentQualificationError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| io_error(path, error))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| io_error(path, error))
}

fn create_preallocated(path: &Path, bytes: u64) -> Result<(), SegmentQualificationError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
    }
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| io_error(path, error))?;
    file.set_len(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| io_error(path, error))
}

fn copy_file_synced(source: &Path, destination: &Path) -> Result<(), SegmentQualificationError> {
    let bytes = fs::read(source).map_err(|error| io_error(source, error))?;
    write_new_synced(destination, &bytes)
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), SegmentQualificationError> {
    fs::create_dir_all(destination).map_err(|error| io_error(destination, error))?;
    let mut entries = fs::read_dir(source)
        .map_err(|error| io_error(source, error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| io_error(source, error))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let target = destination.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| io_error(&entry.path(), error))?
            .is_dir()
        {
            copy_tree(&entry.path(), &target)?;
        } else {
            copy_file_synced(&entry.path(), &target)?;
        }
    }
    sync_directory(destination)
}

fn collect_files(root: &Path) -> Result<Vec<(String, u64)>, SegmentQualificationError> {
    fn visit(
        root: &Path,
        current: &Path,
        files: &mut Vec<(String, u64)>,
    ) -> Result<(), SegmentQualificationError> {
        let mut entries = fs::read_dir(current)
            .map_err(|error| io_error(current, error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| io_error(current, error))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let file_type = entry.file_type().map_err(|error| io_error(&path, error))?;
            if file_type.is_dir() {
                visit(root, &path, files)?;
            } else if file_type.is_file() {
                files.push((
                    relative_path_string(root, &path)?,
                    entry
                        .metadata()
                        .map_err(|error| io_error(&path, error))?
                        .len(),
                ));
            } else {
                return Err(SegmentQualificationError::Corruption {
                    message: format!("unexpected carrier {}", path.display()),
                });
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    visit(root, root, &mut files)?;
    Ok(files)
}

fn directory_encoded_bytes(path: &Path) -> Result<u64, SegmentQualificationError> {
    collect_files(path)?
        .into_iter()
        .try_fold(0_u64, |total, file| {
            total
                .checked_add(file.1)
                .ok_or(SegmentQualificationError::InventoryOverflow)
        })
}

fn relative_path_string(root: &Path, path: &Path) -> Result<String, SegmentQualificationError> {
    let relative =
        path.strip_prefix(root)
            .map_err(|error| SegmentQualificationError::InvalidProfile {
                message: error.to_string(),
            })?;
    Ok(relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn sync_file(file: File, path: &Path) -> Result<(), SegmentQualificationError> {
    file.sync_all().map_err(|error| io_error(path, error))
}

#[cfg(not(target_os = "windows"))]
fn sync_directory(path: &Path) -> Result<(), SegmentQualificationError> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| io_error(path, error))
}

#[cfg(target_os = "windows")]
fn sync_directory(_path: &Path) -> Result<(), SegmentQualificationError> {
    Ok(())
}

fn io_error(path: &Path, error: std::io::Error) -> SegmentQualificationError {
    SegmentQualificationError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, OpenOptions};
    use std::io::{Seek, SeekFrom, Write};
    use std::process::Command;

    use super::*;
    use crate::bench_support::foundation::{
        BACKUP_COMPLETION_FILE_V1, QualificationCreateOutcome, QualificationProfile,
        QualificationRecordKindV1, modeled_post_foundation_manifest, run_profile_contract_vectors,
        synthetic_legacy_manifest,
    };

    const CHILD_ENV: &str = "POINTBREAK_SEGMENT_TEST_CHILD";
    const CHILD_TEST: &str =
        "bench_support::foundation::segments::tests::segment_subprocess_entrypoint";

    #[test]
    fn segment_profile_passes_the_shared_composed_contract() {
        let root = tempfile::tempdir().expect("profile root");
        let backup_parent = tempfile::tempdir().expect("backup parent");
        let profile = SegmentQualificationProfile::open(root.path()).expect("segment profile");

        run_profile_contract_vectors(&profile, &backup_parent.path().join("completed"))
            .expect("shared contract");
    }

    #[test]
    fn both_synthetic_workloads_measure_all_segment_sizes() {
        let roots = tempfile::tempdir().expect("workload roots");
        let legacy = synthetic_legacy_manifest().expect("legacy workload");
        let modeled = modeled_post_foundation_manifest().expect("modeled workload");

        let legacy_evidence = run_segment_workload(&roots.path().join("legacy"), &legacy)
            .expect("legacy segment workload");
        let modeled_evidence = run_segment_workload(&roots.path().join("modeled"), &modeled)
            .expect("modeled segment workload");

        assert_eq!(legacy_evidence.records, legacy.records.len() as u64);
        assert_eq!(modeled_evidence.records, modeled.records.len() as u64);
        assert_eq!(
            modeled_evidence
                .measurements
                .iter()
                .map(|measurement| measurement.segment_bytes)
                .collect::<Vec<_>>(),
            SEGMENT_SIZE_CANDIDATES_V1
        );
        assert_eq!(
            modeled_evidence.selected_segment_bytes,
            DEFAULT_SEGMENT_BYTES_V1
        );
    }

    #[test]
    fn append_failure_points_recover_to_exactly_the_published_prefix() {
        for point in [
            SegmentFailurePointV1::AfterRecordBytes,
            SegmentFailurePointV1::AfterCommitFooter,
            SegmentFailurePointV1::AfterTailSync,
        ] {
            let root = tempfile::tempdir().expect("profile root");
            let profile = SegmentQualificationProfile::open_with_segment_bytes(
                root.path(),
                SEGMENT_SIZE_CANDIDATES_V1[0],
            )
            .expect("segment profile");
            assert!(
                profile
                    .create_once_with_failure("events/one", b"one", point)
                    .is_err()
            );
            drop(profile);

            let reopened = SegmentQualificationProfile::open(root.path()).expect("recover profile");
            assert_eq!(reopened.journal().read("events/one").unwrap(), None);
            assert_eq!(
                reopened
                    .journal()
                    .create_once("events/one", b"one")
                    .unwrap(),
                QualificationCreateOutcome::Created
            );
        }

        for point in [
            SegmentFailurePointV1::AfterHeadPublish,
            SegmentFailurePointV1::AfterIndexPublish,
        ] {
            let root = tempfile::tempdir().expect("profile root");
            let profile = SegmentQualificationProfile::open(root.path()).expect("segment profile");
            assert!(
                profile
                    .create_once_with_failure("events/one", b"one", point)
                    .is_err()
            );
            drop(profile);

            let reopened = SegmentQualificationProfile::open(root.path()).expect("recover profile");
            assert_eq!(
                reopened
                    .journal()
                    .read("events/one")
                    .unwrap()
                    .expect("committed entry")
                    .decoded_bytes,
                b"one"
            );
            assert_eq!(
                reopened
                    .journal()
                    .create_once("events/one", b"one")
                    .unwrap(),
                QualificationCreateOutcome::AlreadyExists
            );
        }
    }

    #[test]
    fn index_is_rebuildable_but_acknowledged_corruption_is_loud() {
        let root = tempfile::tempdir().expect("profile root");
        let profile = SegmentQualificationProfile::open(root.path()).expect("segment profile");
        profile.journal().create_once("events/a", b"a").unwrap();
        profile.journal().create_once("events/b", b"b").unwrap();
        let generation = profile.seal_active().expect("sealed generation");
        drop(profile);

        fs::remove_file(root.path().join(SEGMENT_INDEX_FILE_V1)).expect("remove index");
        let rebuilt = SegmentQualificationProfile::open(root.path()).expect("rebuild index");
        assert_eq!(rebuilt.journal().list().unwrap().len(), 2);
        drop(rebuilt);

        let segment = generation_segment_path(root.path(), generation);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&segment)
            .expect("open sealed segment");
        file.seek(SeekFrom::Start(SEGMENT_FRAME_HEADER_LEN_V1 as u64 + 1))
            .unwrap();
        file.write_all(&[0xff]).unwrap();
        file.sync_all().unwrap();

        assert!(SegmentQualificationProfile::open(root.path()).is_err());
        assert!(matches!(
            SegmentQualificationProfile::diagnose_root(root.path()),
            SegmentDiagnosticStateV1::StructuralCorruption { .. }
        ));
    }

    #[test]
    fn committed_length_discards_only_an_uncommitted_suffix() {
        let root = tempfile::tempdir().expect("profile root");
        let profile = SegmentQualificationProfile::open(root.path()).expect("segment profile");
        profile.journal().create_once("events/a", b"a").unwrap();
        let before = profile.segment_inventory_evidence().unwrap();
        drop(profile);

        let active = root.path().join(&before.active_file);
        let mut file = OpenOptions::new().write(true).open(&active).unwrap();
        file.seek(SeekFrom::Start(before.active_committed_bytes))
            .unwrap();
        file.write_all(b"torn-uncommitted-frame").unwrap();
        file.sync_all().unwrap();
        drop(file);

        let reopened = SegmentQualificationProfile::open(root.path()).expect("recover suffix");
        assert!(matches!(
            reopened.recovery_state(),
            SegmentRecoveryStateV1::DiscardedUncommittedSuffix { .. }
        ));
        assert_eq!(reopened.journal().list().unwrap().len(), 1);

        let head = read_head_for_test(root.path());
        OpenOptions::new()
            .write(true)
            .open(root.path().join(&head.active_file))
            .unwrap()
            .set_len(head.committed_active_bytes.saturating_sub(1))
            .unwrap();
        assert!(SegmentQualificationProfile::open(root.path()).is_err());
    }

    #[test]
    fn generation_publication_is_recoverable_and_reader_pins_block_retirement() {
        let root = tempfile::tempdir().expect("profile root");
        let profile = SegmentQualificationProfile::open(root.path()).expect("segment profile");
        profile.journal().create_once("events/a", b"a").unwrap();
        let first = profile.seal_active().expect("first generation");
        let pin = profile.pin_reader().expect("reader pin");
        assert_eq!(pin.generation(), first);

        profile.journal().create_once("events/b", b"b").unwrap();
        let second = profile.seal_active().expect("second generation");
        assert!(second > first);
        assert!(profile.retire_generation(first).is_err());
        drop(pin);
        profile
            .retire_generation(first)
            .expect("retire unpinned generation");
        assert_eq!(profile.journal().list().unwrap().len(), 2);

        profile.journal().create_once("events/c", b"c").unwrap();
        assert!(
            profile
                .seal_active_with_failure(SegmentFailurePointV1::AfterSealedGenerationSync)
                .is_err()
        );
        drop(profile);
        let reopened =
            SegmentQualificationProfile::open(root.path()).expect("recover orphan generation");
        assert_eq!(reopened.journal().list().unwrap().len(), 3);

        assert!(
            reopened
                .seal_active_with_failure(SegmentFailurePointV1::AfterGenerationPublish)
                .is_err()
        );
        drop(reopened);
        let acknowledged =
            SegmentQualificationProfile::open(root.path()).expect("recover published generation");
        assert_eq!(acknowledged.journal().list().unwrap().len(), 3);
    }

    #[test]
    fn same_process_seal_retry_discards_an_unpublished_generation() {
        let root = tempfile::tempdir().expect("profile root");
        let profile = SegmentQualificationProfile::open(root.path()).expect("segment profile");
        profile.journal().create_once("events/a", b"a").unwrap();
        let expected_generation = read_head_for_test(root.path()).next_generation;

        assert!(
            profile
                .seal_active_with_failure(SegmentFailurePointV1::AfterSealedGenerationSync)
                .is_err()
        );
        assert_eq!(
            profile.seal_active().expect("retry seal"),
            expected_generation
        );
        assert_eq!(profile.journal().list().unwrap().len(), 1);
    }

    #[test]
    fn a_pin_that_vanishes_during_the_scan_is_not_an_error() {
        let root = tempfile::tempdir().expect("profile root");
        let vanished = root.path().join("vanished.pin");

        assert_eq!(read_pin_generation(&vanished).unwrap(), None);
    }

    #[test]
    fn completed_backup_is_verified_before_segment_files_are_opened() {
        let root = tempfile::tempdir().expect("profile root");
        let backup_parent = tempfile::tempdir().expect("backup parent");
        let profile = SegmentQualificationProfile::open(root.path()).expect("segment profile");
        profile.journal().create_once("events/a", b"a").unwrap();
        profile
            .put_content_once(
                "artifacts/a",
                QualificationRecordKindV1::ObjectArtifact,
                b"object",
            )
            .unwrap();
        profile.seal_active().unwrap();

        let completed = backup_parent.path().join("completed");
        profile.backup_to(&completed).expect("completed backup");
        profile
            .verify_restore(&completed)
            .expect("verified restore");

        let inventory = profile.segment_inventory_evidence().unwrap();
        let target = completed.join(
            inventory
                .current_generation_file
                .expect("sealed generation carrier"),
        );
        let mut file = OpenOptions::new().append(true).open(&target).unwrap();
        file.write_all(b"tamper").unwrap();
        file.sync_all().unwrap();
        assert!(profile.verify_restore(&completed).is_err());

        let unmarked = backup_parent.path().join("unmarked");
        profile
            .backup_to(&unmarked)
            .expect("second completed backup");
        fs::remove_file(unmarked.join(BACKUP_COMPLETION_FILE_V1)).unwrap();
        assert!(profile.verify_restore(&unmarked).is_err());
    }

    #[test]
    fn sealed_generation_splits_without_exceeding_the_configured_bound() {
        let root = tempfile::tempdir().expect("profile root");
        let segment_bytes = SEGMENT_SIZE_CANDIDATES_V1[0];
        let profile =
            SegmentQualificationProfile::open_with_segment_bytes(root.path(), segment_bytes)
                .expect("segment profile");
        for index in 0..4_u64 {
            let mut state = index + 1;
            let bytes = (0..96_000)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    state as u8
                })
                .collect::<Vec<_>>();
            profile
                .journal()
                .create_once(&format!("events/{index}"), &bytes)
                .unwrap();
        }
        let generation = profile.seal_active().unwrap();
        let generation_root = generation_directory_path(root.path(), generation);
        let carriers = sealed_segment_paths(&generation_root).unwrap();
        assert!(carriers.len() >= 2);
        assert!(
            carriers
                .iter()
                .all(|carrier| fs::metadata(carrier).unwrap().len() <= segment_bytes)
        );
        assert_eq!(profile.journal().list().unwrap().len(), 4);
    }

    #[test]
    fn inventory_counts_tail_slack_indexes_heads_and_retained_generations() {
        let root = tempfile::tempdir().expect("profile root");
        let profile = SegmentQualificationProfile::open(root.path()).expect("segment profile");
        profile.journal().create_once("events/a", b"a").unwrap();
        let first = profile.seal_active().unwrap();
        let _pin = profile.pin_reader().unwrap();
        profile.journal().create_once("events/b", b"b").unwrap();
        profile.seal_active().unwrap();

        let physical = profile.segment_inventory_evidence().unwrap();
        let common = profile.inventory().unwrap();
        assert!(physical.active_slack_bytes > 0);
        assert!(physical.retained_generations >= 2);
        assert!(physical.retired_generation_bytes > 0);
        assert!(
            common
                .carriers
                .iter()
                .any(|path| path == SEGMENT_HEAD_FILE_V1)
        );
        assert!(
            common
                .carriers
                .iter()
                .any(|path| path == SEGMENT_INDEX_FILE_V1)
        );
        assert!(
            common
                .carriers
                .iter()
                .any(|path| path == &generation_relative_path(first))
        );
        assert!(common.high_water_bytes >= common.allocated_bytes);
    }

    #[test]
    fn real_process_locking_has_one_creator_idempotent_retries_and_loud_conflicts() {
        let root = tempfile::tempdir().expect("profile root");
        SegmentQualificationProfile::open(root.path()).expect("segment profile");

        let mut children = Vec::new();
        for _ in 0..4 {
            children.push(spawn_child(root.path(), "events/shared", "same"));
        }
        let outputs = children
            .into_iter()
            .map(|mut child| child.wait().expect("child status"))
            .collect::<Vec<_>>();
        assert!(outputs.iter().all(|status| status.success()));

        let conflict = spawn_child(root.path(), "events/shared", "different")
            .wait()
            .expect("conflict status");
        assert!(!conflict.success());
        let profile = SegmentQualificationProfile::open(root.path()).expect("reopen profile");
        assert_eq!(profile.journal().list().unwrap().len(), 1);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_reader_pin_blocks_generation_retirement_until_release() {
        let root = tempfile::tempdir().expect("profile root");
        let profile = SegmentQualificationProfile::open(root.path()).expect("segment profile");
        profile.journal().create_once("events/a", b"a").unwrap();
        let first = profile.seal_active().unwrap();
        let pin = profile.pin_reader().unwrap();
        profile.journal().create_once("events/b", b"b").unwrap();
        profile.seal_active().unwrap();

        assert!(profile.retire_generation(first).is_err());
        assert!(pin._handle.metadata().is_ok());
        drop(pin);
        profile.retire_generation(first).unwrap();
    }

    #[test]
    fn crash_fixture_names_every_required_protocol_boundary() {
        let fixture = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/store-foundation/segments/crash-cases.json"
        ));
        let value: serde_json::Value = serde_json::from_str(fixture).expect("fixture JSON");
        assert_eq!(value["schema"], "pointbreak.segment-crash-cases.v1");
        let cases = value["cases"].as_array().expect("case list");
        for required in [
            "record_bytes",
            "commit_footer",
            "tail_sync",
            "head_publication",
            "index_publication",
            "sealed_generation_sync",
            "generation_publication",
        ] {
            assert!(cases.iter().any(|case| case["boundary"] == required));
        }
    }

    fn spawn_child(root: &std::path::Path, key: &str, value: &str) -> std::process::Child {
        Command::new(std::env::current_exe().expect("current test executable"))
            .arg("--exact")
            .arg(CHILD_TEST)
            .arg("--nocapture")
            .env(CHILD_ENV, "create")
            .env("POINTBREAK_SEGMENT_TEST_ROOT", root)
            .env("POINTBREAK_SEGMENT_TEST_KEY", key)
            .env("POINTBREAK_SEGMENT_TEST_VALUE", value)
            .spawn()
            .expect("spawn segment child")
    }

    #[test]
    fn segment_subprocess_entrypoint() {
        if std::env::var_os(CHILD_ENV).is_none() {
            return;
        }
        let root = std::path::PathBuf::from(
            std::env::var_os("POINTBREAK_SEGMENT_TEST_ROOT").expect("child root"),
        );
        let key = std::env::var("POINTBREAK_SEGMENT_TEST_KEY").expect("child key");
        let value = std::env::var("POINTBREAK_SEGMENT_TEST_VALUE").expect("child value");
        let profile = SegmentQualificationProfile::open(&root).expect("child profile");
        profile
            .journal()
            .create_once(&key, value.as_bytes())
            .expect("child create");
    }
}
