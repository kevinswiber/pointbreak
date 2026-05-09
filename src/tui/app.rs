use shore::dump::DumpDocument;
use shore::model::CursorState;
use shore::stream::{LayoutSnapshot, ViewportSpec};

pub(crate) struct TuiApp {
    document: DumpDocument,
    cursor: CursorState,
    viewport: ViewportSpec,
    layout: LayoutSnapshot,
    scroll_top: usize,
    should_quit: bool,
}

impl TuiApp {
    pub(crate) fn new(document: DumpDocument, viewport: ViewportSpec) -> Self {
        let layout = LayoutSnapshot::from_stream(&document.stream, viewport);
        let cursor = document
            .stream
            .rows
            .first()
            .map(|row| CursorState::at_row(row.id.clone()))
            .unwrap_or_else(CursorState::empty);

        Self {
            document,
            cursor,
            viewport,
            layout,
            scroll_top: 0,
            should_quit: false,
        }
    }

    pub(crate) fn cursor(&self) -> &CursorState {
        &self.cursor
    }

    pub(crate) fn document(&self) -> &DumpDocument {
        &self.document
    }

    pub(crate) fn layout(&self) -> &LayoutSnapshot {
        &self.layout
    }

    pub(crate) fn scroll_top(&self) -> usize {
        self.scroll_top
    }

    pub(crate) fn viewport(&self) -> ViewportSpec {
        self.viewport
    }

    pub(crate) fn should_quit(&self) -> bool {
        self.should_quit
    }
}

#[cfg(test)]
mod tests {
    use shore::dump::{DumpDocument, DumpInputSource, DumpInputSummary};
    use shore::model::{
        Anchor, CursorState, DiffFile, DiffRow, DiffRowKind, DiffSnapshot, FileId, FileStatus,
        HunkId, LineRange, ResolutionStatus, ReviewHunk, ReviewId, ReviewNote, ReviewNoteId,
        ReviewNoteSource, ReviewRow, ReviewRowKind, ReviewStream, RowId, Side, SnapshotId,
    };
    use shore::stream::ViewportSpec;

    use super::TuiApp;

    #[test]
    fn tui_app_initializes_from_dump_document() {
        let document = document_with_one_hunk_and_one_note();
        let app = TuiApp::new(document, ViewportSpec::new(80, 10));

        assert_eq!(app.cursor().row_id.as_ref(), Some(&RowId::new("row:0000")));
        assert_eq!(
            app.layout().content_height,
            app.document().stream.rows.len()
        );
        assert_eq!(app.scroll_top(), 0);
        assert_eq!(app.viewport(), ViewportSpec::new(80, 10));
        assert!(!app.should_quit());
    }

    #[test]
    fn tui_app_initializes_from_empty_stream() {
        let review_id = ReviewId::new("review:empty");
        let snapshot = DiffSnapshot::empty(review_id.clone());
        let stream = ReviewStream::empty(review_id);
        let document = DumpDocument::new(
            DumpInputSummary {
                source: DumpInputSource::None,
            },
            snapshot,
            Vec::new(),
            stream,
            Vec::new(),
        );

        let app = TuiApp::new(document, ViewportSpec::new(80, 10));

        assert_eq!(app.cursor(), &CursorState::empty());
        assert_eq!(app.layout().content_height, 0);
        assert_eq!(app.scroll_top(), 0);
        assert_eq!(app.viewport(), ViewportSpec::new(80, 10));
        assert!(!app.should_quit());
    }

    fn document_with_one_hunk_and_one_note() -> DumpDocument {
        let review_id = ReviewId::new("review:test");
        let snapshot_id = SnapshotId::new("snapshot:test");
        let file_id = FileId::new("src/lib.rs");
        let hunk_id = HunkId::new("hunk:0000");
        let note_id = ReviewNoteId::new("note:test");
        let diff_row = DiffRow {
            kind: DiffRowKind::Added,
            old_line: None,
            new_line: Some(1),
            text: "pub fn example() {}".to_owned(),
        };
        let hunk = ReviewHunk {
            id: hunk_id.clone(),
            header: "@@ -0,0 +1,1 @@".to_owned(),
            old_start: 0,
            old_lines: 0,
            new_start: 1,
            new_lines: 1,
            rows: vec![diff_row.clone()],
        };
        let snapshot = DiffSnapshot::new(
            review_id.clone(),
            snapshot_id.clone(),
            vec![DiffFile {
                id: file_id.clone(),
                status: FileStatus::Added,
                old_path: None,
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
                hunks: vec![hunk.clone()],
            }],
        );
        let note = ReviewNote {
            id: note_id.clone(),
            anchor: Anchor {
                file_id: file_id.clone(),
                side: Side::New,
                line_range: LineRange::new(1, 1),
                hunk_signature: hunk.signature(),
                target_text_hash: "sha256:test".to_owned(),
                status: ResolutionStatus::Exact,
            },
            source: ReviewNoteSource::Sidecar,
            title: "Example note".to_owned(),
            body: Some("Note body".to_owned()),
            tags: Vec::new(),
            confidence: None,
            external_source: None,
            author: Some("reviewer".to_owned()),
            created_at: None,
        };
        let rows = vec![
            ReviewRow {
                id: RowId::new("row:0000"),
                ordinal: 0,
                file_id: Some(file_id.clone()),
                hunk_id: None,
                kind: ReviewRowKind::FileHeader {
                    path: "src/lib.rs".to_owned(),
                    status: FileStatus::Added,
                },
            },
            ReviewRow {
                id: RowId::new("row:0001"),
                ordinal: 1,
                file_id: Some(file_id.clone()),
                hunk_id: Some(hunk_id.clone()),
                kind: ReviewRowKind::HunkHeader {
                    header: hunk.header.clone(),
                },
            },
            ReviewRow {
                id: RowId::new("row:0002"),
                ordinal: 2,
                file_id: Some(file_id.clone()),
                hunk_id: Some(hunk_id.clone()),
                kind: ReviewRowKind::Diff { row: diff_row },
            },
            ReviewRow {
                id: RowId::new("row:0003"),
                ordinal: 3,
                file_id: Some(file_id),
                hunk_id: Some(hunk_id),
                kind: ReviewRowKind::Note {
                    note_id,
                    target_row_id: RowId::new("row:0002"),
                    title: "Example note".to_owned(),
                },
            },
        ];
        let stream = ReviewStream {
            review_id,
            snapshot_id,
            rows,
        };

        DumpDocument::new(
            DumpInputSummary {
                source: DumpInputSource::ReviewNotes,
            },
            snapshot,
            vec![note],
            stream,
            Vec::new(),
        )
    }
}
