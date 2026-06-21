use shoreline::git::{git_worktree_root, ingest_tracked_diff};
use shoreline::session::event::{EventType, ShoreEvent};
use shoreline::session::{
    CaptureOptions, ImportNotesOptions, SessionState, capture_worktree_fingerprint,
    capture_worktree_review, ensure_shore_storage_excluded, import_notes,
    load_durable_notes_for_repo, read_events, rebuild_state, store_dir_for_repo,
};

use crate::support::git_repo::GitRepo;
use crate::support::{assert_existing_paths_eq, common_dir_store};

#[test]
fn shore_dir_resolves_to_git_worktree_root_from_subdirectory() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn demo() {}\n");
    let subdir = repo.path().join("src");

    let root = git_worktree_root(&subdir).expect("git root resolves");
    let store_dir = store_dir_for_repo(&subdir).expect("store dir resolves");

    assert_existing_paths_eq(&root, repo.path());
    // From a subdirectory, the public helper resolves the repo's shared common-dir
    // store (`.git/shore`), the same store the read/write seams use.
    assert_eq!(path_file_name(&store_dir), "shore");
    assert_eq!(path_file_name(path_parent(&store_dir)), ".git");
    assert_existing_paths_eq(path_parent(path_parent(&store_dir)), repo.path());
}

#[test]
fn ensure_shore_storage_excluded_uses_local_exclude_without_dirtying_worktree() {
    let repo = GitRepo::new();

    ensure_shore_storage_excluded(repo.path()).expect("exclude entry is written");
    ensure_shore_storage_excluded(repo.path()).expect("exclude entry is idempotent");

    // The local exclude carries exactly one `.shore/data/` entry, even across repeats.
    assert_eq!(
        read_local_exclude(&repo)
            .lines()
            .filter(|line| line.trim() == ".shore/data/")
            .count(),
        1
    );
    // No tracked .gitignore is created, so the worktree stays clean.
    assert!(
        !repo.path().join(".gitignore").exists(),
        "ensure must not create a tracked .gitignore"
    );
    assert!(
        repo.git(["status", "--short"]).stdout.trim().is_empty(),
        "worktree must stay clean after excluding storage"
    );
    // `.shore/data/` is now effectively ignored.
    assert!(shore_is_ignored(&repo));
}

#[test]
fn ensure_shore_storage_excluded_leaves_tracked_gitignore_untouched() {
    let repo = GitRepo::new();
    repo.write(".gitignore", "target/\n");
    repo.commit_all("add gitignore");

    ensure_shore_storage_excluded(repo.path()).expect("exclude entry is written");

    // The tracked .gitignore is never rewritten.
    assert_eq!(repo.read(".gitignore"), "target/\n");
    assert!(
        repo.git(["status", "--short"]).stdout.trim().is_empty(),
        "excluding storage must not modify the tracked .gitignore"
    );
    // `.shore/data/` lands in the local exclude instead, and is ignored.
    assert!(
        read_local_exclude(&repo)
            .lines()
            .any(|line| line.trim() == ".shore/data/")
    );
    assert!(shore_is_ignored(&repo));
}

#[test]
fn ensure_shore_storage_excluded_is_noop_when_gitignore_already_ignores_storage() {
    let repo = GitRepo::new();
    repo.write(
        ".gitignore",
        "# .shore/data/ is intentionally ignored below\n.shore/data\n",
    );
    repo.commit_all("ignore shore storage in gitignore");

    ensure_shore_storage_excluded(repo.path()).expect("existing ignore is respected");

    // The user's .gitignore choice is respected, and no redundant local entry is added.
    assert_eq!(
        repo.read(".gitignore"),
        "# .shore/data/ is intentionally ignored below\n.shore/data\n"
    );
    assert!(
        !read_local_exclude(&repo)
            .lines()
            .any(|line| line.trim().contains(".shore")),
        "must not add a redundant local exclude entry"
    );
}

