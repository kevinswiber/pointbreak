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
use shoreline::session::event::ShoreEvent;
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
