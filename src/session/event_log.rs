use std::path::Path;

use crate::error::Result;
use crate::session::{SessionState, ShoreEvent, ShoreStorePaths, sweep_stale_temp_files};
use crate::storage::{Durability, EventStore, LocalStorage};

pub fn rebuild_state(repo: impl AsRef<Path>) -> Result<SessionState> {
    let paths = ShoreStorePaths::resolve(repo.as_ref())?;
    let worktree_root = paths.worktree_root();
    let shore_dir = paths.shore_dir();
    let storage = LocalStorage::new(shore_dir);
    sweep_stale_temp_files(&storage, shore_dir)?;

    let span = tracing::info_span!("session.rebuild_state", repo = %worktree_root.display());
    let _entered = span.enter();

    let event_store = EventStore::open(shore_dir);
    let state = SessionState::from_events(&event_store.list_events()?)?;
    storage.write_json_atomic(&paths.state_path(), &state, Durability::Projection)?;
    Ok(state)
}

pub fn read_events(repo: impl AsRef<Path>) -> Result<Vec<ShoreEvent>> {
    let paths = ShoreStorePaths::resolve(repo.as_ref())?;
    EventStore::open(paths.shore_dir()).list_events()
}
