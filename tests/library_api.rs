//! End-to-end exercise of the supported in-process library API (see
//! `docs/library-api.md`): capture, an attributed write, typed reads, documented
//! JSON, and forwarding events into a second store — all without the CLI.

mod support;

use std::fs;
use std::path::Path;

use ed25519_dalek::{Signer as _, SigningKey};
use serde_json::Value;
use shoreline::crypto::{EventSignatureBytes, EventSigner, SignerId};
use shoreline::error::{Result, ShoreError};
use shoreline::model::ActorId;
use shoreline::session::event::{
    EventType, InputRequestReasonCode, InputRequestResponseOutcome, ReviewAssessment,
};
use shoreline::session::{
    ArtifactKind, ArtifactRef, AssessmentAddOptions, CaptureOptions, EventVerificationPolicy,
    EventVerificationStatus, EventWriteOutcome, ImportArtifactOptions, ImportArtifactOutcome,
    ImportNotesOptions, IngestEventsOptions, InputRequestFetchOptions, InputRequestListOptions,
    InputRequestOpenOptions, InputRequestRespondOptions, InputRequestStatus,
    InputRequestStatusFilter, ObservationAddOptions, ReloadOutcome, ReviewHistoryOptions,
    RevisionShowOptions, SensitivityKind, SensitivityPolicyOutcome, SensitivitySeverity, TrustSet,
    capture_worktree_review, event_signature_trust_set, export_artifact, fetch_input_request,
    import_artifact, import_notes, ingest_events, list_input_requests, open_input_request,
    read_events, record_observation, referenced_artifacts, respond_input_request, review_history,
    show_revision, verify_event_signature,
};
use support::git_repo::GitRepo;

fn modified_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 {\n    1\n}\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 {\n    2\n}\n");
    repo
}

fn large_body() -> String {
    "large body line\n".repeat(320)
}

fn origin_with_large_input_request() -> (GitRepo, Vec<shoreline::session::event::ShoreEvent>, String)
{
    let origin = modified_repo();
    capture_worktree_review(CaptureOptions::new(origin.path())).unwrap();
    let body = large_body();
    open_input_request(
        InputRequestOpenOptions::new(origin.path())
            .with_track("human:kevin")
            .with_title("Need large body review")
            .with_reason_code(InputRequestReasonCode::ManualDecisionRequired)
            .with_body(body.clone()),
    )
    .unwrap();
    let events = read_events(origin.path()).unwrap();
    (origin, events, body)
}

fn exported_artifacts(repo: &GitRepo, refs: &[ArtifactRef]) -> Vec<(ArtifactRef, Vec<u8>)> {
    refs.iter()
        .map(|artifact| {
            (
                artifact.clone(),
                export_artifact(repo.path(), artifact).unwrap(),
            )
        })
        .collect()
}

fn import_all_artifacts(repo: &GitRepo, artifacts: &[(ArtifactRef, Vec<u8>)]) {
    for (artifact, bytes) in artifacts {
        import_artifact(ImportArtifactOptions::new(
            repo.path(),
            artifact.clone(),
            bytes.clone(),
        ))
        .unwrap();
    }
}

fn remove_object_artifacts(repo: &Path) {
    let artifact_dir = support::common_dir_store(repo).join("artifacts/objects");
    for entry in fs::read_dir(artifact_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            fs::remove_file(path).unwrap();
        }
    }
}

#[derive(Clone)]
struct DeterministicSigner {
    signer_id: SignerId,
    signing_key: SigningKey,
}

