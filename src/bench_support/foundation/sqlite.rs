use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::backup::Backup;
use rusqlite::limits::Limit;
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
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
    QualificationRecordKindV1, publish_completed_backup, qualification_filesystem_name,
    verify_completed_backup,
};
use crate::canonical_hash::sha256_bytes_hex;

pub const MINIMUM_SAFE_SQLITE_VERSION_NUMBER: i32 = 3_051_003;
pub const SQLITE_QUALIFICATION_PROFILE_ID_V1: &str = "pointbreak.sqlite-wal-pbrf.v1";

const SQLITE_DATABASE_FILE: &str = "journal.sqlite3";
const SQLITE_PROFILE_FILE: &str = "profile.pbst";
const SQLITE_CONTENT_DIRECTORY: &str = "content";
const SQLITE_APPLICATION_ID: i32 = 0x5042_4b31;
const SQLITE_USER_VERSION: i32 = 1;
const SQLITE_PAGE_SIZE: u64 = 4096;
const SQLITE_WAL_AUTOCHECKPOINT_PAGES: i64 = 1000;
const MAX_LOGICAL_KEY_BYTES: usize = 4096;
const MAX_EVENT_PBRF_BYTES: i32 = 1024 * 1024 + 4096;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SqliteRuntimeEvidenceV1 {
    pub version: String,
    pub version_number: i32,
    pub source_id: String,
    pub minimum_safe_version_number: i32,
    pub journal_mode: String,
    pub synchronous: String,
    pub fullfsync: bool,
    pub page_size: u64,
    pub wal_autocheckpoint_pages: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SqliteFileEvidenceV1 {
    pub relative_path: String,
    pub encoded_bytes: u64,
    pub allocated_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SqliteInventoryEvidenceV1 {
    pub main_database: SqliteFileEvidenceV1,
    pub wal: Option<SqliteFileEvidenceV1>,
    pub shared_memory: Option<SqliteFileEvidenceV1>,
    pub indexes: Vec<String>,
    pub page_size: u64,
    pub page_count: u64,
    pub freelist_pages: u64,
    pub wal_frames: u64,
    pub last_checkpoint_log_frames: u64,
    pub last_checkpointed_frames: u64,
    pub high_water_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SqliteRecoveryStateV1 {
    Healthy,
    InterruptedBackup,
    InterruptedCheckpoint,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SqliteDiagnosticStateV1 {
    Healthy,
    InterruptedBackup,
    InterruptedCheckpoint,
    RowCorruption {
        logical_key: String,
        message: String,
    },
    StructuralCorruption {
        message: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SqliteCheckpointEvidenceV1 {
    pub busy: bool,
    pub log_frames: u64,
    pub checkpointed_frames: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SqliteRepairRejectionV1 {
    pub logical_key: String,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SqliteCopyOutRepairReportV1 {
    pub copied_journal_rows: u64,
    pub copied_content_carriers: u64,
    pub rejected_journal_rows: Vec<SqliteRepairRejectionV1>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SqliteWorkloadEvidenceV1 {
    pub schema: String,
    pub physical_profile_id: String,
    pub manifest_sha256: String,
    pub records: u64,
    pub journal_records: u64,
    pub content_records: u64,
    pub decoded_bytes: u64,
    pub runtime: SqliteRuntimeEvidenceV1,
    pub checkpoint: SqliteCheckpointEvidenceV1,
    pub sqlite: SqliteInventoryEvidenceV1,
    pub inventory: QualificationInventoryV1,
}

pub fn run_sqlite_workload(
    root: &Path,
    manifest: &QualificationCorpusManifestV1,
) -> Result<SqliteWorkloadEvidenceV1, String> {
    manifest.validate().map_err(|error| error.to_string())?;
    let profile = SqliteQualificationProfile::open(root).map_err(|error| error.to_string())?;
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
                "fresh SQLite workload row {} did not return Created",
                record.logical_key
            ));
        }
        decoded_bytes = decoded_bytes
            .checked_add(record.decoded_bytes.len() as u64)
            .ok_or_else(|| "SQLite workload decoded-byte total overflow".to_owned())?;
    }
    profile.journal().integrity_check()?;
    let checkpoint = profile.checkpoint().map_err(|error| error.to_string())?;
    let sqlite = profile
        .sqlite_inventory_evidence()
        .map_err(|error| error.to_string())?;
    let inventory = profile.inventory()?;
    Ok(SqliteWorkloadEvidenceV1 {
        schema: "pointbreak.sqlite-workload-evidence.v1".to_owned(),
        physical_profile_id: SQLITE_QUALIFICATION_PROFILE_ID_V1.to_owned(),
        manifest_sha256: manifest.manifest_sha256.clone(),
        records: manifest.records.len() as u64,
        journal_records,
        content_records,
        decoded_bytes,
        runtime: profile
            .runtime_evidence()
            .map_err(|error| error.to_string())?,
        checkpoint,
        sqlite,
        inventory,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum SqliteQualificationError {
    #[error("SQLite runtime {actual} is older than the minimum safe WAL version {minimum}")]
    UnsafeRuntime { actual: i32, minimum: i32 },
    #[error("SQLite error during {operation}: {message}")]
    Sqlite {
        operation: &'static str,
        message: String,
    },
    #[error("I/O error at {path}: {message}")]
    Io { path: PathBuf, message: String },
    #[error("invalid SQLite qualification profile: {message}")]
    InvalidProfile { message: String },
    #[error("SQLite journal conflict for logical key {logical_key}")]
    Conflict { logical_key: String },
    #[error("SQLite journal corruption for logical key {logical_key}: {message}")]
    Corruption {
        logical_key: String,
        message: String,
    },
    #[error("SQLite qualification operation was attempted on unsupported filesystem {filesystem}")]
    UnsupportedFilesystem { filesystem: String },
}

pub fn validate_sqlite_runtime_version(
    version_number: i32,
) -> Result<(), SqliteQualificationError> {
    if version_number < MINIMUM_SAFE_SQLITE_VERSION_NUMBER {
        return Err(SqliteQualificationError::UnsafeRuntime {
            actual: version_number,
            minimum: MINIMUM_SAFE_SQLITE_VERSION_NUMBER,
        });
    }
    Ok(())
}

pub fn bundled_sqlite_runtime_evidence() -> Result<SqliteRuntimeEvidenceV1, SqliteQualificationError>
{
    let version_number = rusqlite::version_number();
    validate_sqlite_runtime_version(version_number)?;
    let connection =
        Connection::open_in_memory().map_err(|error| sqlite_error("runtime open", error))?;
    let source_id = connection
        .query_row("SELECT sqlite_source_id()", [], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|error| sqlite_error("runtime source id", error))?;
    Ok(SqliteRuntimeEvidenceV1 {
        version: rusqlite::version().to_owned(),
        version_number,
        source_id,
        minimum_safe_version_number: MINIMUM_SAFE_SQLITE_VERSION_NUMBER,
        journal_mode: "wal".to_owned(),
        synchronous: "full".to_owned(),
        fullfsync: cfg!(target_os = "macos"),
        page_size: SQLITE_PAGE_SIZE,
        wal_autocheckpoint_pages: SQLITE_WAL_AUTOCHECKPOINT_PAGES,
    })
}

struct SqliteQualificationJournal {
    root: PathBuf,
    database_path: PathBuf,
    connection: Mutex<Connection>,
    high_water_bytes: AtomicU64,
    last_checkpoint_log_frames: AtomicU64,
    last_checkpointed_frames: AtomicU64,
}

impl std::fmt::Debug for SqliteQualificationJournal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SqliteQualificationJournal")
            .field("root", &self.root)
            .field("database_path", &self.database_path)
            .finish_non_exhaustive()
    }
}

impl SqliteQualificationJournal {
    fn new(root: &Path, connection: Connection) -> Self {
        Self {
            root: root.to_path_buf(),
            database_path: root.join(SQLITE_DATABASE_FILE),
            connection: Mutex::new(connection),
            high_water_bytes: AtomicU64::new(0),
            last_checkpoint_log_frames: AtomicU64::new(0),
            last_checkpointed_frames: AtomicU64::new(0),
        }
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, SqliteQualificationError> {
        self.connection
            .lock()
            .map_err(|_| SqliteQualificationError::InvalidProfile {
                message: "SQLite connection mutex is poisoned".to_owned(),
            })
    }

    fn sample_high_water(&self) -> Result<u64, SqliteQualificationError> {
        let current =
            recognized_sqlite_files(&self.root)?
                .iter()
                .try_fold(0_u64, |total, file| {
                    total.checked_add(file.allocated_bytes).ok_or_else(|| {
                        SqliteQualificationError::InvalidProfile {
                            message: "SQLite inventory allocation overflow".to_owned(),
                        }
                    })
                })?;
        Ok(self
            .high_water_bytes
            .fetch_max(current, Ordering::Relaxed)
            .max(current))
    }

    fn physical_inventory(&self) -> Result<SqliteInventoryEvidenceV1, SqliteQualificationError> {
        let connection = self.connection()?;
        let page_size = pragma_u64(&connection, "page_size")?;
        let page_count = pragma_u64(&connection, "page_count")?;
        let freelist_pages = pragma_u64(&connection, "freelist_count")?;
        let mut statement = connection
            .prepare("PRAGMA index_list('journal_event')")
            .map_err(|error| sqlite_error("index inventory", error))?;
        let mut indexes = statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|error| sqlite_error("index inventory", error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| sqlite_error("index inventory", error))?;
        indexes.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        drop(statement);
        drop(connection);

        let files = recognized_sqlite_files(&self.root)?;
        let main_database = files
            .iter()
            .find(|file| file.relative_path == SQLITE_DATABASE_FILE)
            .cloned()
            .ok_or_else(|| SqliteQualificationError::InvalidProfile {
                message: "SQLite main database is absent".to_owned(),
            })?;
        let wal = files
            .iter()
            .find(|file| file.relative_path == format!("{SQLITE_DATABASE_FILE}-wal"))
            .cloned();
        let shared_memory = files
            .iter()
            .find(|file| file.relative_path == format!("{SQLITE_DATABASE_FILE}-shm"))
            .cloned();
        let wal_frames = wal
            .as_ref()
            .and_then(|file| {
                file.encoded_bytes
                    .checked_sub(32)
                    .map(|bytes| bytes / (page_size + 24))
            })
            .unwrap_or(0);
        Ok(SqliteInventoryEvidenceV1 {
            main_database,
            wal,
            shared_memory,
            indexes,
            page_size,
            page_count,
            freelist_pages,
            wal_frames,
            last_checkpoint_log_frames: self.last_checkpoint_log_frames.load(Ordering::Relaxed),
            last_checkpointed_frames: self.last_checkpointed_frames.load(Ordering::Relaxed),
            high_water_bytes: self.sample_high_water()?,
        })
    }

    fn run_passive_checkpoint(
        &self,
    ) -> Result<SqliteCheckpointEvidenceV1, SqliteQualificationError> {
        let connection = self.connection()?;
        let (busy, log_frames, checkpointed_frames) = connection
            .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|error| sqlite_error("run passive WAL checkpoint", error))?;
        let log_frames =
            u64::try_from(log_frames).map_err(|_| SqliteQualificationError::InvalidProfile {
                message: "SQLite checkpoint returned a negative WAL frame count".to_owned(),
            })?;
        let checkpointed_frames = u64::try_from(checkpointed_frames).map_err(|_| {
            SqliteQualificationError::InvalidProfile {
                message: "SQLite checkpoint returned a negative completed frame count".to_owned(),
            }
        })?;
        self.last_checkpoint_log_frames
            .store(log_frames, Ordering::Relaxed);
        self.last_checkpointed_frames
            .store(checkpointed_frames, Ordering::Relaxed);
        drop(connection);
        self.sample_high_water()?;
        Ok(SqliteCheckpointEvidenceV1 {
            busy: busy != 0,
            log_frames,
            checkpointed_frames,
        })
    }
}

impl QualificationJournal for SqliteQualificationJournal {
    fn create_once(
        &self,
        logical_key: &str,
        decoded_bytes: &[u8],
    ) -> Result<QualificationCreateOutcome, String> {
        self.create_once_typed(logical_key, decoded_bytes)
            .map_err(|error| error.to_string())
    }

    fn read(&self, logical_key: &str) -> Result<Option<QualificationEntry>, String> {
        self.read_typed(logical_key)
            .map_err(|error| error.to_string())
    }

    fn list(&self) -> Result<Vec<QualificationEntry>, String> {
        self.list_typed().map_err(|error| error.to_string())
    }

    fn head_marker(&self) -> Result<u64, String> {
        self.head_marker_typed().map_err(|error| error.to_string())
    }

    fn integrity_check(&self) -> Result<(), String> {
        self.integrity_check_typed()
            .map_err(|error| error.to_string())
    }
}

impl SqliteQualificationJournal {
    fn create_once_typed(
        &self,
        logical_key: &str,
        decoded_bytes: &[u8],
    ) -> Result<QualificationCreateOutcome, SqliteQualificationError> {
        self.create_once_typed_profiled(logical_key, decoded_bytes, None)
    }

    fn create_once_typed_profiled(
        &self,
        logical_key: &str,
        decoded_bytes: &[u8],
        mut recorder: Option<&mut QualificationPerformanceStageRecorder>,
    ) -> Result<QualificationCreateOutcome, SqliteQualificationError> {
        let (envelope, key_digest, decoded_sha256) =
            measure_profile_stage(&mut recorder, "validate_encode_digest", || {
                validate_logical_key(logical_key)?;
                let envelope = PhysicalRecordV1::encode(
                    logical_key,
                    QualificationRecordKindV1::LegacyEvent,
                    decoded_bytes,
                    &NeverCancelled,
                )
                .map_err(|error| SqliteQualificationError::Corruption {
                    logical_key: logical_key.to_owned(),
                    message: error.to_string(),
                })?;
                if envelope.len() > MAX_EVENT_PBRF_BYTES as usize {
                    return Err(SqliteQualificationError::InvalidProfile {
                        message: format!("encoded event exceeds {MAX_EVENT_PBRF_BYTES} bytes"),
                    });
                }
                Ok((
                    envelope,
                    logical_key_digest(logical_key),
                    Sha256::digest(decoded_bytes),
                ))
            })?;
        let mut connection =
            measure_profile_stage(&mut recorder, "connection_lock", || self.connection())?;
        let transaction = measure_profile_stage(&mut recorder, "begin_immediate", || {
            connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| sqlite_error("begin immediate", error))
        })?;
        let existing = measure_profile_stage(&mut recorder, "health_and_key_lookup", || {
            require_healthy_maintenance_state(&transaction, "create event")?;
            query_raw_entry(&transaction, &key_digest)
        })?;
        if let Some(existing) = existing {
            let entry = decode_raw_entry(existing)?;
            measure_profile_stage(&mut recorder, "idempotent_commit", || {
                transaction
                    .commit()
                    .map_err(|error| sqlite_error("idempotent commit", error))
            })?;
            return if entry.logical_key == logical_key && entry.decoded_bytes == decoded_bytes {
                Ok(QualificationCreateOutcome::AlreadyExists)
            } else {
                Err(SqliteQualificationError::Conflict {
                    logical_key: logical_key.to_owned(),
                })
            };
        }
        measure_profile_stage(&mut recorder, "row_and_head_write", || {
            let head_count = transaction
                .query_row(
                    "SELECT head_count FROM qualification_meta WHERE singleton = 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| sqlite_error("read head", error))?;
            let append_seq = head_count.checked_add(1).ok_or_else(|| {
                SqliteQualificationError::InvalidProfile {
                    message: "SQLite journal head overflow".to_owned(),
                }
            })?;
            let decoded_len = i64::try_from(decoded_bytes.len()).map_err(|_| {
                SqliteQualificationError::InvalidProfile {
                    message: "decoded event length does not fit SQLite".to_owned(),
                }
            })?;
            transaction
                .execute(
                    "INSERT INTO journal_event
                     (append_seq, logical_key, key_digest, decoded_len, decoded_sha256, pbrf)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        append_seq,
                        logical_key,
                        key_digest.as_slice(),
                        decoded_len,
                        decoded_sha256.as_slice(),
                        envelope,
                    ],
                )
                .map_err(|error| sqlite_error("insert journal event", error))?;
            transaction
                .execute(
                    "UPDATE qualification_meta
                     SET head_count = ?1, last_append_ns = ?2
                     WHERE singleton = 1",
                    params![append_seq, unix_time_nanos()],
                )
                .map_err(|error| sqlite_error("advance journal head", error))?;
            Ok(())
        })?;
        measure_profile_stage(&mut recorder, "durable_full_commit", || {
            transaction
                .commit()
                .map_err(|error| sqlite_error("durable full commit", error))
        })?;
        drop(connection);
        measure_profile_stage(&mut recorder, "inventory_observation", || {
            self.sample_high_water()
        })?;
        Ok(QualificationCreateOutcome::Created)
    }

    fn read_typed(
        &self,
        logical_key: &str,
    ) -> Result<Option<QualificationEntry>, SqliteQualificationError> {
        validate_logical_key(logical_key)?;
        let connection = self.connection()?;
        query_raw_entry(&connection, &logical_key_digest(logical_key))?
            .map(decode_raw_entry)
            .transpose()
            .and_then(|entry| match entry {
                Some(entry) if entry.logical_key != logical_key => {
                    Err(SqliteQualificationError::Corruption {
                        logical_key: logical_key.to_owned(),
                        message: "logical-key digest resolves to a different key".to_owned(),
                    })
                }
                entry => Ok(entry),
            })
    }

    fn list_typed(&self) -> Result<Vec<QualificationEntry>, SqliteQualificationError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT append_seq, logical_key, key_digest, decoded_len, decoded_sha256, pbrf
                 FROM journal_event ORDER BY logical_key COLLATE BINARY",
            )
            .map_err(|error| sqlite_error("prepare journal replay", error))?;
        statement
            .query_map([], raw_entry_from_row)
            .map_err(|error| sqlite_error("query journal replay", error))?
            .map(|row| {
                row.map_err(|error| sqlite_error("read journal replay", error))
                    .and_then(decode_raw_entry)
            })
            .collect()
    }

    fn head_marker_typed(&self) -> Result<u64, SqliteQualificationError> {
        let connection = self.connection()?;
        let head = connection
            .query_row(
                "SELECT head_count FROM qualification_meta WHERE singleton = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| sqlite_error("read journal head", error))?;
        u64::try_from(head).map_err(|_| SqliteQualificationError::InvalidProfile {
            message: "SQLite journal head is negative".to_owned(),
        })
    }

    fn integrity_check_typed(&self) -> Result<(), SqliteQualificationError> {
        let connection = self.connection()?;
        require_healthy_maintenance_state(&connection, "complete integrity check")?;
        validate_integrity_and_rows(&connection)
    }
}

fn measure_profile_stage<T>(
    recorder: &mut Option<&mut QualificationPerformanceStageRecorder>,
    stage: &str,
    operation: impl FnOnce() -> Result<T, SqliteQualificationError>,
) -> Result<T, SqliteQualificationError> {
    match recorder.as_deref_mut() {
        Some(recorder) => recorder.measure(stage, operation),
        None => operation(),
    }
}

#[derive(Debug)]
struct RawJournalEntry {
    append_seq: i64,
    logical_key: String,
    key_digest: Vec<u8>,
    decoded_len: i64,
    decoded_sha256: Vec<u8>,
    pbrf: Vec<u8>,
}

fn raw_entry_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawJournalEntry> {
    Ok(RawJournalEntry {
        append_seq: row.get(0)?,
        logical_key: row.get(1)?,
        key_digest: row.get(2)?,
        decoded_len: row.get(3)?,
        decoded_sha256: row.get(4)?,
        pbrf: row.get(5)?,
    })
}

fn query_raw_entry(
    connection: &Connection,
    key_digest: &[u8; 32],
) -> Result<Option<RawJournalEntry>, SqliteQualificationError> {
    connection
        .query_row(
            "SELECT append_seq, logical_key, key_digest, decoded_len, decoded_sha256, pbrf
             FROM journal_event WHERE key_digest = ?1",
            [key_digest.as_slice()],
            raw_entry_from_row,
        )
        .optional()
        .map_err(|error| sqlite_error("read journal row", error))
}

fn decode_raw_entry(raw: RawJournalEntry) -> Result<QualificationEntry, SqliteQualificationError> {
    let corruption = |message: String| SqliteQualificationError::Corruption {
        logical_key: raw.logical_key.clone(),
        message,
    };
    if raw.append_seq <= 0 {
        return Err(corruption("append sequence is not positive".to_owned()));
    }
    if raw.key_digest.as_slice() != logical_key_digest(&raw.logical_key) {
        return Err(corruption("logical-key digest mismatch".to_owned()));
    }
    let decoded = PhysicalRecordV1::decode(&raw.pbrf, &NeverCancelled)
        .map_err(|error| corruption(error.to_string()))?;
    if decoded.record_kind != PhysicalRecordKindV1::Event {
        return Err(corruption("PBRF row is not an event".to_owned()));
    }
    if decoded.logical_key_digest.as_slice() != raw.key_digest {
        return Err(corruption("PBRF logical-key digest mismatch".to_owned()));
    }
    if i64::try_from(decoded.decoded_bytes.len()).ok() != Some(raw.decoded_len) {
        return Err(corruption("decoded length column mismatch".to_owned()));
    }
    if decoded.decoded_sha256.as_slice() != raw.decoded_sha256 {
        return Err(corruption("decoded SHA-256 column mismatch".to_owned()));
    }
    Ok(QualificationEntry {
        logical_key: raw.logical_key,
        decoded_sha256: sha256_bytes_hex(&decoded.decoded_bytes),
        decoded_bytes: decoded.decoded_bytes,
    })
}

pub struct SqliteQualificationProfile {
    root: PathBuf,
    descriptor: QualificationProfileDescriptorV1,
    header: PhysicalStoreHeaderV1,
    journal: SqliteQualificationJournal,
    content: IndependentContentStoreV1,
}

impl std::fmt::Debug for SqliteQualificationProfile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SqliteQualificationProfile")
            .field("root", &self.root)
            .field("descriptor", &self.descriptor)
            .finish_non_exhaustive()
    }
}

impl SqliteQualificationProfile {
    pub fn open(root: &Path) -> Result<Self, SqliteQualificationError> {
        bundled_sqlite_runtime_evidence()?;
        fs::create_dir_all(root).map_err(|error| io_error(root, error))?;
        let root = root.canonicalize().map_err(|error| io_error(root, error))?;
        reject_unsupported_filesystem(&root)?;
        let profile_path = root.join(SQLITE_PROFILE_FILE);
        let database_path = root.join(SQLITE_DATABASE_FILE);
        let profile_exists = profile_path
            .try_exists()
            .map_err(|error| io_error(&profile_path, error))?;
        let database_exists = database_path
            .try_exists()
            .map_err(|error| io_error(&database_path, error))?;
        if profile_exists != database_exists {
            return Err(SqliteQualificationError::InvalidProfile {
                message: "profile bootstrap and SQLite database must be created together"
                    .to_owned(),
            });
        }

        let descriptor = QualificationProfileDescriptorV1 {
            physical_profile_id: SQLITE_QUALIFICATION_PROFILE_ID_V1.to_owned(),
            logical_capabilities: LogicalCapabilityEpochV1::foundation(),
        };
        let header = if profile_exists {
            read_profile_header(&profile_path)?
        } else {
            let mut store_uuid = [0_u8; 16];
            getrandom::fill(&mut store_uuid).map_err(|error| {
                SqliteQualificationError::InvalidProfile {
                    message: format!("store UUID generation failed: {error}"),
                }
            })?;
            let header = PhysicalStoreHeaderV1::new(1, store_uuid);
            write_profile_header(&profile_path, &header)?;
            header
        };

        let connection = open_configured_connection(&database_path, !database_exists)?;
        if database_exists {
            validate_database_metadata(&connection, &descriptor, &header)?;
        } else {
            initialize_database(&connection, &descriptor, &header)?;
        }
        let content = IndependentContentStoreV1::open(&root.join(SQLITE_CONTENT_DIRECTORY))
            .map_err(|error| SqliteQualificationError::InvalidProfile {
                message: error.to_string(),
            })?;
        let journal = SqliteQualificationJournal::new(&root, connection);
        journal.sample_high_water()?;
        Ok(Self {
            root,
            descriptor,
            header,
            journal,
            content,
        })
    }

    pub fn diagnose_root(root: &Path) -> SqliteDiagnosticStateV1 {
        diagnose_sqlite_root(root).unwrap_or_else(|error| {
            SqliteDiagnosticStateV1::StructuralCorruption {
                message: error.to_string(),
            }
        })
    }

    pub fn runtime_evidence(&self) -> Result<SqliteRuntimeEvidenceV1, SqliteQualificationError> {
        bundled_sqlite_runtime_evidence()
    }

    pub fn sqlite_inventory_evidence(
        &self,
    ) -> Result<SqliteInventoryEvidenceV1, SqliteQualificationError> {
        self.journal.physical_inventory()
    }

    pub fn recovery_state(&self) -> Result<SqliteRecoveryStateV1, SqliteQualificationError> {
        let connection = self.journal.connection()?;
        let state = read_maintenance_state(&connection)?;
        match state.as_str() {
            "healthy" => Ok(SqliteRecoveryStateV1::Healthy),
            "backing_up" => Ok(SqliteRecoveryStateV1::InterruptedBackup),
            "checkpointing" => Ok(SqliteRecoveryStateV1::InterruptedCheckpoint),
            _ => Err(SqliteQualificationError::InvalidProfile {
                message: format!("unknown SQLite maintenance state {state}"),
            }),
        }
    }

    pub fn checkpoint(&self) -> Result<SqliteCheckpointEvidenceV1, SqliteQualificationError> {
        self.checkpoint_with_hook(|| {})
    }

    pub(super) fn exercise_allocation_failure(
        &self,
        logical_key: &str,
    ) -> Result<(), SqliteQualificationError> {
        let original_length_limit = {
            let connection = self.journal.connection()?;
            connection
                .set_limit(Limit::SQLITE_LIMIT_LENGTH, 1024)
                .map_err(|error| sqlite_error("set allocation length limit", error))?
        };
        let mut payload = Vec::with_capacity(16 * 1024);
        for block in 0_u32..512 {
            payload.extend_from_slice(&Sha256::digest(block.to_le_bytes()));
        }
        let attempted = self.journal.create_once_typed(logical_key, &payload);
        {
            let connection = self.journal.connection()?;
            connection
                .set_limit(Limit::SQLITE_LIMIT_LENGTH, original_length_limit)
                .map_err(|error| sqlite_error("restore allocation length limit", error))?;
        }
        match attempted {
            Err(SqliteQualificationError::Sqlite { message, .. })
                if {
                    let message = message.to_ascii_lowercase();
                    message.contains("too big") || message.contains("too large")
                } =>
            {
                if self.journal.read_typed(logical_key)?.is_some() {
                    return Err(SqliteQualificationError::InvalidProfile {
                        message: "allocation fault exposed an uncommitted record".to_owned(),
                    });
                }
                if self.journal.create_once_typed(logical_key, b"retry")?
                    != QualificationCreateOutcome::Created
                {
                    return Err(SqliteQualificationError::InvalidProfile {
                        message: "allocation fault retry did not create the record".to_owned(),
                    });
                }
                Ok(())
            }
            Err(error) => Err(SqliteQualificationError::InvalidProfile {
                message: format!("allocation fault returned an unexpected error: {error}"),
            }),
            Ok(outcome) => Err(SqliteQualificationError::InvalidProfile {
                message: format!("allocation fault unexpectedly returned {outcome:?}"),
            }),
        }
    }

    pub(super) fn checkpoint_with_hook<F>(
        &self,
        after_publication: F,
    ) -> Result<SqliteCheckpointEvidenceV1, SqliteQualificationError>
    where
        F: FnOnce(),
    {
        self.set_maintenance_state("healthy", "checkpointing")?;
        after_publication();
        let checkpoint = self.journal.run_passive_checkpoint()?;
        self.set_maintenance_state("checkpointing", "healthy")?;
        Ok(checkpoint)
    }

    pub fn recover_interrupted_checkpoint(&self) -> Result<(), SqliteQualificationError> {
        if self.recovery_state()? != SqliteRecoveryStateV1::InterruptedCheckpoint {
            return Err(SqliteQualificationError::InvalidProfile {
                message: "profile does not have an interrupted checkpoint to recover".to_owned(),
            });
        }
        self.journal.run_passive_checkpoint()?;
        {
            let connection = self.journal.connection()?;
            validate_integrity_and_rows(&connection)?;
        }
        self.set_maintenance_state("checkpointing", "healthy")
    }

    pub fn copy_out_repair(
        &self,
        destination: &Path,
    ) -> Result<SqliteCopyOutRepairReportV1, SqliteQualificationError> {
        if destination
            .try_exists()
            .map_err(|error| io_error(destination, error))?
        {
            return Err(SqliteQualificationError::InvalidProfile {
                message: format!(
                    "copy-out repair destination already exists: {}",
                    destination.display()
                ),
            });
        }
        let raw_rows = {
            let connection = self.journal.connection()?;
            let mut statement = connection
                .prepare(
                    "SELECT append_seq, logical_key, key_digest, decoded_len, decoded_sha256, pbrf
                     FROM journal_event ORDER BY append_seq",
                )
                .map_err(|error| sqlite_error("prepare repair scan", error))?;
            statement
                .query_map([], raw_entry_from_row)
                .map_err(|error| sqlite_error("query repair rows", error))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| sqlite_error("read repair rows", error))?
        };
        let repaired = SqliteQualificationProfile::open(destination)?;
        let mut copied_journal_rows = 0_u64;
        let mut rejected_journal_rows = Vec::new();
        for raw in raw_rows {
            let logical_key = raw.logical_key.clone();
            match decode_raw_entry(raw) {
                Ok(entry) => {
                    repaired
                        .journal
                        .create_once_typed(&entry.logical_key, &entry.decoded_bytes)?;
                    copied_journal_rows += 1;
                }
                Err(error) => rejected_journal_rows.push(SqliteRepairRejectionV1 {
                    logical_key,
                    reason: error.to_string(),
                }),
            }
        }
        let content_inventory =
            self.content
                .inventory()
                .map_err(|error| SqliteQualificationError::InvalidProfile {
                    message: format!("content repair scan failed: {error}"),
                })?;
        for relative in &content_inventory.carriers {
            copy_file_synced(
                &self.content.root().join(relative),
                &repaired.content.root().join(relative),
            )?;
        }
        repaired.journal.integrity_check_typed()?;
        Ok(SqliteCopyOutRepairReportV1 {
            copied_journal_rows,
            copied_content_carriers: content_inventory.carriers.len() as u64,
            rejected_journal_rows,
        })
    }

    fn with_maintenance_writer<T>(
        &self,
        operation: impl FnOnce() -> Result<T, SqliteQualificationError>,
    ) -> Result<T, SqliteQualificationError> {
        let mut connection = open_configured_connection(&self.journal.database_path, false)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| sqlite_error("begin content maintenance", error))?;
        require_healthy_maintenance_state(&transaction, "mutate content")?;
        let result = operation();
        if result.is_ok() {
            transaction
                .commit()
                .map_err(|error| sqlite_error("commit content maintenance", error))?;
            self.journal.sample_high_water()?;
        }
        result
    }

    fn populate_backup(&self, destination: &Path) -> Result<(), SqliteQualificationError> {
        self.set_maintenance_state("healthy", "backing_up")?;
        let result = self.populate_backup_while_fenced(destination);
        let clear = self.set_maintenance_state("backing_up", "healthy");
        match (result, clear) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) => Err(error),
            (Ok(()), Err(error)) | (Err(_), Err(error)) => Err(error),
        }
    }

    fn populate_backup_while_fenced(
        &self,
        destination: &Path,
    ) -> Result<(), SqliteQualificationError> {
        copy_file_synced(
            &self.root.join(SQLITE_PROFILE_FILE),
            &destination.join(SQLITE_PROFILE_FILE),
        )?;
        let source_connection = open_configured_connection(&self.journal.database_path, false)?;
        let backup_path = destination.join(SQLITE_DATABASE_FILE);
        let mut backup_connection = Connection::open(&backup_path)
            .map_err(|error| sqlite_error("open backup database", error))?;
        let backup = Backup::new(&source_connection, &mut backup_connection)
            .map_err(|error| sqlite_error("start online backup", error))?;
        backup
            .run_to_completion(128, Duration::from_millis(5), None)
            .map_err(|error| sqlite_error("run online backup", error))?;
        drop(backup);
        backup_connection
            .execute_batch(
                "UPDATE qualification_meta SET maintenance_state = 'healthy' WHERE singleton = 1;
                 PRAGMA journal_mode=DELETE;
                 PRAGMA synchronous=FULL;",
            )
            .map_err(|error| sqlite_error("finalize backup database", error))?;
        drop(backup_connection);
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&backup_path)
            .and_then(|file| file.sync_all())
            .map_err(|error| io_error(&backup_path, error))?;

        let content_inventory =
            self.content
                .inventory()
                .map_err(|error| SqliteQualificationError::InvalidProfile {
                    message: error.to_string(),
                })?;
        for relative in content_inventory.carriers {
            let source = self.content.root().join(&relative);
            let target = destination.join(SQLITE_CONTENT_DIRECTORY).join(&relative);
            copy_file_synced(&source, &target)?;
        }
        sync_directory(destination)?;
        Ok(())
    }

    fn set_maintenance_state(
        &self,
        expected: &str,
        next: &str,
    ) -> Result<(), SqliteQualificationError> {
        let mut connection = open_configured_connection(&self.journal.database_path, false)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| sqlite_error("begin maintenance-state transition", error))?;
        let changed = transaction
            .execute(
                "UPDATE qualification_meta SET maintenance_state = ?1
                 WHERE singleton = 1 AND maintenance_state = ?2",
                params![next, expected],
            )
            .map_err(|error| sqlite_error("write maintenance state", error))?;
        if changed != 1 {
            return Err(SqliteQualificationError::InvalidProfile {
                message: format!(
                    "maintenance state is not {expected}; cannot transition to {next}"
                ),
            });
        }
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit maintenance-state transition", error))
    }

    fn verify_sqlite_restore(&self, restored_root: &Path) -> Result<(), SqliteQualificationError> {
        let header = read_profile_header(&restored_root.join(SQLITE_PROFILE_FILE))?;
        if header != self.header {
            return Err(SqliteQualificationError::InvalidProfile {
                message: "restored PBST header differs from source".to_owned(),
            });
        }
        let database_path = restored_root.join(SQLITE_DATABASE_FILE);
        let connection =
            Connection::open_with_flags(&database_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|error| sqlite_error("open restored database read-only", error))?;
        validate_database_metadata(&connection, &self.descriptor, &header)?;
        require_healthy_maintenance_state(&connection, "verify restored profile")?;
        validate_integrity_and_rows(&connection)?;
        drop(connection);
        let content_root = restored_root.join(SQLITE_CONTENT_DIRECTORY);
        if content_root
            .try_exists()
            .map_err(|error| io_error(&content_root, error))?
        {
            IndependentContentStoreV1::open(&content_root)
                .map_err(|error| SqliteQualificationError::InvalidProfile {
                    message: error.to_string(),
                })?
                .list()
                .map_err(|error| SqliteQualificationError::InvalidProfile {
                    message: error.to_string(),
                })?;
        }
        Ok(())
    }
}

