use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use shore::dump::{CurrentVerdictStatusName, DumpDocument, DumpInputSource};
use shore::model::{ReviewNoteId, ReviewRow, ReviewRowKind};
use shore::session::ReloadDiagnosticCode;
use shore::session::event::{AcknowledgementNextAction, VerdictDecision};
use shore::sidecar::ReviewNotesDiagnosticCode;

use crate::tui::app::TuiApp;
use crate::tui::view::{DisplayRow, DisplayRowKind};

pub(crate) fn render(frame: &mut Frame<'_>, app: &TuiApp) {
    let area = frame.area();
    if let Some(status_line) = review_status_line(app.document()) {
        let shell = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(area);

        frame.render_widget(header(app.document()), shell[0]);
        frame.render_widget(status_line, shell[1]);
        render_body(frame, app, shell[2]);
        frame.render_widget(footer(app), shell[3]);
    } else {
        let shell = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(area);

        frame.render_widget(header(app.document()), shell[0]);
        render_body(frame, app, shell[1]);
        frame.render_widget(footer(app), shell[2]);
    }
}

fn render_body(frame: &mut Frame<'_>, app: &TuiApp, area: Rect) {
    if area.width < 80 {
        render_stream(frame, app, area);
    } else {
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
            .split(area);
        render_stream(frame, app, panes[0]);
        render_detail(frame, app, panes[1]);
    }
}

fn header(document: &DumpDocument) -> Paragraph<'static> {
    Paragraph::new(format!(
        "shore show | {} | files {} hunks {} rows {} notes {} diagnostics {}",
        input_source(&document.input.source),
        document.summary.file_count,
        document.summary.hunk_count,
        document.summary.row_count,
        document.summary.note_count,
        document.summary.diagnostic_count
    ))
}

fn footer(app: &TuiApp) -> Paragraph<'static> {
    if let Some(error) = app.last_reload_error() {
        Paragraph::new(error.to_owned())
    } else {
        Paragraph::new("q/Esc quit | j/k rows | [/] hunks | {/} note hunks | r reload")
    }
}

fn review_status_line(document: &DumpDocument) -> Option<Paragraph<'static>> {
    let section = document.review_artifacts.as_ref()?;
    let verdict = match section.current_verdict.status {
        CurrentVerdictStatusName::Resolved => format!(
            "verdict: {}",
            verdict_decision_name(section.current_verdict.decision?)
        ),
        CurrentVerdictStatusName::Ambiguous => format!(
            "verdict: ambiguous ({} candidates)",
            section.current_verdict.review_artifact_ids.len()
        ),
        CurrentVerdictStatusName::None => "verdict: none".to_owned(),
    };
    // Accept and obsolete both resolve the current reviewer feedback loop.
    let resolved_acks = section
        .acknowledgements
        .iter()
        .filter(|ack| {
            matches!(
                ack.next_action,
                AcknowledgementNextAction::Accept | AcknowledgementNextAction::Obsolete
            )
        })
        .count();
    let mut parts = vec![
        verdict,
        format!("acks: {resolved_acks}/{}", section.acknowledgements.len()),
    ];
    if section.summary.unreplaced_verdict_count > 1 {
        parts.push(format!(
            "({} unreplaced)",
            section.summary.unreplaced_verdict_count
        ));
    }
    if let Some(stale_suffix) = stale_status_suffix(document) {
        parts.push(stale_suffix);
    }

    Some(Paragraph::new(parts.join(" | ")))
}

fn stale_status_suffix(document: &DumpDocument) -> Option<String> {
    let entries = &document.reload_diagnostics.as_ref()?.entries;
    let mut stale_note_count = 0;
    let mut stale_verdict_count = 0;
    let mut orphan_acknowledgement_count = 0;

    for entry in entries {
        match entry.code {
            ReloadDiagnosticCode::NoteStale | ReloadDiagnosticCode::NoteOrphaned => {
                stale_note_count += 1;
            }
            ReloadDiagnosticCode::VerdictStale => stale_verdict_count += 1,
            ReloadDiagnosticCode::AcknowledgementOrphan => orphan_acknowledgement_count += 1,
        }
    }

    let mut segments = Vec::new();
    push_segment(&mut segments, stale_note_count, "note", "notes");
    push_segment(&mut segments, stale_verdict_count, "verdict", "verdicts");
    push_segment(
        &mut segments,
        orphan_acknowledgement_count,
        "acknowledgement",
        "acknowledgements",
    );

    (!segments.is_empty()).then(|| format!("stale: {}", segments.join(", ")))
}

