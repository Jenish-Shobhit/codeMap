use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::{App, Focus, Input, InputKind, Location, Outcome, View};
use crate::commit::Field;
use crate::custom::Scope;
use crate::open::Opener;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    Redraw,
    Quit,
    Edit(Location),
    Open(Opener, Location),
    Copy(String),
    Browse(String),
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
    if app.confirm.is_some() {
        return confirm_key(app, key);
    }
    if app.commit.open {
        return commit_key(app, key);
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
            if app.view == View::Status && app.status.focus == Focus::Body {
                app.status.focus = Focus::Rail;
                return Action::Redraw;
            }
            return quit(app);
        }
        KeyCode::Char('?') => {
            app.show_help = true;
            return Action::Redraw;
        }
        KeyCode::Char(c @ '1'..='9') if app.view == View::Custom => {
            let Some(&scope) = Scope::ALL.get(c as usize - '1' as usize) else {
                return Action::None;
            };
            app.custom.set_scope(scope);
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
            app.branches_shown();
            app.worktrees_entered();
            app.drain_if_sync();
            return Action::Redraw;
        }
        KeyCode::Char('B') if app.view == View::Branches => return branch_page(app),
        KeyCode::Char('y') if !ctrl && app.view == View::Branches => {
            return match app.branches.selected() {
                Some(branch) => Action::Copy(branch.name.clone()),
                None => Action::None,
            };
        }
        KeyCode::Char('r') if !ctrl && app.view == View::Worktrees => {
            app.worktrees_entered();
            app.drain_if_sync();
            return Action::Redraw;
        }
        KeyCode::Char('y') if !ctrl && app.view == View::Worktrees => {
            return match app.worktrees.selected() {
                Some(entry) => Action::Copy(entry.checkout.path.display().to_string()),
                None => Action::None,
            };
        }
        KeyCode::Char('e') if !ctrl => return edit(app),
        KeyCode::Char('E') => return open(app, Opener::VsCode),
        KeyCode::Char('b') if !ctrl && app.view != View::Worktrees => {
            return open(app, Opener::Browser)
        }
        KeyCode::Char('B') if app.view != View::Worktrees => return open(app, Opener::GitHub),
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
        KeyCode::Char('s') if !ctrl && matches!(app.view, View::Files | View::Changes) => {
            app.cycle_scope();
            return Action::Redraw;
        }
        KeyCode::Char('a') if !ctrl && matches!(app.view, View::Files | View::Changes) => {
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
        View::Custom => custom_key(app, key),
        View::Status => status_key(app, key),
        View::Changes => changes_key(app, key),
        View::History => history_key(app, key),
        View::Branches => branches_key(app, key),
        View::Worktrees => worktrees_key(app, key),
    };
    if r == Action::None && had_message {
        Action::Redraw
    } else {
        r
    }
}

pub fn paste(app: &mut App, text: &str) -> Action {
    if app.confirm.is_some() {
        app.answer(false);
        return Action::Redraw;
    }
    if app.commit.open && !app.commit.busy {
        app.commit.paste(text);
        return Action::Redraw;
    }
    let naming_a_branch = |input: &&mut Input| input.kind == InputKind::BranchName;
    if let Some(name) = app.input.as_mut().filter(naming_a_branch) {
        name.text.extend(text.chars().filter(|c| !c.is_control()));
        return Action::Redraw;
    }
    Action::None
}

fn confirm_key(app: &mut App, key: KeyEvent) -> Action {
    let yes = key.code == KeyCode::Char('y')
        && key.modifiers.is_empty()
        && key.kind == KeyEventKind::Press;
    app.answer(yes);
    Action::Redraw
}

