//! Deterministic builders for the event-signature golden vectors.
//!
//! Every fixture under `tests/fixtures/event_signatures/` is rebuilt in
//! memory from the seeds checked in as `did-key-ed25519.json` (`seedHex`,
//! `unauthorizedSeedHex`). Ed25519 signing is deterministic, so the output is
//! byte-for-byte reproducible by anyone. The regeneration entry point lives in
//! `tests/event_signature_vectors.rs` and is env-gated.

use std::collections::BTreeMap;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use ed25519_dalek::{Signer as _, SigningKey};
use pointbreak::crypto::SignerId;
use pointbreak::model::{
    JournalId, ReviewTargetRef, RevisionId, TargetRef, TaskTargetRef, TrackId, WorkObjectId,
};
use pointbreak::session::event::{
    EventTarget, EventToBeSigned, ShoreEvent, event_signature_pre_authentication_encoding,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// In-memory fixture set: file name → exact file bytes.
pub struct FixtureSet {
    files: BTreeMap<String, Vec<u8>>,
}

impl FixtureSet {
    pub fn file_names(&self) -> Vec<String> {
        self.files.keys().cloned().collect()
    }

    pub fn bytes(&self, name: &str) -> &[u8] {
        self.files
            .get(name)
            .unwrap_or_else(|| panic!("no built fixture named {name}"))
    }

    fn insert_json(&mut self, name: &str, value: &Value) {
        let mut bytes = serde_json::to_vec_pretty(value).expect("serialize fixture json");
        bytes.push(b'\n');
        self.files.insert(name.to_owned(), bytes);
    }
}

struct Keys {
    did_key: String,
    seed: [u8; 32],
    unauthorized_did_key: String,
    unauthorized_seed: [u8; 32],
}

/// A post-signing envelope mutation. The signature is left as signed, so the
/// mutated event must verify with `expected` status.
struct MutationCase {
    file: &'static str,
    mutation: &'static str,
    apply: fn(&mut Value),
}

const MUTATION_CASES: &[MutationCase] = &[
    MutationCase {
        file: "payload-mutated-event.json",
        mutation: "payloadHash and payload body changed after signing",
        apply: mutate_payload,
    },
    MutationCase {
        file: "actor-mutated-event.json",
        mutation: "writer.actorId changed after signing",
        apply: |event| {
            event["writer"]["actorId"] = json!("actor:git-email:bob@example.com");
        },
    },
    MutationCase {
        file: "target-mutated-event.json",
        mutation: "target subjectId changed after signing",
        apply: |event| {
            event["target"]["subjectId"] = json!(format!("subject:sha256:{}", "9".repeat(64)));
        },
    },
    MutationCase {
        file: "timestamp-mutated-event.json",
        mutation: "occurredAt changed after signing",
        apply: |event| {
            event["occurredAt"] = json!("2026-06-04T00:00:00Z");
        },
    },
    MutationCase {
        file: "assertion-mode-mutated-event.json",
        mutation: "assertionMode changed after signing",
        apply: |event| {
            event["assertionMode"] = json!("operative");
        },
    },
];

fn mutate_payload(event: &mut Value) {
    let body = "Mutated event signature vector.";
    event["payload"]["body"] = json!(body);
    event["payload"]["bodyByteSize"] = json!(body.len());
    event["payload"]["bodyContentHash"] = json!(sha256_prefixed(body.as_bytes()));
    let payload_hash = sha256_canonical_json_prefixed(&event["payload"]);
    event["payloadHash"] = json!(payload_hash);
}

/// Builds every regenerable fixture. `fixture_dir` supplies the checked-in
/// seed material (`did-key-ed25519.json`), which is an input, not an output.
pub fn build_all_fixtures(fixture_dir: &Path) -> FixtureSet {
    let keys = read_keys(fixture_dir);
    let mut set = FixtureSet {
        files: BTreeMap::new(),
    };

    // Friendly-actor valid event: writer is a friendly actor id, so the
    // resolved did:key rides in the top-level signer field.
    let friendly = sign_event(base_event(), &keys.did_key, &keys.seed);
    set.insert_json("friendly-valid-event.json", &friendly);

    // Self-certifying valid event: the writer actor id IS the did:key, so the
    // signer field stays null.
    let mut self_certifying = base_event();
    self_certifying["writer"]["actorId"] = json!(keys.did_key.clone());
    let self_certifying = sign_event(self_certifying, &keys.did_key, &keys.seed);
    set.insert_json("self-certifying-valid-event.json", &self_certifying);

    set.insert_json("unsigned-event.json", &base_event());

    let unauthorized = sign_event(
        base_event(),
        &keys.unauthorized_did_key,
        &keys.unauthorized_seed,
    );
    set.insert_json("unauthorized-signer-event.json", &unauthorized);

    let mut mutation_index = Vec::new();
    for case in MUTATION_CASES {
        let mut event = friendly.clone();
        (case.apply)(&mut event);
        set.insert_json(case.file, &event);
        mutation_index.push(json!({
            "expected": "invalid",
            "file": case.file,
            "mutation": case.mutation,
        }));
    }

    // Task-domain pair covering the relocated speaker fact: the payload is
    // hash-bound into the signed view, so flipping sourceSpeaker (with
    // payloadHash recomputed so structural validation passes) must verify
    // invalid via the signature over payloadHash.
    let source_speaker_valid = sign_event(task_event(), &keys.did_key, &keys.seed);
    set.insert_json("source-speaker-valid-event.json", &source_speaker_valid);
    let mut source_speaker_mutated = source_speaker_valid.clone();
    source_speaker_mutated["payload"]["sourceSpeaker"] = json!("agent");
    source_speaker_mutated["payloadHash"] = json!(sha256_canonical_json_prefixed(
        &source_speaker_mutated["payload"]
    ));
    set.insert_json("source-speaker-mutated-event.json", &source_speaker_mutated);
    mutation_index.push(json!({
        "expected": "invalid",
        "file": "source-speaker-mutated-event.json",
        "mutation": "payload sourceSpeaker and payloadHash changed after signing",
    }));

    mutation_index.push(json!({
        "expected": "untrusted_key",
        "file": "unauthorized-signer-event.json",
        "mutation": "signature verifies but signer is not allowed for actor",
    }));
    set.insert_json("mutation-cases.json", &json!({ "cases": mutation_index }));

    // Legacy-payload-view fixture: a signed review-domain proposal whose payload
    // uses the retired artifact-hash wire key, marked payloadVersion 0. At the 1.0
    // format floor there is no read-time upcast, so this view is refused on read;
    // the fixture exercises that rejection while its signature stays valid over the
    // stored legacy bytes.
    let legacy_view = sign_event(
        legacy_view_revision_proposed_event(),
        &keys.did_key,
        &keys.seed,
    );
    set.insert_json("legacy-view-work-object-proposed-event.json", &legacy_view);

    let mut unsupported_alg = friendly.clone();
    unsupported_alg["signature"]["alg"] = json!("ed25519ph");
    set.insert_json("unsupported-alg-event.json", &unsupported_alg);

    let mut unsupported_sig_version = friendly.clone();
    unsupported_sig_version["signature"]["sigVersion"] = json!(99);
    set.insert_json(
        "unsupported-sig-version-event.json",
        &unsupported_sig_version,
    );

    set.insert_json(
        "negative-crypto-cases.json",
        &negative_crypto_cases(&friendly),
    );

    // Canonical TBS view + DSSE pre-authentication encoding for the friendly
    // valid event, via the crate's own signing path.
    let event: ShoreEvent =
        serde_json::from_value(friendly.clone()).expect("friendly fixture decodes");
    let signer = SignerId::parse(&keys.did_key).expect("fixture did:key parses");
    let tbs = EventToBeSigned::from_event(&event, &signer).expect("build to-be-signed view");

    let mut tbs_json = serde_json::to_string_pretty(&tbs)
        .expect("serialize to-be-signed view")
        .into_bytes();
    tbs_json.push(b'\n');
    set.files
        .insert("canonical-tbs-v1.json".to_owned(), tbs_json);

    let mut tbs_bytes = tbs.canonical_bytes().expect("canonical to-be-signed bytes");
    tbs_bytes.push(b'\n');
    set.files
        .insert("canonical-tbs-v1.bytes".to_owned(), tbs_bytes);

    let mut pae_bytes = event_signature_pre_authentication_encoding(&tbs)
        .expect("DSSE pre-authentication encoding bytes");
    pae_bytes.push(b'\n');
    set.files.insert("pae-v1.bytes".to_owned(), pae_bytes);

    set
}

fn read_keys(fixture_dir: &Path) -> Keys {
    let raw = std::fs::read(fixture_dir.join("did-key-ed25519.json")).expect("read seed fixture");
    let value: Value = serde_json::from_slice(&raw).expect("seed fixture is valid json");
    Keys {
        did_key: value["didKey"].as_str().expect("didKey").to_owned(),
        seed: seed_from_hex(value["seedHex"].as_str().expect("seedHex")),
        unauthorized_did_key: value["unauthorizedDidKey"]
            .as_str()
            .expect("unauthorizedDidKey")
            .to_owned(),
        unauthorized_seed: seed_from_hex(
            value["unauthorizedSeedHex"]
                .as_str()
                .expect("unauthorizedSeedHex"),
        ),
    }
}

fn seed_from_hex(hex: &str) -> [u8; 32] {
    let mut seed = [0u8; 32];
    for (i, byte) in seed.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("seed hex digit");
    }
    seed
}

