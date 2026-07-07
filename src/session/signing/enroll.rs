//! Writer side of the committed signature allow-list. The on-disk shape is a
//! custom Pointbreak JSON document — `{"allowedSigners": {"<actor>": ["<did:key>",
//! …]}}` — **not** the OpenSSH `allowed_signers` line format despite the filename.
//! Reader side lives in `trust.rs`; this module adds the symmetric writer.

use std::path::Path;

use serde_json::{Map, Value};

use super::TrustSet;
use crate::crypto::SignerId;
use crate::error::{Result, ShoreError};
use crate::model::ActorId;

/// Repo-relative path to the committed allow-list. Custom Pointbreak JSON, not the
/// OpenSSH `allowed_signers` format despite the filename.
pub const ALLOWED_SIGNERS_REL_PATH: &str = ".shore/allowed-signers.json";

/// Outcome of a single enrollment: whether the `(actor, did:key)` pair was newly
/// added (`true`) or already present (`false`, a no-op).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnrollmentDiff {
    pub added: bool,
}

/// Pure: return `existing` (or an empty set) with `(actor, signer_id)` inserted.
/// Re-enrolling an already-present pair yields an equal set. Ordering is stable
/// because `TrustSet` stores `BTreeMap`/`BTreeSet`.
pub fn enroll_signer(
    existing: Option<TrustSet>,
    actor: &ActorId,
    signer_id: &SignerId,
) -> TrustSet {
    let mut set = existing.unwrap_or_default();
    set.insert_signer(actor.clone(), signer_id.clone());
    set
}

/// Serialize a `TrustSet` back to the on-disk `{"allowedSigners": {...}}` shape.
/// `event_signature_trust_set(trust_set_to_value(&set))` returns an equal set.
pub fn trust_set_to_value(set: &TrustSet) -> Value {
    let mut allowed = Map::new();
    for (actor, signers) in set.allowed_signers() {
        let array = signers
            .iter()
            .map(|signer| Value::String(signer.as_str().to_owned()))
            .collect();
        allowed.insert(actor.as_str().to_owned(), Value::Array(array));
    }
    let mut root = Map::new();
    root.insert("allowedSigners".to_owned(), Value::Object(allowed));
    Value::Object(root)
}

