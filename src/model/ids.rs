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
id_type!(LedgerId);
id_type!(EngagementId);
id_type!(ReviewUnitLineageId);
id_type!(ReviewUnitLineageRoundId);
id_type!(EventId);
id_type!(FileId);
id_type!(ReviewNoteId);
id_type!(RevisionId);
id_type!(ObjectId);
id_type!(HunkId);
id_type!(RowId);
id_type!(WorkUnitId);
id_type!(ActorId);
id_type!(TrackId);
id_type!(ObservationId);
id_type!(InputRequestId);
id_type!(InputRequestResponseId);
id_type!(AssessmentId);
id_type!(WorkObjectId);
id_type!(CheckpointId);
id_type!(ValidationCheckId);
id_type!(CommitAssociationId);
id_type!(RefAssociationId);
id_type!(CommitWithdrawalId);
id_type!(RefWithdrawalId);

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
    fn input_request_id_round_trips_through_serde_and_string() {
        let id = InputRequestId::new("input-request:sha256:abc");

        let json = serde_json::to_string(&id).unwrap();
        let parsed: InputRequestId = serde_json::from_str(&json).unwrap();

        assert_eq!(json, "\"input-request:sha256:abc\"");
        assert_eq!(parsed, id);
        assert_eq!(parsed.as_str(), "input-request:sha256:abc");
    }

    #[test]
    fn input_request_response_id_round_trips_through_serde_and_string() {
        let id = InputRequestResponseId::new("input-request-response:sha256:def");

        let json = serde_json::to_string(&id).unwrap();
        let parsed: InputRequestResponseId = serde_json::from_str(&json).unwrap();

        assert_eq!(json, "\"input-request-response:sha256:def\"");
        assert_eq!(parsed, id);
        assert_eq!(parsed.as_str(), "input-request-response:sha256:def");
    }

    #[test]
    fn input_request_ids_round_trip_with_new_prefixes() {
        let id = InputRequestId::new("input-request:sha256:abc");
        let json = serde_json::to_string(&id).unwrap();
        let parsed: InputRequestId = serde_json::from_str(&json).unwrap();

        assert_eq!(json, "\"input-request:sha256:abc\"");
        assert_eq!(parsed.as_str(), "input-request:sha256:abc");

        let response_id = InputRequestResponseId::new("input-request-response:sha256:def");
        assert_eq!(
            serde_json::to_string(&response_id).unwrap(),
            "\"input-request-response:sha256:def\""
        );
    }

    #[test]
    fn assessment_id_round_trips_through_serde_and_string() {
        let id = AssessmentId::new("assess:sha256:abc");

        let json = serde_json::to_string(&id).unwrap();
        let parsed: AssessmentId = serde_json::from_str(&json).unwrap();

        assert_eq!(json, "\"assess:sha256:abc\"");
        assert_eq!(parsed, id);
        assert_eq!(parsed.as_str(), "assess:sha256:abc");
    }

    #[test]
    fn assessment_id_prefix_is_assess_not_disp() {
        let id = AssessmentId::new("assess:sha256:fixture");

        assert!(id.as_str().starts_with("assess:"));
        assert!(!id.as_str().starts_with("disp:"));
    }

    #[test]
    fn checkpoint_id_round_trips_through_serde_and_string() {
        let id = CheckpointId::new("checkpoint:sha256:abc");

        let json = serde_json::to_string(&id).unwrap();
        let parsed: CheckpointId = serde_json::from_str(&json).unwrap();

        assert_eq!(json, "\"checkpoint:sha256:abc\"");
        assert_eq!(parsed, id);
        assert_eq!(parsed.as_str(), "checkpoint:sha256:abc");
    }

    #[test]
    fn validation_check_id_round_trips_through_serde_and_string() {
        let id = ValidationCheckId::new("validation:sha256:abc".to_string());
        assert_eq!(id.as_str(), "validation:sha256:abc");

        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"validation:sha256:abc\"");

        let back: ValidationCheckId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn validation_check_id_prefix_is_validation() {
        let id = ValidationCheckId::new("validation:sha256:deadbeef".to_string());

        assert!(id.as_str().starts_with("validation:sha256:"));
    }

    #[test]
    fn lineage_ids_round_trip_through_serde_and_string() {
        let lineage_id = ReviewUnitLineageId::new("review-unit-lineage:sha256:abc");
        let round_id = ReviewUnitLineageRoundId::new("review-unit-lineage-round:sha256:def");

        let lineage_json = serde_json::to_string(&lineage_id).unwrap();
        let round_json = serde_json::to_string(&round_id).unwrap();
        let parsed_lineage: ReviewUnitLineageId = serde_json::from_str(&lineage_json).unwrap();
        let parsed_round: ReviewUnitLineageRoundId = serde_json::from_str(&round_json).unwrap();

        assert_eq!(lineage_json, "\"review-unit-lineage:sha256:abc\"");
        assert_eq!(round_json, "\"review-unit-lineage-round:sha256:def\"");
        assert_eq!(parsed_lineage, lineage_id);
        assert_eq!(parsed_round, round_id);
    }
}
