mod branches;
mod changes;
mod custom;
mod dialog;
mod files;
mod history;
mod status;
mod worktrees;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;

use crate::app::{App, Focus, InputKind, View};
use crate::theme::Theme;
use crate::util;

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
    } else if matches!(view, View::Branches | View::Worktrees) {
        ((w as u32 * 36 / 100) as u16).clamp(34, 56)
    } else if matches!(view, View::Files | View::Custom) {
        ((w as u32 * 32 / 100) as u16).clamp(30, 48)
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
            "codeMap: window too small",
            Style::default().fg(t.muted).bg(t.body),
            area.width,
        );
        return;
    }
    let page = app.page();
    let a = areas(area, page);
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
    match page {
        View::Files => files::render(buf, &a, app),
        View::Custom => custom::render(buf, &a, app),
        View::Status => status::render(buf, &a, app),
        View::Changes => changes::render(buf, &a, app),
        View::History => history::render(buf, &a, app),
        View::Branches => branches::render(buf, &a, app),
        View::Worktrees => worktrees::render(buf, &a, app),
    }
    hints_line(buf, &a, app);
    if app.commit.open {
        dialog::commit_box(buf, area, app);
    }
    if let Some(asked) = &app.confirm {
        dialog::question(buf, area, asked, &t);
    }
    if app.show_help {
        help(buf, area, app);
    }
}

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

