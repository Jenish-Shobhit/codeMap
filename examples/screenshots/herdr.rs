//! An authentic capture: codeMap's popup over a real herdr client.
//!
//! Needs herdr 0.9.0 or newer on `PATH` and a release build of codeMap
//! (`CODEMAP_BIN`, by default `target/release/codemap`).
//!
//! Everything runs in a throwaway herdr session that cannot see yours:
//!
//! - `HOME` and every XDG directory point into `/tmp/cmtest-shot-<pid>` (a
//!   short path, because herdr's socket lives there), the calling shell's
//!   environment is cleared, and the session is named `cmtest-shot`;
//! - codeMap is registered in that session's own plugin registry, a
//!   `plugins.json` written next to its config; `herdr plugin link` never runs;
//! - the server and client are stopped with SIGTERM and the directory is
//!   removed at the end.
//!
//! The capture then does what a user does: a workspace on the demo
//! repository, an agent that works a turn (herdr runs `codemap hook` at both
//! status changes), and the `dev.codemap.open` action. A pseudo-terminal
//! carries the client's output into a vt100 parser, whose screen is written to
//! SVG in the same style as the other screenshots.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codemap::herdr::Client;
use codemap::store::Store;
use serde_json::{json, Value};

use crate::{demo, svg};

const SESSION: &str = "cmtest-shot";
const COLS: u16 = 168;
const ROWS: u16 = 54;

struct Dirs {
    base: PathBuf,
    home: PathBuf,
    cfg: PathBuf,
    state: PathBuf,
    plugin: PathBuf,
}