#[test]
fn ensure_shore_storage_excluded_is_idempotent_against_existing_local_exclude_entry() {
    let repo = GitRepo::new();
    // Pre-seed the local exclude as a field-fix workaround would.
    let exclude_path = repo.path().join(".git/info/exclude");
    std::fs::write(&exclude_path, "# local excludes\n.shore/data/\n").expect("seed local exclude");

    ensure_shore_storage_excluded(repo.path()).expect("existing local exclude is respected");

    assert_eq!(
        read_local_exclude(&repo),
        "# local excludes\n.shore/data/\n"
    );
}

#[test]
fn read_events_uses_worktree_store_dir_from_subdirectory() {
    let repo = modified_repo();
    capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

    let events = read_events(repo.path().join("src")).unwrap();

    assert!(!events.is_empty());
}

#[test]
fn rebuild_state_resolves_the_store_from_a_subdirectory() {
    let repo = modified_repo();
    capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
    let store = common_dir_store(repo.path());
    std::fs::remove_file(store.join("state.json")).unwrap();

    rebuild_state(repo.path().join("src")).unwrap();

    assert!(store.join("state.json").is_file());
}

#[test]
fn nested_git_repo_uses_its_own_worktree_root() {
    let outer = GitRepo::new();
    outer.write("nested/.keep", "");
    let nested = outer.path().join("nested");
    GitRepo::init_at(&nested);

    assert_existing_paths_eq(&git_worktree_root(&nested).unwrap(), &nested);
}

fn path_parent(path: &std::path::Path) -> &std::path::Path {
    path.parent().expect("path has parent")
}

fn path_file_name(path: &std::path::Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .expect("path has utf-8 file name")
}

#[test]
fn same_working_tree_diff_produces_same_revision_and_snapshot_ids() {
    let repo = modified_repo();

    let first = capture_worktree_fingerprint(repo.path()).expect("first capture");
    let second = capture_worktree_fingerprint(repo.path()).expect("second capture");

    assert_eq!(first.revision_id, second.revision_id);
    assert_eq!(first.snapshot_id, second.snapshot_id);
    assert!(
        first
            .revision_id
            .as_str()
            .starts_with("rev:worktree:sha256:")
    );
    assert!(first.snapshot_id.as_str().starts_with("snap:git:sha256:"));
}

#[test]
fn shore_state_does_not_affect_revision_fingerprint() {
    let repo = modified_repo();
    ensure_shore_storage_excluded(repo.path()).expect("ignore shore state");

    let before = capture_worktree_fingerprint(repo.path()).expect("capture before shore state");
    repo.write(".shore/data/state.json", "changed notes");
    let after = capture_worktree_fingerprint(repo.path()).expect("capture after shore state");

    assert_eq!(before.revision_id, after.revision_id);
    assert_eq!(before.snapshot_id, after.snapshot_id);
}

#[test]
fn tracked_and_untracked_content_changes_change_revision_id() {
    let repo = modified_repo();
    let before = capture_worktree_fingerprint(repo.path()).expect("capture before untracked");

    repo.write("untracked.rs", "pub fn new() {}\n");
    let after_untracked =
        capture_worktree_fingerprint(repo.path()).expect("capture after untracked");
    assert_ne!(before.revision_id, after_untracked.revision_id);
    assert_ne!(before.snapshot_id, after_untracked.snapshot_id);

    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");
    let after_tracked = capture_worktree_fingerprint(repo.path()).expect("capture after tracked");
    assert_ne!(after_untracked.revision_id, after_tracked.revision_id);
    assert_ne!(after_untracked.snapshot_id, after_tracked.snapshot_id);
}

#[test]
fn git_ingestion_uses_content_derived_snapshot_id() {
    let repo = modified_repo();

    let fingerprint = capture_worktree_fingerprint(repo.path()).expect("capture fingerprint");
    let snapshot = ingest_tracked_diff(repo.path()).expect("ingest snapshot");

    assert_eq!(snapshot.snapshot_id, fingerprint.snapshot_id);
}