/// Build the opaque envelope target for a subject and return both the serialized
/// `{journalId, subjectId?, trackId?}` value and the derived `subjectId`. The
/// signed vectors bind the opaque subjectId (seam S2), and the two
/// work-object-proposed producers fold that same subjectId into their idempotency
/// key, so both are sourced from one live `EventTarget::for_subject` call.
fn subject_target(journal_id: &str, subject: TargetRef, track_id: Option<&str>) -> (Value, String) {
    let target = EventTarget::for_subject(
        JournalId::new(journal_id),
        subject,
        track_id.map(TrackId::new),
    )
    .expect("build event target");
    let subject_id = target
        .subject_id
        .clone()
        .expect("a non-journal subject has an opaque id");
    let value = serde_json::to_value(&target).expect("serialize event target");
    (value, subject_id)
}

/// The shared unsigned envelope every vector derives from. Hashes are
/// recomputed from content; the trailing assert pins the eventId to the value
/// the opaque-coded vectors carry.
fn base_event() -> Value {
    let revision_id = format!("review-unit:sha256:{}", "1".repeat(64));
    let body = "Pinned event signature vector.";
    let payload = json!({
        "body": body,
        "bodyByteSize": body.len(),
        "bodyContentHash": sha256_prefixed(body.as_bytes()),
        "observationId": format!("obs:sha256:{}", "4".repeat(64)),
        "tags": ["event-signature", "golden-vector"],
        "target": {
            "kind": "revision",
            "revisionId": revision_id,
        },
        "title": "Golden vector observation",
    });
    // The envelope binds the opaque subjectId; the structural target stays in the
    // payload above and is reconstructed by the projection.
    let (target, _subject_id) = subject_target(
        "journal:fixture:event-signatures",
        TargetRef::Review(ReviewTargetRef::Revision {
            revision_id: RevisionId::new(&revision_id),
        }),
        None,
    );
    let idempotency_key = format!("t:03:{revision_id}:agent:vector:event-signature-golden-vector");
    let event_id = format!(
        "evt:sha256:{}",
        hex(&Sha256::digest(idempotency_key.as_bytes()))
    );
    let payload_hash = sha256_canonical_json_prefixed(&payload);
    assert_eq!(
        event_id, "evt:sha256:0fe6523806ec94ccbe5380da6583477b32c6ed398c741d483c4b61a5a68a837d",
        "eventId derivation drifted from the pinned vector"
    );

    json!({
        "assertionMode": "advisory",
        "eventId": event_id,
        "eventType": "t:03",
        "idempotencyKey": idempotency_key,
        "occurredAt": "2026-06-03T23:59:00Z",
        "payload": payload,
        "payloadHash": payload_hash,
        "schema": "shore.event",
        "signature": null,
        "signer": null,
        "target": target,
        "version": 1,
        "writer": {
            "actorId": "actor:git-email:alice@example.com",
            "producer": {
                "name": "shore",
                "version": "0.1.0-test",
            }
        },
    })
}

