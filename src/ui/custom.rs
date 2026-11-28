use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::{bold, put, style, Areas, Line, Rail};
use crate::app::App;
use crate::custom::{Group, Item, Scope};
use crate::util;

const NAME_WIDTH: usize = 22;
const DESC_LINES: usize = 6;

pub fn render(buf: &mut Buffer, a: &Areas, app: &mut App) {
    let t = app.theme;
    let body = a.body;
    if !app.custom.loaded {
        put(
            buf,
            body.x,
            body.y,
            "reading customisations…",
            style(t.muted, t.body),
            body.width,
        );
        return;
    }
    let groups = app.custom.groups().to_vec();
    let selected = app.custom.selected_index();
    app.custom.scroll = rail(buf, a.rail, app, &groups, selected);
    if let Some(item) = app.custom.selected().cloned() {
        detail(buf, body, app, &item);
        return;
    }
    put(
        buf,
        body.x,
        body.y,
        nothing(app.custom.scope),
        style(t.muted, t.body),
        body.width,
    );
}

fn nothing(scope: Scope) -> &'static str {
    match scope {
        Scope::Project => "nothing is set up for Claude in this repository",
        Scope::Global => "nothing is set up for Claude in your home folder",
        Scope::Tools => "no tool configuration was found",
        Scope::Agents => "no other agent is set up here",
    }
}

fn rail(buf: &mut Buffer, area: Rect, app: &App, groups: &[Group], selected: usize) -> usize {
    if area.width == 0 {
        return app.custom.scroll;
    }
    let t = app.theme;
    let width = (area.width as usize).saturating_sub(2);
    let mut rows: Vec<(Line, bool)> = Vec::new();
    let mut item_index = 0;
    for g in groups {
        let count = g.items.len().to_string();
        let tone = if g.items.is_empty() { t.muted } else { t.text2 };
        let title = util::pad(&g.title, width.saturating_sub(count.len()));
        rows.push((
            Line::new()
                .push(title, bold(style(tone, t.rail)))
                .push(count, style(tone, t.rail)),
            false,
        ));
        for item in &g.items {
            let chosen = item_index == selected;
            let bg = if chosen { t.active_row } else { t.rail };
            let name_w = NAME_WIDTH.min(width.saturating_sub(2));
            let name_style = if chosen {
                bold(style(t.text, bg))
            } else {
                style(t.text, bg)
            };
            rows.push((
                Line::new()
                    .bg(bg)
                    .push("  ", style(t.muted, bg))
                    .push(util::pad(&item.name, name_w), name_style)
                    .push(format!(" {}", item.hint), style(t.muted, bg)),
                chosen,
            ));
            item_index += 1;
        }
    }
    let height = area.height as usize;
    let chosen_row = rows.iter().position(|(_, chosen)| *chosen).unwrap_or(0);
    let mut scroll = app.custom.scroll.min(rows.len().saturating_sub(1));
    if chosen_row <= scroll {
        scroll = chosen_row.saturating_sub(1);
    } else if height > 0 && chosen_row >= scroll + height {
        scroll = chosen_row + 1 - height;
    }
    let mut r = Rail::new(buf, area, t);
    for (line, _) in rows.into_iter().skip(scroll).take(height) {
        r.line(line);
    }
    scroll
}

fn wrap(text: &str, width: usize, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && util::width(&current) + 1 + util::width(word) > width {
            out.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        out.push(current);
    }
    if out.len() > limit {
        out.truncate(limit);
        if let Some(last) = out.last_mut() {
            last.push('…');
        }
    }
    out
}

fn detail(buf: &mut Buffer, body: Rect, app: &App, item: &Item) {
    let t = app.theme;
    let width = body.width as usize;
    let bottom = body.y + body.height;
    let kind_w = util::width(&item.kind) as u16;
    put(
        buf,
        body.x,
        body.y,
        &item.name,
        bold(style(t.text, t.body)),
        body.width.saturating_sub(kind_w + 2),
    );
    if kind_w < body.width {
        put(
            buf,
            body.x + body.width - kind_w,
            body.y,
            &item.kind,
            style(t.muted, t.body),
            kind_w,
        );
    }
    put(
        buf,
        body.x,
        body.y + 1,
        &util::truncate_left(&util::tilde(&item.path), width),
        style(t.muted, t.body),
        body.width,
    );
    let mut y = body.y + 3;
    if let Some(desc) = &item.desc {
        for line in wrap(desc, width.min(88), DESC_LINES) {
            if y >= bottom {
                return;
            }
            put(buf, body.x, y, &line, style(t.text, t.body), body.width);
            y += 1;
        }
        y += 1;
    }
    let key_w = item
        .facts
        .iter()
        .map(|(k, _)| util::width(k))
        .max()
        .unwrap_or(0);
    for (key, value) in &item.facts {
        if y >= bottom {
            return;
        }
        Line::new()
            .push(util::pad(key, key_w + 2), style(t.text2, t.body))
            .push(value.clone(), style(t.text, t.body))
            .draw(buf, body.x, y, body.width);
        y += 1;
    }
    if item.excerpt.is_empty() || y + 2 >= bottom {
        return;
    }
    y += 1;
    put(
        buf,
        body.x,
        y,
        &"─".repeat(width.min(88)),
        style(t.rule, t.body),
        body.width,
    );
    y += 1;
    for (number, text) in &item.excerpt {
        if y >= bottom {
            return;
        }
        Line::new()
            .push(format!("{number:>4}  "), style(t.muted, t.body))
            .push(util::sanitize_line(text), style(t.text2, t.body))
            .draw(buf, body.x, y, body.width);
        y += 1;
    }
}
