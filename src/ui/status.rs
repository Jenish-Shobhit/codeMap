use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::changes::{diff_line, hunk_name, RowMark};
use super::files::{counts, status_color};
use super::{bold, put, style, Areas, Line, Rail};
use crate::app::{App, Focus};
use crate::git::{FileDiff, FileStatus, Hunk};
use crate::status::{diff_rows, DiffRow, StatusState, Target};
use crate::theme::Theme;
use crate::util::{self, parent_dir};

pub fn render(buf: &mut Buffer, a: &Areas, app: &mut App) {
    let t = app.theme;
    let body = a.body;
    if let Some((text, color)) = notice(app) {
        put(buf, body.x, body.y, &text, style(color, t.body), body.width);
        return;
    }
    list(buf, a.rail, app);
    let Some(file) = app.status.selected() else {
        put(
            buf,
            body.x,
            body.y,
            "nothing to commit, working tree clean",
            style(t.muted, t.body),
            body.width,
        );
        return;
    };
    let under_cursor = match app.status.target() {
        Some(Target::Hunk(h)) => Some(h),
        _ => None,
    };
    let mut head = heading(file, app.status.selected_is_staged(), &t);
    if let Some(h) = under_cursor {
        head = head.push(
            format!("    hunk {} of {}", h + 1, file.hunks.len()),
            style(t.accent, t.body),
        );
    }
    head.draw(buf, body.x, body.y, body.width);
    if file.hunks.is_empty() {
        let color = if file.binary { t.modified } else { t.muted };
        put(
            buf,
            body.x,
            body.y + 2,
            without_hunks(file),
            style(color, t.body),
            body.width,
        );
        return;
    }
    let rows = diff_rows(file);
    let top = app.status.scroll;
    let room = body.height.saturating_sub(1) as usize;
    let focused = app.status.focus == Focus::Body;
    for (i, row) in rows.iter().enumerate().skip(top).take(room) {
        let at = Rect::new(body.x, body.y + 1 + (i - top) as u16, body.width, 1);
        let cursor = focused && i == top;
        let (DiffRow::Hunk(h) | DiffRow::Line(h, _)) = *row;
        if under_cursor == Some(h) && at.x > 0 {
            put(buf, at.x - 1, at.y, "│", bold(style(t.accent, t.body)), 1);
        }
        match *row {
            DiffRow::Hunk(h) => hunk_heading(buf, at, &file.hunks[h], &t, cursor),
            DiffRow::Line(h, l) => diff_line(
                buf,
                at,
                &file.hunks[h].lines[l],
                &t,
                RowMark {
                    cursor,
                    selected: false,
                },
            ),
        }
    }
}

pub(super) fn hints(status: &StatusState) -> Vec<(&'static str, String)> {
    let staged = status.selected_is_staged();
    let on = match status.target() {
        Some(Target::Hunk(_)) => " hunk",
        Some(Target::File) if status.focus == Focus::Body => " file",
        _ => "",
    };
    let stage = if staged { "unstage" } else { "stage" };
    let mut hints = match status.focus {
        Focus::Rail => vec![("j k", "file".to_string())],
        Focus::Body => vec![("j k", "scroll".to_string()), ("] [", "hunk".into())],
    };
    if !status.is_empty() {
        hints.push(("space", format!("{stage}{on}")));
    }
    if !status.is_empty() && !staged {
        hints.push(("x", "discard".into()));
    }
    hints.extend([
        ("c", "commit".to_string()),
        ("e", "editor".into()),
        ("?", "keys".into()),
    ]);
    hints
}

fn notice(app: &App) -> Option<(String, Color)> {
    let t = app.theme;
    if app.not_a_repo() {
        let text = format!("not a git repository · {}", util::tilde(&app.root));
        return Some((text, t.modified));
    }
    if !app.repo_known || !app.status.loaded {
        return Some(("reading git status…".into(), t.muted));
    }
    app.status
        .error
        .as_ref()
        .map(|e| (format!("git: {e}"), t.del))
}

