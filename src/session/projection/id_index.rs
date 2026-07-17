use std::collections::BTreeSet;
use std::path::Path;

use super::read::read_events;
use crate::error::Result;
use crate::session::event::{
    EventType, ReviewAssessmentRecordedPayload, ReviewObservationRecordedPayload,
    RevisionCommitAssociatedPayload, RevisionRefAssociatedPayload, ShoreEvent,
    ValidationCheckRecordedPayload, WorkObjectProposal, WorkObjectProposedPayload,
    decode_input_request_opened_payload,
};

/// Per-kind sets of the **full** opaque ids present in a store, folded from one
/// `read_events` pass. Read-only and wire-neutral (INV-1); the CLI short-id
/// resolver scans these sets. Ids are held as owned strings so the resolver can
/// prefix-match uniformly across kinds.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StoreIdIndex {
    pub revisions: BTreeSet<String>,
    pub objects: BTreeSet<String>,
    pub events: BTreeSet<String>,
    pub observations: BTreeSet<String>,
    pub assessments: BTreeSet<String>,
    pub input_requests: BTreeSet<String>,
    pub validations: BTreeSet<String>,
    pub commit_associations: BTreeSet<String>,
    pub ref_associations: BTreeSet<String>,
}

/// Fold the resolved store's whole event log into per-kind id sets. One journal
/// replay, no index build, no git, no clock — the same read `read_events` does,
/// with a per-event kind dispatch mirroring the `StateReducer` folds and the
/// association fold in `RevisionCommitRangeProjection`.
pub fn store_id_index(repo: &Path) -> Result<StoreIdIndex> {
    fold_index(&read_events(repo)?)
}

