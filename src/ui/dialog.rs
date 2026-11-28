use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::{bold, fill, frame, put, style, Line};
use crate::app::App;
use crate::commit::Field;
use crate::status::Discard;
use crate::theme::Theme;
use crate::util;

const WIDTH: u16 = 74;
const BODY_ROWS: usize = 6;
const LABEL: u16 = 9;

fn centred(area: Rect, rows: u16) -> Rect {
    let w = WIDTH.min(area.width.saturating_sub(4));
    let h = (rows + 2).min(area.height.saturating_sub(2));
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

struct Dialog<'a> {
    buf: &'a mut Buffer,
    inner: Rect,
    y: u16,
}

impl<'a> Dialog<'a> {
    fn open(buf: &'a mut Buffer, area: Rect, title: &str, rows: u16, t: &Theme) -> Self {
        let r = centred(area, rows);
        fill(buf, r, t.rail);
        frame(buf, r, style(t.rule, t.rail));
        put(
            buf,
            r.x + 2,
            r.y,
            &format!(" {title} "),
            bold(style(t.text, t.rail)),
            r.width.saturating_sub(4),
        );
        let inner = Rect::new(
            r.x + 2,
            r.y + 1,
            r.width.saturating_sub(4),
            r.height.saturating_sub(2),
        );
        Dialog {
            buf,
            inner,
            y: inner.y,
        }
    }

    fn row(&mut self, line: Line) {
        if self.y < self.inner.y + self.inner.height {
            line.draw(self.buf, self.inner.x, self.y, self.inner.width);
        }
        self.y += 1;
    }

    fn gap(&mut self) {
        self.y += 1;
    }
}

fn tail(text: &str, width: usize) -> String {
    util::truncate_left(&util::sanitize_line(text), width)
}

fn field<'a>(label: &str, text: String, active: bool, t: &Theme) -> Line<'a> {
    let label_style = if active {
        bold(style(t.text, t.rail))
    } else {
        style(t.muted, t.rail)
    };
    let line = Line::new()
        .push(util::pad(label, LABEL as usize), label_style)
        .push(text, style(t.text, t.rail));
    if active {
        line.push("█", style(t.text, t.rail))
    } else {
        line
    }
}

pub fn commit_box(buf: &mut Buffer, area: Rect, app: &App) {
    let t = app.theme;
    let commit = &app.commit;
    let body: Vec<&str> = commit.body.split('\n').collect();
    let around_body = 7 + commit.problem.len();
    let fits = (area.height as usize).saturating_sub(around_body + 4);
    let body_rows = body.len().min(BODY_ROWS).min(fits).max(1);
    let rows = around_body + body_rows;
    let mut d = Dialog::open(buf, area, &app.commit_title(), rows as u16, &t);
    let room = d.inner.width.saturating_sub(LABEL + 1) as usize;
    let on_subject = commit.field == Field::Subject;
    d.gap();
    d.row(field(
        "subject",
        tail(&commit.subject, room),
        on_subject,
        &t,
    ));
    d.gap();
    let hidden = body.len() - body_rows;
    for (i, text) in body.iter().enumerate().skip(hidden) {
        let label = if i == hidden { "body" } else { "" };
        let last = i + 1 == body.len();
        d.row(field(label, tail(text, room), last && !on_subject, &t));
    }
    d.gap();
    let staged = util::plural(app.status.staged.len(), "file staged", "files staged");
    let mut state = Line::new().push(staged, style(t.text2, t.rail));
    if commit.busy {
        state = state.push(" · committing…", style(t.muted, t.rail));
    }
    d.row(state);
    for said in &commit.problem {
        d.row(Line::new().push(util::sanitize_line(said), style(t.del, t.rail)));
    }
    d.gap();
    let (enter, tab) = if on_subject {
        ("commit", "body")
    } else {
        ("new line", "subject")
    };
    let mut keys = Line::new();
    for (key, does) in [
        ("⏎", enter),
        ("tab", tab),
        ("⌃u", "clear"),
        ("⎋", "close, keeps the draft"),
    ] {
        keys = keys
            .push(key, style(t.text2, t.rail))
            .push(format!(" {does}   "), style(t.muted, t.rail));
    }
    d.row(keys);
}

fn wrapped(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split(' ') {
        match lines.last_mut() {
            Some(line) if util::width(line) + 1 + util::width(word) <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(util::truncate_left(word, width)),
        }
    }
    lines
}

pub fn discard_question(buf: &mut Buffer, area: Rect, asked: &Discard, t: &Theme) {
    let width = WIDTH.min(area.width.saturating_sub(4)).saturating_sub(4) as usize;
    let question = wrapped(&util::sanitize_line(&asked.question()), width.max(1));
    let mut d = Dialog::open(buf, area, "discard", question.len() as u16 + 4, t);
    d.gap();
    for line in question {
        d.row(Line::new().push(line, bold(style(t.text, t.rail))));
    }
    d.row(Line::new().push("this cannot be undone", style(t.del, t.rail)));
    d.gap();
    d.row(
        Line::new()
            .push("y", style(t.text2, t.rail))
            .push(format!(" {}", asked.verb()), style(t.muted, t.rail))
            .push(" · ", style(t.muted, t.rail))
            .push("any other key", style(t.text2, t.rail))
            .push(" keeps", style(t.muted, t.rail)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_wraps_between_words_and_keeps_the_end_of_a_long_path() {
        assert_eq!(
            wrapped("discard 12 changed lines in src/app.rs?", 40),
            ["discard 12 changed lines in src/app.rs?"]
        );
        assert_eq!(
            wrapped("discard 12 changed lines in src/app.rs?", 20),
            ["discard 12 changed", "lines in src/app.rs?"]
        );
        assert_eq!(
            wrapped(
                "delete the untracked file a/very/long/path/to/notes.md?",
                16
            ),
            ["delete the", "untracked file", "…th/to/notes.md?"]
        );
        assert!(wrapped("delete the untracked file notes.md?", 1)
            .iter()
            .all(|line| util::width(line) <= 1));
    }
}
