use serde::{Deserialize, Serialize};

use super::{DispositionId, EventId, InterventionId, ObservationId, ReviewUnitId, Side};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ReviewEndpoint {
    GitCommit {
        commit_oid: String,
        tree_oid: String,
    },
    GitWorkingTree {
        worktree_root: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ReviewUnitSource {
    GitWorktree {
        mode: WorktreeCaptureMode,
        include_untracked: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeCaptureMode {
    CombinedHeadToWorkingTree,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ReviewTargetRef {
    ReviewUnit {
        review_unit_id: ReviewUnitId,
    },
    File {
        review_unit_id: ReviewUnitId,
        file_path: String,
    },
    Range {
        review_unit_id: ReviewUnitId,
        file_path: String,
        side: Side,
        start_line: u32,
        end_line: u32,
    },
    Observation {
        review_unit_id: ReviewUnitId,
        observation_id: ObservationId,
    },
    Intervention {
        review_unit_id: ReviewUnitId,
        intervention_id: InterventionId,
    },
    Disposition {
        review_unit_id: ReviewUnitId,
        disposition_id: DispositionId,
    },
    Event {
        review_unit_id: ReviewUnitId,
        event_id: EventId,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_unit_source_and_endpoints_serialize_with_stable_shape() {
        let source = ReviewUnitSource::GitWorktree {
            mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
            include_untracked: true,
        };
        let base = ReviewEndpoint::GitCommit {
            commit_oid: "abc123".to_owned(),
            tree_oid: "def456".to_owned(),
        };
        let target = ReviewEndpoint::GitWorkingTree {
            worktree_root: "/repo".to_owned(),
        };

        let json = serde_json::json!({
            "source": source,
            "base": base,
            "target": target
        });

        assert_eq!(json["source"]["kind"], "git_worktree");
        assert_eq!(json["source"]["mode"], "combined_head_to_working_tree");
        assert_eq!(json["source"]["includeUntracked"], true);
        assert_eq!(json["base"]["kind"], "git_commit");
        assert_eq!(json["base"]["commitOid"], "abc123");
        assert_eq!(json["base"]["treeOid"], "def456");
        assert_eq!(json["target"]["kind"], "git_working_tree");
        assert_eq!(json["target"]["worktreeRoot"], "/repo");
    }

    #[test]
    fn review_target_can_represent_review_wide_and_range_scope() {
        let review_wide = ReviewTargetRef::ReviewUnit {
            review_unit_id: ReviewUnitId::new("review-unit:sha256:abc"),
        };
        let range = ReviewTargetRef::Range {
            review_unit_id: ReviewUnitId::new("review-unit:sha256:abc"),
            file_path: "src/lib.rs".to_owned(),
            side: Side::New,
            start_line: 10,
            end_line: 12,
        };
        let file = ReviewTargetRef::File {
            review_unit_id: ReviewUnitId::new("review-unit:sha256:abc"),
            file_path: "src/main.rs".to_owned(),
        };
        let event = ReviewTargetRef::Event {
            review_unit_id: ReviewUnitId::new("review-unit:sha256:abc"),
            event_id: EventId::new("evt:sha256:def"),
        };

        let review_wide = serde_json::to_value(review_wide).unwrap();
        let range = serde_json::to_value(range).unwrap();
        let file = serde_json::to_value(file).unwrap();
        let event = serde_json::to_value(event).unwrap();

        assert_eq!(review_wide["kind"], "review_unit");
        assert_eq!(review_wide["reviewUnitId"], "review-unit:sha256:abc");
        assert_eq!(range["kind"], "range");
        assert_eq!(range["reviewUnitId"], "review-unit:sha256:abc");
        assert_eq!(range["filePath"], "src/lib.rs");
        assert_eq!(range["side"], "new");
        assert_eq!(range["startLine"], 10);
        assert_eq!(range["endLine"], 12);
        assert_eq!(file["kind"], "file");
        assert_eq!(file["reviewUnitId"], "review-unit:sha256:abc");
        assert_eq!(file["filePath"], "src/main.rs");
        assert_eq!(event["kind"], "event");
        assert_eq!(event["reviewUnitId"], "review-unit:sha256:abc");
        assert_eq!(event["eventId"], "evt:sha256:def");
    }

    #[test]
    fn review_target_can_represent_observation_and_intervention_scope() {
        let observation = ReviewTargetRef::Observation {
            review_unit_id: ReviewUnitId::new("review-unit:sha256:abc"),
            observation_id: ObservationId::new("obs:sha256:def"),
        };
        let intervention = ReviewTargetRef::Intervention {
            review_unit_id: ReviewUnitId::new("review-unit:sha256:abc"),
            intervention_id: InterventionId::new("intervention:sha256:ghi"),
        };

        let json = serde_json::json!({
            "observation": observation,
            "intervention": intervention
        });

        assert_eq!(json["observation"]["kind"], "observation");
        assert_eq!(json["observation"]["observationId"], "obs:sha256:def");
        assert_eq!(json["intervention"]["kind"], "intervention");
        assert_eq!(
            json["intervention"]["interventionId"],
            "intervention:sha256:ghi"
        );
    }

    #[test]
    fn review_target_can_reference_disposition() {
        let target = ReviewTargetRef::Disposition {
            review_unit_id: ReviewUnitId::new("review-unit:sha256:one"),
            disposition_id: DispositionId::new("disp:sha256:one"),
        };

        let json = serde_json::to_value(&target).unwrap();

        assert_eq!(json["kind"], "disposition");
        assert_eq!(json["reviewUnitId"], "review-unit:sha256:one");
        assert_eq!(json["dispositionId"], "disp:sha256:one");
    }
}
