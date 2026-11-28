mod common;

use codemap::app::{App, Focus, InputKind, Location, Options, Scope, View};
use codemap::custom::Scope as CustomScope;
use codemap::keys::{self, Action};
use codemap::open::Opener;
use codemap::ui;
use common::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use ratatui::Terminal;

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
    key(&mut app, KeyCode::Char('}'));
    let t = text(&mut app);
    assert!(
        t.contains("panemorph/actions/open_selector.py   +50 −13"),
        "{t}"
    );
    assert!(t.contains("@@ "), "{t}");
    snapshot("changes_open_selector", &normalize(&t, pm.tmp.path()));
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(app.reviewed_count(1).0, 1);
    let again = app_for(&pm, View::Changes, true);
    assert_eq!(again.reviewed_count(1).0, 1, "reviewed marks persist");
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
    key(&mut app, KeyCode::Char('P'));
    assert!(app.message.as_deref().unwrap().contains("not inside herdr"));
    assert_eq!(app.drafts.len(), 1);
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
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Enter);
    let t = text(&mut app);
    assert!(app.history.show_diff);
    assert!(t.contains("feat: implement pane and tab workflows"), "{t}");
    assert!(t.contains("panemorph/actions/extract.py  +27"), "{t}");
    key(&mut app, KeyCode::Backspace);
    assert!(!app.history.show_diff);
}

#[test]
fn standalone_without_agent_shows_repo_path_and_head_scope() {
    let pm = panemorph_repo();
    let mut app = app_for(&pm, View::Changes, false);
    assert_eq!(app.scope, Scope::Head);
    let t = text(&mut app);
    assert!(t.contains("since HEAD"), "{t}");
    assert!(t.contains("paneMorph"), "{t}");
}

#[test]
fn not_a_git_repo_state() {
    let t = TempDir::new("plain");
    let state = TempDir::new("plain-state");
    write(t.path(), "tool.py", "def run():\n    return 1\n");
    let mut app = App::new(Options {
        path: Some(t.path().to_path_buf()),
        view: View::Changes,
        context: None,
        client: None,
        store: codemap::store::Store::new(state.path()),
        theme: dracula(),
    });
    app.load_blocking();
    assert!(app.not_a_repo());
    assert!(!app.is_loading());
    let changes = text(&mut app);
    assert!(changes.contains("not a git repository"), "{changes}");
    app.view = View::History;
    assert!(text(&mut app).contains("not a git repository"));
    app.view = View::Files;
    let files = text(&mut app);
    assert!(files.contains("files · 1"), "{files}");
    assert!(files.contains("tool.py   24 B · 2 lines"), "{files}");
    assert!(files.contains("   1  def run():"), "{files}");
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
        store: codemap::store::Store::new(state.path()),
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
    codemap::util::freeze_time(NOW);
    let mut app = App::new(Options {
        path: Some(pm.root.clone()),
        view: View::Changes,
        context: Some(agent_context()),
        client: None,
        store: pm.store.clone(),
        theme: dracula(),
    });
    assert!(app.is_loading());
    let t = text(&mut app);
    assert!(t.contains(" changes "), "{t}");
    assert!(t.contains(" history "), "{t}");
    assert!(t.contains("paneMorph · selector-fix"), "{t}");
    assert!(t.contains("reading git status…"), "{t}");
    app.view = View::History;
    assert!(text(&mut app).contains("reading history…"));
    app.view = View::Files;
    assert!(text(&mut app).contains("reading files…"));
}

#[test]
fn theme_colours_reach_the_screen() {
    let pm = panemorph_repo();
    let mut app = app_for(&pm, View::Changes, true);
    let buf = draw(&mut app, W, H);
    let t = dracula();
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
    key(&mut app, KeyCode::Char('}'));
    let buf = draw(&mut app, W, H);
    let has_add_band = (0..H).any(|y| (0..W).any(|x| buf[(x, y)].bg == t.add_bg));
    assert!(has_add_band);
}

#[test]
fn keys_switch_views_and_close() {
    let pm = panemorph_repo();
    let mut app = app_for(&pm, View::Changes, true);
    text(&mut app);
    key(&mut app, KeyCode::Char('2'));
    assert_eq!(app.view, View::History);
    key(&mut app, KeyCode::Char('1'));
    assert_eq!(app.view, View::Changes);
    key(&mut app, KeyCode::Char('?'));
    assert!(app.show_help);
    let t = text(&mut app);
    assert!(t.contains("╭─ keys ─"), "{t}");
    assert!(t.contains("paste the draft"), "{t}");
    key(&mut app, KeyCode::Esc);
    assert!(!app.show_help);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.changes.focus, Focus::Rail);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.changes.focus, Focus::Body);
    assert_eq!(key(&mut app, KeyCode::Esc), Action::Quit);
    let mut app = app_for(&pm, View::Changes, true);
    assert_eq!(key(&mut app, KeyCode::Char('q')), Action::Quit);
}

