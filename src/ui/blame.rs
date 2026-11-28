use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::{bold, put, style, Areas, Line};
use crate::app::App;
use crate::blame::BlamePage;
use crate::branches::told;
use crate::files::human_size;
use crate::git::blame::{BlameLine, BlameRead, Blamed, BLAME_BYTES};
use crate::theme::Theme;
use crate::util;

const ID_WIDTH: usize = 7;
const DATE_WIDTH: usize = 11;
const NOT_COMMITTED: &str = "not committed";

pub fn summary(app: &App) -> Option<String> {
    let page = app.blame.as_ref()?;
    let mut line = format!("blame {}", util::sanitize_line(&page.path));
    if let Some(blamed) = page.blamed() {
        let count = util::plural(blamed.lines.len(), "line", "lines");
        line.push_str(&format!(" · {count}"));
    }
    Some(line)
}

pub(super) fn hints(app: &App) -> Vec<(&'static str, String)> {
    let mut hints = Vec::new();
    let Some(page) = app.blame.as_ref() else {
        hints.push(("?", "keys".to_string()));
        return hints;
    };
    let committed = page.on_a_committed_line();
    let in_the_diff = page.diff.is_some();
    if in_the_diff {
        hints.push(("⌫", "blame".to_string()));
        hints.push(("j k", "scroll".into()));
    } else {
        hints.push(("⎋", "back".to_string()));
        if page.line_count() > 0 {
            hints.push(("j k", "line".into()));
        }
        if committed {
            hints.push(("⏎", "diff".into()));
        }
    }
    if committed {
        hints.push(("B", "GitHub".into()));
        hints.push(("y", "copy sha".into()));
    }
    let deleted = matches!(page.read, Some(Ok(BlameRead::Deleted)));
    if !deleted {
        hints.push(("e", "editor".into()));
    }
    if !in_the_diff {
        hints.push(("L", "history".into()));
    }
    hints.push(("?", "keys".into()));
    hints
}

pub fn render(buf: &mut Buffer, a: &Areas, app: &mut App) {
    let t = app.theme;
    let body = a.body;
    let Some(page) = app.blame.as_mut() else {
        return;
    };
    if body.width == 0 || body.height == 0 {
        return;
    }
    let name = util::sanitize_line(&page.path);
    match &page.read {
        None => say(
            buf,
            body,
            &[("reading the blame…".into(), style(t.muted, t.body))],
        ),
        Some(Err(e)) => {
            let said = format!("could not blame {name} · {}", told(e));
            say(buf, body, &[(said, style(t.del, t.body))]);
        }
        Some(Ok(BlameRead::Lines(_))) if page.diff.is_some() => diff(buf, body, page, &t),
        Some(Ok(BlameRead::Lines(_))) => lines(buf, body, page, &t),
        Some(Ok(refused)) => say(buf, body, &refusal(&name, refused, &t)),
    }
}

fn say(buf: &mut Buffer, body: Rect, lines: &[(String, Style)]) {
    for (y, (text, look)) in (body.y..body.y + body.height).zip(lines) {
        put(buf, body.x, y, text, *look, body.width);
    }
}

fn refusal(name: &str, read: &BlameRead, t: &Theme) -> Vec<(String, Style)> {
    let (first, second) = match read {
        BlameRead::NotCommitted => (
            format!("{name} is not committed yet · nothing to blame"),
            "a new or untracked file has a blame once it is committed".to_string(),
        ),
        BlameRead::NotRegular => (
            format!("{name} is not a regular file · nothing to blame"),
            "blame reads a text file line by line".to_string(),
        ),
        BlameRead::Deleted => (
            format!("{name} is deleted from the working tree · nothing to blame"),
            "L lists the commits that touched it".to_string(),
        ),
        BlameRead::Binary => (
            format!("{name} is a binary file · nothing to blame"),
            "blame reads a text file line by line".to_string(),
        ),
        BlameRead::TooLarge(size) => (
            format!("{name} is {} · too large to blame here", human_size(*size)),
            format!("blame reads files up to {}", human_size(BLAME_BYTES)),
        ),
        BlameRead::Empty | BlameRead::Lines(_) => {
            (format!("{name} is empty · nothing to blame"), String::new())
        }
    };
    vec![
        (first, style(t.text, t.body)),
        (second, style(t.muted, t.body)),
    ]
}

struct Columns {
    id: bool,
    author: usize,
    date: bool,
    digits: usize,
    room: usize,
}

impl Columns {
    fn fit(width: u16, author_width: usize, count: usize) -> Columns {
        let digits = count.to_string().len().max(3);
        let author = author_width.clamp(6, 16);
        let width = width as usize;
        let text_room = |meta: usize| width.saturating_sub(meta + digits + 3);
        let full = ID_WIDTH + 1 + author + 1 + DATE_WIDTH + 1;
        let (id, author, date) = if text_room(full) >= 40 {
            (true, author, true)
        } else if text_room(ID_WIDTH + 1 + DATE_WIDTH + 1) >= 30 {
            (true, 0, true)
        } else if text_room(ID_WIDTH + 1) >= 16 {
            (true, 0, false)
        } else {
            (false, 0, false)
        };
        let mut columns = Columns {
            id,
            author,
            date,
            digits,
            room: 0,
        };
        columns.room = text_room(columns.meta_width());
        columns
    }

    fn meta_width(&self) -> usize {
        let mut width = 0;
        if self.id {
            width += ID_WIDTH + 1;
        }
        if self.author > 0 {
            width += self.author + 1;
        }
        if self.date {
            width += DATE_WIDTH + 1;
        }
        width
    }
}

