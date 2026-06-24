//! Validated content access over a byte [`ContentStore`].
//!
//! This is the one place that owns "validate then store" and "fetch then
//! validate" for both object artifacts and note bodies — the content-hash
//! validation half that used to be split across the object and body modules and
//! the raw `std::fs` reads under them. The content-hash math itself stays in
//! those modules; this wrapper owns the flow, so one place sits above the byte
//! store and a backend can never re-encode a blob and shift the bytes a digest
//! is validated over.

use std::path::Path;

use super::backend::{ContentStore, LocalContentStore, StoreBackend};
use super::body_artifact::{parse_note_body_artifact, validate_note_body_artifact_bytes};
use super::object_artifact::{
    ObjectArtifact, decode_and_validate_object_artifact, object_content_ref,
};
use crate::error::{Result, ShoreError};
use crate::model::ObjectId;
use crate::storage::{CreateOutcome, RemoveOutcome};

/// Validated put/get for content-addressed artifacts over a byte
/// [`ContentStore`]. Built per operation today; a later change injects the
/// backend handle directly.
pub(crate) struct ContentArtifacts {
    store: Box<dyn ContentStore>,
}

impl ContentArtifacts {
    /// Build over the file content store rooted at `store_dir`.
    pub(crate) fn local(store_dir: &Path) -> Self {
        Self {
            store: Box::new(LocalContentStore::new(store_dir)),
        }
    }

    /// Build over the content store a resolved backend yields. The constructor
    /// production consumers use, so the resolved backend flows through; `local`
    /// stays for `store_dir`-keyed callers and direct file-store access.
    pub(crate) fn from_backend(backend: &StoreBackend) -> Self {
        Self {
            store: backend.content_store(),
        }
    }

    // --- object artifacts ---

    /// Store an object artifact's bytes at `content_ref`, deduping on a
    /// snapshot-content match: a byte-identical artifact already present returns
    /// the existing one; a different artifact under the same locator is a loud
    /// conflict.
    pub(crate) fn put_object(
        &self,
        content_ref: &str,
        bytes: &[u8],
        artifact: ObjectArtifact,
    ) -> Result<ObjectArtifact> {
        match self.store.put_once(content_ref, bytes)? {
            CreateOutcome::Created => Ok(artifact),
            CreateOutcome::AlreadyExists => {
                // The locator already holds a blob, so this read expects it
                // present — an absent blob here is a write race, not the
                // "import the referenced artifacts" case the read surfaces map.
                let existing_bytes = self.store.get(content_ref)?;
                let existing = decode_and_validate_object_artifact(&existing_bytes)?;
                if existing.snapshot == artifact.snapshot {
                    Ok(existing)
                } else {
                    Err(ShoreError::Message(format!(
                        "object artifact conflict for {}",
                        artifact.snapshot.object_id.as_str()
                    )))
                }
            }
        }
    }

    /// Fetch an object artifact's stored bytes, mapping an absent blob to the
    /// canonical "import referenced artifacts" guidance.
    pub(crate) fn read_object_bytes(
        &self,
        content_ref: &str,
        object_id: &ObjectId,
    ) -> Result<Vec<u8>> {
        match self.store.get_if_exists(content_ref)? {
            Some(bytes) => Ok(bytes),
            None => Err(missing_object_artifact(object_id)),
        }
    }

    /// Fetch an object artifact's stored bytes, or `None` when absent (the
    /// resolver tries the next store on a miss).
    pub(crate) fn read_object_bytes_if_exists(&self, content_ref: &str) -> Result<Option<Vec<u8>>> {
        self.store.get_if_exists(content_ref)
    }

    /// Import a fetched object artifact's bytes: decode + validate, confirm the
    /// locator (`object_id`) and the expected content hash, then store
    /// create-if-absent. A blob already present must be the byte-equal artifact
    /// (full compare) or it is a loud conflict — never silently kept.
    pub(crate) fn import_object(
        &self,
        object_id: &ObjectId,
        expected_content_hash: &str,
        bytes: &[u8],
    ) -> Result<CreateOutcome> {
        let artifact = decode_and_validate_object_artifact(bytes)?;
        if artifact.snapshot.object_id != *object_id {
            return Err(ShoreError::Message(format!(
                "object artifact locator mismatch for {}",
                object_id.as_str()
            )));
        }
        if artifact.content_hash != expected_content_hash {
            return Err(ShoreError::Message(format!(
                "object artifact content hash mismatch for {expected_content_hash}"
            )));
        }
        let content_ref = object_content_ref(object_id);
        match self.store.put_once(&content_ref, bytes)? {
            CreateOutcome::Created => Ok(CreateOutcome::Created),
            CreateOutcome::AlreadyExists => {
                let existing = decode_and_validate_object_artifact(&self.store.get(&content_ref)?)?;
                if existing == artifact {
                    Ok(CreateOutcome::AlreadyExists)
                } else {
                    Err(ShoreError::Message(format!(
                        "object artifact conflict for {}",
                        object_id.as_str()
                    )))
                }
            }
        }
    }

