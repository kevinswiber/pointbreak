use std::collections::{BTreeMap, BTreeSet};

use crate::crypto::EventVerificationStatus;
use crate::error::Result;
use crate::model::RevisionId;
use crate::session::event::{
    ArtifactRemovedPayload, EventType, ShoreEvent, WorkObjectProposal, WorkObjectProposedPayload,
};
use crate::session::projection::cosignature::CosignatureIndex;
use crate::session::signing::{RemovalPolicy, TrustSet};
use crate::session::{referenced_artifacts, verify_event_signature};

/// One recorded `ArtifactRemoved` claim over a content hash, kept whole so the
/// operative decision can read its non-reader-relative inputs without re-reading
/// the log: `event.ingest` for the local-possession arm; the
/// signature/signer/writer/occurred_at so `verify_event_signature` runs; and the
/// event itself as the cosignature-index target for the endorsement arm.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemovalClaim {
    pub content_hash: String,
    pub event: ShoreEvent,
}

/// Read-time projection of which content-addressed blobs have a recorded
/// `ArtifactRemoved` claim. A pure function of the event set; nothing new is
/// stored. Hashes are normalized `sha256:<hex>` exactly as written in the
/// payload. Whether a claim is *operative* is a separate, reader-relative
/// decision (the predicate methods below) — never folded at `from_events` time,
/// since verification depends on the reader's trust set.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ArtifactRemovalProjection {
    claims: BTreeMap<String, Vec<RemovalClaim>>,
}

/// The reader-relative operative status of the removal claim(s) over one content
/// hash, under a reader's trust set and a render policy. The `Operative*` arms
/// suppress/erase; the `Claim*` arms render a diagnostic instead. `ClaimInvalid`
/// is the integrity floor — a removal whose own inline signature verifies invalid
/// — and is never operative under any policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemovalOperativeStatus {
    NoClaim,
    OperativePossession,
    OperativeTrusted,
    ClaimUnsigned,
    ClaimUntrusted,
    ClaimInvalid,
}

/// How a capture re-binds a content hash that carries an operative removal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IdentityReuseKind {
    /// A distinct capture binds a content hash already bound by another revision —
    /// content/object reuse (the blob re-materializes while the removal persists).
    ContentObject,
    /// The same revision id is re-bound to the removed content hash.
    Revision,
}

impl IdentityReuseKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            IdentityReuseKind::ContentObject => "content/object reuse",
            IdentityReuseKind::Revision => "revision-id reuse",
        }
    }
}

/// One re-binding of an operatively-removed content hash by a capture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IdentityReuse {
    pub content_hash: String,
    pub revision_id: RevisionId,
    pub kind: IdentityReuseKind,
}

impl ArtifactRemovalProjection {
    pub fn from_events(events: &[ShoreEvent]) -> Result<Self> {
        let mut claims: BTreeMap<String, Vec<RemovalClaim>> = BTreeMap::new();
        for event in events {
            if event.event_type == EventType::ArtifactRemoved {
                let payload: ArtifactRemovedPayload =
                    serde_json::from_value(event.payload.clone())?;
                claims
                    .entry(payload.content_hash.clone())
                    .or_default()
                    .push(RemovalClaim {
                        content_hash: payload.content_hash,
                        event: event.clone(),
                    });
            }
        }
        Ok(Self { claims })
    }

    /// Back-compat claim-presence shim: a removal claim exists for this hash,
    /// regardless of trust. The Phase-1 read path and the compact safety floor
    /// read this before the trust-aware decision narrows it.
    pub fn is_removed(&self, content_hash: &str) -> bool {
        self.claims.contains_key(content_hash)
    }

    /// Every content hash with a claim, regardless of trust — for diagnostics.
    pub fn claimed_hashes(&self) -> impl Iterator<Item = &str> {
        self.claims.keys().map(String::as_str)
    }

