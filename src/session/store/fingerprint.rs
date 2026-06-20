use std::path::Path;

use serde::Serialize;

use crate::canonical_hash::sha256_json_hex;
use crate::error::{Result, ShoreError};
use crate::git::{capture_worktree_diff_files, git_head_oid, git_head_tree_oid, git_worktree_root};
use crate::model::{
    CommitRangeCaptureMode, DiffFile, ReviewEndpoint, ReviewUnitId, ReviewUnitSource, RevisionId,
    SnapshotId, WorktreeCaptureMode,
};

const FINGERPRINT_SCHEMA: &str = "shore.worktree-fingerprint";
const FINGERPRINT_VERSION: u32 = 1;
const SNAPSHOT_FINGERPRINT_SCHEMA: &str = "shore.diff-snapshot-fingerprint";
const SNAPSHOT_FINGERPRINT_VERSION: u32 = 1;
const COMMIT_RANGE_SNAPSHOT_FINGERPRINT_SCHEMA: &str = "shore.commit-range-snapshot-fingerprint";
const COMMIT_RANGE_SNAPSHOT_FINGERPRINT_VERSION: u32 = 1;
const REVIEW_UNIT_FINGERPRINT_SCHEMA: &str = "shore.review-unit-fingerprint";
const REVIEW_UNIT_FINGERPRINT_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeFingerprint {
    pub revision_id: RevisionId,
    pub snapshot_id: SnapshotId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewUnitFingerprint {
    pub revision_id: RevisionId,
    pub snapshot_id: SnapshotId,
    pub review_unit_id: ReviewUnitId,
    source_repo_namespace: String,
    pub source: ReviewUnitSource,
    pub base: ReviewEndpoint,
    pub target: ReviewEndpoint,
}

impl ReviewUnitFingerprint {
    /// Returns the V1 local source namespace used in the ReviewUnit identity hash.
    ///
    /// V1 uses the canonical worktree root, which is intentionally local-only and
    /// may change when a later repo-namespace model lands.
    #[cfg(test)]
    pub(crate) fn source_repo_namespace(&self) -> &str {
        &self.source_repo_namespace
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedReviewUnitEndpoints {
    pub source: ReviewUnitSource,
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

pub fn capture_worktree_fingerprint(repo: &Path) -> Result<WorktreeFingerprint> {
    let files = capture_worktree_diff_files(repo)?;
    worktree_fingerprint_for_files(repo, &files)
}

#[cfg(test)]
pub fn compute_review_unit_fingerprint(repo: &Path) -> Result<ReviewUnitFingerprint> {
    let files = capture_worktree_diff_files(repo)?;
    let files = exclude_shore_storage_files(files);
    review_unit_fingerprint_for_files(repo, &files)
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
        revision_id: RevisionId::new(format!("rev:worktree:sha256:{hash}")),
        snapshot_id: SnapshotId::new(format!("snap:git:sha256:{hash}")),
    })
}

pub(crate) fn resolve_combined_worktree_endpoints(
    repo: &Path,
) -> Result<ResolvedReviewUnitEndpoints> {
    Ok(ResolvedReviewUnitEndpoints {
        source: ReviewUnitSource::GitWorktree {
            mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
            include_untracked: true,
        },
        base: ReviewEndpoint::GitCommit {
            commit_oid: git_head_oid(repo)?,
            tree_oid: git_head_tree_oid(repo)?,
        },
        target: ReviewEndpoint::GitWorkingTree {
            worktree_root: normalized_worktree_root(repo)?,
        },
    })
}

/// Resolve the endpoint pair for a commit-range capture: a `GitCommitRange`
/// source over a `GitCommit` base and a `GitCommit` target. No `GitWorkingTree`
/// is involved, so a range capture is path-free at the target by construction.
pub(crate) fn resolve_commit_range_endpoints(
    base: &ResolvedCommitEndpoint,
    target: &ResolvedCommitEndpoint,
) -> ResolvedReviewUnitEndpoints {
    ResolvedReviewUnitEndpoints {
        source: ReviewUnitSource::GitCommitRange {
            mode: CommitRangeCaptureMode::BaseTreeToTargetTree,
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

pub(crate) fn review_unit_fingerprint_for_files(
    repo: &Path,
    files: &[DiffFile],
) -> Result<ReviewUnitFingerprint> {
    let snapshot_descriptor = SnapshotFingerprintDescriptor {
        schema: SNAPSHOT_FINGERPRINT_SCHEMA,
        version: SNAPSHOT_FINGERPRINT_VERSION,
        files,
    };
    let snapshot_hash = sha256_json_hex(&snapshot_descriptor)?;
    let snapshot_id = SnapshotId::new(format!("snap:git:sha256:{snapshot_hash}"));
    let endpoints = resolve_combined_worktree_endpoints(repo)?;
    review_unit_fingerprint_from_parts(repo, endpoints, snapshot_hash, snapshot_id)
}

/// Range fingerprint: a new snapshot descriptor that hashes the endpoint tree
/// pair plus the rows, so identical diff content under different endpoints
/// cannot collide at the snapshot-artifact layer (the artifact path is keyed by
/// snapshot id). The ReviewUnit identity reuses the unchanged
/// `ReviewUnitFingerprintDescriptor` with the commit-range source and endpoints.
pub(crate) fn commit_range_review_unit_fingerprint_for_files(
    repo: &Path,
    base: &ResolvedCommitEndpoint,
    target: &ResolvedCommitEndpoint,
    files: &[DiffFile],
) -> Result<ReviewUnitFingerprint> {
    let snapshot_hash = sha256_json_hex(&CommitRangeSnapshotFingerprintDescriptor {
        schema: COMMIT_RANGE_SNAPSHOT_FINGERPRINT_SCHEMA,
        version: COMMIT_RANGE_SNAPSHOT_FINGERPRINT_VERSION,
        base_tree_oid: &base.tree_oid,
        target_tree_oid: &target.tree_oid,
        files,
    })?;
    let snapshot_id = SnapshotId::new(format!("snap:git:sha256:{snapshot_hash}"));
    let endpoints = resolve_commit_range_endpoints(base, target);
    review_unit_fingerprint_from_parts(repo, endpoints, snapshot_hash, snapshot_id)
}

/// Shared review-unit identity tail for every source adapter: assemble the
/// repo-namespace-scoped `ReviewUnitFingerprintDescriptor` over the resolved
/// endpoints and the snapshot id, then format the ids. `snapshot_hash` is the
/// adapter's snapshot content hash and also seeds `revision_id`.
fn review_unit_fingerprint_from_parts(
    repo: &Path,
    endpoints: ResolvedReviewUnitEndpoints,
    snapshot_hash: String,
    snapshot_id: SnapshotId,
) -> Result<ReviewUnitFingerprint> {
    let source_repo_namespace = normalized_worktree_root(repo)?;
    let review_unit_descriptor = ReviewUnitFingerprintDescriptor {
        schema: REVIEW_UNIT_FINGERPRINT_SCHEMA,
        version: REVIEW_UNIT_FINGERPRINT_VERSION,
        source_repo_namespace: &source_repo_namespace,
        source: &endpoints.source,
        base: &endpoints.base,
        target: &endpoints.target,
        snapshot_id: &snapshot_id,
    };
    let review_unit_hash = sha256_json_hex(&review_unit_descriptor)?;

    Ok(ReviewUnitFingerprint {
        revision_id: RevisionId::new(format!("rev:git:sha256:{snapshot_hash}")),
        snapshot_id,
        review_unit_id: ReviewUnitId::new(format!("review-unit:sha256:{review_unit_hash}")),
        source_repo_namespace,
        source: endpoints.source,
        base: endpoints.base,
        target: endpoints.target,
    })
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotFingerprintDescriptor<'a> {
    schema: &'static str,
    version: u32,
    /// Snapshot identity hashes the current `DiffFile` serde shape.
    ///
    /// Changing that shape requires bumping `SNAPSHOT_FINGERPRINT_VERSION`.
    files: &'a [DiffFile],
}

/// Range snapshot identity hashes the endpoint tree pair **in addition** to the
/// rows — unlike the worktree descriptor above, which hashes rows only.
///
/// A range snapshot's identity *is* the diff between two trees, so folding the
/// tree pair into `snapshot_id` keeps distinct ranges distinct: "identical rows,
/// different endpoints" (e.g. capture staged work, commit it, then range-capture
/// the same change; or the same patch on two bases) stay separate snapshots
/// rather than over-deduping to one. (Post-#146 the v2 artifact body no longer
/// embeds the endpoints, so this is about snapshot *identity*, not avoiding an
/// artifact-body conflict.) Changing the `DiffFile` serde shape requires bumping
/// `COMMIT_RANGE_SNAPSHOT_FINGERPRINT_VERSION` (same rule as the worktree one).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CommitRangeSnapshotFingerprintDescriptor<'a> {
    schema: &'static str,
    version: u32,
    base_tree_oid: &'a str,
    target_tree_oid: &'a str,
    files: &'a [DiffFile],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewUnitFingerprintDescriptor<'a> {
    schema: &'static str,
    version: u32,
    source_repo_namespace: &'a str,
    source: &'a ReviewUnitSource,
    base: &'a ReviewEndpoint,
    target: &'a ReviewEndpoint,
    snapshot_id: &'a SnapshotId,
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
        CommitRangeCaptureMode, DiffFile, FileId, FileStatus, ReviewEndpoint, ReviewUnitSource,
    };

    #[test]
    fn combined_worktree_capture_resolves_head_commit_and_tree() {
        let repo = modified_repo();

        let endpoints = resolve_combined_worktree_endpoints(repo.path()).unwrap();

        assert!(matches!(
            endpoints.source,
            ReviewUnitSource::GitWorktree { .. }
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
    fn review_unit_id_is_stable_for_same_captured_snapshot() {
        let repo = modified_repo();
        let first = compute_review_unit_fingerprint(repo.path()).unwrap();
        let second = compute_review_unit_fingerprint(repo.path()).unwrap();

        assert_eq!(first.snapshot_id, second.snapshot_id);
        assert_eq!(first.review_unit_id, second.review_unit_id);
    }

    #[test]
    fn tracked_or_untracked_content_changes_review_unit_id() {
        let repo = modified_repo();
        let first = compute_review_unit_fingerprint(repo.path()).unwrap();

        repo.write("new.txt", "new untracked file\n");
        let second = compute_review_unit_fingerprint(repo.path()).unwrap();

        assert_ne!(first.snapshot_id, second.snapshot_id);
        assert_ne!(first.review_unit_id, second.review_unit_id);
    }

    #[test]
    fn shore_directory_is_excluded_from_review_unit_identity() {
        let repo = modified_repo();
        let first = compute_review_unit_fingerprint(repo.path()).unwrap();

        fs::create_dir_all(repo.path().join(".shore/data/events")).unwrap();
        fs::write(repo.path().join(".shore/data/events/noise.json"), "{}").unwrap();
        let second = compute_review_unit_fingerprint(repo.path()).unwrap();

        assert_eq!(first.review_unit_id, second.review_unit_id);
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
    fn source_repo_namespace_is_local_to_canonical_worktree_root_for_v1() {
        let repo_a = modified_repo();
        let repo_b = modified_repo();

        let first = compute_review_unit_fingerprint(repo_a.path()).unwrap();
        let second = compute_review_unit_fingerprint(repo_b.path()).unwrap();

        assert_ne!(
            repo_a.path().canonicalize().unwrap(),
            repo_b.path().canonicalize().unwrap()
        );
        assert_eq!(first.snapshot_id, second.snapshot_id);
        assert_ne!(first.review_unit_id, second.review_unit_id);
        assert_eq!(
            first.source_repo_namespace(),
            repo_a
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .as_ref()
        );
    }

    #[test]
    fn snapshot_fingerprint_descriptor_pins_diff_file_serde_shape() {
        let file = DiffFile {
            id: FileId::new("src/lib.rs"),
            status: FileStatus::Modified,
            old_path: Some("src/lib.rs".to_owned()),
            new_path: Some("src/lib.rs".to_owned()),
            old_mode: Some("100644".to_owned()),
            new_mode: Some("100644".to_owned()),
            old_oid: Some("abc123".to_owned()),
            new_oid: Some("def456".to_owned()),
            similarity: None,
            is_binary: false,
            is_submodule: false,
            is_mode_only: false,
            synthetic: false,
            metadata_rows: Vec::new(),
            hunks: Vec::new(),
        };
        let descriptor = SnapshotFingerprintDescriptor {
            schema: SNAPSHOT_FINGERPRINT_SCHEMA,
            version: SNAPSHOT_FINGERPRINT_VERSION,
            files: std::slice::from_ref(&file),
        };

        let json = serde_json::to_value(&descriptor).unwrap();
        let file = json["files"][0].as_object().unwrap();

        assert!(file.contains_key("new_path"));
        assert!(file.contains_key("metadata_rows"));
        assert!(!file.contains_key("newPath"));
        assert!(!file.contains_key("metadataRows"));
    }

    #[test]
    fn commit_range_endpoints_resolve_to_commit_pair() {
        let repo = committed_repo();
        let base = resolved_endpoint(repo.path(), "HEAD~1");
        let target = resolved_endpoint(repo.path(), "HEAD");

        let endpoints = resolve_commit_range_endpoints(&base, &target);

        assert!(matches!(
            endpoints.source,
            ReviewUnitSource::GitCommitRange {
                mode: CommitRangeCaptureMode::BaseTreeToTargetTree
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
    fn commit_range_review_unit_id_is_stable_for_same_range() {
        let repo = committed_repo();
        let base = resolved_endpoint(repo.path(), "HEAD~1");
        let target = resolved_endpoint(repo.path(), "HEAD");
        let files =
            capture_commit_range_diff_files(repo.path(), &base.commit_oid, &target.commit_oid)
                .unwrap();

        let first =
            commit_range_review_unit_fingerprint_for_files(repo.path(), &base, &target, &files)
                .unwrap();
        let second =
            commit_range_review_unit_fingerprint_for_files(repo.path(), &base, &target, &files)
                .unwrap();

        assert_eq!(first.snapshot_id, second.snapshot_id);
        assert_eq!(first.revision_id, second.revision_id);
        assert_eq!(first.review_unit_id, second.review_unit_id);
    }

    #[test]
    fn commit_range_snapshot_id_differs_from_worktree_snapshot_id_for_identical_rows() {
        let repo = committed_repo();
        let base = resolved_endpoint(repo.path(), "HEAD~1");
        let target = resolved_endpoint(repo.path(), "HEAD");
        let files =
            capture_commit_range_diff_files(repo.path(), &base.commit_oid, &target.commit_oid)
                .unwrap();

        let worktree = review_unit_fingerprint_for_files(repo.path(), &files).unwrap();
        let range =
            commit_range_review_unit_fingerprint_for_files(repo.path(), &base, &target, &files)
                .unwrap();

        assert_ne!(worktree.snapshot_id, range.snapshot_id);
        assert_ne!(worktree.review_unit_id, range.review_unit_id);
        assert!(
            worktree
                .snapshot_id
                .as_str()
                .starts_with("snap:git:sha256:")
        );
        assert!(range.snapshot_id.as_str().starts_with("snap:git:sha256:"));
        assert!(
            range
                .review_unit_id
                .as_str()
                .starts_with("review-unit:sha256:")
        );
    }

    #[test]
    fn commit_range_review_unit_id_scopes_to_local_repo_namespace() {
        let repo_a = committed_repo();
        let base_a = resolved_endpoint(repo_a.path(), "HEAD~1");
        let target_a = resolved_endpoint(repo_a.path(), "HEAD");
        let files_a = capture_commit_range_diff_files(
            repo_a.path(),
            &base_a.commit_oid,
            &target_a.commit_oid,
        )
        .unwrap();
        let first = commit_range_review_unit_fingerprint_for_files(
            repo_a.path(),
            &base_a,
            &target_a,
            &files_a,
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
        )
        .unwrap();
        let second = commit_range_review_unit_fingerprint_for_files(
            repo_b.path(),
            &base_b,
            &target_b,
            &files_b,
        )
        .unwrap();

        assert_eq!(base_a.commit_oid, base_b.commit_oid);
        assert_eq!(target_a.commit_oid, target_b.commit_oid);
        assert_eq!(first.snapshot_id, second.snapshot_id);
        assert_ne!(first.review_unit_id, second.review_unit_id);
    }

    #[test]
    fn commit_range_snapshot_descriptor_pins_schema_and_tree_pair() {
        let file = DiffFile {
            id: FileId::new("src/lib.rs"),
            status: FileStatus::Modified,
            old_path: Some("src/lib.rs".to_owned()),
            new_path: Some("src/lib.rs".to_owned()),
            old_mode: Some("100644".to_owned()),
            new_mode: Some("100644".to_owned()),
            old_oid: Some("abc123".to_owned()),
            new_oid: Some("def456".to_owned()),
            similarity: None,
            is_binary: false,
            is_submodule: false,
            is_mode_only: false,
            synthetic: false,
            metadata_rows: Vec::new(),
            hunks: Vec::new(),
        };
        let descriptor = CommitRangeSnapshotFingerprintDescriptor {
            schema: COMMIT_RANGE_SNAPSHOT_FINGERPRINT_SCHEMA,
            version: COMMIT_RANGE_SNAPSHOT_FINGERPRINT_VERSION,
            base_tree_oid: "basetree0",
            target_tree_oid: "targettree0",
            files: std::slice::from_ref(&file),
        };

        let json = serde_json::to_value(&descriptor).unwrap();

        assert_eq!(json["schema"], "shore.commit-range-snapshot-fingerprint");
        assert_eq!(json["version"], 1);
        assert_eq!(json["baseTreeOid"], "basetree0");
        assert_eq!(json["targetTreeOid"], "targettree0");
        let file = json["files"][0].as_object().unwrap();
        assert!(file.contains_key("new_path"));
        assert!(file.contains_key("metadata_rows"));
        assert!(!file.contains_key("newPath"));
        assert!(!file.contains_key("metadataRows"));
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
