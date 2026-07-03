use serde::Serialize;

use crate::canonical_hash::sha256_json_prefixed;
use crate::error::Result;
use crate::session::event::type_code::type_code;
use crate::session::event::{AssertionMode, EventTarget, ShoreEvent, Writer};

/// Signature-exclusive view of a stored event record.
///
/// `eventRecordHash` (ADR-0008 §Event-Set Root) hashes this view: the whole
/// stored record EXCLUDING `signer`, `signature`, `sourceRef`, `ingest`,
/// `contentEncoding`, and `payloadVersion`. It therefore includes `payload`,
/// `idempotencyKey`, `target`, `writer`, `occurredAt`, `payloadHash`, and
/// `assertionMode`. It is a third digest, distinct from `payloadHash` (payload
/// only) and the signer-inclusive `EventToBeSigned` (TBS) view.
///
/// `ingest` is excluded because it is **per-hop/per-mirror metadata** (like `sourceRef`):
/// `ingest_events` stamps it before storage, so a local copy is unstamped while an
/// ingested copy carries a per-hop `received_at`. A hash covering `ingest` would make two
/// mirrors' copies of one fact diverge — breaking class-(b) transcription and
/// eventSetRoot convergence. (This excludes `ingest` in addition to ADR-0008's written
/// list; that exclusion is a post-approval ADR correction the convergence claim requires.)
///
/// `contentEncoding` and `payloadVersion` are excluded because they describe how the
/// record is stored (content-coding tokens) and how its payload is read (the view
/// version a read-time upcast dispatches on), not what the fact is. Identity is
/// computed over the decoded content, so re-encoding a record or bumping its view
/// version must never re-mint its identity (see ADR-0004's storage-descriptor
/// amendment).
///
/// This view is **exhaustively** the `ShoreEvent` envelope minus exactly six fields
/// (`signer`, `signature`, `sourceRef`, `ingest`, `contentEncoding`, `payloadVersion`).
/// If a field is added to `ShoreEvent`, the include/exclude decision must be made here
/// (the `EventToBeSigned` precedent).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventRecordView<'a> {
    pub schema: &'a str,
    pub version: u32,
    pub event_id: &'a str,
    pub event_type: &'static str,
    pub idempotency_key: &'a str,
    pub target: &'a EventTarget,
    pub writer: &'a Writer,
    pub occurred_at: &'a str,
    pub payload_hash: &'a str,
    pub assertion_mode: AssertionMode,
    pub payload: &'a serde_json::Value,
}

impl<'a> EventRecordView<'a> {
    pub fn from_event(event: &'a ShoreEvent) -> Self {
        Self {
            schema: event.schema.as_str(),
            version: event.version,
            event_id: event.event_id.as_str(),
            event_type: type_code(event.event_type),
            idempotency_key: event.idempotency_key.as_str(),
            target: &event.target,
            writer: &event.writer,
            occurred_at: event.occurred_at.as_str(),
            payload_hash: event.payload_hash.as_str(),
            assertion_mode: event.assertion_mode,
            payload: &event.payload,
        }
    }

    /// Computes `eventRecordHash` as `"sha256:" + hex` over the canonical JSON of this view.
    pub fn event_record_hash(&self) -> Result<String> {
        sha256_json_prefixed(&serde_json::to_value(self)?)
    }
}

#[cfg(test)]
mod tests {
    use crate::crypto::SignerId;
    use crate::session::event::record_hash::EventRecordView;
    use crate::session::event::{
        EventSignature, IngestProvenance, IngestVia, ShoreEvent, SourceRef,
    };

    const FRIENDLY_SIGNER: &str = "did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd";
    const FIXTURE_SIG: &str =
        "3eSMZ69v33GzpAy+qiGTnMFPuMW3CbQPC9c/aY4kPONmdGT2T3tup0UwMtTeZPm+6U02FRWnlP073JFodXm2AQ==";

    fn fixture_event() -> ShoreEvent {
        serde_json::from_str(include_str!(
            "../../../tests/fixtures/event_signatures/friendly-valid-event.json"
        ))
        .expect("fixture event decodes")
    }

    #[test]
    fn signed_and_unsigned_copies_share_event_record_hash() {
        let mut signed = fixture_event();
        signed.signer = Some(SignerId::parse(FRIENDLY_SIGNER).unwrap());
        signed.signature = Some(EventSignature::new_ed25519_v1(FIXTURE_SIG).unwrap());

        let mut unsigned = fixture_event();
        unsigned.signer = None;
        unsigned.signature = None;

        assert_eq!(
            signed.event_record_hash().unwrap(),
            unsigned.event_record_hash().unwrap(),
            "a signed and an unsigned copy of the same fact share eventRecordHash"
        );
    }

