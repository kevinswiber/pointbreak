use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::error::{Result, SchemaBreakRecord, ShoreError};
use crate::model::id_prefix;
use crate::session::event::{AssertionMode, EventType, ShoreEvent};
use crate::session::store::backend::{Journal, JournalEntry, LocalJournal, StoreBackend};
use crate::storage::{CreateOutcome, LocalStorage};

#[derive(Debug)]
pub struct EventStore {
    journal: Box<dyn Journal>,
    /// The file-layout helpers, present only for a file-backed store. They read
    /// events by arbitrary on-disk path (not by logical key), so they serve the
    /// store-migrate, bundle, and tamper-probe consumers and have no meaning for a
    /// non-file backend — an injected memory store carries `None`.
    files: Option<LocalEventFiles>,
}

/// The file-layout half of a file-backed event store: the events directory and a
/// `LocalStorage` for the path-keyed reads (`read_event`, `list_event_file_names`)
/// that sit beside, not on, the key-addressed [`Journal`].
#[derive(Debug)]
struct LocalEventFiles {
    store_dir: PathBuf,
    storage: LocalStorage,
}

impl LocalEventFiles {
    fn new(store_dir: impl AsRef<Path>) -> Self {
        let store_dir = store_dir.as_ref().to_path_buf();
        Self {
            storage: LocalStorage::new(&store_dir),
            store_dir,
        }
    }
}

impl EventStore {
    pub fn open(store_dir: impl AsRef<Path>) -> Self {
        let store_dir = store_dir.as_ref().to_path_buf();
        Self {
            journal: Box::new(LocalJournal::new(&store_dir)),
            files: Some(LocalEventFiles::new(store_dir)),
        }
    }

    /// Build the event wrapper over the journal a resolved backend yields. This
    /// is the constructor production consumers use, so the backend chosen once at
    /// the resolve choke point flows here. `open` stays the convenience
    /// constructor for tests and direct file-store access.
    pub(crate) fn from_backend(backend: &StoreBackend) -> Self {
        let journal = backend.journal();
        let files = match backend {
            StoreBackend::Local(store_dir) => Some(LocalEventFiles::new(store_dir)),
            // The injection-only memory backend has no directory. Every write and
            // read goes through `journal`; the file-layout helpers are
            // file-backend-only, so a memory store simply carries `None`.
            #[cfg(test)]
            StoreBackend::Memory(_) => None,
        };
        Self { journal, files }
    }

    /// The file-layout helpers, which are valid only on a file-backed store. A
    /// memory-backed store never reaches these (its consumers — store-migrate,
    /// bundle, the tamper probes — are all file-backed by construction).
    fn files(&self) -> &LocalEventFiles {
        self.files
            .as_ref()
            .expect("event file-path helpers require a file-backed store")
    }

    pub(crate) fn events_dir(&self) -> PathBuf {
        self.files().store_dir.join("events")
    }

    pub(crate) fn event_path_for_idempotency_key(&self, idempotency_key: &str) -> PathBuf {
        self.events_dir()
            .join(format!("{}.json", event_filename_stem(idempotency_key)))
    }

    pub fn record_event_once(&self, event: &ShoreEvent) -> Result<EventWriteOutcome> {
        let span = tracing::debug_span!(
            "event_store.record_event_once",
            event_id = event.event_id.as_str(),
            event_type = ?event.event_type,
            idempotency_key = event.idempotency_key.as_str(),
        );
        let _entered = span.enter();

        // The journal owns key→address mapping, so the write needs no on-disk path;
        // the prior path-stem check was a tautology over the key-derived filename.
        validate_event(event, None)?;
        let bytes = serde_json::to_vec(event)?;

        match self
            .journal
            .create_event_once(&event.idempotency_key, &bytes)?
        {
            CreateOutcome::Created => {
                tracing::debug!(
                    idempotency_key = %event.idempotency_key,
                    "event_store_write_created"
                );
                Ok(EventWriteOutcome::Created)
            }
            CreateOutcome::AlreadyExists => {
                let existing = self.read_stored_event(&event.idempotency_key)?;
                if existing.payload_hash == event.payload_hash {
                    if event_signature_binding_matches(&existing, event) {
                        tracing::debug!(
                            idempotency_key = %event.idempotency_key,
                            "event_store_write_existing"
                        );
                        Ok(EventWriteOutcome::Existing)
                    } else if existing.event_record_hash()? == event.event_record_hash()? {
                        // Same content record (signer-exclusive eventRecordHash matches),
                        // differently signed: a transcription-eligible co-signature, not a
                        // conflict.
                        tracing::debug!(
                            idempotency_key = %event.idempotency_key,
                            "event_store_write_existing_divergent_signature"
                        );
                        Ok(EventWriteOutcome::ExistingDivergentSignature)
                    } else {
                        // Same idempotencyKey and payloadHash but a different record
                        // (eventRecordHash differs) — an eventId collision without content
                        // identity, not a co-signature. Keep the first-stored copy.
                        tracing::debug!(
                            idempotency_key = %event.idempotency_key,
                            "event_store_write_existing"
                        );
                        Ok(EventWriteOutcome::Existing)
                    }
                } else {
                    Err(ShoreError::Message(format!(
                        "event conflict for idempotency key {}",
                        event.idempotency_key
                    )))
                }
            }
        }
    }

