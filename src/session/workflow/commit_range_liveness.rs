//! Read-time reachability enrichment for the commit-range lifecycle.
//!
//! This is the **only** place git touches the lifecycle: the pure projection
//! never imports it. Given a unit's current commit associations and a repo, it
//! joins each OID against the live-ref set to derive merged/live/orphaned, plus
//! a headline that is withheld whenever the per-OID conditions disagree or the
//! projection already flagged a diagnostic.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;

use crate::error::Result;
use crate::git::{
    Ancestry, git_for_each_ref, git_is_ancestor, git_object_exists, git_rev_parse_commit_oid,
    git_worktree_list,
};
use crate::session::ReviewUnitCommitRangeView;
use crate::session::state::ProjectionDiagnostic;

const SHORT_OID_LEN: usize = 12;

/// Why a commit is orphaned: its object is gone, or it exists but no live ref
/// reaches it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrphanReason {
    ObjectMissing,
    Unreachable,
}

/// A commit's relationship to the live commit graph. Internally tagged on
/// `condition` so `Orphaned` can carry its reason in the same object; distinct
/// from `ResolutionStatus::Orphaned` (both serialize an `"orphaned"` token but
/// never share an object).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "condition")]
pub enum CommitGraphCondition {
    Merged,
    Live,
    Orphaned { reason: OrphanReason },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitLiveness {
    pub commit_oid: String,
    #[serde(flatten)]
    pub condition: CommitGraphCondition,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_branch: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LivenessEnrichment {
    pub per_commit: Vec<CommitLiveness>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headline: Option<CommitGraphCondition>,
}

/// Joins `view`'s current commit associations against the repo's live-ref set.
///
/// Broad default (`integration_ref == None`): a commit is merged when it is an
/// ancestor of some live tip other than itself, live when it is itself a tip,
/// else orphaned. Narrow (`Some(r)`): merged only when an ancestor of `r`. A
/// floating unit (no current commits) short-circuits with an empty result. Any
/// git error other than a missing object propagates, so the caller can render
/// "reachability unknown".
pub fn enrich_liveness(
    view: &ReviewUnitCommitRangeView,
    repo: &Path,
    integration_ref: Option<&str>,
) -> Result<LivenessEnrichment> {
    if view.current_commits.is_empty() {
        return Ok(LivenessEnrichment {
            per_commit: Vec::new(),
            headline: None,
        });
    }

    let tips = live_tips(repo)?;
    let integration = match integration_ref {
        Some(reference) => Some(IntegrationRef {
            oid: git_rev_parse_commit_oid(repo, reference)?,
            label: short_ref_label(reference),
        }),
        None => None,
    };

    let mut cache = HashMap::new();
    let mut seen = HashSet::new();
    let mut per_commit = Vec::new();
    for commit in &view.current_commits {
        if !seen.insert(commit.commit_oid.clone()) {
            continue;
        }
        let (condition, live_branch) = classify(
            repo,
            &commit.commit_oid,
            &tips,
            integration.as_ref(),
            &mut cache,
        )?;
        per_commit.push(CommitLiveness {
            commit_oid: commit.commit_oid.clone(),
            condition,
            live_branch,
        });
    }

    let headline = headline_for(&per_commit, &view.diagnostics);
    Ok(LivenessEnrichment {
        per_commit,
        headline,
    })
}

struct LiveTip {
    oid: String,
    label: Option<String>,
}

struct IntegrationRef {
    oid: String,
    label: String,
}

/// The live-tip set: branch and remote-tracking tips (matched by prefix so
/// nested names like `feat/x` are included), plus linked-worktree HEADs — the
/// only way a detached-worktree commit counts as live. Deduped by OID.
fn live_tips(repo: &Path) -> Result<Vec<LiveTip>> {
    let mut tips = Vec::new();
    let mut seen = HashSet::new();

    for entry in git_for_each_ref(repo, &["refs/heads/", "refs/remotes/"])? {
        if seen.insert(entry.oid.clone()) {
            tips.push(LiveTip {
                label: Some(short_ref_label(&entry.name)),
                oid: entry.oid,
            });
        }
    }

    for worktree in git_worktree_list(repo)? {
        let Some(head) = worktree.head else {
            continue;
        };
        let label = match &worktree.branch {
            Some(branch) => Some(short_ref_label(branch)),
            None => Some(detached_label(&head)),
        };
        if seen.insert(head.clone()) {
            tips.push(LiveTip { oid: head, label });
        }
    }

    Ok(tips)
}

fn classify(
    repo: &Path,
    commit_oid: &str,
    tips: &[LiveTip],
    integration: Option<&IntegrationRef>,
    cache: &mut HashMap<(String, String), Ancestry>,
) -> Result<(CommitGraphCondition, Option<String>)> {
    if !git_object_exists(repo, commit_oid)? {
        return Ok((
            CommitGraphCondition::Orphaned {
                reason: OrphanReason::ObjectMissing,
            },
            None,
        ));
    }

    if let Some(integration) = integration {
        if integration.oid != commit_oid
            && ancestry(repo, commit_oid, &integration.oid, cache)? == Ancestry::Ancestor
        {
            return Ok((
                CommitGraphCondition::Merged,
                Some(integration.label.clone()),
            ));
        }
        if integration.oid == commit_oid {
            return Ok((CommitGraphCondition::Live, Some(integration.label.clone())));
        }
    } else {
        for tip in tips {
            if tip.oid == commit_oid {
                continue;
            }
            if ancestry(repo, commit_oid, &tip.oid, cache)? == Ancestry::Ancestor {
                return Ok((CommitGraphCondition::Merged, tip.label.clone()));
            }
        }
    }

    if let Some(tip) = tips.iter().find(|tip| tip.oid == commit_oid) {
        return Ok((CommitGraphCondition::Live, tip.label.clone()));
    }

    Ok((
        CommitGraphCondition::Orphaned {
            reason: OrphanReason::Unreachable,
        },
        None,
    ))
}

fn ancestry(
    repo: &Path,
    commit_oid: &str,
    tip_oid: &str,
    cache: &mut HashMap<(String, String), Ancestry>,
) -> Result<Ancestry> {
    let key = (commit_oid.to_owned(), tip_oid.to_owned());
    if let Some(ancestry) = cache.get(&key) {
        return Ok(*ancestry);
    }
    let ancestry = git_is_ancestor(repo, commit_oid, tip_oid)?;
    cache.insert(key, ancestry);
    Ok(ancestry)
}

/// A headline only when every per-OID condition agrees and the projection
/// flagged no diagnostics; otherwise withheld.
fn headline_for(
    per_commit: &[CommitLiveness],
    diagnostics: &[ProjectionDiagnostic],
) -> Option<CommitGraphCondition> {
    if !diagnostics.is_empty() {
        return None;
    }
    let mut conditions = per_commit.iter().map(|commit| &commit.condition);
    let first = conditions.next()?.clone();
    if conditions.all(|condition| *condition == first) {
        Some(first)
    } else {
        None
    }
}

/// The short, display label for a full ref: `refs/heads/feat/x` → `feat/x`,
/// `refs/remotes/origin/main` → `origin/main`. Already-short names pass through.
fn short_ref_label(reference: &str) -> String {
    reference
        .strip_prefix("refs/heads/")
        .or_else(|| reference.strip_prefix("refs/remotes/"))
        .unwrap_or(reference)
        .to_owned()
}

/// Honest label for a detached worktree HEAD: never fabricates a branch name.
fn detached_label(oid: &str) -> String {
    format!("(detached @ {})", &oid[..oid.len().min(SHORT_OID_LEN)])
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::process::Command;

    use tempfile::TempDir;

    use super::*;
    use crate::model::RevisionId;
    use crate::session::{CommitEdgeSource, CurrentCommitAssociation};

    struct LivenessRepo {
        root: TempDir,
    }

    impl LivenessRepo {
        /// main: base → mid → tip; plus a dangling commit (child of tip) whose
        /// branch was deleted, so it exists but no live ref reaches it.
        fn new() -> Self {
            let repo = Self {
                root: TempDir::new().unwrap(),
            };
            repo.git(["init"]);
            repo.git(["config", "user.name", "Shore Tests"]);
            repo.git(["config", "user.email", "shore-tests@example.com"]);
            repo.git(["config", "commit.gpgsign", "false"]);

            repo.commit("base", "base\n");
            repo.git(["branch", "-M", "main"]);
            repo.commit("mid", "mid\n");
            repo.commit("tip", "tip\n");

            // A child of tip on a throwaway branch, then delete the branch.
            repo.git(["checkout", "-b", "tmp"]);
            repo.commit("dangling", "dangling\n");
            repo.git(["checkout", "main"]);
            repo.git(["branch", "-D", "tmp"]);

            repo
        }

        fn path(&self) -> &Path {
            self.root.path()
        }

        fn commit(&self, message: &str, contents: &str) {
            std::fs::write(self.path().join("file.txt"), contents).unwrap();
            self.git(["add", "--all"]);
            self.git(["commit", "-m", message]);
        }

        fn oid(&self, rev: &str) -> String {
            let output = Command::new("git")
                .args(["rev-parse", "--verify", rev])
                .current_dir(self.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git rev-parse {rev} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        }

        fn git<I, S>(&self, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<OsStr>,
        {
            self.git_at(self.path(), args);
        }

        fn git_at<I, S>(&self, cwd: &Path, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<OsStr>,
        {
            let status = Command::new("git")
                .args(
                    args.into_iter()
                        .map(|a| a.as_ref().to_owned())
                        .collect::<Vec<_>>(),
                )
                .current_dir(cwd)
                .status()
                .unwrap();
            assert!(status.success());
        }
    }

    fn view_with(oids: &[&str]) -> ReviewUnitCommitRangeView {
        ReviewUnitCommitRangeView {
            review_unit_id: RevisionId::new("review-unit:sha256:test"),
            anchored: !oids.is_empty(),
            current_commits: oids
                .iter()
                .map(|oid| CurrentCommitAssociation {
                    commit_oid: (*oid).to_owned(),
                    tree_oid: format!("{oid}-tree"),
                    commit_association_id: None,
                    source: CommitEdgeSource::CaptureTarget,
                })
                .collect(),
            current_refs: Vec::new(),
            withdrawn_commits: Vec::new(),
            withdrawn_refs: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn condition_of(
        repo: &LivenessRepo,
        oid: &str,
        integration: Option<&str>,
    ) -> CommitGraphCondition {
        enrich_liveness(&view_with(&[oid]), repo.path(), integration)
            .unwrap()
            .per_commit
            .remove(0)
            .condition
    }

    #[test]
    fn merged_live_orphaned_broad_default() {
        let repo = LivenessRepo::new();
        let mid = repo.oid("main~1");
        let tip = repo.oid("main");

        // mid is an ancestor of the tip (another live tip) → Merged.
        assert_eq!(
            condition_of(&repo, &mid, None),
            CommitGraphCondition::Merged
        );
        // The tip is itself a live tip, contained in no other → Live.
        assert_eq!(condition_of(&repo, &tip, None), CommitGraphCondition::Live);

        // A well-formed but absent object → Orphaned(ObjectMissing).
        let missing = "0".repeat(tip.len());
        assert_eq!(
            condition_of(&repo, &missing, None),
            CommitGraphCondition::Orphaned {
                reason: OrphanReason::ObjectMissing
            }
        );
    }

    #[test]
    fn unreachable_existing_commit_is_orphaned_unreachable() {
        let repo = LivenessRepo::new();
        let dangling = repo.dangling_oid();

        assert!(git_object_exists(repo.path(), &dangling).unwrap());
        assert_eq!(
            condition_of(&repo, &dangling, None),
            CommitGraphCondition::Orphaned {
                reason: OrphanReason::Unreachable
            }
        );
    }

    #[test]
    fn floating_unit_skips_reachability() {
        let repo = LivenessRepo::new();
        let enrichment = enrich_liveness(&view_with(&[]), repo.path(), None).unwrap();
        assert!(enrichment.per_commit.is_empty());
        assert!(enrichment.headline.is_none());
    }

    #[test]
    fn headline_withheld_when_conditions_disagree() {
        let repo = LivenessRepo::new();
        let mid = repo.oid("main~1");
        let dangling = repo.dangling_oid();

        let enrichment =
            enrich_liveness(&view_with(&[&mid, &dangling]), repo.path(), None).unwrap();

        assert_eq!(enrichment.per_commit.len(), 2);
        assert!(enrichment.headline.is_none());
    }

    #[test]
    fn headline_present_when_single_condition_and_no_diagnostics() {
        let repo = LivenessRepo::new();
        let tip = repo.oid("main");

        let enrichment = enrich_liveness(&view_with(&[&tip]), repo.path(), None).unwrap();
        assert_eq!(enrichment.headline, Some(CommitGraphCondition::Live));
    }

    #[test]
    fn detached_worktree_head_is_live_with_detached_label() {
        let repo = LivenessRepo::new();
        let parent = TempDir::new().unwrap();
        let linked = parent.path().join("wt");

        // A detached linked worktree, then a commit on it: its HEAD advances to a
        // commit no branch points at — live only via the worktree HEAD.
        repo.git_at(
            repo.path(),
            [
                "worktree",
                "add",
                "--detach",
                linked.to_str().unwrap(),
                "main",
            ],
        );
        std::fs::write(linked.join("file.txt"), "detached\n").unwrap();
        repo.git_at(&linked, ["add", "--all"]);
        repo.git_at(&linked, ["commit", "-m", "detached work"]);
        let detached_oid = {
            let output = Command::new("git")
                .args(["rev-parse", "--verify", "HEAD"])
                .current_dir(&linked)
                .output()
                .unwrap();
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        };

        let enrichment = enrich_liveness(&view_with(&[&detached_oid]), repo.path(), None).unwrap();
        let commit = &enrichment.per_commit[0];

        assert_eq!(commit.condition, CommitGraphCondition::Live);
        assert!(
            commit
                .live_branch
                .as_deref()
                .is_some_and(|label| label.starts_with("(detached @ ")),
            "detached worktree HEAD must carry the honest detached label, got {:?}",
            commit.live_branch
        );
    }

    #[test]
    fn narrow_merged_when_integration_ref_set() {
        let repo = LivenessRepo::new();
        let mid = repo.oid("main~1");
        let dangling = repo.dangling_oid();

        assert_eq!(
            condition_of(&repo, &mid, Some("refs/heads/main")),
            CommitGraphCondition::Merged
        );
        assert_eq!(
            condition_of(&repo, &dangling, Some("refs/heads/main")),
            CommitGraphCondition::Orphaned {
                reason: OrphanReason::Unreachable
            }
        );
    }

    impl LivenessRepo {
        /// The OID of the dangling commit (child of tip, branch deleted). Found
        /// by scanning the reflog for the unreachable child of `main`.
        fn dangling_oid(&self) -> String {
            // `git fsck` is heavyweight; instead read the reflog of HEAD where the
            // tmp commit was created, then verify it is unreachable from main.
            let output = Command::new("git")
                .args(["log", "-g", "--format=%H"])
                .current_dir(self.path())
                .output()
                .unwrap();
            let reflog = String::from_utf8(output.stdout).unwrap();
            let tip = self.oid("main");
            reflog
                .lines()
                .map(str::to_owned)
                .find(|oid| {
                    *oid != tip
                        && git_is_ancestor(self.path(), oid, &tip).unwrap() == Ancestry::NotAncestor
                        && git_is_ancestor(self.path(), &tip, oid).unwrap() == Ancestry::Ancestor
                })
                .expect("a dangling child of tip is in the reflog")
        }
    }
}
