//! A terminal grid rendered to SVG in a macOS-style window.
//!
//! Text is placed glyph by glyph on the cell grid, so columns line up in any
//! monospace font. Box-drawing lines, arrowheads and circles are drawn as
//! vectors: they join cleanly between rows whatever the line height.

use std::collections::BTreeMap;
use std::fmt::Write as _;

pub type Rgb = (u8, u8, u8);

/// What a run of text shares: colour, bold, italic, dim.
type TextStyle = (Rgb, bool, bool, bool);
/// Per-glyph x positions and the escaped glyphs of one run.
type Run = (Vec<String>, String);

/// One terminal cell. `ch` is empty for the trailing half of a wide glyph.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub ch: String,
    pub fg: Rgb,
    pub bg: Rgb,
    pub bold: bool,
    pub italic: bool,
    pub dim: bool,
    pub underline: bool,
}

/// A screen of cells, row-major.
pub struct Grid {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<Cell>,
}

impl Grid {
    pub fn cell(&self, x: usize, y: usize) -> &Cell {
        &self.cells[y * self.cols + x]
    }
}

/// The house style shared by codeMap's and paneMorph's screenshots.
pub struct Style {
    pub font_size: f32,
    pub cell_w: f32,
    pub cell_h: f32,
    pub background: Rgb,
    pub title_bar: Rgb,
    pub border: &'static str,
    pub title_fg: Rgb,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            font_size: 14.0,
            // JetBrains Mono, SF Mono and Menlo advance 0.6em.
            cell_w: 8.4,
            // line-height 1.3
            cell_h: 18.2,
            background: (0x28, 0x2a, 0x36),
            title_bar: (0x21, 0x22, 0x2c),
            border: "rgba(255,255,255,0.10)",
            title_fg: (0x8b, 0x8f, 0xa8),
        }
    }
}

const FONT: &str = "JetBrains Mono, SFMono-Regular, Menlo, Consolas, monospace";
const PAD_X: f32 = 16.0;
const TITLE_H: f32 = 38.0;
const PAD_TOP: f32 = 10.0;
const PAD_BOTTOM: f32 = 14.0;

fn hex((r, g, b): Rgb) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

fn num(v: f32) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

/// Arms of a light box-drawing character: (up, down, left, right, rounded).
fn box_arms(c: char) -> Option<(bool, bool, bool, bool, bool)> {
    Some(match c {
        '─' => (false, false, true, true, false),
        '│' => (true, true, false, false, false),
        '┌' => (false, true, false, true, false),
        '┐' => (false, true, true, false, false),
        '└' => (true, false, false, true, false),
        '┘' => (true, false, true, false, false),
        '╭' => (false, true, false, true, true),
        '╮' => (false, true, true, false, true),
        '╰' => (true, false, false, true, true),
        '╯' => (true, false, true, false, true),
        '├' => (true, true, false, true, false),
        '┤' => (true, true, true, false, false),
        '┬' => (false, true, true, true, false),
        '┴' => (true, false, true, true, false),
        '┼' => (true, true, true, true, false),
        '╴' => (false, false, true, false, false),
        '╶' => (false, false, false, true, false),
        '╵' => (true, false, false, false, false),
        '╷' => (false, true, false, false, false),
        _ => return None,
    })
}

/// Vector shapes for glyphs that fonts draw inconsistently.
enum Shape {
    Triangle(char),
    Disc,
    Ring,
    Bullseye,
    Diamond,
}

fn shape(c: char) -> Option<Shape> {
    Some(match c {
        '▶' | '▼' | '◀' | '▲' => Shape::Triangle(c),
        '●' => Shape::Disc,
        '○' => Shape::Ring,
        '◉' => Shape::Bullseye,
        '◇' => Shape::Diamond,
        _ => return None,
    })
}

