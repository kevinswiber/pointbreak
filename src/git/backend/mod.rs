//! The git backend seam. Every routable `git_*` operation dispatches through a
//! closed [`GitBackendKind`] enum resolved at one choke point ([`dispatch`]);
//! the concrete work lives behind the object-safe [`GitBackend`] trait. Today
//! the only variant shells out to the `git` binary ([`subprocess`]); a library
//! backend can be added later without touching call sites.
//!
//! Capture-time diff and `write-tree` are deliberately **not** trait methods:
//! they stay direct-subprocess free functions so no dispatch path can ever route
//! them away from `git` itself.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::error::{Result, ShoreError};
use crate::git::command::{Ancestry, GitInventoryPath, GitReflogEntry, GitWorktree, RefEntry};

#[cfg(feature = "gix")]
pub(crate) mod gix;
pub(crate) mod subprocess;

#[cfg(feature = "gix")]
use gix::GixBackend;
use subprocess::SubprocessBackend;

/// One method per routable git operation, each mirroring the existing typed
/// return so the three-valued/allowed-status exit semantics stay absorbed inside
/// the operation and no exit code crosses the seam. Object-safe by construction
/// (every method takes `&self` and returns an owned value).
pub(crate) trait GitBackend: Send + Sync {
    // Repository discovery.
    fn worktree_root(&self, repo: &Path) -> Result<PathBuf>;
    fn common_dir(&self, repo: &Path) -> Result<PathBuf>;

    // Read: graph / refs.
    fn is_ancestor(
        &self,
        repo: &Path,
        ancestor_oid: &str,
        descendant_oid: &str,
    ) -> Result<Ancestry>;
    fn independent_commits(&self, repo: &Path, oids: &[String]) -> Result<Vec<String>>;
    fn commit_changed_paths(&self, repo: &Path, commit_oid: &str) -> Result<Vec<String>>;
    fn commit_subjects(
        &self,
        repo: &Path,
        commit_oids: &BTreeSet<String>,
    ) -> Result<BTreeMap<String, String>>;
    fn for_each_ref(&self, repo: &Path, patterns: &[&str]) -> Result<Vec<RefEntry>>;
    fn ref_state_lines(&self, repo: &Path) -> Result<String>;
    fn object_exists(&self, repo: &Path, oid: &str) -> Result<bool>;
    fn default_branch_ref(&self, repo: &Path) -> Result<Option<String>>;
    fn rev_list_range(&self, repo: &Path, range: &str) -> Result<Vec<String>>;
    fn rev_list_reachable(&self, repo: &Path, tips: &[String]) -> Result<HashSet<String>>;
    fn rev_list_reflog_reachable(&self, repo: &Path) -> Result<HashSet<String>>;
    fn reflog_entries(&self, repo: &Path, ref_name: &str) -> Result<Vec<GitReflogEntry>>;
    fn worktree_list(&self, repo: &Path) -> Result<Vec<GitWorktree>>;

    // Read: ignore (the exclude stack is opened/reloaded per call, so an
    // ignore-source mutation is always observed by a later probe).
    fn paths_are_ignored(&self, repo: &Path, pathspecs: &[&str]) -> Result<Vec<bool>>;

    // Read: inventory.
    fn untracked_inventory(&self, repo: &Path) -> Result<Vec<GitInventoryPath>>;
    fn tracked_and_untracked_inventory(&self, repo: &Path) -> Result<Vec<GitInventoryPath>>;
    fn path_is_untracked(&self, repo: &Path, relative_path: &str) -> Result<bool>;

    // Read: config. Option returns — a backend/config miss is `None`, never an
    // error, matching the writer-identity fallback semantics.
    fn config_get(&self, repo: &Path, key: &str) -> Option<String>;
    fn config_path_get(&self, repo: &Path, key: &str) -> Option<String>;

    // Identity-grade scalars.
    fn head_ref(&self, repo: &Path) -> Result<Option<String>>;
    fn head_oid(&self, repo: &Path) -> Result<String>;
    fn head_commit_oid_optional(&self, repo: &Path) -> Result<Option<String>>;
    fn rev_parse_commit_oid(&self, repo: &Path, rev: &str) -> Result<String>;
    fn commit_tree_oid(&self, repo: &Path, commit_oid: &str) -> Result<String>;
    fn empty_tree_oid(&self, repo: &Path) -> Result<String>;
}

/// The closed set of git backends resolved at the [`dispatch`] choke point. The
/// subprocess backend is always present; the in-process `gix` backend is added
/// behind the `gix` cargo feature, so the default build keeps a single-variant
/// enum and stays byte-identical.
pub(crate) enum GitBackendKind {
    Subprocess(SubprocessBackend),
    #[cfg(feature = "gix")]
    Gix(GixBackend),
}

