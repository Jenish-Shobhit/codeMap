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
        gh: no_gh(),
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
        gh: no_gh(),
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
        gh: no_gh(),
    });
    assert!(app.is_loading());
    let t = text(&mut app);
    assert!(t.contains(" status   review   history "), "{t}");
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
    assert_eq!(label.trim(), "review");
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
    for (number, view, label) in [
        ('3', View::History, "history"),
        ('4', View::Branches, "branches"),
        ('1', View::Status, "status"),
        ('2', View::Changes, "review"),
    ] {
        assert_eq!(key(&mut app, KeyCode::Char(number)), Action::Redraw);
        assert_eq!(app.view, view);
        let buf = draw(&mut app, W, H);
        let current: String = (0..W)
            .filter(|&x| buf[(x, 0)].bg == app.theme.accent)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert_eq!(current.trim(), label);
    }
    assert_eq!(key(&mut app, KeyCode::Char('5')), Action::None);
    assert_eq!(app.view, View::Changes);
    key(&mut app, KeyCode::Char('?'));
    assert!(app.show_help);
    let t = text(&mut app);
    assert!(t.contains("╭─ keys ─"), "{t}");
    for needle in [
        "1 2 3 4       status, review, history, branches",
        "paste the draft",
        "branches",
        "⏎  n          switch to the branch, name a new one from HEAD",
        "m  D          merge into the current branch, delete: both ask",
        "B  y          open the pull request or branch page, copy name",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
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
        assert_eq!(View::parse(v.label()), Some(v));
    }
    assert_eq!(View::parse("git"), Some(View::Status));
    assert_eq!(View::parse("Branches"), Some(View::Branches));
    assert_eq!(View::Changes.name(), "changes");
    assert_eq!(View::Changes.label(), "review");
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
        !header.contains("status") && !header.contains("review") && !header.contains("history"),
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
    let git_popup = [View::Status, View::Changes, View::History, View::Branches];
    for tab in git_popup {
        assert_eq!(tab.tabs(), git_popup);
    }
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
    assert!(header.trim_end().ends_with(" demo"), "{header}");
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

fn status_repo() -> PmRepo {
    let pm = panemorph_repo();
    let p = pm.root.as_path();
    git(p, &["mv", "panemorph/doctor.py", "panemorph/checkup.py"]);
    git(p, &["rm", "-q", "panemorph/topology.py"]);
    write(
        p,
        "panemorph/actions/undo.py",
        "def undo(service, pane):\n    service.restore(pane)\n    return 0\n",
    );
    git(
        p,
        &["add", "panemorph/__init__.py", "panemorph/actions/undo.py"],
    );
    write(
        p,
        "panemorph/__init__.py",
        "\"\"\"paneMorph: live pane and tab workflows for Herdr.\"\"\"\n\n__version__ = \"0.1.2\"\n\n__all__ = [\"__version__\"]\n",
    );
    write(p, "notes.md", "# Notes\n\nThe selector needs a timeout.\n");
    pm
}

fn app_at(root: &std::path::Path, state: &TempDir, view: View) -> App {
    let mut app = App::new(Options {
        path: Some(root.to_path_buf()),
        view,
        context: None,
        client: None,
        store: codemap::store::Store::new(state.path()),
        theme: dracula(),
        gh: no_gh(),
    });
    app.load_blocking();
    app
}

fn at(path: &str, line: Option<u32>) -> Location {
    Location {
        path: path.into(),
        line,
        dir: false,
    }
}

