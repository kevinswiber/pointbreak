use serde::{Deserialize, Serialize};

use super::CheckpointId;
use super::review_unit::ReviewTargetRef;

/// Substrate-level discriminator for which domain a work object belongs to.
///
/// Used alongside `WorkObjectId` to give substrate-shaped events polymorphic
/// identity without forcing every domain to share a serialization layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkObjectType {
    ReviewUnit,
    TaskAttempt,
}

/// Within-task-attempt sub-target reference.
///
/// `Checkpoint` is a sub-target of the parent `TaskAttempt`, not a peer
/// `WorkObjectType` variant: the envelope's `work_object_id` and
/// `work_object_type` continue to identify the `TaskAttempt`, and the
/// checkpoint identity lives here. Analogous to how `ReviewTargetRef::Range`
/// addresses a span inside a `ReviewUnit`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum TaskTargetRef {
    TaskAttempt,
    Checkpoint { checkpoint_id: CheckpointId },
}

/// Substrate-level target reference. Externally tagged so that each domain's
/// own internal shape (e.g., `ReviewTargetRef`'s `kind` discriminator) is
/// preserved unchanged inside the variant payload.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetRef {
    Review(ReviewTargetRef),
    Task(TaskTargetRef),
}

#[cfg(test)]
mod tests {
    use crate::model::{
        CheckpointId, ReviewTargetRef, ReviewUnitId, TargetRef, TaskTargetRef, WorkObjectId,
        WorkObjectType,
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
        let review = serde_json::to_string(&WorkObjectType::ReviewUnit).unwrap();
        let task = serde_json::to_string(&WorkObjectType::TaskAttempt).unwrap();

        assert_eq!(review, "\"review_unit\"");
        assert_eq!(task, "\"task_attempt\"");

        let parsed_review: WorkObjectType = serde_json::from_str(&review).unwrap();
        let parsed_task: WorkObjectType = serde_json::from_str(&task).unwrap();
        assert_eq!(parsed_review, WorkObjectType::ReviewUnit);
        assert_eq!(parsed_task, WorkObjectType::TaskAttempt);
    }

    #[test]
    fn task_target_ref_task_attempt_variant_serializes_with_kind_only() {
        let task = TaskTargetRef::TaskAttempt;

        let json = serde_json::to_value(&task).unwrap();
        assert_eq!(json, serde_json::json!({"kind": "task_attempt"}));

        let parsed: TaskTargetRef = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, TaskTargetRef::TaskAttempt);
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
        let target = TargetRef::Review(ReviewTargetRef::ReviewUnit {
            review_unit_id: ReviewUnitId::new("review-unit:sha256:abc"),
        });

        let json = serde_json::to_value(&target).unwrap();

        assert_eq!(json["review"]["kind"], "review_unit");
        assert_eq!(json["review"]["reviewUnitId"], "review-unit:sha256:abc");
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
    fn task_target_ref_checkpoint_is_not_a_work_object_type_variant() {
        assert_eq!(
            serde_json::to_string(&WorkObjectType::ReviewUnit).unwrap(),
            "\"review_unit\""
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
