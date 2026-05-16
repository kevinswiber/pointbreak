use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::canonical_hash::sha256_json_prefixed;
use crate::error::{Result, ShoreError};
use crate::model::{DiffSnapshot, ReviewEndpoint, ReviewUnitId, ReviewUnitSource, SnapshotId};
use crate::session::{ReviewUnitFingerprint, ShoreStorePaths};
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

    let mut artifact = SnapshotArtifact {
        schema: SNAPSHOT_ARTIFACT_SCHEMA.to_owned(),
        version: SNAPSHOT_ARTIFACT_VERSION,
        review_unit_id: fingerprint.review_unit_id.clone(),
        source: fingerprint.source.clone(),
        base: fingerprint.base.clone(),
        target: fingerprint.target.clone(),
        content_hash: String::new(),
        snapshot,
    };
    artifact.content_hash = snapshot_artifact_content_hash(&artifact)?;

    let paths = ShoreStorePaths::resolve(repo.as_ref())?;
    let shore_dir = paths.shore_dir();
    let storage = LocalStorage::new(shore_dir);
    let path = snapshot_artifact_path(shore_dir, &artifact.snapshot.snapshot_id);
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
    let paths = ShoreStorePaths::resolve(repo.as_ref())?;
    let shore_dir = paths.shore_dir();
    let storage = LocalStorage::new(shore_dir);
    let artifact: SnapshotArtifact =
        storage.read_json(&snapshot_artifact_path(shore_dir, snapshot_id))?;
    validate_snapshot_artifact_content_hash(&artifact)?;
    Ok(artifact)
}

fn validate_snapshot_artifact_content_hash(artifact: &SnapshotArtifact) -> Result<()> {
    let expected = snapshot_artifact_content_hash(artifact)?;
    if artifact.content_hash == expected {
        return Ok(());
    }

    Err(ShoreError::Message(format!(
        "snapshot artifact content hash mismatch for {}",
        artifact.snapshot.snapshot_id.as_str()
    )))
}

fn snapshot_artifact_content_hash(artifact: &SnapshotArtifact) -> Result<String> {
    let mut material = serde_json::to_value(artifact)?;
    let Some(object) = material.as_object_mut() else {
        return Err(ShoreError::Message(
            "snapshot artifact hash material must be an object".to_owned(),
        ));
    };
    if object.remove("contentHash").is_none() {
        return Err(ShoreError::Message(
            "snapshot artifact hash material is missing contentHash".to_owned(),
        ));
    }

    sha256_json_prefixed(&material)
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
    use crate::model::{DiffSnapshot, ReviewEndpoint, ReviewId};
    use crate::session::{compute_review_unit_fingerprint, read_snapshot_artifact};

    #[test]
    fn snapshot_artifact_schema_is_pinned_at_shore_snapshot_v1() {
        // Any future elision-aware artifact must bump one of these constants
        // (see docs/adr/adr-0002-large-snapshot-artifact-policy.md, "Future Reversal").
        assert_eq!(super::SNAPSHOT_ARTIFACT_SCHEMA, "shore.snapshot");
        assert_eq!(super::SNAPSHOT_ARTIFACT_VERSION, 1);
    }

    #[test]
    fn captured_text_rows_remain_inline_in_snapshot_artifact() {
        let repo = TestRepo::new();
        repo.write("README.md", "base\n");
        repo.commit_all("base");

        let added = (1..=25).map(|n| format!("line {n}\n")).collect::<String>();
        repo.write("docs/example.md", added);

        let files = capture_worktree_diff_files(repo.path()).unwrap();
        let fingerprint = compute_review_unit_fingerprint(repo.path()).unwrap();
        let snapshot = DiffSnapshot::new(
            ReviewId::new("review:default"),
            fingerprint.snapshot_id.clone(),
            files,
        );
        let artifact = super::write_snapshot_artifact(repo.path(), &fingerprint, snapshot).unwrap();

        let stored = read_snapshot_artifact(repo.path(), &artifact.snapshot.snapshot_id).unwrap();
        let added_file = stored
            .snapshot
            .files
            .iter()
            .find(|f| f.new_path.as_deref() == Some("docs/example.md"))
            .expect("captured added file");

        // V1: every captured row stays inline in the artifact JSON; no elision.
        assert_eq!(added_file.hunks.len(), 1);
        assert_eq!(added_file.hunks[0].rows.len(), 25);
        assert!(added_file.metadata_rows.is_empty());
    }

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

    #[test]
    fn read_snapshot_artifact_rejects_tampered_content() {
        let repo = modified_repo();
        let artifact = write_current_snapshot_artifact(&repo);
        let path =
            snapshot_artifact_path(&repo.path().join(".shore"), &artifact.snapshot.snapshot_id);

        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        json["target"]["worktreeRoot"] = serde_json::json!("/other/repo");
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

        let error = read_snapshot_artifact(repo.path(), &artifact.snapshot.snapshot_id)
            .expect_err("tampered artifact should be rejected");

        assert!(error.to_string().contains("content hash"));
    }

    #[test]
    fn snapshot_artifact_hash_covers_metadata_and_snapshot_rows() {
        let repo = modified_repo();
        let artifact = write_current_snapshot_artifact(&repo);
        let mut changed = artifact.clone();
        changed.target = ReviewEndpoint::GitWorkingTree {
            worktree_root: "/other/repo".to_owned(),
        };

        assert_ne!(
            snapshot_artifact_content_hash(&artifact).unwrap(),
            snapshot_artifact_content_hash(&changed).unwrap()
        );
    }

    #[test]
    fn snapshot_artifact_hash_is_stable_across_json_round_trip() {
        let repo = modified_repo();
        let artifact = write_current_snapshot_artifact(&repo);
        let stored = read_snapshot_artifact(repo.path(), &artifact.snapshot.snapshot_id).unwrap();
        let reparsed: SnapshotArtifact =
            serde_json::from_str(&serde_json::to_string_pretty(&stored).unwrap()).unwrap();

        assert_eq!(
            stored.content_hash,
            snapshot_artifact_content_hash(&stored).unwrap()
        );
        assert_eq!(
            stored.content_hash,
            snapshot_artifact_content_hash(&reparsed).unwrap()
        );
    }

    #[test]
    fn snapshot_artifact_helpers_resolve_shore_dir_from_subdirectory() {
        let repo = modified_repo();
        fs::create_dir_all(repo.path().join("src")).unwrap();
        let files = capture_worktree_diff_files(repo.path()).unwrap();
        let fingerprint = compute_review_unit_fingerprint(repo.path()).unwrap();
        let snapshot = DiffSnapshot::new(
            ReviewId::new("review:default"),
            fingerprint.snapshot_id.clone(),
            files,
        );

        let artifact =
            write_snapshot_artifact(repo.path().join("src"), &fingerprint, snapshot).unwrap();
        let read = read_snapshot_artifact(repo.path().join("src"), &artifact.snapshot.snapshot_id)
            .unwrap();

        assert_eq!(read, artifact);
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
