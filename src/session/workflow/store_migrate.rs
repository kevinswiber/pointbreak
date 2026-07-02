//! One-command store upgrade: nest a flat `.shore/` store under `.shore/data/`
//! and rewrite every event's writer fields in place.
//!
//! Takes a pre-relocation **flat** store (events/artifacts/state.json directly
//! under `.shore/`) and nests its store entries under `.shore/data/`, removes
//! the flat era's wholesale `.shore/` line from `.git/info/exclude` (generating
//! the committed `.shore/.gitignore` instead), then runs the per-event writer
//! migration over every event. The `.shore/` directory itself stays (it now
//! also holds committed config); only the store entries move.
//!
//! Crash-safety: `nest_flat_store` copies each entry into `.shore/data/` first
//! and removes the flat originals only after every copy succeeds. A crash before
//! the removals leaves both a flat and a nested store — caught as a conflict by
//! the resolve guard and refused on re-run — so the worst case is redundant flat
//! copies the operator removes, never data loss.
//!
//! Owner-run via `examples/migrate-store.rs` / `just migrate-store`; NOT a
//! shipped `shore` subcommand.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Result, ShoreError};
use crate::git::{git_info_exclude_path, git_worktree_root};
use crate::session::store::{
    EventMigrateOutcome, EventStore, FLAT_STORE_MARKERS, StoreLayout, detect_store_layout,
    ensure_shore_gitignore, migrate_event_file,
};

/// The flat store entries that move into `.shore/data/` — the same set the
/// resolve guard keys on (`FLAT_STORE_MARKERS`), so detection and relocation can
/// never diverge. Committed config (`delegates.json`, `delegates.local.json`,
/// `allowed-signers.json`) and `data` itself are deliberately left in place at
/// the `.shore/` top level.
const STORE_ENTRIES: &[&str] = FLAT_STORE_MARKERS;

pub struct MigrateStoreOptions {
    repo: PathBuf,
}

impl MigrateStoreOptions {
    pub fn new(repo: impl Into<PathBuf>) -> Self {
        Self { repo: repo.into() }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreMigrateResult {
    pub relocated: bool,
    pub events_rewritten: usize,
    pub events_unchanged: usize,
}

pub fn migrate_store(options: MigrateStoreOptions) -> Result<StoreMigrateResult> {
    let worktree_root = git_worktree_root(&options.repo)?;
    let shore = worktree_root.join(".shore");
    let data = shore.join("data");

    // Classify with the same layout detection the resolve-time guard uses, so a
    // shape that resolve treats as legacy (including a registration-only linked
    // checkout) is exactly the shape this relocates.
    let relocated = match detect_store_layout(&shore) {
        // Partial/interrupted migration → refuse (cannot know which is authoritative).
        StoreLayout::Conflict => {
            return Err(ShoreError::Message(
                "both a flat .shore/ store and a .shore/data/ store are present; refusing to \
                 migrate — inspect both and remove the stale one before re-running"
                    .to_owned(),
            ));
        }
        // A pre-relocation flat store → nest it under data/ (data-safe: copy in,
        // then remove the originals).
        StoreLayout::Flat => {
            nest_flat_store(&shore, &data)?;
            true
        }
        // Already nested, or no store at all → nothing to relocate.
        StoreLayout::Nested | StoreLayout::Fresh => false,
    };

    // 2. Exclude: remove the flat era's wholesale `.shore/` residue from the
    //    repo-local exclude, then generate the committed `.shore/.gitignore`.
    //    Order is load-bearing: while the wholesale line is present every ignore
    //    probe reports covered, so the generate step would no-op. Committed
    //    config stays tracked either way.
    remove_wholesale_shore_exclude(&worktree_root)?;
    ensure_shore_gitignore(&worktree_root)?;

    // 3. Migrate every event file in the nested store (raw JSON; never read_event).
    let store = EventStore::open(&data);
    let (mut rewritten, mut unchanged) = (0usize, 0usize);
    for name in store.list_event_file_names()? {
        match migrate_event_file(&store.events_dir().join(&name))? {
            EventMigrateOutcome::Rewritten => rewritten += 1,
            EventMigrateOutcome::Unchanged => unchanged += 1,
        }
    }
    Ok(StoreMigrateResult {
        relocated,
        events_rewritten: rewritten,
        events_unchanged: unchanged,
    })
}

/// Move the flat store entries from `shore/` into `shore/data/`. Data-safe: the
/// new location is fully populated (copy + the source still intact) before any
/// flat original is removed, so no crash window loses data.
fn nest_flat_store(shore: &Path, data: &Path) -> Result<()> {
    std::fs::create_dir_all(data).map_err(|error| io_error("create data dir", data, error))?;
    for entry in STORE_ENTRIES {
        let src = shore.join(entry);
        if src.exists() {
            copy_recursively(&src, &data.join(entry))?;
        }
    }
    // Only after every copy succeeds: remove the flat originals.
    for entry in STORE_ENTRIES {
        let src = shore.join(entry);
        if !src.exists() {
            continue;
        }
        if src.is_dir() {
            std::fs::remove_dir_all(&src)
                .map_err(|error| io_error("remove flat store dir", &src, error))?;
        } else {
            std::fs::remove_file(&src)
                .map_err(|error| io_error("remove flat store file", &src, error))?;
        }
    }
    Ok(())
}

fn copy_recursively(src: &Path, dst: &Path) -> Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst).map_err(|error| io_error("create dir", dst, error))?;
        for entry in std::fs::read_dir(src).map_err(|error| io_error("read dir", src, error))? {
            let entry = entry.map_err(|error| io_error("read dir entry", src, error))?;
            copy_recursively(&entry.path(), &dst.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| io_error("create parent dir", parent, error))?;
        }
        std::fs::copy(src, dst).map_err(|error| io_error("copy file", src, error))?;
    }
    Ok(())
}

