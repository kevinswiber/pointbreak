//! The in-process `gix` backend. Every routable operation is native gix — there
//! is no forwarding to the subprocess backend — so `POINTBREAK_GIT_BACKEND=gix`
//! routes every routable operation through this file.
//!
//! Handles are opened per call and dropped (never shared across threads); the
//! exclude stack is opened in-call so an ignore-source mutation is always
//! observed by a later probe. The module is named `gix` and shadows the external
//! crate, so the crate is always referred to by its absolute path `::gix`.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use ::gix::bstr::ByteSlice;

use crate::error::{Result, ShoreError};
use crate::git::backend::GitBackend;
use crate::git::command::{Ancestry, GitInventoryPath, GitReflogEntry, GitWorktree, RefEntry};

/// The in-process gix backend. A unit struct: gix repository handles are opened
/// per call, not held.
pub(crate) struct GixBackend;

/// An internal gix-side failure. Result-returning trait methods surface it as
/// [`ShoreError::Message`] — the seam gains no new variant (LB-4) — while the
/// `Option`-returning config helpers swallow it to `None`.
#[derive(Debug)]
pub(crate) struct GitBackendError(String);

impl GitBackendError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for GitBackendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for GitBackendError {}

impl From<GitBackendError> for ShoreError {
    fn from(error: GitBackendError) -> Self {
        ShoreError::Message(error.0)
    }
}

type GixResult<T> = std::result::Result<T, GitBackendError>;

// A parity-harness counter: how many times this backend opened a repository on
// the calling thread. It proves the differential harness actually executed the
// gix backend rather than reading the shared subprocess discovery memo (F6).
// Thread-local so a test's reset/act/assert is immune to concurrent helpers on
// other threads under a shared-process runner.
#[cfg(all(test, feature = "gix-parity"))]
thread_local! {
    static GIX_OPEN_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(all(test, feature = "gix-parity"))]
pub(crate) fn gix_open_count() -> usize {
    GIX_OPEN_COUNT.with(std::cell::Cell::get)
}

#[cfg(all(test, feature = "gix-parity"))]
pub(crate) fn reset_gix_open_count() {
    GIX_OPEN_COUNT.with(|cell| cell.set(0));
}

/// Open `repo` as a gix repository, mapping the open failure to a backend error.
fn open(repo: &Path) -> GixResult<::gix::Repository> {
    #[cfg(all(test, feature = "gix-parity"))]
    GIX_OPEN_COUNT.with(|cell| cell.set(cell.get() + 1));
    ::gix::open(repo).map_err(|error| {
        GitBackendError::new(format!(
            "open git repository at {}: {error}",
            repo.display()
        ))
    })
}

/// Parse a hex object id (SHA-1 or SHA-256 by length) into a gix `ObjectId`.
fn parse_oid(oid: &str) -> GixResult<::gix::ObjectId> {
    ::gix::ObjectId::from_hex(oid.as_bytes())
        .map_err(|error| GitBackendError::new(format!("parse git object id {oid}: {error}")))
}

/// Canonicalize a filesystem path, matching the absolute, symlink-resolved form
/// the subprocess backend returns from `rev-parse`.
fn canonicalize(path: &Path) -> GixResult<PathBuf> {
    std::fs::canonicalize(path)
        .map_err(|error| GitBackendError::new(format!("canonicalize {}: {error}", path.display())))
}

/// Render a `BStr` as an owned `String`, erroring on non-UTF-8 like the
/// subprocess helpers do for the values that must be text.
fn bstr_to_string(bytes: &::gix::bstr::BStr, description: &str) -> GixResult<String> {
    bytes.to_str().map(str::to_owned).map_err(|error| {
        GitBackendError::new(format!("git returned non-utf8 {description}: {error}"))
    })
}

impl GixBackend {
    /// Whether `name` resolves to a commit object, the two-valued check
    /// default-branch resolution needs (a dangling symbolic ref resolves to
    /// nothing and must be skipped).
    fn reference_resolves_to_commit(repository: &::gix::Repository, name: &str) -> bool {
        let Ok(Some(mut reference)) = repository.try_find_reference(name) else {
            return false;
        };
        let Ok(id) = reference.peel_to_id() else {
            return false;
        };
        id.object()
            .map(|object| object.kind == ::gix::object::Kind::Commit)
            .unwrap_or(false)
    }