/// The server to stop and the directory to remove, whatever happens.
struct Cleanup {
    base: PathBuf,
    server: Option<Child>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some(server) = self.server.as_mut() {
            stop(server);
        }
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

/// SIGTERM, a few seconds to exit, then SIGKILL.
fn stop(child: &mut Child) {
    // SAFETY: plain kill(2) on a child we spawned and have not reaped.
    unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub fn capture(manifest: &Path, fixtures: &Path, out: &Path) -> Result<(), String> {
    let bin = std::env::var_os("CODEMAP_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.join("target/release/codemap"));
    if !bin.is_file() {
        return Err(format!(
            "{} not found: run `cargo build --release` first",
            bin.display()
        ));
    }
    let dirs = make_dirs(manifest, &bin)?;
    let mut cleanup = Cleanup {
        base: dirs.base.clone(),
        server: None,
    };
    let repo = demo::build_repo(fixtures, &dirs.home);

    // The server, then a client on a pseudo-terminal.
    let server = herdr(&dirs)
        .arg("server")
        .current_dir(&dirs.home)
        .stdin(Stdio::null())
        .stdout(File::create(dirs.base.join("server.log")).map_err(|e| e.to_string())?)
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("herdr server: {e}"))?;
    cleanup.server = Some(server);
    let socket = dirs
        .cfg
        .join("herdr/sessions")
        .join(SESSION)
        .join("herdr.sock");
    wait(Duration::from_secs(15), || socket.exists())
        .ok_or("the throwaway herdr server did not open its socket")?;
    let client = Client::new(&socket);
    let term = Terminal::spawn(herdr(&dirs).current_dir(&dirs.home))?;
    wait(Duration::from_secs(15), || term.text().trim().len() > 20)
        .ok_or("the herdr client did not paint")?;

    // A workspace on the repository, with a tab named for the task.
    let ws = call(
        &client,
        "workspace.create",
        json!({"cwd": repo.root, "label": "paneMorph", "focus": true}),
    )?;
    let ws_id = ws
        .pointer("/workspace/workspace_id")
        .and_then(Value::as_str)
        .ok_or("workspace.create: no workspace id")?
        .to_string();
    let pane = ws
        .pointer("/root_pane/pane_id")
        .and_then(Value::as_str)
        .ok_or("workspace.create: no root pane")?
        .to_string();
    for w in call(&client, "workspace.list", json!({}))?
        .get("workspaces")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        if let Some(id) = w.get("workspace_id").and_then(Value::as_str) {
            if id != ws_id {
                let _ = client.call("workspace.close", json!({ "workspace_id": id }));
            }
        }
    }
    let info = client.pane_get(&pane).map_err(|e| e.to_string())?;
    if let Some(tab) = info.get("tab_id").and_then(Value::as_str) {
        call(
            &client,
            "tab.rename",
            json!({"tab_id": tab, "label": "selector-fix"}),
        )?;
    }
    call(
        &client,
        "pane.send_text",
        json!({"pane_id": pane, "text": "git log --oneline -5\n"}),
    )?;

    // One agent turn: herdr runs `codemap hook` when it starts and ends.
    let store = Store::new(
        dirs.state
            .join("herdr/plugins")
            .join(codemap::herdr::PLUGIN_ID),
    );
    let report = |state: &str| {
        call(
            &client,
            "pane.report_agent",
            json!({"pane_id": pane, "source": "custom:demo", "agent": "claude", "state": state}),
        )
    };
    report("working")?;
    wait(Duration::from_secs(15), || {
        turns(&store, &repo.root).iter().any(|t| t.open)
    })
    .ok_or("the hook did not record the start of the turn")?;
    demo::apply_turn(fixtures, &repo.root);
    report("idle")?;
    wait(Duration::from_secs(15), || {
        turns(&store, &repo.root)
            .iter()
            .any(|t| !t.open && t.end.is_some())
    })
    .ok_or("the hook did not record the end of the turn")?;

    // ⌃⌥M: the action the key binding runs.
    call(
        &client,
        "plugin.action.invoke",
        json!({"plugin_id": codemap::herdr::PLUGIN_ID, "action_id": "open"}),
    )?;
    wait(Duration::from_secs(20), || {
        let t = term.text();
        t.contains("this turn") && t.contains("PaneMorphService")
    })
    .ok_or_else(|| format!("the popup did not paint:\n{}", term.text()))?;
    // Let the last frame land.
    std::thread::sleep(Duration::from_millis(800));

    let grid = term.grid();
    check_private(&term.text())?;
    let doc = svg::render(&grid, "herdr · paneMorph", &svg::Style::default());
    std::fs::write(out, doc).map_err(|e| e.to_string())?;

    term.send(b"q");
    std::thread::sleep(Duration::from_millis(300));
    drop(term);
    drop(cleanup);
    Ok(())
}

/// Refuse to write a capture that names this machine or its user.
fn check_private(text: &str) -> Result<(), String> {
    let mut names = Vec::new();
    for var in ["USER", "LOGNAME"] {
        if let Ok(v) = std::env::var(var) {
            names.push(v);
        }
    }
    if let Ok(out) = Command::new("hostname").output() {
        let host = String::from_utf8_lossy(&out.stdout).trim().to_string();
        names.push(host.split('.').next().unwrap_or_default().to_string());
    }
    for name in names.iter().filter(|n| n.len() >= 3) {
        let name = name.to_lowercase();
        if let Some(line) = text.lines().find(|l| l.to_lowercase().contains(&name)) {
            return Err(format!(
                "the capture shows {name:?}, not writing it:\n{}",
                line.trim()
            ));
        }
    }
    Ok(())
}

fn make_dirs(manifest: &Path, bin: &Path) -> Result<Dirs, String> {
    let base = PathBuf::from(format!("/tmp/{SESSION}-{}", std::process::id()));
    assert!(base.starts_with("/tmp") && base.to_string_lossy().contains("/cmtest-"));
    let _ = std::fs::remove_dir_all(&base);
    let d = Dirs {
        home: base.join("home"),
        cfg: base.join("cfg"),
        state: base.join("state"),
        plugin: base.join("plugin"),
        base,
    };
    for dir in [
        d.home.clone(),
        d.cfg.join("herdr"),
        d.state.clone(),
        d.base.join("data"),
        d.base.join("cache"),
        d.plugin.join("target/release"),
    ] {
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let copy = |from: &Path, to: &Path| {
        std::fs::copy(from, to)
            .map(|_| ())
            .map_err(|e| format!("{}: {e}", from.display()))
    };
    copy(
        &manifest.join("herdr-plugin.toml"),
        &d.plugin.join("herdr-plugin.toml"),
    )?;
    copy(bin, &d.plugin.join("target/release/codemap"))?;
    // herdr reads the rest of the entry from the manifest when it loads.
    let registry = json!([{
        "plugin_id": codemap::herdr::PLUGIN_ID,
        "name": "codeMap",
        "version": env!("CARGO_PKG_VERSION"),
        "manifest_path": d.plugin.join("herdr-plugin.toml"),
        "plugin_root": d.plugin,
        "enabled": true,
    }]);
    std::fs::write(d.cfg.join("herdr/plugins.json"), registry.to_string())
        .map_err(|e| e.to_string())?;
    let config = r##"onboarding = false

[theme]
name = "dracula"

[theme.custom]
sidebar_bg = "#21222c"

[ui]
window_title = ""
"##;
    std::fs::write(d.cfg.join("herdr/config.toml"), config).map_err(|e| e.to_string())?;
    Ok(d)
}

/// `herdr --session cmtest-shot` with nothing from the calling environment
/// but `PATH`.
fn herdr(d: &Dirs) -> Command {
    let shell = ["/bin/dash", "/usr/bin/dash", "/bin/sh"]
        .into_iter()
        .find(|p| Path::new(p).exists())
        .unwrap_or("/bin/sh");
    let mut c = Command::new("herdr");
    c.env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", &d.home)
        .env("USER", "dev")
        .env("LOGNAME", "dev")
        .env("LANG", "en_US.UTF-8")
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        // dash reads no system rc file, so the prompt stays `$ ` instead of
        // bash's `host:dir user$` from /etc/bashrc.
        .env("SHELL", shell)
        .env("PS1", "$ ")
        .env("XDG_CONFIG_HOME", &d.cfg)
        .env("XDG_STATE_HOME", &d.state)
        .env("XDG_DATA_HOME", d.base.join("data"))
        .env("XDG_CACHE_HOME", d.base.join("cache"))
        .env("HERDR_DISABLE_SOUND", "1")
        .args(["--session", SESSION]);
    c
}

fn call(client: &Client, method: &str, params: Value) -> Result<Value, String> {
    client
        .call(method, params)
        .map_err(|e| format!("{method}: {e}"))
}

fn turns(store: &Store, root: &Path) -> Vec<codemap::store::Turn> {
    store
        .panes_for_repo(root)
        .into_iter()
        .flat_map(|p| p.turns)
        .collect()
}

fn wait(limit: Duration, mut ok: impl FnMut() -> bool) -> Option<()> {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if ok() {
            return Some(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

// ---- a terminal for the client -----------------------------------------------

struct Terminal {
    child: Child,
    master: File,
    parser: Arc<Mutex<vt100::Parser>>,
}

impl Drop for Terminal {
    fn drop(&mut self) {
        stop(&mut self.child);
    }
}

impl Terminal {
    fn spawn(cmd: &mut Command) -> Result<Terminal, String> {
        let (mut master_fd, mut slave_fd) = (0, 0);
        let mut ws = libc::winsize {
            ws_row: ROWS,
            ws_col: COLS,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: openpty fills the two descriptors; we own them afterwards.
        let rc = unsafe {
            libc::openpty(
                &mut master_fd,
                &mut slave_fd,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &raw mut ws,
            )
        };
        if rc != 0 {
            return Err("openpty failed".into());
        }
        let slave = unsafe { OwnedFd::from_raw_fd(slave_fd) };
        let master = unsafe { File::from_raw_fd(master_fd) };
        let io = |fd: &OwnedFd| -> Result<Stdio, String> {
            fd.try_clone().map(Stdio::from).map_err(|e| e.to_string())
        };
        cmd.stdin(io(&slave)?)
            .stdout(io(&slave)?)
            .stderr(io(&slave)?);
        // SAFETY: setsid and TIOCSCTTY are async-signal-safe; they make the
        // pseudo-terminal the child's controlling terminal.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                libc::ioctl(0, libc::TIOCSCTTY as _, 0);
                Ok(())
            });
        }
        let child = cmd.spawn().map_err(|e| format!("herdr client: {e}"))?;
        drop(slave);
        let parser = Arc::new(Mutex::new(vt100::Parser::new(ROWS, COLS, 0)));
        let mut reader = master.try_clone().map_err(|e| e.to_string())?;
        let mut replies = master.try_clone().map_err(|e| e.to_string())?;
        let p = parser.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 65536];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let chunk = &buf[..n];
                let mut parser = p.lock().unwrap();
                parser.process(chunk);
                // Answer the queries a terminal answers, so the client does
                // not wait on them.
                for reply in answers(chunk, parser.screen().cursor_position()) {
                    let _ = replies.write_all(&reply);
                }
            }
        });
        Ok(Terminal {
            child,
            master,
            parser,
        })
    }

    fn send(&self, bytes: &[u8]) {
        let _ = (&self.master).write_all(bytes);
    }

    fn text(&self) -> String {
        self.parser.lock().unwrap().screen().contents()
    }

    fn grid(&self) -> svg::Grid {
        let parser = self.parser.lock().unwrap();
        let screen = parser.screen();
        let (rows, cols) = screen.size();
        let (fg0, bg0) = ((0xf8, 0xf8, 0xf2), (0x28, 0x2a, 0x36));
        let mut cells = Vec::with_capacity(rows as usize * cols as usize);
        for r in 0..rows {
            for c in 0..cols {
                let cell = screen.cell(r, c);
                let Some(cell) = cell else {
                    cells.push(blank(fg0, bg0));
                    continue;
                };
                let mut fg = color(cell.fgcolor(), fg0);
                let mut bg = color(cell.bgcolor(), bg0);
                if cell.inverse() {
                    std::mem::swap(&mut fg, &mut bg);
                }
                let ch = if cell.is_wide_continuation() {
                    String::new()
                } else if cell.has_contents() {
                    cell.contents().to_string()
                } else {
                    " ".to_string()
                };
                cells.push(svg::Cell {
                    ch,
                    fg,
                    bg,
                    bold: cell.bold(),
                    italic: cell.italic(),
                    dim: cell.dim(),
                    underline: cell.underline(),
                });
            }
        }
        svg::Grid {
            cols: cols as usize,
            rows: rows as usize,
            cells,
        }
    }
}