fn commit_key(app: &mut App, key: KeyEvent) -> Action {
    if app.commit.busy {
        return Action::None;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Esc => app.commit.close(),
        KeyCode::Enter if app.commit.field == Field::Subject => app.submit_commit(),
        KeyCode::Enter => app.commit.new_line(),
        KeyCode::Tab | KeyCode::BackTab => app.commit.other_field(),
        KeyCode::Backspace => app.commit.backspace(),
        KeyCode::Char('u') if ctrl => app.commit.clear_field(),
        KeyCode::Char(c) if !ctrl && !alt => app.commit.type_char(c),
        _ => return Action::None,
    }
    Action::Redraw
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

fn branch_page(app: &mut App) -> Action {
    match app.branch_link() {
        Ok(page) => Action::Browse(page),
        Err(why) => {
            app.message = Some(why);
            Action::Redraw
        }
    }
}

fn toggle_focus(app: &mut App) {
    let focus = match app.view {
        View::Files => &mut app.files.focus,
        View::Status => &mut app.status.focus,
        View::Changes => &mut app.changes.focus,
        View::Custom | View::History | View::Branches | View::Worktrees => return,
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
            InputKind::BranchName => {
                let name = input.text.clone();
                app.input = None;
                app.create_branch(&name);
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

fn custom_key(app: &mut App, key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('d') if ctrl => app.custom.move_by(10),
        KeyCode::Char('u') if ctrl => app.custom.move_by(-10),
        KeyCode::Up | KeyCode::Char('k') => app.custom.move_by(-1),
        KeyCode::Down | KeyCode::Char('j') => app.custom.move_by(1),
        KeyCode::Char('g') | KeyCode::Home => app.custom.move_by(-(i32::MAX / 2)),
        KeyCode::Char('G') | KeyCode::End => app.custom.move_by(i32::MAX / 2),
        KeyCode::Right | KeyCode::Char('l') => {
            let next = Scope::ALL
                .iter()
                .position(|s| *s == app.custom.scope)
                .unwrap_or(0)
                + 1;
            if let Some(&scope) = Scope::ALL.get(next) {
                app.custom.set_scope(scope);
            }
        }
        KeyCode::Left | KeyCode::Char('h') => {
            let at = Scope::ALL
                .iter()
                .position(|s| *s == app.custom.scope)
                .unwrap_or(0);
            if let Some(&scope) = at.checked_sub(1).and_then(|i| Scope::ALL.get(i)) {
                app.custom.set_scope(scope);
            }
        }
        KeyCode::Enter => return edit(app),
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

fn status_key(app: &mut App, key: KeyEvent) -> Action {
    let held = KeyModifiers::CONTROL | KeyModifiers::ALT;
    match key.code {
        _ if key.modifiers.intersects(held) => return status_move_key(app, key),
        KeyCode::Char(' ') => app.toggle_staged(),
        KeyCode::Char('c') => app.open_commit_box(),
        KeyCode::Char('x') => app.ask_discard(),
        _ => return status_move_key(app, key),
    }
    Action::Redraw
}

fn status_move_key(app: &mut App, key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if app.status.focus == Focus::Body {
        match key.code {
            KeyCode::Char(']') => app.status.next_hunk(1),
            KeyCode::Char('[') => app.status.next_hunk(-1),
            KeyCode::Char('d') if ctrl => app.status.scroll_by(15),
            KeyCode::Char('u') if ctrl => app.status.scroll_by(-15),
            KeyCode::Up | KeyCode::Char('k') => app.status.scroll_by(-1),
            KeyCode::Down | KeyCode::Char('j') => app.status.scroll_by(1),
            KeyCode::PageDown => app.status.scroll_by(30),
            KeyCode::PageUp => app.status.scroll_by(-30),
            KeyCode::Char('g') | KeyCode::Home => app.status.scroll = 0,
            KeyCode::Char('G') | KeyCode::End => app.status.scroll_by(i32::MAX / 2),
            _ => return Action::None,
        }
        return Action::Redraw;
    }
    match key.code {
        KeyCode::Char('d') if ctrl => app.status.move_by(15),
        KeyCode::Char('u') if ctrl => app.status.move_by(-15),
        KeyCode::Up | KeyCode::Char('k') => app.status.move_by(-1),
        KeyCode::Down | KeyCode::Char('j') => app.status.move_by(1),
        KeyCode::PageDown => app.status.move_by(30),
        KeyCode::PageUp => app.status.move_by(-30),
        KeyCode::Char('g') | KeyCode::Home => app.status.move_by(-(i32::MAX / 2)),
        KeyCode::Char('G') | KeyCode::End => app.status.move_by(i32::MAX / 2),
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

fn branches_key(app: &mut App, key: KeyEvent) -> Action {
    let held = KeyModifiers::CONTROL | KeyModifiers::ALT;
    match key.code {
        _ if key.modifiers.intersects(held) => return branches_move_key(app, key),
        KeyCode::Enter => app.switch_selected(),
        KeyCode::Char('n') => app.start_new_branch(),
        KeyCode::Char('m') => app.ask_merge(),
        KeyCode::Char('D') => app.ask_delete(),
        _ => return branches_move_key(app, key),
    }
    Action::Redraw
}

fn branches_move_key(app: &mut App, key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('d') if ctrl => app.branches_move(15),
        KeyCode::Char('u') if ctrl => app.branches_move(-15),
        KeyCode::Up | KeyCode::Char('k') => app.branches_move(-1),
        KeyCode::Down | KeyCode::Char('j') => app.branches_move(1),
        KeyCode::PageDown => app.branches_move(30),
        KeyCode::PageUp => app.branches_move(-30),
        KeyCode::Char('g') | KeyCode::Home => app.branches_move(-(i32::MAX / 2)),
        KeyCode::Char('G') | KeyCode::End => app.branches_move(i32::MAX / 2),
        _ => return Action::None,
    }
    Action::Redraw
}

fn worktrees_key(app: &mut App, key: KeyEvent) -> Action {
    let held = KeyModifiers::CONTROL | KeyModifiers::ALT;
    match key.code {
        _ if key.modifiers.intersects(held) => return worktrees_move_key(app, key),
        KeyCode::Enter => app.show_selected_worktree(),
        _ => return worktrees_move_key(app, key),
    }
    Action::Redraw
}

fn worktrees_move_key(app: &mut App, key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('d') if ctrl => app.worktrees_move(15),
        KeyCode::Char('u') if ctrl => app.worktrees_move(-15),
        KeyCode::Up | KeyCode::Char('k') => app.worktrees_move(-1),
        KeyCode::Down | KeyCode::Char('j') => app.worktrees_move(1),
        KeyCode::PageDown => app.worktrees_move(30),
        KeyCode::PageUp => app.worktrees_move(-30),
        KeyCode::Char('g') | KeyCode::Home => app.worktrees_move(-(i32::MAX / 2)),
        KeyCode::Char('G') | KeyCode::End => app.worktrees_move(i32::MAX / 2),
        _ => return Action::None,
    }
    Action::Redraw
}