fn push_segment(
    segments: &mut Vec<String>,
    count: usize,
    singular: &'static str,
    plural: &'static str,
) {
    if count > 0 {
        segments.push(format!("{} {}", count, pluralize(count, singular, plural)));
    }
}

fn pluralize(count: usize, singular: &'static str, plural: &'static str) -> &'static str {
    if count == 1 { singular } else { plural }
}

fn render_stream(frame: &mut Frame<'_>, app: &TuiApp, area: Rect) {
    let body_height = area.height.saturating_sub(2) as usize;
    let selected_row_id = app.cursor().row_id.as_ref();
    let lines = app
        .document()
        .stream
        .rows
        .iter()
        .skip(app.scroll_top())
        .take(body_height)
        .map(|row| {
            let display = DisplayRow::from_review_row(row);
            let style = if selected_row_id == Some(&row.id) {
                selected_row_style()
            } else {
                row_style(display.kind)
            };
            Line::styled(display_text(&display), style)
        })
        .collect::<Vec<_>>();

    let paragraph = Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).title("Review"));
    frame.render_widget(paragraph, area);
}

fn render_detail(frame: &mut Frame<'_>, app: &TuiApp, area: Rect) {
    let (title, lines) = if let Some((title, body)) = selected_note_detail(app) {
        (
            "Note",
            vec![
                Line::styled(title, Style::default().add_modifier(Modifier::BOLD)),
                Line::from(""),
                Line::from(body.unwrap_or_else(|| "No note body".to_owned())),
            ],
        )
    } else if let Some(summary) = current_verdict_summary(app.document()) {
        (
            "Verdict",
            vec![Line::from(
                summary.unwrap_or_else(|| "no verdict summary".to_owned()),
            )],
        )
    } else if !app.document().diagnostics.is_empty() {
        (
            "Diagnostics",
            app.document()
                .diagnostics
                .iter()
                .map(|diagnostic| {
                    Line::from(format!(
                        "{}: {}",
                        diagnostic_code(&diagnostic.code),
                        diagnostic.message
                    ))
                })
                .collect::<Vec<_>>(),
        )
    } else {
        (
            "Summary",
            vec![
                Line::from(format!("files: {}", app.document().summary.file_count)),
                Line::from(format!("hunks: {}", app.document().summary.hunk_count)),
                Line::from(format!("rows: {}", app.document().summary.row_count)),
                Line::from(format!("notes: {}", app.document().summary.note_count)),
            ],
        )
    };

    let paragraph = Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(paragraph, area);
}

fn selected_note_detail(app: &TuiApp) -> Option<(String, Option<String>)> {
    let selected_row = selected_row(app)?;
    let ReviewRowKind::Note { note_id, title, .. } = &selected_row.kind else {
        return None;
    };
    let body = note_body(app, note_id);
    Some((title.clone(), body))
}

fn selected_row(app: &TuiApp) -> Option<&ReviewRow> {
    let row_id = app.cursor().row_id.as_ref()?;
    app.document()
        .stream
        .rows
        .iter()
        .find(|row| &row.id == row_id)
}

fn note_body(app: &TuiApp, note_id: &ReviewNoteId) -> Option<String> {
    app.document()
        .notes
        .iter()
        .find(|note| &note.id == note_id)
        .and_then(|note| note.body.clone())
}

fn current_verdict_summary(document: &DumpDocument) -> Option<Option<String>> {
    document
        .review_artifacts
        .as_ref()?
        .verdicts
        .last()
        .map(|verdict| verdict.summary.clone())
}

fn display_text(row: &DisplayRow) -> String {
    if row.prefix.is_empty() {
        row.text.clone()
    } else {
        format!("{:<4} {}", row.prefix, row.text)
    }
}

fn row_style(kind: DisplayRowKind) -> Style {
    match kind {
        DisplayRowKind::FileHeader => Style::default().fg(Color::Cyan),
        DisplayRowKind::HunkHeader => Style::default().fg(Color::Yellow),
        DisplayRowKind::Added => Style::default().fg(Color::Green),
        DisplayRowKind::Removed => Style::default().fg(Color::Red),
        DisplayRowKind::Context => Style::default(),
        DisplayRowKind::Metadata => Style::default().fg(Color::Magenta),
        DisplayRowKind::Note => Style::default().fg(Color::LightBlue),
        DisplayRowKind::Empty => Style::default().fg(Color::DarkGray),
    }
}