#[test]
fn status_view_lists_staged_and_unstaged_files() {
    let pm = status_repo();
    let mut app = app_for(&pm, View::Status, true);
    let t = text(&mut app);
    let header = t.lines().next().unwrap();
    assert!(header.contains(" status   review   history "), "{header}");
    assert!(
        header.contains("paneMorph · selector-fix   main"),
        "{header}"
    );
    assert!(!header.contains("turn 3"), "the turn is the review tab's");
    for needle in [
        " staged · 4",
        "M __init__.py          +1 −2",
        "R checkup.py",
        "D topology.py          −89",
        "A undo.py              +3",
        " not staged · 4",
        "? notes.md             +3",
        "M __init__.py          +2",
        "M open_selector.py     +50 −13",
        "M selector.py          +11 −11",
        "panemorph/__init__.py   +1 −2    staged",
        "@@ top level · 1–3",
        "   3+ __version__ = \"0.1.2\"",
        "tab diff",
        "j k file  space unstage  c commit  e editor  ? keys",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    snapshot("status", &normalize(&t, pm.tmp.path()));

    let buf = draw(&mut app, W, H);
    let theme = dracula();
    let letter = |symbol: &str| {
        (0..H)
            .find(|&y| buf[(1, y)].symbol() == symbol)
            .map(|y| buf[(1, y)].fg)
    };
    assert_eq!(letter("M"), Some(theme.modified));
    assert_eq!(letter("R"), Some(theme.renamed));
    assert_eq!(letter("D"), Some(theme.del));
    assert_eq!(letter("A"), Some(theme.add));
    assert_eq!(letter("?"), Some(theme.add));
    let band = |bg: Color| (0..H).any(|y| (0..W).any(|x| buf[(x, y)].bg == bg));
    assert!(band(theme.add_bg) && band(theme.del_bg));
}

#[test]
fn status_selection_moves_through_both_groups() {
    let pm = status_repo();
    let mut app = app_for(&pm, View::Status, true);
    let shown = |app: &mut App| text(app).lines().nth(2).unwrap().to_string();
    assert!(shown(&mut app).contains("panemorph/__init__.py   +1 −2    staged"));
    key(&mut app, KeyCode::Char('j'));
    let t = text(&mut app);
    assert!(
        t.contains("panemorph/checkup.py  ← panemorph/doctor.py      staged"),
        "{t}"
    );
    assert!(t.contains("renamed, content unchanged"), "{t}");
    key(&mut app, KeyCode::Down);
    let t = text(&mut app);
    assert!(t.contains("panemorph/topology.py   −89    staged"), "{t}");
    assert!(
        t.contains("   1− from __future__ import annotations"),
        "{t}"
    );
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    assert!(shown(&mut app).contains("notes.md   +3    untracked"));
    key(&mut app, KeyCode::Char('j'));
    let t = text(&mut app);
    assert!(
        t.contains("panemorph/__init__.py   +2    not staged"),
        "{t}"
    );
    assert!(t.contains("   5+ __all__ = [\"__version__\"]"), "{t}");
    assert!(!t.contains("0.1.0"), "the staged half stays out\n{t}");
    key(&mut app, KeyCode::Up);
    assert!(shown(&mut app).contains("notes.md"));
    key(&mut app, KeyCode::Char('G'));
    assert!(shown(&mut app).contains("panemorph/actions/selector.py   +11 −11    not staged"));
    assert_eq!(key(&mut app, KeyCode::Char('j')), Action::Redraw);
    assert_eq!(app.status.sel, 7, "the last file stays selected");
    key(&mut app, KeyCode::Char('g'));
    assert_eq!(app.status.sel, 0);
    keys::handle(
        &mut app,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
    );
    assert_eq!(app.status.sel, 7);
    keys::handle(
        &mut app,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    assert_eq!(app.status.sel, 0);
    assert_eq!(key(&mut app, KeyCode::Char('s')), Action::None);
    assert_eq!(app.scope, Scope::Turn, "s belongs to the review tab");
    assert_eq!(key(&mut app, KeyCode::Enter), Action::None);
}

#[test]
fn status_body_scrolls_and_escape_returns_to_the_list() {
    let pm = status_repo();
    let mut app = app_for(&pm, View::Status, true);
    assert_eq!(app.status.focus, Focus::Rail);
    for _ in 0..6 {
        key(&mut app, KeyCode::Char('j'));
    }
    let t = text(&mut app);
    assert!(
        t.contains("panemorph/actions/open_selector.py   +50 −13    not staged"),
        "{t}"
    );
    assert!(
        t.contains("   1  from __future__ import annotations"),
        "{t}"
    );
    assert_eq!(key(&mut app, KeyCode::Tab), Action::Redraw);
    assert_eq!(app.status.focus, Focus::Body);
    let t = text(&mut app);
    assert!(t.contains("tab files"), "{t}");
    assert!(
        t.contains("j k scroll  ] [ hunk  space stage hunk  x discard  c commit  e editor  ? keys"),
        "{t}"
    );
    for _ in 0..4 {
        key(&mut app, KeyCode::Char('j'));
    }
    assert_eq!((app.status.sel, app.status.scroll), (6, 4));
    let t = text(&mut app);
    assert!(t.contains("›   4+ import json"), "{t}");
    assert!(!t.contains("   3  import os"), "{t}");
    key(&mut app, KeyCode::Char('k'));
    assert!(text(&mut app).contains("›   3  import os"));
    keys::handle(
        &mut app,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
    );
    assert_eq!(app.status.scroll, 18);
    keys::handle(
        &mut app,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    assert_eq!(app.status.scroll, 3);
    key(&mut app, KeyCode::Char('G'));
    let last = app.status.scroll;
    assert!(last > 60, "{last}");
    key(&mut app, KeyCode::Char('j'));
    assert_eq!(app.status.scroll, last, "the last row stays on screen");
    let t = text(&mut app);
    assert!(
        t.contains("panemorph/actions/open_selector.py   +50 −13"),
        "the path stays above the scrolled diff\n{t}"
    );
    key(&mut app, KeyCode::Char('g'));
    assert_eq!(app.status.scroll, 0);
    key(&mut app, KeyCode::Char('j'));
    assert_eq!(key(&mut app, KeyCode::Esc), Action::Redraw);
    assert_eq!(app.status.focus, Focus::Rail);
    key(&mut app, KeyCode::Char('k'));
    assert_eq!(
        (app.status.sel, app.status.scroll),
        (5, 0),
        "another file starts at its top"
    );
    assert_eq!(key(&mut app, KeyCode::Esc), Action::Quit);
}

#[test]
fn status_opens_the_selected_file_at_the_first_change_of_the_shown_diff() {
    let pm = status_repo();
    let mut app = app_for(&pm, View::Status, true);
    let staged = at("panemorph/__init__.py", Some(3));
    assert_eq!(app.location(), Some(staged.clone()));
    assert_eq!(
        key(&mut app, KeyCode::Char('e')),
        Action::Edit(staged.clone())
    );
    assert_eq!(
        key(&mut app, KeyCode::Char('E')),
        Action::Open(Opener::VsCode, staged.clone())
    );
    assert_eq!(
        key(&mut app, KeyCode::Char('b')),
        Action::Open(Opener::Browser, staged.clone())
    );
    assert_eq!(
        key(&mut app, KeyCode::Char('B')),
        Action::Open(Opener::GitHub, staged)
    );
    assert_eq!(
        key(&mut app, KeyCode::Char('y')),
        Action::Copy("panemorph/__init__.py:3".into())
    );
    key(&mut app, KeyCode::Char('j'));
    assert_eq!(
        key(&mut app, KeyCode::Char('y')),
        Action::Copy("panemorph/checkup.py".into()),
        "a rename without text changes has no line"
    );
    for _ in 0..4 {
        key(&mut app, KeyCode::Char('j'));
    }
    assert_eq!(
        key(&mut app, KeyCode::Char('y')),
        Action::Copy("panemorph/__init__.py:4".into()),
        "the same file, not staged, opens at its own change"
    );
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Tab);
    assert_eq!(
        key(&mut app, KeyCode::Char('e')),
        Action::Edit(at("panemorph/actions/open_selector.py", Some(4)))
    );
    for _ in 0..5 {
        key(&mut app, KeyCode::Char('j'));
    }
    assert_eq!(
        key(&mut app, KeyCode::Char('e')),
        Action::Edit(at("panemorph/actions/open_selector.py", Some(7))),
        "the first change at or below the top row"
    );
}

#[test]
fn only_the_git_popup_reads_the_status() {
    let pm = status_repo();
    for view in [View::Status, View::Changes, View::History] {
        let app = app_for(&pm, view, true);
        assert!(app.status.loaded, "{view:?}");
        assert_eq!(app.status.len(), 8, "{view:?}");
    }
    for view in [View::Files, View::Custom] {
        let app = app_for(&pm, view, true);
        assert!(!app.status.loaded, "{view:?}");
        assert!(app.status.is_empty(), "{view:?}");
    }
}

#[test]
fn status_loading_state() {
    let pm = status_repo();
    let mut app = App::new(Options {
        path: Some(pm.root.clone()),
        view: View::Status,
        context: Some(agent_context()),
        client: None,
        store: pm.store.clone(),
        theme: dracula(),
        gh: no_gh(),
    });
    let t = text(&mut app);
    assert!(t.contains(" status   review   history "), "{t}");
    assert!(t.contains("reading git status…"), "{t}");
    assert!(!t.contains("staged ·"), "{t}");
    assert_eq!(key(&mut app, KeyCode::Char('j')), Action::Redraw);
    assert_eq!(key(&mut app, KeyCode::Char('e')), Action::None);
    app.load_blocking();
    assert!(text(&mut app).contains(" staged · 4"));
}

#[test]
fn status_not_a_git_repository_state() {
    let plain = TempDir::new("status-plain");
    let state = TempDir::new("status-plain-state");
    write(plain.path(), "tool.py", "def run():\n    return 1\n");
    let mut app = app_at(plain.path(), &state, View::Status);
    let t = text(&mut app);
    assert!(t.contains("not a git repository · "), "{t}");
    assert!(!t.contains("staged ·"), "{t}");
    assert_eq!(key(&mut app, KeyCode::Char('y')), Action::None);
}

#[test]
fn status_clean_tree_state() {
    let repo = repo_with(&[("a.py", "x = 1\n")]);
    let state = TempDir::new("status-clean-state");
    let mut app = app_at(repo.path(), &state, View::Status);
    let t = text(&mut app);
    for needle in [
        " staged · 0",
        " not staged · 0",
        "nothing to commit, working tree clean",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    assert_eq!(key(&mut app, KeyCode::Char('j')), Action::Redraw);
    assert_eq!(key(&mut app, KeyCode::Char('e')), Action::None);
    assert_eq!(key(&mut app, KeyCode::Tab), Action::Redraw);
    assert_eq!(key(&mut app, KeyCode::Char('G')), Action::Redraw);
    assert_eq!(app.status.scroll, 0);
}

#[test]
fn status_read_error_state() {
    let repo = repo_with(&[("a.py", "x = 1\n")]);
    let state = TempDir::new("status-error-state");
    write(repo.path(), "a.py", "x = 2\n");
    let mut app = app_at(repo.path(), &state, View::Status);
    assert!(text(&mut app).contains(" not staged · 1"));
    app.status.load(Err("index file corrupt".into()));
    let t = text(&mut app);
    assert!(t.contains("git: index file corrupt"), "{t}");
    assert!(!t.contains("staged ·"), "{t}");
    assert_eq!(key(&mut app, KeyCode::Char('e')), Action::None);
}

#[test]
fn status_of_a_repository_with_no_commits() {
    let t = TempDir::new("status-unborn");
    let state = TempDir::new("status-unborn-state");
    git(t.path(), &["init", "-q"]);
    write(t.path(), "first.py", "def hello():\n    print('hi')\n");
    write(t.path(), "second.py", "x = 1\n");
    git(t.path(), &["add", "first.py"]);
    let mut app = app_at(t.path(), &state, View::Status);
    let frame = text(&mut app);
    for needle in [
        "no commits yet",
        " staged · 1",
        "A first.py             +2",
        " not staged · 1",
        "? second.py            +1",
        "first.py   +2    staged",
        "   1+ def hello():",
    ] {
        assert!(frame.contains(needle), "missing {needle:?}\n{frame}");
    }
    key(&mut app, KeyCode::Char('j'));
    assert!(text(&mut app).contains("second.py   +1    untracked"));
}

fn ctrl(app: &mut App, c: char) -> Action {
    keys::handle(app, KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

fn numbered(changed: &[usize]) -> String {
    (1..=30)
        .map(|n| {
            if changed.contains(&n) {
                format!("line {n} changed\n")
            } else {
                format!("line {n}\n")
            }
        })
        .collect()
}

fn work_repo() -> (TempDir, TempDir) {
    let repo = repo_with(&[
        ("a.py", "x = 1\n"),
        ("b.py", "y = 1\n"),
        ("f.txt", &numbered(&[])),
        ("gone.py", "z = 1\n"),
    ]);
    let p = repo.path();
    commits_as_test(p);
    write(p, "a.py", "x = 2\n");
    write(p, "b.py", "y = 2\n");
    write(p, "f.txt", &numbered(&[1, 15, 30]));
    write(p, "notes.md", "# Notes\n\nKeep the timeout.\n");
    std::fs::remove_file(p.join("gone.py")).unwrap();
    (repo, TempDir::new("work-state"))
}

fn work_app(root: &std::path::Path, state: &TempDir) -> App {
    let mut app = App::new(Options {
        path: Some(root.to_path_buf()),
        view: View::Status,
        context: Some(agent_context()),
        client: None,
        store: codemap::store::Store::new(state.path()),
        theme: dracula(),
        gh: no_gh(),
    });
    app.load_blocking();
    app
}

fn porcelain(repo: &std::path::Path) -> Vec<String> {
    fingerprint(repo).1.lines().map(str::to_string).collect()
}

fn in_index(repo: &std::path::Path, path: &str) -> String {
    git(repo, &["cat-file", "blob", &format!(":{path}")])
}

fn on_disk(repo: &std::path::Path, path: &str) -> String {
    std::fs::read_to_string(repo.join(path)).unwrap()
}

fn heading(app: &mut App) -> String {
    let buf = draw(app, W, H);
    let body = ui::areas(buf.area, View::Status).body;
    let row: String = (body.x..W)
        .map(|x| buf[(x, body.y)].symbol().to_string())
        .collect();
    row.trim().to_string()
}

fn select(app: &mut App, path: &str, staged: bool) {
    let first = if staged { 0 } else { app.status.staged.len() };
    let group = if staged {
        &app.status.staged
    } else {
        &app.status.unstaged
    };
    let at = group
        .iter()
        .position(|f| f.path == path)
        .unwrap_or_else(|| panic!("{path} is not listed"));
    key(app, KeyCode::Char('g'));
    for _ in 0..first + at {
        key(app, KeyCode::Char('j'));
    }
    assert_eq!(app.status.selected().unwrap().path, path);
    assert_eq!(app.status.selected_is_staged(), staged);
}

fn bar_rows(app: &mut App) -> Vec<String> {
    let buf = draw(app, W, H);
    let accent = app.theme.accent;
    let x = ui::areas(buf.area, View::Status).body.x - 1;
    (0..H)
        .filter(|&y| buf[(x, y)].symbol() == "│" && buf[(x, y)].fg == accent)
        .map(|y| {
            let row: String = (x + 1..W)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect();
            row.trim().to_string()
        })
        .collect()
}

#[test]
fn space_stages_the_selected_file_and_moves_to_the_next() {
    let (repo, state) = work_repo();
    let p = repo.path();
    let mut app = work_app(p, &state);
    let t = text(&mut app);
    for needle in [
        " staged · 0",
        " not staged · 5",
        "a.py   +1 −1    not staged",
        "j k file  space stage  x discard  c commit  e editor  ? keys",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    assert_eq!(key(&mut app, KeyCode::Char(' ')), Action::Redraw);
    assert_eq!(app.message.as_deref(), Some("staged a.py"));
    let t = text(&mut app);
    assert!(
        t.contains(" staged · 1") && t.contains(" not staged · 4"),
        "{t}"
    );
    assert!(
        t.contains("b.py   +1 −1    not staged"),
        "the next file\n{t}"
    );
    assert_eq!(
        porcelain(p),
        [
            "M  a.py",
            " M b.py",
            " M f.txt",
            " D gone.py",
            "?? notes.md"
        ]
    );
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(heading(&mut app), "f.txt   +3 −3    not staged");
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(app.message.as_deref(), Some("staged gone.py"));
    assert_eq!(heading(&mut app), "notes.md   +3    untracked");
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(
        heading(&mut app),
        "f.txt   +3 −3    not staged",
        "the last file of the list gives way to the one above"
    );
    assert_eq!(
        porcelain(p),
        [
            "M  a.py",
            "M  b.py",
            " M f.txt",
            "D  gone.py",
            "A  notes.md"
        ]
    );
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(app.status.unstaged.len(), 0);
    assert_eq!(
        heading(&mut app),
        "f.txt   +3 −3    staged",
        "with nothing left the selection follows the file"
    );
    assert_eq!(app.status.focus, Focus::Rail);
}

#[test]
fn space_unstages_a_staged_file() {
    let (repo, state) = work_repo();
    let p = repo.path();
    git(p, &["add", "-A"]);
    let mut app = work_app(p, &state);
    assert_eq!((app.status.staged.len(), app.status.unstaged.len()), (5, 0));
    select(&mut app, "gone.py", true);
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(app.message.as_deref(), Some("unstaged gone.py"));
    assert_eq!(heading(&mut app), "notes.md   +3    staged");
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(app.message.as_deref(), Some("unstaged notes.md"));
    assert_eq!(heading(&mut app), "f.txt   +3 −3    staged");
    assert_eq!(
        porcelain(p),
        [
            "M  a.py",
            "M  b.py",
            "M  f.txt",
            " D gone.py",
            "?? notes.md"
        ]
    );
    assert_eq!(on_disk(p, "notes.md"), "# Notes\n\nKeep the timeout.\n");
}

#[test]
fn a_staged_rename_unstages_both_of_its_names() {
    let repo = repo_with(&[("old_name.py", "def keep():\n    return 1\n")]);
    let state = TempDir::new("rename-state");
    let p = repo.path();
    git(p, &["mv", "old_name.py", "new_name.py"]);
    let mut app = work_app(p, &state);
    assert_eq!(heading(&mut app), "new_name.py  ← old_name.py      staged");
    key(&mut app, KeyCode::Tab);
    assert!(text(&mut app).contains("space unstage file"));
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(porcelain(p), [" D old_name.py", "?? new_name.py"]);
}

#[test]
fn the_hunk_cursor_follows_the_top_row_of_the_diff() {
    let (repo, state) = work_repo();
    let mut app = work_app(repo.path(), &state);
    select(&mut app, "f.txt", false);
    assert_eq!(bar_rows(&mut app), [""; 0], "the list acts on whole files");
    assert_eq!(heading(&mut app), "f.txt   +3 −3    not staged");
    key(&mut app, KeyCode::Tab);
    assert_eq!(
        heading(&mut app),
        "f.txt   +3 −3    not staged    hunk 1 of 3"
    );
    assert_eq!(
        bar_rows(&mut app),
        [
            "@@ top level · 1–4",
            "1− line 1",
            "1+ line 1 changed",
            "2  line 2",
            "3  line 3",
            "4  line 4"
        ]
    );
    key(&mut app, KeyCode::Char(']'));
    assert_eq!(
        heading(&mut app),
        "f.txt   +3 −3    not staged    hunk 2 of 3"
    );
    let t = text(&mut app);
    snapshot("status_hunk_cursor", &normalize(&t, repo.path()));
    assert_eq!(bar_rows(&mut app).len(), 9);
    assert_eq!(bar_rows(&mut app)[5], "15+ line 15 changed");
    for _ in 0..3 {
        key(&mut app, KeyCode::Char('j'));
    }
    assert_eq!(app.status.hunk_at_cursor(), Some(1), "inside the hunk");
    assert_eq!(bar_rows(&mut app).len(), 6, "what is left of it on screen");
    key(&mut app, KeyCode::Char('['));
    assert_eq!(app.status.scroll, 6, "back to the heading of this hunk");
    key(&mut app, KeyCode::Char('['));
    assert_eq!(
        (app.status.scroll, app.status.hunk_at_cursor()),
        (0, Some(0))
    );
    assert_eq!(key(&mut app, KeyCode::Char('[')), Action::Redraw);
    assert_eq!(app.status.scroll, 0);
    key(&mut app, KeyCode::Char('G'));
    assert_eq!(app.status.hunk_at_cursor(), Some(2));
    key(&mut app, KeyCode::Char(']'));
    assert_eq!(
        app.status.hunk_at_cursor(),
        Some(2),
        "there is no hunk below"
    );
    key(&mut app, KeyCode::Esc);
    assert_eq!(bar_rows(&mut app), [""; 0]);
    assert_eq!(key(&mut app, KeyCode::Char(']')), Action::None);
}

#[test]
fn space_in_the_diff_stages_and_unstages_the_hunk_under_the_cursor() {
    let (repo, state) = work_repo();
    let p = repo.path();
    let mut app = work_app(p, &state);
    select(&mut app, "f.txt", false);
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Char(']'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(app.message.as_deref(), Some("staged hunk 2 of 3 in f.txt"));
    assert_eq!(in_index(p, "f.txt"), numbered(&[15]));
    assert_eq!(on_disk(p, "f.txt"), numbered(&[1, 15, 30]));
    assert_eq!(
        heading(&mut app),
        "f.txt   +2 −2    not staged    hunk 2 of 2",
        "the hunk that was below is under the cursor"
    );
    let t = text(&mut app);
    assert!(t.contains(" staged · 1") && t.contains("M f.txt"), "{t}");
    assert!(t.contains("│@@ line 26 · 27–30"), "{t}");
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(in_index(p, "f.txt"), numbered(&[15, 30]));
    assert_eq!(
        heading(&mut app),
        "f.txt   +1 −1    not staged    hunk 1 of 1"
    );
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(in_index(p, "f.txt"), numbered(&[1, 15, 30]));
    assert_eq!(porcelain(p)[2], "M  f.txt");
    assert_eq!(
        heading(&mut app),
        "gone.py   −1    not staged",
        "the file left the list, so the next one is shown"
    );
    assert_eq!(app.status.focus, Focus::Body);

    key(&mut app, KeyCode::Esc);
    select(&mut app, "f.txt", true);
    key(&mut app, KeyCode::Tab);
    assert!(text(&mut app).contains("space unstage hunk  c commit"));
    key(&mut app, KeyCode::Char(']'));
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(
        app.message.as_deref(),
        Some("unstaged hunk 2 of 3 in f.txt")
    );
    assert_eq!(in_index(p, "f.txt"), numbered(&[1, 30]));
    assert_eq!(heading(&mut app), "f.txt   +2 −2    staged    hunk 2 of 2");
    assert_eq!(on_disk(p, "f.txt"), numbered(&[1, 15, 30]));
}

#[test]
fn space_in_the_diff_takes_the_whole_file_where_a_hunk_means_nothing() {
    let (repo, state) = work_repo();
    let p = repo.path();
    let mut app = work_app(p, &state);
    select(&mut app, "notes.md", false);
    key(&mut app, KeyCode::Tab);
    let t = text(&mut app);
    assert!(t.contains("space stage file  x discard  c commit"), "{t}");
    assert!(!t.contains("hunk 1 of 1"), "{t}");
    assert_eq!(bar_rows(&mut app), [""; 0]);
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(app.message.as_deref(), Some("staged notes.md"));
    assert_eq!(porcelain(p)[4], "A  notes.md");
    key(&mut app, KeyCode::Esc);
    select(&mut app, "gone.py", false);
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(porcelain(p)[3], "D  gone.py");
}

#[test]
fn a_write_that_git_refuses_is_a_message_and_the_list_is_read_again() {
    let (repo, state) = work_repo();
    let p = repo.path();
    let mut app = work_app(p, &state);
    select(&mut app, "notes.md", false);
    std::fs::remove_file(p.join("notes.md")).unwrap();
    assert_eq!(key(&mut app, KeyCode::Char(' ')), Action::Redraw);
    let said = app.message.clone().unwrap();
    assert!(
        said.contains("notes.md") && said.contains("did not match"),
        "{said}"
    );
    assert_eq!(app.status.unstaged.len(), 4, "the list is fresh");
    assert_eq!(heading(&mut app), "gone.py   −1    not staged");

    select(&mut app, "f.txt", false);
    key(&mut app, KeyCode::Tab);
    write(p, "f.txt", &numbered(&[1, 2, 15, 30]));
    let before = fingerprint(p);
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(
        app.message.as_deref(),
        Some("f.txt changed since this diff was read")
    );
    assert_eq!(before, fingerprint(p));
    assert_eq!(
        heading(&mut app),
        "f.txt   +4 −4    not staged    hunk 1 of 3"
    );
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(in_index(p, "f.txt"), numbered(&[1, 2]));
}

fn type_commit(app: &mut App, subject: &str) {
    key(app, KeyCode::Char('c'));
    assert!(app.commit.open, "{:?}", app.message);
    chars(app, subject);
}

#[test]
fn c_opens_the_commit_box_only_when_something_is_staged() {
    let (repo, state) = work_repo();
    let p = repo.path();
    let mut app = work_app(p, &state);
    assert_eq!(key(&mut app, KeyCode::Char('c')), Action::Redraw);
    assert!(!app.commit.open);
    let t = text(&mut app);
    assert!(
        t.contains("nothing staged to commit · space stages the selected file"),
        "{t}"
    );
    assert!(!t.contains("commit to main"), "{t}");
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Char('c'));
    assert!(app.commit.open);
    let t = text(&mut app);
    for needle in [
        "╭─ commit to main ─",
        "subject  █",
        "body",
        "2 files staged",
        "⏎ commit   tab body   ⌃u clear   ⎋ close, keeps the draft",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    assert!(
        !t.contains("space stage"),
        "the hints under the box are off\n{t}"
    );
    chars(&mut app, "fix: q? 1 x y");
    assert_eq!(app.commit.subject, "fix: q? 1 x y");
    assert!(app.commit.open && !app.show_help && app.view == View::Status);
    assert!(app.confirm.is_none() && app.outcome.is_none());
    assert_eq!(key(&mut app, KeyCode::Esc), Action::Redraw);
    assert!(!app.commit.open && app.outcome.is_none());
    assert_eq!(porcelain(p)[..2], ["M  a.py", "M  b.py"]);
    assert_eq!(git(p, &["rev-list", "--count", "HEAD"]).trim(), "1");
}

#[test]
fn the_commit_box_edits_a_subject_and_a_body() {
    let (repo, state) = work_repo();
    let p = repo.path();
    let mut app = work_app(p, &state);
    key(&mut app, KeyCode::Char(' '));
    type_commit(&mut app, "fix: count from twoo");
    key(&mut app, KeyCode::Backspace);
    assert_eq!(key(&mut app, KeyCode::Tab), Action::Redraw);
    chars(&mut app, "One was never right.");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Enter);
    chars(&mut app, "Counting starts at two.");
    assert_eq!(
        app.commit.body,
        "One was never right.\n\nCounting starts at two."
    );
    let t = text(&mut app);
    for needle in [
        "subject  fix: count from two",
        "body     One was never right.",
        "         Counting starts at two.█",
        "1 file staged",
        "⏎ new line   tab subject",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    snapshot("status_commit_box", &normalize(&t, p));
    ctrl(&mut app, 'u');
    assert_eq!(app.commit.body, "");
    assert_eq!(app.commit.subject, "fix: count from two");
    chars(&mut app, "Second try.");
    key(&mut app, KeyCode::Esc);
    assert!(!app.commit.open);
    assert!(!text(&mut app).contains("commit to main"));
    assert_eq!(git(p, &["rev-list", "--count", "HEAD"]).trim(), "1");

    key(&mut app, KeyCode::Char('c'));
    let t = text(&mut app);
    assert!(
        t.contains("subject  fix: count from two"),
        "the draft is back\n{t}"
    );
    assert!(t.contains("body     Second try.█"), "{t}");
    key(&mut app, KeyCode::BackTab);
    ctrl(&mut app, 'u');
    assert_eq!(app.commit.subject, "");
    assert_eq!(key(&mut app, KeyCode::Enter), Action::Redraw);
    let t = text(&mut app);
    assert!(app.commit.open);
    assert!(
        t.contains("an empty subject does not commit · write one first"),
        "{t}"
    );
    assert_eq!(app.commit.body, "Second try.");
    chars(&mut app, "   ");
    key(&mut app, KeyCode::Enter);
    assert!(app.commit.open, "blanks are no subject");
    assert_eq!(git(p, &["rev-list", "--count", "HEAD"]).trim(), "1");
    assert_eq!(porcelain(p)[0], "M  a.py");
}

#[test]
fn enter_in_the_subject_commits_what_is_staged() {
    let (repo, state) = work_repo();
    let p = repo.path();
    let mut app = work_app(p, &state);
    assert_eq!(app.history.commits.len(), 1);
    key(&mut app, KeyCode::Char(' '));
    type_commit(&mut app, "fix: count from two");
    key(&mut app, KeyCode::Tab);
    chars(&mut app, "One was never right.");
    key(&mut app, KeyCode::Enter);
    chars(&mut app, "  Two is.");
    key(&mut app, KeyCode::Tab);
    assert_eq!(key(&mut app, KeyCode::Enter), Action::Redraw);
    assert!(!app.commit.open, "{:?}", app.commit.problem);
    let short = git(p, &["rev-parse", "--short", "HEAD"]);
    assert_eq!(
        app.message.clone().unwrap(),
        format!("committed {} fix: count from two", short.trim())
    );
    assert_eq!(
        git(p, &["log", "-1", "--format=%an%n%B"]),
        "Test\nfix: count from two\n\nOne was never right.\n  Two is.\n\n"
    );
    assert_eq!(
        porcelain(p),
        [" M b.py", " M f.txt", " D gone.py", "?? notes.md"]
    );
    let t = text(&mut app);
    assert!(
        t.contains(" staged · 0") && t.contains(" not staged · 4"),
        "{t}"
    );
    assert!(t.contains(&format!("committed {}", short.trim())), "{t}");
    assert_eq!(app.history.commits.len(), 2, "history is read again");
    assert_eq!(app.history.commits[0].subject, "fix: count from two");
    assert!(
        app.diffs.iter().all(|d| d.path != "a.py"),
        "the review diff is read again"
    );
    assert_eq!(
        app.repo.as_ref().unwrap().head.as_deref().map(|h| &h[..7]),
        Some(&short.trim()[..7])
    );

    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Char('c'));
    assert_eq!(
        (app.commit.subject.as_str(), app.commit.body.as_str()),
        ("", ""),
        "a commit clears the draft"
    );
    assert!(text(&mut app).contains("subject  █"));
}

#[test]
fn a_commit_that_fails_keeps_the_box_and_the_draft() {
    let (repo, state) = work_repo();
    let p = repo.path();
    hook(
        p,
        "pre-commit",
        "#!/bin/sh\necho 'lint: a.py is not formatted' >&2\necho 'run the formatter first' >&2\nexit 1\n",
    );
    let mut app = work_app(p, &state);
    key(&mut app, KeyCode::Char(' '));
    type_commit(&mut app, "fix: count from two");
    key(&mut app, KeyCode::Tab);
    chars(&mut app, "One was never right.");
    key(&mut app, KeyCode::Tab);
    let head = git(p, &["rev-parse", "HEAD"]);
    assert_eq!(key(&mut app, KeyCode::Enter), Action::Redraw);
    assert!(app.commit.open && !app.commit.busy);
    assert_eq!(
        app.commit.problem,
        ["lint: a.py is not formatted", "run the formatter first"]
    );
    let t = text(&mut app);
    for needle in [
        "╭─ commit to main ─",
        "subject  fix: count from two█",
        "body     One was never right.",
        "1 file staged",
        "lint: a.py is not formatted",
        "run the formatter first",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    assert_eq!(git(p, &["rev-parse", "HEAD"]), head);
    assert_eq!(porcelain(p)[0], "M  a.py");
    chars(&mut app, "!");
    assert!(
        app.commit.problem.is_empty(),
        "an edit clears the complaint"
    );
    key(&mut app, KeyCode::Backspace);

    std::fs::remove_file(p.join(".git/hooks/pre-commit")).unwrap();
    key(&mut app, KeyCode::Enter);
    assert!(!app.commit.open);
    assert_eq!(
        git(p, &["log", "-1", "--format=%B"]),
        "fix: count from two\n\nOne was never right.\n\n"
    );
}

#[test]
fn the_commit_box_names_a_detached_head_and_makes_the_first_commit() {
    let (repo, state) = work_repo();
    let p = repo.path();
    git(p, &["checkout", "-q", "--detach"]);
    let mut app = work_app(p, &state);
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Char('c'));
    assert!(text(&mut app).contains("╭─ commit on a detached HEAD ─"));

    let unborn = TempDir::new("commit-unborn");
    let unborn_state = TempDir::new("commit-unborn-state");
    let p = unborn.path();
    git(p, &["init", "-q"]);
    commits_as_test(p);
    write(p, "first.py", "x = 1\n");
    let mut app = work_app(p, &unborn_state);
    assert!(text(&mut app).contains("no commits yet"));
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(porcelain(p), ["A  first.py"]);
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(
        porcelain(p),
        ["?? first.py"],
        "unstaged with no HEAD to restore from"
    );
    key(&mut app, KeyCode::Char(' '));
    type_commit(&mut app, "feat: the first file");
    assert!(text(&mut app).contains("╭─ commit to main ─"));
    key(&mut app, KeyCode::Enter);
    assert!(!app.commit.open, "{:?}", app.commit.problem);
    assert_eq!(git(p, &["log", "--format=%s"]), "feat: the first file\n");
    let t = text(&mut app);
    assert!(!t.contains("no commits yet"), "{t}");
    assert!(t.contains("nothing to commit, working tree clean"), "{t}");
    assert_eq!(app.history.commits.len(), 1);
}

fn question(app: &mut App) -> Vec<String> {
    asked(app, "discard")
}

fn asked(app: &mut App, title: &str) -> Vec<String> {
    let t = text(app);
    let lines: Vec<&str> = t.lines().collect();
    let top = lines
        .iter()
        .position(|l| l.contains(&format!("╭─ {title} ─")))
        .unwrap_or_else(|| panic!("no {title} question on screen\n{t}"));
    lines[top + 1..]
        .iter()
        .take_while(|l| !l.contains('╰'))
        .map(|l| {
            let inside = l.split('│').nth(1).unwrap_or_default();
            inside.trim().to_string()
        })
        .filter(|l| !l.is_empty())
        .collect()
}

#[test]
fn x_asks_before_it_discards_a_file() {
    let (repo, state) = work_repo();
    let p = repo.path();
    let mut app = work_app(p, &state);
    assert_eq!(key(&mut app, KeyCode::Char('x')), Action::Redraw);
    assert!(app.confirm.is_some());
    assert_eq!(
        question(&mut app),
        [
            "discard 2 changed lines in a.py?",
            "this cannot be undone",
            "y discards · any other key keeps"
        ]
    );
    let t = text(&mut app);
    assert!(!t.contains("space stage"), "{t}");
    snapshot("status_discard_question", &normalize(&t, p));
    assert_eq!(on_disk(p, "a.py"), "x = 2\n", "asking changes nothing");
    assert_eq!(key(&mut app, KeyCode::Char('y')), Action::Redraw);
    assert!(app.confirm.is_none());
    assert_eq!(on_disk(p, "a.py"), "x = 1\n");
    assert_eq!(
        app.message.as_deref(),
        Some("discarded the changes to a.py")
    );
    assert_eq!(
        porcelain(p),
        [" M b.py", " M f.txt", " D gone.py", "?? notes.md"]
    );
    assert_eq!(heading(&mut app), "b.py   +1 −1    not staged");
    assert!(
        app.diffs.iter().all(|d| d.path != "a.py"),
        "the review diff is read again"
    );
}

#[test]
fn x_deletes_an_untracked_file_and_restores_a_deleted_one() {
    let (repo, state) = work_repo();
    let p = repo.path();
    let mut app = work_app(p, &state);
    select(&mut app, "notes.md", false);
    key(&mut app, KeyCode::Char('x'));
    assert_eq!(
        question(&mut app),
        [
            "delete the untracked file notes.md?",
            "this cannot be undone",
            "y deletes · any other key keeps"
        ]
    );
    assert!(p.join("notes.md").exists());
    key(&mut app, KeyCode::Char('y'));
    assert!(!p.join("notes.md").exists());
    assert_eq!(app.message.as_deref(), Some("deleted notes.md"));
    assert_eq!(heading(&mut app), "gone.py   −1    not staged");
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Char('x'));
    assert_eq!(question(&mut app)[0], "discard the deletion of gone.py?");
    key(&mut app, KeyCode::Char('y'));
    assert_eq!(on_disk(p, "gone.py"), "z = 1\n");
    assert_eq!(app.message.as_deref(), Some("restored gone.py"));
    assert_eq!(porcelain(p), [" M a.py", " M b.py", " M f.txt"]);
}

#[test]
fn x_in_the_diff_discards_the_hunk_under_the_cursor_only() {
    let (repo, state) = work_repo();
    let p = repo.path();
    let mut app = work_app(p, &state);
    select(&mut app, "f.txt", false);
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Char(']'));
    key(&mut app, KeyCode::Char('x'));
    assert_eq!(
        question(&mut app)[0],
        "discard 2 changed lines in f.txt (hunk 2 of 3)?"
    );
    let before = fingerprint(p).0;
    key(&mut app, KeyCode::Char('y'));
    assert_eq!(on_disk(p, "f.txt"), numbered(&[1, 30]));
    assert_eq!(before, fingerprint(p).0, "the index is not touched");
    assert_eq!(
        app.message.as_deref(),
        Some("discarded hunk 2 of 3 in f.txt")
    );
    assert_eq!(
        heading(&mut app),
        "f.txt   +2 −2    not staged    hunk 2 of 2"
    );
    key(&mut app, KeyCode::Char('x'));
    key(&mut app, KeyCode::Char('y'));
    assert_eq!(on_disk(p, "f.txt"), numbered(&[1]));
    assert_eq!(
        heading(&mut app),
        "f.txt   +1 −1    not staged    hunk 1 of 1"
    );
}

fn keys_other_than_a_plain_y() -> Vec<KeyEvent> {
    let plain = |code| KeyEvent::new(code, KeyModifiers::NONE);
    let held = |c, modifiers| KeyEvent::new(KeyCode::Char(c), modifiers);
    let mut repeated = plain(KeyCode::Char('y'));
    repeated.kind = crossterm::event::KeyEventKind::Repeat;
    vec![
        plain(KeyCode::Enter),
        plain(KeyCode::Esc),
        plain(KeyCode::Char('n')),
        plain(KeyCode::Char('Y')),
        plain(KeyCode::Char('q')),
        plain(KeyCode::Char('x')),
        plain(KeyCode::Char(' ')),
        plain(KeyCode::Char('c')),
        plain(KeyCode::Char('m')),
        plain(KeyCode::Char('D')),
        plain(KeyCode::Char('B')),
        plain(KeyCode::Char('?')),
        plain(KeyCode::Char('2')),
        plain(KeyCode::Tab),
        plain(KeyCode::Backspace),
        plain(KeyCode::Down),
        held('y', KeyModifiers::CONTROL),
        held('y', KeyModifiers::ALT),
        held('y', KeyModifiers::SHIFT),
        repeated,
    ]
}

#[test]
fn only_y_discards() {
    let (repo, state) = work_repo();
    let p = repo.path();
    git(p, &["add", "b.py"]);
    let mut app = work_app(p, &state);
    let before = (fingerprint(p), tree_bytes(p));
    let other_keys = keys_other_than_a_plain_y();
    for target in ["a.py", "notes.md", "gone.py", "f.txt"] {
        select(&mut app, target, false);
        for in_diff in [false, true] {
            for other in other_keys.iter().copied() {
                if in_diff {
                    app.status.focus = Focus::Body;
                }
                key(&mut app, KeyCode::Char('x'));
                assert!(app.confirm.is_some(), "{target}");
                assert_eq!(keys::handle(&mut app, other), Action::Redraw, "{other:?}");
                assert!(app.confirm.is_none(), "{other:?} answers the question");
                assert_eq!(app.message.as_deref(), Some("nothing discarded"));
                assert!(app.outcome.is_none() && !app.show_help && !app.commit.open);
                assert_eq!(app.view, View::Status);
                assert_eq!(app.status.selected().unwrap().path, target, "{other:?}");
                app.status.focus = Focus::Rail;
            }
        }
    }
    key(&mut app, KeyCode::Char('x'));
    assert_eq!(keys::paste(&mut app, "y"), Action::Redraw);
    assert!(app.confirm.is_none(), "pasted text is not an answer");
    assert_eq!(before, (fingerprint(p), tree_bytes(p)));
}

#[test]
fn x_leaves_what_is_staged_and_what_has_nothing_to_restore_from() {
    let (repo, state) = work_repo();
    let p = repo.path();
    git(p, &["add", "b.py"]);
    write(p, "later.py", "w = 4\n");
    git(p, &["add", "--intent-to-add", "later.py"]);
    let mut app = work_app(p, &state);
    let before = (fingerprint(p), tree_bytes(p));
    select(&mut app, "b.py", true);
    for focus in [Focus::Rail, Focus::Body] {
        app.status.focus = focus;
        assert_eq!(key(&mut app, KeyCode::Char('x')), Action::Redraw);
        assert!(app.confirm.is_none());
        assert_eq!(
            app.message.as_deref(),
            Some("x discards only what is not staged · space unstages this first")
        );
        key(&mut app, KeyCode::Char('y'));
    }
    app.status.focus = Focus::Rail;
    select(&mut app, "later.py", false);
    key(&mut app, KeyCode::Char('x'));
    assert!(app.confirm.is_none());
    assert_eq!(
        app.message.as_deref(),
        Some("later.py is new to the index · there is nothing to restore it from")
    );
    key(&mut app, KeyCode::Char('y'));
    assert_eq!(before, (fingerprint(p), tree_bytes(p)));
}

#[test]
fn pasted_text_is_never_a_command_in_the_status_tab() {
    let (repo, state) = work_repo();
    let p = repo.path();
    let mut app = work_app(p, &state);
    let before = (fingerprint(p), tree_bytes(p));
    assert_eq!(keys::paste(&mut app, "xy cq \n"), Action::None);
    assert!(app.confirm.is_none() && !app.commit.open && app.outcome.is_none());
    assert_eq!(before, (fingerprint(p), tree_bytes(p)));

    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Char('c'));
    assert_eq!(
        keys::paste(&mut app, "fix: count\nfrom two\n"),
        Action::Redraw
    );
    assert_eq!(app.commit.subject, "fix: count from two");
    assert!(app.commit.open, "a pasted line break does not commit");
    key(&mut app, KeyCode::Tab);
    keys::paste(&mut app, "One.\r\nTwo.");
    assert_eq!(app.commit.body, "One.\nTwo.");
    assert_eq!(git(p, &["rev-list", "--count", "HEAD"]).trim(), "1");
}

#[test]
fn walking_the_status_tab_writes_nothing() {
    let (repo, state) = work_repo();
    let p = repo.path();
    git(p, &["add", "a.py"]);
    write(p, "a.py", "x = 3\n");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    write(p, "b.py", "y = 2\n");
    let before = (fingerprint(p), tree_bytes(p));
    let mut app = work_app(p, &state);
    assert_eq!((app.status.staged.len(), app.status.unstaged.len()), (1, 5));
    text(&mut app);
    for _ in 0..7 {
        key(&mut app, KeyCode::Char('j'));
        text(&mut app);
    }
    key(&mut app, KeyCode::Char('g'));
    ctrl(&mut app, 'd');
    ctrl(&mut app, 'u');
    select(&mut app, "f.txt", false);
    key(&mut app, KeyCode::Tab);
    for step in [']', ']', ']', '[', 'j', 'j', 'k', 'G', 'g'] {
        key(&mut app, KeyCode::Char(step));
        text(&mut app);
    }
    ctrl(&mut app, 'd');
    key(&mut app, KeyCode::PageDown);
    key(&mut app, KeyCode::PageUp);
    key(&mut app, KeyCode::Char('x'));
    assert!(app.confirm.is_some());
    text(&mut app);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('x'));
    key(&mut app, KeyCode::Char('n'));
    select(&mut app, "notes.md", false);
    key(&mut app, KeyCode::Char('x'));
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('c'));
    assert!(app.commit.open);
    chars(&mut app, "fix: not yet");
    key(&mut app, KeyCode::Tab);
    chars(&mut app, "A body.");
    key(&mut app, KeyCode::Enter);
    text(&mut app);
    key(&mut app, KeyCode::Tab);
    ctrl(&mut app, 'u');
    key(&mut app, KeyCode::Enter);
    assert!(app.commit.open, "an empty subject commits nothing");
    key(&mut app, KeyCode::Esc);
    for held in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
        for c in [' ', 'x'] {
            keys::handle(&mut app, KeyEvent::new(KeyCode::Char(c), held));
            assert!(
                app.confirm.is_none() && app.message.is_none(),
                "{held:?} {c:?}"
            );
        }
    }
    keys::handle(
        &mut app,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT),
    );
    assert!(!app.commit.open);
    keys::paste(&mut app, "xy cy \n");
    key(&mut app, KeyCode::Char('?'));
    text(&mut app);
    key(&mut app, KeyCode::Esc);
    for tab in ['2', '3', '4', '1'] {
        key(&mut app, KeyCode::Char(tab));
        text(&mut app);
    }
    assert_eq!(
        key(&mut app, KeyCode::Char('y')),
        Action::Copy("notes.md:1".into())
    );
    assert_eq!(key(&mut app, KeyCode::Esc), Action::Quit);
    let after = (fingerprint(p), tree_bytes(p));
    assert_eq!(before.0 .0, after.0 .0, ".git changed while looking");
    assert_eq!(before.0 .1, after.0 .1, "git status changed while looking");
    assert_eq!(before.1, after.1, "the working tree changed while looking");
}

