use shore::model::{
    DiffFile, DiffRow, DiffRowKind, FileId, FileStatus, HunkId, LineRange, ResolutionStatus,
    ReviewHunk, ReviewNoteSource, Side,
};
use shore::sidecar::{
    DiagnosticLevel, ReviewNoteEntry, ReviewNoteTarget, ReviewNotesDiagnosticCode, ReviewNotesFile,
    ReviewNotesSidecar, apply_file_order, parse_hunk_agent_context, parse_review_notes_sidecar,
    resolve_notes,
};

#[test]
fn native_review_notes_sidecar_parses_file_order_and_notes() {
    let parsed =
        parse_review_notes_sidecar(include_str!("../fixtures/sidecars/basic-review-notes.json"))
            .expect("valid review notes sidecar parses");

    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    assert_eq!(parsed.sidecar.schema.as_deref(), Some("shore.review-notes"));
    assert_eq!(parsed.sidecar.version, 1);
    assert_eq!(
        parsed.sidecar.summary.as_deref(),
        Some("Review notes parser fixture")
    );

    let paths = parsed
        .sidecar
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(paths, vec!["src/new_name.rs", "src/git/raw.rs"]);

    let renamed = &parsed.sidecar.files[0];
    assert_eq!(renamed.old_path.as_deref(), Some("src/old_name.rs"));
    assert_eq!(
        renamed.summary.as_deref(),
        Some("Narrative summary for renamed file")
    );
    assert_eq!(renamed.notes.len(), 2);
    assert_eq!(renamed.notes[0].id.as_deref(), Some("note-new-range"));
    assert_eq!(renamed.notes[0].title.as_deref(), Some("New-side note"));
    assert_eq!(renamed.notes[0].body.as_deref(), Some("Why it matters"));
    assert_eq!(renamed.notes[0].tags, vec!["risk", "parser"]);
    let target = renamed.notes[0].target.as_ref().expect("note has target");
    assert_eq!(target.side, Side::New);
    assert_eq!(target.start_line, 10);
    assert_eq!(target.end_line, 12);
}

