use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::git::git_worktree_list;
use crate::session::store::bundle::{ImportBundleResult, import_store_bundle};
use crate::session::store::resolution::{
    read_store_registration, register_clone_local_store, resolve_store,
};
use crate::session::store::sensitivity::scan_worktree_sensitivity;
use crate::session::store::store_init::ShoreStorePaths;
use crate::session::workflow::store_status::StoreStatusSensitivity;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoreLinkOptions {
    repo: PathBuf,
}

impl StoreLinkOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoreLinkResult {
    pub mode: String,
    pub store_ref: String,
    pub clone_ref: String,
    pub repository_family_ref: String,
    pub events_created: usize,
    pub events_existing: usize,
    pub artifacts_created: usize,
    pub artifacts_existing: usize,
    pub sensitivity: StoreStatusSensitivity,
}

pub fn link_clone_local_store(options: StoreLinkOptions) -> Result<StoreLinkResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let local_store_dir = paths.shore_dir().to_path_buf();
    let sensitivity = scan_worktree_sensitivity(paths.worktree_root())?;
    let _worktrees = git_worktree_list(paths.worktree_root())?;

    register_clone_local_store(paths.worktree_root())?;
    let registration = read_store_registration(paths.worktree_root())?;
    let resolution = resolve_store(paths.worktree_root())?;
    let imported = import_store_bundle(&local_store_dir, resolution.store_dir())?;

    Ok(StoreLinkResult::from_import(
        registration,
        imported,
        StoreStatusSensitivity::from(sensitivity),
    ))
}

impl StoreLinkResult {
    fn from_import(
        registration: crate::session::store::resolution::StoreRegistration,
        imported: ImportBundleResult,
        sensitivity: StoreStatusSensitivity,
    ) -> Self {
        Self {
            mode: "linked".to_owned(),
            store_ref: registration.store_ref,
            clone_ref: registration.clone_ref,
            repository_family_ref: registration.repository_family_ref,
            events_created: imported.events_created,
            events_existing: imported.events_existing,
            artifacts_created: imported.artifacts_created,
            artifacts_existing: imported.artifacts_existing,
            sensitivity,
        }
    }
}
