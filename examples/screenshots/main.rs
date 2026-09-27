//! The README screenshots, rendered from the real UI.
//!
//! ```sh
//! cargo run --example screenshots            # writes docs/assets/*.svg
//! cargo run --example screenshots -- --text  # also prints each frame as text
//! ```
//!
//! The example builds a throwaway workspace in the system temp directory:
//!
//! - a git repository holding the code of paneMorph, codeMap's public
//!   sibling plugin, vendored in `tests/fixtures/` for the snapshot tests;
//! - a short, invented history around it: two merged branches, a tag and a
//!   second worktree;
//! - one agent turn, recorded in codeMap's side store the way
//!   `codemap hook` records one. The turn is the real change of paneMorph's
//!   commit b8b645e.
//!
//! A stand-in herdr socket answers `pane.get` and `agent.list`, so the header
//! and the worktree rows take the same code path as inside herdr. The keys
//! that mark a hunk reviewed and write a comment go through `keys::handle`.
//!
//! Each view is drawn by `codemap::ui::render_buf` into a ratatui buffer and
//! written to SVG in herdr's Dracula theme. The output is deterministic:
//! commit dates and the clock are fixed. The example reads `tests/fixtures/`,
//! writes `docs/assets/`, and touches nothing else outside its temp directory.

mod svg;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::Command;

use codemap::app::{App, Options, View};
use codemap::herdr::{Client, PluginContext};
use codemap::keys;
use codemap::store::{PaneState, Store, Turn};
use codemap::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use serde_json::{json, Value};

/// Popup sizes (columns, rows) per view: roughly 94% x 92% of a 150 x 50
/// terminal, and wider for Flow so the whole chart fits.
const MAP: (u16, u16) = (140, 46);
const MAP_FILE: (u16, u16) = (140, 34);
const FLOW: (u16, u16) = (176, 50);
const CHANGES: (u16, u16) = (150, 46);
const HISTORY: (u16, u16) = (140, 42);
/// "Now" for relative times: 21 Sep 2026 21:46 UTC.
const NOW: i64 = 1_790_027_160;

