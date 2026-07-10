use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::error::Result;
use crate::model::RevisionId;
use crate::session::event::{EventType, ShoreEvent, WorkObjectProposal, WorkObjectProposedPayload};
use crate::session::state::ProjectionDiagnostic;

/// A revision names a target it supersedes that is not (yet) a known revision.
/// Self-heals once the target backfills as a known revision; never rejects.
pub const SUPERSESSION_TARGET_MISSING_CODE: &str = "supersession_target_missing";
/// A set of revisions supersede each other in a cycle. Scoped to the cycle's
/// revisions; never rejects.
pub const SUPERSESSION_CYCLE_CODE: &str = "supersession_cycle";

/// A fork-tolerant, status-tagging projection of the supersession relation over a
/// set of revisions. Each revision either supersedes zero or more earlier
/// revisions or is itself superseded; a revision not in `superseded` is a current
/// head. Competing heads (a fork) are surfaced, never nulled and never an error;
/// a cycle or a dangling supersession target yields a self-healing diagnostic
/// rather than a rejected write.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupersessionView {
    /// Every current head, store-wide (across all threads). A revision is a head
    /// iff it is not superseded by any known revision.
    pub heads: BTreeSet<RevisionId>,
    /// The union of every revision's superseded targets (restricted to known
    /// revisions).
    pub superseded: BTreeSet<RevisionId>,
    /// Forward map: a revision to the revisions it supersedes (as declared,
    /// including not-yet-known targets).
    pub supersedes: BTreeMap<RevisionId, BTreeSet<RevisionId>>,
    /// Reverse map: a known revision to its direct superseders. Lets a stale-fact
    /// projection name every superseding successor of a target.
    pub superseded_by: BTreeMap<RevisionId, BTreeSet<RevisionId>>,
    /// Connected components of the undirected supersession graph (restricted to
    /// known revisions). A component is one thread; head-selection scopes to it.
    pub components: Vec<BTreeSet<RevisionId>>,
    /// Every revision participating in a directed supersession cycle, flattened
    /// across cycles. Suppression guards consult this to refuse
    /// judgment-subsumption inside a cycled thread; never serialized.
    #[serde(skip)]
    pub cycle_revisions: BTreeSet<RevisionId>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

impl SupersessionView {
    /// Builds the view from synthetic `revision -> supersedes` edges.
    ///
    /// Input contract: exactly one entry per *known* revision, roots included
    /// (a root is `(id, vec![])`). The set of left-hand keys is the known-revision
    /// set; a supersession target not in that set is dangling.
    pub fn from_edges(edges: impl IntoIterator<Item = (RevisionId, Vec<RevisionId>)>) -> Self {
        // Normalize: one declared entry per known revision; collapse each
        // revision's supersession targets to a set (dedup, order-independent).
        let mut declared: BTreeMap<RevisionId, BTreeSet<RevisionId>> = BTreeMap::new();
        for (revision, targets) in edges {
            declared.entry(revision).or_default().extend(targets);
        }
        let known: BTreeSet<RevisionId> = declared.keys().cloned().collect();

        let mut superseded: BTreeSet<RevisionId> = BTreeSet::new();
        let mut supersedes: BTreeMap<RevisionId, BTreeSet<RevisionId>> = BTreeMap::new();
        let mut superseded_by: BTreeMap<RevisionId, BTreeSet<RevisionId>> = BTreeMap::new();
        let mut diagnostics: Vec<ProjectionDiagnostic> = Vec::new();

        for (revision, targets) in &declared {
            if !targets.is_empty() {
                supersedes.insert(revision.clone(), targets.clone());
            }
            for target in targets {
                if known.contains(target) {
                    superseded.insert(target.clone());
                    superseded_by
                        .entry(target.clone())
                        .or_default()
                        .insert(revision.clone());
                } else {
                    diagnostics.push(ProjectionDiagnostic {
                        code: SUPERSESSION_TARGET_MISSING_CODE.to_owned(),
                        message: format!(
                            "revision {} supersedes unknown revision {}",
                            revision.as_str(),
                            target.as_str()
                        ),
                    });
                }
            }
        }

        let heads: BTreeSet<RevisionId> = known.difference(&superseded).cloned().collect();
        let components = connected_components(&known, &declared);

        let mut cycle_revisions: BTreeSet<RevisionId> = BTreeSet::new();
        for cycle in directed_cycles(&known, &declared) {
            let members = cycle
                .iter()
                .map(RevisionId::as_str)
                .collect::<Vec<_>>()
                .join(", ");
            diagnostics.push(ProjectionDiagnostic {
                code: SUPERSESSION_CYCLE_CODE.to_owned(),
                message: format!("revisions form a supersession cycle: {members}"),
            });
            cycle_revisions.extend(cycle);
        }

        Self {
            heads,
            superseded,
            supersedes,
            superseded_by,
            components,
            cycle_revisions,
            diagnostics,
        }
    }

