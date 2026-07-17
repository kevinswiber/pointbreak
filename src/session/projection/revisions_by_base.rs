use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::error::Result;
use crate::model::{ReviewEndpoint, RevisionId};
use crate::session::event::{EventType, ShoreEvent, WorkObjectProposal, WorkObjectProposedPayload};

/// Groups revisions by their optional git base endpoint — strictly a provenance
/// view, orthogonal to supersession. A revision over a non-git object (no git
/// provenance) is absent from every bucket, so this projection is empty for a
/// git-less store. This is a derived read-time index, never an authoritative
/// declared basis (the retired lineage mistake).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionsByBase {
    /// Base commit oid -> the revisions captured from it.
    pub buckets: BTreeMap<String, BTreeSet<RevisionId>>,
}

impl RevisionsByBase {
    pub fn from_events(events: &[ShoreEvent]) -> Result<Self> {
        let mut buckets: BTreeMap<String, BTreeSet<RevisionId>> = BTreeMap::new();
        for event in events
            .iter()
            .filter(|event| event.event_type == EventType::WorkObjectProposed)
        {
            let payload: WorkObjectProposedPayload = serde_json::from_value(event.payload.clone())?;
            // Discriminate the arm: only a review-domain revision has a git base;
            // a task-attempt proposal is skipped, never decoded as a revision.
            if let WorkObjectProposal::Revision { revision, .. } = payload.work_object
                && let Some(provenance) = &revision.git_provenance
                && let ReviewEndpoint::GitCommit { commit_oid, .. } = &provenance.base
            {
                buckets
                    .entry(commit_oid.clone())
                    .or_default()
                    .insert(revision.id.clone());
            }
        }
        Ok(Self { buckets })
    }

    /// The revisions captured from `base_commit_oid` (empty when none).
    pub fn bucket(&self, base_commit_oid: &str) -> BTreeSet<RevisionId> {
        self.buckets
            .get(base_commit_oid)
            .cloned()
            .unwrap_or_default()
    }

    /// Whether any revision is bucketed (false for a git-less store).
    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }

    /// Whether `revision` appears in any base bucket (i.e. it has a git base).
    pub fn contains(&self, revision: &RevisionId) -> bool {
        self.buckets.values().any(|set| set.contains(revision))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        EngagementId, JournalId, ObjectId, RevisionSource, RootCommitCaptureMode,
        WorktreeCaptureMode,
    };
    use crate::session::event::{EventTarget, GitProvenance, Revision, Writer};

    fn rev(suffix: &str) -> RevisionId {
        RevisionId::new(format!("rev:sha256:{suffix}"))
    }

    fn revision_event(suffix: &str, git_provenance: Option<GitProvenance>) -> ShoreEvent {
        let revision_id = rev(suffix);
        ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("work_object_proposed:{}", revision_id.as_str()),
            EventTarget::for_revision(JournalId::new("journal:default"), revision_id.clone(), None)
                .unwrap(),
            Writer::shore_local("test"),
            WorkObjectProposedPayload {
                engagement_id: EngagementId::new(format!("engagement:sha256:{suffix}")),
                work_object: WorkObjectProposal::Revision {
                    revision: Revision {
                        id: revision_id,
                        object_id: ObjectId::new(format!("obj:sha256:{suffix}")),
                        git_provenance,
                    },
                    summary: None,
                    object_artifact_content_hash: format!("sha256:artifact:{suffix}"),
                    supersedes: vec![],
                },
            },
            "2026-06-04T00:00:00Z",
        )
        .unwrap()
    }

    fn git_provenance_based_at(commit_oid: &str) -> GitProvenance {
        GitProvenance {
            source: RevisionSource::GitWorktree {
                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                include_untracked: true,
                pathspecs: Vec::new(),
            },
            base: ReviewEndpoint::GitCommit {
                commit_oid: commit_oid.to_owned(),
                tree_oid: format!("{commit_oid}-tree"),
            },
            target: ReviewEndpoint::GitWorkingTree {
                worktree_root: "/repo".to_owned(),
            },
        }
    }

    fn root_git_provenance() -> GitProvenance {
        GitProvenance {
            source: RevisionSource::GitRootCommit {
                mode: RootCommitCaptureMode::EmptyTreeToTargetTree,
                pathspecs: Vec::new(),
            },
            base: ReviewEndpoint::GitTree {
                tree_oid: "empty-tree".to_owned(),
            },
            target: ReviewEndpoint::GitCommit {
                commit_oid: "target".to_owned(),
                tree_oid: "target-tree".to_owned(),
            },
        }
    }

    fn index_based_git_provenance() -> GitProvenance {
        GitProvenance {
            source: RevisionSource::GitWorktree {
                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                include_untracked: true,
                pathspecs: Vec::new(),
            },
            base: ReviewEndpoint::GitIndex {
                tree_oid: "index-tree".to_owned(),
            },
            target: ReviewEndpoint::GitWorkingTree {
                worktree_root: "/repo".to_owned(),
            },
        }
    }

    #[test]
    fn buckets_git_revisions_by_base_and_excludes_a_non_git_object() {
        let events = vec![
            revision_event("g", Some(git_provenance_based_at("base-1"))),
            // A non-git (markdown-set) revision: no git provenance.
            revision_event("m", None),
        ];
        let by_base = RevisionsByBase::from_events(&events).unwrap();

        assert!(by_base.bucket("base-1").contains(&rev("g")));
        assert!(!by_base.contains(&rev("m")));
    }

    #[test]
    fn two_revisions_sharing_a_base_land_in_one_bucket() {
        let events = vec![
            revision_event("a", Some(git_provenance_based_at("base-1"))),
            revision_event("b", Some(git_provenance_based_at("base-1"))),
        ];
        let by_base = RevisionsByBase::from_events(&events).unwrap();

        assert_eq!(
            by_base.bucket("base-1"),
            [rev("a"), rev("b")].into_iter().collect()
        );
    }

    #[test]
    fn is_empty_for_a_git_less_store() {
        let events = vec![revision_event("m", None)];
        assert!(RevisionsByBase::from_events(&events).unwrap().is_empty());
    }

    #[test]
    fn revision_source_tree_and_index_bases_are_not_commit_base_buckets() {
        let events = vec![
            revision_event("root", Some(root_git_provenance())),
            revision_event("index", Some(index_based_git_provenance())),
            revision_event("commit", Some(git_provenance_based_at("base-1"))),
        ];
        let by_base = RevisionsByBase::from_events(&events).unwrap();

        assert_eq!(
            by_base.bucket("base-1"),
            [rev("commit")].into_iter().collect()
        );
        assert!(!by_base.contains(&rev("root")));
        assert!(!by_base.contains(&rev("index")));
    }
}
