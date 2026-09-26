//! The terminal loop: paint the frame first, then fill in content as the
//! workers deliver it.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{cursor, execute};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::app::{App, Options};
use crate::herdr::{self, PaneOpen};
use crate::keys::{self, Action};

pub struct Trace {
    pub enabled: bool,
    pub t0: Instant,
}

fn restore() {
    let _ = terminal::disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, cursor::Show);
}

fn enter() -> io::Result<()> {
    terminal::enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, cursor::Hide)?;
    Ok(())
}

pub fn run(opts: Options, trace: Trace) -> io::Result<i32> {
    let t_setup = trace.t0.elapsed();
    let mut app = App::new(opts);
    app.started = trace.t0;
    app.pinned = std::env::var("CODEMORPH_PINNED").is_ok_and(|v| v == "1");
    app.start();

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        default_hook(info);
    }));
    enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|f| crate::ui::render(f, &mut app))?;
    let first_paint = trace.t0.elapsed();
    app.first_paint_ms = Some(first_paint.as_secs_f64() * 1000.0);
    let mut content_ms: Option<f64> = None;

    // Follow mode: a pinned split retargets when another pane gets focus.
    let follow_rx = if app.pinned { follow_events() } else { None };

    let mut dirty = false;
    loop {
        if app.poll() {
            dirty = true;
        }
        if content_ms.is_none() && app.files_loaded && !app.is_loading() {
            content_ms = Some(trace.t0.elapsed().as_secs_f64() * 1000.0);
        }
        if let Some(rx) = &follow_rx {
            if let Some(ctx) = rx.try_iter().last() {
                retarget(&mut app, ctx);
                dirty = true;
            }
        }
        if dirty {
            terminal.draw(|f| crate::ui::render(f, &mut app))?;
            dirty = false;
        }
        if !event::poll(Duration::from_millis(30))? {
            continue;
        }
        match event::read()? {
            Event::Key(k) => match keys::handle(&mut app, k) {
                Action::None => {}
                Action::Redraw => dirty = true,
                Action::Quit => break,
                Action::Edit(path, line) => {
                    restore();
                    open_editor(&path, line);
                    enter()?;
                    terminal.clear()?;
                    dirty = true;
                }
                Action::Copy(text) => {
                    app.message = Some(match copy(&text) {
                        true => format!("copied {text}"),
                        false => format!("could not copy {text}"),
                    });
                    dirty = true;
                }
                Action::Pin(placement) => {
                    match pin(&app, placement) {
                        Ok(()) => break,
                        Err(e) => app.message = Some(e),
                    }
                    dirty = true;
                }
            },
            Event::Resize(_, _) => {
                app.map.dirty = true;
                dirty = true;
            }
            _ => {}
        }
    }
    restore();
    if trace.enabled {
        let line = format!(
            "codemorph trace: setup {:.1} ms, first paint {:.1} ms, content {} ms\n",
            t_setup.as_secs_f64() * 1000.0,
            first_paint.as_secs_f64() * 1000.0,
            content_ms
                .map(|m| format!("{m:.1}"))
                .unwrap_or_else(|| "-".into())
        );
        if let Ok(path) = std::env::var("CODEMORPH_TRACE_FILE") {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = f.write_all(line.as_bytes());
            }
        } else {
            eprint!("{line}");
        }
    }
    Ok(0)
}

fn follow_events() -> Option<std::sync::mpsc::Receiver<herdr::PluginContext>> {
    let client = herdr::Client::from_env()?;
    let own = std::env::var("HERDR_PANE_ID").ok();
    let rx = client
        .subscribe(serde_json::json!([{ "type": "pane.focused" }]))
        .ok()?;
    let (tx, out) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for ev in rx {
            let pane_id = ev
                .pointer("/data/pane_id")
                .or_else(|| ev.pointer("/event/data/pane_id"))
                .or_else(|| ev.get("pane_id"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let Some(pane_id) = pane_id else { continue };
            if own.as_deref() == Some(pane_id.as_str()) {
                continue;
            }
            let Ok(pane) = client.pane_get(&pane_id) else {
                continue;
            };
            let s = |k: &str| pane.get(k).and_then(|v| v.as_str()).map(str::to_string);
            let ctx = herdr::PluginContext {
                focused_pane_id: Some(pane_id),
                focused_pane_cwd: s("foreground_cwd").or_else(|| s("cwd")),
                focused_pane_agent: s("agent"),
                focused_pane_status: s("agent_status"),
                workspace_id: s("workspace_id"),
                tab_id: s("tab_id"),
                ..Default::default()
            };
            if tx.send(ctx).is_err() {
                break;
            }
        }
    });
    Some(out)
}

