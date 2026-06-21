//! In-place migration of stored event writer fields.
//!
//! Rewrites a stored event's `writer` object from the legacy shape
//! (`writer.tool`, `writer.role`) to the current `writer.producer`, operating on
//! raw JSON so it bypasses [`crate::session::store::EventStore::read_event`]'s
//! legacy probe (which rejects exactly these events).
//!
//! The rewrite is provably identity-preserving: the `writer` field is outside
//! the `EventToBeSigned`, the eventId (sha256 of the idempotencyKey), the
//! payloadHash (sha256 of the payload), and the content-addressed filename
//! (eventId-derived) — so rewriting it cannot change event identity (verified on
//! a real store: 74/74 events read cleanly with unchanged ids). The transform is
//! idempotent: a producer-shaped event is an unchanged no-op.

use std::path::Path;

use serde_json::Value;

use crate::error::{Result, ShoreError};
use crate::storage::{Durability, LocalStorage};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EventMigrateOutcome {
    Rewritten,
    Unchanged,
}

/// Pure writer transform. Returns true iff it changed the map.
/// - `role` -> removed.
/// - `tool` (object) -> `producer`, only when `producer` is absent. If both are
///   present (a shape that should never occur), prefer the existing `producer`
///   and drop `tool` defensively.
fn migrate_writer_value(writer: &mut serde_json::Map<String, Value>) -> bool {
    let mut changed = false;
    if writer.remove("role").is_some() {
        changed = true;
    }
    if let Some(tool) = writer.remove("tool") {
        changed = true;
        if !writer.contains_key("producer") {
            writer.insert("producer".to_owned(), tool);
        }
    }
    changed
}

/// Read the raw event JSON, apply the writer transform, and atomically rewrite
/// the SAME path iff anything changed. Bypasses `read_event` so legacy
/// `tool`/`role` events (which `read_event` rejects) can be upgraded. The
/// filename is content-addressed by eventId, which excludes `writer`, so the
/// name is stable across the rewrite.
pub(crate) fn migrate_event_file(path: &Path) -> Result<EventMigrateOutcome> {
    let bytes = std::fs::read(path).map_err(|error| io_error("read event", path, error))?;
    let mut value: Value = serde_json::from_slice(&bytes)?;
    let Some(writer) = value.get_mut("writer").and_then(Value::as_object_mut) else {
        return Ok(EventMigrateOutcome::Unchanged); // no writer object: nothing to do
    };
    if !migrate_writer_value(writer) {
        return Ok(EventMigrateOutcome::Unchanged); // already producer-shaped
    }
    let new_bytes = serde_json::to_vec(&value)?;
    // Atomic same-path overwrite via the storage write-temp-then-rename seam.
    LocalStorage::new(path.parent().unwrap_or_else(|| Path::new("."))).write_bytes_atomic(
        path,
        &new_bytes,
        Durability::Durable,
    )?;
    Ok(EventMigrateOutcome::Rewritten)
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> ShoreError {
    ShoreError::Message(format!("{action} {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::LedgerId;
    use crate::session::event::{
        EventTarget, EventType, ReviewInitializedPayload, ShoreEvent, Writer,
    };
    use crate::session::store::EventStore;

    fn sample_producer_event() -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewInitialized,
            "review_initialized:session:default:work:default",
            EventTarget::for_ledger(LedgerId::new("session:default")),
            Writer::shore_local("0.1.0"),
            ReviewInitializedPayload {},
            "2026-05-10T00:00:00Z",
        )
        .expect("event builds")
    }

    #[test]
    fn migrate_writer_renames_tool_to_producer() {
        let mut w =
            serde_json::json!({"actorId":"actor:local","tool":{"name":"shore","version":"0.1.0"}});
        let changed = migrate_writer_value(w.as_object_mut().unwrap());
        assert!(changed);
        assert_eq!(
            w["producer"],
            serde_json::json!({"name":"shore","version":"0.1.0"})
        );
        assert!(w.get("tool").is_none());
    }

    #[test]
    fn migrate_writer_strips_role() {
        let mut w = serde_json::json!({"actorId":"actor:local","role":"reviewer",
            "producer":{"name":"shore","version":"0.1.0"}});
        let changed = migrate_writer_value(w.as_object_mut().unwrap());
        assert!(changed);
        assert!(w.get("role").is_none());
        assert_eq!(w["producer"]["name"], "shore");
    }

    #[test]
    fn migrate_writer_handles_role_and_tool_together() {
        let mut w = serde_json::json!({"actorId":"actor:local","role":"author",
            "tool":{"name":"boardwalk","version":"0.0.1"}});
        assert!(migrate_writer_value(w.as_object_mut().unwrap()));
        assert!(w.get("role").is_none());
        assert!(w.get("tool").is_none());
        assert_eq!(w["producer"]["name"], "boardwalk");
    }

    #[test]
    fn migrate_writer_is_noop_on_producer_only() {
        let mut w = serde_json::json!({"actorId":"actor:local","producer":{"name":"shore","version":"0.1.0"}});
        assert!(!migrate_writer_value(w.as_object_mut().unwrap())); // unchanged
    }

    #[test]
    fn migrate_writer_prefers_existing_producer_when_tool_also_present() {
        let mut w = serde_json::json!({"actorId":"actor:local",
            "producer":{"name":"shore","version":"0.1.0"},
            "tool":{"name":"stale","version":"0.0.0"}});
        assert!(migrate_writer_value(w.as_object_mut().unwrap()));
        // The existing producer wins; the defensive tool is dropped.
        assert_eq!(w["producer"]["name"], "shore");
        assert!(w.get("tool").is_none());
    }

    #[test]
    fn migrate_event_file_makes_legacy_event_readable_without_changing_identity() {
        let dir = tempfile::tempdir().unwrap();
        // Build a CURRENT (producer) event, capture its id/hash/filename, then
        // rewrite its on-disk JSON into the LEGACY shape to simulate an old store.
        let current = sample_producer_event();
        let store = EventStore::open(dir.path());
        store.record_event_once(&current).unwrap();
        let path = store.event_path_for_idempotency_key(current.idempotency_key.as_str());
        let before_name = path.file_name().unwrap().to_owned();

        // Downgrade on disk: producer -> tool, inject role.
        let mut v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let w = v["writer"].as_object_mut().unwrap();
        let producer = w.remove("producer").unwrap();
        w.insert("tool".into(), producer);
        w.insert("role".into(), serde_json::json!("author"));
        std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
        // Sanity: read_event now REJECTS it (the legacy probe).
        assert!(store.read_event(&path).is_err());

        // Migrate the file in place.
        let outcome = migrate_event_file(&path).unwrap();
        assert_eq!(outcome, EventMigrateOutcome::Rewritten);

        // Now it reads cleanly, identity unchanged, filename unchanged.
        let after = store.read_event(&path).unwrap();
        assert_eq!(after.event_id, current.event_id);
        assert_eq!(after.payload_hash, current.payload_hash);
        assert_eq!(path.file_name().unwrap(), before_name);
    }

    #[test]
    fn migrate_event_file_is_idempotent_on_current_event() {
        let dir = tempfile::tempdir().unwrap();
        let current = sample_producer_event();
        let store = EventStore::open(dir.path());
        store.record_event_once(&current).unwrap();
        let path = store.event_path_for_idempotency_key(current.idempotency_key.as_str());

        let outcome = migrate_event_file(&path).unwrap();
        assert_eq!(outcome, EventMigrateOutcome::Unchanged);
        // Still readable and unchanged.
        let after = store.read_event(&path).unwrap();
        assert_eq!(after, current);
    }
}
