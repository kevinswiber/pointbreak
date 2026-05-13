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
id_type!(ObservationId);
id_type!(InterventionId);
id_type!(InterventionResolutionId);
id_type!(DispositionId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observation_id_round_trips_through_serde_and_string() {
        let id = ObservationId::new("obs:sha256:abc");

        let json = serde_json::to_string(&id).unwrap();
        let parsed: ObservationId = serde_json::from_str(&json).unwrap();

        assert_eq!(json, "\"obs:sha256:abc\"");
        assert_eq!(parsed, id);
        assert_eq!(parsed.as_str(), "obs:sha256:abc");
    }

    #[test]
    fn intervention_id_round_trips_through_serde_and_string() {
        let id = InterventionId::new("intervention:sha256:abc");

        let json = serde_json::to_string(&id).unwrap();
        let parsed: InterventionId = serde_json::from_str(&json).unwrap();

        assert_eq!(json, "\"intervention:sha256:abc\"");
        assert_eq!(parsed, id);
        assert_eq!(parsed.as_str(), "intervention:sha256:abc");
    }

    #[test]
    fn intervention_resolution_id_round_trips_through_serde_and_string() {
        let id = InterventionResolutionId::new("intervention-resolution:sha256:def");

        let json = serde_json::to_string(&id).unwrap();
        let parsed: InterventionResolutionId = serde_json::from_str(&json).unwrap();

        assert_eq!(json, "\"intervention-resolution:sha256:def\"");
        assert_eq!(parsed, id);
        assert_eq!(parsed.as_str(), "intervention-resolution:sha256:def");
    }

    #[test]
    fn disposition_id_round_trips_through_serde_and_string() {
        let id = DispositionId::new("disp:sha256:abc");

        let json = serde_json::to_string(&id).unwrap();
        let parsed: DispositionId = serde_json::from_str(&json).unwrap();

        assert_eq!(json, "\"disp:sha256:abc\"");
        assert_eq!(parsed, id);
        assert_eq!(parsed.as_str(), "disp:sha256:abc");
    }
}
