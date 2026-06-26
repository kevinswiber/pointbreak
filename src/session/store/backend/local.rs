//! File-backed implementations of the durable backend traits over
//! [`LocalStorage`]. These preserve today's on-disk layout, hash-sorted listing
//! order, and atomic create-if-absent semantics, so a backend swap is invisible
//! to every stored byte.

use std::path::{Path, PathBuf};

use super::{ContentStore, Journal, JournalEntry};
use crate::error::{Result, ShoreError};
use crate::session::store::event_store::{event_filename_stem, is_event_file};
use crate::storage::{CreateOutcome, Durability, LocalStorage, RemoveOutcome, is_temp_file_path};

/// The file-backed [`Journal`]: events live at
/// `events/<sha256(idempotency_key)>.json` under the store dir.
#[derive(Debug)]
pub(crate) struct LocalJournal {
    storage: LocalStorage,
    store_dir: PathBuf,
}

impl LocalJournal {
    pub(crate) fn new(store_dir: impl AsRef<Path>) -> Self {
        let store_dir = store_dir.as_ref().to_path_buf();
        Self {
            storage: LocalStorage::new(&store_dir),
            store_dir,
        }
    }

    fn events_dir(&self) -> PathBuf {
        self.store_dir.join("events")
    }

    fn event_path(&self, idempotency_key: &str) -> PathBuf {
        self.events_dir()
            .join(format!("{}.json", event_filename_stem(idempotency_key)))
    }
}

impl Journal for LocalJournal {
    fn create_event_once(&self, idempotency_key: &str, bytes: &[u8]) -> Result<CreateOutcome> {
        self.storage.create_file_exclusive(
            &self.event_path(idempotency_key),
            bytes,
            Durability::Durable,
        )
    }

    fn read_event_bytes(&self, idempotency_key: &str) -> Result<Option<Vec<u8>>> {
        self.storage
            .read_bytes_if_exists(&self.event_path(idempotency_key))
    }

    fn event_exists(&self, idempotency_key: &str) -> Result<bool> {
        Ok(self.event_path(idempotency_key).exists())
    }

    fn list_event_entries(&self) -> Result<Vec<JournalEntry>> {
        // `list_dir` already sorts, so this preserves today's hash-sorted order;
        // the same event-file filter keeps temp files and stray names out. Each
        // entry's digest is the file name stem (the sha256 of the logical key it
        // was stored under), which the wrapper checks against the decoded event.
        self.storage
            .list_dir(&self.events_dir())?
            .into_iter()
            .filter(|path| is_event_file(path))
            .map(|path| {
                let key_digest = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .ok_or_else(|| {
                        ShoreError::Message(format!(
                            "event file has no readable name: {}",
                            path.display()
                        ))
                    })?
                    .to_owned();
                let bytes = self.storage.read_bytes(&path)?;
                Ok(JournalEntry { key_digest, bytes })
            })
            .collect()
    }

    #[cfg(test)]
    fn insert_raw(&self, idempotency_key: &str, bytes: &[u8]) -> Result<()> {
        // A plain atomic write at the key's content-addressed path, overwriting
        // any existing file — the create-if-absent dedup is deliberately skipped.
        self.storage.write_bytes_atomic(
            &self.event_path(idempotency_key),
            bytes,
            Durability::Durable,
        )
    }
}

/// The file-backed [`ContentStore`]: blobs live at their store-relative
/// `content_ref` under the store dir.
#[derive(Debug)]
pub(crate) struct LocalContentStore {
    storage: LocalStorage,
}

impl LocalContentStore {
    pub(crate) fn new(store_dir: impl AsRef<Path>) -> Self {
        Self {
            storage: LocalStorage::new(store_dir),
        }
    }
}

impl ContentStore for LocalContentStore {
    fn put_once(&self, content_ref: &str, bytes: &[u8]) -> Result<CreateOutcome> {
        self.storage
            .create_file_exclusive(Path::new(content_ref), bytes, Durability::Durable)
    }

    fn get(&self, content_ref: &str) -> Result<Vec<u8>> {
        self.storage.read_bytes(Path::new(content_ref))
    }

    fn get_if_exists(&self, content_ref: &str) -> Result<Option<Vec<u8>>> {
        self.storage.read_bytes_if_exists(Path::new(content_ref))
    }

    fn remove(&self, content_ref: &str) -> Result<RemoveOutcome> {
        self.storage.remove_file(content_ref)
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>> {
        // `list_dir` returns sorted, store-resolved paths; surface each as a
        // store-relative ref under `prefix`, dropping any non-UTF-8 name.
        Ok(self
            .storage
            .list_dir(Path::new(prefix))?
            .into_iter()
            .filter(|path| !is_temp_file_path(path))
            .filter_map(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| format!("{prefix}/{name}"))
            })
            .collect())
    }

    #[cfg(test)]
    fn put_raw(&self, content_ref: &str, bytes: &[u8]) -> Result<()> {
        // A plain atomic write at the locator, overwriting any existing blob —
        // the create-side validation the wrapper performs is deliberately skipped.
        self.storage
            .write_bytes_atomic(Path::new(content_ref), bytes, Durability::Durable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_list_ignores_in_flight_temp_files() {
        let root = tempfile::tempdir().unwrap();
        let store = LocalContentStore::new(root.path());
        let objects_dir = root.path().join("artifacts/objects");
        std::fs::create_dir_all(&objects_dir).unwrap();
        std::fs::write(objects_dir.join(".shore-write.inflight.tmp"), b"partial").unwrap();

        store.put_once("artifacts/objects/a.json", b"a").unwrap();

        assert_eq!(
            store.list("artifacts/objects").unwrap(),
            vec!["artifacts/objects/a.json".to_owned()]
        );
    }
}
