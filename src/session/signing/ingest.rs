use super::{EventVerificationPolicy, TrustSet, verify_event_signature};
use crate::crypto::EventVerificationStatus;
use crate::error::{Result, ShoreError};
use crate::model::EventId;
use crate::session::event::{EventType, ShoreEvent};
use crate::session::store::EventWriteOutcome;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IngestEventVerification {
    pub event_id: EventId,
    pub status: EventVerificationStatus,
    pub message: Option<String>,
    /// `Some` when the surface reports how the store resolved this event's write;
    /// `None` when the surface reports no per-row outcome — a pre-write rejection,
    /// or an internal surface that does not thread outcomes (bundle import commits
    /// events but returns the untouched verify-pass rows). `None` means "not
    /// reported", not "not written".
    pub write_outcome: Option<EventWriteOutcome>,
}

pub(crate) fn verify_events_for_ingest(
    events: &[ShoreEvent],
    policy: EventVerificationPolicy,
    trust_set: &TrustSet,
) -> Result<Vec<IngestEventVerification>> {
    let mut verification = Vec::with_capacity(events.len());

    for event in events {
        // Detached co-signature carriers are classified by the co-signature gate
        // (against the embedded attestation), not by the inline envelope verifier:
        // a v1 carrier envelope is unsigned, so `verify_event_signature` would read
        // it as `Unsigned` and a strict policy would wrongly reject the carrier. The
        // gate in `ingest_events` owns the family's per-member status.
        if event.event_type == EventType::EventSignatureRecorded {
            continue;
        }
        let status = verify_event_signature(event, trust_set)?;
        let report = IngestEventVerification {
            event_id: event.event_id.clone(),
            status,
            message: verification_message(status),
            write_outcome: None,
        };

        if policy.rejects(status) {
            return Err(ShoreError::EventVerificationRejected {
                event_id: event.event_id.clone(),
                status,
            });
        }

        verification.push(report);
    }

    Ok(verification)
}

fn verification_message(status: EventVerificationStatus) -> Option<String> {
    match status {
        EventVerificationStatus::Valid => None,
        EventVerificationStatus::Invalid => Some("event signature is invalid".to_owned()),
        EventVerificationStatus::UntrustedKey => {
            Some("event signer is not authorized by the trust set".to_owned())
        }
        EventVerificationStatus::Unsigned => Some("event is unsigned".to_owned()),
    }
}
