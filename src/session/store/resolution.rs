use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Result, ShoreError};
use crate::git::{git_common_dir, git_worktree_list};
use crate::session::event::ShoreEvent;
use crate::session::store::backend::StoreBackend;
use crate::session::store::event_store::EventStore;
use crate::session::store::store_config::{StoreMode, resolve_family_binding, resolve_store_mode};
use crate::session::store::store_init::{
    ShoreStorePaths, prepare_store_writer_at, worktree_local_store_is_populated,
};
use crate::session::store::user_level::{read_family_manifest, user_level_store_dir};
use crate::storage::LocalStorage;

/// A domain-named, path-free label for the single resolved store, reported by
/// `shore store status`. With one store per clone there is no registration to
/// derive opaque clone/family refs from, so those are absent.
const STORE_REF_LOCAL: &str = "local";

/// The resolved store tier, threaded through [`store_resolution_for`] so
/// `command_view()` reports the real tier rather than a hardcoded mode.
#[derive(Clone, Debug)]
pub(crate) enum ResolvedTier {
    /// Discardable worktree-local `.shore/data` (the Ephemeral opt-out).
    Ephemeral,
    /// The clone-local common-dir store (`.git/shore`) — the default.
    CloneLocal,
    /// A user-level family store; carries the opaque refs the wire reports.
    UserLevel {
        family_ref: String,
        clone_ref: String,
    },
}

// No `Eq`/`PartialEq`: no resolution is compared whole (tests compare
// `.store_dir()`), and the `StoreBackend` handle is intentionally not comparable.
#[derive(Clone, Debug)]
pub(crate) struct StoreResolution {
    store_dir: PathBuf,
    backend: StoreBackend,
    resolved_tier: ResolvedTier,
}

impl StoreResolution {
    pub(crate) fn store_dir(&self) -> &Path {
        &self.store_dir
    }

    /// The resolved storage backend handle. Journal/content consumers build their
    /// wrappers from this; the `state.json` projection write and the file-only
    /// maintenance paths keep using `store_dir`.
    pub(crate) fn backend(&self) -> &StoreBackend {
        &self.backend
    }

    pub(crate) fn command_view(&self) -> StoreResolutionView {
        match &self.resolved_tier {
            ResolvedTier::CloneLocal => StoreResolutionView {
                mode: "local",
                store_ref: STORE_REF_LOCAL.to_owned(),
                clone_ref: None,
                repository_family_ref: None,
            },
            ResolvedTier::Ephemeral => StoreResolutionView {
                mode: "ephemeral",
                store_ref: STORE_REF_LOCAL.to_owned(),
                clone_ref: None,
                repository_family_ref: None,
            },
            ResolvedTier::UserLevel {
                family_ref,
                clone_ref,
            } => StoreResolutionView {
                mode: "user-level",
                store_ref: family_ref.clone(),
                clone_ref: Some(clone_ref.clone()),
                repository_family_ref: Some(family_ref.clone()),
            },
        }
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
}

impl ReadStore {
    pub(crate) fn store_dir(&self) -> &Path {
        self.resolution.store_dir()
    }

    pub(crate) fn backend(&self) -> &StoreBackend {
        self.resolution.backend()
    }

    /// Inject a resolved read store over an explicit backend — the test-only seam
    /// that lets a unit test drive a read surface over the injection-only
    /// in-memory backend (which `resolve_read_store` never selects, since the
    /// repo-path resolver always yields the local backend). `store_dir` is the
    /// path the file-only helpers would use; in-memory reads never touch it.
    #[cfg(test)]
    pub(crate) fn for_test(store_dir: PathBuf, backend: StoreBackend) -> Self {
        ReadStore {
            resolution: StoreResolution {
                store_dir,
                backend,
                resolved_tier: ResolvedTier::CloneLocal,
            },
        }
    }
}

/// The read seam: read surfaces resolve their store here. With one default store
/// per clone (shared via the common dir, or worktree-local when Ephemeral), a read
/// opens exactly that store.
pub(crate) fn resolve_read_store(repo: impl AsRef<Path>) -> Result<ReadStore> {
    Ok(ReadStore {
        resolution: resolve_store(repo)?,
    })
}

/// The cheap freshness detector for a repo's event log: the journal's head marker
/// (the event count, computed without reading or decoding any event bytes). The
/// inspector's `/api/freshness` poll reads this instead of re-folding and
/// re-hashing the whole log on every tick; the event-set hash stays the
/// authoritative confirm stamp on the full-read surfaces (`/api/history`,
/// `/api/revisions`).
pub fn event_log_head_marker(repo: impl AsRef<Path>) -> Result<u64> {
    resolve_read_store(repo)?.backend().journal().head_marker()
}

/// A discoverability advisory: when this worktree resolves the clone-local store but a
/// sibling worktree of the same physical clone carries a family binding, tell the user
/// their writes are splitting off from the family store. Best-effort — any
/// worktree-enumeration failure returns `None` (the sibling scan must never turn a
/// resolve into an error). Reads only; never writes.
pub fn family_link_advisory(repo: impl AsRef<Path>) -> Result<Option<String>> {
    let repo = repo.as_ref();
    // Only the unbound clone-local tier can split. A family-resolved worktree is already
    // joined; an ephemeral worktree opted out on purpose.
    if !matches!(resolve_store(repo)?.resolved_tier, ResolvedTier::CloneLocal) {
        return Ok(None);
    }
    // This worktree has no binding (else it would not be clone-local), so scanning every
    // worktree — including self, which contributes `None` — needs no self-skip.
    let Ok(worktrees) = git_worktree_list(repo) else {
        return Ok(None);
    };
    for worktree in worktrees {
        if let Ok(Some(binding)) = resolve_family_binding(&worktree.path) {
            return Ok(Some(family_split_advisory_message(&binding.family_ref)));
        }
    }
    Ok(None)
}

fn family_split_advisory_message(slug: &str) -> String {
    format!(
        "a family store `{slug}` is linked for another worktree of this clone, but this \
         worktree is unlinked and writing to the clone-local store (.git/shore). Run \
         `shore store link {slug}` to join it, or re-link the other worktree once to bind \
         every worktree of this clone."
    )
}

/// The write-validation seam: write surfaces resolve their validation/derivation
/// reads here. With one store, the writer-visible event set is exactly that
/// store's events.
#[derive(Clone, Debug)]
pub(crate) struct WriteValidationStore {
    read_store: ReadStore,
}

impl WriteValidationStore {
    pub(crate) fn backend(&self) -> &StoreBackend {
        self.read_store.backend()
    }

