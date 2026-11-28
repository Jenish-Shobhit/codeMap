use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::branches::upstream_words;
use super::files::status_color;
use super::{bold, put, style, Areas, Line, Rail};
use crate::app::App;
use crate::git::branch::short_sha;
use crate::git::worktree::{Change, Changes, Checkout, Uncommitted};
use crate::theme::Theme;
use crate::util;
use crate::worktrees::{shown_path, Agent, Entry, WorktreesState};

const LABEL: usize = 14;
const ROWS_EACH: usize = 2;
const WIDEST_PATH: usize = 60;
const GAP: &str = "  ";

pub fn render(buf: &mut Buffer, a: &Areas, app: &mut App) {
    let t = app.theme;
    let body = a.body;
    if let Some((text, color)) = notice(app) {
        put(buf, body.x, body.y, &text, style(color, t.body), body.width);
        return;
    }
    app.worktrees.scroll = list(buf, a.rail, app);
    let lines = match app.worktrees.selected() {
        Some(entry) => detail(app, entry, body),
        None => vec![muted("no worktrees", &t)],
    };
    for (row, line) in lines.iter().enumerate().take(body.height as usize) {
        line.draw(buf, body.x, body.y + row as u16, body.width);
    }
}

pub(super) fn hints(app: &App) -> Vec<(&'static str, String)> {
    let keys = ("?", "keys".to_string());
    let listed = notice(app).is_none();
    let Some(entry) = app.worktrees.selected().filter(|_| listed) else {
        return vec![keys];
    };
    let copy = ("y", "copy path".to_string());
    let reread = ("r", "reread".to_string());
    if entry.checkout.is_missing() {
        return vec![copy, reread, keys];
    }
    let mut hints = Vec::new();
    if !app.shows(&entry.checkout) {
        hints.push(("⏎", "show this worktree".to_string()));
    }
    hints.extend([
        ("e", "editor".to_string()),
        ("E", "VS Code".into()),
        copy,
        reread,
        keys,
    ]);
    hints
}

fn notice(app: &App) -> Option<(String, Color)> {
    let t = app.theme;
    if app.not_a_repo() {
        let text = format!("not a git repository · {}", util::tilde(&app.root));
        return Some((text, t.modified));
    }
    if !app.worktrees.loaded {
        return Some(("reading worktrees…".into(), t.muted));
    }
    app.worktrees
        .error
        .as_ref()
        .map(|e| (format!("git: {}", util::sanitize_line(e)), t.del))
}

fn muted(text: impl Into<String>, t: &Theme) -> Line<'static> {
    Line::new().push(text, style(t.muted, t.body))
}

fn labelled(label: &str, t: &Theme) -> Line<'static> {
    Line::new().push(util::pad(label, LABEL), style(t.text2, t.body))
}

fn dot(agent: &Agent, t: &Theme) -> (&'static str, Color) {
    if agent.is_working() {
        ("● ", t.add)
    } else {
        ("○ ", t.muted)
    }
}

fn first_shown(selected: usize, top: usize, fit: usize) -> usize {
    let fit = fit.max(1);
    if selected < top {
        selected
    } else if selected >= top + fit {
        selected + 1 - fit
    } else {
        top
    }
}

fn path_line(
    checkout: &Checkout,
    shown: bool,
    selected: bool,
    width: usize,
    t: &Theme,
) -> Line<'static> {
    let bg = if selected { t.active_row } else { t.rail };
    let path = if selected {
        bold(style(t.text, bg))
    } else {
        style(t.text, bg)
    };
    let mark = if shown { "● " } else { "  " };
    let room = width.saturating_sub(util::width(mark));
    Line::new()
        .bg(bg)
        .push(mark, style(t.add, bg))
        .push(util::truncate_left(&shown_path(&checkout.path), room), path)
}

