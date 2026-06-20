use std::collections::BTreeMap;
use std::path::Path;

use crate::error::Result;
use crate::session::body_artifact::load_body_artifact;
use crate::session::event::{EventType, ReviewNoteImportedPayload, ShoreEvent};
use crate::session::state::SessionState;
use crate::session::store::resolution::{resolve_read_store, resolve_write_store};
use crate::session::{EventStore, sweep_stale_temp_files};
use crate::sidecar::{
    ParsedReviewNotes, ReviewNoteEntry, ReviewNoteTarget, ReviewNotesFile, ReviewNotesSidecar,
};
use crate::storage::{Durability, LocalStorage};

fn replay_note_entry(
    payload: &ReviewNoteImportedPayload,
    store_dir: &Path,
) -> Result<ReviewNoteEntry> {
    let target = payload
        .target
        .as_ref()
        .map(|imported_target| ReviewNoteTarget {
            side: imported_target.side,
            start_line: imported_target.start_line,
            end_line: imported_target.end_line,
        });

    let body = if let Some(artifact_path) = &payload.body_artifact_path {
        load_body_artifact(store_dir, artifact_path)?
    } else {
        payload.body.clone()
    };

    Ok(ReviewNoteEntry {
        id: Some(payload.note_id.clone()),
        title: Some(payload.title.clone()),
        body,
        target,
        tags: payload.tags.clone(),
        confidence: payload.confidence.clone(),
        source: payload.external_source.clone(),
        author: payload.author.clone(),
        created_at: payload.created_at.clone(),
    })
}

fn parsed_review_notes_from_imports(
    payloads: &[ReviewNoteImportedPayload],
    store_dir: &Path,
) -> Result<ParsedReviewNotes> {
    let mut file_map: BTreeMap<String, (Option<String>, Vec<ReviewNoteEntry>)> = BTreeMap::new();

    for payload in payloads {
        let entry = replay_note_entry(payload, store_dir)?;

        file_map
            .entry(payload.file_path.clone())
            .or_insert((payload.file_old_path.clone(), Vec::new()))
            .1
            .push(entry);
    }

    let files = file_map
        .into_iter()
        .map(|(path, (old_path, mut notes))| {
            notes.sort_by_key(|note| {
                (
                    note.target.map(|target| target.start_line).unwrap_or(0),
                    note.id.clone().unwrap_or_default(),
                )
            });
            ReviewNotesFile {
                path,
                old_path,
                summary: None,
                notes,
            }
        })
        .collect();

    Ok(ParsedReviewNotes {
        sidecar: ReviewNotesSidecar {
            schema: Some("shore.review-notes".to_owned()),
            version: 1,
            summary: None,
            files,
        },
        diagnostics: Vec::new(),
    })
}

pub fn load_durable_notes_for_repo(repo: impl AsRef<Path>) -> Result<Option<ParsedReviewNotes>> {
    let Some((store_dir, events)) = list_events_if_store_exists(repo)? else {
        return Ok(None);
    };

    let mut imported_payloads = Vec::new();
    for event in events {
        if event.event_type != EventType::ReviewNoteImported {
            continue;
        }

        let payload: ReviewNoteImportedPayload = serde_json::from_value(event.payload)?;
        imported_payloads.push(payload);
    }

    if imported_payloads.is_empty() {
        return Ok(None);
    }

    Ok(Some(parsed_review_notes_from_imports(
        &imported_payloads,
        &store_dir,
    )?))
}

#[cfg(test)]
fn load_or_rebuild_session_state(repo: impl AsRef<Path>) -> Result<Option<SessionState>> {
    let Some((_store_dir, events)) = list_events_if_store_exists(repo)? else {
        return Ok(None);
    };

    Ok(Some(SessionState::from_events(&events)?))
}

pub fn rebuild_state(repo: impl AsRef<Path>) -> Result<SessionState> {
    // Resolve the store the same way read and write surfaces do, so the rebuilt
    // projection lands in (and is replayed from) the resolved store — the shared
    // common-dir store by default — never a stale worktree-local copy.
    let write_store = resolve_write_store(repo.as_ref())?;
    let store_dir = write_store.store_dir();
    let worktree_root = write_store.worktree_root();
    let storage = LocalStorage::new(store_dir);
    sweep_stale_temp_files(&storage, store_dir)?;

    let span = tracing::info_span!("session.rebuild_state", repo = %worktree_root.display());
    let _entered = span.enter();

    let state = SessionState::from_events(&EventStore::open(store_dir).list_events()?)?;
    storage.write_json_atomic(
        &store_dir.join("state.json"),
        &state,
        Durability::Projection,
    )?;
    Ok(state)
}

