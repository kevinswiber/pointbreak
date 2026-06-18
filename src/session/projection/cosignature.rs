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

use serde::Serialize;

use crate::crypto::{EventVerificationStatus, SignerId};
use crate::error::Result;
use crate::model::ActorId;
use crate::session::event::{
    EventSignatureRecordedPayload, EventType, ShoreEvent, resolve_effective_signer,
};
use crate::session::{
    ActorAttributesMap, CosignatureVerification, TrustSet, verify_cosignature,
    verify_event_signature,
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

/// ADR-0013 read-side classification of a co-signature member. Derived at projection
/// from stored bytes + the reader's committed trust set; never stored, never binding.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(dead_code)] // payload (endorser/reason) is read by the deferred policy consumer.
pub(crate) enum CosignatureClassification {
    /// Inline member (#1, any status), or a detached member whose signer is authorized
    /// for the event's own actor. `reason` is set only for the laundering-guard overlap.
    Authoring { reason: Option<AuthoringReason> },
    /// A detached member that verifies and reverse-resolves to exactly one known actor
    /// distinct from the event's actor. Carries the resolved endorser for downstream reads.
    EndorsementTrusted { endorser: ActorId },
    /// A detached member that verifies but cannot be placed as a single distinct known actor.
    EndorsementUntrusted { reason: EndorsementUntrustedReason },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AuthoringReason {
    /// A detached signer authorized for the event's actor that ALSO maps to a distinct
    /// actor — authoring has precedence; deliberately not an endorsement. (authoring_not_endorsement)
    AuthoringNotEndorsement,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EndorsementUntrustedReason {
    /// Resolves to no known actor (not enrolled, not the resolved principal) —
    /// including a bare, unenrolled did:key. (unknown_endorser)
    UnknownEndorser,
    /// Resolves to more than one explicitly enrolled actor. (ambiguous_endorser)
    AmbiguousEndorser,
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
    /// ADR-0013 read-side classification (derived; never stored). Read by
    /// `has_trusted_endorsement` and the deferred policy consumer.
    #[allow(dead_code)]
    pub classification: CosignatureClassification,
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

    /// True when any member classifies `endorsement-trusted` (ADR-0013): an actor vouched
    /// for this change in its own identity. This is the stewardship/policy plane's reader —
    /// it is **non-binding** and feeds NO binding decision (binding reads `has_valid_member`
    /// only). Optionally narrowed by relationship/attributes downstream.
    #[allow(dead_code)]
    pub(crate) fn has_trusted_endorsement(&self) -> bool {
        self.members.iter().any(|member| {
            matches!(
                member.classification,
                CosignatureClassification::EndorsementTrusted { .. }
            )
        })
    }

    /// True when any member verifies cryptographically but is `UntrustedKey`.
    pub(crate) fn has_untrusted_member(&self) -> bool {
        self.members
            .iter()
            .any(|member| member.status == EventVerificationStatus::UntrustedKey)
    }
}

/// Detached `event_signature` carriers grouped by their `target_event_id`, each
/// payload parsed once at build. Build ONCE per multi-target document (INV-5); look
/// up the bucket for a target in O(1). The classifier reads only `&TrustSet` (INV-2).
pub(crate) struct CosignatureIndex<'a> {
    carriers_by_target: BTreeMap<String, Vec<DetachedCarrier<'a>>>,
}

/// One detached `event_signature` carrier with its payload parsed once at build.
struct DetachedCarrier<'a> {
    event: &'a ShoreEvent,
    payload: EventSignatureRecordedPayload,
}

impl<'a> CosignatureIndex<'a> {
    /// Index every detached `event_signature` carrier by its `target_event_id` in a
    /// single pass over the log, parsing each carrier payload exactly once.
    pub(crate) fn build(events: &'a [ShoreEvent]) -> Result<Self> {
        let mut carriers_by_target: BTreeMap<String, Vec<DetachedCarrier<'a>>> = BTreeMap::new();
        for event in events
            .iter()
            .filter(|event| event.event_type == EventType::EventSignatureRecorded)
        {
            let payload: EventSignatureRecordedPayload =
                serde_json::from_value(event.payload.clone())?;
            carriers_by_target
                .entry(payload.target_event_id.as_str().to_owned())
                .or_default()
                .push(DetachedCarrier { event, payload });
        }
        Ok(Self { carriers_by_target })
    }

    /// The co-signature set for `target`: its inline member (#1, if any) unioned with
    /// only the carriers bucketed under its event id. Equivalent to
    /// `cosignatures_for_event` for the same target.
    pub(crate) fn cosignatures_for_target(
        &self,
        target: &ShoreEvent,
        trust: &TrustSet,
    ) -> Result<CosignatureSet> {
        let target_record_hash = target.event_record_hash()?;
        let (inline_key, inline) = inline_member(target, &target_record_hash, trust)?;
        let empty = Vec::new();
        let carriers = self
            .carriers_by_target
            .get(target.event_id.as_str())
            .unwrap_or(&empty);
        let detached = detached_members(carriers, target, inline_key.as_deref(), trust)?;

        let mut members = Vec::with_capacity(inline.is_some() as usize + detached.len());
        if let Some(member) = inline {
            members.push(member);
        }
        members.extend(detached.into_values());
        Ok(CosignatureSet {
            target_event_id: target.event_id.as_str().to_owned(),
            members,
        })
    }
}

/// Build the inline member (#1) for `target`, returning its full-triple dedup key
/// alongside it. `None` when the target carries no inline signature or its signer
/// cannot be resolved. The inline member is the only member that may be `Invalid`.
fn inline_member(
    target: &ShoreEvent,
    target_record_hash: &str,
    trust: &TrustSet,
) -> Result<(Option<String>, Option<CosignatureMember>)> {
    let Some(signature) = &target.signature else {
        return Ok((None, None));
    };
    let status = verify_event_signature(target, trust)?;
    let Some(attesting_signer) = resolve_effective_signer(target)
        .ok()
        .or_else(|| target.signer.clone())
    else {
        return Ok((None, None));
    };
    // The dedup key is the full attestation triple, so a detached carrier
    // transcribing the same inline signature is the SAME member, not a second one
    // (the inline signer/signature IS co-signature #1).
    let key = EventSignatureRecordedPayload::idempotency_key(
        target_record_hash,
        &attesting_signer,
        signature.sig.as_str(),
    );
    let classification = classify_cosignature_member(
        &CosignatureSource::Inline,
        status,
        &attesting_signer,
        &target.writer.actor_id,
        trust,
    );
    Ok((
        Some(key),
        Some(CosignatureMember {
            attesting_signer,
            status,
            source: CosignatureSource::Inline,
            classification,
        }),
    ))
}

/// Build the detached members for `target` from its bucketed carriers, deduped by
/// the full triple (the carrier's own `idempotencyKey`). Keying on a `BTreeMap`
/// makes the union commutative/associative/idempotent and the output
/// order-independent. A carrier equal to the inline triple is already member #1.
fn detached_members(
    carriers: &[DetachedCarrier<'_>],
    target: &ShoreEvent,
    inline_key: Option<&str>,
    trust: &TrustSet,
) -> Result<BTreeMap<String, CosignatureMember>> {
    let mut detached: BTreeMap<String, CosignatureMember> = BTreeMap::new();
    for carrier in carriers {
        let event = carrier.event;
        let payload = &carrier.payload;
        // An `invalid` detached attestation is reader-independent noise and is never
        // a stored member (defense-in-depth on a log that bypassed the gate); a
        // `BindingMismatch` names a different record. Keep only `Valid`/`UntrustedKey`.
        let status = match verify_cosignature(payload, target, trust)? {
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
        if inline_key == Some(event.idempotency_key.as_str()) {
            continue;
        }
        let source = CosignatureSource::Detached {
            carrier_event_id: event.event_id.as_str().to_owned(),
        };
        let classification = classify_cosignature_member(
            &source,
            status,
            &payload.attesting_signer,
            &target.writer.actor_id,
            trust,
        );
        detached
            .entry(event.idempotency_key.clone())
            .or_insert_with(|| CosignatureMember {
                attesting_signer: payload.attesting_signer.clone(),
                status,
                source,
                classification,
            });
    }
    Ok(detached)
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
    CosignatureIndex::build(events)?.cosignatures_for_target(target, trust)
}

/// ADR-0013 classifier (read-side; derived). Reads `status` as the already-computed
/// scope-#1 result (`Valid` ⟺ signer authorized for `target_actor`). Reverse resolution
/// uses EXPLICIT allowed-signers only (INV-3). Pure: no I/O, no `occurredAt`.
pub(crate) fn classify_cosignature_member(
    source: &CosignatureSource,
    status: EventVerificationStatus,
    attesting_signer: &SignerId,
    target_actor: &ActorId,
    trust: &TrustSet,
) -> CosignatureClassification {
    // (1) Inline #1 is the event's own author attestation — authoring at any status.
    if matches!(source, CosignatureSource::Inline) {
        return CosignatureClassification::Authoring { reason: None };
    }
    match status {
        // (2) Detached + Valid: authoring authority for the target's actor.
        EventVerificationStatus::Valid => {
            let launders = trust
                .reverse_resolve(attesting_signer)
                .into_iter()
                .any(|actor| actor != *target_actor);
            CosignatureClassification::Authoring {
                reason: launders.then_some(AuthoringReason::AuthoringNotEndorsement),
            }
        }
        // (3) Detached + UntrustedKey: endorsement candidate.
        EventVerificationStatus::UntrustedKey => {
            let actors = trust.reverse_resolve(attesting_signer);
            match actors.len() {
                1 => CosignatureClassification::EndorsementTrusted {
                    endorser: actors.into_iter().next().expect("len checked"),
                },
                0 => CosignatureClassification::EndorsementUntrusted {
                    reason: EndorsementUntrustedReason::UnknownEndorser,
                },
                _ => CosignatureClassification::EndorsementUntrusted {
                    reason: EndorsementUntrustedReason::AmbiguousEndorser,
                },
            }
        }
        // (4) A detached member is only ever Valid/UntrustedKey (the verify-before-store
        // gate drops Invalid; Unsigned cannot be a detached carrier). Defensive only.
        EventVerificationStatus::Invalid | EventVerificationStatus::Unsigned => {
            CosignatureClassification::Authoring { reason: None }
        }
    }
}

/// Reader-relative endorsement classification, projected from a co-signature member
/// for rendering. The `pub(crate)` `CosignatureClassification` is never exposed (INV-1).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EndorsementClassification {
    #[serde(rename = "endorsement-trusted")]
    EndorsementTrusted,
    UnknownEndorser,
    AmbiguousEndorser,
}

/// One endorsement of a target event, as one reader sees it (ADR-0013, advisory).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndorsementReadback {
    pub classification: EndorsementClassification,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endorser: Option<ActorId>,
    /// Reserved for a future explanatory string; always `None` in this surface.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Sibling enrichment (kind/roles); filled by `enrich_endorser_attributes`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endorser_attributes: Option<EndorserAttributesView>,
}

/// The endorser's attested kind/roles, surfaced beside the classification. Sibling
/// enrichment — never a classifier input (INV-2).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndorserAttributesView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
}

