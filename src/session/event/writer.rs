use serde::{Deserialize, Serialize};

use crate::model::ActorId;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Writer {
    pub actor_id: ActorId,
    pub tool: WriterTool,
}

impl Writer {
    pub fn shore_local(version: impl Into<String>) -> Self {
        Self {
            actor_id: ActorId::new("actor:local"),
            tool: WriterTool {
                name: "shore".to_owned(),
                version: version.into(),
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriterTool {
    pub name: String,
    pub version: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_serialization_has_no_role_key() {
        let writer = Writer::shore_local("0.1.0");
        let json = serde_json::to_value(&writer).unwrap();
        assert!(json.get("role").is_none());
        assert_eq!(json["actorId"], "actor:local");
    }

    #[test]
    fn shore_local_writer_carries_local_actor_and_tool() {
        let writer = Writer::shore_local("0.1.0");
        assert_eq!(writer.actor_id.as_str(), "actor:local");
        assert_eq!(writer.tool.name, "shore");
        assert_eq!(writer.tool.version, "0.1.0");
    }
}
