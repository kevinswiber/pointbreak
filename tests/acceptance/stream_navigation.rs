use shoreline::model::{
    Anchor, DiffFile, DiffRow, DiffRowKind, DiffSnapshot, FileId, FileMetadataKind,
    FileMetadataRow, FileStatus, HunkId, LineRange, ObjectId, ResolutionStatus, ReviewHunk,
    ReviewId, ReviewNote, ReviewNoteId, ReviewNoteSource, ReviewRow, ReviewRowKind, ReviewStream,
    RowId, Side,
};
use shoreline::sidecar::{
    DiagnosticLevel, ReviewNotesDiagnosticCode, ReviewNotesFile, ReviewNotesSidecar,
};
use shoreline::stream::{NavigationCommand, RevealTarget};

#[test]
fn review_stream_emits_deterministic_rows_for_diff_metadata_and_notes() {
    let hunk = text_hunk();
    let note = ReviewNote {
        id: ReviewNoteId::new("note-added"),
        anchor: Anchor {
            file_id: FileId::new("src/lib.rs"),
            side: Side::New,
            line_range: LineRange::new(2, 2),
            hunk_signature: hunk.signature(),
            target_text_hash: "sha256:target".to_owned(),
            status: ResolutionStatus::Exact,
        },
        source: ReviewNoteSource::Sidecar,
        title: "explain added call".to_owned(),
        body: None,
        tags: Vec::new(),
        confidence: None,
        external_source: None,
        author: None,
        created_at: None,
    };
    let snapshot = DiffSnapshot::new(
        ReviewId::new("review-1"),
        ObjectId::new("snapshot-1"),
        vec![text_file(hunk), metadata_file()],
    );

    let stream = ReviewStream::from_snapshot_with_resolved_notes(&snapshot, &[note]);

    let row_ids = stream
        .rows
        .iter()
        .map(|row| row.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        row_ids,
        vec![
            "row:0000", "row:0001", "row:0002", "row:0003", "row:0004", "row:0005", "row:0006",
            "row:0007", "row:0008", "row:0009",
        ]
    );
    assert_eq!(
        stream
            .rows
            .iter()
            .map(|row| row.ordinal)
            .collect::<Vec<_>>(),
        (0..10).collect::<Vec<_>>()
    );

    assert!(matches!(
        &stream.rows[0].kind,
        ReviewRowKind::FileHeader {
            path,
            status: FileStatus::Modified
        } if path == "src/lib.rs"
    ));
    assert_eq!(stream.rows[0].file_id, Some(FileId::new("src/lib.rs")));
    assert_eq!(stream.rows[0].hunk_id, None);

    assert!(matches!(
        &stream.rows[1].kind,
        ReviewRowKind::HunkHeader { header } if header == "@@ -1,2 +1,2 @@"
    ));
    assert_eq!(stream.rows[1].hunk_id, Some(HunkId::new("src/lib.rs:1:1")));

    assert!(matches!(
        &stream.rows[2].kind,
        ReviewRowKind::Diff { row } if row.kind == DiffRowKind::Context && row.text == "fn main() {"
    ));
    assert!(matches!(
        &stream.rows[3].kind,
        ReviewRowKind::Diff { row } if row.kind == DiffRowKind::Removed && row.old_line == Some(2)
    ));
    assert!(matches!(
        &stream.rows[4].kind,
        ReviewRowKind::Diff { row } if row.kind == DiffRowKind::Added && row.new_line == Some(2)
    ));

    assert!(matches!(
        &stream.rows[5].kind,
        ReviewRowKind::Note {
            note_id,
            target_row_id,
            title,
        } if note_id == &ReviewNoteId::new("note-added")
            && target_row_id == &RowId::new("row:0004")
            && title == "explain added call"
    ));
    assert_eq!(stream.rows[5].file_id, Some(FileId::new("src/lib.rs")));
    assert_eq!(stream.rows[5].hunk_id, Some(HunkId::new("src/lib.rs:1:1")));
    for row in &stream.rows[2..=5] {
        assert_eq!(row.file_id, Some(FileId::new("src/lib.rs")));
        assert_eq!(row.hunk_id, Some(HunkId::new("src/lib.rs:1:1")));
    }

    assert!(matches!(
        &stream.rows[6].kind,
        ReviewRowKind::FileHeader {
            path,
            status: FileStatus::Modified
        } if path == "vendor/lib"
    ));
    assert!(matches!(
        &stream.rows[7].kind,
        ReviewRowKind::Metadata { metadata } if metadata.kind == FileMetadataKind::BinarySummary
    ));
    assert!(matches!(
        &stream.rows[8].kind,
        ReviewRowKind::Metadata { metadata } if metadata.kind == FileMetadataKind::ModeChange
    ));
    assert!(matches!(
        &stream.rows[9].kind,
        ReviewRowKind::Metadata { metadata } if metadata.kind == FileMetadataKind::SubmoduleSummary
    ));
    for row in &stream.rows[6..=9] {
        assert_eq!(row.file_id, Some(FileId::new("vendor/lib")));
        assert_eq!(row.hunk_id, None);
    }
}