#[test]
fn first_capture_creates_shore_store_events_artifacts_and_state() {
    let repo = modified_repo();

    let result =
        capture_worktree_review(CaptureOptions::new(repo.path())).expect("capture succeeds");

    let store = common_dir_store(repo.path());
    assert!(store.join("events").is_dir());
    assert!(store.join("artifacts/snapshots").is_dir());
    assert!(store.join("state.json").is_file());
    // Storage is registered in the repository-local exclude, never the tracked
    // worktree .gitignore.
    assert!(
        read_local_exclude(&repo)
            .lines()
            .any(|line| line.trim() == ".shore/data/")
    );
    assert!(
        !repo.path().join(".gitignore").exists(),
        "capture must not create a tracked .gitignore"
    );
    assert_eq!(result.events_created_by_type["work_object_proposed"], 1);

    let state: SessionState =
        serde_json::from_str(&std::fs::read_to_string(store.join("state.json")).unwrap())
            .expect("state decodes");
    assert_eq!(state.current_review_unit_id, Some(result.revision_id));
    assert_eq!(state.review_unit_count, 1);
    assert_eq!(state.event_count, 2);
}

#[test]
fn capture_does_not_dirty_worktree_or_leak_storage_into_snapshot() {
    let repo = GitRepo::new();
    repo.write("src.txt", "alpha\n");
    repo.commit_all("base");

    // The worktree is clean before any Shoreline command runs.
    assert!(
        repo.git(["status", "--short"]).stdout.trim().is_empty(),
        "worktree should start clean"
    );

    capture_worktree_review(CaptureOptions::new(repo.path())).expect("capture succeeds");

    // Initializing review state leaves no tracked .gitignore edit, and the
    // excluded `.shore/data/` storage stays out of git status.
    assert!(
        !repo.path().join(".gitignore").exists(),
        "capture must not create a tracked .gitignore"
    );
    let status = repo.git(["status", "--short"]).stdout;
    assert!(
        status.trim().is_empty(),
        "capture must keep the worktree clean, got:\n{status}"
    );

    // The captured snapshot carries no Shoreline storage or ignore-file rows.
    let snapshot = ingest_tracked_diff(repo.path()).expect("ingest snapshot");
    assert!(
        snapshot.files.iter().all(|file| {
            let mentions_shore_state = |path: &str| {
                path == ".gitignore" || path == ".shore" || path.starts_with(".shore/")
            };
            !file.new_path.as_deref().is_some_and(mentions_shore_state)
                && !file.old_path.as_deref().is_some_and(mentions_shore_state)
        }),
        "snapshot must not include Shoreline storage or .gitignore rows, got: {:?}",
        snapshot.files
    );
}

#[test]
fn capture_unchanged_worktree_is_idempotent() {
    let repo = modified_repo();

    let first = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
    let second = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();

    assert_eq!(first.revision_id, second.revision_id);
    assert_eq!(second.events_created, 0);
    assert!(second.events_existing >= 1);
}

#[test]
fn capture_writer_identity_prefers_git_config_email() {
    let repo = modified_repo();

    let result = capture_worktree_review(CaptureOptions::new(repo.path())).unwrap();
    let events = read_events(repo.path()).expect("events list");
    let event = events
        .iter()
        .find(|event| {
            event.event_type == EventType::WorkObjectProposed
                && event.payload["workObject"]["revision"]["id"] == result.revision_id.as_str()
        })
        .expect("review unit event exists");

    assert_eq!(
        event.writer.actor_id.as_str(),
        "actor:git-email:shore-tests@example.com"
    );
}

#[test]
fn import_notes_from_native_sidecar_records_note_events_and_updates_state() {
    let repo = modified_repo();
    let sidecar = write_native_review_notes(&repo);

    let result =
        import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(&sidecar)).unwrap();
    let state: SessionState = serde_json::from_str(
        &std::fs::read_to_string(common_dir_store(repo.path()).join("state.json")).unwrap(),
    )
    .unwrap();

    assert_eq!(result.note_count, 1);
    assert_eq!(result.notes_created, 1);
    assert_eq!(result.notes_existing, 0);
    assert_eq!(state.note_count, 1);
    assert_eq!(review_note_imported_events(&repo).len(), 1);
}

