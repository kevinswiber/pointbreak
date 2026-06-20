mod support;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::shore;

fn parse_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("valid json on stdout")
}

/// Commit a base version of a file, then leave a modified working-tree copy so a
/// capture has a diff to record.
fn repo_with_pending_change() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo
}

#[test]
fn store_migrate_folds_worktree_local_into_common_dir() {
    let repo = repo_with_pending_change();

    // Seed a pre-flip worktree-local store: capture while the worktree is
    // ephemeral (so the write lands in `.shore/data`), then restore the shared
    // default so the migration runs against a non-ephemeral worktree carrying a
    // legacy worktree-local store.
    let ephemeral = shore([
        "store",
        "mode",
        "ephemeral",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);
    assert!(
        ephemeral.status.success(),
        "ephemeral mode: {}",
        String::from_utf8_lossy(&ephemeral.stderr)
    );
    let capture = shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    assert!(
        capture.status.success(),
        "capture: {}",
        String::from_utf8_lossy(&capture.stderr)
    );
    let shared = shore([
        "store",
        "mode",
        "shared",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);
    assert!(
        shared.status.success(),
        "shared mode: {}",
        String::from_utf8_lossy(&shared.stderr)
    );

    // The seed landed worktree-local; the shared common-dir store is still empty.
    assert!(repo.path().join(".shore/data/events").is_dir());

    let migrate = shore(["store", "migrate", "--repo", repo.path().to_str().unwrap()]);
    assert!(
        migrate.status.success(),
        "migrate: {}",
        String::from_utf8_lossy(&migrate.stderr)
    );

    let stdout = String::from_utf8(migrate.stdout).unwrap();
    let json = parse_json(stdout.as_bytes());
    assert_eq!(json["schema"], "shore.store-migrate");
    // DiagnosticDocument flattens the body to the TOP LEVEL (the `store status`
    // tests assert this shape) — assert top-level fields, NOT json["body"][…].
    assert!(json["eventsCreated"].as_u64().unwrap() >= 1);
    // No raw private paths leak into the JSON (the export-manifest discipline).
    assert!(!stdout.contains(".shore/data"));
    assert!(!stdout.contains(".git"));
    // Non-destructive: the worktree-local store still exists after migration.
    assert!(repo.path().join(".shore/data/events").is_dir());
    // The folded events now resolve from the shared common-dir store.
    assert!(
        support::common_dir_store(repo.path())
            .join("events")
            .is_dir()
    );
}

#[test]
fn store_migrate_refuses_ephemeral_without_include_ephemeral() {
    let repo = repo_with_pending_change();
    let capture = shore(["review", "capture", "--repo", repo.path().to_str().unwrap()]);
    assert!(
        capture.status.success(),
        "capture: {}",
        String::from_utf8_lossy(&capture.stderr)
    );
    // Mark the worktree ephemeral (the `store mode` CLI is the user path).
    let mode = shore([
        "store",
        "mode",
        "ephemeral",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);
    assert!(
        mode.status.success(),
        "mode: {}",
        String::from_utf8_lossy(&mode.stderr)
    );

    let migrate = shore(["store", "migrate", "--repo", repo.path().to_str().unwrap()]);
    assert!(
        !migrate.status.success(),
        "ephemeral migrate must fail without the override"
    );
    let stderr = String::from_utf8(migrate.stderr).unwrap();
    assert!(
        stderr.contains("ephemeral"),
        "refusal names the opt-out: {stderr}"
    );

    // The override succeeds.
    let forced = shore([
        "store",
        "migrate",
        "--include-ephemeral",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);
    assert!(
        forced.status.success(),
        "override migrate: {}",
        String::from_utf8_lossy(&forced.stderr)
    );
}
