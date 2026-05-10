use shore::git::{git_worktree_root, ingest_tracked_diff};
use shore::session::{
    EventType, PublishOptions, RevisionPublishedPayload, SessionState, ShoreEvent,
    capture_worktree_fingerprint, ensure_shore_ignored, publish_worktree_review, read_events,
    rebuild_state, shore_dir_for_repo,
};

use crate::support::git_repo::GitRepo;

#[test]
fn shore_dir_resolves_to_git_worktree_root_from_subdirectory() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn demo() {}\n");
    let subdir = repo.path().join("src");

    let root = git_worktree_root(&subdir).expect("git root resolves");
    let shore_dir = shore_dir_for_repo(&subdir).expect("shore dir resolves");

    let expected_root = repo.path().canonicalize().expect("canonical repo root");
    assert_eq!(root, expected_root);
    assert_eq!(shore_dir, expected_root.join(".shore"));
}

#[test]
fn ensure_shore_ignored_creates_or_updates_root_gitignore_without_duplicates() {
    let repo = GitRepo::new();

    ensure_shore_ignored(repo.path()).expect("ignore entry is written");
    ensure_shore_ignored(repo.path()).expect("ignore entry is idempotent");

    let gitignore = repo.read(".gitignore");
    assert_eq!(
        gitignore
            .lines()
            .filter(|line| line.trim_end() == ".shore/")
            .count(),
        1
    );
}

#[test]
fn ensure_shore_ignored_appends_to_existing_gitignore_with_separator_newline() {
    let repo = GitRepo::new();
    repo.write(".gitignore", "target/\n!.keep");

    ensure_shore_ignored(repo.path()).expect("ignore entry is appended");

    assert_eq!(repo.read(".gitignore"), "target/\n!.keep\n.shore/\n");
}

#[test]
fn ensure_shore_ignored_treats_bare_shore_entry_as_existing_ignore() {
    let repo = GitRepo::new();
    repo.write(
        ".gitignore",
        "# .shore/ is intentionally ignored below\n.shore\n",
    );

    ensure_shore_ignored(repo.path()).expect("bare ignore entry is recognized");

    assert_eq!(
        repo.read(".gitignore"),
        "# .shore/ is intentionally ignored below\n.shore\n"
    );
}

#[test]
fn nested_git_repo_uses_its_own_worktree_root() {
    let outer = GitRepo::new();
    outer.write("nested/.keep", "");
    let nested = outer.path().join("nested");
    GitRepo::init_at(&nested);

    assert_eq!(
        git_worktree_root(&nested).unwrap(),
        nested.canonicalize().expect("canonical nested repo")
    );
}

#[test]
fn same_working_tree_diff_produces_same_revision_and_snapshot_ids() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");

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
fn sidecar_input_does_not_affect_revision_fingerprint() {
    let repo = modified_repo();
    ensure_shore_ignored(repo.path()).expect("ignore shore state");

    let before = capture_worktree_fingerprint(repo.path()).expect("capture before shore state");
    repo.write(".shore/state.json", "changed notes");
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
fn first_publish_creates_shore_store_events_artifacts_and_state() {
    let repo = modified_repo();

    let result =
        publish_worktree_review(PublishOptions::new(repo.path())).expect("publish succeeds");

    assert_eq!(result.review_id.as_str(), "review:default");
    assert_eq!(result.work_unit_id.as_str(), "work:default");
    assert!(repo.path().join(".shore/events").is_dir());
    assert!(repo.path().join(".shore/artifacts/revisions").is_dir());
    assert!(repo.path().join(".shore/artifacts/snapshots").is_dir());
    assert!(repo.path().join(".shore/state.json").is_file());
    assert!(
        repo.read(".gitignore")
            .lines()
            .any(|line| line == ".shore/")
    );
    assert_eq!(result.events_created_by_type["review_initialized"], 1);
    assert_eq!(result.events_created_by_type["revision_published"], 1);
    assert_eq!(result.events_created_by_type["snapshot_observed"], 1);

    let state: SessionState =
        serde_json::from_str(&repo.read(".shore/state.json")).expect("state decodes");
    assert_eq!(state.current_revision_id, Some(result.revision_id));
    assert_eq!(state.current_snapshot_id, Some(result.snapshot_id));
    assert_eq!(state.event_count, 3);
}

#[test]
fn publishing_unchanged_worktree_is_idempotent() {
    let repo = modified_repo();

    let first = publish_worktree_review(PublishOptions::new(repo.path())).unwrap();
    let second = publish_worktree_review(PublishOptions::new(repo.path())).unwrap();

    assert_eq!(first.revision_id, second.revision_id);
    assert_eq!(second.events_created, 0);
    assert!(second.events_existing >= 3);
}

#[test]
fn publishing_changed_worktree_supersedes_previous_revision() {
    let repo = modified_repo();
    let first = publish_worktree_review(PublishOptions::new(repo.path())).unwrap();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 3 }\n");

    let second = publish_worktree_review(PublishOptions::new(repo.path())).unwrap();
    let event = revision_event_for(&repo, &second.revision_id);
    let state: SessionState =
        serde_json::from_str(&repo.read(".shore/state.json")).expect("state decodes");

    assert_ne!(first.revision_id, second.revision_id);
    assert!(event.supersedes_revision_ids.contains(&first.revision_id));
    assert_eq!(state.current_revision_id, Some(second.revision_id));
    assert!(state.diagnostics.is_empty());
}

