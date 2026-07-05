use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    ReviewInitialized,
    WorkObjectProposed,
    ReviewObservationRecorded,
    ReviewAssessmentRecorded,
    InputRequestOpened,
    InputRequestResponded,
    ReviewNoteImported,
    RevisionRefAssociated,
    RevisionRefWithdrawn,
    RevisionCommitAssociated,
    RevisionCommitWithdrawn,
    ValidationCheckRecorded,
    TaskCheckpointCaptured,
    TaskObservationRecorded,
    EventSignatureRecorded,
    ArtifactRemoved,
}

impl EventType {
    /// The snake_case wire string for this event type, matching the serde
    /// representation. Used for per-type counts (e.g. `eventsCreatedByType`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReviewInitialized => "review_initialized",
            Self::WorkObjectProposed => "work_object_proposed",
            Self::ReviewObservationRecorded => "review_observation_recorded",
            Self::ReviewAssessmentRecorded => "review_assessment_recorded",
            Self::InputRequestOpened => "input_request_opened",
            Self::InputRequestResponded => "input_request_responded",
            Self::ReviewNoteImported => "review_note_imported",
            Self::RevisionRefAssociated => "revision_ref_associated",
            Self::RevisionRefWithdrawn => "revision_ref_withdrawn",
            Self::RevisionCommitAssociated => "revision_commit_associated",
            Self::RevisionCommitWithdrawn => "revision_commit_withdrawn",
            Self::ValidationCheckRecorded => "validation_check_recorded",
            Self::TaskCheckpointCaptured => "task_checkpoint_captured",
            Self::TaskObservationRecorded => "task_observation_recorded",
            Self::EventSignatureRecorded => "event_signature_recorded",
            Self::ArtifactRemoved => "artifact_removed",
        }
    }

    /// Every `EventType` variant, in declaration order. The canonical list tests
    /// iterate so a variant is enumerated in exactly one place. Kept complete by
    /// `all_is_exhaustive` below. Test-only: the sole consumers are `#[cfg(test)]`
    /// registries/agreement tests, so gating it keeps the lib pass free of a
    /// dead-code warning under `-D warnings`.
    #[cfg(test)]
    pub(crate) const ALL: [EventType; 16] = [
        Self::ReviewInitialized,
        Self::WorkObjectProposed,
        Self::ReviewObservationRecorded,
        Self::ReviewAssessmentRecorded,
        Self::InputRequestOpened,
        Self::InputRequestResponded,
        Self::ReviewNoteImported,
        Self::RevisionRefAssociated,
        Self::RevisionRefWithdrawn,
        Self::RevisionCommitAssociated,
        Self::RevisionCommitWithdrawn,
        Self::ValidationCheckRecorded,
        Self::TaskCheckpointCaptured,
        Self::TaskObservationRecorded,
        Self::EventSignatureRecorded,
        Self::ArtifactRemoved,
    ];
}

