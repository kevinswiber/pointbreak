use shoreline::git::ingest_tracked_diff;
use shoreline::model::{CursorState, DiffSnapshot, ReviewNote, ReviewRowKind, ReviewStream, RowId};
use shoreline::sidecar::{
    ReviewNoteEntry, ReviewNoteTarget, ReviewNotesFile, ReviewNotesSidecar, apply_file_order,
    resolve_notes,
};
use shoreline::stream::{LayoutSnapshot, NavigationCommand, ViewportSpec};

use crate::support::git_repo::GitRepo;
use crate::support::snapshots::{StreamSummary, stream_summary};

#[test]
fn bounded_large_changeset_exercises_full_read_only_pipeline() {
    let repo = bounded_changeset_repo();
    let snapshot = ingest_tracked_diff(repo.path()).expect("large changeset should ingest");
    let sidecar = large_changeset_review_notes();

    assert_eq!(snapshot.files.len(), 5);
    assert_eq!(hunk_count(&snapshot), 5);

    let ordered = apply_file_order(snapshot.files.clone(), &sidecar);
    assert!(ordered.diagnostics.is_empty(), "{:#?}", ordered.diagnostics);
    let ordered_snapshot = DiffSnapshot::new(
        snapshot.review_id.clone(),
        snapshot.snapshot_id.clone(),
        ordered.files,
    );
    let resolved = resolve_notes(&ordered_snapshot.files, &sidecar);
    assert!(
        resolved.diagnostics.is_empty(),
        "{:#?}",
        resolved.diagnostics
    );
    assert_eq!(resolved.notes.len(), 4);

    let built = ReviewStream::from_snapshot_and_review_notes_sidecar(&snapshot, &sidecar);
    assert!(built.diagnostics.is_empty(), "{:#?}", built.diagnostics);
    assert_eq!(
        file_header_paths(&built.stream),
        vec![
            "src/untracked.rs",
            "src/file_b.rs",
            "src/file_a.rs",
            "assets/data.bin",
            "scripts/run.sh",
        ]
    );

    let summary = stream_summary(&built.stream);
    assert_eq!(
        summary,
        StreamSummary {
            file_headers: 5,
            hunk_headers: 5,
            diff_rows: 33,
            metadata_rows: 2,
            note_rows: 4,
            stale_note_rows: 0,
            empty_rows: 0,
            total_rows: 49,
        }
    );

    assert_navigation_endpoints(&built.stream);

    let layout = LayoutSnapshot::from_stream(&built.stream, ViewportSpec::new(100, 8));
    assert_eq!(layout.content_height, summary.total_rows);
    assert_eq!(layout.row_spans.len(), summary.total_rows);

    let stream_json = serde_json::to_string(&built.stream).expect("stream serializes");
    let decoded_stream: ReviewStream =
        shoreline::model::decode_json(&stream_json).expect("stream deserializes");
    assert_eq!(decoded_stream, built.stream);

    let snapshot_json =
        serde_json::to_string(&ordered_snapshot).expect("ordered snapshot serializes");
    let decoded_snapshot: DiffSnapshot =
        shoreline::model::decode_json(&snapshot_json).expect("ordered snapshot deserializes");
    let notes_json = serde_json::to_string(&resolved.notes).expect("notes serialize");
    let decoded_notes: Vec<ReviewNote> =
        shoreline::model::decode_json(&notes_json).expect("notes deserialize");
    let rebuilt_stream =
        ReviewStream::from_snapshot_with_resolved_notes(&decoded_snapshot, &decoded_notes);

    assert_eq!(stream_summary(&rebuilt_stream), summary);
    assert_eq!(row_ids(&rebuilt_stream), row_ids(&built.stream));
}

fn bounded_changeset_repo() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/file_a.rs", numbered_source("file_a", &[]));
    repo.write("src/file_b.rs", numbered_source("file_b", &[]));
    repo.write("assets/data.bin", [0, 159, 146, 150]);
    repo.write("scripts/run.sh", "#!/bin/sh\necho shore\n");
    repo.commit_all("base");

    repo.write(
        "src/file_a.rs",
        numbered_source("file_a", &[(2, 102), (14, 114)]),
    );
    repo.write(
        "src/file_b.rs",
        numbered_source("file_b", &[(4, 204), (17, 217)]),
    );
    repo.write("assets/data.bin", [0, 159, 146, 151]);
    repo.mark_executable_in_index("scripts/run.sh");
    repo.write(
        "src/untracked.rs",
        "pub fn untracked_01() {}\npub fn untracked_02() {}\npub fn untracked_03() {}\n",
    );

    repo
}

