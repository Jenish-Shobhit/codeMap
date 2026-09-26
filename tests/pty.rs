//! PTY harness: runs the real binary on a pseudo-terminal the way herdr's
//! popup does, measures open-to-first-paint, drives keys and checks that Esc
//! closes it.
//!
//! `CODEMORPH_BIN=target/release/codemorph cargo test --test pty -- --nocapture`
//! measures a release build.

mod common;

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;


/// A minimal terminal screen: enough of VT100 to replay what ratatui and
/// crossterm write (cursor moves, text, clears), ignoring colours.
struct Screen {
    cols: usize,
    rows: usize,
    cells: Vec<Vec<char>>,
    x: usize,
    y: usize,
}

impl Screen {
    fn new(cols: usize, rows: usize) -> Self {
        Screen { cols, rows, cells: vec![vec![' '; cols]; rows], x: 0, y: 0 }
    }

    fn feed(&mut self, bytes: &[u8]) {
        let s = String::from_utf8_lossy(bytes);
        let mut it = s.chars().peekable();
        while let Some(c) = it.next() {
            match c {
                '\x1b' => match it.next() {
                    Some('[') => {
                        let mut params = String::new();
                        let mut fin = ' ';
                        for d in it.by_ref() {
                            if ('@'..='~').contains(&d) {
                                fin = d;
                                break;
                            }
                            params.push(d);
                        }
                        let nums: Vec<usize> = params
                            .trim_start_matches('?')
                            .split(';')
                            .map(|p| p.parse().unwrap_or(0))
                            .collect();
                        match fin {
                            'H' | 'f' => {
                                self.y = nums.first().copied().unwrap_or(1).max(1) - 1;
                                self.x = nums.get(1).copied().unwrap_or(1).max(1) - 1;
                            }
                            'J' if nums.first() == Some(&2) => {
                                self.cells = vec![vec![' '; self.cols]; self.rows];
                            }
                            _ => {}
                        }
                    }
                    Some(']') => {
                        // OSC: skip to BEL or ST
                        while let Some(d) = it.next() {
                            if d == '\x07' {
                                break;
                            }
                            if d == '\x1b' {
                                it.next();
                                break;
                            }
                        }
                    }
                    _ => {}
                },
                '\r' => self.x = 0,
                '\n' => self.y = (self.y + 1).min(self.rows - 1),
                c if c.is_control() => {}
                c => {
                    let w = codemorph::util::char_width(c).max(1);
                    if self.y < self.rows && self.x < self.cols {
                        self.cells[self.y][self.x] = c;
                        if w == 2 && self.x + 1 < self.cols {
                            self.cells[self.y][self.x + 1] = '\0';
                        }
                    }
                    self.x += w;
                }
            }
        }
    }

    fn text(&self) -> String {
        self.cells
            .iter()
            .map(|r| r.iter().filter(|c| **c != '\0').collect::<String>().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

struct Pty {
    child: Child,
    writer: File,
    out: Arc<Mutex<Vec<u8>>>,
    first_byte: Arc<Mutex<Option<Instant>>>,
    started: Instant,
    cols: u16,
    rows: u16,
}

fn binary() -> String {
    std::env::var("CODEMORPH_BIN").unwrap_or_else(|_| env!("CARGO_BIN_EXE_codemorph").to_string())
}

fn spawn(args: &[&str], envs: &[(&str, &str)], cols: u16, rows: u16) -> Pty {
    let mut master: libc::c_int = 0;
    let mut slave: libc::c_int = 0;
    let mut ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: openpty fills the two descriptors; we own them afterwards.
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut ws,
        )
    };
    assert_eq!(rc, 0, "openpty failed");
    let slave = unsafe { OwnedFd::from_raw_fd(slave) };
    let mut cmd = Command::new(binary());
    cmd.args(args)
        .env("TERM", "xterm-256color")
        .env_remove("HERDR_ENV")
        .env_remove("HERDR_SOCKET_PATH")
        .env_remove("HERDR_PLUGIN_CONTEXT_JSON")
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave));
    for (k, v) in envs {
        cmd.env(k, v);
    }
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            libc::ioctl(0, libc::TIOCSCTTY as _, 0);
            Ok(())
        });
    }
    let started = Instant::now();
    let child = cmd.spawn().expect("spawn codemorph");
    let master = unsafe { File::from_raw_fd(master) };
    let writer = master.try_clone().unwrap();
    let out = Arc::new(Mutex::new(Vec::new()));
    let first_byte = Arc::new(Mutex::new(None));
    {
        let out = out.clone();
        let first_byte = first_byte.clone();
        let mut reader = master;
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut fb = first_byte.lock().unwrap();
                        if fb.is_none() {
                            *fb = Some(Instant::now());
                        }
                        out.lock().unwrap().extend_from_slice(&buf[..n]);
                    }
                }
            }
        });
    }
    Pty {
        child,
        writer,
        out,
        first_byte,
        started,
        cols,
        rows,
    }
}