#[test]
fn invalid_review_notes_entries_return_recoverable_diagnostics() {
    let parsed = parse_review_notes_sidecar(include_str!(
        "../fixtures/sidecars/invalid-review-notes.json"
    ))
    .expect("invalid review notes remain recoverable");

    assert_eq!(parsed.sidecar.files.len(), 2);
    assert_eq!(parsed.diagnostics.len(), 4);

    let diagnostics = parsed
        .diagnostics
        .iter()
        .map(|diagnostic| {
            (
                diagnostic.level.clone(),
                diagnostic.code.clone(),
                diagnostic.path.as_str(),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        diagnostics,
        vec![
            (
                DiagnosticLevel::Warning,
                ReviewNotesDiagnosticCode::InvalidRange,
                "files[0].notes[0].target"
            ),
            (
                DiagnosticLevel::Warning,
                ReviewNotesDiagnosticCode::MissingNoteTitle,
                "files[0].notes[1].title"
            ),
            (
                DiagnosticLevel::Warning,
                ReviewNotesDiagnosticCode::MissingNoteTarget,
                "files[0].notes[2].target"
            ),
            (
                DiagnosticLevel::Warning,
                ReviewNotesDiagnosticCode::MissingFilePath,
                "files[1].path"
            ),
        ]
    );
}

#[test]
fn legacy_hunk_agent_context_imports_as_review_notes() {
    let parsed = parse_hunk_agent_context(include_str!(
        "../fixtures/sidecars/legacy-hunk-agent-context.json"
    ))
    .expect("legacy Hunk context imports");

    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    assert_eq!(parsed.sidecar.schema.as_deref(), Some("shore.review-notes"));
    assert_eq!(parsed.sidecar.version, 1);
    assert_eq!(
        parsed.sidecar.summary.as_deref(),
        Some("Legacy Hunk parser fixture")
    );

    let file = &parsed.sidecar.files[0];
    assert_eq!(file.path, "src/new_name.rs");
    assert_eq!(file.old_path.as_deref(), Some("src/old_name.rs"));
    assert_eq!(file.notes.len(), 2);

    let note = &file.notes[0];
    assert_eq!(note.id.as_deref(), Some("ann-new-range"));
    assert_eq!(note.title.as_deref(), Some("New-side annotation"));
    assert_eq!(
        note.body.as_deref(),
        Some("Explains why the new range matters")
    );
    assert_eq!(note.source.as_deref(), Some("hunk"));
    let target = note
        .target
        .as_ref()
        .expect("legacy newRange becomes target");
    assert_eq!(target.side, Side::New);
    assert_eq!(target.start_line, 10);
    assert_eq!(target.end_line, 12);

    let old_note = &file.notes[1];
    assert_eq!(old_note.title.as_deref(), Some("Old-side annotation"));
    assert_eq!(
        old_note.target.as_ref().expect("old range target").side,
        Side::Old
    );
}

#[test]
fn invalid_legacy_hunk_agent_context_import_maps_diagnostics_to_note_paths() {
    let parsed = parse_hunk_agent_context(include_str!(
        "../fixtures/sidecars/legacy-invalid-hunk-agent-context.json"
    ))
    .expect("invalid legacy Hunk context imports recoverably");

    assert_eq!(parsed.sidecar.files.len(), 2);
    assert_eq!(parsed.diagnostics.len(), 4);

    let diagnostics = parsed
        .diagnostics
        .iter()
        .map(|diagnostic| {
            (
                diagnostic.level.clone(),
                diagnostic.code.clone(),
                diagnostic.path.as_str(),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        diagnostics,
        vec![
            (
                DiagnosticLevel::Warning,
                ReviewNotesDiagnosticCode::InvalidRange,
                "files[0].notes[0].target"
            ),
            (
                DiagnosticLevel::Warning,
                ReviewNotesDiagnosticCode::InvalidRange,
                "files[0].notes[1].target"
            ),
            (
                DiagnosticLevel::Warning,
                ReviewNotesDiagnosticCode::MissingNoteTitle,
                "files[0].notes[2].title"
            ),
            (
                DiagnosticLevel::Warning,
                ReviewNotesDiagnosticCode::MissingFilePath,
                "files[1].path"
            ),
        ]
    );
}

#[test]
fn native_review_notes_parser_does_not_accept_hunk_aliases() {
    let parsed = parse_review_notes_sidecar(include_str!(
        "../fixtures/sidecars/legacy-hunk-agent-context.json"
    ))
    .expect("legacy Hunk shape remains JSON-parseable");

    assert_eq!(
        parsed.sidecar.schema.as_deref(),
        Some("shore.agent-context")
    );
    assert!(parsed.sidecar.files[0].notes.is_empty());
    assert!(parsed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == ReviewNotesDiagnosticCode::InvalidSchema && diagnostic.path == "schema"
    }));
    assert!(parsed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == ReviewNotesDiagnosticCode::MissingVersion && diagnostic.path == "version"
    }));
    assert!(parsed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == ReviewNotesDiagnosticCode::MissingNotes
            && diagnostic.path == "files[0].notes"
    }));
}