    // --- note bodies ---

    /// Store a staged note-body artifact's bytes at `content_ref`. The locator is
    /// content-addressed (`artifacts/notes/<sha256(body)>.json`), so a faithful
    /// re-stage of the same body is byte-identical and dedups. A blob already
    /// present under the locator but **differing** from the staged bytes is a
    /// corrupt or tampered store, surfaced as a loud conflict rather than silently
    /// kept and then referenced by the recorded event (the same guard `put_object`
    /// applies to object artifacts).
    pub(crate) fn put_note_body(&self, content_ref: &str, bytes: &[u8]) -> Result<CreateOutcome> {
        match self.store.put_once(content_ref, bytes)? {
            CreateOutcome::Created => Ok(CreateOutcome::Created),
            CreateOutcome::AlreadyExists => {
                let existing = self.store.get(content_ref)?;
                if existing == bytes {
                    Ok(CreateOutcome::AlreadyExists)
                } else {
                    Err(ShoreError::Message(format!(
                        "note body artifact conflict for {content_ref}"
                    )))
                }
            }
        }
    }

    /// Import a fetched note-body artifact's bytes: validate against
    /// `expected_content_hash` (locator-hash + content-hash + schema), then store
    /// create-if-absent. A blob already present must decode to the byte-equal
    /// envelope (decoded compare, not raw-byte, so a byte-different-but-equal
    /// re-encode still dedups) or it is a loud conflict.
    pub(crate) fn import_body(
        &self,
        content_ref: &str,
        expected_content_hash: &str,
        bytes: &[u8],
    ) -> Result<CreateOutcome> {
        let artifact =
            validate_note_body_artifact_bytes(content_ref, expected_content_hash, bytes)?;
        match self.store.put_once(content_ref, bytes)? {
            CreateOutcome::Created => Ok(CreateOutcome::Created),
            CreateOutcome::AlreadyExists => {
                let existing = validate_note_body_artifact_bytes(
                    content_ref,
                    expected_content_hash,
                    &self.store.get(content_ref)?,
                )?;
                if existing == artifact {
                    Ok(CreateOutcome::AlreadyExists)
                } else {
                    Err(ShoreError::Message(format!(
                        "note body artifact conflict for {expected_content_hash}"
                    )))
                }
            }
        }
    }

    /// Fetch a note-body artifact's stored bytes and validate them against
    /// `expected_content_hash` (locator-hash + content-hash check via
    /// `validate_note_body_artifact_bytes`), returning the raw validated bytes. The
    /// export seam needs the bytes, not a parsed body; a missing blob maps to the
    /// canonical "import referenced artifacts" guidance.
    pub(crate) fn read_note_body_bytes(
        &self,
        content_ref: &str,
        expected_content_hash: &str,
    ) -> Result<Vec<u8>> {
        match self.store.get_if_exists(content_ref)? {
            Some(bytes) => {
                validate_note_body_artifact_bytes(content_ref, expected_content_hash, &bytes)?;
                Ok(bytes)
            }
            None => Err(ShoreError::Message(format!(
                "missing artifact {expected_content_hash}; import referenced artifacts before reading"
            ))),
        }
    }

    /// Fetch and parse a note body artifact, mapping an absent blob to the
    /// canonical "import referenced artifacts" guidance.
    pub(crate) fn read_note_body(&self, content_ref: &str) -> Result<String> {
        match self.store.get_if_exists(content_ref)? {
            Some(bytes) => Ok(parse_note_body_artifact(&bytes)?.body),
            None => Err(ShoreError::Message(format!(
                "missing artifact {content_ref}; import referenced artifacts before reading"
            ))),
        }
    }

    // --- content-addressed maintenance (the compact sweep) ---

    /// Every stored locator under `prefix`, in deterministic order.
    pub(crate) fn list_refs(&self, prefix: &str) -> Result<Vec<String>> {
        self.store.list(prefix)
    }

