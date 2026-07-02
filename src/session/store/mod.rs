pub(in crate::session) mod backend;
pub(in crate::session) mod body_artifact;
pub(in crate::session) mod bundle;
pub(in crate::session) mod content;
mod event_migrate;
mod event_store;
pub(in crate::session) mod fingerprint;
pub(in crate::session) mod inventory;
pub(in crate::session) mod object_artifact;
pub(in crate::session) mod resolution;
pub(in crate::session) mod sensitivity;
pub(in crate::session) mod sensitivity_config;
pub(in crate::session) mod store_config;
pub(in crate::session) mod store_init;

pub(crate) use event_migrate::{EventMigrateOutcome, migrate_event_file};
pub use event_store::EventWriteOutcome;
pub(crate) use event_store::{EventStore, SkippedEvent};
#[cfg(test)]
pub use fingerprint::compute_revision_fingerprint;
pub(crate) use fingerprint::worktree_fingerprint_for_files;
pub use fingerprint::{RevisionFingerprint, capture_worktree_fingerprint};
pub use object_artifact::{ObjectArtifact, read_bound_object_artifact, read_object_artifact};
pub use resolution::event_log_head_marker;
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
pub use store_init::{ensure_shore_gitignore, store_dir_for_repo};