    pub(crate) fn validation_events(&self) -> Result<Vec<ShoreEvent>> {
        EventStore::from_backend(self.backend()).list_events()
    }
}

pub(crate) fn resolve_write_validation_store(
    repo: impl AsRef<Path>,
) -> Result<WriteValidationStore> {
    Ok(WriteValidationStore {
        read_store: resolve_read_store(repo)?,
    })
}

/// The write-landing seam: events, artifacts, and `state.json` are written to the
/// resolved store — the common-dir store shared across the clone (default), or the
/// worktree-local `.shore/data` when the worktree is Ephemeral. Reuses
/// [`resolve_store`] so reads and writes can never disagree on the store.
///
/// Concurrency safety rests on content-addressed exclusive-create writes plus a
/// regenerable atomic-rename projection: there is no store-dir lock, and any future
/// lock must be store-directory scoped (never one-clone-one-writer) so a cross-clone
/// store inherits it.
#[derive(Clone, Debug)]
pub(crate) struct WriteStore {
    store_dir: PathBuf,
    worktree_root: PathBuf,
    backend: StoreBackend,
}

impl WriteStore {
    pub(crate) fn store_dir(&self) -> &Path {
        &self.store_dir
    }

    pub(crate) fn worktree_root(&self) -> &Path {
        &self.worktree_root
    }

    pub(crate) fn backend(&self) -> &StoreBackend {
        &self.backend
    }
}

/// Resolve the write landing for `repo`. See [`WriteStore`]. Reuses [`resolve_store`]
/// so it can never disagree with [`resolve_read_store`] on the store boundary.
pub(crate) fn resolve_write_store(repo: impl AsRef<Path>) -> Result<WriteStore> {
    let paths = ShoreStorePaths::resolve(repo.as_ref())?;
    let resolution = resolve_store(repo.as_ref())?;
    Ok(WriteStore {
        store_dir: resolution.store_dir().to_path_buf(),
        worktree_root: paths.worktree_root().to_path_buf(),
        backend: resolution.backend().clone(),
    })
}

/// Prepare the resolved write landing: ensure the store directory layout on the
/// *write* store dir while keeping the generated `.shore/.gitignore` anchored on
/// the worktree root. Delegates to the shared `prepare_store_writer_at` body, so
/// write workflows can never drift on which paths are covered.
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

    // Binding-configuration validation is UNCONDITIONAL: a committed or half
    // binding is a hard error even when ephemeral mode or the legacy guard outranks
    // the user-level arm below. The resolved binding is only *used* by the
    // user-level arm. `resolve_family_binding` reads the same two config documents
    // `resolve_store_mode` reads — no `shore_home` access here.
    let binding = resolve_family_binding(paths.worktree_root())?;

    // Precedence (top-to-bottom decision table): ephemeral opt-out pins the
    // discardable worktree-local store; then the legacy-populated guard; then the
    // user-level family opt-in; then the clone-local common-dir default.
    if resolve_store_mode(paths.worktree_root())? == StoreMode::Ephemeral {
        return store_resolution_for(paths.store_dir().to_path_buf(), ResolvedTier::Ephemeral);
    }

    // A non-ephemeral worktree that still carries a populated worktree-local
    // `.shore/data/` store predates the shared-store default. Direct the user to
    // `shore store migrate` rather than silently reading an empty common-dir store
    // and orphaning the history. This guard lives HERE (resolve_store), not in
    // ShoreStorePaths::resolve, so the `shore store migrate` command — which reads
    // its source via the raw ShoreStorePaths::resolve — is never blocked by it.
    if worktree_local_store_is_populated(paths.store_dir()) {
        return Err(ShoreError::Message(
            "a worktree-local .shore/data/ review store from before the shared-store default \
             was detected. Reads and writes now use the shared store under .git/shore, so this \
             worktree-local store is no longer read automatically. Complete the switch in one \
             command with `shore store migrate --retire-source`, which copies its events and \
             artifacts into the shared store, independently verifies the fold, and then deletes \
             .shore/data/. Or take it in two steps: (1) run `shore store migrate` to copy \
             non-destructively, leaving .shore/data/ in place so you can verify the result \
             first; then (2) delete the .shore/data/ directory. This message keeps appearing \
             until .shore/data/ is removed, by design, so the original store is never discarded \
             before the migration is confirmed. (If this worktree is meant to stay isolated and \
             discardable instead, run `shore store mode ephemeral` and its .shore/data/ store is \
             used as-is.)"
                .to_owned(),
        ));
    }

