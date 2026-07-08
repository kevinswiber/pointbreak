use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::Result;
use crate::git::git_worktree_root;
use crate::session::store::inventory::{
    ArtifactInventoryEntry, RevisionObjectInventory, StoreInventory, scan_store_inventory,
};
use crate::session::store::resolution::resolve_store;
use crate::session::store::sensitivity::{
    SensitivityFinding, SensitivityScan, explain_worktree_sensitivity, scan_worktree_sensitivity,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoreStatusOptions {
    repo: PathBuf,
}

impl StoreStatusOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreStatusResult {
    pub mode: String,
    pub store_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clone_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_family_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_clone_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orphaned: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_write: Option<String>,
    /// A discoverability advisory when this worktree writes to the clone-local store
    /// while a sibling worktree of the clone is linked to a family store. `None` when
    /// there is nothing to advise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family_link_advisory: Option<String>,
    pub inventory: StoreStatusInventory,
    pub sensitivity: StoreStatusSensitivity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreStatusInventory {
    pub event_count: usize,
    pub event_bytes: u64,
    pub artifact_count: usize,
    pub artifact_bytes: u64,
    pub total_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub untracked_bytes: Option<u64>,
    pub largest_artifacts: Vec<StoreStatusArtifactInventory>,
    pub revision_objects: Vec<StoreStatusRevisionObject>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreStatusArtifactInventory {
    pub artifact_ref: String,
    pub artifact_kind: String,
    pub byte_size: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreStatusRevisionObject {
    pub revision_ids: Vec<String>,
    pub object_id: String,
    pub artifact_ref: String,
    pub byte_size: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreStatusSensitivity {
    pub policy_outcome: String,
    pub findings: Vec<StoreStatusSensitivityFinding>,
    /// Unique inventory paths the configured exclude globs skipped — the
    /// audit trail that keeps an over-broad exclude visible, not silent.
    pub excluded_path_count: usize,
    /// Every configured exclude glob with its match count (zero-count globs
    /// included). Glob strings are operator-authored config, safe to render.
    pub exclude_globs: Vec<StoreStatusSensitivityExcludeGlob>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreStatusSensitivityExcludeGlob {
    pub glob: String,
    pub matched: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreStatusSensitivityFinding {
    pub kind: String,
    pub severity: String,
    pub count: usize,
    pub policy_outcome: String,
    pub references: Vec<String>,
}

pub fn store_status(options: StoreStatusOptions) -> Result<StoreStatusResult> {
    let worktree_root = git_worktree_root(&options.repo)?;
    let resolution = resolve_store(&options.repo)?;
    let inventory = scan_store_inventory(resolution.store_dir(), Some(&worktree_root))?;
    let sensitivity = scan_worktree_sensitivity(&worktree_root)?;
    let view = resolution.command_view();
    let lifecycle = match view.repository_family_ref.as_deref() {
        Some(family_ref) => Some(family_lifecycle_fields(resolution.store_dir(), family_ref)?),
        None => None,
    };
    Ok(StoreStatusResult {
        mode: view.mode.to_owned(),
        store_ref: view.store_ref,
        clone_ref: view.clone_ref,
        repository_family_ref: view.repository_family_ref,
        live_clone_count: lifecycle.as_ref().map(|fields| fields.live_clone_count),
        orphaned: lifecycle.as_ref().map(|fields| fields.orphaned),
        last_write: lifecycle.and_then(|fields| fields.last_write),
        family_link_advisory: crate::session::store::resolution::family_link_advisory(
            &options.repo,
        )?,
        inventory: StoreStatusInventory::from(inventory),
        sensitivity: StoreStatusSensitivity::from(sensitivity),
    })
}

/// Per-finding real matched paths for the local-only `store status --show-paths`
/// surface. Deliberately NOT `Serialize`: real worktree paths must never reach
/// the store or any emitted JSON (the sensitivity no-path contract). Produced by
/// [`explain_store_sensitivity`] and rendered to the operator's terminal only.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoreSensitivityPathGroup {
    pub kind: String,
    pub severity: String,
    pub policy_outcome: String,
    pub paths: Vec<String>,
}

/// Resolve the worktree for `repo` and return the real matched paths per finding
/// kind for the local-only explain surface. Shares the exact matchers used by
/// [`store_status`]'s redacting scan (via `explain_worktree_sensitivity`); the
/// caller is responsible for rendering the result to stdout only and never
/// routing it through a `DiagnosticDocument` or any emitted JSON.
pub fn explain_store_sensitivity(repo: impl AsRef<Path>) -> Result<Vec<StoreSensitivityPathGroup>> {
    let worktree_root = git_worktree_root(repo.as_ref())?;
    Ok(explain_worktree_sensitivity(&worktree_root)?
        .into_iter()
        .map(|group| StoreSensitivityPathGroup {
            kind: group.kind,
            severity: group.severity,
            policy_outcome: group.policy_outcome,
            paths: group.paths,
        })
        .collect())
}

struct FamilyLifecycleFields {
    live_clone_count: usize,
    orphaned: bool,
    last_write: Option<String>,
}

/// Populated only when the resolution tier is `UserLevel` — branching on
/// `repository_family_ref.is_some()` rather than a raw tier accessor, since that
/// field is `Some` iff the tier is `UserLevel` per `command_view`'s mapping.
fn family_lifecycle_fields(family_dir: &Path, family_ref: &str) -> Result<FamilyLifecycleFields> {
    use crate::session::store::user_level::{family_last_write, family_liveness};

    let liveness = family_liveness(family_dir, family_ref)?;
    let last_write = family_last_write(family_dir)?;
    Ok(FamilyLifecycleFields {
        live_clone_count: liveness.live_clone_count,
        orphaned: liveness.orphaned,
        last_write,
    })
}

impl From<StoreInventory> for StoreStatusInventory {
    fn from(inventory: StoreInventory) -> Self {
        Self {
            event_count: inventory.event_count,
            event_bytes: inventory.event_bytes,
            artifact_count: inventory.artifact_count,
            artifact_bytes: inventory.artifact_bytes,
            total_bytes: inventory.total_bytes,
            untracked_bytes: inventory.untracked_bytes,
            largest_artifacts: inventory
                .largest_artifacts
                .into_iter()
                .map(StoreStatusArtifactInventory::from)
                .collect(),
            revision_objects: inventory
                .revision_objects
                .into_iter()
                .map(StoreStatusRevisionObject::from)
                .collect(),
        }
    }
}

impl From<ArtifactInventoryEntry> for StoreStatusArtifactInventory {
    fn from(artifact: ArtifactInventoryEntry) -> Self {
        Self {
            artifact_ref: artifact.artifact_ref,
            artifact_kind: artifact.artifact_kind,
            byte_size: artifact.byte_size,
        }
    }
}

impl From<RevisionObjectInventory> for StoreStatusRevisionObject {
    fn from(snapshot: RevisionObjectInventory) -> Self {
        Self {
            revision_ids: snapshot.revision_ids,
            object_id: snapshot.object_id,
            artifact_ref: snapshot.artifact_ref,
            byte_size: snapshot.byte_size,
        }
    }
}

impl From<SensitivityScan> for StoreStatusSensitivity {
    fn from(scan: SensitivityScan) -> Self {
        Self {
            policy_outcome: scan.policy_outcome,
            findings: scan
                .findings
                .into_iter()
                .map(StoreStatusSensitivityFinding::from)
                .collect(),
            excluded_path_count: scan.excluded_path_count,
            exclude_globs: scan
                .exclude_globs
                .into_iter()
                .map(|glob| StoreStatusSensitivityExcludeGlob {
                    glob: glob.glob,
                    matched: glob.matched,
                })
                .collect(),
        }
    }
}

impl From<SensitivityFinding> for StoreStatusSensitivityFinding {
    fn from(finding: SensitivityFinding) -> Self {
        Self {
            kind: finding.kind,
            severity: finding.severity,
            count: finding.count,
            policy_outcome: finding.policy_outcome,
            references: finding.references,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;
    use std::process::Command;

    use tempfile::TempDir;

    use super::*;
    use crate::session::{
        CaptureOptions, StoreLinkOptions, capture_worktree_review, link_store_to_family,
    };

    struct TestRepo {
        root: TempDir,
    }

    impl TestRepo {
        fn new() -> Self {
            let root = TempDir::new().expect("create temp git repository directory");
            let repo = Self { root };
            repo.git(["init"]);
            repo.git(["config", "user.name", "Shore Tests"]);
            repo.git(["config", "user.email", "shore-tests@example.com"]);
            repo.git(["config", "commit.gpgsign", "false"]);
            repo
        }
        fn path(&self) -> &Path {
            self.root.path()
        }
        fn write(&self, path: &str, contents: &str) {
            let path = self.root.path().join(path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, contents).unwrap();
        }
        fn commit_all(&self, message: &str) {
            self.git(["add", "--all"]);
            self.git(["commit", "-m", message]);
        }
        fn git<I, S>(&self, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<OsStr>,
        {
            let output = Command::new("git")
                .args(args)
                .current_dir(self.root.path())
                .output()
                .expect("run git");
            assert!(output.status.success());
        }
    }

    /// Set `SHORE_HOME` for the duration of `f`. nextest's process-per-test keeps the
    /// mutation contained (the `keys/home.rs` seam). SAFETY: single-threaded test
    /// process.
    fn with_shore_home<T>(home: &Path, f: impl FnOnce() -> T) -> T {
        unsafe {
            std::env::set_var("SHORE_HOME", home);
        }
        let out = f();
        unsafe {
            std::env::remove_var("SHORE_HOME");
        }
        out
    }

    #[test]
    fn clone_local_status_omits_all_three_lifecycle_fields() {
        let repo = TestRepo::new();
        repo.write("README.md", "base\n");
        repo.commit_all("base");

        let result = store_status(StoreStatusOptions::new(repo.path())).unwrap();

        assert!(result.live_clone_count.is_none());
        assert!(result.orphaned.is_none());
        assert!(result.last_write.is_none());
        let json = serde_json::to_value(&result).unwrap();
        assert!(
            json.get("liveCloneCount").is_none(),
            "serde skip verified: {json}"
        );
        assert!(
            json.get("orphaned").is_none(),
            "serde skip verified: {json}"
        );
        assert!(
            json.get("lastWrite").is_none(),
            "serde skip verified: {json}"
        );
    }

    #[test]
    fn linked_repo_status_carries_all_five_family_fields() {
        let repo = TestRepo::new();
        repo.write("README.md", "base\n");
        repo.commit_all("base");
        capture_worktree_review(CaptureOptions::new(repo.path()).with_allow_empty()).unwrap();

        let home = tempfile::tempdir().unwrap();
        // The scoped, single-threaded SHORE_HOME seam (nextest's process-per-test
        // contains the mutation). This is the one phase-7 test that needs it — the
        // positive user-level case cannot be reached purely before the CLI link
        // subcommand exists.
        let result = with_shore_home(home.path(), || {
            link_store_to_family(StoreLinkOptions::new(repo.path(), Some("acme".to_owned())))
                .expect("link succeeds against a clean, non-ephemeral, non-sensitive worktree");
            store_status(StoreStatusOptions::new(repo.path()))
        })
        .unwrap();

        assert_eq!(result.mode, "user-level");
        assert_eq!(result.repository_family_ref.as_deref(), Some("acme"));
        assert!(result.clone_ref.is_some());
        assert!(result.live_clone_count.is_some());
        assert!(result.orphaned.is_some());
        // last_write may be None immediately after link if nothing has written to the
        // family store yet; assert presence only via the Option type.
    }

    #[test]
    fn status_surfaces_the_family_link_advisory_for_an_unbound_sibling() {
        let repo = TestRepo::new();
        repo.write("README.md", "base\n");
        repo.commit_all("base");
        // Main carries a legacy binding; add an unbound sibling worktree of the clone.
        repo.write(
            ".shore/store.local.json",
            r#"{"schema":"shore.store-config","version":1,"mode":"shared","familyRef":"acme","cloneRef":"deadbeefdeadbeef"}"#,
        );
        let wt_parent = TempDir::new().unwrap();
        let wt = wt_parent.path().join("sib");
        repo.git(["branch", "sib"]);
        repo.git([
            OsStr::new("worktree"),
            OsStr::new("add"),
            wt.as_os_str(),
            OsStr::new("sib"),
        ]);

        let result = store_status(StoreStatusOptions::new(&wt)).unwrap();
        assert!(
            result
                .family_link_advisory
                .as_deref()
                .is_some_and(|m| m.contains("acme") && m.contains("shore store link")),
            "the unbound sibling is advised to link: {:?}",
            result.family_link_advisory
        );
    }

    #[test]
    fn status_has_no_family_link_advisory_for_a_fresh_clone() {
        let repo = TestRepo::new();
        repo.write("README.md", "base\n");
        repo.commit_all("base");

        let result = store_status(StoreStatusOptions::new(repo.path())).unwrap();
        assert!(result.family_link_advisory.is_none());
        let json = serde_json::to_value(&result).unwrap();
        assert!(
            json.get("familyLinkAdvisory").is_none(),
            "skipped when None: {json}"
        );
    }
}