fn heading(file: &FileDiff, staged: bool, t: &Theme) -> Line<'static> {
    let mut head = Line::new().push(file.path.clone(), bold(style(t.text, t.body)));
    if let Some(old) = &file.old_path {
        head = head.push(format!("  ← {old}"), style(t.renamed, t.body));
    }
    let group = match (staged, file.status) {
        (true, _) => "staged",
        (false, FileStatus::Untracked) => "untracked",
        (false, _) => "not staged",
    };
    counts(head.push("  ", style(t.muted, t.body)), file, t, t.body)
        .push(format!("    {group}"), style(t.muted, t.body))
}

fn without_hunks(file: &FileDiff) -> &'static str {
    match file.status {
        _ if file.binary => "binary",
        FileStatus::Renamed => "renamed, content unchanged",
        FileStatus::Deleted => "deleted",
        FileStatus::Added | FileStatus::Untracked => "empty file",
        FileStatus::Modified | FileStatus::Copied => "no text changes",
    }
}

fn hunk_heading(buf: &mut Buffer, at: Rect, hunk: &Hunk, t: &Theme, cursor: bool) {
    let bg = if cursor { t.active_row } else { t.body };
    Line::new()
        .bg(bg)
        .push("@@ ", style(t.rule, bg))
        .push(hunk_name(hunk), style(t.text2, bg))
        .push(
            format!(" · {}–{}", hunk.new_start, hunk.new_end()),
            style(t.muted, bg),
        )
        .draw(buf, at.x, at.y, at.width);
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ListRow {
    Title(String),
    Folder(String),
    File(usize),
    Gap,
}

fn group(rows: &mut Vec<ListRow>, title: &str, files: &[FileDiff], first: usize) {
    rows.push(ListRow::Title(format!("{title} · {}", files.len())));
    let mut folder = None;
    for (i, file) in files.iter().enumerate() {
        let parent = parent_dir(&file.path);
        if folder != Some(parent) {
            folder = Some(parent);
            rows.push(ListRow::Folder(if parent.is_empty() {
                "./".to_string()
            } else {
                format!("{parent}/")
            }));
        }
        rows.push(ListRow::File(first + i));
    }
}

fn list_rows(status: &StatusState) -> Vec<ListRow> {
    let mut rows = Vec::new();
    group(&mut rows, "staged", &status.staged, 0);
    rows.push(ListRow::Gap);
    group(
        &mut rows,
        "not staged",
        &status.unstaged,
        status.staged.len(),
    );
    rows
}

fn top_showing(rows: &[ListRow], selected: usize, top: usize, height: usize) -> usize {
    let Some(at) = rows.iter().position(|r| *r == ListRow::File(selected)) else {
        return 0;
    };
    let lowest_top = (at + 1).saturating_sub(height);
    if at < top {
        let headings = rows[..at]
            .iter()
            .rev()
            .take_while(|r| matches!(r, ListRow::Title(_) | ListRow::Folder(_)))
            .count();
        (at - headings).max(lowest_top)
    } else {
        top.max(lowest_top)
    }
}

struct Mark {
    selected: bool,
    focused: bool,
}

fn file_line(file: &FileDiff, name_width: usize, mark: Mark, t: &Theme) -> Line<'static> {
    let bg = if mark.selected { t.active_row } else { t.rail };
    let name = file.path.rsplit('/').next().unwrap_or(&file.path);
    let name_style = if mark.selected && mark.focused {
        bold(style(t.text, bg))
    } else {
        style(t.text, bg)
    };
    let line = Line::new()
        .bg(bg)
        .push(
            format!("{} ", file.status.letter()),
            style(status_color(t, file.status), bg),
        )
        .push(util::pad(name, name_width), name_style);
    if file.binary {
        line.push(" bin", style(t.modified, bg))
    } else {
        counts(line, file, t, bg)
    }
}