    /// Collect the paths changed between `base_tree` and `commit_tree` into
    /// `paths`, deduplicating via `seen`. No rename detection (matches
    /// `diff-tree` without `-M`); non-UTF-8 paths are skipped.
    fn collect_changed_paths(
        base_tree: &::gix::Tree<'_>,
        commit_tree: &::gix::Tree<'_>,
        paths: &mut Vec<String>,
        seen: &mut HashSet<String>,
    ) -> GixResult<()> {
        let mut changes = base_tree
            .changes()
            .map_err(|error| GitBackendError::new(format!("diff commit trees: {error}")))?;
        changes
            .for_each_to_obtain_tree(commit_tree, |change| {
                use ::gix::object::tree::diff::Change;
                let (location, entry_mode) = match &change {
                    Change::Addition {
                        location,
                        entry_mode,
                        ..
                    }
                    | Change::Deletion {
                        location,
                        entry_mode,
                        ..
                    }
                    | Change::Modification {
                        location,
                        entry_mode,
                        ..
                    }
                    | Change::Rewrite {
                        location,
                        entry_mode,
                        ..
                    } => (*location, *entry_mode),
                };
                // `diff-tree -r` reports only files; skip the intermediate tree
                // entries gix also emits.
                if !entry_mode.is_tree()
                    && let Ok(path) = location.to_str()
                {
                    let path = path.to_owned();
                    if seen.insert(path.clone()) {
                        paths.push(path);
                    }
                }
                Ok::<_, std::convert::Infallible>(std::ops::ControlFlow::Continue(()))
            })
            .map_err(|error| GitBackendError::new(format!("walk commit tree changes: {error}")))?;
        Ok(())
    }
}

impl GitBackend for GixBackend {
    fn worktree_root(&self, repo: &Path) -> Result<PathBuf> {
        let repository = open(repo)?;
        let workdir = repository.workdir().ok_or_else(|| {
            GitBackendError::new(format!(
                "{} is a bare repository with no worktree",
                repo.display()
            ))
        })?;
        Ok(canonicalize(workdir)?)
    }

    fn common_dir(&self, repo: &Path) -> Result<PathBuf> {
        let repository = open(repo)?;
        // gix returns the common dir non-normalized (e.g. `…/worktrees/x/../..`);
        // canonicalize so parity is on the resolved dir, never the path taken.
        Ok(canonicalize(repository.common_dir())?)
    }

    fn is_ancestor(
        &self,
        repo: &Path,
        ancestor_oid: &str,
        descendant_oid: &str,
    ) -> Result<Ancestry> {
        let repository = open(repo)?;
        let (Ok(ancestor), Ok(descendant)) = (parse_oid(ancestor_oid), parse_oid(descendant_oid))
        else {
            return Ok(Ancestry::MissingObject);
        };
        if !repository.has_object(ancestor) || !repository.has_object(descendant) {
            return Ok(Ancestry::MissingObject);
        }
        if ancestor == descendant {
            return Ok(Ancestry::Ancestor);
        }
        match repository.merge_base(ancestor, descendant) {
            Ok(base) if base.detach() == ancestor => Ok(Ancestry::Ancestor),
            // A resolvable-but-different base, or no common ancestor at all, both
            // mean `ancestor` is not an ancestor of `descendant`.
            Ok(_) | Err(_) => Ok(Ancestry::NotAncestor),
        }
    }

    fn independent_commits(&self, repo: &Path, oids: &[String]) -> Result<Vec<String>> {
        if oids.len() <= 1 {
            return Ok(oids.to_vec());
        }
        let repository = open(repo)?;
        let parsed = oids
            .iter()
            .map(|oid| parse_oid(oid))
            .collect::<GixResult<Vec<_>>>()?;

        // A commit is independent iff it is not an ancestor of any other commit
        // in the set. Equal duplicates keep only their first occurrence.
        let mut independent = Vec::new();
        for (index, candidate) in parsed.iter().enumerate() {
            let mut dominated = false;
            for (other_index, other) in parsed.iter().enumerate() {
                if index == other_index {
                    continue;
                }
                if candidate == other {
                    if other_index < index {
                        dominated = true;
                        break;
                    }
                    continue;
                }
                if let Ok(base) = repository.merge_base(*candidate, *other)
                    && base.detach() == *candidate
                {
                    dominated = true;
                    break;
                }
            }
            if !dominated {
                independent.push(oids[index].clone());
            }
        }
        Ok(independent)
    }

