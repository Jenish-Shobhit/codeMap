//! The checkpoint hook: turn boundaries from agent status changes, keyed by
//! terminal id and agent, stored in the shadow store without touching the
//! repository.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::process::Command;

use codemorph::herdr::{AgentStatusEvent, Client, PluginContext};
use codemorph::hook;
use codemorph::store::{PaneState, Store, MAX_TURNS};
use common::*;
use serde_json::{json, Value};

fn event(pane: &str, status: &str) -> AgentStatusEvent {
    AgentStatusEvent {
        pane_id: pane.into(),
        workspace_id: Some("w1".into()),
        agent_status: status.into(),
        agent: Some("claude".into()),
    }
}

fn ctx(pane: &str, cwd: &std::path::Path) -> PluginContext {
    PluginContext {
        focused_pane_id: Some(pane.into()),
        focused_pane_cwd: Some(cwd.display().to_string()),
        focused_pane_agent: Some("claude".into()),
        ..Default::default()
    }
}

#[test]
fn turns_start_and_end_on_status_changes() {
    let repo = repo_with(&[("app.py", "def main():\n    return 1\n")]);
    let state = TempDir::new("hook-state");
    let store = Store::new(state.path());
    let before = fingerprint(repo.path());
    let c = ctx("w1:p2", repo.path());

    let out = hook::handle(&store, None, &event("w1:p2", "idle"), Some(&c)).unwrap();
    assert_eq!(out.action, "none");

    let out = hook::handle(&store, None, &event("w1:p2", "working"), Some(&c)).unwrap();
    assert_eq!(out.action, "start");
    assert_eq!(out.turn, Some(1));
    let start = out.checkpoint.unwrap();

    // The agent edits during its turn.
    write(
        repo.path(),
        "app.py",
        "def main():\n    return helper()\n\ndef helper():\n    return 1\n",
    );

    // Repeated status reports do nothing.
    let out = hook::handle(&store, None, &event("w1:p2", "working"), Some(&c)).unwrap();
    assert_eq!(out.action, "none");

    let out = hook::handle(&store, None, &event("w1:p2", "done"), Some(&c)).unwrap();
    assert_eq!(out.action, "end");
    assert_eq!(out.turn, Some(1));
    let end = out.checkpoint.unwrap();

    let info = codemorph::git::discover(repo.path()).unwrap();
    let shadow = store.shadow(&info);
    let diff = shadow.diff(&start.tree, &end.tree).unwrap();
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0].path, "app.py");
    let refs = shadow.refs();
    assert!(refs.iter().any(|r| r.ends_with("/1-start")), "{refs:?}");
    assert!(refs.iter().any(|r| r.ends_with("/1-end")), "{refs:?}");

    let pane = store.load_pane(&PaneState::key_for(None, "w1:p2", Some("claude")));
    assert_eq!(pane.turns.len(), 1);
    assert!(!pane.turns[0].open);
    assert_eq!(pane.turns[0].end_status.as_deref(), Some("done"));
    assert_eq!(pane.last_status.as_deref(), Some("done"));

    // A second turn gets number 2.
    let out = hook::handle(&store, None, &event("w1:p2", "working"), Some(&c)).unwrap();
    assert_eq!((out.action, out.turn), ("start", Some(2)));

    // Blocked ends the turn; answering the question reopens the same turn.
    let out = hook::handle(&store, None, &event("w1:p2", "blocked"), Some(&c)).unwrap();
    assert_eq!((out.action, out.turn), ("end", Some(2)));
    let out = hook::handle(&store, None, &event("w1:p2", "working"), Some(&c)).unwrap();
    assert_eq!((out.action, out.turn), ("reopen", Some(2)));
    let out = hook::handle(&store, None, &event("w1:p2", "idle"), Some(&c)).unwrap();
    assert_eq!((out.action, out.turn), ("end", Some(2)));

    // Undo the test's own edit: the repository itself must be untouched.
    write(repo.path(), "app.py", "def main():\n    return 1\n");
    let after = fingerprint(repo.path());
    assert_eq!(before.0, after.0, "the hook wrote into .git");
    assert_eq!(before.1, after.1);
}

#[test]
fn outside_a_repo_only_the_status_is_kept() {
    let dir = TempDir::new("nonrepo");
    let state = TempDir::new("hook-state2");
    let store = Store::new(state.path());
    let c = ctx("w1:p1", dir.path());
    let out = hook::handle(&store, None, &event("w1:p1", "working"), Some(&c)).unwrap();
    assert_eq!(out.action, "none");
    assert!(out.repo.is_none());
    let pane = store.load_pane(&PaneState::key_for(None, "w1:p1", Some("claude")));
    assert_eq!(pane.last_status.as_deref(), Some("working"));
    assert!(pane.turns.is_empty());
}