    // User-level opt-in: a local-only family binding promotes this clone to the
    // family tier. A binding whose family store was forgotten (`shore store forget`,
    // or the dir hand-deleted) resolves to no manifest — a hard, actionable error,
    // never a silent re-create or clone-local fallback.
    if let Some(binding) = binding {
        let family_dir = user_level_store_dir(&binding.family_ref)?;
        if read_family_manifest(&family_dir)?.is_none() {
            return Err(ShoreError::Message(format!(
                "this clone is linked to the user-level family store `{}`, but that store no longer \
                 exists at {} (it was forgotten, or the directory was removed). Re-create and \
                 re-link it with `shore store link {}`, or detach this clone with \
                 `shore store unlink`.",
                binding.family_ref,
                family_dir.display(),
                binding.family_ref,
            )));
        }
        return store_resolution_for(
            family_dir,
            ResolvedTier::UserLevel {
                family_ref: binding.family_ref,
                clone_ref: binding.clone_ref,
            },
        );
    }

    // The common-dir store is the default; its layout is created on first write,
    // so a read before any write resolves the dir without requiring it to exist.
    store_resolution_for(
        clone_local_store_dir(paths.worktree_root())?,
        ResolvedTier::CloneLocal,
    )
}

/// Pair a resolved store directory with the selected backend handle and its tier.
/// Both `resolve_store` return paths route through here so the `SHORE_BACKEND`
/// selection is applied in exactly one place.
fn store_resolution_for(store_dir: PathBuf, tier: ResolvedTier) -> Result<StoreResolution> {
    let backend = select_backend(store_dir.clone())?;
    Ok(StoreResolution {
        store_dir,
        backend,
        resolved_tier: tier,
    })
}

/// Environment variable that selects the durable-storage backend. Unset is the
/// `local` default; `memory` is rejected (it is in-process injection only); any
/// other value is a hard error.
const STORE_BACKEND_ENV: &str = "SHORE_BACKEND";

/// Choose the backend for `store_dir` from the `SHORE_BACKEND` environment.
/// Mechanism mirrors `SHORE_PERF`; the loud unknown-value posture mirrors
/// `StoreMode`.
fn select_backend(store_dir: PathBuf) -> Result<StoreBackend> {
    classify_backend(std::env::var(STORE_BACKEND_ENV), store_dir)
}

/// Pure classifier for [`select_backend`], taking the raw `std::env::var`
/// result so it can be unit-tested without mutating process-global state.
/// Unset or `local` → the file backend; `memory` and any unknown value are
/// loud, actionable errors.
fn classify_backend(
    value: std::result::Result<String, std::env::VarError>,
    store_dir: PathBuf,
) -> Result<StoreBackend> {
    match value.as_deref() {
        Ok("local") | Err(std::env::VarError::NotPresent) => Ok(StoreBackend::Local(store_dir)),
        Ok("memory") => Err(ShoreError::Message(
            "the in-memory store backend is not selectable via SHORE_BACKEND; it is reachable only \
             through in-process injection (a spawned `shore` child would otherwise inherit an empty, \
             lost-on-exit store). Unset SHORE_BACKEND or set it to `local`."
                .to_owned(),
        )),
        Ok(other) => Err(ShoreError::Message(format!(
            "unknown SHORE_BACKEND value `{other}`; the only supported value is `local`, which is \
             also the default when SHORE_BACKEND is unset"
        ))),
        Err(std::env::VarError::NotUnicode(_)) => Err(ShoreError::Message(
            "SHORE_BACKEND is set to a non-UTF-8 value; the only supported value is `local`, which \
             is also the default when SHORE_BACKEND is unset"
                .to_owned(),
        )),
    }
}

pub(crate) fn clone_local_store_dir(worktree_root: &Path) -> Result<PathBuf> {
    Ok(git_common_dir(worktree_root)?.join("shore"))
}

#[cfg(test)]
mod tests {
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;

    use super::*;
    use crate::git::git_common_dir;
    use crate::model::JournalId;
    use crate::session::event::{
        EventTarget, EventType, ReviewInitializedPayload, ShoreEvent, Writer,
    };
    use crate::session::store::store_config::write_store_config;
    use crate::session::store::store_init::ShoreStorePaths;

    #[test]
    fn fresh_unregistered_worktree_resolves_common_dir_by_default() {
        // The shared-store default: an unregistered repo resolves the common-dir
        // store (.git/shore), not the worktree-local .shore/data. No `store link`.
        let repo = GitRepo::new();
        let resolution = resolve_store(repo.path()).unwrap();

        let expected = git_common_dir(repo.path()).unwrap().join("shore");
        assert_existing_paths_eq(resolution.store_dir(), &expected);
        // The worktree-local .shore/data is NOT the resolved store anymore.
        assert_ne!(
            resolution.store_dir(),
            ShoreStorePaths::resolve(repo.path()).unwrap().store_dir()
        );
    }