    fn commit_changed_paths(&self, repo: &Path, commit_oid: &str) -> Result<Vec<String>> {
        let repository = open(repo)?;
        let oid = parse_oid(commit_oid)?;
        let commit = repository
            .find_commit(oid)
            .map_err(|error| GitBackendError::new(format!("find commit {commit_oid}: {error}")))?;
        let commit_tree = commit.tree().map_err(|error| {
            GitBackendError::new(format!("read commit tree {commit_oid}: {error}"))
        })?;

        let mut paths = Vec::new();
        let mut seen = HashSet::new();
        let parents: Vec<_> = commit.parent_ids().collect();
        if parents.is_empty() {
            // A root commit lists its whole tree (`diff-tree --root`).
            let empty = repository.empty_tree();
            Self::collect_changed_paths(&empty, &commit_tree, &mut paths, &mut seen)?;
        } else {
            // A merge commit lists the union of its per-parent diffs (`-m`).
            for parent_id in parents {
                let parent = repository
                    .find_commit(parent_id.detach())
                    .map_err(|error| {
                        GitBackendError::new(format!("find parent commit: {error}"))
                    })?;
                let parent_tree = parent
                    .tree()
                    .map_err(|error| GitBackendError::new(format!("read parent tree: {error}")))?;
                Self::collect_changed_paths(&parent_tree, &commit_tree, &mut paths, &mut seen)?;
            }
        }
        Ok(paths)
    }

    fn commit_subjects(
        &self,
        repo: &Path,
        commit_oids: &BTreeSet<String>,
    ) -> Result<BTreeMap<String, String>> {
        if commit_oids.is_empty() {
            return Ok(BTreeMap::new());
        }
        let repository = open(repo)?;
        let mut subjects = BTreeMap::new();
        for requested in commit_oids {
            let Ok(oid) = parse_oid(requested) else {
                continue;
            };
            let Ok(commit) = repository.find_commit(oid) else {
                continue;
            };
            let Ok(message) = commit.message() else {
                continue;
            };
            let summary = message.summary();
            let Ok(subject) = summary.to_str() else {
                continue;
            };
            let subject = subject.trim();
            if !subject.is_empty() {
                subjects.insert(requested.clone(), subject.to_owned());
            }
        }
        Ok(subjects)
    }

    fn for_each_ref(&self, repo: &Path, patterns: &[&str]) -> Result<Vec<RefEntry>> {
        let repository = open(repo)?;
        let platform = repository
            .references()
            .map_err(|error| GitBackendError::new(format!("open ref database: {error}")))?;

        // Empty patterns lists every ref; otherwise each prefix is listed. The
        // union is sorted by refname to match `for-each-ref`'s default order.
        let iterators = if patterns.is_empty() {
            vec![
                platform
                    .all()
                    .map_err(|error| GitBackendError::new(format!("list refs: {error}")))?,
            ]
        } else {
            let mut iters = Vec::with_capacity(patterns.len());
            for pattern in patterns {
                iters.push(platform.prefixed(pattern.as_bytes()).map_err(|error| {
                    GitBackendError::new(format!("list refs for {pattern}: {error}"))
                })?);
            }
            iters
        };

        let mut entries: Vec<RefEntry> = Vec::new();
        for iterator in iterators {
            for reference in iterator {
                let mut reference = reference
                    .map_err(|error| GitBackendError::new(format!("read ref: {error}")))?;
                let name = bstr_to_string(reference.name().as_bstr(), "ref name")?;
                let oid = reference
                    .peel_to_id()
                    .map_err(|error| GitBackendError::new(format!("peel ref {name}: {error}")))?
                    .detach()
                    .to_string();
                entries.push(RefEntry { name, oid });
            }
        }
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(entries)
    }