    /// The stored bytes for `content_ref`, or `None` when absent.
    pub(crate) fn get_if_exists(&self, content_ref: &str) -> Result<Option<Vec<u8>>> {
        self.store.get_if_exists(content_ref)
    }

    /// Plain unlink of the blob at `content_ref`. The re-hash-before-erase floor
    /// is the caller's, above this.
    pub(crate) fn remove(&self, content_ref: &str) -> Result<RemoveOutcome> {
        self.store.remove(content_ref)
    }
}

fn missing_object_artifact(object_id: &ObjectId) -> ShoreError {
    ShoreError::Message(format!(
        "missing artifact for snapshot {}; import referenced artifacts before reading",
        object_id.as_str()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical_hash::sha256_bytes_hex;
    use crate::model::{DiffSnapshot, ReviewId};
    use crate::session::store::body_artifact::NoteBodyEnvelope;
    use crate::session::store::object_artifact::build_object_artifact_v2;

    /// Every content-wrapper assertion runs over each backend in turn — the file
    /// backend (rooted at a temp dir the guard keeps alive) and the
    /// injection-only in-memory backend — so the put-with-dedup and
    /// get-with-validation flow is proven backend-agnostic.
    fn each_backend() -> Vec<(Option<tempfile::TempDir>, StoreBackend)> {
        let root = tempfile::tempdir().unwrap();
        let store_dir = root.path().join(".shore/data");
        vec![
            (Some(root), StoreBackend::Local(store_dir)),
            (None, StoreBackend::memory()),
        ]
    }

    fn valid_object() -> (ObjectArtifact, Vec<u8>) {
        let artifact =
            build_object_artifact_v2(DiffSnapshot::empty(ReviewId::new("review:test"))).unwrap();
        let bytes = serde_json::to_vec(&artifact).unwrap();
        (artifact, bytes)
    }

    #[test]
    fn object_put_dedup_and_read_validate_hold_over_every_backend() {
        let (artifact, bytes) = valid_object();
        let object_id = artifact.snapshot.object_id.clone();
        let content_ref = "artifacts/objects/test.json";

        for (_guard, backend) in each_backend() {
            let content = ContentArtifacts::from_backend(&backend);

            // First put returns the artifact; a byte-identical second put dedups
            // to the same one (the snapshot matches the stored blob).
            assert_eq!(
                content
                    .put_object(content_ref, &bytes, artifact.clone())
                    .unwrap(),
                artifact
            );
            assert_eq!(
                content
                    .put_object(content_ref, &bytes, artifact.clone())
                    .unwrap(),
                artifact
            );

            // The stored bytes read back verbatim and decode-validate.
            let read = content.read_object_bytes(content_ref, &object_id).unwrap();
            assert_eq!(read, bytes);
            assert_eq!(
                decode_and_validate_object_artifact(&read).unwrap(),
                artifact
            );

            // A different snapshot under the same locator is a loud conflict.
            let other = build_object_artifact_v2(DiffSnapshot::new(
                ReviewId::new("review:other"),
                ObjectId::new("other"),
                Vec::new(),
            ))
            .unwrap();
            let other_bytes = serde_json::to_vec(&other).unwrap();
            assert!(
                content
                    .put_object(content_ref, &other_bytes, other)
                    .unwrap_err()
                    .to_string()
                    .contains("conflict")
            );

            // A missing locator reads as None / the import guidance.
            assert_eq!(
                content
                    .read_object_bytes_if_exists("artifacts/objects/missing.json")
                    .unwrap(),
                None
            );
            assert!(
                content
                    .read_object_bytes("artifacts/objects/missing.json", &object_id)
                    .unwrap_err()
                    .to_string()
                    .contains("import referenced artifacts")
            );
        }
    }

    #[test]
    fn put_note_body_creates_then_dedups_over_every_backend() {
        // The staged-note-body write goes through the wrapper, so a non-Local
        // backend captures it: a first put creates, an identical second put dedups
        // (note bodies are content-addressed), and the body reads back.
        let valid = NoteBodyEnvelope::new("the staged body".to_owned())
            .to_json_bytes()
            .unwrap();
        let content_ref = "artifacts/notes/staged.json";

        for (_guard, backend) in each_backend() {
            let content = ContentArtifacts::from_backend(&backend);
            assert_eq!(
                content.put_note_body(content_ref, &valid).unwrap(),
                CreateOutcome::Created
            );
            assert_eq!(
                content.put_note_body(content_ref, &valid).unwrap(),
                CreateOutcome::AlreadyExists
            );
            assert_eq!(
                content.read_note_body(content_ref).unwrap(),
                "the staged body"
            );

            // A blob already present but differing from the staged bytes is a loud
            // conflict, not a silent keep — a corrupt/tampered store must not be
            // referenced by a freshly recorded event.
            let divergent = NoteBodyEnvelope::new("a different body".to_owned())
                .to_json_bytes()
                .unwrap();
            assert!(
                content
                    .put_note_body(content_ref, &divergent)
                    .unwrap_err()
                    .to_string()
                    .contains("conflict")
            );
        }
    }

    #[test]
    fn note_body_read_parses_and_validates_over_every_backend() {
        let valid = NoteBodyEnvelope::new("the body".to_owned())
            .to_json_bytes()
            .unwrap();
        let wrong_schema = br#"{"schema":"wrong","version":1,"body":"x"}"#;

        for (_guard, backend) in each_backend() {
            let store = backend.content_store();
            store.put_once("artifacts/notes/good.json", &valid).unwrap();
            store
                .put_once("artifacts/notes/bad.json", wrong_schema)
                .unwrap();

            let content = ContentArtifacts::from_backend(&backend);
            assert_eq!(
                content.read_note_body("artifacts/notes/good.json").unwrap(),
                "the body"
            );
            assert!(
                content
                    .read_note_body("artifacts/notes/bad.json")
                    .unwrap_err()
                    .to_string()
                    .contains("Unsupported note body artifact")
            );
            assert!(
                content
                    .read_note_body("artifacts/notes/missing.json")
                    .unwrap_err()
                    .to_string()
                    .contains("import referenced artifacts")
            );
        }
    }

    #[test]
    fn read_note_body_bytes_validates_and_returns_raw_bytes_over_every_backend() {
        let body = "the exported body";
        let valid = NoteBodyEnvelope::new(body.to_owned())
            .to_json_bytes()
            .unwrap();
        let hash = format!("sha256:{}", sha256_bytes_hex(body.as_bytes()));
        let content_ref = format!("artifacts/notes/{}.json", sha256_bytes_hex(body.as_bytes()));

        for (_guard, backend) in each_backend() {
            backend
                .content_store()
                .put_once(&content_ref, &valid)
                .unwrap();
            let content = ContentArtifacts::from_backend(&backend);

            // Valid: returns the stored bytes verbatim.
            assert_eq!(
                content.read_note_body_bytes(&content_ref, &hash).unwrap(),
                valid
            );

            // Content-hash mismatch (right locator, wrong expected hash) is rejected.
            let wrong_hash = "sha256:".to_owned() + &"0".repeat(64);
            assert!(
                content
                    .read_note_body_bytes(&content_ref, &wrong_hash)
                    .unwrap_err()
                    .to_string()
                    .contains("hash mismatch")
            );

            // Missing blob maps to the import guidance.
            assert!(
                content
                    .read_note_body_bytes("artifacts/notes/missing.json", &hash)
                    .unwrap_err()
                    .to_string()
                    .contains("import referenced artifacts")
            );
        }
    }

    #[test]
    fn import_object_validates_locator_hash_and_dedups_or_conflicts_over_every_backend() {
        let (artifact, bytes) = valid_object();
        let object_id = artifact.snapshot.object_id.clone();
        let expected = artifact.content_hash.clone();

        for (_guard, backend) in each_backend() {
            let content = ContentArtifacts::from_backend(&backend);

            // First import creates; a byte-identical second import dedups.
            assert_eq!(
                content
                    .import_object(&object_id, &expected, &bytes)
                    .unwrap(),
                CreateOutcome::Created
            );
            assert_eq!(
                content
                    .import_object(&object_id, &expected, &bytes)
                    .unwrap(),
                CreateOutcome::AlreadyExists
            );

            // Wrong expected content hash → "content hash mismatch".
            let wrong = "sha256:".to_owned() + &"0".repeat(64);
            assert!(
                content
                    .import_object(&object_id, &wrong, &bytes)
                    .unwrap_err()
                    .to_string()
                    .contains("content hash mismatch")
            );

            // Locator mismatch: bytes whose snapshot.object_id != the requested object_id.
            let other = build_object_artifact_v2(DiffSnapshot::new(
                ReviewId::new("review:other"),
                ObjectId::new("obj:sha256:other"),
                Vec::new(),
            ))
            .unwrap();
            let other_bytes = serde_json::to_vec(&other).unwrap();
            assert!(
                content
                    .import_object(&object_id, &other.content_hash, &other_bytes)
                    .unwrap_err()
                    .to_string()
                    .contains("locator mismatch")
            );

            // Conflict: a DIFFERENT valid artifact already occupies this object's locator.
            // Two artifacts sharing object_id `collide` but differing in `review_id` hash to
            // different content (review_id is serialized into the content hash), so the second
            // import collides on the locator and the full-struct compare rejects it.
            let collide = ObjectId::new("obj:sha256:collide");
            let stored = build_object_artifact_v2(DiffSnapshot::new(
                ReviewId::new("review:a"),
                collide.clone(),
                Vec::new(),
            ))
            .unwrap();
            let incoming = build_object_artifact_v2(DiffSnapshot::new(
                ReviewId::new("review:b"),
                collide.clone(),
                Vec::new(),
            ))
            .unwrap();
            assert_ne!(
                stored.content_hash, incoming.content_hash,
                "the fixtures diverge"
            );
            content
                .import_object(
                    &collide,
                    &stored.content_hash,
                    &serde_json::to_vec(&stored).unwrap(),
                )
                .unwrap();
            assert!(
                content
                    .import_object(
                        &collide,
                        &incoming.content_hash,
                        &serde_json::to_vec(&incoming).unwrap()
                    )
                    .unwrap_err()
                    .to_string()
                    .contains("conflict")
            );
        }
    }

    #[test]
    fn import_body_validates_and_dedups_or_conflicts_over_every_backend() {
        let body = "imported body";
        let valid = NoteBodyEnvelope::new(body.to_owned())
            .to_json_bytes()
            .unwrap();
        let hash = format!("sha256:{}", sha256_bytes_hex(body.as_bytes()));
        let content_ref = format!("artifacts/notes/{}.json", sha256_bytes_hex(body.as_bytes()));

        for (_guard, backend) in each_backend() {
            let content = ContentArtifacts::from_backend(&backend);

            // First import creates; a byte-identical re-import dedups.
            assert_eq!(
                content.import_body(&content_ref, &hash, &valid).unwrap(),
                CreateOutcome::Created
            );
            assert_eq!(
                content.import_body(&content_ref, &hash, &valid).unwrap(),
                CreateOutcome::AlreadyExists
            );

            // A byte-DIFFERENT but semantically-equal re-import (same body, different JSON
            // formatting) still dedups — the dedup compares the decoded NoteBodyEnvelope, not
            // raw bytes (matching the live import path and the decoded-content identity rule).
            let pretty =
                serde_json::to_vec_pretty(&NoteBodyEnvelope::new(body.to_owned())).unwrap();
            assert_ne!(pretty, valid, "the fixture is byte-different");
            assert_eq!(
                content.import_body(&content_ref, &hash, &pretty).unwrap(),
                CreateOutcome::AlreadyExists
            );

            // A wrong-schema blob is rejected before the write.
            let wrong_schema = br#"{"schema":"wrong","version":1,"body":"x"}"#;
            assert!(
                content
                    .import_body(&content_ref, &hash, wrong_schema)
                    .unwrap_err()
                    .to_string()
                    .contains("Unsupported note body artifact")
            );
        }
    }

    #[test]
    fn read_then_decode_rejects_tampered_object_bytes_over_every_backend() {
        // Inject object bytes whose stored contentHash no longer matches the
        // snapshot (the write-side validation skipped via `put_raw`), then assert
        // the wrapper read + decode rejects them — the content-hash validation is
        // backend-agnostic.
        let (artifact, bytes) = valid_object();
        let object_id = artifact.snapshot.object_id.clone();
        let content_ref = "artifacts/objects/test.json";
        let mut json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        json["contentHash"] = serde_json::json!(
            "sha256:0000000000000000000000000000000000000000000000000000000000000000"
        );
        let tampered = serde_json::to_vec(&json).unwrap();

        for (_guard, backend) in each_backend() {
            backend
                .content_store()
                .put_raw(content_ref, &tampered)
                .unwrap();

            let content = ContentArtifacts::from_backend(&backend);
            let read = content.read_object_bytes(content_ref, &object_id).unwrap();
            let error = decode_and_validate_object_artifact(&read)
                .expect_err("tampered object bytes are rejected on decode");
            assert!(error.to_string().contains("content hash"));
        }
    }
}
