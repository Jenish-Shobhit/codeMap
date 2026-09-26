//! The renderer through ratatui's TestBackend: golden snapshots of all four
//! views on paneMorph's real code with a recorded agent turn, the states
//! (not a repo, empty repo, loading, unsupported language, big folders),
//! theme colours, and key handling.
//!
//! Snapshots live in tests/snapshots/*.txt. Regenerate with
//! `UPDATE_SNAPSHOTS=1 cargo test --test views`.

mod common;

use codemorph::app::{App, Focus, InputKind, Options, Scope, View};
use codemorph::keys::{self, Action};
use codemorph::map::Level;
use codemorph::ui;
use common::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use ratatui::Terminal;

// A 94% x 92% popup on a 170x54 terminal.
const W: u16 = 160;
const H: u16 = 50;

fn draw(app: &mut App, w: u16, h: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| ui::render(f, app)).unwrap();
    terminal.backend().buffer().clone()
}

fn text(app: &mut App) -> String {
    ui::buffer_text(&draw(app, W, H))
}

/// Replace machine-specific temp paths so snapshots are stable.
fn normalize(s: &str, base: &std::path::Path) -> String {
    let base = base.display().to_string();
    let mut out = String::new();
    for line in s.lines() {
        let mut l = line.replace(&base, "<tmp>");
        if l.contains('…') {
            for k in (6..base.len()).rev() {
                let tail = format!("…{}", &base[base.len() - k..]);
                if l.contains(&tail) {
                    l = l.replace(&tail, "<tmp>");
                    break;
                }
            }
        }
        out.push_str(l.trim_end());
        out.push('\n');
    }
    out
}

fn snapshot(name: &str, actual: &str) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.txt"));
    let update = std::env::var("UPDATE_SNAPSHOTS").is_ok_and(|v| v == "1");
    if update || !path.exists() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap();
    if expected != actual {
        let new = dir.join(format!("{name}.txt.new"));
        std::fs::write(&new, actual).unwrap();
        let first = expected
            .lines()
            .zip(actual.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        panic!(
            "snapshot {name} changed at line {}:\n  expected: {:?}\n  actual:   {:?}\nwrote {}",
            first + 1,
            expected.lines().nth(first).unwrap_or(""),
            actual.lines().nth(first).unwrap_or(""),
            new.display()
        );
    }
}

fn key(app: &mut App, code: KeyCode) -> Action {
    keys::handle(app, KeyEvent::new(code, KeyModifiers::NONE))
}

fn chars(app: &mut App, s: &str) {
    for c in s.chars() {
        key(app, KeyCode::Char(c));
    }
}

