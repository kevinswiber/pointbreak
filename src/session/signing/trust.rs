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

    use super::event_signature_trust_set;
    use crate::crypto::SignerId;

    const ENROLLED: &str = "did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd";
    const ABSENT: &str = "did:key:z6MkpTHR8VNsBxYAAWHut2Geadd9jSwuBV8xRoAnwWsdvktH";

    #[test]
    fn contains_signer_is_actor_agnostic_membership() {
        let trust = event_signature_trust_set(json!({
            "allowedSigners": { "actor:git-email:alice@example.com": [ENROLLED] }
        }))
        .unwrap();

        assert!(trust.contains_signer(&SignerId::parse(ENROLLED).unwrap()));
        assert!(!trust.contains_signer(&SignerId::parse(ABSENT).unwrap()));
    }
}
