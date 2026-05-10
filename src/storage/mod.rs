mod event_store;

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

pub use event_store::{EventStore, EventWriteOutcome};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{Result, ShoreError};

const TEMP_PREFIX: &str = ".shore-write.";
const TEMP_SUFFIX: &str = ".tmp";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Durability {
    Durable,
    Projection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateFileOutcome {
    Created,
    AlreadyExists,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TempSweepAge(Duration);

impl TempSweepAge {
    pub fn zero() -> Self {
        Self(Duration::ZERO)
    }

    pub fn from_duration(duration: Duration) -> Self {
        Self(duration)
    }
}

#[derive(Debug)]
pub struct LocalStorage {
    root: PathBuf,
}

impl LocalStorage {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    pub fn read_bytes(&self, path: &Path) -> Result<Vec<u8>> {
        let path = self.resolve(path);
        fs::read(&path).map_err(|error| io_error("read file", &path, error))
    }

    pub fn read_bytes_if_exists(&self, path: &Path) -> Result<Option<Vec<u8>>> {
        let path = self.resolve(path);
        match fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io_error("read file", &path, error)),
        }
    }

    pub fn read_json<T>(&self, path: &Path) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let bytes = self.read_bytes(path)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn write_json_atomic<T>(&self, path: &Path, value: &T, durability: Durability) -> Result<()>
    where
        T: Serialize,
    {
        let bytes = serde_json::to_vec(value)?;
        self.write_bytes_atomic(path, &bytes, durability)
    }

    pub fn write_bytes_atomic(
        &self,
        path: &Path,
        bytes: &[u8],
        durability: Durability,
    ) -> Result<()> {
        let path = self.resolve(path);
        let parent = parent_dir(&path)?;
        let temp_path = self.write_temp_file(parent, bytes, durability)?;

        match fs::rename(&temp_path, &path) {
            Ok(()) => {
                sync_parent_if_durable(parent, durability)?;
                Ok(())
            }
            Err(error) => {
                let _ = fs::remove_file(&temp_path);
                Err(io_error("rename temp file", &path, error))
            }
        }
    }

    /// Creates `path` only if it does not already exist.
    ///
    /// The implementation writes and syncs a same-directory temp file first, then links it into
    /// place so readers never observe a partially-written target. This requires a local filesystem
    /// that supports hard links, which is expected for `.shore/` inside a normal Git worktree.
    pub fn create_file_exclusive(
        &self,
        path: &Path,
        bytes: &[u8],
        durability: Durability,
    ) -> Result<CreateFileOutcome> {
        let path = self.resolve(path);
        let parent = parent_dir(&path)?;
        let temp_path = self.write_temp_file(parent, bytes, durability)?;

        match fs::hard_link(&temp_path, &path) {
            Ok(()) => {
                fs::remove_file(&temp_path)
                    .map_err(|error| io_error("remove temp file", &temp_path, error))?;
                sync_parent_if_durable(parent, durability)?;
                Ok(CreateFileOutcome::Created)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                fs::remove_file(&temp_path)
                    .map_err(|error| io_error("remove temp file", &temp_path, error))?;
                Ok(CreateFileOutcome::AlreadyExists)
            }
            Err(error) => {
                let _ = fs::remove_file(&temp_path);
                Err(io_error("create file exclusively", &path, error))
            }
        }
    }

    pub fn list_dir(&self, dir: &Path) -> Result<Vec<PathBuf>> {
        let dir = self.resolve(dir);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(io_error("list directory", &dir, error)),
        };

        let mut paths = entries
            .map(|entry| {
                entry
                    .map(|entry| entry.path())
                    .map_err(|error| io_error("read directory entry", &dir, error))
            })
            .collect::<Result<Vec<_>>>()?;
        paths.sort();
        Ok(paths)
    }

    pub fn list_temp_files(&self, dir: &Path) -> Result<Vec<PathBuf>> {
        let dir = self.resolve(dir);
        let mut temp_files = Vec::new();
        collect_temp_files(&dir, &mut temp_files)?;
        temp_files.sort();
        Ok(temp_files)
    }

    pub fn sweep_temp_files(&self, dir: &Path, minimum_age: TempSweepAge) -> Result<()> {
        for path in self.list_temp_files(dir)? {
            if temp_file_is_old_enough(&path, minimum_age)? {
                match fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(io_error("remove temp file", &path, error)),
                }
            }
        }
        Ok(())
    }

    fn resolve(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }

    fn write_temp_file(
        &self,
        parent: &Path,
        bytes: &[u8],
        durability: Durability,
    ) -> Result<PathBuf> {
        fs::create_dir_all(parent).map_err(|error| io_error("create directory", parent, error))?;

        for _ in 0..100 {
            let temp_path = parent.join(next_temp_file_name());
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
            {
                Ok(mut file) => {
                    file.write_all(bytes)
                        .map_err(|error| io_error("write temp file", &temp_path, error))?;
                    if durability == Durability::Durable {
                        file.sync_all()
                            .map_err(|error| io_error("sync temp file", &temp_path, error))?;
                    }
                    return Ok(temp_path);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(io_error("create temp file", &temp_path, error)),
            }
        }

        Err(ShoreError::Message(format!(
            "could not allocate temp file in {}",
            parent.display()
        )))
    }
}

