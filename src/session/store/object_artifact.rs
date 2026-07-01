use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::error::{Result, ShoreError};
use crate::model::{DiffSnapshot, ObjectId};
use crate::session::store::backend::StoreBackend;
use crate::session::store::content::ContentArtifacts;
use crate::session::store::resolution::resolve_read_store;
use crate::session::{RevisionFingerprint, ShoreStorePaths};

const OBJECT_ARTIFACT_SCHEMA: &str = "shore.object";
const OBJECT_ARTIFACT_VERSION: u32 = 2;

/// The object-scoped v2 artifact body (#146). It carries only namespace-
/// independent content, so two worktrees capturing the same `object_id`
/// produce **byte-identical** artifacts that dedup. Revision identity and
/// endpoints (`revision_id`/`source`/`base`/`target`) live in the
/// `WorkObjectProposed` event/projection, never here (INV-1/INV-3).
///
/// All writes and reads are v2. The legacy dual-read that also accepted
/// identity-bearing v1 bodies has been removed: [`decode_and_validate_object_artifact`]
/// now rejects any non-v2 body, and the one-shot store migrator re-emits every
/// artifact as a clean v2 body, so no v1 artifact survives to be read.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectArtifact {
    pub schema: String,
    pub version: u32,
    pub snapshot: DiffSnapshot,
    pub content_hash: String,
    /// Ordered content-coding tokens (compression/encryption) applied to the
    /// stored artifact file in list order at write and reversed on read;
    /// default `[]` is the identity encoding. A sibling of `content_hash`,
    /// never an input to it: the hash is over the decoded body, and this
    /// field describes how the stored bytes decode to that body. Reserved —
    /// no codec populates it yet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_encoding: Vec<String>,
}

/// Write a object artifact through the resolved store's backend handle. Capture
/// resolves the write store once for the whole landing (artifact → event →
/// `state.json` all target the same store). The content-addressed
/// exclusive-create write is idempotent: a byte-identical artifact already
/// present returns `Ok` (INV-2/INV-3); a different artifact under the same path is
/// a loud conflict.
pub(crate) fn write_object_artifact_to(
    backend: &StoreBackend,
    fingerprint: &RevisionFingerprint,
    snapshot: DiffSnapshot,
) -> Result<ObjectArtifact> {
    if snapshot.object_id != fingerprint.object_id {
        return Err(ShoreError::Message(format!(
            "object id {} does not match revision fingerprint {}",
            snapshot.object_id.as_str(),
            fingerprint.object_id.as_str()
        )));
    }

    let artifact = build_object_artifact_v2(snapshot)?;

    // Dedup on the artifact's canonical content hash. The object id is a stable
    // review-content identity, while the artifact body keeps the concrete captured
    // rows and can legitimately change after a rebase (line numbers/blob OIDs)
    // without changing the object id. The capture event binds to this content hash.
    let content = ContentArtifacts::from_backend(backend);
    let content_ref = object_content_ref_for_hash(&artifact.content_hash);
    let bytes = serde_json::to_vec(&artifact)?;
    content.put_object(&content_ref, &bytes, artifact)
}

/// Build a v2 object-scoped artifact with its content hash filled in. The
/// single place that assembles a [`ObjectArtifact`] for writing; reuse it so
/// every native v2 capture of the same snapshot produces byte-identical bytes.
pub(crate) fn build_object_artifact_v2(snapshot: DiffSnapshot) -> Result<ObjectArtifact> {
    let mut artifact = ObjectArtifact {
        schema: OBJECT_ARTIFACT_SCHEMA.to_owned(),
        version: OBJECT_ARTIFACT_VERSION,
        content_hash: String::new(),
        content_encoding: Vec::new(),
        snapshot,
    };
    artifact.content_hash = object_artifact_content_hash(&artifact)?;
    Ok(artifact)
}