#[test]
fn the_commit_box_and_the_question_fit_small_terminals() {
    let (repo, state) = work_repo();
    let p = repo.path();
    let mut app = work_app(p, &state);
    key(&mut app, KeyCode::Char(' '));
    type_commit(&mut app, &"a long subject ".repeat(12));
    key(&mut app, KeyCode::Tab);
    for line in 0..12 {
        chars(&mut app, &format!("body line {line}"));
        key(&mut app, KeyCode::Enter);
    }
    app.commit.refuse("one\ntwo\nthree\nfour\nfive\n");
    for (w, h) in [
        (20u16, 6u16),
        (24, 8),
        (40, 10),
        (60, 20),
        (80, 24),
        (200, 60),
    ] {
        let _ = draw(&mut app, w, h);
    }
    let short = ui::buffer_text(&draw(&mut app, 100, 18));
    assert!(
        short.contains("four") && short.contains("close, keeps the draft"),
        "a short window gives up body lines before what git said\n{short}"
    );
    assert!(short.contains("body     body line 10"), "{short}");
    let t = text(&mut app);
    assert!(
        t.contains("subject  …"),
        "a long subject shows its end\n{t}"
    );
    assert!(
        t.contains("body     body line 7"),
        "the last lines of the body\n{t}"
    );
    assert!(t.contains("four") && !t.contains("five"), "{t}");
    key(&mut app, KeyCode::Esc);
    write(
        p,
        "a/very/long/folder/name/that/goes/on/and/on/and/on/and/on/and/on/for/a/while/notes.md",
        "x\n",
    );
    app.reload_status();
    key(&mut app, KeyCode::Char('G'));
    select(
        &mut app,
        "a/very/long/folder/name/that/goes/on/and/on/and/on/and/on/and/on/for/a/while/notes.md",
        false,
    );
    key(&mut app, KeyCode::Char('x'));
    for (w, h) in [
        (20u16, 6u16),
        (24, 8),
        (40, 10),
        (60, 20),
        (80, 24),
        (200, 60),
    ] {
        let _ = draw(&mut app, w, h);
    }
    let asked = question(&mut app);
    assert_eq!(asked[0], "delete the untracked file");
    assert!(asked[1].ends_with("for/a/while/notes.md?"), "{asked:?}");
}