impl GitBackendKind {
    /// Borrow the active backend as a trait object. The delegating `GitBackend`
    /// impl below routes every method through this one match, so adding a
    /// variant is a single new arm here.
    fn as_backend(&self) -> &dyn GitBackend {
        match self {
            GitBackendKind::Subprocess(backend) => {
                #[cfg(test)]
                subprocess::record_backend_tag(subprocess::BackendTag::Subprocess);
                backend
            }
            #[cfg(feature = "gix")]
            GitBackendKind::Gix(backend) => {
                #[cfg(test)]
                subprocess::record_backend_tag(subprocess::BackendTag::Gix);
                backend
            }
        }
    }
}

impl GitBackend for GitBackendKind {
    fn worktree_root(&self, repo: &Path) -> Result<PathBuf> {
        self.as_backend().worktree_root(repo)
    }

    fn common_dir(&self, repo: &Path) -> Result<PathBuf> {
        self.as_backend().common_dir(repo)
    }

    fn is_ancestor(
        &self,
        repo: &Path,
        ancestor_oid: &str,
        descendant_oid: &str,
    ) -> Result<Ancestry> {
        self.as_backend()
            .is_ancestor(repo, ancestor_oid, descendant_oid)
    }

    fn independent_commits(&self, repo: &Path, oids: &[String]) -> Result<Vec<String>> {
        self.as_backend().independent_commits(repo, oids)
    }

    fn commit_changed_paths(&self, repo: &Path, commit_oid: &str) -> Result<Vec<String>> {
        self.as_backend().commit_changed_paths(repo, commit_oid)
    }

    fn commit_subjects(
        &self,
        repo: &Path,
        commit_oids: &BTreeSet<String>,
    ) -> Result<BTreeMap<String, String>> {
        self.as_backend().commit_subjects(repo, commit_oids)
    }

    fn for_each_ref(&self, repo: &Path, patterns: &[&str]) -> Result<Vec<RefEntry>> {
        self.as_backend().for_each_ref(repo, patterns)
    }

    fn ref_state_lines(&self, repo: &Path) -> Result<String> {
        self.as_backend().ref_state_lines(repo)
    }

    fn object_exists(&self, repo: &Path, oid: &str) -> Result<bool> {
        self.as_backend().object_exists(repo, oid)
    }

    fn default_branch_ref(&self, repo: &Path) -> Result<Option<String>> {
        self.as_backend().default_branch_ref(repo)
    }

    fn rev_list_range(&self, repo: &Path, range: &str) -> Result<Vec<String>> {
        self.as_backend().rev_list_range(repo, range)
    }

    fn rev_list_reachable(&self, repo: &Path, tips: &[String]) -> Result<HashSet<String>> {
        self.as_backend().rev_list_reachable(repo, tips)
    }

    fn rev_list_reflog_reachable(&self, repo: &Path) -> Result<HashSet<String>> {
        self.as_backend().rev_list_reflog_reachable(repo)
    }

    fn reflog_entries(&self, repo: &Path, ref_name: &str) -> Result<Vec<GitReflogEntry>> {
        self.as_backend().reflog_entries(repo, ref_name)
    }

    fn worktree_list(&self, repo: &Path) -> Result<Vec<GitWorktree>> {
        self.as_backend().worktree_list(repo)
    }

    fn paths_are_ignored(&self, repo: &Path, pathspecs: &[&str]) -> Result<Vec<bool>> {
        self.as_backend().paths_are_ignored(repo, pathspecs)
    }

    fn untracked_inventory(&self, repo: &Path) -> Result<Vec<GitInventoryPath>> {
        self.as_backend().untracked_inventory(repo)
    }

    fn tracked_and_untracked_inventory(&self, repo: &Path) -> Result<Vec<GitInventoryPath>> {
        self.as_backend().tracked_and_untracked_inventory(repo)
    }

    fn path_is_untracked(&self, repo: &Path, relative_path: &str) -> Result<bool> {
        self.as_backend().path_is_untracked(repo, relative_path)
    }

    fn config_get(&self, repo: &Path, key: &str) -> Option<String> {
        self.as_backend().config_get(repo, key)
    }

    fn config_path_get(&self, repo: &Path, key: &str) -> Option<String> {
        self.as_backend().config_path_get(repo, key)
    }

    fn head_ref(&self, repo: &Path) -> Result<Option<String>> {
        self.as_backend().head_ref(repo)
    }

    fn head_oid(&self, repo: &Path) -> Result<String> {
        self.as_backend().head_oid(repo)
    }

    fn head_commit_oid_optional(&self, repo: &Path) -> Result<Option<String>> {
        self.as_backend().head_commit_oid_optional(repo)
    }

    fn rev_parse_commit_oid(&self, repo: &Path, rev: &str) -> Result<String> {
        self.as_backend().rev_parse_commit_oid(repo, rev)
    }