    #[test]
    fn fresh_unregistered_worktree_read_write_and_validation_all_resolve_common_dir() {
        let repo = GitRepo::new();
        let expected = git_common_dir(repo.path()).unwrap().join("shore");

        let read = resolve_read_store(repo.path()).unwrap();
        assert_existing_paths_eq(read.store_dir(), &expected);

        let write = resolve_write_store(repo.path()).unwrap();
        assert_existing_paths_eq(write.store_dir(), &expected);

        // The write-validation seam resolves the same store; no divergence in
        // the single-store world.
        let validation = resolve_write_validation_store(repo.path()).unwrap();
        let _ = validation.validation_events().unwrap();
    }

    #[test]
    fn linked_worktree_resolves_shared_common_dir_without_registration() {
        // A real linked worktree resolves the same common-dir store as main, with
        // no registration step — sharing is the default.
        let fixture = LinkedWorktreeFixture::new();
        let expected = git_common_dir(fixture.main.path()).unwrap().join("shore");

        let main = resolve_store(fixture.main.path()).unwrap();
        let linked = resolve_store(&fixture.linked_path).unwrap();
        assert_existing_paths_eq(main.store_dir(), &expected);
        assert_existing_paths_eq(linked.store_dir(), &expected);
        assert_eq!(main.store_dir(), linked.store_dir());
    }

    #[test]
    fn ephemeral_mode_resolves_worktree_local_after_flip() {
        // The surviving opt-out: an Ephemeral worktree still resolves the
        // discardable worktree-local .shore/data.
        let repo = GitRepo::new();
        write_store_config(repo.path(), StoreMode::Ephemeral).unwrap();

        let resolution = resolve_store(repo.path()).unwrap();
        assert_eq!(
            resolution.store_dir(),
            ShoreStorePaths::resolve(repo.path()).unwrap().store_dir()
        );
        assert_eq!(path_file_name(resolution.store_dir()), "data");
    }

    #[test]
    fn ephemeral_mode_pins_read_write_and_validation_to_worktree_local() {
        let repo = GitRepo::new();
        write_store_config(repo.path(), StoreMode::Ephemeral).unwrap();
        let worktree_local = ShoreStorePaths::resolve(repo.path()).unwrap();

        let read = resolve_read_store(repo.path()).unwrap();
        assert_eq!(read.store_dir(), worktree_local.store_dir());
        let write = resolve_write_store(repo.path()).unwrap();
        assert_eq!(write.store_dir(), worktree_local.store_dir());
    }

    #[test]
    fn resolve_store_ignores_a_leftover_registration_file_after_flip() {
        // A residual store-registration.json no longer changes resolution — the
        // bit, not the registration, decides.
        let repo = GitRepo::new();
        let shore = repo.path().join(".shore/data");
        fs::create_dir_all(&shore).unwrap();
        fs::write(shore.join("store-registration.json"), "{}").unwrap();

        let resolution = resolve_store(repo.path()).unwrap();
        let expected = git_common_dir(repo.path()).unwrap().join("shore");
        assert_existing_paths_eq(resolution.store_dir(), &expected);
    }

    #[test]
    fn legacy_worktree_local_store_after_flip_returns_migrate_hint() {
        // After the flip the default store is .git/shore, so a populated
        // worktree-local .shore/data/ is a pre-flip store that must be migrated —
        // never silently ignored in favor of an empty common-dir store. The guard is
        // on resolve_store, NOT on ShoreStorePaths::resolve (which `shore store
        // migrate` uses to read the source — see the raw-resolution test below).
        let repo = GitRepo::new();
        fs::create_dir_all(repo.path().join(".shore/data/events")).unwrap();
        fs::write(repo.path().join(".shore/data/events/aaaa.json"), "{}").unwrap();

        let err = resolve_store(repo.path())
            .expect_err("a populated worktree-local store after the flip must be a loud error");
        let message = err.to_string();
        assert!(
            message.contains("store migrate"),
            "names the fix (`shore store migrate`); got: {message}"
        );
        assert!(
            message.contains("--retire-source"),
            "names the one-command completion; got: {message}"
        );
        assert!(
            message.contains(".shore/data"),
            "names the legacy worktree-local store; got: {message}"
        );
    }

    #[test]
    fn raw_path_resolution_does_not_trip_the_legacy_guard() {
        // The escape valve for `shore store migrate`: ShoreStorePaths::resolve reads
        // a nested worktree-local store without firing the migrate guard, so
        // migration can read its source even after the flip.
        let repo = GitRepo::new();
        fs::create_dir_all(repo.path().join(".shore/data/events")).unwrap();
        fs::write(repo.path().join(".shore/data/events/aaaa.json"), "{}").unwrap();
        ShoreStorePaths::resolve(repo.path())
            .expect("raw path resolution of a nested store is unblocked (migration uses this)");
    }

