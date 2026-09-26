//! Map view: breadcrumbs and this turn's changed functions on the rail, the
//! boxes-and-lines diagram in the body.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::{blit, bold, follow, put, style, Areas, Line, Rail};
use crate::app::{App, Focus, InputKind};
use crate::index::parent_dir;
use crate::map::Level;
use crate::util;

pub fn render(buf: &mut Buffer, a: &Areas, app: &mut App) {
    let body = a.body;
    app.ensure_scene(body.width as usize);
    rail(buf, a.rail, app);
    let t = app.theme;
    if !app.files_loaded {
        put(buf, body.x, body.y, "reading files…", style(t.muted, t.body), body.width);
        return;
    }
    let Some(scene) = &app.map.scene else { return };
    // Header: level and summary.
    let mut head = Line::new().push(scene.level.label(), bold(style(t.text, t.body)));
    let mut summary = scene.summary.clone();
    if app.parse_pending {
        summary = format!("{summary} · parsing {} files…", app.index.source_files().count().saturating_sub(app.index.parsed_count()));
    }
    if !app.marks.is_empty() && !app.scope_info.noun.is_empty() && summary.contains("changed") {
        summary = format!("{summary} {}", app.scope_info.noun);
    }
    head = head.push("  ", style(t.muted, t.body)).push(summary, style(t.muted, t.body));
    if scene.pages > 1 {
        head = head.push(format!("   page {} of {}", scene.page + 1, scene.pages), style(t.text2, t.body));
    }
    if app.index.truncated {
        head = head.push("   huge repo: first 20,000 files", style(t.modified, t.body));
    }
    head.draw(buf, body.x, body.y, body.width);
    if scene.nodes.is_empty() {
        put(buf, body.x, body.y + 2, "empty folder", style(t.muted, t.body), body.width);
        return;
    }
    let view = Rect::new(body.x, body.y + 2, body.width, body.height.saturating_sub(2));
    let Some(canvas) = app.map_canvas() else { return };
    let r = scene.layout.nodes[app.map.cursor.node.min(scene.layout.nodes.len() - 1)];
    let mut scroll = app.map.scroll;
    follow(
        &mut scroll,
        (view.width as usize, view.height as usize),
        (r.x, r.y, r.w, r.h),
        (canvas.w, canvas.h),
    );
    app.map.scroll = scroll;
    blit(buf, view, &canvas, scroll, &t, t.body);
}