fn summary(checkout: &Checkout, t: &Theme) -> Vec<(String, Color)> {
    let uncommitted = match &checkout.changes {
        Changes::Missing => return vec![("folder missing".into(), t.modified)],
        Changes::Unreadable(_) => return vec![("unreadable".into(), t.del)],
        Changes::Read(uncommitted) => uncommitted,
    };
    if uncommitted.is_clean() {
        return vec![("clean".into(), t.muted)];
    }
    let mut spans = vec![(util::plural(uncommitted.files(), "file", "files"), t.text2)];
    if uncommitted.adds() > 0 {
        spans.push((format!(" +{}", uncommitted.adds()), t.add));
    }
    if uncommitted.dels() > 0 {
        spans.push((format!(" −{}", uncommitted.dels()), t.del));
    }
    spans
}

fn detail_line(entry: &Entry, selected: bool, t: &Theme) -> Line<'static> {
    let bg = if selected { t.active_row } else { t.rail };
    let checkout = &entry.checkout;
    let head = if checkout.branch.is_some() {
        t.branch
    } else {
        t.text2
    };
    let plain = style(t.muted, bg);
    let mut line = Line::new()
        .bg(bg)
        .push("    ", plain)
        .push(util::sanitize_line(&checkout.head_label()), style(head, bg));
    for agent in &entry.agents {
        let (mark, tone) = dot(agent, t);
        line = line
            .push(GAP, plain)
            .push(mark, style(tone, bg))
            .push(util::sanitize_line(&agent.name), style(t.text2, bg));
    }
    line = line.push(GAP, plain);
    for (text, color) in summary(checkout, t) {
        line = line.push(text, style(color, bg));
    }
    if checkout.locked.is_some() {
        line = line.push(GAP, plain).push("locked", style(t.modified, bg));
    }
    line
}

fn list(buf: &mut Buffer, area: Rect, app: &App) -> usize {
    let state = &app.worktrees;
    if area.width == 0 || area.height == 0 {
        return state.scroll;
    }
    let t = app.theme;
    let fit = (area.height as usize).saturating_sub(1) / ROWS_EACH;
    let top = first_shown(state.sel, state.scroll.min(state.entries.len()), fit);
    let width = (area.width as usize).saturating_sub(1);
    let mut rail = Rail::new(buf, area, t);
    rail.section(&format!("worktrees · {}", state.entries.len()));
    for (i, entry) in state.entries.iter().enumerate().skip(top).take(fit.max(1)) {
        let selected = i == state.sel;
        let shown = app.shows(&entry.checkout);
        rail.line(path_line(&entry.checkout, shown, selected, width, &t));
        rail.line(detail_line(entry, selected, &t));
    }
    top
}

fn standing(app: &App, checkout: &Checkout, t: &Theme) -> Line<'static> {
    if checkout.is_missing() {
        let said = if checkout.locked.is_some() {
            "the folder is missing · locked, so git worktree prune keeps it"
        } else {
            "the folder is missing · git worktree prune forgets it"
        };
        return Line::new().push(said, style(t.modified, t.body));
    }
    if !app.shows(checkout) {
        return muted("⏎ shows this worktree in every tab", t);
    }
    let said = if app.worktrees.entries.len() == 1 {
        "every tab shows this worktree · the repository has no other"
    } else {
        "every tab shows this worktree"
    };
    Line::new().push(said, style(t.branch, t.body))
}

fn branch_line(checkout: &Checkout, t: &Theme) -> Line<'static> {
    let line = labelled("branch", t);
    match (&checkout.branch, &checkout.head) {
        (Some(branch), Some(_)) => line.push(util::sanitize_line(branch), style(t.branch, t.body)),
        (Some(branch), None) => line
            .push(util::sanitize_line(branch), style(t.branch, t.body))
            .push(" · no commits yet", style(t.muted, t.body)),
        (None, Some(head)) => line.push(
            format!("none · HEAD is detached at {}", short_sha(head)),
            style(t.text, t.body),
        ),
        (None, None) => line.push("none", style(t.muted, t.body)),
    }
}

