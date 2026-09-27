//! What herdr tells a plugin command about where it was invoked:
//! `HERDR_PLUGIN_CONTEXT_JSON` (PluginInvocationContext) and, for event hooks,
//! `HERDR_PLUGIN_EVENT_JSON` (an EventEnvelope).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `WorkspaceWorktreeInfo` in the 0.9.0 schema.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeCtx {
    #[serde(default)]
    pub repo_key: String,
    #[serde(default)]
    pub repo_name: String,
    #[serde(default)]
    pub repo_root: String,
    #[serde(default)]
    pub checkout_path: String,
    #[serde(default)]
    pub is_linked_worktree: bool,
}

/// `PluginInvocationContext` in the 0.9.0 schema. Every field is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginContext {
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub workspace_label: Option<String>,
    #[serde(default)]
    pub workspace_cwd: Option<String>,
    #[serde(default)]
    pub worktree: Option<WorktreeCtx>,
    #[serde(default)]
    pub tab_id: Option<String>,
    #[serde(default)]
    pub tab_label: Option<String>,
    #[serde(default)]
    pub focused_pane_id: Option<String>,
    #[serde(default)]
    pub focused_pane_cwd: Option<String>,
    #[serde(default)]
    pub focused_pane_agent: Option<String>,
    #[serde(default)]
    pub focused_pane_status: Option<String>,
    #[serde(default)]
    pub selected_text: Option<String>,
    #[serde(default)]
    pub invocation_source: Option<String>,
    #[serde(default)]
    pub correlation_id: Option<String>,
}

impl PluginContext {
    pub fn parse(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok()
    }

    /// Read the context herdr injected, preferring the copy `codemap open`
    /// forwarded to the popup (it was taken at the moment of the keypress).
    pub fn from_env() -> Option<Self> {
        for key in ["CODEMAP_CONTEXT_JSON", "HERDR_PLUGIN_CONTEXT_JSON"] {
            if let Ok(json) = std::env::var(key) {
                if let Some(ctx) = Self::parse(&json) {
                    return Some(ctx);
                }
            }
        }
        None
    }

    /// The directory to open on, best first: the focused pane's cwd, the
    /// herdr worktree checkout, then the workspace cwd.
    pub fn candidate_dirs(&self) -> Vec<String> {
        let mut dirs = Vec::new();
        if let Some(cwd) = &self.focused_pane_cwd {
            dirs.push(cwd.clone());
        }
        if let Some(wt) = &self.worktree {
            if !wt.checkout_path.is_empty() {
                dirs.push(wt.checkout_path.clone());
            }
        }
        if let Some(cwd) = &self.workspace_cwd {
            dirs.push(cwd.clone());
        }
        dirs.retain(|d| !d.is_empty());
        dirs.dedup();
        dirs
    }

    pub fn has_agent(&self) -> bool {
        self.focused_pane_agent
            .as_deref()
            .is_some_and(|a| !a.is_empty())
    }
}

/// The part of an event we care about: which pane changed to which status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStatusEvent {
    pub pane_id: String,
    pub workspace_id: Option<String>,
    pub agent_status: String,
    pub agent: Option<String>,
}

/// Parse `HERDR_PLUGIN_EVENT_JSON`. herdr serialises an `EventEnvelope`
/// (`{"event": ..., "data": {"type": "pane_agent_status_changed", ...}}`);
/// subscription streams wrap the same data. Be lenient about the wrapper.
pub fn parse_status_event(json: &str) -> Option<AgentStatusEvent> {
    let value: Value = serde_json::from_str(json).ok()?;
    find_status_payload(&value)
}

fn find_status_payload(value: &Value) -> Option<AgentStatusEvent> {
    if let (Some(pane_id), Some(status)) = (
        value.get("pane_id").and_then(Value::as_str),
        value.get("agent_status").and_then(Value::as_str),
    ) {
        return Some(AgentStatusEvent {
            pane_id: pane_id.to_string(),
            workspace_id: value
                .get("workspace_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            agent_status: status.to_string(),
            agent: value
                .get("agent")
                .and_then(Value::as_str)
                .map(str::to_string),
        });
    }
    for key in ["data", "event", "result"] {
        if let Some(inner) = value.get(key) {
            if inner.is_object() {
                if let Some(found) = find_status_payload(inner) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// Status names as herdr reports them.
pub fn is_working(status: &str) -> bool {
    status == "working"
}

pub fn is_settled(status: &str) -> bool {
    matches!(status, "idle" | "done" | "blocked")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_context() {
        let json = r#"{
            "workspace_id":"w1","workspace_label":"paneMorph","workspace_cwd":"/tmp/pm",
            "worktree":{"repo_key":"k","repo_name":"paneMorph","repo_root":"/tmp/pm",
                        "checkout_path":"/tmp/pm-wt","is_linked_worktree":true},
            "tab_id":"w1:t1","tab_label":"selector-fix",
            "focused_pane_id":"w1:p2","focused_pane_cwd":"/tmp/pm-wt/src",
            "focused_pane_agent":"claude","focused_pane_status":"idle",
            "selected_text":null,"invocation_source":"keybinding","correlation_id":"x",
            "clicked_url":null,"link_handler_id":null
        }"#;
        let ctx = PluginContext::parse(json).unwrap();
        assert_eq!(ctx.focused_pane_id.as_deref(), Some("w1:p2"));
        assert!(ctx.has_agent());
        assert_eq!(
            ctx.candidate_dirs(),
            vec!["/tmp/pm-wt/src", "/tmp/pm-wt", "/tmp/pm"]
        );
        assert!(ctx.worktree.unwrap().is_linked_worktree);
    }

    #[test]
    fn parses_sparse_context() {
        let ctx = PluginContext::parse(r#"{"invocation_source":"api"}"#).unwrap();
        assert!(!ctx.has_agent());
        assert!(ctx.candidate_dirs().is_empty());
    }

    #[test]
    fn parses_event_envelope() {
        let json = r#"{"event":"pane_agent_status_changed","data":{"type":"pane_agent_status_changed",
            "pane_id":"w1:p2","workspace_id":"w1","agent_status":"working","agent":"claude",
            "title":null,"display_agent":null,"state_labels":{}}}"#;
        let ev = parse_status_event(json).unwrap();
        assert_eq!(ev.pane_id, "w1:p2");
        assert_eq!(ev.agent_status, "working");
        assert_eq!(ev.agent.as_deref(), Some("claude"));
        assert!(is_working(&ev.agent_status));
    }

    #[test]
    fn parses_flat_and_dotted_event() {
        let json = r#"{"event":"pane.agent_status_changed","data":{"pane_id":"w2:p1","workspace_id":"w2","agent_status":"idle"}}"#;
        let ev = parse_status_event(json).unwrap();
        assert!(is_settled(&ev.agent_status));
        assert!(
            parse_status_event(r#"{"event":"pane.focused","data":{"pane_id":"w1:p1"}}"#).is_none()
        );
    }
}
