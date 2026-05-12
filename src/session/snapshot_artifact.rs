use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::canonical_hash::sha256_json_hex;
use crate::error::{Result, ShoreError};
use crate::git::git_worktree_root;
use crate::model::{DiffSnapshot, ReviewEndpoint, ReviewUnitId, ReviewUnitSource, SnapshotId};
use crate::session::ReviewUnitFingerprint;
use crate::storage::{CreateFileOutcome, Durability, LocalStorage};

const SNAPSHOT_ARTIFACT_SCHEMA: &str = "shore.snapshot";
const SNAPSHOT_ARTIFACT_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotArtifact {
    pub schema: String,
    pub version: u32,
    pub review_unit_id: ReviewUnitId,
    pub source: ReviewUnitSource,
    pub base: ReviewEndpoint,
    pub target: ReviewEndpoint,
    pub snapshot: DiffSnapshot,
    pub content_hash: String,
}

pub fn write_snapshot_artifact(
    repo: impl AsRef<Path>,
    fingerprint: &ReviewUnitFingerprint,
    snapshot: DiffSnapshot,
) -> Result<SnapshotArtifact> {
    if snapshot.snapshot_id != fingerprint.snapshot_id {
        return Err(ShoreError::Message(format!(
            "snapshot id {} does not match review unit fingerprint {}",
            snapshot.snapshot_id.as_str(),
            fingerprint.snapshot_id.as_str()
        )));
    }

    let artifact = SnapshotArtifact {
        schema: SNAPSHOT_ARTIFACT_SCHEMA.to_owned(),
        version: SNAPSHOT_ARTIFACT_VERSION,
        review_unit_id: fingerprint.review_unit_id.clone(),
        source: fingerprint.source.clone(),
        base: fingerprint.base.clone(),
        target: fingerprint.target.clone(),
        content_hash: format!("sha256:{}", sha256_json_hex(&snapshot)?),
        snapshot,
    };
    let worktree_root = git_worktree_root(repo.as_ref())?;
    let shore_dir = worktree_root.join(".shore");
    let storage = LocalStorage::new(&shore_dir);
    let path = snapshot_artifact_path(&shore_dir, &artifact.snapshot.snapshot_id);
    let bytes = serde_json::to_vec(&artifact)?;

    match storage.create_file_exclusive(&path, &bytes, Durability::Durable)? {
        CreateFileOutcome::Created => Ok(artifact),
        CreateFileOutcome::AlreadyExists => {
            let existing: SnapshotArtifact = storage.read_json(&path)?;
            if existing == artifact {
                Ok(existing)
            } else {
                Err(ShoreError::Message(format!(
                    "snapshot artifact conflict for {}",
                    artifact.snapshot.snapshot_id.as_str()
                )))
            }
        }
    }
}

pub fn read_snapshot_artifact(
    repo: impl AsRef<Path>,
    snapshot_id: &SnapshotId,
) -> Result<SnapshotArtifact> {
    let worktree_root = git_worktree_root(repo.as_ref())?;
    let shore_dir = worktree_root.join(".shore");
    let storage = LocalStorage::new(&shore_dir);
    storage.read_json(&snapshot_artifact_path(&shore_dir, snapshot_id))
}

fn snapshot_artifact_path(shore_dir: &Path, snapshot_id: &SnapshotId) -> PathBuf {
    shore_dir
        .join("artifacts/snapshots")
        .join(format!("{}.json", artifact_file_stem(snapshot_id.as_str())))
}

fn artifact_file_stem(id: &str) -> String {
    // Snapshot IDs include a colon-bearing prefix; hashing keeps artifact
    // filenames portable while the artifact body preserves the readable ID.
    crate::canonical_hash::sha256_bytes_hex(id.as_bytes())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use super::*;
    use crate::git::capture_worktree_diff_files;
    use crate::model::{DiffSnapshot, ReviewId};
    use crate::session::{compute_review_unit_fingerprint, read_snapshot_artifact};

    #[test]
    fn write_snapshot_artifact_stores_full_snapshot() {
        let repo = modified_repo();
        let artifact = write_current_snapshot_artifact(&repo);

        let stored = read_snapshot_artifact(repo.path(), &artifact.snapshot.snapshot_id).unwrap();

        assert_eq!(stored.schema, "shore.snapshot");
        assert_eq!(stored.version, 1);
        assert_eq!(stored.snapshot.snapshot_id, artifact.snapshot.snapshot_id);
        assert_eq!(stored.snapshot.files.len(), 1);
        assert_eq!(
            stored.snapshot.files[0].new_path.as_deref(),
            Some("src/lib.rs")
        );
        assert!(!stored.snapshot.files[0].hunks.is_empty());
    }

    #[test]
    fn stored_snapshot_artifact_survives_worktree_drift() {
        let repo = modified_repo();
        let artifact = write_current_snapshot_artifact(&repo);

        repo.write("src/lib.rs", "pub fn value() -> u32 { 99 }\n");
        let stored = read_snapshot_artifact(repo.path(), &artifact.snapshot.snapshot_id).unwrap();

        assert_eq!(
            stored.snapshot.files[0].new_path.as_deref(),
            Some("src/lib.rs")
        );
        assert!(format!("{:?}", stored.snapshot).contains("2"));
        assert!(!format!("{:?}", stored.snapshot).contains("99"));
    }

    fn write_current_snapshot_artifact(repo: &TestRepo) -> SnapshotArtifact {
        let files = capture_worktree_diff_files(repo.path()).unwrap();
        let fingerprint = compute_review_unit_fingerprint(repo.path()).unwrap();
        let snapshot = DiffSnapshot::new(
            ReviewId::new("review:default"),
            fingerprint.snapshot_id.clone(),
            files,
        );

        write_snapshot_artifact(repo.path(), &fingerprint, snapshot).unwrap()
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
