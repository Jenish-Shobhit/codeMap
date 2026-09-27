//! Live tests against a real, isolated herdr server. Ignored by default.
//!
//! Start a throwaway server first (never your own session), for example:
//!
//! ```sh
//! B=/tmp/cmtest-$(date +%s); mkdir -p $B/cfg $B/state
//! env -u HERDR_SOCKET_PATH -u HERDR_ENV XDG_CONFIG_HOME=$B/cfg XDG_STATE_HOME=$B/state \
//!     SHELL=/bin/sh herdr --session cmtest-x server &
//! CODEMAP_LIVE_SOCKET=$B/cfg/herdr/sessions/cmtest-x/herdr.sock \
//!     cargo test --test live_herdr -- --ignored --nocapture
//! ```
//!
//! The hook is driven the way herdr drives it: a `pane.agent_status_changed`
//! event from the server's own event stream is handed to `codemap hook`
//! with herdr's hook environment. Comments go to a dummy agent pane running
//! `cat`, through the real popup UI on a pty and through `agent.prompt`.

mod common;

use std::process::Command;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use codemap::app::{App, Options, View};
use codemap::herdr::{Client, PluginContext};
use codemap::store::{Comment, PaneState, Store};
use common::pty::*;
use common::*;
use serde_json::{json, Value};

/// The isolated socket, after refusing anything that could be the user's.
fn live_socket() -> Option<String> {
    let sock = std::env::var("CODEMAP_LIVE_SOCKET").ok()?;
    let home = std::env::var("HOME").unwrap_or_default();
    assert!(
        sock.contains("/cmtest-"),
        "refusing {sock}: not an isolated cmtest-* socket"
    );
    assert!(
        !sock.starts_with(&format!("{home}/.config/herdr")),
        "refusing {sock}: that is under your herdr config"
    );
    assert!(
        std::path::Path::new(&sock).exists(),
        "{sock} does not exist"
    );
    Some(sock)
}

fn next_event(rx: &Receiver<Value>, status: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let ev = rx.recv_timeout(left).expect("status event from herdr");
        if ev.pointer("/data/agent_status").and_then(Value::as_str) == Some(status) {
            return ev;
        }
    }
}

fn read_pane(client: &Client, pane: &str) -> String {
    let r = client
        .call(
            "pane.read",
            json!({"pane_id": pane, "source": "recent", "lines": 200}),
        )
        .unwrap();
    r.pointer("/read/text")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| r.to_string())
}

