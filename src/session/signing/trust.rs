use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::Value;

use crate::crypto::SignerId;
use crate::error::{Result, ShoreError};
use crate::model::ActorId;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TrustSet {
    allowed_signers: BTreeMap<ActorId, BTreeSet<SignerId>>,
}

impl TrustSet {
    pub fn from_allowed_signers_file(path: impl AsRef<Path>) -> Result<Self> {
        let bytes =
            std::fs::read(path.as_ref()).map_err(|error| ShoreError::WorkflowInputInvalid {
                reason: format!(
                    "failed to read allowed-signers file {}: {error}",
                    path.as_ref().display()
                ),
            })?;
        event_signature_trust_set(serde_json::from_slice(&bytes)?)
    }

    /// True when `signer` is an authorized signer for **any** actor in the
    /// allow-list. Actor-agnostic membership for surfaces that ask only "is this
    /// key enrolled?" (e.g. `keys list`); unlike `authorizes`, it carries no
    /// self-signing shortcut.
    pub fn contains_signer(&self, signer: &SignerId) -> bool {
        self.allowed_signers
            .values()
            .any(|signers| signers.contains(signer))
    }

    /// Borrow the actor → signers map (read-only). Used by the enrollment writer
    /// to serialize the set back to the on-disk JSON shape and to compute the
    /// added/already-present diff via explicit membership.
    pub(crate) fn allowed_signers(&self) -> &BTreeMap<ActorId, BTreeSet<SignerId>> {
        &self.allowed_signers
    }

    /// Insert `signer` under `actor`, returning whether the pair was newly added.
    /// Re-inserting an existing pair is a no-op (returns `false`).
    pub(crate) fn insert_signer(&mut self, actor: ActorId, signer: SignerId) -> bool {
        self.allowed_signers
            .entry(actor)
            .or_default()
            .insert(signer)
    }

    /// The actors that **explicitly** enroll `signer` in their allowed-signers entry:
    /// `{X : signer ∈ allowed-signers[X]}`. Unlike [`authorizes`], this carries **no**
    /// self-cert shortcut and never manufactures a self actor from a bare `did:key` — a
    /// signer with no explicit mapping resolves to zero actors. This is the
    /// endorsement-classification reverse resolver (ADR-0013); endorsement trust is never
    /// inferred from key syntax.
    pub(crate) fn reverse_resolve(&self, signer: &SignerId) -> BTreeSet<ActorId> {
        self.allowed_signers
            .iter()
            .filter(|(_, signers)| signers.contains(signer))
            .map(|(actor, _)| actor.clone())
            .collect()
    }

    pub fn authorizes(&self, actor: &ActorId, signer: &SignerId, _occurred_at: &str) -> bool {
        if SignerId::parse(actor.as_str())
            .map(|actor_signer| actor_signer == *signer)
            .unwrap_or(false)
        {
            return true;
        }

        self.allowed_signers
            .get(actor)
            .map(|signers| signers.contains(signer))
            .unwrap_or(false)
    }
}

pub fn event_signature_trust_set(value: Value) -> Result<TrustSet> {
    let allowed = value
        .get("allowedSigners")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid_trust_set("missing allowedSigners object"))?;
    let mut allowed_signers = BTreeMap::new();

    for (actor, signers) in allowed {
        let signers = signers.as_array().ok_or_else(|| {
            invalid_trust_set(format!("allowed signers for {actor} must be an array"))
        })?;
        let mut parsed_signers = BTreeSet::new();
        for signer in signers {
            let signer = signer.as_str().ok_or_else(|| {
                invalid_trust_set(format!("allowed signer for {actor} must be a string"))
            })?;
            parsed_signers.insert(SignerId::parse(signer)?);
        }
        allowed_signers.insert(ActorId::new(actor), parsed_signers);
    }

    Ok(TrustSet { allowed_signers })
}

fn invalid_trust_set(reason: impl Into<String>) -> ShoreError {
    ShoreError::WorkflowInputInvalid {
        reason: format!("invalid event signature trust set: {}", reason.into()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{TrustSet, event_signature_trust_set};
    use crate::crypto::SignerId;
    use crate::model::ActorId;

    const ENROLLED: &str = "did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd";
    const ABSENT: &str = "did:key:z6MkpTHR8VNsBxYAAWHut2Geadd9jSwuBV8xRoAnwWsdvktH";

    const KEY_A: &str = "did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd";
    const KEY_B: &str = "did:key:z6MkpTHR8VNsBxYAAWHut2Geadd9jSwuBV8xRoAnwWsdvktH";

    #[test]
    fn contains_signer_is_actor_agnostic_membership() {
        let trust = event_signature_trust_set(json!({
            "allowedSigners": { "actor:git-email:alice@example.com": [ENROLLED] }
        }))
        .unwrap();

        assert!(trust.contains_signer(&SignerId::parse(ENROLLED).unwrap()));
        assert!(!trust.contains_signer(&SignerId::parse(ABSENT).unwrap()));
    }

    #[test]
    fn reverse_resolve_returns_each_enrolling_actor() {
        let trust = event_signature_trust_set(json!({
            "allowedSigners": {
                "actor:git-email:alice@example.com": [KEY_A],
                "actor:agent:bot": [KEY_A, KEY_B]
            }
        }))
        .unwrap();
        let actors = trust.reverse_resolve(&SignerId::parse(KEY_A).unwrap());
        let got: Vec<String> = actors.iter().map(|a| a.as_str().to_owned()).collect();
        assert_eq!(
            got,
            vec![
                "actor:agent:bot".to_string(),
                "actor:git-email:alice@example.com".to_string()
            ]
        );
        // BTreeSet => sorted, deduped.
    }

    #[test]
    fn reverse_resolve_unenrolled_signer_is_empty() {
        let trust = event_signature_trust_set(json!({
            "allowedSigners": { "actor:git-email:alice@example.com": [KEY_A] }
        }))
        .unwrap();
        assert!(
            trust
                .reverse_resolve(&SignerId::parse(KEY_B).unwrap())
                .is_empty()
        );
    }

    #[test]
    fn reverse_resolve_never_self_certs_a_bare_did_key() {
        // A did:key actor whose signer == the actor id is authorized via the self-cert
        // shortcut in `authorizes`, but reverse_resolve must NOT manufacture that self actor
        // (it reads allowed-signers ONLY). An empty allow-list yields zero actors. (INV-3)
        let trust = TrustSet::default();
        let signer = SignerId::parse(KEY_A).unwrap();
        assert!(trust.authorizes(&ActorId::new(KEY_A), &signer, "2026-06-18T00:00:00Z"));
        assert!(trust.reverse_resolve(&signer).is_empty());
    }
}
