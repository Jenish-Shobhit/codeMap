//! Drawing. One frame: the view tabs and context on top, a black rail on the
//! left, the charcoal body on the right, one dim hint line at the bottom.
//! No boxes around panels: herdr draws the popup border (taste rule HC-2).

mod changes;
mod flow_view;
mod history;
mod map_view;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;

use crate::app::{App, Focus, InputKind, View};
use crate::canvas::Canvas;
use crate::theme::Theme;
use crate::util;

/// Areas of one frame.
#[derive(Debug, Clone, Copy)]
pub struct Areas {
    pub header: Rect,
    pub rail: Rect,
    pub body: Rect,
    pub hints: Rect,
    pub rail_hints: Rect,
}

pub fn areas(area: Rect, view: View) -> Areas {
    let w = area.width;
    let rail_w = if w < 70 {
        0
    } else if view == View::History {
        ((w as u32 * 60 / 100) as u16).clamp(40, w.saturating_sub(30))
    } else {
        ((w as u32 * 30 / 100) as u16).clamp(28, 40)
    };
    let content_y = area.y + 2.min(area.height);
    let content_h = area.height.saturating_sub(3);
    let hint_y = area.y + area.height.saturating_sub(1);
    Areas {
        header: Rect::new(area.x, area.y, w, 1.min(area.height)),
        rail: Rect::new(area.x, content_y, rail_w, content_h),
        body: Rect::new(
            area.x + rail_w + if rail_w > 0 { 2 } else { 1 },
            content_y,
            w.saturating_sub(rail_w + if rail_w > 0 { 3 } else { 2 }),
            content_h,
        ),
        hints: Rect::new(
            area.x + rail_w + if rail_w > 0 { 2 } else { 1 },
            hint_y,
            w.saturating_sub(rail_w + 2),
            1,
        ),
        rail_hints: Rect::new(area.x, hint_y, rail_w, 1),
    }
}

pub fn render(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let buf = f.buffer_mut();
    render_buf(buf, area, app);
}

pub fn render_buf(buf: &mut Buffer, area: Rect, app: &mut App) {
    let t = app.theme;
    fill(buf, area, t.body);
    if area.width < 20 || area.height < 6 {
        put(
            buf,
            area.x,
            area.y,
            "codeMorph: window too small",
            Style::default().fg(t.muted).bg(t.body),
            area.width,
        );
        return;
    }
    let a = areas(area, app.view);
    // Rail surface (including its part of the hint row).
    if a.rail.width > 0 {
        fill(
            buf,
            Rect::new(
                area.x,
                area.y + 1,
                a.rail.width + 1,
                area.height.saturating_sub(1),
            ),
            t.rail,
        );
    }
    header(buf, a.header, app);
    match app.view {
        View::Map => map_view::render(buf, &a, app),
        View::Flow => flow_view::render(buf, &a, app),
        View::Changes => changes::render(buf, &a, app),
        View::History => history::render(buf, &a, app),
    }
    hints_line(buf, &a, app);
    if app.show_help {
        help(buf, area, app);
    }
}

// ---- building blocks -----------------------------------------------------

pub fn fill(buf: &mut Buffer, r: Rect, bg: Color) {
    let r = r.intersection(buf.area);
    for y in r.y..r.y + r.height {
        for x in r.x..r.x + r.width {
            let c = &mut buf[(x, y)];
            c.set_symbol(" ");
            c.set_style(Style::default().bg(bg));
        }
    }
}

/// Write text clipped to `max` cells; returns the x after it.
pub fn put(buf: &mut Buffer, x: u16, y: u16, s: &str, style: Style, max: u16) -> u16 {
    if y >= buf.area.y + buf.area.height || max == 0 {
        return x;
    }
    let s = util::truncate(s, max as usize);
    let end = buf.area.x + buf.area.width;
    if x >= end {
        return x;
    }
    let (nx, _) = buf.set_stringn(x, y, &s, (end - x) as usize, style);
    nx
}

pub fn style(fg: Color, bg: Color) -> Style {
    Style::default().fg(fg).bg(bg)
}

pub fn bold(s: Style) -> Style {
    s.add_modifier(Modifier::BOLD)
}

/// A line of styled spans written left to right.
pub struct Line<'a> {
    pub spans: Vec<(String, Style)>,
    pub bg: Option<Color>,
    _p: std::marker::PhantomData<&'a ()>,
}