fn selected_row_style() -> Style {
    Style::default()
        .fg(Color::White)
        .bg(Color::Blue)
        .add_modifier(Modifier::BOLD)
}

fn input_source(source: &DumpInputSource) -> &'static str {
    match source {
        DumpInputSource::None => "no notes",
        DumpInputSource::ReviewNotes => "review notes",
        DumpInputSource::LegacyHunkAgentContext => "legacy hunk",
        DumpInputSource::Durable => "durable",
    }
}

fn diagnostic_code(code: &ReviewNotesDiagnosticCode) -> &'static str {
    match code {
        ReviewNotesDiagnosticCode::InvalidSchema => "invalid_schema",
        ReviewNotesDiagnosticCode::InvalidRange => "invalid_range",
        ReviewNotesDiagnosticCode::MissingFilePath => "missing_file_path",
        ReviewNotesDiagnosticCode::MissingNoteTarget => "missing_note_target",
        ReviewNotesDiagnosticCode::MissingNoteTitle => "missing_note_title",
        ReviewNotesDiagnosticCode::MissingNotes => "missing_notes",
        ReviewNotesDiagnosticCode::MissingVersion => "missing_version",
        ReviewNotesDiagnosticCode::StaleFilePath => "stale_file_path",
        ReviewNotesDiagnosticCode::UnresolvedNote => "unresolved_note",
    }
}

