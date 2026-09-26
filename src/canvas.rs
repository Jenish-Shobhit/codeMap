//! A character grid that draws boxes and lines the way a terminal would.
//!
//! Lines are stored as direction bits per cell (up, down, left, right) and
//! turned into box-drawing glyphs at the end, so crossings and junctions
//! (`┬ ├ ┼ ┘`) come out right no matter in which order things were drawn.
//! Colours are semantic tones that the renderer maps to herdr's theme.

use crate::util;

pub const UP: u8 = 1;
pub const DOWN: u8 = 2;
pub const LEFT: u8 = 4;
pub const RIGHT: u8 = 8;

/// Semantic colour of a cell; the theme decides the actual colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Tone {
    /// Box borders and edges (herdr overlay0).
    #[default]
    Rule,
    /// Primary text.
    Text,
    /// Secondary text (subtext0).
    Dim,
    /// Hints, counts, labels (overlay1).
    Muted,
    /// Added (green).
    Add,
    /// Changed (yellow).
    Mod,
    /// Deleted (red).
    Del,
    /// Branch and path names (mauve).
    Branch,
    /// Blue: renamed, paths in comments, highlighted edges.
    Blue,
    /// The view accent (used sparingly).
    Accent,
    /// Peach: warnings, conflicts, raises.
    Warn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub mask: u8,
    pub round: bool,
    pub arrow: Option<char>,
    pub tone: Tone,
    pub bold: bool,
    /// Background band (selection or active row), by tone of intent.
    pub band: Option<Band>,
    /// The second half of a wide character: draw nothing here.
    pub cont: bool,
    /// Written as text: its character wins over lines, even a space.
    pub txt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Band {
    /// The active row / selected node interior.
    Active,
    /// A selection (v) or search hit.
    Selection,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            ch: ' ',
            mask: 0,
            round: false,
            arrow: None,
            tone: Tone::Rule,
            bold: false,
            band: None,
            cont: false,
            txt: false,
        }
    }
}

impl Cell {
    pub fn glyph(&self) -> char {
        if self.cont {
            return '\0';
        }
        if let Some(a) = self.arrow {
            return a;
        }
        if self.txt || self.ch != ' ' {
            return self.ch;
        }
        line_glyph(self.mask, self.round)
    }

    pub fn is_blank(&self) -> bool {
        self.ch == ' ' && self.mask == 0 && self.arrow.is_none() && !self.cont && !self.txt
    }
}

pub fn line_glyph(mask: u8, round: bool) -> char {
    match mask {
        0 => ' ',
        m if m == LEFT | RIGHT || m == LEFT || m == RIGHT => '─',
        m if m == UP | DOWN || m == UP || m == DOWN => '│',
        m if m == RIGHT | DOWN => {
            if round {
                '╭'
            } else {
                '┌'
            }
        }
        m if m == LEFT | DOWN => {
            if round {
                '╮'
            } else {
                '┐'
            }
        }
        m if m == UP | RIGHT => {
            if round {
                '╰'
            } else {
                '└'
            }
        }
        m if m == UP | LEFT => {
            if round {
                '╯'
            } else {
                '┘'
            }
        }
        m if m == UP | DOWN | RIGHT => '├',
        m if m == UP | DOWN | LEFT => '┤',
        m if m == LEFT | RIGHT | DOWN => '┬',
        m if m == LEFT | RIGHT | UP => '┴',
        _ => '┼',
    }
}

#[derive(Debug, Clone)]
pub struct Canvas {
    pub w: usize,
    pub h: usize,
    cells: Vec<Cell>,
}

impl Canvas {
    pub fn new(w: usize, h: usize) -> Self {
        Canvas {
            w,
            h,
            cells: vec![Cell::default(); w * h],
        }
    }

    pub fn get(&self, x: usize, y: usize) -> Option<&Cell> {
        if x < self.w && y < self.h {
            self.cells.get(y * self.w + x)
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, x: usize, y: usize) -> Option<&mut Cell> {
        if x < self.w && y < self.h {
            self.cells.get_mut(y * self.w + x)
        } else {
            None
        }
    }

    /// Grow to at least w×h, keeping content.
    pub fn ensure(&mut self, w: usize, h: usize) {
        if w <= self.w && h <= self.h {
            return;
        }
        let nw = w.max(self.w);
        let nh = h.max(self.h);
        let mut cells = vec![Cell::default(); nw * nh];
        for y in 0..self.h {
            for x in 0..self.w {
                cells[y * nw + x] = self.cells[y * self.w + x];
            }
        }
        self.w = nw;
        self.h = nh;
        self.cells = cells;
    }

    pub fn bits(&mut self, x: usize, y: usize, bits: u8, tone: Tone) {
        if let Some(c) = self.get_mut(x, y) {
            c.mask |= bits;
            c.tone = tone;
        }
    }

    /// Horizontal line from x1 to x2 (either order), inclusive.
    pub fn hline(&mut self, x1: usize, x2: usize, y: usize, tone: Tone) {
        let (a, b) = if x1 <= x2 { (x1, x2) } else { (x2, x1) };
        if a == b {
            return;
        }
        for x in a..=b {
            let mut bits = 0;
            if x > a {
                bits |= LEFT;
            }
            if x < b {
                bits |= RIGHT;
            }
            self.bits(x, y, bits, tone);
        }
    }

    /// Vertical line from y1 to y2 (either order), inclusive.
    pub fn vline(&mut self, x: usize, y1: usize, y2: usize, tone: Tone) {
        let (a, b) = if y1 <= y2 { (y1, y2) } else { (y2, y1) };
        if a == b {
            return;
        }
        for y in a..=b {
            let mut bits = 0;
            if y > a {
                bits |= UP;
            }
            if y < b {
                bits |= DOWN;
            }
            self.bits(x, y, bits, tone);
        }
    }

    /// An orthogonal polyline through `points`.
    pub fn polyline(&mut self, points: &[(usize, usize)], tone: Tone) {
        for pair in points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if a.1 == b.1 {
                self.hline(a.0, b.0, a.1, tone);
            } else if a.0 == b.0 {
                self.vline(a.0, a.1, b.1, tone);
            } else {
                // Not orthogonal: go vertical first, then horizontal.
                self.vline(a.0, a.1, b.1, tone);
                self.hline(a.0, b.0, b.1, tone);
            }
        }
        if points.len() == 1 {
            let (x, y) = points[0];
            self.bits(x, y, UP | DOWN, tone);
        }
    }

