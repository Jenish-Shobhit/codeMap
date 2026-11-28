mod common;

use std::time::Duration;

use common::pty::*;
use common::*;

fn press(p: &mut Pty, key: &[u8], expect: &str) {
    let from = p.len();
    p.send(key);
    assert!(
        p.wait_for(expect, from, Duration::from_secs(5)).is_some(),
        "after {:?} expected {expect:?}; screen:\n{}",
        String::from_utf8_lossy(key),
        p.screen()
    );
}

#[test]
fn keys_first_paint_and_escape() {
    let pm = panemorph_repo();
    let trace = pm.state.path().join("trace.txt");
    let cfg = pm.state.path().join("herdr-config.toml");
    std::fs::write(&cfg, "[theme]\nname = \"dracula\"\n").unwrap();
    let state = pm.state.path().display().to_string();
    let trace_s = trace.display().to_string();
    let cfg_s = cfg.display().to_string();
    let root = pm.root.display().to_string();
    let mut p = spawn(
        &[&root],
        &[
            ("CODEMAP_STATE_DIR", &state),
            ("CODEMAP_TRACE_FILE", &trace_s),
            ("HERDR_CONFIG_PATH", &cfg_s),
        ],
        160,
        50,
    );
    let painted = p
        .wait_for("history", 0, Duration::from_secs(5))
        .expect("first paint");
    p.wait_for("since HEAD", 0, Duration::from_secs(10))
        .expect("changes content");
    press(&mut p, b"}", "open_selector.py");
    press(&mut p, b" ", "✓ reviewed");
    press(&mut p, b"3", "tag: v0.1.0");
    press(&mut p, b"1", "not staged · 3");
    press(&mut p, b"2", "since HEAD");
    press(&mut p, b"?", "paste the draft");
    p.send(b"\x1b");
    std::thread::sleep(Duration::from_millis(150));
    assert!(
        p.child.try_wait().unwrap().is_none(),
        "Esc on help must not quit"
    );
    p.send(b"\x1b");
    let status = p
        .wait_exit(Duration::from_secs(5))
        .expect("Esc closes the popup");
    assert!(status.success());
    let out = p.text();
    assert!(out.contains("\x1b[?1049l"), "left the alternate screen");
    assert!(out.contains("\x1b[?25h"), "cursor shown again");

    let first_byte = p
        .first_byte
        .lock()
        .unwrap()
        .unwrap()
        .duration_since(p.started);
    let trace = std::fs::read_to_string(&trace).unwrap_or_default();
    let first_paint_ms: f64 = trace
        .split("first paint ")
        .nth(1)
        .and_then(|s| s.split(' ').next())
        .and_then(|s| s.parse().ok())
        .expect("trace records first paint");
    println!(
        "pty: first byte {:.1} ms, tabs visible {:.1} ms, in-process first paint {:.1} ms ({})",
        first_byte.as_secs_f64() * 1000.0,
        painted.as_secs_f64() * 1000.0,
        first_paint_ms,
        trace.trim()
    );
    assert!(first_paint_ms < 150.0, "first paint {first_paint_ms} ms");
    assert!(
        painted < Duration::from_millis(1500),
        "frame visible after {painted:?}"
    );
}

#[test]
fn files_popup_finds_a_file_and_previews_it() {
    let pm = panemorph_repo();
    let state = pm.state.path().display().to_string();
    let root = pm.root.display().to_string();
    let mut p = spawn(
        &[&root, "--view", "files"],
        &[("CODEMAP_STATE_DIR", &state)],
        150,
        44,
    );
    p.wait_for("files · 11", 0, Duration::from_secs(5))
        .expect("the tree");
    press(&mut p, b"/", "pick");
    press(&mut p, b"topology", "matches");
    press(&mut p, b"\r", "files · 11");
    press(&mut p, b"h", "10 files");
    p.send(b"q");
    assert!(p
        .wait_exit(Duration::from_secs(5))
        .expect("q quits")
        .success());
}

