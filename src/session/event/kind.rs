use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    ReviewInitialized,
    ReviewUnitCaptured,
    ReviewObservationRecorded,
    ReviewAssessmentRecorded,
    InterventionRequested,
    InterventionResolved,
    ReviewNoteImported,
    TaskAttemptCaptured,
    TaskCheckpointCaptured,
    TaskObservationRecorded,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_event_types_serialize_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&EventType::TaskAttemptCaptured).unwrap(),
            "\"task_attempt_captured\""
        );
        assert_eq!(
            serde_json::to_string(&EventType::TaskCheckpointCaptured).unwrap(),
            "\"task_checkpoint_captured\""
        );
        assert_eq!(
            serde_json::to_string(&EventType::TaskObservationRecorded).unwrap(),
            "\"task_observation_recorded\""
        );
    }

    #[test]
    fn task_event_types_round_trip_through_serde() {
        for variant in [
            EventType::TaskAttemptCaptured,
            EventType::TaskCheckpointCaptured,
            EventType::TaskObservationRecorded,
        ] {
            let encoded = serde_json::to_string(&variant).unwrap();
            let decoded: EventType = serde_json::from_str(&encoded).unwrap();
            assert_eq!(decoded, variant);
        }
    }

    #[test]
    fn task_event_types_are_distinct_from_review_event_types() {
        let review_domain = [
            EventType::ReviewInitialized,
            EventType::ReviewUnitCaptured,
            EventType::ReviewObservationRecorded,
            EventType::ReviewAssessmentRecorded,
            EventType::InterventionRequested,
            EventType::InterventionResolved,
            EventType::ReviewNoteImported,
        ];
        let task_domain = [
            EventType::TaskAttemptCaptured,
            EventType::TaskCheckpointCaptured,
            EventType::TaskObservationRecorded,
        ];

        for review in review_domain {
            let review_encoded = serde_json::to_string(&review).unwrap();
            for task in task_domain {
                let task_encoded = serde_json::to_string(&task).unwrap();
                assert_ne!(
                    review_encoded, task_encoded,
                    "review variant {review:?} and task variant {task:?} collide on the wire"
                );
            }
        }
    }

    #[test]
    fn deferred_event_types_are_not_present() {
        for candidate in [
            "task_assessment_recorded",
            "source_artifact_imported",
            "review_relation_changed",
            "review_state_change_observed",
            "review_assessment_superseded",
        ] {
            let result: Result<EventType, _> = serde_json::from_str(&format!("\"{candidate}\""));
            assert!(
                result.is_err(),
                "{candidate} must not decode as an event type"
            );
        }
    }

    #[test]
    fn legacy_review_disposition_recorded_event_type_fails_to_decode_after_split() {
        let result: Result<EventType, _> = serde_json::from_str("\"review_disposition_recorded\"");
        assert!(
            result.is_err(),
            "review_disposition_recorded must not decode after the assessment split"
        );
    }
}