/// Remove the flat era's wholesale `.shore/` line from `.git/info/exclude`.
/// Idempotent (a no-op when absent), and removal-only: this migration never
/// writes fresh info/exclude content — exclusion moves to the committed
/// `.shore/.gitignore`. Removing (rather than narrowing) the residue is what
/// stops the over-broad line from hiding committed config, and the probe-gated
/// generate step cannot do it: an existing wholesale line makes every probe
/// report covered.
fn remove_wholesale_shore_exclude(worktree_root: &Path) -> Result<()> {
    let exclude_path = git_info_exclude_path(worktree_root)?;
    let current = match std::fs::read_to_string(&exclude_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_error("read git exclude file", &exclude_path, error)),
    };
    if !current.lines().any(|line| line.trim() == ".shore/") {
        return Ok(());
    }
    let mut rewritten = current
        .lines()
        .filter(|line| line.trim() != ".shore/")
        .collect::<Vec<_>>()
        .join("\n");
    if current.ends_with('\n') && !rewritten.is_empty() {
        rewritten.push('\n');
    }
    std::fs::write(&exclude_path, rewritten)
        .map_err(|error| io_error("write git exclude file", &exclude_path, error))
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> ShoreError {
    ShoreError::Message(format!("{action} {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::*;
    use crate::git::git_paths_are_ignored;
    use crate::model::{EventId, JournalId};
    use crate::session::event::{
        EventTarget, EventType, ReviewInitializedPayload, ShoreEvent, Writer,
    };

    fn git_repo() -> tempfile::TempDir {
        let repo = tempfile::tempdir().expect("create temp git repository directory");
        let output = Command::new("git")
            .arg("init")
            .current_dir(repo.path())
            .output()
            .expect("run git init");
        assert!(output.status.success(), "git init failed");
        repo
    }

    fn sample_event(i: usize) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewInitialized,
            format!("review_initialized:session:{i}:work:default"),
            EventTarget::for_journal(JournalId::new(format!("journal:{i}"))),
            Writer::shore_local("0.1.0"),
            ReviewInitializedPayload {},
            "2026-05-10T00:00:00Z",
        )
        .expect("event builds")
    }

    /// Seed a FLAT store under `shore`: `shore/events/<legacy-writer events>` +
    /// `shore/state.json`. Returns the seeded event ids.
    fn seed_legacy_store(shore: &Path, n: usize) -> Vec<EventId> {
        let store = EventStore::open(shore);
        std::fs::create_dir_all(store.events_dir()).unwrap();
        std::fs::write(shore.join("state.json"), "{}").unwrap();
        let mut ids = Vec::new();
        for i in 0..n {
            let event = sample_event(i);
            ids.push(event.event_id.clone());
            let path = store.event_path_for_idempotency_key(&event.idempotency_key);
            // Downgrade to the legacy writer shape on disk.
            let mut v = serde_json::to_value(&event).unwrap();
            let w = v["writer"].as_object_mut().unwrap();
            let producer = w.remove("producer").unwrap();
            w.insert("tool".into(), producer);
            w.insert("role".into(), serde_json::json!("author"));
            std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
        }
        ids
    }

    /// Seed a nested producer store under `data` (current writer shape).
    fn seed_current_store(data: &Path, n: usize) {
        let store = EventStore::open(data);
        for i in 0..n {
            store.record_event_once(&sample_event(i)).unwrap();
        }
        std::fs::write(data.join("state.json"), "{}").unwrap();
    }

    #[test]
    fn migrate_store_nests_flat_store_and_makes_events_readable() {
        let repo = git_repo();
        let shore = repo.path().join(".shore");
        seed_legacy_store(&shore, 3);
        let result = migrate_store(MigrateStoreOptions::new(repo.path())).unwrap();
        assert!(result.relocated);
        assert_eq!(result.events_rewritten, 3);
        // .shore/ STAYS; only the store entries moved under data/.
        assert!(
            shore.is_dir(),
            ".shore/ itself is preserved (holds config + data)"
        );
        assert!(
            !shore.join("events").exists(),
            "flat events/ moved out of .shore top level"
        );
        assert!(!shore.join("state.json").exists(), "flat state.json moved");
        assert!(shore.join("data/state.json").is_file());
        let events = EventStore::open(shore.join("data")).list_events().unwrap();
        assert_eq!(events.len(), 3); // read cleanly via the standard path
    }

    #[test]
    fn migrate_store_preserves_committed_config_in_place() {
        let repo = git_repo();
        let shore = repo.path().join(".shore");
        seed_legacy_store(&shore, 1);
        std::fs::write(shore.join("delegates.json"), r#"{"delegates":{}}"#).unwrap();
        migrate_store(MigrateStoreOptions::new(repo.path())).unwrap();
        // Config is NOT swept into data/ — it stays at the .shore/ top level.
        assert!(shore.join("delegates.json").is_file());
        assert!(!shore.join("data/delegates.json").exists());
    }

    // The "registration-only legacy store relocates" scenario is retired: store
    // registration was removed with the shared-store default, so there is no
    // top-level registration file for the flat-store relocation to move. The
    // relocation now keys solely on real flat-store markers (events/artifacts/
    // state.json), covered by the flat-store relocation tests above.

    #[test]
    fn migrate_store_is_idempotent_on_second_run() {
        let repo = git_repo();
        seed_legacy_store(&repo.path().join(".shore"), 2);
        migrate_store(MigrateStoreOptions::new(repo.path())).unwrap();
        let second = migrate_store(MigrateStoreOptions::new(repo.path())).unwrap();
        assert!(!second.relocated); // already nested
        assert_eq!(second.events_rewritten, 0); // already producer-shaped
    }

    #[test]
    fn migrate_store_on_already_nested_store_is_clean_noop() {
        let repo = git_repo();
        seed_current_store(&repo.path().join(".shore/data"), 2);
        let result = migrate_store(MigrateStoreOptions::new(repo.path())).unwrap();
        assert!(!result.relocated);
        assert_eq!(result.events_rewritten, 0);
    }

    #[test]
    fn migrate_store_preserves_event_ids() {
        let repo = git_repo();
        let mut ids_before = seed_legacy_store(&repo.path().join(".shore"), 3);
        migrate_store(MigrateStoreOptions::new(repo.path())).unwrap();
        let mut ids_after: Vec<_> = EventStore::open(repo.path().join(".shore/data"))
            .list_events()
            .unwrap()
            .iter()
            .map(|e| e.event_id.clone())
            .collect();
        ids_before.sort();
        ids_after.sort();
        assert_eq!(ids_after, ids_before); // writer is outside the hash
    }

    #[test]
    fn migrate_store_refuses_when_flat_and_nested_both_present() {
        let repo = git_repo();
        seed_legacy_store(&repo.path().join(".shore"), 2); // flat markers
        seed_current_store(&repo.path().join(".shore/data"), 1); // nested markers
        let err = migrate_store(MigrateStoreOptions::new(repo.path()))
            .expect_err("flat + nested must be a refusal, not a silent partial migration");
        assert!(err.to_string().contains(".shore/data"));
        // Neither store mutated by the refused run.
        assert!(repo.path().join(".shore/events").is_dir());
        assert!(repo.path().join(".shore/data/events").is_dir());
    }

    #[test]
    fn migrate_store_removes_wholesale_exclude_and_keeps_committed_config_tracked() {
        let repo = git_repo();
        let shore = repo.path().join(".shore");
        seed_legacy_store(&shore, 1);
        std::fs::write(shore.join("delegates.json"), r#"{"delegates":{}}"#).unwrap();
        // Pre-migration exclude lists the WHOLESALE .shore/ (the flat-era residue),
        // plus an unrelated user line that must survive verbatim.
        let excl = git_info_exclude_path(repo.path()).unwrap();
        std::fs::create_dir_all(excl.parent().unwrap()).unwrap();
        std::fs::write(&excl, "# user line\nbuild-cache/\n.shore/\n").unwrap();

        migrate_store(MigrateStoreOptions::new(repo.path())).unwrap();

        // The wholesale line is REMOVED (not rewritten) — the migration writes no
        // fresh info/exclude content; exclusion moves to the generated
        // .shore/.gitignore. Unrelated user lines survive verbatim.
        assert_eq!(
            std::fs::read_to_string(&excl).unwrap(),
            "# user line\nbuild-cache/\n",
            "the wholesale .shore/ residue is dropped and nothing new is written"
        );
        assert_eq!(
            std::fs::read_to_string(repo.path().join(".shore/.gitignore")).unwrap(),
            "data/\n*.local.json\n"
        );
        // The committed config under .shore/ must NOT be git-ignored after
        // migration, while the store IS still excluded (now via the
        // .shore/.gitignore data/ entry).
        let ignored = git_paths_are_ignored(
            repo.path(),
            &[".shore/delegates.json", ".shore/data/state.json"],
        )
        .unwrap();
        assert_eq!(
            ignored,
            vec![false, true],
            "committed config tracked; store excluded"
        );
    }
}