impl<'a> Line<'a> {
    pub fn new() -> Self {
        Line {
            spans: Vec::new(),
            bg: None,
            _p: std::marker::PhantomData,
        }
    }
    pub fn push(mut self, s: impl Into<String>, st: Style) -> Self {
        self.spans.push((s.into(), st));
        self
    }
    pub fn bg(mut self, c: Color) -> Self {
        self.bg = Some(c);
        self
    }
    pub fn width(&self) -> usize {
        self.spans.iter().map(|(s, _)| util::width(s)).sum()
    }
    pub fn draw(&self, buf: &mut Buffer, x: u16, y: u16, w: u16) {
        if let Some(bg) = self.bg {
            fill(buf, Rect::new(x, y, w, 1), bg);
        }
        let mut cx = x;
        let end = x + w;
        for (s, st) in &self.spans {
            if cx >= end {
                break;
            }
            let st = match self.bg {
                Some(bg) => st.bg(bg),
                None => *st,
            };
            cx = put(buf, cx, y, s, st, end - cx);
        }
    }
}

impl Default for Line<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// Rail rows written top to bottom, with a selection band.
pub struct Rail<'a> {
    pub buf: &'a mut Buffer,
    pub area: Rect,
    pub y: u16,
    pub t: Theme,
}

impl<'a> Rail<'a> {
    pub fn new(buf: &'a mut Buffer, area: Rect, t: Theme) -> Self {
        Rail {
            buf,
            y: area.y,
            area,
            t,
        }
    }

    pub fn room(&self) -> u16 {
        (self.area.y + self.area.height).saturating_sub(self.y)
    }

    pub fn section(&mut self, title: &str) {
        let t = self.t;
        self.line(Line::new().push(title.to_string(), bold(style(t.text2, t.rail))));
    }

    pub fn line(&mut self, line: Line) {
        if self.room() == 0 || self.area.width == 0 {
            return;
        }
        line.draw(
            self.buf,
            self.area.x + 1,
            self.y,
            self.area.width.saturating_sub(1),
        );
        self.y += 1;
    }

    pub fn blank(&mut self) {
        if self.room() > 0 {
            self.y += 1;
        }
    }

    /// Write a line at the rail's last row (footer).
    pub fn footer(&mut self, line: Line) {
        if self.area.height == 0 {
            return;
        }
        let y = self.area.y + self.area.height - 1;
        line.draw(
            self.buf,
            self.area.x + 1,
            y,
            self.area.width.saturating_sub(1),
        );
    }
}

/// Blit part of a canvas into `area`, starting at canvas cell `scroll`.
pub fn blit(
    buf: &mut Buffer,
    area: Rect,
    canvas: &Canvas,
    scroll: (usize, usize),
    t: &Theme,
    bg: Color,
) {
    for dy in 0..area.height as usize {
        let cy = scroll.1 + dy;
        if cy >= canvas.h {
            break;
        }
        let mut dx = 0usize;
        while dx < area.width as usize {
            let cx = scroll.0 + dx;
            if cx >= canvas.w {
                break;
            }
            let cell = canvas.get(cx, cy).copied().unwrap_or_default();
            let g = cell.glyph();
            if g == '\0' {
                dx += 1;
                continue;
            }
            let mut st = Style::default().fg(t.tone(cell.tone)).bg(match cell.band {
                Some(b) => t.band(b),
                None => bg,
            });
            if cell.bold {
                st = st.add_modifier(Modifier::BOLD);
            }
            let x = area.x + dx as u16;
            let y = area.y + dy as u16;
            let w = util::char_width(g).max(1);
            if dx + w > area.width as usize {
                break;
            }
            let mut tmp = [0u8; 4];
            buf.set_string(x, y, g.encode_utf8(&mut tmp), st);
            dx += w;
        }
    }
}

/// Scroll so that the rectangle (x, y, w, h) is visible in a viewport.
pub fn follow(
    scroll: &mut (usize, usize),
    view: (usize, usize),
    rect: (usize, usize, usize, usize),
    content: (usize, usize),
) {
    let (vw, vh) = view;
    let (x, y, w, h) = rect;
    if vw == 0 || vh == 0 {
        return;
    }
    // horizontal
    if x < scroll.0 {
        scroll.0 = x.saturating_sub(2);
    } else if x + w.min(vw) > scroll.0 + vw {
        scroll.0 = (x + w.min(vw)).saturating_sub(vw) + 2;
    }
    // vertical
    if y < scroll.1 {
        scroll.1 = y.saturating_sub(1);
    } else if y + h.min(vh) > scroll.1 + vh {
        scroll.1 = (y + h.min(vh)).saturating_sub(vh) + 1;
    }
    scroll.0 = scroll.0.min(content.0.saturating_sub(vw.min(content.0)));
    scroll.1 = scroll.1.min(content.1.saturating_sub(vh.min(content.1)));
}

// ---- header, hints, help ----------------------------------------------------

