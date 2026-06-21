use serde::Serialize;

pub use super::signature::{EffectiveSignerError, resolve_effective_signer};
use crate::canonical_hash::canonical_json_bytes;
use crate::crypto::SignerId;
use crate::error::Result;
use crate::session::event::{AssertionMode, EventTarget, ShoreEvent};

/// Dead Simple Signing Envelope (DSSE) payload type for v1 event signature bytes.
pub const EVENT_TO_BE_SIGNED_V1_PAYLOAD_TYPE: &str = "application/vnd.shore.event-tbs.v1+json";

/// Canonical producer-fact view used as the body for event signature bytes.
///
/// This is not the whole event minus its signature. It is the explicit set of
/// event-envelope facts that Shoreline signs with Dead Simple Signing Envelope
/// (DSSE) pre-authentication encoding.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventToBeSigned<'a> {
    pub schema: &'a str,
    pub version: u32,
    pub event_type: &'static str,
    pub event_id: &'a str,
    pub payload_hash: &'a str,
    pub target: &'a EventTarget,
    pub actor_id: &'a str,
    pub signer: SignerId,
    pub occurred_at: &'a str,
    pub assertion_mode: AssertionMode,
}

impl<'a> EventToBeSigned<'a> {
    /// Builds the canonical signing view from an event and its resolved effective signer.
    pub fn from_event(event: &'a ShoreEvent, signer: &SignerId) -> Result<Self> {
        event.validate_schema_version()?;

        Ok(Self {
            schema: event.schema.as_str(),
            version: event.version,
            event_type: event.event_type.as_str(),
            event_id: event.event_id.as_str(),
            payload_hash: event.payload_hash.as_str(),
            target: &event.target,
            actor_id: event.writer.actor_id.as_str(),
            signer: signer.clone(),
            occurred_at: event.occurred_at.as_str(),
            assertion_mode: event.assertion_mode,
        })
    }

    /// Serializes this signing view with Shoreline's canonical JSON byte contract.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        canonical_json_bytes(&serde_json::to_value(self)?)
    }
}

/// Builds Dead Simple Signing Envelope (DSSE) pre-authentication encoding bytes.
///
/// The returned bytes use the literal DSSE v1 format:
/// `DSSEv1 SP len(type) SP type SP len(body) SP body`.
pub fn pre_authentication_encoding(payload_type: &str, body: &[u8]) -> Vec<u8> {
    let payload_type = payload_type.as_bytes();
    let type_len = payload_type.len().to_string();
    let body_len = body.len().to_string();
    let mut bytes = Vec::with_capacity(
        b"DSSEv1 ".len()
            + type_len.len()
            + 1
            + payload_type.len()
            + 1
            + body_len.len()
            + 1
            + body.len(),
    );

    bytes.extend_from_slice(b"DSSEv1 ");
    bytes.extend_from_slice(type_len.as_bytes());
    bytes.push(b' ');
    bytes.extend_from_slice(payload_type);
    bytes.push(b' ');
    bytes.extend_from_slice(body_len.as_bytes());
    bytes.push(b' ');
    bytes.extend_from_slice(body);
    bytes
}

/// Builds the canonical event signing view after resolving the event's effective signer.
pub fn event_to_be_signed(
    event: &ShoreEvent,
) -> std::result::Result<EventToBeSigned<'_>, EffectiveSignerError> {
    let signer = resolve_effective_signer(event)?;
    EventToBeSigned::from_event(event, &signer)
        .map_err(|error| EffectiveSignerError::invalid(error.to_string()))
}

