use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    ReviewInitialized,
    ReviewUnitCaptured,
    ReviewObservationRecorded,
    ReviewAssessmentRecorded,
    InputRequestOpened,
    InputRequestResponded,
    ReviewNoteImported,
    ReviewUnitLineageDeclared,
    ReviewUnitLineageRoundRecorded,
    ReviewUnitRefAssociated,
    ReviewUnitRefWithdrawn,
    ReviewUnitCommitAssociated,
    ReviewUnitCommitWithdrawn,
    ValidationCheckRecorded,
    TaskAttemptCaptured,
    TaskCheckpointCaptured,
    TaskObservationRecorded,
    EventSignatureRecorded,
}

impl EventType {
    /// The snake_case wire string for this event type, matching the serde
    /// representation. Used for per-type counts (e.g. `eventsCreatedByType`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReviewInitialized => "review_initialized",
            Self::ReviewUnitCaptured => "review_unit_captured",
            Self::ReviewObservationRecorded => "review_observation_recorded",
            Self::ReviewAssessmentRecorded => "review_assessment_recorded",
            Self::InputRequestOpened => "input_request_opened",
            Self::InputRequestResponded => "input_request_responded",
            Self::ReviewNoteImported => "review_note_imported",
            Self::ReviewUnitLineageDeclared => "review_unit_lineage_declared",
            Self::ReviewUnitLineageRoundRecorded => "review_unit_lineage_round_recorded",
            Self::ReviewUnitRefAssociated => "review_unit_ref_associated",
            Self::ReviewUnitRefWithdrawn => "review_unit_ref_withdrawn",
            Self::ReviewUnitCommitAssociated => "review_unit_commit_associated",
            Self::ReviewUnitCommitWithdrawn => "review_unit_commit_withdrawn",
            Self::ValidationCheckRecorded => "validation_check_recorded",
            Self::TaskAttemptCaptured => "task_attempt_captured",
            Self::TaskCheckpointCaptured => "task_checkpoint_captured",
            Self::TaskObservationRecorded => "task_observation_recorded",
            Self::EventSignatureRecorded => "event_signature_recorded",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_matches_serde_wire_string_for_every_variant() {
        for variant in [
            EventType::ReviewInitialized,
            EventType::ReviewUnitCaptured,
            EventType::ReviewObservationRecorded,
            EventType::ReviewAssessmentRecorded,
            EventType::InputRequestOpened,
            EventType::InputRequestResponded,
            EventType::ReviewNoteImported,
            EventType::ReviewUnitLineageDeclared,
            EventType::ReviewUnitLineageRoundRecorded,
            EventType::ReviewUnitRefAssociated,
            EventType::ReviewUnitRefWithdrawn,
            EventType::ReviewUnitCommitAssociated,
            EventType::ReviewUnitCommitWithdrawn,
            EventType::ValidationCheckRecorded,
            EventType::TaskAttemptCaptured,
            EventType::TaskCheckpointCaptured,
            EventType::TaskObservationRecorded,
            EventType::EventSignatureRecorded,
        ] {
            let serde_wire = serde_json::to_value(variant).unwrap();
            assert_eq!(
                serde_wire,
                serde_json::json!(variant.as_str()),
                "as_str() must equal the serde wire string for {variant:?}"
            );
        }
    }

    #[test]
    fn association_family_wire_strings_match() {
        assert_eq!(
            EventType::ReviewUnitRefAssociated.as_str(),
            "review_unit_ref_associated"
        );
        assert_eq!(
            EventType::ReviewUnitRefWithdrawn.as_str(),
            "review_unit_ref_withdrawn"
        );
        assert_eq!(
            EventType::ReviewUnitCommitAssociated.as_str(),
            "review_unit_commit_associated"
        );
        assert_eq!(
            EventType::ReviewUnitCommitWithdrawn.as_str(),
            "review_unit_commit_withdrawn"
        );
        for variant in [
            EventType::ReviewUnitRefAssociated,
            EventType::ReviewUnitRefWithdrawn,
            EventType::ReviewUnitCommitAssociated,
            EventType::ReviewUnitCommitWithdrawn,
        ] {
            assert_eq!(
                serde_json::to_value(variant).unwrap(),
                serde_json::json!(variant.as_str())
            );
        }
    }

    #[test]
    fn event_signature_event_type_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&EventType::EventSignatureRecorded).unwrap(),
            "\"event_signature_recorded\""
        );
        assert_eq!(
            EventType::EventSignatureRecorded.as_str(),
            "event_signature_recorded"
        );
    }

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
            EventType::InputRequestOpened,
            EventType::InputRequestResponded,
            EventType::ReviewNoteImported,
            EventType::ReviewUnitLineageDeclared,
            EventType::ReviewUnitLineageRoundRecorded,
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

    #[test]
    fn legacy_intervention_event_types_fail_to_decode_after_input_request_rename() {
        for event_type in ["intervention_requested", "intervention_resolved"] {
            let result: Result<EventType, _> = serde_json::from_str(&format!("\"{event_type}\""));
            assert!(
                result.is_err(),
                "{event_type} must not decode after the input request rename"
            );
        }
    }

    #[test]
    fn input_request_event_type_wire_strings_are_stable() {
        assert_eq!(
            serde_json::to_string(&EventType::InputRequestOpened).unwrap(),
            "\"input_request_opened\""
        );
        assert_eq!(
            serde_json::to_string(&EventType::InputRequestResponded).unwrap(),
            "\"input_request_responded\""
        );
    }

    #[test]
    fn validation_event_type_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&EventType::ValidationCheckRecorded).unwrap(),
            "\"validation_check_recorded\""
        );
        assert_eq!(
            EventType::ValidationCheckRecorded.as_str(),
            "validation_check_recorded"
        );
    }

    #[test]
    fn lineage_event_types_serialize_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&EventType::ReviewUnitLineageDeclared).unwrap(),
            "\"review_unit_lineage_declared\""
        );
        assert_eq!(
            serde_json::to_string(&EventType::ReviewUnitLineageRoundRecorded).unwrap(),
            "\"review_unit_lineage_round_recorded\""
        );
    }
}