impl DeterministicSigner {
    fn fixture() -> Self {
        let signing_key = SigningKey::from_bytes(&[
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b,
            0x1c, 0x1d, 0x1e, 0x1f,
        ]);
        let signer_id = SignerId::from_ed25519_public_key(signing_key.verifying_key().to_bytes());

        Self {
            signer_id,
            signing_key,
        }
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

fn trust_for_actor(actor: &str, signer: &DeterministicSigner) -> TrustSet {
    event_signature_trust_set(serde_json::json!({
        "allowedSigners": {
            actor: [signer.signer_id().as_str()]
        }
    }))
    .unwrap()
}

#[test]
fn respond_input_request_can_sign_with_explicit_signer() {
    let origin = modified_repo();
    let signer = DeterministicSigner::fixture();
    let actor = ActorId::new("actor:git-email:alice@example.com");

    capture_worktree_review(CaptureOptions::new(origin.path())).unwrap();
    let opened = open_input_request(
        InputRequestOpenOptions::new(origin.path())
            .with_track("human:kevin")
            .with_title("Need approval")
            .with_reason_code(InputRequestReasonCode::ManualDecisionRequired),
    )
    .unwrap();

    respond_input_request(
        InputRequestRespondOptions::new(origin.path(), opened.input_request_id.clone())
            .with_actor_id(actor.clone())
            .with_outcome(InputRequestResponseOutcome::Approved)
            .sign_with(signer.clone()),
    )
    .unwrap();

    let events = read_events(origin.path()).unwrap();
    let event = events
        .iter()
        .find(|event| event.event_type.as_str() == "input_request_responded")
        .expect("responded event is written");

    assert_eq!(event.writer.actor_id, actor);
    assert_eq!(event.signer.as_ref(), Some(signer.signer_id()));
    let signature = event.signature.as_ref().expect("event is signed");
    assert_eq!(signature.alg, "ed25519");
    assert_eq!(signature.sig_version, 1);
    assert_eq!(
        verify_event_signature(event, &trust_for_actor(actor.as_str(), &signer)).unwrap(),
        EventVerificationStatus::Valid
    );
}

#[test]
fn write_paths_remain_unsigned_by_default() {
    let origin = modified_repo();
    capture_worktree_review(CaptureOptions::new(origin.path())).unwrap();

    record_observation(
        ObservationAddOptions::new(origin.path())
            .with_track("human:kevin")
            .with_title("Unsigned by default"),
    )
    .unwrap();

    let events = read_events(origin.path()).unwrap();
    let event = events
        .iter()
        .find(|event| event.event_type.as_str() == "review_observation_recorded")
        .expect("observation event is written");

    assert!(event.signer.is_none());
    assert!(event.signature.is_none());
}

#[test]
fn review_history_can_include_verification_status_without_artifact_availability_confusion() {
    let origin = modified_repo();
    let signer = DeterministicSigner::fixture();
    let actor = ActorId::new("actor:git-email:alice@example.com");

    capture_worktree_review(
        CaptureOptions::new(origin.path())
            .with_actor_id(actor.clone())
            .sign_with(signer.clone()),
    )
    .unwrap();
    let events = read_events(origin.path()).unwrap();
    let artifacts = referenced_artifacts(&events).unwrap();
    assert!(
        artifacts
            .iter()
            .any(|artifact| artifact.kind() == ArtifactKind::Object),
        "signed capture should reference a object artifact"
    );
    remove_object_artifacts(origin.path());
    assert!(
        artifacts
            .iter()
            .filter(|artifact| artifact.kind() == ArtifactKind::Object)
            .all(|artifact| export_artifact(origin.path(), artifact).is_err()),
        "object artifact availability is separate from signature validity"
    );

    let history = review_history(
        ReviewHistoryOptions::new(origin.path())
            .with_verification_policy(EventVerificationPolicy::advisory())
            .with_trust_set(trust_for_actor(actor.as_str(), &signer)),
    )
    .unwrap();

    let capture_entry = history
        .entries
        .iter()
        .find(|entry| entry.event_type == EventType::WorkObjectProposed)
        .expect("capture event is present in history");
    assert_eq!(
        capture_entry.verification_status,
        Some(EventVerificationStatus::Valid)
    );
    assert!(
        !fs::read_to_string(support::common_dir_store(origin.path()).join("state.json"))
            .unwrap()
            .contains("verificationStatus"),
        "verification status is read-time state, not projection state"
    );
    for entry in fs::read_dir(support::common_dir_store(origin.path()).join("events")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            assert!(
                !fs::read_to_string(path)
                    .unwrap()
                    .contains("verificationStatus"),
                "verification status is not persisted into event files"
            );
        }
    }
}

#[test]
fn all_library_write_options_expose_sign_with() {
    let repo = modified_repo();
    let signer = DeterministicSigner::fixture();
    let request_id = shoreline::model::InputRequestId::new("input-request:sha256:test");

    let _ = CaptureOptions::new(repo.path()).sign_with(signer.clone());
    let _ = InputRequestOpenOptions::new(repo.path()).sign_with(signer.clone());
    let _ = InputRequestRespondOptions::new(repo.path(), request_id).sign_with(signer.clone());
    let _ = ObservationAddOptions::new(repo.path()).sign_with(signer.clone());
    let _ = AssessmentAddOptions::new(repo.path())
        .with_assessment(ReviewAssessment::Accepted)
        .sign_with(signer);
}

/// A federation-bridge-shaped flow: read facts as typed structs, respond on
/// behalf of a remote actor, reproduce the documented JSON, and forward the
/// resulting events into a second clone-local store.
#[test]
fn in_process_consumer_reads_attributes_documents_and_forwards() {
    let origin = modified_repo();

    // Capture and open an operative input request in process.
    capture_worktree_review(CaptureOptions::new(origin.path())).unwrap();
    let opened = open_input_request(
        InputRequestOpenOptions::new(origin.path())
            .with_track("human:kevin")
            .with_title("Need approval")
            .with_reason_code(InputRequestReasonCode::ManualDecisionRequired),
    )
    .unwrap();

    // Respond on behalf of a specific remote reviewer (no env mutation needed).
    respond_input_request(
        InputRequestRespondOptions::new(origin.path(), opened.input_request_id.clone())
            .with_outcome(InputRequestResponseOutcome::Approved)
            .with_actor_id(ActorId::new("actor:agent:remote-reviewer")),
    )
    .unwrap();

    // Read back as typed structs and branch on the typed status (#117).
    let listed = list_input_requests(
        InputRequestListOptions::new(origin.path()).with_status(InputRequestStatusFilter::All),
    )
    .unwrap();
    assert_eq!(listed.input_requests.len(), 1);
    let view = &listed.input_requests[0];
    match view.status {
        InputRequestStatus::Responded => {}
        other => panic!("expected Responded, got {other:?}"),
    }
    assert_eq!(
        view.responses[0].writer.actor_id.as_str(),
        "actor:agent:remote-reviewer",
        "the per-call actor override must be the durable writer"
    );

    // Reproduce the documented `shore.review-input-request-list` JSON in process (#118).
    let document = shoreline::documents::input_request_list_document(listed, None);
    let json: Value = serde_json::to_value(&document).unwrap();
    assert_eq!(json["schema"], "shore.review-input-request-list");
    assert_eq!(json["version"], 1);
    assert_eq!(json["inputRequests"][0]["status"], "responded");
    assert_eq!(
        json["inputRequests"][0]["responses"][0]["writer"]["actorId"],
        "actor:agent:remote-reviewer"
    );

    // Forward the origin's events into a second clone-local store (#119).
    let events = read_events(origin.path()).unwrap();
    assert!(events.len() >= 3, "captured + opened + responded");
    let dest = modified_repo();
    let result = ingest_events(IngestEventsOptions::new(dest.path(), events.clone())).unwrap();
    assert_eq!(result.events_created, events.len());

    // The forwarded, remotely attributed decision is visible in the destination.
    let mirrored = list_input_requests(
        InputRequestListOptions::new(dest.path()).with_status(InputRequestStatusFilter::All),
    )
    .unwrap();
    assert_eq!(mirrored.input_requests.len(), 1);
    assert_eq!(
        mirrored.input_requests[0].status,
        InputRequestStatus::Responded
    );
    assert_eq!(
        mirrored.input_requests[0].responses[0]
            .writer
            .actor_id
            .as_str(),
        "actor:agent:remote-reviewer"
    );

    // Re-ingest is idempotent.
    let again = ingest_events(IngestEventsOptions::new(dest.path(), events)).unwrap();
    assert_eq!(again.events_created, 0);
}

#[test]
fn referenced_artifacts_enumerates_snapshot_and_body_refs() {
    let (_origin, events, _body) = origin_with_large_input_request();

    let refs = referenced_artifacts(&events).unwrap();
    let snapshot_refs = refs
        .iter()
        .filter(|artifact| artifact.kind() == ArtifactKind::Object)
        .collect::<Vec<_>>();
    let body_refs = refs
        .iter()
        .filter(|artifact| artifact.kind() == ArtifactKind::Body)
        .collect::<Vec<_>>();

    assert_eq!(snapshot_refs.len(), 1);
    assert_eq!(body_refs.len(), 1);
    for artifact in &refs {
        assert!(artifact.content_hash().starts_with("sha256:"));
    }
    assert_eq!(
        body_refs[0].content_hash(),
        events
            .iter()
            .find(|event| event.event_type.as_str() == "input_request_opened")
            .and_then(|event| event.payload["bodyContentHash"].as_str())
            .expect("input request body hash")
    );

    let duplicated = events
        .iter()
        .cloned()
        .chain(events.iter().cloned())
        .collect::<Vec<_>>();
    assert_eq!(referenced_artifacts(&duplicated).unwrap(), refs);
}

#[test]
fn full_revision_mirror_imports_artifacts() {
    let origin = modified_repo();
    capture_worktree_review(CaptureOptions::new(origin.path())).unwrap();
    let origin_show = show_revision(RevisionShowOptions::new(origin.path())).unwrap();
    let events = read_events(origin.path()).unwrap();
    let refs = referenced_artifacts(&events).unwrap();
    let artifacts = exported_artifacts(&origin, &refs);
    let dest = modified_repo();

    ingest_events(IngestEventsOptions::new(dest.path(), events)).unwrap();
    import_all_artifacts(&dest, &artifacts);
    let mirrored = show_revision(RevisionShowOptions::new(dest.path())).unwrap();

    assert_eq!(mirrored, origin_show);
}

#[test]
fn large_body_hydrates_after_artifact_import() {
    let (origin, events, body) = origin_with_large_input_request();
    let refs = referenced_artifacts(&events).unwrap();
    assert!(
        refs.iter()
            .any(|artifact| artifact.kind() == ArtifactKind::Body)
    );
    let artifacts = exported_artifacts(&origin, &refs);
    let input_request_id = events
        .iter()
        .find(|event| event.event_type.as_str() == "input_request_opened")
        .map(|event| serde_json::from_value(event.payload["inputRequestId"].clone()).unwrap())
        .expect("input request id");
    let dest = modified_repo();

    ingest_events(IngestEventsOptions::new(dest.path(), events)).unwrap();
    import_all_artifacts(&dest, &artifacts);
    let fetched = fetch_input_request(
        InputRequestFetchOptions::new(dest.path(), input_request_id).with_include_body(true),
    )
    .unwrap();

    assert_eq!(fetched.input_request.body.as_deref(), Some(body.as_str()));
}

#[test]
fn events_only_ingest_reports_missing_artifact() {
    let origin = modified_repo();
    capture_worktree_review(CaptureOptions::new(origin.path())).unwrap();
    let events = read_events(origin.path()).unwrap();
    let dest = modified_repo();

    ingest_events(IngestEventsOptions::new(dest.path(), events)).unwrap();
    let error = show_revision(RevisionShowOptions::new(dest.path()))
        .expect_err("events-only mirror should be missing the object artifact");
    let message = error.to_string();

    assert!(
        message.contains("missing artifact"),
        "unexpected error: {message}"
    );
    assert!(message.contains("import"), "unexpected error: {message}");
}

#[test]
fn artifact_import_rejects_hash_mismatch() {
    let (origin, events, _body) = origin_with_large_input_request();
    let refs = referenced_artifacts(&events).unwrap();
    let body_ref = refs
        .iter()
        .find(|artifact| artifact.kind() == ArtifactKind::Body)
        .expect("body artifact ref")
        .clone();
    let bytes = export_artifact(origin.path(), &body_ref).unwrap();
    let mut tampered: Value = serde_json::from_slice(&bytes).unwrap();
    tampered["body"] = Value::String("tampered".to_owned());
    let dest = modified_repo();

    let error = import_artifact(ImportArtifactOptions::new(
        dest.path(),
        body_ref.clone(),
        serde_json::to_vec(&tampered).unwrap(),
    ))
    .expect_err("mismatched bytes must be rejected");

    assert!(
        error.to_string().contains("content hash mismatch"),
        "unexpected error: {error}"
    );

    let created =
        import_artifact(ImportArtifactOptions::new(dest.path(), body_ref, bytes)).unwrap();
    assert_eq!(created.outcome, ImportArtifactOutcome::Created);
}

#[test]
fn artifact_import_is_idempotent() {
    let origin = modified_repo();
    capture_worktree_review(CaptureOptions::new(origin.path())).unwrap();
    let events = read_events(origin.path()).unwrap();
    let artifact = referenced_artifacts(&events).unwrap().remove(0);
    let bytes = export_artifact(origin.path(), &artifact).unwrap();
    let dest = modified_repo();

    let first = import_artifact(ImportArtifactOptions::new(
        dest.path(),
        artifact.clone(),
        bytes.clone(),
    ))
    .unwrap();
    let second = import_artifact(ImportArtifactOptions::new(dest.path(), artifact, bytes)).unwrap();

    assert_eq!(first.outcome, ImportArtifactOutcome::Created);
    assert_eq!(second.outcome, ImportArtifactOutcome::Existing);
}

#[test]
fn referenced_artifacts_derives_imported_note_body_hash_from_path() {
    let repo = modified_repo();
    let body = large_body();
    let sidecar = serde_json::json!({
        "schema": "shore.review-notes",
        "version": 1,
        "files": [
            {
                "path": "src/lib.rs",
                "notes": [
                    {
                        "id": "note-1",
                        "title": "Imported note",
                        "body": body,
                        "target": { "side": "new", "startLine": 1, "endLine": 1 }
                    }
                ]
            }
        ]
    });
    let sidecar_path =
        repo.write_fixture("review-notes.json", serde_json::to_vec(&sidecar).unwrap());
    import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(sidecar_path)).unwrap();
    let events = read_events(repo.path()).unwrap();
    let note_event = events
        .iter()
        .find(|event| event.event_type.as_str() == "review_note_imported")
        .expect("imported note event");
    let path = note_event.payload["bodyArtifactPath"]
        .as_str()
        .expect("body artifact path");
    let expected_hash = format!(
        "sha256:{}",
        path.strip_prefix("artifacts/notes/")
            .and_then(|path| path.strip_suffix(".json"))
            .expect("note body artifact path stem")
    );
    let sidecar_hash = note_event.payload["sidecarContentHash"]
        .as_str()
        .expect("sidecar hash");