fn main() {
    let print_text = std::env::args().any(|a| a == "--text");
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out_dir = manifest.join("docs/assets");
    std::fs::create_dir_all(&out_dir).expect("create docs/assets");

    let base = std::env::temp_dir().join(format!("codemap-screenshots-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let base = base.canonicalize().unwrap();
    let home = home.canonicalize().unwrap();
    // Paths print as ~/code/paneMorph instead of the temp directory.
    std::env::set_var("HOME", &home);

    let demo = build_demo(&manifest.join("tests/fixtures"), &home, &base);
    let socket = base.join("herdr.sock");
    serve_mock_herdr(&socket, &demo);

    codemap::util::freeze_time(NOW);
    let mut app = App::new(Options {
        path: None,
        view: View::Map,
        context: Some(PluginContext {
            workspace_label: Some("paneMorph".into()),
            tab_label: Some("selector-fix".into()),
            focused_pane_id: Some("w1:p2".into()),
            focused_pane_cwd: Some(demo.root.display().to_string()),
            focused_pane_agent: Some("claude".into()),
            focused_pane_status: Some("idle".into()),
            ..Default::default()
        }),
        client: Some(Client::new(&socket)),
        store: demo.store.clone(),
        theme: dracula(),
    });
    app.load_blocking();

    let mut shots: Vec<(&str, &str, Buffer)> = Vec::new();

    // Map: the package folder, with the functions this turn changed.
    app.view = View::Map;
    shots.push(("map", "Map", frame(&mut app, MAP)));

    // Map, zoomed in: select open_selector.py in its folder, then ⏎.
    let file = "panemorph/actions/open_selector.py";
    app.reveal(file, None);
    frame(&mut app, MAP);
    key(&mut app, KeyCode::Enter);
    shots.push(("map-file", "Map", frame(&mut app, MAP_FILE)));

    // Flow: finish_selection(), the function this turn added.
    frame(&mut app, FLOW);
    let idx = app
        .index
        .symbols(file)
        .and_then(|f| f.find("finish_selection"))
        .expect("open_selector.py defines finish_selection");
    app.open_flow(codemap::index::SymId {
        file: file.into(),
        idx,
    });
    shots.push(("flow", "Flow", frame(&mut app, FLOW)));

    // Changes: open_selector.py, the first hunk marked reviewed, and a
    // comment on the new wait loop, typed with the same keys a user presses.
    app.view = View::Changes;
    frame(&mut app, CHANGES);
    key(&mut app, KeyCode::Char('}'));
    key(&mut app, KeyCode::Char(' '));
    let rows = app.change_rows(app.changes.file);
    let lines = &app.diffs[app.changes.file].hunks;
    let loop_row = rows
        .iter()
        .position(|r| match *r {
            codemap::app::CRow::Line(h, l) => lines[h].lines[l].new_no == Some(43),
            _ => false,
        })
        .expect("open_selector.py line 43 is in the diff");
    app.changes.row = loop_row;
    key(&mut app, KeyCode::Char('v'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('c'));
    for c in "this loop has no timeout; stop after a few seconds".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Enter);
    shots.push(("changes", "Changes", frame(&mut app, CHANGES)));

    // History: the graph, worktrees with their agents, the merge commit.
    app.view = View::History;
    frame(&mut app, HISTORY);
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    shots.push(("history", "History", frame(&mut app, HISTORY)));

    let theme = app.theme;
    for (name, view, buf) in &shots {
        if print_text {
            println!("── {name} ──\n{}", codemap::ui::buffer_text(buf));
        }
        let title = format!("codeMap · {view} · ~/code/paneMorph");
        let doc = svg::render(&grid(buf, &theme), &title, &svg::Style::default());
        let path = out_dir.join(format!("{name}.svg"));
        std::fs::write(&path, doc).expect("write svg");
        println!(
            "wrote {}",
            path.strip_prefix(&manifest).unwrap_or(&path).display()
        );
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// herdr's dracula theme, as a user would set it in herdr's config.toml.
fn dracula() -> Theme {
    Theme::from_config_text(
        "[theme]\nname = \"dracula\"\n[theme.custom]\nsidebar_bg = \"#21222c\"\n",
    )
}

fn frame(app: &mut App, (w, h): (u16, u16)) -> Buffer {
    app.drain();
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    codemap::ui::render_buf(&mut buf, area, app);
    buf
}

fn key(app: &mut App, code: KeyCode) {
    keys::handle(app, KeyEvent::new(code, KeyModifiers::NONE));
    app.drain();
}

// ---- the demo workspace ---------------------------------------------------

struct Demo {
    root: PathBuf,
    worktree: PathBuf,
    store: Store,
}

fn git(dir: &Path, date: &str, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Example Dev")
        .env("GIT_AUTHOR_EMAIL", "dev@example.com")
        .env("GIT_COMMITTER_NAME", "Example Dev")
        .env("GIT_COMMITTER_EMAIL", "dev@example.com")
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .args([
            "-c",
            "init.defaultBranch=main",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "tag.gpgsign=false",
            "-c",
            "maintenance.auto=false",
            "-c",
            "gc.auto=0",
        ])
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let p = entry.path();
        let dest = to.join(entry.file_name());
        if p.is_dir() {
            copy_tree(&p, &dest);
        } else {
            std::fs::copy(&p, &dest).unwrap();
        }
    }
}

fn copy(fx: &Path, root: &Path, rel: &str) {
    let dest = root.join(rel);
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    // The code before the agent's turn, where the fixtures keep it.
    let before = fx.join("panemorph_before").join(rel);
    let src = if before.exists() {
        before
    } else {
        fx.join("panemorph").join(rel)
    };
    std::fs::copy(src, dest).unwrap();
}

fn write(root: &Path, rel: &str, text: &str) {
    std::fs::write(root.join(rel), text).unwrap();
}

fn build_demo(fx: &Path, home: &Path, base: &Path) -> Demo {
    let code = home.join("code");
    let root = code.join("paneMorph");
    std::fs::create_dir_all(&root).unwrap();
    let r = root.as_path();
    let day = |d: u32, h: u32| format!("2026-09-{d:02}T{h:02}:12:00Z");

    let commit = |date: &str, msg: &str| {
        git(r, date, &["add", "-A"]);
        git(r, date, &["commit", "-q", "-m", msg]);
    };
    git(r, &day(12, 10), &["init", "-q"]);
    write(r, "README.md", "# paneMorph\n\nMove live herdr panes.\n");
    write(r, ".gitignore", "__pycache__/\n*.pyc\n");
    copy(fx, r, "panemorph/__init__.py");
    commit(&day(12, 10), "chore: scaffold the package");
    write(r, "LICENSE", "MIT License\n");
    commit(&day(12, 11), "chore: add the MIT license");
    for f in ["api.py", "model.py"] {
        copy(fx, r, &format!("panemorph/{f}"));
    }
    commit(&day(13, 9), "feat: add the herdr API client");
    copy(fx, r, "panemorph/topology.py");
    commit(&day(14, 15), "feat: plan tab merges from the layout tree");
    for f in ["service.py", "doctor.py"] {
        copy(fx, r, &format!("panemorph/{f}"));
    }
    commit(&day(15, 14), "feat: add the pane service and doctor");

    git(r, &day(16, 9), &["checkout", "-q", "-b", "workflows"]);
    for f in [
        "__init__.py",
        "extract.py",
        "open_selector.py",
        "selector.py",
    ] {
        copy(fx, r, &format!("panemorph/actions/{f}"));
    }
    commit(&day(16, 10), "feat: implement pane and tab workflows");
    write(
        r,
        "herdr-plugin.toml",
        "id = \"dev.panemorph\"\nname = \"paneMorph\"\n",
    );
    commit(&day(16, 16), "feat: add the herdr plugin manifest");
    git(r, &day(16, 17), &["checkout", "-q", "main"]);
    write(
        r,
        "README.md",
        "# paneMorph\n\nMove live herdr panes between tabs without restarting them.\n",
    );
    commit(&day(16, 18), "docs: describe the selector");
    git(
        r,
        &day(17, 10),
        &[
            "merge",
            "-q",
            "--no-ff",
            "workflows",
            "-m",
            "Merge branch 'workflows'",
        ],
    );
    git(r, &day(17, 10), &["tag", "v0.1.0"]);

    git(
        r,
        &day(18, 9),
        &["checkout", "-q", "-b", "overlay-selector"],
    );
    write(
        r,
        "herdr-plugin.toml",
        "id = \"dev.panemorph\"\nname = \"paneMorph\"\nplacement = \"overlay\"\n",
    );
    commit(&day(18, 11), "fix: open the selector as an overlay");
    git(r, &day(18, 12), &["checkout", "-q", "main"]);
    write(
        r,
        "CHANGELOG.md",
        "# Changelog\n\n## 0.1.0\n\n- Send and bring panes.\n",
    );
    commit(&day(19, 15), "chore: add a changelog");
    git(
        r,
        &day(20, 10),
        &[
            "merge",
            "-q",
            "--no-ff",
            "overlay-selector",
            "-m",
            "Merge branch 'overlay-selector'",
        ],
    );
    git(r, &day(20, 10), &["branch", "-q", "-D", "overlay-selector"]);

    // A second worktree, where another agent writes docs.
    let worktree = code.join("paneMorph-docs");
    git(
        r,
        &day(21, 9),
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "docs-examples",
            worktree.to_str().unwrap(),
        ],
    );
    write(
        &worktree,
        "README.md",
        "# paneMorph\n\nMove live herdr panes between tabs without restarting them.\n\n## Examples\n",
    );
    git(
        &worktree,
        &day(21, 20),
        &["commit", "-q", "-am", "docs: add usage examples"],
    );

    // The agent's turn, recorded the way `codemap hook` records one.
    let store = Store::new(base.join("state"));
    let info = codemap::git::discover(r).unwrap();
    let shadow = store.shadow(&info);
    let mut start = shadow.snapshot("turn 3 start").unwrap();
    copy_tree(&fx.join("panemorph"), r);
    let mut end = shadow.snapshot("turn 3 end").unwrap();
    start.at = NOW - 420;
    end.at = NOW - 120;
    let pane = PaneState {
        pane_key: "term_1-claude".into(),
        pane_id: "w1:p2".into(),
        terminal_id: Some("term_1".into()),
        agent: Some("claude".into()),
        last_status: Some("idle".into()),
        updated: NOW - 120,
        turns: vec![Turn {
            n: 3,
            repo_root: info.root.to_string_lossy().into_owned(),
            start: Some(start),
            end: Some(end),
            end_status: Some("idle".into()),
            open: false,
        }],
    };
    store.save_pane(&pane).unwrap();
    Demo {
        root: info.root,
        worktree: worktree.canonicalize().unwrap(),
        store,
    }
}

/// A stand-in for herdr's socket: the two read-only calls the UI makes.
fn serve_mock_herdr(socket: &Path, demo: &Demo) {
    let listener = UnixListener::bind(socket).expect("bind mock socket");
    let root = demo.root.display().to_string();
    let worktree = demo.worktree.display().to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut out = stream;
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                let req: Value = serde_json::from_str(line.trim()).unwrap_or(Value::Null);
                line.clear();
                let result = match req["method"].as_str() {
                    Some("pane.get") => json!({"pane": {
                        "pane_id": "w1:p2", "terminal_id": "term_1", "cwd": root,
                        "agent": "claude", "agent_status": "idle",
                    }}),
                    Some("agent.list") => json!({"agents": [
                        {"pane_id": "w1:p2", "name": "claude", "agent_status": "idle", "cwd": root},
                        {"pane_id": "w2:p1", "name": "codex", "agent_status": "working", "cwd": worktree},
                    ]}),
                    _ => Value::Null,
                };
                let reply = if result.is_null() {
                    json!({"id": req["id"], "error": {"code": "unsupported", "message": "mock"}})
                } else {
                    json!({"id": req["id"], "result": result})
                };
                if writeln!(out, "{reply}").is_err() {
                    break;
                }
            }
        }
    });
}