    #[test]
    fn ephemeral_worktree_with_local_store_does_not_trip_the_legacy_guard() {
        // An Ephemeral worktree legitimately keeps .shore/data; resolve_store must
        // resolve it, not error with the migrate hint.
        let repo = GitRepo::new();
        write_store_config(repo.path(), StoreMode::Ephemeral).unwrap();
        fs::create_dir_all(repo.path().join(".shore/data/events")).unwrap();
        fs::write(repo.path().join(".shore/data/events/aaaa.json"), "{}").unwrap();
        let resolution =
            resolve_store(repo.path()).expect("ephemeral resolves its worktree-local store");
        assert_eq!(path_file_name(resolution.store_dir()), "data");
    }

    #[test]
    fn read_store_resolves_the_single_common_dir_store() {
        // The union is gone: reads open exactly one store.
        let repo = GitRepo::new();
        let read = resolve_read_store(repo.path()).unwrap();
        let expected = git_common_dir(repo.path()).unwrap().join("shore");
        assert_existing_paths_eq(read.store_dir(), &expected);
    }

    #[test]
    fn event_log_head_marker_equals_the_event_count() {
        // The cheap marker matches the count a full read would report — without
        // doing the full read.
        let repo = GitRepo::new();
        let store_dir = git_common_dir(repo.path()).unwrap().join("shore");
        record_review_initialized(&store_dir, "session:a");
        record_review_initialized(&store_dir, "session:b");
        record_review_initialized(&store_dir, "session:c");

        let marker = event_log_head_marker(repo.path()).unwrap();
        let direct = EventStore::open(&store_dir).list_events().unwrap().len() as u64;
        assert_eq!(marker, direct);
        assert_eq!(marker, 3);
    }

    #[test]
    fn event_log_head_marker_is_zero_for_a_fresh_repo() {
        // A read before any write resolves the (not-yet-created) store and marks
        // zero, never erroring on the missing event directory.
        let repo = GitRepo::new();
        assert_eq!(event_log_head_marker(repo.path()).unwrap(), 0);
    }

