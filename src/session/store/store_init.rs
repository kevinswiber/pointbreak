use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Result, ShoreError};
use crate::git::{git_path_is_untracked, git_paths_are_ignored, git_worktree_root};
use crate::storage::{LocalStorage, TempSweepAge};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ShoreStorePaths {
    worktree_root: PathBuf,
    store_dir: PathBuf,
}

impl ShoreStorePaths {
    pub(crate) fn resolve(repo: impl AsRef<Path>) -> Result<Self> {
        let worktree_root = git_worktree_root(repo.as_ref())?;
        let store_dir = worktree_root.join(".shore/data");
        // Hard cutover: a pre-relocation flat store (events/state.json directly
        // under `.shore/`) is a loud, actionable error rather than a silent
        // dual-read. Detection keys on the layout, not the directory name, so a
        // `.shore/` that holds only committed config resolves cleanly.
        match detect_store_layout(&worktree_root.join(".shore")) {
            StoreLayout::Conflict => {
                return Err(ShoreError::Message(
                    "both a legacy flat .shore/ store and a migrated .shore/data/ store are \
                     present; this is a partial/interrupted migration — inspect both and remove \
                     the stale one"
                        .to_owned(),
                ));
            }
            StoreLayout::Flat => {
                return Err(ShoreError::Message(
                    "legacy flat .shore/ store detected; this pre-1.0 store format is retired and \
                     no longer supported"
                        .to_owned(),
                ));
            }
            StoreLayout::Fresh | StoreLayout::Nested => {}
        }
        Ok(Self {
            worktree_root,
            store_dir,
        })
    }

    pub(crate) fn worktree_root(&self) -> &Path {
        &self.worktree_root
    }

    pub(crate) fn store_dir(&self) -> &Path {
        &self.store_dir
    }

    #[cfg(test)]
    pub(crate) fn state_path(&self) -> PathBuf {
        self.store_dir.join("state.json")
    }
}

/// The store directory reads and writes for `repo` actually resolve to — the
/// shared common-dir store by default, or the worktree-local `.shore/data` when
/// the worktree is Ephemeral. Delegates to the same resolver the read/write seams
/// use, so a library caller is never pointed at a different store than the CLI.
pub fn store_dir_for_repo(repo: &Path) -> Result<PathBuf> {
    Ok(crate::session::store::resolution::resolve_store(repo)?
        .store_dir()
        .to_path_buf())
}

/// The worktree-local store entries that, when found directly under `.shore/`,
/// mark a pre-relocation flat store. This is the single source of truth shared
/// by the resolve-time layout guard and the migration's relocation step, so the
/// two never diverge on which shapes count as a store. It deliberately excludes
/// the committed config siblings (`delegates.json`, `allowed-signers.json`,
/// `store.json`), so a config-only `.shore/` is not a store.
pub(crate) const FLAT_STORE_MARKERS: &[&str] = &["events", "artifacts", "state.json"];

/// True when any flat-store marker sits directly under `shore`
/// (`<worktree-root>/.shore`) — the pre-relocation layout.
fn flat_store_marker_present(shore: &Path) -> bool {
    FLAT_STORE_MARKERS
        .iter()
        .any(|entry| shore.join(entry).exists())
}

/// True when `<store_dir>` (`<root>/.shore/data`) holds a real worktree-local
/// store (any flat-store marker present), as opposed to an empty/absent dir. The
/// legacy guard on the normal read/write resolution path uses this to direct the
/// user to `shore store migrate` when a worktree-local store predates the shared
/// store default. A config-only `.shore/` (no events/artifacts/state.json under
/// `.shore/data`) is not populated.
pub(crate) fn worktree_local_store_is_populated(store_dir: &Path) -> bool {
    FLAT_STORE_MARKERS
        .iter()
        .any(|marker| store_dir.join(marker).exists())
}

/// The on-disk layout of a `.shore/` directory, classified for the hard-cutover
/// guard. Detection keys on flat-store markers versus the nested `.shore/data/`,
/// never on the `.shore/` directory itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StoreLayout {
    /// No flat-store markers and no `.shore/data/`: a fresh repo or a `.shore/`
    /// that holds only committed config (`delegates.json`).
    Fresh,
    /// Flat-store markers (events/artifacts/state.json directly under `.shore/`)
    /// and no `.shore/data/`: a pre-relocation store that must be migrated.
    Flat,
    /// `.shore/data/` present and no flat markers: the migrated steady state.
    Nested,
    /// Both flat markers and `.shore/data/`: an interrupted/partial migration.
    Conflict,
}