fn wait_pane_text(client: &Client, pane: &str, needle: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let t = read_pane(client, pane);
        if t.contains(needle) || Instant::now() > deadline {
            return t;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Run `codemap hook` with the environment herdr gives event hooks.
fn run_hook(sock: &str, state: &std::path::Path, event: &Value, ctx: &Value) -> (String, Duration) {
    let t = Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_codemap"))
        .arg("hook")
        .current_dir(state)
        .env("HERDR_ENV", "1")
        .env("HERDR_SOCKET_PATH", sock)
        .env("HERDR_PLUGIN_ID", "dev.codemap")
        .env("HERDR_PLUGIN_STATE_DIR", state)
        .env("HERDR_PLUGIN_EVENT", "pane.agent_status_changed")
        .env("HERDR_PLUGIN_EVENT_JSON", event.to_string())
        .env("HERDR_PLUGIN_CONTEXT_JSON", ctx.to_string())
        .output()
        .unwrap();
    let dt = t.elapsed();
    assert!(
        out.status.success(),
        "hook failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    (String::from_utf8_lossy(&out.stdout).trim().to_string(), dt)
}

/// herdr's plugin_context_for_event for a pane event: that pane's details.
fn context_for(client: &Client, pane: &str) -> Value {
    let p = client.pane_get(pane).unwrap();
    json!({
        "workspace_id": p["workspace_id"],
        "workspace_label": "cmtest-agent",
        "tab_id": p["tab_id"],
        "focused_pane_id": pane,
        "focused_pane_cwd": p["cwd"],
        "focused_pane_agent": p["agent"],
        "focused_pane_status": p["agent_status"],
        "invocation_source": "event",
    })
}

#[test]
#[ignore]
fn live_hook_checkpoints_and_comment_delivery() {
    let Some(sock) = live_socket() else {
        eprintln!("CODEMAP_LIVE_SOCKET not set: skipping live test");
        return;
    };
    let client = Client::new(&sock);
    let pong = client.ping().unwrap();
    assert_eq!(pong["version"], "0.9.0");
    println!(
        "live: herdr {} protocol {} at {sock}",
        pong["version"], pong["protocol"]
    );

    let repo = repo_with(&[("app.py", "def main():\n    return 1\n")]);
    let state = TempDir::new("live-state");
    let before = fingerprint(repo.path());

    // A throwaway workspace whose shell stands in for an agent.
    let ws = client
        .call(
            "workspace.create",
            json!({"cwd": repo.path(), "label": "cmtest-agent", "focus": true}),
        )
        .unwrap();
    let ws_id = ws
        .pointer("/workspace/workspace_id")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    let pane = ws
        .pointer("/root_pane/pane_id")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    let terminal_id = ws
        .pointer("/root_pane/terminal_id")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    println!("live: workspace {ws_id}, dummy agent pane {pane} ({terminal_id})");

    let events = client
        .subscribe(json!([{"type": "pane.agent_status_changed", "pane_id": pane}]))
        .unwrap();
    let report = |state: &str| {
        client
            .call(
                "pane.report_agent",
                json!({"pane_id": pane, "source": "custom:cmtest", "agent": "claude", "state": state}),
            )
            .unwrap();
    };

    // --- Hook: turn start and end, from herdr's own events --------------------
    report("working");
    let ev = next_event(&events, "working");
    let (out, t_start) = run_hook(&sock, state.path(), &ev, &context_for(&client, &pane));
    assert!(
        out.starts_with(&format!("start {terminal_id}-claude turn 1")),
        "{out}"
    );
    // The "agent" edits during its turn.
    write(
        repo.path(),
        "app.py",
        "def main():\n    return helper()\n\ndef helper():\n    return 2\n",
    );
    write(repo.path(), "notes.md", "turn notes\n");
    report("idle");
    let ev = next_event(&events, "idle");
    let (out, t_end) = run_hook(&sock, state.path(), &ev, &context_for(&client, &pane));
    assert!(
        out.starts_with(&format!("end {terminal_id}-claude turn 1")),
        "{out}"
    );
    println!(
        "live: hook start {:.0} ms, end {:.0} ms",
        t_start.as_secs_f64() * 1000.0,
        t_end.as_secs_f64() * 1000.0
    );
    let store = Store::new(state.path());
    let key = PaneState::key_for(Some(&terminal_id), &pane, Some("claude"));
    let ps = store.load_pane(&key);
    assert_eq!(ps.turns.len(), 1);
    let turn = &ps.turns[0];
    let info = codemap::git::discover(repo.path()).unwrap();
    let diff = store
        .shadow(&info)
        .diff(
            &turn.start.as_ref().unwrap().tree,
            &turn.end.as_ref().unwrap().tree,
        )
        .unwrap();
    let paths: Vec<&str> = diff.iter().map(|d| d.path.as_str()).collect();
    assert_eq!(paths, vec!["app.py", "notes.md"]);

    // --- The popup UI, inside herdr, on a pty: review and send with P --------
    // The dummy agent: a copy of `cat` named `claude`, so herdr sees an agent
    // process in the foreground and whatever is typed into it shows up.
    let bin = TempDir::new("live-bin");
    let dummy = bin.path().join("claude");
    let example = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/echo_agent");
    if !example.exists() {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let st = Command::new(cargo)
            .args(["build", "--example", "echo_agent"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .status()
            .unwrap();
        assert!(st.success());
    }
    std::fs::copy(&example, &dummy).expect("examples/echo_agent");
    client
        .send_text(&pane, &format!("{}\r", dummy.display()))
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let ctx: PluginContext = serde_json::from_value(context_for(&client, &pane)).unwrap();
    let ctx_json = serde_json::to_string(&ctx).unwrap();
    let cfg = state.path().join("herdr-config.toml");
    std::fs::write(&cfg, "[theme]\nname = \"dracula\"\n").unwrap();
    let state_s = state.path().display().to_string();
    let cfg_s = cfg.display().to_string();
    let mut ui = spawn(
        &["ui"],
        &[
            ("HERDR_ENV", "1"),
            ("HERDR_SOCKET_PATH", &sock),
            ("HERDR_PLUGIN_STATE_DIR", &state_s),
            ("HERDR_PLUGIN_CONTEXT_JSON", &ctx_json),
            ("CODEMAP_VIEW", "changes"),
            ("HERDR_CONFIG_PATH", &cfg_s),
        ],
        160,
        48,
    );
    ui.wait_for("this turn · 2 files", 0, Duration::from_secs(10))
        .unwrap_or_else(|| panic!("changes view of the live turn:\n{}", ui.screen()));
    let screen = ui.screen();
    assert!(screen.contains("cmtest-agent"), "{screen}");
    assert!(screen.contains("turn 1"), "{screen}");
    // A comment on the first hunk, then P.
    let from = ui.len();
    ui.send(b"c");
    ui.wait_for("comment on app.py", from, Duration::from_secs(5))
        .expect("comment prompt");
    ui.send(b"helper() needs a docstring\r");
    ui.wait_for("draft  1 comment", 0, Duration::from_secs(5))
        .expect("draft");
    ui.send(b"P");
    let status = ui
        .wait_exit(Duration::from_secs(10))
        .expect("P closes the popup");
    assert!(status.success());
    let text = wait_pane_text(&client, &pane, "agent got:");
    assert!(
        text.contains("agent got:"),
        "the dummy agent received input:\n{text}"
    );
    let text = wait_pane_text(&client, &pane, "helper() needs a docstring");
    assert!(
        text.contains("Review comments from codeMap:"),
        "pane shows:\n{text}"
    );
    assert!(
        text.contains("app.py:1-") || text.contains("app.py:"),
        "pane shows:\n{text}"
    );
    assert!(
        text.contains("helper() needs a docstring"),
        "pane shows:\n{text}"
    );
    println!("live: P typed the draft into {pane}");

    // --- S: submit with agent.prompt ----------------------------------------
    // Report the (dummy) agent as ready.
    report("working");
    let _ = next_event(&events, "working");
    report("idle");
    let _ = next_event(&events, "idle");
    let mut app = App::new(Options {
        path: None,
        view: View::Changes,
        context: Some(ctx.clone()),
        client: Some(client.clone()),
        store: store.clone(),
        theme: dracula(),
    });
    app.load_blocking();
    app.drafts = vec![Comment {
        path: "app.py".into(),
        start: 4,
        end: 5,
        text: "rename helper to compute".into(),
        hunk: String::new(),
    }];
    app.send_draft(true).expect("agent.prompt accepted");
    let text = wait_pane_text(&client, &pane, "rename helper to compute");
    assert!(
        text.contains("app.py:4-5 — rename helper to compute"),
        "pane shows:\n{text}"
    );
    println!("live: S submitted the draft with agent.prompt");

    // A blocked agent refuses a submit; the draft stays.
    report("blocked");
    let _ = next_event(&events, "blocked");
    app.drafts = vec![Comment {
        path: "app.py".into(),
        start: 1,
        end: 1,
        text: "wait".into(),
        hunk: String::new(),
    }];
    let err = app.send_draft(true).unwrap_err();
    assert!(err.contains("waiting on a question"), "{err}");
    assert_eq!(app.drafts.len(), 1);

    // `codemap open` reaches plugin.pane.open (the plugin is not linked in
    // this throwaway server, so herdr answers plugin_not_found).
    let out = Command::new(env!("CARGO_BIN_EXE_codemap"))
        .args(["open", "--view", "map"])
        .env("HERDR_ENV", "1")
        .env("HERDR_SOCKET_PATH", &sock)
        .env("HERDR_PLUGIN_CONTEXT_JSON", &ctx_json)
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("plugin_not_found"), "{err}");

    // Clean up the throwaway workspace; the repository is untouched.
    client.send_text(&pane, "\x04").ok();
    client
        .call("workspace.close", json!({"workspace_id": ws_id}))
        .unwrap();
    std::fs::remove_file(repo.path().join("notes.md")).unwrap();
    write(repo.path(), "app.py", "def main():\n    return 1\n");
    let after = fingerprint(repo.path());
    assert_eq!(before.0, after.0, ".git changed");
    println!("live: repository .git unchanged");
}

/// Pinned mode follows the focused pane: focusing a pane in another
/// repository retargets the UI.
#[test]
#[ignore]
fn live_follow_mode_retargets_on_focus() {
    let Some(sock) = live_socket() else {
        eprintln!("CODEMAP_LIVE_SOCKET not set: skipping live test");
        return;
    };
    let client = Client::new(&sock);
    let tmp = TempDir::new("follow");
    let a = tmp.path().join("alpha-repo");
    let b = tmp.path().join("beta-repo");
    for (dir, file) in [(&a, "alpha.py"), (&b, "beta.py")] {
        std::fs::create_dir_all(dir).unwrap();
        common::git(dir, &["init", "-q"]);
        write(dir, file, "def f():\n    return 1\n");
        common::git(dir, &["add", "-A"]);
        common::git(dir, &["commit", "-q", "-m", "init"]);
    }
    let ws_a = client
        .call(
            "workspace.create",
            json!({"cwd": a, "label": "cmtest-alpha", "focus": true}),
        )
        .unwrap();
    let ws_b = client
        .call(
            "workspace.create",
            json!({"cwd": b, "label": "cmtest-beta", "focus": false}),
        )
        .unwrap();
    let pane_a = ws_a
        .pointer("/root_pane/pane_id")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    let pane_b = ws_b
        .pointer("/root_pane/pane_id")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    let ctx = json!({"focused_pane_id": pane_a, "focused_pane_cwd": a, "workspace_label": "cmtest-alpha"});
    let state = TempDir::new("follow-state");
    let state_s = state.path().display().to_string();
    let ctx_s = ctx.to_string();
    let mut ui = spawn(
        &["ui"],
        &[
            ("HERDR_ENV", "1"),
            ("HERDR_SOCKET_PATH", &sock),
            ("HERDR_PLUGIN_STATE_DIR", &state_s),
            ("HERDR_PLUGIN_CONTEXT_JSON", &ctx_s),
            ("CODEMAP_PINNED", "1"),
            ("CODEMAP_VIEW", "map"),
        ],
        140,
        40,
    );
    ui.wait_for("alpha.py", 0, Duration::from_secs(10))
        .unwrap_or_else(|| panic!("alpha map:\n{}", ui.screen()));
    std::thread::sleep(Duration::from_millis(300));
    client
        .call("pane.focus", json!({"pane_id": pane_b}))
        .unwrap();
    ui.wait_for("beta.py", 0, Duration::from_secs(10))
        .unwrap_or_else(|| {
            panic!(
                "after focusing {pane_b}, expected beta's map:\n{}",
                ui.screen()
            )
        });
    println!("live: pinned codeMap followed focus from {pane_a} to {pane_b}");
    ui.send(b"q");
    assert!(ui.wait_exit(Duration::from_secs(5)).is_some());
    for ws in [&ws_a, &ws_b] {
        let id = ws
            .pointer("/workspace/workspace_id")
            .and_then(Value::as_str)
            .unwrap();
        client
            .call("workspace.close", json!({"workspace_id": id}))
            .unwrap();
    }
}