/// Read and hash-validate a stored object artifact.
///
/// Reads resolve through the worktree's resolved store — the shared common-dir
/// store by default, or the worktree-local `.shore/data` store when the worktree
/// is ephemeral.
pub fn read_object_artifact(
    repo: impl AsRef<Path>,
    object_id: &ObjectId,
) -> Result<ObjectArtifact> {
    let bytes = read_object_artifact_bytes(repo, object_id)?;
    decode_and_validate_object_artifact(&bytes)
}

pub fn read_bound_object_artifact(
    repo: impl AsRef<Path>,
    object_id: &ObjectId,
    content_hash: &str,
) -> Result<ObjectArtifact> {
    let bytes = read_bound_object_artifact_bytes(repo, object_id, content_hash)?;
    let artifact = decode_and_validate_object_artifact(&bytes)?;
    validate_bound_object_artifact(&artifact, object_id, content_hash)?;
    Ok(artifact)
}

pub(crate) fn read_object_artifact_bytes(
    repo: impl AsRef<Path>,
    object_id: &ObjectId,
) -> Result<Vec<u8>> {
    let read_store = resolve_read_store(repo.as_ref())?;
    let content = ContentArtifacts::from_backend(read_store.backend());
    let legacy_ref = object_content_ref(object_id);
    if let Some(bytes) = content.read_object_bytes_if_exists(&legacy_ref)? {
        return Ok(bytes);
    }
    for content_ref in content.list_refs("artifacts/objects")? {
        let Some(bytes) = content.read_object_bytes_if_exists(&content_ref)? else {
            continue;
        };
        if object_artifact_bytes_match_object_id(&bytes, object_id)? {
            return Ok(bytes);
        }
    }
    Err(missing_object_artifact(object_id))
}

pub(crate) fn read_bound_object_artifact_bytes(
    repo: impl AsRef<Path>,
    object_id: &ObjectId,
    content_hash: &str,
) -> Result<Vec<u8>> {
    let content_ref = object_content_ref_for_hash(content_hash);
    let read_store = resolve_read_store(repo.as_ref())?;
    let content = ContentArtifacts::from_backend(read_store.backend());
    match content.read_object_bytes_if_exists(&content_ref)? {
        Some(bytes) => {
            let artifact = decode_and_validate_object_artifact(&bytes)?;
            validate_bound_object_artifact(&artifact, object_id, content_hash)?;
            Ok(bytes)
        }
        None => {
            // Compatibility with stores written before object artifacts were keyed
            // by content hash. The old path was object-id keyed; it is valid only
            // if the stored blob still matches the event-bound content hash.
            let legacy_ref = object_content_ref(object_id);
            let bytes = content.read_object_bytes(&legacy_ref, object_id)?;
            let artifact = decode_and_validate_object_artifact(&bytes)?;
            validate_bound_object_artifact(&artifact, object_id, content_hash)?;
            Ok(bytes)
        }
    }
}

/// Read a object artifact for WRITE-PATH target validation. Resolves the
/// worktree's store first (matching read surfaces), then falls back to the
/// worktree-local `.shore/data/` when the artifact lives only there — an
/// ephemeral or pre-migration capture the resolved common-dir store does not
/// hold. Both sources are content-addressed and the hash is validated, so the
/// choice is invisible to the caller. This closes a split-brain where a unit's
/// events validate (write-path unit validation reads the resolved store) but its
/// file target could not resolve its artifact from a different store.
pub(crate) fn read_bound_object_artifact_for_write_validation(
    repo: impl AsRef<Path>,
    object_id: &ObjectId,
    content_hash: &str,
) -> Result<ObjectArtifact> {
    let bytes =
        read_bound_object_artifact_bytes_with_local_fallback(repo, object_id, content_hash)?;
    let artifact = decode_and_validate_object_artifact(&bytes)?;
    validate_bound_object_artifact(&artifact, object_id, content_hash)?;
    Ok(artifact)
}

