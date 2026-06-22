pub(in crate::session) mod body_artifact;
pub(in crate::session) mod bundle;
mod event_migrate;
mod event_store;
pub(in crate::session) mod fingerprint;
pub(in crate::session) mod inventory;
pub(in crate::session) mod resolution;
pub(in crate::session) mod sensitivity;
pub(in crate::session) mod snapshot_artifact;
pub(in crate::session) mod store_config;
pub(in crate::session) mod store_init;

pub(crate) use event_migrate::{EventMigrateOutcome, migrate_event_file};
pub(crate) use event_store::{EventStore, EventWriteOutcome};
#[cfg(test)]
pub use fingerprint::compute_review_unit_fingerprint;
pub(crate) use fingerprint::worktree_fingerprint_for_files;
pub use fingerprint::{ReviewUnitFingerprint, capture_worktree_fingerprint};
pub use snapshot_artifact::{SnapshotArtifact, read_snapshot_artifact};
// `StoreMode` and the thin repo-level entry points re-export from `session::mod`
// for the binary crate. The underlying read/write helpers stay crate-internal:
// the resolver reaches them by submodule path and the CLI only ever names the
// `..._for_repo` wrappers.
pub use store_config::{
    StoreMode, StoreModeOutcome, StoreModeSource, resolve_store_mode_for_repo,
    set_store_mode_for_repo,
};
pub(crate) use store_init::{
    FLAT_STORE_MARKERS, ShoreStorePaths, StoreLayout, detect_store_layout, sweep_stale_temp_files,
};
pub use store_init::{
    ensure_local_actor_attributes_excluded, ensure_local_delegates_excluded,
    ensure_shore_storage_excluded, store_dir_for_repo,
};
