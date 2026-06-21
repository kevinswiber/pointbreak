use serde::{Deserialize, Serialize};

use crate::canonical_hash::sha256_json_hex;
use crate::error::Result;
use crate::model::{
    ReviewEndpoint, ReviewUnitLineageId, ReviewUnitLineageRoundId, ReviewUnitSource, RevisionId,
};

const LINEAGE_BASIS_SCHEMA: &str = "shore.review-unit-lineage-basis";
const LINEAGE_BASIS_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUnitLineageBasisV1 {
    pub schema: String,
    pub version: u32,
    pub source: ReviewUnitSource,
    pub base: ReviewEndpoint,
}

impl ReviewUnitLineageBasisV1 {
    pub fn new(source: ReviewUnitSource, base: ReviewEndpoint) -> Self {
        Self {
            schema: LINEAGE_BASIS_SCHEMA.to_owned(),
            version: LINEAGE_BASIS_VERSION,
            source,
            base,
        }
    }

    pub fn from_capture_parts(source: &ReviewUnitSource, base: &ReviewEndpoint) -> Result<Self> {
        Ok(Self::new(source.clone(), base.clone()))
    }
}

impl ReviewUnitLineageRoundId {
    pub fn from_lineage_review_unit(
        lineage_id: &ReviewUnitLineageId,
        review_unit_id: &RevisionId,
    ) -> Result<Self> {
        Ok(Self::new(format!(
            "review-unit-lineage-round:sha256:{}",
            sha256_json_hex(&(lineage_id, review_unit_id))?
        )))
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{
        ReviewEndpoint, ReviewUnitLineageBasisV1, ReviewUnitSource, WorktreeCaptureMode,
    };
    use crate::session::event::GitProvenance;

    #[test]
    fn lineage_basis_serialization_excludes_worktree_paths() {
        let basis = lineage_basis();
        let json = serde_json::to_string(&basis).unwrap();

        assert!(!json.contains("/Users/"));
        assert!(!json.contains("worktreeRoot"));
        assert!(!json.contains(".shore/data"));
        assert!(!json.contains(".git"));
    }

    #[test]
    fn lineage_basis_ignores_worktree_root() {
        let first = capture_provenance("/Users/kevin/worktrees/shoreline/one");
        let second = capture_provenance("/Users/kevin/worktrees/shoreline/two");

        let first_basis =
            ReviewUnitLineageBasisV1::from_capture_parts(&first.source, &first.base).unwrap();
        let second_basis =
            ReviewUnitLineageBasisV1::from_capture_parts(&second.source, &second.base).unwrap();

        assert_eq!(first_basis, second_basis);
    }

    fn lineage_basis() -> ReviewUnitLineageBasisV1 {
        let capture = capture_provenance("/Users/kevin/worktrees/shoreline/one");
        ReviewUnitLineageBasisV1::from_capture_parts(&capture.source, &capture.base).unwrap()
    }

    fn capture_provenance(worktree_root: impl Into<String>) -> GitProvenance {
        GitProvenance {
            source: ReviewUnitSource::GitWorktree {
                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                include_untracked: true,
            },
            base: ReviewEndpoint::GitCommit {
                commit_oid: "base".to_owned(),
                tree_oid: "base-tree".to_owned(),
            },
            target: ReviewEndpoint::GitWorkingTree {
                worktree_root: worktree_root.into(),
            },
        }
    }
}
