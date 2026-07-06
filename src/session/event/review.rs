use serde::{Deserialize, Serialize};

use super::kind::EventType;
use super::payload::EventPayload;
use super::type_code::type_code;
use crate::model::JournalId;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewInitializedPayload {}

impl ReviewInitializedPayload {
    pub fn idempotency_key(journal_id: &JournalId) -> String {
        format!(
            "{}:{}",
            type_code(EventType::ReviewInitialized),
            journal_id.as_str()
        )
    }
}

impl EventPayload for ReviewInitializedPayload {
    fn event_type(&self) -> EventType {
        EventType::ReviewInitialized
    }
}
