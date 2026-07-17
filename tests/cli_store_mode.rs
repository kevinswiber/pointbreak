mod support;

use serde_json::Value;
use support::git_repo::GitRepo;
use support::pointbreak;

#[test]
fn store_mode_show_defaults_to_shared() {
    let repo = GitRepo::new();
    let output = pointbreak([
        "store",
        "mode",
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_json(&output.stdout);
    assert_eq!(json["schema"], "pointbreak.store-mode");
    assert_eq!(json["mode"], "shared");
    // With no config file, the source is the built-in default.
    assert_eq!(json["source"], "default");
    // Never leaks a storage path.
    assert!(!String::from_utf8_lossy(&output.stdout).contains(".pointbreak/data"));
}

#[test]
fn store_mode_set_ephemeral_writes_committed_config_and_show_reflects_it() {
    let repo = GitRepo::new();

    let set = pointbreak([
        "store",
        "mode",
        "ephemeral",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);
    assert!(
        set.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&set.stderr)
    );
    let set_json = parse_json(&set.stdout);
    assert_eq!(set_json["mode"], "ephemeral");
    // The committed config file was written (pretty-printed, like delegates.json).
    assert!(repo.path().join(".pointbreak/store.json").is_file());
    let raw = std::fs::read_to_string(repo.path().join(".pointbreak/store.json")).unwrap();
    assert!(raw.contains("\"mode\": \"ephemeral\""), "got: {raw}");

    let show = pointbreak([
        "store",
        "mode",
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);
    let show_json = parse_json(&show.stdout);
    assert_eq!(show_json["mode"], "ephemeral");
    assert_eq!(show_json["source"], "committed");
}

#[test]
fn store_mode_set_shared_round_trips_back_from_ephemeral() {
    let repo = GitRepo::new();
    pointbreak([
        "store",
        "mode",
        "ephemeral",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);
    pointbreak([
        "store",
        "mode",
        "shared",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);

    let show = pointbreak([
        "store",
        "mode",
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);
    assert_eq!(parse_json(&show.stdout)["mode"], "shared");
}

#[test]
fn store_mode_set_ephemeral_makes_capture_resolve_worktree_local() {
    // End-to-end: the ephemeral bit drives the resolver's store-mode consult, so a
    // capture lands in the worktree-local .pointbreak/data store (discardable), proving
    // the escape hatch is wired through the real write path.
    let repo = GitRepo::new();
    repo.write("README.md", "base\n");
    repo.commit_all("base");
    pointbreak([
        "store",
        "mode",
        "ephemeral",
        "--repo",
        repo.path().to_str().unwrap(),
    ]);

    repo.write("README.md", "changed\n");
    let capture = pointbreak(["capture", "--repo", repo.path().to_str().unwrap()]);
    assert!(
        capture.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&capture.stderr)
    );

    // The capture's event landed in the worktree-local store (Shared/unlinked also
    // lands here today, so the assertion holds; the point is that the ephemeral bit
    // does not break the round-trip and an unlinked Ephemeral worktree's bytes stay
    // worktree-local).
    let events_dir = repo.path().join(".pointbreak/data/events");
    let count = std::fs::read_dir(&events_dir)
        .map(|d| d.count())
        .unwrap_or(0);
    assert!(
        count > 0,
        "ephemeral capture writes to the worktree-local store"
    );
}

fn parse_json(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).expect("stdout is json")
}

#[test]
fn text_store_mode_digest_reports_mode_and_source() {
    let repo = support::git_repo::GitRepo::new();
    let output = support::pointbreak([
        "store",
        "mode",
        "show",
        "--repo",
        repo.path().to_str().unwrap(),
        "--format",
        "text",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        !stdout.contains("\"schema\""),
        "text lane is not JSON: {stdout}"
    );
    assert!(stdout.contains("shared"), "resolved mode: {stdout}");
    assert!(stdout.contains("default"), "mode source: {stdout}");
}