fn read_bound_object_artifact_bytes_with_local_fallback(
    repo: impl AsRef<Path>,
    object_id: &ObjectId,
    content_hash: &str,
) -> Result<Vec<u8>> {
    let content_ref = object_content_ref_for_hash(content_hash);
    let read_store = resolve_read_store(repo.as_ref())?;
    let resolved = ContentArtifacts::from_backend(read_store.backend());
    if let Some(bytes) = resolved.read_object_bytes_if_exists(&content_ref)? {
        return Ok(bytes);
    }

    let local = ShoreStorePaths::resolve(repo.as_ref())?;
    let local = ContentArtifacts::local(local.store_dir());
    if let Some(bytes) = local.read_object_bytes_if_exists(&content_ref)? {
        return Ok(bytes);
    }

    let legacy_ref = object_content_ref(object_id);
    if let Some(bytes) = resolved.read_object_bytes_if_exists(&legacy_ref)? {
        return Ok(bytes);
    }
    local.read_object_bytes(&legacy_ref, object_id)
}

/// The one decode path for stored snapshot-artifact bytes. Strict v2-only:
/// rejects any `version` other than v2 (the object-scoped body), then validates
/// the `contentHash` over the typed v2 struct. The legacy dual-read that also
/// accepted identity-bearing v1 bodies is gone — the one-shot migrator re-emits
/// every artifact as v2, so a v1 body in a migrated store is a stray and is
/// loudly rejected rather than silently accepted.
pub(crate) fn decode_and_validate_object_artifact(bytes: &[u8]) -> Result<ObjectArtifact> {
    let artifact: ObjectArtifact = serde_json::from_slice(bytes)?;
    if artifact.version != OBJECT_ARTIFACT_VERSION {
        return Err(ShoreError::Message(format!(
            "unsupported object artifact version {}; only v{OBJECT_ARTIFACT_VERSION} (object-scoped) is supported",
            artifact.version
        )));
    }
    let expected = object_artifact_content_hash(&artifact)?;
    if artifact.content_hash != expected {
        return Err(ShoreError::Message(format!(
            "object artifact content hash mismatch for {}",
            artifact.snapshot.object_id.as_str()
        )));
    }
    Ok(artifact)
}

/// Hash a v2 artifact's body minus `contentHash` (the value [`build_object_artifact_v2`]
/// stamps in) and minus `contentEncoding` (storage metadata describing how the
/// stored file decodes to the body, never an input to the body's identity).
/// With the object-scoped struct the hashed material is
/// `{schema, version, snapshot}` — namespace-independent (INV-2).
fn object_artifact_content_hash(artifact: &ObjectArtifact) -> Result<String> {
    let mut material = serde_json::to_value(artifact)?;
    let Some(object) = material.as_object_mut() else {
        return Err(ShoreError::Message(
            "object artifact hash material must be an object".to_owned(),
        ));
    };
    if object.remove("contentHash").is_none() {
        return Err(ShoreError::Message(
            "object artifact hash material is missing contentHash".to_owned(),
        ));
    }
    object.remove("contentEncoding");

    sha256_json_prefixed(&material)
}

pub(crate) fn object_artifact_path(store_dir: &Path, object_id: &ObjectId) -> PathBuf {
    store_dir
        .join("artifacts/objects")
        .join(format!("{}.json", artifact_file_stem(object_id.as_str())))
}

pub(crate) fn object_artifact_path_for_hash(store_dir: &Path, content_hash: &str) -> PathBuf {
    store_dir
        .join("artifacts/objects")
        .join(format!("{}.json", content_hash_file_stem(content_hash)))
}

/// The store-relative locator for an object artifact (`artifacts/objects/<hash>.json`),
/// the content-store ref the same file resolves under.
pub(in crate::session::store) fn object_content_ref(object_id: &ObjectId) -> String {
    format!(
        "artifacts/objects/{}.json",
        artifact_file_stem(object_id.as_str())
    )
}

