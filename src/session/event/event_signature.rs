use serde::{Deserialize, Serialize};

use super::kind::EventType;
use super::payload::EventPayload;
use super::signature::EventSignature;
use crate::crypto::SignerId;
use crate::model::EventId;

/// Reserved, non-identity inclusion-proof slot. v1 produces and consumes
/// nothing here; it exists so a relay witness-log proof (ADR-0011 / relay
/// ADR-0004) can be populated later WITHOUT changing any `eventId`. It is
/// kept OUT of the idempotency key, the member-identity triple, and the
/// `eventRecordHash` material, so populating it is strictly additive.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InclusionProof {
    /// Opaque scheme tag (e.g. a future "rfc6962-merkle"); v1 places no
    /// constraint on its value and nothing branches on it.
    pub scheme: String,
    /// Opaque proof material, scheme-defined. Reserved; unused in v1.
    pub proof: String,
}

/// Detached co-signature carrier payload (ADR-0004 amendment D2/D3/D4).
///
/// A co-signature is itself an event. This payload references its target by
/// content identity — `target_event_id` plus the signature-EXCLUSIVE
/// `target_event_record_hash` (the convergent binding key) — and embeds an
/// `attestation` over the target's signer-INCLUSIVE `event-tbs.v1` view (built
/// in `record_event_signature`). Two digests are in play and must never be
/// confused: the attestation signs the signer-inclusive TBS, while the carrier
/// binds the signer-exclusive `targetEventRecordHash`.
///
/// The attestation reuses the existing `EventSignature` wire type — no new
/// `sigVersion`, no new payload type for the signature itself (D4).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventSignatureRecordedPayload {
    /// The target event's `eventId` (a convenience pointer; the binding key is
    /// `target_event_record_hash`).
    pub target_event_id: EventId,
    /// The target's signature-EXCLUSIVE `eventRecordHash`
    /// (`ShoreEvent::event_record_hash`): the convergent content-identity the
    /// carrier binds.
    pub target_event_record_hash: String,
    /// The signer whose attestation this carrier records.
    pub attesting_signer: SignerId,
    /// The attestation: an Ed25519 signature (the existing `EventSignature` wire
    /// type — `{alg, sigVersion, sig}`) over the target's `event-tbs.v1` view
    /// with `signer = attesting_signer`.
    pub attestation: EventSignature,
    /// Reserved non-identity inclusion proof. Absent in v1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inclusion_proof: Option<InclusionProof>,
}

impl EventSignatureRecordedPayload {
    /// Idempotency key = the FULL attestation triple (ADR-0004 amendment D3):
    /// `event_signature_recorded:<targetEventRecordHash>:<attestingSigner>:<sig-encoding>`.
    ///
    /// Follows the established `<event_kind>:<work-object-identity>:<source_key>`
    /// shape (cf. `ReviewAssessmentRecordedPayload::idempotency_key`,
    /// `ReviewUnitLineageRoundRecordedPayload::idempotency_key`). The signature
    /// component carries the full `sig` bytes (`attestation.sig.as_str()`), so
    /// two DISTINCT signatures by one signer key to DISTINCT members — closing
    /// signer-slot poisoning. The `inclusion_proof` is NOT part of this key.
    pub fn idempotency_key(
        target_event_record_hash: &str,
        attesting_signer: &SignerId,
        signature_encoding: &str,
    ) -> String {
        format!(
            "event_signature_recorded:{}:{}:{}",
            target_event_record_hash,
            attesting_signer.as_str(),
            signature_encoding,
        )
    }
}

impl EventPayload for EventSignatureRecordedPayload {
    fn event_type(&self) -> EventType {
        EventType::EventSignatureRecorded
    }
}

#[cfg(test)]
mod tests {
    use crate::crypto::SignerId;
    use crate::model::{EventId, LedgerId};
    use crate::session::event::{
        EventPayload, EventSignature, EventSignatureRecordedPayload, EventTarget, EventType,
        InclusionProof, ShoreEvent, Writer,
    };

    const FRIENDLY_SIGNER: &str = "did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd";

    fn carrier_event(idempotency_key: &str, payload: EventSignatureRecordedPayload) -> ShoreEvent {
        ShoreEvent::new(
            EventType::EventSignatureRecorded,
            idempotency_key,
            EventTarget::for_ledger(LedgerId::new("session:fixture")),
            Writer::shore_local("test"),
            payload,
            "2026-06-04T00:00:00Z",
        )
        .expect("carrier event builds")
    }