#[test]
fn status_tab_shows_what_is_staged_and_what_is_not() {
    let repo = repo_with(&[
        ("app.py", "def main():\n    return 1\n"),
        ("util.py", "def helper():\n    pass\n"),
        ("docs/guide.md", "# Guide\n"),
    ]);
    let root = repo.path();
    write(root, "app.py", "def main():\n    return 2\n");
    write(root, "docs/guide.md", "# Guide\n\nRun it.\n");
    git(root, &["add", "app.py", "docs/guide.md"]);
    write(root, "util.py", "def helper():\n    return 3\n");
    let before = (fingerprint(root), tree_bytes(root));
    let state = TempDir::new("pty-status-state");
    let state_s = state.path().display().to_string();
    let root_s = root.display().to_string();
    let mut p = spawn(
        &[&root_s, "--view", "status"],
        &[("CODEMAP_STATE_DIR", &state_s)],
        120,
        30,
    );
    p.wait_for("not staged · 1", 0, Duration::from_secs(5))
        .expect("the not staged group");
    let screen = p.screen();
    for needle in [
        " status   review   history ",
        " staged · 2",
        "M app.py",
        "M guide.md",
        "M util.py",
        "app.py   +1 −1    staged",
    ] {
        assert!(screen.contains(needle), "missing {needle:?}\n{screen}");
    }
    press(&mut p, b"G", "util.py   +1 −1    not staged");
    press(&mut p, b"\t", "j k scroll");
    press(&mut p, b"]", "hunk 1 of 1");
    press(
        &mut p,
        b"x",
        "discard 2 changed lines in util.py (hunk 1 of 1)?",
    );
    press(&mut p, b"\r", "nothing discarded");
    press(&mut p, b"c", "commit to main");
    press(&mut p, b"not yet", "subject  not yet");
    press(&mut p, b"\x1b", "j k scroll");
    p.send(b"\x1b");
    std::thread::sleep(Duration::from_millis(150));
    assert!(
        p.child.try_wait().unwrap().is_none(),
        "Esc in the diff goes back to the list"
    );
    press(&mut p, b"2", "since HEAD");
    p.send(b"q");
    assert!(p
        .wait_exit(Duration::from_secs(5))
        .expect("q quits")
        .success());
    let after = (fingerprint(root), tree_bytes(root));
    assert_eq!(
        before.0 .0, after.0 .0,
        ".git changed while the popup was open"
    );
    assert_eq!(before.0 .1, after.0 .1, "git status changed");
    assert_eq!(before.1, after.1, "the working tree changed");
}

#[test]
fn status_tab_stages_a_file_and_commits_it() {
    let repo = repo_with(&[
        ("app.py", "def main():\n    return 1\n"),
        ("util.py", "def helper():\n    pass\n"),
    ]);
    let root = repo.path();
    commits_as_test(root);
    write(root, "app.py", "def main():\n    return 2\n");
    write(root, "util.py", "def helper():\n    return 3\n");
    let state = TempDir::new("pty-commit-state");
    let state_s = state.path().display().to_string();
    let root_s = root.display().to_string();
    let mut p = spawn(
        &[&root_s, "--view", "status"],
        &[("CODEMAP_STATE_DIR", &state_s)],
        120,
        30,
    );
    p.wait_for("not staged · 2", 0, Duration::from_secs(5))
        .expect("the not staged group");
    assert!(p.screen().contains("app.py   +1 −1    not staged"));
    assert!(
        p.text().contains("\x1b[?2004h"),
        "the status tab asks the terminal to mark pasted text"
    );
    press(&mut p, b" ", "staged app.py");
    assert!(p.screen().contains(" staged · 1"), "{}", p.screen());
    assert_eq!(
        git(root, &["status", "--porcelain=v1"]),
        "M  app.py\n M util.py\n"
    );

    p.send(b"\x1b[200~xy\x1b[201~");
    press(&mut p, b"?", "stage or unstage the file");
    press(&mut p, b"\x1b", "j k file");
    assert_eq!(
        std::fs::read_to_string(root.join("util.py")).unwrap(),
        "def helper():\n    return 3\n",
        "a pasted x and y discard nothing"
    );

    press(&mut p, b"c", "commit to main");
    assert!(p.screen().contains("1 file staged"), "{}", p.screen());
    press(&mut p, b"fix: return two", "subject  fix: return two");
    press(&mut p, b"\r", "committed ");
    let screen = p.screen();
    assert!(!screen.contains("commit to main"), "{screen}");
    assert!(screen.contains(" staged · 0"), "{screen}");
    assert!(screen.contains(" not staged · 1"), "{screen}");
    assert_eq!(
        git(root, &["log", "--format=%an <%ae> %s"]),
        "Test <test@example.com> fix: return two\nTest <test@example.com> initial\n"
    );
    assert_eq!(
        git(root, &["show", "--format=", "--name-only", "HEAD"]),
        "app.py\n"
    );
    assert_eq!(git(root, &["status", "--porcelain=v1"]), " M util.py\n");
    let short = git(root, &["rev-parse", "--short", "HEAD"]);
    assert!(
        screen.contains(&format!("committed {} fix: return two", short.trim())),
        "{screen}"
    );
    press(&mut p, b"3", "fix: return two");
    p.send(b"q");
    assert!(p
        .wait_exit(Duration::from_secs(5))
        .expect("q quits")
        .success());
    assert!(
        p.text().contains("\x1b[?2004l"),
        "paste marking is off again"
    );
}