pub(in crate::session::store) fn object_content_ref_for_hash(content_hash: &str) -> String {
    format!(
        "artifacts/objects/{}.json",
        content_hash_file_stem(content_hash)
    )
}

fn artifact_file_stem(id: &str) -> String {
    // Snapshot IDs include a colon-bearing prefix; hashing keeps artifact
    // filenames portable while the artifact body preserves the readable ID.
    sha256_bytes_hex(id.as_bytes())
}

fn content_hash_file_stem(content_hash: &str) -> String {
    content_hash
        .strip_prefix("sha256:")
        .filter(|stem| stem.len() == 64 && stem.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_owned)
        .unwrap_or_else(|| sha256_bytes_hex(content_hash.as_bytes()))
}

fn validate_bound_object_artifact(
    artifact: &ObjectArtifact,
    object_id: &ObjectId,
    content_hash: &str,
) -> Result<()> {
    if artifact.snapshot.object_id != *object_id {
        return Err(ShoreError::Message(format!(
            "object artifact locator mismatch for {}",
            object_id.as_str()
        )));
    }
    if artifact.content_hash != content_hash {
        return Err(ShoreError::Message(format!(
            "object artifact content hash mismatch for {content_hash}"
        )));
    }
    Ok(())
}

fn missing_object_artifact(object_id: &ObjectId) -> ShoreError {
    ShoreError::Message(format!(
        "missing artifact for snapshot {}; import referenced artifacts before reading",
        object_id.as_str()
    ))
}

