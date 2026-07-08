use std::path::Path;

use serde::Serialize;

use crate::canonical_hash::sha256_json_hex;
use crate::error::{Result, ShoreError};
use crate::git::{capture_worktree_diff_files, git_head_oid, git_worktree_root};
use crate::model::{
    CommitRangeCaptureMode, DiffFile, DiffRowKind, EngagementId, FileStatus, ObjectId,
    ReviewEndpoint, RevisionId, RevisionSource, RootCommitCaptureMode, StagedCaptureMode,
    UnstagedCaptureMode, WorktreeCaptureMode, id_prefix,
};
use crate::session::event::GitProvenance;

const FINGERPRINT_SCHEMA: &str = "shore.worktree-fingerprint";
const FINGERPRINT_VERSION: u32 = 1;
const REVISION_IDENTITY_SCHEMA: &str = "shore.revision-identity";
const REVISION_IDENTITY_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeFingerprint {
    pub revision_id: RevisionId,
    pub object_id: ObjectId,
}

/// The resolved identity for one capture: the position revision id (derived
/// from object id + git provenance), the derived engagement grouping hint, and
/// the git provenance (source selector + endpoint pair) that the revision wraps.
/// `object_id` is the content-addressed object id used as the snapshot-artifact
/// storage key; the artifact subsystem retains its "snapshot" naming, only the
/// id semantics are the content-only object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionFingerprint {
    pub revision_id: RevisionId,
    pub object_id: ObjectId,
    pub engagement_id: EngagementId,
    pub source: RevisionSource,
    pub base: ReviewEndpoint,
    pub target: ReviewEndpoint,
}

