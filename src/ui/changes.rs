//! Changes view: agents and the turn's files on the rail; the selected
//! file's hunks, reviewed marks and draft comments in the body.

use std::collections::BTreeMap;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use super::{bold, put, style, Areas, Line, Rail};
use crate::app::{App, CRow, Focus};
use crate::git::{DiffLine, FileDiff, FileStatus, LineKind};
use crate::index::parent_dir;
use crate::theme::Theme;
use crate::util;

pub fn render(buf: &mut Buffer, a: &Areas, app: &mut App) {
    rail(buf, a.rail, app);
    let t = app.theme;
    let body = a.body;
    if app.not_a_repo() {
        put(buf, body.x, body.y, &format!("not a git repository · {}", util::tilde(&app.root)), style(t.modified, t.body), body.width);
        put(buf, body.x, body.y + 1, "Map and Flow still work.", style(t.muted, t.body), body.width);
        return;
    }
    if !app.repo_known || !app.diffs_loaded {
        put(buf, body.x, body.y, "reading git status…", style(t.muted, t.body), body.width);
        return;
    }
    if let Some(e) = &app.diff_error {
        put(buf, body.x, body.y, &format!("git: {e}"), style(t.del, t.body), body.width);
        return;
    }
    if app.diffs.is_empty() {
        let msg = match (&app.scope_info.turn_n, app.scope_info.fallback_note.is_none()) {
            (Some(n), true) if matches!(app.scope, crate::app::Scope::Turn) => format!("turn {n} changed nothing"),
            _ => {
                let subject = app.history.commits.first().map(|c| c.subject.clone()).unwrap_or_default();
                if subject.is_empty() {
                    "no changes since HEAD".to_string()
                } else {
                    format!("no changes since HEAD · {subject}")
                }
            }
        };
        put(buf, body.x, body.y, &msg, style(t.muted, t.body), body.width);
        return;
    }
    let file_idx = app.changes.file.min(app.diffs.len() - 1);
    let f = app.diffs[file_idx].clone();
    // File header.
    let (done, total) = app.reviewed_count(file_idx);
    let mut head = Line::new().push(f.path.clone(), bold(style(t.text, t.body)));
    if let Some(old) = &f.old_path {
        head = head.push(format!("  ← {old}"), style(t.renamed, t.body));
    }
    head = head.push("   ", style(t.muted, t.body));
    if f.adds() > 0 {
        head = head.push(format!("+{}", f.adds()), style(t.add, t.body));
    }
    if f.dels() > 0 {
        head = head.push(format!(" −{}", f.dels()), style(t.del, t.body));
    }
    if total > 0 {
        let st = if done == total { style(t.add, t.body) } else { style(t.muted, t.body) };
        head = head.push(format!("    ✓ {done} of {total}"), st);
    }
    head.draw(buf, body.x, body.y, body.width);
    if f.binary {
        let size = std::fs::metadata(app.root.join(&f.path)).map(|m| m.len()).unwrap_or(0);
        put(buf, body.x, body.y + 2, &format!("binary · {}", human(size)), style(t.modified, t.body), body.width);
        return;
    }
    if f.hunks.is_empty() {
        let what = match f.status {
            FileStatus::Renamed => "renamed, content unchanged",
            FileStatus::Deleted => "deleted",
            _ => "no text changes",
        };
        put(buf, body.x, body.y + 2, what, style(t.muted, t.body), body.width);
        return;
    }
    let rows = app.change_rows(file_idx);
    let view_h = body.height.saturating_sub(1) as usize;
    // Keep the cursor visible.
    let cur = app.changes.row.min(rows.len().saturating_sub(1));
    if cur < app.changes.scroll {
        app.changes.scroll = cur;
    } else if cur >= app.changes.scroll + view_h {
        app.changes.scroll = cur + 1 - view_h;
    }
    let sel_range = app.changes.anchor.map(|a| (a.min(cur), a.max(cur)));
    let marks = app.marks.symbols.get(&f.path);
    let _ = marks;
    for (i, row) in rows.iter().enumerate().skip(app.changes.scroll).take(view_h) {
        let y = body.y + 1 + (i - app.changes.scroll) as u16;
        let is_cur = i == cur && app.changes.focus == Focus::Body;
        let in_sel = sel_range.is_some_and(|(a, b)| i >= a && i <= b);
        match row {
            CRow::Hunk(hi) => {
                let h = &f.hunks[*hi];
                let name = hunk_name(app, &f, h.new_start, h);
                let reviewed = app.reviews.is_reviewed(&f.path, &h.hash());
                let bg = if is_cur { t.active_row } else { t.body };
                let mut line = Line::new()
                    .bg(bg)
                    .push("@@ ", style(t.rule, bg))
                    .push(name, style(t.text2, bg))
                    .push(format!(" · {}–{}", h.new_start, h.new_end()), style(t.muted, bg));
                if reviewed {
                    line = line.push("   ✓ reviewed", style(t.add, bg));
                }
                line.draw(buf, body.x, y, body.width);
            }
            CRow::Line(hi, li) => {
                let dl = &f.hunks[*hi].lines[*li];
                diff_line(buf, body.x, y, body.width, dl, &t, is_cur, in_sel);
            }
            CRow::Comment(ci) => {
                let c = &app.drafts[*ci];
                let bg = if is_cur { t.active_row } else { t.body };
                Line::new()
                    .bg(bg)
                    .push("        ⎿ ", style(t.renamed, bg))
                    .push(c.text.clone(), style(t.renamed, bg))
                    .draw(buf, body.x, y, body.width);
            }
        }
    }
}