fn working_context() -> codemap::herdr::PluginContext {
    codemap::herdr::PluginContext {
        focused_pane_status: Some("working".into()),
        ..agent_context()
    }
}

fn git_options(
    root: &std::path::Path,
    state: &TempDir,
    view: View,
    gh: codemap::gh::Gh,
) -> Options {
    Options {
        path: Some(root.to_path_buf()),
        view,
        context: Some(agent_context()),
        client: None,
        store: codemap::store::Store::new(state.path()),
        theme: dracula(),
        gh,
    }
}

fn loaded(options: Options) -> App {
    let mut app = App::new(options);
    app.load_blocking();
    app
}

fn pull_requests_fixture() -> String {
    fixture("gh/pull_requests.json")
}

struct BranchesTab {
    remoted: Remoted,
    _state: TempDir,
    _gh: TempDir,
    app: App,
}

fn branches_tab_over(context: codemap::herdr::PluginContext) -> BranchesTab {
    let remoted = branches_repo();
    let state = TempDir::new("branches-state");
    let bin = TempDir::new("branches-gh");
    let gh = gh_printing(bin.path(), &pull_requests_fixture());
    let app = loaded(Options {
        context: Some(context),
        ..git_options(&remoted.root, &state, View::Branches, gh)
    });
    BranchesTab {
        remoted,
        _state: state,
        _gh: bin,
        app,
    }
}