    #[test]
    fn event_record_hash_differs_from_payload_hash() {
        let event = fixture_event();

        assert_ne!(
            event.event_record_hash().unwrap(),
            event.payload_hash,
            "eventRecordHash covers more than the payload, so it differs from payloadHash"
        );
    }

    #[test]
    fn changing_only_excluded_fields_does_not_change_hash() {
        let baseline = fixture_event().event_record_hash().unwrap();

        let mut only_signer = fixture_event();
        only_signer.signer = Some(SignerId::parse(FRIENDLY_SIGNER).unwrap());
        assert_eq!(only_signer.event_record_hash().unwrap(), baseline);

        let mut only_signature = fixture_event();
        only_signature.signature = Some(EventSignature::new_ed25519_v1(FIXTURE_SIG).unwrap());
        assert_eq!(only_signature.event_record_hash().unwrap(), baseline);

        let mut only_source_ref = fixture_event();
        only_source_ref.source_ref = Some(SourceRef::new("remote", "evt:remote"));
        assert_eq!(only_source_ref.event_record_hash().unwrap(), baseline);

        // Load-bearing: a local unstamped target and an ingested stamped copy of one
        // fact must hash identically (cross-mirror convergence for transcription + root).
        let mut only_ingest = fixture_event();
        only_ingest.ingest = Some(IngestProvenance {
            via: IngestVia::IngestEvents,
            received_at: "unix-ms:1760000000000".to_owned(),
        });
        assert_eq!(only_ingest.event_record_hash().unwrap(), baseline);

        // The storage-encoding descriptor and the payload view version describe
        // how the record is stored and read, not what the fact is: both are
        // hash-excluded so setting them never re-mints the record identity.
        let mut only_content_encoding = fixture_event();
        only_content_encoding.content_encoding = vec!["zstd".to_owned()];
        assert_eq!(only_content_encoding.event_record_hash().unwrap(), baseline);

        let mut only_payload_version = fixture_event();
        only_payload_version.payload_version = 7;
        assert_eq!(only_payload_version.event_record_hash().unwrap(), baseline);
    }

    #[test]
    fn changing_included_fields_changes_hash() {
        let baseline = fixture_event().event_record_hash().unwrap();

        let mut payload_mutated = fixture_event();
        payload_mutated.payload["title"] = serde_json::json!("Different title");
        assert_ne!(payload_mutated.event_record_hash().unwrap(), baseline);

        let mut occurred_at_mutated = fixture_event();
        occurred_at_mutated.occurred_at = "2099-01-01T00:00:00Z".to_owned();
        assert_ne!(occurred_at_mutated.event_record_hash().unwrap(), baseline);

        let mut target_mutated = fixture_event();
        target_mutated.target.journal_id = crate::model::JournalId::new("journal:fixture:mutated");
        assert_ne!(target_mutated.event_record_hash().unwrap(), baseline);
    }

    #[test]
    fn event_record_hash_is_sha256_prefixed() {
        let hash = fixture_event().event_record_hash().unwrap();

        assert!(hash.starts_with("sha256:"), "got {hash}");
        let hex = hash.strip_prefix("sha256:").unwrap();
        assert_eq!(hex.len(), 64, "got {hash}");
        assert!(
            hex.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "expected 64 lowercase hex chars, got {hash}"
        );
    }

    #[test]
    fn event_record_hash_golden_vector() {
        // Cross-store determinism regression lock against a fixed, fully-specified
        // fixture (no ingest). Signature fields are excluded, so the value is stable
        // regardless of whether the fixture carries an inline signature.
        let hash = fixture_event().event_record_hash().unwrap();

        assert_eq!(
            hash,
            "sha256:23933e6fe38b38b812f02cc3751a1c3117af0797d29b19b2b66ed8463dde323d",
        );
    }

    #[test]
    fn serialized_view_omits_signature_and_hop_metadata() {
        let mut event = fixture_event();
        event.content_encoding = vec!["zstd".to_owned()];
        event.payload_version = 7;
        let view = EventRecordView::from_event(&event);
        let value = serde_json::to_value(&view).expect("view serializes");

        assert!(value.get("signer").is_none());
        assert!(value.get("signature").is_none());
        assert!(value.get("sourceRef").is_none());
        assert!(value.get("ingest").is_none());
        assert!(value.get("contentEncoding").is_none());
        assert!(value.get("payloadVersion").is_none());
        // It must include payload + identity-bearing fields.
        assert!(value.get("payload").is_some());
        assert!(value.get("idempotencyKey").is_some());
        assert!(value.get("payloadHash").is_some());
    }
}
