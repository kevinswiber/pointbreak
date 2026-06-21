use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::error::Result;
use crate::model::{
    ReviewUnitLineageBasisV1, ReviewUnitLineageId, ReviewUnitLineageRoundId, RevisionId,
};
use crate::session::event::{
    EventType, ReviewUnitLineageDeclaredPayload, ReviewUnitLineageRoundRecordedPayload, ShoreEvent,
    WorkObjectProposal, WorkObjectProposedPayload,
};
use crate::session::state::ProjectionDiagnostic;

pub const LINEAGE_ROUND_MISSING_REVIEW_UNIT_CODE: &str = "lineage_round_missing_review_unit";
pub const LINEAGE_PREDECESSOR_OUTSIDE_LINEAGE_CODE: &str = "lineage_predecessor_outside_lineage";
pub const LINEAGE_FORKED_SUCCESSOR_CODE: &str = "lineage_forked_successor";
pub const LINEAGE_CYCLE_CODE: &str = "lineage_cycle";
pub const LINEAGE_MULTIPLE_HEADS_CODE: &str = "lineage_multiple_heads";
pub const LINEAGE_DUPLICATE_ROUND_CODE: &str = "lineage_duplicate_round";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUnitLineageProjection {
    pub lineages: BTreeMap<ReviewUnitLineageId, ReviewUnitLineageView>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUnitLineageView {
    pub lineage_id: ReviewUnitLineageId,
    pub basis: Option<ReviewUnitLineageBasisV1>,
    pub head_review_unit_id: Option<RevisionId>,
    pub rounds: Vec<ReviewUnitLineageRoundView>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUnitLineageRoundView {
    pub lineage_id: ReviewUnitLineageId,
    pub round_id: ReviewUnitLineageRoundId,
    pub review_unit_id: RevisionId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predecessor_review_unit_id: Option<RevisionId>,
    pub round_index: Option<usize>,
    pub is_head: bool,
}

impl ReviewUnitLineageProjection {
    pub fn from_events(events: &[ShoreEvent]) -> Result<Self> {
        let mut captured = BTreeSet::new();
        let mut builders = BTreeMap::<ReviewUnitLineageId, LineageBuilder>::new();

        for event in events {
            match event.event_type {
                EventType::WorkObjectProposed => {
                    let payload: WorkObjectProposedPayload =
                        serde_json::from_value(event.payload.clone())?;
                    if let WorkObjectProposal::Revision { revision, .. } = payload.work_object {
                        captured.insert(revision.id);
                    }
                }
                EventType::ReviewUnitLineageDeclared => {
                    let payload: ReviewUnitLineageDeclaredPayload =
                        serde_json::from_value(event.payload.clone())?;
                    builders
                        .entry(payload.lineage_id.clone())
                        .or_insert_with(|| LineageBuilder::new(payload.lineage_id.clone()))
                        .basis
                        .get_or_insert(payload.basis);
                }
                EventType::ReviewUnitLineageRoundRecorded => {
                    let payload: ReviewUnitLineageRoundRecordedPayload =
                        serde_json::from_value(event.payload.clone())?;
                    let builder = builders
                        .entry(payload.lineage_id.clone())
                        .or_insert_with(|| LineageBuilder::new(payload.lineage_id.clone()));
                    builder.add_round(payload);
                }
                _ => {}
            }
        }

        let lineages = builders
            .into_iter()
            .map(|(lineage_id, builder)| {
                let view = builder.finish(&captured);
                (lineage_id, view)
            })
            .collect();

        Ok(Self { lineages })
    }

    pub fn lineage(&self, lineage_id: &ReviewUnitLineageId) -> Option<&ReviewUnitLineageView> {
        self.lineages.get(lineage_id)
    }
}

#[derive(Debug)]
struct LineageBuilder {
    lineage_id: ReviewUnitLineageId,
    basis: Option<ReviewUnitLineageBasisV1>,
    rounds: BTreeMap<RevisionId, ReviewUnitLineageRoundRecordedPayload>,
    duplicate_rounds: BTreeSet<RevisionId>,
}

impl LineageBuilder {
    fn new(lineage_id: ReviewUnitLineageId) -> Self {
        Self {
            lineage_id,
            basis: None,
            rounds: BTreeMap::new(),
            duplicate_rounds: BTreeSet::new(),
        }
    }

    fn add_round(&mut self, payload: ReviewUnitLineageRoundRecordedPayload) {
        let review_unit_id = payload.review_unit_id.clone();
        if self
            .rounds
            .insert(review_unit_id.clone(), payload)
            .is_some()
        {
            self.duplicate_rounds.insert(review_unit_id);
        }
    }

    fn finish(self, captured: &BTreeSet<RevisionId>) -> ReviewUnitLineageView {
        let mut diagnostics = Vec::new();
        for duplicate in &self.duplicate_rounds {
            diagnostics.push(diagnostic(
                LINEAGE_DUPLICATE_ROUND_CODE,
                format!(
                    "lineage {} has duplicate round facts for {}",
                    self.lineage_id.as_str(),
                    duplicate.as_str()
                ),
            ));
        }

        for review_unit_id in self.rounds.keys() {
            if !captured.contains(review_unit_id) {
                diagnostics.push(diagnostic(
                    LINEAGE_ROUND_MISSING_REVIEW_UNIT_CODE,
                    format!(
                        "lineage {} references unknown ReviewUnit {}",
                        self.lineage_id.as_str(),
                        review_unit_id.as_str()
                    ),
                ));
            }
        }

        let round_ids = self.rounds.keys().cloned().collect::<BTreeSet<_>>();
        let mut successors = BTreeMap::<RevisionId, Vec<RevisionId>>::new();
        for round in self.rounds.values() {
            if let Some(predecessor) = &round.predecessor_review_unit_id {
                if !round_ids.contains(predecessor) {
                    diagnostics.push(diagnostic(
                        LINEAGE_PREDECESSOR_OUTSIDE_LINEAGE_CODE,
                        format!(
                            "lineage {} predecessor {} is not in the lineage",
                            self.lineage_id.as_str(),
                            predecessor.as_str()
                        ),
                    ));
                }
                successors
                    .entry(predecessor.clone())
                    .or_default()
                    .push(round.review_unit_id.clone());
            }
        }

        for (predecessor, successor_ids) in &successors {
            if successor_ids.len() > 1 {
                diagnostics.push(diagnostic(
                    LINEAGE_FORKED_SUCCESSOR_CODE,
                    format!(
                        "lineage {} has {} successors from {}",
                        self.lineage_id.as_str(),
                        successor_ids.len(),
                        predecessor.as_str()
                    ),
                ));
            }
        }

        let predecessor_set = successors.keys().cloned().collect::<BTreeSet<_>>();
        let head_candidates = round_ids
            .iter()
            .filter(|review_unit_id| !predecessor_set.contains(*review_unit_id))
            .cloned()
            .collect::<Vec<_>>();
        if self.rounds.len() > 1 && head_candidates.is_empty() {
            diagnostics.push(diagnostic(
                LINEAGE_CYCLE_CODE,
                format!("lineage {} contains a cycle", self.lineage_id.as_str()),
            ));
        }
        if head_candidates.len() > 1 {
            diagnostics.push(diagnostic(
                LINEAGE_MULTIPLE_HEADS_CODE,
                format!(
                    "lineage {} has {} head candidates",
                    self.lineage_id.as_str(),
                    head_candidates.len()
                ),
            ));
        }

        let head_review_unit_id = if diagnostics.is_empty() && head_candidates.len() == 1 {
            Some(head_candidates[0].clone())
        } else {
            None
        };
        let round_indexes = round_indexes(&self.rounds, &successors);
        let mut rounds = self
            .rounds
            .into_values()
            .map(|round| {
                let is_head = head_review_unit_id.as_ref() == Some(&round.review_unit_id);
                ReviewUnitLineageRoundView {
                    lineage_id: round.lineage_id,
                    round_id: round.round_id,
                    round_index: round_indexes.get(&round.review_unit_id).copied(),
                    review_unit_id: round.review_unit_id,
                    predecessor_review_unit_id: round.predecessor_review_unit_id,
                    is_head,
                }
            })
            .collect::<Vec<_>>();
        rounds.sort_by(|left, right| {
            left.round_index.cmp(&right.round_index).then_with(|| {
                left.review_unit_id
                    .as_str()
                    .cmp(right.review_unit_id.as_str())
            })
        });

        ReviewUnitLineageView {
            lineage_id: self.lineage_id,
            basis: self.basis,
            head_review_unit_id,
            rounds,
            diagnostics,
        }
    }
}

fn round_indexes(
    rounds: &BTreeMap<RevisionId, ReviewUnitLineageRoundRecordedPayload>,
    successors: &BTreeMap<RevisionId, Vec<RevisionId>>,
) -> BTreeMap<RevisionId, usize> {
    let roots = rounds
        .values()
        .filter(|round| round.predecessor_review_unit_id.is_none())
        .map(|round| round.review_unit_id.clone())
        .collect::<Vec<_>>();
    let mut indexes = BTreeMap::new();
    for root in roots {
        assign_round_indexes(&root, 0, successors, &mut indexes);
    }
    indexes
}

fn assign_round_indexes(
    review_unit_id: &RevisionId,
    index: usize,
    successors: &BTreeMap<RevisionId, Vec<RevisionId>>,
    indexes: &mut BTreeMap<RevisionId, usize>,
) {
    if indexes.insert(review_unit_id.clone(), index).is_some() {
        return;
    }
    if let Some(next) = successors.get(review_unit_id) {
        for successor in next {
            assign_round_indexes(successor, index + 1, successors, indexes);
        }
    }
}

fn diagnostic(code: &str, message: String) -> ProjectionDiagnostic {
    ProjectionDiagnostic {
        code: code.to_owned(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{
        EngagementId, LedgerId, ReviewEndpoint, ReviewUnitLineageBasisV1, ReviewUnitLineageId,
        ReviewUnitLineageRoundId, ReviewUnitSource, RevisionId, WorktreeCaptureMode,
    };
    use crate::session::event::{
        EventTarget, EventType, GitProvenance, ReviewUnitLineageDeclaredPayload,
        ReviewUnitLineageRoundRecordedPayload, Revision, ShoreEvent, WorkObjectProposal,
        WorkObjectProposedPayload, Writer,
    };
    use crate::session::projection::lineage::ReviewUnitLineageProjection;

    #[test]
    fn single_capture_lineage_has_one_head() {
        let events = vec![
            review_unit_captured("one"),
            lineage_declared("lineage-a"),
            lineage_round("lineage-a", "one", None),
        ];

        let projection = ReviewUnitLineageProjection::from_events(&events).unwrap();
        let lineage = projection
            .lineage(&review_unit_lineage_id("lineage-a"))
            .unwrap();

        assert_eq!(
            lineage.head_review_unit_id.as_ref(),
            Some(&review_unit_id("one"))
        );
    }

    #[test]
    fn same_lineage_successor_becomes_head() {
        let events = vec![
            review_unit_captured("one"),
            review_unit_captured("two"),
            lineage_declared("lineage-a"),
            lineage_round("lineage-a", "one", None),
            lineage_round("lineage-a", "two", Some("one")),
        ];

        let projection = ReviewUnitLineageProjection::from_events(&events).unwrap();
        let lineage = projection
            .lineage(&review_unit_lineage_id("lineage-a"))
            .unwrap();

        assert_eq!(
            lineage.head_review_unit_id.as_ref(),
            Some(&review_unit_id("two"))
        );
        assert_eq!(lineage.rounds[0].round_index, Some(0));
        assert_eq!(lineage.rounds[1].round_index, Some(1));
    }

    #[test]
    fn different_lineages_have_independent_heads() {
        let events = vec![
            review_unit_captured("one"),
            review_unit_captured("two"),
            lineage_declared("lineage-a"),
            lineage_declared("lineage-b"),
            lineage_round("lineage-a", "one", None),
            lineage_round("lineage-b", "two", None),
        ];

        let projection = ReviewUnitLineageProjection::from_events(&events).unwrap();

        assert_eq!(
            projection
                .lineage(&review_unit_lineage_id("lineage-a"))
                .unwrap()
                .head_review_unit_id
                .as_ref(),
            Some(&review_unit_id("one"))
        );
        assert_eq!(
            projection
                .lineage(&review_unit_lineage_id("lineage-b"))
                .unwrap()
                .head_review_unit_id
                .as_ref(),
            Some(&review_unit_id("two"))
        );
    }

    #[test]
    fn missing_captured_review_unit_is_diagnostic_without_head() {
        let events = vec![
            lineage_declared("lineage-a"),
            lineage_round("lineage-a", "missing", None),
        ];

        let projection = ReviewUnitLineageProjection::from_events(&events).unwrap();
        let lineage = projection
            .lineage(&review_unit_lineage_id("lineage-a"))
            .unwrap();

        assert!(lineage.head_review_unit_id.is_none());
        assert!(
            lineage
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code == "lineage_round_missing_review_unit" })
        );
    }

    #[test]
    fn predecessor_outside_lineage_is_diagnostic_without_head() {
        let events = vec![
            review_unit_captured("one"),
            review_unit_captured("two"),
            lineage_declared("lineage-a"),
            lineage_declared("lineage-b"),
            lineage_round("lineage-b", "one", None),
            lineage_round("lineage-a", "two", Some("one")),
        ];

        let projection = ReviewUnitLineageProjection::from_events(&events).unwrap();
        let lineage = projection
            .lineage(&review_unit_lineage_id("lineage-a"))
            .unwrap();

        assert!(lineage.head_review_unit_id.is_none());
        assert!(
            lineage
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code == "lineage_predecessor_outside_lineage" })
        );
    }

    #[test]
    fn forked_successor_is_diagnostic_without_head() {
        let events = vec![
            review_unit_captured("one"),
            review_unit_captured("two"),
            review_unit_captured("three"),
            lineage_declared("lineage-a"),
            lineage_round("lineage-a", "one", None),
            lineage_round("lineage-a", "two", Some("one")),
            lineage_round("lineage-a", "three", Some("one")),
        ];

        let projection = ReviewUnitLineageProjection::from_events(&events).unwrap();
        let lineage = projection
            .lineage(&review_unit_lineage_id("lineage-a"))
            .unwrap();

        assert!(lineage.head_review_unit_id.is_none());
        assert!(
            lineage
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code == "lineage_forked_successor" })
        );
    }

    #[test]
    fn cycle_is_diagnostic_without_head() {
        let events = vec![
            review_unit_captured("one"),
            review_unit_captured("two"),
            lineage_declared("lineage-a"),
            lineage_round("lineage-a", "one", Some("two")),
            lineage_round("lineage-a", "two", Some("one")),
        ];

        let projection = ReviewUnitLineageProjection::from_events(&events).unwrap();
        let lineage = projection
            .lineage(&review_unit_lineage_id("lineage-a"))
            .unwrap();

        assert!(lineage.head_review_unit_id.is_none());
        assert!(
            lineage
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "lineage_cycle")
        );
    }

    fn review_unit_captured(suffix: &str) -> ShoreEvent {
        // A real capture stamps the envelope subject and the payload revision with
        // one minted id; the projection reads the captured revision from the
        // payload, so both must carry the same id here.
        let revision_id = review_unit_id(suffix);
        let snapshot_id = crate::model::ObjectId::new(format!("obj:sha256:{suffix}"));
        ShoreEvent::new(
            EventType::WorkObjectProposed,
            format!("work_object_proposed:{}", revision_id.as_str()),
            EventTarget::for_revision(LedgerId::new("ledger:default"), revision_id.clone(), None),
            Writer::shore_local("test"),
            WorkObjectProposedPayload {
                engagement_id: EngagementId::new(format!(
                    "engagement:sha256:{}",
                    crate::canonical_hash::sha256_bytes_hex(revision_id.as_str().as_bytes())
                )),
                work_object: WorkObjectProposal::Revision {
                    revision: Revision {
                        id: revision_id.clone(),
                        object_id: snapshot_id.clone(),
                        git_provenance: Some(GitProvenance {
                            source: ReviewUnitSource::GitWorktree {
                                mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                                include_untracked: true,
                            },
                            base: ReviewEndpoint::GitCommit {
                                commit_oid: "base".to_owned(),
                                tree_oid: "base-tree".to_owned(),
                            },
                            target: ReviewEndpoint::GitWorkingTree {
                                worktree_root: "/repo".to_owned(),
                            },
                        }),
                    },
                    snapshot_artifact_content_hash: format!("sha256:artifact:{suffix}"),
                },
            },
            "2026-06-04T00:00:00Z",
        )
        .unwrap()
    }

    fn lineage_declared(suffix: &str) -> ShoreEvent {
        let lineage_id = review_unit_lineage_id(suffix);
        ShoreEvent::new(
            EventType::ReviewUnitLineageDeclared,
            ReviewUnitLineageDeclaredPayload::idempotency_key(&lineage_id),
            EventTarget::for_review_unit_lineage(
                LedgerId::new("session:default"),
                lineage_id.clone(),
            ),
            Writer::shore_local("test"),
            ReviewUnitLineageDeclaredPayload {
                lineage_id,
                basis: ReviewUnitLineageBasisV1::new(
                    ReviewUnitSource::GitWorktree {
                        mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                        include_untracked: true,
                    },
                    ReviewEndpoint::GitCommit {
                        commit_oid: "base".to_owned(),
                        tree_oid: "base-tree".to_owned(),
                    },
                ),
            },
            "2026-06-04T00:00:00Z",
        )
        .unwrap()
    }

    fn lineage_round(
        lineage_suffix: &str,
        review_unit_suffix: &str,
        predecessor_suffix: Option<&str>,
    ) -> ShoreEvent {
        let lineage_id = review_unit_lineage_id(lineage_suffix);
        let unit_id = review_unit_id(review_unit_suffix);
        ShoreEvent::new(
            EventType::ReviewUnitLineageRoundRecorded,
            ReviewUnitLineageRoundRecordedPayload::idempotency_key(&lineage_id, &unit_id),
            EventTarget::for_review_unit_lineage(
                LedgerId::new("session:default"),
                lineage_id.clone(),
            ),
            Writer::shore_local("test"),
            ReviewUnitLineageRoundRecordedPayload {
                lineage_id,
                round_id: ReviewUnitLineageRoundId::new(format!(
                    "review-unit-lineage-round:sha256:{review_unit_suffix}"
                )),
                review_unit_id: unit_id,
                predecessor_review_unit_id: predecessor_suffix.map(review_unit_id),
                change_id: None,
            },
            "2026-06-04T00:00:01Z",
        )
        .unwrap()
    }

    fn review_unit_id(suffix: &str) -> RevisionId {
        RevisionId::new(format!("review-unit:sha256:{suffix}"))
    }

    fn review_unit_lineage_id(suffix: &str) -> ReviewUnitLineageId {
        ReviewUnitLineageId::new(format!("review-unit-lineage:sha256:{suffix}"))
    }
}