    fn commit_tree_oid(&self, repo: &Path, commit_oid: &str) -> Result<String> {
        self.as_backend().commit_tree_oid(repo, commit_oid)
    }

    fn empty_tree_oid(&self, repo: &Path) -> Result<String> {
        self.as_backend().empty_tree_oid(repo)
    }
}

static SUBPROCESS_KIND: GitBackendKind = GitBackendKind::Subprocess(SubprocessBackend);

#[cfg(feature = "gix")]
static GIX_KIND: GitBackendKind = GitBackendKind::Gix(GixBackend);

static SUBPROCESS_BACKEND: SubprocessBackend = SubprocessBackend;

/// The environment variable that overrides the compiled backend default. Absent
/// uses the compiled default; `subprocess`/`gix` force every routable operation
/// onto that backend; any other value (empty, non-UTF-8, unknown, or `gix` on a
/// build without the gix feature) is a hard, actionable error.
const POINTBREAK_GIT_BACKEND: &str = "POINTBREAK_GIT_BACKEND";

/// How the process resolves a routable operation's backend. `Compiled` follows
/// the build-time default (subprocess for every routable operation in this
/// phase); the two `Force*` values are the runtime override for diagnostics and
/// immediate mitigation. Resolved once per process (see [`selector`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BackendSelector {
    Compiled,
    ForceSubprocess,
    // On a build without the gix backend, `ForceGix` is unreachable through the
    // environment (`parse_selector` rejects `gix`), so production code never
    // constructs it — but the variant must still exist so `dispatch` can reject a
    // test-injected `ForceGix` (the F4 hard-error contract). It is live in any
    // gix build.
    #[cfg_attr(not(feature = "gix"), allow(dead_code))]
    ForceGix,
}

/// Parse `POINTBREAK_GIT_BACKEND` into a [`BackendSelector`]. Absent is the
/// compiled default; `subprocess`/`gix` are the forced values; empty, non-UTF-8,
/// unknown, and `gix` on a feature-off build are all actionable errors — never a
/// silent fallback. An explicit `gix` with no gix backend fails here rather than
/// quietly resolving to subprocess.
fn parse_selector(raw: Option<&OsStr>) -> Result<BackendSelector> {
    let Some(value) = raw else {
        return Ok(BackendSelector::Compiled);
    };
    let Some(text) = value.to_str() else {
        return Err(ShoreError::Message(format!(
            "{POINTBREAK_GIT_BACKEND} is not valid UTF-8; set it to 'subprocess' or 'gix'"
        )));
    };
    match text {
        "subprocess" => Ok(BackendSelector::ForceSubprocess),
        #[cfg(feature = "gix")]
        "gix" => Ok(BackendSelector::ForceGix),
        #[cfg(not(feature = "gix"))]
        "gix" => Err(ShoreError::Message(format!(
            "{POINTBREAK_GIT_BACKEND}=gix but this build was compiled without the gix backend"
        ))),
        other => Err(ShoreError::Message(format!(
            "{POINTBREAK_GIT_BACKEND}={other:?} is not a known git backend \
             (expected 'subprocess' or 'gix')"
        ))),
    }
}

/// The process-wide backend selector, resolved once from the environment and
/// cached. Tests inject a thread-local override so a bad-selector case never
/// poisons the shared cache for a concurrent test.
fn selector() -> Result<BackendSelector> {
    #[cfg(test)]
    if let Some(injected) = INJECTED_SELECTOR.with(std::cell::Cell::get) {
        return Ok(injected);
    }

    // Cache the parsed value (or its error text) once per process. `ShoreError`
    // is not `Clone`, so the error is cached as its rendered message and rebuilt.
    static CACHED: OnceLock<std::result::Result<BackendSelector, String>> = OnceLock::new();
    CACHED
        .get_or_init(|| {
            parse_selector(std::env::var_os(POINTBREAK_GIT_BACKEND).as_deref())
                .map_err(|error| error.to_string())
        })
        .clone()
        .map_err(ShoreError::Message)
}

/// Validate `POINTBREAK_GIT_BACKEND` at startup, surfacing an actionable error
/// for an empty/non-UTF-8/unknown/feature-off-`gix` value before any subcommand
/// runs. Re-exported from `git/mod.rs` so the separate binary crate's `run_cli`
/// can call it as its single validation boundary; every CLI path flows through
/// there, so the infallible config helpers simply run post-validation.
#[doc(hidden)]
pub fn validate_backend_selector() -> Result<()> {
    selector().map(|_| ())
}