#[test]
fn publish_writer_identity_prefers_git_config_email() {
    let repo = modified_repo();

    let result = publish_worktree_review(PublishOptions::new(repo.path())).unwrap();
    let events = read_events(repo.path()).expect("events list");
    let event = events
        .iter()
        .find(|event| {
            event.event_type == EventType::RevisionPublished
                && event.payload["revisionId"] == result.revision_id.as_str()
        })
        .expect("revision event exists");

    assert_eq!(
        event.writer.actor_id.as_str(),
        "actor:git-email:shore-tests@example.com"
    );
}

#[test]
fn native_review_notes_publish_records_sidecar_observation() {
    let repo = modified_repo();
    let sidecar = write_native_review_notes(&repo);

    let result = publish_worktree_review(
        PublishOptions::new(repo.path()).with_review_notes(sidecar.clone()),
    )
    .expect("publish succeeds");

    let event = sidecar_observed_event(&repo, "review_notes");
    assert_eq!(event.payload["source"], "review_notes");
    assert_eq!(
        event.payload["path"].as_str().unwrap(),
        sidecar.to_string_lossy()
    );
    assert_eq!(event.payload["schema"], "shore.review-notes");
    assert_eq!(event.payload["version"], 1);
    assert_eq!(
        event.payload["byteSize"].as_u64().unwrap(),
        native_review_notes_json().len() as u64
    );
    assert!(
        event.payload["contentHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert_eq!(event.payload["diagnosticCount"], 0);
    assert_eq!(event.payload["diagnosticLevels"]["warning"], 0);
    assert_ne!(event.payload_hash, event.payload["contentHash"]);

    let state: SessionState =
        serde_json::from_str(&repo.read(".shore/state.json")).expect("state decodes");
    assert_eq!(state.sidecar_count, 1);
    assert_eq!(result.events_created_by_type["sidecar_observed"], 1);
}

#[test]
fn legacy_hunk_context_publish_records_legacy_sidecar_observation() {
    let repo = modified_repo();
    let sidecar = write_legacy_hunk_context(&repo);

    publish_worktree_review(
        PublishOptions::new(repo.path()).with_legacy_hunk_agent_context(sidecar.clone()),
    )
    .expect("publish succeeds");

    let event = sidecar_observed_event(&repo, "legacy_hunk_agent_context");
    assert_eq!(event.payload["source"], "legacy_hunk_agent_context");
    assert_eq!(
        event.payload["path"].as_str().unwrap(),
        sidecar.to_string_lossy()
    );
    assert_eq!(event.payload["schema"], "shore.review-notes");
    assert_eq!(event.payload["importedSchema"], "shore.agent-context");
    assert_eq!(event.payload["version"], 1);
}

#[test]
fn sidecar_content_change_records_new_observation_without_changing_revision() {
    let repo = modified_repo();
    let sidecar = write_native_review_notes(&repo);
    let first = publish_worktree_review(
        PublishOptions::new(repo.path()).with_review_notes(sidecar.clone()),
    )
    .unwrap();

    std::fs::write(&sidecar, changed_review_notes_json()).unwrap();
    let second =
        publish_worktree_review(PublishOptions::new(repo.path()).with_review_notes(sidecar))
            .unwrap();

    assert_eq!(first.revision_id, second.revision_id);
    assert_eq!(second.events_created_by_type["sidecar_observed"], 1);
}

#[test]
fn excluded_helper_path_does_not_affect_publish_revision() {
    let repo = modified_repo();
    let log_path = repo.path().join("shore.log");
    std::fs::write(&log_path, "first log\n").unwrap();

    let first = publish_worktree_review(
        PublishOptions::new(repo.path()).with_excluded_helper_path(&log_path),
    )
    .unwrap();

    std::fs::write(&log_path, "changed log\n").unwrap();
    let second = publish_worktree_review(
        PublishOptions::new(repo.path()).with_excluded_helper_path(&log_path),
    )
    .unwrap();

    repo.write("src/unrelated.rs", "pub fn unrelated() {}\n");
    let third = publish_worktree_review(
        PublishOptions::new(repo.path()).with_excluded_helper_path(&log_path),
    )
    .unwrap();

    assert_eq!(first.revision_id, second.revision_id);
    assert_eq!(second.events_created, 0);
    assert_ne!(second.revision_id, third.revision_id);
}

#[test]
fn malformed_sidecar_fails_before_writing_events() {
    let repo = modified_repo();
    let sidecar = repo.path().join("bad-review-notes.json");
    std::fs::write(&sidecar, "{").unwrap();

    let error =
        publish_worktree_review(PublishOptions::new(repo.path()).with_review_notes(sidecar))
            .expect_err("malformed sidecar is fatal");

    assert!(error.to_string().contains("json"));
    assert!(!repo.path().join(".shore").exists());
}

#[test]
fn publish_pipeline_records_events_sidecar_observation_and_bounded_state() {
    let repo = bounded_publish_repo();
    let sidecar = repo.write_fixture("review-notes.json", native_review_notes_json());

    let result =
        publish_worktree_review(PublishOptions::new(repo.path()).with_review_notes(sidecar))
            .expect("publish succeeds");
    let state_json = repo.read(".shore/state.json");
    let state: serde_json::Value = serde_json::from_str(&state_json).expect("state is json");

    assert_eq!(state["schema"], "shore.state");
    assert_eq!(state["eventCount"], 4);
    assert_eq!(state["sidecarCount"], 1);
    assert!(state.get("events").is_none());
    assert_eq!(event_file_count(repo.path()), 4);
    assert_eq!(result.diagnostics.len(), 0);
}

#[test]
fn state_can_be_deleted_and_rebuilt_from_events() {
    let repo = bounded_publish_repo();
    publish_worktree_review(PublishOptions::new(repo.path())).expect("publish succeeds");
    let original_state = repo.read(".shore/state.json");
    std::fs::remove_file(repo.path().join(".shore/state.json")).unwrap();

    let rebuilt = rebuild_state(repo.path()).expect("state rebuilds");
    let rebuilt_state = repo.read(".shore/state.json");

    assert!(repo.path().join(".shore/state.json").is_file());
    assert!(rebuilt.event_count >= 3);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&rebuilt_state).unwrap(),
        serde_json::from_str::<serde_json::Value>(&original_state).unwrap()
    );
}

