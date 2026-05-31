pub(in crate::session) mod body_artifact;
pub(in crate::session) mod bundle;
mod event_store;
pub(in crate::session) mod fingerprint;
pub(in crate::session) mod inventory;
pub(in crate::session) mod manifest;
pub(in crate::session) mod resolution;
pub(in crate::session) mod sensitivity;
pub(in crate::session) mod snapshot_artifact;
pub(in crate::session) mod store_init;

pub(crate) use event_store::{EventStore, EventWriteOutcome};
#[cfg(test)]
pub use fingerprint::compute_review_unit_fingerprint;
pub(crate) use fingerprint::worktree_fingerprint_for_files;
pub use fingerprint::{ReviewUnitFingerprint, capture_worktree_fingerprint};
pub use snapshot_artifact::{SnapshotArtifact, read_snapshot_artifact};
pub(crate) use store_init::{ShoreStorePaths, prepare_shore_writer, sweep_stale_temp_files};
pub use store_init::{ensure_shore_storage_excluded, shore_dir_for_repo};