#[test]
fn map_view() {
    let pm = panemorph_repo();
    let mut app = app_for(&pm, View::Map, true);
    let t = text(&mut app);
    // The folder level of paneMorph's package, like the approved mock.
    assert_eq!(app.map_level(), Level::Dir("panemorph".into()));
    for needle in [
        " map ",
        "○ paneMorph · selector-fix   main   turn 3 · 2m",
        "this turn · 4 functions",
        "M selector_command   open_selector",
        "A finish_selection   open_selector",
        "M main               selector",
        "service.py · PaneMorphService",
        "HerdrClient · 5 methods",
        "scope  last turn · done 9:44 PM",
        "⏎ zoom in  ⌫ zoom out  / search",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    snapshot("map_folder", &normalize(&t, pm.tmp.path()));
}

#[test]
fn map_zooms_into_files_and_back() {
    let pm = panemorph_repo();
    let mut app = app_for(&pm, View::Map, true);
    // Zoom into actions/, then into open_selector.py.
    app.reveal("panemorph/actions/open_selector.py", None);
    text(&mut app);
    assert_eq!(app.map_level(), Level::Dir("panemorph/actions".into()));
    let t = text(&mut app);
    assert!(t.contains("open_selector.py"), "{t}");
    key(&mut app, KeyCode::Enter);
    let t = text(&mut app);
    assert_eq!(
        app.map_level(),
        Level::File("panemorph/actions/open_selector.py".into())
    );
    assert!(t.contains("finish_selection"), "{t}");
    assert!(
        t.contains("panemorph/api.py"),
        "callees appear as external boxes\n{t}"
    );
    snapshot("map_file", &normalize(&t, pm.tmp.path()));
    // Backspace returns to the folder with the file still selected.
    key(&mut app, KeyCode::Backspace);
    text(&mut app);
    assert_eq!(app.map_level(), Level::Dir("panemorph/actions".into()));
    let scene = app.map.scene.as_ref().unwrap();
    assert!(
        matches!(&scene.nodes[app.map.cursor.node].kind, codemorph::map::NodeKind::File(f) if f.ends_with("open_selector.py"))
    );
}

#[test]
fn map_arrow_keys_move_between_boxes() {
    let pm = panemorph_repo();
    let mut app = app_for(&pm, View::Map, true);
    text(&mut app);
    let start = app.map.cursor.node;
    key(&mut app, KeyCode::Down);
    let below = app.map.cursor.node;
    assert_ne!(start, below);
    key(&mut app, KeyCode::Up);
    assert_eq!(app.map.cursor.node, start);
    // j walks the rows of a box.
    let svc = app
        .map
        .scene
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .position(|n| n.title.starts_with("service.py"))
        .unwrap();
    app.map.cursor = codemorph::map::Cursor {
        node: svc,
        row: None,
    };
    key(&mut app, KeyCode::Char('j'));
    assert_eq!(app.map.cursor.row, Some(0));
    let f = app.current_function().unwrap();
    assert_eq!(
        app.index.symbols(&f.file).unwrap().symbols[f.idx].qual(),
        "PaneMorphService.__init__"
    );
}

#[test]
fn flow_view() {
    let pm = panemorph_repo();
    let mut app = app_for(&pm, View::Map, true);
    // Search selector.py's main and open it in Flow.
    key(&mut app, KeyCode::Char('/'));
    chars(&mut app, "selector.py");
    assert!(!app.search_hits.is_empty());
    key(&mut app, KeyCode::Enter);
    text(&mut app);
    app.reveal("panemorph/actions/selector.py", Some(4));
    text(&mut app);
    let main = app
        .index
        .symbols("panemorph/actions/selector.py")
        .unwrap()
        .find("main")
        .unwrap();
    app.open_flow(codemorph::index::SymId {
        file: "panemorph/actions/selector.py".into(),
        idx: main,
    });
    let t = text(&mut app);
    assert_eq!(app.view, View::Flow);
    for needle in [
        "main()  panemorph/actions/selector.py · lines 75–98",
        "◇ mode not in {\"send\", \"bring\"}",
        "◉ return 2",
        " try ",
        "except (HerdrError",
        "A added  M changed this turn",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    // Lines this turn changed carry an M in the gutter.
    let marked = t
        .lines()
        .find(|l| l.contains("Path(result_path).write_text(json.dumps({\"choice\""))
        .unwrap();
    let gutter = marked.split('│').next().unwrap().trim();
    assert!(gutter.ends_with('A') || gutter.ends_with('M'), "{marked}");
    snapshot("flow_main", &normalize(&t, pm.tmp.path()));
    // Enter on the try box opens the first call it makes.
    let try_box = app
        .flow
        .layout
        .as_ref()
        .unwrap()
        .boxes
        .iter()
        .position(|b| b.title.as_deref() == Some("try"))
        .unwrap();
    app.flow.selected = try_box;
    key(&mut app, KeyCode::Enter);
    let t = text(&mut app);
    assert!(
        t.contains("HerdrClient") || t.contains("PaneMorphService"),
        "{t}"
    );
    key(&mut app, KeyCode::Backspace);
    let t = text(&mut app);
    assert!(t.contains("main()  panemorph/actions/selector.py"), "{t}");
}

#[test]
fn changes_view_review_and_comment() {
    let pm = panemorph_repo();
    let mut app = app_for(&pm, View::Changes, true);
    let t = text(&mut app);
    for needle in [
        "agents",
        "claude w1:p2",
        "this turn · 3 files",
        "M open_selector.py",
        "M selector.py",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    // Next file: open_selector.py.
    key(&mut app, KeyCode::Char('}'));
    let t = text(&mut app);
    assert!(
        t.contains("panemorph/actions/open_selector.py   +50 −13"),
        "{t}"
    );
    assert!(t.contains("@@ "), "{t}");
    snapshot("changes_open_selector", &normalize(&t, pm.tmp.path()));
    // space marks the hunk reviewed and persists across restarts.
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(app.reviewed_count(1).0, 1);
    let again = app_for(&pm, View::Changes, true);
    assert_eq!(again.reviewed_count(1).0, 1, "reviewed marks persist");
    // v selects, c comments; the comment joins the draft and shows inline.
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('v'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('c'));
    assert!(matches!(
        app.input.as_ref().unwrap().kind,
        InputKind::Comment { .. }
    ));
    chars(&mut app, "this loop has no timeout");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.drafts.len(), 1);
    let c = &app.drafts[0];
    assert_eq!(c.path, "panemorph/actions/open_selector.py");
    assert!(c.end >= c.start);
    let t = text(&mut app);
    assert!(t.contains("⎿ this loop has no timeout"), "{t}");
    assert!(t.contains("draft  1 comment"), "{t}");
    // Outside herdr, P keeps the draft and says why.
    key(&mut app, KeyCode::Char('P'));
    assert!(app.message.as_deref().unwrap().contains("not inside herdr"));
    assert_eq!(app.drafts.len(), 1);
    // s switches the scope to since-HEAD.
    key(&mut app, KeyCode::Char('s'));
    assert_eq!(app.scope, Scope::Session);
    key(&mut app, KeyCode::Char('s'));
    assert_eq!(app.scope, Scope::Head);
    let t = text(&mut app);
    assert!(t.contains("since HEAD · 3 files"), "{t}");
}

#[test]
fn history_view() {
    let pm = panemorph_repo();
    let mut app = app_for(&pm, View::History, true);
    let t = text(&mut app);
    for needle in [
        "4 commits · 2 branches · 1 worktree · HEAD main",
        "Merge branch 'docs'  main",
        "├─╮",
        "│ ● ",
        "├─╯",
        "tag: v0.1.0",
        "worktrees",
        "M README.md",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    snapshot("history", &normalize(&t, pm.tmp.path()));
    // j moves down, enter shows the diff.
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Enter);
    let t = text(&mut app);
    assert!(app.history.show_diff);
    assert!(t.contains("feat: implement pane and tab workflows"), "{t}");
    assert!(t.contains("panemorph/actions/extract.py  +27"), "{t}");
    key(&mut app, KeyCode::Backspace);
    assert!(!app.history.show_diff);
    // 1 opens the map with that commit's changes marked.
    key(&mut app, KeyCode::Char('1'));
    assert_eq!(app.view, View::Map);
    assert!(matches!(app.scope, Scope::Commit(_)));
    let t = text(&mut app);
    assert!(t.contains("commit "), "{t}");
}

#[test]
fn standalone_without_agent_shows_repo_path_and_head_scope() {
    let pm = panemorph_repo();
    let mut app = app_for(&pm, View::Changes, false);
    // Standalone: the most recent agent's turn in this repo is still shown.
    assert_eq!(app.scope, Scope::Head);
    let t = text(&mut app);
    assert!(t.contains("since HEAD"), "{t}");
    assert!(t.contains("paneMorph"), "{t}");
}

#[test]
fn not_a_git_repo_state() {
    let t = TempDir::new("plain");
    let state = TempDir::new("plain-state");
    write(
        t.path(),
        "tool.py",
        "def run():\n    if x:\n        return 1\n    return 2\n",
    );
    write(t.path(), "notes.md", "# notes\n");
    let mut app = App::new(Options {
        path: Some(t.path().to_path_buf()),
        view: View::Map,
        context: None,
        client: None,
        store: codemorph::store::Store::new(state.path()),
        theme: dracula(),
    });
    app.load_blocking();
    assert!(app.not_a_repo());
    let map = text(&mut app);
    assert!(map.contains("tool.py"), "{map}");
    assert!(map.contains("notes.md"), "{map}");
    assert!(map.contains("no git · structure only"), "{map}");
    app.view = View::Changes;
    let changes = text(&mut app);
    assert!(changes.contains("not a git repository"), "{changes}");
    assert!(changes.contains("Map and Flow still work."), "{changes}");
    app.view = View::History;
    assert!(text(&mut app).contains("not a git repository"));
    // Flow still works.
    app.view = View::Map;
    app.reveal("tool.py", Some(0));
    text(&mut app);
    key(&mut app, KeyCode::Char('2'));
    let flow = text(&mut app);
    assert!(flow.contains("◇ x"), "{flow}");
}

#[test]
fn empty_repo_state() {
    let t = TempDir::new("empty");
    let state = TempDir::new("empty-state");
    git(t.path(), &["init", "-q"]);
    write(t.path(), "first.py", "def hello():\n    print('hi')\n");
    let mut app = App::new(Options {
        path: Some(t.path().to_path_buf()),
        view: View::Changes,
        context: None,
        client: None,
        store: codemorph::store::Store::new(state.path()),
        theme: dracula(),
    });
    app.load_blocking();
    let changes = text(&mut app);
    assert!(changes.contains("A first.py"), "{changes}");
    assert!(changes.contains("no commits yet"), "{changes}");
    app.view = View::History;
    let history = text(&mut app);
    assert!(history.contains("no commits yet"), "{history}");
}

#[test]
fn loading_state_paints_the_frame_first() {
    let pm = panemorph_repo();
    codemorph::util::freeze_time(NOW);
    let mut app = App::new(Options {
        path: Some(pm.root.clone()),
        view: View::Map,
        context: Some(agent_context()),
        client: None,
        store: pm.store.clone(),
        theme: dracula(),
    });
    // Nothing loaded yet: the frame, tabs and rail still draw.
    let t = text(&mut app);
    assert!(t.contains(" map "), "{t}");
    assert!(t.contains("paneMorph · selector-fix"), "{t}");
    assert!(t.contains("reading files…"), "{t}");
    app.view = View::Changes;
    assert!(text(&mut app).contains("reading git status…"));
    app.view = View::History;
    assert!(text(&mut app).contains("reading history…"));
}

#[test]
fn unsupported_language_and_big_folders() {
    let t = TempDir::new("big");
    let state = TempDir::new("big-state");
    git(t.path(), &["init", "-q"]);
    for i in 0..50 {
        write(
            t.path(),
            &format!("pkg/mod_{i:02}.py"),
            &format!("def f{i}():\n    return {i}\n"),
        );
    }
    write(t.path(), "pkg/README.md", "# pkg\n");
    git(t.path(), &["add", "-A"]);
    git(t.path(), &["commit", "-q", "-m", "init"]);
    let mut app = App::new(Options {
        path: Some(t.path().to_path_buf()),
        view: View::Map,
        context: None,
        client: None,
        store: codemorph::store::Store::new(state.path()),
        theme: dracula(),
    });
    app.load_blocking();
    let first = text(&mut app);
    assert!(first.contains("page 1 of 2"), "{first}");
    assert!(first.contains("15 more"), "{first}");
    key(&mut app, KeyCode::Char(']'));
    let second = text(&mut app);
    assert!(second.contains("page 2 of 2"), "{second}");
    assert!(second.contains("README.md"), "{second}");
    // Enter on a markdown box says which grammar is missing.
    app.reveal("pkg/README.md", None);
    text(&mut app);
    key(&mut app, KeyCode::Enter);
    assert!(
        app.message.as_deref().unwrap_or("").contains("no grammar"),
        "{:?}",
        app.message
    );
    // Flow on a non-code file explains itself.
    app.open_flow(codemorph::index::SymId {
        file: "pkg/README.md".into(),
        idx: 0,
    });
    let flow = text(&mut app);
    assert!(flow.contains("no flow for pkg/README.md"), "{flow}");
}

#[test]
fn theme_colours_reach_the_screen() {
    let pm = panemorph_repo();
    let mut app = app_for(&pm, View::Changes, true);
    let buf = draw(&mut app, W, H);
    let t = dracula();
    // The active view label is the only accent block.
    let accent_cells: Vec<u16> = (0..W).filter(|&x| buf[(x, 0)].bg == t.accent).collect();
    assert!(!accent_cells.is_empty());
    let label: String = accent_cells
        .iter()
        .map(|&x| buf[(x, 0)].symbol().to_string())
        .collect();
    assert_eq!(label.trim(), "changes");
    assert_eq!(buf[(0, 5)].bg, Color::Rgb(0, 0, 0), "rail is black");
    assert_eq!(
        buf[(W - 1, 5)].bg,
        Color::Rgb(0x28, 0x2a, 0x36),
        "body is #282a36"
    );
    // Added lines sit on the green band.
    key(&mut app, KeyCode::Char('}'));
    let buf = draw(&mut app, W, H);
    let has_add_band = (0..H).any(|y| (0..W).any(|x| buf[(x, y)].bg == t.add_bg));
    assert!(has_add_band);
}

#[test]
fn keys_switch_views_and_close() {
    let pm = panemorph_repo();
    let mut app = app_for(&pm, View::Map, true);
    text(&mut app);
    key(&mut app, KeyCode::Char('3'));
    assert_eq!(app.view, View::Changes);
    key(&mut app, KeyCode::Char('4'));
    assert_eq!(app.view, View::History);
    key(&mut app, KeyCode::Char('2'));
    assert_eq!(app.view, View::Flow);
    key(&mut app, KeyCode::Char('1'));
    assert_eq!(app.view, View::Map);
    key(&mut app, KeyCode::Char('?'));
    assert!(app.show_help);
    let t = text(&mut app);
    assert!(t.contains("keys") && t.contains("paste the draft"), "{t}");
    key(&mut app, KeyCode::Esc);
    assert!(!app.show_help);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.map.focus, Focus::Rail);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.map.focus, Focus::Body);
    assert_eq!(key(&mut app, KeyCode::Esc), Action::Quit);
    let mut app = app_for(&pm, View::Map, true);
    assert_eq!(key(&mut app, KeyCode::Char('q')), Action::Quit);
}

#[test]
fn narrow_and_small_terminals_do_not_panic() {
    let pm = panemorph_repo();
    for v in View::ALL {
        let mut app = app_for(&pm, v, true);
        for (w, h) in [(40u16, 10u16), (60, 20), (80, 24), (200, 60), (15, 4)] {
            let _ = draw(&mut app, w, h);
        }
    }
}