/// A signed task-domain envelope whose payload carries the relocated
/// `sourceSpeaker` fact.
fn task_event() -> Value {
    let task_attempt_id = format!("task-attempt:sha256:{}", "5".repeat(64));
    let claude_session_uuid = "event-signature-task-vector";
    let payload = json!({
        "engagementId": format!("engagement:sha256:{}", "7".repeat(64)),
        "workObject": {
            "kind": "task_attempt",
            "claudeSessionUuid": claude_session_uuid,
            "initialPromptHash": format!("sha256:{}", "6".repeat(64)),
            "projectPath": "/repo",
            "sourceSpeaker": "user",
            "taskAttemptId": task_attempt_id,
        },
    });
    // The task-attempt WorkObjectProposed folds the opaque subjectId into its
    // idempotency key (the envelope binds the same subjectId).
    let (target, subject_id) = subject_target(
        &format!("journal:claude:{claude_session_uuid}"),
        TargetRef::Task(TaskTargetRef::TaskAttempt {
            task_attempt_id: WorkObjectId::new(&task_attempt_id),
        }),
        None,
    );
    let idempotency_key = format!("t:02:{subject_id}");
    let event_id = format!(
        "evt:sha256:{}",
        hex(&Sha256::digest(idempotency_key.as_bytes()))
    );
    let payload_hash = sha256_canonical_json_prefixed(&payload);

    json!({
        "assertionMode": "advisory",
        "eventId": event_id,
        "eventType": "t:02",
        "idempotencyKey": idempotency_key,
        "occurredAt": "2026-06-03T23:59:30Z",
        "payload": payload,
        "payloadHash": payload_hash,
        "schema": "shore.event",
        "signature": null,
        "signer": null,
        "target": target,
        "version": 1,
        "writer": {
            "actorId": "actor:git-email:alice@example.com",
            "producer": {
                "name": "shore",
                "version": "0.1.0-test",
            }
        },
    })
}

