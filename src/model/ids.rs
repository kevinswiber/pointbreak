use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

id_type!(ReviewId);
id_type!(ReviewUnitId);
id_type!(EventId);
id_type!(FileId);
id_type!(ReviewNoteId);
id_type!(RevisionId);
id_type!(SnapshotId);
id_type!(HunkId);
id_type!(RowId);
id_type!(WorkUnitId);
id_type!(ActorId);
id_type!(TrackId);
id_type!(ReviewArtifactId);
id_type!(AcknowledgementId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_artifact_id_round_trips_through_serde_and_string() {
        let id = ReviewArtifactId::new("review-artifact:sha256:abc");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"review-artifact:sha256:abc\"");
        let parsed: ReviewArtifactId = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, id);
        assert_eq!(parsed.as_str(), "review-artifact:sha256:abc");
    }

    #[test]
    fn acknowledgement_id_round_trips_through_serde_and_string() {
        let id = AcknowledgementId::new("ack:sha256:abc");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"ack:sha256:abc\"");
        let parsed: AcknowledgementId = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, id);
        assert_eq!(parsed.as_str(), "ack:sha256:abc");
    }
}