impl QualificationProfile for SqliteQualificationProfile {
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
        self.with_maintenance_writer(|| {
            self.content
                .put_once(content_key, record_kind, decoded_bytes)
                .map_err(|error| SqliteQualificationError::InvalidProfile {
                    message: error.to_string(),
                })
        })
        .map_err(|error| error.to_string())
    }

    fn read_content(&self, content_key: &str) -> Result<Option<QualificationEntry>, String> {
        self.content
            .read(content_key)
            .map_err(|error| error.to_string())
    }

    fn remove_content(&self, content_key: &str) -> Result<bool, String> {
        self.with_maintenance_writer(|| {
            self.content.remove(content_key).map_err(|error| {
                SqliteQualificationError::InvalidProfile {
                    message: error.to_string(),
                }
            })
        })
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
        self.verify_sqlite_restore(restored_root)
            .map_err(|error| error.to_string())
    }

    fn inventory(&self) -> Result<QualificationInventoryV1, String> {
        let sqlite = self
            .sqlite_inventory_evidence()
            .map_err(|error| error.to_string())?;
        let content = self
            .content
            .inventory()
            .map_err(|error| error.to_string())?;
        let profile_files = collect_profile_files(&self.root).map_err(|error| error.to_string())?;
        let carriers = profile_files
            .iter()
            .map(|file| file.relative_path.clone())
            .collect();
        let encoded_bytes = profile_files
            .iter()
            .try_fold(0_u64, |total, file| total.checked_add(file.encoded_bytes))
            .ok_or_else(|| "SQLite profile encoded-byte inventory overflow".to_owned())?;
        let allocated_bytes = profile_files
            .iter()
            .try_fold(0_u64, |total, file| total.checked_add(file.allocated_bytes))
            .ok_or_else(|| "SQLite profile allocated-byte inventory overflow".to_owned())?;
        let event_logical = {
            let connection = self
                .journal
                .connection()
                .map_err(|error| error.to_string())?;
            let value = connection
                .query_row(
                    "SELECT COALESCE(SUM(decoded_len), 0) FROM journal_event",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| error.to_string())?;
            u64::try_from(value)
                .map_err(|_| "SQLite journal logical-byte total is negative".to_owned())?
        };
        Ok(QualificationInventoryV1 {
            carriers,
            logical_bytes: event_logical
                .checked_add(content.logical_bytes)
                .ok_or_else(|| "SQLite profile logical-byte inventory overflow".to_owned())?,
            encoded_bytes,
            allocated_bytes,
            high_water_bytes: sqlite
                .high_water_bytes
                .saturating_add(content.high_water_bytes)
                .max(allocated_bytes)
                .max(encoded_bytes),
        })
    }
}

