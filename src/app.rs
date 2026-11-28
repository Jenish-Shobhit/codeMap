use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;

use serde_json::Value;

use crate::files::{FilesState, MAX_FILES};
use crate::git::{self, graph, Commit, FileDiff, GraphRow, LineKind, RepoInfo};
use crate::herdr::{Client, PluginContext};
use crate::store::{Comment, PaneState, Reviews, Store};
use crate::theme::Theme;
use crate::util;

pub const HISTORY_LIMIT: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Files,
    Changes,
    History,
}

impl View {
    pub const ALL: [View; 3] = [View::Files, View::Changes, View::History];

    pub fn name(self) -> &'static str {
        match self {
            View::Files => "files",
            View::Changes => "changes",
            View::History => "history",
        }
    }

    pub fn parse(s: &str) -> Option<View> {
        match s.to_ascii_lowercase().as_str() {
            "files" => Some(View::Files),
            "changes" | "review" => Some(View::Changes),
            "history" | "graph" => Some(View::History),
            _ => None,
        }
    }

    pub fn tabs(self) -> &'static [View] {
        match self {
            View::Files => &[View::Files],
            View::Changes | View::History => &[View::Changes, View::History],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Turn,
    Session,
    Head,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentInfo {
    pub pane_id: Option<String>,
    pub terminal_id: Option<String>,
    pub agent: Option<String>,
    pub status: Option<String>,
    pub workspace_label: Option<String>,
    pub tab_label: Option<String>,
}

impl AgentInfo {
    pub fn from_context(ctx: &PluginContext) -> Self {
        AgentInfo {
            pane_id: ctx.focused_pane_id.clone(),
            terminal_id: None,
            agent: ctx.focused_pane_agent.clone().filter(|a| !a.is_empty()),
            status: ctx.focused_pane_status.clone(),
            workspace_label: ctx.workspace_label.clone(),
            tab_label: ctx.tab_label.clone(),
        }
    }

    pub fn label(&self) -> Option<String> {
        match (&self.workspace_label, &self.tab_label) {
            (Some(w), Some(t)) if w != t => Some(format!("{w} · {t}")),
            (Some(w), _) => Some(w.clone()),
            (None, Some(t)) => Some(t.clone()),
            _ => self.agent.clone(),
        }
    }

    pub fn is_working(&self) -> bool {
        self.status.as_deref() == Some("working")
    }

    pub fn has_agent(&self) -> bool {
        self.agent.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    pub path: Option<PathBuf>,
    pub view: View,
    pub context: Option<PluginContext>,
    pub client: Option<Client>,
    pub store: Store,
    pub theme: Theme,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeInfo {
    pub header: String,
    pub rail: String,
    pub noun: String,
    pub fallback_note: Option<String>,
    pub turn_n: Option<u32>,
    pub open: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentRow {
    pub pane_key: String,
    pub label: String,
    pub working: bool,
    pub files: usize,
    pub adds: usize,
    pub dels: usize,
    pub selected: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorktreeRow {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub agents: Vec<(String, bool)>,
    pub current: bool,
}

pub struct RepoLoad {
    pub root: PathBuf,
    pub repo: Result<RepoInfo, Option<String>>,
    pub agent: Option<AgentInfo>,
}

pub enum Msg {
    Repo(Box<RepoLoad>),
    Files {
        files: Vec<String>,
        truncated: bool,
    },
    Changes(Box<ChangeSet>),
    History {
        commits: Result<Vec<Commit>, String>,
        total: usize,
        worktrees: Vec<WorktreeRow>,
    },
    CommitDiff(String, Result<Vec<FileDiff>, String>),
    Notice(String),
}

pub struct ChangeSet {
    pub scope: Scope,
    pub pane_key: Option<String>,
    pub diffs: Result<Vec<FileDiff>, String>,
    pub info: ScopeInfo,
    pub agents: Vec<AgentRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Body,
    Rail,
}

#[derive(Debug, Default)]
pub struct ChangesState {
    pub file: usize,
    pub row: usize,
    pub scroll: usize,
    pub anchor: Option<usize>,
    pub focus: Focus,
}

#[derive(Debug, Default)]
pub struct HistoryState {
    pub commits: Vec<Commit>,
    pub rows: Vec<GraphRow>,
    pub total: usize,
    pub sel: usize,
    pub scroll: usize,
    pub worktrees: Vec<WorktreeRow>,
    pub error: Option<String>,
    pub detail: Option<(String, Vec<FileDiff>)>,
    pub show_diff: bool,
    pub diff_scroll: usize,
    pub loading_detail: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputKind {
    Filter,
    Comment {
        path: String,
        start: u32,
        end: u32,
        hunk: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub kind: InputKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub path: String,
    pub line: Option<u32>,
    pub dir: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CRow {
    Hunk(usize),
    Line(usize, usize),
    Comment(usize),
}

pub struct App {
    pub theme: Theme,
    pub view: View,
    pub store: Store,
    pub client: Option<Client>,
    pub context: Option<PluginContext>,
    pub root: PathBuf,
    pub repo: Option<RepoInfo>,
    pub repo_known: bool,
    pub repo_error: Option<String>,
    pub agent: Option<AgentInfo>,
    pub scope: Scope,
    pub scope_info: ScopeInfo,
    pub pane_key: Option<String>,
    pub diffs: Vec<FileDiff>,
    pub diffs_loaded: bool,
    pub diff_error: Option<String>,
    pub agents: Vec<AgentRow>,
    pub files: FilesState,
    pub changes: ChangesState,
    pub history: HistoryState,
    pub history_loaded: bool,
    pub reviews: Reviews,
    pub drafts: Vec<Comment>,
    pub input: Option<Input>,
    pub message: Option<String>,
    pub show_help: bool,
    pub outcome: Option<Outcome>,
    pub started: Instant,
    pub first_paint_ms: Option<f64>,
    pub pinned: bool,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    sync: bool,
}

impl App {
    pub fn new(opts: Options) -> App {
        let (tx, rx) = mpsc::channel();
        let agent = opts.context.as_ref().map(AgentInfo::from_context);
        let root = opts
            .path
            .clone()
            .or_else(|| {
                opts.context.as_ref().and_then(|c| {
                    c.candidate_dirs()
                        .into_iter()
                        .map(PathBuf::from)
                        .find(|p| p.is_dir())
                })
            })
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let root = root.canonicalize().unwrap_or(root);
        let scope = if agent.as_ref().is_some_and(|a| a.pane_id.is_some()) {
            Scope::Turn
        } else {
            Scope::Head
        };
        App {
            theme: opts.theme,
            view: opts.view,
            store: opts.store,
            client: opts.client,
            context: opts.context,
            root,
            repo: None,
            repo_known: false,
            repo_error: None,
            agent,
            scope,
            scope_info: ScopeInfo::default(),
            pane_key: None,
            diffs: Vec::new(),
            diffs_loaded: false,
            diff_error: None,
            agents: Vec::new(),
            files: FilesState {
                focus: Focus::Rail,
                ..Default::default()
            },
            changes: ChangesState::default(),
            history: HistoryState::default(),
            history_loaded: false,
            reviews: Reviews::default(),
            drafts: Vec::new(),
            input: None,
            message: None,
            show_help: false,
            outcome: None,
            started: Instant::now(),
            first_paint_ms: None,
            pinned: false,
            tx,
            rx,
            sync: false,
        }
    }

    pub fn repo_key(&self) -> String {
        Store::repo_key(&self.root)
    }

    pub fn not_a_repo(&self) -> bool {
        self.repo_known && self.repo.is_none()
    }

    pub fn is_loading(&self) -> bool {
        !self.repo_known || (self.repo.is_some() && !self.diffs_loaded)
    }

    pub fn start(&mut self) {
        let tx = self.tx.clone();
        let root = self.root.clone();
        let client = self.client.clone();
        let agent = self.agent.clone();
        std::thread::spawn(move || load_repo(&tx, root, client, agent));
    }

    pub fn load_blocking(&mut self) {
        self.sync = true;
        let (tx, rx) = mpsc::channel();
        load_repo(
            &tx,
            self.root.clone(),
            self.client.clone(),
            self.agent.clone(),
        );
        drop(tx);
        let msgs: Vec<Msg> = rx.into_iter().collect();
        for m in msgs {
            self.apply(m);
        }
        self.drain();
    }

    pub fn drain(&mut self) {
        for _ in 0..16 {
            let msgs: Vec<Msg> = self.rx.try_iter().collect();
            if msgs.is_empty() {
                break;
            }
            for m in msgs {
                self.apply(m);
            }
        }
    }

    pub fn poll(&mut self) -> bool {
        let msgs: Vec<Msg> = self.rx.try_iter().collect();
        let changed = !msgs.is_empty();
        for m in msgs {
            self.apply(m);
        }
        changed
    }

    fn spawn<F: FnOnce(&Sender<Msg>) + Send + 'static>(&self, f: F) {
        let tx = self.tx.clone();
        if self.sync {
            f(&tx);
        } else {
            std::thread::spawn(move || f(&tx));
        }
    }

    pub fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::Repo(load) => {
                let RepoLoad { root, repo, agent } = *load;
                self.root = root;
                self.repo_known = true;
                match repo {
                    Ok(info) => self.repo = Some(info),
                    Err(e) => {
                        self.repo = None;
                        self.repo_error = e;
                        self.scope = Scope::Head;
                    }
                }
                if let Some(a) = agent {
                    self.agent = Some(a);
                }
                self.reviews = self.store.load_reviews(&self.repo_key());
                self.drafts = self.store.load_drafts(&self.repo_key());
                self.request_files();
                self.request_changes();
                self.request_history();
            }
            Msg::Files { files, truncated } => {
                self.files.tree = crate::files::Tree::new(files);
                self.files.truncated = truncated;
                self.files.loaded = true;
                self.place_files();
            }
            Msg::Changes(set) => {
                let set = *set;
                if set.scope != self.scope {
                    return;
                }
                self.pane_key = set.pane_key;
                self.scope_info = set.info;
                self.agents = set.agents;
                match set.diffs {
                    Ok(d) => {
                        self.diffs = d;
                        self.diff_error = None;
                    }
                    Err(e) => {
                        self.diffs = Vec::new();
                        self.diff_error = Some(e);
                    }
                }
                self.diffs_loaded = true;
                if self.changes.file >= self.diffs.len() {
                    self.changes = ChangesState {
                        focus: self.changes.focus,
                        ..Default::default()
                    };
                }
                self.place_files();
            }
            Msg::History {
                commits,
                total,
                worktrees,
            } => {
                self.history_loaded = true;
                match commits {
                    Ok(c) => {
                        self.history.rows = graph::layout(&c);
                        self.history.commits = c;
                        self.history.error = None;
                    }
                    Err(e) => self.history.error = Some(e),
                }
                self.history.total = total;
                self.history.worktrees = worktrees;
            }
            Msg::CommitDiff(sha, result) => {
                self.history.loading_detail = false;
                match result {
                    Ok(d) => self.history.detail = Some((sha, d)),
                    Err(e) => self.message = Some(e),
                }
            }
            Msg::Notice(s) => self.message = Some(s),
        }
    }

    pub fn request_files(&mut self) {
        let root = self.root.clone();
        let tracked = self.repo.is_some();
        self.spawn(move |tx| {
            let mut files = if tracked {
                git::list_files(&root).unwrap_or_default()
            } else {
                git::walk_files(&root, MAX_FILES + 1)
            };
            let truncated = files.len() > MAX_FILES;
            files.truncate(MAX_FILES);
            let _ = tx.send(Msg::Files { files, truncated });
        });
    }

    pub fn request_changes(&mut self) {
        self.diffs_loaded = false;
        let Some(repo) = self.repo.clone() else {
            self.diffs_loaded = true;
            return;
        };
        let scope = self.scope.clone();
        let store = self.store.clone();
        let agent = self.agent.clone();
        let wanted = self.pane_key.clone();
        self.spawn(move |tx| {
            let set = compute_changes(&store, &repo, &scope, agent.as_ref(), wanted);
            let _ = tx.send(Msg::Changes(Box::new(set)));
        });
    }

    pub fn request_history(&mut self) {
        let Some(repo) = self.repo.clone() else {
            self.history_loaded = true;
            return;
        };
        let client = self.client.clone();
        self.spawn(move |tx| {
            let (commits, total) = if repo.head.is_none() {
                (Ok(Vec::new()), 0)
            } else {
                (
                    graph::log(&repo.root, HISTORY_LIMIT).map_err(|e| e.0),
                    git::count_commits(&repo.root),
                )
            };
            let worktrees = worktree_rows(&repo, client.as_ref());
            let _ = tx.send(Msg::History {
                commits,
                total,
                worktrees,
            });
        });
    }

    pub fn request_commit_diff(&mut self, sha: String) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        self.history.loading_detail = true;
        self.spawn(move |tx| {
            let r = git::show_commit(&repo.root, &sha).map_err(|e| e.0);
            let _ = tx.send(Msg::CommitDiff(sha, r));
        });
    }

    pub fn set_scope(&mut self, scope: Scope) {
        if self.scope == scope {
            return;
        }
        self.scope = scope;
        self.changes = ChangesState {
            focus: self.changes.focus,
            ..Default::default()
        };
        self.request_changes();
        self.drain_if_sync();
    }

    pub fn cycle_scope(&mut self) {
        if self.repo.is_none() {
            return;
        }
        let next = match self.scope {
            Scope::Turn => Scope::Session,
            Scope::Session => Scope::Head,
            Scope::Head => Scope::Turn,
        };
        self.set_scope(next);
        self.message = Some(format!("scope: {}", scope_name(&self.scope)));
    }

    pub fn next_agent(&mut self) {
        if self.agents.len() < 2 {
            self.message = Some("no other agent has checkpoints in this repository".into());
            return;
        }
        let cur = self.agents.iter().position(|a| a.selected).unwrap_or(0);
        let next = &self.agents[(cur + 1) % self.agents.len()];
        self.pane_key = Some(next.pane_key.clone());
        if self.scope == Scope::Head {
            self.scope = Scope::Turn;
        }
        self.request_changes();
        self.drain_if_sync();
    }

    fn drain_if_sync(&mut self) {
        if self.sync {
            self.drain();
        }
    }

    pub fn change_rows(&self, file: usize) -> Vec<CRow> {
        let Some(f) = self.diffs.get(file) else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        for (hi, h) in f.hunks.iter().enumerate() {
            rows.push(CRow::Hunk(hi));
            for (li, l) in h.lines.iter().enumerate() {
                rows.push(CRow::Line(hi, li));
                if let Some(n) = l.new_no {
                    for (ci, c) in self.drafts.iter().enumerate() {
                        if c.path == f.path && c.end == n && l.kind != LineKind::Del {
                            rows.push(CRow::Comment(ci));
                        }
                    }
                }
            }
        }
        rows
    }

    fn hunk_at_row(&self, row: usize) -> Option<usize> {
        let rows = self.change_rows(self.changes.file);
        rows.iter().take(row + 1).rev().find_map(|r| match r {
            CRow::Hunk(h) | CRow::Line(h, _) => Some(*h),
            CRow::Comment(_) => None,
        })
    }

    pub fn changes_move(&mut self, delta: i32) {
        let n = self.change_rows(self.changes.file).len();
        if n == 0 {
            return;
        }
        self.changes.row = (self.changes.row as i32 + delta).clamp(0, n as i32 - 1) as usize;
    }

    pub fn select_file(&mut self, i: usize) {
        if i < self.diffs.len() {
            self.changes.file = i;
            self.changes.row = 0;
            self.changes.scroll = 0;
            self.changes.anchor = None;
        }
    }

    pub fn next_file(&mut self, delta: i32) {
        if self.diffs.is_empty() {
            return;
        }
        let n = self.diffs.len() as i32;
        let i = (self.changes.file as i32 + delta).clamp(0, n - 1) as usize;
        self.select_file(i);
    }

    pub fn next_hunk(&mut self, delta: i32) {
        let rows = self.change_rows(self.changes.file);
        let hunk_rows: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r, CRow::Hunk(_)))
            .map(|(i, _)| i)
            .collect();
        let cur = self.changes.row;
        let target = if delta > 0 {
            hunk_rows.iter().find(|&&r| r > cur).copied()
        } else {
            hunk_rows.iter().rev().find(|&&r| r < cur).copied()
        };
        match target {
            Some(r) => self.changes.row = r,
            None => {
                if delta > 0 && self.changes.file + 1 < self.diffs.len() {
                    self.select_file(self.changes.file + 1);
                } else if delta < 0 && self.changes.file > 0 {
                    self.select_file(self.changes.file - 1);
                    let rows = self.change_rows(self.changes.file);
                    self.changes.row = rows
                        .iter()
                        .rposition(|r| matches!(r, CRow::Hunk(_)))
                        .unwrap_or(0);
                }
            }
        }
    }

    pub fn toggle_reviewed(&mut self) {
        let Some(f) = self.diffs.get(self.changes.file) else {
            return;
        };
        let Some(hi) = self.hunk_at_row(self.changes.row) else {
            return;
        };
        let hash = f.hunks[hi].hash();
        let path = f.path.clone();
        let now = self.reviews.toggle(&path, &hash);
        let _ = self.store.save_reviews(&self.repo_key(), &self.reviews);
        if now {
            self.next_hunk(1);
        }
    }

    pub fn reviewed_count(&self, file: usize) -> (usize, usize) {
        let Some(f) = self.diffs.get(file) else {
            return (0, 0);
        };
        let done = f
            .hunks
            .iter()
            .filter(|h| self.reviews.is_reviewed(&f.path, &h.hash()))
            .count();
        (done, f.hunks.len())
    }

    fn line_range(&self, a: usize, b: usize) -> Option<(u32, u32, String)> {
        let f = self.diffs.get(self.changes.file)?;
        let rows = self.change_rows(self.changes.file);
        let (lo, hi) = (a.min(b), a.max(b));
        let mut lines = Vec::new();
        let mut hunk = None;
        for r in rows.iter().take(hi + 1).skip(lo) {
            if let CRow::Line(h, l) = r {
                hunk.get_or_insert(*h);
                let dl = &f.hunks[*h].lines[*l];
                let n = dl
                    .new_no
                    .or_else(|| f.hunks[*h].lines[*l..].iter().find_map(|x| x.new_no));
                if let Some(n) = n {
                    lines.push(n);
                }
            }
        }
        if lines.is_empty() {
            let h = self.hunk_at_row(lo)?;
            let hk = &f.hunks[h];
            return Some((hk.new_start.max(1), hk.new_end().max(1), hk.hash()));
        }
        let h = hunk.unwrap_or(0);
        Some((
            *lines.iter().min().unwrap(),
            *lines.iter().max().unwrap(),
            f.hunks[h].hash(),
        ))
    }

    pub fn start_comment(&mut self) {
        let Some(f) = self.diffs.get(self.changes.file) else {
            return;
        };
        let path = f.path.clone();
        let anchor = self.changes.anchor.unwrap_or(self.changes.row);
        let Some((start, end, hunk)) = self.line_range(anchor, self.changes.row) else {
            return;
        };
        self.input = Some(Input {
            kind: InputKind::Comment {
                path,
                start,
                end,
                hunk,
            },
            text: String::new(),
        });
    }

    pub fn add_comment(&mut self, path: String, start: u32, end: u32, hunk: String, text: String) {
        if text.trim().is_empty() {
            return;
        }
        self.drafts.push(Comment {
            path,
            start,
            end,
            text: text.trim().to_string(),
            hunk,
        });
        let _ = self.store.save_drafts(&self.repo_key(), &self.drafts);
        self.changes.anchor = None;
        self.message = Some(format!(
            "{} in the draft · P types it into the agent's prompt",
            util::plural(self.drafts.len(), "comment", "comments")
        ));
    }

    pub fn target_pane(&self) -> Option<String> {
        self.agent.as_ref().and_then(|a| a.pane_id.clone())
    }

    pub fn send_draft(&mut self, submit: bool) -> Result<(), String> {
        if self.drafts.is_empty() {
            return Err("no comments yet · c writes one".into());
        }
        let Some(client) = self.client.clone() else {
            return Err("not inside herdr · comments stay in the draft".into());
        };
        let Some(pane) = self.target_pane() else {
            return Err("focused pane has no agent".into());
        };
        let text = crate::store::format_comments(&self.drafts);
        let result = if submit {
            client.agent_prompt(&pane, &text)
        } else {
            client.send_text(&pane, &crate::herdr::paste_payload(&text))
        };
        match result {
            Ok(()) => {
                let n = self.drafts.len();
                self.drafts.clear();
                let _ = self.store.save_drafts(&self.repo_key(), &self.drafts);
                if self.pinned {
                    let _ = client.pane_focus(&pane);
                    self.message = Some(format!("sent {}", util::plural(n, "comment", "comments")));
                } else {
                    self.outcome = Some(Outcome::Quit);
                }
                Ok(())
            }
            Err(e) if e.code == "agent_blocked" => {
                Err("agent is waiting on a question · P pastes instead".into())
            }
            Err(e) if e.code == "agent_not_ready" || e.code == "agent_not_found" => Err(format!(
                "the agent is not ready ({}) · P pastes instead",
                e.code
            )),
            Err(e) => Err(format!("send failed: {e}")),
        }
    }

    pub fn delete_comment_at_cursor(&mut self) {
        let rows = self.change_rows(self.changes.file);
        if let Some(CRow::Comment(ci)) = rows.get(self.changes.row) {
            self.drafts.remove(*ci);
            let _ = self.store.save_drafts(&self.repo_key(), &self.drafts);
            self.message = Some("comment removed".into());
        }
    }

    pub fn location(&self) -> Option<Location> {
        match self.view {
            View::Files => self.file_location(),
            View::Changes => {
                let f = self.diffs.get(self.changes.file)?;
                let (start, _, _) = self
                    .line_range(self.changes.row, self.changes.row)
                    .unwrap_or((1, 1, String::new()));
                Some(Location {
                    path: f.path.clone(),
                    line: Some(start),
                    dir: false,
                })
            }
            View::History => None,
        }
    }

    pub fn history_open(&mut self) {
        let Some(c) = self.history.commits.get(self.history.sel) else {
            return;
        };
        let sha = c.sha.clone();
        self.history.show_diff = true;
        self.history.diff_scroll = 0;
        if self.history.detail.as_ref().map(|(s, _)| s) != Some(&sha) {
            self.request_commit_diff(sha);
            self.drain_if_sync();
        }
    }

    pub fn ensure_commit_detail(&mut self) {
        if self.history.loading_detail {
            return;
        }
        let Some(c) = self.history.commits.get(self.history.sel) else {
            return;
        };
        if self.history.detail.as_ref().map(|(s, _)| s) != Some(&c.sha) {
            let sha = c.sha.clone();
            self.request_commit_diff(sha);
            self.drain_if_sync();
        }
    }

    pub fn history_move(&mut self, delta: i32) {
        if self.history.commits.is_empty() {
            return;
        }
        let n = self.history.commits.len() as i32;
        self.history.sel = (self.history.sel as i32 + delta).clamp(0, n - 1) as usize;
        self.history.show_diff = false;
        let sha = self.history.commits[self.history.sel].sha.clone();
        if self.history.detail.as_ref().map(|(s, _)| s) != Some(&sha) {
            self.request_commit_diff(sha);
            self.drain_if_sync();
        }
    }
}

pub fn scope_name(s: &Scope) -> String {
    match s {
        Scope::Turn => "last turn".into(),
        Scope::Session => "this agent's session".into(),
        Scope::Head => "since HEAD".into(),
    }
}

fn load_repo(tx: &Sender<Msg>, root: PathBuf, client: Option<Client>, agent: Option<AgentInfo>) {
    let mut root = root;
    let mut agent = agent;
    if let (Some(client), Some(a)) = (&client, agent.as_mut()) {
        if let Some(pane_id) = a.pane_id.clone() {
            if let Ok(pane) = client.pane_get(&pane_id) {
                let cwd = pane
                    .get("foreground_cwd")
                    .and_then(Value::as_str)
                    .or_else(|| pane.get("cwd").and_then(Value::as_str))
                    .map(PathBuf::from);
                if let Some(cwd) = cwd.filter(|p| p.is_dir()) {
                    root = cwd;
                }
                a.terminal_id = pane
                    .get("terminal_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if a.agent.is_none() {
                    a.agent = pane
                        .get("agent")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                }
                if let Some(s) = pane.get("agent_status").and_then(Value::as_str) {
                    a.status = Some(s.to_string());
                }
            }
        }
    }
    let (repo, root) = match git::discover(&root) {
        Ok(info) => {
            let r = info.root.clone();
            (Ok(info), r)
        }
        Err(e) => {
            let root = root.canonicalize().unwrap_or(root);
            (Err(e.map(|e| e.0)), root)
        }
    };
    let _ = tx.send(Msg::Repo(Box::new(RepoLoad { root, repo, agent })));
}

pub fn compute_changes(
    store: &Store,
    repo: &RepoInfo,
    scope: &Scope,
    agent: Option<&AgentInfo>,
    wanted: Option<String>,
) -> ChangeSet {
    let panes = store.panes_for_repo(&repo.root);
    let focused_key = agent.and_then(|a| {
        panes
            .iter()
            .find(|p| p.matches_pane(a.pane_id.as_deref(), a.terminal_id.as_deref()))
            .map(|p| p.pane_key.clone())
    });
    let chosen: Option<&PaneState> = wanted
        .as_ref()
        .and_then(|k| panes.iter().find(|p| &p.pane_key == k))
        .or_else(|| {
            focused_key
                .as_ref()
                .and_then(|k| panes.iter().find(|p| &p.pane_key == k))
        })
        .or_else(|| {
            if agent.is_none_or(|a| a.pane_id.is_none()) {
                panes.first()
            } else {
                None
            }
        });
    let shadow = store.shadow(repo);
    let mut now_tree: Option<String> = None;
    let mut now = |shadow: &crate::store::Shadow| -> Option<String> {
        if now_tree.is_none() {
            now_tree = shadow.snapshot("codeMap: now").ok().map(|c| c.tree);
        }
        now_tree.clone()
    };

    let mut agents = Vec::new();
    for p in panes.iter().take(6) {
        let Some(t) = p.last_turn(&repo.root) else {
            continue;
        };
        let (Some(start), end) = (&t.start, &t.end) else {
            continue;
        };
        let target = if t.open {
            now(&shadow)
        } else {
            end.as_ref().map(|e| e.tree.clone())
        };
        let (files, adds, dels) = match target.and_then(|to| shadow.diff(&start.tree, &to).ok()) {
            Some(d) => (
                d.len(),
                d.iter().map(FileDiff::adds).sum(),
                d.iter().map(FileDiff::dels).sum(),
            ),
            None => (0, 0, 0),
        };
        let label = p.agent.clone().unwrap_or_else(|| "agent".into());
        let label = if p.pane_id.is_empty() {
            label
        } else {
            format!("{label} {}", p.pane_id)
        };
        agents.push(AgentRow {
            pane_key: p.pane_key.clone(),
            label,
            working: t.open,
            files,
            adds,
            dels,
            selected: chosen.is_some_and(|c| c.pane_key == p.pane_key),
        });
    }

    let mut info = ScopeInfo::default();
    let diffs: Result<Vec<FileDiff>, String> = match scope {
        Scope::Turn | Scope::Session => {
            let turn_pair = chosen.and_then(|p| {
                let last = p.last_turn(&repo.root)?;
                let first = if *scope == Scope::Session {
                    p.first_turn(&repo.root)?
                } else {
                    last
                };
                let start = first.start.clone()?;
                Some((p, first.clone(), last.clone(), start))
            });
            match turn_pair {
                Some((_p, first, last, start)) => {
                    let (to, to_label) = if last.open {
                        (now(&shadow), None)
                    } else {
                        (
                            last.end.as_ref().map(|e| e.tree.clone()),
                            last.end.as_ref().map(|e| e.at),
                        )
                    };
                    let when = to_label.unwrap_or(start.at);
                    info.turn_n = Some(last.n);
                    info.open = last.open;
                    info.header =
                        format!("turn {} · {}", last.n, util::ago(when, util::now_unix()));
                    info.noun = if *scope == Scope::Session {
                        "this session".into()
                    } else {
                        "this turn".into()
                    };
                    info.rail = if *scope == Scope::Session {
                        format!("session · turns {}–{}", first.n, last.n)
                    } else if last.open {
                        format!("last turn · working since {}", util::clock(start.at))
                    } else {
                        format!("last turn · done {}", util::clock(when))
                    };
                    match to {
                        Some(to) => shadow.diff(&start.tree, &to).map_err(|e| e.0),
                        None => Err("could not snapshot the work tree".into()),
                    }
                }
                None => {
                    info.fallback_note = Some("no turn checkpoint yet · showing since HEAD".into());
                    head_changes(store, repo, &mut info)
                }
            }
        }
        Scope::Head => head_changes(store, repo, &mut info),
    };
    ChangeSet {
        scope: scope.clone(),
        pane_key: chosen.map(|p| p.pane_key.clone()),
        diffs,
        info,
        agents,
    }
}

fn head_changes(
    store: &Store,
    repo: &RepoInfo,
    info: &mut ScopeInfo,
) -> Result<Vec<FileDiff>, String> {
    info.noun = "since HEAD".into();
    info.rail = match &repo.head {
        Some(h) => format!("since HEAD · {}", &h[..h.len().min(7)]),
        None => "no commits yet".into(),
    };
    if info.header.is_empty() {
        info.header = String::new();
    }
    let scratch = store.shadow(repo).dir;
    git::diff_head(repo, &scratch).map_err(|e| e.0)
}

fn worktree_rows(repo: &RepoInfo, client: Option<&Client>) -> Vec<WorktreeRow> {
    let list = git::worktrees(&repo.root).unwrap_or_default();
    let agents: Vec<Value> = client.and_then(|c| c.agent_list().ok()).unwrap_or_default();
    list.into_iter()
        .filter(|w| !w.bare)
        .map(|w| {
            let wp = w.path.canonicalize().unwrap_or(w.path.clone());
            let mut names = Vec::new();
            for a in &agents {
                let cwd = a
                    .get("foreground_cwd")
                    .and_then(Value::as_str)
                    .or_else(|| a.get("cwd").and_then(Value::as_str));
                let Some(cwd) = cwd else { continue };
                let cwd = PathBuf::from(cwd);
                let cwd = cwd.canonicalize().unwrap_or(cwd);
                if !cwd.starts_with(&wp) {
                    continue;
                }
                let label = a
                    .get("display_agent")
                    .and_then(Value::as_str)
                    .or_else(|| a.get("name").and_then(Value::as_str))
                    .or_else(|| a.get("agent").and_then(Value::as_str))
                    .unwrap_or("agent")
                    .to_string();
                let working = a.get("agent_status").and_then(Value::as_str) == Some("working");
                names.push((label, working));
            }
            WorktreeRow {
                current: wp == repo.root,
                path: w.path,
                branch: w.branch.or(w.head.map(|h| h[..h.len().min(7)].to_string())),
                agents: names,
            }
        })
        .collect()
}
