use shoreline::dump::DumpDocument;
use shoreline::model::{CursorState, FileId, HunkId, ReviewRow, RowId};
use shoreline::stream::{LayoutSnapshot, NavigationCommand, RevealTarget, ViewportSpec};

pub(crate) struct TuiApp {
    document: DumpDocument,
    cursor: CursorState,
    viewport: ViewportSpec,
    layout: LayoutSnapshot,
    scroll_top: usize,
    last_reload_error: Option<String>,
    should_quit: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TuiAction {
    RowDown,
    RowUp,
    NextHunk,
    PreviousHunk,
    NextNoteHunk,
    PreviousNoteHunk,
    Resize(ViewportSpec),
    Quit,
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
            last_reload_error: None,
            should_quit: false,
        }
    }

    pub(crate) fn cursor(&self) -> &CursorState {
        &self.cursor
    }

    pub(crate) fn document(&self) -> &DumpDocument {
        &self.document
    }

    #[cfg(test)]
    pub(crate) fn layout(&self) -> &LayoutSnapshot {
        &self.layout
    }

    pub(crate) fn scroll_top(&self) -> usize {
        self.scroll_top
    }

    #[cfg(test)]
    pub(crate) fn viewport(&self) -> ViewportSpec {
        self.viewport
    }

    pub(crate) fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub(crate) fn last_reload_error(&self) -> Option<&str> {
        self.last_reload_error.as_deref()
    }

    pub(crate) fn set_last_reload_error(&mut self, message: impl Into<String>) {
        self.last_reload_error = Some(message.into());
    }

    pub(crate) fn clear_last_reload_error(&mut self) {
        self.last_reload_error = None;
    }

    #[cfg(test)]
    pub(crate) fn current_row_is_visible(&self) -> bool {
        let Some(row_id) = self.cursor.row_id.as_ref() else {
            return true;
        };
        let Some(span) = self.layout.row_span(row_id) else {
            return false;
        };
        let viewport_end = self.scroll_top.saturating_add(self.viewport.height);
        self.scroll_top <= span.start && span.end <= viewport_end
    }

    pub(crate) fn handle_action(&mut self, action: TuiAction) {
        match action {
            TuiAction::RowDown => self.move_row(1),
            TuiAction::RowUp => self.move_row(-1),
            TuiAction::NextHunk => self.navigate(NavigationCommand::NextHunk),
            TuiAction::PreviousHunk => self.navigate(NavigationCommand::PreviousHunk),
            TuiAction::NextNoteHunk => self.navigate(NavigationCommand::NextNoteHunk),
            TuiAction::PreviousNoteHunk => self.navigate(NavigationCommand::PreviousNoteHunk),
            TuiAction::Resize(viewport) => self.resize(viewport),
            TuiAction::Quit => {
                self.should_quit = true;
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn reload_with(&mut self, document: DumpDocument) {
        let preserved_row_id = self.cursor.row_id.clone();
        let preserved_row = preserved_row_id
            .as_ref()
            .and_then(|row_id| self.row_meta(row_id));
        let preserved_file_id = preserved_row.and_then(|row| row.file_id.clone());
        let preserved_hunk_id = preserved_row.and_then(|row| row.hunk_id.clone());

        self.document = document;
        self.layout = LayoutSnapshot::from_stream(&self.document.stream, self.viewport);
        self.cursor = self.restore_cursor(preserved_row_id, preserved_file_id, preserved_hunk_id);
        self.scroll_top = 0;
        if let Some(row_id) = self.cursor.row_id.clone() {
            self.reveal_row(&row_id);
        }
    }

    fn navigate(&mut self, command: NavigationCommand) {
        let result = self.document.stream.navigate(&self.cursor, command);
        self.cursor = result.cursor;
        if let Some(RevealTarget::Row { row_id }) = result.reveal {
            self.reveal_row(&row_id);
        }
    }

    fn move_row(&mut self, offset: isize) {
        let Some(selected_index) = self.selected_row_index() else {
            return;
        };
        let max_index = self.document.stream.rows.len().saturating_sub(1);
        let target_index = selected_index.saturating_add_signed(offset).min(max_index);
        let row_id = self.document.stream.rows[target_index].id.clone();
        self.cursor = CursorState::at_row(row_id.clone());
        self.reveal_row(&row_id);
    }

    fn resize(&mut self, viewport: ViewportSpec) {
        self.viewport = viewport;
        self.layout = LayoutSnapshot::from_stream(&self.document.stream, viewport);
        if let Some(row_id) = self.cursor.row_id.clone() {
            self.reveal_row(&row_id);
        }
    }

    fn selected_row_index(&self) -> Option<usize> {
        let Some(row_id) = self.cursor.row_id.as_ref() else {
            return (!self.document.stream.rows.is_empty()).then_some(0);
        };
        self.document
            .stream
            .rows
            .iter()
            .position(|row| &row.id == row_id)
            .or((!self.document.stream.rows.is_empty()).then_some(0))
    }

    fn reveal_row(&mut self, row_id: &RowId) {
        if let Some(position) = self.layout.reveal_row(row_id) {
            self.scroll_top = position.scroll_top;
        }
    }

    #[allow(dead_code)]
    fn restore_cursor(
        &self,
        prior_row_id: Option<RowId>,
        prior_file_id: Option<FileId>,
        prior_hunk_id: Option<HunkId>,
    ) -> CursorState {
        if let Some(row_id) = prior_row_id.as_ref()
            && self
                .document
                .stream
                .rows
                .iter()
                .any(|row| &row.id == row_id)
        {
            return CursorState::at_row(row_id.clone());
        }

        if let (Some(file_id), Some(hunk_id)) = (prior_file_id.as_ref(), prior_hunk_id.as_ref())
            && let Some(row) = self.document.stream.rows.iter().find(|row| {
                row.file_id.as_ref() == Some(file_id) && row.hunk_id.as_ref() == Some(hunk_id)
            })
        {
            return CursorState::at_row(row.id.clone());
        }

        if let Some(file_id) = prior_file_id.as_ref()
            && let Some(row) = self
                .document
                .stream
                .rows
                .iter()
                .find(|row| row.file_id.as_ref() == Some(file_id))
        {
            return CursorState::at_row(row.id.clone());
        }

        self.document
            .stream
            .rows
            .first()
            .map(|row| CursorState::at_row(row.id.clone()))
            .unwrap_or_else(CursorState::empty)
    }

    #[allow(dead_code)]
    fn row_meta(&self, row_id: &RowId) -> Option<&ReviewRow> {
        self.document
            .stream
            .rows
            .iter()
            .find(|row| &row.id == row_id)
    }
}

#[cfg(test)]
mod tests {
    use shoreline::dump::{DumpDocument, DumpInputSource, DumpInputSummary};
    use shoreline::model::{
        Anchor, CursorState, DiffFile, DiffRow, DiffRowKind, DiffSnapshot, FileId, FileStatus,
        HunkId, LineRange, ObjectId, ResolutionStatus, ReviewHunk, ReviewId, ReviewNote,
        ReviewNoteId, ReviewNoteSource, ReviewRow, ReviewRowKind, ReviewStream, RowId, Side,
    };
    use shoreline::stream::ViewportSpec;

    use super::{TuiAction, TuiApp};

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

    #[test]
    fn next_hunk_action_uses_model_navigation_and_reveals_row() {
        let mut app = app_with_two_hunks(ViewportSpec::new(80, 3));
        app.cursor = CursorState::at_row(RowId::new("row:0001"));

        app.handle_action(TuiAction::NextHunk);

        assert_eq!(app.cursor().row_id.as_ref(), Some(&RowId::new("row:0003")));
        assert_eq!(app.scroll_top(), 3);
    }

    #[test]
    fn previous_hunk_action_clamps_at_first_hunk() {
        let mut app = app_with_two_hunks(ViewportSpec::new(80, 3));
        app.cursor = CursorState::at_row(RowId::new("row:0001"));

        app.handle_action(TuiAction::PreviousHunk);

        assert_eq!(app.cursor().row_id.as_ref(), Some(&RowId::new("row:0001")));
        assert!(app.current_row_is_visible());
    }

    #[test]
    fn next_note_hunk_action_lands_on_note_row() {
        let mut app = app_with_two_hunks(ViewportSpec::new(80, 3));
        app.cursor = CursorState::at_row(RowId::new("row:0001"));

        app.handle_action(TuiAction::NextNoteHunk);

        assert_eq!(app.cursor().row_id.as_ref(), Some(&RowId::new("row:0005")));
        assert!(app.current_row_is_visible());
    }

    #[test]
    fn previous_note_hunk_action_clamps_to_first_note_row() {
        let mut app = app_with_two_hunks(ViewportSpec::new(80, 3));
        app.cursor = CursorState::at_row(RowId::new("row:0005"));

        app.handle_action(TuiAction::PreviousNoteHunk);

        assert_eq!(app.cursor().row_id.as_ref(), Some(&RowId::new("row:0005")));
        assert!(app.current_row_is_visible());
    }

    #[test]
    fn row_actions_move_by_visible_rows_and_clamp() {
        let mut app = app_with_two_hunks(ViewportSpec::new(80, 3));

        app.handle_action(TuiAction::RowDown);
        assert_eq!(app.cursor().row_id.as_ref(), Some(&RowId::new("row:0001")));

        app.handle_action(TuiAction::RowUp);
        assert_eq!(app.cursor().row_id.as_ref(), Some(&RowId::new("row:0000")));

        app.handle_action(TuiAction::RowUp);
        assert_eq!(app.cursor().row_id.as_ref(), Some(&RowId::new("row:0000")));
    }

    #[test]
    fn row_actions_step_into_stale_note_row() {
        let document = document_with_one_hunk_and_one_stale_note();
        let mut app = TuiApp::new(document, ViewportSpec::new(80, 20));
        let stale_row_id = app
            .document()
            .stream
            .rows
            .last()
            .map(|row| row.id.clone())
            .expect("stale row present");

        while app.cursor().row_id.as_ref() != Some(&stale_row_id) {
            app.handle_action(TuiAction::RowDown);
        }

        assert_eq!(app.cursor().row_id, Some(stale_row_id));
        assert!(app.current_row_is_visible());
    }

    #[test]
    fn next_note_hunk_action_skips_stale_note_row() {
        let document = document_with_one_hunk_and_one_stale_note();
        let mut app = TuiApp::new(document, ViewportSpec::new(80, 20));
        app.cursor = CursorState::at_row(RowId::new("row:0000"));

        app.handle_action(TuiAction::NextNoteHunk);

        assert_eq!(app.cursor().row_id.as_ref(), Some(&RowId::new("row:0003")));
    }

    #[test]
    fn resize_recomputes_layout_and_keeps_cursor_visible() {
        let mut app = app_with_two_hunks(ViewportSpec::new(80, 20));
        app.cursor = CursorState::at_row(RowId::new("row:0005"));
        let original_cursor = app.cursor().row_id.clone();

        app.handle_action(TuiAction::Resize(ViewportSpec::new(80, 4)));

        assert_eq!(app.cursor().row_id, original_cursor);
        assert_eq!(app.viewport(), ViewportSpec::new(80, 4));
        assert!(app.current_row_is_visible());
    }

    #[test]
    fn quit_action_sets_quit_state() {
        let mut app = app_with_two_hunks(ViewportSpec::new(80, 3));

        app.handle_action(TuiAction::Quit);

        assert!(app.should_quit());
    }

    #[test]
    fn reload_with_preserves_cursor_when_row_id_still_exists() {
        let document = document_with_two_hunks_and_one_note();
        let mut app = TuiApp::new(document.clone(), ViewportSpec::new(80, 20));
        app.cursor = CursorState::at_row(RowId::new("row:0005"));
        let prior_row_id = app.cursor().row_id.clone();

        app.reload_with(document);

        assert_eq!(app.cursor().row_id, prior_row_id);
    }

    #[test]
    fn reload_with_snaps_to_same_file_and_hunk_when_row_id_churns() {
        let prior = document_with_two_hunks_and_one_note();
        let mut app = TuiApp::new(prior, ViewportSpec::new(80, 20));
        app.cursor = CursorState::at_row(RowId::new("row:0004"));
        let new_document = document_with_two_hunks_and_one_note_with_row_id_churn();

        app.reload_with(new_document);

        assert_eq!(app.cursor().row_id, Some(RowId::new("row:1003")));
    }

    #[test]
    fn reload_with_snaps_to_first_row_when_file_removed() {
        let prior = document_with_two_hunks_and_one_note_in_other_file();
        let mut app = TuiApp::new(prior, ViewportSpec::new(80, 20));
        app.cursor = CursorState::at_row(RowId::new("row:0004"));
        let new_document = document_with_one_hunk_and_one_note();

        app.reload_with(new_document);

        assert_eq!(
            app.cursor().row_id,
            app.document().stream.rows.first().map(|row| row.id.clone())
        );
    }

    #[test]
    fn reload_with_yields_empty_cursor_when_new_stream_is_empty() {
        let prior = document_with_two_hunks_and_one_note();
        let mut app = TuiApp::new(prior, ViewportSpec::new(80, 20));

        app.reload_with(empty_document());

        assert!(app.cursor().row_id.is_none());
    }

    fn document_with_one_hunk_and_one_note() -> DumpDocument {
        let review_id = ReviewId::new("review:test");
        let object_id = ObjectId::new("snapshot:test");
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
            object_id.clone(),
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
            object_id,
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

    fn document_with_one_hunk_and_one_stale_note() -> DumpDocument {
        let mut document = document_with_one_hunk_and_one_note();
        let file_id = FileId::new("src/lib.rs");
        let stale_row_ordinal = document.stream.rows.len();
        document.stream.rows.push(ReviewRow {
            id: RowId::new(format!("row:{stale_row_ordinal:04}")),
            ordinal: stale_row_ordinal,
            file_id: Some(file_id.clone()),
            hunk_id: None,
            kind: ReviewRowKind::StaleNote {
                note_id: ReviewNoteId::new("note:stale"),
                title: "Stale anchor".to_owned(),
                resolution_status: ResolutionStatus::Stale,
                target_path: "src/lib.rs".to_owned(),
                target_line_range: LineRange::new(99, 99),
            },
        });
        document.notes.push(ReviewNote {
            id: ReviewNoteId::new("note:stale"),
            anchor: Anchor {
                file_id,
                side: Side::New,
                line_range: LineRange::new(99, 99),
                hunk_signature: "hunk:stale".to_owned(),
                target_text_hash: String::new(),
                status: ResolutionStatus::Stale,
            },
            source: ReviewNoteSource::Sidecar,
            title: "Stale anchor".to_owned(),
            body: None,
            tags: Vec::new(),
            confidence: None,
            external_source: None,
            author: None,
            created_at: None,
        });
        document
    }

    fn app_with_two_hunks(viewport: ViewportSpec) -> TuiApp {
        TuiApp::new(document_with_two_hunks_and_one_note(), viewport)
    }

    fn empty_document() -> DumpDocument {
        let review_id = ReviewId::new("review:empty");
        DumpDocument::new(
            DumpInputSummary {
                source: DumpInputSource::None,
            },
            DiffSnapshot::empty(review_id.clone()),
            Vec::new(),
            ReviewStream::empty(review_id),
            Vec::new(),
        )
    }

    fn document_with_two_hunks_and_one_note() -> DumpDocument {
        let mut document = document_with_one_hunk_and_one_note();
        let review_id = document.stream.review_id.clone();
        let object_id = document.stream.object_id.clone();
        let file_id = FileId::new("src/lib.rs");
        let second_hunk_id = HunkId::new("hunk:0001");
        let second_diff_row = DiffRow {
            kind: DiffRowKind::Added,
            old_line: None,
            new_line: Some(4),
            text: "pub fn second() {}".to_owned(),
        };

        document.stream = ReviewStream {
            review_id,
            object_id,
            rows: vec![
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
                    hunk_id: Some(HunkId::new("hunk:0000")),
                    kind: ReviewRowKind::HunkHeader {
                        header: "@@ -0,0 +1,1 @@".to_owned(),
                    },
                },
                ReviewRow {
                    id: RowId::new("row:0002"),
                    ordinal: 2,
                    file_id: Some(file_id.clone()),
                    hunk_id: Some(HunkId::new("hunk:0000")),
                    kind: ReviewRowKind::Diff {
                        row: DiffRow {
                            kind: DiffRowKind::Added,
                            old_line: None,
                            new_line: Some(1),
                            text: "pub fn example() {}".to_owned(),
                        },
                    },
                },
                ReviewRow {
                    id: RowId::new("row:0003"),
                    ordinal: 3,
                    file_id: Some(file_id.clone()),
                    hunk_id: Some(second_hunk_id.clone()),
                    kind: ReviewRowKind::HunkHeader {
                        header: "@@ -3,0 +4,1 @@".to_owned(),
                    },
                },
                ReviewRow {
                    id: RowId::new("row:0004"),
                    ordinal: 4,
                    file_id: Some(file_id.clone()),
                    hunk_id: Some(second_hunk_id.clone()),
                    kind: ReviewRowKind::Diff {
                        row: second_diff_row,
                    },
                },
                ReviewRow {
                    id: RowId::new("row:0005"),
                    ordinal: 5,
                    file_id: Some(file_id),
                    hunk_id: Some(second_hunk_id),
                    kind: ReviewRowKind::Note {
                        note_id: ReviewNoteId::new("note:test"),
                        target_row_id: RowId::new("row:0004"),
                        title: "Second hunk note".to_owned(),
                    },
                },
            ],
        };
        document
    }

    fn document_with_two_hunks_and_one_note_with_row_id_churn() -> DumpDocument {
        let mut document = document_with_two_hunks_and_one_note();
        for (index, row) in document.stream.rows.iter_mut().enumerate() {
            row.id = RowId::new(format!("row:{:04}", index + 1000));
        }
        document
    }

    fn document_with_two_hunks_and_one_note_in_other_file() -> DumpDocument {
        let mut document = document_with_two_hunks_and_one_note();
        let file_id = FileId::new("src/other.rs");

        document.snapshot.files[0].id = file_id.clone();
        document.snapshot.files[0].old_path = Some("src/other.rs".to_owned());
        document.snapshot.files[0].new_path = Some("src/other.rs".to_owned());
        if let Some(note) = document.notes.first_mut() {
            note.anchor.file_id = file_id.clone();
        }
        for row in &mut document.stream.rows {
            row.file_id = Some(file_id.clone());
        }

        document
    }
}
