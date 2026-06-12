//! Shared signing fixtures for in-crate unit tests.

use ed25519_dalek::{Signer as _, SigningKey};
use serde_json::json;

use super::{TrustSet, event_signature_trust_set};
use crate::crypto::{EventSignatureBytes, EventSigner, SignerId};
use crate::error::Result;
use crate::model::ActorId;

/// Deterministic seeded Ed25519 signer for test fixtures.
#[derive(Clone)]
pub(crate) struct DeterministicSigner {
    signer_id: SignerId,
    signing_key: SigningKey,
}

impl DeterministicSigner {
    pub(crate) fn from_seed(seed: [u8; 32]) -> Self {
        let signing_key = SigningKey::from_bytes(&seed);
        let signer_id = SignerId::from_ed25519_public_key(signing_key.verifying_key().to_bytes());

        Self {
            signer_id,
            signing_key,
        }
    }

    /// The historical fixture seed (0x00..0x1f) used by the ingest tests.
    pub(crate) fn fixture() -> Self {
        let mut seed = [0u8; 32];
        for (index, byte) in seed.iter_mut().enumerate() {
            *byte = index as u8;
        }
        Self::from_seed(seed)
    }
}

impl EventSigner for DeterministicSigner {
    fn signer_id(&self) -> &SignerId {
        &self.signer_id
    }

    fn sign_event_message(&self, message: &[u8]) -> Result<EventSignatureBytes> {
        let signature = self.signing_key.sign(message);
        Ok(EventSignatureBytes::from_bytes(&signature.to_bytes()))
    }
}

/// Trust set authorizing exactly this signer for this actor.
pub(crate) fn trust_for_actor(actor: &ActorId, signer: &DeterministicSigner) -> TrustSet {
    event_signature_trust_set(json!({
        "allowedSigners": {
            actor.as_str(): [signer.signer_id().as_str()]
        }
    }))
    .expect("trust set builds")
}
