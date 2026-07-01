use serde::Serialize;

use crate::canonical_hash::sha256_json_prefixed;
use crate::error::Result;
use crate::session::event::ShoreEvent;

const EVENT_SET_HASH_SCHEMA: &str = "shore.event-set.v1";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EventSetHashMaterial<'a> {
    schema: &'static str,
    events: Vec<EventSetHashEntry<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EventSetHashEntry<'a> {
    event_id: &'a str,
    payload_hash: &'a str,
}

pub(crate) fn event_set_hash_for_events<'a>(
    events: impl IntoIterator<Item = &'a ShoreEvent>,
) -> Result<String> {
    let mut entries = events
        .into_iter()
        .map(|event| EventSetHashEntry {
            event_id: event.event_id.as_str(),
            payload_hash: event.payload_hash.as_str(),
        })
        .collect::<Vec<_>>();

    // Callers pass EventStore-validated events, where event IDs are unique;
    // duplicate entries would be a different supplied event set and are not collapsed here.
    entries.sort_by_key(|entry| (entry.event_id, entry.payload_hash));

    let material = serde_json::to_value(EventSetHashMaterial {
        schema: EVENT_SET_HASH_SCHEMA,
        events: entries,
    })?;
    sha256_json_prefixed(&material)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::JournalId;
    use crate::session::event::{
        EventTarget, EventType, ReviewInitializedPayload, ShoreEvent, Writer,
    };

    #[test]
    fn event_set_hash_is_order_independent() {
        let mut first = event("one");
        first.payload_hash = format!("sha256:{}", "1".repeat(64));
        let mut second = event("two");
        second.payload_hash = format!("sha256:{}", "2".repeat(64));

        let forward = event_set_hash_for_events(&[first.clone(), second.clone()]).unwrap();
        let reversed = event_set_hash_for_events(&[second, first]).unwrap();

        assert_eq!(forward, reversed);
        assert!(forward.starts_with("sha256:"));
    }

    #[test]
    fn event_set_hash_changes_when_payload_hash_changes() {
        let original = event("one");
        let mut changed = original.clone();
        changed.payload_hash = "sha256:different".to_owned();

        assert_ne!(
            event_set_hash_for_events(&[original]).unwrap(),
            event_set_hash_for_events(&[changed]).unwrap()
        );
    }

    #[test]
    fn event_set_hash_ignores_envelope_only_changes() {
        let original = event("one");
        let mut changed = original.clone();
        changed.occurred_at = "2026-05-13T15:00:00Z".to_owned();
        changed.content_encoding = vec!["zstd".to_owned()];
        changed.payload_version = 7;

        assert_eq!(
            event_set_hash_for_events(&[original]).unwrap(),
            event_set_hash_for_events(&[changed]).unwrap()
        );
    }

    #[test]
    fn event_set_hash_for_empty_set_is_stable() {
        assert_eq!(
            event_set_hash_for_events(&[]).unwrap(),
            event_set_hash_for_events(&[]).unwrap()
        );
    }

    fn event(suffix: &str) -> ShoreEvent {
        let journal_id = JournalId::new(format!("journal:{suffix}"));
        ShoreEvent::new(
            EventType::ReviewInitialized,
            ReviewInitializedPayload::idempotency_key(&journal_id),
            EventTarget::for_journal(journal_id),
            Writer::shore_local("0.1.0"),
            ReviewInitializedPayload {},
            "2026-05-13T14:00:00Z",
        )
        .unwrap()
    }
}
