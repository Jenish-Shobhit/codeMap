use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::{bold, put, style, Areas, Line, Rail};
use crate::app::{App, HISTORY_LIMIT};
use crate::git::{FileDiff, FileStatus};
use crate::history::FileHistory;
use crate::theme::Theme;
use crate::util;

pub fn file_summary(app: &App) -> Option<String> {
    let file = app.file_history.as_ref()?;
    let mut line = format!("history of {}", file.path);
    if file.loaded && file.log.error.is_none() {
        let shown = file.log.commits.len();
        let count = if shown >= HISTORY_LIMIT {
            format!("{shown}+ commits")
        } else {
            util::plural(shown, "commit", "commits")
        };
        line.push_str(&format!(" · {count}"));
    }
    Some(line)
}

pub(super) fn hints(app: &App) -> Vec<(&'static str, String)> {
    let in_a_file = app.file_history.is_some();
    let diff_shown = app.shown_history().show_diff;
    let on_a_commit = app.selected_commit().is_some();
    let mut hints = Vec::new();
    if diff_shown {
        hints.push(("⌫", "commit".to_string()));
        hints.push(("j k", "scroll".into()));
    } else if in_a_file {
        hints.push(("⎋", "back".to_string()));
    }
    if on_a_commit && !diff_shown {
        hints.push(("⏎", "diff".into()));
    }
    if on_a_commit {
        hints.push(("B", "GitHub".into()));
        hints.push(("y", "copy sha".into()));
    }
    if in_a_file && !diff_shown {
        hints.push(("e", "editor".into()));
    }
    hints.push(("?", "keys".into()));
    hints
}

fn no_commits_touch(buf: &mut Buffer, body: Rect, file: &FileHistory, t: &Theme) {
    if let Some(e) = &file.log.error {
        let said = format!("git: {e}");
        put(buf, body.x, body.y, &said, style(t.del, t.body), body.width);
        return;
    }
    let lines = [
        (
            format!("no commit has touched {} yet", file.path),
            style(t.text, t.body),
        ),
        (
            "a new or untracked file has a history once it is committed".to_string(),
            style(t.muted, t.body),
        ),
    ];
    for (y, (text, look)) in (body.y..).zip(lines) {
        put(buf, body.x, y, &text, look, body.width);
    }
}

fn file_line(f: &FileDiff, in_a_file: bool) -> String {
    match &f.old_path {
        Some(old) if in_a_file => format!("{old} → {}", f.path),
        _ => f.path.clone(),
    }
}

pub fn header_summary(app: &App) -> Option<String> {
    let repo = app.repo.as_ref()?;
    if repo.head.is_none() {
        return Some("no commits yet".into());
    }
    let branches: std::collections::BTreeSet<String> = app
        .history
        .commits
        .iter()
        .flat_map(|c| c.labels())
        .filter(|l| !l.starts_with("tag: ") && !l.contains('/'))
        .collect();
    let head = repo
        .branch
        .clone()
        .or_else(|| repo.head.as_ref().map(|h| h[..7].to_string()))
        .unwrap_or_default();
    Some(format!(
        "{} · {} · {} · HEAD {}",
        util::plural(
            app.history.total.max(app.history.commits.len()),
            "commit",
            "commits"
        ),
        util::plural(branches.len().max(1), "branch", "branches"),
        util::plural(app.history.worktrees.max(1), "worktree", "worktrees"),
        head
    ))
}

fn lane_color(app: &App, i: usize) -> Color {
    let t = &app.theme;
    [
        t.accent, t.renamed, t.add, t.modified, t.conflict, t.branch, t.text2,
    ][i % 7]
}