    pub fn read_event(&self, path: &Path) -> Result<ShoreEvent> {
        let bytes = self.files().storage.read_bytes(path)?;
        Self::decode_validated_event(&bytes, Some(path))
    }

    /// Read the stored event for `idempotency_key` back through the journal, then
    /// decode and validate its bytes. `record_event_once` classifies an
    /// already-present entry this way, so the read-back goes through the same byte
    /// surface as the write rather than reaching into the filesystem directly. The
    /// decoded event must carry the key it was requested under — the keyed-read
    /// equivalent of the file backend's filename/key check — so a blob tampered to
    /// hold a different key's content is rejected, not classified.
    fn read_stored_event(&self, idempotency_key: &str) -> Result<ShoreEvent> {
        let bytes = self
            .journal
            .read_event_bytes(idempotency_key)?
            .ok_or_else(|| {
                ShoreError::Message(format!(
                    "stored event for idempotency key {idempotency_key} disappeared during write"
                ))
            })?;
        let event = Self::decode_validated_event(&bytes, None)?;
        if event.idempotency_key != idempotency_key {
            return Err(filename_idempotency_mismatch(idempotency_key));
        }
        Ok(event)
    }

    /// Decode and validate one listed journal entry. The byte read surface carries
    /// no path, so the filename/key check `read_event` performs becomes a digest
    /// check here: the decoded event's key must hash to the entry's content-address
    /// digest, rejecting a blob that drifted from its content-addressed home.
    fn decode_validated_entry(entry: &JournalEntry) -> Result<ShoreEvent> {
        let event = Self::decode_validated_event(&entry.bytes, None)?;
        if event_filename_stem(&event.idempotency_key) != entry.key_digest {
            return Err(filename_idempotency_mismatch(&event.idempotency_key));
        }
        Ok(event)
    }

    /// Decode stored event bytes, rejecting the two recognized schema breaks (a
    /// retired event type, a retired envelope field) with typed errors before
    /// validating the decoded event. `path`, when given, also checks the file name
    /// matches the idempotency key. The byte read surface passes `None` and pairs
    /// this with a digest/key check at the call site instead, since the bytes carry
    /// no path of their own.
    fn decode_validated_event(bytes: &[u8], path: Option<&Path>) -> Result<ShoreEvent> {
        #[derive(serde::Deserialize)]
        struct EventProbe<'a> {
            #[serde(rename = "eventType", borrow)]
            event_type: &'a str,
            #[serde(default)]
            writer: WriterProbe,
        }
        // Two hard breaks, two probe shapes. `role` (ADR-0007): serde ignores
        // unknown fields, so a stored pre-break envelope would otherwise load
        // silently with degraded meaning; the probe makes the break loud.
        // `tool` (ADR-0010): the rename to `producer` makes the break naturally
        // loud (the required field is missing), but only as an opaque serde
        // error; the probe upgrades it to the typed migration error pointing at
        // the doc anchor where the replacement field is documented. The probe
        // runs before full decode, so the typed error wins.
        #[derive(Default, serde::Deserialize)]
        struct WriterProbe {
            role: Option<serde::de::IgnoredAny>,
            tool: Option<serde::de::IgnoredAny>,
        }
        let probe: EventProbe<'_> = serde_json::from_slice(bytes)?;
        if let Some(record) = schema_break_for(probe.event_type) {
            return Err(ShoreError::UnsupportedEventType(record));
        }
        if probe.writer.role.is_some() {
            return Err(ShoreError::UnsupportedEventEnvelope(
                schema_break_for("writer.role").expect("writer.role has a break record"),
            ));
        }
        if probe.writer.tool.is_some() {
            return Err(ShoreError::UnsupportedEventEnvelope(
                schema_break_for("writer.tool").expect("writer.tool has a break record"),
            ));
        }
        let event: ShoreEvent = serde_json::from_slice(bytes)?;
        validate_event(&event, path)?;
        Ok(event)
    }

    pub fn list_events(&self) -> Result<Vec<ShoreEvent>> {
        self.journal
            .list_event_entries()?
            .iter()
            .map(Self::decode_validated_entry)
            .collect()
    }

    /// Read every event, partitioning the two recognized schema-break classes
    /// (a retired event type, a retired envelope field) into `skipped` instead of
    /// aborting the read. Any other failure — genuine corruption, IO, a hash or
    /// id mismatch — still propagates, so nothing unexpected is papered over.
    pub fn list_events_lenient(&self) -> Result<(Vec<ShoreEvent>, Vec<SkippedEvent>)> {
        let mut events = Vec::new();
        let mut skipped = Vec::new();
        for entry in self.journal.list_event_entries()? {
            match Self::decode_validated_entry(&entry) {
                Ok(event) => events.push(event),
                Err(ShoreError::UnsupportedEventType(record)) => skipped.push(SkippedEvent {
                    code: "unsupported_event_type",
                    record,
                }),
                Err(ShoreError::UnsupportedEventEnvelope(record)) => skipped.push(SkippedEvent {
                    code: "unsupported_event_envelope",
                    record,
                }),
                Err(other) => return Err(other),
            }
        }
        Ok((events, skipped))
    }

    /// Event file names in this store, with the same accept/skip rules as
    /// `list_events` but without parsing event JSON. Sorted; a missing events
    /// directory lists as empty.
    pub(crate) fn list_event_file_names(&self) -> Result<Vec<String>> {
        Ok(self
            .files()
            .storage
            .list_dir(&self.events_dir())?
            .into_iter()
            .filter(|path| is_event_file(path))
            .filter_map(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_owned)
            })
            .collect())
    }

    pub(crate) fn event_exists(&self, idempotency_key: &str) -> Result<bool> {
        self.journal.event_exists(idempotency_key)
    }
}

