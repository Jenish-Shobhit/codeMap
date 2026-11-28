use std::collections::BTreeSet;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::{bold, put, style, Areas, Line, Rail};
use crate::app::{App, Focus};
use crate::files::{ancestors, human_size, Body, Preview, Row, PREVIEW_BYTES};
use crate::git::{FileDiff, FileStatus};
use crate::theme::Theme;
use crate::util;

pub fn render(buf: &mut Buffer, a: &Areas, app: &mut App) {
    app.ensure_preview();
    let rows = app.file_rows();
    app.files.sel = app.files.sel.min(rows.len().saturating_sub(1));
    rail(buf, a.rail, app, &rows);
    let t = app.theme;
    let body = a.body;
    if !app.files.loaded {
        note(buf, body, &t, "reading files…");
        return;
    }
    let Some(row) = rows.get(app.files.sel) else {
        let empty = if app.files.filter.is_some() {
            "no file matches"
        } else {
            "no files here"
        };
        note(buf, body, &t, empty);
        return;
    };
    if row.dir {
        folder(buf, body, app, row);
    } else if let Some(preview) = app.files.preview.clone() {
        file(buf, body, app, &preview);
    }
}

fn note(buf: &mut Buffer, body: Rect, t: &Theme, text: &str) {
    put(
        buf,
        body.x,
        body.y,
        text,
        style(t.muted, t.body),
        body.width,
    );
}

pub(super) fn status_color(t: &Theme, status: FileStatus) -> Color {
    match status {
        FileStatus::Added | FileStatus::Untracked => t.add,
        FileStatus::Deleted => t.del,
        FileStatus::Renamed | FileStatus::Copied => t.renamed,
        FileStatus::Modified => t.modified,
    }
}

