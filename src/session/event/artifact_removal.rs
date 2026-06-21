use serde::{Deserialize, Serialize};

use super::kind::EventType;
use super::payload::EventPayload;

/// Content-targeted, reason-free removal fact. The payload carries **only** the
/// `content_hash` it retires; the writer and session ride on the event envelope
/// for provenance and never enter identity, so two peers removing the same blob
/// produce a byte-identical payload and converge rather than conflicting.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRemovedPayload {
    pub content_hash: String,
}

impl ArtifactRemovedPayload {
    pub fn idempotency_key(content_hash: &str) -> String {
        format!("artifact_removed:{content_hash}")
    }
}

impl EventPayload for ArtifactRemovedPayload {
    fn event_type(&self) -> EventType {
        EventType::ArtifactRemoved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::event::EventPayload;

    #[test]
    fn payload_round_trips_camel_case_and_reports_event_type() {
        let p = ArtifactRemovedPayload {
            content_hash: "sha256:abc".to_owned(),
        };
        assert_eq!(p.event_type(), EventType::ArtifactRemoved);
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["contentHash"], "sha256:abc");
        // Only one field — no reason, no review-unit, no envelope identity.
        assert_eq!(
            v.as_object().unwrap().keys().collect::<Vec<_>>(),
            vec!["contentHash"]
        );
        let back: ArtifactRemovedPayload = serde_json::from_value(v).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn idempotency_key_is_content_targeted() {
        assert_eq!(
            ArtifactRemovedPayload::idempotency_key("sha256:abc"),
            "artifact_removed:sha256:abc"
        );
    }
}

#[cfg(test)]
mod convergence_tests {
    use super::*;
    use crate::model::{ActorId, LedgerId};
    use crate::session::event::{EventTarget, EventType, ShoreEvent, Writer, WriterProducer};
    use crate::session::projection::freshness::event_set_hash_for_events;
    use crate::session::{EventStore, EventWriteOutcome};

    fn writer_for(name: &str) -> Writer {
        Writer {
            actor_id: ActorId::new(format!("actor:{name}")),
            producer: WriterProducer {
                name: "shore".to_owned(),
                version: "test".to_owned(),
            },
        }
    }

    fn removal_event(
        content_hash: &str,
        session: &str,
        writer: &str,
        occurred_at: &str,
    ) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ArtifactRemoved,
            ArtifactRemovedPayload::idempotency_key(content_hash),
            EventTarget::for_ledger(LedgerId::new(format!("session:{session}"))),
            writer_for(writer),
            ArtifactRemovedPayload {
                content_hash: content_hash.to_owned(),
            },
            occurred_at,
        )
        .unwrap()
    }

    #[test]
    fn same_content_hash_converges_across_sessions() {
        let a = removal_event("sha256:blob", "alpha", "alice", "2026-06-19T00:00:00Z");
        let b = removal_event("sha256:blob", "beta", "bob", "2026-06-19T09:09:09Z");

        assert_eq!(
            a.event_id, b.event_id,
            "event id is key-derived from the content hash"
        );
        assert_eq!(
            a.payload_hash, b.payload_hash,
            "writer/session/occurredAt not in payload"
        );
        assert_ne!(
            a.writer, b.writer,
            "envelope writer differs — and that is fine"
        );
        assert_ne!(
            a.occurred_at, b.occurred_at,
            "occurredAt differs — and is not arbitrated"
        );
        assert_eq!(
            event_set_hash_for_events([&a]).unwrap(),
            event_set_hash_for_events([&b]).unwrap(),
            "event-set contribution is identical"
        );
    }

    #[test]
    fn re_record_returns_existing_and_distinct_blob_is_a_new_member() {
        let root = tempfile::tempdir().unwrap();
        let store = EventStore::open(root.path().join(".shore/data"));

        let a = removal_event("sha256:blob", "alpha", "alice", "2026-06-19T00:00:00Z");
        let b = removal_event("sha256:blob", "beta", "bob", "2026-06-19T09:09:09Z");
        let other = removal_event("sha256:other", "alpha", "alice", "2026-06-19T00:00:00Z");

        assert_eq!(
            store.record_event_once(&a).unwrap(),
            EventWriteOutcome::Created
        );
        assert_eq!(
            store.record_event_once(&b).unwrap(),
            EventWriteOutcome::Existing,
            "same blob removed by another session converges to Existing (keep-first-stored)"
        );
        assert_eq!(
            store.record_event_once(&other).unwrap(),
            EventWriteOutcome::Created,
            "a distinct content hash is a new member, not a conflict"
        );

        // Keep-first-stored: the persisted record is `a`, never re-arbitrated by
        // `b`'s later occurredAt.
        let stored = store
            .list_events()
            .unwrap()
            .into_iter()
            .find(|event| event.idempotency_key == a.idempotency_key)
            .expect("the removal fact is persisted");
        assert_eq!(stored.occurred_at, a.occurred_at);
        assert_ne!(stored.occurred_at, b.occurred_at);
    }
}