fn large_changeset_review_notes() -> ReviewNotesSidecar {
    ReviewNotesSidecar {
        schema: Some("shore.review-notes".to_owned()),
        version: 1,
        summary: None,
        files: vec![
            sidecar_file("src/untracked.rs", &[("note-untracked", 2)]),
            sidecar_file("src/file_b.rs", &[("note-file-b-4", 4)]),
            sidecar_file(
                "src/file_a.rs",
                &[("note-file-a-2", 2), ("note-file-a-14", 14)],
            ),
        ],
    }
}

fn sidecar_file(path: &str, notes: &[(&str, u32)]) -> ReviewNotesFile {
    ReviewNotesFile {
        path: path.to_owned(),
        old_path: None,
        summary: None,
        notes: notes
            .iter()
            .map(|(title, line)| ReviewNoteEntry {
                id: Some(format!("sidecar:{title}")),
                title: Some((*title).to_owned()),
                body: None,
                target: Some(ReviewNoteTarget {
                    side: shoreline::model::Side::New,
                    start_line: *line,
                    end_line: *line,
                }),
                tags: Vec::new(),
                confidence: None,
                source: None,
                author: None,
                created_at: None,
            })
            .collect(),
    }
}

fn numbered_source(prefix: &str, replacements: &[(u32, u32)]) -> String {
    (1..=20)
        .map(|line| {
            let value = replacements
                .iter()
                .find_map(|(target_line, value)| (*target_line == line).then_some(*value))
                .unwrap_or(line);
            format!("pub fn {prefix}_{line:02}() -> u32 {{ {value} }}\n")
        })
        .collect()
}

fn hunk_count(snapshot: &DiffSnapshot) -> usize {
    snapshot.files.iter().map(|file| file.hunks.len()).sum()
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

fn assert_navigation_endpoints(stream: &ReviewStream) {
    let hunk_rows = row_ids_matching(stream, |kind| {
        matches!(kind, ReviewRowKind::HunkHeader { .. })
    });
    assert_eq!(hunk_rows.len(), 5);

    let previous_from_first = stream.navigate(
        &CursorState::at_row(hunk_rows[0].clone()),
        NavigationCommand::PreviousHunk,
    );
    assert_eq!(
        previous_from_first.cursor,
        CursorState::at_row(hunk_rows[0].clone())
    );
    assert!(previous_from_first.clamped);

    let next_from_first = stream.navigate(
        &CursorState::at_row(hunk_rows[0].clone()),
        NavigationCommand::NextHunk,
    );
    assert_eq!(
        next_from_first.cursor,
        CursorState::at_row(hunk_rows[1].clone())
    );
    assert!(!next_from_first.clamped);

    let next_from_last = stream.navigate(
        &CursorState::at_row(hunk_rows[4].clone()),
        NavigationCommand::NextHunk,
    );
    assert_eq!(
        next_from_last.cursor,
        CursorState::at_row(hunk_rows[4].clone())
    );
    assert!(next_from_last.clamped);

    let note_rows = row_ids_matching(stream, |kind| matches!(kind, ReviewRowKind::Note { .. }));
    assert_eq!(note_rows.len(), 4);

    let first_noted = stream.navigate(
        &CursorState::at_row(RowId::new("row:0000")),
        NavigationCommand::NextNoteHunk,
    );
    assert_eq!(
        first_noted.cursor,
        CursorState::at_row(note_rows[0].clone())
    );
    assert!(!first_noted.clamped);

    let last_noted = note_rows.last().expect("note row exists").clone();
    let next_from_last_note = stream.navigate(
        &CursorState::at_row(last_noted.clone()),
        NavigationCommand::NextNoteHunk,
    );
    assert_eq!(next_from_last_note.cursor, CursorState::at_row(last_noted));
    assert!(next_from_last_note.clamped);
}

fn row_ids_matching(
    stream: &ReviewStream,
    predicate: impl Fn(&ReviewRowKind) -> bool,
) -> Vec<RowId> {
    stream
        .rows
        .iter()
        .filter_map(|row| predicate(&row.kind).then_some(row.id.clone()))
        .collect()
}

fn row_ids(stream: &ReviewStream) -> Vec<RowId> {
    stream.rows.iter().map(|row| row.id.clone()).collect()
}
