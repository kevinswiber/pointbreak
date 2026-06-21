use serde::{Deserialize, Serialize};

use super::kind::EventType;
use super::payload::EventPayload;
use super::task::SourceSpeaker;
use crate::model::{
    EngagementId, ObjectId, ReviewEndpoint, ReviewUnitSource, RevisionId, WorkObjectId,
};

/// The git provenance of a revision: the resolved source selector and endpoint
/// pair the revision's object was captured from. Absent (`None` on the parent
/// `Revision`) for a non-git object, where there is no commit/tree pair to
/// record. Identity-neutral: provenance feeds the revision id, not the
/// content-only object id.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitProvenance {
    pub source: ReviewUnitSource,
    pub base: ReviewEndpoint,
    pub target: ReviewEndpoint,
}

/// A revision: a concrete position over a content-only object, plus optional git
/// provenance. The `object_id` is a reference to the content-addressed object
/// (stored once, shared by clones with identical content); the revision id is
/// the position that supersession references.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Revision {
    pub id: RevisionId,
    pub object_id: ObjectId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_provenance: Option<GitProvenance>,
}

/// The proposed work object of a generative move, tagged by domain. The arm must
/// match the addressed subject's domain (a review subject carries a `Revision`,
/// a task subject a `TaskAttempt`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum WorkObjectProposal {
    Revision {
        revision: Revision,
        /// Content hash of the stored snapshot artifact this revision's object
        /// was captured into. A binding fact about the artifact (not part of
        /// revision identity); the artifact-transfer layer resolves the
        /// snapshot artifact by it.
        snapshot_artifact_content_hash: String,
    },
    TaskAttempt {
        task_attempt_id: WorkObjectId,
        project_path: String,
        claude_session_uuid: String,
        initial_prompt_hash: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        predecessor: Option<WorkObjectId>,
        #[serde(skip_serializing_if = "Option::is_none")]
        base_snapshot_fingerprint: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_speaker: Option<SourceSpeaker>,
    },
}

/// The generative move payload, domain-neutral over the review and task
/// verticals. Carries the shared write-time-derived `engagement_id` hint and the
/// tagged proposed work object. This is an advisory proposal, never an
/// instruction.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkObjectProposedPayload {
    pub engagement_id: EngagementId,
    pub work_object: WorkObjectProposal,
}

impl EventPayload for WorkObjectProposedPayload {
    fn event_type(&self) -> EventType {
        EventType::WorkObjectProposed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CommitRangeCaptureMode, WorktreeCaptureMode};

    fn git_provenance() -> GitProvenance {
        GitProvenance {
            source: ReviewUnitSource::GitWorktree {
                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                include_untracked: true,
            },
            base: ReviewEndpoint::GitCommit {
                commit_oid: "abc".to_owned(),
                tree_oid: "def".to_owned(),
            },
            target: ReviewEndpoint::GitWorkingTree {
                worktree_root: "/repo".to_owned(),
            },
        }
    }

    fn revision_payload() -> WorkObjectProposedPayload {
        WorkObjectProposedPayload {
            engagement_id: EngagementId::new("engagement:sha256:e"),
            work_object: WorkObjectProposal::Revision {
                revision: Revision {
                    id: RevisionId::new("rev:sha256:r"),
                    object_id: ObjectId::new("obj:sha256:o"),
                    git_provenance: Some(git_provenance()),
                },
                snapshot_artifact_content_hash: "sha256:artifact".to_owned(),
            },
        }
    }

    #[test]
    fn revision_arm_round_trips_and_tags_with_kind() {
        let payload = revision_payload();
        let json = serde_json::to_value(&payload).unwrap();

        assert_eq!(json["engagementId"], "engagement:sha256:e");
        assert_eq!(json["workObject"]["kind"], "revision");
        assert_eq!(json["workObject"]["revision"]["id"], "rev:sha256:r");
        assert_eq!(json["workObject"]["revision"]["objectId"], "obj:sha256:o");
        assert_eq!(
            json["workObject"]["snapshotArtifactContentHash"],
            "sha256:artifact"
        );
        assert_eq!(
            json["workObject"]["revision"]["gitProvenance"]["source"]["kind"],
            "git_worktree"
        );

        let parsed: WorkObjectProposedPayload = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, payload);
    }

    #[test]
    fn revision_arm_omits_git_provenance_when_absent() {
        let payload = WorkObjectProposedPayload {
            engagement_id: EngagementId::new("engagement:sha256:e"),
            work_object: WorkObjectProposal::Revision {
                revision: Revision {
                    id: RevisionId::new("rev:sha256:r"),
                    object_id: ObjectId::new("obj:sha256:o"),
                    git_provenance: None,
                },
                snapshot_artifact_content_hash: "sha256:artifact".to_owned(),
            },
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert!(
            json["workObject"]["revision"]
                .get("gitProvenance")
                .is_none()
        );

        let parsed: WorkObjectProposedPayload = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, payload);
    }

    #[test]
    fn task_attempt_arm_round_trips_and_tags_with_kind() {
        let payload = WorkObjectProposedPayload {
            engagement_id: EngagementId::new("engagement:sha256:t"),
            work_object: WorkObjectProposal::TaskAttempt {
                task_attempt_id: WorkObjectId::new("task-attempt:sha256:a"),
                project_path: "/repo".to_owned(),
                claude_session_uuid: "uuid-1".to_owned(),
                initial_prompt_hash: "sha256:prompt".to_owned(),
                predecessor: None,
                base_snapshot_fingerprint: None,
                source_speaker: Some(SourceSpeaker::Agent),
            },
        };
        let json = serde_json::to_value(&payload).unwrap();

        assert_eq!(json["workObject"]["kind"], "task_attempt");
        assert_eq!(json["workObject"]["taskAttemptId"], "task-attempt:sha256:a");
        assert_eq!(json["workObject"]["sourceSpeaker"], "agent");
        assert!(json["workObject"].get("predecessor").is_none());

        let parsed: WorkObjectProposedPayload = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, payload);
    }

    #[test]
    fn unused_range_mode_is_constructible() {
        // Keep the commit-range mode import exercised for provenance shaping.
        let _ = CommitRangeCaptureMode::BaseTreeToTargetTree;
    }

    #[test]
    fn payload_reports_work_object_proposed_event_type() {
        assert_eq!(
            revision_payload().event_type(),
            EventType::WorkObjectProposed
        );
    }
}