fn object_artifact_bytes_match_object_id(bytes: &[u8], object_id: &ObjectId) -> Result<bool> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    Ok(value
        .get("snapshot")
        .and_then(|snapshot| snapshot.get("object_id"))
        .and_then(|value| value.as_str())
        == Some(object_id.as_str()))
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
    use crate::model::{DiffSnapshot, ObjectId, ReviewId};
    use crate::session::store::backend::StoreBackend;
    use crate::session::store::resolution::resolve_store;
    use crate::session::{
        CaptureOptions, CommitRangeSpec, capture_review, compute_revision_fingerprint,
        read_object_artifact,
    };

    #[test]
    fn write_object_artifact_routes_through_each_backend() {
        // The object write goes through the resolved backend handle, so the same
        // write that capture performs is exercisable over the injection-only
        // in-memory backend as well as the file backend — a non-Local backend
        // would capture the object write.
        let repo = modified_repo();
        let files = capture_worktree_diff_files(repo.path()).unwrap();
        let fingerprint = compute_revision_fingerprint(repo.path()).unwrap();
        let snapshot = DiffSnapshot::new(
            ReviewId::new("review:default"),
            fingerprint.object_id.clone(),
            files,
        );

        let root = tempfile::tempdir().unwrap();
        let backends = [
            StoreBackend::Local(root.path().join(".shore/data")),
            StoreBackend::memory(),
        ];
        for backend in backends {
            let artifact =
                write_object_artifact_to(&backend, &fingerprint, snapshot.clone()).unwrap();
            assert_eq!(artifact.version, 2);

            // The artifact reads back through the same backend's content store and
            // decode-validates.
            let content_ref = object_content_ref_for_hash(&artifact.content_hash);
            let read = ContentArtifacts::from_backend(&backend)
                .read_object_bytes(&content_ref, &artifact.snapshot.object_id)
                .unwrap();
            assert_eq!(
                decode_and_validate_object_artifact(&read).unwrap(),
                artifact
            );

            // A second write of the same snapshot dedups to the existing artifact.
            let deduped =
                write_object_artifact_to(&backend, &fingerprint, snapshot.clone()).unwrap();
            assert_eq!(deduped, artifact, "a second write dedups");
        }
    }

    #[test]
    fn object_artifact_schema_is_pinned_at_shore_object_v2() {
        // The artifact stores a content object, so its schema is `shore.object`.
        // Native writes are v2. Any future elision-aware artifact must bump one of
        // these constants (see docs/adr/adr-0002-large-snapshot-artifact-policy.md).
        assert_eq!(super::OBJECT_ARTIFACT_SCHEMA, "shore.object");
        assert_eq!(super::OBJECT_ARTIFACT_VERSION, 2);
    }

    #[test]
    fn object_artifact_body_uses_object_id_wire_key() {
        // The stored artifact body finishes Snapshot->Object on the wire: the
        // content-only id serializes under `object_id` (value already `obj:`),
        // not the legacy `snapshot_id` field name.
        let snapshot = DiffSnapshot::new(
            ReviewId::new("review:default"),
            ObjectId::new("obj:sha256:abc"),
            Vec::new(),
        );
        let artifact = build_object_artifact_v2(snapshot).unwrap();

        let json = serde_json::to_value(&artifact.snapshot).unwrap();
        assert!(
            json.get("object_id").is_some(),
            "artifact snapshot body must serialize the object id under `object_id`"
        );
        assert!(
            json.get("snapshot_id").is_none(),
            "the legacy `snapshot_id` wire key must be gone"
        );
        assert!(
            json["object_id"].as_str().unwrap().starts_with("obj:"),
            "the object id value is unchanged (already `obj:`)"
        );
    }

    #[test]
    fn object_artifact_body_is_snapshot_scoped_v2() {
        let repo = modified_repo();
        let artifact = write_current_object_artifact(&repo);

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

        assert_eq!(a.object_id, b.object_id);
        // A commit-range revision keys off the content object plus the commit/tree
        // provenance, both of which a real clone preserves, so the revision id
        // converges across the two repos (the local repo path never enters it).
        assert_eq!(a.revision_id, b.revision_id);
        assert_eq!(
            a.object_artifact_content_hash,
            b.object_artifact_content_hash
        );

        let bytes_a = fs::read(object_artifact_path_for_hash(
            &resolved_store_dir(repo_a.path()),
            &a.object_artifact_content_hash,
        ))
        .unwrap();
        let bytes_b = fs::read(object_artifact_path_for_hash(
            &resolved_store_dir(repo_b.path()),
            &b.object_artifact_content_hash,
        ))
        .unwrap();
        assert_eq!(
            bytes_a, bytes_b,
            "object-scoped artifacts must be byte-identical"
        );
    }

    #[test]
    fn v1_snapshot_body_is_rejected_after_the_break() {
        // The dual-read is gone: an identity-bearing v1 body no longer decodes;
        // only the object-scoped v2 body does. A clean store carries only v2
        // artifacts (the one-shot migrator re-emits them), so a stray v1 body is a
        // loud rejection, not a silently-accepted legacy shape.
        let repo = modified_repo();
        let artifact = write_current_object_artifact(&repo);
        let path =
            object_artifact_path_for_hash(&resolved_store_dir(repo.path()), &artifact.content_hash);
        let v1_bytes = rewrite_as_v1(&fs::read(&path).unwrap());

        let error = decode_and_validate_object_artifact(&v1_bytes)
            .expect_err("a v1 body must be rejected after the break");
        assert!(
            error.to_string().contains("v2"),
            "rejection must name the supported v2 shape, got: {error}"
        );
    }

    #[test]
    fn v2_snapshot_body_decodes_cleanly() {
        let repo = modified_repo();
        let artifact = write_current_object_artifact(&repo);
        let path =
            object_artifact_path_for_hash(&resolved_store_dir(repo.path()), &artifact.content_hash);
        let v2_bytes = fs::read(&path).unwrap();

        let decoded = decode_and_validate_object_artifact(&v2_bytes).unwrap();
        assert_eq!(decoded.version, 2);
        assert_eq!(decoded, artifact);
    }

    #[test]
    fn decode_rejects_a_tampered_v2_artifact() {
        let repo = modified_repo();
        let artifact = write_current_object_artifact(&repo);
        let path =
            object_artifact_path_for_hash(&resolved_store_dir(repo.path()), &artifact.content_hash);
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        // Mutate a body field without re-stamping the hash.
        value["snapshot"]["files"][0]["new_path"] = serde_json::json!("/evil");

        let error = decode_and_validate_object_artifact(&serde_json::to_vec(&value).unwrap())
            .expect_err("tampered v2 artifact is rejected");
        assert!(error.to_string().contains("content hash"));
    }

    #[test]
    fn write_dedups_against_a_pre_existing_v2_artifact() {
        // Within a store, a snapshot path already holding a v2 artifact accepts a
        // second write of the same snapshot without conflict and returns the
        // existing artifact, never rewriting it.
        let repo = modified_repo();
        let files = capture_worktree_diff_files(repo.path()).unwrap();
        let fingerprint = compute_revision_fingerprint(repo.path()).unwrap();
        let snapshot = DiffSnapshot::new(
            ReviewId::new("review:default"),
            fingerprint.object_id.clone(),
            files,
        );
        let store_dir = ShoreStorePaths::resolve(repo.path())
            .unwrap()
            .store_dir()
            .to_path_buf();
        let backend = StoreBackend::Local(store_dir.clone());

        let first = write_object_artifact_to(&backend, &fingerprint, snapshot.clone()).unwrap();
        assert_eq!(first.version, 2);
        let path = object_artifact_path_for_hash(&store_dir, &first.content_hash);
        let on_disk = fs::read(&path).unwrap();

        let deduped = write_object_artifact_to(&backend, &fingerprint, snapshot).unwrap();
        assert_eq!(deduped, first, "dedup returns the existing v2 artifact");
        assert_eq!(fs::read(&path).unwrap(), on_disk, "artifact left untouched");
    }

    #[test]
    fn captured_text_rows_remain_inline_in_object_artifact() {
        let repo = TestRepo::new();
        repo.write("README.md", "base\n");
        repo.commit_all("base");

        let added = (1..=25).map(|n| format!("line {n}\n")).collect::<String>();
        repo.write("docs/example.md", added);

        let files = capture_worktree_diff_files(repo.path()).unwrap();
        let fingerprint = compute_revision_fingerprint(repo.path()).unwrap();
        let snapshot = DiffSnapshot::new(
            ReviewId::new("review:default"),
            fingerprint.object_id.clone(),
            files,
        );
        let artifact = write_object_artifact(repo.path(), &fingerprint, snapshot).unwrap();

        let stored = read_object_artifact(repo.path(), &artifact.snapshot.object_id).unwrap();
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
    fn write_object_artifact_stores_full_snapshot() {
        let repo = modified_repo();
        let artifact = write_current_object_artifact(&repo);

        let stored = read_object_artifact(repo.path(), &artifact.snapshot.object_id).unwrap();

        assert_eq!(stored.schema, "shore.object");
        assert_eq!(stored.version, 2);
        assert_eq!(stored.snapshot.object_id, artifact.snapshot.object_id);
        assert_eq!(stored.snapshot.files.len(), 1);
        assert_eq!(
            stored.snapshot.files[0].new_path.as_deref(),
            Some("src/lib.rs")
        );
        assert!(!stored.snapshot.files[0].hunks.is_empty());
    }

    #[test]
    fn stored_object_artifact_survives_worktree_drift() {
        let repo = modified_repo();
        let artifact = write_current_object_artifact(&repo);

        repo.write("src/lib.rs", "pub fn value() -> u32 { 99 }\n");
        let stored = read_object_artifact(repo.path(), &artifact.snapshot.object_id).unwrap();

        assert_eq!(
            stored.snapshot.files[0].new_path.as_deref(),
            Some("src/lib.rs")
        );
        assert!(format!("{:?}", stored.snapshot).contains("2"));
        assert!(!format!("{:?}", stored.snapshot).contains("99"));
    }

    #[test]
    fn read_object_artifact_rejects_tampered_content() {
        let repo = modified_repo();
        let artifact = write_current_object_artifact(&repo);
        let path =
            object_artifact_path_for_hash(&resolved_store_dir(repo.path()), &artifact.content_hash);

        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        // Tamper a field inside the v2 content hash. `DiffFile` is snake_case,
        // unlike the camelCase `ObjectArtifact` wrapper.
        json["snapshot"]["files"][0]["new_path"] = serde_json::json!("/evil");
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

        let error = read_object_artifact(repo.path(), &artifact.snapshot.object_id)
            .expect_err("tampered artifact should be rejected");

        assert!(error.to_string().contains("content hash"));
    }

    #[test]
    fn content_encoding_is_excluded_from_object_artifact_content_hash() {
        // The body content hash is over the decoded body ({schema, version,
        // snapshot}); contentEncoding describes how the stored file decodes to
        // that body, so it is a sibling of contentHash, never an input to it.
        // Recompute the hash on both sides — comparing the copied content_hash
        // field would pass even if the hasher wrongly folded the encoding.
        let snapshot = DiffSnapshot::new(
            ReviewId::new("review:default"),
            ObjectId::new("obj:sha256:abc"),
            Vec::new(),
        );
        let plain = build_object_artifact_v2(snapshot.clone()).unwrap();
        let mut encoded = build_object_artifact_v2(snapshot).unwrap();
        encoded.content_encoding = vec!["zstd".to_owned()];

        assert_eq!(
            object_artifact_content_hash(&plain).unwrap(),
            object_artifact_content_hash(&encoded).unwrap()
        );
        assert_eq!(
            plain.content_hash,
            object_artifact_content_hash(&plain).unwrap(),
            "a fresh build stamps the hash it recomputes"
        );
    }

    #[test]
    fn object_artifact_hash_covers_snapshot_rows() {
        let repo = modified_repo();
        let artifact = write_current_object_artifact(&repo);
        let mut changed = artifact.clone();
        changed.snapshot.files.clear();

        assert_ne!(
            object_artifact_content_hash(&artifact).unwrap(),
            object_artifact_content_hash(&changed).unwrap()
        );
    }

    #[test]
    fn stored_artifact_hash_excludes_highlighting() {
        // A snapshot whose file WOULD highlight (real Rust code). Both read-time, view-only channels
        // — syntax tokens and intraline emphasis — live only on the wire DTO, never on the stored
        // artifact, so its content hash stays self-consistent regardless of any highlighting or
        // emphasis elsewhere in the codebase. Keeping both off the stored type is also what preserves
        // the byte-identical dedup that
        // `same_range_in_two_repos_produces_byte_identical_artifacts` relies on.
        let snapshot = DiffSnapshot::new(
            ReviewId::new("review:default"),
            ObjectId::new("obj:sha256:highlightable"),
            vec![crate::model::DiffFile {
                id: crate::model::FileId::new("src/lib.rs"),
                status: crate::model::FileStatus::Modified,
                old_path: Some("src/lib.rs".to_owned()),
                new_path: Some("src/lib.rs".to_owned()),
                old_mode: None,
                new_mode: None,
                old_oid: None,
                new_oid: None,
                similarity: None,
                is_binary: false,
                is_submodule: false,
                is_mode_only: false,
                synthetic: false,
                metadata_rows: Vec::new(),
                hunks: vec![crate::model::ReviewHunk {
                    id: crate::model::HunkId::new("hunk:0"),
                    header: "@@ -1 +1 @@".to_owned(),
                    old_start: 1,
                    old_lines: 1,
                    new_start: 1,
                    new_lines: 1,
                    rows: vec![crate::model::DiffRow {
                        kind: crate::model::DiffRowKind::Added,
                        old_line: None,
                        new_line: Some(1),
                        text: "pub fn value() -> u32 { 1 }".to_owned(),
                    }],
                }],
            }],
        );
        let artifact = build_object_artifact_v2(snapshot).unwrap();

        let recomputed = object_artifact_content_hash(&artifact).unwrap();
        assert_eq!(artifact.content_hash, recomputed);

        let serialized = serde_json::to_string(&artifact).unwrap();
        assert!(
            !serialized.contains("\"tokens\""),
            "stored ObjectArtifact must never serialize tokens"
        );
        assert!(
            !serialized.contains("\"emphasis\""),
            "stored ObjectArtifact must never serialize intraline emphasis (INV-A)"
        );
    }

    #[test]
    fn object_artifact_hash_is_stable_across_json_round_trip() {
        let repo = modified_repo();
        let artifact = write_current_object_artifact(&repo);
        let stored = read_object_artifact(repo.path(), &artifact.snapshot.object_id).unwrap();
        let reparsed: ObjectArtifact =
            serde_json::from_str(&serde_json::to_string_pretty(&stored).unwrap()).unwrap();

        assert_eq!(
            stored.content_hash,
            object_artifact_content_hash(&stored).unwrap()
        );
        assert_eq!(
            stored.content_hash,
            object_artifact_content_hash(&reparsed).unwrap()
        );
    }

    #[test]
    fn object_artifact_helpers_resolve_shore_dir_from_subdirectory() {
        let repo = modified_repo();
        fs::create_dir_all(repo.path().join("src")).unwrap();
        let files = capture_worktree_diff_files(repo.path()).unwrap();
        let fingerprint = compute_revision_fingerprint(repo.path()).unwrap();
        let snapshot = DiffSnapshot::new(
            ReviewId::new("review:default"),
            fingerprint.object_id.clone(),
            files,
        );

        let artifact =
            write_object_artifact(repo.path().join("src"), &fingerprint, snapshot).unwrap();
        let read =
            read_object_artifact(repo.path().join("src"), &artifact.snapshot.object_id).unwrap();

        assert_eq!(read, artifact);
    }

    #[test]
    fn write_validation_artifact_read_prefers_resolved_store_when_present() {
        let repo = modified_repo();
        // The artifact lands in the resolved (shared common-dir) store, exactly
        // where capture writes it, so the read resolves it without any fallback.
        let artifact = write_current_object_artifact(&repo);

        let read = read_bound_object_artifact_for_write_validation(
            repo.path(),
            &artifact.snapshot.object_id,
            &artifact.content_hash,
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
        let fingerprint = compute_revision_fingerprint(repo.path()).unwrap();

        let missing_hash = format!("sha256:{}", "0".repeat(64));
        let error = read_bound_object_artifact_for_write_validation(
            repo.path(),
            &fingerprint.object_id,
            &missing_hash,
        )
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

    /// Test convenience: write a object artifact to the worktree's resolved
    /// write store — the shared common-dir store by default, exactly where
    /// capture lands it. Production has a single snapshot writer
    /// (`write_object_artifact_to`); capture resolves the write store once and
    /// calls it directly. Resolving the store here keeps these reads and the
    /// production read surface on the same store.
    fn write_object_artifact(
        repo: impl AsRef<Path>,
        fingerprint: &RevisionFingerprint,
        snapshot: DiffSnapshot,
    ) -> Result<ObjectArtifact> {
        let resolution = resolve_store(repo.as_ref())?;
        write_object_artifact_to(resolution.backend(), fingerprint, snapshot)
    }

    fn write_current_object_artifact(repo: &TestRepo) -> ObjectArtifact {
        let files = capture_worktree_diff_files(repo.path()).unwrap();
        let fingerprint = compute_revision_fingerprint(repo.path()).unwrap();
        let snapshot = DiffSnapshot::new(
            ReviewId::new("review:default"),
            fingerprint.object_id.clone(),
            files,
        );

        write_object_artifact(repo.path(), &fingerprint, snapshot).unwrap()
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
    /// `object_id` while the differing canonical worktree root mints a distinct
    /// `revision_id` — exactly the two-worktree shape of #146.
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
