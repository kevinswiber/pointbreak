use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::event_signature::assemble_and_record_cosignature;
use crate::crypto::EventVerificationStatus;
use crate::error::{Result, ShoreError};
use crate::session::event::{
    EventSignatureRecordedPayload, EventType, IngestVia, ShoreEvent, resolve_effective_signer,
    stamp_ingest_provenance,
};
use crate::session::state::{ProjectionDiagnostic, SessionState};
use crate::session::store::resolution::{prepare_write_landing, resolve_write_store};
use crate::session::{
    COSIGNATURE_BINDING_MISMATCH_CODE, COSIGNATURE_INVALID_CODE, COSIGNATURE_TARGET_PENDING_CODE,
    COSIGNATURE_UNTRUSTED_SIGNER_CODE, CosignatureGateDecision, EventStore,
    EventVerificationPolicy, EventWriteOutcome, IngestEventVerification, TrustSet,
    current_timestamp, gate_cosignature_for_store, is_valid_actor_id, verify_events_for_ingest,
    writer_from_options,
};
use crate::storage::{Durability, LocalStorage};

/// Options for ingesting one or more pre-formed events into a repo's `.shore/data`
/// store — for example events produced on another machine and forwarded over a
/// network, or merged from another clone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IngestEventsOptions {
    repo: PathBuf,
    events: Vec<ShoreEvent>,
    verification_policy: EventVerificationPolicy,
    trust_set: TrustSet,
}

impl IngestEventsOptions {
    pub fn new(repo: impl AsRef<Path>, events: Vec<ShoreEvent>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            events,
            verification_policy: EventVerificationPolicy::advisory(),
            trust_set: TrustSet::default(),
        }
    }

    pub fn with_verification_policy(mut self, policy: EventVerificationPolicy) -> Self {
        self.verification_policy = policy;
        self
    }

    pub fn with_trust_set(mut self, trust_set: TrustSet) -> Self {
        self.trust_set = trust_set;
        self
    }
}

/// Options for ingesting a single pre-formed event. Thin convenience over
/// [`IngestEventsOptions`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportEventOptions {
    repo: PathBuf,
    event: ShoreEvent,
    verification_policy: EventVerificationPolicy,
    trust_set: TrustSet,
}

impl ImportEventOptions {
    pub fn new(repo: impl AsRef<Path>, event: ShoreEvent) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            event,
            verification_policy: EventVerificationPolicy::advisory(),
            trust_set: TrustSet::default(),
        }
    }

    pub fn with_verification_policy(mut self, policy: EventVerificationPolicy) -> Self {
        self.verification_policy = policy;
        self
    }

    pub fn with_trust_set(mut self, trust_set: TrustSet) -> Self {
        self.trust_set = trust_set;
        self
    }
}

/// The outcome of an ingest: how many events were newly written vs. already
/// present (idempotent re-ingest), a per-type breakdown of the newly written
/// events, and the projection diagnostics after the rebuild.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IngestEventsResult {
    pub events_created: usize,
    pub events_existing: usize,
    pub events_created_by_type: BTreeMap<String, usize>,
    /// One row per verified event. Rows for non-carrier events appear in input
    /// order; a stored detached co-signature carrier appends its row when the
    /// write loop stores it; a dropped carrier has no row. On a successful ingest
    /// every row's `write_outcome` is `Some`.
    pub verification: Vec<IngestEventVerification>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

/// Ingest a single pre-formed event. See [`ingest_events`].
pub fn import_event(options: ImportEventOptions) -> Result<IngestEventsResult> {
    ingest_events(
        IngestEventsOptions::new(options.repo, vec![options.event])
            .with_verification_policy(options.verification_policy)
            .with_trust_set(options.trust_set),
    )
}

/// Ingest pre-formed events into the repo's durable store, preserving the
/// store's append-only, content-addressed, idempotent, and conflict semantics.
///
/// Each event is recorded through the same primitive the domain write workflows
/// use, so a re-ingest of an already-present event is a no-op (`events_existing`)
/// and an event that collides with a different payload under the same
/// idempotency key is rejected. Malformed envelopes (bad `eventId`/`payloadHash`
/// /schema) are rejected, and the writer attribution is validated up front: an
/// event whose `writer.actor_id` is not a well-formed `actor:` id is rejected
/// before anything is written, so the whole batch is atomic on attribution.
///
/// After recording, the projection (`state.json`) is rebuilt once from the full
/// event log. If a write fails partway through a batch (e.g. a conflict), the
/// events already written remain durable and the projection is still rebuilt to
/// match what is on disk before the error is returned — re-ingesting the batch
/// is safe.
pub fn ingest_events(options: IngestEventsOptions) -> Result<IngestEventsResult> {
    let write_store = resolve_write_store(&options.repo)?;
    let store_dir = write_store.store_dir();
    let storage = LocalStorage::new(store_dir);
    prepare_write_landing(&write_store, &storage)?;

    // Reject malformed attribution before any write so the batch is atomic on
    // attribution: a bad actor id can never partially corrupt the log.
    for event in &options.events {
        if !is_valid_actor_id(event.writer.actor_id.as_str()) {
            return Err(ShoreError::InvalidEvent {
                message: format!(
                    "ingested event {} has a malformed writer actor id: {}",
                    event.event_id.as_str(),
                    event.writer.actor_id.as_str()
                ),
            });
        }
    }

    let mut verification = verify_events_for_ingest(
        &options.events,
        options.verification_policy,
        &options.trust_set,
    )?;

    let stamped = stamp_ingest_provenance(
        &options.events,
        IngestVia::IngestEvents,
        &current_timestamp(),
    );

    let event_store = EventStore::from_backend(write_store.backend());
    let worktree_root = write_store.worktree_root();
    let mut events_created = 0usize;
    let mut events_existing = 0usize;
    let mut events_created_by_type: BTreeMap<String, usize> = BTreeMap::new();
    let mut ingest_diagnostics = Vec::new();
    let mut write_error = None;
    // The nth non-carrier stamped event corresponds to `verification[n]`: both are
    // input-ordered with carriers skipped, and carrier rows only ever append past
    // the initial segment. Index matching, not event-id matching — duplicate event
    // ids in one batch would mis-assign by id.
    let mut verified_row_cursor = 0usize;

    for event in &stamped {
        // A standalone detached co-signature carrier from a peer flows through the
        // family verify-before-store gate, NOT the plain record path: the gate is
        // the always-on family rule (reject `invalid`, keep `untrusted_key`/`valid`,
        // drop on absent target), independent of `EventVerificationPolicy`.
        if event.event_type == EventType::EventSignatureRecorded {
            match ingest_detached_cosignature(
                &event_store,
                event,
                &options.trust_set,
                &mut verification,
                &mut ingest_diagnostics,
            ) {
                Ok((created, existing)) => {
                    events_created += created;
                    events_existing += existing;
                    if created > 0 {
                        *events_created_by_type
                            .entry(event.event_type.as_str().to_owned())
                            .or_default() += 1;
                    }
                }
                Err(err) => {
                    write_error = Some(err);
                    break;
                }
            }
            continue;
        }

        let row_index = verified_row_cursor;
        verified_row_cursor += 1;
        match event_store.record_event_once(event) {
            Ok(outcome) => {
                verification[row_index].write_outcome = Some(outcome);
                match outcome {
                    EventWriteOutcome::Created => {
                        events_created += 1;
                        *events_created_by_type
                            .entry(event.event_type.as_str().to_owned())
                            .or_default() += 1;
                    }
                    EventWriteOutcome::Existing => events_existing += 1,
                    EventWriteOutcome::ExistingDivergentSignature => {
                        // Class-(b) dissolution: a divergent inline signature over the same
                        // content record is not a conflict — the store keeps its first-stored
                        // copy and transcribes the incoming attestation into a co-signature
                        // carrier, converging the set to both signers with no winner-selection.
                        events_existing += 1;
                        match transcribe_divergent_signature(
                            &event_store,
                            event,
                            worktree_root,
                            &options.trust_set,
                            &mut ingest_diagnostics,
                        ) {
                            Ok((created, existing)) => {
                                events_existing += existing;
                                if created > 0 {
                                    events_created += created;
                                    *events_created_by_type
                                        .entry(
                                            EventType::EventSignatureRecorded.as_str().to_owned(),
                                        )
                                        .or_default() += 1;
                                }
                            }
                            Err(err) => {
                                write_error = Some(err);
                                break;
                            }
                        }
                    }
                }
            }
            Err(err) => {
                write_error = Some(err);
                break;
            }
        }
    }

    // Rebuild the projection from whatever is durably on disk — even on a
    // partial-batch failure — so state.json never drifts from the event log.
    let events = event_store.list_events()?;
    let state = SessionState::from_events(&events)?;
    storage.write_json_atomic(
        &store_dir.join("state.json"),
        &state,
        Durability::Projection,
    )?;
    let mut diagnostics = state.diagnostics;
    diagnostics.extend(ingest_diagnostics);

    if let Some(err) = write_error {
        return Err(err);
    }

    Ok(IngestEventsResult {
        events_created,
        events_existing,
        events_created_by_type,
        verification,
        diagnostics,
    })
}

