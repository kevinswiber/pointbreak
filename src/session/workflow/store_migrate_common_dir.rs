//! Consent-gated, non-destructive fold of a worktree-local `.shore/data` store
//! into the common-dir store (`<git-common-dir>/shore`).
//!
//! This is the user's path across the shared-default flip: it copies events and
//! artifacts forward via `import_store_bundle` (content-addressed, idempotent,
//! source untouched) so a worktree's prior captures are reachable from the common
//! dir. It NEVER deletes the source, NEVER registers anything (registration is
//! being retired), and NEVER runs on a hot path — only the `shore store migrate`
//! subcommand / `just migrate-store-common-dir` driver invoke it. It REFUSES an ephemeral or
//! scanned-sensitive worktree unless the caller passes an explicit override, so
//! sensitive throwaway bytes are never silently fanned into the shared store.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Result, ShoreError};
use crate::session::store::bundle::{ImportBundleResult, import_store_bundle};
use crate::session::store::resolution::clone_local_store_dir;
use crate::session::store::sensitivity::scan_worktree_sensitivity;
use crate::session::store::store_config::{StoreMode, resolve_store_mode};
use crate::session::store::store_init::ShoreStorePaths;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrateToCommonDirOptions {
    repo: PathBuf,
    include_ephemeral: bool,
}

impl MigrateToCommonDirOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            include_ephemeral: false,
        }
    }

    /// Opt in to migrating an ephemeral / scanned-sensitive worktree. Off by
    /// default: the migration refuses such a worktree without this override
    /// (no silent fan-in of sensitive bytes into the shared store).
    pub fn with_include_ephemeral(mut self, include_ephemeral: bool) -> Self {
        self.include_ephemeral = include_ephemeral;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrateToCommonDirResult {
    pub events_created: usize,
    pub events_existing: usize,
    pub artifacts_created: usize,
    pub artifacts_existing: usize,
    /// True when the source had nothing to migrate (no worktree-local store).
    /// Only reported once the consent gate has passed: an ephemeral/sensitive
    /// worktree is refused first, even when its source store is empty, so a
    /// refusal is never silently downgraded to a `sourceEmpty` no-op.
    pub source_empty: bool,
}

/// The sentinel `scan_worktree_sensitivity` emits for a worktree that must not be
/// fanned into the shared store without an explicit override.
const SENSITIVITY_BLOCK: &str = "block";

pub fn migrate_store_to_common_dir(
    options: MigrateToCommonDirOptions,
) -> Result<MigrateToCommonDirResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let worktree_root = paths.worktree_root().to_path_buf();
    let source = paths.store_dir().to_path_buf(); // worktree-local .shore/data

    // Consent gate: refuse an ephemeral or scanned-sensitive worktree unless the
    // caller explicitly opted in. Checked BEFORE any write to the common dir, and
    // deliberately before the missing-source no-op below: a refusal is uniform for
    // an ephemeral worktree and is never downgraded to a `source_empty` success.
    if !options.include_ephemeral {
        if resolve_store_mode(&worktree_root)? == StoreMode::Ephemeral {
            return Err(ShoreError::Message(
                "refusing to migrate an ephemeral worktree into the shared store; \
                 re-run with the include-ephemeral override to fan it in"
                    .to_owned(),
            ));
        }
        let scan = scan_worktree_sensitivity(&worktree_root)?;
        if scan.policy_outcome == SENSITIVITY_BLOCK {
            return Err(ShoreError::Message(
                "refusing to migrate a worktree flagged sensitive into the shared store; \
                 re-run with the include-ephemeral override to fan it in"
                    .to_owned(),
            ));
        }
    }

    // Nothing to migrate if the worktree has no local store yet.
    if !source.join("events").exists() {
        return Ok(MigrateToCommonDirResult {
            events_created: 0,
            events_existing: 0,
            artifacts_created: 0,
            artifacts_existing: 0,
            source_empty: true,
        });
    }

    // Source is resolved via the raw `ShoreStorePaths::resolve` and the target via
    // `clone_local_store_dir` (= `<git-common-dir>/shore`); both are reused, neither
    // recomputed. `import_store_bundle` only reads the source — this fn performs no
    // `remove`/`remove_dir` on it, unlike the in-place flat-store relocation, which
    // is a different migration and must not be conflated.
    let target = clone_local_store_dir(&worktree_root)?;
    let imported = import_store_bundle(&source, &target)?;
    Ok(MigrateToCommonDirResult::from_import(imported))
}