    /// A box with its top-left corner at (x, y). Clears its interior.
    pub fn rect(&mut self, x: usize, y: usize, w: usize, h: usize, tone: Tone, round: bool) {
        if w < 2 || h < 2 {
            return;
        }
        let (r, b) = (x + w - 1, y + h - 1);
        for yy in y + 1..b {
            for xx in x + 1..r {
                if let Some(c) = self.get_mut(xx, yy) {
                    *c = Cell::default();
                }
            }
        }
        self.hline(x, r, y, tone);
        self.hline(x, r, b, tone);
        self.vline(x, y, b, tone);
        self.vline(r, y, b, tone);
        for (cx, cy) in [(x, y), (r, y), (x, b), (r, b)] {
            if let Some(c) = self.get_mut(cx, cy) {
                c.round = round;
            }
        }
    }

    /// Write text; returns the x after the last cell written.
    pub fn text(&mut self, x: usize, y: usize, s: &str, tone: Tone, bold: bool) -> usize {
        let mut cx = x;
        for ch in s.chars() {
            let w = util::char_width(ch);
            if w == 0 {
                continue;
            }
            if cx + w > self.w || y >= self.h {
                break;
            }
            if let Some(c) = self.get_mut(cx, y) {
                c.ch = ch;
                c.tone = tone;
                c.bold = bold;
                c.arrow = None;
                c.cont = false;
                c.txt = true;
            }
            if w == 2 {
                if let Some(c) = self.get_mut(cx + 1, y) {
                    c.ch = ' ';
                    c.cont = true;
                    c.mask = 0;
                }
            }
            cx += w;
        }
        cx
    }

    /// Text clipped to `max` cells (with …).
    pub fn text_max(
        &mut self,
        x: usize,
        y: usize,
        s: &str,
        max: usize,
        tone: Tone,
        bold: bool,
    ) -> usize {
        let t = util::truncate(s, max);
        self.text(x, y, &t, tone, bold)
    }

    pub fn arrow(&mut self, x: usize, y: usize, ch: char, tone: Tone) {
        if let Some(c) = self.get_mut(x, y) {
            c.arrow = Some(ch);
            c.tone = tone;
        }
    }

    pub fn band(&mut self, x1: usize, x2: usize, y: usize, band: Band) {
        for x in x1..=x2 {
            if let Some(c) = self.get_mut(x, y) {
                c.band = Some(band);
            }
        }
    }

    pub fn set_tone(&mut self, x: usize, y: usize, tone: Tone) {
        if let Some(c) = self.get_mut(x, y) {
            c.tone = tone;
        }
    }

    /// True when every cell in the rectangle is blank.
    pub fn is_free(&self, x: usize, y: usize, w: usize, h: usize) -> bool {
        for yy in y..y + h {
            for xx in x..x + w {
                match self.get(xx, yy) {
                    Some(c) if c.is_blank() => {}
                    _ => return false,
                }
            }
        }
        true
    }

    pub fn row_text(&self, y: usize) -> String {
        let mut s = String::new();
        for x in 0..self.w {
            let g = self.cells[y * self.w + x].glyph();
            if g != '\0' {
                s.push(g);
            }
        }
        s.trim_end().to_string()
    }

    /// Plain text rendering (for tests and snapshots).
    pub fn to_text(&self) -> String {
        let mut lines: Vec<String> = (0..self.h).map(|y| self.row_text(y)).collect();
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junctions_merge() {
        let mut c = Canvas::new(12, 6);
        c.rect(0, 0, 8, 3, Tone::Rule, false);
        // an edge leaving the bottom border
        c.vline(3, 2, 4, Tone::Rule);
        c.hline(3, 9, 4, Tone::Rule);
        c.vline(9, 4, 5, Tone::Rule);
        c.arrow(9, 5, '▼', Tone::Rule);
        c.text(2, 1, "main", Tone::Text, false);
        let t = c.to_text();
        assert_eq!(
            t,
            "┌──────┐\n│ main │\n└──┬───┘\n   │\n   └─────┐\n         ▼"
        );
    }

    #[test]
    fn crossings_and_round_corners() {
        let mut c = Canvas::new(5, 5);
        c.hline(0, 4, 2, Tone::Rule);
        c.vline(2, 0, 4, Tone::Rule);
        assert_eq!(c.row_text(2), "──┼──");
        let mut c = Canvas::new(4, 3);
        c.rect(0, 0, 4, 3, Tone::Rule, true);
        assert_eq!(c.to_text(), "╭──╮\n│  │\n╰──╯");
    }

    #[test]
    fn wide_text_takes_two_cells() {
        let mut c = Canvas::new(6, 1);
        let end = c.text(0, 0, "日本", Tone::Text, false);
        assert_eq!(end, 4);
        assert_eq!(c.row_text(0), "日本");
    }
}