/// How the store resolved a single event write. The store keeps the first stored
/// event under an idempotency key (first-stored-wins); `ExistingDivergentSignature`
/// means the same content record arrived under a different signature binding — an
/// incoming attestation, when present and resolvable, transcribes into a detached
/// co-signature carrier rather than replacing the stored event, and an unsigned
/// divergent duplicate is a clean keep-first no-op.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventWriteOutcome {
    Created,
    Existing,
    ExistingDivergentSignature,
}

impl EventWriteOutcome {
    /// The canonical outcome code — exactly the serde snake_case wire form.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Existing => "existing",
            Self::ExistingDivergentSignature => "existing_divergent_signature",
        }
    }
}

impl std::fmt::Display for EventWriteOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A stored event that a lenient read recognized as a retired schema and skipped
/// rather than decoded. `code` is the diagnostic class — set by which break the
/// reader caught — and `record` carries the structured break detail.
#[derive(Clone, Debug)]
pub struct SkippedEvent {
    /// "unsupported_event_type" | "unsupported_event_envelope".
    pub code: &'static str,
    pub record: SchemaBreakRecord,
}

fn event_signature_binding_matches(existing: &ShoreEvent, candidate: &ShoreEvent) -> bool {
    existing.signer == candidate.signer && existing.signature == candidate.signature
}

fn validate_event(event: &ShoreEvent, path: Option<&Path>) -> Result<()> {
    event.validate_schema_version()?;

    if event.event_type == EventType::ReviewAssessmentRecorded
        && event.assertion_mode != AssertionMode::Operative
    {
        return Err(ShoreError::InvalidEvent {
            message: "review_assessment_recorded events must use assertionMode Operative"
                .to_owned(),
        });
    }

    let expected_event_id = format!(
        "{}:sha256:{}",
        id_prefix::EVENT,
        sha256_bytes_hex(event.idempotency_key.as_bytes())
    );
    if event.event_id.as_str() != expected_event_id {
        return Err(ShoreError::Message(format!(
            "eventId mismatch for idempotency key {}",
            event.idempotency_key
        )));
    }

    let expected_payload_hash = sha256_json_prefixed(&event.payload)?;
    if event.payload_hash != expected_payload_hash {
        return Err(ShoreError::Message(format!(
            "payloadHash mismatch for event {}",
            event.event_id.as_str()
        )));
    }

    if let Some(path) = path
        && let Some(stem) = path.file_stem().and_then(|stem| stem.to_str())
    {
        let expected_stem = event_filename_stem(&event.idempotency_key);
        if stem != expected_stem {
            return Err(filename_idempotency_mismatch(&event.idempotency_key));
        }
    }

    Ok(())
}

/// The stored event's content-address home disagrees with its idempotency key —
/// a relocated, renamed, or tampered blob. Shared by the path check, the keyed
/// read-back, and the listing decode so all three speak with one voice.
fn filename_idempotency_mismatch(idempotency_key: &str) -> ShoreError {
    ShoreError::Message(format!(
        "event filename does not match idempotencyKey for {idempotency_key}"
    ))
}

pub(in crate::session::store) fn event_filename_stem(idempotency_key: &str) -> String {
    sha256_bytes_hex(idempotency_key.as_bytes())
}

pub(in crate::session::store) fn is_event_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.len() == 69 && name.ends_with(".json"))
}