impl QualificationPerformanceProbe for SqliteQualificationProfile {
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
                        Some(&mut recorder),
                    )
                    .map_err(|_| "SQLite profiled append failed".to_owned())?;
                if outcome != QualificationCreateOutcome::Created {
                    return Err("SQLite profiled append did not create a fresh record".to_owned());
                }
            }
            QualificationPerformanceOperationV1::StrictReplay => {
                let entries = recorder
                    .measure("query_decode_verify", || self.journal.list())
                    .map_err(|_| "SQLite profiled replay failed".to_owned())?;
                std::hint::black_box(entries);
            }
            QualificationPerformanceOperationV1::KeyedRead => {
                let entry = recorder
                    .measure("indexed_lookup_decode_verify", || {
                        self.journal.read(request.logical_key)
                    })
                    .map_err(|_| "SQLite profiled keyed read failed".to_owned())?
                    .ok_or_else(|| "SQLite profiled keyed read omitted a record".to_owned())?;
                if entry.decoded_bytes != request.decoded_bytes {
                    return Err("SQLite profiled keyed read returned different bytes".to_owned());
                }
            }
            QualificationPerformanceOperationV1::OpenRecovery => {
                let reopened = recorder.measure("open_and_metadata_validation", || {
                    SqliteQualificationProfile::open(&self.root)
                        .map_err(|_| "SQLite profiled reopen failed".to_owned())
                })?;
                recorder.measure("integrity_and_row_validation", || {
                    reopened
                        .journal()
                        .integrity_check()
                        .map_err(|_| "SQLite profiled integrity validation failed".to_owned())
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

fn require_healthy_maintenance_state(
    connection: &Connection,
    operation: &'static str,
) -> Result<(), SqliteQualificationError> {
    let state = read_maintenance_state(connection)?;
    if state != "healthy" {
        return Err(SqliteQualificationError::InvalidProfile {
            message: format!("cannot {operation} while maintenance state is {state}"),
        });
    }
    Ok(())
}

fn diagnose_sqlite_root(root: &Path) -> Result<SqliteDiagnosticStateV1, SqliteQualificationError> {
    let header = read_profile_header(&root.join(SQLITE_PROFILE_FILE))?;
    let descriptor = QualificationProfileDescriptorV1 {
        physical_profile_id: SQLITE_QUALIFICATION_PROFILE_ID_V1.to_owned(),
        logical_capabilities: LogicalCapabilityEpochV1::foundation(),
    };
    let connection = Connection::open_with_flags(
        root.join(SQLITE_DATABASE_FILE),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|error| sqlite_error("open SQLite diagnosis", error))?;
    validate_database_metadata(&connection, &descriptor, &header)?;
    match read_maintenance_state(&connection)?.as_str() {
        "backing_up" => return Ok(SqliteDiagnosticStateV1::InterruptedBackup),
        "checkpointing" => return Ok(SqliteDiagnosticStateV1::InterruptedCheckpoint),
        "healthy" => {}
        state => {
            return Err(SqliteQualificationError::InvalidProfile {
                message: format!("unknown SQLite maintenance state {state}"),
            });
        }
    }
    match validate_integrity_and_rows(&connection) {
        Ok(()) => Ok(SqliteDiagnosticStateV1::Healthy),
        Err(SqliteQualificationError::Corruption {
            logical_key,
            message,
        }) => Ok(SqliteDiagnosticStateV1::RowCorruption {
            logical_key,
            message,
        }),
        Err(error) => Ok(SqliteDiagnosticStateV1::StructuralCorruption {
            message: error.to_string(),
        }),
    }
}

fn read_maintenance_state(connection: &Connection) -> Result<String, SqliteQualificationError> {
    connection
        .query_row(
            "SELECT maintenance_state FROM qualification_meta WHERE singleton = 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| sqlite_error("read maintenance state", error))
}

fn validate_integrity_and_rows(connection: &Connection) -> Result<(), SqliteQualificationError> {
    let mut statement = connection
        .prepare("PRAGMA integrity_check")
        .map_err(|error| sqlite_error("prepare integrity check", error))?;
    let messages = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| sqlite_error("run integrity check", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| sqlite_error("read integrity check", error))?;
    if messages.as_slice() != ["ok"] {
        return Err(SqliteQualificationError::InvalidProfile {
            message: format!("SQLite integrity_check failed: {}", messages.join("; ")),
        });
    }
    drop(statement);
    let (head, count, minimum, maximum) = connection
        .query_row(
            "SELECT m.head_count, COUNT(e.append_seq),
                    COALESCE(MIN(e.append_seq), 0), COALESCE(MAX(e.append_seq), 0)
             FROM qualification_meta m LEFT JOIN journal_event e ON TRUE
             WHERE m.singleton = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .map_err(|error| sqlite_error("validate journal head", error))?;
    if head != count || (count > 0 && (minimum != 1 || maximum != count)) {
        return Err(SqliteQualificationError::InvalidProfile {
            message: format!(
                "journal head/count sequence mismatch: head={head}, count={count}, min={minimum}, max={maximum}"
            ),
        });
    }
    validate_all_rows(connection)
}

fn initialize_database(
    connection: &Connection,
    descriptor: &QualificationProfileDescriptorV1,
    header: &PhysicalStoreHeaderV1,
) -> Result<(), SqliteQualificationError> {
    connection
        .execute_batch(
            "CREATE TABLE qualification_meta (
                 singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                 physical_profile_id TEXT NOT NULL,
                 store_uuid BLOB NOT NULL CHECK (length(store_uuid) = 16),
                 capability_epoch TEXT NOT NULL,
                 required_capabilities_json TEXT NOT NULL,
                 format_version INTEGER NOT NULL CHECK (format_version = 1),
                 head_count INTEGER NOT NULL CHECK (head_count >= 0),
                 last_append_ns INTEGER,
                 maintenance_state TEXT NOT NULL CHECK (maintenance_state IN ('healthy', 'backing_up', 'checkpointing'))
             ) STRICT;
             CREATE TABLE journal_event (
                 append_seq INTEGER PRIMARY KEY CHECK (append_seq > 0),
                 logical_key TEXT NOT NULL CHECK (length(logical_key) BETWEEN 1 AND 4096),
                 key_digest BLOB NOT NULL CHECK (length(key_digest) = 32),
                 decoded_len INTEGER NOT NULL CHECK (decoded_len BETWEEN 0 AND 1048576),
                 decoded_sha256 BLOB NOT NULL CHECK (length(decoded_sha256) = 32),
                 pbrf BLOB NOT NULL CHECK (length(pbrf) BETWEEN 192 AND 1052672)
             ) STRICT;
             CREATE UNIQUE INDEX journal_event_logical_key_uq
                 ON journal_event(logical_key COLLATE BINARY);
             CREATE UNIQUE INDEX journal_event_key_digest_uq
                 ON journal_event(key_digest);",
        )
        .map_err(|error| sqlite_error("create SQLite qualification schema", error))?;
    let required =
        serde_json::to_string(&descriptor.logical_capabilities.required).map_err(|error| {
            SqliteQualificationError::InvalidProfile {
                message: format!("capability metadata serialization failed: {error}"),
            }
        })?;
    connection
        .execute(
            "INSERT INTO qualification_meta
             (singleton, physical_profile_id, store_uuid, capability_epoch,
              required_capabilities_json, format_version, head_count, maintenance_state)
             VALUES (1, ?1, ?2, ?3, ?4, 1, 0, 'healthy')",
            params![
                descriptor.physical_profile_id,
                header.store_uuid.as_slice(),
                descriptor.logical_capabilities.epoch,
                required,
            ],
        )
        .map_err(|error| sqlite_error("write SQLite qualification metadata", error))?;
    Ok(())
}

fn validate_database_metadata(
    connection: &Connection,
    descriptor: &QualificationProfileDescriptorV1,
    header: &PhysicalStoreHeaderV1,
) -> Result<(), SqliteQualificationError> {
    let application_id = pragma_i64(connection, "application_id")?;
    let user_version = pragma_i64(connection, "user_version")?;
    if application_id != i64::from(SQLITE_APPLICATION_ID)
        || user_version != i64::from(SQLITE_USER_VERSION)
    {
        return Err(SqliteQualificationError::InvalidProfile {
            message: format!(
                "SQLite application/schema version mismatch: application_id={application_id}, user_version={user_version}"
            ),
        });
    }
    let (profile_id, store_uuid, capability_epoch, required_json, format_version) = connection
        .query_row(
            "SELECT physical_profile_id, store_uuid, capability_epoch,
                    required_capabilities_json, format_version
             FROM qualification_meta WHERE singleton = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .map_err(|error| sqlite_error("read SQLite qualification metadata", error))?;
    let required: Vec<String> = serde_json::from_str(&required_json).map_err(|error| {
        SqliteQualificationError::InvalidProfile {
            message: format!("capability metadata is invalid: {error}"),
        }
    })?;
    if profile_id != descriptor.physical_profile_id
        || store_uuid != header.store_uuid
        || capability_epoch != descriptor.logical_capabilities.epoch
        || required != descriptor.logical_capabilities.required
        || format_version != 1
    {
        return Err(SqliteQualificationError::InvalidProfile {
            message: "SQLite metadata differs from the PBST/profile descriptor".to_owned(),
        });
    }
    Ok(())
}

fn validate_all_rows(connection: &Connection) -> Result<(), SqliteQualificationError> {
    let mut statement = connection
        .prepare(
            "SELECT append_seq, logical_key, key_digest, decoded_len, decoded_sha256, pbrf
             FROM journal_event ORDER BY logical_key COLLATE BINARY",
        )
        .map_err(|error| sqlite_error("prepare restored journal validation", error))?;
    for row in statement
        .query_map([], raw_entry_from_row)
        .map_err(|error| sqlite_error("query restored journal", error))?
    {
        decode_raw_entry(row.map_err(|error| sqlite_error("read restored journal", error))?)?;
    }
    Ok(())
}

fn open_configured_connection(
    database_path: &Path,
    initialize: bool,
) -> Result<Connection, SqliteQualificationError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = Connection::open_with_flags(database_path, flags)
        .map_err(|error| sqlite_error("open SQLite qualification database", error))?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| sqlite_error("set SQLite busy timeout", error))?;
    if initialize {
        connection
            .pragma_update(None, "page_size", SQLITE_PAGE_SIZE as i64)
            .map_err(|error| sqlite_error("set SQLite page size", error))?;
        connection
            .pragma_update(None, "application_id", SQLITE_APPLICATION_ID)
            .map_err(|error| sqlite_error("set SQLite application id", error))?;
        connection
            .pragma_update(None, "user_version", SQLITE_USER_VERSION)
            .map_err(|error| sqlite_error("set SQLite user version", error))?;
    }
    let journal_mode = connection
        .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))
        .map_err(|error| sqlite_error("enable SQLite WAL", error))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(SqliteQualificationError::InvalidProfile {
            message: format!("SQLite refused WAL mode and returned {journal_mode}"),
        });
    }
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(|error| sqlite_error("set SQLite FULL synchronous mode", error))?;
    connection
        .pragma_update(None, "cell_size_check", true)
        .map_err(|error| sqlite_error("enable SQLite cell-size checks", error))?;
    connection
        .pragma_update(None, "wal_autocheckpoint", SQLITE_WAL_AUTOCHECKPOINT_PAGES)
        .map_err(|error| sqlite_error("set SQLite WAL autocheckpoint", error))?;
    connection
        .pragma_update(None, "locking_mode", "NORMAL")
        .map_err(|error| sqlite_error("set SQLite normal locking", error))?;
    #[cfg(target_os = "macos")]
    connection
        .pragma_update(None, "fullfsync", true)
        .map_err(|error| sqlite_error("enable SQLite fullfsync", error))?;
    connection
        .set_limit(Limit::SQLITE_LIMIT_LENGTH, MAX_EVENT_PBRF_BYTES)
        .map_err(|error| sqlite_error("set SQLite row-size limit", error))?;
    Ok(connection)
}