/// Adding an `EventType` variant breaks this match until the variant is also
/// appended to [`EventType::ALL`]. It exists only to make that coupling a compile
/// error rather than a silent test-coverage gap; it is invoked from the tests so
/// it carries no runtime cost and trips no dead-code lint.
#[cfg(test)]
fn all_is_exhaustive(event_type: EventType) {
    match event_type {
        EventType::ReviewInitialized
        | EventType::WorkObjectProposed
        | EventType::ReviewObservationRecorded
        | EventType::ReviewAssessmentRecorded
        | EventType::InputRequestOpened
        | EventType::InputRequestResponded
        | EventType::ReviewNoteImported
        | EventType::RevisionRefAssociated
        | EventType::RevisionRefWithdrawn
        | EventType::RevisionCommitAssociated
        | EventType::RevisionCommitWithdrawn
        | EventType::ValidationCheckRecorded
        | EventType::TaskCheckpointCaptured
        | EventType::TaskObservationRecorded
        | EventType::EventSignatureRecorded
        | EventType::ArtifactRemoved => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_lists_every_variant_once() {
        // Every variant appears exactly once, and the count matches the enum arity.
        assert_eq!(EventType::ALL.len(), 16);
        for variant in EventType::ALL {
            let occurrences = EventType::ALL.iter().filter(|&&v| v == variant).count();
            assert_eq!(
                occurrences, 1,
                "{variant:?} must appear exactly once in ALL"
            );
            // Anchor the completeness reminder so a new variant forces an ALL update.
            all_is_exhaustive(variant);
        }
        // ALL agrees with the serde round-trip surface for every listed variant.
        for variant in EventType::ALL {
            let encoded = serde_json::to_string(&variant).unwrap();
            let decoded: EventType = serde_json::from_str(&encoded).unwrap();
            assert_eq!(decoded, variant);
        }
    }

    #[test]
    fn as_str_matches_serde_wire_string_for_every_variant() {
        for variant in [
            EventType::ReviewInitialized,
            EventType::WorkObjectProposed,
            EventType::ReviewObservationRecorded,
            EventType::ReviewAssessmentRecorded,
            EventType::InputRequestOpened,
            EventType::InputRequestResponded,
            EventType::ReviewNoteImported,
            EventType::RevisionRefAssociated,
            EventType::RevisionRefWithdrawn,
            EventType::RevisionCommitAssociated,
            EventType::RevisionCommitWithdrawn,
            EventType::ValidationCheckRecorded,
            EventType::TaskCheckpointCaptured,
            EventType::TaskObservationRecorded,
            EventType::EventSignatureRecorded,
            EventType::ArtifactRemoved,
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
            EventType::RevisionRefAssociated.as_str(),
            "revision_ref_associated"
        );
        assert_eq!(
            EventType::RevisionRefWithdrawn.as_str(),
            "revision_ref_withdrawn"
        );
        assert_eq!(
            EventType::RevisionCommitAssociated.as_str(),
            "revision_commit_associated"
        );
        assert_eq!(
            EventType::RevisionCommitWithdrawn.as_str(),
            "revision_commit_withdrawn"
        );
        for variant in [
            EventType::RevisionRefAssociated,
            EventType::RevisionRefWithdrawn,
            EventType::RevisionCommitAssociated,
            EventType::RevisionCommitWithdrawn,
        ] {
            assert_eq!(
                serde_json::to_value(variant).unwrap(),
                serde_json::json!(variant.as_str())
            );
        }
    }

    #[test]
    fn artifact_removed_event_type_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&EventType::ArtifactRemoved).unwrap(),
            "\"artifact_removed\""
        );
        assert_eq!(EventType::ArtifactRemoved.as_str(), "artifact_removed");
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
            EventType::ReviewObservationRecorded,
            EventType::ReviewAssessmentRecorded,
            EventType::InputRequestOpened,
            EventType::InputRequestResponded,
            EventType::ReviewNoteImported,
        ];
        let task_domain = [
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
    fn work_object_proposed_event_type_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&EventType::WorkObjectProposed).unwrap(),
            "\"work_object_proposed\""
        );
        assert_eq!(
            EventType::WorkObjectProposed.as_str(),
            "work_object_proposed"
        );
    }

    #[test]
    fn collapsed_capture_event_types_no_longer_decode() {
        for legacy in ["review_unit_captured", "task_attempt_captured"] {
            let result: Result<EventType, _> = serde_json::from_str(&format!("\"{legacy}\""));
            assert!(
                result.is_err(),
                "{legacy} must not decode after the generative-move collapse"
            );
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
    fn legacy_association_event_type_wire_strings_no_longer_decode() {
        for legacy in [
            "review_unit_ref_associated",
            "review_unit_ref_withdrawn",
            "review_unit_commit_associated",
            "review_unit_commit_withdrawn",
        ] {
            let result: Result<EventType, _> = serde_json::from_str(&format!("\"{legacy}\""));
            assert!(
                result.is_err(),
                "{legacy} must not decode after the association family renames to revision_*"
            );
        }
    }

    #[test]
    fn retired_lineage_event_types_no_longer_decode() {
        for retired in [
            "review_unit_lineage_declared",
            "review_unit_lineage_round_recorded",
        ] {
            let result: Result<EventType, _> = serde_json::from_str(&format!("\"{retired}\""));
            assert!(
                result.is_err(),
                "{retired} must not decode after lineage is retired for supersession"
            );
        }
    }
}