#[test]
fn reimporting_same_sidecar_is_idempotent() {
    let repo = modified_repo();
    let sidecar = write_native_review_notes(&repo);

    let first =
        import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(&sidecar)).unwrap();
    let second =
        import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(&sidecar)).unwrap();
    let state: SessionState = serde_json::from_str(
        &std::fs::read_to_string(common_dir_store(repo.path()).join("state.json")).unwrap(),
    )
    .unwrap();

    assert_eq!(first.notes_created, 1);
    assert_eq!(second.notes_created, 0);
    assert_eq!(second.notes_existing, 1);
    assert_eq!(state.note_count, 1);
    assert_eq!(review_note_imported_events(&repo).len(), 1);
}

#[test]
fn changing_imported_note_creates_one_new_durable_event() {
    let repo = modified_repo();
    let sidecar = write_native_review_notes(&repo);

    import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(&sidecar)).unwrap();
    std::fs::write(&sidecar, changed_review_notes_json()).unwrap();

    let result =
        import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(&sidecar)).unwrap();
    let state: SessionState = serde_json::from_str(
        &std::fs::read_to_string(common_dir_store(repo.path()).join("state.json")).unwrap(),
    )
    .unwrap();

    assert_eq!(result.notes_created, 1);
    assert_eq!(result.notes_existing, 0);
    assert_eq!(state.note_count, 2);
    assert_eq!(review_note_imported_events(&repo).len(), 2);
}

#[test]
fn importing_notes_auto_initializes_shore() {
    let repo = modified_repo();
    let sidecar = write_native_review_notes(&repo);

    let result =
        import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(&sidecar)).unwrap();
    let store = common_dir_store(repo.path());
    let state: SessionState =
        serde_json::from_str(&std::fs::read_to_string(store.join("state.json")).unwrap()).unwrap();

    assert!(store.join("events").is_dir());
    assert!(store.join("state.json").is_file());
    assert_eq!(result.note_count, 1);
    assert_eq!(state.note_count, 1);
    assert_eq!(state.current_revision_id, None);
    assert_eq!(state.current_snapshot_id, None);
}

#[test]
fn malformed_import_sidecar_fails_without_creating_note_events() {
    let repo = modified_repo();
    let sidecar = repo.path().join("bad-review-notes.json");
    std::fs::write(&sidecar, "{").unwrap();

    let error =
        import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(&sidecar)).unwrap_err();

    assert!(error.to_string().contains("json"));
    assert!(!repo.path().join(".shore").exists());
}

#[test]
fn large_note_body_is_written_to_content_addressed_artifact() {
    let repo = modified_repo();
    let sidecar = write_review_notes_with_body(&repo, &"x".repeat(5000));

    import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(&sidecar)).unwrap();
    let note = first_note_event(&repo);

    assert!(note.payload["body"].is_null());
    assert!(
        note.payload["bodyArtifactPath"]
            .as_str()
            .unwrap()
            .starts_with("artifacts/notes/")
    );
    assert_eq!(
        note_body_artifact_file_count(&common_dir_store(repo.path())),
        1
    );
}

#[test]
fn small_note_body_remains_inline_without_note_body_artifact() {
    let repo = modified_repo();
    let sidecar = write_review_notes_with_body(&repo, "small body");

    import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(&sidecar)).unwrap();
    let note = first_note_event(&repo);

    assert_eq!(note.payload["body"], "small body");
    assert!(note.payload["bodyArtifactPath"].is_null());
    assert_eq!(
        note_body_artifact_file_count(&common_dir_store(repo.path())),
        0
    );
}