/// Ingest a standalone detached co-signature carrier from a peer through the shared
/// verify-before-store gate. Returns `(events_created, events_existing)` for the
/// carrier, pushing the carrier's embedded-attestation status to `verification` and
/// any drop/authorization diagnostics. A carrier is an ordinary event: when stored
/// it rides the same event-set machinery as every event, with no separate channel.
fn ingest_detached_cosignature(
    event_store: &EventStore,
    event: &ShoreEvent,
    trust: &TrustSet,
    verification: &mut Vec<IngestEventVerification>,
    diagnostics: &mut Vec<ProjectionDiagnostic>,
) -> Result<(usize, usize)> {
    let payload: EventSignatureRecordedPayload = serde_json::from_value(event.payload.clone())?;
    // Resolve the target by content identity. The store keys on the idempotency-key,
    // so there is no eventId path lookup; scan the event set for the named target.
    let stored = event_store.list_events()?;
    let target = stored
        .iter()
        .find(|stored_event| stored_event.event_id == payload.target_event_id);

    match gate_cosignature_for_store(&payload, target, trust)? {
        CosignatureGateDecision::Store(status) => {
            let outcome = event_store.record_event_once(event)?;
            let counts = match outcome {
                EventWriteOutcome::Created => (1, 0),
                // A carrier's identity is the full attestation triple, so a divergent
                // carrier would be a distinct member (a different eventId), never a
                // divergent signature of the same carrier.
                EventWriteOutcome::Existing | EventWriteOutcome::ExistingDivergentSignature => {
                    (0, 1)
                }
            };
            // Report the carrier's EMBEDDED-attestation status (not its unsigned
            // envelope) in the verification vector.
            verification.push(IngestEventVerification {
                event_id: event.event_id.clone(),
                status,
                message: cosignature_verification_message(status),
                write_outcome: Some(outcome),
            });
            if status == EventVerificationStatus::UntrustedKey
                && outcome == EventWriteOutcome::Created
                && let Some(target) = target
            {
                diagnostics.push(cosignature_untrusted_signer_diagnostic(
                    event.event_id.as_str(),
                    payload.target_event_id.as_str(),
                    payload.attesting_signer.as_str(),
                    target.writer.actor_id.as_str(),
                ));
            }
            Ok(counts)
        }
        CosignatureGateDecision::DropInvalid => {
            diagnostics.push(drop_diagnostic(
                COSIGNATURE_INVALID_CODE,
                format!(
                    "detached co-signature {} over event {} has an invalid attestation and was not stored",
                    event.event_id.as_str(),
                    payload.target_event_id.as_str()
                ),
            ));
            Ok((0, 0))
        }
        CosignatureGateDecision::TargetPending => {
            // No replay here: the carrier leaves no trace and is re-offered by the
            // sync cursor once its target arrives.
            diagnostics.push(drop_diagnostic(
                COSIGNATURE_TARGET_PENDING_CODE,
                format!(
                    "detached co-signature {} targets event {}, which is not present; not stored",
                    event.event_id.as_str(),
                    payload.target_event_id.as_str()
                ),
            ));
            Ok((0, 0))
        }
        CosignatureGateDecision::BindingMismatch => {
            diagnostics.push(drop_diagnostic(
                COSIGNATURE_BINDING_MISMATCH_CODE,
                format!(
                    "detached co-signature {} binds an eventRecordHash that does not match its target {}; not stored",
                    event.event_id.as_str(),
                    payload.target_event_id.as_str()
                ),
            ));
            Ok((0, 0))
        }
    }
}

/// Transcribe an incoming divergent inline attestation into a co-signature carrier.
/// The incoming attestation is a real signature the importer RECEIVED and can
/// verify — re-homing it is transcription, never minting; the co-signer's private
/// key is never required (the relay never signs as the reviewer). Returns
/// `(events_created, events_existing)` for the transcribed carrier.
fn transcribe_divergent_signature(
    event_store: &EventStore,
    event: &ShoreEvent,
    worktree_root: &Path,
    trust: &TrustSet,
    diagnostics: &mut Vec<ProjectionDiagnostic>,
) -> Result<(usize, usize)> {
    // The divergent outcome required a stored event under the same idempotencyKey;
    // it is the kept first-stored copy and shares the incoming event's eventRecordHash.
    let stored_target = event_store
        .read_event(&event_store.event_path_for_idempotency_key(&event.idempotency_key))?;
    // An incoming duplicate with no inline attestation carries nothing to transcribe
    // (e.g. a signed local event vs. an unsigned peer duplicate — divergent binding,
    // matching eventRecordHash). The stored copy is kept; the set is unchanged. This
    // is a clean no-op, never a batch-failing error.
    let Some(attestation) = event.signature.clone() else {
        return Ok((0, 0));
    };
    // The attesting signer is the incoming event's EFFECTIVE signer: its top-level
    // `signer`, or the did:key from a self-certifying actor_id (where `signer` is
    // intentionally omitted). Reading `event.signer` directly would drop a
    // self-certifying attestation. A signed-but-unresolvable signer is a malformed
    // event with no verifiable attestation to transcribe — also a no-op.
    let attesting_signer = match resolve_effective_signer(event) {
        Ok(signer) => signer,
        Err(_) => return Ok((0, 0)),
    };

    // The carrier is authored by the importer; its envelope writer is the local
    // identity, orthogonal to the embedded attestation.
    let writer = writer_from_options(worktree_root, None);
    let record = assemble_and_record_cosignature(
        event_store,
        &stored_target,
        &attesting_signer,
        &attestation,
        writer,
        trust,
        current_timestamp(),
    )?;

    match record.decision {
        CosignatureGateDecision::Store(status) => {
            let outcome = record
                .write_outcome
                .expect("a stored decision yields a write outcome");
            let counts = match outcome {
                EventWriteOutcome::Created => (1, 0),
                EventWriteOutcome::Existing | EventWriteOutcome::ExistingDivergentSignature => {
                    (0, 1)
                }
            };
            if status == EventVerificationStatus::UntrustedKey
                && outcome == EventWriteOutcome::Created
            {
                diagnostics.push(cosignature_untrusted_signer_diagnostic(
                    record.carrier.event_id.as_str(),
                    stored_target.event_id.as_str(),
                    attesting_signer.as_str(),
                    stored_target.writer.actor_id.as_str(),
                ));
            }
            Ok(counts)
        }
        // An invalid incoming inline attestation is reader-independent noise and is
        // never transcribed. BindingMismatch/TargetPending are impossible here (the
        // sharpened predicate guaranteed a present target with a matching hash).
        CosignatureGateDecision::DropInvalid
        | CosignatureGateDecision::BindingMismatch
        | CosignatureGateDecision::TargetPending => Ok((0, 0)),
    }
}

fn drop_diagnostic(code: &str, message: String) -> ProjectionDiagnostic {
    ProjectionDiagnostic {
        code: code.to_owned(),
        message,
    }
}

/// An authorization observation (never a divergence report): the merged co-signature
/// is real and the set unioned cleanly, but its signer is not authorized for the
/// claimed actor in this reader's trust set.
fn cosignature_untrusted_signer_diagnostic(
    carrier_event_id: &str,
    target_event_id: &str,
    attesting_signer: &str,
    claimed_actor: &str,
) -> ProjectionDiagnostic {
    ProjectionDiagnostic {
        code: COSIGNATURE_UNTRUSTED_SIGNER_CODE.to_owned(),
        message: format!(
            "merged co-signature {carrier_event_id} over event {target_event_id} is signed by \
             {attesting_signer}, which is not authorized for actor {claimed_actor} in this trust set"
        ),
    }
}