fn list(buf: &mut Buffer, area: Rect, app: &mut App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let t = app.theme;
    let rows = list_rows(&app.status);
    let unborn = app.repo.as_ref().is_some_and(|r| r.is_empty());
    let height = (area.height as usize).saturating_sub(if unborn { 2 } else { 0 });
    app.status.list_scroll = top_showing(&rows, app.status.sel, app.status.list_scroll, height);
    let focused = app.status.focus == Focus::Rail;
    let name_width = (area.width as usize).saturating_sub(13).clamp(8, 20);
    let files: Vec<&FileDiff> = app
        .status
        .staged
        .iter()
        .chain(&app.status.unstaged)
        .collect();
    let mut rail = Rail::new(buf, area, t);
    for row in rows.iter().skip(app.status.list_scroll).take(height) {
        match row {
            ListRow::Title(title) => rail.section(title),
            ListRow::Folder(folder) => {
                rail.line(Line::new().push(folder.clone(), style(t.text2, t.rail)))
            }
            ListRow::File(i) => {
                let mark = Mark {
                    selected: *i == app.status.sel,
                    focused,
                };
                rail.line(file_line(files[*i], name_width, mark, &t))
            }
            ListRow::Gap => rail.blank(),
        }
    }
    if unborn {
        rail.footer(Line::new().push("no commits yet", style(t.muted, t.rail)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git;

    fn file(path: &str) -> FileDiff {
        FileDiff {
            path: path.into(),
            old_path: None,
            status: FileStatus::Modified,
            binary: false,
            hunks: Vec::new(),
        }
    }

    fn status(staged: &[&str], unstaged: &[&str]) -> StatusState {
        let mut state = StatusState::default();
        state.load(Ok(git::Status {
            staged: staged.iter().map(|p| file(p)).collect(),
            unstaged: unstaged.iter().map(|p| file(p)).collect(),
        }));
        state
    }

    fn title(text: &str) -> ListRow {
        ListRow::Title(text.into())
    }

    fn folder(text: &str) -> ListRow {
        ListRow::Folder(text.into())
    }

    #[test]
    fn the_list_names_each_folder_once_under_both_titles() {
        let rows = list_rows(&status(
            &["src/app.rs", "README.md", "src/keys.rs"],
            &["src/app.rs"],
        ));
        assert_eq!(
            rows,
            [
                title("staged · 3"),
                folder("./"),
                ListRow::File(0),
                folder("src/"),
                ListRow::File(1),
                ListRow::File(2),
                ListRow::Gap,
                title("not staged · 1"),
                folder("src/"),
                ListRow::File(3),
            ]
        );
    }

    #[test]
    fn a_clean_tree_still_lists_both_titles() {
        let rows = list_rows(&status(&[], &[]));
        assert_eq!(
            rows,
            [title("staged · 0"), ListRow::Gap, title("not staged · 0")]
        );
    }

    #[test]
    fn the_list_scrolls_to_keep_the_selection_and_its_headings_in_view() {
        let rows = list_rows(&status(&["a.rs", "b.rs"], &["src/c.rs", "src/d.rs"]));
        assert_eq!(top_showing(&rows, 0, 0, 4), 0);
        assert_eq!(top_showing(&rows, 3, 0, 4), 5, "down to the last file");
        assert_eq!(top_showing(&rows, 2, 5, 4), 5, "already in view");
        assert_eq!(top_showing(&rows, 2, 8, 4), 5, "up, with title and folder");
        assert_eq!(top_showing(&rows, 2, 8, 2), 6, "no room for the title");
        assert_eq!(
            top_showing(&rows, 1, 5, 4),
            3,
            "up to a file without headings"
        );
        assert_eq!(top_showing(&rows, 0, 5, 4), 0);
        assert_eq!(top_showing(&rows, 9, 3, 4), 0, "nothing selected");
    }
}
