//! Golden event-signature vectors.
//!
//! The fixture set under `tests/fixtures/event_signatures/` is fully
//! reproducible from the seeds in `did-key-ed25519.json`. To regenerate after
//! an intentional contract change:
//!
//! ```sh
//! UPDATE_EVENT_SIGNATURE_FIXTURES=1 cargo nextest run \
//!     -E 'test(regenerate_event_signature_fixtures)' --run-ignored all
//! ```

use std::path::{Path, PathBuf};

use serde_json::Value;
use shoreline::crypto::SignerId;
use shoreline::model::{EventId, LedgerId};
use shoreline::session::event::{
    EventSignature, EventSignatureRecordedPayload, EventTarget, EventType, IngestProvenance,
    IngestVia, ShoreEvent, Writer,
};
use shoreline::session::{
    EventVerificationStatus, event_signature_pre_authentication_encoding,
    event_signature_trust_set, event_to_be_signed, verify_event_signature,
};

mod support;

use support::event_signature_fixtures::build_all_fixtures;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/event_signatures")
}

fn fixture_path(name: &str) -> PathBuf {
    fixture_dir().join(name)
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    let mut bytes = std::fs::read(fixture_path(name)).expect("read byte fixture");
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    bytes
}

fn fixture_json(name: &str) -> Value {
    let bytes = std::fs::read(fixture_path(name)).expect("read json fixture");
    serde_json::from_slice(&bytes).expect("fixture is valid json")
}

fn fixture_event(name: &str) -> ShoreEvent {
    serde_json::from_value(fixture_json(name)).expect("fixture event decodes")
}

const FRIENDLY_SIGNER: &str = "did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd";

fn carrier_event(idempotency_key: &str, payload: EventSignatureRecordedPayload) -> ShoreEvent {
    ShoreEvent::new(
        EventType::EventSignatureRecorded,
        idempotency_key,
        EventTarget::for_ledger(LedgerId::new("ledger:fixture")),
        Writer::shore_local("test"),
        payload,
        "2026-06-04T00:00:00Z",
    )
    .expect("carrier builds")
}

fn carrier_payload(signer: &SignerId, sig: &str) -> EventSignatureRecordedPayload {
    EventSignatureRecordedPayload {
        target_event_id: EventId::new("evt:sha256:target"),
        target_event_record_hash: "sha256:rec".to_owned(),
        attesting_signer: signer.clone(),
        attestation: EventSignature::new_ed25519_v1(sig).unwrap(),
        inclusion_proof: None,
    }
}

/// Cross-store determinism lock: the carrier `idempotencyKey` (hence its `eventId`)
/// derives from the full attestation triple `(targetEventRecordHash, attestingSigner,
/// signature)`. Two distinct signatures by one signer are two distinct members; an
/// identical triple is idempotent. Keying on `(target, signer)` would fail the
/// distinct-signature case (signer-slot poisoning).
#[test]
fn golden_cosignature_idempotency_key_derives_from_full_triple() {
    let signer = SignerId::parse(FRIENDLY_SIGNER).unwrap();

    let key = EventSignatureRecordedPayload::idempotency_key("sha256:rec", &signer, "SIG_BASE64");
    assert_eq!(
        key,
        "event_signature_recorded:sha256:rec:did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd:SIG_BASE64"
    );

    let key_a = EventSignatureRecordedPayload::idempotency_key("sha256:rec", &signer, "SIGAAA");
    let key_b = EventSignatureRecordedPayload::idempotency_key("sha256:rec", &signer, "SIGBBB");
    let event_a = carrier_event(&key_a, carrier_payload(&signer, "SIGAAA"));
    let event_b = carrier_event(&key_b, carrier_payload(&signer, "SIGBBB"));
    assert_ne!(
        event_a.event_id, event_b.event_id,
        "two distinct signatures by one signer are two distinct members"
    );

    let again = carrier_event(&key_a, carrier_payload(&signer, "SIGAAA"));
    assert_eq!(
        event_a.event_id, again.event_id,
        "an identical triple is idempotent"
    );
}

/// `eventRecordHash` signature-blindness at the public-API boundary: a signed and an
/// unsigned copy of one fact share it. The pinned literal is the cross-store
/// regression lock; the in-crate unit golden
/// (`session::event::record_hash::tests::event_record_hash_golden_vector`) covers the
/// same fixture, so this asserts the public surface, not a second contract.
#[test]
fn golden_event_record_hash_is_signature_blind() {
    let signed = fixture_event("friendly-valid-event.json");
    let mut unsigned = signed.clone();
    unsigned.signer = None;
    unsigned.signature = None;

    let signed_hash = signed.event_record_hash().unwrap();
    assert_eq!(signed_hash, unsigned.event_record_hash().unwrap());
    assert_eq!(
        signed_hash,
        "sha256:e04fb798786f1babe03708637673c1bd0492d872108dfe363957b916130a028b"
    );
}

