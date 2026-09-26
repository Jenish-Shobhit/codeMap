//! History view: the commit graph (lanes, branches, tags, HEAD) and the
//! worktrees with their agents on a wide rail; the selected commit, or its
//! diff, in the body.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::{bold, put, style, Areas, Line, Rail};
use crate::app::App;
use crate::git::FileStatus;
use crate::util;

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
        util::plural(app.history.total.max(app.history.commits.len()), "commit", "commits"),
        util::plural(branches.len().max(1), "branch", "branches"),
        util::plural(app.history.worktrees.len().max(1), "worktree", "worktrees"),
        head
    ))
}

fn lane_color(app: &App, i: usize) -> Color {
    let t = &app.theme;
    [t.accent, t.renamed, t.add, t.modified, t.conflict, t.branch, t.text2][i % 7]
}

pub fn render(buf: &mut Buffer, a: &Areas, app: &mut App) {
    let t = app.theme;
    rail(buf, a.rail, app);
    let body = a.body;
    if app.not_a_repo() {
        put(buf, body.x, body.y, &format!("not a git repository · {}", util::tilde(&app.root)), style(t.modified, t.body), body.width);
        put(buf, body.x, body.y + 1, "Map and Flow still work.", style(t.muted, t.body), body.width);
        return;
    }
    if !app.history_loaded {
        put(buf, body.x, body.y, "reading history…", style(t.muted, t.body), body.width);
        return;
    }
    if app.history.commits.is_empty() {
        put(buf, body.x, body.y, "no commits yet", style(t.muted, t.body), body.width);
        return;
    }
    let sel = app.history.sel.min(app.history.commits.len() - 1);
    let c = app.history.commits[sel].clone();
    if app.history.show_diff {
        let files = app
            .history
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
                let area = Rect::new(body.x, body.y + 2, body.width, body.height.saturating_sub(2));
                let total = super::changes::render_diff_list(buf, area, &files, app.history.diff_scroll, &t);
                app.history.diff_scroll = app.history.diff_scroll.min(total.saturating_sub(1));
            }
            None => {
                put(buf, body.x, body.y + 2, "reading the diff…", style(t.muted, t.body), body.width);
            }
        }
        return;
    }
    let mut y = body.y;
    put(buf, body.x, y, c.short(), style(t.muted, t.body), body.width);
    y += 1;
    // Subject, wrapped.
    for chunk in wrap(&c.subject, body.width as usize) {
        put(buf, body.x, y, &chunk, bold(style(t.text, t.body)), body.width);
        y += 1;
    }
    let mut who = Line::new().push(format!("{} · {}", c.author, util::date_time(c.time)), style(t.muted, t.body));
    let labels = c.labels();
    if !labels.is_empty() {
        who = who.push(format!("   {}", labels.join("  ")), style(t.branch, t.body));
    }
    who.draw(buf, body.x, y, body.width);
    y += 2;
    match app.history.detail.as_ref().filter(|(s, _)| *s == c.sha) {
        Some((_, files)) => {
            for f in files {
                if y >= body.y + body.height {
                    break;
                }
                let lc = match f.status {
                    FileStatus::Added => t.add,
                    FileStatus::Deleted => t.del,
                    FileStatus::Renamed | FileStatus::Copied => t.renamed,
                    FileStatus::Modified => t.modified,
                };
                let mut line = Line::new()
                    .push(format!("{} ", f.status.letter()), style(lc, t.body))
                    .push(util::pad(&f.path, (body.width as usize).saturating_sub(16).min(44)), style(t.text, t.body));
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
            put(buf, body.x, y, "reading files…", style(t.muted, t.body), body.width);
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

fn rail(buf: &mut Buffer, area: Rect, app: &mut App) {
    if area.width == 0 {
        return;
    }
    let t = app.theme;
    let wt_rows = if app.history.worktrees.is_empty() { 0 } else { app.history.worktrees.len().min(4) + 2 };
    let graph_h = (area.height as usize).saturating_sub(1 + wt_rows);
    // Scroll the graph so the selected commit stays visible.
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
    let lane_w = app.history.rows.iter().map(|g| g.text().chars().count()).max().unwrap_or(1).min(24);
    for (ri, row) in app.history.rows.iter().enumerate().skip(app.history.scroll).take(graph_h) {
        let sel = row.commit == Some(app.history.sel);
        let bg = if sel { t.active_row } else { t.rail };
        let mut line = Line::new().bg(bg);
        // lanes, coloured per lane
        let cells: String = row.cells.iter().collect();
        let mut lane = String::new();
        let mut last_color = None;
        for (i, ch) in cells.chars().enumerate().take(lane_w) {
            let color = if ch == '●' {
                if app.history.commits[row.commit.unwrap_or(0)].is_head() { t.add } else { t.text }
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
            line = line.push(format!("{} ", c.short()), style(t.muted, bg)).push(
                c.subject.clone(),
                if sel { bold(style(t.text, bg)) } else { style(t.text, bg) },
            );
            let labels = c.labels();
            if !labels.is_empty() {
                line = line.push(format!("  {}", labels.join("  ")), style(t.branch, bg));
            }
        }
        let _ = ri;
        r.line(line);
    }
    if !app.history.worktrees.is_empty() {
        r.y = area.y + area.height.saturating_sub(1 + wt_rows as u16);
        r.blank();
        r.section("worktrees");
        for w in app.history.worktrees.iter().take(4) {
            let branch_w = w.branch.as_ref().map(|b| util::width(b)).unwrap_or(0);
            let path_w = (area.width as usize).saturating_sub(branch_w + 6).max(12);
            let mut line = Line::new()
                .push(util::truncate_left(&util::tilde(&w.path), path_w), style(if w.current { t.text } else { t.text2 }, t.rail))
                .push("  ", style(t.muted, t.rail))
                .push(w.branch.clone().unwrap_or_default(), style(t.branch, t.rail));
            for (name, working) in &w.agents {
                line = line
                    .push("   ", style(t.muted, t.rail))
                    .push(if *working { "● " } else { "○ " }, style(if *working { t.add } else { t.muted }, t.rail))
                    .push(name.clone(), style(t.text2, t.rail));
            }
            r.line(line);
        }
    }
}
