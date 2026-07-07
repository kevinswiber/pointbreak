use serde::{Deserialize, Serialize};

use super::ShoreEvent;
use crate::crypto::{EventSignatureBytes, EventVerificationStatus, SignerId};
use crate::error::Result as ShoreResult;

pub const ED25519_SIGNATURE_ALG: &str = "ed25519";
pub const EVENT_SIGNATURE_VERSION_V1: u32 = 1;

/// Single Ed25519 producer signature attached to a Pointbreak event.
///
/// Version 1 signatures sign Dead Simple Signing Envelope (DSSE)
/// pre-authentication encoding bytes for the event's canonical
/// `EventToBeSigned` view.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventSignature {
    pub alg: String,
    pub sig_version: u32,
    pub sig: EventSignatureBytes,
}

impl EventSignature {
    pub fn new_ed25519_v1(sig: impl Into<String>) -> ShoreResult<Self> {
        Ok(Self {
            alg: ED25519_SIGNATURE_ALG.to_owned(),
            sig_version: EVENT_SIGNATURE_VERSION_V1,
            sig: EventSignatureBytes::parse(sig)?,
        })
    }

    pub fn ed25519_v1(sig: EventSignatureBytes) -> Self {
        Self {
            alg: ED25519_SIGNATURE_ALG.to_owned(),
            sig_version: EVENT_SIGNATURE_VERSION_V1,
            sig,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectiveSignerError {
    status: EventVerificationStatus,
    reason: String,
}

impl EffectiveSignerError {
    pub fn status(&self) -> EventVerificationStatus {
        self.status
    }

    fn unsigned(reason: impl Into<String>) -> Self {
        Self {
            status: EventVerificationStatus::Unsigned,
            reason: reason.into(),
        }
    }

    pub(crate) fn invalid(reason: impl Into<String>) -> Self {
        Self {
            status: EventVerificationStatus::Invalid,
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for EffectiveSignerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "effective signer resolution failed: {}", self.reason)
    }
}

impl std::error::Error for EffectiveSignerError {}

pub fn resolve_effective_signer(
    event: &ShoreEvent,
) -> std::result::Result<SignerId, EffectiveSignerError> {
    if event.signature.is_none() {
        return Err(EffectiveSignerError::unsigned("event is unsigned"));
    }

    if let Some(signer) = &event.signer {
        return Ok(signer.clone());
    }

    SignerId::parse(event.writer.actor_id.as_str())
        .map_err(|_| EffectiveSignerError::invalid("signed friendly actor event is missing signer"))
}