/// Name for a hunk header: the function around its first change.
fn hunk_name(app: &App, f: &FileDiff, _start: u32, h: &crate::git::Hunk) -> String {
    let first_change = h
        .lines
        .iter()
        .find(|l| l.kind != LineKind::Context)
        .and_then(|l| l.new_no)
        .unwrap_or(h.new_start);
    if let Some(fs) = app.index.symbols(&f.path) {
        if let Some(i) = fs.callable_at_line(first_change) {
            return fs.symbols[i].qual();
        }
    }
    if !h.section.is_empty() {
        return util::truncate(&h.section, 40);
    }
    "top level".into()
}

fn human(n: u64) -> String {
    if n < 1024 {
        format!("{n} B")
    } else {
        format!("{} KB", n / 1024)
    }
}

pub fn diff_line(buf: &mut Buffer, x: u16, y: u16, w: u16, dl: &DiffLine, t: &Theme, cursor: bool, selected: bool) {
    let (sign, band, sign_color, text_color): (&str, Color, Color, Color) = match dl.kind {
        LineKind::Add => ("+", t.add_bg, t.add, t.text),
        LineKind::Del => ("−", t.del_bg, t.del, t.text),
        LineKind::Context => (" ", t.body, t.muted, t.text2),
    };
    let bg = if selected {
        t.selection
    } else if cursor && dl.kind == LineKind::Context {
        t.active_row
    } else {
        band
    };
    let no = dl.new_no.or(dl.old_no).map(|n| n.to_string()).unwrap_or_default();
    let marker = if cursor { "›" } else { " " };
    let st = |c: Color| -> Style { style(c, bg) };
    let text = util::sanitize_line(&dl.text);
    Line::new()
        .bg(bg)
        .push(marker, bold(st(t.text)))
        .push(format!("{no:>4}"), st(t.muted))
        .push(sign, st(sign_color))
        .push(" ", st(text_color))
        .push(text, if cursor { bold(st(text_color)) } else { st(text_color) })
        .draw(buf, x, y, w);
}