fn branches_tab() -> BranchesTab {
    branches_tab_over(agent_context())
}

fn on_branch(app: &App) -> String {
    app.branches
        .selected()
        .map(|b| b.name.clone())
        .unwrap_or_default()
}

fn select_branch(app: &mut App, name: &str) {
    key(app, KeyCode::Char('g'));
    while on_branch(app) != name {
        let before = app.branches.sel;
        key(app, KeyCode::Char('j'));
        assert_ne!(app.branches.sel, before, "{name} is not listed");
    }
}

fn local_branches(repo: &std::path::Path) -> Vec<String> {
    git(repo, &["branch", "--format=%(refname:short)"])
        .lines()
        .map(str::to_string)
        .collect()
}

fn checked_out(repo: &std::path::Path) -> String {
    git(repo, &["branch", "--show-current"]).trim().to_string()
}

fn commit_at(repo: &std::path::Path, revision: &str) -> String {
    git(repo, &["rev-parse", revision]).trim().to_string()
}

fn colour_of(app: &mut App, symbol: &str) -> Option<Color> {
    let buf = draw(app, W, H);
    (0..H)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .find(|at| buf[*at].symbol() == symbol)
        .map(|at| buf[at].fg)
}

#[test]
fn branches_view_lists_local_branches_their_upstreams_and_pull_requests() {
    let mut tab = branches_tab();
    let app = &mut tab.app;
    assert_eq!(on_branch(app), "main", "it opens on the current branch");
    let t = text(app);
    let header = t.lines().next().unwrap();
    assert!(
        header.contains(" status   review   history   branches "),
        "{header}"
    );
    assert!(
        header.contains("paneMorph · selector-fix   main"),
        "{header}"
    );
    for needle in [
        " local · 5",
        "   feat/ahead  ↑2           #14 ✓",
        "   feat/both   ↑2 ↓1        #12 ✗",
        "   local-only  no upstream  #9 …",
        " ● main",
        "   merged      no upstream  #8 ✓",
        " origin · 3 branches",
        "main    current branch",
        "upstream      origin/main · in step",
        "pull request  none",
        "main is the default branch",
        "j k branch",
        "n new  B GitHub  y copy name  ? keys",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    assert!(!t.contains("⏎ switch"), "the current branch is not offered");
    snapshot("branches", &t);

    let theme = dracula();
    assert_eq!(colour_of(app, "●"), Some(theme.add));
    assert_eq!(colour_of(app, "✓"), Some(theme.add));
    assert_eq!(colour_of(app, "✗"), Some(theme.del));
    assert_eq!(colour_of(app, "…"), Some(theme.modified));
    let buf = draw(app, W, H);
    let current_tab: String = (0..W)
        .filter(|&x| buf[(x, 0)].bg == theme.accent)
        .map(|x| buf[(x, 0)].symbol().to_string())
        .collect();
    assert_eq!(current_tab.trim(), "branches");

    select_branch(app, "feat/both");
    let t = text(app);
    for needle in [
        "upstream      origin/feat/both · 2 commits ahead, 1 behind",
        "pull request  #12 fix: keep both sides of the selector",
        "              open · into main · 1 of 3 checks passed ✗",
        "3 commits not on main",
        " fix: the second local step",
        " fix: the first local step",
        " fix: the shared start",
        "⏎ switch  n new  m merge  D delete  B pull request  y copy name  ? keys",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    assert!(
        !t.contains("only on the remote"),
        "what the upstream has is not this branch's"
    );
    let newest = t.find("the second local step").unwrap();
    assert!(newest < t.find("the shared start").unwrap(), "newest first");
    snapshot("branches_pull_request", &t);
}

#[test]
fn each_branch_tells_its_upstream_pull_request_and_commits_in_words() {
    let mut tab = branches_tab();
    let app = &mut tab.app;
    select_branch(app, "feat/ahead");
    let t = text(app);
    for needle in [
        "upstream      origin/feat/ahead · 2 commits ahead",
        "pull request  #14 feat: count ahead of the remote",
        "open · into main · 3 of 3 checks passed ✓",
        "3 commits not on main",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    select_branch(app, "local-only");
    let t = text(app);
    for needle in [
        "upstream      none",
        "pull request  #9 docs: notes for the next release",
        "open · into release/1.x · 0 of 2 checks passed …",
        "3 commits not on main",
        " docs: note 3",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    select_branch(app, "merged");
    let t = text(app);
    for needle in [
        "pull request  #8 feat: the try that landed",
        "merged · into main · 1 of 1 checks passed ✓",
        "no commits that main does not have",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    assert!(!t.contains("current branch"), "{t}");
}

#[test]
fn a_branch_in_another_worktree_names_where_it_is_checked_out() {
    let remoted = branches_repo();
    let elsewhere = remoted.tmp.path().join("elsewhere");
    git(
        &remoted.root,
        &[
            "worktree",
            "add",
            "-q",
            elsewhere.to_str().unwrap(),
            "merged",
        ],
    );
    let state = TempDir::new("branches-elsewhere-state");
    let mut app = loaded(git_options(&remoted.root, &state, View::Branches, no_gh()));
    select_branch(&mut app, "merged");
    let t = text(&mut app);
    let heading = t
        .lines()
        .find(|l| l.contains("merged    checked out in "))
        .unwrap_or_else(|| panic!("the worktree is not named\n{t}"));
    assert!(heading.ends_with("/elsewhere"), "{heading}");
    assert!(t.contains(" local · 5") && t.contains(" ● main"), "{t}");
}

#[test]
fn branches_selection_moves_and_stops_at_both_ends() {
    let mut tab = branches_tab();
    let app = &mut tab.app;
    assert_eq!(on_branch(app), "main");
    assert_eq!(key(app, KeyCode::Down), Action::Redraw);
    assert_eq!(on_branch(app), "merged");
    assert_eq!(key(app, KeyCode::Char('j')), Action::Redraw);
    assert_eq!(on_branch(app), "merged", "the last branch stays selected");
    key(app, KeyCode::Char('g'));
    assert_eq!(on_branch(app), "feat/ahead");
    key(app, KeyCode::Up);
    assert_eq!(on_branch(app), "feat/ahead");
    key(app, KeyCode::Char('j'));
    key(app, KeyCode::Char('j'));
    key(app, KeyCode::Char('k'));
    assert_eq!(on_branch(app), "feat/both");
    key(app, KeyCode::Char('G'));
    assert_eq!(on_branch(app), "merged");
    ctrl(app, 'u');
    assert_eq!(on_branch(app), "feat/ahead");
    ctrl(app, 'd');
    assert_eq!(on_branch(app), "merged");
    key(app, KeyCode::Home);
    assert_eq!(on_branch(app), "feat/ahead");
    key(app, KeyCode::End);
    assert_eq!(on_branch(app), "merged");
    key(app, KeyCode::PageUp);
    assert_eq!(on_branch(app), "feat/ahead");
    key(app, KeyCode::PageDown);
    assert_eq!(on_branch(app), "merged");
    assert!(
        text(app).contains("no commits that main does not have"),
        "the detail follows the selection"
    );
    assert_eq!(key(app, KeyCode::Tab), Action::Redraw);
    assert_eq!(on_branch(app), "merged");
    assert_eq!(key(app, KeyCode::Char('s')), Action::None);
    assert_eq!(key(app, KeyCode::Esc), Action::Quit);
}

#[test]
fn a_long_list_scrolls_with_the_selection_and_keeps_the_remotes_in_view() {
    let remoted = repo_with_origin();
    let p = remoted.root.as_path();
    for n in 0..40 {
        git(p, &["branch", "-q", &format!("topic/{n:02}")]);
    }
    let state = TempDir::new("branches-long-state");
    let mut app = loaded(git_options(p, &state, View::Branches, no_gh()));
    let frame = |app: &mut App| ui::buffer_text(&draw(app, 120, 16));
    let t = frame(&mut app);
    assert!(t.contains(" local · 41") && t.contains("topic/00"), "{t}");
    assert!(t.contains(" origin · 1 branch"), "{t}");
    key(&mut app, KeyCode::Char('G'));
    assert_eq!(on_branch(&app), "topic/39");
    let t = frame(&mut app);
    assert!(t.contains("topic/39") && !t.contains("local · 41"), "{t}");
    assert!(t.contains(" origin · 1 branch"), "{t}");
    key(&mut app, KeyCode::Char('g'));
    let t = frame(&mut app);
    assert!(t.contains(" local · 41") && t.contains(" ● main"), "{t}");
}

#[test]
fn branches_loading_state() {
    let remoted = branches_repo();
    let state = TempDir::new("branches-loading-state");
    let mut app = App::new(git_options(&remoted.root, &state, View::Branches, no_gh()));
    let t = text(&mut app);
    assert!(t.contains(" status   review   history   branches "), "{t}");
    assert!(t.contains("reading branches…"), "{t}");
    assert!(!t.contains("local ·"), "{t}");
    for code in [
        KeyCode::Char('j'),
        KeyCode::Enter,
        KeyCode::Char('n'),
        KeyCode::Char('m'),
        KeyCode::Char('D'),
    ] {
        assert_eq!(key(&mut app, code), Action::Redraw);
        assert!(app.confirm.is_none() && app.input.is_none() && app.message.is_none());
    }
    assert_eq!(key(&mut app, KeyCode::Char('y')), Action::None);
    assert_eq!(key(&mut app, KeyCode::Char('B')), Action::Redraw);
    assert_eq!(app.message.as_deref(), Some("no branch is selected"));
    app.load_blocking();
    assert!(text(&mut app).contains(" local · 5"));
}

#[test]
fn branches_not_a_git_repository_state() {
    let plain = TempDir::new("branches-plain");
    let state = TempDir::new("branches-plain-state");
    let bin = TempDir::new("branches-plain-gh");
    write(plain.path(), "tool.py", "def run():\n    return 1\n");
    let gh = fake_gh(bin.path(), "touch \"$(dirname \"$0\")/ran\"\necho '[]'");
    let mut app = loaded(git_options(plain.path(), &state, View::Branches, gh));
    let t = text(&mut app);
    assert!(t.contains("not a git repository · "), "{t}");
    assert!(!t.contains("local ·"), "{t}");
    for code in [
        KeyCode::Enter,
        KeyCode::Char('n'),
        KeyCode::Char('m'),
        KeyCode::Char('D'),
    ] {
        key(&mut app, code);
        assert!(app.confirm.is_none() && app.input.is_none());
    }
    assert!(!bin.path().join("ran").exists(), "gh has nothing to read");
    assert!(!plain.path().join(".git").exists());
}

#[test]
fn branches_of_a_repository_with_no_commits() {
    let unborn = TempDir::new("branches-unborn");
    let state = TempDir::new("branches-unborn-state");
    git(unborn.path(), &["init", "-q"]);
    write(unborn.path(), "first.py", "x = 1\n");
    let mut app = loaded(git_options(unborn.path(), &state, View::Branches, no_gh()));
    let t = text(&mut app);
    for needle in [" local · 0", "no commits yet", "n new branch  ? keys"] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    for code in [KeyCode::Enter, KeyCode::Char('m'), KeyCode::Char('D')] {
        assert_eq!(key(&mut app, code), Action::Redraw);
        assert!(app.confirm.is_none() && app.message.is_none());
    }
    assert_eq!(key(&mut app, KeyCode::Char('y')), Action::None);
    key(&mut app, KeyCode::Char('n'));
    chars(&mut app, "trunk");
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        app.message.as_deref(),
        Some("created trunk from main and switched to it")
    );
    let t = text(&mut app);
    assert!(t.lines().next().unwrap().ends_with("   trunk"), "{t}");
    assert!(
        t.contains(" local · 0"),
        "a branch is born with its first commit"
    );
    assert_eq!(
        git(unborn.path(), &["symbolic-ref", "HEAD"]).trim(),
        "refs/heads/trunk"
    );
}

#[test]
fn branches_of_a_repository_with_one_branch_and_no_remote() {
    let repo = repo_with(&[("a.py", "x = 1\n")]);
    let state = TempDir::new("branches-single-state");
    let bin = TempDir::new("branches-single-gh");
    let gh = fake_gh(bin.path(), "touch \"$(dirname \"$0\")/ran\"\necho '[]'");
    let mut app = loaded(git_options(repo.path(), &state, View::Branches, gh));
    let t = text(&mut app);
    for needle in [
        " local · 1",
        " ● main  no upstream",
        "main    current branch",
        "upstream      none",
        "no pull request information · this repository has no remote",
        "main is the default branch",
        "n new  B GitHub  y copy name  ? keys",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    assert!(
        !t.contains("branches\n") && !t.contains(" · 0 branches"),
        "{t}"
    );
    assert!(
        !bin.path().join("ran").exists(),
        "gh is not run without a remote"
    );
    snapshot("branches_one_branch", &t);
    assert_eq!(key(&mut app, KeyCode::Enter), Action::Redraw);
    assert_eq!(app.message.as_deref(), Some("already on main"));
    key(&mut app, KeyCode::Char('m'));
    assert_eq!(
        app.message.as_deref(),
        Some("main is the current branch · a branch does not merge into itself")
    );
    assert_eq!(key(&mut app, KeyCode::Char('B')), Action::Redraw);
    assert_eq!(
        app.message.as_deref(),
        Some("this repository has no origin remote · nothing to open")
    );
}

#[test]
fn a_repository_without_main_or_master_has_no_default_branch_to_compare_with() {
    let repo = repo_with(&[("a.py", "x = 1\n")]);
    git(repo.path(), &["branch", "-q", "-m", "develop"]);
    let state = TempDir::new("branches-develop-state");
    let mut app = loaded(git_options(repo.path(), &state, View::Branches, no_gh()));
    let t = text(&mut app);
    assert!(t.contains(" ● develop"), "{t}");
    assert!(t.contains("no default branch to compare with"), "{t}");
}

#[test]
fn a_branch_read_that_fails_shows_what_git_said() {
    let mut tab = branches_tab();
    let app = &mut tab.app;
    app.branches
        .load(Err("unknown field name: worktreepath".into()), None);
    let t = text(app);
    assert!(t.contains("git: unknown field name: worktreepath"), "{t}");
    assert!(!t.contains("local ·"), "{t}");
    for code in [KeyCode::Enter, KeyCode::Char('m'), KeyCode::Char('D')] {
        key(app, code);
        assert!(app.confirm.is_none());
    }
}

#[test]
fn a_read_asked_for_before_a_newer_one_is_dropped() {
    use codemap::app::Msg;
    use codemap::branches::Ahead;
    use codemap::git::branch::Unique;

    let mut tab = branches_tab();
    let app = &mut tab.app;
    select_branch(app, "feat/both");
    let newest = app.branches.reads;
    app.apply(Msg::Branches {
        asked_after: newest - 1,
        read: Ok(Default::default()),
    });
    assert_eq!(app.branches.local.len(), 5, "the older answer is dropped");
    app.apply(Msg::Ahead {
        read: newest - 1,
        branch: "feat/both".into(),
        found: Ok(Unique::default()),
    });
    assert!(
        matches!(&app.branches.ahead["feat/both"], Ahead::Known(unique) if unique.total == 3),
        "and so are commits counted against an older list"
    );
    app.apply(Msg::Branches {
        asked_after: newest,
        read: Ok(Default::default()),
    });
    assert!(app.branches.local.is_empty());
    assert!(text(app).contains("no local branches"));
}

#[test]
fn enter_switches_to_the_selected_branch_and_reads_everything_again() {
    let mut tab = branches_tab();
    let p = tab.remoted.root.clone();
    let app = &mut tab.app;
    write(&p, "scratch.md", "not committed\n");
    select_branch(app, "feat/ahead");
    assert_eq!(key(app, KeyCode::Enter), Action::Redraw);
    assert!(app.confirm.is_none(), "an idle agent is not asked about");
    assert_eq!(app.message.as_deref(), Some("switched to feat/ahead"));
    assert_eq!(checked_out(&p), "feat/ahead");
    let t = text(app);
    let header = t.lines().next().unwrap();
    assert!(
        header.contains("paneMorph · selector-fix   feat/ahead"),
        "{header}"
    );
    for needle in [
        " ● feat/ahead  ↑2",
        "   main ",
        "feat/ahead    current branch",
        "switched to feat/ahead",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    assert_eq!(on_branch(app), "feat/ahead");
    assert_eq!(
        app.repo.as_ref().unwrap().branch.as_deref(),
        Some("feat/ahead")
    );
    assert_eq!(
        app.repo.as_ref().unwrap().head.as_deref(),
        Some(commit_at(&p, "feat/ahead").as_str())
    );
    assert_eq!(app.status.unstaged.len(), 1, "status is read again");
    assert!(
        app.diffs.iter().any(|d| d.path == "scratch.md"),
        "the review diff is read again"
    );
    let head = app.history.commits.iter().find(|c| c.is_head()).unwrap();
    assert_eq!(
        head.subject, "feat: count to three",
        "history is read again"
    );

    assert_eq!(key(app, KeyCode::Enter), Action::Redraw);
    assert_eq!(app.message.as_deref(), Some("already on feat/ahead"));
    select_branch(app, "main");
    key(app, KeyCode::Enter);
    assert_eq!(checked_out(&p), "main");
    assert!(text(app).contains(" ● main"));
}

#[test]
fn a_switch_git_refuses_shows_what_git_said_and_changes_nothing() {
    let mut tab = branches_tab();
    let p = tab.remoted.root.clone();
    let app = &mut tab.app;
    write(&p, "notes.md", "mine, not committed\n");
    let elsewhere = tab.remoted.tmp.path().join("elsewhere");
    git(
        &p,
        &[
            "worktree",
            "add",
            "-q",
            elsewhere.to_str().unwrap(),
            "merged",
        ],
    );
    let before = (fingerprint(&p), tree_bytes(&p));
    select_branch(app, "local-only");
    assert_eq!(key(app, KeyCode::Enter), Action::Redraw);
    let said = app.message.clone().unwrap();
    assert!(said.contains("notes.md"), "git names the file: {said}");
    assert!(!said.contains('\n') && !said.contains('\t'), "{said:?}");
    assert_eq!(on_branch(app), "local-only");
    assert!(text(app).contains(" ● main"));

    select_branch(app, "merged");
    key(app, KeyCode::Enter);
    let said = app.message.clone().unwrap();
    assert!(
        said.contains("merged") && said.contains(elsewhere.to_str().unwrap()),
        "git names the worktree: {said}"
    );
    assert_eq!(checked_out(&p), "main");
    assert_eq!(before, (fingerprint(&p), tree_bytes(&p)));
    assert!(
        text(app).contains("merged    checked out in "),
        "the list is read again"
    );
}

#[test]
fn enter_asks_first_when_the_agent_is_working_in_this_worktree() {
    let mut tab = branches_tab_over(working_context());
    let p = tab.remoted.root.clone();
    let app = &mut tab.app;
    assert!(text(app).contains("● paneMorph · selector-fix"));
    select_branch(app, "feat/ahead");
    assert_eq!(key(app, KeyCode::Enter), Action::Redraw);
    assert!(app.confirm.is_some());
    assert_eq!(
        asked(app, "switch"),
        [
            "switch to feat/ahead while the agent is working?",
            "the agent is working here: switching moves the files under it",
            "y switches · any other key keeps"
        ]
    );
    let t = text(app);
    assert!(!t.contains("⏎ switch"), "the hints under the box are off");
    snapshot("branches_switch_question", &t);
    assert_eq!(checked_out(&p), "main", "asking changes nothing");
    assert_eq!(key(app, KeyCode::Char('n')), Action::Redraw);
    assert!(app.confirm.is_none() && app.input.is_none());
    assert_eq!(app.message.as_deref(), Some("nothing switched"));
    assert_eq!(checked_out(&p), "main");

    key(app, KeyCode::Enter);
    assert_eq!(key(app, KeyCode::Char('y')), Action::Redraw);
    assert_eq!(app.message.as_deref(), Some("switched to feat/ahead"));
    assert_eq!(checked_out(&p), "feat/ahead");

    select_branch(app, "local-only");
    key(app, KeyCode::Char('m'));
    assert_eq!(
        asked(app, "merge"),
        [
            "merge local-only into feat/ahead?",
            "the agent is working here: merging changes the files under it",
            "y merges · any other key keeps"
        ]
    );
    key(app, KeyCode::Esc);
    assert_eq!(app.message.as_deref(), Some("nothing merged"));
}

#[test]
fn n_names_a_new_branch_from_head_and_switches_to_it() {
    let mut tab = branches_tab();
    let p = tab.remoted.root.clone();
    let app = &mut tab.app;
    write(&p, "app.py", "x = 2\n");
    assert_eq!(key(app, KeyCode::Char('n')), Action::Redraw);
    assert_eq!(app.input.as_ref().unwrap().kind, InputKind::BranchName);
    let t = text(app);
    assert!(
        t.contains("new branch from main › █   ⏎ create and switch  ⎋ cancel"),
        "{t}"
    );
    chars(app, "fix/query-y1?");
    key(app, KeyCode::Backspace);
    assert!(app.input.is_some() && !app.show_help && app.outcome.is_none());
    assert_eq!(
        app.view,
        View::Branches,
        "typed digits are part of the name"
    );
    let t = text(app);
    assert!(t.contains("new branch from main › fix/query-y1█"), "{t}");
    snapshot("branches_new_branch", &t);
    assert_eq!(local_branches(&p).len(), 5, "typing creates nothing");
    assert_eq!(key(app, KeyCode::Enter), Action::Redraw);
    assert!(app.input.is_none());
    assert_eq!(
        app.message.as_deref(),
        Some("created fix/query-y1 from main and switched to it")
    );
    assert_eq!(checked_out(&p), "fix/query-y1");
    assert_eq!(commit_at(&p, "fix/query-y1"), commit_at(&p, "main"));
    assert_eq!(on_disk(&p, "app.py"), "x = 2\n");
    assert_eq!(on_branch(app), "fix/query-y1");
    let t = text(app);
    for needle in [
        " local · 6",
        " ● fix/query-y1  no upstream",
        "fix/query-y1    current branch",
        "no commits that main does not have",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    assert!(t.lines().next().unwrap().contains("fix/query-y1"), "{t}");

    key(app, KeyCode::Char('n'));
    assert!(text(app).contains("new branch from fix/query-y1 › █"));
    key(app, KeyCode::Esc);
}

#[test]
fn escape_or_a_name_git_would_not_take_creates_nothing() {
    let mut tab = branches_tab();
    let p = tab.remoted.root.clone();
    let app = &mut tab.app;
    let before = (fingerprint(&p), tree_bytes(&p));
    key(app, KeyCode::Char('n'));
    chars(app, "half-typed");
    assert_eq!(key(app, KeyCode::Esc), Action::Redraw);
    assert!(app.input.is_none() && app.message.is_none() && app.outcome.is_none());

    let refused = |app: &mut App, typed: &str| -> String {
        key(app, KeyCode::Char('n'));
        chars(app, typed);
        assert_eq!(key(app, KeyCode::Enter), Action::Redraw);
        assert!(app.input.is_none());
        app.message.clone().unwrap()
    };
    assert_eq!(
        refused(app, "two words"),
        "\"two words\" is not a valid branch name: nothing created"
    );
    assert!(text(app).contains("\"two words\" is not a valid branch name"));
    assert_eq!(
        refused(app, "local-only"),
        "a branch named local-only already exists: nothing created"
    );
    assert_eq!(
        refused(app, "-D"),
        "\"-D\" is not a valid branch name: nothing created"
    );
    assert_eq!(
        refused(app, "@{-1}"),
        "\"@{-1}\" is not a valid branch name: nothing created"
    );
    assert_eq!(refused(app, ""), "a branch needs a name · nothing created");
    assert_eq!(
        refused(app, "   "),
        "a branch needs a name · nothing created"
    );

    key(app, KeyCode::Char('n'));
    assert_eq!(keys::paste(app, "feat/pasted\r\n"), Action::Redraw);
    assert_eq!(app.input.as_ref().unwrap().text, "feat/pasted");
    assert!(app.input.is_some(), "a pasted line break creates nothing");
    ctrl(app, 'u');
    assert_eq!(app.input.as_ref().unwrap().text, "");
    key(app, KeyCode::Esc);

    assert_eq!(before, (fingerprint(&p), tree_bytes(&p)));
    assert_eq!(local_branches(&p).len(), 5);
    assert_eq!(checked_out(&p), "main");
    assert_eq!(on_branch(app), "main");
}

#[test]
fn m_asks_then_merges_the_selected_branch_into_the_current_one() {
    let mut tab = branches_tab();
    let p = tab.remoted.root.clone();
    let app = &mut tab.app;
    select_branch(app, "local-only");
    let main_before = commit_at(&p, "main");
    assert_eq!(key(app, KeyCode::Char('m')), Action::Redraw);
    assert_eq!(
        asked(app, "merge"),
        [
            "merge local-only into main?",
            "y merges · any other key keeps"
        ]
    );
    snapshot("branches_merge_question", &text(app));
    assert_eq!(commit_at(&p, "main"), main_before, "asking changes nothing");
    assert_eq!(key(app, KeyCode::Char('y')), Action::Redraw);
    assert!(app.confirm.is_none());
    assert_eq!(
        app.message.as_deref(),
        Some("fast-forwarded main to local-only")
    );
    assert_eq!(commit_at(&p, "main"), commit_at(&p, "local-only"));
    assert_eq!(checked_out(&p), "main");
    assert_eq!(on_branch(app), "local-only");
    let t = text(app);
    assert!(t.contains("no commits that main does not have"), "{t}");
    assert!(t.contains(" ● main        ↑3"), "main is ahead now\n{t}");

    select_branch(app, "feat/ahead");
    key(app, KeyCode::Char('m'));
    key(app, KeyCode::Char('y'));
    assert_eq!(app.message.as_deref(), Some("merged feat/ahead into main"));
    assert_eq!(
        git(&p, &["log", "-1", "--format=%s"]).trim(),
        "Merge branch 'feat/ahead'"
    );
    assert_eq!(
        git(&p, &["log", "-1", "--format=%P"])
            .split_whitespace()
            .count(),
        2
    );
    assert_eq!(fingerprint(&p).1, "", "nothing is left to commit");
    let head = app.history.commits.iter().find(|c| c.is_head()).unwrap();
    assert_eq!(head.subject, "Merge branch 'feat/ahead'");

    key(app, KeyCode::Char('m'));
    key(app, KeyCode::Char('y'));
    assert_eq!(
        app.message.as_deref(),
        Some("main already has everything on feat/ahead")
    );

    select_branch(app, "main");
    let before = (fingerprint(&p), tree_bytes(&p));
    assert_eq!(key(app, KeyCode::Char('m')), Action::Redraw);
    assert!(app.confirm.is_none());
    assert_eq!(
        app.message.as_deref(),
        Some("main is the current branch · a branch does not merge into itself")
    );
    key(app, KeyCode::Char('y'));
    assert_eq!(before, (fingerprint(&p), tree_bytes(&p)));
}

#[test]
fn a_merge_that_stops_on_conflicts_is_left_for_the_status_tab() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    commit_file(p, "notes.md", "the note on main\n", "docs: a note on main");
    let state = TempDir::new("branches-conflict-state");
    let mut app = loaded(git_options(p, &state, View::Branches, no_gh()));
    let ours = commit_at(p, "main");
    select_branch(&mut app, "local-only");
    key(&mut app, KeyCode::Char('m'));
    assert_eq!(key(&mut app, KeyCode::Char('y')), Action::Redraw);
    assert_eq!(
        app.message.as_deref(),
        Some("the merge of local-only stopped on conflicts in 1 file · the Status tab (1) lists them")
    );
    assert_eq!(commit_at(p, "main"), ours, "no commit was made");
    assert_eq!(commit_at(p, "MERGE_HEAD"), commit_at(p, "local-only"));
    assert_eq!(fingerprint(p).1, "AA notes.md\n", "nothing was resolved");
    assert!(on_disk(p, "notes.md").contains("<<<<<<<"));

    key(&mut app, KeyCode::Char('1'));
    assert_eq!(app.view, View::Status);
    let t = text(&mut app);
    assert!(
        t.contains(" not staged · 1") && t.contains("notes.md"),
        "{t}"
    );
    assert!(t.contains("<<<<<<<"), "{t}");

    key(&mut app, KeyCode::Char('4'));
    select_branch(&mut app, "feat/ahead");
    key(&mut app, KeyCode::Char('m'));
    key(&mut app, KeyCode::Char('y'));
    let said = app.message.clone().unwrap();
    assert!(
        !said.contains("stopped on conflicts") && !said.is_empty(),
        "a second merge is refused in git's own words: {said}"
    );
    assert_eq!(fingerprint(p).1, "AA notes.md\n");
}

#[test]
fn a_merge_a_hook_refuses_to_commit_says_it_is_not_finished() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    commit_file(p, "app.py", "x = 2\n", "fix: main moves on");
    hook(
        p,
        "pre-merge-commit",
        "#!/bin/sh\necho 'lint: merges need a ticket' >&2\nexit 1\n",
    );
    let state = TempDir::new("branches-hook-state");
    let mut app = loaded(git_options(p, &state, View::Branches, no_gh()));
    select_branch(&mut app, "local-only");
    key(&mut app, KeyCode::Char('m'));
    key(&mut app, KeyCode::Char('y'));
    let said = app.message.clone().unwrap();
    assert!(
        said.starts_with(
            "the merge of local-only is not committed · the Status tab (1) shows it · "
        ) && said.contains("lint: merges need a ticket"),
        "{said}"
    );
    assert_eq!(fingerprint(p).1, "A  notes.md\n");
    assert_eq!(app.status.staged.len(), 1, "status is read again");
}

#[test]
fn shift_d_says_what_would_be_lost_before_it_deletes() {
    let mut tab = branches_tab();
    let p = tab.remoted.root.clone();
    let app = &mut tab.app;
    select_branch(app, "merged");
    assert_eq!(key(app, KeyCode::Char('D')), Action::Redraw);
    assert_eq!(
        asked(app, "delete"),
        [
            "delete the branch merged?",
            "y deletes · any other key keeps"
        ]
    );
    assert_eq!(local_branches(&p).len(), 5, "asking deletes nothing");
    let tip = commit_at(&p, "merged");
    assert_eq!(key(app, KeyCode::Char('y')), Action::Redraw);
    assert_eq!(
        app.message.clone().unwrap(),
        format!("deleted merged (was {})", &tip[..7])
    );
    assert_eq!(
        local_branches(&p),
        ["feat/ahead", "feat/both", "local-only", "main"]
    );
    assert_eq!(on_branch(app), "main", "the row the deleted branch was on");
    let t = text(app);
    assert!(t.contains(" local · 4") && !t.contains("merged  "), "{t}");

    select_branch(app, "local-only");
    key(app, KeyCode::Char('D'));
    assert_eq!(
        asked(app, "delete"),
        [
            "delete the branch local-only?",
            "3 commits exist only on this branch and would become unreachable",
            "y deletes · any other key keeps"
        ]
    );
    let t = text(app);
    snapshot("branches_delete_question", &t);
    let buf = draw(app, W, H);
    let warned = (0..H).any(|y| {
        let row: String = (0..W).map(|x| buf[(x, y)].symbol().to_string()).collect();
        let at = row
            .find("3 commits exist only")
            .map(|byte| row[..byte].chars().count());
        at.is_some_and(|x| buf[(x as u16, y)].fg == dracula().del)
    });
    assert!(warned, "what would be lost is in the colour of a deletion");
    assert_eq!(key(app, KeyCode::Esc), Action::Redraw);
    assert_eq!(app.message.as_deref(), Some("nothing deleted"));
    assert_eq!(local_branches(&p).len(), 4);

    key(app, KeyCode::Char('D'));
    key(app, KeyCode::Char('y'));
    assert_eq!(local_branches(&p), ["feat/ahead", "feat/both", "main"]);
    assert_eq!(on_branch(app), "main");
    assert_eq!(checked_out(&p), "main");

    select_branch(app, "feat/ahead");
    key(app, KeyCode::Char('D'));
    assert_eq!(
        asked(app, "delete")[1],
        "2 commits exist only on this branch and would become unreachable",
        "the one that was pushed is still on origin/feat/ahead"
    );
    key(app, KeyCode::Char('q'));
    assert!(app.outcome.is_none());
    assert_eq!(
        git(
            &tab.remoted.origin,
            &["branch", "--format=%(refname:short)"]
        ),
        "feat/ahead\nfeat/both\nmain\n",
        "nothing is deleted on the remote"
    );
}

#[test]
fn the_current_branch_and_one_in_another_worktree_are_not_deleted() {
    let mut tab = branches_tab();
    let p = tab.remoted.root.clone();
    let app = &mut tab.app;
    let elsewhere = tab.remoted.tmp.path().join("elsewhere");
    git(
        &p,
        &[
            "worktree",
            "add",
            "-q",
            elsewhere.to_str().unwrap(),
            "merged",
        ],
    );
    let before = (fingerprint(&p), tree_bytes(&p));
    assert_eq!(on_branch(app), "main");
    assert_eq!(key(app, KeyCode::Char('D')), Action::Redraw);
    assert!(app.confirm.is_none(), "there is nothing to ask");
    assert_eq!(
        app.message.as_deref(),
        Some("main is the current branch: switch to another one before deleting it")
    );
    assert_eq!(key(app, KeyCode::Char('y')), Action::Copy("main".into()));

    select_branch(app, "merged");
    key(app, KeyCode::Char('D'));
    assert!(app.confirm.is_none());
    let said = app.message.clone().unwrap();
    assert!(
        said.starts_with("merged is checked out in ") && said.ends_with(": nothing deleted"),
        "{said}"
    );
    key(app, KeyCode::Char('y'));
    assert_eq!(before, (fingerprint(&p), tree_bytes(&p)));
    assert_eq!(local_branches(&p).len(), 5);
}

#[test]
fn a_branch_that_moved_after_the_question_is_not_deleted() {
    let mut tab = branches_tab();
    let p = tab.remoted.root.clone();
    let app = &mut tab.app;
    select_branch(app, "merged");
    key(app, KeyCode::Char('D'));
    assert_eq!(asked(app, "delete").len(), 2, "nothing would be lost");
    git(&p, &["branch", "-q", "-f", "merged", "local-only"]);
    git(&p, &["branch", "-q", "-D", "local-only"]);
    key(app, KeyCode::Char('y'));
    assert_eq!(
        app.message.as_deref(),
        Some("merged changed since the question was asked: nothing deleted")
    );
    assert!(local_branches(&p).contains(&"merged".to_string()));
    key(app, KeyCode::Char('D'));
    assert_eq!(
        asked(app, "delete")[1],
        "3 commits exist only on this branch and would become unreachable"
    );
}

#[test]
fn only_y_answers_a_branch_question() {
    let mut tab = branches_tab_over(working_context());
    let p = tab.remoted.root.clone();
    let app = &mut tab.app;
    let before = (fingerprint(&p), tree_bytes(&p));
    for (opens, declined) in [
        (KeyCode::Enter, "nothing switched"),
        (KeyCode::Char('m'), "nothing merged"),
        (KeyCode::Char('D'), "nothing deleted"),
    ] {
        for other in keys_other_than_a_plain_y() {
            select_branch(app, "local-only");
            key(app, opens);
            assert!(app.confirm.is_some(), "{opens:?}");
            assert_eq!(keys::handle(app, other), Action::Redraw, "{other:?}");
            assert!(app.confirm.is_none(), "{other:?} answers the question");
            assert_eq!(app.message.as_deref(), Some(declined), "{other:?}");
            assert!(app.outcome.is_none() && !app.show_help && app.input.is_none());
            assert_eq!(app.view, View::Branches);
            assert_eq!(on_branch(app), "local-only", "{other:?}");
        }
        key(app, opens);
        assert_eq!(keys::paste(app, "y"), Action::Redraw);
        assert!(app.confirm.is_none(), "pasted text is not an answer");
        assert_eq!(app.message.as_deref(), Some(declined));
    }
    assert_eq!(keys::paste(app, "\rmyDyn\n"), Action::None);
    assert!(app.confirm.is_none() && app.input.is_none());
    assert_eq!(before, (fingerprint(&p), tree_bytes(&p)));
    assert_eq!(checked_out(&p), "main");
}

#[test]
fn shift_b_opens_the_pull_request_or_the_branch_on_github() {
    let mut tab = branches_tab();
    let p = tab.remoted.root.clone();
    let app = &mut tab.app;
    select_branch(app, "feat/both");
    assert_eq!(
        key(app, KeyCode::Char('B')),
        Action::Browse("https://github.com/acme/widgets/pull/12".into())
    );
    select_branch(app, "main");
    assert_eq!(key(app, KeyCode::Char('B')), Action::Redraw);
    assert_eq!(
        app.message.as_deref(),
        Some("origin is not on GitHub · nothing to open")
    );
    git(
        &p,
        &[
            "remote",
            "set-url",
            "origin",
            "git@github.com:acme/widgets.git",
        ],
    );
    assert_eq!(
        key(app, KeyCode::Char('B')),
        Action::Browse("https://github.com/acme/widgets/tree/main".into())
    );
    select_branch(app, "feat/ahead");
    assert_eq!(
        key(app, KeyCode::Char('B')),
        Action::Browse("https://github.com/acme/widgets/pull/14".into()),
        "the pull request comes first"
    );

    assert_eq!(
        key(app, KeyCode::Char('y')),
        Action::Copy("feat/ahead".into())
    );
    for nothing_to_open in ['e', 'E', 'b'] {
        assert_eq!(key(app, KeyCode::Char(nothing_to_open)), Action::None);
    }
    assert_eq!(app.location(), None);
}

#[test]
fn a_branch_without_a_pull_request_opens_under_its_name_on_origin() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    git(p, &["branch", "-q", "-m", "feat/ahead", "counting"]);
    git(
        p,
        &[
            "remote",
            "set-url",
            "origin",
            "https://github.com/acme/widgets",
        ],
    );
    let state = TempDir::new("branches-link-state");
    let mut app = loaded(git_options(p, &state, View::Branches, no_gh()));
    select_branch(&mut app, "counting");
    assert!(text(&mut app).contains("upstream      origin/feat/ahead · 2 commits ahead"));
    assert_eq!(
        key(&mut app, KeyCode::Char('B')),
        Action::Browse("https://github.com/acme/widgets/tree/feat/ahead".into())
    );
    select_branch(&mut app, "local-only");
    assert_eq!(
        key(&mut app, KeyCode::Char('B')),
        Action::Browse("https://github.com/acme/widgets/tree/local-only".into())
    );
}

fn counting_gh(bin: &std::path::Path) -> codemap::gh::Gh {
    write(bin, "pull-requests.json", &pull_requests_fixture());
    fake_gh(
        bin,
        "here=$(dirname \"$0\")\necho run >> \"$here/runs\"\nexec cat \"$here/pull-requests.json\"",
    )
}

fn runs(bin: &std::path::Path) -> usize {
    std::fs::read_to_string(bin.join("runs"))
        .map(|log| log.lines().count())
        .unwrap_or(0)
}

#[test]
fn gh_runs_once_and_only_when_the_branches_tab_is_first_shown() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    let state = TempDir::new("branches-lazy-state");
    let bin = TempDir::new("branches-lazy-gh");
    let gh = counting_gh(bin.path());
    for view in [
        View::Files,
        View::Custom,
        View::Status,
        View::Changes,
        View::History,
    ] {
        let mut app = loaded(git_options(p, &state, view, gh.clone()));
        text(&mut app);
        assert_eq!(runs(bin.path()), 0, "{view:?} does not ask gh");
        assert_eq!(app.branches.pulls, codemap::branches::Pulls::NotAsked);
    }
    let mut app = loaded(git_options(p, &state, View::Status, gh.clone()));
    assert!(app.branches.loaded, "the branches are read with the popup");
    for tab in ['2', '3', '1'] {
        key(&mut app, KeyCode::Char(tab));
        text(&mut app);
    }
    assert_eq!(runs(bin.path()), 0);
    key(&mut app, KeyCode::Char('4'));
    assert_eq!(runs(bin.path()), 1);
    let t = text(&mut app);
    assert!(
        t.contains("#14 ✓") && t.contains("pull request  none"),
        "{t}"
    );
    for tab in ['1', '4', '3', '4'] {
        key(&mut app, KeyCode::Char(tab));
        text(&mut app);
    }
    key(&mut app, KeyCode::Char('n'));
    chars(&mut app, "feat/another");
    key(&mut app, KeyCode::Enter);
    assert_eq!(checked_out(p), "feat/another");
    assert_eq!(runs(bin.path()), 1, "once for the popup");
    assert!(text(&mut app).contains("#14 ✓"), "and it is kept");

    let mut direct = App::new(git_options(p, &state, View::Branches, gh));
    text(&mut direct);
    assert_eq!(runs(bin.path()), 1);
    direct.load_blocking();
    assert_eq!(runs(bin.path()), 2);

    let files = loaded(git_options(p, &state, View::Files, no_gh()));
    assert!(!files.branches.loaded, "only the git popup reads branches");
}

#[test]
fn the_tab_works_without_pull_request_information() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    let state = TempDir::new("branches-no-gh-state");
    let bin = TempDir::new("branches-no-gh");
    let mut app = loaded(git_options(p, &state, View::Branches, no_gh()));
    select_branch(&mut app, "feat/both");
    let t = text(&mut app);
    for needle in [
        "   feat/both   ↑2 ↓1",
        "no pull request information · gh is not installed",
        "3 commits not on main",
        "B GitHub",
    ] {
        assert!(t.contains(needle), "missing {needle:?}\n{t}");
    }
    assert!(!t.contains('#') && !t.contains("pull request  "), "{t}");
    key(&mut app, KeyCode::Enter);
    assert_eq!(checked_out(p), "feat/both");

    let reason = |gh: codemap::gh::Gh| -> String {
        let mut app = loaded(git_options(p, &state, View::Branches, gh));
        let t = text(&mut app);
        assert!(t.contains(" local · 5"), "{t}");
        let line = t
            .lines()
            .find(|l| l.contains("no pull request information"))
            .unwrap_or_else(|| panic!("no reason on screen\n{t}"));
        line.split(" · ").last().unwrap().to_string()
    };
    assert_eq!(
        reason(fake_gh(
            bin.path(),
            "echo 'To get started with GitHub CLI, please run:  gh auth login' >&2\nexit 4"
        )),
        "gh is not signed in to GitHub"
    );
    assert_eq!(
        reason(fake_gh(
            bin.path(),
            "echo 'error connecting to api.github.com' >&2\nexit 1"
        )),
        "gh could not reach GitHub"
    );
    assert_eq!(
        reason(gh_printing(bin.path(), "<html>502 Bad Gateway</html>")),
        "gh printed something codeMap could not read"
    );
    let slow = codemap::gh::Gh {
        timeout: std::time::Duration::from_millis(200),
        ..fake_gh(bin.path(), "sleep 30\necho '[]'")
    };
    let started = std::time::Instant::now();
    assert_eq!(reason(slow), "gh did not answer within 0.2 seconds");
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}

#[test]
fn walking_the_branches_tab_writes_nothing() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    let elsewhere = remoted.tmp.path().join("elsewhere");
    git(
        p,
        &[
            "worktree",
            "add",
            "-q",
            elsewhere.to_str().unwrap(),
            "merged",
        ],
    );
    write(p, "app.py", "x = 2\n");
    git(p, &["add", "app.py"]);
    write(p, "app.py", "x = 3\n");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    write(p, "scratch.md", "not committed\n");
    let before = (fingerprint(p), tree_bytes(p));
    let state = TempDir::new("branches-walk-state");
    let bin = TempDir::new("branches-walk-gh");
    let gh = counting_gh(bin.path());
    let mut app = loaded(Options {
        context: Some(working_context()),
        ..git_options(p, &state, View::Branches, gh)
    });
    assert_eq!(runs(bin.path()), 1, "looking may ask gh, and nothing else");
    text(&mut app);
    key(&mut app, KeyCode::Char('g'));
    let mut questions = 0;
    for _ in 0..5 {
        text(&mut app);
        for opens in [KeyCode::Enter, KeyCode::Char('m'), KeyCode::Char('D')] {
            for declines in [KeyCode::Esc, KeyCode::Char('n'), KeyCode::Enter] {
                key(&mut app, opens);
                if app.confirm.is_some() {
                    questions += 1;
                    text(&mut app);
                    key(&mut app, declines);
                }
                assert!(app.confirm.is_none() && app.input.is_none());
            }
            key(&mut app, opens);
            if app.confirm.is_some() {
                keys::paste(&mut app, "y\n");
            }
            assert!(app.confirm.is_none() && app.input.is_none());
        }
        key(&mut app, KeyCode::Char('j'));
    }
    assert_eq!(
        questions, 33,
        "four to switch to, four to merge, three to delete"
    );
    ctrl(&mut app, 'u');
    ctrl(&mut app, 'd');
    for step in [
        KeyCode::Char('G'),
        KeyCode::Char('k'),
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::Tab,
    ] {
        key(&mut app, step);
        text(&mut app);
    }
    key(&mut app, KeyCode::Char('n'));
    chars(&mut app, "feat/not-yet");
    text(&mut app);
    key(&mut app, KeyCode::Esc);
    for typed in ["", "two words", "main", "@{-1}", "-D"] {
        key(&mut app, KeyCode::Char('n'));
        chars(&mut app, typed);
        key(&mut app, KeyCode::Enter);
        assert!(app.input.is_none());
    }
    key(&mut app, KeyCode::Char('n'));
    keys::paste(&mut app, "feat/pasted\n");
    key(&mut app, KeyCode::Esc);
    for held in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
        for code in [
            KeyCode::Enter,
            KeyCode::Char('n'),
            KeyCode::Char('m'),
            KeyCode::Char('D'),
        ] {
            keys::handle(&mut app, KeyEvent::new(code, held));
            assert!(
                app.confirm.is_none() && app.input.is_none() && app.message.is_none(),
                "{held:?} {code:?}"
            );
        }
    }
    keys::paste(&mut app, "\rnmDy\n");
    select_branch(&mut app, "feat/both");
    assert!(matches!(
        key(&mut app, KeyCode::Char('B')),
        Action::Browse(_)
    ));
    assert_eq!(
        key(&mut app, KeyCode::Char('y')),
        Action::Copy("feat/both".into())
    );
    for nothing in ['e', 'E', 'b'] {
        assert_eq!(key(&mut app, KeyCode::Char(nothing)), Action::None);
    }
    key(&mut app, KeyCode::Char('?'));
    text(&mut app);
    key(&mut app, KeyCode::Esc);
    for tab in ['1', '2', '3', '4'] {
        key(&mut app, KeyCode::Char(tab));
        text(&mut app);
    }
    assert_eq!(key(&mut app, KeyCode::Esc), Action::Quit);
    assert_eq!(runs(bin.path()), 1);
    let after = (fingerprint(p), tree_bytes(p));
    assert_eq!(before.0 .0, after.0 .0, ".git changed while looking");
    assert_eq!(before.0 .1, after.0 .1, "git status changed while looking");
    assert_eq!(before.1, after.1, "the working tree changed while looking");
}

