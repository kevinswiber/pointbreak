use std::path::{Path, PathBuf};

use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::error::{Result, SchemaBreakRecord, ShoreError};
use crate::session::event::{AssertionMode, EventType, ShoreEvent};
use crate::storage::{CreateFileOutcome, Durability, LocalStorage};

#[derive(Debug)]
pub struct EventStore {
    store_dir: PathBuf,
    storage: LocalStorage,
}

impl EventStore {
    pub fn open(store_dir: impl AsRef<Path>) -> Self {
        let store_dir = store_dir.as_ref().to_path_buf();
        Self {
            storage: LocalStorage::new(&store_dir),
            store_dir,
        }
    }

    pub(crate) fn events_dir(&self) -> PathBuf {
        self.store_dir.join("events")
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

        validate_event(
            event,
            Some(&self.event_path_for_idempotency_key(&event.idempotency_key)),
        )?;
        let path = self.event_path_for_idempotency_key(&event.idempotency_key);
        let bytes = serde_json::to_vec(event)?;

        match self
            .storage
            .create_file_exclusive(&path, &bytes, Durability::Durable)?
        {
            CreateFileOutcome::Created => {
                tracing::debug!(path = %path.display(), "event_store_write_created");
                Ok(EventWriteOutcome::Created)
            }
            CreateFileOutcome::AlreadyExists => {
                let existing = self.read_event(&path)?;
                if existing.payload_hash == event.payload_hash {
                    if event_signature_binding_matches(&existing, event) {
                        tracing::debug!(path = %path.display(), "event_store_write_existing");
                        Ok(EventWriteOutcome::Existing)
                    } else if existing.event_record_hash()? == event.event_record_hash()? {
                        // Same content record (signer-exclusive eventRecordHash matches),
                        // differently signed: a transcription-eligible co-signature, not a
                        // conflict.
                        tracing::debug!(
                            path = %path.display(),
                            "event_store_write_existing_divergent_signature"
                        );
                        Ok(EventWriteOutcome::ExistingDivergentSignature)
                    } else {
                        // Same idempotencyKey and payloadHash but a different record
                        // (eventRecordHash differs) — an eventId collision without content
                        // identity, not a co-signature. Keep the first-stored copy.
                        tracing::debug!(path = %path.display(), "event_store_write_existing");
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
        let bytes = self.storage.read_bytes(path)?;
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
        let probe: EventProbe<'_> = serde_json::from_slice(&bytes)?;
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
        let event: ShoreEvent = serde_json::from_slice(&bytes)?;
        validate_event(&event, Some(path))?;
        Ok(event)
    }

    pub fn list_events(&self) -> Result<Vec<ShoreEvent>> {
        self.list_event_file_names()?
            .into_iter()
            .map(|name| self.read_event(&self.events_dir().join(name)))
            .collect()
    }

    /// Event file names in this store, with the same accept/skip rules as
    /// `list_events` but without parsing event JSON. Sorted; a missing events
    /// directory lists as empty.
    pub(crate) fn list_event_file_names(&self) -> Result<Vec<String>> {
        Ok(self
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
        Ok(self
            .event_path_for_idempotency_key(idempotency_key)
            .exists())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventWriteOutcome {
    Created,
    Existing,
    ExistingDivergentSignature,
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
        "evt:sha256:{}",
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
            return Err(ShoreError::Message(format!(
                "event filename does not match idempotencyKey for {}",
                event.idempotency_key
            )));
        }
    }

    Ok(())
}

fn event_filename_stem(idempotency_key: &str) -> String {
    sha256_bytes_hex(idempotency_key.as_bytes())
}

fn is_event_file(path: &Path) -> bool {
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
    fn legacy_review_disposition_recorded_event_returns_typed_unsupported_error() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("legacy.json");
        fs::write(
            &path,
            r#"{"eventType":"review_disposition_recorded","schema":"shore.event","version":1,"eventId":"evt:sha256:0","idempotencyKey":"x","target":{},"writer":{},"occurredAt":"2026-05-19T00:00:00Z","payloadHash":"sha256:0","payload":{}}"#,
        )
        .unwrap();
        let store = EventStore::open(root.path());

        let err = store
            .read_event(&path)
            .expect_err("legacy event type must be rejected");

        assert!(matches!(
            err,
            ShoreError::UnsupportedEventType(ref record)
                if record.retired == "review_disposition_recorded"
        ));
        assert!(
            err.to_string()
                .contains("docs/assessment-model.md#legacy-disposition-events")
        );
    }

    #[test]
    fn legacy_intervention_events_return_typed_unsupported_error_after_input_request_rename() {
        for legacy_event_type in ["intervention_requested", "intervention_resolved"] {
            let err =
                read_legacy_event(legacy_event_type).expect_err("legacy event must be rejected");

            assert!(matches!(
                err,
                ShoreError::UnsupportedEventType(ref record)
                    if record.retired == legacy_event_type
            ));
        }
    }

    #[test]
    fn stored_events_carrying_writer_role_return_typed_legacy_error() {
        let (_root, store) = temp_event_store();
        let event = review_initialized_event();
        let path = store.event_path_for_idempotency_key(&event.idempotency_key);
        fs::create_dir_all(store.events_dir()).unwrap();

        // Pre-break envelope shape: the writer object carries a role field.
        let mut json = serde_json::to_value(event).unwrap();
        json["writer"]["role"] = serde_json::json!("reviewer");
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

        let err = store
            .read_event(&path)
            .expect_err("role-bearing stored event must be rejected");

        assert!(matches!(err, ShoreError::UnsupportedEventEnvelope(_)));
        assert!(
            err.to_string()
                .contains("docs/storage-model.md#legacy-writer-role-events"),
            "error carries the public migration anchor; got: {err}"
        );
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
    fn read_event_rejects_stored_legacy_writer_tool_envelope() {
        let (_root, store) = temp_event_store();
        let event = review_initialized_event();
        let path = store.event_path_for_idempotency_key(&event.idempotency_key);
        fs::create_dir_all(store.events_dir()).unwrap();

        // Pre-rename envelope shape: the writer object carries `tool` and no
        // `producer` key.
        let mut json = serde_json::to_value(event).unwrap();
        let writer = json["writer"].as_object_mut().unwrap();
        writer.remove("producer");
        writer.insert(
            "tool".to_owned(),
            serde_json::json!({ "name": "shore", "version": "0.1.0" }),
        );
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

        let err = store
            .read_event(&path)
            .expect_err("tool-bearing stored event must be rejected");

        assert!(
            matches!(err, ShoreError::UnsupportedEventEnvelope(_)),
            "expected the typed migration error, got: {err:?}"
        );
        assert!(
            err.to_string()
                .contains("docs/storage-model.md#legacy-writer-tool-events"),
            "error carries the public migration anchor; got: {err}"
        );
        assert!(
            err.to_string().contains("writer.tool"),
            "error names the retired envelope field; got: {err}"
        );
    }

    #[test]
    fn read_event_rejects_stored_pre_reshape_target_envelope() {
        // A stored event whose `target` carries the old flat shape (a sessionId
        // plus reviewUnitId/snapshotId optionals and no `subject`) must be loudly
        // rejected: `subject` is now the single, non-optional address, so the old
        // shape cannot decode. There is no silent upgrade.
        let (_root, store) = temp_event_store();
        let event = review_initialized_event();
        let path = store.event_path_for_idempotency_key(&event.idempotency_key);
        fs::create_dir_all(store.events_dir()).unwrap();

        let mut json = serde_json::to_value(event).unwrap();
        json["target"] = serde_json::json!({
            "sessionId": "session:default",
            "reviewUnitId": "review-unit:sha256:legacy",
            "revisionId": "rev:git:sha256:legacy",
            "snapshotId": "snap:git:sha256:legacy",
        });
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

        let error = store
            .read_event(&path)
            .expect_err("a pre-reshape target envelope must be rejected");

        // The decode fails on a missing non-optional field of the reshaped target
        // (`journalId` / `subject`) — the old flat shape carries neither.
        let message = error.to_string();
        assert!(
            message.contains("journalId") || message.contains("subject"),
            "rejection names a missing reshaped-target field; got: {error}"
        );
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
    fn read_event_rejects_payload_hash_mismatch() {
        let (_root, store) = temp_event_store();
        let event = review_initialized_event();
        let path = store.event_path_for_idempotency_key(&event.idempotency_key);
        store.record_event_once(&event).unwrap();

        let mut json = serde_json::to_value(&event).unwrap();
        json["payloadHash"] = serde_json::json!("sha256:wrong");
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

        let error = store
            .read_event(&path)
            .expect_err("payload hash mismatch is rejected");

        assert!(error.to_string().contains("payloadHash"));
    }

    #[test]
    fn read_event_rejects_event_id_mismatch() {
        let (_root, store) = temp_event_store();
        let event = review_initialized_event();
        let path = store.event_path_for_idempotency_key(&event.idempotency_key);
        store.record_event_once(&event).unwrap();

        let mut json = serde_json::to_value(&event).unwrap();
        json["eventId"] = serde_json::json!("evt:sha256:wrong");
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

        let error = store
            .read_event(&path)
            .expect_err("event id mismatch is rejected");

        assert!(error.to_string().contains("eventId"));
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

    fn temp_event_store() -> (tempfile::TempDir, EventStore) {
        let root = tempfile::tempdir().unwrap();
        let store = EventStore::open(root.path().join(".shore/data"));
        (root, store)
    }

    fn read_legacy_event(event_type: &str) -> Result<ShoreEvent> {
        let (_root, store) = temp_event_store();
        let event = review_initialized_event();
        let path = store.event_path_for_idempotency_key(&event.idempotency_key);
        fs::create_dir_all(store.events_dir()).unwrap();

        let mut json = serde_json::to_value(event)?;
        json["eventType"] = serde_json::json!(event_type);
        fs::write(&path, serde_json::to_vec(&json)?).unwrap();

        store.read_event(&path)
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
            ),
            Writer::shore_local("test"),
            ReviewAssessmentRecordedPayload {
                assessment_id,
                target: target_ref,
                assessment: ReviewAssessment::Accepted,
                summary: Some("Ship it".to_owned()),
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