/// Classify the store layout under `shore` (`<worktree-root>/.shore`). A
/// config-only `.shore/` (committed `delegates.json` and no store) is `Fresh`,
/// because the probes look only for flat-store markers and the nested dir.
pub(crate) fn detect_store_layout(shore: &Path) -> StoreLayout {
    let nested = shore.join("data").exists();
    let flat = flat_store_marker_present(shore);
    match (flat, nested) {
        (true, true) => StoreLayout::Conflict,
        (true, false) => StoreLayout::Flat,
        (false, true) => StoreLayout::Nested,
        (false, false) => StoreLayout::Fresh,
    }
}

pub(crate) fn ensure_store_dirs(store_dir: &Path) -> Result<()> {
    for dir in [
        store_dir.join("events"),
        store_dir.join("artifacts/notes"),
        store_dir.join("artifacts/objects"),
    ] {
        fs::create_dir_all(&dir).map_err(|error| io_error("create directory", &dir, error))?;
    }
    Ok(())
}

pub(crate) fn sweep_stale_temp_files(storage: &LocalStorage, store_dir: &Path) -> Result<()> {
    storage.sweep_temp_files(store_dir, TempSweepAge::workflow_startup())
}

/// Shared writer setup against an explicit store dir and worktree root: sweep stale temp
/// files, ensure the store directory layout, and — only when the store lands inside the
/// worktree's `.shore/` (the ephemeral opt-in) — ensure the committed `.shore/.gitignore`
/// covers it. The shared common-dir store lives inside `.git/`, which git already
/// ignores, so a shared-store write generates nothing: a capture must never mutate the
/// worktree it is capturing (that would fork the content-only object id between a
/// worktree capture and a range capture of identical content). The write-landing seam's
/// `prepare_write_landing` calls this with the resolved write store dir and the worktree
/// root, so every write workflow shares one exclusion body.
pub(crate) fn prepare_store_writer_at(
    storage: &LocalStorage,
    store_dir: &Path,
    worktree_root: &Path,
) -> Result<()> {
    sweep_stale_temp_files(storage, store_dir)?;
    ensure_store_dirs(store_dir)?;
    if store_dir.starts_with(worktree_root.join(".shore")) {
        ensure_shore_gitignore(worktree_root)?;
    }
    Ok(())
}

/// One canonical probe → line mapping for the committed `.shore/.gitignore`.
/// `data/` covers the opt-in ephemeral store; `*.local.json` covers every
/// private `.local.json` override. Probes are worktree-relative paths checked
/// against ALL standard ignore sources, so user-managed ignore files are
/// respected and never duplicated.
const SHORE_GITIGNORE_SPECS: [(&str, &str); 4] = [
    (".shore/data/state.json", "data/"),
    (".shore/delegates.local.json", "*.local.json"),
    (".shore/actor-attributes.local.json", "*.local.json"),
    (".shore/store.local.json", "*.local.json"),
];

/// Worktree-relative path of the file Shore may generate and capture must not
/// sweep in while it is untracked and Shore-generated.
const SHORE_GITIGNORE_RELATIVE_PATH: &str = ".shore/.gitignore";

/// The canonical gitignore lines Shore can write into a fresh `.shore/.gitignore`,
/// in generation order and deduplicated, derived from [`SHORE_GITIGNORE_SPECS`] so
/// the generator and the capture-suppression oracle share one source of truth.
/// Today: `data/` then `*.local.json`.
fn canonical_shore_gitignore_lines() -> Vec<&'static str> {
    let mut lines: Vec<&'static str> = Vec::new();
    for (_, line) in SHORE_GITIGNORE_SPECS {
        if !lines.contains(&line) {
            lines.push(line);
        }
    }
    lines
}