#[test]
fn corrupt_state_json_is_ignored_and_rebuilt_from_events() {
    let repo = bounded_publish_repo();
    publish_worktree_review(PublishOptions::new(repo.path())).expect("publish succeeds");
    let original_state = repo.read(".shore/state.json");
    std::fs::write(repo.path().join(".shore/state.json"), "{").unwrap();

    rebuild_state(repo.path()).expect("state rebuilds from events");
    let rebuilt_state = repo.read(".shore/state.json");

    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&rebuilt_state).unwrap(),
        serde_json::from_str::<serde_json::Value>(&original_state).unwrap()
    );
}

#[test]
fn event_store_detects_corrupted_event_payload_hash() {
    let repo = bounded_publish_repo();
    publish_worktree_review(PublishOptions::new(repo.path())).expect("publish succeeds");
    corrupt_first_event_payload(repo.path());

    let error = rebuild_state(repo.path()).expect_err("corrupt event is rejected");

    assert!(error.to_string().contains("payload"));
}

fn modified_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo
}

fn bounded_publish_repo() -> GitRepo {
    let repo = modified_repo();
    repo.write("src/untracked.rs", "pub fn untracked() -> u32 { 3 }\n");
    repo
}

fn event_file_count(repo: &std::path::Path) -> usize {
    std::fs::read_dir(repo.join(".shore/events"))
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
        })
        .count()
}

fn corrupt_first_event_payload(repo: &std::path::Path) {
    let event_path = std::fs::read_dir(repo.join(".shore/events"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .expect("event file exists");
    let mut event: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&event_path).unwrap()).unwrap();
    event["payload"]["revisionId"] = serde_json::json!("rev:worktree:sha256:corrupted");
    std::fs::write(&event_path, serde_json::to_vec(&event).unwrap()).unwrap();
}

fn revision_event_for(
    repo: &GitRepo,
    revision_id: &shore::model::RevisionId,
) -> RevisionPublishedPayload {
    read_events(repo.path())
        .expect("events list")
        .into_iter()
        .find_map(|event| {
            if event.event_type == EventType::RevisionPublished
                && event.payload["revisionId"] == revision_id.as_str()
            {
                Some(serde_json::from_value(event.payload).expect("revision payload decodes"))
            } else {
                None
            }
        })
        .expect("revision event exists")
}

fn sidecar_observed_event(repo: &GitRepo, source: &str) -> ShoreEvent {
    read_events(repo.path())
        .expect("events list")
        .into_iter()
        .find(|event| {
            event.event_type == EventType::SidecarObserved && event.payload["source"] == source
        })
        .expect("sidecar observed event exists")
}

fn write_native_review_notes(repo: &GitRepo) -> std::path::PathBuf {
    let sidecar = repo.path().join("review-notes.json");
    std::fs::write(&sidecar, native_review_notes_json()).unwrap();
    sidecar
}

fn write_legacy_hunk_context(repo: &GitRepo) -> std::path::PathBuf {
    let sidecar = repo.path().join("agent-context.json");
    std::fs::write(&sidecar, legacy_hunk_context_json()).unwrap();
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

fn legacy_hunk_context_json() -> &'static str {
    r#"{
  "schema": "shore.agent-context",
  "files": [
    {
      "path": "src/lib.rs",
      "annotations": [
        {
          "summary": "Changed return value",
          "newRange": [1, 1]
        }
      ]
    }
  ]
}"#
}
