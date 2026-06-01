use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::error::{Result, ShoreError};
use crate::session::event::ShoreEvent;
use crate::session::store::body_artifact::NoteBodyEnvelope;
use crate::session::store::snapshot_artifact::SnapshotArtifact;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoreInventory {
    pub event_count: usize,
    pub event_bytes: u64,
    pub artifact_count: usize,
    pub artifact_bytes: u64,
    pub total_bytes: u64,
    pub untracked_bytes: Option<u64>,
    pub largest_artifacts: Vec<ArtifactInventoryEntry>,
    pub review_unit_snapshots: Vec<ReviewUnitSnapshotInventory>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtifactInventoryEntry {
    pub artifact_ref: String,
    pub artifact_kind: String,
    pub byte_size: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewUnitSnapshotInventory {
    pub review_unit_id: String,
    pub snapshot_id: String,
    pub artifact_ref: String,
    pub byte_size: u64,
}

pub(crate) fn scan_store_inventory(
    store_dir: &Path,
    worktree_root: Option<&Path>,
) -> Result<StoreInventory> {
    let (event_count, event_bytes) = scan_events(&store_dir.join("events"))?;
    let mut artifact_entries = Vec::new();
    let mut review_unit_snapshots = Vec::new();

    let (snapshot_count, snapshot_bytes) = scan_snapshot_artifacts(
        &store_dir.join("artifacts/snapshots"),
        &mut artifact_entries,
        &mut review_unit_snapshots,
    )?;
    let (note_count, note_bytes) =
        scan_note_artifacts(&store_dir.join("artifacts/notes"), &mut artifact_entries)?;

    artifact_entries.sort_by(|left, right| {
        right
            .byte_size
            .cmp(&left.byte_size)
            .then_with(|| left.artifact_ref.cmp(&right.artifact_ref))
    });
    artifact_entries.truncate(5);
    review_unit_snapshots.sort_by(|left, right| {
        left.review_unit_id
            .cmp(&right.review_unit_id)
            .then_with(|| left.snapshot_id.cmp(&right.snapshot_id))
    });

    let artifact_count = snapshot_count + note_count;
    let artifact_bytes = snapshot_bytes + note_bytes;
    Ok(StoreInventory {
        event_count,
        event_bytes,
        artifact_count,
        artifact_bytes,
        total_bytes: event_bytes + artifact_bytes,
        untracked_bytes: worktree_root.map(git_untracked_bytes).transpose()?,
        largest_artifacts: artifact_entries,
        review_unit_snapshots,
    })
}

fn scan_events(events_dir: &Path) -> Result<(usize, u64)> {
    let mut count = 0;
    let mut bytes = 0;
    for path in list_files(events_dir)? {
        if !is_event_file(&path) {
            continue;
        }
        let contents =
            fs::read(&path).map_err(|error| io_error("read event file", &path, error))?;
        let event: ShoreEvent = serde_json::from_slice(&contents)?;
        event.validate_schema_version()?;
        count += 1;
        bytes += contents.len() as u64;
    }
    Ok((count, bytes))
}

fn scan_snapshot_artifacts(
    snapshots_dir: &Path,
    artifacts: &mut Vec<ArtifactInventoryEntry>,
    snapshots: &mut Vec<ReviewUnitSnapshotInventory>,
) -> Result<(usize, u64)> {
    let mut count = 0;
    let mut bytes = 0;
    for path in list_files(snapshots_dir)? {
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let contents =
            fs::read(&path).map_err(|error| io_error("read snapshot artifact", &path, error))?;
        let artifact: SnapshotArtifact = serde_json::from_slice(&contents)?;
        if artifact.schema != "shore.snapshot" || artifact.version != 1 {
            continue;
        }
        let byte_size = contents.len() as u64;
        let artifact_ref = format!("snapshot:{}", artifact.snapshot.snapshot_id.as_str());
        artifacts.push(ArtifactInventoryEntry {
            artifact_ref: artifact_ref.clone(),
            artifact_kind: "snapshot".to_owned(),
            byte_size,
        });
        snapshots.push(ReviewUnitSnapshotInventory {
            review_unit_id: artifact.review_unit_id.as_str().to_owned(),
            snapshot_id: artifact.snapshot.snapshot_id.as_str().to_owned(),
            artifact_ref,
            byte_size,
        });
        count += 1;
        bytes += byte_size;
    }
    Ok((count, bytes))
}

fn scan_note_artifacts(
    notes_dir: &Path,
    artifacts: &mut Vec<ArtifactInventoryEntry>,
) -> Result<(usize, u64)> {
    let mut count = 0;
    let mut bytes = 0;
    for path in list_files(notes_dir)? {
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let contents =
            fs::read(&path).map_err(|error| io_error("read note artifact", &path, error))?;
        let artifact: NoteBodyEnvelope = serde_json::from_slice(&contents)?;
        if artifact.schema != "shore.note-body" || artifact.version != 1 {
            continue;
        }
        let byte_size = contents.len() as u64;
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| {
                ShoreError::Message(format!(
                    "note artifact has invalid file name: {}",
                    path.display()
                ))
            })?;
        artifacts.push(ArtifactInventoryEntry {
            artifact_ref: format!("note-body:sha256:{stem}"),
            artifact_kind: "note_body".to_owned(),
            byte_size,
        });
        count += 1;
        bytes += byte_size;
    }
    Ok((count, bytes))
}

fn git_untracked_bytes(worktree_root: &Path) -> Result<u64> {
    let args = ["ls-files", "--others", "--exclude-standard", "-z", "--"];
    let output = Command::new("git")
        .args(args)
        .current_dir(worktree_root)
        .output()
        .map_err(|error| ShoreError::Message(format!("run git {:?}: {error}", args)))?;

    if !output.status.success() {
        return Err(ShoreError::GitCommand {
            command: format!("{args:?}"),
            status: output.status.to_string(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    let mut bytes = 0;
    for raw_path in output.stdout.split(|byte| *byte == b'\0') {
        if raw_path.is_empty() {
            continue;
        }
        let relative_path = String::from_utf8(raw_path.to_vec()).map_err(|error| {
            ShoreError::Message(format!("untracked inventory path is not utf-8: {error}"))
        })?;
        let path = worktree_root.join(relative_path);
        let metadata = fs::metadata(&path)
            .map_err(|error| io_error("read untracked file metadata", &path, error))?;
        if metadata.is_file() {
            bytes += metadata.len();
        }
    }
    Ok(bytes)
}

fn list_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io_error("list directory", dir, error)),
    };

    let mut files = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|error| io_error("read directory entry", dir, error))?
            .path();
        if path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn is_event_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.len() == 69 && name.ends_with(".json"))
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> ShoreError {
    ShoreError::Message(format!("{action} {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use tempfile::TempDir;

    use super::*;
    use crate::session::{
        CaptureOptions, ObservationAddOptions, ShoreStorePaths, capture_worktree_review,
        record_observation,
    };

    #[test]
    fn scan_store_inventory_counts_events_artifacts_and_bytes() {
        let repo = TestRepo::new();
        repo.write("README.md", "base\n");
        repo.commit_all("base");
        repo.write("README.md", "changed\n");
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        record_observation(
            ObservationAddOptions::new(repo.path())
                .with_track("agent:inventory")
                .with_title("large body")
                .with_body("x".repeat(4097)),
        )
        .unwrap();

        let shore_dir = ShoreStorePaths::resolve(repo.path())
            .unwrap()
            .shore_dir()
            .to_path_buf();
        let inventory = scan_store_inventory(&shore_dir, Some(repo.path())).unwrap();

        let (event_count, event_bytes) = directory_file_stats(&shore_dir.join("events"));
        let (snapshot_count, snapshot_bytes) =
            directory_file_stats(&shore_dir.join("artifacts/snapshots"));
        let (note_count, note_bytes) = directory_file_stats(&shore_dir.join("artifacts/notes"));

        assert_eq!(inventory.event_count, event_count);
        assert_eq!(inventory.event_bytes, event_bytes);
        assert_eq!(inventory.artifact_count, snapshot_count + note_count);
        assert_eq!(inventory.artifact_bytes, snapshot_bytes + note_bytes);
        assert_eq!(
            inventory.total_bytes,
            event_bytes + snapshot_bytes + note_bytes
        );
        assert_eq!(inventory.untracked_bytes, Some(0));
        assert!(inventory.largest_artifacts.iter().all(|artifact| {
            !artifact.artifact_ref.contains("artifacts/")
                && !artifact.artifact_ref.contains(".shore")
                && !artifact.artifact_ref.contains("state.json")
        }));
        assert!(inventory.review_unit_snapshots.iter().any(|snapshot| {
            snapshot.review_unit_id == capture.review_unit_id.as_str()
                && snapshot.snapshot_id == capture.snapshot_id.as_str()
                && snapshot.byte_size > 0
        }));
    }

    fn directory_file_stats(dir: &Path) -> (usize, u64) {
        let mut count = 0;
        let mut bytes = 0;
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                count += 1;
                bytes += fs::metadata(path).unwrap().len();
            }
        }
        (count, bytes)
    }

    struct TestRepo {
        root: TempDir,
    }

    impl TestRepo {
        fn new() -> Self {
            let root = TempDir::new().expect("create temp git repository directory");
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

        fn write(&self, path: &str, contents: &str) {
            let path = self.root.path().join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, contents).unwrap();
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
                .unwrap_or_else(|error| {
                    panic!(
                        "run git {:?} in {}: {error}",
                        args,
                        self.root.path().display()
                    )
                });
            assert!(
                output.status.success(),
                "git {:?} failed in {}\nstdout:\n{}\nstderr:\n{}",
                args,
                self.root.path().display(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
