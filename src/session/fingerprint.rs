use std::path::Path;

use serde::Serialize;

use crate::canonical_hash::sha256_json_hex;
use crate::error::{Result, ShoreError};
use crate::git::{capture_worktree_diff_files, git_head_oid, git_head_tree_oid, git_worktree_root};
use crate::model::{
    DiffFile, ReviewEndpoint, ReviewUnitId, ReviewUnitSource, RevisionId, SnapshotId,
    WorktreeCaptureMode,
};

const FINGERPRINT_SCHEMA: &str = "shore.worktree-fingerprint";
const FINGERPRINT_VERSION: u32 = 1;
const SNAPSHOT_FINGERPRINT_SCHEMA: &str = "shore.diff-snapshot-fingerprint";
const SNAPSHOT_FINGERPRINT_VERSION: u32 = 1;
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
    pub fn source_repo_namespace(&self) -> &str {
        &self.source_repo_namespace
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedReviewUnitEndpoints {
    pub source: ReviewUnitSource,
    pub base: ReviewEndpoint,
    pub target: ReviewEndpoint,
}

pub fn capture_worktree_fingerprint(repo: &Path) -> Result<WorktreeFingerprint> {
    let files = capture_worktree_diff_files(repo)?;
    worktree_fingerprint_for_files(repo, &files)
}

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

fn exclude_shore_storage_files(files: Vec<DiffFile>) -> Vec<DiffFile> {
    files
        .into_iter()
        .filter(|file| {
            !file.old_path.as_deref().is_some_and(is_shore_storage_path)
                && !file.new_path.as_deref().is_some_and(is_shore_storage_path)
        })
        .collect()
}

fn is_shore_storage_path(path: &str) -> bool {
    path == ".shore" || path.starts_with(".shore/")
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

fn normalized_worktree_root(repo: &Path) -> Result<String> {
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
    use crate::model::{DiffFile, FileId, FileStatus, ReviewEndpoint, ReviewUnitSource};

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

        fs::create_dir_all(repo.path().join(".shore/events")).unwrap();
        fs::write(repo.path().join(".shore/events/noise.json"), "{}").unwrap();
        let second = compute_review_unit_fingerprint(repo.path()).unwrap();

        assert_eq!(first.review_unit_id, second.review_unit_id);
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

    fn modified_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        repo
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