impl MigrateToCommonDirResult {
    fn from_import(imported: ImportBundleResult) -> Self {
        Self {
            events_created: imported.events_created,
            events_existing: imported.events_existing,
            artifacts_created: imported.artifacts_created,
            artifacts_existing: imported.artifacts_existing,
            source_empty: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use super::{MigrateToCommonDirOptions, migrate_store_to_common_dir};
    use crate::git::git_common_dir;
    use crate::session::store::store_config::{StoreMode, write_store_config};
    use crate::session::{CaptureOptions, EventStore, capture_worktree_review};

    struct TestRepo {
        root: tempfile::TempDir,
    }

    impl TestRepo {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("create temp git repository directory");
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

        fn write(&self, path: impl AsRef<Path>, contents: impl AsRef<[u8]>) {
            let path = self.root.path().join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directories");
            }
            fs::write(path, contents).expect("write test repository file");
        }

        fn commit_all(&self, message: &str) {
            self.git(["add", "--all"]);
            self.git(["commit", "-m", message]);
        }

        fn git<I, S>(&self, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<OsStr>,
        {
            let args = args
                .into_iter()
                .map(|arg| arg.as_ref().to_owned())
                .collect::<Vec<_>>();
            let output = Command::new("git")
                .args(&args)
                .current_dir(self.root.path())
                .output()
                .unwrap_or_else(|error| panic!("run git {args:?}: {error}"));
            assert!(
                output.status.success(),
                "git {args:?} failed\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
        }
    }

    fn modified_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        repo
    }

    #[test]
    fn folds_worktree_local_store_into_common_dir_non_destructively() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let local = repo.path().join(".shore/data");
        let common = git_common_dir(repo.path()).unwrap().join("shore");
        assert!(
            !common.exists(),
            "common-dir store does not exist before migration"
        );

        let result =
            migrate_store_to_common_dir(MigrateToCommonDirOptions::new(repo.path())).unwrap();

        // Events + the snapshot artifact landed in the common dir.
        assert!(result.events_created >= 1);
        assert!(result.artifacts_created >= 1);
        assert!(common.join("events").is_dir());
        assert!(common.join("artifacts/snapshots").is_dir());
        assert!(common.join("state.json").is_file());
        // Source is NEVER deleted (non-destructive).
        assert!(local.join("events").is_dir());
        let source_events = EventStore::open(&local).list_events().unwrap();
        assert!(!source_events.is_empty());
    }

    #[test]
    fn re_run_is_idempotent_and_reports_existing() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        let first =
            migrate_store_to_common_dir(MigrateToCommonDirOptions::new(repo.path())).unwrap();
        let second =
            migrate_store_to_common_dir(MigrateToCommonDirOptions::new(repo.path())).unwrap();

        assert!(first.events_created >= 1);
        assert_eq!(second.events_created, 0, "nothing new on re-run");
        assert!(
            second.events_existing >= 1,
            "re-run reports the already-present events"
        );
        assert_eq!(second.artifacts_created, 0);
        assert!(second.artifacts_existing >= 1);
    }

    #[test]
    fn refuses_an_ephemeral_worktree_without_include_ephemeral() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        // Mark the worktree ephemeral via the store-config writer.
        write_store_config(repo.path(), StoreMode::Ephemeral).unwrap();

        let error = migrate_store_to_common_dir(MigrateToCommonDirOptions::new(repo.path()))
            .expect_err("an ephemeral worktree must refuse without an explicit override");

        assert!(
            error.to_string().contains("ephemeral"),
            "the refusal names the ephemeral opt-out: {error}"
        );
        // Refused before any write to the common dir.
        let common = git_common_dir(repo.path()).unwrap().join("shore");
        assert!(
            !common.join("events").exists(),
            "no fan-in happened on a refused ephemeral migration"
        );
    }

    #[test]
    fn include_ephemeral_override_migrates_an_ephemeral_worktree() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        write_store_config(repo.path(), StoreMode::Ephemeral).unwrap();

        let result = migrate_store_to_common_dir(
            MigrateToCommonDirOptions::new(repo.path()).with_include_ephemeral(true),
        )
        .unwrap();

        assert!(result.events_created >= 1);
    }

    #[test]
    fn ephemeral_empty_worktree_refuses_before_reporting_source_empty() {
        // An ephemeral worktree with no local store yet is refused (the consent gate
        // runs before the missing-source no-op), so the refusal is uniform and never
        // downgraded to a `source_empty` success. The override then reports the empty
        // source honestly.
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");
        write_store_config(repo.path(), StoreMode::Ephemeral).unwrap();
        assert!(!repo.path().join(".shore/data/events").exists());

        let error = migrate_store_to_common_dir(MigrateToCommonDirOptions::new(repo.path()))
            .expect_err("an ephemeral worktree refuses even with an empty source store");
        assert!(error.to_string().contains("ephemeral"));

        let overridden = migrate_store_to_common_dir(
            MigrateToCommonDirOptions::new(repo.path()).with_include_ephemeral(true),
        )
        .unwrap();
        assert!(
            overridden.source_empty,
            "an empty source reports sourceEmpty once consent passes"
        );
        assert_eq!(overridden.events_created, 0);
    }

    #[test]
    fn source_shore_data_is_never_deleted_by_migration() {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let local = repo.path().join(".shore/data");
        let before = EventStore::open(&local).list_event_file_names().unwrap();

        migrate_store_to_common_dir(MigrateToCommonDirOptions::new(repo.path())).unwrap();

        let after = EventStore::open(&local).list_event_file_names().unwrap();
        assert_eq!(before, after, "the source store is byte-for-byte preserved");
    }
}