fn header(buf: &mut Buffer, r: Rect, app: &App) {
    let t = app.theme;
    let mut x = r.x + 1;
    for v in View::ALL {
        let name = v.name();
        let st = if v == app.view {
            bold(style(t.on_accent, t.accent))
        } else {
            style(t.text2, t.body)
        };
        x = put(
            buf,
            x,
            r.y,
            &format!(" {name} "),
            st,
            r.width.saturating_sub(x - r.x),
        );
        x += 1;
    }
    // Right side: agent, branch, scope.
    let mut ctx = Line::new();
    if app.view == View::History {
        if let Some(line) = history::header_summary(app) {
            ctx = ctx.push(line, style(t.muted, t.body));
        }
    } else {
        if let Some(agent) = &app.agent {
            if agent.pane_id.is_some() {
                let (dot, tone) = if agent.is_working() {
                    ("● ", t.add)
                } else {
                    ("○ ", t.muted)
                };
                ctx = ctx.push(dot, style(tone, t.body));
                ctx = ctx.push(
                    agent.label().unwrap_or_else(|| util::tilde(&app.root)),
                    style(t.text, t.body),
                );
            } else {
                ctx = ctx.push(util::tilde(&app.root), style(t.text, t.body));
            }
        } else {
            ctx = ctx.push(util::tilde(&app.root), style(t.text, t.body));
        }
        if let Some(repo) = &app.repo {
            let b = repo
                .branch
                .clone()
                .or_else(|| repo.head.as_ref().map(|h| h[..7.min(h.len())].to_string()))
                .unwrap_or_else(|| "no commits".into());
            ctx = ctx
                .push("   ", style(t.muted, t.body))
                .push(b, style(t.branch, t.body));
        }
        if !app.scope_info.header.is_empty() {
            ctx = ctx
                .push("   ", style(t.muted, t.body))
                .push(app.scope_info.header.clone(), style(t.muted, t.body));
        }
    }
    let w = ctx.width() as u16;
    let avail = (r.x + r.width).saturating_sub(x + 2);
    if w <= avail {
        ctx.draw(buf, r.x + r.width - w - 1, r.y, w);
    } else if avail > 8 {
        // keep the start (agent) visible
        ctx.draw(buf, x + 2, r.y, avail);
    }
}

fn view_hints(app: &App) -> Vec<(&'static str, String)> {
    match app.view {
        View::Map => {
            let mut h = vec![
                ("⏎", "zoom in".to_string()),
                ("⌫", "zoom out".into()),
                ("/", "search".into()),
            ];
            if let Some(f) = app.current_function() {
                if let Some(s) = app
                    .index
                    .symbols(&f.file)
                    .and_then(|fs| fs.symbols.get(f.idx))
                {
                    h.push(("2", format!("flow of {}", s.name)));
                }
            }
            if app.map.scene.as_ref().is_some_and(|s| s.pages > 1) {
                h.push(("] [", "page".into()));
            }
            h.push(("?", "keys".into()));
            h
        }
        View::Flow => vec![
            ("⏎", "open a call in Flow".to_string()),
            ("⌫", "back".into()),
            ("e", "edit at line".into()),
            ("1", "map".into()),
            ("3", "changes".into()),
            ("?", "keys".into()),
        ],
        View::Changes => vec![
            ("] [", "hunk".to_string()),
            ("space", "reviewed".into()),
            ("v", "select".into()),
            ("c", "comment".into()),
            ("P", "send".into()),
            ("s", "scope".into()),
            ("?", "keys".into()),
        ],
        View::History => {
            if app.history.show_diff {
                vec![
                    ("⌫", "commit".to_string()),
                    ("j k", "scroll".into()),
                    ("?", "keys".into()),
                ]
            } else {
                vec![
                    ("⏎", "diff".to_string()),
                    ("1", "map of this commit".into()),
                    ("?", "keys".into()),
                ]
            }
        }
    }
}

fn hints_line(buf: &mut Buffer, a: &Areas, app: &App) {
    let t = app.theme;
    // Rail hint.
    if a.rail_hints.width > 0 {
        let rail_hint = match (app.view, app.focus()) {
            (_, Focus::Rail) => "tab body",
            (View::Map, _) => "/ search  tab list",
            (View::Flow, _) => "tab functions",
            (View::Changes, _) => "tab files",
            (View::History, _) => "j k commit",
        };
        put(
            buf,
            a.rail_hints.x + 1,
            a.rail_hints.y,
            rail_hint,
            style(t.muted, t.rail),
            a.rail_hints.width.saturating_sub(1),
        );
    }
    let r = a.hints;
    if let Some(input) = &app.input {
        let (prompt, text, keys) = match &input.kind {
            InputKind::Search => (
                "/ ".to_string(),
                input.text.clone(),
                "   ↑↓ pick  ⏎ go  ⎋ cancel",
            ),
            InputKind::Comment {
                path, start, end, ..
            } => {
                let anchor = if end > start {
                    format!("{path}:{start}-{end}")
                } else {
                    format!("{path}:{start}")
                };
                (
                    format!("comment on {anchor} › "),
                    input.text.clone(),
                    "   ⏎ save  ⎋ cancel",
                )
            }
        };
        let line = Line::new()
            .push(prompt, style(t.text2, t.body))
            .push(text, style(t.text, t.body))
            .push("█", style(t.text, t.body))
            .push(keys, style(t.muted, t.body));
        line.draw(buf, r.x, r.y, r.width);
        return;
    }
    if let Some(msg) = &app.message {
        put(buf, r.x, r.y, msg, style(t.text2, t.body), r.width);
        return;
    }
    let mut line = Line::new();
    for (i, (k, v)) in view_hints(app).into_iter().enumerate() {
        if i > 0 {
            line = line.push("  ", style(t.muted, t.body));
        }
        line = line
            .push(k, style(t.text2, t.body))
            .push(format!(" {v}"), style(t.muted, t.body));
    }
    line.draw(buf, r.x, r.y, r.width);
}