/// True when `body` is byte-identical to a `.shore/.gitignore` Shore itself could
/// have generated: non-empty, LF-terminated, and its lines form an ordered,
/// duplicate-free subsequence of [`canonical_shore_gitignore_lines`]. Pure — no
/// git-ignore probing (an existing file covers its own probes, so a live probe
/// would self-contradict). A user-edited body (extra line, comment, reorder,
/// duplicate, blank line), any non-LF line ending (Shore writes LF only, so a
/// `\r` stays attached to the split line and fails the exact match), or any body
/// without a trailing newline is rejected.
fn body_is_purely_shore_generated(body: &str) -> bool {
    // Strip exactly the trailing LF, then split on LF only. `str::lines()` would
    // also swallow a `\r`, letting a CRLF body pass as if it were LF — which is not
    // byte-identical to what Shore generates.
    let Some(without_trailing_newline) = body.strip_suffix('\n') else {
        return false;
    };
    if without_trailing_newline.is_empty() {
        return false;
    }
    let canonical = canonical_shore_gitignore_lines();
    let mut next = 0usize; // advancing cursor into `canonical` enforces order + no dupes
    for line in without_trailing_newline.split('\n') {
        let Some(offset) = canonical[next..]
            .iter()
            .position(|candidate| *candidate == line)
        else {
            return false;
        };
        next += offset + 1;
    }
    true
}