    /// Builds the view from the event log: each review-domain generative move
    /// yields one `(revision_id, supersedes)` edge. The task-attempt arm is
    /// skipped (it carries no supersession in this revision-domain projection),
    /// never decoded as a revision — discriminating by the payload arm, never by
    /// the event type alone.
    pub fn from_events(events: &[ShoreEvent]) -> Result<Self> {
        let mut edges: Vec<(RevisionId, Vec<RevisionId>)> = Vec::new();
        for event in events
            .iter()
            .filter(|event| event.event_type == EventType::WorkObjectProposed)
        {
            let payload: WorkObjectProposedPayload = serde_json::from_value(event.payload.clone())?;
            if let WorkObjectProposal::Revision {
                revision,
                supersedes,
                ..
            } = payload.work_object
            {
                edges.push((revision.id, supersedes));
            }
        }
        Ok(Self::from_edges(edges))
    }

    /// The connected component (thread) containing `revision`, or `None` when the
    /// revision is unknown.
    pub fn component_of(&self, revision: &RevisionId) -> Option<&BTreeSet<RevisionId>> {
        self.components
            .iter()
            .find(|component| component.contains(revision))
    }

    /// The current heads of `revision`'s thread: its component intersected with
    /// the global head set (`>= 2` means competing heads). Thread-scoped, never
    /// the global `heads`.
    pub fn heads_for(&self, revision: &RevisionId) -> BTreeSet<RevisionId> {
        match self.component_of(revision) {
            Some(component) => component.intersection(&self.heads).cloned().collect(),
            None => BTreeSet::new(),
        }
    }

    /// The superseding successors of `revision` — every revision that directly
    /// supersedes it. Empty when `revision` is a head (a fact on it is current);
    /// non-empty marks a fact on it as stale, naming *all* superseding successors
    /// (improving on a single-head flag).
    pub fn stale_by_superseding_revision(&self, revision: &RevisionId) -> BTreeSet<RevisionId> {
        self.superseded_by
            .get(revision)
            .cloned()
            .unwrap_or_default()
    }
}

/// Union-find over the *undirected* supersession graph (an edge for every
/// declared `revision -> target` pair where both endpoints are known). Each
/// known revision lands in exactly one component; an isolated revision is its
/// own component.
fn connected_components(
    known: &BTreeSet<RevisionId>,
    declared: &BTreeMap<RevisionId, BTreeSet<RevisionId>>,
) -> Vec<BTreeSet<RevisionId>> {
    let nodes: Vec<&RevisionId> = known.iter().collect();
    let index_of: BTreeMap<&RevisionId, usize> =
        nodes.iter().enumerate().map(|(i, r)| (*r, i)).collect();
    let mut parent: Vec<usize> = (0..nodes.len()).collect();

    for (revision, targets) in declared {
        let Some(&a) = index_of.get(revision) else {
            continue;
        };
        for target in targets {
            if let Some(&b) = index_of.get(target) {
                let ra = find(&mut parent, a);
                let rb = find(&mut parent, b);
                if ra != rb {
                    parent[ra] = rb;
                }
            }
        }
    }

    let mut groups: BTreeMap<usize, BTreeSet<RevisionId>> = BTreeMap::new();
    for (i, revision) in nodes.iter().enumerate() {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().insert((*revision).clone());
    }
    groups.into_values().collect()
}

fn find(parent: &mut [usize], mut node: usize) -> usize {
    while parent[node] != node {
        parent[node] = parent[parent[node]];
        node = parent[node];
    }
    node
}