fn verdict_decision_name(decision: VerdictDecision) -> &'static str {
    match decision {
        VerdictDecision::Pass => "pass",
        VerdictDecision::PassMinorNit => "pass_minor_nit",
        VerdictDecision::RequestChanges => "request_changes",
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;
    use shore::dump::{
        AcknowledgementView, CurrentVerdictDumpView, CurrentVerdictStatusName, DumpDocument,
        DumpInputSource, DumpInputSummary, ReloadDiagnosticsSection, ReviewArtifactsSection,
        ReviewArtifactsSummary, VerdictView,
    };
    use shore::model::{
        Anchor, DiffFile, DiffRow, DiffRowKind, DiffSnapshot, FileId, FileStatus, HunkId,
        LineRange, ResolutionStatus, ReviewHunk, ReviewId, ReviewNote, ReviewNoteId,
        ReviewNoteSource, ReviewRow, ReviewRowKind, ReviewStream, RowId, Side, SnapshotId,
    };
    use shore::session::event::{AcknowledgementNextAction, VerdictDecision, Writer};
    use shore::session::{ReloadDiagnostic, ReloadDiagnosticCode};
    use shore::sidecar::{DiagnosticLevel, ReviewNotesDiagnostic, ReviewNotesDiagnosticCode};
    use shore::stream::ViewportSpec;

    use super::render;
    use crate::tui::app::{TuiAction, TuiApp};

    #[test]
    fn render_frame_shows_diff_stream_and_note_detail() {
        let mut app = app_with_note(ViewportSpec::new(100, 20));
        app.handle_action(TuiAction::NextNoteHunk);

        let buffer = render_to_buffer(&app, 100, 20);

        assert!(buffer_contains(&buffer, "src/lib.rs"));
        assert!(buffer_contains(&buffer, "@@"));
        assert!(buffer_contains(
            &buffer,
            "decode_json keeps the error boundary explicit"
        ));
        assert!(buffer_contains(&buffer, "Full review note body"));
        assert!(buffer_contains(&buffer, "q"));
    }

    #[test]
    fn render_frame_shows_diagnostics_when_no_note_is_selected() {
        let app = app_with_diagnostic(ViewportSpec::new(100, 20));

        let buffer = render_to_buffer(&app, 100, 20);

        assert!(buffer_contains(&buffer, "Diagnostics"));
        assert!(buffer_contains(&buffer, "missing_note_title"));
        assert!(buffer_contains(&buffer, "missing title"));
    }

    #[test]
    fn render_frame_marks_selected_row() {
        let app = app_with_note(ViewportSpec::new(100, 20));

        let buffer = render_to_buffer(&app, 100, 20);

        assert_eq!(buffer[(1, 2)].style().bg, Some(Color::Blue));
    }

    #[test]
    fn render_frame_omits_detail_pane_below_eighty_columns() {
        let mut app = app_with_note(ViewportSpec::new(60, 12));
        app.handle_action(TuiAction::NextNoteHunk);

        let buffer = render_to_buffer(&app, 60, 12);

        assert!(buffer_contains(&buffer, "src/lib.rs"));
        assert!(!buffer_contains(&buffer, "Full review note body"));
    }

    #[test]
    fn render_frame_handles_tiny_terminals_without_panic() {
        let app = app_with_note(ViewportSpec::new(20, 4));

        let buffer = render_to_buffer(&app, 20, 4);

        assert_eq!(buffer.area.width, 20);
        assert_eq!(buffer.area.height, 4);
    }

    #[test]
    fn render_frame_shows_verdict_status_line_when_durable_state_present() {
        let document = sample_document_with_verdict(VerdictDecision::Pass, 1, 1);
        let app = TuiApp::new(document, ViewportSpec::new(80, 24));

        let buffer = render_to_buffer(&app, 80, 24);
        let text = buffer_text(&buffer);

        assert!(
            text.contains("verdict: pass"),
            "verdict status line missing; got:\n{text}"
        );
        assert!(
            text.contains("acks: 1/1"),
            "ack ratio missing; got:\n{text}"
        );
    }

    #[test]
    fn render_frame_shows_stale_suffix_when_reload_diagnostics_present() {
        let document = sample_document_with_reload_diagnostics(2, 1, 0);
        let app = TuiApp::new(document, ViewportSpec::new(80, 24));

        let buffer = render_to_buffer(&app, 80, 24);
        let text = buffer_text(&buffer);

        assert!(
            text.contains("stale: 2 notes, 1 verdict"),
            "stale suffix missing; got:\n{text}"
        );
    }

    #[test]
    fn render_frame_hides_stale_suffix_when_diagnostics_empty() {
        let document = sample_document_with_review_artifacts_no_staleness();
        let app = TuiApp::new(document, ViewportSpec::new(80, 24));

        let buffer = render_to_buffer(&app, 80, 24);
        let text = buffer_text(&buffer);

        assert!(
            !text.contains("stale:"),
            "stale suffix should be hidden when no diagnostics"
        );
    }

    #[test]
    fn render_frame_hides_verdict_line_when_no_durable_state() {
        let document = sample_document_without_review_artifacts();
        let app = TuiApp::new(document, ViewportSpec::new(80, 24));

        let buffer = render_to_buffer(&app, 80, 24);
        let text = buffer_text(&buffer);

        assert!(
            !text.contains("verdict:"),
            "verdict line should be hidden when review_artifacts is None"
        );
    }

    #[test]
    fn render_frame_shows_ambiguous_marker_when_two_unreplaced_verdicts() {
        let document = sample_document_with_ambiguous_verdicts(2);
        let app = TuiApp::new(document, ViewportSpec::new(80, 24));

        let buffer = render_to_buffer(&app, 80, 24);
        let text = buffer_text(&buffer);

        assert!(text.contains("ambiguous"), "ambiguous marker missing");
        assert!(text.contains("(2"), "candidate count missing");
    }

    #[test]
    fn render_frame_shows_verdict_summary_in_detail_pane_fall_through() {
        let document = sample_document_with_verdict_summary("ship it");
        let app = TuiApp::new(document, ViewportSpec::new(80, 24));

        let buffer = render_to_buffer(&app, 80, 24);
        let text = buffer_text(&buffer);

        assert!(
            text.contains("ship it"),
            "verdict summary missing from detail pane; got:\n{text}"
        );
    }

    #[test]
    fn render_frame_handles_tiny_terminals_without_panic_with_verdict_line() {
        let document = sample_document_with_verdict(VerdictDecision::Pass, 1, 1);
        let app = TuiApp::new(document, ViewportSpec::new(10, 3));

        let _buffer = render_to_buffer(&app, 10, 3);
    }

    #[test]
    fn render_frame_shows_reload_hint_in_footer() {
        let app = app_with_note(ViewportSpec::new(100, 20));

        let buffer = render_to_buffer(&app, 100, 20);
        let text = buffer_text(&buffer);

        assert!(
            text.contains("r reload"),
            "reload hint missing; got:\n{text}"
        );
    }

    #[test]
    fn render_frame_shows_reload_error_in_footer_when_present() {
        let mut app = app_with_note(ViewportSpec::new(100, 20));
        app.set_last_reload_error("reload failed: boom");

        let buffer = render_to_buffer(&app, 100, 20);
        let text = buffer_text(&buffer);

        assert!(
            text.contains("reload failed: boom"),
            "reload error missing; got:\n{text}"
        );
    }

    fn render_to_buffer(app: &TuiApp, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("create terminal");
        terminal
            .draw(|frame| render(frame, app))
            .expect("draw frame");
        terminal.backend().buffer().clone()
    }

    fn buffer_contains(buffer: &Buffer, needle: &str) -> bool {
        buffer_text(buffer).contains(needle)
    }

    fn buffer_text(buffer: &Buffer) -> String {
        let mut text = String::new();
        for row in 0..buffer.area.height {
            let line = (0..buffer.area.width)
                .map(|column| buffer[(column, row)].symbol())
                .collect::<String>();
            text.push_str(line.trim_end());
            text.push('\n');
        }
        text
    }

    fn app_with_note(viewport: ViewportSpec) -> TuiApp {
        TuiApp::new(document_with_note(Vec::new()), viewport)
    }

    fn app_with_diagnostic(viewport: ViewportSpec) -> TuiApp {
        TuiApp::new(
            document_with_note(vec![ReviewNotesDiagnostic {
                level: DiagnosticLevel::Warning,
                code: ReviewNotesDiagnosticCode::MissingNoteTitle,
                path: "files[0].notes[0].title".to_owned(),
                message: "missing title".to_owned(),
            }]),
            viewport,
        )
    }

    fn document_with_note(diagnostics: Vec<ReviewNotesDiagnostic>) -> DumpDocument {
        let review_id = ReviewId::new("review:test");
        let snapshot_id = SnapshotId::new("snapshot:test");
        let file_id = FileId::new("src/lib.rs");
        let hunk_id = HunkId::new("hunk:0000");
        let note_id = ReviewNoteId::new("note:test");
        let diff_row = DiffRow {
            kind: DiffRowKind::Added,
            old_line: None,
            new_line: Some(9),
            text: "pub fn decode_json() {}".to_owned(),
        };
        let hunk = ReviewHunk {
            id: hunk_id.clone(),
            header: "@@ -8,0 +9,1 @@".to_owned(),
            old_start: 8,
            old_lines: 0,
            new_start: 9,
            new_lines: 1,
            rows: vec![diff_row.clone()],
        };
        let snapshot = DiffSnapshot::new(
            review_id.clone(),
            snapshot_id.clone(),
            vec![DiffFile {
                id: file_id.clone(),
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
                hunks: vec![hunk.clone()],
            }],
        );
        let note = ReviewNote {
            id: note_id.clone(),
            anchor: Anchor {
                file_id: file_id.clone(),
                side: Side::New,
                line_range: LineRange::new(9, 9),
                hunk_signature: hunk.signature(),
                target_text_hash: "sha256:test".to_owned(),
                status: ResolutionStatus::Exact,
            },
            source: ReviewNoteSource::Sidecar,
            title: "decode_json keeps the error boundary explicit".to_owned(),
            body: Some("Full review note body in markdown.".to_owned()),
            tags: Vec::new(),
            confidence: None,
            external_source: None,
            author: Some("reviewer".to_owned()),
            created_at: None,
        };
        let stream = ReviewStream {
            review_id,
            snapshot_id,
            rows: vec![
                ReviewRow {
                    id: RowId::new("row:0000"),
                    ordinal: 0,
                    file_id: Some(file_id.clone()),
                    hunk_id: None,
                    kind: ReviewRowKind::FileHeader {
                        path: "src/lib.rs".to_owned(),
                        status: FileStatus::Modified,
                    },
                },
                ReviewRow {
                    id: RowId::new("row:0001"),
                    ordinal: 1,
                    file_id: Some(file_id.clone()),
                    hunk_id: Some(hunk_id.clone()),
                    kind: ReviewRowKind::HunkHeader {
                        header: hunk.header,
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
                        note_id: note_id.clone(),
                        target_row_id: RowId::new("row:0002"),
                        title: note.title.clone(),
                    },
                },
            ],
        };

        DumpDocument::new(
            DumpInputSummary {
                source: DumpInputSource::ReviewNotes,
            },
            snapshot,
            vec![note],
            stream,
            diagnostics,
        )
    }

    fn sample_document_without_review_artifacts() -> DumpDocument {
        document_with_note(Vec::new())
    }

    fn sample_document_with_verdict(
        decision: VerdictDecision,
        resolved_acks: usize,
        total_acks: usize,
    ) -> DumpDocument {
        let mut document = document_with_note(Vec::new());
        document.review_artifacts = Some(sample_review_artifacts(
            CurrentVerdictStatusName::Resolved,
            Some(decision),
            vec!["artifact:1".to_owned()],
            Some("ship it".to_owned()),
            1,
            resolved_acks,
            total_acks,
        ));
        document
    }

    fn sample_document_with_reload_diagnostics(
        note_count: usize,
        verdict_count: usize,
        acknowledgement_count: usize,
    ) -> DumpDocument {
        let mut document = sample_document_with_verdict(VerdictDecision::Pass, 1, 1);
        let mut entries = Vec::new();
        entries.extend((0..note_count).map(|index| ReloadDiagnostic {
            code: if index % 2 == 0 {
                ReloadDiagnosticCode::NoteStale
            } else {
                ReloadDiagnosticCode::NoteOrphaned
            },
            message: format!("note:{} stale", index + 1),
        }));
        entries.extend((0..verdict_count).map(|index| ReloadDiagnostic {
            code: ReloadDiagnosticCode::VerdictStale,
            message: format!("verdict:{} stale", index + 1),
        }));
        entries.extend((0..acknowledgement_count).map(|index| ReloadDiagnostic {
            code: ReloadDiagnosticCode::AcknowledgementOrphan,
            message: format!("ack:{} orphaned", index + 1),
        }));
        document.reload_diagnostics = Some(ReloadDiagnosticsSection { entries });
        document
    }

    fn sample_document_with_review_artifacts_no_staleness() -> DumpDocument {
        let mut document = sample_document_with_verdict(VerdictDecision::Pass, 1, 1);
        document.reload_diagnostics = None;
        document
    }

    fn sample_document_with_ambiguous_verdicts(candidate_count: usize) -> DumpDocument {
        let mut document = document_with_note(Vec::new());
        let review_artifact_ids = (0..candidate_count)
            .map(|index| format!("artifact:{}", index + 1))
            .collect::<Vec<_>>();
        document.review_artifacts = Some(sample_review_artifacts(
            CurrentVerdictStatusName::Ambiguous,
            None,
            review_artifact_ids,
            Some("needs reviewer choice".to_owned()),
            candidate_count,
            1,
            1,
        ));
        document
    }

    fn sample_document_with_verdict_summary(summary: &str) -> DumpDocument {
        let mut document = document_with_note(Vec::new());
        document.review_artifacts = Some(sample_review_artifacts(
            CurrentVerdictStatusName::Resolved,
            Some(VerdictDecision::Pass),
            vec!["artifact:1".to_owned()],
            Some(summary.to_owned()),
            1,
            1,
            1,
        ));
        document
    }

    fn sample_review_artifacts(
        status: CurrentVerdictStatusName,
        decision: Option<VerdictDecision>,
        review_artifact_ids: Vec<String>,
        verdict_summary: Option<String>,
        unreplaced_verdict_count: usize,
        resolved_acks: usize,
        total_acks: usize,
    ) -> ReviewArtifactsSection {
        ReviewArtifactsSection {
            verdicts: vec![VerdictView {
                id: "artifact:1".to_owned(),
                work_unit_id: "work:default".to_owned(),
                revision_id: "rev:current".to_owned(),
                decision: VerdictDecision::Pass,
                summary: verdict_summary,
                replaces: Vec::new(),
                reviewer: Writer::shore_local_reviewer("0.1.0"),
                replaced: false,
                stale: false,
            }],
            acknowledgements: (0..total_acks)
                .map(|index| AcknowledgementView {
                    id: format!("ack:{}", index + 1),
                    review_artifact_id: "artifact:1".to_owned(),
                    next_action: if index < resolved_acks {
                        AcknowledgementNextAction::Accept
                    } else {
                        AcknowledgementNextAction::Address
                    },
                    reason: Some("ack".to_owned()),
                    acknowledger: Writer::shore_local_author("0.1.0"),
                    stale: false,
                })
                .collect(),
            current_verdict: CurrentVerdictDumpView {
                status,
                decision,
                review_artifact_ids,
            },
            summary: ReviewArtifactsSummary {
                verdict_count: 1,
                acknowledgement_count: total_acks,
                unreplaced_verdict_count,
            },
        }
    }
}
