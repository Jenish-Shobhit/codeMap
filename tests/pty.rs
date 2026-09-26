//! PTY harness: runs the real binary on a pseudo-terminal the way herdr's
//! popup does, measures open-to-first-paint, drives keys and checks that Esc
//! closes it.
//!
//! `CODEMORPH_BIN=target/release/codemorph cargo test --test pty -- --nocapture`
//! measures a release build.

mod common;

use std::time::Duration;

use common::pty::*;
use common::*;

/// Text of a key press, then the view's marker text to wait for.
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
            ("CODEMORPH_STATE_DIR", &state),
            ("CODEMORPH_TRACE_FILE", &trace_s),
            ("HERDR_CONFIG_PATH", &cfg_s),
        ],
        160,
        50,
    );
    // The frame: tabs are drawn before any git or parsing work lands.
    let painted = p
        .wait_for("history", 0, Duration::from_secs(5))
        .expect("first paint");
    // Content fills in.
    p.wait_for("PaneMorphService", 0, Duration::from_secs(10))
        .expect("map content");
    // 3: changes (standalone: since HEAD)
    press(&mut p, b"3", "since HEAD");
    // } next file, space marks reviewed
    press(&mut p, b"}", "open_selector.py");
    press(&mut p, b" ", "✓ reviewed");
    // 4: history
    press(&mut p, b"4", "tag: v0.1.0");
    // 2: flow, 1: map
    press(&mut p, b"2", "flow");
    press(&mut p, b"1", "zoom in");
    // /: search, then Esc cancels the search only
    press(&mut p, b"/", "type a symbol");
    press(&mut p, b"snapshot_for", "PaneMorphService.snapshot_for");
    press(&mut p, b"\x1b", "zoom in");
    // ?: keys, Esc closes the help
    press(&mut p, b"?", "paste the draft");
    p.send(b"\x1b");
    std::thread::sleep(Duration::from_millis(150));
    assert!(
        p.child.try_wait().unwrap().is_none(),
        "Esc on help must not quit"
    );
    // Esc closes codeMorph.
    p.send(b"\x1b");
    let status = p
        .wait_exit(Duration::from_secs(5))
        .expect("Esc closes the popup");
    assert!(status.success());
    // The terminal is restored: alternate screen left, cursor shown.
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
    // The doc's target: open to first paint under 150 ms on a small repo.
    assert!(first_paint_ms < 150.0, "first paint {first_paint_ms} ms");
    assert!(
        painted < Duration::from_millis(1500),
        "frame visible after {painted:?}"
    );
}

#[test]
fn q_quits_and_ctrl_c_quits() {
    let t = TempDir::new("pty-small");
    write(t.path(), "a.py", "def f():\n    return 1\n");
    let root = t.path().display().to_string();
    let state = t.path().join(".st").display().to_string();
    let mut p = spawn(&[&root], &[("CODEMORPH_STATE_DIR", &state)], 100, 30);
    p.wait_for("a.py", 0, Duration::from_secs(5))
        .expect("map of a plain folder");
    p.send(b"q");
    assert!(p
        .wait_exit(Duration::from_secs(5))
        .expect("q quits")
        .success());

    let mut p = spawn(&[&root], &[("CODEMORPH_STATE_DIR", &state)], 100, 30);
    p.wait_for("a.py", 0, Duration::from_secs(5)).expect("map");
    p.send(b"\x03");
    assert!(
        p.wait_exit(Duration::from_secs(5)).is_some(),
        "ctrl-c quits"
    );
}
