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
use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::git::command::{Ancestry, GitReflogEntry, GitWorktree, RefEntry};

pub(crate) mod subprocess;

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
}

/// The closed set of git backends resolved at the [`dispatch`] choke point.
/// Today it holds only the subprocess backend; a library variant is added behind
/// a cargo feature later without changing call sites.
pub(crate) enum GitBackendKind {
    Subprocess(SubprocessBackend),
}

impl GitBackendKind {
    /// Borrow the active backend as a trait object. The delegating `GitBackend`
    /// impl below routes every method through this one match, so adding a
    /// variant is a single new arm here.
    fn as_backend(&self) -> &dyn GitBackend {
        match self {
            GitBackendKind::Subprocess(backend) => backend,
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
}

static SUBPROCESS_KIND: GitBackendKind = GitBackendKind::Subprocess(SubprocessBackend);

/// Resolve the backend for a routable operation. There is no selector yet, so
/// dispatch is trivial and infallible: every routable op runs on the subprocess
/// backend.
pub(crate) fn dispatch() -> &'static GitBackendKind {
    &SUBPROCESS_KIND
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
        assert!(dispatch().worktree_root(repo.path()).is_ok());
        assert!(
            dispatch()
                .for_each_ref(repo.path(), &["refs/heads/"])
                .is_ok()
        );
    }
}
