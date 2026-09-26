//! Keys. The popup receives every key, so nothing reaches herdr or the agent.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::{App, Focus, InputKind, Outcome, View};

/// What the event loop should do after a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    Redraw,
    Quit,
    /// Leave the terminal to the editor, then come back.
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
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter) {
            app.show_help = false;
        }
        return Action::Redraw;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    // Global keys.
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
        KeyCode::Char('1') if app.view == View::History => {
            app.map_of_commit();
            return Action::Redraw;
        }
        KeyCode::Char('1') => {
            if app.view == View::Flow {
                // show the charted function on the map
                if let Some(t) = app.flow.target.clone() {
                    app.reveal(&t.file, Some(t.idx));
                }
            }
            app.view = View::Map;
            return Action::Redraw;
        }
        KeyCode::Char('2') => {
            if app.view == View::Map {
                if let Some(f) = app.current_function() {
                    app.open_flow(f);
                }
            }
            app.view = View::Flow;
            return Action::Redraw;
        }
        KeyCode::Char('3') => {
            app.view = View::Changes;
            return Action::Redraw;
        }
        KeyCode::Char('4') => {
            app.view = View::History;
            if app.history.detail.is_none() && !app.history.commits.is_empty() {
                app.history_move(0);
            }
            return Action::Redraw;
        }
        KeyCode::Char('/') => {
            app.start_search();
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
        KeyCode::Char('s') if !ctrl && matches!(app.view, View::Changes | View::Map | View::Flow) => {
            app.cycle_scope();
            return Action::Redraw;
        }
        KeyCode::Char('a') if !ctrl && matches!(app.view, View::Changes | View::Map) => {
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
        View::Map => map_key(app, key),
        View::Flow => flow_key(app, key),
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
    let f = match app.view {
        View::Map => &mut app.map.focus,
        View::Flow => &mut app.flow.focus,
        View::Changes => &mut app.changes.focus,
        View::History => return,
    };
    *f = if *f == Focus::Body { Focus::Rail } else { Focus::Body };
    if app.view == View::Flow && app.flow.focus == Focus::Rail {
        let cur = app.flow.target.as_ref().map(|t| t.idx);
        app.flow.rail_sel = app
            .flow_rail_functions()
            .iter()
            .position(|(i, _)| Some(*i) == cur)
            .unwrap_or(0);
    }
}

fn input_key(app: &mut App, key: KeyEvent) -> Action {
    let Some(input) = app.input.as_mut() else { return Action::None };
    match key.code {
        KeyCode::Esc => {
            app.input = None;
            app.search_hits.clear();
        }
        KeyCode::Enter => match input.kind.clone() {
            InputKind::Search => app.accept_search(),
            InputKind::Comment { path, start, end, hunk } => {
                let text = input.text.clone();
                app.input = None;
                app.add_comment(path, start, end, hunk, text);
            }
        },
        KeyCode::Backspace => {
            input.text.pop();
            if input.kind == InputKind::Search {
                app.update_search();
            }
        }
        KeyCode::Up if input.kind == InputKind::Search => {
            app.search_sel = app.search_sel.saturating_sub(1);
        }
        KeyCode::Down if input.kind == InputKind::Search => {
            if app.search_sel + 1 < app.search_hits.len() {
                app.search_sel += 1;
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
            if input.kind == InputKind::Search {
                app.update_search();
            }
        }
        _ => return Action::None,
    }
    Action::Redraw
}

fn map_key(app: &mut App, key: KeyEvent) -> Action {
    if app.map.focus == Focus::Rail {
        let n = app.marks.changed_symbols(&app.index).len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => app.map.rail_sel = app.map.rail_sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                if app.map.rail_sel + 1 < n {
                    app.map.rail_sel += 1;
                }
            }
            KeyCode::Enter => {
                if let Some((file, idx, _)) = app.marks.changed_symbols(&app.index).get(app.map.rail_sel).cloned() {
                    app.reveal(&file, Some(idx));
                    app.map.focus = Focus::Body;
                }
            }
            _ => return Action::None,
        }
        return Action::Redraw;
    }
    match key.code {
        KeyCode::Left | KeyCode::Char('h') => app.map_move(-1, 0),
        KeyCode::Right | KeyCode::Char('l') => app.map_move(1, 0),
        KeyCode::Up => app.map_move(0, -1),
        KeyCode::Down => app.map_move(0, 1),
        KeyCode::Char('j') => app.map_row(1),
        KeyCode::Char('k') => app.map_row(-1),
        KeyCode::Enter => app.map_enter(),
        KeyCode::Backspace | KeyCode::Delete => app.map_back(),
        KeyCode::Char(']') | KeyCode::PageDown => app.map_page(1),
        KeyCode::Char('[') | KeyCode::PageUp => app.map_page(-1),
        _ => return Action::None,
    }
    Action::Redraw
}

fn flow_key(app: &mut App, key: KeyEvent) -> Action {
    if app.flow.focus == Focus::Rail {
        let fns = app.flow_rail_functions();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => app.flow.rail_sel = app.flow.rail_sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                if app.flow.rail_sel + 1 < fns.len() {
                    app.flow.rail_sel += 1;
                }
            }
            KeyCode::Enter => {
                if let (Some((idx, _)), Some(t)) = (fns.get(app.flow.rail_sel), app.flow.target.clone()) {
                    app.open_flow(crate::index::SymId { file: t.file, idx: *idx });
                    app.flow.focus = Focus::Body;
                }
            }
            _ => return Action::None,
        }
        return Action::Redraw;
    }
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => app.flow_move(-1),
        KeyCode::Down | KeyCode::Char('j') => app.flow_move(1),
        KeyCode::Left | KeyCode::Char('h') => app.flow_side(-1),
        KeyCode::Right | KeyCode::Char('l') => app.flow_side(1),
        KeyCode::Char('g') | KeyCode::Home => app.flow.selected = 0,
        KeyCode::Char('G') | KeyCode::End => {
            if let Some(l) = &app.flow.layout {
                app.flow.selected = l.boxes.len().saturating_sub(1);
            }
        }
        KeyCode::Enter => app.flow_enter(),
        KeyCode::Backspace | KeyCode::Delete => app.flow_back(),
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
            KeyCode::Up | KeyCode::Char('k') => app.history.diff_scroll = app.history.diff_scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => app.history.diff_scroll += 1,
            KeyCode::Char('d') if ctrl => app.history.diff_scroll += 15,
            KeyCode::Char('u') if ctrl => app.history.diff_scroll = app.history.diff_scroll.saturating_sub(15),
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