    let refs = referenced_artifacts(&events).unwrap();
    let body_ref = refs
        .iter()
        .find(|artifact| artifact.kind() == ArtifactKind::Body)
        .expect("imported note body artifact ref");

    assert_eq!(body_ref.content_hash(), expected_hash);
    assert_ne!(body_ref.content_hash(), sidecar_hash);
}

#[test]
fn library_api_docs_document_artifact_transfer_surface() {
    let docs = std::fs::read_to_string("docs/library-api.md").expect("read library API docs");

    assert!(docs.contains("### Artifacts"));
    assert!(docs.contains("referenced_artifacts"));
    assert!(docs.contains("export_artifact"));
    assert!(docs.contains("import_artifact"));
}

#[test]
fn event_signature_contract_docs_exist_and_reserve_deferred_surfaces() {
    let adr =
        std::fs::read_to_string("docs/adr/adr-0004-event-signatures.md").expect("ADR-0004 exists");
    assert!(adr.contains("application/vnd.shore.event-tbs.v1+json"));
    assert!(adr.contains("valid / invalid / untrusted_key / unsigned"));
    // `eventSetRoot` stays reserved/unimplemented (out of scope); the assertion stays.
    assert!(adr.contains("eventSetRoot"));
    assert!(adr.contains("relay_attestation"));

    // The detached co-signature amendment landed (plan 0068). `eventRecordHash` is
    // now implemented and documented as signature-exclusive; the carrier binds
    // `targetEventRecordHash` (not `targetPayloadHash`) with no new `sigVersion`.
    assert!(adr.contains("## Amendment: Detached Co-Signature Event Family"));
    assert!(adr.contains("targetEventRecordHash"));
    assert!(adr.contains("eventRecordHash"));
    assert!(adr.contains("signature-exclusive"));
    assert!(adr.contains("no new `sigVersion`"));

    // STOP E2 resolved Branch B (fold): ADR-0008 is in-repo, and its `eventRecordHash`
    // exclusion prose names `ingest` (the owner-ratified hop-exclusion correction).
    let adr_0008 = std::fs::read_to_string("docs/adr/adr-0008-cross-peer-conflict-policy.md")
        .expect("ADR-0008 exists in-repo");
    assert!(adr_0008.contains("eventRecordHash"));
    assert!(adr_0008.contains("signature-exclusive"));
    assert!(adr_0008.contains("ingest"));

    let api = std::fs::read_to_string("docs/library-api.md").expect("read library API docs");
    assert!(api.contains("advisory"));
    assert!(api.contains("integrity-strict"));
    assert!(api.contains("trusted-strict"));
}