/// Builds Dead Simple Signing Envelope (DSSE) pre-authentication encoding for an event signature.
pub fn event_signature_pre_authentication_encoding(tbs: &EventToBeSigned<'_>) -> Result<Vec<u8>> {
    Ok(pre_authentication_encoding(
        EVENT_TO_BE_SIGNED_V1_PAYLOAD_TYPE,
        &tbs.canonical_bytes()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        EVENT_TO_BE_SIGNED_V1_PAYLOAD_TYPE, EventToBeSigned, event_to_be_signed,
        pre_authentication_encoding, resolve_effective_signer,
    };
    use crate::crypto::{EventVerificationStatus, SignerId};
    use crate::model::{ActorId, EventId};
    use crate::session::event::{
        AssertionMode, EventType, IngestProvenance, IngestVia, ShoreEvent, SourceRef,
    };

    const FRIENDLY_SIGNER: &str = "did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd";

    fn fixture_event() -> ShoreEvent {
        serde_json::from_str(include_str!(
            "../../../tests/fixtures/event_signatures/friendly-valid-event.json"
        ))
        .expect("fixture event decodes")
    }

    fn fixture_bytes(name: &str) -> Vec<u8> {
        let mut bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/event_signatures")
                .join(name),
        )
        .expect("fixture bytes are readable");
        if bytes.last() == Some(&b'\n') {
            bytes.pop();
        }
        bytes
    }

    #[test]
    fn event_to_be_signed_canonical_bytes_match_fixture() {
        let event = fixture_event();
        let signer = SignerId::parse(FRIENDLY_SIGNER).unwrap();
        let tbs = EventToBeSigned::from_event(&event, &signer).unwrap();

        assert_eq!(
            tbs.canonical_bytes().unwrap(),
            fixture_bytes("canonical-tbs-v1.bytes")
        );
    }

    #[test]
    fn event_to_be_signed_excludes_payload_and_hop_metadata() {
        let mut event = fixture_event();
        event.source_ref = Some(SourceRef::new("remote", "evt:remote"));
        event.ingest = Some(IngestProvenance {
            via: IngestVia::IngestEvents,
            received_at: "unix-ms:1760000000000".to_owned(),
        });
        let signer = SignerId::parse(FRIENDLY_SIGNER).unwrap();
        let tbs = EventToBeSigned::from_event(&event, &signer).unwrap();

        let value = serde_json::from_slice::<serde_json::Value>(&tbs.canonical_bytes().unwrap())
            .expect("to-be-signed view is JSON");

        assert!(value.get("payload").is_none());
        assert!(value.get("sourceRef").is_none());
        assert!(value.get("signature").is_none());
        assert!(value.get("sigVersion").is_none());
        assert!(value.get("role").is_none());
        assert!(value.get("ingest").is_none());
    }

    #[test]
    fn stamped_event_canonical_tbs_bytes_match_unstamped_fixture() {
        // The ingest stamp is hop-added metadata: stamping the golden fixture event
        // must leave the canonical to-be-signed bytes byte-identical.
        let mut event = fixture_event();
        event.ingest = Some(IngestProvenance {
            via: IngestVia::BundleApply,
            received_at: "unix-ms:1760000000000".to_owned(),
        });
        let signer = SignerId::parse(FRIENDLY_SIGNER).unwrap();
        let tbs = EventToBeSigned::from_event(&event, &signer).unwrap();

        assert_eq!(
            tbs.canonical_bytes().unwrap(),
            fixture_bytes("canonical-tbs-v1.bytes")
        );
    }

    #[test]
    fn friendly_actor_signed_event_requires_top_level_signer() {
        let mut event = fixture_event();
        event.signer = None;

        let error = resolve_effective_signer(&event).unwrap_err();

        assert_eq!(error.status(), EventVerificationStatus::Invalid);
    }

    #[test]
    fn self_certifying_actor_omits_signer_but_to_be_signed_view_binds_resolved_did_key() {
        let mut event = fixture_event();
        event.writer.actor_id = ActorId::new(FRIENDLY_SIGNER);
        event.signer = None;

        let signer = resolve_effective_signer(&event).unwrap();
        let tbs = EventToBeSigned::from_event(&event, &signer).unwrap();

        assert_eq!(tbs.signer.as_str(), event.writer.actor_id.as_str());
    }

    #[test]
    fn event_to_be_signed_wrapper_binds_top_level_signer() {
        let event = fixture_event();

        let tbs = event_to_be_signed(&event).unwrap();

        assert_eq!(tbs.signer.as_str(), FRIENDLY_SIGNER);
    }

    #[test]
    fn every_current_event_type_builds_to_be_signed_view_without_family_specific_code() {
        let signer = SignerId::parse(FRIENDLY_SIGNER).unwrap();

        for event in all_fixture_event_families() {
            EventToBeSigned::from_event(&event, &signer).unwrap_or_else(|error| {
                panic!(
                    "{:?} must build to-be-signed view: {error}",
                    event.event_type
                )
            });
        }
    }

    #[test]
    fn validation_check_recorded_builds_to_be_signed_view() {
        let signer = SignerId::parse(FRIENDLY_SIGNER).unwrap();
        let event = all_fixture_event_families()
            .into_iter()
            .find(|event| event.event_type == EventType::ValidationCheckRecorded)
            .expect("validation fixture event included");

        let tbs = EventToBeSigned::from_event(&event, &signer).unwrap();

        assert_eq!(tbs.event_type, "validation_check_recorded");
        assert_eq!(tbs.assertion_mode, AssertionMode::Advisory);
    }

    #[test]
    fn pre_authentication_encoding_uses_literal_dsse_format_and_payload_type() {
        let body = br#"{"schema":"shore.event"}"#;

        assert_eq!(
            pre_authentication_encoding(EVENT_TO_BE_SIGNED_V1_PAYLOAD_TYPE, body),
            b"DSSEv1 39 application/vnd.shore.event-tbs.v1+json 24 {\"schema\":\"shore.event\"}"
                .to_vec()
        );
    }

    fn all_fixture_event_families() -> Vec<ShoreEvent> {
        let base = fixture_event();

        [
            EventType::ReviewInitialized,
            EventType::WorkObjectProposed,
            EventType::ReviewObservationRecorded,
            EventType::ReviewAssessmentRecorded,
            EventType::InputRequestOpened,
            EventType::InputRequestResponded,
            EventType::ReviewNoteImported,
            EventType::ReviewUnitLineageDeclared,
            EventType::ReviewUnitLineageRoundRecorded,
            EventType::ReviewUnitRefAssociated,
            EventType::ReviewUnitRefWithdrawn,
            EventType::ReviewUnitCommitAssociated,
            EventType::ReviewUnitCommitWithdrawn,
            EventType::ValidationCheckRecorded,
            EventType::TaskCheckpointCaptured,
            EventType::TaskObservationRecorded,
            EventType::EventSignatureRecorded,
            EventType::ArtifactRemoved,
        ]
        .into_iter()
        .enumerate()
        .map(|(idx, event_type)| {
            let mut event = base.clone();
            event.event_type = event_type;
            event.event_id = EventId::new(format!("evt:sha256:{idx:064x}"));
            event
        })
        .collect()
    }
}
