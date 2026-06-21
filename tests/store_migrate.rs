mod support;

use shoreline::model::LedgerId;
use shoreline::session::event::{
    EventTarget, EventType, ReviewInitializedPayload, ShoreEvent, Writer,
};
use shoreline::session::{
    MigrateStoreOptions, StoreMode, migrate_store, read_events, set_store_mode_for_repo,
};
use support::git_repo::GitRepo;

/// Build a unique event via the public builder and return its content-addressed
/// flat-store path plus a legacy-writer-shaped JSON body (producer downgraded to
/// `tool`, with a `role` injected) — the on-disk shape `migrate_store` upgrades.
fn legacy_event_file(i: usize) -> (String, String) {
    let event = ShoreEvent::new(
        EventType::ReviewInitialized,
        format!("review_initialized:ledger:{i}"),
        EventTarget::for_ledger(LedgerId::new(format!("ledger:{i}"))),
        Writer::shore_local("0.1.0"),
        ReviewInitializedPayload {},
        "2026-05-10T00:00:00Z",
    )
    .expect("event builds");
    // The filename stem is the eventId hash (sha256 of the idempotencyKey), which
    // excludes the writer — so it is stable across the writer migration.
    let stem = event
        .event_id
        .as_str()
        .strip_prefix("evt:sha256:")
        .expect("event id is sha256-prefixed");
    let filename = format!(".shore/events/{stem}.json");
    let mut value = serde_json::to_value(&event).unwrap();
    let writer = value["writer"].as_object_mut().unwrap();
    let producer = writer.remove("producer").unwrap();
    writer.insert("tool".into(), producer);
    writer.insert("role".into(), serde_json::json!("author"));
    (filename, serde_json::to_string(&value).unwrap())
}

#[test]
fn migrate_store_via_public_api_nests_and_upgrades_a_flat_store() {
    let repo = GitRepo::new();
    // Seed a flat legacy store via the public event builder.
    for i in 0..2 {
        let (filename, json) = legacy_event_file(i);
        repo.write(filename, json);
    }
    repo.write(".shore/state.json", "{}");

    // A flat store is a loud error before migration (the resolve-time guard).
    assert!(
        read_events(repo.path()).is_err(),
        "a legacy flat store must error before migration, not read silently"
    );

    let result = migrate_store(MigrateStoreOptions::new(repo.path())).expect("migration succeeds");
    assert!(result.relocated);
    assert_eq!(result.events_rewritten, 2);

    // This relocation nests the flat store under the worktree-local `.shore/data`;
    // folding it into the shared common-dir store is the separate `store migrate`
    // step. A non-ephemeral worktree resolves the common-dir store by default, so
    // pin the worktree ephemeral to read the relocated nested store through the
    // public API and confirm every event reads cleanly.
    set_store_mode_for_repo(repo.path(), StoreMode::Ephemeral).expect("pin worktree-local read");
    let events = read_events(repo.path()).expect("events read after migration");
    assert_eq!(events.len(), 2);
    assert!(!repo.path().join(".shore/events").exists());
    assert!(repo.path().join(".shore/data/state.json").is_file());
}

#[test]
fn migrate_store_via_public_api_is_a_clean_noop_on_a_repo_without_a_store() {
    let repo = GitRepo::new();
    let result =
        migrate_store(MigrateStoreOptions::new(repo.path())).expect("no-store migrate is a no-op");
    assert!(!result.relocated);
    assert_eq!(result.events_rewritten, 0);
}
