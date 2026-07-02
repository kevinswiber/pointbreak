//! Consent-gated, non-destructive fold of a worktree-local `.shore/data` store
//! into the common-dir store (`<git-common-dir>/shore`).
//!
//! This is the user's path across the shared-default flip: it copies events and
//! artifacts forward via `import_store_bundle` (content-addressed, idempotent,
//! source untouched) so a worktree's prior captures are reachable from the common
//! dir. It never deletes the source BY DEFAULT — the opt-in retire-source
//! completion deletes it only after `verify_source_subset_of_target` confirms
//! every durable source file in the shared store. It NEVER registers anything
//! (registration is retired) and NEVER runs on a hot path — only the
//! `shore store migrate` subcommand / `just migrate-store-common-dir` driver
//! invoke it. It REFUSES an ephemeral or scanned-sensitive worktree unless the
//! caller passes an explicit override, so sensitive throwaway bytes are never
//! silently fanned into the shared store.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Result, ShoreError};
use crate::session::store::bundle::{
    ImportBundleResult, import_store_bundle, verify_source_subset_of_target,
};
use crate::session::store::resolution::clone_local_store_dir;
use crate::session::store::sensitivity::scan_worktree_sensitivity;
use crate::session::store::store_config::{StoreMode, resolve_store_mode};
use crate::session::store::store_init::ShoreStorePaths;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrateToCommonDirOptions {
    repo: PathBuf,
    include_ephemeral: bool,
    retire_source: bool,
}

impl MigrateToCommonDirOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            include_ephemeral: false,
            retire_source: false,
        }
    }

    /// Opt in to migrating an ephemeral / scanned-sensitive worktree. Off by
    /// default: the migration refuses such a worktree without this override
    /// (no silent fan-in of sensitive bytes into the shared store).
    pub fn with_include_ephemeral(mut self, include_ephemeral: bool) -> Self {
        self.include_ephemeral = include_ephemeral;
        self
    }

    /// Opt in to deleting the worktree-local `.shore/data` after the fold is
    /// independently verified (every source event and artifact file present in
    /// the shared store; see `verify_source_subset_of_target`), so reads
    /// resolve in one command. Off by default: the source is never discarded
    /// before the migration is confirmed.
    pub fn with_retire_source(mut self, retire_source: bool) -> Self {
        self.retire_source = retire_source;
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
    /// True when `--retire-source` deleted the worktree-local `.shore/data`
    /// (after a verified fold, or as a no-durable-files husk).
    pub source_retired: bool,
    /// Files the retire verification confirmed in the shared store; zero when
    /// the retire was not requested or nothing needed verifying.
    pub verified_events: usize,
    pub verified_artifacts: usize,
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

    // The retire path classifies the source by PHYSICAL FILE COUNTS, never
    // directory existence: the writer pre-creates empty events/ and
    // artifacts/… dirs, so a directory-existence test would misroute an
    // empty-events-plus-orphan-artifacts source down the populated path and
    // delete unverified bytes.
    if options.retire_source {
        match classify_retire_source(&source)? {
            RetireSourceShape::Populated => {} // fold, verify, then delete below
            RetireSourceShape::Husk => {
                let source_retired = source.exists();
                if source_retired {
                    std::fs::remove_dir_all(&source).map_err(|error| {
                        ShoreError::Message(format!(
                            "remove retired source store {}: {error}",
                            source.display()
                        ))
                    })?;
                }
                return Ok(MigrateToCommonDirResult {
                    events_created: 0,
                    events_existing: 0,
                    artifacts_created: 0,
                    artifacts_existing: 0,
                    source_empty: true,
                    source_retired,
                    verified_events: 0,
                    verified_artifacts: 0,
                });
            }
            RetireSourceShape::ArtifactsWithoutEvents => {
                return Err(ShoreError::Message(format!(
                    "refusing to retire {}: it holds artifact files but no event files, so the \
                     fold cannot verify them; inspect the artifacts manually before deleting \
                     anything",
                    source.display()
                )));
            }
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
            source_retired: false,
            verified_events: 0,
            verified_artifacts: 0,
        });
    }

    // Source is resolved via the raw `ShoreStorePaths::resolve` and the target via
    // `clone_local_store_dir` (= `<git-common-dir>/shore`); both are reused, neither
    // recomputed. `import_store_bundle` only reads the source — by default this fn
    // performs no `remove`/`remove_dir` on it; the opt-in retire below deletes it
    // only after `verify_source_subset_of_target` confirms every durable source
    // file in the target. (The in-place flat-store relocation is a different
    // migration and must not be conflated.)
    let target = clone_local_store_dir(&worktree_root)?;
    let imported = import_store_bundle(&source, &target)?;
    let mut result = MigrateToCommonDirResult::from_import(imported);
    if options.retire_source {
        let verification = verify_source_subset_of_target(&source, &target)?;
        std::fs::remove_dir_all(&source).map_err(|error| {
            ShoreError::Message(format!(
                "remove retired source store {}: {error}",
                source.display()
            ))
        })?;
        result.source_retired = true;
        result.verified_events = verification.verified_events;
        result.verified_artifacts = verification.verified_artifacts;
    }
    Ok(result)
}

