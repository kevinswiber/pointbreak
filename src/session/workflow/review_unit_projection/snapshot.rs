use std::path::Path;

use super::identity::ReviewUnitProjectionIdentity;
use crate::error::{Result, ShoreError};
use crate::model::DiffSnapshot;
use crate::session::projection::ArtifactRemovalProjection;
use crate::session::snapshot_artifact::read_snapshot_artifact;

/// Whether a content-addressed snapshot resolved to bytes or to a recorded
/// removal. The join layer returns this instead of erroring when the bound
/// content hash has an `ArtifactRemoved` fact, so a removed-and-swept snapshot
/// renders as an explained absence rather than a hard missing-artifact error.
pub(crate) enum SnapshotContent {
    Present(DiffSnapshot),
    Removed { content_hash: String },
}

/// Resolve the bound snapshot, mapping a recorded removal of its content hash to
/// `Removed` before the byte read. The removed-vs-missing decision lives here, at
/// the layer that holds the event set — the storage byte readers stay
/// event-unaware. A still-missing, *not*-removed artifact falls through to the
/// reader's hard error (removed != not-yet-synced).
pub(super) fn resolve_snapshot_content(
    repo: &Path,
    review_unit: &ReviewUnitProjectionIdentity,
    removal: &ArtifactRemovalProjection,
) -> Result<SnapshotContent> {
    if removal.is_removed(&review_unit.snapshot_artifact_content_hash) {
        return Ok(SnapshotContent::Removed {
            content_hash: review_unit.snapshot_artifact_content_hash.clone(),
        });
    }
    Ok(SnapshotContent::Present(load_bound_snapshot_artifact(
        repo,
        review_unit,
    )?))
}

pub(super) fn load_bound_snapshot_artifact(
    repo: &Path,
    review_unit: &ReviewUnitProjectionIdentity,
) -> Result<DiffSnapshot> {
    let artifact = read_snapshot_artifact(repo, &review_unit.snapshot_id)?;
    // INV-3: bind via the namespace-independent snapshot_id + content_hash only.
    // Identity (review_unit_id/source/base/target) lives in the ReviewUnitCaptured
    // event/projection, never the content-addressed artifact body.
    if artifact.snapshot.snapshot_id != review_unit.snapshot_id {
        return Err(ShoreError::Message(format!(
            "snapshot artifact metadata mismatch for {}",
            review_unit.id.as_str()
        )));
    }
    if artifact.content_hash != review_unit.snapshot_artifact_content_hash {
        return Err(ShoreError::Message(format!(
            "snapshot artifact content hash mismatch for {}",
            review_unit.id.as_str()
        )));
    }

    Ok(artifact.snapshot)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use super::*;
    use crate::model::{ReviewEndpoint, ReviewUnitId};
    use crate::session::event::EventType;
    use crate::session::{
        CaptureOptions, CaptureResult, CommitRangeSpec, EventStore, capture_review,
    };

    #[test]
    fn binds_by_snapshot_id_and_content_hash_ignoring_endpoint_identity() {
        // Capture a range so a real artifact + capture event exist in .shore/data.
        let repo = committed_repo();
        let captured = capture_range(&repo);

        // The authentic projection identity binds.
        let authentic = identity_from(&captured, repo.path());
        load_bound_snapshot_artifact(repo.path(), &authentic).unwrap();

        // A second identity over the SAME snapshot + content hash but a DIFFERENT
        // review_unit_id and a different worktree target also binds — identity is
        // not read from the artifact body (INV-3).
        let other = ReviewUnitProjectionIdentity {
            id: ReviewUnitId::new("review-unit:sha256:other-worktree"),
            target: ReviewEndpoint::GitWorkingTree {
                worktree_root: "/some/other/worktree".to_owned(),
            },
            ..authentic.clone()
        };
        let snapshot = load_bound_snapshot_artifact(repo.path(), &other).unwrap();
        assert_eq!(snapshot.snapshot_id, captured.snapshot_id);
    }

    #[test]
    fn rejects_content_hash_mismatch() {
        let repo = committed_repo();
        let captured = capture_range(&repo);
        let mut tampered = identity_from(&captured, repo.path());
        tampered.snapshot_artifact_content_hash = "sha256:not-the-real-hash".to_owned();
        let err = load_bound_snapshot_artifact(repo.path(), &tampered).unwrap_err();
        assert!(err.to_string().contains("content hash"));
    }

    fn capture_range(repo: &TestRepo) -> CaptureResult {
        capture_review(
            CaptureOptions::new(repo.path()).with_commit_range(CommitRangeSpec::new("HEAD~1")),
        )
        .unwrap()
    }

    /// Build a `ReviewUnitProjectionIdentity` from a `CaptureResult` the way the
    /// projection's `selected_review_unit_capture` would — sourcing every field
    /// from the capture event/result, never from the artifact body.
    fn identity_from(captured: &CaptureResult, repo: &Path) -> ReviewUnitProjectionIdentity {
        let events = EventStore::open(resolved_store_dir(repo))
            .list_events()
            .unwrap();
        let event = events
            .iter()
            .find(|event| {
                event.event_type == EventType::ReviewUnitCaptured
                    && event.payload["reviewUnitId"] == captured.review_unit_id.as_str()
            })
            .expect("capture event");
        ReviewUnitProjectionIdentity {
            id: captured.review_unit_id.clone(),
            session_id: captured.session_id.clone(),
            source: captured.source.clone(),
            base: captured.base.clone(),
            target: captured.target.clone(),
            revision_id: captured.revision_id.clone(),
            snapshot_id: captured.snapshot_id.clone(),
            snapshot_artifact_content_hash: captured.snapshot_artifact_content_hash.clone(),
            capture_event_id: event.event_id.clone(),
        }
    }

    /// The store a workflow actually lands in for `repo` — the shared common-dir
    /// store by default. Reads that follow a workflow resolve here, not the raw
    /// worktree-local `.shore/data`.
    fn resolved_store_dir(repo: &Path) -> std::path::PathBuf {
        crate::git::git_common_dir(repo).unwrap().join("shore")
    }

    fn committed_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        repo.commit_all("change");
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
