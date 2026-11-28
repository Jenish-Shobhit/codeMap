use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::app::{App, Location, Msg, View};
use crate::git::worktree::{self, Checkout};
use crate::herdr::{Client, PluginContext};
use crate::store::Store;
use crate::util;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    pub pane_id: Option<String>,
    pub kind: Option<String>,
    pub name: String,
    pub status: Option<String>,
    pub workspace_id: Option<String>,
    pub tab_id: Option<String>,
}

fn text(listed: &Value, field: &str) -> Option<String> {
    listed
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn folder_name(path: &Path) -> Option<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

impl Agent {
    fn listed(listed: &Value) -> Agent {
        let kind = text(listed, "agent");
        Agent {
            pane_id: text(listed, "pane_id"),
            name: text(listed, "display_agent")
                .or_else(|| text(listed, "name"))
                .or_else(|| kind.clone())
                .unwrap_or_else(|| "agent".into()),
            kind,
            status: text(listed, "agent_status"),
            workspace_id: text(listed, "workspace_id"),
            tab_id: text(listed, "tab_id"),
        }
    }

    pub fn is_working(&self) -> bool {
        self.status.as_deref() == Some("working")
    }

    pub fn context(&self, worktree: &Path) -> PluginContext {
        PluginContext {
            workspace_id: self.workspace_id.clone(),
            workspace_label: folder_name(worktree),
            tab_id: self.tab_id.clone(),
            tab_label: Some(self.name.clone()),
            focused_pane_id: self.pane_id.clone(),
            focused_pane_cwd: Some(worktree.display().to_string()),
            focused_pane_agent: self.kind.clone(),
            focused_pane_status: self.status.clone(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub checkout: Checkout,
    pub agents: Vec<Agent>,
}

impl Entry {
    pub fn adopted(&self) -> Option<&Agent> {
        let mut with_a_pane = self.agents.iter().filter(|agent| agent.pane_id.is_some());
        with_a_pane
            .clone()
            .find(|agent| agent.is_working())
            .or_else(|| with_a_pane.next())
    }
}

fn working_directory(listed: &Value) -> Option<PathBuf> {
    let cwd = text(listed, "foreground_cwd").or_else(|| text(listed, "cwd"))?;
    let cwd = PathBuf::from(cwd);
    Some(cwd.canonicalize().unwrap_or(cwd))
}

fn innermost(checkouts: &[Checkout], cwd: &Path) -> Option<usize> {
    (0..checkouts.len())
        .filter(|at| cwd.starts_with(&checkouts[*at].path))
        .max_by_key(|at| checkouts[*at].path.components().count())
}

pub fn with_agents(checkouts: Vec<Checkout>, listed: &[Value]) -> Vec<Entry> {
    let mut agents: Vec<Vec<Agent>> = vec![Vec::new(); checkouts.len()];
    for agent in listed {
        let inside = working_directory(agent).and_then(|cwd| innermost(&checkouts, &cwd));
        if let Some(at) = inside {
            agents[at].push(Agent::listed(agent));
        }
    }
    checkouts
        .into_iter()
        .zip(agents)
        .map(|(checkout, agents)| Entry { checkout, agents })
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    pub entries: Vec<Entry>,
    pub agents_known: bool,
}

pub fn read(root: &Path, store: &Store, client: Option<&Client>) -> Result<Listing, String> {
    let checkouts = worktree::read(root, &|info| store.shadow(info).dir).map_err(|e| e.0)?;
    let listed = client.and_then(|client| client.agent_list().ok());
    Ok(Listing {
        agents_known: listed.is_some(),
        entries: with_agents(checkouts, &listed.unwrap_or_default()),
    })
}

#[derive(Debug, Default)]
pub struct WorktreesState {
    pub entries: Vec<Entry>,
    pub agents_known: bool,
    pub loaded: bool,
    pub error: Option<String>,
    pub sel: usize,
    pub scroll: usize,
    pub asks: u64,
    pub outdated: bool,
}

impl WorktreesState {
    fn at(&self, path: &Path) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.checkout.path == path)
    }

    pub fn load(&mut self, read: Result<Listing, String>, showing: &Path) {
        let kept = self.selected().map(|entry| entry.checkout.path.clone());
        match read {
            Ok(listing) => {
                self.entries = listing.entries;
                self.agents_known = listing.agents_known;
                self.error = None;
            }
            Err(e) => {
                self.entries.clear();
                self.agents_known = false;
                self.error = Some(e);
            }
        }
        self.loaded = true;
        let wanted = kept
            .and_then(|path| self.at(&path))
            .or_else(|| self.at(showing));
        self.sel = wanted
            .unwrap_or(self.sel)
            .min(self.entries.len().saturating_sub(1));
    }

    pub fn selected(&self) -> Option<&Entry> {
        self.entries.get(self.sel)
    }

    pub fn move_by(&mut self, delta: i32) {
        let last = self.entries.len().saturating_sub(1) as i64;
        self.sel = (self.sel as i64 + delta as i64).clamp(0, last) as usize;
    }

    pub fn location(&self) -> Option<Location> {
        let checkout = &self.selected()?.checkout;
        (!checkout.is_missing()).then(|| Location {
            path: checkout.path.display().to_string(),
            line: None,
            dir: true,
        })
    }

    pub fn carried_to_another_worktree(self) -> WorktreesState {
        WorktreesState {
            outdated: true,
            ..self
        }
    }
}

const NAMED_IN_A_MESSAGE: usize = 40;

pub fn shown_path(path: &Path) -> String {
    util::sanitize_line(&util::tilde(path))
}

fn named(path: &Path) -> String {
    util::truncate_left(&shown_path(path), NAMED_IN_A_MESSAGE)
}

impl App {
    pub fn worktrees_shown(&mut self) {
        let asked = self.worktrees.asks > 0 && !self.worktrees.outdated;
        if !asked {
            self.worktrees_entered();
        }
    }

    pub fn worktrees_entered(&mut self) {
        if self.view != View::Worktrees {
            return;
        }
        let Some(repo) = self.repo.clone() else {
            return;
        };
        self.worktrees.asks += 1;
        self.worktrees.outdated = false;
        let ask = self.worktrees.asks;
        let store = self.store.clone();
        let client = self.client.clone();
        self.spawn(move |tx| {
            let read = read(&repo.root, &store, client.as_ref());
            let _ = tx.send(Msg::Worktrees { ask, read });
        });
    }

    pub fn worktrees_read(&mut self, ask: u64, read: Result<Listing, String>) {
        if ask != self.worktrees.asks {
            return;
        }
        let showing = self.root.clone();
        self.worktrees.load(read, &showing);
    }

    pub fn worktrees_move(&mut self, delta: i32) {
        self.worktrees.move_by(delta);
    }

    pub fn shows(&self, checkout: &Checkout) -> bool {
        checkout.path == self.root
    }

    pub fn show_selected_worktree(&mut self) {
        let Some(entry) = self.worktrees.selected() else {
            return;
        };
        let path = entry.checkout.path.clone();
        let named = named(&path);
        let refused = if self.shows(&entry.checkout) {
            Some(format!("already showing {named}"))
        } else if entry.checkout.is_missing() || !path.is_dir() {
            Some(format!("{named} is missing · nothing to show"))
        } else {
            None
        };
        if refused.is_some() {
            self.message = refused;
            return;
        }
        let followed = entry.adopted().cloned();
        let context = followed.as_ref().map(|agent| agent.context(&path));
        let worktrees = std::mem::take(&mut self.worktrees).carried_to_another_worktree();
        self.show_worktree(path, context, worktrees);
        self.message = Some(match followed {
            Some(agent) => format!(
                "every tab now shows {named} · following {}",
                util::sanitize_line(&agent.name)
            ),
            None => format!("every tab now shows {named}"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::worktree::Changes;
    use serde_json::json;

    fn checkout(path: &str) -> Checkout {
        Checkout {
            path: PathBuf::from(path),
            head: Some("a384f27ac8b1".into()),
            branch: Some("main".into()),
            upstream: None,
            locked: None,
            prunable: None,
            changes: Changes::Read(Default::default()),
        }
    }

    fn agent(name: &str, status: &str, pane: Option<&str>) -> Agent {
        Agent {
            pane_id: pane.map(str::to_string),
            kind: Some(name.into()),
            name: name.into(),
            status: Some(status.into()),
            workspace_id: None,
            tab_id: None,
        }
    }

    fn listing(paths: &[&str]) -> Result<Listing, String> {
        Ok(Listing {
            entries: with_agents(paths.iter().map(|p| checkout(p)).collect(), &[]),
            agents_known: true,
        })
    }

    fn on(state: &WorktreesState) -> String {
        state
            .selected()
            .map(|entry| entry.checkout.path.display().to_string())
            .unwrap_or_default()
    }

    const NOWHERE: &str = "/codemap-test/nowhere";

    #[test]
    fn an_agent_belongs_to_the_innermost_worktree_around_its_folder() {
        let checkouts = vec![
            checkout("/codemap-test/repo"),
            checkout("/codemap-test/repo/.worktrees/inner"),
            checkout("/codemap-test/beside"),
        ];
        let listed = [
            json!({"pane_id": "w1:p1", "agent": "claude", "agent_status": "idle", "cwd": "/codemap-test/repo"}),
            json!({"pane_id": "w1:p2", "agent": "codex", "name": "reviewer", "agent_status": "working", "cwd": "/", "foreground_cwd": "/codemap-test/repo/.worktrees/inner/src/ui"}),
            json!({"pane_id": "w1:p3", "agent": "pi", "display_agent": "Pi", "name": "helper", "agent_status": "blocked", "cwd": "/codemap-test/repo/docs"}),
            json!({"pane_id": "w1:p4", "agent": "claude", "agent_status": "working", "cwd": "/codemap-test/elsewhere"}),
            json!({"pane_id": "w1:p5", "agent": "claude", "agent_status": "working", "cwd": "/codemap-test/beside-the-point"}),
            json!({"pane_id": "w1:p6", "agent_status": "unknown"}),
            json!({"pane_id": "w1:p7", "cwd": "/codemap-test/beside", "workspace_id": "w1", "tab_id": "w1:t2"}),
        ];
        let entries = with_agents(checkouts, &listed);
        let names = |at: usize| -> Vec<&str> {
            entries[at]
                .agents
                .iter()
                .map(|agent| agent.name.as_str())
                .collect()
        };
        assert_eq!(names(0), ["claude", "Pi"]);
        assert_eq!(names(1), ["reviewer"]);
        assert_eq!(names(2), ["agent"]);
        assert!(entries[1].agents[0].is_working());
        assert_eq!(entries[1].agents[0].kind.as_deref(), Some("codex"));
        assert_eq!(entries[0].agents[1].status.as_deref(), Some("blocked"));
        let unnamed = &entries[2].agents[0];
        assert_eq!(
            (unnamed.kind.as_ref(), unnamed.status.as_ref()),
            (None, None)
        );
        assert_eq!(unnamed.tab_id.as_deref(), Some("w1:t2"));
        assert!(with_agents(Vec::new(), &listed).is_empty());
    }

    #[test]
    fn the_agent_to_follow_is_the_first_working_one_else_the_first() {
        let with = |agents: Vec<Agent>| Entry {
            checkout: checkout("/codemap-test/repo"),
            agents,
        };
        let pane = |entry: &Entry| entry.adopted().and_then(|agent| agent.pane_id.clone());
        assert_eq!(pane(&with(Vec::new())), None);
        let idle_then_working = with(vec![
            agent("claude", "idle", Some("w1:p1")),
            agent("codex", "working", Some("w1:p2")),
            agent("pi", "working", Some("w1:p3")),
        ]);
        assert_eq!(pane(&idle_then_working).as_deref(), Some("w1:p2"));
        let none_working = with(vec![
            agent("claude", "blocked", Some("w1:p1")),
            agent("codex", "idle", Some("w1:p2")),
        ]);
        assert_eq!(pane(&none_working).as_deref(), Some("w1:p1"));
        let without_a_pane = with(vec![
            agent("ghost", "working", None),
            agent("claude", "idle", Some("w1:p9")),
        ]);
        assert_eq!(pane(&without_a_pane).as_deref(), Some("w1:p9"));
    }

    #[test]
    fn the_followed_agent_is_described_as_herdr_describes_a_focused_pane() {
        let followed = Agent {
            workspace_id: Some("w2".into()),
            tab_id: Some("w2:t1".into()),
            name: "reviewer".into(),
            ..agent("codex", "working", Some("w2:p1"))
        };
        let context = followed.context(Path::new("/code/paneMorph-docs"));
        assert_eq!(context.focused_pane_id.as_deref(), Some("w2:p1"));
        assert_eq!(
            context.focused_pane_cwd.as_deref(),
            Some("/code/paneMorph-docs")
        );
        assert_eq!(context.focused_pane_agent.as_deref(), Some("codex"));
        assert_eq!(context.focused_pane_status.as_deref(), Some("working"));
        assert_eq!(
            (context.workspace_id.as_deref(), context.tab_id.as_deref()),
            (Some("w2"), Some("w2:t1"))
        );
        let info = crate::app::AgentInfo::from_context(&context);
        assert_eq!(info.label().as_deref(), Some("paneMorph-docs · reviewer"));
        assert!(info.is_working());
    }

    #[test]
    fn the_list_opens_on_the_worktree_the_popup_shows() {
        let mut state = WorktreesState::default();
        state.load(
            listing(&["/code/repo", "/code/wt-a", "/code/wt-b"]),
            Path::new("/code/wt-b"),
        );
        assert_eq!((on(&state).as_str(), state.sel), ("/code/wt-b", 2));
        assert!(state.loaded && state.agents_known);
        let mut elsewhere = WorktreesState::default();
        elsewhere.load(listing(&["/code/repo", "/code/wt-a"]), Path::new(NOWHERE));
        assert_eq!(on(&elsewhere), "/code/repo");
    }

    #[test]
    fn a_later_read_keeps_the_selected_worktree_by_its_path() {
        let mut state = WorktreesState::default();
        state.load(
            listing(&["/code/repo", "/code/wt-a", "/code/wt-b"]),
            Path::new("/code/repo"),
        );
        state.move_by(1);
        state.load(
            listing(&["/code/repo", "/code/new", "/code/wt-a", "/code/wt-b"]),
            Path::new("/code/repo"),
        );
        assert_eq!(on(&state), "/code/wt-a", "by path, not by row");
        state.load(
            listing(&["/code/repo", "/code/wt-b"]),
            Path::new("/code/repo"),
        );
        assert_eq!(
            on(&state),
            "/code/repo",
            "a removed one gives way to the shown one"
        );
        state.move_by(1);
        state.load(listing(&["/code/other"]), Path::new(NOWHERE));
        assert_eq!(on(&state), "/code/other");
        state.load(listing(&[]), Path::new(NOWHERE));
        assert_eq!((state.sel, state.selected()), (0, None));
    }

    #[test]
    fn the_selection_stops_at_both_ends() {
        let mut state = WorktreesState::default();
        state.move_by(3);
        assert_eq!(state.sel, 0);
        state.load(
            listing(&["/code/repo", "/code/wt-a", "/code/wt-b"]),
            Path::new("/code/repo"),
        );
        state.move_by(i32::MAX / 2);
        assert_eq!(on(&state), "/code/wt-b");
        state.move_by(-(i32::MAX / 2));
        assert_eq!(on(&state), "/code/repo");
    }

    #[test]
    fn a_read_that_fails_keeps_the_error_and_no_worktrees() {
        let mut state = WorktreesState::default();
        state.load(
            listing(&["/code/repo", "/code/wt-a"]),
            Path::new("/code/repo"),
        );
        state.move_by(1);
        state.load(Err("not a git repository".into()), Path::new("/code/repo"));
        assert!(state.loaded && state.entries.is_empty() && !state.agents_known);
        assert_eq!(state.error.as_deref(), Some("not a git repository"));
        assert_eq!((state.sel, state.location()), (0, None));
    }

    #[test]
    fn the_folder_of_a_worktree_is_a_place_to_open_unless_it_is_missing() {
        let mut state = WorktreesState::default();
        state.load(
            listing(&["/code/repo", "/code/gone"]),
            Path::new("/code/repo"),
        );
        assert_eq!(
            state.location(),
            Some(Location {
                path: "/code/repo".into(),
                line: None,
                dir: true
            })
        );
        state.entries[1].checkout.changes = Changes::Missing;
        state.move_by(1);
        assert_eq!(state.location(), None);
    }

    #[test]
    fn a_list_carried_to_another_worktree_is_read_again_there() {
        let mut state = WorktreesState::default();
        state.load(
            listing(&["/code/repo", "/code/wt-a"]),
            Path::new("/code/repo"),
        );
        state.asks = 1;
        state.move_by(1);
        let carried = state.carried_to_another_worktree();
        assert!(carried.outdated && carried.loaded);
        assert_eq!((carried.asks, on(&carried).as_str()), (1, "/code/wt-a"));
    }
}