    fn ref_state_lines(&self, repo: &Path) -> Result<String> {
        let repository = open(repo)?;
        let platform = repository
            .references()
            .map_err(|error| GitBackendError::new(format!("open ref database: {error}")))?;
        let mut lines: Vec<(String, String)> = Vec::new();
        for prefix in ["refs/heads/", "refs/remotes/"] {
            let iterator = platform.prefixed(prefix.as_bytes()).map_err(|error| {
                GitBackendError::new(format!("list refs for {prefix}: {error}"))
            })?;
            for reference in iterator {
                let mut reference = reference
                    .map_err(|error| GitBackendError::new(format!("read ref: {error}")))?;
                let name = bstr_to_string(reference.name().as_bstr(), "ref name")?;
                let symref = match reference.target() {
                    ::gix::refs::TargetRef::Symbolic(target) => {
                        bstr_to_string(target.as_bstr(), "symref target")?
                    }
                    ::gix::refs::TargetRef::Object(_) => String::new(),
                };
                let oid = reference
                    .peel_to_id()
                    .map_err(|error| GitBackendError::new(format!("peel ref {name}: {error}")))?
                    .detach()
                    .to_string();
                lines.push((name.clone(), format!("{oid} {name} {symref}")));
            }
        }
        lines.sort_by(|left, right| left.0.cmp(&right.0));
        let mut text = lines
            .into_iter()
            .map(|(_, line)| line)
            .collect::<Vec<_>>()
            .join("\n");
        if !text.is_empty() {
            text.push('\n');
        }
        Ok(text)
    }

    fn object_exists(&self, repo: &Path, oid: &str) -> Result<bool> {
        let repository = open(repo)?;
        Ok(parse_oid(oid)
            .map(|id| repository.has_object(id))
            .unwrap_or(false))
    }

    fn default_branch_ref(&self, repo: &Path) -> Result<Option<String>> {
        let repository = open(repo)?;
        if let Ok(Some(origin_head)) = repository.try_find_reference("refs/remotes/origin/HEAD")
            && let ::gix::refs::TargetRef::Symbolic(target) = origin_head.target()
        {
            let target = bstr_to_string(target.as_bstr(), "origin/HEAD target")?;
            if Self::reference_resolves_to_commit(&repository, &target) {
                return Ok(Some(target));
            }
        }
        for candidate in ["refs/heads/main", "refs/heads/master"] {
            if Self::reference_resolves_to_commit(&repository, candidate) {
                return Ok(Some(candidate.to_owned()));
            }
        }
        Ok(None)
    }

    fn rev_list_range(&self, repo: &Path, range: &str) -> Result<Vec<String>> {
        if !range.contains("..") {
            return Err(ShoreError::Message(format!(
                "'{range}' is not a commit range; expected the form '<a>..<b>'"
            )));
        }
        let repository = open(repo)?;
        let unresolvable = || {
            ShoreError::Message(format!(
                "cannot resolve commit range '{range}' in this repository"
            ))
        };
        let (base, tip) = range.split_once("..").ok_or_else(unresolvable)?;
        // Strip the extra dot of a symmetric-difference `a...b` off the tip side.
        let tip = tip.strip_prefix('.').unwrap_or(tip);
        let base = if base.is_empty() { "HEAD" } else { base };
        let tip = if tip.is_empty() { "HEAD" } else { tip };

        let tip_id = repository
            .rev_parse_single(tip)
            .map_err(|_| unresolvable())?
            .detach();
        let base_id = repository
            .rev_parse_single(base)
            .map_err(|_| unresolvable())?
            .detach();

        let walk = repository
            .rev_walk([tip_id])
            .with_hidden([base_id])
            .all()
            .map_err(|_| unresolvable())?;
        let mut oids = Vec::new();
        for info in walk {
            let info = info.map_err(|error| {
                ShoreError::Message(format!("walk commit range '{range}': {error}"))
            })?;
            oids.push(info.id.to_string());
        }
        Ok(oids)
    }

    fn rev_list_reachable(&self, repo: &Path, tips: &[String]) -> Result<HashSet<String>> {
        if tips.is_empty() {
            return Ok(HashSet::new());
        }
        let repository = open(repo)?;
        let tip_ids = tips
            .iter()
            .map(|tip| parse_oid(tip))
            .collect::<GixResult<Vec<_>>>()?;
        let walk = repository
            .rev_walk(tip_ids)
            .all()
            .map_err(|error| GitBackendError::new(format!("walk reachable commits: {error}")))?;
        let mut reachable = HashSet::new();
        for info in walk {
            let info =
                info.map_err(|error| GitBackendError::new(format!("walk reachable: {error}")))?;
            reachable.insert(info.id.to_string());
        }
        Ok(reachable)
    }