fn fold_index(events: &[ShoreEvent]) -> Result<StoreIdIndex> {
    let mut index = StoreIdIndex::default();
    for event in events {
        // Every recorded event carries its own opaque event id.
        index.events.insert(event.event_id.as_str().to_owned());
        match event.event_type {
            EventType::WorkObjectProposed => {
                let payload: WorkObjectProposedPayload =
                    serde_json::from_value(event.payload.clone())?;
                if let WorkObjectProposal::Revision { revision, .. } = payload.work_object {
                    index.revisions.insert(revision.id.as_str().to_owned());
                    index.objects.insert(revision.object_id.as_str().to_owned());
                }
            }
            EventType::ReviewObservationRecorded => {
                let payload: ReviewObservationRecordedPayload =
                    serde_json::from_value(event.payload.clone())?;
                index
                    .observations
                    .insert(payload.observation_id.as_str().to_owned());
            }
            EventType::ReviewAssessmentRecorded => {
                let payload: ReviewAssessmentRecordedPayload =
                    serde_json::from_value(event.payload.clone())?;
                index
                    .assessments
                    .insert(payload.assessment_id.as_str().to_owned());
            }
            EventType::InputRequestOpened => {
                let payload = decode_input_request_opened_payload(event.payload.clone())?;
                index
                    .input_requests
                    .insert(payload.input_request_id.as_str().to_owned());
            }
            EventType::ValidationCheckRecorded => {
                let payload: ValidationCheckRecordedPayload =
                    serde_json::from_value(event.payload.clone())?;
                index
                    .validations
                    .insert(payload.validation_check_id.as_str().to_owned());
            }
            EventType::RevisionCommitAssociated => {
                let payload: RevisionCommitAssociatedPayload =
                    serde_json::from_value(event.payload.clone())?;
                index
                    .commit_associations
                    .insert(payload.commit_association_id.as_str().to_owned());
            }
            EventType::RevisionRefAssociated => {
                let payload: RevisionRefAssociatedPayload =
                    serde_json::from_value(event.payload.clone())?;
                index
                    .ref_associations
                    .insert(payload.ref_association_id.as_str().to_owned());
            }
            _ => {}
        }
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::fold_index;
    use crate::model::{
        AssessmentId, CommitAssociationId, EngagementId, InputRequestId, JournalId, ObjectId,
        ObservationId, RefAssociationId, ReviewEndpoint, ReviewTargetRef, RevisionId,
        ValidationCheckId, ValidationStatus, ValidationTarget, ValidationTrigger,
    };
    use crate::session::event::{
        EventTarget, EventType, InputRequestOpenedPayload, InputRequestReasonCode,
        ReviewAssessment, ReviewAssessmentRecordedPayload, ReviewObservationRecordedPayload,
        Revision, RevisionCommitAssociatedPayload, RevisionRefAssociatedPayload, ShoreEvent,
        ValidationCheckRecordedPayload, WorkObjectProposal, WorkObjectProposedPayload, Writer,
    };

    /// The revision every non-revision fixture event addresses.
    fn revision_target() -> EventTarget {
        EventTarget::for_revision(
            JournalId::new("journal:default"),
            RevisionId::new("rev:sha256:one"),
            None,
        )
        .unwrap()
    }

    fn revision_event(revision_id: &str, object_id: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("work_object_proposed:{revision_id}"),
            EventTarget::for_revision(
                JournalId::new("journal:default"),
                RevisionId::new(revision_id),
                None,
            )
            .unwrap(),
            Writer::shore_local("0.1.0"),
            WorkObjectProposedPayload {
                engagement_id: EngagementId::new("engagement:sha256:e"),
                work_object: WorkObjectProposal::Revision {
                    revision: Revision {
                        id: RevisionId::new(revision_id),
                        object_id: ObjectId::new(object_id),
                        git_provenance: None,
                    },
                    summary: None,
                    object_artifact_content_hash: "sha256:artifact".to_owned(),
                    supersedes: vec![],
                },
            },
            "2026-05-10T00:00:00Z",
        )
        .unwrap()
    }

    fn observation_event(observation_id: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewObservationRecorded,
            format!("review_observation_recorded:{observation_id}"),
            revision_target(),
            Writer::shore_local("0.1.0"),
            ReviewObservationRecordedPayload {
                observation_id: ObservationId::new(observation_id),
                target: ReviewTargetRef::Revision {
                    revision_id: RevisionId::new("rev:sha256:one"),
                },
                title: "Observation".to_owned(),
                body: None,
                body_content_type: Default::default(),
                body_artifact_path: None,
                body_byte_size: None,
                body_content_hash: None,
                tags: Vec::new(),
                confidence: None,
                supersedes_observation_ids: Vec::new(),
                responds_to_observation_ids: Vec::new(),
            },
            "2026-05-10T00:00:01Z",
        )
        .unwrap()
    }

    fn assessment_event(assessment_id: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewAssessmentRecorded,
            format!("review_assessment_recorded:{assessment_id}"),
            revision_target(),
            Writer::shore_local("0.1.0"),
            ReviewAssessmentRecordedPayload {
                assessment_id: AssessmentId::new(assessment_id),
                target: ReviewTargetRef::Revision {
                    revision_id: RevisionId::new("rev:sha256:one"),
                },
                assessment: ReviewAssessment::Accepted,
                summary: None,
                summary_content_type: Default::default(),
                summary_artifact_path: None,
                summary_byte_size: None,
                summary_content_hash: None,
                replaces_assessment_ids: Vec::new(),
                related_observation_ids: Vec::new(),
                related_input_request_ids: Vec::new(),
            },
            "2026-05-10T00:00:02Z",
        )
        .unwrap()
    }

    fn validation_event(validation_check_id: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::ValidationCheckRecorded,
            format!("validation_check_recorded:{validation_check_id}"),
            revision_target(),
            Writer::shore_local("0.1.0"),
            ValidationCheckRecordedPayload {
                validation_check_id: ValidationCheckId::new(validation_check_id),
                target: ValidationTarget::Revision {
                    revision_id: RevisionId::new("rev:sha256:one"),
                },
                check_name: "cargo test".to_owned(),
                command: None,
                status: ValidationStatus::Passed,
                exit_code: Some(0),
                trigger: ValidationTrigger::Manual,
                source_fingerprint: None,
                summary: None,
                summary_content_type: Default::default(),
                summary_artifact_path: None,
                summary_byte_size: None,
                summary_content_hash: None,
                started_at: None,
                completed_at: None,
                log_artifact_content_hashes: Vec::new(),
            },
            "2026-05-10T00:00:03Z",
        )
        .unwrap()
    }

    fn input_request_event(input_request_id: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::InputRequestOpened,
            format!("input_request_opened:{input_request_id}"),
            revision_target(),
            Writer::shore_local("0.1.0"),
            InputRequestOpenedPayload {
                input_request_id: InputRequestId::new(input_request_id),
                target: ReviewTargetRef::Revision {
                    revision_id: RevisionId::new("rev:sha256:one"),
                },
                reason_code: InputRequestReasonCode::ManualDecisionRequired,
                title: "Need input".to_owned(),
                body: None,
                body_content_type: Default::default(),
                body_artifact_path: None,
                body_byte_size: None,
                body_content_hash: None,
                target_fingerprint: None,
                task_target: None,
            },
            "2026-05-10T00:00:04Z",
        )
        .unwrap()
    }

    fn commit_association_event(commit_association_id: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::RevisionCommitAssociated,
            format!("revision_commit_associated:{commit_association_id}"),
            revision_target(),
            Writer::shore_local("0.1.0"),
            RevisionCommitAssociatedPayload {
                commit_association_id: CommitAssociationId::new(commit_association_id),
                target: ReviewTargetRef::Revision {
                    revision_id: RevisionId::new("rev:sha256:one"),
                },
                commit: ReviewEndpoint::GitCommit {
                    commit_oid: "oid".to_owned(),
                    tree_oid: "oid-tree".to_owned(),
                },
            },
            "2026-05-10T00:00:05Z",
        )
        .unwrap()
    }

    fn ref_association_event(ref_association_id: &str) -> ShoreEvent {
        ShoreEvent::new(
            EventType::RevisionRefAssociated,
            format!("revision_ref_associated:{ref_association_id}"),
            revision_target(),
            Writer::shore_local("0.1.0"),
            RevisionRefAssociatedPayload {
                ref_association_id: RefAssociationId::new(ref_association_id),
                target: ReviewTargetRef::Revision {
                    revision_id: RevisionId::new("rev:sha256:one"),
                },
                ref_name: "refs/heads/feat/x".to_owned(),
                head_oid: "head-oid".to_owned(),
            },
            "2026-05-10T00:00:06Z",
        )
        .unwrap()
    }

    #[test]
    fn fold_index_collects_one_full_id_per_kind() {
        let events = vec![
            revision_event("rev:sha256:0aa1", "obj:sha256:0bb2"),
            observation_event("obs:sha256:0cc3"),
            assessment_event("assess:sha256:0dd4"),
            validation_event("validation:sha256:0ee5"),
            input_request_event("input-request:sha256:0ff6"),
            commit_association_event("assoc-commit:sha256:0aa7"),
            ref_association_event("assoc-ref:sha256:0bb8"),
        ];

        let index = fold_index(&events).unwrap();

        assert!(index.revisions.contains("rev:sha256:0aa1"));
        assert!(index.objects.contains("obj:sha256:0bb2"));
        assert!(index.observations.contains("obs:sha256:0cc3"));
        assert!(index.assessments.contains("assess:sha256:0dd4"));
        assert!(index.validations.contains("validation:sha256:0ee5"));
        assert!(index.input_requests.contains("input-request:sha256:0ff6"));
        assert!(
            !index.commit_associations.is_empty()
                && index
                    .commit_associations
                    .iter()
                    .all(|id| id.starts_with("assoc-commit:"))
        );
        assert!(
            !index.ref_associations.is_empty()
                && index
                    .ref_associations
                    .iter()
                    .all(|id| id.starts_with("assoc-ref:"))
        );
        // Every recorded event contributes its own opaque event id.
        assert_eq!(index.events.len(), 7);
    }
}