/// Render `grid` as an SVG document with a window frame titled `title`.
pub fn render(grid: &Grid, title: &str, st: &Style) -> String {
    let (cw, ch) = (st.cell_w, st.cell_h);
    let width = (PAD_X * 2.0 + grid.cols as f32 * cw).ceil();
    let height = (TITLE_H + PAD_TOP + grid.rows as f32 * ch + PAD_BOTTOM).ceil();
    let mut s = String::new();
    let _ = write!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" role="img" aria-label="{label}">"#,
        w = width,
        h = height,
        label = esc(title)
    );
    let _ = write!(
        s,
        "<title>{}</title><style>text{{font-family:{FONT};font-size:{}px;white-space:pre;font-variant-ligatures:none}}.b{{font-weight:700}}.i{{font-style:italic}}.d{{opacity:.6}}</style>",
        esc(title),
        num(st.font_size)
    );
    // Window: rounded body, title bar with its lower corners square.
    let _ = write!(
        s,
        r#"<rect x="0.5" y="0.5" width="{}" height="{}" rx="10" fill="{}" stroke="{}"/>"#,
        num(width - 1.0),
        num(height - 1.0),
        hex(st.background),
        st.border
    );
    let _ = write!(
        s,
        r#"<path d="M1 {t}V11A10 10 0 0 1 11 1H{r}A10 10 0 0 1 {w1} 11V{t}Z" fill="{}"/>"#,
        hex(st.title_bar),
        t = num(TITLE_H),
        r = num(width - 11.0),
        w1 = num(width - 1.0),
    );
    let _ = write!(
        s,
        r#"<line x1="1" y1="{y}" x2="{x2}" y2="{y}" stroke="{}"/>"#,
        st.border,
        y = num(TITLE_H + 0.5),
        x2 = num(width - 1.0)
    );
    for (i, c) in ["#ff5f57", "#febc2e", "#28c840"].iter().enumerate() {
        let _ = write!(
            s,
            r#"<circle cx="{}" cy="{}" r="6" fill="{c}"/>"#,
            20 + i * 20,
            num(TITLE_H / 2.0)
        );
    }
    let _ = write!(
        s,
        r#"<text x="{}" y="{}" text-anchor="middle" fill="{}" style="font-size:13px">{}</text>"#,
        num(width / 2.0),
        num(TITLE_H / 2.0 + 4.5),
        hex(st.title_fg),
        esc(title)
    );
    let _ = write!(
        s,
        r#"<g transform="translate({} {})">"#,
        num(PAD_X),
        num(TITLE_H + PAD_TOP)
    );

    // Backgrounds: one rect per run of equal colour on a row.
    s.push_str(r#"<g shape-rendering="crispEdges">"#);
    for y in 0..grid.rows {
        let mut x = 0;
        while x < grid.cols {
            let bg = grid.cell(x, y).bg;
            let mut end = x + 1;
            while end < grid.cols && grid.cell(end, y).bg == bg {
                end += 1;
            }
            if bg != st.background {
                let _ = write!(
                    s,
                    r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{}"/>"#,
                    num(x as f32 * cw),
                    num(y as f32 * ch),
                    num((end - x) as f32 * cw),
                    num(ch),
                    hex(bg)
                );
            }
            x = end;
        }
    }
    s.push_str("</g>");

    // Box-drawing lines: one stroked path per colour.
    let mut lines: BTreeMap<Rgb, String> = BTreeMap::new();
    let mut shapes = String::new();
    // Text: per row, one <text> per style with per-glyph x positions.
    let mut texts = String::new();
    let mut underlines = String::new();
    for y in 0..grid.rows {
        let top = y as f32 * ch;
        let cy = top + ch / 2.0;
        let baseline = cy + st.font_size * 0.35;
        let mut runs: BTreeMap<TextStyle, Run> = BTreeMap::new();
        for x in 0..grid.cols {
            let cell = grid.cell(x, y);
            let left = x as f32 * cw;
            let cx = left + cw / 2.0;
            if cell.underline {
                let _ = write!(
                    underlines,
                    r#"<rect x="{}" y="{}" width="{}" height="1" fill="{}"/>"#,
                    num(left),
                    num(baseline + 2.0),
                    num(cw),
                    hex(cell.fg)
                );
            }
            let mut chars = cell.ch.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                if !cell.ch.trim().is_empty() {
                    let e = runs
                        .entry((cell.fg, cell.bold, cell.italic, cell.dim))
                        .or_default();
                    e.0.push(num(left));
                    e.1.push_str(&esc(&cell.ch));
                }
                continue;
            };
            if c == ' ' {
                continue;
            }
            if let Some((up, down, l, r, round)) = box_arms(c) {
                let d = lines.entry(cell.fg).or_default();
                let (x0, x1, y0, y1) = (left, left + cw, top, top + ch);
                let corner = (up || down)
                    && (l || r)
                    && [up, down, l, r].iter().filter(|b| **b).count() == 2;
                if corner {
                    // One polyline through the centre, rounded if asked.
                    let hx = if l { x0 } else { x1 };
                    let vy = if up { y0 } else { y1 };
                    if round {
                        let rad = cw.min(ch) / 2.0;
                        let hx_in = if l { cx - rad } else { cx + rad };
                        let vy_in = if up { cy - rad } else { cy + rad };
                        let _ = write!(
                            d,
                            "M{} {}H{}Q{} {} {} {}V{}",
                            num(hx),
                            num(cy),
                            num(hx_in),
                            num(cx),
                            num(cy),
                            num(cx),
                            num(vy_in),
                            num(vy)
                        );
                    } else {
                        let _ = write!(d, "M{} {}H{}V{}", num(hx), num(cy), num(cx), num(vy));
                    }
                } else {
                    if l || r {
                        let _ = write!(
                            d,
                            "M{} {}H{}",
                            num(if l { x0 } else { cx }),
                            num(cy),
                            num(if r { x1 } else { cx })
                        );
                    }
                    if up || down {
                        let _ = write!(
                            d,
                            "M{} {}V{}",
                            num(cx),
                            num(if up { y0 } else { cy }),
                            num(if down { y1 } else { cy })
                        );
                    }
                }
                continue;
            }
            if let Some(sh) = shape(c) {
                let fill = hex(cell.fg);
                let r = st.font_size * 0.3;
                match sh {
                    Shape::Triangle(t) => {
                        let (a, b) = (r * 1.05, r * 0.95);
                        let pts = match t {
                            '▶' => [(cx - b, cy - a), (cx + b, cy), (cx - b, cy + a)],
                            '◀' => [(cx + b, cy - a), (cx - b, cy), (cx + b, cy + a)],
                            '▼' => [(cx - a, cy - b), (cx + a, cy - b), (cx, cy + b)],
                            _ => [(cx - a, cy + b), (cx + a, cy + b), (cx, cy - b)],
                        };
                        let _ = write!(
                            shapes,
                            r#"<path d="M{} {}L{} {}L{} {}Z" fill="{fill}"/>"#,
                            num(pts[0].0),
                            num(pts[0].1),
                            num(pts[1].0),
                            num(pts[1].1),
                            num(pts[2].0),
                            num(pts[2].1)
                        );
                    }
                    Shape::Disc => {
                        let _ = write!(
                            shapes,
                            r#"<circle cx="{}" cy="{}" r="{}" fill="{fill}"/>"#,
                            num(cx),
                            num(cy),
                            num(r)
                        );
                    }
                    Shape::Ring | Shape::Bullseye => {
                        let _ = write!(
                            shapes,
                            r#"<circle cx="{}" cy="{}" r="{}" fill="none" stroke="{fill}" stroke-width="1.3"/>"#,
                            num(cx),
                            num(cy),
                            num(r - 0.4)
                        );
                        if matches!(sh, Shape::Bullseye) {
                            let _ = write!(
                                shapes,
                                r#"<circle cx="{}" cy="{}" r="{}" fill="{fill}"/>"#,
                                num(cx),
                                num(cy),
                                num(r * 0.45)
                            );
                        }
                    }
                    Shape::Diamond => {
                        let a = r * 1.1;
                        let _ = write!(
                            shapes,
                            r#"<path d="M{} {}L{} {}L{} {}L{} {}Z" fill="none" stroke="{fill}" stroke-width="1.3" stroke-linejoin="round"/>"#,
                            num(cx),
                            num(cy - a),
                            num(cx + a * 0.85),
                            num(cy),
                            num(cx),
                            num(cy + a),
                            num(cx - a * 0.85),
                            num(cy)
                        );
                    }
                }
                continue;
            }
            let e = runs
                .entry((cell.fg, cell.bold, cell.italic, cell.dim))
                .or_default();
            e.0.push(num(left));
            e.1.push_str(&esc(&cell.ch));
        }
        for ((fg, bold, italic, dim), (xs, text)) in runs {
            let mut class = Vec::new();
            if bold {
                class.push("b");
            }
            if italic {
                class.push("i");
            }
            if dim {
                class.push("d");
            }
            let class = if class.is_empty() {
                String::new()
            } else {
                format!(r#" class="{}""#, class.join(" "))
            };
            let _ = write!(
                texts,
                r#"<text x="{}" y="{}" fill="{}"{class}>{}</text>"#,
                xs.join(" "),
                num(baseline),
                hex(fg),
                text
            );
        }
    }
    for (fg, d) in lines {
        let _ = write!(
            s,
            r#"<path d="{d}" fill="none" stroke="{}" stroke-width="1.2" stroke-linejoin="round"/>"#,
            hex(fg)
        );
    }
    s.push_str(&shapes);
    s.push_str(&underlines);
    s.push_str(&texts);
    s.push_str("</g></svg>\n");
    s
}