impl Pty {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.out.lock().unwrap()).into_owned()
    }

    /// The screen as the terminal would show it now.
    fn screen(&self) -> String {
        let mut sc = Screen::new(self.cols as usize, self.rows as usize);
        sc.feed(&self.out.lock().unwrap());
        sc.text()
    }

    /// Wait until the screen shows `needle` (once `from` bytes have arrived).
    fn wait_for(&self, needle: &str, from: usize, timeout: Duration) -> Option<Duration> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.len() > from && self.screen().contains(needle) {
                return Some(self.started.elapsed());
            }
            if Instant::now() > deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(3));
        }
    }

    fn len(&self) -> usize {
        self.out.lock().unwrap().len()
    }

    fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).unwrap();
        self.writer.flush().unwrap();
    }

    fn wait_exit(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(Some(s)) = self.child.try_wait() {
                return Some(s);
            }
            if Instant::now() > deadline {
                let _ = self.child.kill();
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

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
    let painted = p.wait_for("history", 0, Duration::from_secs(5)).expect("first paint");
    // Content fills in.
    p.wait_for("PaneMorphService", 0, Duration::from_secs(10)).expect("map content");
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
    assert!(p.child.try_wait().unwrap().is_none(), "Esc on help must not quit");
    // Esc closes codeMorph.
    p.send(b"\x1b");
    let status = p.wait_exit(Duration::from_secs(5)).expect("Esc closes the popup");
    assert!(status.success());
    // The terminal is restored: alternate screen left, cursor shown.
    let out = p.text();
    assert!(out.contains("\x1b[?1049l"), "left the alternate screen");
    assert!(out.contains("\x1b[?25h"), "cursor shown again");

    let first_byte = p.first_byte.lock().unwrap().unwrap().duration_since(p.started);
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
    assert!(painted < Duration::from_millis(1500), "frame visible after {painted:?}");
}

#[test]
fn q_quits_and_ctrl_c_quits() {
    let t = TempDir::new("pty-small");
    write(t.path(), "a.py", "def f():\n    return 1\n");
    let root = t.path().display().to_string();
    let state = t.path().join(".st").display().to_string();
    let mut p = spawn(&[&root], &[("CODEMORPH_STATE_DIR", &state)], 100, 30);
    p.wait_for("a.py", 0, Duration::from_secs(5)).expect("map of a plain folder");
    p.send(b"q");
    assert!(p.wait_exit(Duration::from_secs(5)).expect("q quits").success());

    let mut p = spawn(&[&root], &[("CODEMORPH_STATE_DIR", &state)], 100, 30);
    p.wait_for("a.py", 0, Duration::from_secs(5)).expect("map");
    p.send(b"\x03");
    assert!(p.wait_exit(Duration::from_secs(5)).is_some(), "ctrl-c quits");
}