fn keep_in_view(page: &mut BlamePage, rows: usize) {
    let count = page.line_count();
    page.sel = page.sel.min(count.saturating_sub(1));
    if rows == 0 {
        return;
    }
    if page.centre {
        page.scroll = page.sel.saturating_sub(rows / 2);
        page.centre = false;
    }
    page.scroll = page.scroll.min(count.saturating_sub(rows));
    if page.sel < page.scroll {
        page.scroll = page.sel;
    } else if page.sel >= page.scroll + rows {
        page.scroll = page.sel + 1 - rows;
    }
}

fn lines(buf: &mut Buffer, body: Rect, page: &mut BlamePage, t: &Theme) {
    let rows = body.height.saturating_sub(2) as usize;
    keep_in_view(page, rows);
    let page = &*page;
    let Some(blamed) = page.blamed() else {
        return;
    };
    detail(buf, body, page, blamed, t);
    let columns = Columns::fit(body.width, page.author_width, blamed.lines.len());
    let shown = blamed.lines.iter().enumerate().skip(page.scroll).take(rows);
    for (y, (i, line)) in (body.y + 2..).zip(shown) {
        let starts_a_run = i == page.scroll || blamed.lines[i - 1].commit != line.commit;
        let mark = Mark {
            starts_a_run,
            selected: i == page.sel,
        };
        row(blamed, line, &columns, mark, t).draw(buf, body.x, y, body.width);
    }
}

fn detail(buf: &mut Buffer, body: Rect, page: &BlamePage, blamed: &Blamed, t: &Theme) {
    let Some(line) = page.selected_line() else {
        return;
    };
    let strip = match blamed.commit_of(line) {
        Some(c) => Line::new()
            .push(format!("{} ", c.short()), style(t.muted, t.body))
            .push(util::sanitize_line(&c.summary), bold(style(t.text, t.body)))
            .push(
                format!(
                    "   {} · {}",
                    util::sanitize_line(&c.author),
                    util::date_time(c.time)
                ),
                style(t.muted, t.body),
            ),
        None => Line::new()
            .push("not committed yet", bold(style(t.text2, t.body)))
            .push(
                format!(" · line {} changed since the last commit", line.number),
                style(t.muted, t.body),
            ),
    };
    strip.draw(buf, body.x, body.y, body.width);
}

#[derive(Clone, Copy)]
struct Mark {
    starts_a_run: bool,
    selected: bool,
}

fn visible(text: &str, room: usize) -> String {
    text.chars().take(room.saturating_mul(4)).collect()
}

fn row(
    blamed: &Blamed,
    line: &BlameLine,
    columns: &Columns,
    mark: Mark,
    t: &Theme,
) -> Line<'static> {
    let bg = if mark.selected { t.active_row } else { t.body };
    let mut out = Line::new().bg(bg);
    let meta = columns.meta_width();
    if meta > 0 && !mark.starts_a_run {
        out = out.push(" ".repeat(meta), style(t.muted, bg));
    } else if meta > 0 {
        out = match blamed.commit_of(line) {
            Some(c) => {
                let mut by =
                    Line::new().push(util::pad(c.short(), ID_WIDTH + 1), style(t.accent, bg));
                if columns.author > 0 {
                    let author = util::sanitize_line(&c.author);
                    by = by.push(util::pad(&author, columns.author + 1), style(t.text2, bg));
                }
                if columns.date {
                    by = by.push(
                        util::pad(&util::date(c.time), DATE_WIDTH + 1),
                        style(t.muted, bg),
                    );
                }
                out.spans.extend(by.spans);
                out
            }
            None => out.push(util::pad(NOT_COMMITTED, meta), style(t.muted, bg)),
        };
    }
    let number = format!("{:>width$}", line.number, width = columns.digits);
    let (number_look, pointer) = if mark.selected {
        (bold(style(t.text, bg)), " › ")
    } else {
        (style(t.muted, bg), "   ")
    };
    let text_look = match (line.commit, mark.selected) {
        (_, true) => style(t.text, bg),
        (None, false) => style(t.text2, bg),
        (Some(_), false) => style(t.text, bg),
    };
    out.push(number, number_look)
        .push(pointer, style(t.accent, bg))
        .push(
            util::sanitize_line(&visible(&line.text, columns.room)),
            text_look,
        )
}

fn diff(buf: &mut Buffer, body: Rect, page: &mut BlamePage, t: &Theme) {
    let Some(blamed) = page.blamed() else {
        return;
    };
    let Some(sha) = page.diff.as_ref().map(|d| d.sha.clone()) else {
        return;
    };
    let commit = blamed.commits.iter().find(|c| c.sha == sha);
    let (short, subject) = commit
        .map(|c| (c.short().to_string(), util::sanitize_line(&c.summary)))
        .unwrap_or_default();
    Line::new()
        .push(short, style(t.muted, t.body))
        .push(format!(" {subject}"), bold(style(t.text, t.body)))
        .push("   ⌫ back", style(t.muted, t.body))
        .draw(buf, body.x, body.y, body.width);
    let Some(diff) = page.diff.as_mut() else {
        return;
    };
    let area = Rect::new(
        body.x,
        body.y.saturating_add(2),
        body.width,
        body.height.saturating_sub(2),
    );
    match &diff.files {
        Some(files) => {
            let total = super::changes::render_diff_list(buf, area, files, diff.scroll, t);
            diff.scroll = diff.scroll.min(total.saturating_sub(1));
        }
        None => say(
            buf,
            area,
            &[("reading the diff…".into(), style(t.muted, t.body))],
        ),
    }
}
