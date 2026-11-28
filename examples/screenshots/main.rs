mod demo;
mod herdr;
mod svg;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};

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

const FILES: (u16, u16) = (140, 40);
const CUSTOM: (u16, u16) = (140, 34);
const CHANGES: (u16, u16) = (150, 46);
const HISTORY: (u16, u16) = (128, 40);
const NOW: i64 = 1_790_027_160;

fn main() {
    let print_text = std::env::args().any(|a| a == "--text");
    let with_herdr = std::env::args().any(|a| a == "--herdr");
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out_dir = manifest.join("docs/assets");
    std::fs::create_dir_all(&out_dir).expect("create docs/assets");

    let base = std::env::temp_dir().join(format!("codemap-screenshots-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let base = base.canonicalize().unwrap();
    let home = home.canonicalize().unwrap();
    std::env::set_var("HOME", &home);

    let fixtures = manifest.join("tests/fixtures");
    let repo = demo::build_repo(&fixtures, &home);
    let store = record_turn(&fixtures, &repo, &base);
    let socket = base.join("herdr.sock");
    serve_mock_herdr(&socket, &repo);

    codemap::util::freeze_time(NOW);
    let mut app = App::new(Options {
        path: None,
        view: View::Files,
        context: Some(PluginContext {
            workspace_label: Some("paneMorph".into()),
            tab_label: Some("selector-fix".into()),
            focused_pane_id: Some("w1:p2".into()),
            focused_pane_cwd: Some(repo.root.display().to_string()),
            focused_pane_agent: Some("claude".into()),
            focused_pane_status: Some("idle".into()),
            ..Default::default()
        }),
        client: Some(Client::new(&socket)),
        store,
        theme: dracula(),
    });
    app.load_blocking();

    let mut shots: Vec<(&str, &str, Buffer)> = Vec::new();

    frame(&mut app, FILES);
    app.select_path("panemorph/actions/open_selector.py");
    shots.push(("files", "Files", frame(&mut app, FILES)));

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

    app.view = View::History;
    frame(&mut app, HISTORY);
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    shots.push(("history", "History", frame(&mut app, HISTORY)));

    demo::add_customisations(&repo.root, &home);
    let mut custom = App::new(Options {
        path: Some(repo.root.clone()),
        view: View::Custom,
        context: None,
        client: None,
        store: Store::new(base.join("custom-state")),
        theme: dracula(),
    });
    custom.custom.roots = Some(codemap::custom::Roots {
        repo: repo.root.clone(),
        home: home.clone(),
        config: home.join(".config"),
    });
    custom.load_blocking();
    frame(&mut custom, CUSTOM);
    for _ in 0..3 {
        key(&mut custom, KeyCode::Char('j'));
    }
    shots.push((
        "customisations",
        "Customisations",
        frame(&mut custom, CUSTOM),
    ));

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

    if with_herdr {
        let path = out_dir.join("herdr.svg");
        match herdr::capture(&manifest, &fixtures, &path) {
            Ok(()) => println!("wrote docs/assets/herdr.svg"),
            Err(e) => {
                eprintln!("herdr capture failed: {e}");
                std::process::exit(1);
            }
        }
    }
}

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

fn record_turn(fixtures: &Path, repo: &demo::Repo, base: &Path) -> Store {
    let store = Store::new(base.join("state"));
    let info = codemap::git::discover(&repo.root).unwrap();
    let shadow = store.shadow(&info);
    let mut start = shadow.snapshot("turn 3 start").unwrap();
    demo::apply_turn(fixtures, &repo.root);
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
    store
}

fn serve_mock_herdr(socket: &Path, repo: &demo::Repo) {
    let listener = UnixListener::bind(socket).expect("bind mock socket");
    let root = repo.root.display().to_string();
    let worktree = repo.worktree.display().to_string();
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

fn rgb(c: Color, fallback: svg::Rgb) -> svg::Rgb {
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