impl RevisionFingerprint {
    /// The git provenance the revision wraps: the resolved source selector and
    /// endpoint pair. Always present on the live git capture path.
    pub fn git_provenance(&self) -> GitProvenance {
        GitProvenance {
            source: self.source.clone(),
            base: self.base.clone(),
            target: self.target.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedRevisionEndpoints {
    pub source: RevisionSource,
    pub base: ReviewEndpoint,
    pub target: ReviewEndpoint,
}

/// A resolved commit endpoint for one side of a range capture: the commit OID
/// and its tree OID. Callers resolve revs to these in the workflow before
/// fingerprinting; spellings never reach identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedCommitEndpoint {
    pub commit_oid: String,
    pub tree_oid: String,
}

/// A resolved tree endpoint for selectors whose base side is a tree object
/// without a corresponding commit object, such as empty-tree root capture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedTreeEndpoint {
    pub tree_oid: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedIndexEndpoint {
    pub tree_oid: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ResolvedStagedBaseEndpoint {
    Commit(ResolvedCommitEndpoint),
    Tree(ResolvedTreeEndpoint),
}

pub fn capture_worktree_fingerprint(repo: &Path) -> Result<WorktreeFingerprint> {
    let files = capture_worktree_diff_files(repo)?;
    worktree_fingerprint_for_files(repo, &files)
}

#[cfg(test)]
pub fn compute_revision_fingerprint(repo: &Path) -> Result<RevisionFingerprint> {
    let files = capture_worktree_diff_files(repo)?;
    let files = exclude_shore_storage_files(files);
    revision_fingerprint_for_files(repo, &files, &[])
}

pub(crate) fn worktree_fingerprint_for_files(
    repo: &Path,
    files: &[DiffFile],
) -> Result<WorktreeFingerprint> {
    let descriptor = WorktreeFingerprintDescriptor {
        schema: FINGERPRINT_SCHEMA,
        version: FINGERPRINT_VERSION,
        worktree_root: normalized_worktree_root(repo)?,
        base_head: git_head_oid(repo)?,
        files,
    };
    let hash = sha256_json_hex(&descriptor)?;

    Ok(WorktreeFingerprint {
        revision_id: RevisionId::new(format!("{}:worktree:sha256:{hash}", id_prefix::REVISION)),
        object_id: ObjectId::new(format!("{}:git:sha256:{hash}", id_prefix::OBJECT)),
    })
}

pub(crate) fn resolve_combined_worktree_endpoints(
    repo: &Path,
    base: &ResolvedStagedBaseEndpoint,
    include_untracked: bool,
    pathspecs: &[String],
) -> Result<ResolvedRevisionEndpoints> {
    let base = match base {
        ResolvedStagedBaseEndpoint::Commit(base) => ReviewEndpoint::GitCommit {
            commit_oid: base.commit_oid.clone(),
            tree_oid: base.tree_oid.clone(),
        },
        ResolvedStagedBaseEndpoint::Tree(base) => ReviewEndpoint::GitTree {
            tree_oid: base.tree_oid.clone(),
        },
    };
    Ok(ResolvedRevisionEndpoints {
        source: RevisionSource::GitWorktree {
            mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
            include_untracked,
            pathspecs: pathspecs.to_vec(),
        },
        base,
        target: ReviewEndpoint::GitWorkingTree {
            worktree_root: normalized_worktree_root(repo)?,
        },
    })
}

/// Resolve the endpoint pair for a commit-range capture: a `GitCommitRange`
/// source over a `GitCommit` base and a `GitCommit` target. No `GitWorkingTree`
/// is involved, so a range capture is worktree-path-free at the target by
/// construction. The pathspec set is provenance — how the source was addressed,
/// sibling to the capture mode; empty means the whole repository and
/// serializes to nothing.
pub(crate) fn resolve_commit_range_endpoints(
    base: &ResolvedCommitEndpoint,
    target: &ResolvedCommitEndpoint,
    pathspecs: &[String],
) -> ResolvedRevisionEndpoints {
    ResolvedRevisionEndpoints {
        source: RevisionSource::GitCommitRange {
            mode: CommitRangeCaptureMode::BaseTreeToTargetTree,
            pathspecs: pathspecs.to_vec(),
        },
        base: ReviewEndpoint::GitCommit {
            commit_oid: base.commit_oid.clone(),
            tree_oid: base.tree_oid.clone(),
        },
        target: ReviewEndpoint::GitCommit {
            commit_oid: target.commit_oid.clone(),
            tree_oid: target.tree_oid.clone(),
        },
    }
}

pub(crate) fn resolve_root_commit_endpoints(
    base: &ResolvedTreeEndpoint,
    target: &ResolvedCommitEndpoint,
    pathspecs: &[String],
) -> ResolvedRevisionEndpoints {
    ResolvedRevisionEndpoints {
        source: RevisionSource::GitRootCommit {
            mode: RootCommitCaptureMode::EmptyTreeToTargetTree,
            pathspecs: pathspecs.to_vec(),
        },
        base: ReviewEndpoint::GitTree {
            tree_oid: base.tree_oid.clone(),
        },
        target: ReviewEndpoint::GitCommit {
            commit_oid: target.commit_oid.clone(),
            tree_oid: target.tree_oid.clone(),
        },
    }
}

pub(crate) fn resolve_staged_endpoints(
    base: &ResolvedStagedBaseEndpoint,
    target: &ResolvedIndexEndpoint,
    pathspecs: &[String],
) -> ResolvedRevisionEndpoints {
    let base = match base {
        ResolvedStagedBaseEndpoint::Commit(base) => ReviewEndpoint::GitCommit {
            commit_oid: base.commit_oid.clone(),
            tree_oid: base.tree_oid.clone(),
        },
        ResolvedStagedBaseEndpoint::Tree(base) => ReviewEndpoint::GitTree {
            tree_oid: base.tree_oid.clone(),
        },
    };
    ResolvedRevisionEndpoints {
        source: RevisionSource::GitStaged {
            mode: StagedCaptureMode::BaseTreeToIndexTree,
            pathspecs: pathspecs.to_vec(),
        },
        base,
        target: ReviewEndpoint::GitIndex {
            tree_oid: target.tree_oid.clone(),
        },
    }
}

pub(crate) fn resolve_unstaged_endpoints(
    repo: &Path,
    base: &ResolvedIndexEndpoint,
    include_untracked: bool,
    pathspecs: &[String],
) -> Result<ResolvedRevisionEndpoints> {
    Ok(ResolvedRevisionEndpoints {
        source: RevisionSource::GitUnstaged {
            mode: UnstagedCaptureMode::IndexTreeToWorkingTree,
            include_untracked,
            pathspecs: pathspecs.to_vec(),
        },
        base: ReviewEndpoint::GitIndex {
            tree_oid: base.tree_oid.clone(),
        },
        target: ReviewEndpoint::GitWorkingTree {
            worktree_root: normalized_worktree_root(repo)?,
        },
    })
}

#[cfg(test)]
pub(crate) fn revision_fingerprint_for_files(
    repo: &Path,
    files: &[DiffFile],
    pathspecs: &[String],
) -> Result<RevisionFingerprint> {
    let base = resolve_head_or_empty_tree_base(repo)?;
    worktree_revision_fingerprint_for_files(repo, &base, files, true, pathspecs)
}

pub(crate) fn worktree_revision_fingerprint_for_files(
    repo: &Path,
    base: &ResolvedStagedBaseEndpoint,
    files: &[DiffFile],
    include_untracked: bool,
    pathspecs: &[String],
) -> Result<RevisionFingerprint> {
    let endpoints = resolve_combined_worktree_endpoints(repo, base, include_untracked, pathspecs)?;
    revision_fingerprint_from_parts(endpoints, files)
}

/// Range fingerprint over the commit-range source and resolved endpoint pair.
/// The content-only object id converges for identical content; the revision id
/// distinguishes the range from a worktree capture of the same content via its
/// git provenance.
pub(crate) fn commit_range_revision_fingerprint_for_files(
    repo: &Path,
    base: &ResolvedCommitEndpoint,
    target: &ResolvedCommitEndpoint,
    files: &[DiffFile],
    pathspecs: &[String],
) -> Result<RevisionFingerprint> {
    let _ = repo;
    let endpoints = resolve_commit_range_endpoints(base, target, pathspecs);
    revision_fingerprint_from_parts(endpoints, files)
}

pub(crate) fn root_commit_revision_fingerprint_for_files(
    repo: &Path,
    base: &ResolvedTreeEndpoint,
    target: &ResolvedCommitEndpoint,
    files: &[DiffFile],
    pathspecs: &[String],
) -> Result<RevisionFingerprint> {
    let _ = repo;
    let endpoints = resolve_root_commit_endpoints(base, target, pathspecs);
    revision_fingerprint_from_parts(endpoints, files)
}

pub(crate) fn staged_revision_fingerprint_for_files(
    repo: &Path,
    base: &ResolvedStagedBaseEndpoint,
    target: &ResolvedIndexEndpoint,
    files: &[DiffFile],
    pathspecs: &[String],
) -> Result<RevisionFingerprint> {
    let _ = repo;
    let endpoints = resolve_staged_endpoints(base, target, pathspecs);
    revision_fingerprint_from_parts(endpoints, files)
}

pub(crate) fn unstaged_revision_fingerprint_for_files(
    repo: &Path,
    base: &ResolvedIndexEndpoint,
    include_untracked: bool,
    files: &[DiffFile],
    pathspecs: &[String],
) -> Result<RevisionFingerprint> {
    let endpoints = resolve_unstaged_endpoints(repo, base, include_untracked, pathspecs)?;
    revision_fingerprint_from_parts(endpoints, files)
}

#[cfg(test)]
fn resolve_head_or_empty_tree_base(repo: &Path) -> Result<ResolvedStagedBaseEndpoint> {
    if let Some(commit_oid) = crate::git::git_head_commit_oid_optional(repo)? {
        let tree_oid = crate::git::git_commit_tree_oid(repo, &commit_oid)?;
        Ok(ResolvedStagedBaseEndpoint::Commit(ResolvedCommitEndpoint {
            commit_oid,
            tree_oid,
        }))
    } else {
        Ok(ResolvedStagedBaseEndpoint::Tree(ResolvedTreeEndpoint {
            tree_oid: crate::git::git_empty_tree_oid(repo)?,
        }))
    }
}

/// Shared identity tail for every source adapter. Mints the content-only object
/// id, then derives the position revision id from the object id plus the git
/// provenance (succession-independent — never from any successor set), and the
/// engagement grouping hint from the revision (every capture is a root here, so
/// it seeds from its own revision). The repo namespace and git OIDs stay out of
/// the content object id; they live in the provenance the revision wraps.
fn revision_fingerprint_from_parts(
    endpoints: ResolvedRevisionEndpoints,
    files: &[DiffFile],
) -> Result<RevisionFingerprint> {
    let object_id = object_identity(files);
    let provenance = GitProvenance {
        source: endpoints.source.clone(),
        base: endpoints.base.clone(),
        target: endpoints.target.clone(),
    };
    let revision_id = revision_id_from(&object_id, Some(&provenance))?;
    let engagement_id = engagement_id_from_root(&revision_id);

    Ok(RevisionFingerprint {
        revision_id,
        object_id,
        engagement_id,
        source: endpoints.source,
        base: endpoints.base,
        target: endpoints.target,
    })
}

/// Derive a position revision id from a content object id plus its optional git
/// provenance. Succession-independent: a later successor never re-keys this
/// revision. `None` provenance yields a revision over a non-git object.
pub(in crate::session) fn revision_id_from(
    object_id: &ObjectId,
    git_provenance: Option<&GitProvenance>,
) -> Result<RevisionId> {
    let descriptor = RevisionIdentityDescriptor {
        schema: REVISION_IDENTITY_SCHEMA,
        version: REVISION_IDENTITY_VERSION,
        object_id,
        git_provenance,
    };
    let hash = sha256_json_hex(&descriptor)?;
    Ok(RevisionId::new(format!(
        "{}:sha256:{hash}",
        id_prefix::REVISION
    )))
}

/// Derive the engagement grouping hint for a root generative move (empty
/// supersedes): it seeds deterministically from its own revision, so two clones
/// that mint the same revision converge to the same engagement.
pub(in crate::session) fn engagement_id_from_root(revision_id: &RevisionId) -> EngagementId {
    let hash = crate::canonical_hash::sha256_bytes_hex(revision_id.as_str().as_bytes());
    EngagementId::new(format!("{}:sha256:{hash}", id_prefix::ENGAGEMENT))
}

/// Derive a provisional engagement hint for a generative move whose superseded
/// targets are all not-yet-present (dangling). It seeds deterministically from
/// the sorted target ids, so the move groups stably until a target backfills and
/// the read projection self-heals the grouping. `supersedes` must be sorted and
/// deduped by the caller.
pub(in crate::session) fn engagement_id_provisional(supersedes: &[RevisionId]) -> EngagementId {
    let joined = supersedes
        .iter()
        .map(RevisionId::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    let hash = crate::canonical_hash::sha256_bytes_hex(joined.as_bytes());
    EngagementId::new(format!("{}:sha256:{hash}", id_prefix::ENGAGEMENT))
}

#[cfg(test)]
fn exclude_shore_storage_files(files: Vec<DiffFile>) -> Vec<DiffFile> {
    files
        .into_iter()
        .filter(|file| {
            !file.old_path.as_deref().is_some_and(is_shore_storage_path)
                && !file.new_path.as_deref().is_some_and(is_shore_storage_path)
        })
        .collect()
}

#[cfg(test)]
fn is_shore_storage_path(path: &str) -> bool {
    path == ".shore/data" || path.starts_with(".shore/data/")
}

/// Revision identity hashes the content object id plus its optional git
/// provenance, so the revision id is succession-independent (no `supersedes`
/// participates) and convergent for identical content + provenance across
/// clones. Changing the shape bumps `REVISION_IDENTITY_VERSION`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RevisionIdentityDescriptor<'a> {
    schema: &'static str,
    version: u32,
    object_id: &'a ObjectId,
    git_provenance: Option<&'a GitProvenance>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorktreeFingerprintDescriptor<'a> {
    schema: &'static str,
    version: u32,
    worktree_root: String,
    base_head: String,
    files: &'a [DiffFile],
}

const OBJECT_IDENTITY_SCHEMA: &str = "shore.object-identity";
const OBJECT_IDENTITY_VERSION: u32 = 1;

/// Content-only, git-optional, path-sorted object identity.
///
/// Hashes a projection of the diff that keeps only what is intrinsic to the
/// change's content — each file's path, rename source, status, and intrinsic
/// flags, plus every row's kind and text — and drops everything positional or
/// git-derived (blob OIDs, file modes, line and hunk numbers, file/hunk ids,
/// and the local repo namespace). The file set is sorted by path so identity is
/// set-based rather than capture-array-ordered; rows stay in their natural diff
/// order. Two clones of identical content converge to one id, and a change with
/// no git blobs (e.g. a markdown set) is still expressible.
pub(in crate::session) fn object_identity(files: &[DiffFile]) -> ObjectId {
    let mut projected: Vec<ContentOnlyFile<'_>> = files.iter().map(content_only_file).collect();
    projected.sort_by(|a, b| (a.path, a.old_path).cmp(&(b.path, b.old_path)));
    let descriptor = ContentOnlyObject {
        schema: OBJECT_IDENTITY_SCHEMA,
        version: OBJECT_IDENTITY_VERSION,
        files: projected,
    };
    let hash =
        sha256_json_hex(&descriptor).expect("content-only object projection always serializes");
    ObjectId::new(format!("{}:sha256:{hash}", id_prefix::OBJECT))
}

/// Project a `DiffFile` to its content-only identity view: keep the path, rename
/// source, status, and intrinsic flags; flatten every hunk's rows to kind + text;
/// drop blob oids, modes, line/hunk numbers, and file/hunk ids.
fn content_only_file(file: &DiffFile) -> ContentOnlyFile<'_> {
    ContentOnlyFile {
        path: file.new_path.as_deref(),
        old_path: file.old_path.as_deref(),
        status: &file.status,
        is_binary: file.is_binary,
        is_submodule: file.is_submodule,
        is_mode_only: file.is_mode_only,
        synthetic: file.synthetic,
        rows: file
            .hunks
            .iter()
            .flat_map(|hunk| hunk.rows.iter())
            .map(|row| ContentOnlyRow {
                kind: &row.kind,
                text: &row.text,
            })
            .collect(),
    }
}

#[derive(Serialize)]
struct ContentOnlyObject<'a> {
    schema: &'static str,
    version: u32,
    files: Vec<ContentOnlyFile<'a>>,
}

#[derive(Serialize)]
struct ContentOnlyFile<'a> {
    path: Option<&'a str>,
    old_path: Option<&'a str>,
    status: &'a FileStatus,
    is_binary: bool,
    is_submodule: bool,
    is_mode_only: bool,
    synthetic: bool,
    rows: Vec<ContentOnlyRow<'a>>,
}

#[derive(Serialize)]
struct ContentOnlyRow<'a> {
    kind: &'a DiffRowKind,
    text: &'a str,
}