// ---- ratatui buffer to grid ----------------------------------------------

fn rgb(c: Color, fallback: svg::Rgb) -> svg::Rgb {
    // The Dracula terminal palette, for the few named colours.
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Reset => fallback,
        Color::Black => (0x21, 0x22, 0x2c),
        Color::Red | Color::LightRed => (0xff, 0x55, 0x55),
        Color::Green | Color::LightGreen => (0x50, 0xfa, 0x7b),
        Color::Yellow | Color::LightYellow => (0xf1, 0xfa, 0x8c),
        Color::Blue | Color::LightBlue => (0xbd, 0x93, 0xf9),
        Color::Magenta | Color::LightMagenta => (0xff, 0x79, 0xc6),
        Color::Cyan | Color::LightCyan => (0x8b, 0xe9, 0xfd),
        Color::Gray => (0xbf, 0xbf, 0xbf),
        Color::DarkGray => (0x62, 0x72, 0xa4),
        Color::White => (0xf8, 0xf8, 0xf2),
        Color::Indexed(_) => fallback,
    }
}

fn grid(buf: &Buffer, theme: &Theme) -> svg::Grid {
    let area = buf.area;
    let fg0 = rgb(theme.text, (0xf8, 0xf8, 0xf2));
    let bg0 = rgb(theme.body, (0x28, 0x2a, 0x36));
    let mut cells = Vec::with_capacity(area.area() as usize);
    for y in area.y..area.y + area.height {
        let mut skip = 0;
        for x in area.x..area.x + area.width {
            let c = &buf[(x, y)];
            let mut fg = rgb(c.fg, fg0);
            let mut bg = rgb(c.bg, bg0);
            if c.modifier.contains(Modifier::REVERSED) {
                std::mem::swap(&mut fg, &mut bg);
            }
            let ch = if skip > 0 {
                skip -= 1;
                String::new()
            } else {
                let w = codemap::util::width(c.symbol());
                skip = w.saturating_sub(1);
                c.symbol().to_string()
            };
            cells.push(svg::Cell {
                ch,
                fg,
                bg,
                bold: c.modifier.contains(Modifier::BOLD),
                italic: c.modifier.contains(Modifier::ITALIC),
                dim: c.modifier.contains(Modifier::DIM),
                underline: c.modifier.contains(Modifier::UNDERLINED),
            });
        }
    }
    svg::Grid {
        cols: area.width as usize,
        rows: area.height as usize,
        cells,
    }
}