#[test]
fn golden_to_be_signed_and_pre_authentication_encoding_bytes_match_fixtures() {
    let event = fixture_event("friendly-valid-event.json");
    let tbs = event_to_be_signed(&event).expect("build EventToBeSigned");

    assert_eq!(
        tbs.canonical_bytes()
            .expect("build canonical to-be-signed bytes"),
        fixture_bytes("canonical-tbs-v1.bytes")
    );
    assert_eq!(
        event_signature_pre_authentication_encoding(&tbs)
            .expect("build DSSE pre-authentication encoding bytes"),
        fixture_bytes("pae-v1.bytes")
    );
}

#[test]
fn golden_verification_statuses_match_fixtures() {
    assert_status("friendly-valid-event.json", EventVerificationStatus::Valid);
    assert_status(
        "source-speaker-valid-event.json",
        EventVerificationStatus::Valid,
    );
    assert_status(
        "source-speaker-mutated-event.json",
        EventVerificationStatus::Invalid,
    );
    assert_status(
        "self-certifying-valid-event.json",
        EventVerificationStatus::Valid,
    );
    assert_status("unsigned-event.json", EventVerificationStatus::Unsigned);
    assert_status(
        "unauthorized-signer-event.json",
        EventVerificationStatus::UntrustedKey,
    );
    assert_status(
        "payload-mutated-event.json",
        EventVerificationStatus::Invalid,
    );
    assert_status("actor-mutated-event.json", EventVerificationStatus::Invalid);
    assert_status(
        "target-mutated-event.json",
        EventVerificationStatus::Invalid,
    );
    assert_status(
        "timestamp-mutated-event.json",
        EventVerificationStatus::Invalid,
    );
    assert_status(
        "assertion-mode-mutated-event.json",
        EventVerificationStatus::Invalid,
    );
    assert_status(
        "unsupported-alg-event.json",
        EventVerificationStatus::Invalid,
    );
    assert_status(
        "unsupported-sig-version-event.json",
        EventVerificationStatus::Invalid,
    );
}

#[test]
fn stamped_signed_fixture_event_still_verifies_valid() {
    // ADR-0009: the ingest stamp is outside the to-be-signed view, so stamping
    // a signed event cannot invalidate its signature.
    let mut event = fixture_event("friendly-valid-event.json");
    event.ingest = Some(IngestProvenance {
        via: IngestVia::IngestEvents,
        received_at: "unix-ms:1760000000000".to_owned(),
    });
    let trust_set =
        event_signature_trust_set(fixture_json("did-key-ed25519.json")).expect("build trust set");

    assert_eq!(
        verify_event_signature(&event, &trust_set).expect("verify stamped fixture event"),
        EventVerificationStatus::Valid
    );
}

#[test]
fn vector_fixture_inventory_covers_required_case_families() {
    for required in [
        "canonical-tbs-v1.json",
        "canonical-tbs-v1.bytes",
        "pae-v1.bytes",
        "did-key-ed25519.json",
        "friendly-valid-event.json",
        "self-certifying-valid-event.json",
        "unsigned-event.json",
        "unauthorized-signer-event.json",
        "payload-mutated-event.json",
        "actor-mutated-event.json",
        "target-mutated-event.json",
        "timestamp-mutated-event.json",
        "assertion-mode-mutated-event.json",
        "source-speaker-valid-event.json",
        "source-speaker-mutated-event.json",
        "unsupported-alg-event.json",
        "unsupported-sig-version-event.json",
        "mutation-cases.json",
        "negative-crypto-cases.json",
    ] {
        assert!(
            fixture_path(required).is_file(),
            "missing event signature fixture {required}"
        );
    }

    let did_key = fixture_json("did-key-ed25519.json");
    assert_eq!(
        did_key["didKey"].as_str(),
        Some("did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd")
    );
    assert_eq!(
        did_key["publicKeyHex"].as_str(),
        Some("03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8")
    );

    let mutation_cases = fixture_json("mutation-cases.json");
    let mutation_names = mutation_cases["cases"]
        .as_array()
        .expect("mutation cases are an array")
        .iter()
        .map(|case| case["file"].as_str().expect("mutation file"))
        .collect::<Vec<_>>();
    for required in [
        "payload-mutated-event.json",
        "actor-mutated-event.json",
        "target-mutated-event.json",
        "timestamp-mutated-event.json",
        "assertion-mode-mutated-event.json",
        "source-speaker-mutated-event.json",
        "unauthorized-signer-event.json",
    ] {
        assert!(mutation_names.contains(&required));
    }

    let negative_cases = fixture_json("negative-crypto-cases.json");
    let negative_names = negative_cases["cases"]
        .as_array()
        .expect("negative crypto cases are an array")
        .iter()
        .map(|case| case["name"].as_str().expect("negative case name"))
        .collect::<Vec<_>>();
    for required in [
        "unsupported_alg",
        "unsupported_sig_version",
        "truncated_signature",
        "over_long_signature",
        "all_zero_public_key",
        "small_order_public_key",
        "non_canonical_public_key",
    ] {
        assert!(negative_names.contains(&required));
    }
}

