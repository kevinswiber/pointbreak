use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Result, ShoreError};
use crate::model::{ActorId, EventId, ReviewId, RevisionId, SnapshotId, WorkUnitId};

const EVENT_SCHEMA: &str = "shore.event";
const EVENT_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShoreEvent {
    pub schema: String,
    pub version: u32,
    pub event_id: EventId,
    pub event_type: EventType,
    #[serde(deserialize_with = "deserialize_non_empty_idempotency_key")]
    pub idempotency_key: String,
    pub target: EventTarget,
    pub writer: Writer,
    pub occurred_at: String,
    pub payload_hash: String,
    pub payload: serde_json::Value,
}

impl ShoreEvent {
    pub fn new<P>(
        event_type: EventType,
        idempotency_key: impl Into<String>,
        target: EventTarget,
        writer: Writer,
        payload: P,
        occurred_at: impl Into<String>,
    ) -> Result<Self>
    where
        P: EventPayload,
    {
        if event_type != payload.event_type() {
            return Err(ShoreError::InvalidEvent {
                message: format!(
                    "payload type {:?} does not match event type {:?}",
                    payload.event_type(),
                    event_type
                ),
            });
        }

        let idempotency_key = idempotency_key.into();
        if idempotency_key.trim().is_empty() {
            return Err(ShoreError::InvalidEvent {
                message: "idempotencyKey cannot be empty".to_owned(),
            });
        }

        let payload = serde_json::to_value(payload)?;
        let payload_hash = sha256_json_value(&payload)?;
        let event_id = EventId::new(format!(
            "evt:sha256:{}",
            sha256_bytes(idempotency_key.as_bytes())
        ));

        Ok(Self {
            schema: EVENT_SCHEMA.to_owned(),
            version: EVENT_VERSION,
            event_id,
            event_type,
            idempotency_key,
            target,
            writer,
            occurred_at: occurred_at.into(),
            payload_hash,
            payload,
        })
    }