/// Lower a co-signature set's endorsement members to readbacks (trust-only — INV-2).
/// Authoring members are not endorsements and are not surfaced. Enrichment is applied
/// separately by `enrich_endorser_attributes`.
pub(crate) fn endorsement_readbacks(set: &CosignatureSet) -> Vec<EndorsementReadback> {
    set.members
        .iter()
        .filter_map(|member| match &member.classification {
            CosignatureClassification::EndorsementTrusted { endorser } => {
                Some(EndorsementReadback {
                    classification: EndorsementClassification::EndorsementTrusted,
                    endorser: Some(endorser.clone()),
                    reason: None,
                    endorser_attributes: None,
                })
            }
            CosignatureClassification::EndorsementUntrusted { reason } => {
                Some(EndorsementReadback {
                    classification: match reason {
                        EndorsementUntrustedReason::UnknownEndorser => {
                            EndorsementClassification::UnknownEndorser
                        }
                        EndorsementUntrustedReason::AmbiguousEndorser => {
                            EndorsementClassification::AmbiguousEndorser
                        }
                    },
                    endorser: None,
                    reason: None,
                    endorser_attributes: None,
                })
            }
            CosignatureClassification::Authoring { .. } => None,
        })
        .collect()
}

/// Decorate each readback that has a resolved endorser with that endorser's attested
/// kind/roles. Sibling enrichment, applied AFTER classification — never a classifier
/// input (INV-2). A `None` map, a readback without a resolved endorser, or an endorser
/// with no attested attributes is a no-op (no field rendered).
pub(crate) fn enrich_endorser_attributes(
    readbacks: &mut [EndorsementReadback],
    attributes: Option<&ActorAttributesMap>,
) {
    let Some(attributes) = attributes else {
        return;
    };
    for readback in readbacks.iter_mut() {
        let Some(endorser) = readback.endorser.as_ref() else {
            continue;
        };
        let resolved = attributes.resolve(endorser);
        if resolved.kind().is_none() && resolved.roles().is_empty() {
            continue;
        }
        readback.endorser_attributes = Some(EndorserAttributesView {
            kind: resolved.kind().map(str::to_owned),
            roles: resolved.roles().iter().cloned().collect(),
        });
    }
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
    fn index_matches_single_shot_for_every_target() {
        // A log with a co-signed target (inline authoring + a detached endorsement),
        // plus the carrier event (which resolves to an empty set as a target). The
        // grouped index must agree with the per-call single shot for every target.
        let signer_a = DeterministicSigner::from_seed(SIGNER_A_SEED);
        let signer_b = DeterministicSigner::from_seed(SIGNER_B_SEED);
        let target = inline_signed(&signer_a);
        let carrier = detached_carrier(&target, &signer_b);
        let endorser = crate::model::ActorId::new("actor:git-email:kevin@swiber.dev");
        let trust = crate::session::event_signature_trust_set(serde_json::json!({
            "allowedSigners": {
                target.writer.actor_id.as_str(): [signer_a.signer_id().as_str()],
                endorser.as_str(): [signer_b.signer_id().as_str()]
            }
        }))
        .unwrap();
        let events = vec![target, carrier];

        let index = CosignatureIndex::build(&events).unwrap();
        for target in &events {
            let via_index = index.cosignatures_for_target(target, &trust).unwrap();
            let via_single =
                cosignatures_for_event(&events, target.event_id.as_str(), &trust).unwrap();
            assert_eq!(
                via_index,
                via_single,
                "index path must equal single-shot for {}",
                target.event_id.as_str()
            );
        }
    }

    #[test]
    fn endorsement_readbacks_lowers_trusted_and_untrusted_members() {
        // A set with an inline (Authoring) member, a detached EndorsementTrusted member
        // (signer_b enrolled under a distinct actor), and a detached EndorsementUntrusted
        // {UnknownEndorser} member (signer_c enrolled under nobody).
        let signer_a = DeterministicSigner::from_seed(SIGNER_A_SEED);
        let signer_b = DeterministicSigner::from_seed(SIGNER_B_SEED);
        let signer_c = DeterministicSigner::from_seed([63u8; 32]);
        let target = inline_signed(&signer_a);
        let carrier_trusted = detached_carrier(&target, &signer_b);
        let carrier_unknown = detached_carrier(&target, &signer_c);
        let endorser = crate::model::ActorId::new("actor:git-email:a@example.com");
        let trust = crate::session::event_signature_trust_set(serde_json::json!({
            "allowedSigners": {
                target.writer.actor_id.as_str(): [signer_a.signer_id().as_str()],
                endorser.as_str(): [signer_b.signer_id().as_str()]
            }
        }))
        .unwrap();
        let events = vec![target.clone(), carrier_trusted, carrier_unknown];
        let set = cosignatures_for_event(&events, target.event_id.as_str(), &trust).unwrap();

        let readbacks = endorsement_readbacks(&set);
        // Authoring (the inline member) is NOT surfaced; the two endorsements are.
        assert_eq!(readbacks.len(), 2);
        let trusted = readbacks
            .iter()
            .find(|r| r.classification == EndorsementClassification::EndorsementTrusted)
            .expect("a trusted endorsement");
        assert_eq!(
            trusted.endorser.as_ref().map(|a| a.as_str()),
            Some("actor:git-email:a@example.com")
        );
        let unknown = readbacks
            .iter()
            .find(|r| r.classification == EndorsementClassification::UnknownEndorser)
            .expect("an unknown endorsement");
        assert!(unknown.endorser.is_none());
        // No enrichment in this task.
        assert!(readbacks.iter().all(|r| r.endorser_attributes.is_none()));
    }

    #[test]
    fn endorsement_classification_serializes_kebab_and_snake() {
        assert_eq!(
            serde_json::to_value(EndorsementClassification::EndorsementTrusted).unwrap(),
            serde_json::json!("endorsement-trusted")
        );
        assert_eq!(
            serde_json::to_value(EndorsementClassification::UnknownEndorser).unwrap(),
            serde_json::json!("unknown_endorser")
        );
        assert_eq!(
            serde_json::to_value(EndorsementClassification::AmbiguousEndorser).unwrap(),
            serde_json::json!("ambiguous_endorser")
        );
    }

    #[test]
    fn enrich_sets_kind_and_roles_only_for_resolved_endorsers() {
        let mut readbacks = vec![
            EndorsementReadback {
                classification: EndorsementClassification::EndorsementTrusted,
                endorser: Some(ActorId::new("actor:git-email:a@example.com")),
                reason: None,
                endorser_attributes: None,
            },
            EndorsementReadback {
                classification: EndorsementClassification::UnknownEndorser,
                endorser: None,
                reason: None,
                endorser_attributes: None,
            },
        ];
        let attrs = crate::session::actor_attributes_from_value(serde_json::json!({
            "actors": {
                "actor:git-email:a@example.com": { "kind": "human", "roles": ["reviewer"] }
            }
        }))
        .unwrap();
        enrich_endorser_attributes(&mut readbacks, Some(&attrs));
        let enriched = readbacks[0].endorser_attributes.as_ref().unwrap();
        assert_eq!(enriched.kind.as_deref(), Some("human"));
        assert!(enriched.roles.contains(&"reviewer".to_string()));
        // No resolved endorser → no attributes (and an empty resolution sets no field).
        assert!(
            readbacks[1].endorser_attributes.is_none(),
            "no endorser → no attributes"
        );
    }

    #[test]
    fn enrich_is_a_noop_without_a_map() {
        let mut readbacks = vec![EndorsementReadback {
            classification: EndorsementClassification::EndorsementTrusted,
            endorser: Some(ActorId::new("actor:git-email:a@example.com")),
            reason: None,
            endorser_attributes: None,
        }];
        enrich_endorser_attributes(&mut readbacks, None);
        assert!(readbacks[0].endorser_attributes.is_none());
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

    fn signer(seed: u8) -> SignerId {
        crate::crypto::SignerId::from_ed25519_public_key([seed; 32])
    }

    #[test]
    fn projected_inline_member_is_authoring_classification() {
        let signer_a = DeterministicSigner::from_seed(SIGNER_A_SEED);
        let target = inline_signed(&signer_a);
        let trust = trust_for_actor(&target.writer.actor_id.clone(), &signer_a);
        let set = cosignatures_for_event(
            std::slice::from_ref(&target),
            target.event_id.as_str(),
            &trust,
        )
        .unwrap();
        let inline = set
            .members
            .iter()
            .find(|m| m.source == CosignatureSource::Inline)
            .unwrap();
        assert!(matches!(
            inline.classification,
            CosignatureClassification::Authoring { reason: None }
        ));
    }

    #[test]
    fn projected_endorsement_member_is_endorsement_trusted() {
        // Target authored by signer_a (actor A). signer_b is enrolled ONLY under a distinct
        // endorser actor, so its detached member is UntrustedKey for A → endorsement-trusted.
        let signer_a = DeterministicSigner::from_seed(SIGNER_A_SEED);
        let signer_b = DeterministicSigner::from_seed(SIGNER_B_SEED);
        let target = inline_signed(&signer_a);
        let carrier = detached_carrier(&target, &signer_b);
        let endorser = crate::model::ActorId::new("actor:git-email:kevin@swiber.dev");
        let trust = crate::session::event_signature_trust_set(serde_json::json!({
            "allowedSigners": {
                target.writer.actor_id.as_str(): [signer_a.signer_id().as_str()],
                endorser.as_str(): [signer_b.signer_id().as_str()]
            }
        }))
        .unwrap();
        let set =
            cosignatures_for_event(&[target.clone(), carrier], target.event_id.as_str(), &trust)
                .unwrap();
        let detached = set
            .members
            .iter()
            .find(|m| matches!(m.source, CosignatureSource::Detached { .. }))
            .unwrap();
        assert_eq!(detached.status, EventVerificationStatus::UntrustedKey);
        assert!(matches!(
            &detached.classification,
            CosignatureClassification::EndorsementTrusted { endorser: e } if *e == endorser
        ));
    }

    #[test]
    fn inline_member_is_always_authoring() {
        let trust = TrustSet::default();
        let target = ActorId::new("actor:agent:claude-code");
        let c = classify_cosignature_member(
            &CosignatureSource::Inline,
            EventVerificationStatus::UntrustedKey, // even a non-Valid inline is failed AUTHORING.
            &signer(1),
            &target,
            &trust,
        );
        assert!(matches!(
            c,
            CosignatureClassification::Authoring { reason: None }
        ));
    }

    #[test]
    fn detached_valid_is_authoring() {
        let target = ActorId::new("actor:agent:claude-code");
        let trust = crate::session::event_signature_trust_set(serde_json::json!({
            "allowedSigners": { target.as_str(): [signer(2).as_str()] }
        }))
        .unwrap();
        let c = classify_cosignature_member(
            &CosignatureSource::Detached {
                carrier_event_id: "evt:sha256:x".into(),
            },
            EventVerificationStatus::Valid,
            &signer(2),
            &target,
            &trust,
        );
        assert!(matches!(
            c,
            CosignatureClassification::Authoring { reason: None }
        ));
    }

    #[test]
    fn detached_valid_signer_mapping_to_a_distinct_actor_is_authoring_not_endorsement() {
        let target = ActorId::new("actor:agent:claude-code");
        // Same key enrolled under BOTH the target actor and a distinct actor → laundering guard.
        let trust = crate::session::event_signature_trust_set(serde_json::json!({
            "allowedSigners": {
                target.as_str(): [signer(3).as_str()],
                "actor:git-email:kevin@swiber.dev": [signer(3).as_str()]
            }
        }))
        .unwrap();
        let c = classify_cosignature_member(
            &CosignatureSource::Detached {
                carrier_event_id: "evt:sha256:x".into(),
            },
            EventVerificationStatus::Valid,
            &signer(3),
            &target,
            &trust,
        );
        assert!(matches!(
            c,
            CosignatureClassification::Authoring {
                reason: Some(AuthoringReason::AuthoringNotEndorsement)
            }
        ));
    }

    #[test]
    fn detached_untrusted_resolving_to_one_distinct_actor_is_endorsement_trusted() {
        let target = ActorId::new("actor:agent:claude-code");
        let endorser = ActorId::new("actor:git-email:kevin@swiber.dev");
        // signer enrolled ONLY under the endorser (not the target) → UntrustedKey for target.
        let trust = crate::session::event_signature_trust_set(serde_json::json!({
            "allowedSigners": { endorser.as_str(): [signer(4).as_str()] }
        }))
        .unwrap();
        let c = classify_cosignature_member(
            &CosignatureSource::Detached {
                carrier_event_id: "evt:sha256:x".into(),
            },
            EventVerificationStatus::UntrustedKey,
            &signer(4),
            &target,
            &trust,
        );
        assert!(matches!(
            c,
            CosignatureClassification::EndorsementTrusted { endorser: e } if e == endorser
        ));
    }

    #[test]
    fn detached_untrusted_unenrolled_signer_is_unknown_endorser() {
        let target = ActorId::new("actor:agent:claude-code");
        let trust = TrustSet::default(); // bare, unenrolled did:key style → zero actors (INV-3).
        let c = classify_cosignature_member(
            &CosignatureSource::Detached {
                carrier_event_id: "evt:sha256:x".into(),
            },
            EventVerificationStatus::UntrustedKey,
            &signer(5),
            &target,
            &trust,
        );
        assert!(matches!(
            c,
            CosignatureClassification::EndorsementUntrusted {
                reason: EndorsementUntrustedReason::UnknownEndorser
            }
        ));
    }

    #[test]
    fn detached_untrusted_resolving_to_many_actors_is_ambiguous_endorser() {
        let target = ActorId::new("actor:agent:claude-code");
        let trust = crate::session::event_signature_trust_set(serde_json::json!({
            "allowedSigners": {
                "actor:git-email:kevin@swiber.dev": [signer(6).as_str()],
                "actor:git-email:alice@example.com": [signer(6).as_str()]
            }
        }))
        .unwrap();
        let c = classify_cosignature_member(
            &CosignatureSource::Detached {
                carrier_event_id: "evt:sha256:x".into(),
            },
            EventVerificationStatus::UntrustedKey,
            &signer(6),
            &target,
            &trust,
        );
        assert!(matches!(
            c,
            CosignatureClassification::EndorsementUntrusted {
                reason: EndorsementUntrustedReason::AmbiguousEndorser
            }
        ));
    }

    #[test]
    fn has_trusted_endorsement_true_only_with_an_endorsement_trusted_member() {
        let signer_a = DeterministicSigner::from_seed(SIGNER_A_SEED);
        let signer_b = DeterministicSigner::from_seed(SIGNER_B_SEED);
        let target = inline_signed(&signer_a);
        let carrier = detached_carrier(&target, &signer_b);
        let endorser = crate::model::ActorId::new("actor:git-email:kevin@swiber.dev");
        let trust = crate::session::event_signature_trust_set(serde_json::json!({
            "allowedSigners": {
                target.writer.actor_id.as_str(): [signer_a.signer_id().as_str()],
                endorser.as_str(): [signer_b.signer_id().as_str()]
            }
        }))
        .unwrap();
        let set =
            cosignatures_for_event(&[target.clone(), carrier], target.event_id.as_str(), &trust)
                .unwrap();

        assert!(
            set.has_trusted_endorsement(),
            "an own-identity endorser is a trusted endorsement"
        );
        // Binding is unaffected: the endorsement member is UntrustedKey, so it never binds.
        assert!(
            !set.has_valid_member() || set.inline_status() == Some(EventVerificationStatus::Valid)
        );
    }

    #[test]
    fn has_trusted_endorsement_false_for_authoring_only_set() {
        // Only the inline author attestation (authoring) — no endorsement.
        let signer_a = DeterministicSigner::from_seed(SIGNER_A_SEED);
        let target = inline_signed(&signer_a);
        let trust = trust_for_actor(&target.writer.actor_id.clone(), &signer_a);
        let set = cosignatures_for_event(
            std::slice::from_ref(&target),
            target.event_id.as_str(),
            &trust,
        )
        .unwrap();
        assert!(!set.has_trusted_endorsement());
        assert!(
            set.has_valid_member(),
            "the inline author attestation still binds (unchanged)"
        );
    }

    #[test]
    fn has_trusted_endorsement_false_for_unknown_endorser() {
        // signer_b enrolled under NO actor → its detached member is unknown_endorser, not trusted.
        let signer_a = DeterministicSigner::from_seed(SIGNER_A_SEED);
        let signer_b = DeterministicSigner::from_seed(SIGNER_B_SEED);
        let target = inline_signed(&signer_a);
        let carrier = detached_carrier(&target, &signer_b);
        let trust = trust_for_actor(&target.writer.actor_id.clone(), &signer_a);
        let set =
            cosignatures_for_event(&[target.clone(), carrier], target.event_id.as_str(), &trust)
                .unwrap();
        assert!(!set.has_trusted_endorsement());
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

    #[test]
    fn endorsement_carriers_for_one_triple_converge_byte_identical_across_writers() {
        use crate::session::event::{EventTarget, Writer, WriterProducer};

        let signer = DeterministicSigner::from_seed(SIGNER_B_SEED);
        let target = inline_signed(&DeterministicSigner::from_seed(SIGNER_A_SEED));

        // One attestation triple (target_record_hash, attesting_signer, attestation.sig).
        let attesting_signer = signer.signer_id().clone();
        let record_hash = target.event_record_hash().unwrap();
        let tbs = EventToBeSigned::from_event(&target, &attesting_signer).unwrap();
        let pae = event_signature_pre_authentication_encoding(&tbs).unwrap();
        let sig = signer.sign_event_message(&pae).unwrap();
        let payload = EventSignatureRecordedPayload {
            target_event_id: target.event_id.clone(),
            target_event_record_hash: record_hash.clone(),
            attesting_signer: attesting_signer.clone(),
            attestation: EventSignature::ed25519_v1(sig),
            inclusion_proof: None,
        };
        let key = EventSignatureRecordedPayload::idempotency_key(
            &record_hash,
            &attesting_signer,
            payload.attestation.sig.as_str(),
        );

        // The carrier payload carries NO meaning/relation field (INV-2): with inclusion_proof
        // absent, the serialized payload has EXACTLY the current key set — and explicitly none
        // of `relation` / `endorser` / `classification`. (A same-valued stored marker would not
        // diverge the hashes below, so this key-set assertion is the part that catches it.)
        let payload_json = serde_json::to_value(&payload).unwrap();
        let keys: std::collections::BTreeSet<&str> = payload_json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "attestation",
                "attestingSigner",
                "targetEventId",
                "targetEventRecordHash"
            ]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>(),
            "endorsement carrier payload must carry no meaning/relation field"
        );
        for forbidden in ["relation", "endorser", "classification"] {
            assert!(
                !payload_json.as_object().unwrap().contains_key(forbidden),
                "carrier payload must not carry `{forbidden}` (INV-2: derived or identity-bearing, never an excluded payload field)"
            );
        }

        // Two carriers for the SAME triple, differing ONLY in the envelope writer.
        let carrier = |actor: &str| {
            ShoreEvent::new(
                EventType::EventSignatureRecorded,
                key.clone(),
                EventTarget::for_event_signature(target.target.session_id.clone()),
                Writer {
                    actor_id: crate::model::ActorId::new(actor),
                    producer: WriterProducer {
                        name: "shore".into(),
                        version: "test".into(),
                    },
                },
                payload.clone(),
                "2026-06-04T00:00:00Z",
            )
            .unwrap()
        };
        let mirror_a = carrier("actor:git-email:alice@example.com");
        let mirror_b = carrier("actor:agent:bob");

        // Envelope writers differ...
        assert_ne!(mirror_a.writer.actor_id, mirror_b.writer.actor_id);
        // ...but identity + payload converge byte-for-byte.
        assert_eq!(mirror_a.idempotency_key, mirror_b.idempotency_key);
        assert_eq!(mirror_a.event_id, mirror_b.event_id);
        assert_eq!(mirror_a.payload_hash, mirror_b.payload_hash);
        assert_eq!(
            event_set_hash_for_events([&mirror_a]).unwrap(),
            event_set_hash_for_events([&mirror_b]).unwrap(),
            "envelope-only writer differences must not affect eventSetHash"
        );
    }
}