/// The retire-path classification of a worktree-local source store, by durable
/// file counts under `events/` and `artifacts/` (excluding in-flight `*.tmp`
/// files; the regenerable store-root `state.json` is a sibling of those trees
/// and is never counted — a nested file merely NAMED `state.json` is durable).
enum RetireSourceShape {
    /// Event files present: fold, verify, then delete.
    Populated,
    /// No durable files at all (absent dir, or only state.json/temp/empty
    /// dirs): removable without a fold.
    Husk,
    /// Artifact files but no event files: never silently deletable.
    ArtifactsWithoutEvents,
}

fn classify_retire_source(source: &Path) -> Result<RetireSourceShape> {
    let events = count_durable_files(&source.join("events"))?;
    let artifacts = count_durable_files(&source.join("artifacts"))?;
    Ok(match (events, artifacts) {
        (0, 0) => RetireSourceShape::Husk,
        (0, _) => RetireSourceShape::ArtifactsWithoutEvents,
        _ => RetireSourceShape::Populated,
    })
}

/// Count durable files under `dir` recursively; a missing dir counts zero.
/// Skips only in-flight `*.tmp` files, mirroring `verify_source_subset_of_target`.
fn count_durable_files(dir: &Path) -> Result<usize> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(ShoreError::Message(format!(
                "read store directory {} for retire classification: {error}",
                dir.display()
            )));
        }
    };
    let mut count = 0;
    for entry in entries {
        let entry = entry.map_err(|error| {
            ShoreError::Message(format!(
                "read store directory entry under {} for retire classification: {error}",
                dir.display()
            ))
        })?;
        if entry.path().is_dir() {
            count += count_durable_files(&entry.path())?;
        } else if !entry.file_name().to_string_lossy().ends_with(".tmp") {
            count += 1;
        }
    }
    Ok(count)
}

