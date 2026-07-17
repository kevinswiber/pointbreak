use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Result, ShoreError};
use crate::git::git_untracked_inventory;
use crate::model::id_prefix;
use crate::session::event::{EventType, ShoreEvent};
use crate::session::store::body_artifact::NoteBodyEnvelope;
use crate::session::store::object_artifact::ObjectArtifact;

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
    pub revision_objects: Vec<RevisionObjectInventory>,
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
pub(crate) struct RevisionObjectInventory {
    /// The revisions that captured this object, sorted and deduped. Under the
    /// object-scoped artifact one artifact may be shared by several revisions, so
    /// identity is joined from each capture event's `object_id` plus bound object
    /// artifact hash.
    pub revision_ids: Vec<String>,
    pub object_id: String,
    pub artifact_ref: String,
    pub byte_size: u64,
}

pub(crate) fn scan_store_inventory(
    store_dir: &Path,
    worktree_root: Option<&Path>,
) -> Result<StoreInventory> {
    let (event_count, event_bytes) = scan_events(&store_dir.join("events"))?;
    let mut artifact_entries = Vec::new();
    let mut revision_objects = Vec::new();

    let capture_owners = capture_owners_by_snapshot(&store_dir.join("events"))?;
    let (snapshot_count, snapshot_bytes) = scan_object_artifacts(
        &store_dir.join("artifacts/objects"),
        &capture_owners,
        &mut artifact_entries,
        &mut revision_objects,
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
    // One object artifact is one entry now, so sort by object_id alone — this
    // avoids an empty-`revision_ids` edge in the comparator.
    revision_objects.sort_by(|left, right| left.object_id.cmp(&right.object_id));

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
        revision_objects,
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

fn scan_object_artifacts(
    objects_dir: &Path,
    capture_owners: &BTreeMap<(String, String), BTreeSet<String>>,
    artifacts: &mut Vec<ArtifactInventoryEntry>,
    objects: &mut Vec<RevisionObjectInventory>,
) -> Result<(usize, u64)> {
    let mut count = 0;
    let mut bytes = 0;
    for path in list_files(objects_dir)? {
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let contents =
            fs::read(&path).map_err(|error| io_error("read object artifact", &path, error))?;
        let artifact: ObjectArtifact = serde_json::from_slice(&contents)?;
        // Count v2 object artifacts (their body declares schema `shore.object`).
        if artifact.schema != "shore.object" || !matches!(artifact.version, 1 | 2) {
            continue;
        }
        let byte_size = contents.len() as u64;
        let object_id = artifact.snapshot.object_id.as_str().to_owned();
        let artifact_ref = format!("{}:{}", id_prefix::ARTIFACT_OBJECT, artifact.content_hash);
        let revision_ids = capture_owners
            .get(&(object_id.clone(), artifact.content_hash.clone()))
            .map(|ids| ids.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        artifacts.push(ArtifactInventoryEntry {
            artifact_ref: artifact_ref.clone(),
            artifact_kind: "object".to_owned(),
            byte_size,
        });
        objects.push(RevisionObjectInventory {
            revision_ids,
            object_id,
            artifact_ref,
            byte_size,
        });
        count += 1;
        bytes += byte_size;
    }
    Ok((count, bytes))
}

/// Join `(object_id, object_artifact_content_hash) → {revision_ids}` from the
/// capture events. Identity lives in the event log, so this is the inventory's
/// only source for the revisions that bind a particular object artifact. A
/// `BTreeSet` keeps the ids sorted and deduped for deterministic output.
fn capture_owners_by_snapshot(
    events_dir: &Path,
) -> Result<BTreeMap<(String, String), BTreeSet<String>>> {
    let mut owners: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for path in list_files(events_dir)? {
        if !is_event_file(&path) {
            continue;
        }
        let contents =
            fs::read(&path).map_err(|error| io_error("read event file", &path, error))?;
        let event: ShoreEvent = serde_json::from_slice(&contents)?;
        if event.event_type != EventType::WorkObjectProposed {
            continue;
        }
        // The captured revision and content binding ride the payload's tagged work
        // object; the object artifact is joined by object id plus artifact hash.
        let revision = &event.payload["workObject"]["revision"];
        let (Some(object_id), Some(content_hash), Some(revision_id)) = (
            revision["objectId"].as_str(),
            event.payload["workObject"]["objectArtifactContentHash"].as_str(),
            revision["id"].as_str(),
        ) else {
            continue;
        };
        owners
            .entry((object_id.to_owned(), content_hash.to_owned()))
            .or_default()
            .insert(revision_id.to_owned());
    }
    Ok(owners)
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
            artifact_ref: format!("{}:sha256:{stem}", id_prefix::NOTE_BODY),
            artifact_kind: "note_body".to_owned(),
            byte_size,
        });
        count += 1;
        bytes += byte_size;
    }
    Ok((count, bytes))
}