pub fn read_events(repo: impl AsRef<Path>) -> Result<Vec<ShoreEvent>> {
    let read_store = resolve_read_store(repo.as_ref())?;
    EventStore::open(read_store.store_dir()).list_events()
}

fn list_events_if_store_exists(
    repo: impl AsRef<Path>,
) -> Result<Option<(std::path::PathBuf, Vec<ShoreEvent>)>> {
    let read_store = resolve_read_store(repo.as_ref())?;
    let store_dir = read_store.store_dir().to_path_buf();
    if !store_dir.exists() {
        return Ok(None);
    }

    let events = EventStore::open(&store_dir).list_events()?;
    Ok(Some((store_dir, events)))
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::*;
    use crate::model::{SessionId, Side, WorkUnitId};
    use crate::session::event::{
        EventTarget, ImportedNoteTarget, ReviewInitializedPayload, ShoreEvent, SidecarSource,
        Writer,
    };

    fn test_payload_for(file_path: &str, note_id: &str) -> ReviewNoteImportedPayload {
        ReviewNoteImportedPayload {
            sidecar_source: SidecarSource::ReviewNotes,
            note_id: note_id.to_owned(),
            file_path: file_path.to_owned(),
            file_old_path: None,
            target: Some(ImportedNoteTarget {
                side: Side::New,
                start_line: 2,
                end_line: 3,
            }),
            title: format!("Title for {}", note_id),
            body: Some(format!("Body for {}", note_id)),
            body_artifact_path: None,
            body_byte_size: None,
            tags: vec!["tag".to_owned()],
            confidence: Some("high".to_owned()),
            external_source: Some("tool".to_owned()),
            author: Some("reviewer".to_owned()),
            created_at: Some("2026-05-10T00:00:00Z".to_owned()),
            sidecar_content_hash: "sha256:abc".to_owned(),
        }
    }

    #[test]
    fn imported_note_payload_converts_to_review_note_entry() {
        let payload = ReviewNoteImportedPayload {
            sidecar_source: SidecarSource::ReviewNotes,
            note_id: "note:123".to_owned(),
            file_path: "src/lib.rs".to_owned(),
            file_old_path: None,
            target: Some(ImportedNoteTarget {
                side: Side::New,
                start_line: 2,
                end_line: 3,
            }),
            title: "Durable title".to_owned(),
            body: Some("Durable body".to_owned()),
            body_artifact_path: None,
            body_byte_size: None,
            tags: vec!["tag".to_owned()],
            confidence: Some("high".to_owned()),
            external_source: Some("tool".to_owned()),
            author: Some("reviewer".to_owned()),
            created_at: Some("2026-05-10T00:00:00Z".to_owned()),
            sidecar_content_hash: "sha256:abc".to_owned(),
        };

        let entry = replay_note_entry(&payload, Path::new(".shore/data")).expect("entry builds");

        assert_eq!(entry.id.as_deref(), Some("note:123"));
        assert_eq!(entry.title.as_deref(), Some("Durable title"));
        assert_eq!(entry.body.as_deref(), Some("Durable body"));
        assert_eq!(entry.target.unwrap().start_line, 2);
    }

    #[test]
    fn imported_notes_group_by_file_without_using_event_order_as_file_order() {
        let events = vec![
            test_payload_for("b.rs", "note:b"),
            test_payload_for("a.rs", "note:a"),
        ];

        let parsed =
            parsed_review_notes_from_imports(&events, Path::new(".shore/data")).expect("parses");

        assert_eq!(parsed.sidecar.files.len(), 2);
        assert_eq!(
            parsed
                .sidecar
                .files
                .iter()
                .map(|f| f.notes.len())
                .sum::<usize>(),
            2
        );
        assert_eq!(parsed.sidecar.files[0].path, "a.rs");
        assert_eq!(parsed.sidecar.files[1].path, "b.rs");
    }

    #[test]
    fn artifact_backed_note_body_is_loaded_from_artifact() {
        let store_dir = tempfile::tempdir().expect("create shore dir");
        let artifact_path = "artifacts/notes/note-abc.json";
        let artifact_file = store_dir.path().join(artifact_path);
        std::fs::create_dir_all(artifact_file.parent().expect("artifact parent"))
            .expect("create artifact dir");
        std::fs::write(
            &artifact_file,
            r#"{"schema":"shore.note-body","version":1,"body":"Artifact body"}"#,
        )
        .expect("write artifact");

        let mut payload = test_payload_for("src/lib.rs", "note:with-artifact");
        payload.body = None;
        payload.body_artifact_path = Some(artifact_path.to_owned());
        payload.body_byte_size = Some(256);

        let entry = replay_note_entry(&payload, store_dir.path()).expect("entry builds");

        assert_eq!(entry.body.as_deref(), Some("Artifact body"));
    }

    #[test]
    fn artifact_body_path_must_stay_under_artifacts_notes() {
        let mut payload = test_payload_for("src/lib.rs", "note:escape");
        payload.body = None;
        payload.body_artifact_path = Some("../outside.json".to_owned());

        let error = replay_note_entry(&payload, Path::new(".shore/data"))
            .expect_err("path should be rejected");
        assert!(error.to_string().contains("Invalid artifact path"));
    }

    #[test]
    fn artifact_body_schema_must_match() {
        let store_dir = tempfile::tempdir().expect("create shore dir");
        let artifact_path = "artifacts/notes/note-abc.json";
        let artifact_file = store_dir.path().join(artifact_path);
        std::fs::create_dir_all(artifact_file.parent().expect("artifact parent"))
            .expect("create artifact dir");
        std::fs::write(
            &artifact_file,
            r#"{"schema":"wrong.schema","version":2,"body":"Artifact body"}"#,
        )
        .expect("write artifact");

        let mut payload = test_payload_for("src/lib.rs", "note:wrong-schema");
        payload.body = None;
        payload.body_artifact_path = Some(artifact_path.to_owned());

        let error =
            replay_note_entry(&payload, store_dir.path()).expect_err("schema should be rejected");
        assert!(error.to_string().contains("Unsupported note body artifact"));
    }

    #[test]
    fn load_or_rebuild_session_state_returns_none_when_shore_dir_absent() {
        let repo = tempfile::tempdir().expect("create repo");
        Command::new("git")
            .args(["init"])
            .current_dir(repo.path())
            .output()
            .unwrap();

        let state = load_or_rebuild_session_state(repo.path()).unwrap();

        assert!(state.is_none());
    }

    #[test]
    fn load_or_rebuild_session_state_rebuilds_from_events_when_store_present() {
        let repo = test_repo_with(vec![review_initialized()]);

        let state = load_or_rebuild_session_state(repo.path()).unwrap();
        let state = state.expect("state should be present when the resolved store exists");

        assert_eq!(state.event_count, 1);
    }

    fn test_repo_with(events: Vec<ShoreEvent>) -> tempfile::TempDir {
        let repo = tempfile::tempdir().expect("create repo");
        Command::new("git")
            .args(["init"])
            .current_dir(repo.path())
            .output()
            .unwrap();
        // Write to the resolved store (the shared common-dir store), where the
        // read surfaces look — not the raw worktree-local `.shore/data`.
        let store_dir = resolve_read_store(repo.path())
            .unwrap()
            .store_dir()
            .to_path_buf();
        std::fs::create_dir_all(store_dir.join("events")).unwrap();
        let store = EventStore::open(&store_dir);
        for event in events {
            store.record_event_once(&event).unwrap();
        }
        repo
    }

    fn review_initialized() -> ShoreEvent {
        ShoreEvent::new(
            EventType::ReviewInitialized,
            "review_initialized:session:default:work:default",
            EventTarget::new(
                SessionId::new("session:default"),
                WorkUnitId::new("work:default"),
            ),
            Writer::shore_local("0.1.0"),
            ReviewInitializedPayload {},
            "2026-05-10T00:00:00Z",
        )
        .unwrap()
    }
}
