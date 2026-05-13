use std::path::{Path, PathBuf};

use serde_json::json;

use crate::canonical_hash::{sha256_bytes_hex, sha256_json_prefixed};
use crate::error::{Result, ShoreError};
use crate::model::WorkUnitId;
use crate::session::body_artifact::{BodyArtifactOutcome, stage_body_artifact};
use crate::session::event::{
    EventTarget, EventType, ImportedNoteTarget, ReviewInitializedPayload,
    ReviewNoteImportedPayload, ShoreEvent, SidecarSource,
};
use crate::session::{
    ProjectionDiagnostic, SessionState, ShoreStorePaths, current_timestamp, prepare_shore_writer,
    writer_from_git_config,
};
use crate::sidecar::{ReviewNoteEntry, ReviewNoteTarget, ReviewNotesFile, ReviewNotesSidecar};
use crate::storage::{Durability, EventStore, EventWriteOutcome, LocalStorage};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NoteImportRecord {
    pub(crate) idempotency_key: String,
    pub(crate) payload: ReviewNoteImportedPayload,
    pub(crate) body_artifact_bytes: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportNotesOptions {
    repo: PathBuf,
    review_notes: Option<PathBuf>,
}

impl ImportNotesOptions {
    pub fn new(repo: impl AsRef<Path>) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            review_notes: None,
        }
    }

    pub fn with_review_notes(mut self, path: impl AsRef<Path>) -> Self {
        self.review_notes = Some(path.as_ref().to_path_buf());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportNotesResult {
    pub note_count: usize,
    pub notes_created: usize,
    pub notes_existing: usize,
    pub state_path: PathBuf,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

pub fn import_notes(options: ImportNotesOptions) -> Result<ImportNotesResult> {
    let paths = ShoreStorePaths::resolve(&options.repo)?;
    let worktree_root = paths.worktree_root();
    let (sidecar_source, sidecar_content_hash, sidecar) = parsed_sidecar_input(&options)?;

    let shore_dir = paths.shore_dir();
    let storage = LocalStorage::new(shore_dir);
    prepare_shore_writer(&paths, &storage)?;

    let event_store = EventStore::open(shore_dir);
    let existing_state = SessionState::from_events(&event_store.list_events()?)?;

    let review_id = existing_state.review_id.clone();
    let work_unit_id = existing_state.work_unit_id.clone();
    let target = EventTarget::new(review_id.clone(), work_unit_id.clone());
    let writer = writer_from_git_config(worktree_root);
    let occurred_at = current_timestamp();

    match event_store.record_event_once(&ShoreEvent::new(
        EventType::ReviewInitialized,
        ReviewInitializedPayload::idempotency_key(&review_id, &work_unit_id),
        target.clone(),
        writer.clone(),
        ReviewInitializedPayload {},
        occurred_at.clone(),
    )?)? {
        EventWriteOutcome::Created | EventWriteOutcome::Existing => {}
    }

    let records = extract_note_import_records(
        &sidecar,
        sidecar_source,
        &work_unit_id,
        &sidecar_content_hash,
    )?;

    let mut notes_created = 0;
    let mut notes_existing = 0;
    for record in records {
        if !event_store.event_exists(&record.idempotency_key)?
            && let (Some(artifact_path), Some(bytes)) = (
                record.payload.body_artifact_path.as_ref(),
                record.body_artifact_bytes.as_ref(),
            )
        {
            storage.write_bytes_atomic(Path::new(artifact_path), bytes, Durability::Durable)?;
        }

        let event = ShoreEvent::new(
            EventType::ReviewNoteImported,
            record.idempotency_key,
            target.clone(),
            writer.clone(),
            record.payload,
            occurred_at.clone(),
        )?;

        match event_store.record_event_once(&event)? {
            EventWriteOutcome::Created => notes_created += 1,
            EventWriteOutcome::Existing => notes_existing += 1,
        }
    }

    let state = SessionState::from_events(&event_store.list_events()?)?;
    let state_path = paths.state_path();
    storage.write_json_atomic(&state_path, &state, Durability::Projection)?;

    Ok(ImportNotesResult {
        note_count: state.note_count,
        notes_created,
        notes_existing,
        state_path,
        diagnostics: state.diagnostics,
    })
}

pub(crate) fn extract_note_import_records(
    sidecar: &ReviewNotesSidecar,
    sidecar_source: SidecarSource,
    work_unit_id: &WorkUnitId,
    sidecar_content_hash: &str,
) -> Result<Vec<NoteImportRecord>> {
    let mut records = Vec::new();

    for file in &sidecar.files {
        for note in &file.notes {
            let note_id = stable_note_id(file, note)?;
            let (body, body_artifact_path, body_artifact_bytes, body_byte_size) = match note
                .body
                .as_deref()
            {
                Some(body) => match stage_body_artifact(body.as_bytes())? {
                    BodyArtifactOutcome::Inline { .. } => (Some(body.to_owned()), None, None, None),
                    BodyArtifactOutcome::Artifact {
                        relative_path,
                        byte_size,
                        body_envelope,
                    } => (
                        None,
                        Some(relative_path),
                        Some(body_envelope.to_json_bytes()?),
                        Some(byte_size as usize),
                    ),
                },
                None => (None, None, None, None),
            };

            let payload = ReviewNoteImportedPayload {
                sidecar_source,
                note_id: note_id.clone(),
                file_path: file.path.clone(),
                file_old_path: file.old_path.clone(),
                target: note.target.map(imported_note_target),
                title: note.title.clone().unwrap_or_default(),
                body,
                body_artifact_path,
                body_byte_size,
                tags: note.tags.clone(),
                confidence: note.confidence.clone(),
                external_source: note.source.clone(),
                author: note.author.clone(),
                created_at: note.created_at.clone(),
                sidecar_content_hash: sidecar_content_hash.to_owned(),
            };

            records.push(NoteImportRecord {
                idempotency_key: format!(
                    "review_note_imported:{}:{}:{}",
                    sidecar_source_key(sidecar_source),
                    work_unit_id.as_str(),
                    note_id
                ),
                payload,
                body_artifact_bytes,
            });
        }
    }

    Ok(records)
}

fn stable_note_id(file: &ReviewNotesFile, note: &ReviewNoteEntry) -> Result<String> {
    if let Some(explicit_id) = note.id.as_deref().filter(|id| !id.trim().is_empty()) {
        return Ok(format!("note:{explicit_id}"));
    }

    let content_hash = sha256_json_prefixed(&json!({
        "filePath": file.path,
        "oldPath": file.old_path,
        "side": note.target.map(|target| target.side),
        "startLine": note.target.map(|target| target.start_line),
        "endLine": note.target.map(|target| target.end_line),
        "title": note.title.clone().unwrap_or_default(),
        "body": note.body,
        "tags": note.tags,
    }))?;

    Ok(format!("note:{content_hash}"))
}

fn imported_note_target(target: ReviewNoteTarget) -> ImportedNoteTarget {
    ImportedNoteTarget {
        side: target.side,
        start_line: target.start_line,
        end_line: target.end_line,
    }
}

fn sidecar_source_key(source: SidecarSource) -> &'static str {
    match source {
        SidecarSource::ReviewNotes => "review_notes",
    }
}

type ParsedSidecarInput = (SidecarSource, String, ReviewNotesSidecar);

fn parsed_sidecar_input(options: &ImportNotesOptions) -> Result<ParsedSidecarInput> {
    if let Some(path) = &options.review_notes {
        let input = crate::sidecar::read_review_notes_sidecar_file(path)?;
        let parsed = crate::sidecar::parse_review_notes_sidecar(&input.text)?;
        return Ok((
            SidecarSource::ReviewNotes,
            format!("sha256:{}", sha256_bytes_hex(&input.bytes)),
            parsed.sidecar,
        ));
    }

    Err(ShoreError::Message(
        "exactly one review-notes input must be supplied".to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::process::Command;

    use tempfile::TempDir;

    use super::*;
    use crate::model::Side;
    use crate::session::body_artifact::BODY_INLINE_LIMIT;
    use crate::sidecar::{ReviewNotesFile, ReviewNotesSidecar};

    #[test]
    fn extracted_note_records_use_explicit_id_when_present() {
        let sidecar = sidecar_with_notes(vec![note_with_id("my-id", "Same title", Some("Body"))]);

        let records = extract_note_import_records(
            &sidecar,
            SidecarSource::ReviewNotes,
            &WorkUnitId::new("work:default"),
            "sha256:sidecar",
        )
        .expect("records extract");

        assert_eq!(
            records[0].idempotency_key,
            "review_note_imported:review_notes:work:default:note:my-id"
        );
        assert_eq!(records[0].payload.note_id, "note:my-id");
    }

    #[test]
    fn extracted_note_records_hash_identity_when_id_is_missing() {
        let sidecar = sidecar_with_notes(vec![note_without_id("Same title", Some("Body"))]);

        let records = extract_note_import_records(
            &sidecar,
            SidecarSource::ReviewNotes,
            &WorkUnitId::new("work:default"),
            "sha256:sidecar",
        )
        .expect("records extract");

        assert!(
            records[0]
                .idempotency_key
                .starts_with("review_note_imported:review_notes:work:default:note:sha256:")
        );
        assert!(records[0].payload.note_id.starts_with("note:sha256:"));
    }

    #[test]
    fn extracted_note_records_are_stable_for_identical_notes() {
        let sidecar = sidecar_with_notes(vec![
            note_without_id("Same title", Some("Body")),
            note_without_id("Same title", Some("Body")),
        ]);

        let records = extract_note_import_records(
            &sidecar,
            SidecarSource::ReviewNotes,
            &WorkUnitId::new("work:default"),
            "sha256:sidecar",
        )
        .expect("records extract");

        assert_eq!(records[0].idempotency_key, records[1].idempotency_key);
    }

    #[test]
    fn extracted_note_records_differ_for_different_notes() {
        let sidecar = sidecar_with_notes(vec![
            note_without_id("Same title", Some("Body")),
            note_without_id("Different title", Some("Body")),
        ]);

        let records = extract_note_import_records(
            &sidecar,
            SidecarSource::ReviewNotes,
            &WorkUnitId::new("work:default"),
            "sha256:sidecar",
        )
        .expect("records extract");

        assert_ne!(records[0].idempotency_key, records[1].idempotency_key);
    }

    #[test]
    fn extracted_note_records_externalize_large_bodies() {
        let large_body = "x".repeat(BODY_INLINE_LIMIT + 1);
        let sidecar = sidecar_with_notes(vec![note_without_id("Same title", Some(&large_body))]);

        let records = extract_note_import_records(
            &sidecar,
            SidecarSource::ReviewNotes,
            &WorkUnitId::new("work:default"),
            "sha256:sidecar",
        )
        .expect("records extract");

        assert_eq!(records[0].payload.body, None);
        assert!(
            records[0]
                .payload
                .body_artifact_path
                .as_deref()
                .unwrap()
                .starts_with("artifacts/notes/")
        );
        assert!(records[0].body_artifact_bytes.is_some());
        assert_eq!(records[0].payload.body_byte_size, Some(large_body.len()));
    }

    #[test]
    fn import_notes_missing_file_fails_before_shore_writes() {
        let repo = TestRepo::new();
        let missing = repo.path().join("missing-review-notes.json");

        let error = import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(&missing))
            .expect_err("missing file fails");

        assert!(error.to_string().contains("missing-review-notes.json"));
        assert!(!repo.path().join(".shore").exists());
    }

    #[test]
    fn import_notes_writes_events_and_state() {
        let repo = TestRepo::new();
        let sidecar = repo.path().join("review-notes.json");
        fs::write(&sidecar, native_review_notes_json()).expect("write sidecar");

        let result = import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(&sidecar))
            .expect("import succeeds");

        assert!(repo.path().join(".shore/events").is_dir());
        assert!(repo.path().join(".shore/state.json").is_file());
        assert!(result.note_count > 0);
        assert_eq!(result.notes_created, 1);
        let state: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(repo.path().join(".shore/state.json")).unwrap(),
        )
        .expect("state json");
        assert_eq!(state["noteCount"], 1);
        assert_eq!(state["eventCount"], 2);
    }

    fn sidecar_with_notes(notes: Vec<ReviewNoteEntry>) -> ReviewNotesSidecar {
        ReviewNotesSidecar {
            schema: Some("shore.review-notes".to_owned()),
            version: 1,
            summary: None,
            files: vec![ReviewNotesFile {
                path: "src/lib.rs".to_owned(),
                old_path: None,
                summary: None,
                notes,
            }],
        }
    }

    fn note_with_id(id: &str, title: &str, body: Option<&str>) -> ReviewNoteEntry {
        ReviewNoteEntry {
            id: Some(id.to_owned()),
            title: Some(title.to_owned()),
            body: body.map(str::to_owned),
            target: Some(ReviewNoteTarget {
                side: Side::New,
                start_line: 1,
                end_line: 1,
            }),
            tags: vec!["parser".to_owned()],
            confidence: Some("high".to_owned()),
            source: Some("external".to_owned()),
            author: Some("reviewer".to_owned()),
            created_at: Some("2026-05-10T00:00:00Z".to_owned()),
        }
    }

    fn note_without_id(title: &str, body: Option<&str>) -> ReviewNoteEntry {
        ReviewNoteEntry {
            id: None,
            ..note_with_id("ignored", title, body)
        }
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
          "id": "note-1",
          "title": "Imported note",
          "body": "Body",
          "target": { "side": "new", "startLine": 1, "endLine": 1 }
        }
      ]
    }
  ]
}"#
    }

    struct TestRepo {
        root: TempDir,
    }

    impl TestRepo {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("tempdir");
            run_git(root.path(), ["init"]);
            run_git(
                root.path(),
                ["config", "user.email", "shore-tests@example.com"],
            );
            run_git(root.path(), ["config", "user.name", "Shore Tests"]);
            fs::create_dir_all(root.path().join("src")).expect("create src dir");
            fs::write(
                root.path().join("src/lib.rs"),
                "pub fn value() -> u32 { 1 }\n",
            )
            .expect("write source");
            Self { root }
        }

        fn path(&self) -> &Path {
            self.root.path()
        }
    }

    fn run_git<const N: usize>(repo: &Path, args: [&str; N]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(repo)
            .status()
            .expect("run git");
        assert!(status.success(), "git {:?} failed", args);
    }
}
