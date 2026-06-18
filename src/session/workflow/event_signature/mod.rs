//! Construction of detached co-signature carrier events.
//!
//! `record_event_signature` builds and records a co-signature over an existing
//! target event, mirroring `record_assessment`'s resolve → `ShoreEvent::new` →
//! record → rebuild-state flow. The crux that makes a co-signature distinct from
//! an ordinary signed write: the embedded **attestation** is a direct signature
//! over the **target's** signer-inclusive `event-tbs.v1` view (with `signer` set
//! to the attesting signer), not over the carrier event being written.
//!
//! **Two digests, never confused.** The attestation signs the signer-INCLUSIVE
//! target TBS (the view naming the attesting signer); the carrier binds the
//! signer-EXCLUSIVE `targetEventRecordHash`. They cover different field sets and
//! are not interchangeable.
//!
//! **Carrier-envelope orthogonality (ADR-0004 amendment D4).** The carrier's own
//! envelope `signer`/`signature` are independent of the embedded `attestation`.
//! A co-signature's trust rests entirely on the embedded attestation verifying
//! against the trust set — never on who wrapped it in an event. In v1 the carrier
//! is not inline-signed; the attestation and the carrier envelope are kept
//! strictly separate so a future reader cannot conflate them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::crypto::{EventSigner, SignerId};
use crate::error::{Result, ShoreError};
use crate::model::{ActorId, EventId};
use crate::session::event::{
    EventSignature, EventSignatureRecordedPayload, EventTarget, EventToBeSigned, EventType,
    ShoreEvent, Writer, event_signature_pre_authentication_encoding,
};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store::resolution::resolve_write_validation_store;
use crate::session::store_init::{ShoreStorePaths, prepare_shore_writer};
use crate::session::workflow::write_store::fact_batch_only_diagnostics;
use crate::session::{
    CosignatureGateDecision, EventStore, EventWriteOutcome, TrustSet, current_timestamp,
    gate_cosignature_for_store, writer_from_options,
};
use crate::storage::{Durability, LocalStorage};

/// Options for recording a detached co-signature over an existing target event.
///
/// The attesting signer is required and passed directly (it IS the co-signature,
/// and the construction must sign the target's bytes itself) — it is not routed
/// through `EventSigningOptions`, which would only sign the carrier's own
/// envelope.
pub struct EventSignatureRecordOptions {
    repo: PathBuf,
    target_event_id: EventId,
    attesting_signer: Arc<dyn EventSigner + Send + Sync>,
    actor_id: Option<ActorId>,
}

impl EventSignatureRecordOptions {
    pub fn new<S>(repo: impl AsRef<Path>, target_event_id: EventId, attesting_signer: S) -> Self
    where
        S: EventSigner + Send + Sync + 'static,
    {
        Self {
            repo: repo.as_ref().to_path_buf(),
            target_event_id,
            attesting_signer: Arc::new(attesting_signer),
            actor_id: None,
        }
    }