#[test]
fn review_stream_emits_empty_state_when_snapshot_has_no_changes() {
    let snapshot = DiffSnapshot::new(
        ReviewId::new("review-1"),
        ObjectId::new("snapshot-1"),
        Vec::new(),
    );

    let stream = ReviewStream::from_snapshot_with_resolved_notes(&snapshot, &[]);

    assert_eq!(stream.review_id, ReviewId::new("review-1"));
    assert_eq!(stream.object_id, ObjectId::new("snapshot-1"));
    assert_eq!(stream.rows.len(), 1);
    assert_eq!(stream.rows[0].id, RowId::new("row:0000"));
    assert_eq!(stream.rows[0].ordinal, 0);
    assert_eq!(stream.rows[0].file_id, None);
    assert_eq!(stream.rows[0].hunk_id, None);
    assert!(matches!(
        &stream.rows[0].kind,
        ReviewRowKind::EmptyState { message } if message == "no changes"
    ));
}

#[test]
fn review_stream_from_review_notes_applies_order_and_dedupes_stale_path_diagnostics() {
    let snapshot = DiffSnapshot::new(
        ReviewId::new("review-1"),
        ObjectId::new("snapshot-1"),
        vec![modified_file("src/a.rs"), modified_file("src/b.rs")],
    );
    let sidecar = ReviewNotesSidecar {
        schema: Some("shore.review-notes".to_owned()),
        version: 1,
        summary: None,
        files: vec![sidecar_file("src/b.rs"), sidecar_file("src/stale.rs")],
    };

    let built = ReviewStream::from_snapshot_and_review_notes_sidecar(&snapshot, &sidecar);

    let file_headers = built
        .stream
        .rows
        .iter()
        .filter_map(|row| match &row.kind {
            ReviewRowKind::FileHeader { path, .. } => Some(path.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(file_headers, vec!["src/b.rs", "src/a.rs"]);
    assert_eq!(built.diagnostics.len(), 1);
    assert_eq!(built.diagnostics[0].level, DiagnosticLevel::Warning);
    assert_eq!(
        built.diagnostics[0].code,
        ReviewNotesDiagnosticCode::StaleFilePath
    );
    assert_eq!(built.diagnostics[0].path, "files[1].path");
}

#[test]
fn navigation_moves_between_full_hunk_headers_and_clamps_at_edges() {
    let stream = navigation_stream();

    let next = stream.navigate(
        &shoreline::model::CursorState::at_row(RowId::new("row:0001")),
        NavigationCommand::NextHunk,
    );
    assert_eq!(
        next.cursor,
        shoreline::model::CursorState::at_row(RowId::new("row:0005"))
    );
    assert_eq!(
        next.reveal,
        Some(RevealTarget::Row {
            row_id: RowId::new("row:0005")
        })
    );
    assert!(!next.clamped);

    let previous = stream.navigate(&next.cursor, NavigationCommand::PreviousHunk);
    assert_eq!(
        previous.cursor,
        shoreline::model::CursorState::at_row(RowId::new("row:0001"))
    );
    assert_eq!(
        previous.reveal,
        Some(RevealTarget::Row {
            row_id: RowId::new("row:0001")
        })
    );
    assert!(!previous.clamped);

    let clamped_previous = stream.navigate(&previous.cursor, NavigationCommand::PreviousHunk);
    assert_eq!(clamped_previous.cursor, previous.cursor);
    assert!(clamped_previous.clamped);

    let clamped_next = stream.navigate(
        &shoreline::model::CursorState::at_row(RowId::new("row:0012")),
        NavigationCommand::NextHunk,
    );
    assert_eq!(
        clamped_next.cursor,
        shoreline::model::CursorState::at_row(RowId::new("row:0012"))
    );
    assert!(clamped_next.clamped);
}

#[test]
fn note_navigation_targets_note_rows_from_unnoted_hunks() {
    let stream = navigation_stream();

    let next = stream.navigate(
        &shoreline::model::CursorState::at_row(RowId::new("row:0007")),
        NavigationCommand::NextNoteHunk,
    );
    assert_eq!(
        next.cursor,
        shoreline::model::CursorState::at_row(RowId::new("row:0011"))
    );
    assert_eq!(
        next.reveal,
        Some(RevealTarget::Row {
            row_id: RowId::new("row:0011")
        })
    );
    assert!(is_note_row(row_by_id(&stream, "row:0011")));
    assert!(!next.clamped);

    let previous = stream.navigate(
        &shoreline::model::CursorState::at_row(RowId::new("row:0007")),
        NavigationCommand::PreviousNoteHunk,
    );
    assert_eq!(
        previous.cursor,
        shoreline::model::CursorState::at_row(RowId::new("row:0004"))
    );
    assert!(is_note_row(row_by_id(&stream, "row:0004")));
    assert!(!previous.clamped);
}

#[test]
fn note_navigation_clamps_to_last_note_when_current_hunk_is_past_it() {
    let stream = navigation_stream();

    let before_first_note = stream.navigate(
        &shoreline::model::CursorState::at_row(RowId::new("row:0001")),
        NavigationCommand::PreviousNoteHunk,
    );
    assert_eq!(
        before_first_note.cursor,
        shoreline::model::CursorState::at_row(RowId::new("row:0004"))
    );
    assert!(before_first_note.clamped);

    let from_unnoted_tail = stream.navigate(
        &shoreline::model::CursorState::at_row(RowId::new("row:0012")),
        NavigationCommand::NextNoteHunk,
    );
    assert_eq!(
        from_unnoted_tail.cursor,
        shoreline::model::CursorState::at_row(RowId::new("row:0011"))
    );
    assert!(from_unnoted_tail.clamped);

    let from_last_note =
        stream.navigate(&from_unnoted_tail.cursor, NavigationCommand::NextNoteHunk);
    assert_eq!(from_last_note.cursor, from_unnoted_tail.cursor);
    assert_eq!(from_last_note.reveal, from_unnoted_tail.reveal);
    assert!(from_last_note.clamped);
}

#[test]
fn cursor_state_round_trips_through_json() {
    let cursor = shoreline::model::CursorState::at_row(RowId::new("row:0011"));

    let json = serde_json::to_string(&cursor).expect("cursor serializes");
    let decoded: shoreline::model::CursorState =
        serde_json::from_str(&json).expect("cursor deserializes");

    assert_eq!(decoded, cursor);
}

fn text_file(hunk: ReviewHunk) -> DiffFile {
    DiffFile {
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
    }
}

fn navigation_stream() -> ReviewStream {
    let first = navigation_hunk("src/lib.rs:10:10", 10, "first noted");
    let second = navigation_hunk("src/lib.rs:20:20", 20, "middle unnoted");
    let third = navigation_hunk("src/lib.rs:30:30", 30, "last noted");
    let fourth = navigation_hunk("src/lib.rs:40:40", 40, "tail unnoted");
    let notes = vec![
        hunk_note("note-first", &first, 11),
        hunk_note("note-last", &third, 31),
    ];
    let snapshot = DiffSnapshot::new(
        ReviewId::new("review-1"),
        ObjectId::new("snapshot-1"),
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
            hunks: vec![first, second, third, fourth],
        }],
    );

    ReviewStream::from_snapshot_with_resolved_notes(&snapshot, &notes)
}

fn navigation_hunk(id: &str, start: u32, added_text: &str) -> ReviewHunk {
    ReviewHunk {
        id: HunkId::new(id),
        header: format!("@@ -{start},1 +{start},2 @@"),
        old_start: start,
        old_lines: 1,
        new_start: start,
        new_lines: 2,
        rows: vec![
            DiffRow {
                kind: DiffRowKind::Context,
                old_line: Some(start),
                new_line: Some(start),
                text: format!("context {start}"),
            },
            DiffRow {
                kind: DiffRowKind::Added,
                old_line: None,
                new_line: Some(start + 1),
                text: added_text.to_owned(),
            },
        ],
    }
}

fn hunk_note(id: &str, hunk: &ReviewHunk, line: u32) -> ReviewNote {
    ReviewNote {
        id: ReviewNoteId::new(id),
        anchor: Anchor {
            file_id: FileId::new("src/lib.rs"),
            side: Side::New,
            line_range: LineRange::new(line, line),
            hunk_signature: hunk.signature(),
            target_text_hash: "sha256:target".to_owned(),
            status: ResolutionStatus::Exact,
        },
        source: ReviewNoteSource::Sidecar,
        title: id.to_owned(),
        body: None,
        tags: Vec::new(),
        confidence: None,
        external_source: None,
        author: None,
        created_at: None,
    }
}

fn row_by_id<'a>(stream: &'a ReviewStream, row_id: &str) -> &'a ReviewRow {
    stream
        .rows
        .iter()
        .find(|row| row.id == RowId::new(row_id))
        .expect("row exists")
}