    fn payload_for(
        record_hash: &str,
        signer: &SignerId,
        sig: &str,
    ) -> EventSignatureRecordedPayload {
        EventSignatureRecordedPayload {
            target_event_id: EventId::new("evt:sha256:target"),
            target_event_record_hash: record_hash.to_owned(),
            attesting_signer: signer.clone(),
            attestation: EventSignature::new_ed25519_v1(sig).unwrap(),
            inclusion_proof: None,
        }
    }

    #[test]
    fn event_signature_idempotency_key_is_full_attestation_triple() {
        let signer = SignerId::parse(FRIENDLY_SIGNER).unwrap();

        let key =
            EventSignatureRecordedPayload::idempotency_key("sha256:rec", &signer, "SIG_BASE64");

        assert_eq!(
            key,
            "event_signature_recorded:sha256:rec:did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd:SIG_BASE64"
        );
    }

    #[test]
    fn two_distinct_signatures_by_one_signer_yield_distinct_event_ids() {
        let signer = SignerId::parse(FRIENDLY_SIGNER).unwrap();

        let key_a = EventSignatureRecordedPayload::idempotency_key("sha256:rec", &signer, "SIGAAA");
        let key_b = EventSignatureRecordedPayload::idempotency_key("sha256:rec", &signer, "SIGBBB");
        let event_a = carrier_event(&key_a, payload_for("sha256:rec", &signer, "SIGAAA"));
        let event_b = carrier_event(&key_b, payload_for("sha256:rec", &signer, "SIGBBB"));

        assert_ne!(
            event_a.event_id, event_b.event_id,
            "two distinct signatures by one signer are two distinct members, never two slot claimants"
        );
    }

    #[test]
    fn identical_attestation_triple_is_idempotent() {
        let signer = SignerId::parse(FRIENDLY_SIGNER).unwrap();

        let key_a = EventSignatureRecordedPayload::idempotency_key("sha256:rec", &signer, "SIGAAA");
        let key_b = EventSignatureRecordedPayload::idempotency_key("sha256:rec", &signer, "SIGAAA");
        assert_eq!(key_a, key_b);

        let event_a = carrier_event(&key_a, payload_for("sha256:rec", &signer, "SIGAAA"));
        let event_b = carrier_event(&key_b, payload_for("sha256:rec", &signer, "SIGAAA"));
        assert_eq!(event_a.event_id, event_b.event_id);
    }

    #[test]
    fn populating_inclusion_proof_does_not_change_idempotency_key_or_event_id() {
        let signer = SignerId::parse(FRIENDLY_SIGNER).unwrap();
        let key = EventSignatureRecordedPayload::idempotency_key("sha256:rec", &signer, "SIGAAA");

        let without = carrier_event(&key, payload_for("sha256:rec", &signer, "SIGAAA"));

        let mut proof_payload = payload_for("sha256:rec", &signer, "SIGAAA");
        proof_payload.inclusion_proof = Some(InclusionProof {
            scheme: "reserved".to_owned(),
            proof: "opaque".to_owned(),
        });
        let with = carrier_event(&key, proof_payload);

        assert_eq!(without.idempotency_key, with.idempotency_key);
        assert_eq!(without.event_id, with.event_id);
        // The proof is part of the payload, so it changes payloadHash — but never identity.
        assert_ne!(without.payload_hash, with.payload_hash);
    }

    #[test]
    fn event_signature_payload_round_trips_and_skips_absent_inclusion_proof() {
        let signer = SignerId::parse(FRIENDLY_SIGNER).unwrap();
        let payload = payload_for("sha256:rec", &signer, "SIGAAA");

        let json = serde_json::to_value(&payload).unwrap();
        assert!(json.get("inclusionProof").is_none());
        assert_eq!(json["targetEventRecordHash"], "sha256:rec");
        assert_eq!(json["attestingSigner"], FRIENDLY_SIGNER);
        let round: EventSignatureRecordedPayload = serde_json::from_value(json).unwrap();
        assert_eq!(round, payload);

        let mut with_proof = payload;
        with_proof.inclusion_proof = Some(InclusionProof {
            scheme: "reserved".to_owned(),
            proof: "opaque".to_owned(),
        });
        let json = serde_json::to_value(&with_proof).unwrap();
        assert_eq!(json["inclusionProof"]["scheme"], "reserved");
        let round: EventSignatureRecordedPayload = serde_json::from_value(json).unwrap();
        assert_eq!(round, with_proof);
    }

    #[test]
    fn payload_event_type_matches_discriminant() {
        let signer = SignerId::parse(FRIENDLY_SIGNER).unwrap();
        let payload = payload_for("sha256:rec", &signer, "SIGAAA");

        assert_eq!(payload.event_type(), EventType::EventSignatureRecorded);
    }
}