    /// Attribute the carrier's envelope writer to an explicit actor. This is the
    /// recorder identity, independent of the attesting signer.
    pub fn with_actor_id(mut self, actor_id: ActorId) -> Self {
        self.actor_id = Some(actor_id);
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventSignatureRecordResult {
    /// The carrier's `eventId`.
    pub event_id: EventId,
    pub target_event_id: EventId,
    pub target_event_record_hash: String,
    pub attesting_signer: SignerId,
    pub events_created: usize,
    pub events_existing: usize,
    pub events_created_by_type: BTreeMap<String, usize>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn record_event_signature(
    options: EventSignatureRecordOptions,
) -> Result<EventSignatureRecordResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let worktree_root = paths.worktree_root();
    let store_dir = paths.store_dir();
    let storage = LocalStorage::new(store_dir);
    prepare_shore_writer(&paths, &storage)?;

    // The write half keeps the LOCAL prior batch for the single-writer state.json.
    let event_store = EventStore::open(store_dir);
    let events = event_store.list_events()?;

    // Resolve the target against the writer-visible union so a linked-only target
    // still resolves. A co-signature whose target is not present cannot be verified
    // (the target is needed to reconstruct its TBS view and recompute its
    // `eventRecordHash`), so an absent target is refused here. The cross-store
    // target-absent case is the ingest gate's `cosignature_target_pending`; the
    // replay-after-backfill ordering is deferred to the sync plane, not invented here.
    let validation_store = resolve_write_validation_store(&options.repo)?;
    let validation_events = validation_store.validation_events()?;
    let target = validation_events
        .into_iter()
        .find(|event| event.event_id == options.target_event_id)
        .ok_or_else(|| {
            ShoreError::Message(format!(
                "co-signature target event not found: {}",
                options.target_event_id.as_str()
            ))
        })?;

    // The carrier binds the signer-EXCLUSIVE content identity.
    let target_event_record_hash = target.event_record_hash()?;

    // The attestation signs the target's signer-INCLUSIVE TBS view, with `signer`
    // overridden to the attesting signer — NOT the target's own effective signer.
    // (`event_to_be_signed` would resolve the target's signer; we must sign the view
    // that names the attesting signer.)
    let signer = options.attesting_signer.as_ref();
    let attesting_signer = signer.signer_id().clone();
    let tbs = EventToBeSigned::from_event(&target, &attesting_signer)?;
    let pae = event_signature_pre_authentication_encoding(&tbs)?;
    let sig_bytes = signer.sign_event_message(&pae)?;
    let attestation = EventSignature::ed25519_v1(sig_bytes);

    let writer = writer_from_options(worktree_root, options.actor_id.as_ref());
    // Build, gate, and record the carrier through the shared assembly path. Trust
    // does not gate storage, so an empty trust set is correct here; the target is
    // always present, so only `DropInvalid`/`BindingMismatch` can refuse a
    // locally-constructed carrier (both indicate a broken signer — surface loudly).
    let record = assemble_and_record_cosignature(
        &event_store,
        &target,
        &attesting_signer,
        &attestation,
        writer,
        &TrustSet::default(),
        current_timestamp(),
    )?;
    if let Some(code) = record.decision.drop_diagnostic_code() {
        return Err(ShoreError::Message(format!(
            "refusing to store co-signature carrier ({code})"
        )));
    }

    let outcome = record
        .write_outcome
        .expect("a stored decision yields a write outcome");
    let event_id = record.carrier.event_id.clone();
    let mut events_created_by_type = BTreeMap::new();
    let (events_created, events_existing) = match outcome {
        EventWriteOutcome::Created => {
            events_created_by_type.insert(EventType::EventSignatureRecorded.as_str().to_owned(), 1);
            (1, 0)
        }
        EventWriteOutcome::Existing | EventWriteOutcome::ExistingDivergentSignature => (0, 1),
    };

    let state = SessionState::from_prior_events_and_committed(&events, &record.carrier, outcome)?;
    storage.write_json_atomic(&paths.state_path(), &state, Durability::Projection)?;

    let mut diagnostics = state.diagnostics;
    diagnostics.extend(fact_batch_only_diagnostics(&validation_store));

    Ok(EventSignatureRecordResult {
        event_id,
        target_event_id: target.event_id,
        target_event_record_hash,
        attesting_signer,
        events_created,
        events_existing,
        events_created_by_type,
        diagnostics,
    })
}

/// The result of assembling and gating one co-signature carrier: the built carrier,
/// the verify-before-store gate decision, and the store outcome (`Some` only when
/// the gate permitted storage, `None` on any drop decision).
pub(crate) struct CosignatureRecord {
    pub carrier: ShoreEvent,
    pub decision: CosignatureGateDecision,
    pub write_outcome: Option<EventWriteOutcome>,
}

/// Assemble a co-signature carrier over `target` from an attestation already in
/// hand, run it through the shared verify-before-store gate, and record it when the
/// gate permits. This is the one assembly path shared by local construction (which
/// supplies a freshly-signed attestation) and ingest transcription (which supplies
/// an attestation it received and can verify — transcription, never minting). The
/// carrier's `idempotencyKey` is the full attestation triple, so an identical triple
/// always reduces to the same member.
pub(crate) fn assemble_and_record_cosignature(
    event_store: &EventStore,
    target: &ShoreEvent,
    attesting_signer: &SignerId,
    attestation: &EventSignature,
    writer: Writer,
    trust: &TrustSet,
    occurred_at: String,
) -> Result<CosignatureRecord> {
    let target_event_record_hash = target.event_record_hash()?;
    let payload = EventSignatureRecordedPayload {
        target_event_id: target.event_id.clone(),
        target_event_record_hash: target_event_record_hash.clone(),
        attesting_signer: attesting_signer.clone(),
        attestation: attestation.clone(),
        inclusion_proof: None,
    };
    let idempotency_key = EventSignatureRecordedPayload::idempotency_key(
        &target_event_record_hash,
        attesting_signer,
        attestation.sig.as_str(),
    );
    // v1: the carrier is NOT inline-signed; the co-signature lives entirely in the
    // payload `attestation`, never in the carrier envelope (D4 orthogonality).
    let carrier = ShoreEvent::new(
        EventType::EventSignatureRecorded,
        idempotency_key,
        EventTarget::for_event_signature(target.target.session_id.clone()),
        writer,
        payload.clone(),
        occurred_at,
    )?;

    let decision = gate_cosignature_for_store(&payload, Some(target), trust)?;
    let write_outcome = if decision.stores() {
        Some(event_store.record_event_once(&carrier)?)
    } else {
        None
    };

    Ok(CosignatureRecord {
        carrier,
        decision,
        write_outcome,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{EventVerificationStatus, TestEd25519Signer, verify_ed25519_strict};
    use crate::model::ActorId;
    use crate::session::event::EventSignatureRecordedPayload;
    use crate::session::{CaptureOptions, EventStore, capture_worktree_review};

    fn modified_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 {\n    1\n}\n");
        repo.git(&["add", "."]);
        repo.git(&["commit", "-m", "base"]);
        repo.write("src/lib.rs", "pub fn value() -> u32 {\n    2\n}\n");
        repo
    }

    fn captured_target(repo: &TestRepo) -> EventId {
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let events = EventStore::open(repo.path().join(".shore/data"))
            .list_events()
            .unwrap();
        events
            .iter()
            .find(|event| event.event_type == EventType::ReviewUnitCaptured)
            .expect("captured review unit event present")
            .event_id
            .clone()
    }

    fn stored_carrier(repo: &TestRepo) -> ShoreEvent {
        let events = EventStore::open(repo.path().join(".shore/data"))
            .list_events()
            .unwrap();
        events
            .into_iter()
            .find(|event| event.event_type == EventType::EventSignatureRecorded)
            .expect("carrier event present")
    }

    fn stored_target(repo: &TestRepo, target_event_id: &EventId) -> ShoreEvent {
        let events = EventStore::open(repo.path().join(".shore/data"))
            .list_events()
            .unwrap();
        events
            .into_iter()
            .find(|event| &event.event_id == target_event_id)
            .expect("target event present")
    }

    fn verify_attestation(
        target: &ShoreEvent,
        payload: &EventSignatureRecordedPayload,
    ) -> EventVerificationStatus {
        let tbs = EventToBeSigned::from_event(target, &payload.attesting_signer).unwrap();
        let pae = event_signature_pre_authentication_encoding(&tbs).unwrap();
        verify_ed25519_strict(
            &payload.attesting_signer,
            &pae,
            payload.attestation.sig.as_str(),
        )
        .unwrap()
    }

    fn carrier_payload(carrier: &ShoreEvent) -> EventSignatureRecordedPayload {
        serde_json::from_value(carrier.payload.clone()).unwrap()
    }

    #[test]
    fn record_event_signature_constructs_carrier_that_verifies_valid() {
        let repo = modified_repo();
        let target_event_id = captured_target(&repo);
        let signer = TestEd25519Signer::from_seed([3u8; 32]);
        let signer_id = signer.signer_id().clone();

        let result = record_event_signature(EventSignatureRecordOptions::new(
            repo.path(),
            target_event_id.clone(),
            signer,
        ))
        .unwrap();

        assert_eq!(result.events_created, 1);
        assert_eq!(result.events_existing, 0);
        assert_eq!(result.attesting_signer, signer_id);
        assert_eq!(result.target_event_id, target_event_id);
        assert_eq!(result.events_created_by_type["event_signature_recorded"], 1);

        let carrier = stored_carrier(&repo);
        let payload = carrier_payload(&carrier);
        assert_eq!(payload.attesting_signer, signer_id);
        assert_eq!(
            payload.target_event_record_hash,
            stored_target(&repo, &target_event_id)
                .event_record_hash()
                .unwrap()
        );
        assert_eq!(
            verify_attestation(&stored_target(&repo, &target_event_id), &payload),
            EventVerificationStatus::Valid
        );
    }

    #[test]
    fn enrolled_actor_endorsement_classifies_endorsement_trusted_end_to_end() {
        use crate::crypto::TestEd25519Signer;
        use crate::session::event_signature_trust_set;
        use crate::session::projection::cosignature::{
            CosignatureClassification, CosignatureSource, cosignatures_for_event,
        };

        // A captured target authored by the repo's git identity actor (NOT the endorser).
        let repo = modified_repo();
        let target_event_id = captured_target(&repo);
        let target = stored_target(&repo, &target_event_id);
        let target_actor = target.writer.actor_id.clone();

        // The endorser: a distinct actor whose key signs in its OWN identity.
        let endorser_signer = TestEd25519Signer::from_seed([42u8; 32]);
        let endorser_signer_id = endorser_signer.signer_id().clone();
        let endorser_actor = ActorId::new("actor:git-email:kevin@swiber.dev");
        assert_ne!(
            endorser_actor, target_actor,
            "endorser must differ from the target's author"
        );

        // Endorse through the REAL producer the CLI uses (carrier writer = endorser's own actor).
        record_event_signature(
            EventSignatureRecordOptions::new(repo.path(), target_event_id.clone(), endorser_signer)
                .with_actor_id(endorser_actor.clone()),
        )
        .unwrap();

        // Project with a trust set that enrolls the endorser's key ONLY under the endorser
        // actor (so the carrier is UntrustedKey for the target's actor → endorsement candidate).
        let events = EventStore::open(repo.path().join(".shore/data"))
            .list_events()
            .unwrap();
        let trust = event_signature_trust_set(serde_json::json!({
            "allowedSigners": { endorser_actor.as_str(): [endorser_signer_id.as_str()] }
        }))
        .unwrap();
        let set = cosignatures_for_event(&events, target_event_id.as_str(), &trust).unwrap();

        let detached = set
            .members
            .iter()
            .find(|m| matches!(m.source, CosignatureSource::Detached { .. }))
            .expect("the endorsement carrier is a detached member");
        assert!(
            matches!(&detached.classification,
                CosignatureClassification::EndorsementTrusted { endorser } if *endorser == endorser_actor),
            "an enrolled actor's own-identity endorsement classifies endorsement-trusted: {:?}",
            detached.classification
        );
        assert!(
            set.has_trusted_endorsement(),
            "has_trusted_endorsement() is true"
        );

        // Binding is unaffected by the endorsement member (ADR-0013 binding/stewardship
        // split): the endorsement carrier is UntrustedKey (it is not authorized for the
        // target's actor), so it never contributes to `has_valid_member`.
        assert_eq!(detached.status, EventVerificationStatus::UntrustedKey);
        assert!(
            !set.has_valid_member(),
            "an UntrustedKey endorsement does not make the set binding-valid"
        );
    }

    #[test]
    fn inline_author_signature_is_cosignature_one_with_no_transformation() {
        let repo = modified_repo();
        let target_event_id = captured_target(&repo);
        let target = stored_target(&repo, &target_event_id);

        let signer = TestEd25519Signer::from_seed([9u8; 32]);
        // The inline author signature S would produce over the target's TBS.
        let inline_tbs = EventToBeSigned::from_event(&target, signer.signer_id()).unwrap();
        let inline_pae = event_signature_pre_authentication_encoding(&inline_tbs).unwrap();
        let inline_sig = signer.sign_event_message(&inline_pae).unwrap();

        record_event_signature(EventSignatureRecordOptions::new(
            repo.path(),
            target_event_id,
            signer,
        ))
        .unwrap();

        let payload = carrier_payload(&stored_carrier(&repo));
        assert_eq!(
            payload.attestation.sig.as_str(),
            inline_sig.as_str(),
            "the same signer over the same target produces a byte-identical attestation"
        );
    }

    #[test]
    fn attestation_signs_signer_inclusive_target_tbs_not_carrier() {
        let repo = modified_repo();
        let target_event_id = captured_target(&repo);
        let signer = TestEd25519Signer::from_seed([4u8; 32]);

        record_event_signature(EventSignatureRecordOptions::new(
            repo.path(),
            target_event_id.clone(),
            signer,
        ))
        .unwrap();

        let carrier = stored_carrier(&repo);
        let payload = carrier_payload(&carrier);
        let target = stored_target(&repo, &target_event_id);

        // Verifies against the target's TBS with signer = attesting signer.
        assert_eq!(
            verify_attestation(&target, &payload),
            EventVerificationStatus::Valid
        );
        // Does NOT verify when checked as if it signed the carrier's own TBS.
        assert_ne!(
            verify_attestation(&carrier, &payload),
            EventVerificationStatus::Valid
        );
    }

    #[test]
    fn two_signers_over_one_target_yield_two_distinct_carriers() {
        let repo = modified_repo();
        let target_event_id = captured_target(&repo);

        let a = record_event_signature(EventSignatureRecordOptions::new(
            repo.path(),
            target_event_id.clone(),
            TestEd25519Signer::from_seed([1u8; 32]),
        ))
        .unwrap();
        let b = record_event_signature(EventSignatureRecordOptions::new(
            repo.path(),
            target_event_id,
            TestEd25519Signer::from_seed([2u8; 32]),
        ))
        .unwrap();

        assert_ne!(a.event_id, b.event_id);
        let carriers: Vec<_> = EventStore::open(repo.path().join(".shore/data"))
            .list_events()
            .unwrap()
            .into_iter()
            .filter(|event| event.event_type == EventType::EventSignatureRecorded)
            .collect();
        assert_eq!(carriers.len(), 2);
    }

    #[test]
    fn resubmitting_identical_cosignature_is_idempotent() {
        let repo = modified_repo();
        let target_event_id = captured_target(&repo);

        let first = record_event_signature(EventSignatureRecordOptions::new(
            repo.path(),
            target_event_id.clone(),
            TestEd25519Signer::from_seed([7u8; 32]),
        ))
        .unwrap();
        let second = record_event_signature(EventSignatureRecordOptions::new(
            repo.path(),
            target_event_id,
            TestEd25519Signer::from_seed([7u8; 32]),
        ))
        .unwrap();

        assert_eq!(first.events_created, 1);
        assert_eq!(second.events_created, 0);
        assert_eq!(second.events_existing, 1);
        assert_eq!(first.event_id, second.event_id);
    }

    #[test]
    fn cosignature_target_absent_errors() {
        let repo = modified_repo();
        captured_target(&repo);

        let result = record_event_signature(EventSignatureRecordOptions::new(
            repo.path(),
            EventId::new(
                "evt:sha256:0000000000000000000000000000000000000000000000000000000000000000",
            ),
            TestEd25519Signer::from_seed([5u8; 32]),
        ));

        assert!(
            result.is_err(),
            "a co-signature whose target is absent is refused"
        );
    }

    #[test]
    fn carrier_envelope_is_not_inline_signed_in_v1() {
        let repo = modified_repo();
        let target_event_id = captured_target(&repo);

        record_event_signature(EventSignatureRecordOptions::new(
            repo.path(),
            target_event_id,
            TestEd25519Signer::from_seed([6u8; 32]),
        ))
        .unwrap();

        let carrier = stored_carrier(&repo);
        assert!(carrier.signer.is_none());
        assert!(carrier.signature.is_none());
    }

    #[test]
    fn carrier_with_explicit_actor_id_records_that_envelope_writer() {
        let repo = modified_repo();
        let target_event_id = captured_target(&repo);

        record_event_signature(
            EventSignatureRecordOptions::new(
                repo.path(),
                target_event_id,
                TestEd25519Signer::from_seed([8u8; 32]),
            )
            .with_actor_id(ActorId::new("actor:agent:cosigner")),
        )
        .unwrap();

        let carrier = stored_carrier(&repo);
        assert_eq!(carrier.writer.actor_id.as_str(), "actor:agent:cosigner");
    }

    struct TestRepo {
        root: tempfile::TempDir,
    }

    impl TestRepo {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("create temp git repository directory");
            let repo = Self { root };
            repo.git(&["init"]);
            repo.git(&["config", "user.name", "Shore Tests"]);
            repo.git(&["config", "user.email", "shore-tests@example.com"]);
            repo.git(&["config", "commit.gpgsign", "false"]);
            repo
        }

        fn path(&self) -> &Path {
            self.root.path()
        }

        fn write(&self, path: &str, contents: &str) {
            let path = self.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }

        fn git(&self, args: &[&str]) {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(self.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
                args,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