#[test]
fn reimporting_same_long_body_reuses_content_addressed_artifact_path() {
    let repo = modified_repo();
    let sidecar = write_review_notes_with_body(&repo, &"x".repeat(5000));

    import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(&sidecar)).unwrap();
    let first_path = first_note_event(&repo).payload["bodyArtifactPath"]
        .as_str()
        .unwrap()
        .to_owned();
    let second =
        import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(&sidecar)).unwrap();
    let second_path = first_note_event(&repo).payload["bodyArtifactPath"]
        .as_str()
        .unwrap()
        .to_owned();

    assert_eq!(second.notes_created, 0);
    assert_eq!(second.notes_existing, 1);
    assert_eq!(first_path, second_path);
}

#[test]
fn ledger_pipeline_records_capture_import_and_bounded_state() {
    let repo = bounded_ledger_repo();
    let state_json =
        std::fs::read_to_string(common_dir_store(repo.path()).join("state.json")).unwrap();
    let state: serde_json::Value = serde_json::from_str(&state_json).expect("state is json");

    assert_eq!(state["schema"], "shore.state");
    assert_eq!(state["eventCount"], 4);
    assert!(
        state["eventSetHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert_eq!(state["reviewUnitCount"], 1);
    assert_eq!(state["noteCount"], 1);
    assert!(state.get("events").is_none());
    assert_eq!(event_file_count(&common_dir_store(repo.path())), 4);
}

#[test]
fn state_event_set_hash_changes_when_events_change() {
    let repo = modified_repo();
    capture_worktree_review(CaptureOptions::new(repo.path())).expect("capture succeeds");
    let store = common_dir_store(repo.path());
    let capture_state: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(store.join("state.json")).unwrap())
            .expect("capture state");

    let sidecar = repo.write_fixture("review-notes.json", native_review_notes_json());
    import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(sidecar))
        .expect("notes import succeeds");
    let import_state: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(store.join("state.json")).unwrap())
            .expect("import state");

    assert_eq!(capture_state["eventCount"], 2);
    assert_eq!(import_state["eventCount"], 4);
    assert_ne!(capture_state["eventSetHash"], import_state["eventSetHash"]);
}

#[test]
fn state_can_be_deleted_and_rebuilt_from_events() {
    let repo = bounded_ledger_repo();
    let store = common_dir_store(repo.path());
    let original_state = std::fs::read_to_string(store.join("state.json")).unwrap();
    std::fs::remove_file(store.join("state.json")).unwrap();

    let rebuilt = rebuild_state(repo.path()).expect("state rebuilds");
    let rebuilt_state = std::fs::read_to_string(store.join("state.json")).unwrap();

    assert!(store.join("state.json").is_file());
    assert!(rebuilt.event_count >= 1);
    let original: serde_json::Value = serde_json::from_str(&original_state).unwrap();
    let rebuilt: serde_json::Value = serde_json::from_str(&rebuilt_state).unwrap();
    assert_eq!(rebuilt, original);
}

#[test]
fn corrupt_state_json_is_ignored_and_rebuilt_from_events() {
    let repo = bounded_ledger_repo();
    let store = common_dir_store(repo.path());
    let original_state = std::fs::read_to_string(store.join("state.json")).unwrap();
    std::fs::write(store.join("state.json"), "{").unwrap();

    rebuild_state(repo.path()).expect("state rebuilds from events");
    let rebuilt_state = std::fs::read_to_string(store.join("state.json")).unwrap();

    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&rebuilt_state).unwrap(),
        serde_json::from_str::<serde_json::Value>(&original_state).unwrap()
    );
}

#[test]
fn event_store_detects_corrupted_event_payload_hash() {
    let repo = bounded_ledger_repo();
    corrupt_first_event_payload(&common_dir_store(repo.path()));

    let error = rebuild_state(repo.path()).expect_err("corrupt event is rejected");

    assert!(error.to_string().contains("payload"));
}

#[test]
fn load_durable_notes_for_repo_returns_none_without_shore_store() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");

    let parsed = load_durable_notes_for_repo(repo.path()).expect("load succeeds");

    assert_eq!(parsed, None);
    assert!(!repo.path().join(".shore").exists());
}

