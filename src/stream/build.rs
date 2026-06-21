use crate::model::{
    DiffFile, DiffRow, DiffSnapshot, FileId, FileStatus, HunkId, ReviewNote, ReviewRow,
    ReviewRowKind, ReviewStream, RowId,
};
use crate::sidecar::{ReviewNotesDiagnostic, ReviewNotesSidecar, apply_file_order, resolve_notes};

const STALE_HUNK_SENTINEL: &str = "hunk:stale";
const ORPHANED_HUNK_SENTINEL: &str = "hunk:orphaned";
pub const ORPHAN_SECTION_PATH: &str = "<orphaned notes>";

fn build_review_stream(snapshot: &DiffSnapshot, notes: &[ReviewNote]) -> ReviewStream {
    let builder = StreamBuilder::new(snapshot, notes);
    builder.build()
}

impl ReviewStream {
    pub fn from_snapshot_with_resolved_notes(
        snapshot: &DiffSnapshot,
        notes: &[ReviewNote],
    ) -> Self {
        build_review_stream(snapshot, notes)
    }

    pub fn from_snapshot_and_review_notes_sidecar(
        snapshot: &DiffSnapshot,
        sidecar: &ReviewNotesSidecar,
    ) -> BuiltReviewStream {
        build_review_stream_from_review_notes_sidecar(snapshot, sidecar)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuiltReviewStream {
    pub stream: ReviewStream,
    pub diagnostics: Vec<ReviewNotesDiagnostic>,
}

fn build_review_stream_from_review_notes_sidecar(
    snapshot: &DiffSnapshot,
    sidecar: &ReviewNotesSidecar,
) -> BuiltReviewStream {
    let ordered = apply_file_order(snapshot.files.clone(), sidecar);
    let ordered_snapshot = DiffSnapshot::new(
        snapshot.review_id.clone(),
        snapshot.snapshot_id.clone(),
        ordered.files,
    );
    let resolved = resolve_notes(&ordered_snapshot.files, sidecar);
    let mut diagnostics = ordered.diagnostics;
    extend_unique_review_notes_diagnostics(&mut diagnostics, resolved.diagnostics);

    BuiltReviewStream {
        stream: build_review_stream(&ordered_snapshot, &resolved.notes),
        diagnostics,
    }
}

fn extend_unique_review_notes_diagnostics(
    diagnostics: &mut Vec<ReviewNotesDiagnostic>,
    new_diagnostics: Vec<ReviewNotesDiagnostic>,
) {
    for diagnostic in new_diagnostics {
        if !diagnostics
            .iter()
            .any(|existing| existing.code == diagnostic.code && existing.path == diagnostic.path)
        {
            diagnostics.push(diagnostic);
        }
    }
}

struct StreamBuilder<'a> {
    snapshot: &'a DiffSnapshot,
    notes: &'a [ReviewNote],
    rows: Vec<ReviewRow>,
}

impl<'a> StreamBuilder<'a> {
    fn new(snapshot: &'a DiffSnapshot, notes: &'a [ReviewNote]) -> Self {
        Self {
            snapshot,
            notes,
            rows: Vec::new(),
        }
    }

    fn build(mut self) -> ReviewStream {
        if self.snapshot.files.is_empty() && !self.has_orphan_notes() {
            self.push_row(
                None,
                None,
                ReviewRowKind::EmptyState {
                    message: "no changes".to_owned(),
                },
            );
        } else {
            for file in &self.snapshot.files {
                self.push_file(file);
            }
            self.push_orphan_section();
        }

        ReviewStream {
            review_id: self.snapshot.review_id.clone(),
            snapshot_id: self.snapshot.snapshot_id.clone(),
            rows: self.rows,
        }
    }

    fn push_file(&mut self, file: &DiffFile) {
        self.push_row(
            Some(file.id.clone()),
            None,
            ReviewRowKind::FileHeader {
                path: display_path(file),
                status: file.status.clone(),
            },
        );

        for metadata in &file.metadata_rows {
            self.push_row(
                Some(file.id.clone()),
                None,
                ReviewRowKind::Metadata {
                    metadata: metadata.clone(),
                },
            );
        }

        for hunk in &file.hunks {
            let hunk_signature = hunk.signature();
            self.push_row(
                Some(file.id.clone()),
                Some(hunk.id.clone()),
                ReviewRowKind::HunkHeader {
                    header: hunk.header.clone(),
                },
            );

            for diff_row in &hunk.rows {
                let target_row_id = self.push_row(
                    Some(file.id.clone()),
                    Some(hunk.id.clone()),
                    ReviewRowKind::Diff {
                        row: diff_row.clone(),
                    },
                );

                for note in self.note_rows_for_row(file, &hunk_signature, diff_row) {
                    self.push_row(
                        Some(file.id.clone()),
                        Some(hunk.id.clone()),
                        ReviewRowKind::Note {
                            note_id: note.note_id,
                            target_row_id: target_row_id.clone(),
                            title: note.title,
                        },
                    );
                }
            }
        }

        let stale_notes = self
            .notes
            .iter()
            .filter(|note| {
                note.anchor.file_id == file.id && note.anchor.hunk_signature == STALE_HUNK_SENTINEL
            })
            .cloned()
            .collect::<Vec<_>>();
        for note in stale_notes {
            self.push_row(Some(file.id.clone()), None, stale_note_row_kind(&note));
        }
    }

