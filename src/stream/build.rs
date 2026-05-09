use crate::model::{
    DiffFile, DiffRow, DiffSnapshot, FileId, HunkId, ReviewNote, ReviewRow, ReviewRowKind,
    ReviewStream, RowId,
};
use crate::sidecar::{ReviewNotesDiagnostic, ReviewNotesSidecar, apply_file_order, resolve_notes};

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
        if self.snapshot.files.is_empty() {
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

    fn push_row(
        &mut self,
        file_id: Option<FileId>,
        hunk_id: Option<HunkId>,
        kind: ReviewRowKind,
    ) -> RowId {
        let ordinal = self.rows.len();
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

fn display_path(file: &DiffFile) -> String {
    file.new_path
        .clone()
        .or_else(|| file.old_path.clone())
        .unwrap_or_else(|| file.id.as_str().to_owned())
}