#[test]
fn load_durable_notes_for_repo_replays_imported_notes() {
    let repo = modified_repo();
    let sidecar = write_native_review_notes(&repo);
    import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(&sidecar))
        .expect("notes import succeeds");

    let parsed = load_durable_notes_for_repo(repo.path())
        .expect("load succeeds")
        .expect("durable notes exist");

    assert_eq!(parsed.sidecar.files.len(), 1);
    assert_eq!(parsed.sidecar.files[0].notes.len(), 1);
    assert_eq!(
        parsed.sidecar.files[0].notes[0].title.as_deref(),
        Some("Changed return value")
    );
}

#[test]
fn load_durable_notes_for_repo_returns_none_with_empty_store() {
    let repo = modified_repo();
    capture_worktree_review(CaptureOptions::new(repo.path())).expect("capture succeeds");

    assert!(common_dir_store(repo.path()).join("events").exists());

    let parsed = load_durable_notes_for_repo(repo.path()).expect("load succeeds");

    assert_eq!(parsed, None);
}

#[test]
fn load_durable_notes_for_repo_resolves_large_body_artifact() {
    let repo = modified_repo();
    let large_body = "x".repeat(5000);
    let sidecar = write_review_notes_with_body(&repo, &large_body);
    import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(&sidecar))
        .expect("notes import succeeds");

    let parsed = load_durable_notes_for_repo(repo.path())
        .unwrap()
        .expect("durable notes exist");

    assert_eq!(
        parsed.sidecar.files[0].notes[0].body.as_deref(),
        Some(&large_body[..])
    );
}

#[test]
fn artifacts_notes_directory_is_not_a_complete_note_body_inventory() {
    let repo = GitRepo::new();
    repo.write_fixture("README.md", "# fixture\n");
    repo.commit_all("init");

    let sidecar = repo.path().join("two-notes.json");
    let large = "x".repeat(5_000);
    std::fs::write(
        &sidecar,
        review_notes_with_two_bodies_json("small body", &large),
    )
    .unwrap();

    import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(&sidecar)).unwrap();

    // Two ReviewNoteImported events, but only ONE materialized body artifact:
    // the large body. The small body remains inline in its event payload.
    let imported_count = read_events(repo.path())
        .unwrap()
        .iter()
        .filter(|e| e.event_type == EventType::ReviewNoteImported)
        .count();
    assert_eq!(imported_count, 2, "expected two ReviewNoteImported events");
    assert_eq!(
        note_body_artifact_file_count(&common_dir_store(repo.path())),
        1,
        "artifacts/notes/ is an overflow store — only the large body should materialize",
    );
}

#[test]
fn load_durable_notes_for_repo_still_works_with_small_inline_bodies() {
    let repo = modified_repo();
    let small_body = "small body content";
    let sidecar = write_review_notes_with_body(&repo, small_body);
    import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(&sidecar))
        .expect("notes import succeeds");

    let parsed = load_durable_notes_for_repo(repo.path())
        .unwrap()
        .expect("durable notes exist");

    assert_eq!(
        parsed.sidecar.files[0].notes[0].body.as_deref(),
        Some(small_body)
    );
}

fn modified_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo
}

fn bounded_ledger_repo() -> GitRepo {
    let repo = modified_repo();
    capture_worktree_review(CaptureOptions::new(repo.path())).expect("capture succeeds");
    let sidecar = repo.write_fixture("review-notes.json", native_review_notes_json());
    import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(sidecar))
        .expect("notes import succeeds");
    repo
}

fn read_local_exclude(repo: &GitRepo) -> String {
    std::fs::read_to_string(repo.path().join(".git/info/exclude")).unwrap_or_default()
}

fn shore_is_ignored(repo: &GitRepo) -> bool {
    // `git check-ignore` prints the path when it is ignored and exits 1 (no
    // output) otherwise, so a non-empty stdout means storage is excluded.
    let output = std::process::Command::new("git")
        .args(["check-ignore", ".shore/data/state.json"])
        .current_dir(repo.path())
        .output()
        .expect("run git check-ignore");
    !output.stdout.is_empty()
}