    #[test]
    fn write_validation_events_are_exactly_the_single_store_events() {
        // The union collapsed: validation events == the resolved store's events.
        let repo = GitRepo::new();
        let store_dir = git_common_dir(repo.path()).unwrap().join("shore");
        record_review_initialized(&store_dir, "session:a");
        record_review_initialized(&store_dir, "session:b");

        let validation = resolve_write_validation_store(repo.path()).unwrap();
        let events = validation.validation_events().unwrap();
        let direct = EventStore::open(&store_dir).list_events().unwrap();
        assert_eq!(events.len(), direct.len());
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn command_view_reports_the_single_store_without_registration_refs() {
        // No more "linked" mode / clone/repository-family refs — one store.
        let repo = GitRepo::new();
        let resolution = resolve_store(repo.path()).unwrap();
        let json = serde_json::to_string(&resolution.command_view()).unwrap();
        assert!(!json.contains("\"cloneRef\""));
        assert!(!json.contains("\"repositoryFamilyRef\""));
        assert!(json.contains("\"mode\":\"local\""));
    }

    #[test]
    fn write_and_read_resolve_the_same_store() {
        let repo = GitRepo::new();
        let write = resolve_write_store(repo.path()).unwrap();
        let read = resolve_read_store(repo.path()).unwrap();
        assert_eq!(write.store_dir(), read.store_dir());
    }

    #[test]
    fn command_view_maps_clone_local_tier_to_local_mode() {
        // The existing wire shape is unchanged: mode "local", store_ref "local",
        // no clone/family refs.
        let resolution =
            store_resolution_for(PathBuf::from("/tmp/cl"), ResolvedTier::CloneLocal).unwrap();
        let view = resolution.command_view();
        assert_eq!(view.mode, "local");
        assert_eq!(view.store_ref, "local");
        assert!(view.clone_ref.is_none());
        assert!(view.repository_family_ref.is_none());
    }

    #[test]
    fn command_view_maps_ephemeral_tier_to_ephemeral_mode() {
        // Behavior change: an ephemeral resolution now reports "ephemeral", not the
        // old hardcoded "local".
        let resolution =
            store_resolution_for(PathBuf::from("/tmp/eph"), ResolvedTier::Ephemeral).unwrap();
        let view = resolution.command_view();
        assert_eq!(view.mode, "ephemeral");
        assert_eq!(view.store_ref, "local");
        assert!(view.clone_ref.is_none());
        assert!(view.repository_family_ref.is_none());
    }

    #[test]
    fn command_view_maps_user_level_tier_to_family_refs() {
        let resolution = store_resolution_for(
            PathBuf::from("/tmp/fam"),
            ResolvedTier::UserLevel {
                family_ref: "acme-web".to_owned(),
                clone_ref: "0123abcd4567ef89".to_owned(),
            },
        )
        .unwrap();
        let view = resolution.command_view();
        assert_eq!(view.mode, "user-level");
        assert_eq!(view.store_ref, "acme-web");
        assert_eq!(view.repository_family_ref.as_deref(), Some("acme-web"));
        assert_eq!(view.clone_ref.as_deref(), Some("0123abcd4567ef89"));
    }

    #[test]
    fn user_level_binding_write_and_read_resolve_the_same_family_store() {
        use crate::session::store::store_config::set_family_binding_for_repo;
        use crate::session::store::user_level::{
            ensure_family_store_scaffold, user_level_store_dir,
        };

        let repo = GitRepo::new();
        let home = TempDir::new().unwrap();
        // SAFETY: single-threaded test; nextest isolates each test in its own
        // process, and SHORE_HOME is the documented hermetic seam (keys/home.rs).
        unsafe {
            std::env::set_var("SHORE_HOME", home.path());
        }

        let slug = "acme-web";
        let family_dir = user_level_store_dir(slug).unwrap();
        ensure_family_store_scaffold(&family_dir, slug, &[]).unwrap();
        set_family_binding_for_repo(repo.path(), slug, "0123abcd4567ef89").unwrap();

        let write = resolve_write_store(repo.path()).unwrap();
        let read = resolve_read_store(repo.path()).unwrap();
        unsafe {
            std::env::remove_var("SHORE_HOME");
        }

        assert_eq!(write.store_dir(), read.store_dir());
        assert_existing_paths_eq(write.store_dir(), &family_dir);
    }

    #[test]
    fn a_worktree_of_a_linked_clone_resolves_the_family_store() {
        use crate::session::store::store_config::write_common_dir_binding;
        use crate::session::store::user_level::{
            ensure_family_store_scaffold, user_level_store_dir,
        };

        let fixture = LinkedWorktreeFixture::new();
        let home = TempDir::new().unwrap();
        // SAFETY: single-threaded test; nextest isolates each test in its own process.
        unsafe {
            std::env::set_var("SHORE_HOME", home.path());
        }

        let slug = "fam";
        let family_dir = user_level_store_dir(slug).unwrap();
        ensure_family_store_scaffold(&family_dir, slug, &[]).unwrap();
        // The binding lives in the shared common dir — write it once (this is what
        // `store link` on the main checkout does).
        let common = git_common_dir(fixture.main.path()).unwrap();
        write_common_dir_binding(&common, slug, "0123abcd4567ef89").unwrap();

        // Main AND the added worktree both resolve the family store (heal).
        let main_read = resolve_read_store(fixture.main.path()).unwrap();
        let wt_read = resolve_read_store(&fixture.linked_path).unwrap();
        let wt_write = resolve_write_store(&fixture.linked_path).unwrap();
        let wt_validation = resolve_write_validation_store(&fixture.linked_path).unwrap();
        let _ = wt_validation.validation_events().unwrap();
        unsafe {
            std::env::remove_var("SHORE_HOME");
        }

        assert_existing_paths_eq(main_read.store_dir(), &family_dir);
        assert_existing_paths_eq(wt_read.store_dir(), &family_dir);
        assert_existing_paths_eq(wt_write.store_dir(), &family_dir);
        // Regression guard: the worktree must NOT resolve the clone-local .git/shore.
        assert_ne!(
            wt_read.store_dir(),
            git_common_dir(fixture.main.path()).unwrap().join("shore")
        );
    }

    #[test]
    fn an_ephemeral_worktree_still_escapes_even_with_a_common_dir_binding() {
        use crate::session::store::store_config::write_common_dir_binding;

        // A common-dir binding is present, but this worktree opted out (ephemeral,
        // per-worktree): arm 1 still wins, so the discardable .shore/data is used.
        let repo = GitRepo::new();
        write_store_config(repo.path(), StoreMode::Ephemeral).unwrap();
        let common = git_common_dir(repo.path()).unwrap();
        write_common_dir_binding(&common, "fam", "0123abcd4567ef89").unwrap();

        let resolution = resolve_store(repo.path()).unwrap();
        assert_eq!(path_file_name(resolution.store_dir()), "data");
    }

    #[test]
    fn advisory_fires_for_an_unbound_worktree_of_a_legacy_linked_clone() {
        let fixture = LinkedWorktreeFixture::new();
        // Main carries a LEGACY per-worktree binding; the linked worktree is unbound.
        fs::create_dir_all(fixture.main.path().join(".shore")).unwrap();
        fs::write(
            fixture.main.path().join(".shore/store.local.json"),
            r#"{"schema":"shore.store-config","version":1,"mode":"shared","familyRef":"shoreline","cloneRef":"deadbeefdeadbeef"}"#,
        )
        .unwrap();

        let advisory = family_link_advisory(&fixture.linked_path)
            .unwrap()
            .expect("advisory fires");
        assert!(
            advisory.contains("shoreline"),
            "names the family: {advisory}"
        );
        assert!(
            advisory.contains("shore store link"),
            "actionable: {advisory}"
        );
    }

    #[test]
    fn advisory_is_silent_for_a_fresh_unlinked_clone() {
        let fixture = LinkedWorktreeFixture::new();
        assert!(
            family_link_advisory(&fixture.linked_path)
                .unwrap()
                .is_none()
        );
        assert!(family_link_advisory(fixture.main.path()).unwrap().is_none());
    }

    #[test]
    fn advisory_is_silent_for_an_ephemeral_worktree() {
        let repo = GitRepo::new();
        write_store_config(repo.path(), StoreMode::Ephemeral).unwrap();
        assert!(family_link_advisory(repo.path()).unwrap().is_none());
    }

    #[test]
    fn advisory_is_silent_when_this_worktree_already_resolves_the_family() {
        use crate::session::store::store_config::write_common_dir_binding;
        use crate::session::store::user_level::{
            ensure_family_store_scaffold, user_level_store_dir,
        };

        let repo = GitRepo::new();
        let home = TempDir::new().unwrap();
        unsafe {
            std::env::set_var("SHORE_HOME", home.path());
        }
        let slug = "fam";
        ensure_family_store_scaffold(&user_level_store_dir(slug).unwrap(), slug, &[]).unwrap();
        write_common_dir_binding(
            &git_common_dir(repo.path()).unwrap(),
            slug,
            "0123abcd4567ef89",
        )
        .unwrap();

        let advisory = family_link_advisory(repo.path()).unwrap();
        unsafe {
            std::env::remove_var("SHORE_HOME");
        }
        assert!(
            advisory.is_none(),
            "a family-resolved worktree is not advised"
        );
    }

    #[test]
    fn ephemeral_mode_outranks_a_family_binding() {
        // A local file carrying BOTH the ephemeral opt-out and a (valid, local)
        // binding resolves ephemeral: the binding is *validated* unconditionally
        // but only *used* by the user-level arm, which the ephemeral arm outranks —
        // so `shore_home` is never consulted here.
        let repo = GitRepo::new();
        fs::create_dir_all(repo.path().join(".shore")).unwrap();
        fs::write(
            repo.path().join(".shore/store.local.json"),
            r#"{"schema":"shore.store-config","version":1,"mode":"ephemeral","familyRef":"acme-web","cloneRef":"0123abcd4567ef89"}"#,
        )
        .unwrap();
        let resolution = resolve_store(repo.path()).unwrap();
        assert_eq!(path_file_name(resolution.store_dir()), "data");
    }

    #[test]
    fn a_committed_family_binding_hard_errors_even_under_ephemeral_mode() {
        // Binding validation is unconditional. A committed binding must fail loudly
        // even when the ephemeral arm would otherwise short-circuit resolution before
        // the user-level arm — the error exists to stop the binding being committed
        // at all, not merely to stop it resolving.
        let repo = GitRepo::new();
        fs::create_dir_all(repo.path().join(".shore")).unwrap();
        fs::write(
            repo.path().join(".shore/store.json"),
            r#"{"schema":"shore.store-config","version":1,"mode":"shared","familyRef":"acme-web","cloneRef":"0123abcd4567ef89"}"#,
        )
        .unwrap();
        fs::write(
            repo.path().join(".shore/store.local.json"),
            r#"{"schema":"shore.store-config","version":1,"mode":"ephemeral"}"#,
        )
        .unwrap();
        let err = resolve_store(repo.path())
            .expect_err("a committed binding is a hard error regardless of mode");
        assert!(
            err.to_string().contains("store.json"),
            "names the committed file: {err}"
        );
    }

    #[test]
    fn legacy_populated_store_outranks_a_family_binding() {
        // A populated worktree-local .shore/data AND a binding: the legacy migrate
        // guard fires before the user-level arm.
        let repo = GitRepo::new();
        fs::create_dir_all(repo.path().join(".shore/data/events")).unwrap();
        fs::write(repo.path().join(".shore/data/events/aaaa.json"), "{}").unwrap();
        fs::write(
            repo.path().join(".shore/store.local.json"),
            r#"{"schema":"shore.store-config","version":1,"mode":"shared","familyRef":"acme-web","cloneRef":"0123abcd4567ef89"}"#,
        )
        .unwrap();
        let err = resolve_store(repo.path())
            .expect_err("the legacy guard fires before the user-level arm");
        assert!(err.to_string().contains("store migrate"), "got: {err}");
    }

    #[test]
    fn a_dangling_family_binding_is_a_hard_error_naming_both_fixes() {
        let repo = GitRepo::new();
        let home = TempDir::new().unwrap();
        unsafe {
            std::env::set_var("SHORE_HOME", home.path());
        }
        // Bind the clone but never scaffold the family store (no family.json).
        fs::create_dir_all(repo.path().join(".shore")).unwrap();
        fs::write(
            repo.path().join(".shore/store.local.json"),
            r#"{"schema":"shore.store-config","version":1,"mode":"shared","familyRef":"acme-web","cloneRef":"0123abcd4567ef89"}"#,
        )
        .unwrap();
        let result = resolve_store(repo.path());
        unsafe {
            std::env::remove_var("SHORE_HOME");
        }
        let message = result
            .expect_err("a dangling family_ref is a hard error")
            .to_string();
        assert!(
            message.contains("shore store link"),
            "names the re-link fix: {message}"
        );
        assert!(
            message.contains("unlink"),
            "names the unlink fix: {message}"
        );
        assert!(
            message.contains("acme-web"),
            "names the forgotten family: {message}"
        );
    }

    #[test]
    fn classify_backend_defaults_to_local_when_unset_or_local() {
        // Unset → the default file backend, wrapping the resolved dir.
        let dir = PathBuf::from("/tmp/shore-store");
        let backend = classify_backend(Err(std::env::VarError::NotPresent), dir.clone()).unwrap();
        assert_eq!(backend_dir(&backend), dir.as_path());
        // An explicit `local` is the same default.
        let backend = classify_backend(Ok("local".to_owned()), dir.clone()).unwrap();
        assert_eq!(backend_dir(&backend), dir.as_path());
    }

    #[test]
    fn classify_backend_rejects_memory_as_injection_only() {
        // `memory` must never be reachable through the env var: a spawned child
        // would inherit an empty, lost-on-exit store. It is in-process injection
        // only.
        let message = classify_backend(Ok("memory".to_owned()), PathBuf::from("/tmp/store"))
            .expect_err("memory is not env-selectable")
            .to_string();
        assert!(
            message.contains("SHORE_BACKEND"),
            "names the env var: {message}"
        );
        assert!(
            message.contains("injection"),
            "explains it is injection-only: {message}"
        );
    }

    #[test]
    fn classify_backend_hard_errors_on_an_unknown_value() {
        // An unrecognized value is a loud error, never a silent fallback.
        let message = classify_backend(Ok("ndjson".to_owned()), PathBuf::from("/tmp/store"))
            .expect_err("an unknown backend value is rejected")
            .to_string();
        assert!(
            message.contains("ndjson"),
            "names the offending value: {message}"
        );
        assert!(
            message.contains("local"),
            "names the supported value: {message}"
        );
    }

    #[test]
    fn read_write_and_validation_resolve_the_same_local_backend() {
        // The handle is carried on every resolution and read/write/validation all
        // agree on it, so a future backend choice can never split mid-operation.
        let repo = GitRepo::new();
        let read = resolve_read_store(repo.path()).unwrap();
        let write = resolve_write_store(repo.path()).unwrap();
        let validation = resolve_write_validation_store(repo.path()).unwrap();

        assert!(matches!(read.backend(), StoreBackend::Local(_)));
        assert!(matches!(write.backend(), StoreBackend::Local(_)));
        assert!(matches!(validation.backend(), StoreBackend::Local(_)));
        assert_eq!(backend_dir(read.backend()), backend_dir(write.backend()));
        assert_eq!(
            backend_dir(read.backend()),
            backend_dir(validation.backend())
        );
        // DD-consistent for local: the handle wraps the resolved store dir.
        assert_eq!(backend_dir(read.backend()), read.store_dir());
    }

    #[test]
    fn select_backend_reads_the_environment_and_defaults_to_local() {
        // Exercises the real env read (not just the pure classifier): with
        // SHORE_BACKEND unset — the normal test/CI environment — the selector
        // resolves the file backend at the given dir. This deliberately does not
        // mutate SHORE_BACKEND: it is read by every resolve, so setting it here
        // would poison concurrent resolves in a shared-process test runner. The
        // reject-on-unknown and reject-on-memory paths are covered by the pure
        // `classify_backend` tests above.
        let dir = PathBuf::from("/tmp/shore-store");
        let backend = select_backend(dir.clone()).unwrap();
        assert_eq!(backend_dir(&backend), dir.as_path());
    }

    fn backend_dir(backend: &StoreBackend) -> &Path {
        match backend {
            StoreBackend::Local(dir) => dir.as_path(),
            StoreBackend::Memory(_) => unreachable!("the selector never yields the memory backend"),
        }
    }

    #[test]
    fn prepare_write_landing_creates_dirs_on_the_common_dir_store() {
        let repo = GitRepo::new();
        let write = resolve_write_store(repo.path()).unwrap();
        let storage = LocalStorage::new(write.store_dir());

        prepare_write_landing(&write, &storage).unwrap();

        assert!(write.store_dir().join("events").is_dir());
        assert!(write.store_dir().join("artifacts/objects").is_dir());
        // The common-dir store, not the worktree-local one.
        let worktree_local = ShoreStorePaths::resolve(repo.path()).unwrap();
        assert_ne!(write.store_dir(), worktree_local.store_dir());
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
            EventTarget::for_journal(JournalId::new(session)),
            Writer::shore_local("0.1.0"),
            ReviewInitializedPayload {},
            "2026-05-10T00:00:00Z",
        )
        .expect("event builds")
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

    /// Compare two paths for filesystem identity, tolerating a not-yet-created
    /// leaf: canonicalize the deepest existing ancestor (so macOS `/var` →
    /// `/private/var` symlinks normalize) and re-append the rest. The common-dir
    /// store (`.git/shore`) does not exist until first write, but its parent does.
    fn assert_existing_paths_eq(actual: &Path, expected: &Path) {
        fn normalize(path: &Path) -> PathBuf {
            let mut ancestor = path.to_path_buf();
            let mut tail: Vec<std::ffi::OsString> = Vec::new();
            loop {
                if ancestor.exists() {
                    let mut base = ancestor.canonicalize().expect("ancestor canonicalizes");
                    for part in tail.iter().rev() {
                        base.push(part);
                    }
                    return base;
                }
                match (ancestor.file_name(), ancestor.parent()) {
                    (Some(name), Some(parent)) => {
                        tail.push(name.to_owned());
                        ancestor = parent.to_path_buf();
                    }
                    _ => return path.to_path_buf(),
                }
            }
        }
        assert_eq!(normalize(actual), normalize(expected));
    }

    fn path_file_name(path: &Path) -> &str {
        path.file_name()
            .and_then(|name| name.to_str())
            .expect("path has utf-8 file name")
    }
}