#[test]
fn native_review_notes_apply_order_and_resolve_to_minimal_anchors() {
    let files = vec![modified_file("src/other.rs"), annotated_file()];
    let sidecar = ReviewNotesSidecar {
        schema: Some("shore.review-notes".to_owned()),
        version: 1,
        summary: None,
        files: vec![
            ReviewNotesFile {
                path: "src/lib.rs".to_owned(),
                old_path: None,
                summary: None,
                notes: vec![
                    review_note("added row", Side::New, 2, 2),
                    review_note("removed row", Side::Old, 2, 2),
                    review_note("context row", Side::New, 3, 3),
                    review_note("multi-line range", Side::New, 3, 4),
                ],
            },
            ReviewNotesFile {
                path: "src/stale.rs".to_owned(),
                old_path: None,
                summary: None,
                notes: Vec::new(),
            },
            ReviewNotesFile {
                path: "src/other.rs".to_owned(),
                old_path: None,
                summary: None,
                notes: Vec::new(),
            },
        ],
    };

    let ordered = apply_file_order(files, &sidecar);

    let paths = ordered
        .files
        .iter()
        .map(|file| file.new_path.as_deref().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(paths, vec!["src/lib.rs", "src/other.rs"]);
    assert_eq!(ordered.diagnostics.len(), 1);
    assert_eq!(
        ordered.diagnostics[0].code,
        ReviewNotesDiagnosticCode::StaleFilePath
    );
    assert_eq!(ordered.diagnostics[0].path, "files[1].path");

    let resolved = resolve_notes(&ordered.files, &sidecar);

    assert_eq!(resolved.notes.len(), 4);
    assert_eq!(resolved.diagnostics.len(), 1);
    assert_eq!(
        resolved.diagnostics[0].code,
        ReviewNotesDiagnosticCode::StaleFilePath
    );
    assert_eq!(resolved.diagnostics[0].path, "files[1].path");

    assert_note(
        note_by_title(&resolved.notes, "added row"),
        Side::New,
        LineRange::new(2, 2),
        "sha256:569dd3149acd6f05a7736e6ff2e3aed60f472171aeb74a3cc43c6e6813ca8c8c",
    );
    assert_note(
        note_by_title(&resolved.notes, "removed row"),
        Side::Old,
        LineRange::new(2, 2),
        "sha256:489f4336b2747c25479b8e1409c076c44baf968183ec052ade602214795fdde9",
    );
    assert_note(
        note_by_title(&resolved.notes, "context row"),
        Side::New,
        LineRange::new(3, 3),
        "sha256:c0e9a8fcaa59a634469252d037aba2004dfdb824b565c72102f58dba2a4134d0",
    );
    assert_note(
        note_by_title(&resolved.notes, "multi-line range"),
        Side::New,
        LineRange::new(3, 4),
        "sha256:7dfd8a717636663d5e7bc9b988060d22ad6fea1ca6e8034cda845e304f67c633",
    );
}

fn assert_note(
    note: &shore::model::ReviewNote,
    side: Side,
    line_range: LineRange,
    target_text_hash: &str,
) {
    assert_eq!(note.source, ReviewNoteSource::Sidecar);
    assert_eq!(note.anchor.file_id, FileId::new("src/lib.rs"));
    assert_eq!(note.anchor.side, side);
    assert_eq!(note.anchor.line_range, line_range);
    assert_eq!(note.anchor.status, ResolutionStatus::Exact);
    assert_eq!(
        note.anchor.hunk_signature,
        "sha256:0ed5a197d3bd2107a7250d2e36c33328dcf6c135e84bec170f342e22397a64f7"
    );
    assert_eq!(note.anchor.target_text_hash, target_text_hash);
}

fn note_by_title<'a>(
    notes: &'a [shore::model::ReviewNote],
    title: &str,
) -> &'a shore::model::ReviewNote {
    notes
        .iter()
        .find(|note| note.title == title)
        .expect("note exists")
}

fn review_note(title: &str, side: Side, start_line: u32, end_line: u32) -> ReviewNoteEntry {
    ReviewNoteEntry {
        id: Some(format!("note-{title}")),
        title: Some(title.to_owned()),
        body: Some(format!("body for {title}")),
        target: Some(ReviewNoteTarget {
            side,
            start_line,
            end_line,
        }),
        tags: vec!["fixture".to_owned()],
        confidence: Some("high".to_owned()),
        source: Some("test".to_owned()),
        author: Some("codex".to_owned()),
        created_at: Some("2026-05-09T00:00:00Z".to_owned()),
    }
}

fn annotated_file() -> DiffFile {
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
        hunks: vec![ReviewHunk {
            id: HunkId::new("src/lib.rs:1:1"),
            header: "@@ -1,4 +1,5 @@".to_owned(),
            old_start: 1,
            old_lines: 4,
            new_start: 1,
            new_lines: 5,
            rows: vec![
                context_row(1, 1, "fn main() {"),
                removed_row(2, "    old_call();"),
                added_row(2, "    new_call();"),
                context_row(3, 3, "    keep();"),
                added_row(4, "    extra();"),
                context_row(4, 5, "}"),
            ],
        }],
    }
}

fn context_row(old_line: u32, new_line: u32, text: &str) -> DiffRow {
    DiffRow {
        kind: DiffRowKind::Context,
        old_line: Some(old_line),
        new_line: Some(new_line),
        text: text.to_owned(),
    }
}

fn added_row(new_line: u32, text: &str) -> DiffRow {
    DiffRow {
        kind: DiffRowKind::Added,
        old_line: None,
        new_line: Some(new_line),
        text: text.to_owned(),
    }
}

fn removed_row(old_line: u32, text: &str) -> DiffRow {
    DiffRow {
        kind: DiffRowKind::Removed,
        old_line: Some(old_line),
        new_line: None,
        text: text.to_owned(),
    }
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