pub fn render(buf: &mut Buffer, a: &Areas, app: &mut App) {
    let t = app.theme;
    app.ensure_commit_detail();
    if app.file_history.is_some() {
        file_rail(buf, a.rail, app);
    } else {
        rail(buf, a.rail, app);
    }
    let body = a.body;
    if app.not_a_repo() {
        put(
            buf,
            body.x,
            body.y,
            &format!("not a git repository · {}", util::tilde(&app.root)),
            style(t.modified, t.body),
            body.width,
        );
        return;
    }
    if !app.shown_history_loaded() {
        put(
            buf,
            body.x,
            body.y,
            "reading history…",
            style(t.muted, t.body),
            body.width,
        );
        return;
    }
    if app.shown_history().commits.is_empty() {
        match &app.file_history {
            Some(file) => no_commits_touch(buf, body, file, &t),
            None => {
                put(
                    buf,
                    body.x,
                    body.y,
                    "no commits yet",
                    style(t.muted, t.body),
                    body.width,
                );
            }
        }
        return;
    }
    let in_a_file = app.file_history.is_some();
    let shown = app.shown_history_mut();
    let sel = shown.sel.min(shown.commits.len() - 1);
    let c = shown.commits[sel].clone();
    if shown.show_diff {
        let files = shown
            .detail
            .as_ref()
            .filter(|(s, _)| *s == c.sha)
            .map(|(_, d)| d.clone());
        let mut head = Line::new()
            .push(c.short().to_string(), style(t.muted, t.body))
            .push(format!(" {}", c.subject), bold(style(t.text, t.body)));
        head = head.push("   ⌫ back", style(t.muted, t.body));
        head.draw(buf, body.x, body.y, body.width);
        match files {
            Some(files) => {
                let area = Rect::new(
                    body.x,
                    body.y + 2,
                    body.width,
                    body.height.saturating_sub(2),
                );
                let total =
                    super::changes::render_diff_list(buf, area, &files, shown.diff_scroll, &t);
                shown.diff_scroll = shown.diff_scroll.min(total.saturating_sub(1));
            }
            None => {
                put(
                    buf,
                    body.x,
                    body.y + 2,
                    "reading the diff…",
                    style(t.muted, t.body),
                    body.width,
                );
            }
        }
        return;
    }
    let mut y = body.y;
    put(
        buf,
        body.x,
        y,
        c.short(),
        style(t.muted, t.body),
        body.width,
    );
    y += 1;
    for chunk in wrap(&c.subject, body.width as usize) {
        put(
            buf,
            body.x,
            y,
            &chunk,
            bold(style(t.text, t.body)),
            body.width,
        );
        y += 1;
    }
    let mut who = Line::new().push(
        format!("{} · {}", c.author, util::date_time(c.time)),
        style(t.muted, t.body),
    );
    let labels = c.labels();
    if !labels.is_empty() {
        who = who.push(format!("   {}", labels.join("  ")), style(t.branch, t.body));
    }
    who.draw(buf, body.x, y, body.width);
    y += 2;
    match shown.detail.as_ref().filter(|(s, _)| *s == c.sha) {
        Some((_, files)) => {
            for f in files {
                if y >= body.y + body.height {
                    break;
                }
                let lc = match f.status {
                    FileStatus::Added | FileStatus::Untracked => t.add,
                    FileStatus::Deleted => t.del,
                    FileStatus::Renamed | FileStatus::Copied => t.renamed,
                    FileStatus::Modified => t.modified,
                };
                let mut line = Line::new()
                    .push(format!("{} ", f.status.letter()), style(lc, t.body))
                    .push(
                        util::pad(
                            &file_line(f, in_a_file),
                            (body.width as usize).saturating_sub(16).min(44),
                        ),
                        style(t.text, t.body),
                    );
                if f.adds() > 0 {
                    line = line.push(format!(" +{}", f.adds()), style(t.add, t.body));
                }
                if f.dels() > 0 {
                    line = line.push(format!(" −{}", f.dels()), style(t.del, t.body));
                }
                line.draw(buf, body.x, y, body.width);
                y += 1;
            }
        }
        None => {
            put(
                buf,
                body.x,
                y,
                "reading files…",
                style(t.muted, t.body),
                body.width,
            );
        }
    }
}

