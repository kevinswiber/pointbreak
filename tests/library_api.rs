//! End-to-end exercise of the supported in-process library API (see
//! `docs/library-api.md`): capture, an attributed write, typed reads, documented
//! JSON, and forwarding events into a second store — all without the CLI.

mod support;

use serde_json::Value;
use shoreline::model::ActorId;
use shoreline::session::event::{InputRequestReasonCode, InputRequestResponseOutcome};
use shoreline::session::{
    ArtifactKind, ArtifactRef, CaptureOptions, ImportArtifactOptions, ImportArtifactOutcome,
    ImportNotesOptions, IngestEventsOptions, InputRequestFetchOptions, InputRequestListOptions,
    InputRequestOpenOptions, InputRequestRespondOptions, InputRequestStatus,
    InputRequestStatusFilter, ReloadOutcome, ReviewUnitShowOptions, capture_worktree_review,
    export_artifact, fetch_input_request, import_artifact, import_notes, ingest_events,
    list_input_requests, open_input_request, read_events, referenced_artifacts,
    respond_input_request, show_review_unit,
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
    let document = shoreline::documents::input_request_list_document(listed);
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
        .filter(|artifact| artifact.kind() == ArtifactKind::Snapshot)
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
fn full_review_unit_mirror_imports_artifacts() {
    let origin = modified_repo();
    capture_worktree_review(CaptureOptions::new(origin.path())).unwrap();
    let origin_show = show_review_unit(ReviewUnitShowOptions::new(origin.path())).unwrap();
    let events = read_events(origin.path()).unwrap();
    let refs = referenced_artifacts(&events).unwrap();
    let artifacts = exported_artifacts(&origin, &refs);
    let dest = modified_repo();

    ingest_events(IngestEventsOptions::new(dest.path(), events)).unwrap();
    import_all_artifacts(&dest, &artifacts);
    let mirrored = show_review_unit(ReviewUnitShowOptions::new(dest.path())).unwrap();

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
    let error = show_review_unit(ReviewUnitShowOptions::new(dest.path()))
        .expect_err("events-only mirror should be missing the snapshot artifact");
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
    assert!(docs.contains("not authenticity"));
    assert!(docs.contains("signatures"));
}

/// `ReloadOutcome` is part of the supported surface and must be nameable from a
/// non-test external build (#117).
#[test]
fn reload_outcome_is_publicly_nameable() {
    fn _accepts(_: ReloadOutcome) {}
    let _: Option<ReloadOutcome> = None;
}
