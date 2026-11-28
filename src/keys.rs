use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::{App, Focus, InputKind, Outcome, View};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    Redraw,
    Quit,
    Edit(std::path::PathBuf, u32),
    Copy(String),
    Pin(&'static str),
}

pub fn handle(app: &mut App, key: KeyEvent) -> Action {
    if key.kind == KeyEventKind::Release {
        return Action::None;
    }
    let had_message = app.message.take().is_some();
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        app.outcome = Some(Outcome::Quit);
        return Action::Quit;
    }
    if app.input.is_some() {
        return input_key(app, key);
    }
    if app.show_help {
        if matches!(
            key.code,
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter
        ) {
            app.show_help = false;
        }
        return Action::Redraw;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('q') if !ctrl => return quit(app),
        KeyCode::Esc => {
            if app.view == View::History && app.history.show_diff {
                app.history.show_diff = false;
                return Action::Redraw;
            }
            if app.view == View::Changes && app.changes.anchor.is_some() {
                app.changes.anchor = None;
                return Action::Redraw;
            }
            return quit(app);
        }
        KeyCode::Char('?') => {
            app.show_help = true;
            return Action::Redraw;
        }
        KeyCode::Char('1') => {
            app.view = View::Changes;
            return Action::Redraw;
        }
        KeyCode::Char('2') => {
            app.view = View::History;
            if app.history.detail.is_none() && !app.history.commits.is_empty() {
                app.history_move(0);
            }
            return Action::Redraw;
        }
        KeyCode::Char('e') if !ctrl => {
            return match app.location() {
                Some((path, line)) => Action::Edit(app.root.join(path), line),
                None => Action::None,
            };
        }
        KeyCode::Char('y') if !ctrl => {
            return match app.location() {
                Some((path, line)) => Action::Copy(format!("{path}:{line}")),
                None => Action::None,
            };
        }
        KeyCode::Char('p') if !ctrl => return Action::Pin("split"),
        KeyCode::Char('T') => return Action::Pin("tab"),
        KeyCode::Char('s') if !ctrl && app.view == View::Changes => {
            app.cycle_scope();
            return Action::Redraw;
        }
        KeyCode::Char('a') if !ctrl && app.view == View::Changes => {
            app.next_agent();
            return Action::Redraw;
        }
        KeyCode::Tab | KeyCode::BackTab => {
            toggle_focus(app);
            return Action::Redraw;
        }
        _ => {}
    }
    let r = match app.view {
        View::Changes => changes_key(app, key),
        View::History => history_key(app, key),
    };
    if r == Action::None && had_message {
        Action::Redraw
    } else {
        r
    }
}

fn quit(app: &mut App) -> Action {
    app.outcome = Some(Outcome::Quit);
    Action::Quit
}

fn toggle_focus(app: &mut App) {
    if app.view != View::Changes {
        return;
    }
    app.changes.focus = match app.changes.focus {
        Focus::Body => Focus::Rail,
        Focus::Rail => Focus::Body,
    };
}

fn input_key(app: &mut App, key: KeyEvent) -> Action {
    let Some(input) = app.input.as_mut() else {
        return Action::None;
    };
    match key.code {
        KeyCode::Esc => app.input = None,
        KeyCode::Enter => {
            let InputKind::Comment {
                path,
                start,
                end,
                hunk,
            } = input.kind.clone();
            let text = input.text.clone();
            app.input = None;
            app.add_comment(path, start, end, hunk, text);
        }
        KeyCode::Backspace => {
            input.text.pop();
        }
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                if c == 'u' {
                    input.text.clear();
                }
            } else {
                input.text.push(c);
            }
        }
        _ => return Action::None,
    }
    Action::Redraw
}

fn changes_key(app: &mut App, key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if app.changes.focus == Focus::Rail {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => app.next_file(-1),
            KeyCode::Down | KeyCode::Char('j') => app.next_file(1),
            KeyCode::Enter => app.changes.focus = Focus::Body,
            _ => return Action::None,
        }
        return Action::Redraw;
    }
    match key.code {
        KeyCode::Char('d') if ctrl => app.changes_move(15),
        KeyCode::Char('u') if ctrl => app.changes_move(-15),
        KeyCode::Up | KeyCode::Char('k') => app.changes_move(-1),
        KeyCode::Down | KeyCode::Char('j') => app.changes_move(1),
        KeyCode::PageDown => app.changes_move(30),
        KeyCode::PageUp => app.changes_move(-30),
        KeyCode::Char('g') | KeyCode::Home => app.changes.row = 0,
        KeyCode::Char('G') | KeyCode::End => app.changes_move(i32::MAX / 2),
        KeyCode::Char(']') => app.next_hunk(1),
        KeyCode::Char('[') => app.next_hunk(-1),
        KeyCode::Char('}') => app.next_file(1),
        KeyCode::Char('{') => app.next_file(-1),
        KeyCode::Char(' ') => app.toggle_reviewed(),
        KeyCode::Char('v') => {
            app.changes.anchor = match app.changes.anchor {
                Some(_) => None,
                None => Some(app.changes.row),
            }
        }
        KeyCode::Char('c') => app.start_comment(),
        KeyCode::Char('x') => app.delete_comment_at_cursor(),
        KeyCode::Char('P') => {
            if let Err(e) = app.send_draft(false) {
                app.message = Some(e);
            }
            if app.outcome.is_some() {
                return Action::Quit;
            }
        }
        KeyCode::Char('S') => {
            if let Err(e) = app.send_draft(true) {
                app.message = Some(e);
            }
            if app.outcome.is_some() {
                return Action::Quit;
            }
        }
        _ => return Action::None,
    }
    Action::Redraw
}

fn history_key(app: &mut App, key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if app.history.show_diff {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                app.history.diff_scroll = app.history.diff_scroll.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => app.history.diff_scroll += 1,
            KeyCode::Char('d') if ctrl => app.history.diff_scroll += 15,
            KeyCode::Char('u') if ctrl => {
                app.history.diff_scroll = app.history.diff_scroll.saturating_sub(15)
            }
            KeyCode::PageDown | KeyCode::Char(' ') => app.history.diff_scroll += 30,
            KeyCode::PageUp => app.history.diff_scroll = app.history.diff_scroll.saturating_sub(30),
            KeyCode::Backspace | KeyCode::Delete | KeyCode::Left => app.history.show_diff = false,
            KeyCode::Enter => app.history.show_diff = false,
            _ => return Action::None,
        }
        return Action::Redraw;
    }
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => app.history_move(-1),
        KeyCode::Down | KeyCode::Char('j') => app.history_move(1),
        KeyCode::Char('d') if ctrl => app.history_move(15),
        KeyCode::Char('u') if ctrl => app.history_move(-15),
        KeyCode::PageDown => app.history_move(30),
        KeyCode::PageUp => app.history_move(-30),
        KeyCode::Char('g') | KeyCode::Home => app.history_move(-(i32::MAX / 2)),
        KeyCode::Char('G') | KeyCode::End => app.history_move(i32::MAX / 2),
        KeyCode::Enter | KeyCode::Right => app.history_open(),
        _ => return Action::None,
    }
    Action::Redraw
}
