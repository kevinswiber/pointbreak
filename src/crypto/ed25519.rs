use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use ed25519_dalek::{Signature, VerifyingKey};
#[cfg(test)]
use ed25519_dalek::{Signer as _, SigningKey};
use serde::{Deserialize, Serialize};

use crate::crypto::SignerId;
use crate::error::Result;

/// Result of verifying a Pointbreak event's optional Ed25519 producer signature.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventVerificationStatus {
    Valid,
    Invalid,
    UntrustedKey,
    Unsigned,
}

impl EventVerificationStatus {
    /// The canonical status code — exactly the serde snake_case wire form.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Invalid => "invalid",
            Self::UntrustedKey => "untrusted_key",
            Self::Unsigned => "unsigned",
        }
    }
}

impl std::fmt::Display for EventVerificationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Base64-encoded Ed25519 signature bytes for a signed event.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventSignatureBytes(String);

impl EventSignatureBytes {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        Ok(Self(value.into()))
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(BASE64_STANDARD.encode(bytes))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_base64(&self) -> bool {
        BASE64_STANDARD.decode(self.0.as_bytes()).is_ok()
    }
}

/// Signs Dead Simple Signing Envelope (DSSE) pre-authentication encoding bytes.
pub trait EventSigner {
    fn signer_id(&self) -> &SignerId;
    fn sign_event_message(&self, message: &[u8]) -> Result<EventSignatureBytes>;
}

/// A boxed `EventSigner` is itself an `EventSigner`, forwarding to the inner value.
/// This lets the CLI resolve layer carry either a file-backed or an agent-backed
/// signer as one `Box<dyn EventSigner + Send + Sync>` and still call the unchanged
/// generic `EventSigningOptions::sign_with<S: EventSigner + Send + Sync + 'static>`
/// — the boxed trait object satisfies the `S: EventSigner` bound with no change to
/// `sign_with` or to this trait.
impl EventSigner for Box<dyn EventSigner + Send + Sync> {
    fn signer_id(&self) -> &SignerId {
        (**self).signer_id()
    }

    fn sign_event_message(&self, message: &[u8]) -> Result<EventSignatureBytes> {
        (**self).sign_event_message(message)
    }
}

pub fn verify_ed25519_strict(
    signer: &SignerId,
    message: &[u8],
    signature: &str,
) -> Result<EventVerificationStatus> {
    let public_key = match signer.ed25519_public_key() {
        Ok(public_key) => public_key,
        Err(_) => return Ok(EventVerificationStatus::Invalid),
    };
    let verifying_key = match VerifyingKey::from_bytes(&public_key) {
        Ok(verifying_key) => verifying_key,
        Err(_) => return Ok(EventVerificationStatus::Invalid),
    };
    let signature_bytes = match BASE64_STANDARD.decode(signature.as_bytes()) {
        Ok(signature_bytes) => signature_bytes,
        Err(_) => return Ok(EventVerificationStatus::Invalid),
    };
    let signature = match Signature::from_slice(&signature_bytes) {
        Ok(signature) => signature,
        Err(_) => return Ok(EventVerificationStatus::Invalid),
    };

    match verifying_key.verify_strict(message, &signature) {
        Ok(()) => Ok(EventVerificationStatus::Valid),
        Err(_) => Ok(EventVerificationStatus::Invalid),
    }
}

#[cfg(test)]
pub(crate) struct TestEd25519Signer {
    signer_id: SignerId,
    signing_key: SigningKey,
}

#[cfg(test)]
impl TestEd25519Signer {
    pub(crate) fn from_seed(seed: [u8; 32]) -> Self {
        let signing_key = SigningKey::from_bytes(&seed);
        let signer_id = SignerId::from_ed25519_public_key(signing_key.verifying_key().to_bytes());

        Self {
            signer_id,
            signing_key,
        }
    }
}

#[cfg(test)]
impl EventSigner for TestEd25519Signer {
    fn signer_id(&self) -> &SignerId {
        &self.signer_id
    }