fn validate_logical_key(logical_key: &str) -> Result<(), SqliteQualificationError> {
    if logical_key.is_empty() || logical_key.len() > MAX_LOGICAL_KEY_BYTES {
        return Err(SqliteQualificationError::InvalidProfile {
            message: format!(
                "logical key length must be between 1 and {MAX_LOGICAL_KEY_BYTES} bytes"
            ),
        });
    }
    Ok(())
}

fn read_profile_header(path: &Path) -> Result<PhysicalStoreHeaderV1, SqliteQualificationError> {
    let bytes = fs::read(path).map_err(|error| io_error(path, error))?;
    let header = PhysicalStoreHeaderV1::decode(&bytes).map_err(|error| {
        SqliteQualificationError::InvalidProfile {
            message: error.to_string(),
        }
    })?;
    if header.profile_id != 1 {
        return Err(SqliteQualificationError::InvalidProfile {
            message: format!("PBST profile {} is not SQLite", header.profile_id),
        });
    }
    Ok(header)
}

fn write_profile_header(
    path: &Path,
    header: &PhysicalStoreHeaderV1,
) -> Result<(), SqliteQualificationError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| io_error(path, error))?;
    file.write_all(&header.encode())
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error(path, error))?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

fn reject_unsupported_filesystem(root: &Path) -> Result<(), SqliteQualificationError> {
    let filesystem = qualification_filesystem_name(root);
    let normalized = filesystem.to_ascii_lowercase();
    if ["nfs", "smb", "cifs", "afpfs", "fuse.rclone", "fuse.sshfs"]
        .iter()
        .any(|unsupported| normalized.contains(unsupported))
    {
        return Err(SqliteQualificationError::UnsupportedFilesystem { filesystem });
    }
    Ok(())
}