/// Read-or-init `path`, add `(actor, signer_id)`, and write it back. Reports
/// whether the pair was newly added. Creates the parent directory tree if absent.
/// Serialization is stable (sorted keys), so re-enrolling an existing pair
/// rewrites byte-identical content.
pub fn stage_enrollment(
    path: &Path,
    actor: &ActorId,
    signer_id: &SignerId,
) -> Result<EnrollmentDiff> {
    let existing = if path.exists() {
        Some(TrustSet::from_allowed_signers_file(path)?)
    } else {
        None
    };
    // `added` must be computed from EXPLICIT map membership, never `TrustSet::authorizes`:
    // `authorizes` returns true on the `actor == signer` self-signing shortcut even when the
    // pair is not in the allow-list, so a self-certifying did:key actor would report
    // `added: false` while we write a new explicit entry. Check the map directly.
    let already_present = existing
        .as_ref()
        .and_then(|set| set.allowed_signers().get(actor))
        .is_some_and(|signers| signers.contains(signer_id));
    let added = !already_present;
    let updated = enroll_signer(existing, actor, signer_id);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            ShoreError::Message(format!("create {}: {error}", parent.display()))
        })?;
    }
    let mut bytes = serde_json::to_vec_pretty(&trust_set_to_value(&updated))?;
    bytes.push(b'\n');
    std::fs::write(path, &bytes)
        .map_err(|error| ShoreError::Message(format!("write {}: {error}", path.display())))?;
    Ok(EnrollmentDiff { added })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::SignerId;
    use crate::model::ActorId;
    use crate::session::signing::event_signature_trust_set;

    const ACTOR: &str = "actor:git-email:alice@example.com";
    const DID_A: &str = "did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd";
    // A second, distinct valid Ed25519 did:key for the "append" case.
    const DID_B: &str = "did:key:z6MkpTHR8VNsBxYAAWHut2Geadd9jSwuBV8xRoAnwWsdvktH";

    #[test]
    fn enroll_into_empty_creates_single_mapping() {
        let actor = ActorId::new(ACTOR);
        let signer = SignerId::parse(DID_A).unwrap();

        let set = enroll_signer(None, &actor, &signer);

        // Round-trips through the existing reader and authorizes the pair.
        let value = trust_set_to_value(&set);
        let reparsed = event_signature_trust_set(value).unwrap();
        assert!(reparsed.authorizes(&actor, &signer, "2026-06-16T00:00:00Z"));
    }

    #[test]
    fn second_did_for_same_actor_appends_sorted() {
        let actor = ActorId::new(ACTOR);
        let first = SignerId::parse(DID_A).unwrap();
        let second = SignerId::parse(DID_B).unwrap();

        let set = enroll_signer(Some(enroll_signer(None, &actor, &first)), &actor, &second);

        let value = trust_set_to_value(&set);
        // Both did:keys present, and the array is sorted (BTreeSet ordering).
        let signers = value["allowedSigners"][ACTOR].as_array().unwrap();
        let rendered: Vec<&str> = signers.iter().map(|s| s.as_str().unwrap()).collect();
        let mut expected = vec![DID_A, DID_B];
        expected.sort_unstable();
        assert_eq!(rendered, expected);
    }

    #[test]
    fn re_enroll_of_existing_pair_is_a_noop() {
        let actor = ActorId::new(ACTOR);
        let signer = SignerId::parse(DID_A).unwrap();

        let once = enroll_signer(None, &actor, &signer);
        let twice = enroll_signer(Some(once.clone()), &actor, &signer);

        assert_eq!(once, twice, "re-enrolling the same pair changes nothing");
    }

    #[test]
    fn serialized_top_level_key_is_allowed_signers_camel_case() {
        let actor = ActorId::new(ACTOR);
        let signer = SignerId::parse(DID_A).unwrap();
        let value = trust_set_to_value(&enroll_signer(None, &actor, &signer));

        assert!(
            value.get("allowedSigners").is_some(),
            "top-level key is the camelCase allowedSigners, not OpenSSH lines"
        );
    }

    #[test]
    fn stage_enrollment_into_fresh_dir_creates_readable_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".shore/allowed-signers.json");
        let actor = ActorId::new(ACTOR);
        let signer = SignerId::parse(DID_A).unwrap();

        let diff = stage_enrollment(&path, &actor, &signer).unwrap();
        assert!(diff.added, "first enrollment of a pair reports added");

        // The existing reader loads the file back with the entry.
        let reloaded = TrustSet::from_allowed_signers_file(&path).unwrap();
        assert!(reloaded.authorizes(&actor, &signer, "2026-06-16T00:00:00Z"));
    }

    #[test]
    fn stage_enrollment_is_byte_stable_on_re_enroll() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".shore/allowed-signers.json");
        let actor = ActorId::new(ACTOR);
        let signer = SignerId::parse(DID_A).unwrap();

        let first = stage_enrollment(&path, &actor, &signer).unwrap();
        let before = std::fs::read(&path).unwrap();
        let second = stage_enrollment(&path, &actor, &signer).unwrap();
        let after = std::fs::read(&path).unwrap();

        assert!(first.added && !second.added, "second enrollment is a no-op");
        assert_eq!(before, after, "re-enroll leaves the file bytes unchanged");
    }

    #[test]
    fn added_is_true_for_self_certifying_did_key_actor_first_enroll() {
        // Regression: a did:key actor whose actor id == its signer would make
        // `TrustSet::authorizes` return true via the self-signing shortcut even
        // with an empty map. `added` must instead reflect explicit map membership,
        // so the first enroll of such an actor reports added: true.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".shore/allowed-signers.json");
        let did_actor = ActorId::new(DID_A); // actor id IS the did:key
        let signer = SignerId::parse(DID_A).unwrap();

        let diff = stage_enrollment(&path, &did_actor, &signer).unwrap();
        assert!(
            diff.added,
            "first explicit enroll reports added even when actor == signer"
        );
        // And it is genuinely a no-op the second time.
        assert!(!stage_enrollment(&path, &did_actor, &signer).unwrap().added);
    }
}
