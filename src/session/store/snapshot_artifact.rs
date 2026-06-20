use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::canonical_hash::sha256_json_prefixed;
use crate::error::{Result, ShoreError};
use crate::model::{DiffSnapshot, SnapshotId};
use crate::session::store::resolution::resolve_read_store;
use crate::session::{ReviewUnitFingerprint, ShoreStorePaths};
use crate::storage::{CreateFileOutcome, Durability, LocalStorage};

const SNAPSHOT_ARTIFACT_SCHEMA: &str = "shore.snapshot";
const SNAPSHOT_ARTIFACT_VERSION: u32 = 2;

/// The snapshot-scoped v2 artifact body (#146). It carries only namespace-
/// independent content, so two worktrees capturing the same `snapshot_id`
/// produce **byte-identical** artifacts that dedup. ReviewUnit identity and
/// endpoints (`review_unit_id`/`source`/`base`/`target`) live in the
/// `ReviewUnitCaptured` event/projection, never here (INV-1/INV-3).
///
/// New writes are v2. Pre-existing v1 artifacts — whose body also embedded the
/// identity/endpoint fields — stay readable via [`decode_and_validate_snapshot_artifact`]
/// (the dual-read escape hatch); their extra fields are ignored on deserialize
/// and their `content_hash` is validated over the stored body, so a v1 artifact's
/// hash still matches the capture event that bound it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotArtifact {
    pub schema: String,
    pub version: u32,
    pub snapshot: DiffSnapshot,
    pub content_hash: String,
}