fn is_note_row(row: &ReviewRow) -> bool {
    matches!(row.kind, ReviewRowKind::Note { .. })
}

fn modified_file(path: &str) -> DiffFile {
    DiffFile {
        id: FileId::new(path),
        status: FileStatus::Modified,
        old_path: Some(path.to_owned()),
        new_path: Some(path.to_owned()),
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
        hunks: Vec::new(),
    }
}

fn sidecar_file(path: &str) -> ReviewNotesFile {
    ReviewNotesFile {
        path: path.to_owned(),
        old_path: None,
        summary: None,
        notes: Vec::new(),
    }
}

fn metadata_file() -> DiffFile {
    DiffFile {
        id: FileId::new("vendor/lib"),
        status: FileStatus::Modified,
        old_path: Some("vendor/lib".to_owned()),
        new_path: Some("vendor/lib".to_owned()),
        old_mode: Some("160000".to_owned()),
        new_mode: Some("160000".to_owned()),
        old_oid: Some("old".to_owned()),
        new_oid: Some("new".to_owned()),
        similarity: None,
        is_binary: true,
        is_submodule: true,
        is_mode_only: true,
        synthetic: false,
        metadata_rows: vec![
            FileMetadataRow {
                kind: FileMetadataKind::BinarySummary,
                text: "binary files differ".to_owned(),
            },
            FileMetadataRow {
                kind: FileMetadataKind::ModeChange,
                text: "mode changed 100644 -> 100755".to_owned(),
            },
            FileMetadataRow {
                kind: FileMetadataKind::SubmoduleSummary,
                text: "submodule changed old -> new".to_owned(),
            },
        ],
        hunks: Vec::new(),
    }
}

fn text_hunk() -> ReviewHunk {
    ReviewHunk {
        id: HunkId::new("src/lib.rs:1:1"),
        header: "@@ -1,2 +1,2 @@".to_owned(),
        old_start: 1,
        old_lines: 2,
        new_start: 1,
        new_lines: 2,
        rows: vec![
            DiffRow {
                kind: DiffRowKind::Context,
                old_line: Some(1),
                new_line: Some(1),
                text: "fn main() {".to_owned(),
            },
            DiffRow {
                kind: DiffRowKind::Removed,
                old_line: Some(2),
                new_line: None,
                text: "    old_call();".to_owned(),
            },
            DiffRow {
                kind: DiffRowKind::Added,
                old_line: None,
                new_line: Some(2),
                text: "    new_call();".to_owned(),
            },
        ],
    }
}
