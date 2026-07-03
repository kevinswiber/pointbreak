use serde::{Deserialize, Serialize};

use super::revision::ReviewTargetRef;
use super::{CheckpointId, RevisionId, WorkObjectId};

/// The kind of work object a subject addresses, derived from the subject's
/// domain variant rather than asserted as a standalone field. `Revision` is the
/// review-domain work object (the captured, fact-carrying unit); `TaskAttempt`
/// is the task-domain work object.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkObjectType {
    Revision,
    TaskAttempt,
}

/// The activity a journal engagement carries. The single domain axis: a
/// `Review` engagement addresses `Revision` work objects, a `Task` engagement
/// addresses `TaskAttempt` work objects. The subject's domain is derived from /
/// type-checked against this, never an independently asserted wire field.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngagementType {
    Review,
    Task,
}

/// Task-domain subject reference, self-identifying like [`ReviewTargetRef`]:
/// every variant carries the id its opaque `subjectId` is derived from.
///
/// `Checkpoint` is a sub-target of the parent `TaskAttempt`, not a peer
/// `WorkObjectType` variant: the addressed work object stays the `TaskAttempt`,
/// and the checkpoint identity lives here. Analogous to how
/// `ReviewTargetRef::Range` addresses a span inside a `Revision`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum TaskTargetRef {
    TaskAttempt { task_attempt_id: WorkObjectId },
    Checkpoint { checkpoint_id: CheckpointId },
}

/// The single shared subject identity carried by every event envelope. The
/// outer variant is the *domain* (= the engagement type); the inner is the
/// *work object*. `Journal` is the fieldless carrier for genuinely subject-less
/// events (the detached co-signature carrier, content removal, and pre-revision
/// journal events), which address their real target by payload content.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetRef {
    Journal,
    Review(ReviewTargetRef),
    Task(TaskTargetRef),
}

/// The engagement domain a subject belongs to, derived from its variant.
/// `Journal` carriers have no domain.
pub fn engagement_type_of_subject(subject: &TargetRef) -> Option<EngagementType> {
    match subject {
        TargetRef::Journal => None,
        TargetRef::Review(_) => Some(EngagementType::Review),
        TargetRef::Task(_) => Some(EngagementType::Task),
    }
}

/// The work-object kind a subject addresses, derived from its variant. `Journal`
/// carriers address no work object.
pub fn work_object_type_of_subject(subject: &TargetRef) -> Option<WorkObjectType> {
    match subject {
        TargetRef::Journal => None,
        TargetRef::Review(_) => Some(WorkObjectType::Revision),
        TargetRef::Task(_) => Some(WorkObjectType::TaskAttempt),
    }
}