fn rail(buf: &mut Buffer, area: Rect, app: &App) {
    if area.width == 0 {
        return;
    }
    let t = app.theme;
    let mut r = Rail::new(buf, area, t);
    if !app.agents.is_empty() {
        r.section("agents");
        for a in &app.agents {
            let bg = t.rail;
            let (dot, c) = if a.working { ("● ", t.add) } else { ("○ ", t.muted) };
            let mut line = Line::new()
                .push(dot, style(c, bg))
                .push(util::pad(&a.label, 14), if a.selected { bold(style(t.text, bg)) } else { style(t.text2, bg) })
                .push(format!(" {}", util::plural(a.files, "file", "files")), style(t.muted, bg));
            if a.adds > 0 {
                line = line.push(format!(" +{}", a.adds), style(t.add, bg));
            }
            if a.dels > 0 {
                line = line.push(format!(" −{}", a.dels), style(t.del, bg));
            }
            r.line(line);
        }
        r.blank();
    }
    let noun = if app.scope_info.noun.is_empty() { "changes".to_string() } else { app.scope_info.noun.clone() };
    r.section(&format!("{noun} · {}", util::plural(app.diffs.len(), "file", "files")));
    // Files grouped by folder.
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, f) in app.diffs.iter().enumerate() {
        groups.entry(parent_dir(&f.path).to_string()).or_default().push(i);
    }
    let footer = 2 + if app.drafts.is_empty() { 0 } else { 2 };
    'outer: for (dir, idxs) in &groups {
        if r.room() <= footer {
            break;
        }
        r.line(Line::new().push(if dir.is_empty() { "./".to_string() } else { format!("{dir}/") }, style(t.text2, t.rail)));
        for &i in idxs {
            if r.room() <= footer {
                r.line(Line::new().push("…", style(t.muted, t.rail)));
                break 'outer;
            }
            let f = &app.diffs[i];
            let sel = i == app.changes.file;
            let bg = if sel { t.active_row } else { t.rail };
            let letter = f.status.letter();
            let lc = match f.status {
                FileStatus::Added => t.add,
                FileStatus::Deleted => t.del,
                FileStatus::Renamed | FileStatus::Copied => t.renamed,
                FileStatus::Modified => t.modified,
            };
            let name = f.path.rsplit('/').next().unwrap_or(&f.path);
            let (done, total) = app.reviewed_count(i);
            let mut line = Line::new()
                .bg(bg)
                .push(format!("{letter} "), style(lc, bg))
                .push(util::pad(name, 20), if sel { bold(style(t.text, bg)) } else { style(t.text, bg) });
            if f.binary {
                line = line.push(" bin", style(t.modified, bg));
            } else {
                if f.adds() > 0 {
                    line = line.push(format!(" +{}", f.adds()), style(t.add, bg));
                }
                if f.dels() > 0 {
                    line = line.push(format!(" −{}", f.dels()), style(t.del, bg));
                }
            }
            if total > 0 && done == total {
                line = line.push(" ✓", style(t.add, bg));
            }
            r.line(line);
        }
    }
    if !app.drafts.is_empty() {
        r.blank();
        r.line(
            Line::new()
                .push("draft  ", bold(style(t.text2, t.rail)))
                .push(util::plural(app.drafts.len(), "comment", "comments"), style(t.renamed, t.rail))
                .push(" · P sends", style(t.muted, t.rail)),
        );
    }
    let scope = app.scope_info.fallback_note.clone().unwrap_or_else(|| app.scope_info.rail.clone());
    r.footer(
        Line::new()
            .push("scope  ", bold(style(t.text2, t.rail)))
            .push(scope, style(t.muted, t.rail)),
    );
}

/// All files of a diff as rows (History's commit diff).
pub fn render_diff_list(buf: &mut Buffer, area: Rect, files: &[FileDiff], scroll: usize, t: &Theme) -> usize {
    let mut rows: Vec<Line> = Vec::new();
    for f in files {
        let mut head = Line::new().push(f.path.clone(), bold(style(t.text, t.body)));
        if f.adds() > 0 {
            head = head.push(format!("  +{}", f.adds()), style(t.add, t.body));
        }
        if f.dels() > 0 {
            head = head.push(format!(" −{}", f.dels()), style(t.del, t.body));
        }
        rows.push(head);
        if f.binary {
            rows.push(Line::new().push("  binary", style(t.modified, t.body)));
        }
        for h in &f.hunks {
            let mut head = Line::new().push("@@ ", style(t.rule, t.body));
            if h.section.is_empty() {
                head = head.push(format!("lines {}–{}", h.new_start, h.new_end()), style(t.muted, t.body));
            } else {
                head = head
                    .push(util::truncate(&h.section, 50), style(t.text2, t.body))
                    .push(format!(" · {}–{}", h.new_start, h.new_end()), style(t.muted, t.body));
            }
            rows.push(head);
            for dl in &h.lines {
                let (sign, bg, sc, tc) = match dl.kind {
                    LineKind::Add => ("+", t.add_bg, t.add, t.text),
                    LineKind::Del => ("−", t.del_bg, t.del, t.text),
                    LineKind::Context => (" ", t.body, t.muted, t.text2),
                };
                let no = dl.new_no.or(dl.old_no).map(|n| n.to_string()).unwrap_or_default();
                rows.push(
                    Line::new()
                        .bg(bg)
                        .push(format!(" {no:>4}"), style(t.muted, bg))
                        .push(sign, style(sc, bg))
                        .push(" ", style(tc, bg))
                        .push(util::sanitize_line(&dl.text), style(tc, bg)),
                );
            }
        }
        rows.push(Line::new());
    }
    let total = rows.len();
    for (i, line) in rows.iter().skip(scroll).take(area.height as usize).enumerate() {
        line.draw(buf, area.x, area.y + i as u16, area.width);
    }
    total
}