fn upstream_line(checkout: &Checkout, t: &Theme) -> Line<'static> {
    let line = labelled("upstream", t);
    match &checkout.upstream {
        Some(upstream) => line.push(upstream_words(upstream), style(t.text, t.body)),
        None => line.push("none", style(t.muted, t.body)),
    }
}

fn agent_lines(state: &WorktreesState, entry: &Entry, t: &Theme) -> Vec<Line<'static>> {
    if !state.agents_known {
        return Vec::new();
    }
    if entry.agents.is_empty() {
        return vec![labelled("agents", t).push("none", style(t.muted, t.body))];
    }
    let longest = entry
        .agents
        .iter()
        .map(|agent| util::width(&agent.name))
        .max()
        .unwrap_or(0);
    entry
        .agents
        .iter()
        .enumerate()
        .map(|(i, agent)| {
            let (mark, tone) = dot(agent, t);
            let status = agent.status.as_deref().unwrap_or("unknown");
            labelled(if i == 0 { "agents" } else { "" }, t)
                .push(mark, style(tone, t.body))
                .push(
                    util::pad(&util::sanitize_line(&agent.name), longest),
                    style(t.text, t.body),
                )
                .push(
                    format!("  {}", util::sanitize_line(status)),
                    style(t.muted, t.body),
                )
        })
        .collect()
}

