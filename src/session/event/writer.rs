use serde::{Deserialize, Serialize};

use crate::model::ActorId;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Writer {
    pub actor_id: ActorId,
    pub producer: WriterProducer,
}

impl Writer {
    pub fn shore_local(version: impl Into<String>) -> Self {
        Self {
            actor_id: ActorId::new("actor:local"),
            producer: WriterProducer {
                name: "shore".to_owned(),
                version: version.into(),
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriterProducer {
    pub name: String,
    pub version: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_serialization_uses_producer_key_and_has_no_tool_key() {
        let writer = Writer::shore_local("0.1.0");
        let json = serde_json::to_value(&writer).unwrap();
        assert!(json.get("tool").is_none());
        assert_eq!(json["producer"]["name"], "shore");
        assert_eq!(json["producer"]["version"], "0.1.0");
        // ADR-0007 pin carried forward: still no role key either.
        assert!(json.get("role").is_none());
        assert_eq!(json["actorId"], "actor:local");
    }

    #[test]
    fn writer_round_trips_producer_field() {
        let writer = Writer::shore_local("0.1.0");
        let json = serde_json::to_string(&writer).unwrap();
        let back: Writer = serde_json::from_str(&json).unwrap();
        assert_eq!(back, writer);
        assert_eq!(writer.actor_id.as_str(), "actor:local");
        assert_eq!(writer.producer.name, "shore");
        assert_eq!(writer.producer.version, "0.1.0");
    }
}
