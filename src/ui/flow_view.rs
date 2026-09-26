//! Flow view: the function list and its calls on the rail, the flowchart in
//! the body.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::{blit, bold, follow, put, style, Areas, Line, Rail};
use crate::app::{App, Focus};
use crate::index::SymId;
use crate::util;

pub fn render(buf: &mut Buffer, a: &Areas, app: &mut App) {
    app.ensure_flow();
    rail(buf, a.rail, app);
    let t = app.theme;
    let body = a.body;
    if let Some(err) = &app.flow.error {
        put(
            buf,
            body.x,
            body.y,
            err,
            style(t.modified, t.body),
            body.width,
        );
        return;
    }
    let (Some(_chart), Some(layout)) = (&app.flow.chart, &app.flow.layout) else {
        let msg = if !app.files_loaded || app.parse_pending {
            "reading functions…"
        } else {
            "pick a function: move to one in the map (1) and press 2, or search with /"
        };
        put(buf, body.x, body.y, msg, style(t.muted, t.body), body.width);
        return;
    };
    let Some(target) = app.flow.target.clone() else {
        return;
    };
    let sym = app.flow_symbol().cloned();
    // Header: name, where, called from.
    let mut head = Line::new();
    if let Some(s) = &sym {
        head = head
            .push(format!("{}()", s.qual()), bold(style(t.text, t.body)))
            .push(
                format!("  {} · lines {}–{}", target.file, s.start_line, s.end_line),
                style(t.muted, t.body),
            );
    }
    let callers = callers_text(app, &target);
    if !callers.is_empty() {
        head = head.push(format!("   called from {callers}"), style(t.muted, t.body));
    }
    head.draw(buf, body.x, body.y, body.width);
    // Legend when anything is marked.
    let marks = app.flow_line_marks();
    let view = Rect::new(
        body.x,
        body.y + 2,
        body.width,
        body.height.saturating_sub(2),
    );
    let selected = (app.flow.focus == Focus::Body).then_some(app.flow.selected);
    let canvas = layout.draw(&marks, selected);
    let mut scroll = app.flow.scroll;
    if let Some(b) = layout.boxes.get(app.flow.selected) {
        follow(
            &mut scroll,
            (view.width as usize, view.height as usize),
            (
                b.x.saturating_sub(crate::flow::GUTTER),
                b.y,
                b.w + crate::flow::GUTTER,
                b.h,
            ),
            (canvas.w, canvas.h),
        );
    }
    // Keep the change-mark gutter visible when the chart fits.
    if canvas.w <= view.width as usize {
        scroll.0 = 0;
    }
    app.flow.scroll = scroll;
    blit(buf, view, &canvas, scroll, &t, t.body);
    let in_range = sym
        .as_ref()
        .is_some_and(|s| marks.range(s.start_line..=s.end_line).next().is_some());
    if in_range {
        let legend = Line::new()
            .push("A", style(t.add, t.body))
            .push(" added  ", style(t.muted, t.body))
            .push("M", style(t.modified, t.body))
            .push(
                format!(" changed {}", app.scope_info.noun),
                style(t.muted, t.body),
            );
        let w = legend.width() as u16;
        if body.width > w + 2 {
            legend.draw(buf, body.x + body.width - w, body.y + 1, w);
        }
    }
}

fn callers_text(app: &App, target: &SymId) -> String {
    let mut names: Vec<String> = app
        .index
        .callers_of(target)
        .iter()
        .map(|c| match c.from_sym {
            Some(s) => app
                .index
                .symbols(&c.from_file)
                .and_then(|f| f.symbols.get(s))
                .map(|x| x.qual())
                .unwrap_or_default(),
            None => {
                let stem = c.from_file.rsplit('/').next().unwrap_or(&c.from_file);
                if stem.ends_with(".py") {
                    "__main__".to_string()
                } else {
                    format!("{stem} (top level)")
                }
            }
        })
        .collect();
    names.sort();
    names.dedup();
    let n = names.len();
    if n > 3 {
        format!("{} +{}", names[..3].join(", "), n - 3)
    } else {
        names.join(", ")
    }
}

fn rail(buf: &mut Buffer, area: Rect, app: &App) {
    if area.width == 0 {
        return;
    }
    let t = app.theme;
    let mut r = Rail::new(buf, area, t);
    r.section("flow");
    let Some(target) = &app.flow.target else {
        r.line(Line::new().push("no function selected", style(t.muted, t.rail)));
        return;
    };
    r.line(Line::new().push(
        util::truncate_left(&target.file, area.width as usize - 2),
        style(t.branch, t.rail),
    ));
    let marks = app.marks.symbols.get(&target.file);
    for (i, (idx, qual)) in app.flow_rail_functions().into_iter().enumerate() {
        if r.room() <= 8 {
            r.line(Line::new().push("…", style(t.muted, t.rail)));
            break;
        }
        let current = idx == target.idx;
        let sel = app.flow.focus == Focus::Rail && app.flow.rail_sel == i;
        let bg = if sel || (current && app.flow.focus == Focus::Body) {
            t.active_row
        } else {
            t.rail
        };
        let mark = marks.and_then(|m| m.get(&idx)).copied();
        let mut line = Line::new().bg(bg).push(
            util::pad(&qual, area.width.saturating_sub(5) as usize),
            if current {
                bold(style(t.text, bg))
            } else {
                style(t.text2, bg)
            },
        );
        if let Some(m) = mark {
            line = line.push(
                format!(" {m}"),
                style(if m == 'A' { t.add } else { t.modified }, bg),
            );
        }
        r.line(line);
    }
    r.blank();
    let calls = app.index.calls_from(&target.file, target.idx);
    if !calls.is_empty() {
        r.section("calls");
        let mut seen = Vec::new();
        for c in calls {
            if seen.contains(&c.to) {
                continue;
            }
            seen.push(c.to.clone());
            if r.room() <= 3 {
                break;
            }
            let name = app
                .index
                .symbols(&c.to.file)
                .and_then(|f| f.symbols.get(c.to.idx))
                .map(|s| s.qual())
                .unwrap_or_default();
            let stem =
                c.to.file
                    .rsplit('/')
                    .next()
                    .unwrap_or(&c.to.file)
                    .to_string();
            r.line(
                Line::new()
                    .push(util::pad(&name, 22), style(t.text2, t.rail))
                    .push(" ", style(t.muted, t.rail))
                    .push(stem, style(t.muted, t.rail)),
            );
        }
    }
    r.footer(
        Line::new()
            .push("scope  ", bold(style(t.text2, t.rail)))
            .push(app.scope_info.rail.clone(), style(t.muted, t.rail)),
    );
}