pub const KEYS: &[(&str, &[(&str, &str)])] = &[
    (
        "views",
        &[
            ("1 2 3 4", "map, flow, changes, history"),
            ("tab", "move between rail and body"),
            ("/", "search a symbol or file"),
            ("e  y", "edit in $EDITOR at the line, copy path:line"),
            (
                "p  T",
                "pin as a split that follows the agent, open in a tab",
            ),
            ("?  q ⎋", "keys, close"),
        ],
    ),
    (
        "map",
        &[
            ("← → ↑ ↓", "move between boxes"),
            ("j k", "move through a box's rows"),
            ("⏎  ⌫", "zoom in, zoom out"),
            ("] [", "next or previous page"),
        ],
    ),
    (
        "flow",
        &[
            ("↑ ↓  ← →", "move between boxes"),
            ("⏎  ⌫", "open a call, go back"),
        ],
    ),
    (
        "changes",
        &[
            ("j k", "move by line"),
            ("] [  } {", "next or previous hunk, file"),
            ("space", "mark the hunk reviewed"),
            ("v  c  x", "select lines, comment, delete a comment"),
            ("P  S", "paste the draft into the agent's prompt, submit it"),
            ("s  a", "scope (turn, session, HEAD), next agent"),
        ],
    ),
    (
        "history",
        &[
            ("j k", "move between commits"),
            ("⏎  ⌫", "show the diff, back"),
            ("1", "map of this commit"),
        ],
    ),
];

fn help(buf: &mut Buffer, area: Rect, app: &App) {
    let t = app.theme;
    let rows: usize = KEYS.iter().map(|(_, k)| k.len() + 2).sum::<usize>() + 1;
    let w = 66u16.min(area.width.saturating_sub(4));
    let h = (rows as u16 + 2).min(area.height.saturating_sub(2));
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let r = Rect::new(x, y, w, h);
    fill(buf, r, t.rail);
    let mut c = Canvas::new(w as usize, h as usize);
    c.rect(
        0,
        0,
        w as usize,
        h as usize,
        crate::canvas::Tone::Rule,
        true,
    );
    c.text(2, 0, " keys ", crate::canvas::Tone::Text, true);
    blit(buf, r, &c, (0, 0), &t, t.rail);
    let mut yy = y + 1;
    for (section, keys) in KEYS {
        if yy >= y + h - 1 {
            break;
        }
        put(buf, x + 2, yy, section, bold(style(t.text2, t.rail)), w - 4);
        yy += 1;
        for (k, d) in keys.iter() {
            if yy >= y + h - 1 {
                break;
            }
            put(buf, x + 3, yy, k, style(t.text, t.rail), 12);
            put(
                buf,
                x + 17,
                yy,
                d,
                style(t.muted, t.rail),
                w.saturating_sub(19),
            );
            yy += 1;
        }
        yy += 1;
    }
}

impl App {
    pub fn focus(&self) -> Focus {
        match self.view {
            View::Map => self.map.focus,
            View::Flow => self.flow.focus,
            View::Changes => self.changes.focus,
            View::History => Focus::Body,
        }
    }
}

/// Render the whole app into plain text (tests, snapshots).
pub fn render_text(app: &mut App, w: u16, h: u16) -> String {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    render_buf(&mut buf, area, app);
    buffer_text(&buf)
}

pub fn buffer_text(buf: &Buffer) -> String {
    let area = buf.area;
    let mut out = String::new();
    for y in area.y..area.y + area.height {
        let mut line = String::new();
        let mut skip = 0;
        for x in area.x..area.x + area.width {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let sym = buf[(x, y)].symbol();
            line.push_str(sym);
            let w = util::width(sym);
            if w > 1 {
                skip = w - 1;
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}