/// Keep Pointbreak's generated/private files out of Git status via a committed
/// `.shore/.gitignore` — visible in the working tree, scoped to the directory,
/// and shared through clone — never by mutating the hidden, per-clone
/// `.git/info/exclude`. A path already ignored by any standard source is
/// skipped, so this is a no-op in a repo that manages its own ignores.
pub fn ensure_shore_gitignore(worktree_root: &Path) -> Result<()> {
    let probes: Vec<&str> = SHORE_GITIGNORE_SPECS
        .iter()
        .map(|(probe, _)| *probe)
        .collect();
    let ignored = git_paths_are_ignored(worktree_root, &probes)?;
    let mut missing: Vec<&str> = Vec::new();
    for ((_, line), is_ignored) in SHORE_GITIGNORE_SPECS.iter().zip(ignored) {
        if !is_ignored && !missing.contains(line) {
            missing.push(line);
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    append_shore_gitignore_lines(worktree_root, &missing)
}

/// True when `<worktree_root>/.shore/.gitignore` is an **untracked** file whose
/// bytes are byte-identical to what Shore itself generates. A tracked (committed)
/// file — clean or modified, even one edited back to a canonical body — a
/// user-edited untracked file, or an absent file all report false, so a real
/// reviewable change is never hidden. Reads the bytes first (a fast NotFound
/// short-circuit for the common no-file case) and applies the pure oracle before
/// the git probe, so the subprocess runs only for a genuinely Shore-shaped file.
/// Do not reorder the git probe ahead of the pure check.
pub(crate) fn generated_gitignore_is_capture_suppressible(worktree_root: &Path) -> Result<bool> {
    let path = worktree_root.join(SHORE_GITIGNORE_RELATIVE_PATH);
    let body = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(io_error("read .shore/.gitignore", &path, error)),
    };
    if !body_is_purely_shore_generated(&body) {
        return Ok(false);
    }
    git_path_is_untracked(worktree_root, SHORE_GITIGNORE_RELATIVE_PATH)
}

/// Absolute paths of Shore-generated files a worktree capture should filter out of
/// its inventory right now — currently just `.shore/.gitignore`, and only while it
/// is untracked and byte-identical to what Shore generates. Returned as absolute
/// paths ready for [`crate::git::IngestOptions::exclude_helper_path`], which records
/// nothing in provenance, so the suppression never folds into the revision id.
/// Empty when nothing is suppressible.
pub(crate) fn shore_generated_excluded_paths(worktree_root: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    if generated_gitignore_is_capture_suppressible(worktree_root)? {
        paths.push(worktree_root.join(SHORE_GITIGNORE_RELATIVE_PATH));
    }
    Ok(paths)
}

/// Append `lines` (each newline-terminated) to `<worktree_root>/.shore/.gitignore`,
/// creating `.shore/` and the file as needed and normalizing a missing trailing
/// newline on existing content. Callers pass only not-yet-ignored lines.
fn append_shore_gitignore_lines(worktree_root: &Path, lines: &[&str]) -> Result<()> {
    let path = worktree_root.join(".shore/.gitignore");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| io_error("create .shore directory", parent, error))?;
    }
    let current = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(io_error("read .shore/.gitignore", &path, error)),
    };
    let mut updated = current;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    for line in lines {
        updated.push_str(line);
        updated.push('\n');
    }
    fs::write(&path, updated).map_err(|error| io_error("write .shore/.gitignore", &path, error))
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> ShoreError {
    ShoreError::Message(format!("{action} {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::*;
    use crate::git::git_info_exclude_path;

    #[test]
    fn shore_store_paths_resolve_from_subdirectory() {
        let repo = git_repo();
        fs::create_dir_all(repo.path().join("src/nested")).unwrap();
        let paths = ShoreStorePaths::resolve(repo.path().join("src/nested")).unwrap();

        assert_existing_paths_eq(paths.worktree_root(), repo.path());
        // The store dir is now <root>/.shore/data.
        assert_eq!(path_file_name(paths.store_dir()), "data");
        assert_eq!(path_file_name(path_parent(paths.store_dir())), ".shore");
        assert_existing_paths_eq(path_parent(path_parent(paths.store_dir())), repo.path());
        // state.json is <root>/.shore/data/state.json.
        assert_eq!(path_file_name(paths.state_path().as_path()), "state.json");
        assert_eq!(
            path_file_name(path_parent(paths.state_path().as_path())),
            "data"
        );
        assert_eq!(
            path_file_name(path_parent(path_parent(paths.state_path().as_path()))),
            ".shore"
        );
    }

    #[test]
    fn public_shore_dir_helper_resolves_the_same_store_as_the_read_write_seams() {
        let repo = git_repo();

        let from_public_helper = store_dir_for_repo(repo.path()).unwrap();
        let from_resolver = crate::session::store::resolution::resolve_store(repo.path())
            .unwrap()
            .store_dir()
            .to_path_buf();

        assert_eq!(from_public_helper, from_resolver);
        // A fresh (non-ephemeral) repo resolves the shared common-dir store, not
        // the raw worktree-local `.shore/data`.
        assert_eq!(path_file_name(&from_public_helper), "shore");
    }

    #[test]
    fn public_shore_dir_helper_resolves_the_user_level_family_store_when_bound() {
        use crate::session::store::store_config::set_family_binding_for_repo;
        use crate::session::store::user_level::{
            ensure_family_store_scaffold, user_level_store_dir,
        };

        let repo = git_repo();
        let home = tempfile::tempdir().unwrap();
        // SAFETY: single-threaded test; nextest isolates each test in its own
        // process; SHORE_HOME is the documented hermetic seam (keys/home.rs).
        unsafe {
            std::env::set_var("SHORE_HOME", home.path());
        }

        let slug = "acme-web";
        let family_dir = user_level_store_dir(slug).unwrap();
        ensure_family_store_scaffold(&family_dir, slug, &[]).unwrap();
        set_family_binding_for_repo(repo.path(), slug, "0123abcd4567ef89").unwrap();

        let from_public_helper = store_dir_for_repo(repo.path()).unwrap();
        let from_resolver = crate::session::store::resolution::resolve_store(repo.path())
            .unwrap()
            .store_dir()
            .to_path_buf();
        unsafe {
            std::env::remove_var("SHORE_HOME");
        }

        assert_eq!(from_public_helper, from_resolver);
        // Both are computed from the same SHORE_HOME root, so they are byte-equal.
        assert_eq!(from_public_helper, family_dir);
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

    #[test]
    fn prepare_store_writer_at_creates_store_dirs_and_shore_gitignore() {
        let repo = git_repo();
        let paths = ShoreStorePaths::resolve(repo.path()).unwrap();
        let storage = LocalStorage::new(paths.store_dir());

        prepare_store_writer_at(&storage, paths.store_dir(), paths.worktree_root()).unwrap();

        assert!(paths.store_dir().join("events").is_dir());
        assert!(paths.store_dir().join("artifacts/notes").is_dir());
        assert!(paths.store_dir().join("artifacts/objects").is_dir());

        // Exclusion rides the committed .shore/.gitignore — never the hidden
        // repo-local exclude and never the root .gitignore.
        assert!(
            !repo.path().join(".gitignore").exists(),
            "writer setup must not create a root .gitignore"
        );
        let body = fs::read_to_string(repo.path().join(".shore/.gitignore")).unwrap();
        assert_eq!(body, "data/\n*.local.json\n");
        let exclude = git_info_exclude_path(repo.path()).unwrap();
        if exclude.exists() {
            let exclude_body = fs::read_to_string(&exclude).unwrap();
            assert!(
                !exclude_body.contains(".shore"),
                "no .shore entry lands in info/exclude: {exclude_body}"
            );
        }
    }

    #[test]
    fn prepare_store_writer_appends_missing_gitignore_lines_preserving_existing_body() {
        let repo = git_repo();
        let paths = ShoreStorePaths::resolve(repo.path()).unwrap();
        let storage = LocalStorage::new(paths.store_dir());

        // Seed a user-owned .shore/.gitignore with NO trailing newline so the test
        // also exercises newline normalization. Its pattern ignores none of the
        // probe paths, so both canonical lines append after it.
        fs::create_dir_all(repo.path().join(".shore")).unwrap();
        fs::write(
            repo.path().join(".shore/.gitignore"),
            "# existing\nvendor-cache/",
        )
        .unwrap();

        prepare_store_writer_at(&storage, paths.store_dir(), paths.worktree_root()).unwrap();

        // Assert the FULL file body: pre-existing content survives verbatim, the
        // missing trailing newline is normalized, and the canonical lines follow.
        let body = fs::read_to_string(repo.path().join(".shore/.gitignore")).unwrap();
        assert_eq!(
            body, "# existing\nvendor-cache/\ndata/\n*.local.json\n",
            "must preserve the existing body verbatim and append the missing lines \
             with exact newline handling"
        );
    }

    #[test]
    fn prepare_store_writer_at_covers_probe_paths_and_keeps_committed_config_tracked() {
        let repo = git_repo();
        let paths = ShoreStorePaths::resolve(repo.path()).unwrap();
        let storage = LocalStorage::new(paths.store_dir());

        prepare_store_writer_at(&storage, paths.store_dir(), paths.worktree_root()).unwrap();

        // The store dir and every private .local.json override are ignored…
        let ignored = git_paths_are_ignored(
            repo.path(),
            &[
                ".shore/data/state.json",
                ".shore/delegates.local.json",
                ".shore/actor-attributes.local.json",
                ".shore/store.local.json",
            ],
        )
        .unwrap();
        assert_eq!(ignored, vec![true, true, true, true]);
        // …while the committed config siblings stay tracked.
        let committed = git_paths_are_ignored(
            repo.path(),
            &[
                ".shore/store.json",
                ".shore/delegates.json",
                ".shore/actor-attributes.json",
            ],
        )
        .unwrap();
        assert_eq!(committed, vec![false, false, false]);
    }

    #[test]
    fn prepare_store_writer_at_generates_nothing_for_the_shared_common_dir_store() {
        let repo = git_repo();
        // The shared store lives inside .git/, which git already ignores; a
        // shared-store write must not mutate the worktree (no generated
        // .shore/.gitignore) — that is what keeps a capture from forking the
        // content-only object id of the worktree it is capturing.
        let store_dir = repo.path().join(".git/shore");
        let storage = LocalStorage::new(&store_dir);

        prepare_store_writer_at(&storage, &store_dir, repo.path()).unwrap();

        assert!(store_dir.join("events").is_dir());
        assert!(
            !repo.path().join(".shore/.gitignore").exists(),
            "a shared-store write generates no .shore/.gitignore"
        );
        assert!(
            !repo.path().join(".shore").exists(),
            "a shared-store write creates nothing under the worktree"
        );
    }

    #[test]
    fn prepare_store_writer_at_is_idempotent() {
        let repo = git_repo();
        let paths = ShoreStorePaths::resolve(repo.path()).unwrap();
        let storage = LocalStorage::new(paths.store_dir());

        // The probe reads the pre-append ignore state, so a second run must see the
        // now-covered probes as already-ignored and append nothing.
        prepare_store_writer_at(&storage, paths.store_dir(), paths.worktree_root()).unwrap();
        prepare_store_writer_at(&storage, paths.store_dir(), paths.worktree_root()).unwrap();

        let body = fs::read_to_string(repo.path().join(".shore/.gitignore")).unwrap();
        for line in ["data/", "*.local.json"] {
            let hits = body.lines().filter(|l| l.trim() == line).count();
            assert_eq!(
                hits, 1,
                "{line} must be written at most once across repeated runs"
            );
        }
    }

    #[test]
    fn body_oracle_accepts_every_body_shore_can_generate() {
        // The three non-empty ordered subsequences of [data/, *.local.json].
        assert!(body_is_purely_shore_generated("data/\n*.local.json\n"));
        assert!(body_is_purely_shore_generated("data/\n"));
        assert!(body_is_purely_shore_generated("*.local.json\n"));
    }

    #[test]
    fn body_oracle_rejects_user_touched_or_malformed_bodies() {
        assert!(!body_is_purely_shore_generated(
            "data/\n*.local.json\nmine/\n"
        )); // extra line
        assert!(!body_is_purely_shore_generated(
            "# mine\ndata/\n*.local.json\n"
        )); // comment
        assert!(!body_is_purely_shore_generated("*.local.json\ndata/\n")); // reordered
        assert!(!body_is_purely_shore_generated("data/\ndata/\n")); // duplicate
        assert!(!body_is_purely_shore_generated("data/\n\n*.local.json\n")); // blank line
        assert!(!body_is_purely_shore_generated("data/\n*.local.json")); // no trailing newline
        assert!(!body_is_purely_shore_generated("")); // empty
        assert!(!body_is_purely_shore_generated("\n")); // lone newline
    }

    #[test]
    fn body_oracle_rejects_non_lf_line_endings() {
        // Shore writes LF only; a CRLF or bare-CR body is not byte-identical, even
        // though its visible lines match. `str::lines()` would strip the `\r` — the
        // oracle must not, or a user-touched CRLF file could be wrongly suppressed.
        assert!(!body_is_purely_shore_generated("data/\r\n*.local.json\r\n")); // CRLF
        assert!(!body_is_purely_shore_generated("data/\r\n")); // CRLF, single line
        assert!(!body_is_purely_shore_generated("data/\r*.local.json\n")); // bare CR separator
    }

    #[test]
    fn untracked_canonical_gitignore_is_suppressible() {
        let repo = git_repo();
        ensure_shore_gitignore(repo.path()).unwrap(); // writes canonical, untracked
        assert!(generated_gitignore_is_capture_suppressible(repo.path()).unwrap());
        assert_eq!(
            shore_generated_excluded_paths(repo.path()).unwrap(),
            vec![repo.path().join(".shore/.gitignore")]
        );
    }

    #[test]
    fn absent_gitignore_is_not_suppressible() {
        let repo = git_repo();
        assert!(!generated_gitignore_is_capture_suppressible(repo.path()).unwrap());
        assert!(
            shore_generated_excluded_paths(repo.path())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn user_edited_untracked_gitignore_is_not_suppressible() {
        let repo = git_repo();
        fs::create_dir_all(repo.path().join(".shore")).unwrap();
        fs::write(
            repo.path().join(".shore/.gitignore"),
            "data/\n*.local.json\nmine/\n",
        )
        .unwrap();
        assert!(!generated_gitignore_is_capture_suppressible(repo.path()).unwrap());
        assert!(
            shore_generated_excluded_paths(repo.path())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn committed_gitignore_is_not_suppressible_even_when_canonical() {
        let repo = git_repo();
        fs::create_dir_all(repo.path().join(".shore")).unwrap();
        fs::write(
            repo.path().join(".shore/.gitignore"),
            "data/\n*.local.json\n",
        )
        .unwrap();
        commit_all(&repo); // now TRACKED, byte-identical to canonical
        // Byte-oracle passes, but the untracked gate fails ⇒ not suppressible.
        assert!(!generated_gitignore_is_capture_suppressible(repo.path()).unwrap());
        assert!(
            shore_generated_excluded_paths(repo.path())
                .unwrap()
                .is_empty()
        );
    }

    /// Commit everything in a `git_repo()` (which has no identity configured).
    fn commit_all(repo: &tempfile::TempDir) {
        for args in [
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "user.name", "t"],
            vec!["config", "commit.gpgsign", "false"],
            vec!["add", "--all"],
            vec!["commit", "-m", "x"],
        ] {
            let out = Command::new("git")
                .args(&args)
                .current_dir(repo.path())
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?} failed");
        }
    }

    #[test]
    fn ensure_shore_gitignore_writes_the_two_canonical_lines() {
        let repo = git_repo();
        ensure_shore_gitignore(repo.path()).unwrap();
        let body = fs::read_to_string(repo.path().join(".shore/.gitignore")).unwrap();
        assert_eq!(body, "data/\n*.local.json\n");
        // The mechanism is the committed .shore/.gitignore — never the repo-local
        // exclude and never the root .gitignore. (`git init` may seed a commented
        // info/exclude template, so assert on content, not existence.)
        let exclude = git_info_exclude_path(repo.path()).unwrap();
        if exclude.exists() {
            let exclude_body = fs::read_to_string(&exclude).unwrap();
            assert!(
                !exclude_body.contains(".shore"),
                "no .shore entry lands in info/exclude: {exclude_body}"
            );
        }
        assert!(!repo.path().join(".gitignore").exists());
    }

    #[test]
    fn ensure_shore_gitignore_is_idempotent() {
        let repo = git_repo();
        ensure_shore_gitignore(repo.path()).unwrap();
        ensure_shore_gitignore(repo.path()).unwrap();
        let body = fs::read_to_string(repo.path().join(".shore/.gitignore")).unwrap();
        assert_eq!(
            body, "data/\n*.local.json\n",
            "each line is written at most once"
        );
    }

    #[test]
    fn ensure_shore_gitignore_appends_missing_lines_preserving_user_content() {
        let repo = git_repo();
        // A user-owned .shore/.gitignore that already covers the data/ store (via
        // its own spelling) but not the local overrides, with no trailing newline
        // so the append also exercises newline normalization.
        fs::create_dir_all(repo.path().join(".shore")).unwrap();
        fs::write(repo.path().join(".shore/.gitignore"), "# mine\ndata").unwrap();
        ensure_shore_gitignore(repo.path()).unwrap();
        let body = fs::read_to_string(repo.path().join(".shore/.gitignore")).unwrap();
        // The user's `data` line (no slash) is a basename pattern that matches the
        // data directory, so the .shore/data/state.json probe reports ignored and
        // only *.local.json is appended; existing content survives verbatim.
        assert_eq!(body, "# mine\ndata\n*.local.json\n");
    }

    #[test]
    fn ensure_shore_gitignore_is_a_noop_when_probes_are_already_ignored() {
        let repo = git_repo();
        // Any standard ignore source counts — here the root .gitignore covers both
        // the store dir and the local overrides, so nothing is written.
        fs::write(
            repo.path().join(".gitignore"),
            ".shore/data/\n.shore/*.local.json\n",
        )
        .unwrap();
        ensure_shore_gitignore(repo.path()).unwrap();
        assert!(
            !repo.path().join(".shore/.gitignore").exists(),
            "user-managed ignore files are respected; no file is generated"
        );
    }

    #[test]
    fn shore_gitignore_covers_all_four_probe_paths() {
        let repo = git_repo();
        ensure_shore_gitignore(repo.path()).unwrap();
        let ignored = crate::git::git_paths_are_ignored(
            repo.path(),
            &[
                ".shore/data/state.json",
                ".shore/delegates.local.json",
                ".shore/actor-attributes.local.json",
                ".shore/store.local.json",
            ],
        )
        .unwrap();
        assert_eq!(ignored, vec![true, true, true, true]);
        // The committed config siblings are never ignored.
        let committed = crate::git::git_paths_are_ignored(
            repo.path(),
            &[".shore/store.json", ".shore/delegates.json"],
        )
        .unwrap();
        assert_eq!(committed, vec![false, false]);
    }

    #[test]
    fn prepare_store_writer_at_preserves_fresh_temp_files() {
        let repo = git_repo();
        let paths = ShoreStorePaths::resolve(repo.path()).unwrap();
        fs::create_dir_all(paths.store_dir().join("events")).unwrap();
        let temp = paths.store_dir().join("events/.shore-write.fresh.tmp");
        fs::write(&temp, "in flight").unwrap();
        let storage = LocalStorage::new(paths.store_dir());

        prepare_store_writer_at(&storage, paths.store_dir(), paths.worktree_root()).unwrap();

        assert_eq!(fs::read_to_string(temp).unwrap(), "in flight");
    }

    #[test]
    fn legacy_flat_store_is_a_loud_error() {
        let repo = git_repo();
        // Pre-migration FLAT store: events + state.json directly under .shore/,
        // no .shore/data/. At the 1.0 format floor this pre-1.0 layout is retired,
        // so it is a loud error rather than a silent dual-read or a migration offer.
        fs::create_dir_all(repo.path().join(".shore/events")).unwrap();
        fs::write(repo.path().join(".shore/state.json"), "{}").unwrap();

        let err = ShoreStorePaths::resolve(repo.path())
            .expect_err("legacy flat .shore/ store must be a loud error");
        let message = err.to_string();
        assert!(
            message.contains("no longer supported"),
            "reads as a retired format; got: {message}"
        );
        assert!(
            message.contains(".shore"),
            "names the legacy store; got: {message}"
        );
    }

    #[test]
    fn both_flat_and_nested_store_is_a_conflict_error() {
        let repo = git_repo();
        // Interrupted/partial migration left BOTH the flat store and the nested
        // one. Must be LOUD — never silently prefer .shore/data/ and orphan the
        // flat store.
        fs::create_dir_all(repo.path().join(".shore/events")).unwrap();
        fs::create_dir_all(repo.path().join(".shore/data/events")).unwrap();
        let err = ShoreStorePaths::resolve(repo.path())
            .expect_err("flat + nested store must be a conflict");
        let message = err.to_string();
        assert!(
            message.contains(".shore/data"),
            "names the nested store; got: {message}"
        );
        assert!(
            message.contains("both") || message.contains("conflict"),
            "reads as a conflict: {message}"
        );
    }

    #[test]
    fn migrated_nested_store_resolves_cleanly() {
        let repo = git_repo();
        // Post-migration steady state: only the nested store, no flat markers.
        fs::create_dir_all(repo.path().join(".shore/data/events")).unwrap();
        let paths = ShoreStorePaths::resolve(repo.path()).expect("nested store resolves");
        assert_eq!(path_file_name(paths.store_dir()), "data");
    }

    #[test]
    fn store_registration_json_is_no_longer_a_flat_store_marker() {
        // Registration is retired: a lone store-registration.json is not a store,
        // so it does not trip the flat-store layout guard.
        let repo = git_repo();
        fs::create_dir_all(repo.path().join(".shore")).unwrap();
        fs::write(repo.path().join(".shore/store-registration.json"), "{}").unwrap();

        let layout = detect_store_layout(&repo.path().join(".shore"));
        assert_eq!(layout, StoreLayout::Fresh);
    }

    #[test]
    fn config_only_shore_dir_is_not_a_legacy_store() {
        let repo = git_repo();
        // .shore/ holds ONLY committed config (no store yet). Must NOT trip the
        // legacy guard — committed config now legitimately lives under .shore/.
        fs::create_dir_all(repo.path().join(".shore")).unwrap();
        fs::write(
            repo.path().join(".shore/delegates.json"),
            r#"{"delegates":{}}"#,
        )
        .unwrap();
        let paths = ShoreStorePaths::resolve(repo.path()).expect("config-only .shore/ resolves");
        assert_eq!(path_file_name(paths.store_dir()), "data");
    }

    #[test]
    fn fresh_repo_with_no_shore_dir_resolves_cleanly() {
        let repo = git_repo();
        let paths = ShoreStorePaths::resolve(repo.path()).expect("fresh repo resolves");
        assert_eq!(path_file_name(paths.store_dir()), "data");
    }

    fn git_repo() -> tempfile::TempDir {
        let repo = tempfile::tempdir().expect("create temp git repository directory");
        let output = Command::new("git")
            .arg("init")
            .current_dir(repo.path())
            .output()
            .expect("run git init");
        assert!(
            output.status.success(),
            "git init failed in {}:\nstdout:\n{}\nstderr:\n{}",
            repo.path().display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        repo
    }
}