    fn rev_list_reflog_reachable(&self, repo: &Path) -> Result<HashSet<String>> {
        let repository = open(repo)?;
        let platform = repository
            .references()
            .map_err(|error| GitBackendError::new(format!("open ref database: {error}")))?;
        // Gather every reflog entry tip across all refs, then walk from them —
        // the set `rev-list --reflog` reports. A repository with no reflogs yields
        // an empty set, the truthful "nothing is reflog-retained".
        let mut tips: Vec<::gix::ObjectId> = Vec::new();
        let all = platform
            .all()
            .map_err(|error| GitBackendError::new(format!("list refs: {error}")))?;
        for reference in all {
            let reference =
                reference.map_err(|error| GitBackendError::new(format!("read ref: {error}")))?;
            let mut log = reference.log_iter();
            if let Ok(Some(entries)) = log.all() {
                for entry in entries {
                    let Ok(entry) = entry else { continue };
                    // The forward reflog iterator yields raw hex; parse to an oid.
                    if let Ok(oid) = ::gix::ObjectId::from_hex(entry.new_oid) {
                        tips.push(oid);
                    }
                }
            }
        }
        if tips.is_empty() {
            return Ok(HashSet::new());
        }
        let walk = repository
            .rev_walk(tips)
            .all()
            .map_err(|error| GitBackendError::new(format!("walk reflog-reachable: {error}")))?;
        let mut reachable = HashSet::new();
        for info in walk {
            let info =
                info.map_err(|error| GitBackendError::new(format!("walk reflog: {error}")))?;
            reachable.insert(info.id.to_string());
        }
        Ok(reachable)
    }

    fn reflog_entries(&self, repo: &Path, ref_name: &str) -> Result<Vec<GitReflogEntry>> {
        let repository = open(repo)?;
        let Ok(Some(reference)) = repository.try_find_reference(ref_name) else {
            return Ok(Vec::new());
        };
        let mut log = reference.log_iter();
        // Newest first, matching `git log -g`.
        let Ok(Some(entries)) = log.rev() else {
            return Ok(Vec::new());
        };
        let mut result = Vec::new();
        for entry in entries {
            let entry = entry
                .map_err(|error| GitBackendError::new(format!("read reflog entry: {error}")))?;
            result.push(GitReflogEntry {
                new_oid: entry.new_oid.to_string(),
                subject: entry.message.to_str_lossy().trim().to_owned(),
            });
        }
        Ok(result)
    }

    fn worktree_list(&self, repo: &Path) -> Result<Vec<GitWorktree>> {
        let repository = open(repo)?;
        let mut worktrees = Vec::new();

        // The main worktree row (git lists it first).
        if let Some(workdir) = repository.workdir() {
            let head_ref = repository.head_ref().ok().flatten();
            let (branch, detached) = match head_ref {
                Some(reference) => (
                    Some(bstr_to_string(
                        reference.name().as_bstr(),
                        "worktree branch",
                    )?),
                    false,
                ),
                None => (None, true),
            };
            worktrees.push(GitWorktree {
                // git's porcelain prints the canonicalized worktree path.
                path: canonicalize(workdir).unwrap_or_else(|_| workdir.to_path_buf()),
                head: repository.head_id().ok().map(|id| id.to_string()),
                branch,
                detached,
                bare: false,
            });
        }

        for proxy in repository
            .worktrees()
            .map_err(|error| GitBackendError::new(format!("list linked worktrees: {error}")))?
        {
            let base = proxy
                .base()
                .map_err(|error| GitBackendError::new(format!("resolve worktree base: {error}")))?;
            let linked = proxy.into_repo_with_possibly_inaccessible_worktree().ok();
            let (head, branch, detached) = match linked {
                Some(linked) => {
                    let head_ref = linked.head_ref().ok().flatten();
                    let (branch, detached) = match head_ref {
                        Some(reference) => (
                            Some(bstr_to_string(
                                reference.name().as_bstr(),
                                "worktree branch",
                            )?),
                            false,
                        ),
                        None => (None, true),
                    };
                    (
                        linked.head_id().ok().map(|id| id.to_string()),
                        branch,
                        detached,
                    )
                }
                None => (None, None, false),
            };
            worktrees.push(GitWorktree {
                path: canonicalize(&base).unwrap_or(base),
                head,
                branch,
                detached,
                bare: false,
            });
        }
        Ok(worktrees)
    }