/// ADR-0010 consequence check: the `writer.tool` → `writer.producer` rename is
/// envelope spelling only. The signed view excludes the producer fact, so the
/// golden TBS/PAE bytes are byte-identical to the pre-rename fixtures, every
/// `sigVersion` stays 1, and no envelope fixture carries a `tool` key.
#[test]
fn producer_rename_left_signed_material_untouched() {
    use sha2::{Digest, Sha256};

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    // Digests pinned to the current signed envelope shape. Regeneration must
    // reproduce these bytes exactly; an unexpected change here is a
    // stop-the-line signal that an edit touched the signed material.
    const CANONICAL_TBS_SHA256: &str =
        "7a0f9904caae98242ae46a3d9e7e51f329eda98372465c11f167cc3972d8a65d";
    const PAE_SHA256: &str = "6de453dccddfce319d0e14097dead678bfd5b181e46cb04471f1d008269670d4";

    let canonical_tbs =
        std::fs::read(fixture_path("canonical-tbs-v1.bytes")).expect("read canonical tbs bytes");
    let pae = std::fs::read(fixture_path("pae-v1.bytes")).expect("read pae bytes");
    assert_eq!(
        sha256_hex(&canonical_tbs),
        CANONICAL_TBS_SHA256,
        "canonical-tbs-v1.bytes must reproduce the pinned digest"
    );
    assert_eq!(
        sha256_hex(&pae),
        PAE_SHA256,
        "pae-v1.bytes must reproduce the pinned digest"
    );

    let mut envelope_fixtures = std::fs::read_dir(fixture_dir())
        .expect("read fixture dir")
        .map(|entry| entry.expect("dir entry").file_name())
        .filter_map(|name| name.to_str().map(str::to_owned))
        .filter(|name| name.ends_with("-event.json"))
        .collect::<Vec<_>>();
    envelope_fixtures.sort();
    assert!(
        !envelope_fixtures.is_empty(),
        "expected envelope fixtures to walk"
    );

    for name in envelope_fixtures {
        let raw = std::fs::read_to_string(fixture_path(&name)).expect("read envelope fixture");
        assert!(
            !raw.contains("\"tool\""),
            "envelope fixture {name} must carry no tool key after the producer rename"
        );
        let event: Value = serde_json::from_str(&raw).expect("envelope fixture is valid json");
        let writer = event["writer"]
            .as_object()
            .unwrap_or_else(|| panic!("{name} has a writer object"));
        assert!(
            writer.get("tool").is_none() && writer.contains_key("producer"),
            "writer in {name} carries producer and no tool key"
        );
        // `unsupported-sig-version-event.json` deliberately carries an
        // unsupported sigVersion as a negative vector; every other signed
        // fixture must keep sigVersion 1 through the rename.
        if name != "unsupported-sig-version-event.json"
            && let Some(signature) = event.get("signature").and_then(Value::as_object)
        {
            assert_eq!(
                signature["sigVersion"], 1,
                "every signed envelope in {name} keeps sigVersion 1"
            );
        }
    }
}

#[test]
fn regenerated_fixture_bytes_are_deterministic_and_match_checked_in_fixtures() {
    let first = build_all_fixtures(&fixture_dir());
    let second = build_all_fixtures(&fixture_dir());
    assert_eq!(
        first.file_names(),
        second.file_names(),
        "fixture builder is deterministic"
    );

    for name in first.file_names() {
        assert_eq!(
            first.bytes(&name),
            second.bytes(&name),
            "fixture builder output for {name} is deterministic"
        );
        let on_disk = std::fs::read(fixture_path(&name)).expect("fixture file readable");
        assert_eq!(
            first.bytes(&name),
            &on_disk[..],
            "checked-in fixture {name} is reproducible from the builders"
        );
    }
}

#[test]
#[ignore = "regenerates golden fixtures; run with UPDATE_EVENT_SIGNATURE_FIXTURES=1"]
fn regenerate_event_signature_fixtures() {
    if std::env::var("UPDATE_EVENT_SIGNATURE_FIXTURES").is_err() {
        return;
    }
    let fixtures = build_all_fixtures(&fixture_dir());
    for name in fixtures.file_names() {
        std::fs::write(fixture_path(&name), fixtures.bytes(&name)).expect("write fixture");
    }
}

fn assert_status(fixture: &str, expected: EventVerificationStatus) {
    let event = fixture_event(fixture);
    let trust_set =
        event_signature_trust_set(fixture_json("did-key-ed25519.json")).expect("build trust set");

    assert_eq!(
        verify_event_signature(&event, &trust_set).expect("verify fixture event"),
        expected
    );
}
