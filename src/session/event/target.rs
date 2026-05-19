use serde::{Deserialize, Serialize};

use crate::model::{
    ReviewTargetRef, ReviewUnitId, RevisionId, SessionId, SnapshotId, TargetRef, TrackId,
    WorkObjectId, WorkObjectType, WorkUnitId,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventTarget {
    pub session_id: SessionId,
    /// Work-unit target used by review-level events that do not yet target a
    /// captured ReviewUnit, such as initialization and imported review notes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_unit_id: Option<WorkUnitId>,
    /// Substrate-level work-object identity. Populated by `for_work_object`
    /// for domains whose work object is not a ReviewUnit. Stays `None` for
    /// review-domain events so their on-the-wire shape is unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_object_id: Option<WorkObjectId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_object_type: Option<WorkObjectType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_unit_id: Option<ReviewUnitId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<RevisionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<SnapshotId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_id: Option<TrackId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<TargetRef>,
}

impl EventTarget {
    pub fn new(session_id: SessionId, work_unit_id: WorkUnitId) -> Self {
        Self {
            session_id,
            work_unit_id: Some(work_unit_id),
            work_object_id: None,
            work_object_type: None,
            review_unit_id: None,
            revision_id: None,
            snapshot_id: None,
            track_id: None,
            subject: None,
        }
    }

    pub fn for_review_unit(
        session_id: SessionId,
        review_unit_id: ReviewUnitId,
        revision_id: RevisionId,
        snapshot_id: SnapshotId,
    ) -> Self {
        Self {
            session_id,
            work_unit_id: None,
            work_object_id: None,
            work_object_type: None,
            review_unit_id: Some(review_unit_id.clone()),
            revision_id: Some(revision_id),
            snapshot_id: Some(snapshot_id),
            track_id: None,
            subject: Some(TargetRef::Review(ReviewTargetRef::ReviewUnit {
                review_unit_id,
            })),
        }
    }

    /// Substrate-shaped constructor: populates only `session_id` and the
    /// substrate-level identity pair. Domain-specific fields are `None`;
    /// the domain-specific shape rides on the work-object type's own
    /// payload when needed (Phase 3 task-supervision events).
    pub fn for_work_object(
        session_id: SessionId,
        work_object_id: WorkObjectId,
        work_object_type: WorkObjectType,
    ) -> Self {
        Self {
            session_id,
            work_unit_id: None,
            work_object_id: Some(work_object_id),
            work_object_type: Some(work_object_type),
            review_unit_id: None,
            revision_id: None,
            snapshot_id: None,
            track_id: None,
            subject: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_target_for_review_unit_still_serializes_with_session_id() {
        // Backward-compat guard. The envelope-level identifier renames from
        // `reviewId` to `sessionId`, but review-domain identity fields keep
        // their on-the-wire shape.
        let target = EventTarget::for_review_unit(
            SessionId::new("session:sha256:r"),
            ReviewUnitId::new("review-unit:sha256:u"),
            RevisionId::new("rev:sha256:rev"),
            SnapshotId::new("snap:sha256:snap"),
        );

        let json = serde_json::to_value(&target).unwrap();

        let keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            vec![
                "reviewUnitId",
                "revisionId",
                "sessionId",
                "snapshotId",
                "subject",
            ]
        );
        assert!(json.get("reviewId").is_none());
        assert!(json.get("workObjectId").is_none());
        assert!(json.get("workObjectType").is_none());
    }

    #[test]
    fn event_target_new_serializes_session_id_and_work_unit_id() {
        let target = EventTarget::new(
            SessionId::new("session:default"),
            WorkUnitId::new("work:default"),
        );

        let json = serde_json::to_value(&target).unwrap();

        let keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, vec!["sessionId", "workUnitId"]);
        assert!(json.get("reviewId").is_none());
        assert!(json.get("workObjectId").is_none());
        assert!(json.get("workObjectType").is_none());
    }

    #[test]
    fn event_target_for_work_object_populates_substrate_fields_only() {
        let target = EventTarget::for_work_object(
            SessionId::new("session:claude:abc"),
            WorkObjectId::new("task-attempt:sha256:t"),
            WorkObjectType::TaskAttempt,
        );

        let json = serde_json::to_value(&target).unwrap();

        let keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, vec!["sessionId", "workObjectId", "workObjectType"]);
        assert_eq!(json["sessionId"], "session:claude:abc");
        assert_eq!(json["workObjectId"], "task-attempt:sha256:t");
        assert_eq!(json["workObjectType"], "task_attempt");
        // Review-domain and legacy envelope identity fields stay absent.
        assert!(json.get("reviewId").is_none());
        assert!(json.get("workUnitId").is_none());
        assert!(json.get("reviewUnitId").is_none());
        assert!(json.get("revisionId").is_none());
        assert!(json.get("snapshotId").is_none());
        assert!(json.get("trackId").is_none());
        assert!(json.get("subject").is_none());
    }

    #[test]
    fn event_target_for_work_object_round_trips_through_serde() {
        let target = EventTarget::for_work_object(
            SessionId::new("session:sha256:r"),
            WorkObjectId::new("task-attempt:sha256:t"),
            WorkObjectType::TaskAttempt,
        );

        let json = serde_json::to_string(&target).unwrap();
        let parsed: EventTarget = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed, target);
    }

    #[test]
    fn event_target_for_review_unit_subject_serializes_with_review_external_tag() {
        let target = EventTarget::for_review_unit(
            SessionId::new("session:default"),
            ReviewUnitId::new("review-unit:sha256:u"),
            RevisionId::new("rev:sha256:rev"),
            SnapshotId::new("snap:sha256:snap"),
        );

        let json = serde_json::to_value(&target).unwrap();

        assert_eq!(json["subject"]["review"]["kind"], "review_unit");
        assert_eq!(
            json["subject"]["review"]["reviewUnitId"],
            "review-unit:sha256:u"
        );
        assert!(json["subject"].get("kind").is_none());
    }

    #[test]
    fn event_target_rejects_legacy_subject_review_target_ref_shape() {
        let legacy = r#"{"sessionId":"session:default","reviewUnitId":"u","revisionId":"r","snapshotId":"s","subject":{"kind":"review_unit","reviewUnitId":"u"}}"#;

        let result: Result<EventTarget, _> = serde_json::from_str(legacy);

        assert!(
            result.is_err(),
            "legacy untagged subject JSON must not deserialize, got {:?}",
            result.ok()
        );
    }

    #[test]
    fn event_target_rejects_legacy_review_id_shape() {
        // Shore is unreleased; no migration shim is supported. Legacy
        // event-target JSON that names the envelope identifier `reviewId`
        // must fail to deserialize once the rename lands.
        let legacy = r#"{"reviewId":"review:default","workUnitId":"work:default"}"#;

        let result: Result<EventTarget, _> = serde_json::from_str(legacy);

        assert!(
            result.is_err(),
            "legacy reviewId envelope JSON must not deserialize, got {:?}",
            result.ok()
        );
    }
}