    fn paths_are_ignored(&self, repo: &Path, pathspecs: &[&str]) -> Result<Vec<bool>> {
        if pathspecs.is_empty() {
            return Ok(Vec::new());
        }
        let repository = open(repo)?;
        let worktree = repository.worktree().ok_or_else(|| {
            GitBackendError::new(format!(
                "{} has no worktree to check ignores against",
                repo.display()
            ))
        })?;
        // A fresh exclude stack per call, so any ignore-source mutation earlier in
        // the process (info/exclude append or a committed .gitignore write) is
        // observed here (LB-5 epoch rule).
        let mut stack = worktree
            .excludes(None)
            .map_err(|error| GitBackendError::new(format!("open exclude stack: {error}")))?;
        let mut verdicts = Vec::with_capacity(pathspecs.len());
        for pathspec in pathspecs {
            let platform = stack.at_path(pathspec, None).map_err(|error| {
                GitBackendError::new(format!("check ignore for {pathspec}: {error}"))
            })?;
            verdicts.push(platform.is_excluded());
        }
        Ok(verdicts)
    }

    fn untracked_inventory(&self, repo: &Path) -> Result<Vec<GitInventoryPath>> {
        let repository = open(repo)?;
        let paths = dirwalk_untracked_paths(&repository)?;
        Ok(paths
            .into_iter()
            .map(|bytes| GitInventoryPath::new(&bytes))
            .collect())
    }

    fn tracked_and_untracked_inventory(&self, repo: &Path) -> Result<Vec<GitInventoryPath>> {
        let repository = open(repo)?;
        // Tracked paths from the index, plus untracked paths from the dirwalk —
        // the composition git prints for `ls-files -co`.
        let mut paths: Vec<Vec<u8>> = Vec::new();
        let index = repository
            .index()
            .map_err(|error| GitBackendError::new(format!("open index: {error}")))?;
        for entry in index.entries() {
            paths.push(entry.path(&index).to_vec());
        }
        paths.extend(dirwalk_untracked_paths(&repository)?);
        paths.sort();
        paths.dedup();
        Ok(paths
            .into_iter()
            .map(|bytes| GitInventoryPath::new(&bytes))
            .collect())
    }

    fn path_is_untracked(&self, repo: &Path, relative_path: &str) -> Result<bool> {
        let repository = open(repo)?;
        let untracked = dirwalk_untracked_paths(&repository)?;
        Ok(untracked
            .iter()
            .any(|bytes| bytes.as_slice() == relative_path.as_bytes()))
    }

    fn config_get(&self, repo: &Path, key: &str) -> Option<String> {
        let repository = open(repo).ok()?;
        let snapshot = repository.config_snapshot();
        let value = snapshot.string(key)?;
        let text = value.to_str_lossy().trim().to_owned();
        (!text.is_empty()).then_some(text)
    }

    fn config_path_get(&self, repo: &Path, key: &str) -> Option<String> {
        let repository = open(repo).ok()?;
        let snapshot = repository.config_snapshot();
        let path = snapshot.trusted_path(key)?.ok()?;
        let text = path.to_string_lossy().trim().to_owned();
        (!text.is_empty()).then_some(text)
    }

    fn head_ref(&self, repo: &Path) -> Result<Option<String>> {
        let repository = open(repo)?;
        let head = repository
            .head()
            .map_err(|error| GitBackendError::new(format!("read HEAD: {error}")))?;
        match head.referent_name() {
            Some(name) => Ok(Some(bstr_to_string(name.as_bstr(), "HEAD symbolic ref")?)),
            None => Ok(None),
        }
    }

    fn head_oid(&self, repo: &Path) -> Result<String> {
        let repository = open(repo)?;
        let id = repository
            .head_id()
            .map_err(|error| GitBackendError::new(format!("resolve HEAD oid: {error}")))?;
        Ok(id.to_string())
    }