fn recognized_sqlite_files(
    root: &Path,
) -> Result<Vec<SqliteFileEvidenceV1>, SqliteQualificationError> {
    [
        SQLITE_DATABASE_FILE.to_owned(),
        format!("{SQLITE_DATABASE_FILE}-wal"),
        format!("{SQLITE_DATABASE_FILE}-shm"),
    ]
    .into_iter()
    .filter_map(|name| {
        let path = root.join(&name);
        match path.try_exists() {
            Ok(true) => Some(file_evidence(root, &path)),
            Ok(false) => None,
            Err(error) => Some(Err(io_error(&path, error))),
        }
    })
    .collect()
}

fn collect_profile_files(
    root: &Path,
) -> Result<Vec<SqliteFileEvidenceV1>, SqliteQualificationError> {
    let mut directories = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = directories.pop() {
        let entries = fs::read_dir(&directory).map_err(|error| io_error(&directory, error))?;
        for entry in entries {
            let entry = entry.map_err(|error| io_error(&directory, error))?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|error| io_error(&path, error))?;
            if file_type.is_dir() {
                directories.push(path);
            } else if file_type.is_file() {
                files.push(file_evidence(root, &path)?);
            } else {
                return Err(SqliteQualificationError::InvalidProfile {
                    message: format!(
                        "unexpected non-file carrier in SQLite profile: {}",
                        path.display()
                    ),
                });
            }
        }
    }
    files.sort_by(|left, right| {
        left.relative_path
            .as_bytes()
            .cmp(right.relative_path.as_bytes())
    });
    Ok(files)
}

