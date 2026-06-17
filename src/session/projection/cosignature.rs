//! Read-only projection of an event's co-signature set.
//!
//! `cosignatures(event)` is the event's inline attestation (member #1, if present)
//! unioned with every detached `event_signature` carrier targeting it, each member
//! tagged with its reader-relative verification status. The set is a grow-only set
//! (G-Set): member identity is the full attestation triple, so the union is
//! commutative, associative, and idempotent and the result is order-independent.
//!
//! Three invariants are load-bearing and structural here, so a future reader cannot
//! reintroduce the hazards they close:
//! - The dedup key is the **full attestation triple** — carried by the detached
//!   carrier's own `eventId` — never `(target, signer)`. Two distinct signatures by
//!   one signer are two members; an identical re-submission collapses to one.
//! - Only the **inline** member may be `Invalid`. A structurally invalid detached
//!   attestation is rejected before storage, so it is never in the log to project.
//! - There is **no** separate reconciliation: every member is an ordinary event
//!   already covered by the shipped, signature-blind event-set hash. A store missing
//!   a member just yields a smaller set and backfills the event on the next sync.

use std::collections::BTreeMap;

use crate::crypto::{EventVerificationStatus, SignerId};
use crate::error::Result;
use crate::session::event::{
    EventSignatureRecordedPayload, EventType, ShoreEvent, resolve_effective_signer,
};
use crate::session::{
    CosignatureVerification, TrustSet, verify_cosignature, verify_event_signature,
};

/// Where a co-signature set member came from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CosignatureSource {
    /// Member #1: the target event's own inline signer/signature. At most one.
    Inline,
    /// A detached `event_signature` carrier targeting the event. `carrier_event_id`
    /// is the carrier's `eventId` — the full-triple identity the dedup keys on.
    Detached { carrier_event_id: String },
}

/// One member of an event's co-signature set, tagged with its reader-relative status.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CosignatureMember {
    /// The attesting signer (`did:key`).
    pub attesting_signer: SignerId,
    /// Per-member status. Detached members are only ever `Valid`/`UntrustedKey`; the
    /// inline member may also be `Invalid`/`Unsigned`.
    pub status: EventVerificationStatus,
    pub source: CosignatureSource,
}

/// The projected co-signature set for one target event. A G-Set: order-independent,
/// deduped by the full attestation triple.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CosignatureSet {
    pub target_event_id: String,
    pub members: Vec<CosignatureMember>,
}

impl CosignatureSet {
    /// The inline member's status if the target carries an inline attestation,
    /// otherwise `None`. Arm (a) of the binding predicate reads only this.
    pub(crate) fn inline_status(&self) -> Option<EventVerificationStatus> {
        self.members
            .iter()
            .find(|member| member.source == CosignatureSource::Inline)
            .map(|member| member.status)
    }

    /// True when any member verifies `Valid` (a bound co-signer for the claimed
    /// actor, since `Valid` already folds in allowed-signers authorization).
    pub(crate) fn has_valid_member(&self) -> bool {
        self.members
            .iter()
            .any(|member| member.status == EventVerificationStatus::Valid)
    }

    /// True when any member verifies cryptographically but is `UntrustedKey`.
    pub(crate) fn has_untrusted_member(&self) -> bool {
        self.members
            .iter()
            .any(|member| member.status == EventVerificationStatus::UntrustedKey)
    }
}