    fn note_rows_for_row(
        &self,
        file: &DiffFile,
        hunk_signature: &str,
        row: &DiffRow,
    ) -> Vec<NoteRowData> {
        self.notes
            .iter()
            .filter(|note| {
                note.anchor.file_id == file.id
                    && note.anchor.hunk_signature == hunk_signature
                    && row
                        .line_on_side(note.anchor.side)
                        .is_some_and(|line| line == note.anchor.line_range.end)
            })
            .map(|note| NoteRowData {
                note_id: note.id.clone(),
                title: note.title.clone(),
            })
            .collect()
    }

    fn push_orphan_section(&mut self) {
        let orphan_notes = self
            .notes
            .iter()
            .filter(|note| note.anchor.hunk_signature == ORPHANED_HUNK_SENTINEL)
            .cloned()
            .collect::<Vec<_>>();
        if orphan_notes.is_empty() {
            return;
        }

        self.push_row(
            None,
            None,
            ReviewRowKind::FileHeader {
                path: ORPHAN_SECTION_PATH.to_owned(),
                status: FileStatus::Modified,
            },
        );
        for note in orphan_notes {
            self.push_row(None, None, stale_note_row_kind(&note));
        }
    }

    fn has_orphan_notes(&self) -> bool {
        self.notes
            .iter()
            .any(|note| note.anchor.hunk_signature == ORPHANED_HUNK_SENTINEL)
    }

