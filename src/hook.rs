//! `codemorph hook`: herdr runs it on `pane.agent_status_changed`.
//!
//! A change to `working` starts a turn and a change from `working` to
//! `idle`, `done` or `blocked` ends it; each boundary snapshots the agent's
//! work tree into the shadow store. The event carries no previous status, so
//! the last status per pane lives in the state dir. Panes are keyed by their
//! terminal id and agent, because pane ids change when panes move.

use std::path::PathBuf;

use serde_json::Value;

use crate::git;
use crate::herdr::context::{is_settled, is_working, parse_status_event, AgentStatusEvent};
use crate::herdr::{Client, PluginContext};
use crate::store::{Checkpoint, PaneState, Store, Turn, MAX_TURNS};
use crate::util;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookOutcome {
    /// "start", "end", "reopen" or "none".
    pub action: &'static str,
    pub pane_key: String,
    pub turn: Option<u32>,
    pub checkpoint: Option<Checkpoint>,
    pub repo: Option<PathBuf>,
}

/// Entry point: read the event from the environment herdr set.
pub fn run() -> i32 {
    let Ok(json) = std::env::var("HERDR_PLUGIN_EVENT_JSON") else {
        eprintln!("codemorph hook: HERDR_PLUGIN_EVENT_JSON is not set");
        return 0;
    };
    let Some(event) = parse_status_event(&json) else {
        return 0;
    };
    let ctx = std::env::var("HERDR_PLUGIN_CONTEXT_JSON")
        .ok()
        .and_then(|j| PluginContext::parse(&j));
    let store = Store::from_env();
    let client = Client::from_env();
    match handle(&store, client.as_ref(), &event, ctx.as_ref()) {
        Ok(out) => {
            println!(
                "{} {} turn {} {}",
                out.action,
                out.pane_key,
                out.turn
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "-".into()),
                out.checkpoint.map(|c| c.commit).unwrap_or_default()
            );
            0
        }
        Err(e) => {
            eprintln!("codemorph hook: {e}");
            1
        }
    }
}

pub fn handle(
    store: &Store,
    client: Option<&Client>,
    event: &AgentStatusEvent,
    ctx: Option<&PluginContext>,
) -> Result<HookOutcome, String> {
    // Where is the agent working?
    let ctx_matches =
        ctx.is_some_and(|c| c.focused_pane_id.as_deref() == Some(event.pane_id.as_str()));
    let mut cwd: Option<String> = ctx
        .filter(|_| ctx_matches)
        .and_then(|c| c.focused_pane_cwd.clone());
    let mut terminal_id: Option<String> = None;
    let mut agent = event.agent.clone().or_else(|| {
        ctx.filter(|_| ctx_matches)
            .and_then(|c| c.focused_pane_agent.clone())
    });
    if let Some(client) = client {
        if let Ok(pane) = client.pane_get(&event.pane_id) {
            let s = |k: &str| pane.get(k).and_then(Value::as_str).map(str::to_string);
            cwd = s("foreground_cwd").or(cwd).or_else(|| s("cwd"));
            terminal_id = s("terminal_id");
            agent = agent.or_else(|| s("agent"));
        }
    }
    if cwd.is_none() {
        cwd = ctx.and_then(|c| c.worktree.as_ref().map(|w| w.checkout_path.clone()));
    }
    let key = PaneState::key_for(terminal_id.as_deref(), &event.pane_id, agent.as_deref());
    let mut state = store.load_pane(&key);
    state.pane_id = event.pane_id.clone();
    if terminal_id.is_some() {
        state.terminal_id = terminal_id;
    }
    if agent.is_some() {
        state.agent = agent;
    }
    let prev = state.last_status.clone();
    let new = event.agent_status.clone();
    state.last_status = Some(new.clone());
    state.updated = util::now_unix();

    let mut out = HookOutcome {
        action: "none",
        pane_key: key.clone(),
        turn: None,
        checkpoint: None,
        repo: None,
    };
    if prev.as_deref() == Some(new.as_str()) {
        store.save_pane(&state).map_err(|e| e.to_string())?;
        return Ok(out);
    }
    let repo = cwd
        .as_deref()
        .and_then(|d| git::discover(std::path::Path::new(d)).ok());
    let Some(repo) = repo else {
        // Not in a repository: remember the status, nothing to snapshot.
        store.save_pane(&state).map_err(|e| e.to_string())?;
        return Ok(out);
    };
    out.repo = Some(repo.root.clone());
    let root_s = repo.root.to_string_lossy().into_owned();
    let shadow = store.shadow(&repo);
    let was_working = prev.as_deref().is_some_and(is_working);

    if is_working(&new) && !was_working {
        let reopen = prev.as_deref() == Some("blocked")
            && state.turns.last().is_some_and(|t| {
                t.repo_root == root_s && t.end_status.as_deref() == Some("blocked")
            });
        if reopen {
            // Answering a question continues the same turn.
            let t = state.turns.last_mut().unwrap();
            t.open = true;
            out.action = "reopen";
            out.turn = Some(t.n);
        } else {
            let n = state.turns.last().map(|t| t.n + 1).unwrap_or(1);
            let cp = shadow
                .snapshot(&format!("codeMorph: {key} turn {n} start"))
                .map_err(|e| e.0)?;
            let _ = shadow.update_ref(&format!("refs/codemorph/{key}/{n}-start"), &cp.commit);
            state.turns.push(Turn {
                n,
                repo_root: root_s,
                start: Some(cp.clone()),
                end: None,
                end_status: None,
                open: true,
            });
            out.action = "start";
            out.turn = Some(n);
            out.checkpoint = Some(cp);
        }
    } else if was_working && is_settled(&new) {
        let open = state
            .turns
            .iter_mut()
            .rev()
            .find(|t| t.open && t.repo_root == root_s);
        if let Some(t) = open {
            let n = t.n;
            let cp = shadow
                .snapshot(&format!("codeMorph: {key} turn {n} end"))
                .map_err(|e| e.0)?;
            let _ = shadow.update_ref(&format!("refs/codemorph/{key}/{n}-end"), &cp.commit);
            t.end = Some(cp.clone());
            t.end_status = Some(new.clone());
            t.open = false;
            out.action = "end";
            out.turn = Some(n);
            out.checkpoint = Some(cp);
        }
    }
    // Retention: keep the last MAX_TURNS turns.
    while state.turns.len() > MAX_TURNS {
        let old = state.turns.remove(0);
        let _ = shadow.delete_ref(&format!("refs/codemorph/{key}/{}-start", old.n));
        let _ = shadow.delete_ref(&format!("refs/codemorph/{key}/{}-end", old.n));
    }
    store.save_pane(&state).map_err(|e| e.to_string())?;
    Ok(out)
}