fn retarget(app: &mut App, ctx: herdr::PluginContext) {
    let Some(cwd) = ctx.focused_pane_cwd.clone() else {
        return;
    };
    let new_root = crate::git::discover(std::path::Path::new(&cwd))
        .map(|r| r.root)
        .unwrap_or_else(|_| std::path::PathBuf::from(&cwd));
    let same_root = new_root == app.root;
    let same_agent = app.agent.as_ref().and_then(|a| a.pane_id.clone()) == ctx.focused_pane_id;
    if same_root && same_agent {
        return;
    }
    let opts = Options {
        path: Some(new_root),
        view: app.view,
        context: Some(ctx),
        client: app.client.clone(),
        store: app.store.clone(),
        theme: app.theme,
    };
    let pinned = app.pinned;
    *app = App::new(opts);
    app.pinned = pinned;
    app.start();
}

fn pin(app: &App, placement: &'static str) -> Result<(), String> {
    let client = app.client.clone().ok_or("pinning needs herdr")?;
    let mut env = vec![
        ("CODEMORPH_VIEW".to_string(), app.view.name().to_string()),
        ("CODEMORPH_PINNED".to_string(), "1".to_string()),
    ];
    if let Some(ctx) = &app.context {
        if let Ok(json) = serde_json::to_string(ctx) {
            env.push(("CODEMORPH_CONTEXT_JSON".to_string(), json));
        }
    }
    let req = PaneOpen {
        plugin_id: herdr::PLUGIN_ID.to_string(),
        entrypoint: "ui".to_string(),
        placement: placement.to_string(),
        width: None,
        height: None,
        target_pane_id: app.target_pane(),
        cwd: None,
        env,
        focus: placement == "tab",
    };
    client
        .plugin_pane_open(&req)
        .map(|_| ())
        .map_err(|e| format!("could not open a {placement}: {e}"))
}

fn open_editor(path: &std::path::Path, line: u32) {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    let mut parts = editor.split_whitespace();
    let Some(prog) = parts.next() else { return };
    let mut cmd = std::process::Command::new(prog);
    cmd.args(parts);
    let name = std::path::Path::new(prog)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let p = path.display().to_string();
    match name.as_str() {
        "code" | "cursor" | "zed" | "subl" => {
            if name == "code" || name == "cursor" {
                cmd.arg("-g");
            }
            cmd.arg(format!("{p}:{line}"));
        }
        "hx" | "helix" => {
            cmd.arg(format!("{p}:{line}"));
        }
        _ => {
            cmd.arg(format!("+{line}")).arg(&p);
        }
    }
    let _ = cmd.status();
}

fn copy(text: &str) -> bool {
    use std::process::{Command, Stdio};
    for (prog, args) in [
        ("pbcopy", vec![]),
        ("wl-copy", vec![]),
        ("xclip", vec!["-selection", "clipboard"]),
    ] {
        if let Ok(mut child) = Command::new(prog)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            if child.wait().is_ok_and(|s| s.success()) {
                return true;
            }
        }
    }
    // OSC 52: ask the terminal to copy.
    let b64 = base64(text.as_bytes());
    let mut out = io::stdout();
    let _ = write!(out, "\x1b]52;c;{b64}\x07");
    let _ = out.flush();
    true
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_encodes() {
        assert_eq!(super::base64(b"a.py:42"), "YS5weTo0Mg==");
        assert_eq!(super::base64(b"abc"), "YWJj");
    }
}