fn rail(buf: &mut Buffer, area: Rect, app: &App) {
    if area.width == 0 {
        return;
    }
    let t = app.theme;
    let mut r = Rail::new(buf, area, t);
    // Search results replace the rail while typing.
    if let Some(input) = &app.input {
        if input.kind == InputKind::Search {
            r.section("search");
            if app.search_hits.is_empty() {
                r.line(Line::new().push(
                    if input.text.is_empty() { "type a symbol or file name" } else { "no match" },
                    style(t.muted, t.rail),
                ));
            }
            for (i, hit) in app.search_hits.iter().enumerate() {
                if r.room() <= 1 {
                    break;
                }
                let sel = i == app.search_sel;
                let bg = if sel { t.active_row } else { t.rail };
                let file = hit.file.rsplit('/').next().unwrap_or(&hit.file).to_string();
                let mut line = Line::new().bg(bg);
                if hit.sym.is_some() {
                    let room = (area.width as usize).saturating_sub(2);
                    let fits = util::width(&hit.label) + 2 + util::width(&file) <= room;
                    if fits {
                        let name_w = room.saturating_sub(util::width(&file) + 1);
                        line = line
                            .push(util::pad(&hit.label, name_w), style(t.text, bg))
                            .push(" ", style(t.muted, bg))
                            .push(file, style(t.branch, bg));
                    } else {
                        line = line.push(hit.label.clone(), style(t.text, bg));
                    }
                } else {
                    line = line.push(hit.label.clone(), style(t.text2, bg));
                }
                r.line(line);
            }
            return;
        }
    }
    r.section("map");
    // Breadcrumbs from the root to the current level.
    let level = app.map_level();
    let path = match &level {
        Level::Dir(d) => d.clone(),
        Level::File(f) => f.clone(),
    };
    let mut crumbs: Vec<String> = Vec::new();
    if !path.is_empty() {
        let mut acc = String::new();
        for seg in path.split('/') {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(seg);
            crumbs.push(acc.clone());
        }
    }
    let root_name = app
        .root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let root_count = app.index.files.len();
    r.line(
        Line::new()
            .push(format!("{} ", util::pad(&format!("{root_name}/"), 17)), style(if crumbs.is_empty() { t.text } else { t.text2 }, t.rail))
            .push(util::plural(root_count, "file", "files"), style(t.muted, t.rail)),
    );
    for (depth, c) in crumbs.iter().enumerate() {
        let is_file = matches!(level, Level::File(_)) && depth == crumbs.len() - 1;
        let name = c.rsplit('/').next().unwrap_or(c);
        let label = format!("{}{}{}", "  ".repeat(depth + 1), name, if is_file { "" } else { "/" });
        let count = if is_file {
            app.index
                .symbols(c)
                .map(|s| util::plural(s.symbols.iter().filter(|x| x.kind.is_callable()).count(), "function", "functions"))
                .unwrap_or_default()
        } else {
            util::plural(app.index.files.iter().filter(|f| crate::index::in_dir(f, c)).count(), "file", "files")
        };
        let last = depth == crumbs.len() - 1;
        r.line(
            Line::new()
                .push(format!("{} ", util::pad(&label, 17)), style(if last { t.text } else { t.text2 }, t.rail))
                .push(count, style(t.muted, t.rail)),
        );
    }
    r.blank();
    // Changed functions in the scope.
    let changed = app.marks.changed_symbols(&app.index);
    let noun = if app.scope_info.noun.is_empty() { "changed".to_string() } else { app.scope_info.noun.clone() };
    r.section(&format!("{noun} · {}", util::plural(changed.len(), "function", "functions")));
    if changed.is_empty() {
        let note = if !app.diffs_loaded && app.repo.is_some() {
            "reading changes…"
        } else if app.not_a_repo() {
            "not a git repository"
        } else if app.diffs.is_empty() {
            "nothing changed"
        } else {
            "changes are outside functions"
        };
        r.line(Line::new().push(note, style(t.muted, t.rail)));
    }
    let footer_rows = 2;
    for (i, (file, idx, mark)) in changed.iter().enumerate() {
        if r.room() <= footer_rows {
            let more = changed.len() - i;
            r.line(Line::new().push(format!("… {more} more"), style(t.muted, t.rail)));
            break;
        }
        let Some(s) = app.index.symbols(file).and_then(|f| f.symbols.get(*idx)) else { continue };
        let sel = app.map.focus == Focus::Rail && app.map.rail_sel == i;
        let bg = if sel { t.active_row } else { t.rail };
        let stem = file.rsplit('/').next().unwrap_or(file);
        let stem = stem.rsplit_once('.').map(|(a, _)| a).unwrap_or(stem);
        r.line(
            Line::new()
                .bg(bg)
                .push(format!("{mark} "), style(if *mark == 'A' { t.add } else { t.modified }, bg))
                .push(util::pad(&s.name, 18), if sel { bold(style(t.text, bg)) } else { style(t.text, bg) })
                .push(" ", style(t.muted, bg))
                .push(stem.to_string(), style(t.branch, bg)),
        );
    }
    let scope = if let Some(note) = &app.scope_info.fallback_note {
        note.clone()
    } else if app.not_a_repo() {
        "no git · structure only".into()
    } else {
        app.scope_info.rail.clone()
    };
    r.footer(
        Line::new()
            .push("scope  ", bold(style(t.text2, t.rail)))
            .push(scope, style(t.muted, t.rail)),
    );
    let _ = parent_dir;
}