fn blank(fg: svg::Rgb, bg: svg::Rgb) -> svg::Cell {
    svg::Cell {
        ch: " ".into(),
        fg,
        bg,
        bold: false,
        italic: false,
        dim: false,
        underline: false,
    }
}

/// Replies to device-status, device-attribute and colour queries.
fn answers(chunk: &[u8], (row, col): (u16, u16)) -> Vec<Vec<u8>> {
    let s = String::from_utf8_lossy(chunk);
    let mut out = Vec::new();
    if s.contains("\x1b[6n") {
        out.push(format!("\x1b[{};{}R", row + 1, col + 1).into_bytes());
    }
    if s.contains("\x1b[c") || s.contains("\x1b[0c") {
        out.push(b"\x1b[?62;22c".to_vec());
    }
    if s.contains("\x1b]10;?") {
        out.push(b"\x1b]10;rgb:f8f8/f8f8/f2f2\x1b\\".to_vec());
    }
    if s.contains("\x1b]11;?") {
        out.push(b"\x1b]11;rgb:2828/2a2a/3636\x1b\\".to_vec());
    }
    out
}

/// A terminal colour, with the Dracula palette for the 16 named ones.
fn color(c: vt100::Color, default: svg::Rgb) -> svg::Rgb {
    const ANSI: [svg::Rgb; 16] = [
        (0x21, 0x22, 0x2c),
        (0xff, 0x55, 0x55),
        (0x50, 0xfa, 0x7b),
        (0xf1, 0xfa, 0x8c),
        (0xbd, 0x93, 0xf9),
        (0xff, 0x79, 0xc6),
        (0x8b, 0xe9, 0xfd),
        (0xf8, 0xf8, 0xf2),
        (0x62, 0x72, 0xa4),
        (0xff, 0x6e, 0x6e),
        (0x69, 0xff, 0x94),
        (0xff, 0xff, 0xa5),
        (0xd6, 0xac, 0xff),
        (0xff, 0x92, 0xdf),
        (0xa4, 0xff, 0xff),
        (0xff, 0xff, 0xff),
    ];
    match c {
        vt100::Color::Default => default,
        vt100::Color::Rgb(r, g, b) => (r, g, b),
        vt100::Color::Idx(i) if i < 16 => ANSI[i as usize],
        vt100::Color::Idx(i) if i < 232 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            (level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        vt100::Color::Idx(i) => {
            let v = 8 + (i - 232) * 10;
            (v, v, v)
        }
    }
}