fn cosignature_verification_message(status: EventVerificationStatus) -> Option<String> {
    match status {
        EventVerificationStatus::Valid => None,
        EventVerificationStatus::UntrustedKey => {
            Some("co-signature signer is not authorized by the trust set".to_owned())
        }
        EventVerificationStatus::Invalid => Some("co-signature attestation is invalid".to_owned()),
        EventVerificationStatus::Unsigned => {
            Some("co-signature carrier has no attestation".to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use serde_json::json;

    use super::*;
    use crate::canonical_hash::sha256_json_prefixed;
    use crate::crypto::{EventSignatureBytes, EventSigner, EventVerificationStatus};
    use crate::model::{
        ActorId, InputRequestId, InputRequestResponseId, JournalId, TargetRef, TaskTargetRef,
        WorkObjectId,
    };
    use crate::session::event::{
        AssertionMode, EventSignature, EventSignatureRecordedPayload, EventToBeSigned, EventType,
        IngestProvenance, IngestVia, InputRequestReasonCode, InputRequestResponseOutcome,
        event_signature_pre_authentication_encoding,
    };
    use crate::session::projection::freshness::event_set_hash_for_events;
    use crate::session::projection::task::{
        AgentResumptionProjection, AgentResumptionState, ResumptionBindingPolicy,
        agent_resumption_from_events,
    };
    use crate::session::projection::test_support::{
        reader_actor, task_attempt_event, task_input_request_event_with_target, user_response_event,
    };
    use crate::session::signing::test_support::{DeterministicSigner, trust_for_actor};
    use crate::session::store::resolution::{resolve_read_store, resolve_store};
    use crate::session::{
        CaptureOptions, EventSignatureRecordOptions, EventVerificationPolicy,
        InputRequestListOptions, InputRequestOpenOptions, InputRequestRespondOptions,
        InputRequestStatus, InputRequestStatusFilter, TrustSet, capture_worktree_review,
        event_signature_trust_set, list_input_requests, open_input_request, record_event_signature,
        respond_input_request, verify_event_signature,
    };

    struct TestRepo {
        root: tempfile::TempDir,
    }

    impl TestRepo {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("create temp git repository directory");
            let repo = Self { root };
            repo.git(["init"]);
            repo.git(["config", "user.name", "Shore Tests"]);
            repo.git(["config", "user.email", "shore-tests@example.com"]);
            repo.git(["config", "commit.gpgsign", "false"]);
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

        fn commit_all(&self, message: &str) {
            self.git(["add", "."]);
            self.git(["commit", "-m", message]);
        }

        fn git<I, S>(&self, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<std::ffi::OsStr>,
        {
            let output = Command::new("git")
                .args(args)
                .current_dir(self.path())
                .output()
                .expect("run git command");
            assert!(output.status.success(), "git failed");
        }
    }

    fn modified_repo() -> TestRepo {
        let repo = TestRepo::new();
        repo.write("src/lib.rs", "pub fn value() -> u32 {\n    1\n}\n");
        repo.commit_all("base");
        repo.write("src/lib.rs", "pub fn value() -> u32 {\n    2\n}\n");
        repo
    }

    /// Build an origin store with a captured review unit + one responded input
    /// request, returning its full event log.
    fn origin_events() -> (TestRepo, Vec<ShoreEvent>) {
        let repo = modified_repo();
        capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
        let opened = open_input_request(
            InputRequestOpenOptions::new(repo.path())
                .with_track("human:kevin")
                .with_title("Need approval")
                .with_reason_code(InputRequestReasonCode::ManualDecisionRequired),
        )
        .unwrap();
        respond_input_request(
            InputRequestRespondOptions::new(repo.path(), opened.input_request_id.clone())
                .with_outcome(InputRequestResponseOutcome::Approved)
                .with_actor_id(ActorId::new("actor:agent:remote-reviewer")),
        )
        .unwrap();
        let events = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap();
        (repo, events)
    }

    fn dest_repo() -> TestRepo {
        // The destination only needs a valid repo root to host its resolved store.
        modified_repo()
    }

    /// The store a workflow (capture, ingest, signature) actually lands in for a
    /// repo — the shared common-dir store by default. Reads that follow such a
    /// workflow resolve here, never the raw worktree-local `.shore/data`.
    fn resolved_store_dir(repo: &Path) -> PathBuf {
        crate::git::git_common_dir(repo).unwrap().join("shore")
    }

    #[test]
    fn linked_ingest_lands_events_in_clone_local_store() {
        use crate::git::git_common_dir;
        use crate::session::ShoreStorePaths;

        let (_origin, events) = origin_events();
        let dest = dest_repo();

        ingest_events(IngestEventsOptions::new(dest.path(), events)).unwrap();

        // Write-through (INV-1): ingested events land in the clone-local store.
        let clone_local = git_common_dir(dest.path()).unwrap().join("shore");
        assert!(
            !EventStore::open(&clone_local)
                .list_events()
                .unwrap()
                .is_empty(),
            "ingest lands events in the clone-local store"
        );
        // The worktree-local `.shore/data` received no ingested events.
        let local = ShoreStorePaths::resolve(dest.path()).unwrap();
        assert!(
            EventStore::open(local.store_dir())
                .list_events()
                .unwrap_or_default()
                .is_empty(),
            "worktree-local store received no ingested events in linked mode"
        );
    }

    fn on_disk_state(repo: &Path) -> serde_json::Value {
        serde_json::from_str(
            &std::fs::read_to_string(resolved_store_dir(repo).join("state.json")).unwrap(),
        )
        .unwrap()
    }

    fn replayed_state(repo: &Path) -> serde_json::Value {
        let events = EventStore::open(resolved_store_dir(repo))
            .list_events()
            .unwrap();
        serde_json::to_value(SessionState::from_events(&events).unwrap()).unwrap()
    }

    fn signed_captured_event() -> (ShoreEvent, DeterministicSigner, ActorId) {
        let repo = modified_repo();
        let signer = DeterministicSigner::fixture();
        let actor = ActorId::new("actor:git-email:alice@example.com");
        capture_worktree_review(
            CaptureOptions::new(repo.path())
                .with_actor_id(actor.clone())
                .sign_with(signer.clone()),
        )
        .unwrap();
        let event = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap()
            .into_iter()
            .find(|event| event.event_type == EventType::WorkObjectProposed)
            .unwrap();

        (event, signer, actor)
    }

    fn invalid_signed_event() -> (ShoreEvent, TrustSet) {
        let (mut event, signer, actor) = signed_captured_event();
        event.payload["tamperedAfterSigning"] = json!(true);
        event.payload_hash = sha256_json_prefixed(&event.payload).unwrap();
        let trust = trust_for_actor(&actor, &signer);

        (event, trust)
    }

    fn unsigned_event() -> ShoreEvent {
        let (_origin, events) = origin_events();
        events
            .into_iter()
            .find(|event| event.event_type == EventType::WorkObjectProposed)
            .unwrap()
    }

    fn stored_event_count(repo: &Path) -> usize {
        let events_dir = resolved_store_dir(repo).join("events");
        if !events_dir.exists() {
            return 0;
        }

        EventStore::open(resolved_store_dir(repo))
            .list_events()
            .unwrap()
            .len()
    }

    #[test]
    fn advisory_ingest_accepts_invalid_signature_and_reports_status() {
        let (event, trust) = invalid_signed_event();
        let dest = dest_repo();

        let result = ingest_events(
            IngestEventsOptions::new(dest.path(), vec![event.clone()])
                .with_verification_policy(EventVerificationPolicy::advisory())
                .with_trust_set(trust),
        )
        .unwrap();

        assert_eq!(result.events_created, 1);
        assert_eq!(result.verification.len(), 1);
        assert_eq!(result.verification[0].event_id, event.event_id);
        assert_eq!(
            result.verification[0].status,
            EventVerificationStatus::Invalid
        );

        let (untrusted, _signer, _actor) = signed_captured_event();
        let untrusted_dest = dest_repo();
        let untrusted_result = ingest_events(
            IngestEventsOptions::new(untrusted_dest.path(), vec![untrusted])
                .with_verification_policy(EventVerificationPolicy::advisory()),
        )
        .unwrap();
        assert_eq!(untrusted_result.events_created, 1);
        assert_eq!(
            untrusted_result.verification[0].status,
            EventVerificationStatus::UntrustedKey
        );

        let unsigned = unsigned_event();
        let unsigned_dest = dest_repo();
        let unsigned_result = ingest_events(
            IngestEventsOptions::new(unsigned_dest.path(), vec![unsigned])
                .with_verification_policy(EventVerificationPolicy::advisory()),
        )
        .unwrap();
        assert_eq!(unsigned_result.events_created, 1);
        assert_eq!(
            unsigned_result.verification[0].status,
            EventVerificationStatus::Unsigned
        );
    }

    #[test]
    fn integrity_strict_rejects_invalid_but_accepts_unsigned() {
        let (invalid, trust) = invalid_signed_event();
        let rejected_dest = dest_repo();

        let error = ingest_events(
            IngestEventsOptions::new(rejected_dest.path(), vec![invalid])
                .with_verification_policy(EventVerificationPolicy::integrity_strict())
                .with_trust_set(trust),
        )
        .unwrap_err();

        assert!(
            error.to_string().contains("invalid"),
            "unexpected error: {error}"
        );
        assert_eq!(stored_event_count(rejected_dest.path()), 0);

        let unsigned = unsigned_event();
        let accepted_dest = dest_repo();
        let result = ingest_events(
            IngestEventsOptions::new(accepted_dest.path(), vec![unsigned])
                .with_verification_policy(EventVerificationPolicy::integrity_strict()),
        )
        .unwrap();

        assert_eq!(result.events_created, 1);
        assert_eq!(
            result.verification[0].status,
            EventVerificationStatus::Unsigned
        );
    }

    #[test]
    fn trusted_strict_rejects_untrusted_and_unsigned_unless_allowed() {
        let (untrusted, _signer, _actor) = signed_captured_event();
        let untrusted_dest = dest_repo();

        let untrusted_error = ingest_events(
            IngestEventsOptions::new(untrusted_dest.path(), vec![untrusted])
                .with_verification_policy(EventVerificationPolicy::trusted_strict())
                .with_trust_set(TrustSet::default()),
        )
        .unwrap_err();

        assert!(
            untrusted_error.to_string().contains("untrusted_key"),
            "unexpected error: {untrusted_error}"
        );
        assert_eq!(stored_event_count(untrusted_dest.path()), 0);

        let unsigned = unsigned_event();
        let unsigned_dest = dest_repo();
        let unsigned_error = ingest_events(
            IngestEventsOptions::new(unsigned_dest.path(), vec![unsigned.clone()])
                .with_verification_policy(EventVerificationPolicy::trusted_strict()),
        )
        .unwrap_err();

        assert!(
            unsigned_error.to_string().contains("unsigned"),
            "unexpected error: {unsigned_error}"
        );
        assert_eq!(stored_event_count(unsigned_dest.path()), 0);

        let allowed_unsigned_dest = dest_repo();
        let result = ingest_events(
            IngestEventsOptions::new(allowed_unsigned_dest.path(), vec![unsigned])
                .with_verification_policy(
                    EventVerificationPolicy::trusted_strict().with_allow_unsigned(true),
                ),
        )
        .unwrap();

        assert_eq!(result.events_created, 1);
        assert_eq!(
            result.verification[0].status,
            EventVerificationStatus::Unsigned
        );
    }

    /// Re-sign a copy of `event` with `signer` over the target's signer-inclusive
    /// TBS view, producing a genuine, verifiable inline attestation.
    fn signed_copy(event: &ShoreEvent, signer: &DeterministicSigner) -> ShoreEvent {
        let mut copy = event.clone();
        copy.signer = None;
        copy.signature = None;
        copy.ingest = None;
        let tbs = EventToBeSigned::from_event(&copy, signer.signer_id()).unwrap();
        let pae = event_signature_pre_authentication_encoding(&tbs).unwrap();
        let sig = signer.sign_event_message(&pae).unwrap();
        copy.signer = Some(signer.signer_id().clone());
        copy.signature = Some(EventSignature::ed25519_v1(sig));
        copy
    }

    fn two_signer_trust(
        actor: &ActorId,
        a: &DeterministicSigner,
        b: &DeterministicSigner,
    ) -> TrustSet {
        event_signature_trust_set(json!({
            "allowedSigners": {
                actor.as_str(): [a.signer_id().as_str(), b.signer_id().as_str()],
            }
        }))
        .unwrap()
    }

    fn carrier_in(repo: &Path) -> ShoreEvent {
        EventStore::open(resolved_store_dir(repo))
            .list_events()
            .unwrap()
            .into_iter()
            .find(|event| event.event_type == EventType::EventSignatureRecorded)
            .expect("a transcribed/ingested carrier is present")
    }

    fn carrier_payload(carrier: &ShoreEvent) -> EventSignatureRecordedPayload {
        serde_json::from_value(carrier.payload.clone()).unwrap()
    }

    /// Build a peer store that captured + signed a target and authored a detached
    /// co-signature carrier over it, returning `(target, carrier)`.
    fn peer_target_and_carrier(
        signer: &DeterministicSigner,
        actor: &ActorId,
    ) -> (ShoreEvent, ShoreEvent) {
        let repo = modified_repo();
        capture_worktree_review(
            CaptureOptions::new(repo.path())
                .with_actor_id(actor.clone())
                .sign_with(signer.clone()),
        )
        .unwrap();
        let target = EventStore::open(resolved_store_dir(repo.path()))
            .list_events()
            .unwrap()
            .into_iter()
            .find(|event| event.event_type == EventType::WorkObjectProposed)
            .unwrap();
        record_event_signature(EventSignatureRecordOptions::new(
            repo.path(),
            target.event_id.clone(),
            signer.clone(),
        ))
        .unwrap();
        let carrier = carrier_in(repo.path());
        (target, carrier)
    }

    #[test]
    fn divergent_signature_ingest_transcribes_incoming_attestation_to_a_cosignature() {
        let (base, _fixture, actor) = signed_captured_event();
        let signer_a = DeterministicSigner::from_seed([41u8; 32]);
        let signer_b = DeterministicSigner::from_seed([42u8; 32]);
        let copy_a = signed_copy(&base, &signer_a);
        let copy_b = signed_copy(&base, &signer_b);
        let trust = two_signer_trust(&actor, &signer_a, &signer_b);
        let dest = dest_repo();

        ingest_events(
            IngestEventsOptions::new(dest.path(), vec![copy_a.clone()])
                .with_trust_set(trust.clone()),
        )
        .unwrap();
        // The ingest path holds NO signer for B — transcription works purely from
        // the received inline signature (transcription, never minting).
        let result = ingest_events(
            IngestEventsOptions::new(dest.path(), vec![copy_b.clone()]).with_trust_set(trust),
        )
        .unwrap();

        assert_eq!(result.events_created, 1, "the transcribed carrier");
        assert_eq!(result.events_existing, 1, "the divergent original kept");
        assert_eq!(result.events_created_by_type["event_signature_recorded"], 1);

        let carrier = carrier_in(dest.path());
        let payload = carrier_payload(&carrier);
        assert_eq!(payload.attesting_signer, *signer_b.signer_id());
        assert_eq!(
            payload.attestation.sig.as_str(),
            copy_b.signature.as_ref().unwrap().sig.as_str(),
            "the carrier transcribes the received signature byte-for-byte, never re-signs"
        );

        // The first-stored A-signed event is kept as the stored target.
        let stored = EventStore::open(resolved_store_dir(dest.path()))
            .list_events()
            .unwrap();
        let stored_target = stored
            .iter()
            .find(|event| event.event_type == EventType::WorkObjectProposed)
            .unwrap();
        assert_eq!(stored_target.signer.as_ref().unwrap(), signer_a.signer_id());
    }

    #[test]
    fn divergent_signature_ingest_emits_no_divergence_diagnostic() {
        let (base, _fixture, actor) = signed_captured_event();
        let signer_a = DeterministicSigner::from_seed([43u8; 32]);
        let signer_b = DeterministicSigner::from_seed([44u8; 32]);
        let trust = two_signer_trust(&actor, &signer_a, &signer_b);
        let dest = dest_repo();

        ingest_events(
            IngestEventsOptions::new(dest.path(), vec![signed_copy(&base, &signer_a)])
                .with_trust_set(trust.clone()),
        )
        .unwrap();
        let result = ingest_events(
            IngestEventsOptions::new(dest.path(), vec![signed_copy(&base, &signer_b)])
                .with_trust_set(trust),
        )
        .unwrap();

        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "divergent_signature_existing_event"),
            "the divergence signal is retired"
        );
        // Both signers trusted → a silent reconciliation, no seam diagnostic at all.
        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "cosignature_untrusted_signer")
        );
        assert_eq!(
            carrier_payload(&carrier_in(dest.path())).attesting_signer,
            *signer_b.signer_id()
        );
    }

    #[test]
    fn untrusted_merged_cosigner_yields_exactly_one_authorization_diagnostic() {
        let (base, _fixture, actor) = signed_captured_event();
        let signer_a = DeterministicSigner::from_seed([45u8; 32]);
        let signer_b = DeterministicSigner::from_seed([46u8; 32]);
        // Only A is trusted; B's merged co-signature is real but unauthorized here.
        let trust = trust_for_actor(&actor, &signer_a);
        let dest = dest_repo();

        ingest_events(
            IngestEventsOptions::new(dest.path(), vec![signed_copy(&base, &signer_a)])
                .with_trust_set(trust.clone()),
        )
        .unwrap();
        let result = ingest_events(
            IngestEventsOptions::new(dest.path(), vec![signed_copy(&base, &signer_b)])
                .with_trust_set(trust),
        )
        .unwrap();

        let authorization: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "cosignature_untrusted_signer")
            .collect();
        assert_eq!(authorization.len(), 1);
        let message = &authorization[0].message;
        assert!(message.contains(signer_b.signer_id().as_str()));
        assert!(message.contains(actor.as_str()));
        assert!(!message.contains("disagree"));
        assert!(!message.contains("kept the first stored"));
        // The untrusted_key carrier is still stored (kept, not dropped).
        assert_eq!(result.events_created, 1);
        assert_eq!(
            carrier_payload(&carrier_in(dest.path())).attesting_signer,
            *signer_b.signer_id()
        );
    }

    #[test]
    fn reingest_divergent_signature_is_idempotent_no_repeat_diagnostic() {
        let (base, _fixture, actor) = signed_captured_event();
        let signer_a = DeterministicSigner::from_seed([47u8; 32]);
        let signer_b = DeterministicSigner::from_seed([48u8; 32]);
        let trust = trust_for_actor(&actor, &signer_a); // B stays untrusted
        let copy_b = signed_copy(&base, &signer_b);
        let dest = dest_repo();

        ingest_events(
            IngestEventsOptions::new(dest.path(), vec![signed_copy(&base, &signer_a)])
                .with_trust_set(trust.clone()),
        )
        .unwrap();
        ingest_events(
            IngestEventsOptions::new(dest.path(), vec![copy_b.clone()])
                .with_trust_set(trust.clone()),
        )
        .unwrap();
        // Second pass of the same divergent event: no new merge.
        let again = ingest_events(
            IngestEventsOptions::new(dest.path(), vec![copy_b]).with_trust_set(trust),
        )
        .unwrap();

        assert_eq!(again.events_created, 0, "no new carrier on re-ingest");
        assert!(
            again
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "cosignature_untrusted_signer"),
            "the authorization observation describes a merge that did not recur"
        );
        let carriers = EventStore::open(resolved_store_dir(dest.path()))
            .list_events()
            .unwrap()
            .into_iter()
            .filter(|event| event.event_type == EventType::EventSignatureRecorded)
            .count();
        assert_eq!(carriers, 1, "exactly one transcribed carrier");
    }

    #[test]
    fn invalid_incoming_inline_attestation_is_not_transcribed() {
        let (base, _fixture, actor) = signed_captured_event();
        let signer_a = DeterministicSigner::from_seed([49u8; 32]);
        let signer_b = DeterministicSigner::from_seed([50u8; 32]);
        let trust = two_signer_trust(&actor, &signer_a, &signer_b);
        let dest = dest_repo();

        ingest_events(
            IngestEventsOptions::new(dest.path(), vec![signed_copy(&base, &signer_a)])
                .with_trust_set(trust.clone()),
        )
        .unwrap();

        // A divergent event whose inline attestation is cryptographically invalid.
        let mut invalid = signed_copy(&base, &signer_b);
        invalid.signature = Some(EventSignature::ed25519_v1(EventSignatureBytes::from_bytes(
            &[0u8; 64],
        )));
        let result = ingest_events(
            IngestEventsOptions::new(dest.path(), vec![invalid]).with_trust_set(trust),
        )
        .unwrap();

        assert_eq!(
            result.events_created, 0,
            "invalid inline sig is not transcribed"
        );
        let carriers = EventStore::open(resolved_store_dir(dest.path()))
            .list_events()
            .unwrap()
            .into_iter()
            .filter(|event| event.event_type == EventType::EventSignatureRecorded)
            .count();
        assert_eq!(carriers, 0, "the set stays single-member");
    }

    #[test]
    fn signed_local_then_unsigned_peer_duplicate_neither_errors_nor_transcribes() {
        // A signed local event and an unsigned peer duplicate of the same fact have
        // matching eventRecordHash but a divergent binding — so the divergent arm
        // fires. The unsigned duplicate carries no attestation to transcribe, so it
        // must be a clean no-op, never an error that fails the whole ingest batch.
        let (base, _fixture, actor) = signed_captured_event();
        let signer_a = DeterministicSigner::from_seed([60u8; 32]);
        let trust = trust_for_actor(&actor, &signer_a);
        let dest = dest_repo();

        ingest_events(
            IngestEventsOptions::new(dest.path(), vec![signed_copy(&base, &signer_a)])
                .with_trust_set(trust.clone()),
        )
        .unwrap();

        let mut unsigned = base.clone();
        unsigned.signer = None;
        unsigned.signature = None;
        unsigned.ingest = None;
        let result = ingest_events(
            IngestEventsOptions::new(dest.path(), vec![unsigned]).with_trust_set(trust),
        )
        .unwrap();

        assert_eq!(
            result.events_created, 0,
            "an unsigned duplicate adds no co-signature"
        );
        let carriers = EventStore::open(resolved_store_dir(dest.path()))
            .list_events()
            .unwrap()
            .into_iter()
            .filter(|event| event.event_type == EventType::EventSignatureRecorded)
            .count();
        assert_eq!(carriers, 0);
    }

    #[test]
    fn self_certifying_divergent_signature_transcribes_via_effective_signer() {
        // A self-certifying signed event omits the top-level `signer` (its signer is
        // the did:key in actor_id). Transcription must resolve the EFFECTIVE signer,
        // not read `event.signer` directly, or the attestation is dropped.
        let (base, _fixture, _actor) = signed_captured_event();
        let signer = DeterministicSigner::from_seed([63u8; 32]);
        let did_key = signer.signer_id().clone();

        let mut signed = base.clone();
        signed.writer.actor_id = ActorId::new(did_key.as_str());
        signed.signer = None;
        signed.signature = None;
        let tbs = EventToBeSigned::from_event(&signed, &did_key).unwrap();
        let pae = event_signature_pre_authentication_encoding(&tbs).unwrap();
        signed.signature = Some(EventSignature::ed25519_v1(
            signer.sign_event_message(&pae).unwrap(),
        ));

        // The stored copy is the same fact, unsigned (same actor → same eventRecordHash).
        let mut unsigned = signed.clone();
        unsigned.signature = None;

        let dest = dest_repo();
        ingest_events(IngestEventsOptions::new(dest.path(), vec![unsigned])).unwrap();
        let result = ingest_events(IngestEventsOptions::new(dest.path(), vec![signed])).unwrap();

        assert_eq!(
            result.events_created, 1,
            "the self-certifying attestation transcribes via the effective signer"
        );
        assert_eq!(
            carrier_payload(&carrier_in(dest.path())).attesting_signer,
            did_key
        );
    }

    #[test]
    fn two_mirrors_converge_to_the_same_cosignature_signer_set() {
        let (base, _fixture, actor) = signed_captured_event();
        let signer_a = DeterministicSigner::from_seed([51u8; 32]);
        let signer_b = DeterministicSigner::from_seed([52u8; 32]);
        let trust = two_signer_trust(&actor, &signer_a, &signer_b);
        let copy_a = signed_copy(&base, &signer_a);
        let copy_b = signed_copy(&base, &signer_b);

        let cosigner_set = |repo: &Path| {
            let stored = EventStore::open(resolved_store_dir(repo))
                .list_events()
                .unwrap();
            let mut signers: Vec<String> = Vec::new();
            for event in &stored {
                if event.event_type == EventType::WorkObjectProposed
                    && let Some(signer) = &event.signer
                {
                    signers.push(signer.as_str().to_owned());
                }
                if event.event_type == EventType::EventSignatureRecorded {
                    signers.push(carrier_payload(event).attesting_signer.as_str().to_owned());
                }
            }
            signers.sort();
            signers
        };

        // Mirror 1: first-stored A, then ingested B.
        let mirror1 = dest_repo();
        ingest_events(
            IngestEventsOptions::new(mirror1.path(), vec![copy_a.clone()])
                .with_trust_set(trust.clone()),
        )
        .unwrap();
        ingest_events(
            IngestEventsOptions::new(mirror1.path(), vec![copy_b.clone()])
                .with_trust_set(trust.clone()),
        )
        .unwrap();

        // Mirror 2: first-stored B, then ingested A.
        let mirror2 = dest_repo();
        ingest_events(
            IngestEventsOptions::new(mirror2.path(), vec![copy_b]).with_trust_set(trust.clone()),
        )
        .unwrap();
        ingest_events(IngestEventsOptions::new(mirror2.path(), vec![copy_a]).with_trust_set(trust))
            .unwrap();

        let expected = {
            let mut both = vec![
                signer_a.signer_id().as_str().to_owned(),
                signer_b.signer_id().as_str().to_owned(),
            ];
            both.sort();
            both
        };
        assert_eq!(cosigner_set(mirror1.path()), expected);
        assert_eq!(
            cosigner_set(mirror2.path()),
            expected,
            "no winner-selection"
        );
    }

    #[test]
    fn peer_valid_detached_carrier_ingests_and_joins_the_set() {
        let signer = DeterministicSigner::from_seed([53u8; 32]);
        let actor = ActorId::new("actor:git-email:alice@example.com");
        let (target, carrier) = peer_target_and_carrier(&signer, &actor);
        let trust = trust_for_actor(&actor, &signer);
        let dest = dest_repo();

        ingest_events(
            IngestEventsOptions::new(dest.path(), vec![target]).with_trust_set(trust.clone()),
        )
        .unwrap();
        let before = EventStore::open(resolved_store_dir(dest.path()))
            .list_events()
            .unwrap();
        let before_hash = event_set_hash_for_events(&before).unwrap();

        let result = ingest_events(
            IngestEventsOptions::new(dest.path(), vec![carrier.clone()]).with_trust_set(trust),
        )
        .unwrap();

        assert_eq!(result.events_created, 1);
        assert_eq!(result.events_created_by_type["event_signature_recorded"], 1);
        let verification = result
            .verification
            .iter()
            .find(|entry| entry.event_id == carrier.event_id)
            .expect("the carrier's embedded-attestation status is reported");
        assert_eq!(verification.status, EventVerificationStatus::Valid);

        // The carrier rides the ordinary, signature-blind event-set machinery.
        let after = EventStore::open(resolved_store_dir(dest.path()))
            .list_events()
            .unwrap();
        assert_ne!(before_hash, event_set_hash_for_events(&after).unwrap());
        assert!(after.iter().any(|event| event.event_id == carrier.event_id));
    }

    #[test]
    fn peer_untrusted_detached_carrier_is_stored_and_flagged() {
        let signer = DeterministicSigner::from_seed([54u8; 32]);
        let actor = ActorId::new("actor:git-email:alice@example.com");
        let (target, carrier) = peer_target_and_carrier(&signer, &actor);
        let dest = dest_repo();

        // Target ingested with no trust set; the carrier's signer is untrusted here.
        ingest_events(IngestEventsOptions::new(dest.path(), vec![target])).unwrap();
        let result =
            ingest_events(IngestEventsOptions::new(dest.path(), vec![carrier.clone()])).unwrap();

        assert_eq!(
            result.events_created, 1,
            "untrusted_key is kept, not dropped"
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "cosignature_untrusted_signer")
        );
        assert!(
            EventStore::open(resolved_store_dir(dest.path()))
                .list_events()
                .unwrap()
                .iter()
                .any(|event| event.event_id == carrier.event_id)
        );
    }

    #[test]
    fn peer_invalid_detached_carrier_is_dropped_even_under_advisory_policy() {
        let signer = DeterministicSigner::from_seed([55u8; 32]);
        let actor = ActorId::new("actor:git-email:alice@example.com");
        let (target, carrier) = peer_target_and_carrier(&signer, &actor);
        let trust = trust_for_actor(&actor, &signer);
        let dest = dest_repo();

        ingest_events(
            IngestEventsOptions::new(dest.path(), vec![target]).with_trust_set(trust.clone()),
        )
        .unwrap();

        let mut tampered = carrier.clone();
        tampered.payload["attestation"]["sig"] =
            json!(EventSignatureBytes::from_bytes(&[0u8; 64]).as_str());
        tampered.payload_hash = sha256_json_prefixed(&tampered.payload).unwrap();

        // Even the advisory policy (which keeps an invalid INLINE signature) must
        // drop an invalid DETACHED carrier — the family rule overrides it.
        let result = ingest_events(
            IngestEventsOptions::new(dest.path(), vec![tampered.clone()])
                .with_verification_policy(EventVerificationPolicy::advisory())
                .with_trust_set(trust),
        )
        .unwrap();

        assert_eq!(result.events_created, 0);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "cosignature_invalid")
        );
        assert!(
            EventStore::open(resolved_store_dir(dest.path()))
                .list_events()
                .unwrap()
                .iter()
                .all(|event| event.event_id != tampered.event_id),
            "the invalid carrier was not stored"
        );
    }

    #[test]
    fn target_absent_detached_carrier_pends_then_stores_after_backfill() {
        let signer = DeterministicSigner::from_seed([56u8; 32]);
        let actor = ActorId::new("actor:git-email:alice@example.com");
        let (target, carrier) = peer_target_and_carrier(&signer, &actor);
        let trust = trust_for_actor(&actor, &signer);
        let dest = dest_repo();

        // Carrier arrives before its target: rejected, no trace.
        let pending = ingest_events(
            IngestEventsOptions::new(dest.path(), vec![carrier.clone()])
                .with_trust_set(trust.clone()),
        )
        .unwrap();
        assert_eq!(pending.events_created, 0);
        assert!(pending.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "cosignature_target_pending"
                && diagnostic
                    .message
                    .contains(carrier_payload(&carrier).target_event_id.as_str())
        }));
        assert_eq!(
            stored_event_count(dest.path()),
            0,
            "no marker, no queue, no trace"
        );

        // Backfill the target, then re-offer the SAME carrier: it stores cleanly,
        // proving the reject left no poisoning trace (replay-safe, no scheduler).
        ingest_events(
            IngestEventsOptions::new(dest.path(), vec![target]).with_trust_set(trust.clone()),
        )
        .unwrap();
        let replayed = ingest_events(
            IngestEventsOptions::new(dest.path(), vec![carrier.clone()]).with_trust_set(trust),
        )
        .unwrap();
        assert_eq!(replayed.events_created, 1);
        assert!(
            EventStore::open(resolved_store_dir(dest.path()))
                .list_events()
                .unwrap()
                .iter()
                .any(|event| event.event_id == carrier.event_id)
        );
    }

    #[test]
    fn idempotent_reingest_of_detached_carrier_is_existing() {
        let signer = DeterministicSigner::from_seed([57u8; 32]);
        let actor = ActorId::new("actor:git-email:alice@example.com");
        let (target, carrier) = peer_target_and_carrier(&signer, &actor);
        let trust = trust_for_actor(&actor, &signer);
        let dest = dest_repo();

        ingest_events(
            IngestEventsOptions::new(dest.path(), vec![target]).with_trust_set(trust.clone()),
        )
        .unwrap();
        ingest_events(
            IngestEventsOptions::new(dest.path(), vec![carrier.clone()])
                .with_trust_set(trust.clone()),
        )
        .unwrap();
        let again = ingest_events(
            IngestEventsOptions::new(dest.path(), vec![carrier]).with_trust_set(trust),
        )
        .unwrap();

        assert_eq!(again.events_created, 0);
        assert_eq!(again.events_existing, 1);
    }

    #[test]
    fn ingest_treats_timestamp_only_unsigned_replay_as_existing_without_signature_diagnostic() {
        let first = unsigned_event();
        let mut second = first.clone();
        second.occurred_at = "2026-06-04T00:00:00Z".to_owned();
        let dest = dest_repo();

        let first_result =
            ingest_events(IngestEventsOptions::new(dest.path(), vec![first.clone()])).unwrap();
        assert_eq!(first_result.events_created, 1);

        let second_result =
            ingest_events(IngestEventsOptions::new(dest.path(), vec![second])).unwrap();

        assert_eq!(second_result.events_created, 0);
        assert_eq!(second_result.events_existing, 1);
        assert!(
            second_result
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "divergent_signature_existing_event")
        );
        let mut stored = EventStore::open(resolved_store_dir(dest.path()))
            .list_events()
            .unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(
            stored[0].ingest.as_ref().unwrap().via,
            IngestVia::IngestEvents
        );
        stored[0].ingest = None;
        assert_eq!(stored, vec![first]);
    }

    #[test]
    fn ingest_stamps_ingest_provenance_on_every_written_event() {
        let (_origin, events) = origin_events();
        let dest = dest_repo();

        ingest_events(IngestEventsOptions::new(dest.path(), events.clone())).unwrap();

        let stored = EventStore::open(resolved_store_dir(dest.path()))
            .list_events()
            .unwrap();
        assert_eq!(stored.len(), events.len());
        for event in &stored {
            let stamp = event
                .ingest
                .as_ref()
                .expect("every ingested event is stamped");
            assert_eq!(stamp.via, IngestVia::IngestEvents);
            assert!(stamp.received_at.starts_with("unix-ms:"));
        }
    }

    #[test]
    fn ingest_overwrites_inbound_ingest_stamp_with_local_stamp() {
        // A stamp in arriving bytes is some other store's bookkeeping; only the
        // local importer's stamp means anything here (ADR-0009).
        let mut event = unsigned_event();
        event.ingest = Some(IngestProvenance {
            via: IngestVia::BundleApply,
            received_at: "unix-ms:1".to_owned(),
        });
        let dest = dest_repo();

        import_event(ImportEventOptions::new(dest.path(), event)).unwrap();

        let stored = EventStore::open(resolved_store_dir(dest.path()))
            .list_events()
            .unwrap();
        let stamp = stored[0].ingest.as_ref().unwrap();
        assert_eq!(stamp.via, IngestVia::IngestEvents);
        assert_ne!(stamp.received_at, "unix-ms:1");
    }

    #[test]
    fn reingest_of_locally_authored_event_leaves_stored_event_unstamped() {
        // Author locally, then ingest the store's own events back into it:
        // Existing outcome, first-stored-wins, the stored files stay unstamped —
        // a locally authored event can never acquire a stamp after the fact.
        let (origin, events) = origin_events();

        let result =
            ingest_events(IngestEventsOptions::new(origin.path(), events.clone())).unwrap();
        assert_eq!(result.events_created, 0);
        assert_eq!(result.events_existing, events.len());

        let stored = EventStore::open(resolved_store_dir(origin.path()))
            .list_events()
            .unwrap();
        assert!(stored.iter().all(|event| event.ingest.is_none()));
    }

    #[test]
    fn reingest_of_ingested_event_keeps_first_stamp() {
        // Ingest twice into the same destination; the stored stamp does not
        // change on the second pass — an ingested event can never lose (or
        // churn) its stamp.
        let (_origin, events) = origin_events();
        let dest = dest_repo();

        ingest_events(IngestEventsOptions::new(dest.path(), events.clone())).unwrap();
        let first_stamps: Vec<_> = EventStore::open(resolved_store_dir(dest.path()))
            .list_events()
            .unwrap()
            .into_iter()
            .map(|event| event.ingest)
            .collect();
        assert!(first_stamps.iter().all(Option::is_some));

        let second = ingest_events(IngestEventsOptions::new(dest.path(), events)).unwrap();
        assert_eq!(second.events_created, 0);

        let second_stamps: Vec<_> = EventStore::open(resolved_store_dir(dest.path()))
            .list_events()
            .unwrap()
            .into_iter()
            .map(|event| event.ingest)
            .collect();
        assert_eq!(second_stamps, first_stamps);
    }

    #[test]
    fn stamped_signed_event_still_verifies_valid_after_ingest() {
        let (event, signer, actor) = signed_captured_event();
        let trust = trust_for_actor(&actor, &signer);
        let dest = dest_repo();

        ingest_events(
            IngestEventsOptions::new(dest.path(), vec![event]).with_trust_set(trust.clone()),
        )
        .unwrap();

        let stored = EventStore::open(resolved_store_dir(dest.path()))
            .list_events()
            .unwrap();
        assert!(stored[0].ingest.is_some());
        assert_eq!(
            verify_event_signature(&stored[0], &trust).unwrap(),
            EventVerificationStatus::Valid
        );
    }

    #[test]
    fn ingest_events_reconstructs_projection_and_is_idempotent() {
        let (_origin, events) = origin_events();
        let total = events.len();
        assert!(
            total >= 3,
            "expected captured + opened + responded, got {total}"
        );
        let dest = dest_repo();

        let first = ingest_events(IngestEventsOptions::new(dest.path(), events.clone())).unwrap();
        assert_eq!(first.events_created, total);
        assert_eq!(first.events_existing, 0);

        // The forwarded responded input request is visible in the destination.
        let listed = list_input_requests(
            InputRequestListOptions::new(dest.path()).with_status(InputRequestStatusFilter::All),
        )
        .unwrap();
        assert_eq!(listed.input_requests.len(), 1);
        assert_eq!(
            listed.input_requests[0].status,
            InputRequestStatus::Responded
        );
        // The forwarded actor attribution is preserved through ingest.
        assert_eq!(
            listed.input_requests[0].responses[0]
                .writer
                .actor_id
                .as_str(),
            "actor:agent:remote-reviewer"
        );

        // Projection equals a full replay, and re-ingest is a no-op.
        assert_eq!(on_disk_state(dest.path()), replayed_state(dest.path()));
        let second = ingest_events(IngestEventsOptions::new(dest.path(), events)).unwrap();
        assert_eq!(second.events_created, 0);
        assert_eq!(second.events_existing, total);
    }

    #[test]
    fn import_event_records_a_single_event() {
        let (_origin, events) = origin_events();
        let captured = events
            .iter()
            .find(|event| event.event_type == EventType::WorkObjectProposed)
            .unwrap()
            .clone();
        let dest = dest_repo();

        let result = import_event(ImportEventOptions::new(dest.path(), captured.clone())).unwrap();
        assert_eq!(result.events_created, 1);
        assert_eq!(result.events_created_by_type["work_object_proposed"], 1);

        let again = import_event(ImportEventOptions::new(dest.path(), captured)).unwrap();
        assert_eq!(again.events_created, 0);
        assert_eq!(again.events_existing, 1);
    }

    #[test]
    fn ingest_rejects_malformed_writer_actor_id() {
        let (_origin, events) = origin_events();
        let mut bad = events[0].clone();
        bad.writer.actor_id = ActorId::new("not-an-actor-id");
        let dest = dest_repo();

        let error = import_event(ImportEventOptions::new(dest.path(), bad)).unwrap_err();
        assert!(
            error.to_string().contains("malformed writer actor id"),
            "unexpected error: {error}"
        );
        // Nothing was written (attribution is validated before any write).
        assert!(
            !resolved_store_dir(dest.path()).join("events").exists() || {
                EventStore::open(resolved_store_dir(dest.path()))
                    .list_events()
                    .unwrap()
                    .is_empty()
            }
        );
    }

    #[test]
    fn ingest_conflict_mid_batch_keeps_projection_consistent_with_disk() {
        let (_origin, events) = origin_events();
        let opened = events
            .iter()
            .find(|event| event.event_type == EventType::InputRequestOpened)
            .unwrap()
            .clone();
        // A conflicting event: same idempotency key (and eventId) but a different
        // payload under a recomputed, self-consistent payload hash.
        let mut conflict = opened.clone();
        let mut payload = conflict.payload.clone();
        payload["title"] = serde_json::json!("a different title");
        conflict.payload = payload;
        conflict.payload_hash =
            crate::canonical_hash::sha256_json_prefixed(&conflict.payload).unwrap();

        let captured = events
            .iter()
            .find(|event| event.event_type == EventType::WorkObjectProposed)
            .unwrap()
            .clone();

        let dest = dest_repo();
        let batch = vec![captured, opened, conflict];
        let error = ingest_events(IngestEventsOptions::new(dest.path(), batch)).unwrap_err();
        assert!(
            error.to_string().contains("conflict"),
            "unexpected error: {error}"
        );

        // The good events are durable and the projection matches the event log on disk.
        assert_eq!(on_disk_state(dest.path()), replayed_state(dest.path()));
    }

    // -- end-to-end: ingest/bundle -> resumption binding (ADR-0009) ----------
    //
    // The relay consequence is implicit throughout: a relay that strips
    // signatures converts a bindable response into ingested_unsigned.

    /// Task-shaped origin event set: attempt + operative task-targeted input
    /// request + operative Approved response (last element).
    fn task_resumption_events() -> (Vec<ShoreEvent>, WorkObjectId) {
        let task_attempt_id = WorkObjectId::new("task-attempt:sha256:ta");
        let session_id = JournalId::new("journal:claude:uuid-1");
        let input_request_id = InputRequestId::new("input-request:sha256:1");
        let response_id = InputRequestResponseId::new("input-request-response:sha256:r");

        let events = vec![
            task_attempt_event(
                &task_attempt_id,
                &session_id,
                "uuid-1",
                "2026-05-18T00:00:00Z",
            ),
            task_input_request_event_with_target(
                &task_attempt_id,
                &session_id,
                &input_request_id,
                "source:approve",
                "2026-05-18T00:00:02Z",
                TargetRef::Task(TaskTargetRef::TaskAttempt {
                    task_attempt_id: task_attempt_id.clone(),
                }),
                "needs approval",
            ),
            user_response_event(
                &input_request_id,
                &response_id,
                InputRequestResponseOutcome::Approved,
                AssertionMode::Operative,
                "2026-05-18T00:00:03Z",
            ),
        ];
        (events, task_attempt_id)
    }

    /// Writes the events as local-authored facts — the same store primitive
    /// the domain workflows and the adapter write path use; no seam, no stamp.
    fn local_authored_store(events: &[ShoreEvent]) -> (tempfile::TempDir, EventStore) {
        let root = tempfile::tempdir().unwrap();
        let store = EventStore::open(root.path().join(".shore/data"));
        for event in events {
            store.record_event_once(event).unwrap();
        }
        (root, store)
    }

    fn resumption_projection(
        stored: &[ShoreEvent],
        task_attempt_id: &WorkObjectId,
        trust: &TrustSet,
        policy: ResumptionBindingPolicy,
    ) -> AgentResumptionProjection {
        agent_resumption_from_events(stored, task_attempt_id, &reader_actor(), trust, policy)
            .unwrap()
    }

    fn identity_reason(projection: &AgentResumptionProjection) -> Option<String> {
        projection
            .diagnostics
            .iter()
            .find(|d| d.code == "agent_resumption_response_identity_not_binding")
            .and_then(|d| d.reason.clone())
    }

    #[test]
    fn local_unsigned_response_binds_in_its_own_store_zero_config() {
        // Possession is the trust root: a human responding in their own
        // worktree binds with zero keys, zero configuration.
        let (events, task_attempt_id) = task_resumption_events();
        let (_root, store) = local_authored_store(&events);

        let stored = store.list_events().unwrap();
        assert!(stored.iter().all(|event| event.ingest.is_none()));
        let projection = resumption_projection(
            &stored,
            &task_attempt_id,
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        );

        assert!(projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ready);
    }

    #[test]
    fn ingested_unsigned_response_is_blocked_ingested_unsigned_in_destination() {
        let (events, task_attempt_id) = task_resumption_events();
        let dest = dest_repo();

        ingest_events(IngestEventsOptions::new(dest.path(), events)).unwrap();

        let stored = EventStore::open(resolved_store_dir(dest.path()))
            .list_events()
            .unwrap();
        let projection = resumption_projection(
            &stored,
            &task_attempt_id,
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        );

        assert!(!projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Blocked);
        assert_eq!(
            identity_reason(&projection).as_deref(),
            Some("ingested_unsigned")
        );
        // The forwarded claimed actorId is preserved as a reported fact but
        // does not bind.
        let response_view = projection.selected_response.as_ref().unwrap();
        assert_eq!(
            response_view.envelope.writer.actor_id.as_str(),
            "actor:claude_code:user"
        );
        assert!(!response_view.identity_treated_as_binding);
    }

    #[test]
    fn ingested_reviewer_signed_response_binds_via_verified_signer_arm() {
        // The reviewer holds the key end-to-end: the response is signed
        // before it ever leaves the origin.
        let (mut events, task_attempt_id) = task_resumption_events();
        let signer = DeterministicSigner::fixture();
        crate::session::sign_event_if_requested(
            events.last_mut().expect("response event"),
            &crate::session::EventSigningOptions::sign_with(signer.clone()),
        )
        .unwrap();
        let trust = trust_for_actor(&ActorId::new("actor:claude_code:user"), &signer);
        let dest = dest_repo();

        ingest_events(IngestEventsOptions::new(dest.path(), events).with_trust_set(trust.clone()))
            .unwrap();

        let stored = EventStore::open(resolved_store_dir(dest.path()))
            .list_events()
            .unwrap();
        let response = stored
            .iter()
            .find(|event| event.event_type == EventType::InputRequestResponded)
            .unwrap();
        assert!(response.ingest.is_some(), "seam stamped the stored copy");
        let projection = resumption_projection(
            &stored,
            &task_attempt_id,
            &trust,
            ResumptionBindingPolicy::default(),
        );

        assert!(projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ready);
    }

    #[test]
    fn signature_stripped_in_transit_response_is_blocked_ingested_unsigned() {
        // The relay consequence: a hop that strips signatures converts a
        // bindable response into ingested_unsigned.
        let (mut events, task_attempt_id) = task_resumption_events();
        let signer = DeterministicSigner::fixture();
        let response = events.last_mut().expect("response event");
        crate::session::sign_event_if_requested(
            response,
            &crate::session::EventSigningOptions::sign_with(signer.clone()),
        )
        .unwrap();
        response.signer = None;
        response.signature = None;
        let trust = trust_for_actor(&ActorId::new("actor:claude_code:user"), &signer);
        let dest = dest_repo();

        ingest_events(IngestEventsOptions::new(dest.path(), events).with_trust_set(trust.clone()))
            .unwrap();

        let stored = EventStore::open(resolved_store_dir(dest.path()))
            .list_events()
            .unwrap();
        let projection = resumption_projection(
            &stored,
            &task_attempt_id,
            &trust,
            ResumptionBindingPolicy::default(),
        );

        assert!(!projection.may_resume);
        assert_eq!(
            identity_reason(&projection).as_deref(),
            Some("ingested_unsigned")
        );
    }

    #[test]
    fn bundle_applied_response_has_parity_with_ingest() {
        use crate::session::store::bundle::import_store_bundle;

        // Unsigned: bundle apply stamps, so the response stops binding.
        let (events, task_attempt_id) = task_resumption_events();
        let (_source_root, _source_store) = local_authored_store(&events);
        let target = tempfile::tempdir().unwrap();
        import_store_bundle(
            _source_root.path().join(".shore/data"),
            target.path().join(".shore/data"),
        )
        .unwrap();
        let stored = EventStore::open(target.path().join(".shore/data"))
            .list_events()
            .unwrap();
        let projection = resumption_projection(
            &stored,
            &task_attempt_id,
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        );
        assert!(!projection.may_resume);
        assert_eq!(
            identity_reason(&projection).as_deref(),
            Some("ingested_unsigned")
        );

        // Signed + authorized: binds via arm (b) through bundle apply too.
        let (mut signed_events, task_attempt_id) = task_resumption_events();
        let signer = DeterministicSigner::fixture();
        crate::session::sign_event_if_requested(
            signed_events.last_mut().expect("response event"),
            &crate::session::EventSigningOptions::sign_with(signer.clone()),
        )
        .unwrap();
        let trust = trust_for_actor(&ActorId::new("actor:claude_code:user"), &signer);
        let (signed_source_root, _signed_source_store) = local_authored_store(&signed_events);
        let signed_target = tempfile::tempdir().unwrap();
        import_store_bundle(
            signed_source_root.path().join(".shore/data"),
            signed_target.path().join(".shore/data"),
        )
        .unwrap();
        let stored = EventStore::open(signed_target.path().join(".shore/data"))
            .list_events()
            .unwrap();
        let projection = resumption_projection(
            &stored,
            &task_attempt_id,
            &trust,
            ResumptionBindingPolicy::default(),
        );
        assert!(projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ready);
    }

    #[test]
    fn verified_only_store_blocks_even_its_own_unsigned_response() {
        // Choosing verified-only is choosing that nothing binds without a
        // key — including the store's own unsigned responses.
        let (events, task_attempt_id) = task_resumption_events();
        let (_root, store) = local_authored_store(&events);

        let stored = store.list_events().unwrap();
        let projection = resumption_projection(
            &stored,
            &task_attempt_id,
            &TrustSet::default(),
            ResumptionBindingPolicy::VerifiedOnly,
        );

        assert!(!projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Blocked);
        assert_eq!(
            identity_reason(&projection).as_deref(),
            Some("policy_excludes_local")
        );
    }

    // ADR-0009 through the shared common-dir store: with one default store per
    // clone, every worktree resolves the SAME store with no `store link` step,
    // and the resumption-binding predicate is a pure function of the events
    // actually read — never of which worktree reads them. A response authored
    // directly in the shared store is local-authored and unstamped, so it binds
    // by possession from any sibling worktree. (The stamped/ingested_unsigned
    // outcome is exercised through the real `ingest_events` path elsewhere in
    // this module.) These fixtures extend the binding outcome matrix in
    // projection/task.rs through a real worktree pair and the shared read seam.

    /// A committed main repo plus seed and reader worktrees that share one
    /// common-dir store, with `events` written into that shared store via the
    /// seed's resolved write landing — no `store link`.
    fn shared_resumption_pair(
        events: &[ShoreEvent],
    ) -> (
        TestRepo,
        tempfile::TempDir,
        std::path::PathBuf,
        std::path::PathBuf,
    ) {
        let main = TestRepo::new();
        main.write("README.md", "base\n");
        main.commit_all("base");

        let parent = tempfile::tempdir().unwrap();
        let seed = parent.path().join("seed");
        let reader = parent.path().join("reader");
        main.git(["worktree", "add", "-b", "seed", seed.to_str().unwrap()]);
        main.git(["worktree", "add", "-b", "reader", reader.to_str().unwrap()]);

        // The seed writes into the store it shares with every sibling worktree
        // (the common-dir store), so the reader sees the same events with no
        // link step.
        let shared = resolve_store(&seed).unwrap();
        let shared_store = EventStore::open(shared.store_dir());
        for event in events {
            shared_store.record_event_once(event).unwrap();
        }
        (main, parent, seed, reader)
    }

    /// Events as a sibling worktree's reads see them: through the read seam,
    /// resolving the shared common-dir store with no link step.
    fn shared_store_events(repo: &Path) -> Vec<ShoreEvent> {
        let read_store = resolve_read_store(repo).unwrap();
        EventStore::open(read_store.store_dir())
            .list_events()
            .unwrap()
    }

    #[test]
    fn sibling_worktree_unsigned_response_binds_by_possession_from_shared_store() {
        // The shared common-dir store holds the seed's local-authored, unstamped
        // response. A sibling worktree reads it through the same store with no
        // link, and possession binds it identically.
        let (events, task_attempt_id) = task_resumption_events();
        let (_main, _parent, seed, reader) = shared_resumption_pair(&events);

        // Seed and reader resolve the same store, and its events are unstamped.
        assert_eq!(
            resolve_store(&seed).unwrap().store_dir(),
            resolve_store(&reader).unwrap().store_dir()
        );
        let stored = shared_store_events(&reader);
        assert!(stored.iter().all(|event| event.ingest.is_none()));
        let projection = resumption_projection(
            &stored,
            &task_attempt_id,
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        );
        assert!(projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ready);
    }

    #[test]
    fn sibling_worktree_signed_authorized_response_binds_via_verified_signer() {
        let (mut events, task_attempt_id) = task_resumption_events();
        let signer = DeterministicSigner::fixture();
        crate::session::sign_event_if_requested(
            events.last_mut().expect("response event"),
            &crate::session::EventSigningOptions::sign_with(signer.clone()),
        )
        .unwrap();
        let trust = trust_for_actor(&ActorId::new("actor:claude_code:user"), &signer);
        let (_main, _parent, _seed, reader) = shared_resumption_pair(&events);

        let stored = shared_store_events(&reader);
        let projection = resumption_projection(
            &stored,
            &task_attempt_id,
            &trust,
            ResumptionBindingPolicy::default(),
        );

        // Arm (b) verified-signer binds identically from any worktree's read.
        assert!(projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ready);
    }

    #[test]
    fn authoring_worktree_reads_its_own_unsigned_response_as_binding_from_shared_store() {
        // Post default-share: the author's reads resolve the same shared store it
        // wrote to, whose copy is local-authored and unstamped — so the author's
        // own unsigned response keeps binding by possession. There is no stamping
        // copy step to flip it to ingested_unsigned.
        let (events, task_attempt_id) = task_resumption_events();
        let (_main, _parent, seed, _reader) = shared_resumption_pair(&events);

        let stored = shared_store_events(&seed);
        assert!(stored.iter().all(|event| event.ingest.is_none()));
        let projection = resumption_projection(
            &stored,
            &task_attempt_id,
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        );
        assert!(projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ready);
    }

    // -- write-side companion: cross-worktree-AUTHORED response ----------------
    //
    // The cross-worktree READ of a response is covered above; this covers a
    // response *authored* in a sibling worktree against a request authored in
    // another. With one shared store, both worktrees write into and read from the
    // same common-dir store, so a response authored in the reader's worktree is
    // immediately visible — and unstamped — to every sibling. The binding
    // predicate is store-agnostic: possession binds the unstamped response.

    fn cross_worktree_response_pair(
        origin_events: &[ShoreEvent],
        response_event: &ShoreEvent,
    ) -> (
        TestRepo,
        tempfile::TempDir,
        std::path::PathBuf,
        std::path::PathBuf,
    ) {
        let main = TestRepo::new();
        main.write("README.md", "base\n");
        main.commit_all("base");

        let parent = tempfile::tempdir().unwrap();
        let seed = parent.path().join("seed");
        let reader = parent.path().join("reader");
        main.git(["worktree", "add", "-b", "seed", seed.to_str().unwrap()]);
        main.git(["worktree", "add", "-b", "reader", reader.to_str().unwrap()]);

        // The seed authors the task attempt + input request into the shared
        // common-dir store; the reader authors only the response into the same
        // shared store. No link step.
        let shared = resolve_store(&seed).unwrap();
        let seed_store = EventStore::open(shared.store_dir());
        for event in origin_events {
            seed_store.record_event_once(event).unwrap();
        }
        let reader_store = resolve_store(&reader).unwrap();
        EventStore::open(reader_store.store_dir())
            .record_event_once(response_event)
            .unwrap();
        (main, parent, seed, reader)
    }

    #[test]
    fn cross_worktree_unsigned_response_binds_by_possession_from_shared_store() {
        let (events, task_attempt_id) = task_resumption_events();
        let response = events.last().expect("response event").clone();
        let origin = &events[..events.len() - 1];
        let (_main, _parent, _seed, reader) = cross_worktree_response_pair(origin, &response);

        // The reader's response is in the shared store, unstamped, and visible
        // without any link step.
        let stored = shared_store_events(&reader);
        assert!(stored.iter().all(|event| event.ingest.is_none()));
        assert!(
            stored
                .iter()
                .any(|event| event.event_type == EventType::InputRequestResponded),
            "the cross-worktree response is visible in the shared store"
        );

        let projection = resumption_projection(
            &stored,
            &task_attempt_id,
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        );

        assert!(projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ready);
    }

    #[test]
    fn cross_worktree_signed_authorized_response_binds_from_shared_store() {
        let (mut events, task_attempt_id) = task_resumption_events();
        let signer = DeterministicSigner::fixture();
        crate::session::sign_event_if_requested(
            events.last_mut().expect("response event"),
            &crate::session::EventSigningOptions::sign_with(signer.clone()),
        )
        .unwrap();
        let trust = trust_for_actor(&ActorId::new("actor:claude_code:user"), &signer);
        let response = events.last().expect("response event").clone();
        let origin = &events[..events.len() - 1];
        let (_main, _parent, _seed, reader) = cross_worktree_response_pair(origin, &response);

        let stored = shared_store_events(&reader);

        let projection = resumption_projection(
            &stored,
            &task_attempt_id,
            &trust,
            ResumptionBindingPolicy::default(),
        );

        // Arm (b) verified-signer binds identically from the shared store.
        assert!(projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ready);
    }

    #[test]
    fn cross_worktree_unsigned_response_is_immediately_visible_in_shared_store() {
        let (events, task_attempt_id) = task_resumption_events();
        let response = events.last().expect("response event").clone();
        let origin = &events[..events.len() - 1];
        let (_main, _parent, seed, reader) = cross_worktree_response_pair(origin, &response);

        // Seed and reader resolve the same store; the response authored in the
        // reader is visible from the seed's read with no link step.
        assert_eq!(
            resolve_store(&seed).unwrap().store_dir(),
            resolve_store(&reader).unwrap().store_dir()
        );
        let from_seed = shared_store_events(&seed);
        assert!(
            from_seed
                .iter()
                .find(|event| event.event_type == EventType::InputRequestResponded)
                .expect("the cross-worktree response is in the shared store")
                .ingest
                .is_none(),
            "the cross-worktree response is local-authored and unstamped"
        );

        let projection = resumption_projection(
            &from_seed,
            &task_attempt_id,
            &TrustSet::default(),
            ResumptionBindingPolicy::default(),
        );

        // The unstamped response binds via arm (a) possession from any sibling.
        assert!(projection.may_resume);
        assert_eq!(projection.state, AgentResumptionState::Ready);
    }
    #[test]
    fn strict_rejection_is_classifiable_without_message_parsing() {
        let (untrusted, _signer, _actor) = signed_captured_event();
        let expected_id = untrusted.event_id.clone();
        let dest = dest_repo();

        let error = ingest_events(
            IngestEventsOptions::new(dest.path(), vec![untrusted])
                .with_verification_policy(EventVerificationPolicy::trusted_strict())
                .with_trust_set(TrustSet::default()),
        )
        .unwrap_err();

        let rendered = error.to_string();
        match &error {
            ShoreError::EventVerificationRejected { event_id, status } => {
                assert_eq!(*event_id, expected_id);
                assert_eq!(*status, EventVerificationStatus::UntrustedKey);
            }
            other => panic!("expected EventVerificationRejected, got {other:?}"),
        }
        assert_eq!(
            rendered,
            format!(
                "event signature verification rejected event {} with status untrusted_key",
                expected_id.as_str()
            )
        );
        assert_eq!(stored_event_count(dest.path()), 0);
    }

    #[test]
    fn import_event_rejection_is_classifiable_without_message_parsing() {
        let unsigned = unsigned_event();
        let expected_id = unsigned.event_id.clone();
        let dest = dest_repo();

        let error = import_event(
            ImportEventOptions::new(dest.path(), unsigned)
                .with_verification_policy(EventVerificationPolicy::trusted_strict()),
        )
        .unwrap_err();

        match &error {
            ShoreError::EventVerificationRejected { event_id, status } => {
                assert_eq!(*event_id, expected_id);
                assert_eq!(*status, EventVerificationStatus::Unsigned);
            }
            other => panic!("expected EventVerificationRejected, got {other:?}"),
        }
        assert_eq!(stored_event_count(dest.path()), 0);
    }
    #[test]
    fn ingest_reports_per_event_write_outcomes_in_input_order() {
        let (_origin, events) = origin_events();
        let expected_ids: Vec<_> = events.iter().map(|event| event.event_id.clone()).collect();
        let dest = dest_repo();

        let first = ingest_events(IngestEventsOptions::new(dest.path(), events.clone())).unwrap();
        assert_eq!(first.verification.len(), expected_ids.len());
        for (row, expected_id) in first.verification.iter().zip(&expected_ids) {
            assert_eq!(row.event_id, *expected_id, "rows stay in input order");
            assert_eq!(row.write_outcome, Some(EventWriteOutcome::Created));
        }

        let again = ingest_events(IngestEventsOptions::new(dest.path(), events)).unwrap();
        for row in &again.verification {
            assert_eq!(row.write_outcome, Some(EventWriteOutcome::Existing));
        }
    }

    #[test]
    fn duplicate_events_in_one_batch_stamp_rows_by_index() {
        // Two copies of the SAME event in one batch share an event_id; only index
        // matching assigns their outcomes correctly (id-matching would mis-assign).
        let (_origin, events) = origin_events();
        let event = events
            .into_iter()
            .find(|event| event.event_type == EventType::WorkObjectProposed)
            .unwrap();
        let dest = dest_repo();

        let result = ingest_events(IngestEventsOptions::new(
            dest.path(),
            vec![event.clone(), event],
        ))
        .unwrap();

        assert_eq!(result.verification.len(), 2);
        assert_eq!(
            result.verification[0].event_id,
            result.verification[1].event_id
        );
        assert_eq!(
            result.verification[0].write_outcome,
            Some(EventWriteOutcome::Created)
        );
        assert_eq!(
            result.verification[1].write_outcome,
            Some(EventWriteOutcome::Existing)
        );
    }

    #[test]
    fn divergent_signature_row_reports_existing_divergent_signature() {
        let (base, _fixture, actor) = signed_captured_event();
        let signer_a = DeterministicSigner::from_seed([61u8; 32]);
        let signer_b = DeterministicSigner::from_seed([62u8; 32]);
        let trust = two_signer_trust(&actor, &signer_a, &signer_b);
        let dest = dest_repo();

        ingest_events(
            IngestEventsOptions::new(dest.path(), vec![signed_copy(&base, &signer_a)])
                .with_trust_set(trust.clone()),
        )
        .unwrap();
        let result = ingest_events(
            IngestEventsOptions::new(dest.path(), vec![signed_copy(&base, &signer_b)])
                .with_trust_set(trust),
        )
        .unwrap();

        // The input event's row carries the divergence signal; the transcribed
        // carrier is a separate created event with no verification row.
        assert_eq!(
            result.verification.len(),
            1,
            "no row for the transcribed carrier"
        );
        assert_eq!(
            result.verification[0].write_outcome,
            Some(EventWriteOutcome::ExistingDivergentSignature)
        );
        assert_eq!(result.events_created, 1, "the transcribed carrier");
        assert_eq!(result.events_existing, 1, "the divergent original kept");
    }

    #[test]
    fn stored_detached_carrier_row_reports_its_write_outcome() {
        let signer = DeterministicSigner::fixture();
        let actor = ActorId::new("actor:git-email:alice@example.com");
        let (target, carrier) = peer_target_and_carrier(&signer, &actor);
        let trust = trust_for_actor(&actor, &signer);
        let dest = dest_repo();

        let result = ingest_events(
            IngestEventsOptions::new(dest.path(), vec![target.clone(), carrier.clone()])
                .with_trust_set(trust),
        )
        .unwrap();

        // Non-carrier rows first (input order), then the stored carrier's appended row.
        let target_row = result
            .verification
            .iter()
            .find(|row| row.event_id == target.event_id)
            .expect("target row");
        assert_eq!(target_row.write_outcome, Some(EventWriteOutcome::Created));
        let carrier_row = result
            .verification
            .iter()
            .find(|row| row.event_id == carrier.event_id)
            .expect("stored carrier row is appended");
        assert_eq!(carrier_row.write_outcome, Some(EventWriteOutcome::Created));
    }
}
