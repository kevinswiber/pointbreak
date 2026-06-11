use serde::{Deserialize, Serialize};

use super::kind::EventType;
use super::payload::EventPayload;
use crate::model::{
    ReviewUnitId, ReviewUnitLineageBasisV1, ReviewUnitLineageId, ReviewUnitLineageRoundId,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUnitLineageDeclaredPayload {
    pub lineage_id: ReviewUnitLineageId,
    pub basis: ReviewUnitLineageBasisV1,
}

impl ReviewUnitLineageDeclaredPayload {
    pub fn idempotency_key(lineage_id: &ReviewUnitLineageId) -> String {
        format!("review_unit_lineage_declared:{}", lineage_id.as_str())
    }
}

impl EventPayload for ReviewUnitLineageDeclaredPayload {
    fn event_type(&self) -> EventType {
        EventType::ReviewUnitLineageDeclared
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUnitLineageRoundRecordedPayload {
    pub lineage_id: ReviewUnitLineageId,
    pub round_id: ReviewUnitLineageRoundId,
    pub review_unit_id: ReviewUnitId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predecessor_review_unit_id: Option<ReviewUnitId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_id: Option<String>,
}

impl ReviewUnitLineageRoundRecordedPayload {
    pub fn idempotency_key(
        lineage_id: &ReviewUnitLineageId,
        review_unit_id: &ReviewUnitId,
    ) -> String {
        format!(
            "review_unit_lineage_round_recorded:{}:{}",
            lineage_id.as_str(),
            review_unit_id.as_str()
        )
    }
}

impl EventPayload for ReviewUnitLineageRoundRecordedPayload {
    fn event_type(&self) -> EventType {
        EventType::ReviewUnitLineageRoundRecorded
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{
        ReviewEndpoint, ReviewUnitId, ReviewUnitLineageBasisV1, ReviewUnitLineageId,
        ReviewUnitLineageRoundId, ReviewUnitSource, SessionId, WorktreeCaptureMode,
    };
    use crate::session::event::{
        EventPayload, EventTarget, EventType, ReviewUnitLineageDeclaredPayload,
        ReviewUnitLineageRoundRecordedPayload, ShoreEvent, Writer,
    };

    #[test]
    fn lineage_round_idempotency_key_is_lineage_and_review_unit_scoped() {
        let lineage_id = ReviewUnitLineageId::new("review-unit-lineage:sha256:abc");
        let review_unit_id = ReviewUnitId::new("review-unit:sha256:def");

        let key =
            ReviewUnitLineageRoundRecordedPayload::idempotency_key(&lineage_id, &review_unit_id);

        assert_eq!(
            key,
            "review_unit_lineage_round_recorded:review-unit-lineage:sha256:abc:review-unit:sha256:def"
        );
    }

    #[test]
    fn lineage_event_payload_and_target_are_path_free() {
        let lineage_id = ReviewUnitLineageId::new("review-unit-lineage:sha256:abc");
        let review_unit_id = ReviewUnitId::new("review-unit:sha256:def");
        let round_id = ReviewUnitLineageRoundId::new("review-unit-lineage-round:sha256:ghi");
        let payload = ReviewUnitLineageRoundRecordedPayload {
            lineage_id: lineage_id.clone(),
            round_id,
            review_unit_id,
            predecessor_review_unit_id: None,
            change_id: Some("Iabc123".to_owned()),
        };
        let target =
            EventTarget::for_review_unit_lineage(SessionId::new("session:default"), lineage_id);

        let combined = format!(
            "{}\n{}",
            serde_json::to_string(&payload).unwrap(),
            serde_json::to_string(&target).unwrap()
        );

        assert!(!combined.contains("/Users/"));
        assert!(!combined.contains("worktreeRoot"));
        assert!(!combined.contains(".shore"));
        assert!(!combined.contains(".git"));
    }

    #[test]
    fn lineage_payloads_match_event_types() {
        let lineage_id = ReviewUnitLineageId::new("review-unit-lineage:sha256:abc");
        let review_unit_id = ReviewUnitId::new("review-unit:sha256:def");
        let basis = ReviewUnitLineageBasisV1::new(
            ReviewUnitSource::GitWorktree {
                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                include_untracked: true,
            },
            ReviewEndpoint::GitCommit {
                commit_oid: "abc123".to_owned(),
                tree_oid: "def456".to_owned(),
            },
        );
        let declared = ReviewUnitLineageDeclaredPayload {
            lineage_id: lineage_id.clone(),
            basis,
        };
        let recorded = ReviewUnitLineageRoundRecordedPayload {
            lineage_id: lineage_id.clone(),
            round_id: ReviewUnitLineageRoundId::new("review-unit-lineage-round:sha256:ghi"),
            review_unit_id,
            predecessor_review_unit_id: None,
            change_id: None,
        };

        let declare_event = ShoreEvent::new(
            EventType::ReviewUnitLineageDeclared,
            ReviewUnitLineageDeclaredPayload::idempotency_key(&lineage_id),
            EventTarget::for_review_unit_lineage(SessionId::new("session:default"), lineage_id),
            Writer::shore_local("test"),
            declared,
            "2026-06-04T00:00:00Z",
        )
        .unwrap();

        assert_eq!(
            declare_event.event_type,
            EventType::ReviewUnitLineageDeclared
        );
        assert_eq!(
            recorded.event_type(),
            EventType::ReviewUnitLineageRoundRecorded
        );
    }
}