fn wrap(s: &str, w: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        if !cur.is_empty() && util::width(&cur) + 1 + util::width(word) > w {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.truncate(3);
    out
}

fn file_rail(buf: &mut Buffer, area: Rect, app: &mut App) {
    if area.width == 0 {
        return;
    }
    let t = app.theme;
    let Some(file) = app.file_history.as_mut() else {
        return;
    };
    let rows = (area.height as usize).saturating_sub(1).max(1);
    let log = &mut file.log;
    if log.sel < log.scroll {
        log.scroll = log.sel;
    } else if log.sel >= log.scroll + rows {
        log.scroll = log.sel + 1 - rows;
    }
    let file = &*file;
    let mut r = Rail::new(buf, area, t);
    r.section(&format!("history of {}", file.path));
    if let Some(e) = &file.log.error {
        r.line(Line::new().push(format!("git: {e}"), style(t.del, t.rail)));
    }
    for (i, c) in file
        .log
        .commits
        .iter()
        .enumerate()
        .skip(file.log.scroll)
        .take(rows)
    {
        let sel = i == file.log.sel;
        let bg = if sel { t.active_row } else { t.rail };
        let subject = if sel {
            bold(style(t.text, bg))
        } else {
            style(t.text, bg)
        };
        let mut line = Line::new()
            .bg(bg)
            .push(format!("{} ", c.short()), style(t.muted, bg))
            .push(c.subject.clone(), subject);
        let labels = c.labels();
        if !labels.is_empty() {
            line = line.push(format!("  {}", labels.join("  ")), style(t.branch, bg));
        }
        if let Some(from) = file.renamed_from(i) {
            let how = if file.copied_from(i) {
                "copied"
            } else {
                "renamed"
            };
            line = line.push(format!("  {how} from {from}"), style(t.renamed, bg));
        }
        r.line(line);
    }
}

fn rail(buf: &mut Buffer, area: Rect, app: &mut App) {
    if area.width == 0 {
        return;
    }
    let t = app.theme;
    let graph_h = (area.height as usize).saturating_sub(1);
    let sel_row = app
        .history
        .rows
        .iter()
        .position(|r| r.commit == Some(app.history.sel))
        .unwrap_or(0);
    if sel_row < app.history.scroll {
        app.history.scroll = sel_row;
    } else if sel_row >= app.history.scroll + graph_h.max(1) {
        app.history.scroll = sel_row + 1 - graph_h.max(1);
    }
    let mut r = Rail::new(buf, area, t);
    r.section("history");
    if let Some(e) = &app.history.error {
        r.line(Line::new().push(format!("git: {e}"), style(t.del, t.rail)));
    }
    let lane_w = app
        .history
        .rows
        .iter()
        .map(|g| g.text().chars().count())
        .max()
        .unwrap_or(1)
        .min(24);
    for (ri, row) in app
        .history
        .rows
        .iter()
        .enumerate()
        .skip(app.history.scroll)
        .take(graph_h)
    {
        let sel = row.commit == Some(app.history.sel);
        let bg = if sel { t.active_row } else { t.rail };
        let mut line = Line::new().bg(bg);
        let cells: String = row.cells.iter().collect();
        let mut lane = String::new();
        let mut last_color = None;
        for (i, ch) in cells.chars().enumerate().take(lane_w) {
            let color = if ch == '●' {
                if app.history.commits[row.commit.unwrap_or(0)].is_head() {
                    t.add
                } else {
                    t.text
                }
            } else {
                lane_color(app, row.colors.get(i).copied().unwrap_or(0))
            };
            if last_color.is_some_and(|c| c != color) {
                line = line.push(std::mem::take(&mut lane), style(last_color.unwrap(), bg));
            }
            lane.push(ch);
            last_color = Some(color);
        }
        if !lane.is_empty() {
            line = line.push(lane, style(last_color.unwrap_or(t.rule), bg));
        }
        let pad = lane_w.saturating_sub(cells.chars().count().min(lane_w));
        line = line.push(" ".repeat(pad + 1), style(t.muted, bg));
        if let Some(ci) = row.commit {
            let c = &app.history.commits[ci];
            line = line
                .push(format!("{} ", c.short()), style(t.muted, bg))
                .push(
                    c.subject.clone(),
                    if sel {
                        bold(style(t.text, bg))
                    } else {
                        style(t.text, bg)
                    },
                );
            let labels = c.labels();
            if !labels.is_empty() {
                line = line.push(format!("  {}", labels.join("  ")), style(t.branch, bg));
            }
        }
        let _ = ri;
        r.line(line);
    }
}