    pub fn validate_schema_version(&self) -> Result<()> {
        if self.schema == EVENT_SCHEMA && self.version == EVENT_VERSION {
            return Ok(());
        }

        Err(ShoreError::UnsupportedEventSchemaVersion {
            schema: self.schema.clone(),
            version: self.version,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    ReviewInitialized,
    RevisionPublished,
    SnapshotObserved,
    SidecarObserved,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventTarget {
    pub review_id: ReviewId,
    pub work_unit_id: WorkUnitId,
}

impl EventTarget {
    pub fn new(review_id: ReviewId, work_unit_id: WorkUnitId) -> Self {
        Self {
            review_id,
            work_unit_id,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Writer {
    pub actor_id: ActorId,
    pub role: WriterRole,
    pub tool: WriterTool,
}

impl Writer {
    pub fn shore_local_author(version: impl Into<String>) -> Self {
        Self {
            actor_id: ActorId::new("actor:local"),
            role: WriterRole::Author,
            tool: WriterTool {
                name: "shore".to_owned(),
                version: version.into(),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriterRole {
    Author,
    Reviewer,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriterTool {
    pub name: String,
    pub version: String,
}

pub trait EventPayload: Serialize {
    fn event_type(&self) -> EventType;
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewInitializedPayload {}

impl EventPayload for ReviewInitializedPayload {
    fn event_type(&self) -> EventType {
        EventType::ReviewInitialized
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionPublishedPayload {
    pub revision_id: RevisionId,
    pub supersedes_revision_ids: Vec<RevisionId>,
}

impl EventPayload for RevisionPublishedPayload {
    fn event_type(&self) -> EventType {
        EventType::RevisionPublished
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotObservedPayload {
    pub snapshot_id: SnapshotId,
    pub revision_id: RevisionId,
}

impl EventPayload for SnapshotObservedPayload {
    fn event_type(&self) -> EventType {
        EventType::SnapshotObserved
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarObservedPayload {
    pub source: SidecarSource,
    pub path: String,
    pub content_hash: String,
    pub schema: Option<String>,
    pub version: Option<u32>,
    pub diagnostic_count: usize,
}

impl EventPayload for SidecarObservedPayload {
    fn event_type(&self) -> EventType {
        EventType::SidecarObserved
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidecarSource {
    ReviewNotes,
    LegacyHunkAgentContext,
}

#[cfg(test)]
struct FixedClock(String);

#[cfg(test)]
impl FixedClock {
    fn at(timestamp: impl Into<String>) -> Self {
        Self(timestamp.into())
    }
}

#[cfg(test)]
impl From<FixedClock> for String {
    fn from(clock: FixedClock) -> Self {
        clock.0
    }
}

fn sha256_json_value(value: &serde_json::Value) -> Result<String> {
    let canonical = canonical_json_value(value);
    let bytes = serde_json::to_vec(&canonical)?;
    Ok(format!("sha256:{}", sha256_bytes(&bytes)))
}

fn canonical_json_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.iter().map(canonical_json_value).collect())
        }
        serde_json::Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();

            let mut canonical = serde_json::Map::new();
            for key in keys {
                let value = object
                    .get(key)
                    .expect("key collected from object remains present");
                canonical.insert(key.clone(), canonical_json_value(value));
            }

            serde_json::Value::Object(canonical)
        }
        _ => value.clone(),
    }
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_lower(hasher.finalize().as_slice())
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn deserialize_non_empty_idempotency_key<'de, D>(
    deserializer: D,
) -> std::result::Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.trim().is_empty() {
        return Err(serde::de::Error::custom("idempotencyKey cannot be empty"));
    }

    Ok(value)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::error::ShoreError;
    use crate::model::ReviewId;

    #[test]
    fn event_envelope_serializes_with_required_idempotency_key_and_payload_hash() {
        let event = ShoreEvent::new(
            EventType::RevisionPublished,
            "revision_published:explicit:work:default:rev:worktree:sha256:abc",
            EventTarget::new(
                ReviewId::new("review:default"),
                WorkUnitId::new("work:default"),
            ),
            Writer::shore_local_author("0.1.0"),
            RevisionPublishedPayload {
                revision_id: RevisionId::new("rev:worktree:sha256:abc"),
                supersedes_revision_ids: vec![],
            },
            FixedClock::at("2026-05-09T20:42:45Z"),
        )
        .expect("event builds");

        let json = serde_json::to_value(&event).expect("event serializes");

        assert_eq!(json["schema"], "shore.event");
        assert_eq!(json["version"], 1);
        assert_eq!(json["eventType"], "revision_published");
        assert_eq!(
            json["idempotencyKey"],
            "revision_published:explicit:work:default:rev:worktree:sha256:abc"
        );
        assert!(json["eventId"].as_str().unwrap().starts_with("evt:sha256:"));
        assert!(json["payloadHash"].as_str().unwrap().starts_with("sha256:"));
    }

    #[test]
    fn event_envelope_rejects_empty_idempotency_key() {
        let error = ShoreEvent::new(
            EventType::ReviewInitialized,
            "",
            EventTarget::new(
                ReviewId::new("review:default"),
                WorkUnitId::new("work:default"),
            ),
            Writer::shore_local_author("0.1.0"),
            ReviewInitializedPayload {},
            FixedClock::at("2026-05-09T20:42:45Z"),
        )
        .expect_err("empty idempotency key is invalid");

        assert!(error.to_string().contains("idempotency"));
    }

    #[test]
    fn event_envelope_rejects_empty_idempotency_key_on_decode() {
        let mut json = serde_json::to_value(valid_revision_published_event()).unwrap();
        json["idempotencyKey"] = json!("");

        let error = serde_json::from_value::<ShoreEvent>(json)
            .expect_err("empty idempotency key cannot decode");

        assert!(error.to_string().contains("idempotencyKey"));
    }

    #[test]
    fn event_id_is_deterministic_from_idempotency_key() {
        let first = valid_revision_published_event();
        let second = ShoreEvent::new(
            EventType::RevisionPublished,
            "revision_published:explicit:work:default:rev:worktree:sha256:abc",
            EventTarget::new(
                ReviewId::new("review:default"),
                WorkUnitId::new("work:default"),
            ),
            Writer::shore_local_author("0.1.0"),
            RevisionPublishedPayload {
                revision_id: RevisionId::new("rev:worktree:sha256:different-payload"),
                supersedes_revision_ids: vec![],
            },
            FixedClock::at("2026-05-09T21:00:00Z"),
        )
        .expect("event builds");

        assert_eq!(second.event_id, first.event_id);
        assert_ne!(second.payload_hash, first.payload_hash);
    }

    #[test]
    fn payload_hash_uses_canonical_object_key_order() {
        let first: serde_json::Value =
            serde_json::from_str(r#"{"outer":{"b":2,"a":1},"items":[{"d":4,"c":3}]}"#)
                .expect("json parses");
        let second: serde_json::Value =
            serde_json::from_str(r#"{"items":[{"c":3,"d":4}],"outer":{"a":1,"b":2}}"#)
                .expect("json parses");

        assert_eq!(
            sha256_json_value(&second).unwrap(),
            sha256_json_value(&first).unwrap()
        );
        assert_eq!(
            serde_json::to_string(&canonical_json_value(&first)).unwrap(),
            r#"{"items":[{"c":3,"d":4}],"outer":{"a":1,"b":2}}"#
        );
    }

    #[test]
    fn event_envelope_allows_unknown_optional_fields_for_same_version() {
        let mut json = serde_json::to_value(valid_revision_published_event()).unwrap();
        json["futureOptionalField"] = json!("kept-compatible");

        let event: ShoreEvent =
            serde_json::from_value(json).expect("unknown optional field is ignored");

        assert_eq!(event.version, 1);
    }

    #[test]
    fn event_envelope_round_trips_through_serde() {
        let event = valid_revision_published_event();

        let json = serde_json::to_string(&event).expect("event serializes");
        let decoded: ShoreEvent = serde_json::from_str(&json).expect("event deserializes");

        assert_eq!(decoded, event);
    }

    #[test]
    fn event_envelope_has_typed_unsupported_schema_version_validation() {
        let mut event = valid_revision_published_event();
        event.schema = "shore.event".to_owned();
        event.version = 2;

        let error = event
            .validate_schema_version()
            .expect_err("version 2 is unsupported");

        assert!(matches!(
            error,
            ShoreError::UnsupportedEventSchemaVersion { .. }
        ));
    }

    fn valid_revision_published_event() -> ShoreEvent {
        ShoreEvent::new(
            EventType::RevisionPublished,
            "revision_published:explicit:work:default:rev:worktree:sha256:abc",
            EventTarget::new(
                ReviewId::new("review:default"),
                WorkUnitId::new("work:default"),
            ),
            Writer::shore_local_author("0.1.0"),
            RevisionPublishedPayload {
                revision_id: RevisionId::new("rev:worktree:sha256:abc"),
                supersedes_revision_ids: vec![],
            },
            FixedClock::at("2026-05-09T20:42:45Z"),
        )
        .expect("event builds")
    }
}