pub(crate) fn normalized_worktree_root(repo: &Path) -> Result<String> {
    let root = git_worktree_root(repo)?;
    let root = root.canonicalize().map_err(|error| {
        ShoreError::Message(format!(
            "canonicalize git worktree root {}: {error}",
            root.display()
        ))
    })?;
    Ok(root.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use super::*;
    use crate::git::{
        capture_commit_range_diff_files, git_commit_tree_oid, git_rev_parse_commit_oid,
    };
    use crate::model::{
        CommitRangeCaptureMode, DiffFile, DiffRow, DiffRowKind, FileId, FileStatus, HunkId,
        ReviewEndpoint, ReviewHunk, RevisionSource,
    };

    fn content_diff_row(
        kind: DiffRowKind,
        old_line: Option<u32>,
        new_line: Option<u32>,
        text: &str,
    ) -> DiffRow {
        DiffRow {
            kind,
            old_line,
            new_line,
            text: text.to_owned(),
        }
    }

    /// Build a single-hunk `DiffFile`. The file id, blob oids, mode, and hunk
    /// numbering are exactly the inputs `object_identity` must ignore, so the
    /// helper lets a test vary them while holding the content (path, status,
    /// row kind + text) fixed.
    #[allow(clippy::too_many_arguments)]
    fn content_file(
        file_id: &str,
        path: &str,
        status: FileStatus,
        old_oid: Option<&str>,
        new_oid: Option<&str>,
        mode: Option<&str>,
        hunk_start: u32,
        rows: Vec<DiffRow>,
    ) -> DiffFile {
        let row_count = rows.len() as u32;
        DiffFile {
            id: FileId::new(file_id),
            status,
            old_path: Some(path.to_owned()),
            new_path: Some(path.to_owned()),
            old_mode: mode.map(str::to_owned),
            new_mode: mode.map(str::to_owned),
            old_oid: old_oid.map(str::to_owned),
            new_oid: new_oid.map(str::to_owned),
            similarity: None,
            is_binary: false,
            is_submodule: false,
            is_mode_only: false,
            synthetic: false,
            metadata_rows: Vec::new(),
            hunks: vec![ReviewHunk {
                id: HunkId::new(format!("{file_id}#hunk")),
                header: format!("@@ -{hunk_start} +{hunk_start} @@"),
                old_start: hunk_start,
                old_lines: row_count,
                new_start: hunk_start,
                new_lines: row_count,
                rows,
            }],
        }
    }

    fn clone_a_files() -> Vec<DiffFile> {
        vec![content_file(
            "src/lib.rs",
            "src/lib.rs",
            FileStatus::Modified,
            Some("aaa0001"),
            Some("bbb0002"),
            Some("100644"),
            1,
            vec![content_diff_row(
                DiffRowKind::Added,
                None,
                Some(1),
                "pub fn value() -> u32 { 2 }",
            )],
        )]
    }

    fn clone_b_files() -> Vec<DiffFile> {
        // Identical content, a different clone: different blob oids, file mode,
        // file/hunk ids, and line/hunk numbers — every input object identity
        // must drop. Only path, status, and the row's kind + text are shared.
        vec![content_file(
            "fileid:other-clone",
            "src/lib.rs",
            FileStatus::Modified,
            Some("ccc0003"),
            Some("ddd0004"),
            Some("100755"),
            41,
            vec![content_diff_row(
                DiffRowKind::Added,
                None,
                Some(99),
                "pub fn value() -> u32 { 2 }",
            )],
        )]
    }

    fn sample_files() -> Vec<DiffFile> {
        vec![
            content_file(
                "a",
                "src/a.rs",
                FileStatus::Modified,
                Some("o1"),
                Some("n1"),
                Some("100644"),
                1,
                vec![content_diff_row(
                    DiffRowKind::Added,
                    None,
                    Some(1),
                    "let a = 1;",
                )],
            ),
            content_file(
                "b",
                "src/b.rs",
                FileStatus::Added,
                None,
                Some("n2"),
                Some("100644"),
                1,
                vec![content_diff_row(
                    DiffRowKind::Added,
                    None,
                    Some(1),
                    "let b = 2;",
                )],
            ),
        ]
    }

    fn markdown_set_files() -> Vec<DiffFile> {
        // No git blobs, no modes: a pure content set must still hash to a stable id.
        vec![content_file(
            "NOTES.md",
            "NOTES.md",
            FileStatus::Added,
            None,
            None,
            None,
            1,
            vec![content_diff_row(
                DiffRowKind::Added,
                None,
                Some(1),
                "# Notes",
            )],
        )]
    }

    #[test]
    fn object_identity_converges_for_two_clones() {
        // Identical content under different namespaces / blob oids -> one id.
        assert_eq!(
            object_identity(&clone_a_files()),
            object_identity(&clone_b_files())
        );
    }

    #[test]
    fn object_identity_is_path_set_based_not_array_ordered() {
        let mut files = sample_files();
        let forward = object_identity(&files);
        files.reverse();
        let reversed = object_identity(&files);
        assert_eq!(forward, reversed);
    }

    #[test]
    fn object_identity_expresses_a_non_git_markdown_set() {
        let id = object_identity(&markdown_set_files());
        assert!(
            id.as_str().starts_with("obj:"),
            "expected obj: prefix, got {}",
            id.as_str()
        );
    }

    #[test]
    fn object_identity_distinguishes_different_content() {
        // Converse guard: identity is content-sensitive, not vacuously constant.
        assert_ne!(
            object_identity(&clone_a_files()),
            object_identity(&markdown_set_files())
        );
    }

    #[test]
    fn scoped_worktree_endpoints_record_the_pathspec_set_in_the_source() {
        let repo = modified_repo();
        let pathspecs = vec!["src".to_owned()];
        let base = resolved_staged_base(repo.path());

        let endpoints =
            resolve_combined_worktree_endpoints(repo.path(), &base, false, &pathspecs).unwrap();

        let RevisionSource::GitWorktree {
            pathspecs: recorded,
            ..
        } = endpoints.source
        else {
            panic!("expected a worktree source");
        };
        assert_eq!(recorded, pathspecs);
    }

    #[test]
    fn scoped_range_endpoints_record_the_pathspec_set_in_the_source() {
        let repo = committed_repo();
        let base = resolved_endpoint(repo.path(), "HEAD~1");
        let target = resolved_endpoint(repo.path(), "HEAD");
        let pathspecs = vec!["src".to_owned()];

        let endpoints = resolve_commit_range_endpoints(&base, &target, &pathspecs);

        let RevisionSource::GitCommitRange {
            pathspecs: recorded,
            ..
        } = endpoints.source
        else {
            panic!("expected a commit-range source");
        };
        assert_eq!(recorded, pathspecs);
    }

    #[test]
    fn pathspec_scope_forks_the_revision_id_but_not_the_object_id() {
        // Identity layering: identical captured content under a different
        // declared scope is the same content object at a different revision
        // position — the same layering as worktree-vs-range provenance.
        let repo = modified_repo();
        let files = capture_worktree_diff_files(repo.path()).unwrap();

        let unscoped = revision_fingerprint_for_files(repo.path(), &files, &[]).unwrap();
        let scoped =
            revision_fingerprint_for_files(repo.path(), &files, &["src".to_owned()]).unwrap();
        let scoped_other =
            revision_fingerprint_for_files(repo.path(), &files, &["docs".to_owned()]).unwrap();

        assert_eq!(unscoped.object_id, scoped.object_id);
        assert_ne!(unscoped.revision_id, scoped.revision_id);
        assert_ne!(scoped.revision_id, scoped_other.revision_id);
    }

    #[test]
    fn equal_pathspec_sets_mint_equal_revision_ids() {
        let repo = modified_repo();
        let files = capture_worktree_diff_files(repo.path()).unwrap();
        let scope = vec!["src".to_owned()];

        let first = revision_fingerprint_for_files(repo.path(), &files, &scope).unwrap();
        let second = revision_fingerprint_for_files(repo.path(), &files, &scope).unwrap();

        assert_eq!(first.revision_id, second.revision_id);
        assert_eq!(first.engagement_id, second.engagement_id);
    }

    #[test]
    fn combined_worktree_capture_resolves_head_commit_and_tree() {
        let repo = modified_repo();
        let base = resolved_staged_base(repo.path());

        let endpoints =
            resolve_combined_worktree_endpoints(repo.path(), &base, false, &[]).unwrap();

        assert!(matches!(
            endpoints.source,
            RevisionSource::GitWorktree { .. }
        ));
        match endpoints.base {
            ReviewEndpoint::GitCommit {
                commit_oid,
                tree_oid,
            } => {
                assert!(!commit_oid.is_empty());
                assert!(!tree_oid.is_empty());
                assert_ne!(commit_oid, tree_oid);
            }
            other => panic!("unexpected base endpoint: {other:?}"),
        }
        match endpoints.target {
            ReviewEndpoint::GitWorkingTree { worktree_root } => {
                assert_eq!(
                    worktree_root,
                    repo.path().canonicalize().unwrap().to_string_lossy()
                );
            }
            other => panic!("unexpected target endpoint: {other:?}"),
        }
    }

    #[test]
    fn revision_id_is_stable_for_same_captured_snapshot() {
        let repo = modified_repo();
        let first = compute_revision_fingerprint(repo.path()).unwrap();
        let second = compute_revision_fingerprint(repo.path()).unwrap();

        assert_eq!(first.object_id, second.object_id);
        assert_eq!(first.revision_id, second.revision_id);
        assert_eq!(first.engagement_id, second.engagement_id);
    }

    #[test]
    fn tracked_or_untracked_content_changes_revision_id() {
        let repo = modified_repo();
        let first = compute_revision_fingerprint(repo.path()).unwrap();

        repo.write("new.txt", "new untracked file\n");
        let second = compute_revision_fingerprint(repo.path()).unwrap();

        assert_ne!(first.object_id, second.object_id);
        assert_ne!(first.revision_id, second.revision_id);
    }

    #[test]
    fn shore_directory_is_excluded_from_revision_identity() {
        let repo = modified_repo();
        let first = compute_revision_fingerprint(repo.path()).unwrap();

        fs::create_dir_all(repo.path().join(".shore/data/events")).unwrap();
        fs::write(repo.path().join(".shore/data/events/noise.json"), "{}").unwrap();
        let second = compute_revision_fingerprint(repo.path()).unwrap();

        assert_eq!(first.revision_id, second.revision_id);
    }

    #[test]
    fn excludes_nested_shore_data_storage_paths() {
        assert!(is_shore_storage_path(".shore/data"));
        assert!(is_shore_storage_path(".shore/data/events/abc.json"));
        assert!(is_shore_storage_path(".shore/data/state.json"));
        // The bare .shore/ dir and the pre-migration flat store are NOT the
        // storage path.
        assert!(!is_shore_storage_path(".shore"));
        assert!(!is_shore_storage_path(".shore/events/abc.json"));
        // Committed config siblings under .shore/ are NOT store storage and must
        // stay visible in review fingerprints (a delegates.json edit is a real
        // reviewable change).
        assert!(!is_shore_storage_path(".shore/delegates.json"));
        assert!(!is_shore_storage_path(".shore/allowed-signers.json"));
    }

    #[test]
    fn worktree_capture_revision_id_distinguishes_repos_but_object_converges() {
        // A worktree capture's provenance includes its working-tree path, so two
        // repos with identical content share one content object but mint distinct
        // revisions. The object id is the content-only identity.
        let repo_a = modified_repo();
        let repo_b = modified_repo();

        let first = compute_revision_fingerprint(repo_a.path()).unwrap();
        let second = compute_revision_fingerprint(repo_b.path()).unwrap();

        assert_ne!(
            repo_a.path().canonicalize().unwrap(),
            repo_b.path().canonicalize().unwrap()
        );
        assert_eq!(first.object_id, second.object_id);
        assert_ne!(first.revision_id, second.revision_id);
    }

    #[test]
    fn object_identity_drops_the_repo_namespace_and_git_oids() {
        // The content object never folds the worktree path or git blob oids: a
        // worktree change's object id is purely its diff content.
        let id = object_identity(&clone_a_files());
        assert!(id.as_str().starts_with("obj:sha256:"));
    }

    #[test]
    fn commit_range_endpoints_resolve_to_commit_pair() {
        let repo = committed_repo();
        let base = resolved_endpoint(repo.path(), "HEAD~1");
        let target = resolved_endpoint(repo.path(), "HEAD");

        let endpoints = resolve_commit_range_endpoints(&base, &target, &[]);

        assert!(matches!(
            endpoints.source,
            RevisionSource::GitCommitRange {
                mode: CommitRangeCaptureMode::BaseTreeToTargetTree,
                ..
            }
        ));
        let (base_commit, base_tree) = match endpoints.base {
            ReviewEndpoint::GitCommit {
                commit_oid,
                tree_oid,
            } => (commit_oid, tree_oid),
            other => panic!("unexpected base endpoint: {other:?}"),
        };
        let (target_commit, target_tree) = match endpoints.target {
            ReviewEndpoint::GitCommit {
                commit_oid,
                tree_oid,
            } => (commit_oid, tree_oid),
            other => panic!("unexpected target endpoint: {other:?}"),
        };
        assert!(!base_commit.is_empty() && !base_tree.is_empty());
        assert!(!target_commit.is_empty() && !target_tree.is_empty());
        assert_ne!(base_commit, target_commit);
        assert_ne!(base_tree, target_tree);
    }

    #[test]
    fn root_commit_endpoints_record_tree_base_and_commit_target() {
        let base = ResolvedTreeEndpoint {
            tree_oid: "empty-tree".to_owned(),
        };
        let target = ResolvedCommitEndpoint {
            commit_oid: "target".to_owned(),
            tree_oid: "target-tree".to_owned(),
        };

        let endpoints = resolve_root_commit_endpoints(&base, &target, &[]);

        assert!(matches!(
            endpoints.source,
            RevisionSource::GitRootCommit { .. }
        ));
        assert_eq!(
            endpoints.base,
            ReviewEndpoint::GitTree {
                tree_oid: "empty-tree".to_owned()
            }
        );
        assert_eq!(
            endpoints.target,
            ReviewEndpoint::GitCommit {
                commit_oid: "target".to_owned(),
                tree_oid: "target-tree".to_owned()
            }
        );
    }

    #[test]
    fn root_commit_revision_fingerprint_uses_tree_base_provenance() {
        let repo = committed_repo();
        let base = ResolvedTreeEndpoint {
            tree_oid: "empty-tree".to_owned(),
        };
        let target = resolved_endpoint(repo.path(), "HEAD");
        let files = clone_a_files();

        let fingerprint =
            root_commit_revision_fingerprint_for_files(repo.path(), &base, &target, &files, &[])
                .unwrap();

        assert_eq!(fingerprint.object_id, object_identity(&files));
        assert!(matches!(
            fingerprint.source,
            RevisionSource::GitRootCommit { .. }
        ));
        assert_eq!(
            fingerprint.base,
            ReviewEndpoint::GitTree {
                tree_oid: "empty-tree".to_owned()
            }
        );
        assert!(matches!(
            fingerprint.target,
            ReviewEndpoint::GitCommit { .. }
        ));
    }

    #[test]
    fn commit_range_revision_id_is_stable_for_same_range() {
        let repo = committed_repo();
        let base = resolved_endpoint(repo.path(), "HEAD~1");
        let target = resolved_endpoint(repo.path(), "HEAD");
        let files =
            capture_commit_range_diff_files(repo.path(), &base.commit_oid, &target.commit_oid, &[])
                .unwrap();

        let first =
            commit_range_revision_fingerprint_for_files(repo.path(), &base, &target, &files, &[])
                .unwrap();
        let second =
            commit_range_revision_fingerprint_for_files(repo.path(), &base, &target, &files, &[])
                .unwrap();

        assert_eq!(first.object_id, second.object_id);
        assert_eq!(first.revision_id, second.revision_id);
        assert_eq!(first.engagement_id, second.engagement_id);
    }

    #[test]
    fn commit_range_revision_id_differs_from_worktree_for_identical_rows() {
        // Identical content under different provenance (worktree vs commit range)
        // converges to one content object but mints distinct revisions.
        let repo = committed_repo();
        let base = resolved_endpoint(repo.path(), "HEAD~1");
        let target = resolved_endpoint(repo.path(), "HEAD");
        let files =
            capture_commit_range_diff_files(repo.path(), &base.commit_oid, &target.commit_oid, &[])
                .unwrap();

        let worktree = revision_fingerprint_for_files(repo.path(), &files, &[]).unwrap();
        let range =
            commit_range_revision_fingerprint_for_files(repo.path(), &base, &target, &files, &[])
                .unwrap();

        assert_eq!(worktree.object_id, range.object_id);
        assert_ne!(worktree.revision_id, range.revision_id);
        assert!(worktree.object_id.as_str().starts_with("obj:sha256:"));
        assert!(range.revision_id.as_str().starts_with("rev:sha256:"));
    }

    #[test]
    fn commit_range_revision_id_scopes_to_local_repo_namespace() {
        let repo_a = committed_repo();
        let base_a = resolved_endpoint(repo_a.path(), "HEAD~1");
        let target_a = resolved_endpoint(repo_a.path(), "HEAD");
        let files_a = capture_commit_range_diff_files(
            repo_a.path(),
            &base_a.commit_oid,
            &target_a.commit_oid,
            &[],
        )
        .unwrap();
        let first = commit_range_revision_fingerprint_for_files(
            repo_a.path(),
            &base_a,
            &target_a,
            &files_a,
            &[],
        )
        .unwrap();

        // A real clone preserves commit/tree oids, so the only differing identity
        // input is the canonical worktree root (the V1 source_repo_namespace).
        let repo_b = clone_repo(&repo_a);
        let base_b = resolved_endpoint(repo_b.path(), "HEAD~1");
        let target_b = resolved_endpoint(repo_b.path(), "HEAD");
        let files_b = capture_commit_range_diff_files(
            repo_b.path(),
            &base_b.commit_oid,
            &target_b.commit_oid,
            &[],
        )
        .unwrap();
        let second = commit_range_revision_fingerprint_for_files(
            repo_b.path(),
            &base_b,
            &target_b,
            &files_b,
            &[],
        )
        .unwrap();

        // A real clone preserves commit/tree oids AND content, and a commit-range
        // capture's provenance carries no worktree path, so both clones converge
        // to one revision id — content + provenance are identical.
        assert_eq!(base_a.commit_oid, base_b.commit_oid);
        assert_eq!(target_a.commit_oid, target_b.commit_oid);
        assert_eq!(first.object_id, second.object_id);
        assert_eq!(first.revision_id, second.revision_id);
    }

    fn modified_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        repo
    }

    fn committed_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        repo.commit_all("change");
        repo
    }

    fn resolved_endpoint(repo: &Path, rev: &str) -> ResolvedCommitEndpoint {
        let commit_oid = git_rev_parse_commit_oid(repo, rev).unwrap();
        let tree_oid = git_commit_tree_oid(repo, &commit_oid).unwrap();
        ResolvedCommitEndpoint {
            commit_oid,
            tree_oid,
        }
    }

    fn resolved_staged_base(repo: &Path) -> ResolvedStagedBaseEndpoint {
        ResolvedStagedBaseEndpoint::Commit(resolved_endpoint(repo, "HEAD"))
    }

    fn clone_repo(source: &TestRepo) -> TestRepo {
        let root = tempfile::tempdir().expect("create clone directory");
        let status = Command::new("git")
            .args(["clone", "--quiet"])
            .arg(source.path())
            .arg(root.path())
            .status()
            .expect("run git clone");
        assert!(status.success(), "git clone failed");
        TestRepo { root }
    }

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
                .unwrap_or_else(|error| panic!("run git {:?}: {error}", args));

            assert!(
                output.status.success(),
                "git {:?} failed\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
                args,
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