#[test]
fn revision_supersession_contract_docs_exist() {
    let storage = std::fs::read_to_string("docs/storage-model.md").expect("read storage model");
    let workflow =
        std::fs::read_to_string("docs/review-workflow.md").expect("read review workflow");
    let cli = std::fs::read_to_string("docs/cli-reference.md").expect("read CLI reference");
    let combined = format!("{storage}\n{workflow}\n{cli}");

    // The supersession-DAG succession contract that replaced lineage is documented across the
    // public docs, and succession facts stay signable under the generic event contract.
    for required in [
        "supersedes",
        "supersession DAG",
        "competing",
        "stale_by_superseding_revision",
        "--supersedes",
        "EventToBeSigned",
        "Dead Simple Signing Envelope (DSSE)",
        "pre-authentication encoding",
    ] {
        assert!(
            combined.contains(required),
            "supersession contract docs should mention {required}"
        );
    }

    // The retired scalar-lineage surface no longer appears as current behavior in the prose docs;
    // supersession supersedes it. (ADR-0005's status flip is a separate, later change.)
    for retired in [
        "review_unit_lineage_round_recorded",
        "lineageId",
        "roundIndex",
        "headReviewUnitId",
        "stale_by_newer_round",
        "--lineage",
    ] {
        assert!(
            !combined.contains(retired),
            "retired lineage token {retired} should be gone from the prose docs"
        );
    }

    for forbidden in [format!("Plan {}", "0055"), ["Gum", "bo"].join("")] {
        assert!(!combined.contains(&forbidden));
    }
}