fn event_file_count(store: &std::path::Path) -> usize {
    std::fs::read_dir(store.join("events"))
        .map(|entries| {
            entries
                .filter(|entry| {
                    entry.as_ref().is_ok_and(|entry| {
                        entry.path().extension().and_then(|ext| ext.to_str()) == Some("json")
                    })
                })
                .count()
        })
        .unwrap_or_default()
}

fn note_body_artifact_file_count(store: &std::path::Path) -> usize {
    let dir = store.join("artifacts/notes");
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter(|entry| {
                    entry.as_ref().is_ok_and(|entry| {
                        entry.path().extension().and_then(|ext| ext.to_str()) == Some("json")
                    })
                })
                .count()
        })
        .unwrap_or_default()
}

fn corrupt_first_event_payload(store: &std::path::Path) {
    let mut event_files = std::fs::read_dir(store.join("events"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    event_files.sort();

    let event_path = event_files.first().expect("event exists");
    let mut event: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(event_path).unwrap()).unwrap();
    event["payload"]["tampered"] = serde_json::Value::Bool(true);
    std::fs::write(event_path, serde_json::to_string_pretty(&event).unwrap()).unwrap();
}

fn review_note_imported_events(repo: &GitRepo) -> Vec<ShoreEvent> {
    read_events(repo.path())
        .expect("events list")
        .into_iter()
        .filter(|event| event.event_type == EventType::ReviewNoteImported)
        .collect()
}

fn first_note_event(repo: &GitRepo) -> ShoreEvent {
    review_note_imported_events(repo)
        .into_iter()
        .next()
        .expect("review note imported event exists")
}

fn write_native_review_notes(repo: &GitRepo) -> std::path::PathBuf {
    let sidecar = repo.path().join("review-notes.json");
    std::fs::write(&sidecar, native_review_notes_json()).unwrap();
    sidecar
}

fn write_review_notes_with_body(repo: &GitRepo, body: &str) -> std::path::PathBuf {
    let sidecar = repo.path().join("review-notes.json");
    std::fs::write(&sidecar, review_notes_with_body_json(body)).unwrap();
    sidecar
}

fn native_review_notes_json() -> &'static str {
    r#"{
  "schema": "shore.review-notes",
  "version": 1,
  "files": [
    {
      "path": "src/lib.rs",
      "notes": [
        {
          "title": "Changed return value",
          "target": { "side": "new", "startLine": 1, "endLine": 1 }
        }
      ]
    }
  ]
}"#
}

fn changed_review_notes_json() -> &'static str {
    r#"{
  "schema": "shore.review-notes",
  "version": 1,
  "summary": "changed only in sidecar",
  "files": [
    {
      "path": "src/lib.rs",
      "notes": [
        {
          "title": "Changed return value again",
          "target": { "side": "new", "startLine": 1, "endLine": 1 }
        }
      ]
    }
  ]
}"#
}

fn review_notes_with_body_json(body: &str) -> String {
    format!(
        r#"{{
  "schema": "shore.review-notes",
  "version": 1,
  "files": [
    {{
      "path": "src/lib.rs",
      "notes": [
        {{
          "title": "Changed return value",
          "body": {body:?},
          "target": {{ "side": "new", "startLine": 1, "endLine": 1 }}
        }}
      ]
    }}
  ]
}}"#
    )
}

fn review_notes_with_two_bodies_json(small_body: &str, large_body: &str) -> String {
    format!(
        r#"{{
  "schema": "shore.review-notes",
  "version": 1,
  "files": [
    {{
      "path": "src/lib.rs",
      "notes": [
        {{
          "title": "small",
          "body": {small_body:?},
          "target": {{ "side": "new", "startLine": 1, "endLine": 1 }}
        }},
        {{
          "title": "large",
          "body": {large_body:?},
          "target": {{ "side": "new", "startLine": 2, "endLine": 2 }}
        }}
      ]
    }}
  ]
}}"#
    )
}