/// Resolve the backend for a routable operation. Fallible because it surfaces the
/// selector error: an unset/`subprocess`/`Compiled` selector routes to the
/// subprocess backend, an explicit `gix` selector routes to the native gix
/// backend, and a feature-off explicit `gix` is a hard error (only a compiled
/// default may fall back to subprocess).
pub(crate) fn dispatch() -> Result<&'static GitBackendKind> {
    match selector()? {
        BackendSelector::ForceSubprocess | BackendSelector::Compiled => Ok(&SUBPROCESS_KIND),
        #[cfg(feature = "gix")]
        BackendSelector::ForceGix => Ok(&GIX_KIND),
        #[cfg(not(feature = "gix"))]
        BackendSelector::ForceGix => Err(ShoreError::Message(format!(
            "{POINTBREAK_GIT_BACKEND}=gix but this build was compiled without the gix backend"
        ))),
    }
}

/// The direct subprocess handle for the two non-routable operations — write-tree
/// and (via the ingest pipeline) capture diff. It never consults [`dispatch`], so
/// no selector or class default can route these identity-bearing operations away
/// from `git` itself; that is what keeps their "subprocess by construction"
/// guarantee structural rather than configured.
pub(crate) fn subprocess_backend() -> &'static SubprocessBackend {
    &SUBPROCESS_BACKEND
}

// A test-only, thread-local backend selector override. Thread-local (like the
// Phase 1 instrumentation) so a test's inject/act/reset is never perturbed by a
// concurrent test on another thread under a shared-process runner.
#[cfg(test)]
thread_local! {
    static INJECTED_SELECTOR: std::cell::Cell<Option<BackendSelector>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn inject_selector(selector: BackendSelector) {
    INJECTED_SELECTOR.with(|cell| cell.set(Some(selector)));
}

#[cfg(test)]
pub(crate) fn reset_selector() {
    INJECTED_SELECTOR.with(|cell| cell.set(None));
}

#[cfg(test)]
mod tests {
    use subprocess::run_git;
    use tempfile::TempDir;

    use super::*;

    fn init_repo() -> TempDir {
        let dir = TempDir::new().expect("create temp git repository directory");
        run_git(dir.path(), ["init"]).unwrap();
        run_git(dir.path(), ["config", "user.name", "Shore Tests"]).unwrap();
        run_git(
            dir.path(),
            ["config", "user.email", "shore-tests@example.com"],
        )
        .unwrap();
        run_git(dir.path(), ["config", "commit.gpgsign", "false"]).unwrap();
        std::fs::write(dir.path().join("file.txt"), "one\n").unwrap();
        run_git(dir.path(), ["add", "--all"]).unwrap();
        run_git(dir.path(), ["commit", "-m", "first"]).unwrap();
        dir
    }

    #[test]
    fn subprocess_backend_resolves_discovery_and_graph() {
        let repo = init_repo();
        let backend = SubprocessBackend;

        let root = backend.worktree_root(repo.path()).unwrap();
        assert_eq!(
            root.canonicalize().unwrap(),
            repo.path().canonicalize().unwrap()
        );
        assert!(backend.common_dir(repo.path()).is_ok());

        let entries = backend.for_each_ref(repo.path(), &["refs/heads/"]).unwrap();
        assert!(
            entries
                .iter()
                .any(|entry| entry.name.starts_with("refs/heads/"))
        );
    }

    #[test]
    fn dispatch_routes_through_the_subprocess_backend() {
        let repo = init_repo();
        // The choke point resolves the same discovery/graph contract as the
        // backend directly, proving call sites can dispatch through the enum.
        assert!(dispatch().unwrap().worktree_root(repo.path()).is_ok());
        assert!(
            dispatch()
                .unwrap()
                .for_each_ref(repo.path(), &["refs/heads/"])
                .is_ok()
        );
    }

    #[test]
    fn selector_rejects_bad_values_and_feature_off_gix() {
        assert!(parse_selector(Some(OsStr::new("libgit2"))).is_err());
        assert!(parse_selector(Some(OsStr::new(""))).is_err());
        assert_eq!(parse_selector(None).unwrap(), BackendSelector::Compiled);
        assert_eq!(
            parse_selector(Some(OsStr::new("subprocess"))).unwrap(),
            BackendSelector::ForceSubprocess
        );
        #[cfg(not(feature = "gix"))]
        assert!(parse_selector(Some(OsStr::new("gix"))).is_err());
        #[cfg(feature = "gix")]
        assert_eq!(
            parse_selector(Some(OsStr::new("gix"))).unwrap(),
            BackendSelector::ForceGix
        );
    }

    #[cfg(not(feature = "gix"))]
    #[test]
    fn dispatch_rejects_feature_off_force_gix() {
        // A feature-off build cannot resolve an explicit gix selection: an
        // injected `ForceGix` errors rather than collapsing to subprocess.
        inject_selector(BackendSelector::ForceGix);
        assert!(dispatch().is_err());
        reset_selector();
    }
}