/// Compute `cosignatures(event)` for the target with `target_event_id`, over the
/// supplied event log and trust set. The result is independent of the order
/// `events` is presented in, and a duplicate attestation never double-counts.
pub(crate) fn cosignatures_for_event(
    events: &[ShoreEvent],
    target_event_id: &str,
    trust: &TrustSet,
) -> Result<CosignatureSet> {
    let Some(target) = events
        .iter()
        .find(|event| event.event_id.as_str() == target_event_id)
    else {
        // A read-only projection: an absent target has no inline member and its
        // detached members cannot be status-classified. The binding caller treats
        // absence as "no attempt / no fact".
        return Ok(CosignatureSet {
            target_event_id: target_event_id.to_owned(),
            members: Vec::new(),
        });
    };

    let target_record_hash = target.event_record_hash()?;

    // Member #1: the inline attestation, kept at whatever status (it is the only
    // member that may be `Invalid`). Its dedup key is the full attestation triple,
    // so a detached carrier transcribing the same inline signature is the SAME
    // member, not a second one (the inline signer/signature IS co-signature #1).
    let mut inline_member: Option<(String, CosignatureMember)> = None;
    if let Some(signature) = &target.signature {
        let status = verify_event_signature(target, trust)?;
        if let Some(attesting_signer) = resolve_effective_signer(target)
            .ok()
            .or_else(|| target.signer.clone())
        {
            let key = EventSignatureRecordedPayload::idempotency_key(
                &target_record_hash,
                &attesting_signer,
                signature.sig.as_str(),
            );
            inline_member = Some((
                key,
                CosignatureMember {
                    attesting_signer,
                    status,
                    source: CosignatureSource::Inline,
                },
            ));
        }
    }
    let inline_key = inline_member.as_ref().map(|(key, _)| key.clone());

    // Detached members, deduped by the full triple (the carrier's own
    // idempotencyKey, which derives from the triple). Keying on a `BTreeMap` makes
    // the union structurally commutative/associative/idempotent and the output
    // order-independent.
    let mut detached: BTreeMap<String, CosignatureMember> = BTreeMap::new();
    for event in events
        .iter()
        .filter(|event| event.event_type == EventType::EventSignatureRecorded)
    {
        let payload: EventSignatureRecordedPayload = serde_json::from_value(event.payload.clone())?;
        if payload.target_event_id.as_str() != target_event_id {
            continue;
        }
        // An `invalid` detached attestation is reader-independent noise and is never
        // a stored member (defense-in-depth on a log that bypassed the gate); a
        // `BindingMismatch` names a different record. Keep only `Valid`/`UntrustedKey`.
        let status = match verify_cosignature(&payload, target, trust)? {
            CosignatureVerification::Attested(status @ EventVerificationStatus::Valid)
            | CosignatureVerification::Attested(status @ EventVerificationStatus::UntrustedKey) => {
                status
            }
            CosignatureVerification::Attested(_) | CosignatureVerification::BindingMismatch => {
                continue;
            }
        };
        // The carrier's idempotencyKey is the full-triple key. If it equals the
        // inline member's triple, it is the same attestation — already member #1.
        if inline_key.as_deref() == Some(event.idempotency_key.as_str()) {
            continue;
        }
        detached
            .entry(event.idempotency_key.clone())
            .or_insert_with(|| CosignatureMember {
                attesting_signer: payload.attesting_signer,
                status,
                source: CosignatureSource::Detached {
                    carrier_event_id: event.event_id.as_str().to_owned(),
                },
            });
    }

    let mut members = Vec::with_capacity(inline_member.is_some() as usize + detached.len());
    if let Some((_, member)) = inline_member {
        members.push(member);
    }
    members.extend(detached.into_values());

    Ok(CosignatureSet {
        target_event_id: target_event_id.to_owned(),
        members,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{EventSignatureBytes, EventSigner};
    use crate::session::event::{
        EventSignature, EventToBeSigned, event_signature_pre_authentication_encoding,
    };
    use crate::session::projection::freshness::event_set_hash_for_events;
    use crate::session::signing::test_support::{DeterministicSigner, trust_for_actor};

    const SIGNER_A_SEED: [u8; 32] = [61u8; 32];
    const SIGNER_B_SEED: [u8; 32] = [62u8; 32];

    fn fixture_target() -> ShoreEvent {
        serde_json::from_str(include_str!(
            "../../../tests/fixtures/event_signatures/friendly-valid-event.json"
        ))
        .expect("fixture event decodes")
    }

    fn inline_signed(signer: &DeterministicSigner) -> ShoreEvent {
        let mut event = fixture_target();
        event.signer = None;
        event.signature = None;
        let tbs = EventToBeSigned::from_event(&event, signer.signer_id()).unwrap();
        let pae = event_signature_pre_authentication_encoding(&tbs).unwrap();
        let sig = signer.sign_event_message(&pae).unwrap();
        event.signer = Some(signer.signer_id().clone());
        event.signature = Some(EventSignature::ed25519_v1(sig));
        event
    }

    fn detached_carrier(target: &ShoreEvent, signer: &DeterministicSigner) -> ShoreEvent {
        let attesting_signer = signer.signer_id().clone();
        let tbs = EventToBeSigned::from_event(target, &attesting_signer).unwrap();
        let pae = event_signature_pre_authentication_encoding(&tbs).unwrap();
        let sig = signer.sign_event_message(&pae).unwrap();
        let payload = EventSignatureRecordedPayload {
            target_event_id: target.event_id.clone(),
            target_event_record_hash: target.event_record_hash().unwrap(),
            attesting_signer,
            attestation: EventSignature::ed25519_v1(sig),
            inclusion_proof: None,
        };
        let key = EventSignatureRecordedPayload::idempotency_key(
            &target.event_record_hash().unwrap(),
            signer.signer_id(),
            payload.attestation.sig.as_str(),
        );
        crate::session::event::ShoreEvent::new(
            EventType::EventSignatureRecorded,
            key,
            crate::session::event::EventTarget::for_event_signature(
                target.target.session_id.clone(),
                target.event_id.clone(),
            ),
            crate::session::event::Writer::shore_local("test"),
            payload,
            "2026-06-04T00:00:00Z",
        )
        .unwrap()
    }

    fn two_signer_trust(
        actor: &crate::model::ActorId,
        a: &DeterministicSigner,
        b: &DeterministicSigner,
    ) -> TrustSet {
        crate::session::event_signature_trust_set(serde_json::json!({
            "allowedSigners": {
                actor.as_str(): [a.signer_id().as_str(), b.signer_id().as_str()],
            }
        }))
        .unwrap()
    }

    #[test]
    fn two_signer_fact_projects_a_two_member_set() {
        let signer_a = DeterministicSigner::from_seed(SIGNER_A_SEED);
        let signer_b = DeterministicSigner::from_seed(SIGNER_B_SEED);
        let target = inline_signed(&signer_a);
        let carrier = detached_carrier(&target, &signer_b);
        let trust = two_signer_trust(&target.writer.actor_id.clone(), &signer_a, &signer_b);

        let set =
            cosignatures_for_event(&[target.clone(), carrier], target.event_id.as_str(), &trust)
                .unwrap();

        assert_eq!(set.members.len(), 2);
        let inline = &set.members[0];
        assert_eq!(inline.source, CosignatureSource::Inline);
        assert_eq!(inline.attesting_signer, *signer_a.signer_id());
        assert_eq!(inline.status, EventVerificationStatus::Valid);
        let detached = &set.members[1];
        assert!(matches!(
            detached.source,
            CosignatureSource::Detached { .. }
        ));
        assert_eq!(detached.attesting_signer, *signer_b.signer_id());
        assert_eq!(detached.status, EventVerificationStatus::Valid);
    }

    #[test]
    fn identical_resubmitted_attestation_does_not_double_count() {
        let signer_a = DeterministicSigner::from_seed(SIGNER_A_SEED);
        let signer_b = DeterministicSigner::from_seed(SIGNER_B_SEED);
        let target = inline_signed(&signer_a);
        let carrier = detached_carrier(&target, &signer_b);
        let trust = two_signer_trust(&target.writer.actor_id.clone(), &signer_a, &signer_b);

        let set = cosignatures_for_event(
            &[target.clone(), carrier.clone(), carrier],
            target.event_id.as_str(),
            &trust,
        )
        .unwrap();

        assert_eq!(
            set.members.len(),
            2,
            "the duplicate carrier collapses to one member"
        );
    }

    #[test]
    fn inline_and_detached_of_the_same_attestation_dedup_to_one_member() {
        // The dedup key is the full triple, not (target, signer): a detached carrier
        // transcribing the target's own inline signature is the SAME attestation
        // (co-signature #1), so it does not double-count.
        let signer_a = DeterministicSigner::from_seed(SIGNER_A_SEED);
        let target = inline_signed(&signer_a);
        let carrier = detached_carrier(&target, &signer_a);
        let trust = trust_for_actor(&target.writer.actor_id.clone(), &signer_a);

        let set =
            cosignatures_for_event(&[target.clone(), carrier], target.event_id.as_str(), &trust)
                .unwrap();

        assert_eq!(set.members.len(), 1);
        assert_eq!(set.members[0].source, CosignatureSource::Inline);
    }

    #[test]
    fn projection_is_order_independent() {
        let signer_a = DeterministicSigner::from_seed(SIGNER_A_SEED);
        let signer_b = DeterministicSigner::from_seed(SIGNER_B_SEED);
        let target = inline_signed(&signer_a);
        let carrier = detached_carrier(&target, &signer_b);
        let trust = two_signer_trust(&target.writer.actor_id.clone(), &signer_a, &signer_b);

        let forward = cosignatures_for_event(
            &[target.clone(), carrier.clone()],
            target.event_id.as_str(),
            &trust,
        )
        .unwrap();
        let reversed =
            cosignatures_for_event(&[carrier, target.clone()], target.event_id.as_str(), &trust)
                .unwrap();

        assert_eq!(forward, reversed);
    }

    #[test]
    fn unsigned_target_has_empty_inline_slot() {
        let signer_b = DeterministicSigner::from_seed(SIGNER_B_SEED);
        let mut target = fixture_target();
        target.signer = None;
        target.signature = None;
        let trust = trust_for_actor(&target.writer.actor_id.clone(), &signer_b);

        let empty = cosignatures_for_event(
            std::slice::from_ref(&target),
            target.event_id.as_str(),
            &trust,
        )
        .unwrap();
        assert!(empty.members.is_empty());

        let carrier = detached_carrier(&target, &signer_b);
        let with_detached =
            cosignatures_for_event(&[target.clone(), carrier], target.event_id.as_str(), &trust)
                .unwrap();
        assert_eq!(with_detached.members.len(), 1);
        assert!(matches!(
            with_detached.members[0].source,
            CosignatureSource::Detached { .. }
        ));
    }

    #[test]
    fn inline_member_may_be_invalid_detached_members_are_not() {
        let signer_a = DeterministicSigner::from_seed(SIGNER_A_SEED);
        let signer_b = DeterministicSigner::from_seed(SIGNER_B_SEED);
        let mut target = inline_signed(&signer_a);
        // Tamper the inline signature → Invalid.
        target.signature = Some(EventSignature::ed25519_v1(EventSignatureBytes::from_bytes(
            &[0u8; 64],
        )));
        let carrier = detached_carrier(&target, &signer_b);
        let trust = two_signer_trust(&target.writer.actor_id.clone(), &signer_a, &signer_b);

        let set =
            cosignatures_for_event(&[target.clone(), carrier], target.event_id.as_str(), &trust)
                .unwrap();

        let inline = set
            .members
            .iter()
            .find(|member| member.source == CosignatureSource::Inline)
            .unwrap();
        assert_eq!(inline.status, EventVerificationStatus::Invalid);
        let detached = set
            .members
            .iter()
            .find(|member| matches!(member.source, CosignatureSource::Detached { .. }))
            .unwrap();
        assert_eq!(detached.status, EventVerificationStatus::Valid);
    }

    #[test]
    fn untrusted_detached_member_is_kept() {
        let signer_a = DeterministicSigner::from_seed(SIGNER_A_SEED);
        let signer_b = DeterministicSigner::from_seed(SIGNER_B_SEED);
        let target = inline_signed(&signer_a);
        let carrier = detached_carrier(&target, &signer_b);
        // Only A trusted; B's detached member is untrusted but kept.
        let trust = trust_for_actor(&target.writer.actor_id.clone(), &signer_a);

        let set =
            cosignatures_for_event(&[target.clone(), carrier], target.event_id.as_str(), &trust)
                .unwrap();

        let detached = set
            .members
            .iter()
            .find(|member| matches!(member.source, CosignatureSource::Detached { .. }))
            .unwrap();
        assert_eq!(detached.status, EventVerificationStatus::UntrustedKey);
    }

    #[test]
    fn cosignature_events_are_in_event_set_hash() {
        let signer_a = DeterministicSigner::from_seed(SIGNER_A_SEED);
        let signer_b = DeterministicSigner::from_seed(SIGNER_B_SEED);
        let target = inline_signed(&signer_a);
        let carrier = detached_carrier(&target, &signer_b);

        let target_only = event_set_hash_for_events([&target]).unwrap();
        let with_carrier = event_set_hash_for_events([&target, &carrier]).unwrap();
        let reversed = event_set_hash_for_events([&carrier, &target]).unwrap();

        assert_ne!(
            target_only, with_carrier,
            "the carrier rides the shipped set hash"
        );
        assert_eq!(with_carrier, reversed, "the set hash is order-independent");
    }
}