fn header(buf: &mut Buffer, r: Rect, app: &App) {
    let t = app.theme;
    let mut x = r.x + 1;
    let tabs: Vec<(&str, bool)> = if app.view == View::Custom {
        crate::custom::Scope::ALL
            .iter()
            .map(|s| (s.label(), *s == app.custom.scope))
            .collect()
    } else {
        app.view
            .tabs()
            .iter()
            .map(|v| (v.label(), *v == app.view))
            .collect()
    };
    for (name, current) in tabs {
        let st = if current {
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
    let mut ctx = Line::new();
    if let Some(line) = history::file_summary(app) {
        ctx = ctx.push(line, style(t.muted, t.body));
    } else if app.view == View::History {
        if let Some(line) = history::header_summary(app) {
            ctx = ctx.push(line, style(t.muted, t.body));
        }
    } else if app.view == View::Custom {
        let folder = app
            .root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| util::tilde(&app.root));
        ctx = ctx.push(folder, style(t.text, t.body));
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
        let turn_shown = !matches!(app.view, View::Status | View::Branches | View::Worktrees);
        if turn_shown && !app.scope_info.header.is_empty() {
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
        ctx.draw(buf, x + 2, r.y, avail);
    }
}

fn view_hints(app: &App) -> Vec<(&'static str, String)> {
    match app.page() {
        View::Custom => {
            let mut hints = vec![
                ("1-4", "scope".to_string()),
                ("⏎", "open".into()),
                ("E", "VS Code".into()),
                ("b", "browser".into()),
                ("y", "copy path".into()),
            ];
            if app.file_for_history().is_ok() {
                hints.push(("L", "history".into()));
            }
            hints.push(("?", "keys".into()));
            hints
        }
        View::Files => {
            let mut hints = vec![
                ("⏎", "open".to_string()),
                ("e", "editor".into()),
                ("E", "VS Code".into()),
                ("b", "browser".into()),
                ("B", "GitHub".into()),
            ];
            if app.file_for_history().is_ok() {
                hints.push(("L", "history".into()));
            }
            hints.extend([("/", "find".to_string()), ("?", "keys".into())]);
            hints
        }
        View::Status => status::hints(&app.status),
        View::Changes => {
            let mut hints = vec![
                ("] [", "hunk".to_string()),
                ("space", "reviewed".into()),
                ("v", "select".into()),
                ("c", "comment".into()),
                ("P", "send".into()),
                ("s", "scope".into()),
            ];
            if app.file_for_history().is_ok() {
                hints.push(("L", "history".into()));
            }
            hints.push(("?", "keys".into()));
            hints
        }
        View::History => history::hints(app),
        View::Branches => branches::hints(app),
        View::Worktrees => worktrees::hints(app),
    }
}

fn hints_line(buf: &mut Buffer, a: &Areas, app: &App) {
    let t = app.theme;
    if a.rail_hints.width > 0 {
        let rail_hint = match (app.page(), app.focus()) {
            (View::Custom, _) => "j k move  h l scope",
            (View::Files, Focus::Rail) => "tab preview",
            (View::Files, Focus::Body) => "tab tree",
            (View::Status, Focus::Rail) => "tab diff",
            (View::Branches, _) => "j k branch",
            (View::Worktrees, _) => "j k worktree",
            (_, Focus::Rail) => "tab body",
            (View::Status | View::Changes, _) => "tab files",
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
        let (prompt, keys) = match &input.kind {
            InputKind::Filter => ("/ ".to_string(), "   ↑↓ pick  ⏎ go  ⎋ cancel"),
            InputKind::Comment {
                path, start, end, ..
            } => {
                let anchor = if end > start {
                    format!("{path}:{start}-{end}")
                } else {
                    format!("{path}:{start}")
                };
                (format!("comment on {anchor} › "), "   ⏎ save  ⎋ cancel")
            }
            InputKind::BranchName => (
                format!("new branch from {} › ", app.head_label()),
                "   ⏎ create and switch  ⎋ cancel",
            ),
        };
        let text = input.text.clone();
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
    if app.commit.open || app.confirm.is_some() {
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
        "every popup",
        &[
            ("tab", "move between the list and the body"),
            ("e  E", "open in the editor beside the agent, in VS Code"),
            ("b  B", "open the file in the browser, on GitHub"),
            ("y", "copy path:line"),
            ("L", "history of a file in Files, Status, Review"),
            ("s", "what counts as changed: turn, session, HEAD"),
            (
                "p  T",
                "pin as a split that follows the agent, open in a tab",
            ),
            ("?  q ⎋", "keys, close"),
        ],
    ),
    (
        "files",
        &[
            ("j k", "move through the tree"),
            ("l h", "open or close a folder, step into the preview"),
            ("⏎", "open a folder, or a file in the editor"),
            ("/", "find a file by name"),
        ],
    ),
    (
        "customisations",
        &[
            ("1 2 3 4", "project, global, tools, other agents"),
            ("j k  h l", "move through the list, change scope"),
            ("⏎", "open the file in the editor"),
            ("L", "history of a file inside the repository"),
        ],
    ),
    (
        "status",
        &[
            ("1 2 3 4 5", "status, review, history, branches, worktrees"),
            ("j k", "move through the staged and not staged files"),
            ("tab  ⎋", "read the diff, go back to the files"),
            ("] [", "next or previous hunk of the diff"),
            ("space", "stage or unstage the file, in the diff the hunk"),
            ("c  x", "commit what is staged, discard after asking"),
        ],
    ),
    (
        "review",
        &[
            ("j k", "move by line"),
            ("] [  } {", "next or previous hunk, file"),
            ("space", "mark the hunk reviewed"),
            ("v  c  x", "select lines, comment, delete a comment"),
            ("P  S", "paste the draft into the agent's prompt, submit it"),
            ("a", "next agent"),
        ],
    ),
    (
        "history",
        &[
            ("j k", "move between commits"),
            ("⏎  ⌫", "show the diff, back"),
            ("B  y", "open the commit on GitHub, copy its full sha"),
            ("⎋", "from the history of a file, back to where L was"),
            ("e  E  b", "in the history of a file, open the file"),
        ],
    ),
    (
        "branches",
        &[
            ("⏎  n", "switch to the branch, name a new one from HEAD"),
            ("m  D", "merge into the current branch, delete: both ask"),
            ("B  y", "open the pull request or branch page, copy name"),
        ],
    ),
    (
        "worktrees",
        &[("⏎  y  r", "show in every tab, copy its path, read again")],
    ),
];

fn frame(buf: &mut Buffer, r: Rect, st: Style) {
    if r.width < 2 || r.height < 2 {
        return;
    }
    let (right, bottom) = (r.x + r.width - 1, r.y + r.height - 1);
    let side = "─".repeat(r.width as usize - 2);
    put(buf, r.x, r.y, &format!("╭{side}╮"), st, r.width);
    put(buf, r.x, bottom, &format!("╰{side}╯"), st, r.width);
    for y in r.y + 1..bottom {
        put(buf, r.x, y, "│", st, 1);
        put(buf, right, y, "│", st, 1);
    }
}

enum HelpRow {
    Section(&'static str),
    Key(&'static str, &'static str),
    Gap,
}

fn help_sections(
    app: &App,
) -> Vec<&'static (&'static str, &'static [(&'static str, &'static str)])> {
    let mut names: Vec<&str> = app.view.tabs().iter().map(|v| v.label()).collect();
    if app.file_history.is_some() && !names.contains(&View::History.label()) {
        names.push(View::History.label());
    }
    KEYS.iter()
        .enumerate()
        .filter(|(i, (name, _))| *i == 0 || names.contains(name))
        .map(|(_, section)| section)
        .collect()
}

fn help_rows(app: &App) -> Vec<HelpRow> {
    let mut rows = Vec::new();
    for (name, keys) in help_sections(app) {
        rows.push(HelpRow::Section(name));
        rows.extend(keys.iter().map(|(k, d)| HelpRow::Key(k, d)));
        rows.push(HelpRow::Gap);
    }
    rows
}

fn help(buf: &mut Buffer, area: Rect, app: &mut App) {
    let t = app.theme;
    let rows = help_rows(app);
    let w = 66u16.min(area.width.saturating_sub(4));
    let h = (rows.len() as u16 + 2).min(area.height.saturating_sub(2));
    if w < 4 || h < 3 {
        return;
    }
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let r = Rect::new(x, y, w, h);
    let room = (h - 2) as usize;
    let last_top = rows.len().saturating_sub(room);
    app.help_scroll = app.help_scroll.min(last_top);
    let top = app.help_scroll;
    fill(buf, r, t.rail);
    frame(buf, r, style(t.rule, t.rail));
    put(buf, x + 2, y, " keys ", bold(style(t.text, t.rail)), w - 4);
    for (yy, row) in (y + 1..).zip(rows.iter().skip(top).take(room)) {
        match row {
            HelpRow::Section(name) => {
                put(buf, x + 2, yy, name, bold(style(t.text2, t.rail)), w - 4);
            }
            HelpRow::Key(k, d) => {
                put(buf, x + 3, yy, k, style(t.text, t.rail), 12);
                put(
                    buf,
                    x + 17,
                    yy,
                    d,
                    style(t.muted, t.rail),
                    w.saturating_sub(19),
                );
            }
            HelpRow::Gap => {}
        }
    }
    let more = |text: &str| format!(" {text} ");
    if top > 0 {
        let above = more("↑ more");
        let at = x + w.saturating_sub(util::width(&above) as u16 + 2);
        put(buf, at, y, &above, style(t.text2, t.rail), w - 4);
    }
    if top < last_top {
        let below = more("↓ more · j k scroll");
        let at = x + w.saturating_sub(util::width(&below) as u16 + 2);
        put(buf, at, y + h - 1, &below, style(t.text2, t.rail), w - 4);
    }
}

impl App {
    pub fn focus(&self) -> Focus {
        match self.page() {
            View::Files => self.files.focus,
            View::Custom => Focus::Rail,
            View::Status => self.status.focus,
            View::Changes => self.changes.focus,
            View::History => Focus::Body,
            View::Branches | View::Worktrees => Focus::Rail,
        }
    }
}

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