#[test]
fn retention_keeps_the_last_fifty_turns() {
    let repo = repo_with(&[("a.py", "x = 1\n")]);
    let state = TempDir::new("hook-state3");
    let store = Store::new(state.path());
    let c = ctx("w1:p3", repo.path());
    for i in 0..(MAX_TURNS + 3) {
        write(repo.path(), "a.py", &format!("x = {i}\n"));
        hook::handle(&store, None, &event("w1:p3", "working"), Some(&c)).unwrap();
        hook::handle(&store, None, &event("w1:p3", "idle"), Some(&c)).unwrap();
    }
    let pane = store.load_pane(&PaneState::key_for(None, "w1:p3", Some("claude")));
    assert_eq!(pane.turns.len(), MAX_TURNS);
    assert_eq!(pane.turns[0].n, 4);
    let info = codemorph::git::discover(repo.path()).unwrap();
    let refs = store.shadow(&info).refs();
    assert_eq!(refs.len(), MAX_TURNS * 2, "old refs are deleted");
    assert!(!refs.iter().any(|r| r.ends_with("/1-start")));
}

/// A fake herdr answering pane.get, so the hook keys by terminal id and
/// follows the pane's foreground cwd.
fn fake_herdr(_dir: &std::path::Path, cwd: &std::path::Path) -> std::path::PathBuf {
    // Unix socket paths are limited to ~104 bytes on macOS; temp_dir() is long.
    let sock = std::path::PathBuf::from(format!(
        "/tmp/cmh-{}-{}.sock",
        std::process::id(),
        codemorph::util::now_unix_ms()
    ));
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock).unwrap();
    let cwd = cwd.display().to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { break };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                continue;
            }
            let req: Value = serde_json::from_str(line.trim()).unwrap();
            let result = match req["method"].as_str() {
                Some("pane.get") => json!({"type": "pane_info", "pane": {
                    "pane_id": req["params"]["pane_id"], "terminal_id": "term_42",
                    "workspace_id": "w1", "tab_id": "w1:t1", "focused": true,
                    "agent_status": "working", "agent": "claude", "revision": 1,
                    "cwd": "/", "foreground_cwd": cwd.clone()
                }}),
                _ => json!({"type": "ok"}),
            };
            let mut s = stream;
            let _ = writeln!(s, "{}", json!({"id": req["id"], "result": result}));
        }
    });
    sock
}

#[test]
fn keys_by_terminal_id_and_follows_foreground_cwd() {
    let repo = repo_with(&[("m.py", "y = 1\n")]);
    let sockdir = TempDir::new("sock");
    let sock = fake_herdr(sockdir.path(), repo.path());
    let client = Client::new(&sock);
    let state = TempDir::new("hook-state4");
    let store = Store::new(state.path());
    // The context has no cwd for this pane: pane.get provides it.
    let out = hook::handle(&store, Some(&client), &event("w9:p9", "working"), None).unwrap();
    assert_eq!(out.action, "start");
    assert_eq!(out.pane_key, "term_42-claude");
    assert_eq!(out.repo.as_deref(), Some(repo.path()));
    // After a move the pane id changes but the terminal id does not.
    let out = hook::handle(&store, Some(&client), &event("w2:p1", "idle"), None).unwrap();
    assert_eq!(out.action, "end");
    assert_eq!(out.pane_key, "term_42-claude");
    let _ = std::fs::remove_file(&sock);
}

#[test]
fn the_binary_runs_as_herdr_runs_it() {
    let repo = repo_with(&[("app.py", "def f():\n    pass\n")]);
    let state = TempDir::new("hook-bin");
    let ctx_json = serde_json::to_string(&ctx("w1:p7", repo.path())).unwrap();
    let run = |status: &str| {
        let ev = json!({"event": "pane_agent_status_changed", "data": {
            "type": "pane_agent_status_changed", "pane_id": "w1:p7", "workspace_id": "w1",
            "agent_status": status, "agent": "claude", "title": null, "display_agent": null,
            "state_labels": {}}});
        let out = Command::new(env!("CARGO_BIN_EXE_codemorph"))
            .arg("hook")
            .current_dir(state.path())
            .env_remove("HERDR_SOCKET_PATH")
            .env("HERDR_ENV", "1")
            .env("HERDR_PLUGIN_EVENT", "pane.agent_status_changed")
            .env("HERDR_PLUGIN_EVENT_JSON", ev.to_string())
            .env("HERDR_PLUGIN_CONTEXT_JSON", &ctx_json)
            .env("HERDR_PLUGIN_STATE_DIR", state.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let t = std::time::Instant::now();
    assert!(run("working").starts_with("start w1_p7-claude turn 1"));
    write(repo.path(), "app.py", "def f():\n    return 2\n");
    assert!(run("idle").starts_with("end w1_p7-claude turn 1"));
    let elapsed = t.elapsed();
    // Two hook runs, each with a snapshot: well inside the 250 ms budget each.
    assert!(elapsed.as_millis() < 2_000, "hook took {elapsed:?}");
    assert!(state.path().join("panes/w1_p7-claude.json").is_file());
}
