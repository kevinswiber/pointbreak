use std::collections::BTreeSet;

use crate::error::Result;
use crate::session::event::{ArtifactRemovedPayload, EventType, ShoreEvent};

/// Read-time projection of which content-addressed blobs have been removed. A
/// pure function of the event set; nothing new is stored. Hashes are normalized
/// `sha256:<hex>` exactly as written in the payload.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ArtifactRemovalProjection {
    pub removed: BTreeSet<String>,
}

impl ArtifactRemovalProjection {
    pub fn from_events(events: &[ShoreEvent]) -> Result<Self> {
        let mut removed = BTreeSet::new();
        for event in events {
            if event.event_type == EventType::ArtifactRemoved {
                let payload: ArtifactRemovedPayload =
                    serde_json::from_value(event.payload.clone())?;
                removed.insert(payload.content_hash);
            }
        }
        Ok(Self { removed })
    }

    pub fn is_removed(&self, content_hash: &str) -> bool {
        self.removed.contains(content_hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ActorId, LedgerId};
    use crate::session::event::{EventTarget, Writer, WriterProducer};

    fn removal_event(content_hash: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ArtifactRemoved,
            ArtifactRemovedPayload::idempotency_key(content_hash),
            EventTarget::for_ledger(LedgerId::new("session:fixture")),
            Writer {
                actor_id: ActorId::new("actor:fixture"),
                producer: WriterProducer {
                    name: "shore".to_owned(),
                    version: "test".to_owned(),
                },
            },
            ArtifactRemovedPayload {
                content_hash: content_hash.to_owned(),
            },
            "2026-06-19T00:00:00Z",
        )
        .unwrap()
    }

    #[test]
    fn from_events_collects_removed_content_hashes() {
        let events = vec![removal_event("sha256:a"), removal_event("sha256:b")];
        let projection = ArtifactRemovalProjection::from_events(&events).unwrap();
        assert!(projection.is_removed("sha256:a"));
        assert!(projection.is_removed("sha256:b"));
        assert!(!projection.is_removed("sha256:c"));
    }

    #[test]
    fn from_events_ignores_non_removal_events() {
        // Any non-removal event contributes nothing to the removed set.
        let mut other = removal_event("sha256:anything");
        other.event_type = EventType::ReviewInitialized;
        let projection = ArtifactRemovalProjection::from_events(&[other]).unwrap();
        assert!(!projection.is_removed("sha256:anything"));
        assert!(projection.removed.is_empty());
    }
}