    /// The reader-relative operative status of the claim(s) over `content_hash`.
    /// The strongest claim wins: any operative arm makes it operative; otherwise
    /// the most actionable non-operative reason is reported. The invalid floor
    /// never lifts and never raises the reason above `ClaimInvalid`.
    pub(crate) fn operative_status(
        &self,
        content_hash: &str,
        trust: &TrustSet,
        policy: RemovalPolicy,
        cosig: &CosignatureIndex<'_>,
    ) -> Result<RemovalOperativeStatus> {
        let Some(claims) = self.claims.get(content_hash) else {
            return Ok(RemovalOperativeStatus::NoClaim);
        };
        // Weakest seed; any non-invalid claim raises it, an operative claim returns early.
        let mut best = RemovalOperativeStatus::ClaimInvalid;
        for claim in claims {
            let (status, arm_a, arm_b) = self.evaluate(claim, trust, cosig)?;
            if status == EventVerificationStatus::Invalid {
                // Integrity floor: never operative, never lifts; leaves `best` at
                // ClaimInvalid unless a later claim raises it.
                continue;
            }
            let operative = match policy {
                RemovalPolicy::Advisory => false,
                RemovalPolicy::PossessionOrTrusted => arm_a || arm_b,
                RemovalPolicy::TrustedStrict => arm_b,
            };
            if operative {
                return Ok(if arm_b {
                    RemovalOperativeStatus::OperativeTrusted
                } else {
                    RemovalOperativeStatus::OperativePossession
                });
            }
            best = max_status(best, claim_reason(status));
        }
        Ok(best)
    }

    /// Whether the removal of `content_hash` is operative for render-time
    /// suppression under `policy`.
    pub(crate) fn is_operative_suppress(
        &self,
        content_hash: &str,
        trust: &TrustSet,
        policy: RemovalPolicy,
        cosig: &CosignatureIndex<'_>,
    ) -> Result<bool> {
        Ok(matches!(
            self.operative_status(content_hash, trust, policy, cosig)?,
            RemovalOperativeStatus::OperativePossession | RemovalOperativeStatus::OperativeTrusted
        ))
    }

    /// Compact's FIXED erase-eligibility rule. It NEVER reads the render preset:
    /// a laxer render policy can never widen what is irreversibly erased. A blob
    /// is erase-eligible iff its removal would be operative under
    /// `PossessionOrTrusted` (possession OR trust, never the invalid floor).
    pub(crate) fn is_erase_eligible(
        &self,
        content_hash: &str,
        trust: &TrustSet,
        cosig: &CosignatureIndex<'_>,
    ) -> Result<bool> {
        self.is_operative_suppress(
            content_hash,
            trust,
            RemovalPolicy::PossessionOrTrusted,
            cosig,
        )
    }

    /// `(verification status, arm_a, arm_b)` for one claim. Reader-relative; never
    /// cached. The status is returned (not a derived `invalid` bool) so the caller
    /// can both apply the invalid floor AND classify a non-operative claim
    /// (unsigned vs untrusted) from this single evaluation, with no re-verify.
    fn evaluate(
        &self,
        claim: &RemovalClaim,
        trust: &TrustSet,
        cosig: &CosignatureIndex<'_>,
    ) -> Result<(EventVerificationStatus, bool, bool)> {
        let status = verify_event_signature(&claim.event, trust)?;
        let arm_a = claim.event.ingest.is_none();
        let arm_b = status == EventVerificationStatus::Valid
            || cosig
                .cosignatures_for_target(&claim.event, trust)?
                .has_trusted_endorsement();
        Ok((status, arm_a, arm_b))
    }

    /// Claimed content hashes that no event in `events` references — the removal
    /// target was never present in this store. Computed against the canonical
    /// referenced-artifact index, which covers BOTH object/snapshot blobs AND
    /// note-body blobs, so a legitimate note-body removal is not falsely reported
    /// as missing. (`ArtifactRemoved` itself references no artifact, so a removed
    /// hash counts as referenced only when some other event binds it.)
    pub(crate) fn target_missing_diagnostics(&self, events: &[ShoreEvent]) -> Result<Vec<String>> {
        let referenced: BTreeSet<String> = referenced_artifacts(events)?
            .into_iter()
            .map(|artifact| artifact.content_hash().to_owned())
            .collect();
        Ok(self
            .claimed_hashes()
            .filter(|hash| !referenced.contains(*hash))
            .map(str::to_owned)
            .collect())
    }