#[test]
fn library_api_documents_event_signing_surface() {
    let api = std::fs::read_to_string("docs/library-api.md").expect("read library API docs");

    for required in [
        "EventVerificationPolicy",
        "EventVerificationStatus",
        "sign_with",
        "writer.actorId = did:key:",
        "EventToBeSigned",
        "Dead Simple Signing Envelope (DSSE)",
        "pre-authentication encoding",
        "artifact availability",
        "different signer or signature",
        "existing_divergent_signature",
    ] {
        assert!(
            api.contains(required),
            "library API docs should mention {required}"
        );
    }
}

/// `ReloadOutcome` is part of the supported surface and must be nameable from a
/// non-test external build (#117).
#[test]
fn reload_outcome_is_publicly_nameable() {
    fn _accepts(_: ReloadOutcome) {}
    let _: Option<ReloadOutcome> = None;
}

/// The `--local` override exclusion helpers must be reachable from outside the crate
/// so the possession-based `identity enroll`/`attest` CLIs can keep their `.local.json`
/// overrides out of git status (INV-A/INV-E).
#[test]
fn local_override_exclusion_helpers_are_public() {
    let repo = support::git_repo::GitRepo::new();
    // Reachable from outside the crate ⇒ they are pub (INV-A). A no-op on an
    // already-clean repo; the point is that this compiles and runs.
    shoreline::session::ensure_local_delegates_excluded(repo.path()).unwrap();
    shoreline::session::ensure_local_actor_attributes_excluded(repo.path()).unwrap();
    let exclude = std::fs::read_to_string(repo.path().join(".git/info/exclude")).unwrap();
    assert!(
        exclude
            .lines()
            .any(|l| l.trim() == ".shore/delegates.local.json")
    );
    assert!(
        exclude
            .lines()
            .any(|l| l.trim() == ".shore/actor-attributes.local.json")
    );
}