impl MigrateToCommonDirResult {
    fn from_import(imported: ImportBundleResult) -> Self {
        Self {
            events_created: imported.events_created,
            events_existing: imported.events_existing,
            artifacts_created: imported.artifacts_created,
            artifacts_existing: imported.artifacts_existing,
            source_empty: false,
            source_retired: false,
            verified_events: 0,
            verified_artifacts: 0,
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

    /// Seed a pre-shared-default capture: a populated worktree-local `.shore/data`
    /// store, which is exactly the source `shore store migrate` folds forward. We
    /// capture under ephemeral mode (so the write lands in `.shore/data`), then
    /// restore the default Shared mode so the migration runs against a
    /// non-ephemeral worktree carrying a legacy worktree-local store.
    fn seed_worktree_local_capture(repo: &TestRepo) {
        write_store_config(repo.path(), StoreMode::Ephemeral).unwrap();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        write_store_config(repo.path(), StoreMode::Shared).unwrap();
        assert!(
            repo.path().join(".shore/data/events").is_dir(),
            "the seed lands a worktree-local store to migrate"
        );
    }

    #[test]
    fn folds_worktree_local_store_into_common_dir_non_destructively() {
        let repo = modified_repo();
        seed_worktree_local_capture(&repo);
        let local = repo.path().join(".shore/data");
        let common = git_common_dir(repo.path()).unwrap().join("shore");
        assert!(
            !common.join("events").exists(),
            "common-dir store has no events before migration"
        );

        let result =
            migrate_store_to_common_dir(MigrateToCommonDirOptions::new(repo.path())).unwrap();

        // Events + the object artifact landed in the common dir.
        assert!(result.events_created >= 1);
        assert!(result.artifacts_created >= 1);
        assert!(common.join("events").is_dir());
        assert!(common.join("artifacts/objects").is_dir());
        assert!(common.join("state.json").is_file());
        // Source is NEVER deleted (non-destructive).
        assert!(local.join("events").is_dir());
        let source_events = EventStore::open(&local).list_events().unwrap();
        assert!(!source_events.is_empty());
    }

    #[test]
    fn re_run_is_idempotent_and_reports_existing() {
        let repo = modified_repo();
        seed_worktree_local_capture(&repo);

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
        seed_worktree_local_capture(&repo);
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
        seed_worktree_local_capture(&repo);
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
    fn retire_source_deletes_the_source_after_a_verified_fold() {
        let repo = modified_repo();
        seed_worktree_local_capture(&repo);

        let result = migrate_store_to_common_dir(
            MigrateToCommonDirOptions::new(repo.path()).with_retire_source(true),
        )
        .unwrap();

        assert!(result.source_retired);
        assert!(result.verified_events >= 1);
        assert!(result.verified_artifacts >= 1);
        assert!(
            !repo.path().join(".shore/data").exists(),
            "the verified fold retires the worktree-local store"
        );
        // The committed config siblings under .shore/ survive — only data/ goes.
        assert!(repo.path().join(".shore/store.json").is_file());
    }

    #[test]
    fn default_migrate_still_never_deletes_the_source() {
        let repo = modified_repo();
        seed_worktree_local_capture(&repo);

        let result =
            migrate_store_to_common_dir(MigrateToCommonDirOptions::new(repo.path())).unwrap();

        assert!(!result.source_retired);
        assert_eq!(result.verified_events, 0);
        assert_eq!(result.verified_artifacts, 0);
        assert!(repo.path().join(".shore/data/events").is_dir());
    }

    #[test]
    fn retire_source_refuses_and_preserves_source_when_verification_fails() {
        let repo = modified_repo();
        seed_worktree_local_capture(&repo);
        // Poison the fold with an ENVELOPE-only divergent copy of one source
        // event pre-planted in the target (same idempotency key and payload,
        // different occurredAt). Payload divergence would fail at import
        // preflight instead; envelope divergence dedups to the first-stored
        // target record, so only the verification can catch it.
        let source = repo.path().join(".shore/data");
        let common = git_common_dir(repo.path()).unwrap().join("shore");
        let name = EventStore::open(&source)
            .list_event_file_names()
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(source.join("events").join(&name)).unwrap()).unwrap();
        value["occurredAt"] = serde_json::Value::String("2020-01-01T00:00:00Z".to_owned());
        fs::create_dir_all(common.join("events")).unwrap();
        fs::write(
            common.join("events").join(&name),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();

        let error = migrate_store_to_common_dir(
            MigrateToCommonDirOptions::new(repo.path()).with_retire_source(true),
        )
        .expect_err("a divergent target must fail verification and refuse the retire");

        assert!(
            error.to_string().contains("not deleted") || error.to_string().contains("left"),
            "the error says the source survives: {error}"
        );
        assert!(
            repo.path().join(".shore/data/events").is_dir(),
            "source untouched"
        );
    }

    #[test]
    fn retire_source_removes_a_husk_source_with_no_durable_files() {
        let repo = modified_repo();
        // A .shore/data holding only a stale state.json plus the EMPTY dirs the
        // writer pre-creates trips the populated guard but has nothing durable:
        // state.json is a regenerable projection, the dirs are empty.
        // Classification is by FILE COUNT, so the empty events/ dir must not
        // route this down the populated path.
        fs::create_dir_all(repo.path().join(".shore/data/events")).unwrap();
        fs::create_dir_all(repo.path().join(".shore/data/artifacts/objects")).unwrap();
        fs::write(repo.path().join(".shore/data/state.json"), "{}").unwrap();

        let result = migrate_store_to_common_dir(
            MigrateToCommonDirOptions::new(repo.path()).with_retire_source(true),
        )
        .unwrap();

        assert!(result.source_retired);
        assert!(!repo.path().join(".shore/data").exists());
    }

    #[test]
    fn retire_source_refuses_orphan_artifacts_under_an_empty_events_dir() {
        // The regression the plan review caught: the writer pre-creates events/,
        // so a directory-existence husk test would send this source down the
        // populated path — zero events means the fold verifies nothing — and
        // then delete the orphan artifact bytes unverified.
        let repo = modified_repo();
        fs::create_dir_all(repo.path().join(".shore/data/events")).unwrap();
        fs::create_dir_all(repo.path().join(".shore/data/artifacts/objects")).unwrap();
        fs::write(
            repo.path()
                .join(".shore/data/artifacts/objects/orphan.json"),
            "{}",
        )
        .unwrap();

        let error = migrate_store_to_common_dir(
            MigrateToCommonDirOptions::new(repo.path()).with_retire_source(true),
        )
        .expect_err("artifact files without event files must never be silently deleted");
        assert!(error.to_string().contains("artifact"));
        assert!(
            repo.path()
                .join(".shore/data/artifacts/objects/orphan.json")
                .is_file(),
            "the unverified bytes survive"
        );
    }

    #[test]
    fn retire_source_refuses_a_nested_file_named_state_json_under_artifacts() {
        // Only the STORE-ROOT state.json is regenerable; a nested file merely
        // NAMED state.json is durable bytes — it must classify as an artifact
        // file (refusal), never be skipped as a husk and deleted unverified.
        let repo = modified_repo();
        fs::create_dir_all(repo.path().join(".shore/data/events")).unwrap();
        fs::create_dir_all(repo.path().join(".shore/data/artifacts/objects")).unwrap();
        fs::write(
            repo.path().join(".shore/data/artifacts/objects/state.json"),
            "{}",
        )
        .unwrap();

        let error = migrate_store_to_common_dir(
            MigrateToCommonDirOptions::new(repo.path()).with_retire_source(true),
        )
        .expect_err("a nested state.json is durable bytes, never silently deletable");
        assert!(error.to_string().contains("artifact"));
        assert!(
            repo.path()
                .join(".shore/data/artifacts/objects/state.json")
                .is_file(),
            "the unverified bytes survive"
        );
    }

    #[test]
    fn retire_source_refuses_a_source_with_artifacts_but_no_events_dir() {
        let repo = modified_repo();
        fs::create_dir_all(repo.path().join(".shore/data/artifacts/objects")).unwrap();
        fs::write(
            repo.path()
                .join(".shore/data/artifacts/objects/orphan.json"),
            "{}",
        )
        .unwrap();

        let error = migrate_store_to_common_dir(
            MigrateToCommonDirOptions::new(repo.path()).with_retire_source(true),
        )
        .expect_err("artifacts without events must never be silently deleted");
        assert!(error.to_string().contains("artifact"));
        assert!(repo.path().join(".shore/data/artifacts").is_dir());
    }

    #[test]
    fn retire_source_refuses_a_populated_source_carrying_an_orphan_artifact() {
        // A populated source whose artifacts/ holds a file no event references:
        // the fold does not carry it, so the physical-walk verification finds it
        // missing in the target and the retire refuses rather than deleting the
        // only copy.
        let repo = modified_repo();
        seed_worktree_local_capture(&repo);
        fs::write(
            repo.path()
                .join(".shore/data/artifacts/objects/orphan.json"),
            "{}",
        )
        .unwrap();

        let error = migrate_store_to_common_dir(
            MigrateToCommonDirOptions::new(repo.path()).with_retire_source(true),
        )
        .expect_err("an orphan artifact must fail verification and block the retire");
        assert!(
            error.to_string().contains("orphan.json"),
            "names the file: {error}"
        );
        assert!(
            repo.path().join(".shore/data/events").is_dir(),
            "source untouched on refusal"
        );
    }

    #[test]
    fn retire_source_still_respects_the_ephemeral_refusal() {
        let repo = modified_repo();
        seed_worktree_local_capture(&repo);
        write_store_config(repo.path(), StoreMode::Ephemeral).unwrap();

        let error = migrate_store_to_common_dir(
            MigrateToCommonDirOptions::new(repo.path()).with_retire_source(true),
        )
        .expect_err("the consent gate fires before any fold or retire");
        assert!(error.to_string().contains("ephemeral"));
        assert!(repo.path().join(".shore/data/events").is_dir());
    }

    #[test]
    fn source_shore_data_is_never_deleted_by_migration() {
        let repo = modified_repo();
        seed_worktree_local_capture(&repo);
        let local = repo.path().join(".shore/data");
        let before = EventStore::open(&local).list_event_file_names().unwrap();

        migrate_store_to_common_dir(MigrateToCommonDirOptions::new(repo.path())).unwrap();

        let after = EventStore::open(&local).list_event_file_names().unwrap();
        assert_eq!(before, after, "the source store is byte-for-byte preserved");
    }
}