    /// Captures that re-bind a content hash carrying an OPERATIVE removal. A hash
    /// bound by ≥2 distinct revisions while operatively removed is content/object
    /// reuse; the same revision id re-bound to it is revision-id reuse. Reading
    /// the per-capture `WorkObjectProposed` binding map distinguishes
    /// content/object reuse from revision reuse.
    pub(crate) fn identity_reuse_diagnostics(
        &self,
        events: &[ShoreEvent],
        trust: &TrustSet,
        policy: RemovalPolicy,
        cosig: &CosignatureIndex<'_>,
    ) -> Result<Vec<IdentityReuse>> {
        let bindings = binding_revisions_by_content_hash(events)?;
        let mut out = Vec::new();
        for (content_hash, revisions) in &bindings {
            if !self.is_operative_suppress(content_hash, trust, policy, cosig)? {
                continue;
            }
            let mut counts: BTreeMap<&RevisionId, usize> = BTreeMap::new();
            for revision in revisions {
                *counts.entry(revision).or_default() += 1;
            }
            let distinct = counts.len();
            for (revision, count) in counts {
                let kind = if count >= 2 {
                    IdentityReuseKind::Revision
                } else if distinct >= 2 {
                    IdentityReuseKind::ContentObject
                } else {
                    continue;
                };
                out.push(IdentityReuse {
                    content_hash: content_hash.clone(),
                    revision_id: revision.clone(),
                    kind,
                });
            }
        }
        Ok(out)
    }
}

/// Map each content hash to the ordered list of revision ids whose
/// `WorkObjectProposed` captures bound it (duplicates preserved, so a revision
/// re-binding the same hash is visible as a repeat).
fn binding_revisions_by_content_hash(
    events: &[ShoreEvent],
) -> Result<BTreeMap<String, Vec<RevisionId>>> {
    let mut map: BTreeMap<String, Vec<RevisionId>> = BTreeMap::new();
    for event in events {
        if event.event_type != EventType::WorkObjectProposed {
            continue;
        }
        let payload: WorkObjectProposedPayload = serde_json::from_value(event.payload.clone())?;
        if let WorkObjectProposal::Revision {
            revision,
            object_artifact_content_hash,
            ..
        } = payload.work_object
        {
            map.entry(object_artifact_content_hash)
                .or_default()
                .push(revision.id);
        }
    }
    Ok(map)
}

/// Classify a non-operative, non-invalid claim by its verification status:
/// `Unsigned` is ratifiable as `ClaimUnsigned`; an untrusted-key signature is
/// `ClaimUntrusted`. (`Invalid` never reaches here — it is the floor; `Valid`
/// never reaches here — it is operative.)
fn claim_reason(status: EventVerificationStatus) -> RemovalOperativeStatus {
    match status {
        EventVerificationStatus::Unsigned => RemovalOperativeStatus::ClaimUnsigned,
        _ => RemovalOperativeStatus::ClaimUntrusted,
    }
}

