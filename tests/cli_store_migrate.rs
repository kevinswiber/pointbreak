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
    let capture = shore(["capture", "--repo", repo.path().to_str().unwrap()]);
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
    // The default run reports the source honestly un-retired (additive field).
    assert_eq!(json["sourceRetired"], Value::Bool(false));
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
fn store_migrate_retire_source_completes_in_one_command() {
    let repo = repo_with_pending_change();
    let repo_arg = repo.path().to_str().unwrap().to_owned();

    // Seed a pre-flip worktree-local store, same shape as the fold test.
    assert!(
        shore(["store", "mode", "ephemeral", "--repo", &repo_arg])
            .status
            .success()
    );
    assert!(shore(["capture", "--repo", &repo_arg]).status.success());
    assert!(
        shore(["store", "mode", "shared", "--repo", &repo_arg])
            .status
            .success()
    );
    assert!(repo.path().join(".shore/data/events").is_dir());

    let migrate = shore(["store", "migrate", "--retire-source", "--repo", &repo_arg]);
    assert!(
        migrate.status.success(),
        "migrate --retire-source: {}",
        String::from_utf8_lossy(&migrate.stderr)
    );

    let stdout = String::from_utf8(migrate.stdout).unwrap();
    let json = parse_json(stdout.as_bytes());
    assert_eq!(json["sourceRetired"], Value::Bool(true));
    assert!(json["verifiedEvents"].as_u64().unwrap() >= 1);
    assert!(json["verifiedArtifacts"].as_u64().unwrap() >= 1);
    // No raw private paths leak into the JSON.
    assert!(!stdout.contains(".shore/data"));
    assert!(!stdout.contains(".git"));
    // The verified fold retired the source in the same command.
    assert!(!repo.path().join(".shore/data").exists());
    assert!(
        support::common_dir_store(repo.path())
            .join("events")
            .is_dir()
    );
}

#[test]
fn store_migrate_excluded_fixture_paths_unblock_the_gate() {
    let repo = repo_with_pending_change();
    let repo_arg = repo.path().to_str().unwrap().to_owned();
    // A committed private-key fixture would flag the worktree `block`; the
    // committed exclude config is the targeted alternative to the blanket
    // --include-ephemeral override.
    repo.write(
        "fixtures/dev.pem",
        "-----BEGIN PRIVATE KEY-----\nredacted\n",
    );
    repo.write(
        ".shore/sensitivity.json",
        r#"{"schema":"shore.sensitivity-config","version":1,"excludeGlobs":["fixtures/**"]}"#,
    );
    repo.commit_all("fixture and exclude config");

    // Seed a worktree-local store to migrate.
    assert!(
        shore(["store", "mode", "ephemeral", "--repo", &repo_arg])
            .status
            .success()
    );
    assert!(shore(["capture", "--repo", &repo_arg]).status.success());
    assert!(
        shore(["store", "mode", "shared", "--repo", &repo_arg])
            .status
            .success()
    );

    // Without the exclude config this would refuse; with it, the gate passes
    // and the migrate output carries the audit count.
    let migrate = shore(["store", "migrate", "--repo", &repo_arg]);
    assert!(
        migrate.status.success(),
        "excluded fixture must unblock the gate: {}",
        String::from_utf8_lossy(&migrate.stderr)
    );
    let json = parse_json(&migrate.stdout);
    assert_eq!(json["sensitivityExcludedPathCount"], 1);
}

#[test]
fn store_migrate_block_refusal_names_the_targeted_exclude_alternative() {
    let repo = repo_with_pending_change();
    let repo_arg = repo.path().to_str().unwrap().to_owned();
    repo.write("keys/dev.pem", "-----BEGIN PRIVATE KEY-----\nredacted\n");
    repo.commit_all("sensitive fixture");
    assert!(
        shore(["store", "mode", "ephemeral", "--repo", &repo_arg])
            .status
            .success()
    );
    assert!(shore(["capture", "--repo", &repo_arg]).status.success());
    assert!(
        shore(["store", "mode", "shared", "--repo", &repo_arg])
            .status
            .success()
    );

    let migrate = shore(["store", "migrate", "--repo", &repo_arg]);
    assert!(!migrate.status.success(), "a block finding still refuses");
    let stderr = String::from_utf8_lossy(&migrate.stderr);
    assert!(
        stderr.contains("sensitivity.json"),
        "the refusal names the targeted alternative: {stderr}"
    );
    assert!(
        stderr.contains("include-ephemeral"),
        "the blanket override stays documented: {stderr}"
    );
}

#[test]
fn store_migrate_include_ephemeral_omits_the_excluded_count() {
    let repo = repo_with_pending_change();
    let repo_arg = repo.path().to_str().unwrap().to_owned();
    assert!(
        shore(["store", "mode", "ephemeral", "--repo", &repo_arg])
            .status
            .success()
    );
    assert!(shore(["capture", "--repo", &repo_arg]).status.success());

    // --include-ephemeral skips the gate scan entirely, so no count is
    // reported (absent, not zero — zero would claim a scan ran).
    let migrate = shore([
        "store",
        "migrate",
        "--include-ephemeral",
        "--repo",
        &repo_arg,
    ]);
    assert!(
        migrate.status.success(),
        "override migrate: {}",
        String::from_utf8_lossy(&migrate.stderr)
    );
    let json = parse_json(&migrate.stdout);
    assert!(
        json.get("sensitivityExcludedPathCount").is_none(),
        "the count is omitted when the scan did not run: {json}"
    );
}

#[test]
fn store_migrate_refuses_ephemeral_without_include_ephemeral() {
    let repo = repo_with_pending_change();
    let capture = shore(["capture", "--repo", repo.path().to_str().unwrap()]);
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
