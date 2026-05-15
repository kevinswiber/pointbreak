use serde_json::Value;
use shore::dump::{DumpDocument, DumpInputSource, DumpInputSummary};
use shore::model::{
    Anchor, DiffFile, DiffRow, DiffRowKind, DiffSnapshot, FileId, FileStatus, HunkId, LineRange,
    ResolutionStatus, ReviewHunk, ReviewId, ReviewNote, ReviewNoteId, ReviewNoteSource, ReviewRow,
    ReviewRowKind, ReviewStream, RowId, Side, SnapshotId,
};
use shore::sidecar::{
    DiagnosticLevel, ParsedReviewNotes, ReviewNoteEntry, ReviewNoteTarget, ReviewNotesDiagnostic,
    ReviewNotesDiagnosticCode, ReviewNotesFile, ReviewNotesSidecar,
};

use crate::support::git_repo::GitRepo;

#[test]
fn dump_document_serializes_summary_diagnostics_and_stream_rows() {
    let snapshot = snapshot_with_one_hunk();
    let hunk = &snapshot.files[0].hunks[0];
    let note = ReviewNote {
        id: ReviewNoteId::new("note:demo"),
        anchor: Anchor {
            file_id: FileId::new("src/lib.rs"),
            side: Side::New,
            line_range: LineRange::new(1, 1),
            hunk_signature: hunk.signature(),
            target_text_hash: "sha256:demo".to_owned(),
            status: ResolutionStatus::Exact,
        },
        source: ReviewNoteSource::Sidecar,
        title: "Demo note".to_owned(),
        body: Some("Details".to_owned()),
        tags: vec!["demo".to_owned()],
        confidence: Some("high".to_owned()),
        external_source: Some("reviewer".to_owned()),
        author: Some("human reviewer".to_owned()),
        created_at: Some("2026-05-09T03:16:51Z".to_owned()),
    };
    let stream =
        ReviewStream::from_snapshot_with_resolved_notes(&snapshot, std::slice::from_ref(&note));
    let diagnostic = ReviewNotesDiagnostic {
        level: DiagnosticLevel::Warning,
        code: ReviewNotesDiagnosticCode::MissingNoteTitle,
        path: "files[0].notes[0].title".to_owned(),
        message: "review note is missing title".to_owned(),
    };

    let document = DumpDocument::new(
        DumpInputSummary {
            source: DumpInputSource::ReviewNotes,
        },
        snapshot,
        vec![note],
        stream,
        vec![diagnostic],
    );

    let json = serde_json::to_value(&document).expect("dump document serializes");

    assert_eq!(json["schema"], "shore.dump");
    assert_eq!(json["version"], 1);
    assert_eq!(json["input"]["source"], "review_notes");
    assert_eq!(json["summary"]["file_count"], 1);
    assert_eq!(json["summary"]["hunk_count"], 1);
    assert_eq!(json["summary"]["row_count"], 4);
    assert_eq!(json["summary"]["note_count"], 1);
    assert_eq!(json["summary"]["diagnostic_count"], 1);
    assert_eq!(json["diagnostics"][0]["level"], "warning");
    assert_eq!(json["diagnostics"][0]["code"], "missing_note_title");
    assert_eq!(json["diagnostics"][0]["path"], "files[0].notes[0].title");
    assert_eq!(
        json["diagnostics"][0]["message"],
        "review note is missing title"
    );
    assert_eq!(
        json["stream"]["rows"]
            .as_array()
            .expect("rows are array")
            .len(),
        4
    );
}