fn collect_temp_files(dir: &Path, temp_files: &mut Vec<PathBuf>) -> Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_error("list directory", dir, error)),
    };

    for entry in entries {
        let entry = entry.map_err(|error| io_error("read directory entry", dir, error))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| io_error("read directory entry type", &path, error))?;
        if file_type.is_dir() {
            collect_temp_files(&path, temp_files)?;
        } else if is_temp_file_path(&path) {
            temp_files.push(path);
        }
    }

    Ok(())
}

fn is_temp_file_path(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    file_name.starts_with(TEMP_PREFIX) && file_name.ends_with(TEMP_SUFFIX)
}

fn temp_file_is_old_enough(path: &Path, minimum_age: TempSweepAge) -> Result<bool> {
    let modified = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map_err(|error| io_error("read temp file metadata", path, error))?;
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::ZERO);
    Ok(age >= minimum_age.0)
}

fn next_temp_file_name() -> String {
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{TEMP_PREFIX}{}.{counter}{TEMP_SUFFIX}", std::process::id())
}

fn parent_dir(path: &Path) -> Result<&Path> {
    path.parent()
        .ok_or_else(|| ShoreError::Message(format!("path has no parent: {}", path.display())))
}

fn sync_parent_if_durable(parent: &Path, durability: Durability) -> Result<()> {
    if durability == Durability::Projection {
        return Ok(());
    }

    File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|error| io_error("sync parent directory", parent, error))
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> ShoreError {
    ShoreError::Message(format!("{action} {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn atomic_write_creates_parent_dirs_and_leaves_no_temp_file() {
        let root = tempfile::tempdir().unwrap();
        let storage = LocalStorage::new(root.path());
        let path = root.path().join("nested/state.json");

        storage
            .write_bytes_atomic(&path, br#"{"ok":true}"#, Durability::Durable)
            .unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), br#"{"ok":true}"#);
        assert!(storage.list_temp_files(root.path()).unwrap().is_empty());
    }

    #[test]
    fn stale_temp_files_are_swept_by_known_prefix() {
        let root = tempfile::tempdir().unwrap();
        let stale = root.path().join(".shore-write.stale.tmp");
        std::fs::write(&stale, b"partial").unwrap();

        let storage = LocalStorage::new(root.path());
        storage
            .sweep_temp_files(root.path(), TempSweepAge::zero())
            .unwrap();

        assert!(!stale.exists());
    }

    #[test]
    fn fresh_temp_files_are_preserved_by_non_zero_sweep_age() {
        let root = tempfile::tempdir().unwrap();
        let fresh = root.path().join(".shore-write.fresh.tmp");
        std::fs::write(&fresh, b"partial").unwrap();

        let storage = LocalStorage::new(root.path());
        storage
            .sweep_temp_files(
                root.path(),
                TempSweepAge::from_duration(Duration::from_secs(60)),
            )
            .unwrap();

        assert!(fresh.exists());

        storage
            .sweep_temp_files(root.path(), TempSweepAge::zero())
            .unwrap();
        assert!(!fresh.exists());
    }

    #[test]
    fn byte_api_exists_below_json_convenience_api() {
        let root = tempfile::tempdir().unwrap();
        let storage = LocalStorage::new(root.path());
        let path = root.path().join("value.json");

        storage
            .write_json_atomic(&path, &json!({"value": 1}), Durability::Projection)
            .unwrap();
        let bytes = storage.read_bytes(&path).unwrap();

        assert!(String::from_utf8(bytes).unwrap().contains("\"value\""));
    }

    #[test]
    fn exclusive_create_reports_existing_without_overwriting() {
        let root = tempfile::tempdir().unwrap();
        let storage = LocalStorage::new(root.path());
        let path = root.path().join("events/event.json");

        assert_eq!(
            storage
                .create_file_exclusive(&path, b"first", Durability::Durable)
                .unwrap(),
            CreateFileOutcome::Created
        );
        assert_eq!(
            storage
                .create_file_exclusive(&path, b"second", Durability::Durable)
                .unwrap(),
            CreateFileOutcome::AlreadyExists
        );
        assert_eq!(storage.read_bytes(&path).unwrap(), b"first");
        assert_eq!(
            storage
                .read_bytes_if_exists(&root.path().join("missing"))
                .unwrap(),
            None
        );
    }
}
