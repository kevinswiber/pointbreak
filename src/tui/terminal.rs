use std::error::Error;
use std::io::{self, stdout};
use std::path::Path;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use shore::dump::DumpDocument;
use shore::session::reload_session;
use shore::stream::ViewportSpec;

use crate::tui::app::{TuiAction, TuiApp};
use crate::tui::render::render;

type TerminalResult<T> = Result<T, Box<dyn Error>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TerminalAction {
    App(TuiAction),
    Reload,
}

impl From<TuiAction> for TerminalAction {
    fn from(action: TuiAction) -> Self {
        Self::App(action)
    }
}

pub(crate) fn run<F>(mut app: TuiApp, repo: &Path, load_document: F) -> TerminalResult<()>
where
    F: Fn() -> shore::error::Result<DumpDocument>,
{
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    let initial_size = terminal.size()?;
    app.handle_action(TuiAction::Resize(ViewportSpec::new(
        initial_size.width as usize,
        initial_size.height as usize,
    )));

    while !app.should_quit() {
        terminal.draw(|frame| render(frame, &app))?;
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }

        match event::read()? {
            Event::Key(key) => {
                app.clear_last_reload_error();
                if let Some(action) = action_for_key(key) {
                    match action {
                        TerminalAction::Reload => match reload_session(repo, &load_document) {
                            Ok(outcome) => {
                                app.reload_with(outcome.document);
                            }
                            Err(error) => {
                                app.set_last_reload_error(format!("reload failed: {error}"));
                            }
                        },
                        TerminalAction::App(app_action) => app.handle_action(app_action),
                    }
                }
            }
            Event::Resize(width, height) => {
                app.handle_action(TuiAction::Resize(ViewportSpec::new(
                    width as usize,
                    height as usize,
                )));
            }
            _ => {}
        }
    }

    Ok(())
}

fn action_for_key(key: KeyEvent) -> Option<TerminalAction> {
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }

    match key.code {
        KeyCode::Esc => Some(TuiAction::Quit.into()),
        KeyCode::Up => Some(TuiAction::RowUp.into()),
        KeyCode::Down => Some(TuiAction::RowDown.into()),
        KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
            Some(TuiAction::Quit.into())
        }
        KeyCode::Char('q') if is_unmodified(key.modifiers) => Some(TuiAction::Quit.into()),
        KeyCode::Char('j') if is_unmodified(key.modifiers) => Some(TuiAction::RowDown.into()),
        KeyCode::Char('k') if is_unmodified(key.modifiers) => Some(TuiAction::RowUp.into()),
        KeyCode::Char('r') if is_unmodified(key.modifiers) => Some(TerminalAction::Reload),
        KeyCode::Char(']') if allows_shift(key.modifiers) => Some(TuiAction::NextHunk.into()),
        KeyCode::Char('[') if allows_shift(key.modifiers) => Some(TuiAction::PreviousHunk.into()),
        KeyCode::Char('}') if allows_shift(key.modifiers) => Some(TuiAction::NextNoteHunk.into()),
        KeyCode::Char('{') if allows_shift(key.modifiers) => {
            Some(TuiAction::PreviousNoteHunk.into())
        }
        _ => None,
    }
}

fn is_unmodified(modifiers: KeyModifiers) -> bool {
    modifiers.is_empty()
}

fn allows_shift(modifiers: KeyModifiers) -> bool {
    modifiers.is_empty() || modifiers == KeyModifiers::SHIFT
}

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> TerminalResult<Self> {
        enable_raw_mode()?;
        if let Err(error) = execute!(stdout(), EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(Box::new(error));
        }
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{TerminalAction, TerminalResult, action_for_key, run};
    use crate::tui::app::TuiAction;

    #[test]
    fn key_events_map_to_quit_actions() {
        assert_eq!(action_for_key(key_char('q')), Some(TuiAction::Quit.into()));
        assert_eq!(
            action_for_key(key_code(KeyCode::Esc)),
            Some(TuiAction::Quit.into())
        );
        assert_eq!(
            action_for_key(key_code_with_modifiers(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL
            )),
            Some(TuiAction::Quit.into())
        );
    }

    #[test]
    fn key_events_map_to_row_actions() {
        assert_eq!(
            action_for_key(key_char('j')),
            Some(TuiAction::RowDown.into())
        );
        assert_eq!(
            action_for_key(key_code(KeyCode::Down)),
            Some(TuiAction::RowDown.into())
        );
        assert_eq!(action_for_key(key_char('k')), Some(TuiAction::RowUp.into()));
        assert_eq!(
            action_for_key(key_code(KeyCode::Up)),
            Some(TuiAction::RowUp.into())
        );
    }

    #[test]
    fn key_events_map_to_hunk_actions() {
        assert_eq!(
            action_for_key(key_char(']')),
            Some(TuiAction::NextHunk.into())
        );
        assert_eq!(
            action_for_key(key_char('[')),
            Some(TuiAction::PreviousHunk.into())
        );
        assert_eq!(
            action_for_key(key_char('}')),
            Some(TuiAction::NextNoteHunk.into())
        );
        assert_eq!(
            action_for_key(key_char('{')),
            Some(TuiAction::PreviousNoteHunk.into())
        );
    }

    #[test]
    fn key_events_map_to_reload_actions() {
        assert_eq!(action_for_key(key_char('r')), Some(TerminalAction::Reload));
    }

    #[test]
    fn unrelated_key_events_are_ignored() {
        assert_eq!(action_for_key(key_char('x')), None);
        assert_eq!(
            action_for_key(key_code_with_modifiers(
                KeyCode::Char('q'),
                KeyModifiers::CONTROL
            )),
            None
        );
    }

    #[test]
    fn run_has_terminal_result_signature() {
        fn _smoke(app: crate::tui::app::TuiApp, repo: &std::path::Path) -> TerminalResult<()> {
            run(
                app,
                repo,
                || -> shore::error::Result<shore::dump::DumpDocument> {
                    unreachable!("compile-only smoke")
                },
            )
        }

        let _run: fn(crate::tui::app::TuiApp, &std::path::Path) -> TerminalResult<()> = _smoke;
    }

    fn key_char(ch: char) -> KeyEvent {
        key_code(KeyCode::Char(ch))
    }

    fn key_code(code: KeyCode) -> KeyEvent {
        key_code_with_modifiers(code, KeyModifiers::NONE)
    }

    fn key_code_with_modifiers(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }
}