    fn sign_event_message(&self, message: &[u8]) -> Result<EventSignatureBytes> {
        let signature = self.signing_key.sign(message);
        Ok(EventSignatureBytes::from_bytes(&signature.to_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{
        EventSignatureBytes, EventSigner, EventVerificationStatus, TestEd25519Signer,
        verify_ed25519_strict,
    };
    use crate::crypto::SignerId;
    use crate::error::Result;

    struct NegativeCryptoCase {
        name: String,
        signer: NegativeSigner,
        message: Vec<u8>,
        signature: EventSignatureBytes,
    }

    enum NegativeSigner {
        Parsed(SignerId),
        Malformed,
    }

    impl NegativeCryptoCase {
        fn verify(&self) -> Result<EventVerificationStatus> {
            match &self.signer {
                NegativeSigner::Parsed(signer) => {
                    verify_ed25519_strict(signer, &self.message, self.signature.as_str())
                }
                NegativeSigner::Malformed => Ok(EventVerificationStatus::Invalid),
            }
        }
    }

    #[test]
    fn ed25519_signer_trait_produces_base64_signature_that_verifies_strictly() {
        let signer = TestEd25519Signer::from_seed([7u8; 32]);
        let message = b"DSSEv1 4 test 5 hello";

        let sig = signer.sign_event_message(message).unwrap();

        assert!(sig.is_base64());
        assert_eq!(
            verify_ed25519_strict(signer.signer_id(), message, sig.as_str()).unwrap(),
            EventVerificationStatus::Valid
        );
    }

    #[test]
    fn boxed_event_signer_forwards_to_the_inner_signer() {
        let inner = TestEd25519Signer::from_seed([5u8; 32]);
        let expected_id = inner.signer_id().clone();
        let boxed: Box<dyn EventSigner + Send + Sync> = Box::new(inner);
        let message = b"DSSEv1 4 test 5 hello";

        assert_eq!(boxed.signer_id(), &expected_id);
        let via_box = boxed.sign_event_message(message).unwrap();
        assert_eq!(
            verify_ed25519_strict(boxed.signer_id(), message, via_box.as_str()).unwrap(),
            EventVerificationStatus::Valid
        );
    }

    #[test]
    fn negative_crypto_vectors_are_invalid() {
        for case in negative_crypto_cases() {
            assert_eq!(
                case.verify().unwrap(),
                EventVerificationStatus::Invalid,
                "{} must verify as invalid",
                case.name
            );
        }
    }

    fn negative_crypto_cases() -> Vec<NegativeCryptoCase> {
        let fixture = fixture_json("negative-crypto-cases.json");
        let message = fixture_bytes("pae-v1.bytes");
        let valid_signature = fixture_json("friendly-valid-event.json")["signature"]["sig"]
            .as_str()
            .expect("friendly fixture signature")
            .to_owned();

        fixture["cases"]
            .as_array()
            .expect("negative cases are an array")
            .iter()
            .filter_map(|case| negative_crypto_case(case, &message, &valid_signature))
            .collect()
    }

    fn negative_crypto_case(
        case: &Value,
        message: &[u8],
        valid_signature: &str,
    ) -> Option<NegativeCryptoCase> {
        let name = case["name"].as_str()?.to_owned();
        match name.as_str() {
            "truncated_signature" | "over_long_signature" | "all_zero_public_key" => {
                let event = &case["event"];
                Some(NegativeCryptoCase {
                    name,
                    signer: parse_negative_signer(event["signer"].as_str()?),
                    message: message.to_vec(),
                    signature: EventSignatureBytes::parse(event["signature"]["sig"].as_str()?)
                        .unwrap(),
                })
            }
            "small_order_public_key" | "non_canonical_public_key" => Some(NegativeCryptoCase {
                name,
                signer: parse_negative_signer(case["didKey"].as_str()?),
                message: message.to_vec(),
                signature: EventSignatureBytes::parse(valid_signature).unwrap(),
            }),
            _ => None,
        }
    }

    fn parse_negative_signer(value: &str) -> NegativeSigner {
        match SignerId::parse(value) {
            Ok(signer) => NegativeSigner::Parsed(signer),
            Err(_) => NegativeSigner::Malformed,
        }
    }

    fn fixture_bytes(name: &str) -> Vec<u8> {
        let mut bytes = std::fs::read(fixture_path(name)).expect("read byte fixture");
        if bytes.last() == Some(&b'\n') {
            bytes.pop();
        }
        bytes
    }

    fn fixture_json(name: &str) -> Value {
        let bytes = std::fs::read(fixture_path(name)).expect("read json fixture");
        serde_json::from_slice(&bytes).expect("fixture is valid json")
    }

    fn fixture_path(name: &str) -> std::path::PathBuf {
        crate::test_fixtures::manifest_dir()
            .join("tests/fixtures/event_signatures")
            .join(name)
    }
    #[test]
    fn verification_status_string_form_matches_wire_form() {
        for (status, code) in [
            (EventVerificationStatus::Valid, "valid"),
            (EventVerificationStatus::Invalid, "invalid"),
            (EventVerificationStatus::UntrustedKey, "untrusted_key"),
            (EventVerificationStatus::Unsigned, "unsigned"),
        ] {
            assert_eq!(status.as_str(), code);
            assert_eq!(status.to_string(), code);
            assert_eq!(
                serde_json::to_value(status).unwrap(),
                Value::String(code.to_owned()),
                "as_str must equal the serde wire form"
            );
        }
    }
}