#[test]
fn the_branch_questions_and_the_name_prompt_fit_small_terminals() {
    let remoted = repo_with_origin();
    let p = remoted.root.as_path();
    let long = "feat/a-very-long-branch-name-that-goes-on-and-on-and-on-and-on-and-on-for-a-while";
    git(p, &["switch", "-q", "-c", long]);
    commit_file(p, "long.py", "x = 1\n", &"a long subject ".repeat(12));
    git(p, &["switch", "-q", "main"]);
    let state = TempDir::new("branches-small-state");
    let bin = TempDir::new("branches-small-gh");
    let pull = format!(
        "[{{\"number\": 123456, \"state\": \"OPEN\", \"headRefName\": \"{long}\", \"baseRefName\": \"{long}\", \"title\": \"{}\", \"url\": \"https://github.com/acme/widgets/pull/123456\"}}]",
        "a long title ".repeat(20)
    );
    let mut app = loaded(Options {
        context: Some(working_context()),
        ..git_options(p, &state, View::Branches, gh_printing(bin.path(), &pull))
    });
    let sizes = [
        (20u16, 6u16),
        (24, 8),
        (40, 10),
        (60, 20),
        (80, 24),
        (200, 60),
    ];
    select_branch(&mut app, long);
    for opens in [KeyCode::Enter, KeyCode::Char('m'), KeyCode::Char('D')] {
        for (w, h) in sizes {
            let _ = draw(&mut app, w, h);
        }
        key(&mut app, opens);
        assert!(app.confirm.is_some());
        for (w, h) in sizes {
            let _ = draw(&mut app, w, h);
        }
        key(&mut app, KeyCode::Esc);
    }
    key(&mut app, KeyCode::Char('D'));
    let question = asked(&mut app, "delete");
    assert_eq!(question[0], "delete the branch");
    assert!(question[1].ends_with("for-a-while?"), "{question:?}");
    assert_eq!(
        question[2],
        "1 commit exists only on this branch and would become unreachable"
    );
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('n'));
    chars(&mut app, &"a-long-name-".repeat(20));
    for (w, h) in sizes {
        let _ = draw(&mut app, w, h);
    }
}