fn file_evidence(
    root: &Path,
    path: &Path,
) -> Result<SqliteFileEvidenceV1, SqliteQualificationError> {
    let metadata = fs::metadata(path).map_err(|error| io_error(path, error))?;
    let relative_path = path
        .strip_prefix(root)
        .map_err(|error| SqliteQualificationError::InvalidProfile {
            message: format!("inventory path is outside the profile: {error}"),
        })?
        .to_string_lossy()
        .replace('\\', "/");
    Ok(SqliteFileEvidenceV1 {
        relative_path,
        encoded_bytes: metadata.len(),
        allocated_bytes: allocated_file_bytes(&metadata),
    })
}

fn copy_file_synced(source: &Path, target: &Path) -> Result<(), SqliteQualificationError> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
    }
    let mut input = File::open(source).map_err(|error| io_error(source, error))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|error| io_error(target, error))?;
    std::io::copy(&mut input, &mut output).map_err(|error| io_error(target, error))?;
    output.sync_all().map_err(|error| io_error(target, error))?;
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), SqliteQualificationError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| io_error(path, error))?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), SqliteQualificationError> {
    Ok(())
}

#[cfg(unix)]
fn allocated_file_bytes(metadata: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.blocks().saturating_mul(512)
}

#[cfg(not(unix))]
fn allocated_file_bytes(metadata: &fs::Metadata) -> u64 {
    metadata.len()
}

fn pragma_i64(connection: &Connection, name: &str) -> Result<i64, SqliteQualificationError> {
    connection
        .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
        .map_err(|error| sqlite_error("read SQLite pragma", error))
}

fn pragma_u64(connection: &Connection, name: &str) -> Result<u64, SqliteQualificationError> {
    let value = pragma_i64(connection, name)?;
    u64::try_from(value).map_err(|_| SqliteQualificationError::InvalidProfile {
        message: format!("SQLite pragma {name} returned negative value {value}"),
    })
}

fn unix_time_nanos() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_nanos()).ok())
        .unwrap_or(i64::MAX)
}

fn sqlite_error(operation: &'static str, error: rusqlite::Error) -> SqliteQualificationError {
    SqliteQualificationError::Sqlite {
        operation,
        message: error.to_string(),
    }
}