/// The single source of truth for retired event types and envelope shapes.
/// Given a retired identifier (an event-type wire tag or an envelope field
/// name), returns the structured break record naming where its migration
/// guidance lives. Returns `None` for identifiers that are still supported.
fn schema_break_for(retired: &str) -> Option<SchemaBreakRecord> {
    let (broken_at, anchor) = match retired {
        "review_disposition_recorded" => {
            ("0.1", "docs/assessment-model.md#legacy-disposition-events")
        }
        "intervention_requested" | "intervention_resolved" => (
            "0.1",
            "docs/input-request-model.md#legacy-intervention-events",
        ),
        "writer.role" => ("0.1", "docs/storage-model.md#legacy-writer-role-events"),
        "writer.tool" => ("0.1", "docs/storage-model.md#legacy-writer-tool-events"),
        _ => return None,
    };
    Some(SchemaBreakRecord {
        retired: retired.to_owned(),
        broken_at: broken_at.to_owned(),
        anchor: anchor.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::crypto::SignerId;
    use crate::model::{AssessmentId, JournalId, ReviewTargetRef, RevisionId, TargetRef, TrackId};
    use crate::session::event::{
        AssertionMode, EventSignature, EventTarget, EventType, IngestProvenance, IngestVia,
        ReviewAssessment, ReviewAssessmentRecordedPayload, ReviewInitializedPayload,
        ReviewNoteImportedPayload, ShoreEvent, Writer,
    };
    use crate::session::state::SessionState;

    #[test]
    fn schema_break_for_covers_every_retired_identifier() {
        for (retired, anchor_suffix) in [
            (
                "review_disposition_recorded",
                "assessment-model.md#legacy-disposition-events",
            ),
            (
                "intervention_requested",
                "input-request-model.md#legacy-intervention-events",
            ),
            (
                "intervention_resolved",
                "input-request-model.md#legacy-intervention-events",
            ),
            ("writer.role", "storage-model.md#legacy-writer-role-events"),
            ("writer.tool", "storage-model.md#legacy-writer-tool-events"),
        ] {
            let record =
                schema_break_for(retired).expect("a retired identifier has a break record");
            assert_eq!(record.retired, retired);
            assert!(!record.broken_at.is_empty());
            assert!(
                record.anchor.ends_with(anchor_suffix),
                "anchor was {}",
                record.anchor
            );
        }
        assert!(schema_break_for("review_observation_recorded").is_none());
    }

    #[test]
    fn event_path_is_sha256_of_idempotency_key() {
        let root = tempfile::tempdir().unwrap();
        let store = EventStore::open(root.path().join(".shore/data"));

        let path =
            store.event_path_for_idempotency_key("review_initialized:review:default:work:default");

        assert_eq!(
            path.parent().unwrap(),
            root.path().join(".shore/data/events")
        );
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            "922a9f73c057fa93d31156c391cb0ca441dfa8c1f3cd9cf94a497e8f309675be.json"
        );
    }

    #[test]
    fn recording_same_event_twice_returns_existing_without_rewriting() {
        let (_root, store) = temp_event_store();
        let event = review_initialized_event();

        assert_eq!(
            store.record_event_once(&event).unwrap(),
            EventWriteOutcome::Created
        );
        assert!(store.event_exists(&event.idempotency_key).unwrap());
        assert_eq!(
            store.record_event_once(&event).unwrap(),
            EventWriteOutcome::Existing
        );
        assert_eq!(store.list_events().unwrap(), vec![event]);
    }

    #[test]
    fn replay_with_new_occurred_at_returns_existing_when_payload_matches() {
        let (_root, store) = temp_event_store();
        let first = review_initialized_event_at("2026-05-10T00:00:00Z");
        let retry = review_initialized_event_at("2026-05-10T00:01:00Z");

        store.record_event_once(&first).unwrap();

        assert_eq!(
            store.record_event_once(&retry).unwrap(),
            EventWriteOutcome::Existing
        );
        assert_eq!(store.list_events().unwrap(), vec![first]);
    }

    #[test]
    fn same_payload_hash_but_different_signature_returns_existing_divergent_signature() {
        let (_root, store) = temp_event_store();
        let first = signed_review_initialized_event(
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==",
        );
        let second = signed_review_initialized_event(
            "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB==",
        );
        assert_eq!(first.idempotency_key, second.idempotency_key);
        assert_eq!(first.payload_hash, second.payload_hash);

        assert_eq!(
            store.record_event_once(&first).unwrap(),
            EventWriteOutcome::Created
        );
        assert_eq!(
            store.record_event_once(&second).unwrap(),
            EventWriteOutcome::ExistingDivergentSignature
        );
        assert_eq!(store.list_events().unwrap(), vec![first]);
    }

    #[test]
    fn divergent_signature_with_differing_event_record_hash_is_plain_existing() {
        // Same idempotencyKey and payloadHash but a different occurredAt diverges the
        // signer-exclusive eventRecordHash: not the same content record, so not a
        // transcription-eligible co-signature. First-stored wins as plain Existing.
        let (_root, store) = temp_event_store();
        let first = signed_review_initialized_event(
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==",
        );
        let mut second = signed_review_initialized_event(
            "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB==",
        );
        second.occurred_at = "2026-06-04T00:00:00Z".to_owned();

        assert_eq!(first.idempotency_key, second.idempotency_key);
        assert_eq!(first.payload_hash, second.payload_hash);
        assert_ne!(
            first.event_record_hash().unwrap(),
            second.event_record_hash().unwrap()
        );

        store.record_event_once(&first).unwrap();
        assert_eq!(
            store.record_event_once(&second).unwrap(),
            EventWriteOutcome::Existing
        );
    }

    #[test]
    fn same_idempotency_key_with_conflicting_payload_is_an_error() {
        let (_root, store) = temp_event_store();
        let first = review_initialized_event();
        let conflicting = conflicting_event_with_same_idempotency_key(&first);

        store.record_event_once(&first).unwrap();
        let error = store
            .record_event_once(&conflicting)
            .expect_err("conflict is rejected");

        assert!(error.to_string().contains("conflict"));
    }

    #[test]
    fn record_event_rejects_advisory_review_assessment_recorded() {
        let (_root, store) = temp_event_store();
        let mut event = review_assessment_recorded_event();
        event.assertion_mode = AssertionMode::Advisory;

        let error = store
            .record_event_once(&event)
            .expect_err("advisory assessment event is invalid");

        assert!(error.to_string().contains("assertionMode Operative"));
    }

    #[test]
    fn role_free_stored_events_read_cleanly() {
        let (_root, store) = temp_event_store();
        let event = review_initialized_event();
        let path = store.event_path_for_idempotency_key(&event.idempotency_key);
        store.record_event_once(&event).unwrap();

        let read = store.read_event(&path).expect("current-shape event reads");
        assert_eq!(read, event);
    }

    #[test]
    fn read_event_accepts_producer_keyed_envelope() {
        let (_root, store) = temp_event_store();
        let event = review_initialized_event();
        let path = store.event_path_for_idempotency_key(&event.idempotency_key);
        store.record_event_once(&event).unwrap();

        // A current producer-keyed event reads cleanly; the tool probe must not
        // be over-eager.
        let read = store
            .read_event(&path)
            .expect("producer-keyed event reads cleanly");
        assert_eq!(read, event);
    }

    #[test]
    fn read_event_rejects_filename_idempotency_mismatch() {
        let (_root, store) = temp_event_store();
        let event = review_initialized_event();
        let wrong_path = store.events_dir().join(format!("{}.json", "0".repeat(64)));
        std::fs::create_dir_all(store.events_dir()).unwrap();
        fs::write(&wrong_path, serde_json::to_vec(&event).unwrap()).unwrap();

        let error = store
            .read_event(&wrong_path)
            .expect_err("filename mismatch is rejected");

        assert!(error.to_string().contains("idempotencyKey"));
    }

    #[test]
    fn list_events_ignores_temp_files_and_unknown_suffixes() {
        let (_root, store) = temp_event_store();
        let event = review_initialized_event();
        store.record_event_once(&event).unwrap();
        fs::write(
            store.events_dir().join(".shore-write.partial.tmp"),
            b"partial",
        )
        .unwrap();
        fs::write(store.events_dir().join("README.txt"), b"ignore me").unwrap();

        assert_eq!(store.list_events().unwrap(), vec![event]);
    }

    #[test]
    fn list_event_file_names_ignores_temp_files_and_unknown_suffixes() {
        let (_root, store) = temp_event_store();
        let event = review_initialized_event();
        store.record_event_once(&event).unwrap();
        fs::write(
            store.events_dir().join(".shore-write.partial.tmp"),
            b"partial",
        )
        .unwrap();
        fs::write(store.events_dir().join("README.txt"), b"ignore me").unwrap();

        let expected = format!("{}.json", event_filename_stem(&event.idempotency_key));
        assert_eq!(store.list_event_file_names().unwrap(), vec![expected]);
    }

    #[test]
    fn list_event_file_names_with_missing_events_dir_is_empty() {
        let (_root, store) = temp_event_store();

        assert_eq!(store.list_event_file_names().unwrap(), Vec::<String>::new());
    }

    #[test]
    fn record_event_once_is_existing_across_ingest_stamp_differences_first_stored_wins() {
        // A locally authored stored event can never acquire a stamp after the
        // fact; an ingested event can never lose (or swap) its first stamp.
        let (_root, store) = temp_event_store();
        let unstamped = review_initialized_event();
        let mut stamped = unstamped.clone();
        stamped.ingest = Some(IngestProvenance {
            via: IngestVia::IngestEvents,
            received_at: "unix-ms:1760000000000".to_owned(),
        });

        // stored unstamped + incoming stamped => Existing, stored file stays unstamped
        assert_eq!(
            store.record_event_once(&unstamped).unwrap(),
            EventWriteOutcome::Created
        );
        assert_eq!(
            store.record_event_once(&stamped).unwrap(),
            EventWriteOutcome::Existing
        );
        let path = store.event_path_for_idempotency_key(&unstamped.idempotency_key);
        assert!(store.read_event(&path).unwrap().ingest.is_none());

        // stored stamped + incoming differently-stamped => Existing, first stamp kept
        let (_root_two, store_two) = temp_event_store();
        let mut later_stamp = unstamped.clone();
        later_stamp.ingest = Some(IngestProvenance {
            via: IngestVia::BundleApply,
            received_at: "unix-ms:1760000000001".to_owned(),
        });
        assert_eq!(
            store_two.record_event_once(&stamped).unwrap(),
            EventWriteOutcome::Created
        );
        assert_eq!(
            store_two.record_event_once(&later_stamp).unwrap(),
            EventWriteOutcome::Existing
        );
        let path_two = store_two.event_path_for_idempotency_key(&stamped.idempotency_key);
        assert_eq!(
            store_two.read_event(&path_two).unwrap().ingest,
            stamped.ingest
        );
    }

    #[test]
    fn payload_hash_is_stable_across_a_journal_round_trip() {
        let (_root, store) = temp_event_store();
        assert_payload_hash_is_stable_across_a_round_trip(&store);
    }

    #[test]
    fn payload_hash_is_stable_across_a_round_trip_on_the_in_memory_backend() {
        // The O1 pin holds through any backend: an injected in-memory store
        // re-serializes the same payload and re-derives the same payloadHash with
        // no filesystem involved.
        assert_payload_hash_is_stable_across_a_round_trip(&in_memory_event_store());
    }

    #[test]
    fn co_signature_decision_holds_over_the_in_memory_backend() {
        // The whole Created → Existing → ExistingDivergentSignature → conflict
        // ladder, and a byte-stable list round-trip, without a filesystem. If any
        // arm of this decision had been written inside LocalStorage rather than the
        // wrapper, the in-memory backend could not reproduce it — this is the
        // honesty test for the co-signature wrapper.
        let store = in_memory_event_store();
        let first = review_initialized_event();
        assert_eq!(
            store.record_event_once(&first).unwrap(),
            EventWriteOutcome::Created
        );
        assert!(store.event_exists(&first.idempotency_key).unwrap());

        // A replay with a later occurredAt but the same payload is Existing, and
        // does not rewrite the stored copy.
        let retry = review_initialized_event_at("2026-05-10T00:01:00Z");
        assert_eq!(
            store.record_event_once(&retry).unwrap(),
            EventWriteOutcome::Existing
        );
        assert_eq!(store.list_events().unwrap(), vec![first]);

        // Same payloadHash and eventRecordHash, a different signature: a
        // transcription-eligible co-signature, not a conflict.
        let signed_store = in_memory_event_store();
        let signed_a = signed_review_initialized_event(
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==",
        );
        let signed_b = signed_review_initialized_event(
            "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB==",
        );
        assert_eq!(
            signed_store.record_event_once(&signed_a).unwrap(),
            EventWriteOutcome::Created
        );
        assert_eq!(
            signed_store.record_event_once(&signed_b).unwrap(),
            EventWriteOutcome::ExistingDivergentSignature
        );

        // Same idempotencyKey, a different payload: a loud conflict.
        let conflict_store = in_memory_event_store();
        let base = review_initialized_event();
        let conflicting = conflicting_event_with_same_idempotency_key(&base);
        conflict_store.record_event_once(&base).unwrap();
        assert!(
            conflict_store
                .record_event_once(&conflicting)
                .unwrap_err()
                .to_string()
                .contains("conflict")
        );
    }

    #[test]
    fn list_events_rejects_payload_hash_mismatch_over_every_backend() {
        for_each_event_backend(|store, journal| {
            let event = review_initialized_event();
            let mut json = serde_json::to_value(&event).unwrap();
            json["payloadHash"] = serde_json::json!("sha256:wrong");
            journal
                .insert_raw(&event.idempotency_key, &serde_json::to_vec(&json).unwrap())
                .unwrap();

            let error = store
                .list_events()
                .expect_err("a payloadHash mismatch is rejected on replay");
            assert!(error.to_string().contains("payloadHash"));
        });
    }

    #[test]
    fn list_events_rejects_event_id_mismatch_over_every_backend() {
        for_each_event_backend(|store, journal| {
            let event = review_initialized_event();
            let mut json = serde_json::to_value(&event).unwrap();
            json["eventId"] = serde_json::json!("evt:sha256:wrong");
            journal
                .insert_raw(&event.idempotency_key, &serde_json::to_vec(&json).unwrap())
                .unwrap();

            let error = store
                .list_events()
                .expect_err("an eventId mismatch is rejected on replay");
            assert!(error.to_string().contains("eventId"));
        });
    }

    #[test]
    fn list_events_rejects_retired_event_type_over_every_backend() {
        for_each_event_backend(|store, journal| {
            let event = review_initialized_event();
            let mut json = serde_json::to_value(&event).unwrap();
            json["eventType"] = serde_json::json!("review_disposition_recorded");
            journal
                .insert_raw(&event.idempotency_key, &serde_json::to_vec(&json).unwrap())
                .unwrap();

            let error = store
                .list_events()
                .expect_err("a retired event type is rejected on replay");
            assert!(matches!(
                error,
                ShoreError::UnsupportedEventType(ref record)
                    if record.retired == "review_disposition_recorded"
            ));
        });
    }

    #[test]
    fn list_events_rejects_legacy_writer_role_envelope_over_every_backend() {
        for_each_event_backend(|store, journal| {
            let event = review_initialized_event();
            let mut json = serde_json::to_value(&event).unwrap();
            json["writer"]["role"] = serde_json::json!("reviewer");
            journal
                .insert_raw(&event.idempotency_key, &serde_json::to_vec(&json).unwrap())
                .unwrap();

            let error = store
                .list_events()
                .expect_err("a role-bearing envelope is rejected on replay");
            assert!(matches!(error, ShoreError::UnsupportedEventEnvelope(_)));
            assert!(
                error
                    .to_string()
                    .contains("docs/storage-model.md#legacy-writer-role-events")
            );
        });
    }

    #[test]
    fn list_events_rejects_legacy_writer_tool_envelope_over_every_backend() {
        for_each_event_backend(|store, journal| {
            let event = review_initialized_event();
            let mut json = serde_json::to_value(&event).unwrap();
            let writer = json["writer"].as_object_mut().unwrap();
            writer.remove("producer");
            writer.insert(
                "tool".to_owned(),
                serde_json::json!({ "name": "shore", "version": "0.1.0" }),
            );
            journal
                .insert_raw(&event.idempotency_key, &serde_json::to_vec(&json).unwrap())
                .unwrap();

            let error = store
                .list_events()
                .expect_err("a tool-bearing envelope is rejected on replay");
            assert!(matches!(error, ShoreError::UnsupportedEventEnvelope(_)));
            assert!(
                error
                    .to_string()
                    .contains("docs/storage-model.md#legacy-writer-tool-events")
            );
            assert!(error.to_string().contains("writer.tool"));
        });
    }

    #[test]
    fn list_events_rejects_pre_reshape_target_envelope_over_every_backend() {
        for_each_event_backend(|store, journal| {
            let event = review_initialized_event();
            let mut json = serde_json::to_value(&event).unwrap();
            json["target"] = serde_json::json!({
                "sessionId": "session:default",
                "reviewUnitId": "review-unit:sha256:legacy",
                "revisionId": "rev:git:sha256:legacy",
                "snapshotId": "snap:git:sha256:legacy",
            });
            journal
                .insert_raw(&event.idempotency_key, &serde_json::to_vec(&json).unwrap())
                .unwrap();

            let error = store
                .list_events()
                .expect_err("a pre-reshape target envelope is rejected on replay");
            let message = error.to_string();
            assert!(
                message.contains("journalId") || message.contains("subject"),
                "rejection names a missing reshaped-target field; got: {error}"
            );
        });
    }

    #[test]
    fn list_events_rejects_a_mislocated_entry_over_every_backend() {
        for_each_event_backend(|store, journal| {
            // A valid event stored under a key whose content-address digest is not
            // its home: the listing replay-check must reject it, and the lenient
            // read must hard-error (this is corruption, not a retired schema).
            let event = review_initialized_event();
            journal
                .insert_raw("not:this:events:home", &serde_json::to_vec(&event).unwrap())
                .unwrap();

            assert!(
                store
                    .list_events()
                    .expect_err("a mislocated entry is rejected on replay")
                    .to_string()
                    .contains("idempotencyKey")
            );
            assert!(store.list_events_lenient().is_err());
        });
    }

    #[test]
    fn list_events_lenient_skips_retired_schemas_over_every_backend() {
        for_each_event_backend(|store, journal| {
            let valid = review_initialized_event();
            store.record_event_once(&valid).unwrap();

            // A retired-type blob and a writer.role blob, injected raw: the probe
            // rejects them before full decode, so no valid signature/hash is needed.
            journal
                .insert_raw(
                    "retired:type:key",
                    br#"{"eventType":"review_disposition_recorded"}"#,
                )
                .unwrap();
            journal
                .insert_raw(
                    "retired:envelope:key",
                    br#"{"eventType":"review_initialized","writer":{"role":"x"}}"#,
                )
                .unwrap();

            let (events, skipped) = store.list_events_lenient().unwrap();
            assert_eq!(events.len(), 1, "the one valid event survives");
            assert_eq!(skipped.len(), 2);
            let codes: std::collections::BTreeSet<_> = skipped.iter().map(|s| s.code).collect();
            assert!(codes.contains(&"unsupported_event_type"));
            assert!(codes.contains(&"unsupported_event_envelope"));
        });
    }

    #[test]
    fn list_events_lenient_hard_errors_on_genuine_corruption_over_every_backend() {
        for_each_event_backend(|store, journal| {
            // Not a retired schema — a malformed event that fails decode for
            // another reason must propagate, not be silently skipped.
            journal
                .insert_raw("corrupt:key", br#"{"eventType":"review_initialized"}"#)
                .unwrap();
            assert!(store.list_events_lenient().is_err());
        });
    }

    /// Events may re-serialize in any byte layout as long as the payload Value
    /// round-trips losslessly (canonical key-sort erases byte order at hash time).
    /// Record an event whose payload carries the hazardous shapes —
    /// large/negative integers, non-ASCII text, deliberately unsorted keys — read
    /// it back through the listing surface, and assert payloadHash is byte-stable
    /// and re-derives identically.
    fn assert_payload_hash_is_stable_across_a_round_trip(store: &EventStore) {
        let mut event = review_initialized_event();
        event.payload = serde_json::json!({
            "zeta": 1,
            "alpha": -9_007_199_254_740_993_i64,
            "huge": 9_007_199_254_740_993_i64,
            "text": "café ☕ 日本語 — déjà vu",
            "nested": { "b": 2, "a": 1 },
        });
        event.payload_hash = sha256_json_prefixed(&event.payload).unwrap();
        let original_hash = event.payload_hash.clone();

        store.record_event_once(&event).unwrap();
        let read_back = store.list_events().unwrap();

        assert_eq!(read_back.len(), 1);
        assert_eq!(read_back[0].payload_hash, original_hash);
        assert_eq!(
            sha256_json_prefixed(&read_back[0].payload).unwrap(),
            original_hash,
            "the payload digest re-derives identically after a storage round trip"
        );
    }

    #[test]
    fn listing_order_is_hash_sorted_and_renders_a_stable_projection() {
        // The listing order is load-bearing: a first-seen-wins reducer folds
        // events in slice order. The byte listing must come back in the same
        // hash-sorted order as the file names, and the projection rendered from it
        // must be byte-identical to one rendered from a direct file-name read.
        let (_root, store) = temp_event_store();
        // Insertion order deliberately differs from the hash-sorted file order.
        for session in ["session:c", "session:a", "session:b", "session:d"] {
            store
                .record_event_once(&review_initialized_event_for(session))
                .unwrap();
        }

        let by_listing = store.list_events().unwrap();
        let listed_file_names: Vec<String> = by_listing
            .iter()
            .map(|event| format!("{}.json", event_filename_stem(&event.idempotency_key)))
            .collect();
        assert_eq!(
            listed_file_names,
            store.list_event_file_names().unwrap(),
            "the byte listing is in the same hash-sorted order as the file names"
        );

        let by_file_name: Vec<ShoreEvent> = store
            .list_event_file_names()
            .unwrap()
            .into_iter()
            .map(|name| store.read_event(&store.events_dir().join(name)).unwrap())
            .collect();
        let from_listing =
            serde_json::to_vec(&SessionState::from_events(&by_listing).unwrap()).unwrap();
        let from_file_name =
            serde_json::to_vec(&SessionState::from_events(&by_file_name).unwrap()).unwrap();
        assert_eq!(from_listing, from_file_name);
    }

    fn review_initialized_event_for(session: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewInitialized,
            format!("review_initialized:{session}:work:default"),
            EventTarget::for_journal(JournalId::new(session)),
            Writer::shore_local("0.1.0"),
            ReviewInitializedPayload {},
            "2026-05-10T00:00:00Z",
        )
        .expect("event builds")
    }

    fn temp_event_store() -> (tempfile::TempDir, EventStore) {
        let root = tempfile::tempdir().unwrap();
        let store = EventStore::open(root.path().join(".shore/data"));
        (root, store)
    }

    /// An event wrapper over a fresh injection-only in-memory backend — no temp
    /// dir, no filesystem. The same production constructor (`from_backend`)
    /// resolved consumers use, so the honesty tests exercise the real wiring.
    fn in_memory_event_store() -> EventStore {
        EventStore::from_backend(&StoreBackend::memory())
    }

    /// Run a tamper assertion against each backend: a wrapper (`EventStore`) and a
    /// raw `Journal` handle over the **same** underlying store, so the test injects
    /// bytes through the journal (bypassing write-side validation) and asserts the
    /// wrapper's read-side validation rejects them — identically for both backends.
    fn for_each_event_backend(mut assertion: impl FnMut(&EventStore, &dyn Journal)) {
        let root = tempfile::tempdir().unwrap();
        let local = StoreBackend::Local(root.path().join(".shore/data"));
        let local_journal = local.journal();
        assertion(&EventStore::from_backend(&local), local_journal.as_ref());

        let memory = StoreBackend::memory();
        let memory_journal = memory.journal();
        assertion(&EventStore::from_backend(&memory), memory_journal.as_ref());
    }

    fn review_initialized_event() -> ShoreEvent {
        review_initialized_event_at("2026-05-10T00:00:00Z")
    }

    fn review_initialized_event_at(occurred_at: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewInitialized,
            "review_initialized:session:default:work:default",
            EventTarget::for_journal(JournalId::new("journal:default")),
            Writer::shore_local("0.1.0"),
            ReviewInitializedPayload {},
            occurred_at,
        )
        .expect("event builds")
    }

    fn signed_review_initialized_event(signature: &str) -> ShoreEvent {
        let mut event = review_initialized_event();
        event.signer = Some(
            SignerId::parse("did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd").unwrap(),
        );
        event.signature = Some(EventSignature::new_ed25519_v1(signature).unwrap());
        event
    }

    fn review_assessment_recorded_event() -> ShoreEvent {
        let revision_id = RevisionId::new("review-unit:sha256:one");
        let track_id = TrackId::new("human:kevin");
        let assessment_id = AssessmentId::new("assess:sha256:one");
        let target_ref = ReviewTargetRef::Revision {
            revision_id: revision_id.clone(),
        };

        ShoreEvent::new(
            EventType::ReviewAssessmentRecorded,
            ReviewAssessmentRecordedPayload::idempotency_key(
                &revision_id,
                &track_id,
                assessment_id.as_str(),
            ),
            EventTarget::for_subject(
                JournalId::new("journal:default"),
                TargetRef::Review(target_ref.clone()),
                Some(track_id),
            )
            .unwrap(),
            Writer::shore_local("test"),
            ReviewAssessmentRecordedPayload {
                assessment_id,
                target: target_ref,
                assessment: ReviewAssessment::Accepted,
                summary: Some("Ship it".to_owned()),
                summary_content_type: Default::default(),
                summary_artifact_path: None,
                summary_byte_size: Some(7),
                summary_content_hash: Some("sha256:summary".to_owned()),
                replaces_assessment_ids: Vec::new(),
                related_observation_ids: Vec::new(),
                related_input_request_ids: Vec::new(),
            },
            "2026-05-10T00:00:00Z",
        )
        .expect("event builds")
    }

    fn conflicting_event_with_same_idempotency_key(event: &ShoreEvent) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewNoteImported,
            event.idempotency_key.clone(),
            event.target.clone(),
            event.writer.clone(),
            ReviewNoteImportedPayload {
                sidecar_source: crate::session::event::SidecarSource::ReviewNotes,
                note_id: "note:conflict".to_owned(),
                file_path: "src/lib.rs".to_owned(),
                file_old_path: None,
                target: None,
                title: "Conflicting payload".to_owned(),
                body: None,
                body_artifact_path: None,
                body_byte_size: None,
                tags: Vec::new(),
                confidence: None,
                external_source: None,
                author: None,
                created_at: None,
                sidecar_content_hash: "sha256:sidecar".to_owned(),
            },
            event.occurred_at.clone(),
        )
        .expect("conflicting event builds")
    }
}
