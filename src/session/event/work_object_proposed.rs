use serde::{Deserialize, Serialize};

use super::kind::EventType;
use super::payload::EventPayload;
use super::task::SourceSpeaker;
use crate::model::{
    EngagementId, ObjectId, ReviewEndpoint, RevisionId, RevisionSource, WorkObjectId,
};

/// The git provenance of a revision: the resolved source selector and endpoint
/// pair the revision's object was captured from. Absent (`None` on the parent
/// `Revision`) for a non-git object, where there is no commit/tree pair to
/// record. Identity-neutral: provenance feeds the revision id, not the
/// content-only object id.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitProvenance {
    pub source: RevisionSource,
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
        /// Content hash of the stored object artifact this revision's object
        /// was captured into. A binding fact about the artifact (not part of
        /// revision identity); the artifact-transfer layer resolves the
        /// object artifact by it.
        object_artifact_content_hash: String,
        /// The revisions this one supersedes (an evolution forward-pointer). Sorted
        /// and deduped before hashing, so set-equal inputs converge byte-for-byte;
        /// empty (a root revision) serializes to nothing, leaving an existing root
        /// capture's payload hash unchanged. References a revision position, never
        /// the revision's content object.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        supersedes: Vec<RevisionId>,
    },
    TaskAttempt {
        task_attempt_id: WorkObjectId,
        project_path: String,
        claude_session_uuid: String,
        initial_prompt_hash: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        predecessor: Option<WorkObjectId>,
        #[serde(skip_serializing_if = "Option::is_none")]
        base_state_fingerprint: Option<String>,
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
    use crate::model::{CommitRangeCaptureMode, JournalId, WorktreeCaptureMode};

    fn git_provenance() -> GitProvenance {
        GitProvenance {
            source: RevisionSource::GitWorktree {
                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                include_untracked: true,
                pathspecs: Vec::new(),
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
                object_artifact_content_hash: "sha256:artifact".to_owned(),
                supersedes: vec![],
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
            json["workObject"]["objectArtifactContentHash"],
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
                object_artifact_content_hash: "sha256:artifact".to_owned(),
                supersedes: vec![],
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
    fn revision_arm_serializes_object_artifact_content_hash() {
        // The capture payload names the stored object artifact's content hash
        // under the object vocabulary; the legacy `snapshotArtifactContentHash`
        // key is retired.
        let payload = revision_payload();
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(
            json["workObject"]["objectArtifactContentHash"],
            "sha256:artifact"
        );
        assert!(
            json["workObject"]
                .get("snapshotArtifactContentHash")
                .is_none()
        );
    }

    #[test]
    fn task_attempt_arm_serializes_base_state_fingerprint() {
        let payload = WorkObjectProposedPayload {
            engagement_id: EngagementId::new("engagement:sha256:t"),
            work_object: WorkObjectProposal::TaskAttempt {
                task_attempt_id: WorkObjectId::new("task-attempt:sha256:a"),
                project_path: "/repo".to_owned(),
                claude_session_uuid: "uuid-1".to_owned(),
                initial_prompt_hash: "sha256:prompt".to_owned(),
                predecessor: None,
                base_state_fingerprint: Some("sha256:base".to_owned()),
                source_speaker: None,
            },
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["workObject"]["baseStateFingerprint"], "sha256:base");
        assert!(json["workObject"].get("baseSnapshotFingerprint").is_none());
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
                base_state_fingerprint: None,
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
    fn scoped_provenance_serializes_pathspecs_inside_the_source() {
        let mut provenance = git_provenance();
        provenance.source = RevisionSource::GitWorktree {
            mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
            include_untracked: true,
            pathspecs: vec!["packages/foo".to_owned()],
        };
        let payload = WorkObjectProposedPayload {
            engagement_id: EngagementId::new("engagement:sha256:e"),
            work_object: WorkObjectProposal::Revision {
                revision: Revision {
                    id: RevisionId::new("rev:sha256:r"),
                    object_id: ObjectId::new("obj:sha256:o"),
                    git_provenance: Some(provenance),
                },
                object_artifact_content_hash: "sha256:artifact".to_owned(),
                supersedes: vec![],
            },
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(
            json["workObject"]["revision"]["gitProvenance"]["source"]["pathspecs"][0],
            "packages/foo"
        );
        let parsed: WorkObjectProposedPayload = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, payload);
    }

    #[test]
    fn unscoped_provenance_payload_omits_pathspecs() {
        // The revision_payload() helper is unscoped; its serialized source must
        // carry no pathspecs key, keeping existing payload hashes and
        // signatures stable.
        let json = serde_json::to_value(&revision_payload()).unwrap();
        assert!(
            json["workObject"]["revision"]["gitProvenance"]["source"]
                .get("pathspecs")
                .is_none()
        );
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

    #[test]
    fn empty_supersedes_is_omitted_from_serialization() {
        // A root revision (no supersedes) serializes with no `supersedes` key, so
        // an existing root capture's payload hash is unchanged by the new field.
        let payload = revision_payload();
        let json = serde_json::to_value(&payload).unwrap();
        assert!(json["workObject"].get("supersedes").is_none());
    }

    #[test]
    fn non_empty_supersedes_round_trips_as_an_array() {
        let payload = WorkObjectProposedPayload {
            engagement_id: EngagementId::new("engagement:sha256:e"),
            work_object: WorkObjectProposal::Revision {
                revision: Revision {
                    id: RevisionId::new("rev:sha256:r"),
                    object_id: ObjectId::new("obj:sha256:o"),
                    git_provenance: None,
                },
                object_artifact_content_hash: "sha256:artifact".to_owned(),
                supersedes: vec![
                    RevisionId::new("rev:sha256:a"),
                    RevisionId::new("rev:sha256:b"),
                ],
            },
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["workObject"]["supersedes"][0], "rev:sha256:a");
        assert_eq!(json["workObject"]["supersedes"][1], "rev:sha256:b");

        let parsed: WorkObjectProposedPayload = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, payload);
    }

    #[test]
    fn set_equal_supersedes_converge_to_one_payload_hash() {
        use crate::session::event::{EventTarget, ShoreEvent, Writer};
        use crate::session::workflow::util::sorted_unique;

        fn event_with_supersedes(supersedes: Vec<RevisionId>) -> ShoreEvent {
            let revision_id = RevisionId::new("rev:sha256:x");
            ShoreEvent::new(
                EventType::WorkObjectProposed,
                format!("work_object_proposed:{}", revision_id.as_str()),
                EventTarget::for_revision(
                    JournalId::new("journal:default"),
                    revision_id.clone(),
                    None,
                ),
                Writer::shore_local("test"),
                WorkObjectProposedPayload {
                    engagement_id: EngagementId::new("engagement:sha256:e"),
                    work_object: WorkObjectProposal::Revision {
                        revision: Revision {
                            id: revision_id,
                            object_id: ObjectId::new("obj:sha256:o"),
                            git_provenance: None,
                        },
                        object_artifact_content_hash: "sha256:artifact".to_owned(),
                        supersedes,
                    },
                },
                "2026-06-04T00:00:00Z",
            )
            .unwrap()
        }

        let a = event_with_supersedes(sorted_unique(vec![
            RevisionId::new("rev:sha256:b"),
            RevisionId::new("rev:sha256:a"),
            RevisionId::new("rev:sha256:b"),
        ]));
        let b = event_with_supersedes(sorted_unique(vec![
            RevisionId::new("rev:sha256:a"),
            RevisionId::new("rev:sha256:b"),
        ]));
        assert_eq!(a.payload_hash, b.payload_hash);
    }
}
