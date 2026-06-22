use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::Result;
use crate::git::git_worktree_root;
use crate::session::store::inventory::{
    ArtifactInventoryEntry, RevisionSnapshotInventory, StoreInventory, scan_store_inventory,
};
use crate::session::store::resolution::resolve_store;
use crate::session::store::sensitivity::{
    SensitivityFinding, SensitivityScan, scan_worktree_sensitivity,
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
    pub revision_snapshots: Vec<StoreStatusRevisionSnapshot>,
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
pub struct StoreStatusRevisionSnapshot {
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
    Ok(StoreStatusResult {
        mode: view.mode.to_owned(),
        store_ref: view.store_ref,
        clone_ref: view.clone_ref,
        repository_family_ref: view.repository_family_ref,
        inventory: StoreStatusInventory::from(inventory),
        sensitivity: StoreStatusSensitivity::from(sensitivity),
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
            revision_snapshots: inventory
                .revision_snapshots
                .into_iter()
                .map(StoreStatusRevisionSnapshot::from)
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

impl From<RevisionSnapshotInventory> for StoreStatusRevisionSnapshot {
    fn from(snapshot: RevisionSnapshotInventory) -> Self {
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