/// The sensitivity-scan vocabulary is a typed public contract (#150): kinds,
/// severities, and the block > warn > allow lattice are nameable, and their wire
/// strings are byte-for-byte the scanner's output.
#[test]
fn sensitivity_vocabulary_is_publicly_nameable_and_stable() {
    fn _accepts(_: SensitivityKind, _: SensitivitySeverity, _: SensitivityPolicyOutcome) {}

    let kind_names: Vec<&str> = SensitivityKind::ALL
        .iter()
        .map(|kind| kind.as_str())
        .collect();
    assert_eq!(
        kind_names,
        [
            "known_token",
            "private_key",
            "high_entropy",
            "sensitive_filename",
            "generated_path",
        ]
    );
    let severity_names: Vec<&str> = SensitivitySeverity::ALL
        .iter()
        .map(|severity| severity.as_str())
        .collect();
    assert_eq!(severity_names, ["medium", "high"]);
    let outcome_names: Vec<&str> = SensitivityPolicyOutcome::ALL
        .iter()
        .map(|outcome| outcome.as_str())
        .collect();
    assert_eq!(outcome_names, ["allow", "warn", "block"]);

    // serde forms, Display, and parse round-trip agree with as_str.
    for kind in SensitivityKind::ALL {
        assert_eq!(
            serde_json::to_value(kind).unwrap(),
            Value::String(kind.as_str().to_owned())
        );
        assert_eq!(kind.to_string(), kind.as_str());
        assert_eq!(SensitivityKind::parse(kind.as_str()), Some(kind));
    }
    for severity in SensitivitySeverity::ALL {
        assert_eq!(
            serde_json::to_value(severity).unwrap(),
            Value::String(severity.as_str().to_owned())
        );
        assert_eq!(severity.to_string(), severity.as_str());
        assert_eq!(
            SensitivitySeverity::parse(severity.as_str()),
            Some(severity)
        );
    }
    for outcome in SensitivityPolicyOutcome::ALL {
        assert_eq!(
            serde_json::to_value(outcome).unwrap(),
            Value::String(outcome.as_str().to_owned())
        );
        assert_eq!(outcome.to_string(), outcome.as_str());
        assert_eq!(
            SensitivityPolicyOutcome::parse(outcome.as_str()),
            Some(outcome)
        );
    }
    assert_eq!(SensitivityKind::parse("unknown_kind"), None);
    assert_eq!(SensitivitySeverity::parse(""), None);
    assert_eq!(SensitivityPolicyOutcome::parse("BLOCK"), None);

    // Kind metadata pins the scanner's severity/outcome assignments.
    use SensitivityKind::*;
    for (kind, severity, outcome) in [
        (
            KnownToken,
            SensitivitySeverity::High,
            SensitivityPolicyOutcome::Block,
        ),
        (
            PrivateKey,
            SensitivitySeverity::High,
            SensitivityPolicyOutcome::Block,
        ),
        (
            HighEntropy,
            SensitivitySeverity::Medium,
            SensitivityPolicyOutcome::Warn,
        ),
        (
            SensitiveFilename,
            SensitivitySeverity::Medium,
            SensitivityPolicyOutcome::Warn,
        ),
        (
            GeneratedPath,
            SensitivitySeverity::Medium,
            SensitivityPolicyOutcome::Warn,
        ),
    ] {
        assert_eq!(kind.severity(), severity);
        assert_eq!(kind.policy_outcome(), outcome);
    }

    // The lattice: ordering and combine.
    assert!(SensitivitySeverity::Medium < SensitivitySeverity::High);
    assert!(SensitivityPolicyOutcome::Allow < SensitivityPolicyOutcome::Warn);
    assert!(SensitivityPolicyOutcome::Warn < SensitivityPolicyOutcome::Block);
    assert_eq!(
        SensitivityPolicyOutcome::combine([]),
        SensitivityPolicyOutcome::Allow
    );
    assert_eq!(
        SensitivityPolicyOutcome::combine([SensitivityPolicyOutcome::Warn]),
        SensitivityPolicyOutcome::Warn
    );
    assert_eq!(
        SensitivityPolicyOutcome::combine([
            SensitivityPolicyOutcome::Warn,
            SensitivityPolicyOutcome::Block,
            SensitivityPolicyOutcome::Allow,
        ]),
        SensitivityPolicyOutcome::Block
    );
}