#[test]
fn dump_document_stream_row_kind_is_externally_tagged() {
    let snapshot = snapshot_with_one_hunk();
    let hunk = &snapshot.files[0].hunks[0];
    let note = ReviewNote {
        id: ReviewNoteId::new("note:demo"),
        anchor: Anchor {
            file_id: FileId::new("src/lib.rs"),
            side: Side::New,
            line_range: LineRange::new(1, 1),
            hunk_signature: hunk.signature(),
            target_text_hash: "sha256:demo".to_owned(),
            status: ResolutionStatus::Exact,
        },
        source: ReviewNoteSource::Sidecar,
        title: "Demo note".to_owned(),
        body: None,
        tags: Vec::new(),
        confidence: None,
        external_source: None,
        author: None,
        created_at: None,
    };
    let stream =
        ReviewStream::from_snapshot_with_resolved_notes(&snapshot, std::slice::from_ref(&note));

    let document = DumpDocument::new(
        DumpInputSummary {
            source: DumpInputSource::ReviewNotes,
        },
        snapshot,
        vec![note],
        stream,
        Vec::new(),
    );
    let json = serde_json::to_value(&document).expect("dump document serializes");

    let rows = json["stream"]["rows"].as_array().expect("rows are array");

    for (idx, row) in rows.iter().enumerate() {
        let kind = row["kind"]
            .as_object()
            .unwrap_or_else(|| panic!("row {idx} kind must be an object, got {row:#?}"));
        assert_eq!(
            kind.len(),
            1,
            "row {idx} kind must have exactly one variant tag, got {kind:#?}"
        );
    }

    let file_header_row = rows
        .iter()
        .find(|row| {
            row["kind"]
                .as_object()
                .is_some_and(|k| k.contains_key("file_header"))
        })
        .expect("file_header row present");
    let file_header = &file_header_row["kind"]["file_header"];
    assert_object_keys(file_header, &["path", "status"]);
    assert_eq!(file_header["path"], "src/lib.rs");
    assert_eq!(file_header["status"], "modified");

    let note_row = rows
        .iter()
        .find(|row| {
            row["kind"]
                .as_object()
                .is_some_and(|k| k.contains_key("note"))
        })
        .expect("note row present");
    let note_kind = &note_row["kind"]["note"];
    assert_object_keys(note_kind, &["note_id", "target_row_id", "title"]);
    assert_eq!(note_kind["note_id"], "note:demo");
    assert_eq!(note_kind["title"], "Demo note");
    assert!(
        note_kind["target_row_id"].is_string(),
        "target_row_id is a row id string"
    );
}

#[test]
fn dump_document_pins_stale_note_kind_envelope() {
    let review_id = ReviewId::new("review:test");
    let snapshot = DiffSnapshot::empty(review_id.clone());

    let stale_row = ReviewRow {
        id: RowId::new("row:stale"),
        ordinal: 0,
        file_id: Some(FileId::new("src/lib.rs")),
        hunk_id: None,
        kind: ReviewRowKind::StaleNote {
            note_id: ReviewNoteId::new("note:stale"),
            title: "Stale review note".to_owned(),
            resolution_status: ResolutionStatus::Stale,
            target_path: "src/lib.rs".to_owned(),
            target_line_range: LineRange::new(99, 99),
        },
    };
    let orphan_row = ReviewRow {
        id: RowId::new("row:orphan"),
        ordinal: 1,
        file_id: None,
        hunk_id: None,
        kind: ReviewRowKind::StaleNote {
            note_id: ReviewNoteId::new("note:orphan"),
            title: "Orphan review note".to_owned(),
            resolution_status: ResolutionStatus::Orphaned,
            target_path: "src/gone.rs".to_owned(),
            target_line_range: LineRange::new(1, 3),
        },
    };
    let stream = ReviewStream {
        review_id: review_id.clone(),
        snapshot_id: snapshot.snapshot_id.clone(),
        rows: vec![stale_row, orphan_row],
    };

    let document = DumpDocument::new(
        DumpInputSummary {
            source: DumpInputSource::ReviewNotes,
        },
        snapshot,
        Vec::new(),
        stream,
        Vec::new(),
    );
    let json = serde_json::to_value(&document).expect("dump document serializes");

    let rows = json["stream"]["rows"].as_array().expect("rows are array");
    assert_eq!(rows.len(), 2, "expected exactly two rows: {json:#?}");

    for row in rows {
        assert_eq!(
            row["kind"].as_object().expect("kind is object").len(),
            1,
            "row kind must have exactly one variant tag: {row:#?}"
        );
    }

    let stale_kinds: Vec<&serde_json::Value> = rows
        .iter()
        .filter_map(|row| row["kind"].as_object()?.get("stale_note"))
        .collect();
    assert_eq!(stale_kinds.len(), 2);

    let stale_value = stale_kinds
        .iter()
        .find(|v| v["resolution_status"] == "stale")
        .expect("stale row present");
    assert_object_keys(
        stale_value,
        &[
            "note_id",
            "title",
            "resolution_status",
            "target_path",
            "target_line_range",
        ],
    );
    assert_eq!(stale_value["note_id"], "note:stale");
    assert_eq!(stale_value["title"], "Stale review note");
    assert_eq!(stale_value["target_path"], "src/lib.rs");
    assert_eq!(stale_value["target_line_range"]["start"], 99);
    assert_eq!(stale_value["target_line_range"]["end"], 99);

    let orphan_value = stale_kinds
        .iter()
        .find(|v| v["resolution_status"] == "orphaned")
        .expect("orphan row present");
    assert_object_keys(
        orphan_value,
        &[
            "note_id",
            "title",
            "resolution_status",
            "target_path",
            "target_line_range",
        ],
    );
    assert_eq!(orphan_value["note_id"], "note:orphan");
    assert_eq!(orphan_value["target_path"], "src/gone.rs");
    assert_eq!(orphan_value["target_line_range"]["start"], 1);
    assert_eq!(orphan_value["target_line_range"]["end"], 3);
}

