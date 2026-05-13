use std::path::{Path, PathBuf};

use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::error::{Result, ShoreError};
use crate::session::ShoreEvent;
use crate::storage::{CreateFileOutcome, Durability, LocalStorage};

#[derive(Debug)]
pub struct EventStore {
    shore_dir: PathBuf,
    storage: LocalStorage,
}

impl EventStore {
    pub fn open(shore_dir: impl AsRef<Path>) -> Self {
        let shore_dir = shore_dir.as_ref().to_path_buf();
        Self {
            storage: LocalStorage::new(&shore_dir),
            shore_dir,
        }
    }

    pub(crate) fn events_dir(&self) -> PathBuf {
        self.shore_dir.join("events")
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
                    tracing::debug!(path = %path.display(), "event_store_write_existing");
                    Ok(EventWriteOutcome::Existing)
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
        let event: ShoreEvent = serde_json::from_slice(&bytes)?;
        validate_event(&event, Some(path))?;
        Ok(event)
    }

    pub fn list_events(&self) -> Result<Vec<ShoreEvent>> {
        self.storage
            .list_dir(&self.events_dir())?
            .into_iter()
            .filter(|path| is_event_file(path))
            .map(|path| self.read_event(&path))
            .collect()
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
}

fn validate_event(event: &ShoreEvent, path: Option<&Path>) -> Result<()> {
    event.validate_schema_version()?;

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

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::model::{ReviewId, WorkUnitId};
    use crate::session::event::ReviewNoteImportedPayload;
    use crate::session::{EventTarget, EventType, ReviewInitializedPayload, ShoreEvent, Writer};

    #[test]
    fn event_path_is_sha256_of_idempotency_key() {
        let root = tempfile::tempdir().unwrap();
        let store = EventStore::open(root.path().join(".shore"));

        let path =
            store.event_path_for_idempotency_key("review_initialized:review:default:work:default");

        assert_eq!(path.parent().unwrap(), root.path().join(".shore/events"));
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

    fn temp_event_store() -> (tempfile::TempDir, EventStore) {
        let root = tempfile::tempdir().unwrap();
        let store = EventStore::open(root.path().join(".shore"));
        (root, store)
    }

    fn review_initialized_event() -> ShoreEvent {
        review_initialized_event_at("2026-05-10T00:00:00Z")
    }

    fn review_initialized_event_at(occurred_at: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewInitialized,
            "review_initialized:review:default:work:default",
            EventTarget::new(
                ReviewId::new("review:default"),
                WorkUnitId::new("work:default"),
            ),
            Writer::shore_local_author("0.1.0"),
            ReviewInitializedPayload {},
            occurred_at,
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
                sidecar_source: crate::session::SidecarSource::ReviewNotes,
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
