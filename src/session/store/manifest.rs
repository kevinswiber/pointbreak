use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, ShoreError};
use crate::storage::{CreateFileOutcome, Durability, LocalStorage};

const STORE_MANIFEST_SCHEMA: &str = "shore.store-manifest";
const STORE_MANIFEST_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoreManifest {
    pub schema: String,
    pub version: u32,
    pub store_id: String,
    pub clone_id: String,
    pub repository_family_id: String,
    pub git: StoreGitProvenance,
}

impl StoreManifest {
    fn new(git: StoreGitProvenance) -> Result<Self> {
        let store_id = random_identity("store")?;
        let clone_id = random_identity("clone")?;
        Ok(Self {
            schema: STORE_MANIFEST_SCHEMA.to_owned(),
            version: STORE_MANIFEST_VERSION,
            store_id,
            repository_family_id: clone_id.clone(),
            clone_id,
            git,
        })
    }

    fn validate_schema_version(&self) -> Result<()> {
        if self.schema == STORE_MANIFEST_SCHEMA && self.version == STORE_MANIFEST_VERSION {
            return Ok(());
        }

        Err(ShoreError::Message(format!(
            "unsupported store manifest schema/version: {} v{}",
            self.schema, self.version
        )))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoreGitProvenance {
    pub common_dir: String,
    pub git_dir: String,
    pub worktree_root: String,
    pub object_format: String,
}

pub(crate) fn load_or_create_store_manifest(
    store_dir: &Path,
    git: StoreGitProvenance,
) -> Result<StoreManifest> {
    let path = manifest_path(store_dir);
    let storage = LocalStorage::new(store_dir);
    let manifest = StoreManifest::new(git)?;
    let bytes = serde_json::to_vec(&manifest)?;

    match storage.create_file_exclusive(&path, &bytes, Durability::Durable)? {
        CreateFileOutcome::Created => Ok(manifest),
        CreateFileOutcome::AlreadyExists => read_store_manifest(store_dir),
    }
}

pub(crate) fn read_store_manifest(store_dir: &Path) -> Result<StoreManifest> {
    let path = manifest_path(store_dir);
    let storage = LocalStorage::new(store_dir);
    let manifest: StoreManifest = storage.read_json(&path)?;
    manifest.validate_schema_version()?;
    Ok(manifest)
}

fn manifest_path(store_dir: &Path) -> PathBuf {
    store_dir.join("manifest.json")
}

fn random_identity(kind: &str) -> Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| ShoreError::Message(format!("generate {kind} identity: {error}")))?;
    Ok(format!("{kind}:random:{}", hex_lower(&bytes)))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn load_or_create_manifest_mints_store_and_clone_identity() {
        let store_dir = tempfile::tempdir().unwrap();
        let manifest = load_or_create_store_manifest(store_dir.path(), test_provenance()).unwrap();

        assert_eq!(manifest.schema, "shore.store-manifest");
        assert_eq!(manifest.version, 1);
        assert!(manifest.store_id.starts_with("store:random:"));
        assert!(manifest.clone_id.starts_with("clone:random:"));
        assert_eq!(manifest.repository_family_id, manifest.clone_id);
    }

    #[test]
    fn load_or_create_manifest_uses_random_clone_ids() {
        let first_store = tempfile::tempdir().unwrap();
        let second_store = tempfile::tempdir().unwrap();

        let first = load_or_create_store_manifest(first_store.path(), test_provenance()).unwrap();
        let second = load_or_create_store_manifest(second_store.path(), test_provenance()).unwrap();

        assert_ne!(first.store_id, second.store_id);
        assert_ne!(first.clone_id, second.clone_id);
        assert_ne!(first.repository_family_id, second.repository_family_id);
    }

    #[test]
    fn manifest_identity_does_not_reuse_git_paths_or_remotes() {
        let store_dir = tempfile::tempdir().unwrap();
        let provenance = StoreGitProvenance {
            common_dir: "/repo/.git".to_owned(),
            git_dir: "/repo/.git/worktrees/linked".to_owned(),
            worktree_root: "/repo-linked".to_owned(),
            object_format: "sha1".to_owned(),
        };

        let manifest = load_or_create_store_manifest(store_dir.path(), provenance).unwrap();

        for identity in [
            manifest.store_id.as_str(),
            manifest.clone_id.as_str(),
            manifest.repository_family_id.as_str(),
        ] {
            assert!(!identity.contains("/repo"));
            assert!(!identity.contains("github.com"));
            assert!(!identity.contains("example.com"));
        }
    }

    #[test]
    fn reading_existing_manifest_preserves_ids() {
        let store_dir = tempfile::tempdir().unwrap();
        let first = load_or_create_store_manifest(store_dir.path(), test_provenance()).unwrap();

        let second = load_or_create_store_manifest(
            store_dir.path(),
            StoreGitProvenance {
                common_dir: "/moved/.git".to_owned(),
                git_dir: "/moved/.git/worktrees/linked".to_owned(),
                worktree_root: "/moved-linked".to_owned(),
                object_format: "sha256".to_owned(),
            },
        )
        .unwrap();

        assert_eq!(second.store_id, first.store_id);
        assert_eq!(second.clone_id, first.clone_id);
        assert_eq!(second.repository_family_id, first.repository_family_id);
        assert_eq!(
            fs::read_to_string(store_dir.path().join("manifest.json"))
                .unwrap()
                .matches(&first.clone_id)
                .count(),
            2,
            "clone ID should appear as cloneId and default repositoryFamilyId"
        );
    }

    fn test_provenance() -> StoreGitProvenance {
        StoreGitProvenance {
            common_dir: "/repo/.git".to_owned(),
            git_dir: "/repo/.git".to_owned(),
            worktree_root: "/repo".to_owned(),
            object_format: "sha1".to_owned(),
        }
    }
}