/// Write a snapshot artifact to an explicit store dir (the resolved write store).
/// Capture resolves the write store once for the whole landing (artifact → event
/// → `state.json` all target the same dir). The content-addressed
/// exclusive-create write is idempotent: a byte-identical artifact already
/// present returns `Ok` (INV-2/INV-3); a different artifact under the same path is
/// a loud conflict.
pub(crate) fn write_snapshot_artifact_to(
    store_dir: &Path,
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

    let artifact = build_snapshot_artifact_v2(snapshot)?;

    let storage = LocalStorage::new(store_dir);
    let path = snapshot_artifact_path(store_dir, &artifact.snapshot.snapshot_id);
    let bytes = serde_json::to_vec(&artifact)?;

    match storage.create_file_exclusive(&path, &bytes, Durability::Durable)? {
        CreateFileOutcome::Created => Ok(artifact),
        CreateFileOutcome::AlreadyExists => {
            // Dedup on snapshot-content match, regardless of version (INV-7,
            // dual-read): the path is keyed by `snapshot_id`, so an existing valid
            // artifact whose `snapshot` equals ours holds the same content. Two
            // fresh worktrees write byte-identical v2 artifacts; a new v2 capture
            // over a pre-existing v1 artifact dedups against the (untouched) v1
            // body — #146 is fixed without rewriting any signed history. We return
            // the existing artifact (whatever version), so the capture event binds
            // to the hash actually on disk.
            let existing_bytes = std::fs::read(&path).map_err(|error| {
                missing_artifact_or_io(error, &artifact.snapshot.snapshot_id, &path)
            })?;
            let existing = decode_and_validate_snapshot_artifact(&existing_bytes)?;
            if existing.snapshot == artifact.snapshot {
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

/// Build a v2 snapshot-scoped artifact with its content hash filled in. The
/// single place that assembles a [`SnapshotArtifact`] for writing; reuse it so
/// every native v2 capture of the same snapshot produces byte-identical bytes.
pub(crate) fn build_snapshot_artifact_v2(snapshot: DiffSnapshot) -> Result<SnapshotArtifact> {
    let mut artifact = SnapshotArtifact {
        schema: SNAPSHOT_ARTIFACT_SCHEMA.to_owned(),
        version: SNAPSHOT_ARTIFACT_VERSION,
        content_hash: String::new(),
        snapshot,
    };
    artifact.content_hash = snapshot_artifact_content_hash(&artifact)?;
    Ok(artifact)
}

/// Read and hash-validate a stored snapshot artifact.
///
/// Reads resolve through the worktree's resolved store — the shared common-dir
/// store by default, or the worktree-local `.shore/data` store when the worktree
/// is ephemeral.
pub fn read_snapshot_artifact(
    repo: impl AsRef<Path>,
    snapshot_id: &SnapshotId,
) -> Result<SnapshotArtifact> {
    let bytes = read_snapshot_artifact_bytes(repo, snapshot_id)?;
    decode_and_validate_snapshot_artifact(&bytes)
}

pub(crate) fn read_snapshot_artifact_bytes(
    repo: impl AsRef<Path>,
    snapshot_id: &SnapshotId,
) -> Result<Vec<u8>> {
    let read_store = resolve_read_store(repo.as_ref())?;
    let path = snapshot_artifact_path(read_store.store_dir(), snapshot_id);
    std::fs::read(&path).map_err(|error| missing_artifact_or_io(error, snapshot_id, &path))
}

/// Read a snapshot artifact for WRITE-PATH target validation. Resolves the
/// worktree's store first (matching read surfaces), then falls back to the
/// worktree-local `.shore/data/` when the artifact lives only there — an
/// ephemeral or pre-migration capture the resolved common-dir store does not
/// hold. Both sources are content-addressed and the hash is validated, so the
/// choice is invisible to the caller. This closes a split-brain where a unit's
/// events validate (write-path unit validation reads the resolved store) but its
/// file target could not resolve its artifact from a different store.
pub(crate) fn read_snapshot_artifact_for_write_validation(
    repo: impl AsRef<Path>,
    snapshot_id: &SnapshotId,
) -> Result<SnapshotArtifact> {
    let bytes = read_snapshot_artifact_bytes_with_local_fallback(repo, snapshot_id)?;
    decode_and_validate_snapshot_artifact(&bytes)
}

fn read_snapshot_artifact_bytes_with_local_fallback(
    repo: impl AsRef<Path>,
    snapshot_id: &SnapshotId,
) -> Result<Vec<u8>> {
    let read_store = resolve_read_store(repo.as_ref())?;
    let resolved_path = snapshot_artifact_path(read_store.store_dir(), snapshot_id);
    match std::fs::read(&resolved_path) {
        Ok(bytes) => Ok(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Fall back to the worktree-local store: the case where the unit's
            // content-addressed artifact lives only in `.shore/data` (an
            // ephemeral or pre-migration capture) and not in the resolved store.
            let local = ShoreStorePaths::resolve(repo.as_ref())?;
            let local_path = snapshot_artifact_path(local.store_dir(), snapshot_id);
            std::fs::read(&local_path)
                .map_err(|error| missing_artifact_or_io(error, snapshot_id, &local_path))
        }
        Err(error) => Err(missing_artifact_or_io(error, snapshot_id, &resolved_path)),
    }
}

/// Shared error mapping for snapshot-artifact byte reads: a missing file yields
/// the canonical "import referenced artifacts" message; any other I/O error is
/// reported with its path. The read surface and the write-validation fallback
/// differ only in whether a `NotFound` triggers the local fallback before this.
fn missing_artifact_or_io(
    error: std::io::Error,
    snapshot_id: &SnapshotId,
    path: &Path,
) -> ShoreError {
    if error.kind() == std::io::ErrorKind::NotFound {
        return ShoreError::Message(format!(
            "missing artifact for snapshot {}; import referenced artifacts before reading",
            snapshot_id.as_str()
        ));
    }
    ShoreError::Message(format!("read file {}: {error}", path.display()))
}

/// The one decode path for stored snapshot-artifact bytes (INV-6). Validates the
/// `contentHash` over the **raw stored body minus `contentHash`**, then
/// deserializes into the v2-shaped struct. The raw validation is version-agnostic
/// — each version's writer hashed its own present body minus `contentHash` (v1:
/// the full body including the identity/endpoint fields; v2: the snapshot-scoped
/// body) — so this accepts and validates **both** v1 and v2 (the dual-read escape
/// hatch). A v1 artifact's extra identity fields are ignored on deserialize, and
/// the returned struct's `content_hash` is whatever was stored, so the
/// `ReviewUnitCaptured` event that bound it still matches (INV-3).
///
/// TODO(remove-dual-read): once every affected repo has converged to v2 (no v1
/// artifacts remain), drop the raw/version-agnostic branch and restore a strict
/// v2-only decode (reject `version != 2`, hash over the typed struct). See plan
/// 0074 `findings/todo-remove-dual-read-after-fleet-migration.md`.
pub(crate) fn decode_and_validate_snapshot_artifact(bytes: &[u8]) -> Result<SnapshotArtifact> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    validate_raw_artifact_content_hash(&value)?;
    let artifact: SnapshotArtifact = serde_json::from_value(value)?;
    Ok(artifact)
}

/// Validate a stored artifact's `contentHash` over its raw JSON body minus
/// `contentHash`. Version-agnostic by construction (it hashes whatever body is
/// present), so it covers both v1 and v2 stored shapes — the dual-read decode and
/// any caller validating raw artifact bytes share this single hash scope.
fn validate_raw_artifact_content_hash(value: &serde_json::Value) -> Result<()> {
    let mut material = value.clone();
    let Some(object) = material.as_object_mut() else {
        return Err(ShoreError::Message(
            "snapshot artifact hash material must be an object".to_owned(),
        ));
    };
    let Some(stored) = object
        .remove("contentHash")
        .and_then(|value| value.as_str().map(str::to_owned))
    else {
        return Err(ShoreError::Message(
            "snapshot artifact hash material is missing contentHash".to_owned(),
        ));
    };

    if sha256_json_prefixed(&material)? == stored {
        return Ok(());
    }

    let snapshot_id = value
        .get("snapshot")
        .and_then(|snapshot| snapshot.get("snapshot_id"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("<unknown>");
    Err(ShoreError::Message(format!(
        "snapshot artifact content hash mismatch for {snapshot_id}"
    )))
}

/// Hash a v2 artifact's body minus `contentHash` (the value [`build_snapshot_artifact_v2`]
/// stamps in). With the snapshot-scoped struct the hashed material is
/// `{schema, version, snapshot}` — namespace-independent (INV-2).
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

pub(crate) fn snapshot_artifact_path(store_dir: &Path, snapshot_id: &SnapshotId) -> PathBuf {
    store_dir
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
    use crate::canonical_hash::sha256_json_prefixed;
    use crate::git::capture_worktree_diff_files;
    use crate::model::{DiffSnapshot, ReviewId};
    use crate::session::store::resolution::resolve_store;
    use crate::session::{
        CaptureOptions, CommitRangeSpec, capture_review, compute_review_unit_fingerprint,
        read_snapshot_artifact,
    };

    #[test]
    fn snapshot_artifact_schema_is_pinned_at_shore_snapshot_v2() {
        // Native writes are v2. Any future elision-aware artifact must bump one of
        // these constants (see docs/adr/adr-0002-large-snapshot-artifact-policy.md).
        assert_eq!(super::SNAPSHOT_ARTIFACT_SCHEMA, "shore.snapshot");
        assert_eq!(super::SNAPSHOT_ARTIFACT_VERSION, 2);
    }

    #[test]
    fn snapshot_artifact_body_is_snapshot_scoped_v2() {
        let repo = modified_repo();
        let artifact = write_current_snapshot_artifact(&repo);

        let json = serde_json::to_value(&artifact).unwrap();
        let keys = json
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            keys,
            ["contentHash", "schema", "snapshot", "version"]
                .iter()
                .map(|key| key.to_string())
                .collect::<std::collections::BTreeSet<_>>(),
            "v2 body carries no reviewUnitId/source/base/target"
        );
        assert_eq!(artifact.version, 2);
    }

    #[test]
    fn same_range_in_two_repos_produces_byte_identical_artifacts() {
        let repo_a = committed_repo();
        let repo_b = clone_repo(&repo_a); // real clone preserves commit/tree oids

        let a = capture_range(&repo_a, "HEAD~1");
        let b = capture_range(&repo_b, "HEAD~1");

        assert_eq!(a.snapshot_id, b.snapshot_id);
        assert_ne!(a.review_unit_id, b.review_unit_id); // identity namespace unchanged (0011 B2)
        assert_eq!(
            a.snapshot_artifact_content_hash,
            b.snapshot_artifact_content_hash
        );

        let bytes_a = fs::read(snapshot_artifact_path(
            &resolved_store_dir(repo_a.path()),
            &a.snapshot_id,
        ))
        .unwrap();
        let bytes_b = fs::read(snapshot_artifact_path(
            &resolved_store_dir(repo_b.path()),
            &b.snapshot_id,
        ))
        .unwrap();
        assert_eq!(
            bytes_a, bytes_b,
            "snapshot-scoped artifacts must be byte-identical"
        );
    }

    #[test]
    fn decode_accepts_a_v1_artifact_and_keeps_its_stored_hash() {
        // Dual-read (INV-6): a pre-existing v1 artifact stays readable; its extra
        // identity fields are ignored and its stored v1 hash is preserved so the
        // capture event that bound it still matches.
        let repo = modified_repo();
        let artifact = write_current_snapshot_artifact(&repo);
        let path = snapshot_artifact_path(
            &resolved_store_dir(repo.path()),
            &artifact.snapshot.snapshot_id,
        );
        let v1_bytes = rewrite_as_v1(&fs::read(&path).unwrap());

        let decoded = decode_and_validate_snapshot_artifact(&v1_bytes).unwrap();
        assert_eq!(decoded.version, 1);
        assert_eq!(decoded.snapshot.snapshot_id, artifact.snapshot.snapshot_id);
        // The struct preserves the stored v1 content hash, not a recomputed v2 one.
        let v1_hash: serde_json::Value =
            serde_json::from_slice::<serde_json::Value>(&v1_bytes).unwrap()["contentHash"].clone();
        assert_eq!(decoded.content_hash, v1_hash.as_str().unwrap());
        assert_ne!(decoded.content_hash, artifact.content_hash);
    }

    #[test]
    fn decode_rejects_a_tampered_v1_artifact() {
        let repo = modified_repo();
        let artifact = write_current_snapshot_artifact(&repo);
        let path = snapshot_artifact_path(
            &resolved_store_dir(repo.path()),
            &artifact.snapshot.snapshot_id,
        );
        let mut value: serde_json::Value =
            serde_json::from_slice(&rewrite_as_v1(&fs::read(&path).unwrap())).unwrap();
        // Mutate a body field without re-stamping the hash.
        value["target"]["worktreeRoot"] = serde_json::json!("/evil");

        let error = decode_and_validate_snapshot_artifact(&serde_json::to_vec(&value).unwrap())
            .expect_err("tampered v1 artifact is rejected");
        assert!(error.to_string().contains("content hash"));
    }

    #[test]
    fn write_dedups_against_a_pre_existing_v1_artifact() {
        // The dual-read #146 fix within a store: a snapshot path already holding a
        // v1 artifact accepts a new v2 write of the same snapshot without conflict,
        // returns the (untouched) v1 artifact, and never rewrites it — so a signed
        // v1 capture that bound the v1 hash keeps validating.
        let repo = modified_repo();
        let files = capture_worktree_diff_files(repo.path()).unwrap();
        let fingerprint = compute_review_unit_fingerprint(repo.path()).unwrap();
        let snapshot = DiffSnapshot::new(
            ReviewId::new("review:default"),
            fingerprint.snapshot_id.clone(),
            files,
        );
        let store_dir = ShoreStorePaths::resolve(repo.path())
            .unwrap()
            .store_dir()
            .to_path_buf();

        // First write lands a native v2 artifact; downgrade it to a v1 body, as an
        // old binary would have written it.
        let v2 = write_snapshot_artifact_to(&store_dir, &fingerprint, snapshot.clone()).unwrap();
        assert_eq!(v2.version, 2);
        let path = snapshot_artifact_path(&store_dir, &fingerprint.snapshot_id);
        let v1_bytes = rewrite_as_v1(&fs::read(&path).unwrap());
        fs::write(&path, &v1_bytes).unwrap();

        // Second write of the same snapshot dedups against the v1 body.
        let deduped = write_snapshot_artifact_to(&store_dir, &fingerprint, snapshot).unwrap();
        assert_eq!(deduped.version, 1, "dedup returns the existing v1 artifact");
        assert_eq!(
            fs::read(&path).unwrap(),
            v1_bytes,
            "v1 artifact left untouched"
        );
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
        let artifact = write_snapshot_artifact(repo.path(), &fingerprint, snapshot).unwrap();

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
        assert_eq!(stored.version, 2);
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
        let path = snapshot_artifact_path(
            &resolved_store_dir(repo.path()),
            &artifact.snapshot.snapshot_id,
        );

        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        // Tamper a field inside the v2 content hash. `DiffFile` is snake_case,
        // unlike the camelCase `SnapshotArtifact` wrapper.
        json["snapshot"]["files"][0]["new_path"] = serde_json::json!("/evil");
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

        let error = read_snapshot_artifact(repo.path(), &artifact.snapshot.snapshot_id)
            .expect_err("tampered artifact should be rejected");

        assert!(error.to_string().contains("content hash"));
    }

    #[test]
    fn snapshot_artifact_hash_covers_snapshot_rows() {
        let repo = modified_repo();
        let artifact = write_current_snapshot_artifact(&repo);
        let mut changed = artifact.clone();
        changed.snapshot.files.clear();

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

    #[test]
    fn write_validation_artifact_read_prefers_resolved_store_when_present() {
        let repo = modified_repo();
        // The artifact lands in the resolved (shared common-dir) store, exactly
        // where capture writes it, so the read resolves it without any fallback.
        let artifact = write_current_snapshot_artifact(&repo);

        let read = read_snapshot_artifact_for_write_validation(
            repo.path(),
            &artifact.snapshot.snapshot_id,
        )
        .unwrap();

        assert_eq!(read, artifact);
    }

    // The worktree-local read fallback's old premise — a non-ephemeral worktree
    // that captured locally and had not yet copied its artifact into a separate
    // linked store — no longer exists: with one shared store by default, a
    // populated worktree-local `.shore/data` is a pre-default store that the
    // legacy guard routes to `shore store migrate` rather than reading through.
    // The dedicated fallback test is retired; `..._prefers_resolved_store...` and
    // `..._missing_everywhere...` cover the surviving write-validation reads.

    #[test]
    fn write_validation_artifact_read_missing_everywhere_errors_clearly() {
        let repo = modified_repo();
        let fingerprint = compute_review_unit_fingerprint(repo.path()).unwrap();

        let error =
            read_snapshot_artifact_for_write_validation(repo.path(), &fingerprint.snapshot_id)
                .expect_err("an artifact absent from both stores errors");

        assert!(
            error.to_string().contains("missing artifact for snapshot"),
            "got: {error}"
        );
        assert!(
            error
                .to_string()
                .contains("import referenced artifacts before reading"),
            "got: {error}"
        );
    }

    /// Test convenience: write a snapshot artifact to the worktree's resolved
    /// write store — the shared common-dir store by default, exactly where
    /// capture lands it. Production has a single snapshot writer
    /// (`write_snapshot_artifact_to`); capture resolves the write store once and
    /// calls it directly. Resolving the store here keeps these reads and the
    /// production read surface on the same store.
    fn write_snapshot_artifact(
        repo: impl AsRef<Path>,
        fingerprint: &ReviewUnitFingerprint,
        snapshot: DiffSnapshot,
    ) -> Result<SnapshotArtifact> {
        let store_dir = resolve_store(repo.as_ref())?.store_dir().to_path_buf();
        write_snapshot_artifact_to(&store_dir, fingerprint, snapshot)
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

    /// The store a capture/write actually lands in for `repo` — the shared
    /// common-dir store by default.
    fn resolved_store_dir(repo: &Path) -> PathBuf {
        resolve_store(repo).unwrap().store_dir().to_path_buf()
    }

    fn committed_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        repo.commit_all("change");
        repo
    }

    /// Real `git clone` of `source` into a fresh temp dir. Cloning preserves the
    /// commit/tree OIDs, so the same `--base HEAD~1` range captures the same
    /// `snapshot_id` while the differing canonical worktree root mints a distinct
    /// `review_unit_id` — exactly the two-worktree shape of #146.
    fn clone_repo(source: &TestRepo) -> TestRepo {
        let root = tempfile::tempdir().expect("create clone temp directory");
        let status = Command::new("git")
            .args(["clone", "--quiet"])
            .arg(source.path())
            .arg(root.path())
            .status()
            .expect("run git clone");
        assert!(status.success(), "git clone failed");
        let clone = TestRepo { root };
        clone.git(["config", "user.name", "Shore Tests"]);
        clone.git(["config", "user.email", "shore-tests@example.com"]);
        clone.git(["config", "commit.gpgsign", "false"]);
        clone
    }

    fn capture_range(repo: &TestRepo, base_rev: &str) -> crate::session::CaptureResult {
        capture_review(
            CaptureOptions::new(repo.path()).with_commit_range(CommitRangeSpec::new(base_rev)),
        )
        .unwrap()
    }

    /// Rewrite a v2 artifact's bytes into a faithful **v1** artifact: re-add the
    /// identity/endpoint fields the v1 body carried, set `version: 1`, and
    /// recompute `contentHash` over the full v1 body, so it passes the
    /// version-agnostic validation the dual-read decode applies.
    fn rewrite_as_v1(bytes: &[u8]) -> Vec<u8> {
        let mut value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        let object = value.as_object_mut().unwrap();
        object.insert("version".to_owned(), serde_json::json!(1));
        object.insert(
            "reviewUnitId".to_owned(),
            serde_json::json!("review-unit:sha256:legacy"),
        );
        object.insert(
            "source".to_owned(),
            serde_json::json!({ "kind": "git_working_tree" }),
        );
        object.insert(
            "base".to_owned(),
            serde_json::json!({ "kind": "git_working_tree", "worktreeRoot": "/legacy" }),
        );
        object.insert(
            "target".to_owned(),
            serde_json::json!({ "kind": "git_working_tree", "worktreeRoot": "/legacy" }),
        );
        object.remove("contentHash");
        let hash = sha256_json_prefixed(&value).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("contentHash".to_owned(), serde_json::json!(hash));
        serde_json::to_vec(&value).unwrap()
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
