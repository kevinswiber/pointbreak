use std::fs;
use std::path::PathBuf;
use std::process::Command;

use pointbreak::session::{
    ImportArtifactOptions, IngestEventsOptions, import_artifact, ingest_events,
    referenced_artifacts,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::tempdir;

#[allow(dead_code)]
#[path = "../examples/support/review_example_pack.rs"]
mod pack_support;

fn pack_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/review/checkout-refactor")
}

#[test]
fn canonical_review_example_pack_exists() {
    let manifest = pack_root().join("manifest.json");
    assert!(
        manifest.is_file(),
        "canonical Review example manifest is missing: {}",
        manifest.display()
    );
}

#[test]
fn current_exporter_and_materialize_hint_use_pointbreak() {
    let exporter = include_str!("../examples/support/review_example_pack.rs");
    let command = include_str!("../examples/review_example_pack.rs");

    assert!(exporter.contains("name: \"pointbreak\".to_owned()"));
    assert!(command.contains("pointbreak inspect --repo"));
    assert!(!command.contains("shore inspect --repo"));
}

#[test]
fn canonical_review_example_manifest_pins_the_record_and_all_authoritative_files() {
    let manifest_path = pack_root().join("manifest.json");
    let manifest: Value = serde_json::from_slice(
        &fs::read(&manifest_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", manifest_path.display())),
    )
    .expect("manifest is valid JSON");

    assert_eq!(manifest["schema"], "pointbreak.review-example-pack");
    assert_eq!(manifest["version"], 1);
    assert_eq!(manifest["name"], "checkout-refactor");
    assert_eq!(manifest["classification"], "reproducible_sample_record");
    assert_eq!(manifest["producer"]["name"], "shore");
    assert_eq!(manifest["producer"]["version"], "0.5.0");
    let producer_commit = manifest["producer"]["commit"]
        .as_str()
        .expect("producer commit");
    assert_eq!(producer_commit.len(), 40);
    assert!(producer_commit.bytes().all(|byte| byte.is_ascii_hexdigit()));

    assert_eq!(manifest["record"]["eventCount"], 13);
    assert_eq!(
        manifest["record"]["eventSetHash"],
        "sha256:cabdabbbdf88ab71b43faee14cc28bf8e407e5c2bfc18d07af4bba126da12243"
    );
    assert_eq!(
        manifest["record"]["revision"],
        "rev:sha256:fa6981d38de12a850da707b69657e7a9153120c92a0dd08f534fbb40394d885f"
    );
    assert_eq!(
        manifest["record"]["track"],
        "example:marketing-review-proof"
    );
    assert_eq!(manifest["record"]["selectedAssessment"], "accepted");
    assert_eq!(manifest["record"]["verificationStatus"], "unsigned");
    assert_eq!(
        manifest["record"]["writerActors"],
        serde_json::json!([
            "actor:agent:pointbreak-example-author",
            "actor:agent:pointbreak-example-reviewer"
        ])
    );

    assert_eq!(manifest["events"]["path"], "events.json");
    assert_eq!(manifest["events"]["count"], 13);
    assert_eq!(manifest["source"]["bundlePath"], "source.bundle");
    assert_eq!(manifest["source"]["bundleRef"], "refs/heads/main");
    assert_eq!(
        manifest["source"]["base"]["commitOid"],
        "f1a8ed1801f669b1b846e482d198092cd6e617df"
    );
    assert_eq!(
        manifest["source"]["target"]["commitOid"],
        "3e7b4b3e1e1e7cccfc14a4c724204ff381b315e4"
    );
    assert_eq!(
        manifest["source"]["response"]["commitOid"],
        "c4f50c2dc010f69f9080d0ad6b0999728568c3c1"
    );
    for pointer in [
        "/source/base/treeOid",
        "/source/target/treeOid",
        "/source/response/treeOid",
    ] {
        let oid = manifest.pointer(pointer).and_then(Value::as_str).unwrap();
        assert_eq!(oid.len(), 40, "manifest field {pointer} is not a Git OID");
        assert!(oid.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
    assert_eq!(
        manifest["documents"]["history"]["path"],
        "exports/history.json"
    );
    assert_eq!(
        manifest["documents"]["history"]["schema"],
        "pointbreak.review-history"
    );
    assert_eq!(manifest["documents"]["history"]["version"], 1);
    assert_eq!(
        manifest["documents"]["revision"]["path"],
        "exports/revision.json"
    );
    assert_eq!(
        manifest["documents"]["revision"]["schema"],
        "pointbreak.review-revision"
    );
    assert_eq!(manifest["documents"]["revision"]["version"], 2);

    for pointer in [
        "/events/sha256",
        "/source/bundleSha256",
        "/documents/history/sha256",
        "/documents/revision/sha256",
    ] {
        let digest = manifest
            .pointer(pointer)
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("manifest field {pointer} is missing"));
        assert_eq!(
            digest.len(),
            64,
            "manifest field {pointer} is not a SHA-256"
        );
        assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    let artifacts = manifest["artifacts"]
        .as_array()
        .expect("artifacts is an array");
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0]["kind"], "object");
    assert_eq!(
        artifacts[0]["contentHash"],
        "sha256:c366c7cb8d826536573781f9136d9ccbcebc17301cc6aaba0b8a4f1c2f641327"
    );
    assert_ne!(
        artifacts[0]["contentHash"]
            .as_str()
            .unwrap()
            .trim_start_matches("sha256:"),
        artifacts[0]["sha256"]
            .as_str()
            .expect("artifact byte digest")
    );
}

#[test]
fn canonical_review_example_has_the_complete_causal_record() {
    let events: Vec<pointbreak::session::event::ShoreEvent> = serde_json::from_slice(
        &fs::read(pack_root().join("events.json")).expect("read events.json"),
    )
    .expect("events.json is valid JSON");
    assert_eq!(events.len(), 13);

    let event_types = events
        .iter()
        .map(|event| event.event_type.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        event_types,
        [
            "work_object_proposed",
            "revision_ref_associated",
            "review_observation_recorded",
            "validation_check_recorded",
            "input_request_opened",
            "review_assessment_recorded",
            "review_observation_recorded",
            "review_observation_recorded",
            "validation_check_recorded",
            "input_request_responded",
            "validation_check_recorded",
            "review_assessment_recorded",
            "revision_commit_associated",
        ]
    );
    assert!(events.iter().all(|event| event.signature.is_none()));

    let revision: Value = serde_json::from_slice(
        &fs::read(pack_root().join("exports/revision.json")).expect("read revision document"),
    )
    .expect("revision document is valid JSON");
    assert_eq!(revision["schema"], "pointbreak.review-revision");
    assert_eq!(revision["version"], 2);
    assert_eq!(
        revision["eventSetHash"],
        "sha256:cabdabbbdf88ab71b43faee14cc28bf8e407e5c2bfc18d07af4bba126da12243"
    );
    assert_eq!(revision["eventCount"], 13);
    assert_eq!(revision["currentAssessment"]["assessment"], "accepted");

    let assessments = revision["assessments"].as_array().expect("assessments");
    assert_eq!(assessments.len(), 2);
    assert_eq!(assessments[0]["assessment"], "needs_changes");
    assert_eq!(assessments[0]["status"], "replaced");
    assert_eq!(assessments[1]["assessment"], "accepted");
    assert_eq!(assessments[1]["status"], "current");
    assert_eq!(
        assessments[1]["replaces"],
        serde_json::json!([assessments[0]["id"]])
    );

    let request = &revision["inputRequests"][0];
    assert_eq!(request["reasonCode"], "manual_decision_required");
    assert_eq!(request["status"], "responded");
    assert_eq!(request["responses"][0]["outcome"], "approved");

    let validations = revision["validationChecks"]
        .as_array()
        .expect("validations");
    assert_eq!(validations.len(), 3);
    assert_eq!(validations[0]["status"], "failed");
    assert_eq!(validations[1]["status"], "passed");
    assert_eq!(validations[2]["status"], "passed");

    let response_commit = &revision["commitRange"]["currentCommits"][1];
    assert_eq!(response_commit["source"], "association");
    assert_eq!(
        response_commit["commitOid"],
        "c4f50c2dc010f69f9080d0ad6b0999728568c3c1"
    );
    assert!(revision["commitRange"].get("liveness").is_none());
}

#[test]
fn canonical_review_example_materializes_through_public_apis() {
    pack_support::verify_pack(&pack_root()).expect("verify canonical pack");
    let temp = tempdir().expect("temporary directory");
    let output = temp.path().join("checkout-refactor");
    pack_support::materialize_pack(&pack_root(), &output).expect("materialize canonical pack");

    let log = Command::new("git")
        .arg("-C")
        .arg(&output)
        .args(["log", "--format=%H", "--reverse"])
        .output()
        .expect("read materialized git log");
    assert!(log.status.success());
    assert_eq!(
        String::from_utf8(log.stdout)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        [
            "f1a8ed1801f669b1b846e482d198092cd6e617df",
            "3e7b4b3e1e1e7cccfc14a4c724204ff381b315e4",
            "c4f50c2dc010f69f9080d0ad6b0999728568c3c1",
        ]
    );

    let manifest: Value = serde_json::from_slice(
        &fs::read(pack_root().join("manifest.json")).expect("read manifest"),
    )
    .expect("manifest JSON");
    for name in ["base", "target", "response"] {
        let commit = manifest["source"][name]["commitOid"].as_str().unwrap();
        let expected_tree = manifest["source"][name]["treeOid"].as_str().unwrap();
        let tree = Command::new("git")
            .arg("-C")
            .arg(&output)
            .args(["rev-parse", &format!("{commit}^{{tree}}")])
            .output()
            .unwrap();
        assert!(tree.status.success());
        assert_eq!(
            String::from_utf8(tree.stdout).unwrap().trim(),
            expected_tree
        );
    }

    let test = Command::new("node")
        .arg("checkout.test.js")
        .current_dir(&output)
        .status()
        .expect("run materialized source tests");
    assert!(test.success());

    let events: Vec<pointbreak::session::event::ShoreEvent> =
        serde_json::from_slice(&fs::read(pack_root().join("events.json")).expect("read events"))
            .expect("deserialize events");
    let second_ingest = ingest_events(IngestEventsOptions::new(&output, events.clone()))
        .expect("idempotent event ingest");
    assert_eq!(second_ingest.events_created, 0);
    assert_eq!(second_ingest.events_existing, 13);

    for artifact in referenced_artifacts(&events).expect("artifact refs") {
        let entry = manifest["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["contentHash"] == artifact.content_hash())
            .expect("artifact manifest entry");
        let result = import_artifact(ImportArtifactOptions::new(
            &output,
            artifact,
            fs::read(pack_root().join(entry["path"].as_str().unwrap())).unwrap(),
        ))
        .expect("idempotent artifact import");
        assert_eq!(
            result.outcome,
            pointbreak::session::ImportArtifactOutcome::Existing
        );
    }
}

#[test]
fn canonical_review_example_rejects_corruption_and_nonempty_destinations() {
    let temp = tempdir().expect("temporary directory");
    let corrupt = temp.path().join("corrupt-pack");
    copy_dir(&pack_root(), &corrupt);
    let artifact = fs::read_dir(corrupt.join("artifacts"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(&artifact, b"corrupt").unwrap();
    let error = pack_support::verify_pack(&corrupt).unwrap_err().to_string();
    assert!(
        error.contains("digest mismatch"),
        "unexpected error: {error}"
    );

    let destination = temp.path().join("nonempty");
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("keep"), b"do not replace").unwrap();
    let error = pack_support::materialize_pack(&pack_root(), &destination)
        .unwrap_err()
        .to_string();
    assert!(error.contains("not empty"), "unexpected error: {error}");
    assert_eq!(
        fs::read(destination.join("keep")).unwrap(),
        b"do not replace"
    );
}

#[test]
fn canonical_review_example_rejects_unknown_schema_and_forged_relationships() {
    let temp = tempdir().expect("temporary directory");

    let unknown_schema = temp.path().join("unknown-schema");
    copy_dir(&pack_root(), &unknown_schema);
    let manifest_path = unknown_schema.join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["schema"] = Value::String("pointbreak.unknown-pack".to_owned());
    write_json(&manifest_path, &manifest);
    let error = pack_support::verify_pack(&unknown_schema)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("manifest.schema"),
        "unexpected error: {error}"
    );

    let forged_relationship = temp.path().join("forged-relationship");
    copy_dir(&pack_root(), &forged_relationship);
    let revision_path = forged_relationship.join("exports/revision.json");
    let mut revision: Value = serde_json::from_slice(&fs::read(&revision_path).unwrap()).unwrap();
    revision["assessments"][1]["replaces"] = serde_json::json!(["assess:forged"]);
    write_json(&revision_path, &revision);

    let manifest_path = forged_relationship.join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["documents"]["revision"]["sha256"] =
        Value::String(sha256(&fs::read(&revision_path).unwrap()));
    write_json(&manifest_path, &manifest);
    let error = pack_support::verify_pack(&forged_relationship)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("accepted replacement"),
        "unexpected error: {error}"
    );
}

fn copy_dir(source: &std::path::Path, destination: &std::path::Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn write_json(path: &std::path::Path, value: &Value) {
    let mut bytes = serde_json::to_vec_pretty(value).unwrap();
    bytes.push(b'\n');
    fs::write(path, bytes).unwrap();
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