fn git_untracked_bytes(worktree_root: &Path) -> Result<u64> {
    let mut bytes = 0;
    for raw_path in git_untracked_inventory(worktree_root)? {
        let relative_path = raw_path.into_utf8_string("untracked inventory path")?;
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
    use crate::model::{EngagementId, JournalId, RevisionId};
    use crate::session::event::{
        EventTarget, EventType, GitProvenance, Revision, WorkObjectProposal,
        WorkObjectProposedPayload, Writer,
    };
    use crate::session::store::resolution::resolve_store;
    use crate::session::{
        CaptureOptions, CaptureResult, EventStore, ObservationAddOptions, capture_worktree_review,
        record_observation,
    };

    #[test]
    fn revision_objects_list_all_capturing_units_for_a_shared_snapshot() {
        let repo = TestRepo::new();
        repo.write("README.md", "base\n");
        repo.commit_all("base");
        repo.write("README.md", "changed\n");
        let capture = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

        // A second capture event over the SAME snapshot id, different review unit id
        // (simulating a second worktree's capture sharing the byte-identical artifact).
        let store_dir = resolve_store(repo.path())
            .unwrap()
            .store_dir()
            .to_path_buf();
        write_second_capture_event_for_same_snapshot(&store_dir, &capture);

        let inventory = scan_store_inventory(&store_dir, Some(repo.path())).unwrap();
        let entry = inventory
            .revision_objects
            .iter()
            .find(|snapshot| snapshot.object_id == capture.object_id.as_str())
            .expect("snapshot inventory entry");
        assert!(
            entry
                .revision_ids
                .contains(&capture.revision_id.as_str().to_owned())
        );
        assert!(
            entry
                .revision_ids
                .contains(&"review-unit:sha256:second-worktree".to_owned())
        );
        assert!(entry.revision_ids.windows(2).all(|w| w[0] <= w[1])); // sorted
    }

    /// Build and record a minimal capture event referencing `capture`'s snapshot
    /// id under a different review unit id and idempotency key, mirroring a second
    /// worktree capturing the same range.
    fn write_second_capture_event_for_same_snapshot(store_dir: &Path, capture: &CaptureResult) {
        // The envelope subject and the payload revision carry the same minted id,
        // as a real capture stamps both; the inventory join reads the revision id
        // from the payload.
        let revision_id = RevisionId::new("review-unit:sha256:second-worktree");
        let event = ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("work_object_proposed:{}", revision_id.as_str()),
            EventTarget::for_revision(JournalId::new("journal:default"), revision_id.clone(), None)
                .unwrap(),
            Writer::shore_local("0.1.0"),
            WorkObjectProposedPayload {
                engagement_id: EngagementId::new(format!(
                    "engagement:sha256:{}",
                    crate::canonical_hash::sha256_bytes_hex(revision_id.as_str().as_bytes())
                )),
                work_object: WorkObjectProposal::Revision {
                    revision: Revision {
                        id: revision_id.clone(),
                        object_id: capture.object_id.clone(),
                        git_provenance: Some(GitProvenance {
                            source: capture.source.clone(),
                            base: capture.base.clone(),
                            target: capture.target.clone(),
                        }),
                    },
                    summary: None,
                    object_artifact_content_hash: capture.object_artifact_content_hash.clone(),
                    supersedes: vec![],
                },
            },
            "2026-01-01T00:00:00Z",
        )
        .unwrap();
        EventStore::open(store_dir)
            .record_event_once(&event)
            .unwrap();
    }

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

        let store_dir = resolve_store(repo.path())
            .unwrap()
            .store_dir()
            .to_path_buf();
        let inventory = scan_store_inventory(&store_dir, Some(repo.path())).unwrap();

        let (event_count, event_bytes) = directory_file_stats(&store_dir.join("events"));
        let (snapshot_count, snapshot_bytes) =
            directory_file_stats(&store_dir.join("artifacts/objects"));
        let (note_count, note_bytes) = directory_file_stats(&store_dir.join("artifacts/notes"));

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
                && !artifact.artifact_ref.contains(".pointbreak/data")
                && !artifact.artifact_ref.contains("state.json")
        }));
        assert!(inventory.revision_objects.iter().any(|snapshot| {
            snapshot
                .revision_ids
                .contains(&capture.revision_id.as_str().to_owned())
                && snapshot.object_id == capture.object_id.as_str()
                && snapshot.byte_size > 0
        }));
    }

    #[test]
    fn scan_store_inventory_counts_unignored_untracked_bytes() {
        let repo = TestRepo::new();
        repo.write("tracked.txt", "tracked\n");
        repo.commit_all("base");
        repo.write("untracked.txt", "12345");
        repo.write("ignored.txt", "ignored bytes");
        fs::write(repo.path().join(".git/info/exclude"), "ignored.txt\n").unwrap();
        let empty_store = TempDir::new().expect("create empty store directory");

        let inventory = scan_store_inventory(empty_store.path(), Some(repo.path())).unwrap();

        assert_eq!(inventory.untracked_bytes, Some(5));
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