fn said_by_git(label: &str, why: &Option<String>, t: &Theme) -> Option<Line<'static>> {
    let why = why.as_ref()?;
    let said = if why.is_empty() {
        "yes".to_string()
    } else {
        util::sanitize_line(why)
    };
    Some(labelled(label, t).push(said, style(t.modified, t.body)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row<'a> {
    Title(&'static str, usize),
    File(&'a Change),
    Gap,
    More(usize),
}

fn change_rows(uncommitted: &Uncommitted, room: usize) -> Vec<Row<'_>> {
    let mut rows = vec![Row::Title("staged", uncommitted.staged.len())];
    rows.extend(uncommitted.staged.iter().map(Row::File));
    rows.push(Row::Gap);
    rows.push(Row::Title("not staged", uncommitted.unstaged.len()));
    rows.extend(uncommitted.unstaged.iter().map(Row::File));
    if rows.len() <= room {
        return rows;
    }
    let is_file = |row: &Row| matches!(row, Row::File(_));
    let files = rows.iter().filter(|row| is_file(row)).count();
    rows.truncate(room.saturating_sub(1));
    while rows.last().is_some_and(|row| !is_file(row)) {
        rows.pop();
    }
    let listed = rows.iter().filter(|row| is_file(row)).count();
    if room > 0 {
        rows.push(Row::More(files - listed));
    }
    rows
}

fn change_line(change: &Change, path_width: usize, t: &Theme) -> Line<'static> {
    let path = util::truncate_left(&util::sanitize_line(&change.path), path_width);
    let mut line = Line::new()
        .push(
            format!("{} ", change.status.letter()),
            style(status_color(t, change.status), t.body),
        )
        .push(util::pad(&path, path_width), style(t.text, t.body));
    if change.binary {
        return line.push(" bin", style(t.modified, t.body));
    }
    if change.adds > 0 {
        line = line.push(format!(" +{}", change.adds), style(t.add, t.body));
    }
    if change.dels > 0 {
        line = line.push(format!(" −{}", change.dels), style(t.del, t.body));
    }
    line
}

fn changes_lines(checkout: &Checkout, body: Rect, room: usize, t: &Theme) -> Vec<Line<'static>> {
    let uncommitted = match &checkout.changes {
        Changes::Missing => return Vec::new(),
        Changes::Unreadable(why) => {
            let said = format!("git: {}", util::sanitize_line(why));
            return vec![Line::new().push(said, style(t.del, t.body))];
        }
        Changes::Read(uncommitted) => uncommitted,
    };
    if uncommitted.is_clean() {
        return vec![muted("nothing to commit, working tree clean", t)];
    }
    let rows = change_rows(uncommitted, room);
    let longest = rows
        .iter()
        .filter_map(|row| match row {
            Row::File(change) => Some(util::width(&change.path)),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let path_width = longest
        .min((body.width as usize).saturating_sub(16))
        .clamp(8, WIDEST_PATH);
    rows.iter()
        .map(|row| match row {
            Row::Title(title, files) => {
                Line::new().push(format!("{title} · {files}"), bold(style(t.text2, t.body)))
            }
            Row::File(change) => change_line(change, path_width, t),
            Row::Gap => Line::new(),
            Row::More(files) => muted(format!("and {files} more"), t),
        })
        .collect()
}

fn detail(app: &App, entry: &Entry, body: Rect) -> Vec<Line<'static>> {
    let t = app.theme;
    let checkout = &entry.checkout;
    let path = util::truncate_left(&shown_path(&checkout.path), body.width as usize);
    let mut lines = vec![
        Line::new().push(path, bold(style(t.text, t.body))),
        standing(app, checkout, &t),
        Line::new(),
        branch_line(checkout, &t),
        upstream_line(checkout, &t),
    ];
    lines.extend(agent_lines(&app.worktrees, entry, &t));
    lines.extend(said_by_git("locked", &checkout.locked, &t));
    lines.extend(said_by_git("prunable", &checkout.prunable, &t));
    lines.push(Line::new());
    let room = (body.height as usize).saturating_sub(lines.len());
    lines.extend(changes_lines(checkout, body, room, &t));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::FileStatus;

    fn change(path: &str) -> Change {
        Change {
            path: path.into(),
            status: FileStatus::Modified,
            adds: 1,
            dels: 0,
            binary: false,
        }
    }

    fn uncommitted(staged: &[&str], unstaged: &[&str]) -> Uncommitted {
        Uncommitted {
            staged: staged.iter().map(|p| change(p)).collect(),
            unstaged: unstaged.iter().map(|p| change(p)).collect(),
        }
    }

    fn told(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                Row::Title(title, files) => format!("{title} · {files}"),
                Row::File(change) => change.path.clone(),
                Row::Gap => String::new(),
                Row::More(files) => format!("and {files} more"),
            })
            .collect()
    }

    #[test]
    fn the_list_scrolls_to_keep_the_selected_worktree_in_view() {
        assert_eq!(first_shown(0, 0, 3), 0);
        assert_eq!(first_shown(2, 0, 3), 0, "already in view");
        assert_eq!(first_shown(3, 0, 3), 1, "down past the last one");
        assert_eq!(first_shown(1, 4, 3), 1, "up");
        assert_eq!(first_shown(5, 4, 0), 5, "a rail with room for none");
    }

    #[test]
    fn changed_files_are_grouped_as_staged_and_not_staged() {
        let both = uncommitted(&["a.py"], &["b.py", "c.py"]);
        assert_eq!(
            told(&change_rows(&both, 10)),
            ["staged · 1", "a.py", "", "not staged · 2", "b.py", "c.py"]
        );
        assert_eq!(told(&change_rows(&both, 6)).len(), 6, "an exact fit");
        let only_unstaged = uncommitted(&[], &["b.py"]);
        assert_eq!(
            told(&change_rows(&only_unstaged, 10)),
            ["staged · 0", "", "not staged · 1", "b.py"]
        );
    }

    #[test]
    fn files_past_the_room_are_counted_not_listed() {
        let many = uncommitted(&["a.py", "b.py"], &["c.py", "d.py", "e.py"]);
        assert_eq!(
            told(&change_rows(&many, 7)),
            [
                "staged · 2",
                "a.py",
                "b.py",
                "",
                "not staged · 3",
                "c.py",
                "and 2 more"
            ]
        );
        assert_eq!(
            told(&change_rows(&many, 6)),
            ["staged · 2", "a.py", "b.py", "and 3 more"],
            "a title with no file under it is not left hanging"
        );
        assert_eq!(told(&change_rows(&many, 2)), ["and 5 more"]);
        assert_eq!(told(&change_rows(&many, 1)), ["and 5 more"]);
        assert!(change_rows(&many, 0).is_empty());
    }
}