/// A signed review-domain `work_object_proposed` envelope pinned at a legacy
/// payload view: the artifact binding rides under the retired
/// `snapshotArtifactContentHash` wire key and the envelope carries
/// `payloadVersion: 0`. At the 1.0 format floor this view is refused on read (no
/// read-time upcast); the fixture exercises that rejection. The signature is over
/// the stored (legacy) payload bytes and stays valid regardless.
fn legacy_view_revision_proposed_event() -> Value {
    let revision_id = format!("rev:sha256:{}", "8".repeat(64));
    let object_id = format!("obj:sha256:{}", "9".repeat(64));
    let payload = json!({
        "engagementId": format!("engagement:sha256:{}", "b".repeat(64)),
        "workObject": {
            "kind": "revision",
            "revision": {
                "id": revision_id,
                "objectId": object_id,
            },
            "snapshotArtifactContentHash": format!("sha256:{}", "a".repeat(64)),
        },
    });
    // The review-domain WorkObjectProposed folds the opaque subjectId into its
    // idempotency key (the envelope binds the same subjectId).
    let (target, subject_id) = subject_target(
        "journal:fixture:event-signatures",
        TargetRef::Review(ReviewTargetRef::Revision {
            revision_id: RevisionId::new(&revision_id),
        }),
        None,
    );
    let idempotency_key = format!("t:02:{subject_id}");
    let event_id = format!(
        "evt:sha256:{}",
        hex(&Sha256::digest(idempotency_key.as_bytes()))
    );
    let payload_hash = sha256_canonical_json_prefixed(&payload);

    json!({
        "assertionMode": "advisory",
        "eventId": event_id,
        "eventType": "t:02",
        "idempotencyKey": idempotency_key,
        "occurredAt": "2026-06-03T23:59:45Z",
        "payload": payload,
        "payloadHash": payload_hash,
        "payloadVersion": 0,
        "schema": "shore.event",
        "signature": null,
        "signer": null,
        "target": target,
        "version": 1,
        "writer": {
            "actorId": "actor:git-email:alice@example.com",
            "producer": {
                "name": "shore",
                "version": "0.1.0-test",
            }
        },
    })
}

