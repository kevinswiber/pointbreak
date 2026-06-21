use std::fmt;
use std::sync::{Arc, Mutex};

use crate::crypto::{EventSigner, SignerId};
use crate::error::Result;
use crate::session::event::{
    EventSignature, EventToBeSigned, ShoreEvent, event_signature_pre_authentication_encoding,
};

type SharedEventSigner = Arc<dyn EventSigner + Send + Sync>;

/// A write-once slot a best-effort signing degrade records its reason into, so the
/// caller can surface why an event was left unsigned without the write gating.
pub type BestEffortSkipSink = Arc<Mutex<Option<String>>>;

#[derive(Clone, Default)]
pub struct EventSigningOptions {
    signer_id: Option<SignerId>,
    signer: Option<SharedEventSigner>,
    best_effort: bool,
    skip_sink: Option<BestEffortSkipSink>,
}

impl EventSigningOptions {
    pub fn sign_with<S>(signer: S) -> Self
    where
        S: EventSigner + Send + Sync + 'static,
    {
        let signer_id = signer.signer_id().clone();

        Self {
            signer_id: Some(signer_id),
            signer: Some(Arc::new(signer)),
            best_effort: false,
            skip_sink: None,
        }
    }

    /// Sign **best-effort**: if `sign_event_message` fails at write time, the event
    /// is left unsigned (the write does NOT gate) and the reason is recorded into
    /// `skip_sink`. Used for the network-backed agent signer, whose sign can fail
    /// at the real sign even though the resolve-layer pre-flight passed. The strict
    /// `sign_with` path is unaffected — its signer errors still propagate.
    pub fn sign_with_best_effort<S>(signer: S, skip_sink: BestEffortSkipSink) -> Self
    where
        S: EventSigner + Send + Sync + 'static,
    {
        let signer_id = signer.signer_id().clone();

        Self {
            signer_id: Some(signer_id),
            signer: Some(Arc::new(signer)),
            best_effort: true,
            skip_sink: Some(skip_sink),
        }
    }

    fn signer(&self) -> Option<&SharedEventSigner> {
        self.signer.as_ref()
    }
}

impl fmt::Debug for EventSigningOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventSigningOptions")
            .field("signer_id", &self.signer_id)
            .finish_non_exhaustive()
    }
}

impl PartialEq for EventSigningOptions {
    fn eq(&self, other: &Self) -> bool {
        self.signer_id == other.signer_id
    }
}

impl Eq for EventSigningOptions {}

pub(crate) fn sign_event_if_requested(
    event: &mut ShoreEvent,
    signing: &EventSigningOptions,
) -> Result<()> {
    let Some(signer) = signing.signer() else {
        return Ok(());
    };

    let signer_id = signer.signer_id();
    let signing_view = EventToBeSigned::from_event(event, signer_id)?;
    let message = event_signature_pre_authentication_encoding(&signing_view)?;
    // The only degraded error is a best-effort signer's sign failure: the
    // network-backed agent signer can fail at the real sign even though the
    // resolve-layer pre-flight passed (per-key confirmation deny, or the agent
    // dying/locking in the resolve→sign window). Leaving the event unsigned keeps
    // the write from gating. Everything above (event construction, serialization)
    // still propagates via `?`, and a strict signer's error propagates below — so a
    // genuine bug is never masked.
    let signature = match signer.sign_event_message(&message) {
        Ok(signature) => signature,
        Err(error) if signing.best_effort => {
            if let Some(sink) = &signing.skip_sink
                && let Ok(mut slot) = sink.lock()
            {
                *slot = Some(error.to_string());
            }
            return Ok(());
        }
        Err(error) => return Err(error),
    };

    if event.writer.actor_id.as_str() == signer_id.as_str() {
        event.signer = None;
    } else {
        event.signer = Some(signer_id.clone());
    }
    event.signature = Some(EventSignature::ed25519_v1(signature));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::EventSignatureBytes;
    use crate::keys::FileEd25519Signer;
    use crate::model::LedgerId;
    use crate::session::event::{EventTarget, EventType, ReviewInitializedPayload, Writer};

    /// A signer that always fails to sign — models the network agent refusing the
    /// real sign. Its `signer_id` is a valid did:key so event construction succeeds
    /// and only the sign step fails.
    struct AlwaysErrSigner {
        signer_id: SignerId,
    }

    impl AlwaysErrSigner {
        fn new() -> Self {
            Self {
                signer_id: SignerId::from_ed25519_public_key([3_u8; 32]),
            }
        }
    }

    impl EventSigner for AlwaysErrSigner {
        fn signer_id(&self) -> &SignerId {
            &self.signer_id
        }
        fn sign_event_message(&self, _message: &[u8]) -> Result<EventSignatureBytes> {
            Err(crate::error::ShoreError::Message(
                "agent refused the sign".to_owned(),
            ))
        }
    }

    fn sample_event() -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewInitialized,
            "review_initialized:session:t:work:default",
            EventTarget::for_ledger(LedgerId::new("session:t")),
            Writer::shore_local("0.1.0"),
            ReviewInitializedPayload {},
            "2026-05-10T00:00:00Z",
        )
        .expect("event builds")
    }

    fn skip_sink() -> BestEffortSkipSink {
        Arc::new(Mutex::new(None))
    }

    #[test]
    fn best_effort_sign_failure_leaves_the_event_unsigned_and_does_not_gate() {
        let mut event = sample_event();
        let sink = skip_sink();
        let options =
            EventSigningOptions::sign_with_best_effort(AlwaysErrSigner::new(), sink.clone());

        // The write does NOT gate: a best-effort sign failure returns Ok.
        sign_event_if_requested(&mut event, &options).expect("best-effort failure must not gate");
        assert!(event.signature.is_none(), "the event is left unsigned");
        assert!(
            sink.lock().unwrap().is_some(),
            "the skip reason is recorded for the caller to surface"
        );
    }

    #[test]
    fn strict_sign_failure_still_propagates() {
        // CONTROL: a strict signer that errors at sign time must still propagate —
        // the degrade is best-effort-scoped and never masks a real bug.
        let mut event = sample_event();
        let options = EventSigningOptions::sign_with(AlwaysErrSigner::new());
        assert!(
            sign_event_if_requested(&mut event, &options).is_err(),
            "the strict path is unchanged — a signer error still propagates"
        );
    }

    #[test]
    fn best_effort_sign_success_attaches_the_signature() {
        // A healthy best-effort signer signs normally — best-effort only changes the
        // failure behavior, not the happy path.
        let mut event = sample_event();
        let sink = skip_sink();
        let options = EventSigningOptions::sign_with_best_effort(
            FileEd25519Signer::from_seed([9_u8; 32]),
            sink.clone(),
        );

        sign_event_if_requested(&mut event, &options).expect("a healthy sign succeeds");
        assert!(event.signature.is_some(), "the event is signed");
        assert!(sink.lock().unwrap().is_none(), "no skip reason on success");
    }
}