/// The revision a subject addresses, if any. Every review-domain variant keys on
/// a `revision_id`; the journal carrier and task subjects address no revision.
pub fn subject_revision_id(subject: &TargetRef) -> Option<&RevisionId> {
    match subject {
        TargetRef::Review(review) => match review {
            ReviewTargetRef::Revision { revision_id }
            | ReviewTargetRef::File { revision_id, .. }
            | ReviewTargetRef::Range { revision_id, .. }
            | ReviewTargetRef::Observation { revision_id, .. }
            | ReviewTargetRef::InputRequest { revision_id, .. }
            | ReviewTargetRef::Assessment { revision_id, .. }
            | ReviewTargetRef::Event { revision_id, .. } => Some(revision_id),
        },
        TargetRef::Task(_) | TargetRef::Journal => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::model::work_object::{engagement_type_of_subject, work_object_type_of_subject};
    use crate::model::{
        CheckpointId, EngagementType, ReviewTargetRef, RevisionId, TargetRef, TaskTargetRef,
        WorkObjectId, WorkObjectType,
    };

    #[test]
    fn work_object_id_round_trips_through_serde_and_string() {
        let id = WorkObjectId::new("task-attempt:sha256:abc");

        let json = serde_json::to_string(&id).unwrap();
        let parsed: WorkObjectId = serde_json::from_str(&json).unwrap();

        assert_eq!(json, "\"task-attempt:sha256:abc\"");
        assert_eq!(parsed, id);
        assert_eq!(parsed.as_str(), "task-attempt:sha256:abc");
    }

    #[test]
    fn work_object_type_serializes_with_snake_case_kind() {
        let revision = serde_json::to_string(&WorkObjectType::Revision).unwrap();
        let task = serde_json::to_string(&WorkObjectType::TaskAttempt).unwrap();

        assert_eq!(revision, "\"revision\"");
        assert_eq!(task, "\"task_attempt\"");

        let parsed_revision: WorkObjectType = serde_json::from_str(&revision).unwrap();
        let parsed_task: WorkObjectType = serde_json::from_str(&task).unwrap();
        assert_eq!(parsed_revision, WorkObjectType::Revision);
        assert_eq!(parsed_task, WorkObjectType::TaskAttempt);
    }

    #[test]
    fn engagement_type_serializes_with_snake_case() {
        assert_eq!(
            serde_json::to_string(&EngagementType::Review).unwrap(),
            "\"review\""
        );
        assert_eq!(
            serde_json::to_string(&EngagementType::Task).unwrap(),
            "\"task\""
        );
    }

    #[test]
    fn domain_and_work_object_kind_derive_from_the_subject_variant() {
        let review = TargetRef::Review(ReviewTargetRef::Revision {
            revision_id: RevisionId::new("rev:sha256:abc"),
        });
        let task = TargetRef::Task(TaskTargetRef::TaskAttempt {
            task_attempt_id: WorkObjectId::new("task-attempt:sha256:abc"),
        });

        assert_eq!(
            engagement_type_of_subject(&review),
            Some(EngagementType::Review)
        );
        assert_eq!(
            work_object_type_of_subject(&review),
            Some(WorkObjectType::Revision)
        );
        assert_eq!(
            engagement_type_of_subject(&task),
            Some(EngagementType::Task)
        );
        assert_eq!(
            work_object_type_of_subject(&task),
            Some(WorkObjectType::TaskAttempt)
        );
        assert_eq!(engagement_type_of_subject(&TargetRef::Journal), None);
        assert_eq!(work_object_type_of_subject(&TargetRef::Journal), None);
    }

    #[test]
    fn task_target_ref_task_attempt_variant_serializes_kind_and_attempt_id() {
        let task = TaskTargetRef::TaskAttempt {
            task_attempt_id: WorkObjectId::new("task-attempt:sha256:ta"),
        };

        let json = serde_json::to_value(&task).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "task_attempt",
                "taskAttemptId": "task-attempt:sha256:ta"
            })
        );

        let parsed: TaskTargetRef = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, task);
    }

    #[test]
    fn task_target_ref_checkpoint_variant_serializes_kind_and_checkpoint_id() {
        let task = TaskTargetRef::Checkpoint {
            checkpoint_id: CheckpointId::new("checkpoint:sha256:c"),
        };

        let json = serde_json::to_value(&task).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "checkpoint",
                "checkpointId": "checkpoint:sha256:c"
            })
        );

        let parsed: TaskTargetRef = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, task);
    }

    #[test]
    fn target_ref_review_wraps_review_target_ref_externally_tagged() {
        let target = TargetRef::Review(ReviewTargetRef::Revision {
            revision_id: RevisionId::new("rev:sha256:abc"),
        });

        let json = serde_json::to_value(&target).unwrap();

        assert_eq!(json["review"]["kind"], "revision");
        assert_eq!(json["review"]["revisionId"], "rev:sha256:abc");
        assert!(json.get("task").is_none());
    }

    #[test]
    fn target_ref_task_wraps_task_target_ref_with_external_task_tag() {
        let target = TargetRef::Task(TaskTargetRef::Checkpoint {
            checkpoint_id: CheckpointId::new("checkpoint:sha256:c"),
        });

        let json = serde_json::to_value(&target).unwrap();

        assert_eq!(json["task"]["kind"], "checkpoint");
        assert_eq!(json["task"]["checkpointId"], "checkpoint:sha256:c");
        assert!(json.get("review").is_none());
    }

    #[test]
    fn target_ref_journal_carrier_serializes_as_a_bare_tag() {
        let target = TargetRef::Journal;

        let json = serde_json::to_value(&target).unwrap();
        assert_eq!(json, serde_json::json!("journal"));

        let parsed: TargetRef = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, TargetRef::Journal);
    }

    #[test]
    fn task_target_ref_checkpoint_is_not_a_work_object_type_variant() {
        assert_eq!(
            serde_json::to_string(&WorkObjectType::Revision).unwrap(),
            "\"revision\""
        );
        assert_eq!(
            serde_json::to_string(&WorkObjectType::TaskAttempt).unwrap(),
            "\"task_attempt\""
        );

        let decoded: Result<WorkObjectType, _> = serde_json::from_str("\"checkpoint\"");
        assert!(
            decoded.is_err(),
            "WorkObjectType must reject `checkpoint` — it is a sub-target of TaskAttempt, not a peer work-object type"
        );
    }
}