pub(super) fn counts(line: Line<'static>, d: &FileDiff, t: &Theme, bg: Color) -> Line<'static> {
    let mut line = line;
    if d.adds() > 0 {
        line = line.push(format!(" +{}", d.adds()), style(t.add, bg));
    }
    if d.dels() > 0 {
        line = line.push(format!(" −{}", d.dels()), style(t.del, bg));
    }
    line
}

fn folder(buf: &mut Buffer, body: Rect, app: &App, row: &Row) {
    let t = app.theme;
    let total = app.files.tree.count_under(&row.path);
    Line::new()
        .push(format!("{}/", row.path), bold(style(t.text, t.body)))
        .push(
            format!("   {}", util::plural(total, "file", "files")),
            style(t.muted, t.body),
        )
        .draw(buf, body.x, body.y, body.width);
    let changed = app.changed_under(&row.path);
    let noun = if app.scope_info.noun.is_empty() {
        "this scope".to_string()
    } else {
        app.scope_info.noun.clone()
    };
    if changed.is_empty() {
        put(
            buf,
            body.x,
            body.y + 2,
            &format!("nothing changed here in {noun}"),
            style(t.muted, t.body),
            body.width,
        );
        return;
    }
    put(
        buf,
        body.x,
        body.y + 2,
        &format!("changed in {noun}"),
        style(t.text2, t.body),
        body.width,
    );
    let room = body.height.saturating_sub(3) as usize;
    for (i, d) in changed.iter().take(room).enumerate() {
        let inside = d.path[row.path.len()..].trim_start_matches('/');
        let line = Line::new()
            .push(
                format!("{} ", d.status.letter()),
                style(status_color(&t, d.status), t.body),
            )
            .push(inside.to_string(), style(t.text, t.body));
        counts(line, d, &t, t.body).draw(buf, body.x, body.y + 3 + i as u16, body.width);
    }
}

fn file(buf: &mut Buffer, body: Rect, app: &mut App, preview: &Preview) {
    let t = app.theme;
    let mut head = Line::new().push(preview.path.clone(), bold(style(t.text, t.body)));
    let size = human_size(preview.size);
    let facts = match &preview.body {
        Body::Text(lines) => format!("   {size} · {}", util::plural(lines.len(), "line", "lines")),
        _ => format!("   {size}"),
    };
    head = head.push(facts, style(t.muted, t.body));
    if let Some(d) = app.file_diff(&preview.path) {
        head = head.push(
            format!("   {}", d.status.letter()),
            style(status_color(&t, d.status), t.body),
        );
        head = counts(head, d, &t, t.body);
    }
    head.draw(buf, body.x, body.y, body.width);
    let lines = match &preview.body {
        Body::Text(lines) => lines,
        Body::Binary => {
            let at = Rect::new(body.x, body.y + 2, body.width, 1);
            note(buf, at, &t, "binary · b opens it in the browser");
            return;
        }
        Body::Empty => {
            let at = Rect::new(body.x, body.y + 2, body.width, 1);
            note(buf, at, &t, "empty file");
            return;
        }
        Body::Unreadable(e) => {
            put(
                buf,
                body.x,
                body.y + 2,
                &format!("cannot read this file: {e}"),
                style(t.del, t.body),
                body.width,
            );
            return;
        }
    };
    let view_h = body.height.saturating_sub(1) as usize;
    app.files.preview_scroll = app.files.preview_scroll.min(lines.len().saturating_sub(1));
    let top = app.files.preview_scroll;
    let added = app.added_lines(&preview.path);
    let digits = lines.len().to_string().len().max(4);
    let focused = app.files.focus == Focus::Body;
    for (i, text) in lines.iter().enumerate().skip(top).take(view_h) {
        let y = body.y + 1 + (i - top) as u16;
        let number = i as u32 + 1;
        let is_added = added.contains(&number);
        let is_top = focused && i == top;
        let bg = if is_top { t.active_row } else { t.body };
        Line::new()
            .bg(bg)
            .push(format!("{number:>digits$}"), style(t.muted, bg))
            .push(if is_added { "+" } else { " " }, style(t.add, bg))
            .push(" ", style(t.text, bg))
            .push(
                util::sanitize_line(text),
                style(if is_added { t.text } else { t.text2 }, bg),
            )
            .draw(buf, body.x, y, body.width);
    }
    let shown_all = top + view_h >= lines.len();
    if preview.truncated && shown_all && lines.len() - top < view_h {
        let y = body.y + 1 + (lines.len() - top) as u16;
        put(
            buf,
            body.x,
            y,
            &format!("… the first {} are shown", human_size(PREVIEW_BYTES)),
            style(t.muted, t.body),
            body.width,
        );
    }
}

fn rail(buf: &mut Buffer, area: Rect, app: &mut App, rows: &[Row]) {
    if area.width == 0 {
        return;
    }
    let t = app.theme;
    let filtering = app.files.filter.as_deref().is_some_and(|q| !q.is_empty());
    let title = if filtering {
        format!("matches · {}", rows.len())
    } else if app.files.truncated {
        format!("files · first {}", app.files.tree.len())
    } else {
        format!("files · {}", app.files.tree.len())
    };
    let list_h = (area.height as usize).saturating_sub(3);
    if app.files.sel < app.files.scroll {
        app.files.scroll = app.files.sel;
    } else if list_h > 0 && app.files.sel >= app.files.scroll + list_h {
        app.files.scroll = app.files.sel + 1 - list_h;
    }
    let changed_dirs: BTreeSet<&str> = app.diffs.iter().flat_map(|d| ancestors(&d.path)).collect();
    let focused = app.files.focus == Focus::Rail;
    let name_w = (area.width as usize).saturating_sub(4);
    let mut r = Rail::new(buf, area, t);
    r.section(&title);
    for (i, row) in rows.iter().enumerate().skip(app.files.scroll).take(list_h) {
        let selected = i == app.files.sel;
        let bg = if selected { t.active_row } else { t.rail };
        let label = if filtering {
            util::truncate_left(&row.path, name_w)
        } else if row.dir {
            let arrow = if app.files.open.contains(&row.path) {
                "▾"
            } else {
                "▸"
            };
            format!("{}{arrow} {}/", "  ".repeat(row.depth), row.name())
        } else {
            format!("{}  {}", "  ".repeat(row.depth), row.name())
        };
        let name_style = match (row.dir, selected && focused) {
            (_, true) => bold(style(t.text, bg)),
            (true, false) => style(t.text2, bg),
            (false, false) => style(t.text, bg),
        };
        let (mark, mark_color) = if row.dir {
            match changed_dirs.contains(row.path.as_str()) {
                true => ('●', t.modified),
                false => (' ', t.muted),
            }
        } else {
            match app.file_diff(&row.path) {
                Some(d) => (d.status.letter(), status_color(&t, d.status)),
                None => (' ', t.muted),
            }
        };
        r.line(
            Line::new()
                .bg(bg)
                .push(util::pad(&label, name_w), name_style)
                .push(format!(" {mark}"), style(mark_color, bg)),
        );
    }
    let scope = app
        .scope_info
        .fallback_note
        .clone()
        .unwrap_or_else(|| app.scope_info.rail.clone());
    r.footer(
        Line::new()
            .push("marks  ", bold(style(t.text2, t.rail)))
            .push(scope, style(t.muted, t.rail)),
    );
}