    fn head_commit_oid_optional(&self, repo: &Path) -> Result<Option<String>> {
        let repository = open(repo)?;
        let head = repository
            .head()
            .map_err(|error| GitBackendError::new(format!("read HEAD: {error}")))?;
        Ok(head.id().map(|id| id.to_string()))
    }

    fn rev_parse_commit_oid(&self, repo: &Path, rev: &str) -> Result<String> {
        let repository = open(repo)?;
        let cannot_resolve = || {
            ShoreError::Message(format!(
                "cannot resolve '{rev}' to a commit in this repository"
            ))
        };
        let object = repository
            .rev_parse_single(format!("{rev}^{{commit}}").as_str())
            .map_err(|_| cannot_resolve())?;
        Ok(object.detach().to_string())
    }

    fn commit_tree_oid(&self, repo: &Path, commit_oid: &str) -> Result<String> {
        let repository = open(repo)?;
        let cannot_resolve = || {
            ShoreError::Message(format!(
                "cannot resolve '{commit_oid}' to a tree in this repository"
            ))
        };
        let oid = parse_oid(commit_oid).map_err(|_| cannot_resolve())?;
        let commit = repository.find_commit(oid).map_err(|_| cannot_resolve())?;
        let tree_id = commit.tree_id().map_err(|_| cannot_resolve())?;
        Ok(tree_id.detach().to_string())
    }

    fn empty_tree_oid(&self, repo: &Path) -> Result<String> {
        let repository = open(repo)?;
        Ok(::gix::ObjectId::empty_tree(repository.object_hash()).to_string())
    }
}

/// Every non-ignored untracked path in `repository`, as raw bytes in sorted
/// order — the `ls-files --others --exclude-standard` set. Byte paths preserve
/// non-UTF-8 filenames.
fn dirwalk_untracked_paths(repository: &::gix::Repository) -> GixResult<Vec<Vec<u8>>> {
    use ::gix::dir::entry::Status;
    use ::gix::dir::walk::EmissionMode;

    let index = repository
        .index()
        .map_err(|error| GitBackendError::new(format!("open index: {error}")))?;
    let options = repository
        .dirwalk_options()
        .map_err(|error| GitBackendError::new(format!("dirwalk options: {error}")))?
        .emit_untracked(EmissionMode::Matching)
        .emit_ignored(None)
        .emit_tracked(false)
        .emit_pruned(false)
        .emit_empty_directories(false);
    let iter = repository
        .dirwalk_iter(index, None::<&str>, Default::default(), options)
        .map_err(|error| GitBackendError::new(format!("walk worktree: {error}")))?;

    let mut paths = Vec::new();
    for item in iter {
        let item = item.map_err(|error| GitBackendError::new(format!("walk entry: {error}")))?;
        if item.entry.status == Status::Untracked {
            paths.push(item.entry.rela_path.to_vec());
        }
    }
    paths.sort();
    Ok(paths)
}

#[cfg(all(test, feature = "gix"))]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::git::backend::subprocess::{SubprocessBackend, run_git};

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
        fs::write(dir.path().join("file.txt"), "one\n").unwrap();
        run_git(dir.path(), ["add", "--all"]).unwrap();
        run_git(dir.path(), ["commit", "-m", "first"]).unwrap();
        dir
    }

    #[test]
    fn gix_reads_match_subprocess_on_a_simple_repo() {
        let repo = init_repo();
        let gix = GixBackend;
        let subprocess = SubprocessBackend;

        assert_eq!(
            gix.head_oid(repo.path()).unwrap(),
            subprocess.head_oid(repo.path()).unwrap()
        );
        assert_eq!(
            gix.for_each_ref(repo.path(), &["refs/heads/"]).unwrap(),
            subprocess
                .for_each_ref(repo.path(), &["refs/heads/"])
                .unwrap()
        );
        assert_eq!(
            gix.empty_tree_oid(repo.path()).unwrap(),
            subprocess.empty_tree_oid(repo.path()).unwrap()
        );
        let head = gix.head_oid(repo.path()).unwrap();
        assert!(gix.object_exists(repo.path(), &head).unwrap());
        assert_eq!(
            gix.head_ref(repo.path()).unwrap(),
            subprocess.head_ref(repo.path()).unwrap()
        );
    }
}