fn assert_object_keys(value: &serde_json::Value, expected: &[&str]) {
    let actual: std::collections::BTreeSet<&str> = value
        .as_object()
        .expect("value is object")
        .keys()
        .map(String::as_str)
        .collect();
    let expected: std::collections::BTreeSet<&str> = expected.iter().copied().collect();
    assert_eq!(
        actual, expected,
        "JSON object keys differ from expected contract"
    );
}

#[test]
fn dump_input_source_serializes_as_snake_case() {
    assert_eq!(
        input_source_value(DumpInputSource::None),
        Value::String("none".to_owned())
    );
    assert_eq!(
        input_source_value(DumpInputSource::ReviewNotes),
        Value::String("review_notes".to_owned())
    );
}

#[test]
fn dump_from_repo_builds_stream_without_notes() {
    let repo = dump_repo();

    let document = DumpDocument::from_repo(repo.path()).expect("repo-only dump builds");

    assert_eq!(document.input.source, DumpInputSource::None);
    assert_eq!(document.summary.file_count, 2);
    assert_eq!(document.summary.note_count, 0);
    assert_eq!(document.summary.diagnostic_count, 0);
    assert!(document.summary.row_count > 0);
    assert!(document.notes.is_empty());
}

#[test]
fn dump_from_parsed_review_notes_orders_files_and_resolves_notes() {
    let repo = dump_repo();
    let parsed = ParsedReviewNotes {
        sidecar: dump_review_notes_sidecar(),
        diagnostics: Vec::new(),
    };

    let document =
        DumpDocument::from_parsed_review_notes(repo.path(), parsed).expect("dump builds");

    assert_eq!(document.input.source, DumpInputSource::ReviewNotes);
    assert_eq!(document.summary.file_count, 2);
    assert_eq!(document.summary.note_count, 1);
    assert_eq!(document.summary.diagnostic_count, 0);
    assert_eq!(
        snapshot_paths(&document.snapshot),
        vec!["src/untracked.rs", "src/lib.rs"]
    );
    assert_eq!(
        file_header_paths(&document.stream),
        vec!["src/untracked.rs", "src/lib.rs"]
    );
    assert!(
        document
            .stream
            .rows
            .iter()
            .any(|row| { matches!(row.kind, ReviewRowKind::Note { .. }) })
    );
}

#[test]
fn dump_from_parsed_review_notes_preserves_parser_diagnostics() {
    let repo = dump_repo();
    let parsed = ParsedReviewNotes {
        sidecar: ReviewNotesSidecar {
            schema: Some("shore.review-notes".to_owned()),
            version: 1,
            summary: None,
            files: Vec::new(),
        },
        diagnostics: vec![ReviewNotesDiagnostic {
            level: DiagnosticLevel::Warning,
            code: ReviewNotesDiagnosticCode::MissingVersion,
            path: "version".to_owned(),
            message: "review notes sidecar is missing version".to_owned(),
        }],
    };

    let document =
        DumpDocument::from_parsed_review_notes(repo.path(), parsed).expect("dump builds");

    assert_eq!(document.summary.diagnostic_count, 1);
    assert_eq!(
        document.diagnostics[0].code,
        ReviewNotesDiagnosticCode::MissingVersion
    );
}

fn input_source_value(source: DumpInputSource) -> Value {
    serde_json::to_value(DumpInputSummary { source })
        .expect("input summary serializes")
        .get("source")
        .expect("source field exists")
        .clone()
}

fn dump_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.write("src/untracked.rs", "pub fn untracked() -> u32 { 3 }\n");
    repo
}