/// Signs an unsigned envelope over the crate's own to-be-signed view and DSSE
/// pre-authentication encoding, mirroring `sign_event_if_requested`: the
/// top-level signer is set only when the writer actor is not the did:key.
fn sign_event(mut event: Value, did_key: &str, seed: &[u8; 32]) -> Value {
    let shore_event: ShoreEvent =
        serde_json::from_value(event.clone()).expect("unsigned envelope decodes");
    let signer = SignerId::parse(did_key).expect("did:key parses");
    let tbs = EventToBeSigned::from_event(&shore_event, &signer).expect("build to-be-signed view");
    let message = event_signature_pre_authentication_encoding(&tbs)
        .expect("DSSE pre-authentication encoding");
    let signature = SigningKey::from_bytes(seed).sign(&message);

    event["signature"] = json!({
        "alg": "ed25519",
        "sig": BASE64_STANDARD.encode(signature.to_bytes()),
        "sigVersion": 1,
    });
    event["signer"] = if event["writer"]["actorId"] == json!(did_key) {
        Value::Null
    } else {
        json!(did_key)
    };
    event
}

fn negative_crypto_cases(friendly: &Value) -> Value {
    let valid_sig = friendly["signature"]["sig"]
        .as_str()
        .expect("friendly fixture is signed");

    let mut truncated = friendly.clone();
    truncated["signature"]["sig"] = json!(valid_sig.trim_end_matches('='));

    let mut over_long = friendly.clone();
    over_long["signature"]["sig"] = json!(format!("{}AAA=", valid_sig.trim_end_matches('=')));

    // did:key encoding of the all-zero Ed25519 public key; the signature is
    // the friendly one, so only key validation can reject it.
    let mut all_zero_key = friendly.clone();
    all_zero_key["signer"] = json!(SignerId::from_ed25519_public_key([0u8; 32]).as_str());

    json!({
        "cases": [
            {
                "expected": "invalid",
                "file": "unsupported-alg-event.json",
                "name": "unsupported_alg",
            },
            {
                "expected": "invalid",
                "file": "unsupported-sig-version-event.json",
                "name": "unsupported_sig_version",
            },
            {
                "event": truncated,
                "expected": "invalid",
                "name": "truncated_signature",
            },
            {
                "event": over_long,
                "expected": "invalid",
                "name": "over_long_signature",
            },
            {
                "event": all_zero_key,
                "expected": "invalid",
                "name": "all_zero_public_key",
            },
            {
                "didKey": "did:key:z6MkiTBz1AcnfJ9q6G9iP2LP6pJZtbdTFbFiB5k4qqQCrRZ",
                "expected": "invalid",
                "name": "small_order_public_key",
            },
            {
                "didKey": "did:key:z6MkiTBz1AcnfJ9q6G9iP2LP6pJZtbdTFbFiB5k4qqQCrRZ1",
                "expected": "invalid",
                "name": "non_canonical_public_key",
            },
        ]
    })
}

fn sha256_prefixed(bytes: &[u8]) -> String {
    format!("sha256:{}", hex(&Sha256::digest(bytes)))
}

/// Canonical JSON hash: serde_json maps are key-sorted, so compact
/// serialization of a `Value` is the canonical byte form.
fn sha256_canonical_json_prefixed(value: &Value) -> String {
    let bytes = serde_json::to_vec(value).expect("serialize canonical json");
    sha256_prefixed(&bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