/// Strongly-connected components of the *directed* supersession graph (edge
/// `revision -> target` for known targets) that are cycles: an SCC of more than
/// one revision, or a single revision that supersedes itself. Iterative Tarjan,
/// so a deep supersession chain cannot overflow the stack.
fn directed_cycles(
    known: &BTreeSet<RevisionId>,
    declared: &BTreeMap<RevisionId, BTreeSet<RevisionId>>,
) -> Vec<BTreeSet<RevisionId>> {
    let nodes: Vec<&RevisionId> = known.iter().collect();
    let index_of: BTreeMap<&RevisionId, usize> =
        nodes.iter().enumerate().map(|(i, r)| (*r, i)).collect();
    let n = nodes.len();
    let adjacency: Vec<Vec<usize>> = nodes
        .iter()
        .map(|revision| {
            declared
                .get(*revision)
                .into_iter()
                .flatten()
                .filter_map(|target| index_of.get(target).copied())
                .collect()
        })
        .collect();

    const UNVISITED: usize = usize::MAX;
    let mut index = vec![UNVISITED; n];
    let mut lowlink = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut next_index = 0usize;
    let mut sccs: Vec<Vec<usize>> = Vec::new();

    for start in 0..n {
        if index[start] != UNVISITED {
            continue;
        }
        // Each frame is (node, next-unvisited-child cursor into adjacency[node]).
        let mut call_stack: Vec<(usize, usize)> = vec![(start, 0)];
        while let Some(&(node, cursor)) = call_stack.last() {
            if cursor == 0 {
                index[node] = next_index;
                lowlink[node] = next_index;
                next_index += 1;
                stack.push(node);
                on_stack[node] = true;
            }
            if cursor < adjacency[node].len() {
                let child = adjacency[node][cursor];
                call_stack.last_mut().unwrap().1 += 1;
                if index[child] == UNVISITED {
                    call_stack.push((child, 0));
                } else if on_stack[child] {
                    lowlink[node] = lowlink[node].min(index[child]);
                }
            } else {
                if lowlink[node] == index[node] {
                    let mut scc = Vec::new();
                    loop {
                        let popped = stack.pop().expect("tarjan stack non-empty at scc root");
                        on_stack[popped] = false;
                        scc.push(popped);
                        if popped == node {
                            break;
                        }
                    }
                    sccs.push(scc);
                }
                call_stack.pop();
                if let Some(&(parent, _)) = call_stack.last() {
                    lowlink[parent] = lowlink[parent].min(lowlink[node]);
                }
            }
        }
    }

    sccs.into_iter()
        .filter(|scc| scc.len() > 1 || (scc.len() == 1 && adjacency[scc[0]].contains(&scc[0])))
        .map(|scc| scc.into_iter().map(|i| nodes[i].clone()).collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rev(suffix: &str) -> RevisionId {
        RevisionId::new(format!("rev:sha256:{suffix}"))
    }

    fn set<const N: usize>(items: [RevisionId; N]) -> BTreeSet<RevisionId> {
        items.into_iter().collect()
    }

    #[test]
    fn current_head_is_the_revision_not_in_superseded() {
        // A <- B (B supersedes A): heads = {B}, superseded = {A}. A is a KNOWN
        // root -> explicit (A, []), so it is not dangling.
        let v = SupersessionView::from_edges([(rev("A"), vec![]), (rev("B"), vec![rev("A")])]);
        assert_eq!(v.heads, set([rev("B")]));
        assert_eq!(v.superseded, set([rev("A")]));
        assert!(v.diagnostics.is_empty());
    }

    #[test]
    fn fork_surfaces_competing_heads_never_nulls() {
        // A superseded by BOTH B and C: heads = {B, C} (competing), never a null
        // head, never an error.
        let v = SupersessionView::from_edges([
            (rev("A"), vec![]),
            (rev("B"), vec![rev("A")]),
            (rev("C"), vec![rev("A")]),
        ]);
        assert_eq!(v.heads, set([rev("B"), rev("C")]));
        assert!(v.diagnostics.is_empty());
    }

    #[test]
    fn superseded_by_names_all_direct_superseders() {
        // A superseded by B and C -> superseded_by[A] == {B, C} (what the
        // stale-fact projection consumes).
        let v = SupersessionView::from_edges([
            (rev("A"), vec![]),
            (rev("B"), vec![rev("A")]),
            (rev("C"), vec![rev("A")]),
        ]);
        assert_eq!(
            v.superseded_by.get(&rev("A")),
            Some(&set([rev("B"), rev("C")]))
        );
    }

    #[test]
    fn supersedes_is_set_deduped_per_revision() {
        // Duplicate declared targets collapse to a set.
        let v = SupersessionView::from_edges([
            (rev("A"), vec![]),
            (rev("B"), vec![]),
            (rev("C"), vec![rev("A"), rev("B"), rev("A")]),
        ]);
        assert_eq!(
            v.supersedes.get(&rev("C")),
            Some(&set([rev("A"), rev("B")]))
        );
    }

    #[test]
    fn cycle_members_are_exposed_for_suppression_guards() {
        // a <-> b cycle plus an unrelated acyclic thread: cycle_revisions names
        // exactly the cycle's members, never the healthy thread.
        let v = SupersessionView::from_edges([
            (rev("A"), vec![rev("B")]),
            (rev("B"), vec![rev("A")]),
            (rev("X"), vec![]),
            (rev("Y"), vec![rev("X")]),
        ]);
        assert_eq!(v.cycle_revisions, set([rev("A"), rev("B")]));

        let acyclic =
            SupersessionView::from_edges([(rev("X"), vec![]), (rev("Y"), vec![rev("X")])]);
        assert!(acyclic.cycle_revisions.is_empty());
    }

    #[test]
    fn cycle_yields_a_diagnostic_affecting_only_the_cycle() {
        let v =
            SupersessionView::from_edges([(rev("A"), vec![rev("B")]), (rev("B"), vec![rev("A")])]);
        let cycle = v
            .diagnostics
            .iter()
            .find(|d| d.code == SUPERSESSION_CYCLE_CODE)
            .expect("cycle diagnostic");
        assert!(cycle.message.contains("rev:sha256:A"));
        assert!(cycle.message.contains("rev:sha256:B"));
    }

    #[test]
    fn dangling_supersedes_self_heals_and_never_rejects() {
        // B supersedes a not-yet-present X: diagnostic, but B is a valid head and
        // the write is never rejected.
        let v = SupersessionView::from_edges([(rev("B"), vec![rev("X")])]);
        assert!(
            v.diagnostics
                .iter()
                .any(|d| d.code == SUPERSESSION_TARGET_MISSING_CODE)
        );
        assert!(v.heads.contains(&rev("B")));
    }

    #[test]
    fn dangling_target_self_heals_when_it_backfills() {
        // Once X is a known revision, the dangling diagnostic clears and the
        // supersession edge resolves (X is superseded by B).
        let v = SupersessionView::from_edges([(rev("X"), vec![]), (rev("B"), vec![rev("X")])]);
        assert!(v.diagnostics.is_empty());
        assert_eq!(v.superseded, set([rev("X")]));
        assert_eq!(v.heads, set([rev("B")]));
    }

    #[test]
    fn heads_for_is_thread_scoped_across_unrelated_components() {
        // Thread 1: A <- {B, C} (competing). Thread 2 (unrelated): Z (root).
        let v = SupersessionView::from_edges([
            (rev("A"), vec![]),
            (rev("B"), vec![rev("A")]),
            (rev("C"), vec![rev("A")]),
            (rev("Z"), vec![]),
        ]);
        assert_eq!(v.heads, set([rev("B"), rev("C"), rev("Z")])); // global heads span both threads
        assert_eq!(v.heads_for(&rev("A")), set([rev("B"), rev("C")])); // thread-scoped; Z never leaks
        assert_eq!(v.heads_for(&rev("B")), set([rev("B"), rev("C")])); // a head seed -> its component's heads
        assert_eq!(v.heads_for(&rev("Z")), set([rev("Z")])); // single-head thread resolves cleanly
        assert_eq!(v.components.len(), 2);
    }

    #[test]
    fn component_of_is_none_for_an_unknown_revision() {
        let v = SupersessionView::from_edges([(rev("A"), vec![])]);
        assert!(v.component_of(&rev("missing")).is_none());
        assert!(v.heads_for(&rev("missing")).is_empty());
    }

    #[test]
    fn stale_by_superseding_revision_names_all_successors() {
        // A superseded by both B and C: a fact on A is stale, naming B and C; a
        // fact on a head names nobody.
        let v = SupersessionView::from_edges([
            (rev("A"), vec![]),
            (rev("B"), vec![rev("A")]),
            (rev("C"), vec![rev("A")]),
        ]);
        assert_eq!(
            v.stale_by_superseding_revision(&rev("A")),
            set([rev("B"), rev("C")])
        );
        assert!(v.stale_by_superseding_revision(&rev("B")).is_empty());
    }

    mod from_events {
        use super::super::*;
        use super::rev;
        use crate::model::{
            EngagementId, JournalId, ObjectId, ReviewEndpoint, RevisionSource, TargetRef,
            TaskTargetRef, WorkObjectId, WorktreeCaptureMode,
        };
        use crate::session::event::{EventTarget, GitProvenance, Revision, Writer};

        fn revision_event(suffix: &str, supersedes: Vec<RevisionId>) -> ShoreEvent {
            let revision_id = rev(suffix);
            ShoreEvent::new(
                EventType::WorkObjectProposed,
                format!("work_object_proposed:{}", revision_id.as_str()),
                EventTarget::for_revision(
                    JournalId::new("journal:default"),
                    revision_id.clone(),
                    None,
                )
                .unwrap(),
                Writer::shore_local("test"),
                WorkObjectProposedPayload {
                    engagement_id: EngagementId::new(format!("engagement:sha256:{suffix}")),
                    work_object: WorkObjectProposal::Revision {
                        revision: Revision {
                            id: revision_id,
                            object_id: ObjectId::new(format!("obj:sha256:{suffix}")),
                            git_provenance: Some(GitProvenance {
                                source: RevisionSource::GitWorktree {
                                    mode: WorktreeCaptureMode::CombinedHeadToWorkingTree,
                                    include_untracked: true,
                                    pathspecs: Vec::new(),
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
                        object_artifact_content_hash: format!("sha256:artifact:{suffix}"),
                        supersedes,
                    },
                },
                "2026-06-04T00:00:00Z",
            )
            .unwrap()
        }

        fn task_attempt_event(suffix: &str) -> ShoreEvent {
            ShoreEvent::new(
                EventType::WorkObjectProposed,
                format!("task-capture:{suffix}"),
                EventTarget::for_subject(
                    JournalId::new("journal:default"),
                    TargetRef::Task(TaskTargetRef::TaskAttempt {
                        task_attempt_id: WorkObjectId::new(format!("task-attempt:sha256:{suffix}")),
                    }),
                    None,
                )
                .unwrap(),
                Writer::shore_local("test"),
                WorkObjectProposedPayload {
                    engagement_id: EngagementId::new(format!("engagement:sha256:{suffix}")),
                    work_object: WorkObjectProposal::TaskAttempt {
                        task_attempt_id: WorkObjectId::new(format!("task-attempt:sha256:{suffix}")),
                        project_path: "/repo".to_owned(),
                        claude_session_uuid: format!("uuid-{suffix}"),
                        initial_prompt_hash: format!("sha256:prompt:{suffix}"),
                        predecessor: None,
                        base_state_fingerprint: None,
                        source_speaker: None,
                    },
                },
                "2026-06-04T00:00:01Z",
            )
            .unwrap()
        }

        #[test]
        fn reads_supersedes_into_competing_heads_skipping_the_task_arm() {
            // A, then B and C both supersede A; plus a task-attempt proposal sharing
            // the store. The revision arm yields the edges; the task arm is skipped,
            // never decoded as a revision, never an error.
            let events = vec![
                revision_event("a", vec![]),
                revision_event("b", vec![rev("a")]),
                revision_event("c", vec![rev("a")]),
                task_attempt_event("t"),
            ];
            let view = SupersessionView::from_events(&events).unwrap();

            assert_eq!(view.heads, [rev("b"), rev("c")].into_iter().collect());
            assert_eq!(view.superseded, [rev("a")].into_iter().collect());
            assert!(view.diagnostics.is_empty());
        }
    }
}
