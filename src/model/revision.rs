use serde::{Deserialize, Serialize};

use super::{AssessmentId, EventId, InputRequestId, ObservationId, RevisionId, Side};

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
pub enum RevisionSource {
    GitWorktree {
        mode: WorktreeCaptureMode,
        include_untracked: bool,
        /// Capture-time git pathspec scope; empty means the whole repository.
        /// Ordering and normalization are owned by the capture workflow.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pathspecs: Vec<String>,
    },
    /// Commit-range source selector (research 0004 Q1): lowers to a
    /// `git_commit` base endpoint and a `git_commit` target endpoint. Carries
    /// no rev spellings: resolved OIDs live in the endpoints, and spellings
    /// must not participate in Revision identity (storing `--base main` vs
    /// `--base <oid>` would manufacture distinct units for identical content).
    GitCommitRange {
        mode: CommitRangeCaptureMode,
        /// Capture-time git pathspec scope; empty means the whole repository.
        /// Ordering and normalization are owned by the capture workflow.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pathspecs: Vec<String>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeCaptureMode {
    CombinedHeadToWorkingTree,
}

/// How a commit-range snapshot was produced. V1 is a direct two-tree diff
/// (`git diff <base> <target>`), not a merge-base (`...`) comparison; a future
/// merge-base adapter is a separate selector per research 0004 Q1.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitRangeCaptureMode {
    BaseTreeToTargetTree,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ReviewTargetRef {
    Revision {
        revision_id: RevisionId,
    },
    File {
        revision_id: RevisionId,
        file_path: String,
    },
    Range {
        revision_id: RevisionId,
        file_path: String,
        side: Side,
        start_line: u32,
        end_line: u32,
    },
    Observation {
        revision_id: RevisionId,
        observation_id: ObservationId,
    },
    InputRequest {
        revision_id: RevisionId,
        input_request_id: InputRequestId,
    },
    Assessment {
        revision_id: RevisionId,
        assessment_id: AssessmentId,
    },
    Event {
        revision_id: RevisionId,
        event_id: EventId,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_source_and_endpoints_serialize_with_stable_shape() {
        let source = RevisionSource::GitWorktree {
            mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
            include_untracked: true,
            pathspecs: Vec::new(),
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
    fn commit_range_source_serializes_with_stable_shape() {
        let source = RevisionSource::GitCommitRange {
            mode: CommitRangeCaptureMode::BaseTreeToTargetTree,
            pathspecs: Vec::new(),
        };

        let json = serde_json::to_value(&source).unwrap();

        assert_eq!(json["kind"], "git_commit_range");
        assert_eq!(json["mode"], "base_tree_to_target_tree");
        // Untracked files cannot participate in a tree diff; the field is absent, not false.
        assert!(json.get("includeUntracked").is_none());

        let round_tripped: RevisionSource = serde_json::from_value(json).unwrap();
        assert_eq!(round_tripped, source);
    }

    #[test]
    fn commit_range_capture_serialization_is_path_free() {
        // Source + commit/commit endpoint pair: the serialized capture identity surface
        // for a range capture must never contain a worktree path.
        let json = serde_json::json!({
            "source": RevisionSource::GitCommitRange {
                mode: CommitRangeCaptureMode::BaseTreeToTargetTree,
                pathspecs: Vec::new(),
            },
            "base": ReviewEndpoint::GitCommit {
                commit_oid: "abc123".to_owned(),
                tree_oid: "def456".to_owned(),
            },
            "target": ReviewEndpoint::GitCommit {
                commit_oid: "0a1b2c".to_owned(),
                tree_oid: "3d4e5f".to_owned(),
            },
        });

        let text = json.to_string();
        assert!(!text.contains("worktreeRoot"));
        assert_eq!(json["target"]["kind"], "git_commit");
        assert_eq!(json["target"]["commitOid"], "0a1b2c");
    }

    #[test]
    fn unscoped_sources_serialize_without_a_pathspecs_key() {
        // Empty means unscoped and must be absent on the wire, so existing
        // revision ids, payload hashes, and signatures are untouched.
        let worktree = serde_json::to_value(RevisionSource::GitWorktree {
            mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
            include_untracked: true,
            pathspecs: Vec::new(),
        })
        .unwrap();
        let range = serde_json::to_value(RevisionSource::GitCommitRange {
            mode: CommitRangeCaptureMode::BaseTreeToTargetTree,
            pathspecs: Vec::new(),
        })
        .unwrap();
        assert!(worktree.get("pathspecs").is_none());
        assert!(range.get("pathspecs").is_none());
    }

    #[test]
    fn scoped_sources_round_trip_the_pathspec_set() {
        let source = RevisionSource::GitCommitRange {
            mode: CommitRangeCaptureMode::BaseTreeToTargetTree,
            pathspecs: vec!["docs/spec".to_owned(), "packages/foo".to_owned()],
        };
        let json = serde_json::to_value(&source).unwrap();
        assert_eq!(json["pathspecs"][0], "docs/spec");
        assert_eq!(json["pathspecs"][1], "packages/foo");
        let round_tripped: RevisionSource = serde_json::from_value(json).unwrap();
        assert_eq!(round_tripped, source);
    }

    #[test]
    fn legacy_source_json_without_pathspecs_decodes_as_unscoped() {
        // Every event written before this field exists must keep decoding.
        let legacy = serde_json::json!({
            "kind": "git_worktree",
            "mode": "combined_head_to_working_tree",
            "includeUntracked": true
        });
        let source: RevisionSource = serde_json::from_value(legacy).unwrap();
        let RevisionSource::GitWorktree { pathspecs, .. } = source else {
            panic!("expected a worktree source");
        };
        assert!(pathspecs.is_empty());
    }

    #[test]
    fn review_target_can_represent_review_wide_and_range_scope() {
        let review_wide = ReviewTargetRef::Revision {
            revision_id: RevisionId::new("rev:sha256:abc"),
        };
        let range = ReviewTargetRef::Range {
            revision_id: RevisionId::new("rev:sha256:abc"),
            file_path: "src/lib.rs".to_owned(),
            side: Side::New,
            start_line: 10,
            end_line: 12,
        };
        let file = ReviewTargetRef::File {
            revision_id: RevisionId::new("rev:sha256:abc"),
            file_path: "src/main.rs".to_owned(),
        };
        let event = ReviewTargetRef::Event {
            revision_id: RevisionId::new("rev:sha256:abc"),
            event_id: EventId::new("evt:sha256:def"),
        };

        let review_wide = serde_json::to_value(review_wide).unwrap();
        let range = serde_json::to_value(range).unwrap();
        let file = serde_json::to_value(file).unwrap();
        let event = serde_json::to_value(event).unwrap();

        assert_eq!(review_wide["kind"], "revision");
        assert_eq!(review_wide["revisionId"], "rev:sha256:abc");
        assert_eq!(range["kind"], "range");
        assert_eq!(range["revisionId"], "rev:sha256:abc");
        assert_eq!(range["filePath"], "src/lib.rs");
        assert_eq!(range["side"], "new");
        assert_eq!(range["startLine"], 10);
        assert_eq!(range["endLine"], 12);
        assert_eq!(file["kind"], "file");
        assert_eq!(file["revisionId"], "rev:sha256:abc");
        assert_eq!(file["filePath"], "src/main.rs");
        assert_eq!(event["kind"], "event");
        assert_eq!(event["revisionId"], "rev:sha256:abc");
        assert_eq!(event["eventId"], "evt:sha256:def");
    }

    #[test]
    fn review_target_can_represent_observation_and_input_request_scope() {
        let observation = ReviewTargetRef::Observation {
            revision_id: RevisionId::new("rev:sha256:abc"),
            observation_id: ObservationId::new("obs:sha256:def"),
        };
        let input_request = ReviewTargetRef::InputRequest {
            revision_id: RevisionId::new("rev:sha256:abc"),
            input_request_id: InputRequestId::new("input-request:sha256:ghi"),
        };

        let json = serde_json::json!({
            "observation": observation,
            "inputRequest": input_request
        });

        assert_eq!(json["observation"]["kind"], "observation");
        assert_eq!(json["observation"]["observationId"], "obs:sha256:def");
        assert_eq!(json["inputRequest"]["kind"], "input_request");
        assert_eq!(
            json["inputRequest"]["inputRequestId"],
            "input-request:sha256:ghi"
        );
        assert!(json["inputRequest"].get("interventionId").is_none());
    }

    #[test]
    fn review_target_ref_input_request_variant_wire_shape_is_kind_input_request() {
        let target = ReviewTargetRef::InputRequest {
            revision_id: RevisionId::new("rev:sha256:one"),
            input_request_id: InputRequestId::new("input-request:sha256:one"),
        };

        let json = serde_json::to_value(&target).unwrap();

        assert_eq!(json["kind"], "input_request");
        assert_eq!(json["revisionId"], "rev:sha256:one");
        assert_eq!(json["inputRequestId"], "input-request:sha256:one");
        assert!(json.get("interventionId").is_none());

        let round_tripped: ReviewTargetRef = serde_json::from_value(json).unwrap();
        assert_eq!(round_tripped, target);
    }

    #[test]
    fn review_target_ref_legacy_intervention_kind_fails_to_decode() {
        let legacy = serde_json::json!({
            "kind": "intervention",
            "revisionId": "rev:sha256:one",
            "inputRequestId": "input-request:sha256:one"
        });

        let error = serde_json::from_value::<ReviewTargetRef>(legacy).unwrap_err();
        assert!(
            error.to_string().contains("unknown variant"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn review_target_ref_assessment_variant_wire_shape_is_kind_assessment_with_assessment_id() {
        let target = ReviewTargetRef::Assessment {
            revision_id: RevisionId::new("rev:sha256:one"),
            assessment_id: AssessmentId::new("assess:sha256:one"),
        };

        let json = serde_json::to_value(&target).unwrap();

        assert_eq!(json["kind"], "assessment");
        assert_eq!(json["revisionId"], "rev:sha256:one");
        assert_eq!(json["assessmentId"], "assess:sha256:one");

        let round_tripped: ReviewTargetRef = serde_json::from_value(json).unwrap();
        assert_eq!(round_tripped, target);
    }
}