#[test]
fn view_names_parse_and_the_removed_views_do_not() {
    for v in View::ALL {
        assert_eq!(View::parse(v.name()), Some(v));
    }
    for gone in ["map", "flow", "open", "1", "2"] {
        assert_eq!(View::parse(gone), None, "{gone}");
    }
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

#[test]
fn files_view_opens_on_the_first_changed_file() {
    let pm = panemorph_repo();
    let mut app = app_for(&pm, View::Files, true);
    let t = text(&mut app);
    let header = t.lines().next().unwrap();
    assert!(header.contains(" files "), "{header}");
    assert!(
        !header.contains("changes") && !header.contains("history"),
        "{header}"
    );
    for needle in [
        "files · 11",
        "▾ panemorph/",
        "▾ actions/",
        "open_selector.py",
        "panemorph/__init__.py   79 B · 3 lines   M +1 −2",
        "3+ __version__ = \"0.1.2\"",
        "marks  last turn · done 9:44 PM",
        "⏎ open  e editor  E VS Code  b browser  B GitHub  / find",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    snapshot("files_tree", &normalize(&t, pm.tmp.path()));
    assert_eq!(
        key(&mut app, KeyCode::Enter),
        Action::Edit(Location {
            path: "panemorph/__init__.py".into(),
            line: Some(3),
            dir: false,
        })
    );
    assert_eq!(
        key(&mut app, KeyCode::Char('E')),
        Action::Open(
            Opener::VsCode,
            Location {
                path: "panemorph/__init__.py".into(),
                line: Some(3),
                dir: false,
            }
        )
    );
    assert!(matches!(
        key(&mut app, KeyCode::Char('b')),
        Action::Open(Opener::Browser, _)
    ));
    assert!(matches!(
        key(&mut app, KeyCode::Char('B')),
        Action::Open(Opener::GitHub, _)
    ));
    assert_eq!(
        key(&mut app, KeyCode::Char('y')),
        Action::Copy("panemorph/__init__.py:3".into())
    );
}

#[test]
fn files_tree_folds_and_summarises_folders() {
    let pm = panemorph_repo();
    let mut app = app_for(&pm, View::Files, true);
    text(&mut app);
    let open_rows = app.file_rows().len();
    key(&mut app, KeyCode::Char('h'));
    assert_eq!(app.selected_file_row().unwrap().path, "panemorph");
    let t = text(&mut app);
    for needle in [
        "panemorph/   10 files",
        "changed in this turn",
        "M __init__.py +1 −2",
        "M actions/open_selector.py +50 −13",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    assert_eq!(
        key(&mut app, KeyCode::Char('y')),
        Action::Copy("panemorph".into())
    );
    assert_eq!(key(&mut app, KeyCode::Enter), Action::Redraw);
    assert_eq!(app.file_rows().len(), 2, "the folder is closed");
    assert!(text(&mut app).contains("▸ panemorph/"));
    key(&mut app, KeyCode::Char('l'));
    assert_eq!(app.file_rows().len(), open_rows);
    key(&mut app, KeyCode::Char('l'));
    assert_eq!(app.selected_file_row().unwrap().path, "panemorph/actions");
    key(&mut app, KeyCode::Char('G'));
    assert_eq!(app.selected_file_row().unwrap().path, "README.md");
    let t = text(&mut app);
    assert!(t.contains("README.md   "), "{t}");
    assert!(
        !t.contains("README.md   M"),
        "README.md is unchanged this turn\n{t}"
    );
}

#[test]
fn files_preview_scrolls_and_escape_returns_to_the_tree() {
    let pm = panemorph_repo();
    let mut app = app_for(&pm, View::Files, true);
    text(&mut app);
    key(&mut app, KeyCode::Char('k'));
    assert_eq!(
        app.selected_file_row().unwrap().path,
        "panemorph/actions/selector.py"
    );
    text(&mut app);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.files.focus, Focus::Body);
    for _ in 0..9 {
        key(&mut app, KeyCode::Char('j'));
    }
    let t = text(&mut app);
    assert!(t.contains("  10  from panemorph.api import"), "{t}");
    assert!(!t.contains("   9\n") && !t.contains("import curses"), "{t}");
    assert_eq!(
        key(&mut app, KeyCode::Char('e')),
        Action::Edit(Location {
            path: "panemorph/actions/selector.py".into(),
            line: Some(10),
            dir: false,
        })
    );
    key(&mut app, KeyCode::Char('G'));
    assert_eq!(app.files.preview_scroll, 101);
    assert_eq!(key(&mut app, KeyCode::Esc), Action::Redraw);
    assert_eq!(app.files.focus, Focus::Rail);
    assert_eq!(key(&mut app, KeyCode::Esc), Action::Quit);
}

#[test]
fn files_filter_finds_a_file_and_reveals_it() {
    let pm = panemorph_repo();
    let mut app = app_for(&pm, View::Files, true);
    text(&mut app);
    key(&mut app, KeyCode::Char('/'));
    assert!(matches!(
        app.input.as_ref().unwrap().kind,
        InputKind::Filter
    ));
    chars(&mut app, "topo");
    let t = text(&mut app);
    assert!(t.contains("matches · "), "{t}");
    assert_eq!(app.file_rows()[0].path, "panemorph/topology.py");
    assert!(t.contains("panemorph/topology.py   2 KB · 89 lines"), "{t}");
    assert!(t.contains("/ topo█"), "{t}");
    key(&mut app, KeyCode::Enter);
    assert!(app.input.is_none() && app.files.filter.is_none());
    assert_eq!(
        app.selected_file_row().unwrap().path,
        "panemorph/topology.py"
    );
    let t = text(&mut app);
    assert!(t.contains("files · 11"), "{t}");
    assert!(t.contains("panemorph/topology.py   "), "{t}");

    key(&mut app, KeyCode::Char('/'));
    chars(&mut app, "zzz");
    assert!(text(&mut app).contains("no file matches"));
    key(&mut app, KeyCode::Esc);
    assert!(app.input.is_none() && app.files.filter.is_none());
    assert_eq!(
        app.selected_file_row().unwrap().path,
        "panemorph/topology.py",
        "escape puts the selection back"
    );
}

#[test]
fn number_keys_stay_inside_a_popup() {
    let pm = panemorph_repo();
    let mut app = app_for(&pm, View::Files, true);
    assert_eq!(key(&mut app, KeyCode::Char('2')), Action::None);
    assert_eq!(app.view, View::Files);
    assert_eq!(View::Files.tabs(), [View::Files]);
    assert_eq!(View::History.tabs(), [View::Changes, View::History]);
}

#[test]
fn customisations_view_lists_what_the_agent_runs_with() {
    let (tmp, roots) = custom_roots();
    let state = TempDir::new("custom-state");
    let mut app = custom_app(&roots, state.path());
    assert_eq!(app.custom.scope, CustomScope::Project);
    let t = text(&mut app);
    let header = t.lines().next().unwrap();
    for tab in [" project ", " global ", " tools ", " other agents "] {
        assert!(header.contains(tab), "{header}");
    }
    for needle in [
        "Instructions",
        "CLAUDE.md",
        "Skills",
        "review",
        "Agents",
        "planner",
        "/ship",
        "PreToolUse",
        "docs",
        "Memory",
        "A small service with a review workflow.",
        "Sections  Rules · Releases",
        "Size      11 lines",
        "instructions · project",
        "1-4 scope",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    snapshot("custom_project", &normalize(&t, tmp.path()));

    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    let t = text(&mut app);
    assert!(t.contains("skill · project"), "{t}");
    assert!(
        t.contains("Review a diff for bugs before it is committed."),
        "{t}"
    );
    assert!(t.contains("   6  # Review"), "{t}");
    let skill = roots.repo.join(".claude/skills/review/SKILL.md");
    assert_eq!(
        key(&mut app, KeyCode::Enter),
        Action::Edit(Location {
            path: skill.display().to_string(),
            line: None,
            dir: false,
        })
    );
    assert_eq!(
        key(&mut app, KeyCode::Char('y')),
        Action::Copy(skill.display().to_string())
    );
}

#[test]
fn customisations_scopes_switch_with_numbers_and_arrows() {
    let (tmp, roots) = custom_roots();
    let state = TempDir::new("custom-state");
    let mut app = custom_app(&roots, state.path());
    key(&mut app, KeyCode::Char('2'));
    assert_eq!(app.custom.scope, CustomScope::Global);
    let t = text(&mut app);
    for needle in [
        "CLAUDE.md              empty",
        "Empty file. Nothing is loaded from it.",
        "settings.json          opus",
        "frontend-design        official",
        "Hooks",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    key(&mut app, KeyCode::Char('l'));
    assert_eq!(app.custom.scope, CustomScope::Tools);
    let t = text(&mut app);
    for needle in [
        "herdr",
        "config.toml            theme gruvbox",
        "Backups       1 hidden",
        "yazi",
        "neovim",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    assert!(!t.contains("config.toml.bak"), "backups stay hidden\n{t}");
    snapshot("custom_tools", &normalize(&t, tmp.path()));
    key(&mut app, KeyCode::Char('4'));
    let t = text(&mut app);
    assert!(t.contains("In this repository"), "{t}");
    assert!(t.contains("AGENTS.md"), "{t}");
    assert!(t.contains("Codex"), "{t}");
    key(&mut app, KeyCode::Char('h'));
    assert_eq!(app.custom.scope, CustomScope::Tools);
    assert_eq!(key(&mut app, KeyCode::Char('s')), Action::None);
    assert_eq!(key(&mut app, KeyCode::Tab), Action::Redraw);
    assert_eq!(key(&mut app, KeyCode::Esc), Action::Quit);
}

#[test]
fn customisations_opens_on_global_when_the_project_has_none() {
    let (_tmp, mut roots) = custom_roots();
    let bare = TempDir::new("custom-bare");
    roots.repo = bare.path().to_path_buf();
    let state = TempDir::new("custom-state");
    let mut app = custom_app(&roots, state.path());
    assert_eq!(app.custom.scope, CustomScope::Global);
    key(&mut app, KeyCode::Char('1'));
    let t = text(&mut app);
    assert!(
        t.contains("nothing is set up for Claude in this repository"),
        "{t}"
    );
    assert!(t.contains("Skills"), "empty kinds still show\n{t}");
}
