use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, ShoreError};
use crate::git::{git_absolute_git_dir, git_common_dir, git_object_format};
use crate::session::event::ShoreEvent;
use crate::session::store::event_store::EventStore;
use crate::session::store::manifest::{
    StoreGitProvenance, StoreManifest, load_or_create_store_manifest, read_store_manifest,
};
use crate::session::store::store_config::{StoreMode, resolve_store_mode};
use crate::session::store::store_init::{
    ShoreStorePaths, ensure_shore_storage_excluded, prepare_store_writer_at,
};
use crate::storage::{Durability, LocalStorage};

const STORE_REGISTRATION_SCHEMA: &str = "shore.store-registration";
const STORE_REGISTRATION_VERSION: u32 = 1;
const WORKTREE_LOCAL_STORE_REF: &str = "worktree-local";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoreResolution {
    pub mode: StoreResolutionMode,
    store_dir: PathBuf,
    registration: Option<StoreRegistration>,
    manifest: Option<StoreManifest>,
}

impl StoreResolution {
    pub(crate) fn store_dir(&self) -> &Path {
        &self.store_dir
    }

    pub(crate) fn command_view(&self) -> StoreResolutionView {
        match (&self.mode, &self.registration, &self.manifest) {
            (StoreResolutionMode::CloneLocal, Some(registration), Some(_manifest)) => {
                StoreResolutionView {
                    mode: "linked",
                    store_ref: registration.store_ref.clone(),
                    clone_ref: Some(registration.clone_ref.clone()),
                    repository_family_ref: Some(registration.repository_family_ref.clone()),
                }
            }
            _ => StoreResolutionView {
                mode: "local",
                store_ref: WORKTREE_LOCAL_STORE_REF.to_owned(),
                clone_ref: None,
                repository_family_ref: None,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StoreResolutionMode {
    WorktreeLocal,
    CloneLocal,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoreRegistration {
    schema: String,
    version: u32,
    mode: String,
    pub store_ref: String,
    pub clone_ref: String,
    pub repository_family_ref: String,
}

impl StoreRegistration {
    fn clone_local(manifest: &StoreManifest) -> Self {
        Self {
            schema: STORE_REGISTRATION_SCHEMA.to_owned(),
            version: STORE_REGISTRATION_VERSION,
            mode: "cloneLocal".to_owned(),
            store_ref: manifest.store_id.clone(),
            clone_ref: manifest.clone_id.clone(),
            repository_family_ref: manifest.repository_family_id.clone(),
        }
    }

    fn validate_schema_version(&self) -> Result<()> {
        if self.schema == STORE_REGISTRATION_SCHEMA && self.version == STORE_REGISTRATION_VERSION {
            return Ok(());
        }

        Err(ShoreError::Message(format!(
            "unsupported store registration schema/version: {} v{}",
            self.schema, self.version
        )))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoreResolutionView {
    pub mode: &'static str,
    pub store_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clone_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_family_ref: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ReadStore {
    pub resolution: StoreResolution,
    /// Event file names present in the worktree-local `.shore/data/events/` but
    /// absent from the resolved linked store. Always empty in WorktreeLocal
    /// mode. Filenames are content-addressed (eventId-derived), so a name
    /// match is an identity match.
    pub local_only_event_files: Vec<String>,
}

impl ReadStore {
    pub(crate) fn store_dir(&self) -> &Path {
        self.resolution.store_dir()
    }
}

/// The read seam: read surfaces resolve their store here so linked mode reads
/// the clone-local store. After write-through (INV-1) writes land in that same
/// store, so `local_only_event_files` is non-empty only for **residual** events
/// written before this worktree ran `shore store link`; they are surfaced by the
/// divergence diagnostic, never unioned into reads.
pub(crate) fn resolve_read_store(repo: impl AsRef<Path>) -> Result<ReadStore> {
    let paths = ShoreStorePaths::resolve(repo.as_ref())?;
    let resolution = resolve_store(repo)?;
    let local_only_event_files = match resolution.mode {
        StoreResolutionMode::WorktreeLocal => Vec::new(),
        StoreResolutionMode::CloneLocal => {
            let local = EventStore::open(paths.store_dir()).list_event_file_names()?;
            let linked: HashSet<String> = EventStore::open(resolution.store_dir())
                .list_event_file_names()?
                .into_iter()
                .collect();
            let mut local_only: Vec<String> = local
                .into_iter()
                .filter(|name| !linked.contains(name))
                .collect();
            local_only.sort();
            local_only
        }
    };
    Ok(ReadStore {
        resolution,
        local_only_event_files,
    })
}

/// The write-validation seam: write surfaces resolve their *validation and
/// derivation reads* here so a fact written in a linked checkout is validated
/// against everything the writer can see — the linked store ∪ any **residual**
/// worktree-local events not yet in it. After write-through (INV-1) the write
/// itself lands in the clone-local store, so the residual set is empty in steady
/// state and this union reduces to the linked store; the union remains a safety
/// net for residual pre-link events and for the unlinked (worktree-local) case.
///
/// Three seams, deliberately distinct, but after write-through all three resolve
/// the SAME store in linked mode (the union being a residual-handling safety net,
/// not the common path):
/// - [`resolve_read_store`] — read surfaces; store-only, residual local events
///   are surfaced by diagnostic and never unioned into the result.
/// - [`resolve_write_validation_store`] — write-path validation/derivation
///   reads; the writer-visible **union** (this type).
/// - [`resolve_write_store`] — the write landing where events, artifacts, and
///   `state.json` are written (the clone-local store in linked mode, INV-1).
#[derive(Clone, Debug)]
pub(crate) struct WriteValidationStore {
    read_store: ReadStore,
    worktree_store_dir: PathBuf,
}

impl WriteValidationStore {
    /// The writer-visible union, deduplicated by event id and sorted ascending
    /// by event id. In WorktreeLocal mode this reduces to the plain local event
    /// list: `local_only_event_files` is empty there by construction, and the
    /// resolved store *is* the worktree `.shore/data`.
    pub(crate) fn validation_events(&self) -> Result<Vec<ShoreEvent>> {
        let mut merged = EventStore::open(self.read_store.store_dir()).list_events()?;
        let local_only = EventStore::open(&self.worktree_store_dir)
            .read_events_by_file_names(&self.read_store.local_only_event_files)?;
        merged.extend(local_only);
        // `local_only_event_files` is the filename DIFFERENCE, so the linked and
        // local-only sets are already disjoint; dedup defensively by event id,
        // then sort so error text and tests are deterministic (projections are
        // order-independent, so the sort is never load-bearing for correctness).
        merged.sort_by(|a, b| a.event_id.as_str().cmp(b.event_id.as_str()));
        merged.dedup_by(|a, b| a.event_id == b.event_id);
        Ok(merged)
    }
}

pub(crate) fn resolve_write_validation_store(
    repo: impl AsRef<Path>,
) -> Result<WriteValidationStore> {
    let worktree_store_dir = ShoreStorePaths::resolve(repo.as_ref())?
        .store_dir()
        .to_path_buf();
    Ok(WriteValidationStore {
        read_store: resolve_read_store(repo)?,
        worktree_store_dir,
    })
}

/// The write-landing seam (INV-1): in `CloneLocal` mode the write lands in the
/// clone-local store (`.git/shore`); in `WorktreeLocal` mode it lands in the
/// worktree-local `.shore/data`. Mirrors [`resolve_read_store`]'s mode logic so
/// reads and writes resolve the SAME store in linked mode, closing the
/// read/write asymmetry that otherwise leaves a linked worktree's own captures
/// invisible to its own reads.
///
/// Concurrency safety rests on content-addressed exclusive-create writes plus a
/// regenerable atomic-rename projection (INV-3): there is no store-dir lock in
/// V1, and any future lock must be store-directory scoped (never
/// one-clone-one-writer) so a cross-clone store inherits it.
#[derive(Clone, Debug)]
pub(crate) struct WriteStore {
    store_dir: PathBuf,
    worktree_root: PathBuf,
}

impl WriteStore {
    pub(crate) fn store_dir(&self) -> &Path {
        &self.store_dir
    }

    pub(crate) fn worktree_root(&self) -> &Path {
        &self.worktree_root
    }
}

/// Resolve the write landing for `repo`. See [`WriteStore`]. Deliberately reuses
/// [`resolve_store`] (not a fresh mode computation) so it can never disagree with
/// [`resolve_read_store`] on the mode boundary (INV-1, INV-7): an unlinked repo
/// has no registration → `WorktreeLocal` → the worktree-local `.shore/data`.
pub(crate) fn resolve_write_store(repo: impl AsRef<Path>) -> Result<WriteStore> {
    let paths = ShoreStorePaths::resolve(repo.as_ref())?;
    let resolution = resolve_store(repo.as_ref())?;
    let store_dir = match resolution.mode {
        StoreResolutionMode::WorktreeLocal => paths.store_dir().to_path_buf(),
        StoreResolutionMode::CloneLocal => resolution.store_dir().to_path_buf(),
    };
    Ok(WriteStore {
        store_dir,
        worktree_root: paths.worktree_root().to_path_buf(),
    })
}

/// Prepare the resolved write landing: ensure the store directory layout on the
/// *write* store dir (the clone-local store in linked mode) while keeping the
/// `.git/info/exclude` entries anchored on the worktree root (INV-1). Delegates
/// to the same shared body as `prepare_shore_writer`, so the two never drift on
/// which excludes are written.
pub(crate) fn prepare_write_landing(
    write_store: &WriteStore,
    storage: &LocalStorage,
) -> Result<()> {
    prepare_store_writer_at(
        storage,
        write_store.store_dir(),
        write_store.worktree_root(),
    )
}

pub(crate) fn resolve_store(repo: impl AsRef<Path>) -> Result<StoreResolution> {
    let paths = ShoreStorePaths::resolve(repo.as_ref())?;

    // The opt-out is consulted first: an Ephemeral worktree pins its data to the
    // discardable worktree-local store regardless of any clone-local
    // registration. A Shared (default, including absent-config) mode falls
    // through to today's registration-gated logic UNCHANGED — this consult adds
    // the escape hatch without changing the default placement. A later change
    // removes the registration branch below and gates resolution solely on the
    // store mode (Shared → common-dir, Ephemeral → worktree-local); both gates
    // coexist for now so this stays a no-op under today's default.
    if resolve_store_mode(paths.worktree_root())? == StoreMode::Ephemeral {
        return Ok(StoreResolution {
            mode: StoreResolutionMode::WorktreeLocal,
            store_dir: paths.store_dir().to_path_buf(),
            registration: None,
            manifest: None,
        });
    }

    let Some(registration) = read_store_registration_if_exists(paths.worktree_root())? else {
        return Ok(StoreResolution {
            mode: StoreResolutionMode::WorktreeLocal,
            store_dir: paths.store_dir().to_path_buf(),
            registration: None,
            manifest: None,
        });
    };

    let store_dir = clone_local_store_dir(paths.worktree_root())?;
    let manifest = read_store_manifest(&store_dir)?;
    validate_registration_matches_manifest(&registration, &manifest)?;

    Ok(StoreResolution {
        mode: StoreResolutionMode::CloneLocal,
        store_dir,
        registration: Some(registration),
        manifest: Some(manifest),
    })
}

pub(crate) fn register_clone_local_store(repo: impl AsRef<Path>) -> Result<StoreRegistration> {
    let paths = ShoreStorePaths::resolve(repo.as_ref())?;
    let store_dir = clone_local_store_dir(paths.worktree_root())?;
    let manifest = load_or_create_store_manifest(
        &store_dir,
        StoreGitProvenance {
            common_dir: path_string(&git_common_dir(paths.worktree_root())?, "common-dir")?,
            git_dir: path_string(&git_absolute_git_dir(paths.worktree_root())?, "git-dir")?,
            worktree_root: path_string(paths.worktree_root(), "worktree root")?,
            object_format: git_object_format(paths.worktree_root())?,
        },
    )?;
    let registration = StoreRegistration::clone_local(&manifest);

    ensure_shore_storage_excluded(paths.worktree_root())?;
    let path = store_registration_path(paths.worktree_root());
    let storage = LocalStorage::new(paths.store_dir());
    storage.write_json_atomic(&path, &registration, Durability::Durable)?;
    Ok(registration)
}

pub(crate) fn read_store_registration(repo: impl AsRef<Path>) -> Result<StoreRegistration> {
    let paths = ShoreStorePaths::resolve(repo.as_ref())?;
    read_store_registration_path(&store_registration_path(paths.worktree_root()))
}

fn read_store_registration_if_exists(worktree_root: &Path) -> Result<Option<StoreRegistration>> {
    let path = store_registration_path(worktree_root);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error("read store registration", &path, error)),
    };

    let registration: StoreRegistration = serde_json::from_slice(&bytes)?;
    registration.validate_schema_version()?;
    Ok(Some(registration))
}

fn read_store_registration_path(path: &Path) -> Result<StoreRegistration> {
    let bytes = fs::read(path).map_err(|error| io_error("read store registration", path, error))?;
    let registration: StoreRegistration = serde_json::from_slice(&bytes)?;
    registration.validate_schema_version()?;
    Ok(registration)
}

fn validate_registration_matches_manifest(
    registration: &StoreRegistration,
    manifest: &StoreManifest,
) -> Result<()> {
    if registration.store_ref == manifest.store_id
        && registration.clone_ref == manifest.clone_id
        && registration.repository_family_ref == manifest.repository_family_id
    {
        return Ok(());
    }

    Err(ShoreError::Message(
        "store registration does not match clone-local manifest".to_owned(),
    ))
}

pub(crate) fn clone_local_store_dir(worktree_root: &Path) -> Result<PathBuf> {
    Ok(git_common_dir(worktree_root)?.join("shore"))
}

fn store_registration_path(worktree_root: &Path) -> PathBuf {
    worktree_root.join(".shore/data/store-registration.json")
}

fn path_string(path: &Path, description: &str) -> Result<String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        ShoreError::Message(format!(
            "git {description} path is not utf-8: {}",
            path.display()
        ))
    })
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> ShoreError {
    ShoreError::Message(format!("{action} {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;

    use super::*;
    use crate::git::git_common_dir;
    use crate::model::{SessionId, WorkUnitId};
    use crate::session::event::{
        EventTarget, EventType, ReviewInitializedPayload, ShoreEvent, Writer,
    };
    use crate::session::store::manifest::read_store_manifest;
    use crate::session::store::store_config::write_store_config;
    use crate::session::store::store_init::ShoreStorePaths;

    #[test]
    fn unlinked_repository_resolves_to_worktree_local_store() {
        let repo = GitRepo::new();

        let resolution = resolve_store(repo.path()).unwrap();

        assert_eq!(resolution.mode, StoreResolutionMode::WorktreeLocal);
        assert_eq!(path_file_name(resolution.store_dir()), "data");
        assert_eq!(
            path_file_name(path_parent(resolution.store_dir())),
            ".shore"
        );
        assert_existing_paths_eq(
            path_parent(path_parent(resolution.store_dir())),
            repo.path(),
        );
        assert_eq!(
            ShoreStorePaths::resolve(repo.path()).unwrap().store_dir(),
            resolution.store_dir()
        );
    }

    #[test]
    fn linked_worktree_reads_worktree_local_registration_file() {
        let fixture = LinkedWorktreeFixture::new();

        let created = register_clone_local_store(&fixture.linked_path).unwrap();
        let read = read_store_registration(&fixture.linked_path).unwrap();

        assert_eq!(read, created);
        assert!(
            fixture
                .linked_path
                .join(".shore/data/store-registration.json")
                .is_file()
        );
    }

    #[test]
    fn registration_points_to_clone_local_shared_store() {
        let fixture = LinkedWorktreeFixture::new();

        let registration = register_clone_local_store(&fixture.linked_path).unwrap();
        let resolution = resolve_store(&fixture.linked_path).unwrap();

        let common_dir = git_common_dir(fixture.main.path()).unwrap();
        let expected_store = common_dir.join("shore");
        assert_eq!(resolution.mode, StoreResolutionMode::CloneLocal);
        assert_existing_paths_eq(resolution.store_dir(), &expected_store);

        let manifest = read_store_manifest(&expected_store).unwrap();
        assert_eq!(registration.store_ref, manifest.store_id);
        assert_eq!(registration.clone_ref, manifest.clone_id);
        assert_eq!(
            registration.repository_family_ref,
            manifest.repository_family_id
        );
    }

    #[test]
    fn resolve_read_store_worktree_local_has_no_local_only_events() {
        let repo = GitRepo::new();
        write_event_file(&repo.path().join(".shore/data"), 'a');

        let read_store = resolve_read_store(repo.path()).unwrap();

        assert_eq!(
            read_store.resolution.mode,
            StoreResolutionMode::WorktreeLocal
        );
        assert_eq!(path_file_name(read_store.store_dir()), "data");
        assert!(read_store.local_only_event_files.is_empty());
    }

    #[test]
    fn resolve_read_store_linked_reports_local_events_absent_from_linked_store() {
        let fixture = LinkedWorktreeFixture::new();
        register_clone_local_store(&fixture.linked_path).unwrap();
        let local_name = write_event_file(&fixture.linked_path.join(".shore/data"), 'a');

        let read_store = resolve_read_store(&fixture.linked_path).unwrap();

        assert_eq!(read_store.resolution.mode, StoreResolutionMode::CloneLocal);
        assert_eq!(read_store.local_only_event_files, vec![local_name]);
    }

    #[test]
    fn resolve_read_store_linked_with_synced_events_reports_none() {
        let fixture = LinkedWorktreeFixture::new();
        register_clone_local_store(&fixture.linked_path).unwrap();
        let resolution = resolve_store(&fixture.linked_path).unwrap();
        write_event_file(&fixture.linked_path.join(".shore/data"), 'b');
        write_event_file(resolution.store_dir(), 'b');

        let read_store = resolve_read_store(&fixture.linked_path).unwrap();

        assert_eq!(read_store.resolution.mode, StoreResolutionMode::CloneLocal);
        assert!(read_store.local_only_event_files.is_empty());
    }

    #[test]
    fn resolve_read_store_linked_without_local_events_dir_reports_none() {
        let fixture = LinkedWorktreeFixture::new();
        register_clone_local_store(&fixture.linked_path).unwrap();

        let read_store = resolve_read_store(&fixture.linked_path).unwrap();

        assert_eq!(read_store.resolution.mode, StoreResolutionMode::CloneLocal);
        assert!(read_store.local_only_event_files.is_empty());
    }

    fn write_event_file(store_dir: &Path, fill: char) -> String {
        let name = format!("{}.json", fill.to_string().repeat(64));
        let events_dir = store_dir.join("events");
        fs::create_dir_all(&events_dir).unwrap();
        fs::write(events_dir.join(&name), b"{}").unwrap();
        name
    }

    #[test]
    fn write_validation_events_worktree_local_returns_plain_local_events() {
        let repo = GitRepo::new();
        let shore = repo.path().join(".shore/data");
        let a = record_review_initialized(&shore, "session:a");
        let b = record_review_initialized(&shore, "session:b");

        let store = resolve_write_validation_store(repo.path()).unwrap();
        let events = store.validation_events().unwrap();

        let listed = EventStore::open(&shore).list_events().unwrap();
        assert_eq!(events.len(), listed.len());
        assert_eq!(events.len(), 2);
        assert!(events.iter().any(|event| event.event_id == a.event_id));
        assert!(events.iter().any(|event| event.event_id == b.event_id));
    }

    #[test]
    fn write_validation_events_linked_unions_linked_store_and_unsynced_local() {
        let fixture = LinkedWorktreeFixture::new();
        register_clone_local_store(&fixture.linked_path).unwrap();
        let resolution = resolve_store(&fixture.linked_path).unwrap();

        let a = record_review_initialized(resolution.store_dir(), "session:a");
        let b = record_review_initialized(&fixture.linked_path.join(".shore/data"), "session:b");

        let store = resolve_write_validation_store(&fixture.linked_path).unwrap();
        let events = store.validation_events().unwrap();

        assert!(
            events.iter().any(|event| event.event_id == a.event_id),
            "linked-store event A is in the writer-visible union"
        );
        assert!(
            events.iter().any(|event| event.event_id == b.event_id),
            "unsynced local event B is in the writer-visible union"
        );
    }

    #[test]
    fn write_validation_events_linked_after_full_sync_has_no_duplicates() {
        let fixture = LinkedWorktreeFixture::new();
        register_clone_local_store(&fixture.linked_path).unwrap();
        let resolution = resolve_store(&fixture.linked_path).unwrap();

        // Same event in both stores: the post-`store link` state.
        let a = record_review_initialized(resolution.store_dir(), "session:a");
        record_review_initialized(&fixture.linked_path.join(".shore/data"), "session:a");

        let store = resolve_write_validation_store(&fixture.linked_path).unwrap();
        let events = store.validation_events().unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, a.event_id);
    }

    #[test]
    fn write_validation_events_are_sorted_by_event_id_for_determinism() {
        let fixture = LinkedWorktreeFixture::new();
        register_clone_local_store(&fixture.linked_path).unwrap();
        let resolution = resolve_store(&fixture.linked_path).unwrap();

        record_review_initialized(resolution.store_dir(), "session:a");
        record_review_initialized(&fixture.linked_path.join(".shore/data"), "session:b");
        record_review_initialized(&fixture.linked_path.join(".shore/data"), "session:c");

        let store = resolve_write_validation_store(&fixture.linked_path).unwrap();
        let events = store.validation_events().unwrap();

        let ids: Vec<&str> = events.iter().map(|event| event.event_id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(
            ids, sorted,
            "validation events are sorted ascending by event id"
        );
        assert_eq!(events.len(), 3);
    }

    fn record_review_initialized(store_dir: &Path, session: &str) -> ShoreEvent {
        let event = review_initialized_event_for_session(session);
        EventStore::open(store_dir)
            .record_event_once(&event)
            .unwrap();
        event
    }

    fn review_initialized_event_for_session(session: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewInitialized,
            format!("review_initialized:{session}:work:default"),
            EventTarget::new(SessionId::new(session), WorkUnitId::new("work:default")),
            Writer::shore_local("0.1.0"),
            ReviewInitializedPayload {},
            "2026-05-10T00:00:00Z",
        )
        .expect("event builds")
    }

    #[test]
    fn command_view_uses_opaque_refs_instead_of_raw_paths() {
        let fixture = LinkedWorktreeFixture::new();
        register_clone_local_store(&fixture.linked_path).unwrap();

        let resolution = resolve_store(&fixture.linked_path).unwrap();
        let json = serde_json::to_string(&resolution.command_view()).unwrap();

        assert!(json.contains("\"mode\":\"linked\""));
        assert!(json.contains("\"storeRef\":\"store:random:"));
        assert!(json.contains("\"cloneRef\":\"clone:random:"));
        assert!(json.contains("\"repositoryFamilyRef\":\"clone:random:"));
        assert!(!json.contains(fixture.main.path().to_str().unwrap()));
        assert!(!json.contains(fixture.linked_path.to_str().unwrap()));
        assert!(!json.contains(".git"));
    }

    #[test]
    fn resolve_write_store_unlinked_lands_worktree_local() {
        let repo = GitRepo::new();
        let write = resolve_write_store(repo.path()).unwrap();
        // worktree-local: <root>/.shore/data (same dir ShoreStorePaths resolves).
        assert_eq!(
            ShoreStorePaths::resolve(repo.path()).unwrap().store_dir(),
            write.store_dir()
        );
        assert_existing_paths_eq(write.worktree_root(), repo.path());
    }

    #[test]
    fn resolve_write_store_linked_lands_clone_local() {
        let fixture = LinkedWorktreeFixture::new();
        register_clone_local_store(&fixture.linked_path).unwrap();

        let write = resolve_write_store(&fixture.linked_path).unwrap();

        let expected = git_common_dir(fixture.main.path()).unwrap().join("shore");
        // Linked mode lands in the clone-local store (.git/shore).
        assert_existing_paths_eq(write.store_dir(), &expected);
        // worktree_root is still the (linked) worktree's root, for exclude entries.
        assert_existing_paths_eq(write.worktree_root(), &fixture.linked_path);
    }

    #[test]
    fn resolve_write_store_linked_resolves_same_dir_as_read_store() {
        // The point of the change: in linked mode the write landing and the read
        // store are the SAME directory (closes the read/write asymmetry, INV-1).
        let fixture = LinkedWorktreeFixture::new();
        register_clone_local_store(&fixture.linked_path).unwrap();

        let write = resolve_write_store(&fixture.linked_path).unwrap();
        let read = resolve_read_store(&fixture.linked_path).unwrap();

        assert_eq!(write.store_dir(), read.store_dir());
    }

    #[test]
    fn prepare_write_landing_creates_dirs_on_the_write_store_in_linked_mode() {
        let fixture = LinkedWorktreeFixture::new();
        register_clone_local_store(&fixture.linked_path).unwrap();
        let write = resolve_write_store(&fixture.linked_path).unwrap();
        let storage = LocalStorage::new(write.store_dir());

        prepare_write_landing(&write, &storage).unwrap();

        // events/ + artifacts/* created under the CLONE-LOCAL store, not the worktree-local one.
        assert!(write.store_dir().join("events").is_dir());
        assert!(write.store_dir().join("artifacts/snapshots").is_dir());
        // The worktree-local .shore/data is NOT where new dirs were created.
        let worktree_local = ShoreStorePaths::resolve(&fixture.linked_path).unwrap();
        assert_ne!(write.store_dir(), worktree_local.store_dir());
    }

    #[test]
    fn write_through_steady_state_has_no_divergence_and_union_reduces_to_clone_local() {
        // Facet 1 closure (INV-1/INV-6): after write-through an event lives in the
        // clone-local store with nothing stranded worktree-local, so the read seam
        // reports no local-only divergence and the write-validation union reduces
        // to the clone-local store's events.
        let fixture = LinkedWorktreeFixture::new();
        register_clone_local_store(&fixture.linked_path).unwrap();
        let resolution = resolve_store(&fixture.linked_path).unwrap();
        // Write-through end state: the event lands in the clone-local store only.
        record_review_initialized(resolution.store_dir(), "session:write-through");

        let read = resolve_read_store(&fixture.linked_path).unwrap();
        assert!(
            read.local_only_event_files.is_empty(),
            "no residual worktree-local divergence after write-through"
        );

        let validation = resolve_write_validation_store(&fixture.linked_path).unwrap();
        let union = validation.validation_events().unwrap();
        let direct = EventStore::open(resolution.store_dir())
            .list_events()
            .unwrap();
        assert_eq!(union.len(), direct.len());
        assert_eq!(union.len(), 1, "the union is exactly the clone-local store");
    }

    #[test]
    fn shared_mode_default_is_unchanged_registration_still_decides() {
        // With no store-config (Shared default), an unlinked repo stays
        // WorktreeLocal exactly as today — the bit is consulted but does not flip
        // anything.
        let repo = GitRepo::new();
        let resolution = resolve_store(repo.path()).unwrap();
        assert_eq!(resolution.mode, StoreResolutionMode::WorktreeLocal);
        assert_eq!(
            ShoreStorePaths::resolve(repo.path()).unwrap().store_dir(),
            resolution.store_dir()
        );
    }

    #[test]
    fn shared_mode_linked_still_resolves_clone_local() {
        // A registered (linked) worktree under Shared mode resolves CloneLocal —
        // unchanged.
        let fixture = LinkedWorktreeFixture::new();
        register_clone_local_store(&fixture.linked_path).unwrap();
        write_store_config(&fixture.linked_path, StoreMode::Shared).unwrap();

        let resolution = resolve_store(&fixture.linked_path).unwrap();
        assert_eq!(resolution.mode, StoreResolutionMode::CloneLocal);
    }

    #[test]
    fn ephemeral_mode_forces_worktree_local_even_when_registered() {
        // The escape hatch: an Ephemeral bit pins worktree-local DESPITE a
        // clone-local registration that would otherwise resolve CloneLocal.
        let fixture = LinkedWorktreeFixture::new();
        register_clone_local_store(&fixture.linked_path).unwrap();
        write_store_config(&fixture.linked_path, StoreMode::Ephemeral).unwrap();

        let resolution = resolve_store(&fixture.linked_path).unwrap();
        assert_eq!(resolution.mode, StoreResolutionMode::WorktreeLocal);
        assert_eq!(
            ShoreStorePaths::resolve(&fixture.linked_path)
                .unwrap()
                .store_dir(),
            resolution.store_dir()
        );
    }

    #[test]
    fn ephemeral_mode_pins_write_store_worktree_local() {
        // The write-landing path honors the pin: an Ephemeral worktree writes to
        // .shore/data.
        let fixture = LinkedWorktreeFixture::new();
        register_clone_local_store(&fixture.linked_path).unwrap();
        write_store_config(&fixture.linked_path, StoreMode::Ephemeral).unwrap();

        let write = resolve_write_store(&fixture.linked_path).unwrap();
        let worktree_local = ShoreStorePaths::resolve(&fixture.linked_path).unwrap();
        assert_eq!(write.store_dir(), worktree_local.store_dir());
    }

    #[test]
    fn ephemeral_mode_scopes_read_and_validation_to_worktree_local() {
        // Read + write-validation paths resolve the same worktree-local dir under
        // Ephemeral.
        let fixture = LinkedWorktreeFixture::new();
        register_clone_local_store(&fixture.linked_path).unwrap();
        write_store_config(&fixture.linked_path, StoreMode::Ephemeral).unwrap();

        let read = resolve_read_store(&fixture.linked_path).unwrap();
        assert_eq!(read.resolution.mode, StoreResolutionMode::WorktreeLocal);
        // WorktreeLocal mode reports no local-only divergence by construction.
        assert!(read.local_only_event_files.is_empty());

        let worktree_local = ShoreStorePaths::resolve(&fixture.linked_path).unwrap();
        assert_eq!(read.store_dir(), worktree_local.store_dir());
    }

    struct LinkedWorktreeFixture {
        main: GitRepo,
        _linked_parent: TempDir,
        linked_path: PathBuf,
    }

    impl LinkedWorktreeFixture {
        fn new() -> Self {
            let main = GitRepo::new();
            main.write("README.md", "base\n");
            main.git(["add", "--all"]);
            main.git(["commit", "-m", "base"]);

            let linked_parent = TempDir::new().expect("create linked worktree parent");
            let linked_path = linked_parent.path().join("linked");
            main.git_os([
                OsString::from("worktree"),
                OsString::from("add"),
                OsString::from("-b"),
                OsString::from("linked"),
                linked_path.as_os_str().to_owned(),
            ]);

            Self {
                main,
                _linked_parent: linked_parent,
                linked_path,
            }
        }
    }

    struct GitRepo {
        root: TempDir,
    }

    impl GitRepo {
        fn new() -> Self {
            let root = TempDir::new().expect("create temp git repository directory");
            let repo = Self { root };
            repo.git(["init"]);
            repo.git(["config", "user.name", "Shore Tests"]);
            repo.git(["config", "user.email", "shore-tests@example.com"]);
            repo.git(["config", "commit.gpgsign", "false"]);
            repo
        }

        fn path(&self) -> &Path {
            self.root.path()
        }

        fn write(&self, path: &str, contents: &str) {
            let path = self.root.path().join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, contents).unwrap();
        }

        fn git<I, S>(&self, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<OsStr>,
        {
            run_git(self.root.path(), args);
        }

        fn git_os<I>(&self, args: I)
        where
            I: IntoIterator<Item = OsString>,
        {
            run_git(self.root.path(), args);
        }
    }

    fn run_git<I, S>(cwd: &Path, args: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args = args
            .into_iter()
            .map(|arg| arg.as_ref().to_owned())
            .collect::<Vec<_>>();
        let output = std::process::Command::new("git")
            .args(&args)
            .current_dir(cwd)
            .output()
            .unwrap_or_else(|error| panic!("run git {:?} in {}: {error}", args, cwd.display()));
        assert!(
            output.status.success(),
            "git {:?} failed in {}\nstdout:\n{}\nstderr:\n{}",
            args,
            cwd.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn assert_existing_paths_eq(actual: &Path, expected: &Path) {
        assert_eq!(
            actual.canonicalize().expect("canonicalize actual path"),
            expected.canonicalize().expect("canonicalize expected path")
        );
    }

    fn path_parent(path: &Path) -> &Path {
        path.parent().expect("path has parent")
    }

    fn path_file_name(path: &Path) -> &str {
        path.file_name()
            .and_then(|name| name.to_str())
            .expect("path has utf-8 file name")
    }
}