    fn push_row(
        &mut self,
        file_id: Option<FileId>,
        hunk_id: Option<HunkId>,
        kind: ReviewRowKind,
    ) -> RowId {
        let ordinal = self.rows.len();
        // RowId is opaque in the dump contract — see README "Behavior" notes under `shore dump`.
        // The width of this padding is not load-bearing; consumers must use `ordinal` for position.
        let id = RowId::new(format!("row:{ordinal:04}"));
        self.rows.push(ReviewRow {
            id: id.clone(),
            ordinal,
            file_id,
            hunk_id,
            kind,
        });
        id
    }
}

struct NoteRowData {
    note_id: crate::model::ReviewNoteId,
    title: String,
}

fn stale_note_row_kind(note: &ReviewNote) -> ReviewRowKind {
    ReviewRowKind::StaleNote {
        note_id: note.id.clone(),
        title: note.title.clone(),
        resolution_status: note.anchor.status.clone(),
        target_path: note.anchor.file_id.as_str().to_owned(),
        target_line_range: note.anchor.line_range.clone(),
    }
}

fn display_path(file: &DiffFile) -> String {
    file.new_path
        .clone()
        .or_else(|| file.old_path.clone())
        .unwrap_or_else(|| file.id.as_str().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        Anchor, DiffFile, DiffRow, DiffRowKind, DiffSnapshot, FileId, FileStatus, HunkId,
        LineRange, ObjectId, ResolutionStatus, ReviewHunk, ReviewId, ReviewNote, ReviewNoteId,
        ReviewNoteSource, ReviewRowKind, Side,
    };

    #[test]
    fn stream_builder_emits_stale_note_row_after_file_hunks() {
        let snapshot = snapshot_with_one_file_one_hunk();
        let notes = vec![stale_note("note:stale", "src/lib.rs")];

        let stream = ReviewStream::from_snapshot_with_resolved_notes(&snapshot, &notes);

        let kinds = stream
            .rows
            .iter()
            .map(|row| row.kind.clone())
            .collect::<Vec<_>>();

        assert!(matches!(kinds[0], ReviewRowKind::FileHeader { .. }));
        assert!(matches!(kinds[1], ReviewRowKind::HunkHeader { .. }));
        assert!(matches!(kinds[2], ReviewRowKind::Diff { .. }));
        match &kinds[3] {
            ReviewRowKind::StaleNote {
                note_id,
                resolution_status,
                target_path,
                target_line_range,
                ..
            } => {
                assert_eq!(note_id.as_str(), "note:stale");
                assert_eq!(*resolution_status, ResolutionStatus::Stale);
                assert_eq!(target_path, "src/lib.rs");
                assert_eq!(target_line_range.start, 99);
                assert_eq!(target_line_range.end, 99);
            }
            other => panic!("expected StaleNote, got {other:?}"),
        }
    }

    #[test]
    fn stream_builder_emits_orphan_section_after_files() {
        let snapshot = snapshot_with_one_file_one_hunk();
        let notes = vec![orphan_note("note:orphan", "src/gone.rs")];

        let stream = ReviewStream::from_snapshot_with_resolved_notes(&snapshot, &notes);

        let len = stream.rows.len();
        assert!(len >= 5, "expected at least 5 rows, got {len}");
        match &stream.rows[len - 2].kind {
            ReviewRowKind::FileHeader { path, .. } => {
                assert_eq!(path, ORPHAN_SECTION_PATH);
            }
            other => panic!("expected synthetic FileHeader, got {other:?}"),
        }
        match &stream.rows[len - 1].kind {
            ReviewRowKind::StaleNote {
                note_id,
                resolution_status,
                target_path,
                ..
            } => {
                assert_eq!(note_id.as_str(), "note:orphan");
                assert_eq!(*resolution_status, ResolutionStatus::Orphaned);
                assert_eq!(target_path, "src/gone.rs");
            }
            other => panic!("expected orphan StaleNote, got {other:?}"),
        }
    }

    #[test]
    fn stream_builder_omits_orphan_section_when_no_orphan_notes() {
        let snapshot = snapshot_with_one_file_one_hunk();
        let notes: Vec<ReviewNote> = Vec::new();

        let stream = ReviewStream::from_snapshot_with_resolved_notes(&snapshot, &notes);

        assert!(
            stream.rows.iter().all(
                |row| !matches!(&row.kind, ReviewRowKind::FileHeader { path, .. } if path == ORPHAN_SECTION_PATH)
            ),
            "no synthetic orphan header expected; got rows {:?}",
            stream.rows.iter().map(|r| &r.kind).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn stream_builder_emits_orphan_section_when_snapshot_is_empty() {
        let snapshot = empty_snapshot();
        let notes = vec![orphan_note("note:orphan", "src/gone.rs")];

        let stream = ReviewStream::from_snapshot_with_resolved_notes(&snapshot, &notes);

        match &stream.rows.first().map(|row| &row.kind) {
            Some(ReviewRowKind::FileHeader { path, .. }) => {
                assert_eq!(path, ORPHAN_SECTION_PATH);
            }
            other => panic!("expected synthetic orphan header first; got {other:?}"),
        }
        assert!(
            stream
                .rows
                .iter()
                .all(|row| !matches!(row.kind, ReviewRowKind::EmptyState { .. })),
            "empty-state row should be suppressed when orphan section is present; got rows {:?}",
            stream.rows.iter().map(|r| &r.kind).collect::<Vec<_>>(),
        );
        assert!(
            stream.rows.iter().any(|row| matches!(
                &row.kind,
                ReviewRowKind::StaleNote { resolution_status, .. }
                    if *resolution_status == ResolutionStatus::Orphaned
            )),
            "expected at least one orphaned StaleNote row",
        );
    }

    #[test]
    fn stream_builder_stale_row_carries_no_hunk_id() {
        let snapshot = snapshot_with_one_file_one_hunk();
        let notes = vec![stale_note("note:stale", "src/lib.rs")];

        let stream = ReviewStream::from_snapshot_with_resolved_notes(&snapshot, &notes);

        let stale_row = stream
            .rows
            .iter()
            .find(|row| matches!(row.kind, ReviewRowKind::StaleNote { .. }))
            .expect("stale row present");
        assert_eq!(
            stale_row.file_id.as_ref().map(FileId::as_str),
            Some("src/lib.rs")
        );
        assert!(
            stale_row.hunk_id.is_none(),
            "stale row has no hunk to point at"
        );
    }

    fn snapshot_with_one_file_one_hunk() -> DiffSnapshot {
        let review_id = ReviewId::new("review:test");
        let snapshot_id = ObjectId::new("snapshot:test");
        let file_id = FileId::new("src/lib.rs");
        let hunk = ReviewHunk {
            id: HunkId::new("src/lib.rs:1:1"),
            header: "@@ -0,0 +1,1 @@".to_owned(),
            old_start: 0,
            old_lines: 0,
            new_start: 1,
            new_lines: 1,
            rows: vec![DiffRow {
                kind: DiffRowKind::Added,
                old_line: None,
                new_line: Some(1),
                text: "line one".to_owned(),
            }],
        };
        DiffSnapshot::new(
            review_id,
            snapshot_id,
            vec![DiffFile {
                id: file_id,
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

    fn empty_snapshot() -> DiffSnapshot {
        DiffSnapshot::new(
            ReviewId::new("review:test"),
            ObjectId::new("snapshot:test"),
            Vec::new(),
        )
    }

    fn stale_note(note_id: &str, path: &str) -> ReviewNote {
        ReviewNote {
            id: ReviewNoteId::new(note_id),
            anchor: Anchor {
                file_id: FileId::new(path),
                side: Side::New,
                line_range: LineRange::new(99, 99),
                hunk_signature: STALE_HUNK_SENTINEL.to_owned(),
                target_text_hash: String::new(),
                status: ResolutionStatus::Stale,
            },
            source: ReviewNoteSource::Sidecar,
            title: "Stale".to_owned(),
            body: None,
            tags: Vec::new(),
            confidence: None,
            external_source: None,
            author: None,
            created_at: None,
        }
    }

    fn orphan_note(note_id: &str, path: &str) -> ReviewNote {
        let mut note = stale_note(note_id, path);
        note.anchor.hunk_signature = ORPHANED_HUNK_SENTINEL.to_owned();
        note.anchor.status = ResolutionStatus::Orphaned;
        note
    }
}