#[test]
fn branches_tab_creates_a_branch_switches_back_and_deletes_it() {
    let remoted = repo_with_origin();
    let root = remoted.root.as_path();
    let state = TempDir::new("pty-branches-state");
    let bin = TempDir::new("pty-branches-gh");
    let gh = gh_printing(bin.path(), "[]");
    let state_s = state.path().display().to_string();
    let root_s = root.display().to_string();
    let gh_s = gh.program.display().to_string();
    let branches = || git(root, &["branch", "--format=%(refname:short)"]);
    let current = || git(root, &["branch", "--show-current"]);
    let mut p = spawn(
        &[&root_s, "--view", "branches"],
        &[("CODEMAP_STATE_DIR", &state_s), ("CODEMAP_GH", &gh_s)],
        120,
        30,
    );
    p.wait_for("pull request  none", 0, Duration::from_secs(5))
        .expect("the branches and what the fake gh printed");
    let screen = p.screen();
    for needle in [
        " status   review   history   branches ",
        " local · 1",
        " ● main",
        " origin · 1 branch",
        "main    current branch",
        "upstream      origin/main · in step",
    ] {
        assert!(screen.contains(needle), "missing {needle:?}\n{screen}");
    }
    assert!(
        p.text().contains("\x1b[?2004h"),
        "the branches tab asks the terminal to mark pasted text"
    );

    press(&mut p, b"n", "new branch from main ›");
    press(&mut p, b"feat/typed", "› feat/typed");
    assert_eq!(branches(), "main\n", "typing creates nothing");
    press(
        &mut p,
        b"\r",
        "created feat/typed from main and switched to it",
    );
    assert_eq!(branches(), "feat/typed\nmain\n");
    assert_eq!(current(), "feat/typed\n");
    let screen = p.screen();
    assert!(screen.contains(" local · 2"), "{screen}");
    assert!(screen.contains(" ● feat/typed"), "{screen}");

    press(&mut p, b"j", "upstream      origin/main · in step");
    press(&mut p, b"\r", "switched to main");
    assert_eq!(current(), "main\n");

    press(&mut p, b"k", "upstream      none");
    press(&mut p, b"D", "delete the branch feat/typed?");
    press(&mut p, b"\x1b[200~y\x1b[201~", "nothing deleted");
    assert_eq!(
        branches(),
        "feat/typed\nmain\n",
        "a pasted y deletes nothing"
    );
    press(&mut p, b"D", "delete the branch feat/typed?");
    press(&mut p, b"\r", "nothing deleted");
    press(&mut p, b"D", "y deletes");
    press(&mut p, b"y", "deleted feat/typed");
    assert_eq!(branches(), "main\n");
    assert_eq!(current(), "main\n");
    let screen = p.screen();
    assert!(screen.contains(" local · 1"), "{screen}");
    assert!(!screen.contains("delete the branch"), "{screen}");

    press(&mut p, b"1", "nothing to commit, working tree clean");
    p.send(b"q");
    assert!(p
        .wait_exit(Duration::from_secs(5))
        .expect("q quits")
        .success());
    assert!(
        p.text().contains("\x1b[?2004l"),
        "paste marking is off again"
    );
    assert_eq!(
        git(&remoted.origin, &["branch", "--format=%(refname:short)"]),
        "main\n",
        "the remote was never written to"
    );
}

#[test]
fn q_quits_and_ctrl_c_quits() {
    let t = TempDir::new("pty-small");
    write(t.path(), "a.py", "def f():\n    return 1\n");
    let root = t.path().display().to_string();
    let state = t.path().join(".st").display().to_string();
    let mut p = spawn(&[&root], &[("CODEMAP_STATE_DIR", &state)], 100, 30);
    p.wait_for("not a git repository", 0, Duration::from_secs(5))
        .expect("a plain folder");
    p.send(b"q");
    assert!(p
        .wait_exit(Duration::from_secs(5))
        .expect("q quits")
        .success());

    let mut p = spawn(&[&root], &[("CODEMAP_STATE_DIR", &state)], 100, 30);
    p.wait_for("not a git repository", 0, Duration::from_secs(5))
        .expect("a plain folder");
    p.send(b"\x03");
    assert!(
        p.wait_exit(Duration::from_secs(5)).is_some(),
        "ctrl-c quits"
    );
}

#[test]
#[ignore]
fn smoke_own_repo() {
    let state = TempDir::new("smoke-state");
    let state_s = state.path().display().to_string();
    let root = env!("CARGO_MANIFEST_DIR");
    let trace = state.path().join("trace.txt");
    let trace_s = trace.display().to_string();
    let mut p = spawn(
        &[root],
        &[
            ("CODEMAP_STATE_DIR", &state_s),
            ("CODEMAP_TRACE_FILE", &trace_s),
        ],
        180,
        52,
    );
    p.wait_for("history", 0, Duration::from_secs(5))
        .expect("paint");
    std::thread::sleep(Duration::from_millis(1500));
    println!("{}\n", p.screen());
    for (k, name) in [(b"1", "status"), (b"3", "history"), (b"2", "review")] {
        p.send(k);
        std::thread::sleep(Duration::from_millis(700));
        println!("===== {name}\n{}\n", p.screen());
    }
    p.send(b"\x1b");
    p.wait_exit(Duration::from_secs(5));
    println!("{}", std::fs::read_to_string(&trace).unwrap_or_default());
}