/// Keep the most actionable non-operative reason across a hash's claims:
/// `ClaimUntrusted` over `ClaimUnsigned`, both over the `ClaimInvalid` floor
/// (which survives only when every claim is invalid).
fn max_status(a: RemovalOperativeStatus, b: RemovalOperativeStatus) -> RemovalOperativeStatus {
    fn rank(status: RemovalOperativeStatus) -> u8 {
        match status {
            RemovalOperativeStatus::ClaimInvalid => 0,
            RemovalOperativeStatus::ClaimUnsigned => 1,
            RemovalOperativeStatus::ClaimUntrusted => 2,
            // Operative statuses and NoClaim are never compared here.
            _ => 3,
        }
    }
    if rank(b) >= rank(a) { b } else { a }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{EventSignatureBytes, EventSigner};
    use crate::model::{ActorId, EngagementId, JournalId, ObjectId, RevisionId};
    use crate::session::event::{
        EventSignature, EventSignatureRecordedPayload, EventTarget, EventToBeSigned,
        IngestProvenance, IngestVia, Revision, WorkObjectProposal, WorkObjectProposedPayload,
        Writer, WriterProducer, event_signature_pre_authentication_encoding,
    };
    use crate::session::projection::cosignature::CosignatureIndex;
    use crate::session::signing::TrustSet;
    use crate::session::signing::test_support::{DeterministicSigner, trust_for_actor};

    /// A `WorkObjectProposed` capture binding `object_id` + `content_hash` under
    /// `revision_id`, the oracle the reuse diagnostic reads.
    fn work_object_proposed(revision_id: &str, object_id: &str, content_hash: &str) -> ShoreEvent {
        let revision_id = RevisionId::new(revision_id);
        ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("work_object_proposed:{}", revision_id.as_str()),
            EventTarget::for_revision(JournalId::new("journal:fixture"), revision_id.clone(), None)
                .unwrap(),
            Writer::shore_local("test"),
            WorkObjectProposedPayload {
                engagement_id: EngagementId::new("engagement:sha256:fixture"),
                work_object: WorkObjectProposal::Revision {
                    revision: Revision {
                        id: revision_id,
                        object_id: ObjectId::new(object_id),
                        git_provenance: None,
                    },
                    summary: None,
                    object_artifact_content_hash: content_hash.to_owned(),
                    supersedes: vec![],
                },
            },
            "2026-06-19T00:00:00Z",
        )
        .unwrap()
    }

    const REMOVER_SEED: [u8; 32] = [71u8; 32];
    const ENDORSER_SEED: [u8; 32] = [72u8; 32];

    fn remover_actor() -> ActorId {
        ActorId::new("actor:git-email:remover@example.com")
    }

    fn endorser_actor() -> ActorId {
        ActorId::new("actor:git-email:endorser@example.com")
    }

    /// A bare unsigned, locally-authored (`ingest = None`) removal for `content_hash`.
    fn removal_event(content_hash: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ArtifactRemoved,
            ArtifactRemovedPayload::idempotency_key(content_hash),
            EventTarget::for_journal(JournalId::new("journal:fixture")),
            Writer {
                actor_id: remover_actor(),
                producer: WriterProducer {
                    name: "shore".to_owned(),
                    version: "test".to_owned(),
                },
            },
            ArtifactRemovedPayload {
                content_hash: content_hash.to_owned(),
            },
            "2026-06-19T00:00:00Z",
        )
        .unwrap()
    }

    /// Mark an event as ingested through a foreign-event seam (`ingest = Some`),
    /// which drops the local-possession arm.
    fn ingested(mut event: ShoreEvent) -> ShoreEvent {
        event.ingest = Some(IngestProvenance {
            via: IngestVia::IngestEvents,
            received_at: "2026-06-19T01:00:00Z".to_owned(),
        });
        event
    }

    fn sign_inline(mut event: ShoreEvent, signer: &DeterministicSigner) -> ShoreEvent {
        event.signer = None;
        event.signature = None;
        let tbs = EventToBeSigned::from_event(&event, signer.signer_id()).unwrap();
        let pae = event_signature_pre_authentication_encoding(&tbs).unwrap();
        let sig = signer.sign_event_message(&pae).unwrap();
        event.signer = Some(signer.signer_id().clone());
        event.signature = Some(EventSignature::ed25519_v1(sig));
        event
    }

    /// A detached `event_signature` carrier endorsing `target`, signed by `signer`.
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
        ShoreEvent::new(
            EventType::EventSignatureRecorded,
            key,
            EventTarget::for_journal(target.target.journal_id.clone()),
            Writer::shore_local("test"),
            payload,
            "2026-06-04T00:00:00Z",
        )
        .unwrap()
    }

    /// Trust enrolling `signer` as an endorser under a distinct actor (so its
    /// detached attestation classifies endorsement-trusted for the remover).
    fn endorser_trust(signer: &DeterministicSigner) -> TrustSet {
        crate::session::event_signature_trust_set(serde_json::json!({
            "allowedSigners": {
                endorser_actor().as_str(): [signer.signer_id().as_str()]
            }
        }))
        .unwrap()
    }

    fn status_of(
        events: &[ShoreEvent],
        content_hash: &str,
        trust: &TrustSet,
        policy: RemovalPolicy,
    ) -> RemovalOperativeStatus {
        let projection = ArtifactRemovalProjection::from_events(events).unwrap();
        let cosig = CosignatureIndex::build(events).unwrap();
        projection
            .operative_status(content_hash, trust, policy, &cosig)
            .unwrap()
    }

    #[test]
    fn from_events_collects_removed_content_hashes() {
        let events = vec![removal_event("sha256:a"), removal_event("sha256:b")];
        let projection = ArtifactRemovalProjection::from_events(&events).unwrap();
        assert!(projection.is_removed("sha256:a"));
        assert!(projection.is_removed("sha256:b"));
        assert!(!projection.is_removed("sha256:c"));
    }

    #[test]
    fn from_events_ignores_non_removal_events() {
        // Any non-removal event contributes no claim.
        let mut other = removal_event("sha256:anything");
        other.event_type = EventType::ReviewInitialized;
        let projection = ArtifactRemovalProjection::from_events(&[other]).unwrap();
        assert!(!projection.is_removed("sha256:anything"));
        assert!(projection.claimed_hashes().next().is_none());
    }

    #[test]
    fn possessed_unsigned_removal_is_operative_under_default() {
        let events = vec![removal_event("sha256:h")];
        assert_eq!(
            status_of(
                &events,
                "sha256:h",
                &TrustSet::default(),
                RemovalPolicy::PossessionOrTrusted
            ),
            RemovalOperativeStatus::OperativePossession
        );
    }

    #[test]
    fn possessed_unsigned_removal_is_not_operative_under_trusted_strict() {
        let events = vec![removal_event("sha256:h")];
        let projection = ArtifactRemovalProjection::from_events(&events).unwrap();
        let cosig = CosignatureIndex::build(&events).unwrap();
        assert!(
            !projection
                .is_operative_suppress(
                    "sha256:h",
                    &TrustSet::default(),
                    RemovalPolicy::TrustedStrict,
                    &cosig
                )
                .unwrap()
        );
    }

    #[test]
    fn ingested_unsigned_removal_is_claim_unsigned() {
        let events = vec![ingested(removal_event("sha256:h"))];
        assert_eq!(
            status_of(
                &events,
                "sha256:h",
                &TrustSet::default(),
                RemovalPolicy::PossessionOrTrusted
            ),
            RemovalOperativeStatus::ClaimUnsigned
        );
    }

    #[test]
    fn ingested_untrusted_key_removal_is_claim_untrusted() {
        let signer = DeterministicSigner::from_seed(REMOVER_SEED);
        // A crypto-valid signature, but no trust authorizes the signer → UntrustedKey.
        let events = vec![ingested(sign_inline(removal_event("sha256:h"), &signer))];
        assert_eq!(
            status_of(
                &events,
                "sha256:h",
                &TrustSet::default(),
                RemovalPolicy::PossessionOrTrusted
            ),
            RemovalOperativeStatus::ClaimUntrusted
        );
    }

    #[test]
    fn ingested_removal_with_trusted_endorsement_is_operative() {
        let endorser = DeterministicSigner::from_seed(ENDORSER_SEED);
        let removal = ingested(removal_event("sha256:h"));
        let carrier = detached_carrier(&removal, &endorser);
        let events = vec![removal, carrier];
        assert_eq!(
            status_of(
                &events,
                "sha256:h",
                &endorser_trust(&endorser),
                RemovalPolicy::PossessionOrTrusted
            ),
            RemovalOperativeStatus::OperativeTrusted
        );
    }

    #[test]
    fn valid_trusted_signer_removal_is_operative() {
        let signer = DeterministicSigner::from_seed(REMOVER_SEED);
        let events = vec![ingested(sign_inline(removal_event("sha256:h"), &signer))];
        let trust = trust_for_actor(&remover_actor(), &signer);
        assert_eq!(
            status_of(
                &events,
                "sha256:h",
                &trust,
                RemovalPolicy::PossessionOrTrusted
            ),
            RemovalOperativeStatus::OperativeTrusted
        );
    }

    #[test]
    fn invalid_signature_removal_is_claim_invalid_even_possessed() {
        let signer = DeterministicSigner::from_seed(REMOVER_SEED);
        // Locally authored (ingest = None) but the inline signature is tampered.
        let mut event = sign_inline(removal_event("sha256:h"), &signer);
        event.signature = Some(EventSignature::ed25519_v1(EventSignatureBytes::from_bytes(
            &[0u8; 64],
        )));
        let events = vec![event];
        assert_eq!(
            status_of(
                &events,
                "sha256:h",
                &trust_for_actor(&remover_actor(), &signer),
                RemovalPolicy::PossessionOrTrusted
            ),
            RemovalOperativeStatus::ClaimInvalid
        );
    }

    #[test]
    fn target_missing_flags_a_removal_over_an_unreferenced_hash() {
        // A removal whose content hash is referenced by no event in the store.
        let events = vec![removal_event("sha256:ghost")];
        let projection = ArtifactRemovalProjection::from_events(&events).unwrap();
        let missing = projection.target_missing_diagnostics(&events).unwrap();
        assert_eq!(missing, vec!["sha256:ghost".to_owned()]);
    }

    #[test]
    fn target_missing_does_not_flag_a_referenced_removed_hash() {
        // A capture references the hash, so its removal is not target-missing.
        let events = vec![
            work_object_proposed("review-unit:a", "snap:a", "sha256:real"),
            removal_event("sha256:real"),
        ];
        let projection = ArtifactRemovalProjection::from_events(&events).unwrap();
        assert!(
            projection
                .target_missing_diagnostics(&events)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn identity_reuse_flags_two_distinct_captures_of_an_operatively_removed_hash() {
        // Two distinct revisions bind the same content hash; a possessed
        // (operative) removal targets it → content/object reuse.
        let content_hash = "sha256:shared";
        let events = vec![
            work_object_proposed("review-unit:a", "snap:a", content_hash),
            work_object_proposed("review-unit:b", "snap:b", content_hash),
            removal_event(content_hash),
        ];
        let projection = ArtifactRemovalProjection::from_events(&events).unwrap();
        let cosig = CosignatureIndex::build(&events).unwrap();
        let reuse = projection
            .identity_reuse_diagnostics(
                &events,
                &TrustSet::default(),
                RemovalPolicy::PossessionOrTrusted,
                &cosig,
            )
            .unwrap();
        assert!(
            reuse
                .iter()
                .any(|r| r.content_hash == content_hash
                    && r.kind == IdentityReuseKind::ContentObject)
        );
    }

    #[test]
    fn identity_reuse_ignores_a_non_operative_removal() {
        // The same shared hash, but the removal is ingested + unsigned → a
        // non-operative claim under the default policy; reuse is not flagged.
        let content_hash = "sha256:shared";
        let events = vec![
            work_object_proposed("review-unit:a", "snap:a", content_hash),
            work_object_proposed("review-unit:b", "snap:b", content_hash),
            ingested(removal_event(content_hash)),
        ];
        let projection = ArtifactRemovalProjection::from_events(&events).unwrap();
        let cosig = CosignatureIndex::build(&events).unwrap();
        let reuse = projection
            .identity_reuse_diagnostics(
                &events,
                &TrustSet::default(),
                RemovalPolicy::PossessionOrTrusted,
                &cosig,
            )
            .unwrap();
        assert!(reuse.is_empty());
    }

    #[test]
    fn erase_eligible_ignores_render_preset() {
        // A possessed unsigned removal is erase-eligible (possession), even though
        // a TrustedStrict render preset would not suppress it.
        let events = vec![removal_event("sha256:h")];
        let projection = ArtifactRemovalProjection::from_events(&events).unwrap();
        let cosig = CosignatureIndex::build(&events).unwrap();
        assert!(
            projection
                .is_erase_eligible("sha256:h", &TrustSet::default(), &cosig)
                .unwrap()
        );
        assert!(
            !projection
                .is_operative_suppress(
                    "sha256:h",
                    &TrustSet::default(),
                    RemovalPolicy::TrustedStrict,
                    &cosig
                )
                .unwrap()
        );
    }
}