/// The sensitivity vocabulary is documented as a supported public contract (#150),
/// including the conformance fixture and the relay's deliberate known_token
/// divergence.
#[test]
fn library_api_documents_sensitivity_vocabulary() {
    let api = std::fs::read_to_string("docs/library-api.md").expect("read library API docs");

    for required in [
        "SensitivityKind",
        "SensitivitySeverity",
        "SensitivityPolicyOutcome",
        "known_token",
        "sensitive_filename",
        "`block` > `warn` > `allow`",
        "conformance-vectors.json",
        "anywhere in a token",
    ] {
        assert!(
            api.contains(required),
            "library API docs should mention {required}"
        );
    }
}

/// A strict-policy rejection is classifiable by variant from an external consumer,
/// with the event id and status available without parsing message text (#144).
#[test]
fn ingest_rejection_is_classifiable_by_variant() {
    let (_origin, events, _body) = origin_with_large_input_request();
    let event = events.first().expect("origin produced events").clone();
    let expected_id = event.event_id.clone();
    let dest = GitRepo::new();

    let error = ingest_events(
        IngestEventsOptions::new(dest.path(), vec![event])
            .with_verification_policy(EventVerificationPolicy::trusted_strict()),
    )
    .unwrap_err();

    match error {
        ShoreError::EventVerificationRejected { event_id, status } => {
            assert_eq!(event_id, expected_id);
            assert_eq!(status, EventVerificationStatus::Unsigned);
        }
        other => panic!("expected EventVerificationRejected, got {other:?}"),
    }
}