fn dump_review_notes_sidecar() -> ReviewNotesSidecar {
    ReviewNotesSidecar {
        schema: Some("shore.review-notes".to_owned()),
        version: 1,
        summary: None,
        files: vec![
            ReviewNotesFile {
                path: "src/untracked.rs".to_owned(),
                old_path: None,
                summary: None,
                notes: vec![ReviewNoteEntry {
                    id: Some("note:untracked".to_owned()),
                    title: Some("Untracked note".to_owned()),
                    body: None,
                    target: Some(ReviewNoteTarget {
                        side: Side::New,
                        start_line: 1,
                        end_line: 1,
                    }),
                    tags: Vec::new(),
                    confidence: None,
                    source: None,
                    author: None,
                    created_at: None,
                }],
            },
            ReviewNotesFile {
                path: "src/lib.rs".to_owned(),
                old_path: None,
                summary: None,
                notes: Vec::new(),
            },
        ],
    }
}

fn snapshot_paths(snapshot: &DiffSnapshot) -> Vec<&str> {
    snapshot
        .files
        .iter()
        .filter_map(|file| file.new_path.as_deref().or(file.old_path.as_deref()))
        .collect()
}

fn file_header_paths(stream: &ReviewStream) -> Vec<&str> {
    stream
        .rows
        .iter()
        .filter_map(|row| match &row.kind {
            ReviewRowKind::FileHeader { path, .. } => Some(path.as_str()),
            _ => None,
        })
        .collect()
}

fn snapshot_with_one_hunk() -> DiffSnapshot {
    let hunk = ReviewHunk {
        id: HunkId::new("hunk:1"),
        header: "@@ -1 +1 @@".to_owned(),
        old_start: 1,
        old_lines: 1,
        new_start: 1,
        new_lines: 1,
        rows: vec![DiffRow {
            kind: DiffRowKind::Added,
            old_line: None,
            new_line: Some(1),
            text: "pub fn demo() {}".to_owned(),
        }],
    };

    DiffSnapshot::new(
        ReviewId::new("review:test"),
        SnapshotId::new("snapshot:test"),
        vec![DiffFile {
            id: FileId::new("src/lib.rs"),
            status: FileStatus::Modified,
            old_path: Some("src/lib.rs".to_owned()),
            new_path: Some("src/lib.rs".to_owned()),
            old_mode: None,
            new_mode: None,
            old_oid: None,
            new_oid: None,
            similarity: None,
            is_binary: false,
            is_submodule: false,
            is_mode_only: false,
            synthetic: false,
            metadata_rows: Vec::new(),
            hunks: vec![hunk],
        }],
    )
}

#[test]
fn dump_input_source_durable_serializes_as_snake_case() {
    assert_eq!(
        serde_json::to_string(&DumpInputSource::Durable).unwrap(),
        "\"durable\""
    );
}

#[test]
fn from_repo_with_options_uses_durable_notes_when_present() {
    use shore::session::{ImportNotesOptions, import_notes};

    use crate::support::git_repo::GitRepo;

    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");

    let sidecar = repo.path().join("review-notes.json");
    std::fs::write(
        &sidecar,
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
}"#,
    )
    .unwrap();

    import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(&sidecar))
        .expect("notes import succeeds");

    let document = DumpDocument::from_repo_with_options(repo.path(), Default::default())
        .expect("document builds");

    assert_eq!(document.input.source, DumpInputSource::Durable);
    assert_eq!(document.summary.note_count, 1);
}

#[test]
fn from_repo_with_options_durable_preserves_snapshot_file_order() {
    use shore::session::{ImportNotesOptions, import_notes};

    use crate::support::git_repo::GitRepo;

    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.write("tests/test.rs", "pub fn test() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    repo.write("tests/test.rs", "pub fn test() -> u32 { 2 }\n");

    let sidecar = repo.path().join("review-notes.json");
    std::fs::write(
        &sidecar,
        r#"{
  "schema": "shore.review-notes",
  "version": 1,
  "files": [
    {
      "path": "tests/test.rs",
      "notes": [
        {
          "title": "Test change",
          "target": { "side": "new", "startLine": 1, "endLine": 1 }
        }
      ]
    },
    {
      "path": "src/lib.rs",
      "notes": []
    }
  ]
}"#,
    )
    .unwrap();

    import_notes(ImportNotesOptions::new(repo.path()).with_review_notes(&sidecar))
        .expect("notes import succeeds");

    let document = DumpDocument::from_repo_with_options(repo.path(), Default::default())
        .expect("document builds");

    assert_eq!(document.input.source, DumpInputSource::Durable);
    let modified_files: Vec<_> = document
        .snapshot
        .files
        .iter()
        .filter_map(|f| f.new_path.as_deref())
        .filter(|p| p.contains("src/") || p.contains("tests/"))
        .collect();
    assert_eq!(modified_files, vec!["src/lib.rs", "tests/test.rs"]);
}
