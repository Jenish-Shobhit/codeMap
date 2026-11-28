use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::{App, Focus, Input, InputKind, Location, Outcome, View};
use crate::open::Opener;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    Redraw,
    Quit,
    Edit(Location),
    Open(Opener, Location),
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
            if app.view == View::Files && app.files.focus == Focus::Body {
                app.files.focus = Focus::Rail;
                return Action::Redraw;
            }
            return quit(app);
        }
        KeyCode::Char('?') => {
            app.show_help = true;
            return Action::Redraw;
        }
        KeyCode::Char(c @ '1'..='9') => {
            let Some(&view) = app.view.tabs().get(c as usize - '1' as usize) else {
                return Action::None;
            };
            app.view = view;
            if view == View::History
                && app.history.detail.is_none()
                && !app.history.commits.is_empty()
            {
                app.history_move(0);
            }
            return Action::Redraw;
        }
        KeyCode::Char('e') if !ctrl => return edit(app),
        KeyCode::Char('E') => return open(app, Opener::VsCode),
        KeyCode::Char('b') if !ctrl => return open(app, Opener::Browser),
        KeyCode::Char('B') => return open(app, Opener::GitHub),
        KeyCode::Char('y') if !ctrl => {
            return match app.location() {
                Some(Location {
                    path,
                    line: Some(line),
                    ..
                }) => Action::Copy(format!("{path}:{line}")),
                Some(Location { path, .. }) => Action::Copy(path),
                None => Action::None,
            };
        }
        KeyCode::Char('/') if app.view == View::Files => {
            app.start_filter();
            app.input = Some(Input {
                kind: InputKind::Filter,
                text: String::new(),
            });
            return Action::Redraw;
        }
        KeyCode::Char('p') if !ctrl => return Action::Pin("split"),
        KeyCode::Char('T') => return Action::Pin("tab"),
        KeyCode::Char('s') if !ctrl && app.view != View::History => {
            app.cycle_scope();
            return Action::Redraw;
        }
        KeyCode::Char('a') if !ctrl && app.view != View::History => {
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
        View::Files => files_key(app, key),
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

fn edit(app: &App) -> Action {
    app.location().map(Action::Edit).unwrap_or(Action::None)
}

fn open(app: &App, opener: Opener) -> Action {
    match app.location() {
        Some(location) => Action::Open(opener, location),
        None => Action::None,
    }
}

fn toggle_focus(app: &mut App) {
    let focus = match app.view {
        View::Files => &mut app.files.focus,
        View::Changes => &mut app.changes.focus,
        View::History => return,
    };
    *focus = match *focus {
        Focus::Body => Focus::Rail,
        Focus::Rail => Focus::Body,
    };
}

fn input_key(app: &mut App, key: KeyEvent) -> Action {
    let Some(input) = app.input.as_mut() else {
        return Action::None;
    };
    let filtering = input.kind == InputKind::Filter;
    match key.code {
        KeyCode::Esc => {
            app.input = None;
            if filtering {
                app.finish_filter(false);
            }
        }
        KeyCode::Enter => match input.kind.clone() {
            InputKind::Filter => {
                app.input = None;
                app.finish_filter(true);
            }
            InputKind::Comment {
                path,
                start,
                end,
                hunk,
            } => {
                let text = input.text.clone();
                app.input = None;
                app.add_comment(path, start, end, hunk, text);
            }
        },
        KeyCode::Up if filtering => app.files_move(-1),
        KeyCode::Down if filtering => app.files_move(1),
        KeyCode::Backspace => {
            input.text.pop();
            let text = input.text.clone();
            if filtering {
                app.update_filter(&text);
            }
        }
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                if c == 'u' {
                    input.text.clear();
                }
            } else {
                input.text.push(c);
            }
            let text = input.text.clone();
            if filtering {
                app.update_filter(&text);
            }
        }
        _ => return Action::None,
    }
    Action::Redraw
}

fn files_key(app: &mut App, key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if app.files.focus == Focus::Body {
        match key.code {
            KeyCode::Char('d') if ctrl => app.preview_scroll_by(15),
            KeyCode::Char('u') if ctrl => app.preview_scroll_by(-15),
            KeyCode::Up | KeyCode::Char('k') => app.preview_scroll_by(-1),
            KeyCode::Down | KeyCode::Char('j') => app.preview_scroll_by(1),
            KeyCode::PageDown | KeyCode::Char(' ') => app.preview_scroll_by(30),
            KeyCode::PageUp => app.preview_scroll_by(-30),
            KeyCode::Char('g') | KeyCode::Home => app.files.preview_scroll = 0,
            KeyCode::Char('G') | KeyCode::End => app.preview_scroll_by(i32::MAX / 2),
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Backspace => {
                app.files.focus = Focus::Rail
            }
            KeyCode::Enter => return edit(app),
            _ => return Action::None,
        }
        return Action::Redraw;
    }
    match key.code {
        KeyCode::Char('d') if ctrl => app.files_move(15),
        KeyCode::Char('u') if ctrl => app.files_move(-15),
        KeyCode::Up | KeyCode::Char('k') => app.files_move(-1),
        KeyCode::Down | KeyCode::Char('j') => app.files_move(1),
        KeyCode::PageDown => app.files_move(30),
        KeyCode::PageUp => app.files_move(-30),
        KeyCode::Char('g') | KeyCode::Home => app.files_move(-(i32::MAX / 2)),
        KeyCode::Char('G') | KeyCode::End => app.files_move(i32::MAX / 2),
        KeyCode::Right | KeyCode::Char('l') => app.files_right(),
        KeyCode::Left | KeyCode::Char('h') => app.files_left(),
        KeyCode::Enter => {
            if app.selected_file_row().is_some_and(|r| !r.dir) {
                return edit(app);
            }
            app.files_toggle();
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