/// Strict-policy ingest rejection is a documented, typed contract (#144).
#[test]
fn library_api_documents_typed_ingest_rejection() {
    let api = std::fs::read_to_string("docs/library-api.md").expect("read library API docs");

    for required in [
        "EventVerificationRejected",
        "event signature verification rejected event",
    ] {
        assert!(
            api.contains(required),
            "library API docs should mention {required}"
        );
    }
}

/// `EventWriteOutcome` is part of the supported surface (#145): nameable from an
/// external build, with snake_case wire strings agreed between serde, as_str, and
/// Display.
#[test]
fn event_write_outcome_is_publicly_nameable() {
    fn _accepts(_: EventWriteOutcome) {}

    for (outcome, name) in [
        (EventWriteOutcome::Created, "created"),
        (EventWriteOutcome::Existing, "existing"),
        (
            EventWriteOutcome::ExistingDivergentSignature,
            "existing_divergent_signature",
        ),
    ] {
        assert_eq!(outcome.as_str(), name);
        assert_eq!(outcome.to_string(), name);
        assert_eq!(
            serde_json::to_value(outcome).unwrap(),
            Value::String(name.to_owned())
        );
        let parsed: EventWriteOutcome =
            serde_json::from_value(Value::String(name.to_owned())).unwrap();
        assert_eq!(parsed, outcome);
    }
}

/// Per-event write outcomes are a documented contract (#145): row shape, input
/// ordering, and the wire strings a forwarding consumer maps into its receipts.
#[test]
fn library_api_documents_per_event_write_outcomes() {
    let api = std::fs::read_to_string("docs/library-api.md").expect("read library API docs");

    for required in [
        "write_outcome",
        "EventWriteOutcome",
        "existing_divergent_signature",
        "input order",
    ] {
        assert!(
            api.contains(required),
            "library API docs should mention {required}"
        );
    }
}