fn io_error(path: &Path, error: std::io::Error) -> SqliteQualificationError {
    SqliteQualificationError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Seek, SeekFrom, Write};
    use std::process::{Child, Command};
    use std::time::Duration;
    use std::{fs, thread};

    use super::{
        MINIMUM_SAFE_SQLITE_VERSION_NUMBER, SQLITE_DATABASE_FILE, SqliteDiagnosticStateV1,
        SqliteQualificationProfile, SqliteRecoveryStateV1, bundled_sqlite_runtime_evidence,
        validate_sqlite_runtime_version,
    };
    use crate::bench_support::foundation::{
        QualificationProfile, modeled_post_foundation_manifest, run_profile_contract_vectors,
        run_sqlite_workload, synthetic_legacy_manifest,
    };

    const CHILD_TEST: &str =
        "bench_support::foundation::sqlite::tests::sqlite_subprocess_entrypoint";
    const CHILD_MODE: &str = "POINTBREAK_SQLITE_TEST_CHILD_MODE";
    const CHILD_ROOT: &str = "POINTBREAK_SQLITE_TEST_ROOT";
    const CHILD_RESULT: &str = "POINTBREAK_SQLITE_TEST_RESULT";
    const CHILD_KEY: &str = "POINTBREAK_SQLITE_TEST_KEY";
    const CHILD_BYTES: &str = "POINTBREAK_SQLITE_TEST_BYTES";
    const CHILD_BACKUP: &str = "POINTBREAK_SQLITE_TEST_BACKUP";

    #[test]
    fn runtime_guard_rejects_unsafe_wal_versions_and_accepts_the_bundled_build() {
        assert!(validate_sqlite_runtime_version(3_051_002).is_err());
        assert!(validate_sqlite_runtime_version(MINIMUM_SAFE_SQLITE_VERSION_NUMBER).is_ok());

        let evidence = bundled_sqlite_runtime_evidence().expect("bundled runtime evidence");
        assert!(evidence.version_number >= MINIMUM_SAFE_SQLITE_VERSION_NUMBER);
        assert!(!evidence.version.is_empty());
        assert!(!evidence.source_id.is_empty());
    }

    #[test]
    fn sqlite_profile_passes_the_shared_composed_contract() {
        let root = tempfile::tempdir().expect("profile root");
        let backup_parent = tempfile::tempdir().expect("backup parent");
        let backup = backup_parent.path().join("completed");
        let profile = SqliteQualificationProfile::open(root.path()).expect("SQLite profile");

        let report = run_profile_contract_vectors(&profile, &backup).expect("shared contract");

        assert_eq!(report.scenario, "shared-profile-contract-v1");
        assert!(report.inventory.expect("inventory").carriers.len() >= 4);
    }

    #[test]
    fn both_synthetic_workloads_pass_the_sqlite_candidate_driver() {
        let roots = tempfile::tempdir().expect("workload roots");
        let legacy = synthetic_legacy_manifest().expect("legacy manifest");
        let modeled = modeled_post_foundation_manifest().expect("modeled manifest");

        let legacy_evidence = run_sqlite_workload(&roots.path().join("legacy"), &legacy)
            .expect("legacy SQLite workload");
        let modeled_evidence = run_sqlite_workload(&roots.path().join("modeled"), &modeled)
            .expect("modeled SQLite workload");

        assert_eq!(legacy_evidence.records, legacy.records.len() as u64);
        assert_eq!(modeled_evidence.records, modeled.records.len() as u64);
        assert!(legacy_evidence.journal_records > 0);
        assert!(modeled_evidence.content_records > 0);
        assert!(
            legacy_evidence.inventory.high_water_bytes >= legacy_evidence.inventory.allocated_bytes
        );
        assert!(modeled_evidence.sqlite.wal.is_some());
    }

    #[test]
    fn sqlite_inventory_accounts_for_database_sidecars_indexes_and_page_state() {
        let root = tempfile::tempdir().expect("profile root");
        let profile = SqliteQualificationProfile::open(root.path()).expect("SQLite profile");
        profile
            .journal()
            .create_once("events/inventory", b"inventory")
            .expect("event");
        fs::write(root.path().join("extra-carrier"), b"accounted")
            .expect("additional profile carrier");

        {
            let connection = profile.journal.connection().expect("SQLite connection");
            let journal_mode = connection
                .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
                .expect("journal mode");
            let synchronous = connection
                .pragma_query_value(None, "synchronous", |row| row.get::<_, i64>(0))
                .expect("synchronous policy");
            assert_eq!(journal_mode, "wal");
            assert_eq!(synchronous, 2);
            #[cfg(target_os = "macos")]
            assert_eq!(
                connection
                    .pragma_query_value(None, "fullfsync", |row| row.get::<_, i64>(0))
                    .expect("fullfsync policy"),
                1
            );
        }

        let physical = profile
            .sqlite_inventory_evidence()
            .expect("SQLite inventory evidence");
        let common = profile.inventory().expect("common inventory");

        assert!(physical.main_database.encoded_bytes > 0);
        assert!(physical.wal.is_some());
        assert!(physical.shared_memory.is_some());
        assert!(physical.indexes.len() >= 2);
        assert_eq!(physical.page_size, 4096);
        assert!(physical.page_count > 0);
        let current_sqlite_allocation = physical.main_database.allocated_bytes
            + physical.wal.as_ref().map_or(0, |file| file.allocated_bytes)
            + physical
                .shared_memory
                .as_ref()
                .map_or(0, |file| file.allocated_bytes);
        assert!(physical.high_water_bytes >= current_sqlite_allocation);
        assert!(common.high_water_bytes >= common.allocated_bytes);
        assert!(common.carriers.iter().any(|path| path == "extra-carrier"));
        assert!(common.carriers.iter().any(|path| path == "journal.sqlite3"));
        assert!(
            common
                .carriers
                .iter()
                .any(|path| path == "journal.sqlite3-wal")
        );
        assert!(
            common
                .carriers
                .iter()
                .any(|path| path == "journal.sqlite3-shm")
        );
    }

    #[test]
    fn completed_backup_is_verified_before_sqlite_is_opened() {
        let root = tempfile::tempdir().expect("profile root");
        let backup_parent = tempfile::tempdir().expect("backup parent");
        let profile = SqliteQualificationProfile::open(root.path()).expect("SQLite profile");
        profile
            .journal()
            .create_once("events/backup", b"backup")
            .expect("event");
        let completed = backup_parent.path().join("completed");
        profile.backup_to(&completed).expect("backup");
        profile
            .verify_restore(&completed)
            .expect("verified restore");
        let descriptor = profile.descriptor().expect("descriptor");
        let manifest =
            crate::bench_support::foundation::verify_completed_backup(&completed, &descriptor)
                .expect("completed manifest");
        let restored_root = backup_parent.path().join("restored");
        fs::create_dir(&restored_root).expect("restored root");
        for carrier in manifest.carriers {
            let source = completed.join(&carrier.relative_path);
            let target = restored_root.join(&carrier.relative_path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).expect("restore carrier parent");
            }
            fs::copy(source, target).expect("restore carrier");
        }
        let restored = SqliteQualificationProfile::open(&restored_root).expect("restored profile");
        assert_eq!(
            restored.recovery_state().expect("restored state"),
            SqliteRecoveryStateV1::Healthy
        );
        assert_eq!(
            restored
                .journal()
                .read("events/backup")
                .expect("restored read")
                .expect("restored row")
                .decoded_bytes,
            b"backup"
        );

        let unmarked = backup_parent.path().join("unmarked");
        fs::create_dir(&unmarked).expect("unmarked root");
        fs::copy(
            completed.join("journal.sqlite3"),
            unmarked.join("journal.sqlite3"),
        )
        .expect("main-only copy");
        assert!(profile.verify_restore(&unmarked).is_err());

        let unmarked_completed = backup_parent.path().join("unmarked-completed");
        profile
            .backup_to(&unmarked_completed)
            .expect("unmarked source backup");
        fs::remove_file(unmarked_completed.join("pointbreak-backup-complete-v1.json"))
            .expect("remove marker");
        assert!(profile.verify_restore(&unmarked_completed).is_err());

        let incomplete = backup_parent.path().join("incomplete");
        profile
            .backup_to(&incomplete)
            .expect("incomplete source backup");
        fs::remove_file(incomplete.join("profile.pbst")).expect("remove profile bootstrap");
        assert!(profile.verify_restore(&incomplete).is_err());

        let altered = backup_parent.path().join("altered");
        profile.backup_to(&altered).expect("altered source backup");
        fs::OpenOptions::new()
            .append(true)
            .open(altered.join(SQLITE_DATABASE_FILE))
            .expect("altered database")
            .write_all(b"altered")
            .expect("alter backup");
        assert!(profile.verify_restore(&altered).is_err());

        let added = backup_parent.path().join("added");
        profile.backup_to(&added).expect("added source backup");
        fs::write(added.join("unexpected-carrier"), b"unexpected").expect("added carrier");
        assert!(profile.verify_restore(&added).is_err());
    }

    #[test]
    fn begin_immediate_and_unique_keys_are_atomic_across_processes() {
        let root = tempfile::tempdir().expect("profile root");
        drop(SqliteQualificationProfile::open(root.path()).expect("initialize profile"));
        let results = tempfile::tempdir().expect("result root");
        let mut children = (0..4)
            .map(|index| {
                spawn_child(
                    "create",
                    root.path(),
                    &results.path().join(format!("same-{index}")),
                    "events/race",
                    "same-bytes",
                    None,
                )
            })
            .collect::<Vec<_>>();
        for child in &mut children {
            assert!(child.wait().expect("child status").success());
        }
        let outcomes = (0..4)
            .map(|index| {
                fs::read_to_string(results.path().join(format!("same-{index}")))
                    .expect("child outcome")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| outcome.as_str() == "Created")
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| outcome.as_str() == "AlreadyExists")
                .count(),
            3
        );

        let conflict_path = results.path().join("conflict");
        let mut conflict = spawn_child(
            "create",
            root.path(),
            &conflict_path,
            "events/race",
            "different-bytes",
            None,
        );
        assert!(conflict.wait().expect("conflict status").success());
        assert!(
            fs::read_to_string(conflict_path)
                .expect("conflict outcome")
                .contains("conflict")
        );
    }

    #[test]
    fn created_acknowledgement_survives_immediate_process_kill_and_reopen() {
        let root = tempfile::tempdir().expect("profile root");
        drop(SqliteQualificationProfile::open(root.path()).expect("initialize profile"));
        let signals = tempfile::tempdir().expect("signal root");
        let ready = signals.path().join("created");
        let mut child = spawn_child(
            "create-and-wait",
            root.path(),
            &ready,
            "events/durable",
            "durable-bytes",
            None,
        );
        wait_for_file(&ready);
        child.kill().expect("kill child after acknowledgement");
        let _ = child.wait();

        let reopened = SqliteQualificationProfile::open(root.path()).expect("reopen after kill");
        assert_eq!(
            reopened
                .journal()
                .read("events/durable")
                .expect("durable read")
                .expect("durable row")
                .decoded_bytes,
            b"durable-bytes"
        );
        reopened.journal().integrity_check().expect("integrity");
    }

    #[test]
    fn reader_snapshot_stays_stable_during_process_write_checkpoint_and_backup() {
        let root = tempfile::tempdir().expect("profile root");
        let profile = SqliteQualificationProfile::open(root.path()).expect("profile");
        profile
            .journal()
            .create_once("events/before", b"before")
            .expect("seed event");
        let reader = rusqlite::Connection::open(root.path().join(SQLITE_DATABASE_FILE))
            .expect("reader connection");
        reader.execute_batch("BEGIN").expect("reader transaction");
        let before = reader
            .query_row("SELECT COUNT(*) FROM journal_event", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("initial count");

        let process_root = tempfile::tempdir().expect("process result root");
        let result = process_root.path().join("writer");
        let backup = process_root.path().join("backup");
        let mut child = spawn_child(
            "write-checkpoint-backup",
            root.path(),
            &result,
            "events/after",
            "after",
            Some(&backup),
        );
        assert!(child.wait().expect("writer status").success());
        assert_eq!(fs::read_to_string(&result).expect("writer result"), "ok");
        assert_eq!(
            reader
                .query_row("SELECT COUNT(*) FROM journal_event", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("snapshot count"),
            before
        );
        reader.execute_batch("COMMIT").expect("reader commit");
        assert_eq!(
            reader
                .query_row("SELECT COUNT(*) FROM journal_event", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("fresh count"),
            before + 1
        );
        profile.verify_restore(&backup).expect("concurrent backup");
    }

    #[test]
    fn interrupted_checkpoint_is_loud_and_explicitly_recoverable() {
        let root = tempfile::tempdir().expect("profile root");
        let profile = SqliteQualificationProfile::open(root.path()).expect("profile");
        profile
            .journal()
            .create_once("events/checkpoint", &vec![b'x'; 128 * 1024])
            .expect("seed event");
        drop(profile);
        let signals = tempfile::tempdir().expect("signal root");
        let ready = signals.path().join("checkpointing");
        let mut child = spawn_child(
            "checkpoint-and-wait",
            root.path(),
            &ready,
            "unused",
            "unused",
            None,
        );
        wait_for_file(&ready);
        child.kill().expect("kill checkpoint child");
        let _ = child.wait();

        let reopened =
            SqliteQualificationProfile::open(root.path()).expect("reopen interrupted profile");
        assert_eq!(
            reopened.recovery_state().expect("recovery state"),
            SqliteRecoveryStateV1::InterruptedCheckpoint
        );
        assert!(reopened.journal().integrity_check().is_err());
        reopened
            .recover_interrupted_checkpoint()
            .expect("recover checkpoint");
        assert_eq!(
            reopened.recovery_state().expect("recovered state"),
            SqliteRecoveryStateV1::Healthy
        );
        reopened.journal().integrity_check().expect("integrity");
    }

    #[test]
    fn corrupt_rows_are_rejected_and_copy_out_repair_reports_the_gap() {
        let root = tempfile::tempdir().expect("profile root");
        let repair_parent = tempfile::tempdir().expect("repair parent");
        let profile = SqliteQualificationProfile::open(root.path()).expect("profile");
        profile
            .journal()
            .create_once("events/good", b"good")
            .expect("good event");
        profile
            .journal()
            .create_once("events/corrupt", b"corrupt")
            .expect("corrupt event");
        {
            let connection = profile.journal.connection().expect("connection");
            connection
                .execute_batch("PRAGMA ignore_check_constraints=ON")
                .expect("allow corruption fixture");
            connection
                .execute(
                    "UPDATE journal_event SET pbrf = substr(pbrf, 1, 8)
                     WHERE logical_key = 'events/corrupt'",
                    [],
                )
                .expect("truncate row");
        }
        assert!(profile.journal().read("events/corrupt").is_err());
        assert!(profile.journal().integrity_check().is_err());

        let repaired_root = repair_parent.path().join("repaired");
        let report = profile
            .copy_out_repair(&repaired_root)
            .expect("copy-out repair");
        assert_eq!(report.copied_journal_rows, 1);
        assert_eq!(report.rejected_journal_rows.len(), 1);
        assert_eq!(
            report.rejected_journal_rows[0].logical_key,
            "events/corrupt"
        );
        let repaired = SqliteQualificationProfile::open(&repaired_root).expect("repaired profile");
        assert!(
            repaired
                .journal()
                .read("events/good")
                .expect("good read")
                .is_some()
        );
        assert!(
            repaired
                .journal()
                .read("events/corrupt")
                .expect("corrupt omitted")
                .is_none()
        );
    }

    #[test]
    fn decoded_digest_column_mismatch_is_loud() {
        let root = tempfile::tempdir().expect("profile root");
        let profile = SqliteQualificationProfile::open(root.path()).expect("profile");
        profile
            .journal()
            .create_once("events/digest", b"digest")
            .expect("event");
        profile
            .journal
            .connection()
            .expect("connection")
            .execute(
                "UPDATE journal_event SET decoded_sha256 = zeroblob(32)
                 WHERE logical_key = 'events/digest'",
                [],
            )
            .expect("digest mutation");
        let error = profile
            .journal()
            .read("events/digest")
            .expect_err("digest mismatch");
        assert!(error.contains("SHA-256"));
    }

    #[test]
    fn btree_corruption_is_classified_as_structural_and_never_repaired_in_place() {
        let root = tempfile::tempdir().expect("profile root");
        let profile = SqliteQualificationProfile::open(root.path()).expect("profile");
        profile
            .journal()
            .create_once("events/btree", &vec![b'b'; 16 * 1024])
            .expect("event");
        let root_page = profile
            .journal
            .connection()
            .expect("connection")
            .query_row(
                "SELECT rootpage FROM sqlite_schema WHERE name = 'journal_event'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("journal root page");
        let root_page = u64::try_from(root_page).expect("positive journal root page");
        profile.checkpoint().expect("checkpoint");
        drop(profile);
        let checkpoint = rusqlite::Connection::open(root.path().join(SQLITE_DATABASE_FILE))
            .expect("checkpoint connection");
        checkpoint
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .expect("truncate checkpoint");
        drop(checkpoint);

        let database_path = root.path().join(SQLITE_DATABASE_FILE);
        let mut database = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&database_path)
            .expect("database file");
        database
            .seek(SeekFrom::Start((root_page - 1) * 4096))
            .expect("journal root offset");
        database
            .write_all(&[0_u8; 4096])
            .expect("destroy journal B-tree page");
        database.sync_all().expect("corruption sync");

        assert!(matches!(
            SqliteQualificationProfile::diagnose_root(root.path()),
            SqliteDiagnosticStateV1::StructuralCorruption { .. }
        ));
    }

    #[test]
    fn corruption_fixture_names_every_required_state() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/store-foundation/sqlite/corruption-cases.json"
        )))
        .expect("corruption fixture");
        assert_eq!(fixture["cases"].as_array().expect("cases").len(), 4);
    }

    #[test]
    fn sqlite_subprocess_entrypoint() {
        let Ok(mode) = std::env::var(CHILD_MODE) else {
            return;
        };
        let root = std::path::PathBuf::from(std::env::var_os(CHILD_ROOT).expect("child root"));
        let result =
            std::path::PathBuf::from(std::env::var_os(CHILD_RESULT).expect("child result path"));
        let key = std::env::var(CHILD_KEY).expect("child key");
        let bytes = std::env::var(CHILD_BYTES).expect("child bytes");
        let profile = SqliteQualificationProfile::open(&root).expect("child profile");
        match mode.as_str() {
            "create" => {
                let outcome = profile.journal().create_once(&key, bytes.as_bytes());
                write_synced(
                    &result,
                    &match outcome {
                        Ok(outcome) => format!("{outcome:?}"),
                        Err(error) => error,
                    },
                );
            }
            "create-and-wait" => {
                assert_eq!(
                    profile
                        .journal()
                        .create_once(&key, bytes.as_bytes())
                        .expect("child create"),
                    crate::bench_support::foundation::QualificationCreateOutcome::Created
                );
                write_synced(&result, "Created");
                loop {
                    thread::sleep(Duration::from_secs(1));
                }
            }
            "write-checkpoint-backup" => {
                profile
                    .journal()
                    .create_once(&key, bytes.as_bytes())
                    .expect("child write");
                profile.checkpoint().expect("child checkpoint");
                let backup = std::path::PathBuf::from(
                    std::env::var_os(CHILD_BACKUP).expect("child backup path"),
                );
                profile.backup_to(&backup).expect("child backup");
                write_synced(&result, "ok");
            }
            "checkpoint-and-wait" => {
                profile
                    .checkpoint_with_hook(|| {
                        write_synced(&result, "checkpointing");
                        loop {
                            thread::sleep(Duration::from_secs(1));
                        }
                    })
                    .expect("checkpoint hook");
            }
            other => panic!("unknown child mode {other}"),
        }
    }

    fn spawn_child(
        mode: &str,
        root: &std::path::Path,
        result: &std::path::Path,
        key: &str,
        bytes: &str,
        backup: Option<&std::path::Path>,
    ) -> Child {
        let mut command = Command::new(std::env::current_exe().expect("current test executable"));
        command
            .arg("--exact")
            .arg(CHILD_TEST)
            .arg("--nocapture")
            .env(CHILD_MODE, mode)
            .env(CHILD_ROOT, root)
            .env(CHILD_RESULT, result)
            .env(CHILD_KEY, key)
            .env(CHILD_BYTES, bytes);
        if let Some(backup) = backup {
            command.env(CHILD_BACKUP, backup);
        }
        command.spawn().expect("spawn SQLite child")
    }

    fn wait_for_file(path: &std::path::Path) {
        for _ in 0..500 {
            if path.try_exists().expect("signal existence") {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for {}", path.display());
    }

    fn write_synced(path: &std::path::Path, value: &str) {
        let mut file = fs::File::create(path).expect("result file");
        file.write_all(value.as_bytes()).expect("result bytes");
        file.sync_all().expect("result sync");
    }
}
