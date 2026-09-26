//! Application state, background loading and the actions behind the keys.
//!
//! The UI thread owns all state. Slow work (git, parsing, checkpoints) runs
//! on worker threads that send `Msg`s back, so the frame paints first and
//! content fills in as it arrives.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Instant, SystemTime};

use serde_json::Value;

use crate::canvas::Canvas;
use crate::flow::{self, FlowChart, FlowLayout};
use crate::git::{self, graph, Commit, FileDiff, FileStatus, GraphRow, LineKind, RepoInfo};
use crate::herdr::{Client, PluginContext};
use crate::index::{in_dir, parent_dir, Index, SearchHit, SymId};
use crate::lang::{self, FileSymbols, Lang};
use crate::layout;
use crate::map::{self, Cursor, Level, Marks, NodeKind, Scene};
use crate::store::{Comment, PaneState, Reviews, Store};
use crate::theme::Theme;
use crate::util;

/// Listing cap: bigger repositories are marked truncated.
pub const MAX_FILES: usize = 20_000;
/// Parse everything up front below this many source files; above it, parse a
/// folder when the map zooms into it.
pub const EAGER_PARSE: usize = 1_500;
pub const HISTORY_LIMIT: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Map,
    Flow,
    Changes,
    History,
}

impl View {
    pub const ALL: [View; 4] = [View::Map, View::Flow, View::Changes, View::History];

    pub fn name(self) -> &'static str {
        match self {
            View::Map => "map",
            View::Flow => "flow",
            View::Changes => "changes",
            View::History => "history",
        }
    }

    pub fn parse(s: &str) -> Option<View> {
        match s.to_ascii_lowercase().as_str() {
            "map" | "1" | "open" => Some(View::Map),
            "flow" | "2" => Some(View::Flow),
            "changes" | "review" | "3" => Some(View::Changes),
            "history" | "graph" | "4" => Some(View::History),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// The agent's last turn: start checkpoint to end (or to now if open).
    Turn,
    /// Everything since this agent's first checkpoint.
    Session,
    /// Working tree against HEAD.
    Head,
    /// One commit (from History).
    Commit(String),
}

/// The agent codeMorph follows (inside herdr).
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

/// Where and how to start.
#[derive(Debug, Clone)]
pub struct Options {
    pub path: Option<PathBuf>,
    pub view: View,
    pub context: Option<PluginContext>,
    pub client: Option<Client>,
    pub store: Store,
    pub theme: Theme,
}

/// Summary of the scope in effect, for the header and rails.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeInfo {
    /// "turn 3 · 2m" (header, right side)
    pub header: String,
    /// "last turn · done 9:44 PM" (rail footer)
    pub rail: String,
    /// "this turn" / "since HEAD" (rail section titles)
    pub noun: String,
    pub fallback_note: Option<String>,
    pub turn_n: Option<u32>,
    pub open: bool,
}

/// An agent with checkpoints in this repository, for the Changes rail.
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
    /// (agent label, working)
    pub agents: Vec<(String, bool)>,
    pub current: bool,
}

/// What repository discovery found.
pub struct RepoLoad {
    pub root: PathBuf,
    pub repo: Result<RepoInfo, Option<String>>,
    pub files: Vec<String>,
    pub truncated: bool,
    pub agent: Option<AgentInfo>,
}

pub enum Msg {
    Repo(Box<RepoLoad>),
    Parsed(Vec<(String, FileSymbols, (SystemTime, u64))>, bool),
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
    pub old_quals: BTreeMap<String, BTreeSet<String>>,
    pub info: ScopeInfo,
    pub agents: Vec<AgentRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Body,
    Rail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Select {
    Path(String),
    Symbol(String, usize),
}

#[derive(Debug, Default)]
pub struct MapState {
    pub level: Option<Level>,
    pub page: usize,
    pub scene: Option<Scene>,
    pub cursor: Cursor,
    pub scroll: (usize, usize),
    pub focus: Focus,
    pub rail_sel: usize,
    pub dirty: bool,
    pub pending_select: Option<Select>,
    pub built_width: usize,
    pub built_page: usize,
}

#[derive(Debug, Default)]
pub struct FlowState {
    pub target: Option<SymId>,
    pub back: Vec<SymId>,
    pub chart: Option<FlowChart>,
    pub layout: Option<FlowLayout>,
    pub selected: usize,
    pub scroll: (usize, usize),
    pub error: Option<String>,
    pub focus: Focus,
    pub rail_sel: usize,
    pub dirty: bool,
}

#[derive(Debug, Default)]
pub struct ChangesState {
    pub file: usize,
    /// Cursor row within the selected file's rows.
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
    /// Index into `commits`.
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
    Search,
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

/// What the process should do after the UI closes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Quit,
    Edit { path: PathBuf, line: u32 },
}

/// One row of the Changes body for the selected file.
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
    pub index: Index,
    pub files_loaded: bool,
    pub parse_pending: bool,
    pub scope: Scope,
    pub scope_info: ScopeInfo,
    pub pane_key: Option<String>,
    pub diffs: Vec<FileDiff>,
    pub diffs_loaded: bool,
    pub diff_error: Option<String>,
    pub old_quals: BTreeMap<String, BTreeSet<String>>,
    pub marks: Marks,
    pub agents: Vec<AgentRow>,
    pub map: MapState,
    pub flow: FlowState,
    pub changes: ChangesState,
    pub history: HistoryState,
    pub history_loaded: bool,
    pub reviews: Reviews,
    pub drafts: Vec<Comment>,
    pub input: Option<Input>,
    pub search_hits: Vec<SearchHit>,
    pub search_sel: usize,
    pub message: Option<String>,
    pub show_help: bool,
    pub outcome: Option<Outcome>,
    pub started: Instant,
    pub first_paint_ms: Option<f64>,
    /// Popup vs pinned split/tab (follow mode).
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
            index: Index::default(),
            files_loaded: false,
            parse_pending: false,
            scope,
            scope_info: ScopeInfo::default(),
            pane_key: None,
            diffs: Vec::new(),
            diffs_loaded: false,
            diff_error: None,
            old_quals: BTreeMap::new(),
            marks: Marks::default(),
            agents: Vec::new(),
            map: MapState {
                dirty: true,
                ..Default::default()
            },
            flow: FlowState::default(),
            changes: ChangesState::default(),
            history: HistoryState::default(),
            history_loaded: false,
            reviews: Reviews::default(),
            drafts: Vec::new(),
            input: None,
            search_hits: Vec::new(),
            search_sel: 0,
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
        !self.files_loaded || self.parse_pending || (self.repo.is_some() && !self.diffs_loaded)
    }

    /// Start background loading (the interactive path).
    pub fn start(&mut self) {
        let tx = self.tx.clone();
        let root = self.root.clone();
        let client = self.client.clone();
        let agent = self.agent.clone();
        std::thread::spawn(move || load_repo(&tx, root, client, agent));
    }

    /// Load everything synchronously (tests and snapshots).
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

    /// Apply queued messages; in sync mode keep going until quiet.
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

    /// Process worker messages; true if anything changed.
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
                let RepoLoad {
                    root,
                    repo,
                    files,
                    truncated,
                    agent,
                } = *load;
                self.root = root.clone();
                self.repo_known = true;
                match repo {
                    Ok(info) => self.repo = Some(info),
                    Err(e) => {
                        self.repo = None;
                        self.repo_error = e;
                        if self.scope != Scope::Head {
                            self.scope = Scope::Head;
                        }
                    }
                }
                if let Some(a) = agent {
                    self.agent = Some(a);
                }
                let mut index = Index::new(&root, files);
                index.truncated = truncated;
                self.index = index;
                self.files_loaded = true;
                self.reviews = self.store.load_reviews(&self.repo_key());
                self.drafts = self.store.load_drafts(&self.repo_key());
                if self.map.level.is_none() {
                    self.map.level = Some(Level::Dir(self.initial_dir()));
                }
                self.map.dirty = true;
                self.flow.dirty = true;
                self.request_parse_initial();
                self.request_changes();
                self.request_history();
            }
            Msg::Parsed(items, done) => {
                for (file, syms, stamp) in items {
                    self.index.insert_parsed(&file, syms, stamp);
                }
                self.index.resolve();
                if done {
                    self.parse_pending = false;
                }
                self.recompute_marks();
                self.map.dirty = true;
                self.flow.dirty = true;
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
                self.old_quals = set.old_quals;
                self.diffs_loaded = true;
                self.recompute_marks();
                if self.changes.file >= self.diffs.len() {
                    self.changes = ChangesState {
                        focus: self.changes.focus,
                        ..Default::default()
                    };
                }
                self.map.dirty = true;
                self.flow.dirty = true;
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
                if self.history.detail.is_none() {
                    if let Some(c) = self.history.commits.get(self.history.sel) {
                        let sha = c.sha.clone();
                        self.request_commit_diff(sha);
                    }
                }
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

    fn initial_dir(&self) -> String {
        let start = self
            .context
            .as_ref()
            .and_then(|c| c.focused_pane_cwd.clone())
            .map(PathBuf::from);
        if let Some(cwd) = start {
            let cwd = cwd.canonicalize().unwrap_or(cwd);
            if let Ok(rel) = cwd.strip_prefix(&self.root) {
                let rel = rel.to_string_lossy().replace('\\', "/");
                if !rel.is_empty() && self.index.files.iter().any(|f| in_dir(f, &rel)) {
                    return rel;
                }
            }
        }
        // A repository whose root holds one folder of code opens inside it.
        let subs = self.index.subdirs("");
        let root_code = self
            .index
            .files_in("")
            .iter()
            .filter(|f| Lang::from_path(f).is_some())
            .count();
        let code_subs: Vec<&(String, usize)> = subs
            .iter()
            .filter(|(d, _)| {
                self.index
                    .files
                    .iter()
                    .any(|f| in_dir(f, d) && Lang::from_path(f).is_some())
            })
            .collect();
        if code_subs.len() == 1 && root_code == 0 {
            return code_subs[0].0.clone();
        }
        String::new()
    }

    // ---- requests -----------------------------------------------------------

    fn request_parse_initial(&mut self) {
        let sources: Vec<String> = self.index.source_files().cloned().collect();
        let targets: Vec<String> = if sources.len() <= EAGER_PARSE {
            sources
        } else {
            let dir = match &self.map.level {
                Some(Level::Dir(d)) => d.clone(),
                _ => String::new(),
            };
            sources
                .into_iter()
                .filter(|f| parent_dir(f) == dir)
                .collect()
        };
        self.request_parse(targets);
    }

    pub fn request_parse(&mut self, files: Vec<String>) {
        let files: Vec<String> = files
            .into_iter()
            .filter(|f| !self.index.is_parsed(f))
            .collect();
        if files.is_empty() {
            return;
        }
        self.parse_pending = true;
        let root = self.root.clone();
        self.spawn(move |tx| {
            let mut batch = Vec::new();
            let mut last = Instant::now();
            let n = files.len();
            for (i, f) in files.into_iter().enumerate() {
                if let Some(item) = parse_file(&root, &f) {
                    batch.push(item);
                }
                let done = i + 1 == n;
                if done || (batch.len() >= 32 && last.elapsed().as_millis() > 50) {
                    let _ = tx.send(Msg::Parsed(std::mem::take(&mut batch), done));
                    last = Instant::now();
                }
            }
        });
    }

    /// Parse a folder when the map zooms into it (huge repositories).
    pub fn ensure_dir_parsed(&mut self, dir: &str) {
        let files: Vec<String> = self
            .index
            .files
            .iter()
            .filter(|f| in_dir(f, dir) && Lang::from_path(f).is_some() && !self.index.is_parsed(f))
            .take(1_000)
            .cloned()
            .collect();
        self.request_parse(files);
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

    fn recompute_marks(&mut self) {
        let old = &self.old_quals;
        self.marks = map::compute_marks(&self.index, &self.diffs, &|d: &FileDiff| {
            old.get(d.old_path.as_deref().unwrap_or(&d.path)).cloned()
        });
    }

    // ---- scope ----------------------------------------------------------------

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
            Scope::Head | Scope::Commit(_) => Scope::Turn,
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
        if matches!(self.scope, Scope::Head | Scope::Commit(_)) {
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

    // ---- map ----------------------------------------------------------------

    pub fn map_level(&self) -> Level {
        self.map.level.clone().unwrap_or(Level::Dir(String::new()))
    }

    /// Rebuild the scene when something changed (or the width did).
    pub fn ensure_scene(&mut self, width: usize) {
        if !self.files_loaded {
            return;
        }
        if !self.map.dirty && self.map.scene.is_some() && self.map.built_width == width {
            return;
        }
        let level = self.map_level();
        if let Level::Dir(d) = &level {
            if self.index.source_files().count() > EAGER_PARSE {
                let d = d.clone();
                self.ensure_dir_parsed(&d);
            }
        }
        if let Level::File(f) = &level {
            if !self.index.is_parsed(f) && Lang::from_path(f).is_some() {
                let f = f.clone();
                self.request_parse(vec![f]);
                self.drain_if_sync();
            }
        }
        // Keep the selection on the same thing across rebuilds.
        if self.map.pending_select.is_none() {
            if let Some(scene) = &self.map.scene {
                if scene.level == level && self.map.built_page == self.map.page {
                    self.map.pending_select = selection_of(scene, self.map.cursor);
                }
            }
        }
        let mut scene = map::build(
            &self.index,
            &self.marks,
            &level,
            self.map.page,
            width.max(40),
        );
        self.map.page = scene.page;
        let mut cursor = Cursor::default();
        if let Some(sel) = self.map.pending_select.take() {
            let find = |scene: &Scene| match &sel {
                Select::Path(p) => {
                    map::node_for_path(scene, p).map(|n| Cursor { node: n, row: None })
                }
                Select::Symbol(f, i) => map::node_for_symbol(scene, f, *i),
            };
            let mut found = find(&scene);
            // In a paginated folder, turn to the page that holds it.
            if found.is_none() && scene.pages > 1 {
                for page in 0..scene.pages {
                    let other = map::build(&self.index, &self.marks, &level, page, width.max(40));
                    if let Some(c) = find(&other) {
                        found = Some(c);
                        scene = other;
                        self.map.page = page;
                        break;
                    }
                }
            }
            if let Some(c) = found {
                cursor = c;
            }
        } else if self.map.scene.as_ref().is_some_and(|s| s.level == level)
            && self.map.built_page == self.map.page
        {
            cursor = self.map.cursor;
        } else if let Some(first_changed) = scene.nodes.iter().position(|n| n.mark.is_some()) {
            cursor.node = first_changed;
        }
        if cursor.node >= scene.nodes.len() {
            cursor = Cursor::default();
        }
        if let Some(r) = cursor.row {
            if scene
                .nodes
                .get(cursor.node)
                .is_none_or(|n| r >= n.rows.len())
            {
                cursor.row = None;
            }
        }
        self.map.cursor = cursor;
        self.map.scene = Some(scene);
        self.map.dirty = false;
        self.map.built_width = width;
        self.map.built_page = self.map.page;
    }

    pub fn map_canvas(&self) -> Option<Canvas> {
        self.map.scene.as_ref().map(|s| {
            map::draw(
                s,
                (self.map.focus == Focus::Body).then_some(self.map.cursor),
            )
        })
    }

    /// The function the cursor is on (for Flow and the hint line).
    pub fn current_function(&self) -> Option<SymId> {
        let scene = self.map.scene.as_ref()?;
        let node = scene.nodes.get(self.map.cursor.node)?;
        let candidate = match self.map.cursor.row {
            Some(r) => node.rows.get(r).and_then(|row| row.sym.clone()),
            None => node.symbol(),
        };
        let (file, idx) = candidate.or_else(|| node.rows.iter().find_map(|r| r.sym.clone()))?;
        let fs = self.index.symbols(&file)?;
        let s = fs.symbols.get(idx)?;
        if s.kind.is_callable() {
            Some(SymId { file, idx })
        } else {
            // a class: its first method
            let m = map::methods_of(fs, idx).into_iter().next()?;
            Some(SymId { file, idx: m })
        }
    }

    pub fn map_move(&mut self, dx: i32, dy: i32) {
        let Some(scene) = &self.map.scene else { return };
        if let Some(n) = layout::neighbor(&scene.layout, self.map.cursor.node, dx, dy) {
            self.map.cursor = Cursor { node: n, row: None };
        } else if scene.nodes.is_empty() {
            self.map.cursor = Cursor::default();
        }
    }

    /// j/k: rows inside the selected box, then the next box in reading order.
    pub fn map_row(&mut self, delta: i32) {
        let Some(scene) = &self.map.scene else { return };
        let Some(node) = scene.nodes.get(self.map.cursor.node) else {
            return;
        };
        let selectable: Vec<usize> = node
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.sym.is_some())
            .map(|(i, _)| i)
            .collect();
        let pos = self
            .map
            .cursor
            .row
            .and_then(|r| selectable.iter().position(|&x| x == r));
        let next = match (pos, delta > 0) {
            (None, true) => selectable.first().copied(),
            (Some(p), true) => selectable.get(p + 1).copied(),
            (Some(0), false) => None,
            (Some(p), false) => selectable.get(p - 1).copied(),
            (None, false) => None,
        };
        match (next, pos, delta > 0) {
            (Some(r), _, _) => self.map.cursor.row = Some(r),
            (None, Some(0), false) => self.map.cursor.row = None,
            _ => {
                // leave the box: next/previous node in reading order
                let mut order: Vec<usize> = (0..scene.nodes.len()).collect();
                order.sort_by_key(|&i| (scene.layout.nodes[i].y, scene.layout.nodes[i].x));
                let at = order
                    .iter()
                    .position(|&i| i == self.map.cursor.node)
                    .unwrap_or(0);
                let target = if delta > 0 {
                    order.get(at + 1)
                } else if at > 0 {
                    order.get(at - 1)
                } else {
                    None
                };
                if let Some(&t) = target {
                    let rows = &scene.nodes[t].rows;
                    let row = if delta < 0 {
                        rows.iter().rposition(|r| r.sym.is_some())
                    } else {
                        None
                    };
                    self.map.cursor = Cursor { node: t, row };
                }
            }
        }
    }

    pub fn map_enter(&mut self) {
        let Some(scene) = &self.map.scene else { return };
        let Some(node) = scene.nodes.get(self.map.cursor.node).cloned() else {
            return;
        };
        let row_sym = self
            .map
            .cursor
            .row
            .and_then(|r| node.rows.get(r))
            .and_then(|r| r.sym.clone());
        match &node.kind {
            NodeKind::Dir(d) => self.zoom_to(Level::Dir(d.clone()), None),
            NodeKind::File(f) => {
                let sel = row_sym.map(|(f, i)| Select::Symbol(f, i));
                if Lang::from_path(f).is_some() {
                    self.zoom_to(Level::File(f.clone()), sel);
                } else {
                    self.message = Some(format!(
                        "no grammar for {} · codeMorph reads {}",
                        f,
                        lang::supported_extensions()
                    ));
                }
            }
            NodeKind::External(f) => self.zoom_to(Level::File(f.clone()), None),
            NodeKind::Symbol { file, idx } => {
                let target = match row_sym {
                    Some((f, i)) => Some(SymId { file: f, idx: i }),
                    None => self.current_function().or_else(|| {
                        Some(SymId {
                            file: file.clone(),
                            idx: *idx,
                        })
                    }),
                };
                if let Some(t) = target {
                    self.open_flow(t);
                }
            }
            NodeKind::More => self.map_page(1),
        }
    }

    pub fn zoom_to(&mut self, level: Level, select: Option<Select>) {
        self.map.level = Some(level);
        self.map.page = 0;
        self.map.pending_select = select;
        self.map.scroll = (0, 0);
        self.map.dirty = true;
    }

    pub fn map_back(&mut self) {
        let level = self.map_level();
        if let Some(parent) = level.parent() {
            let path = match &level {
                Level::File(f) => f.clone(),
                Level::Dir(d) => d.clone(),
            };
            self.zoom_to(parent, Some(Select::Path(path)));
        }
    }

    pub fn map_page(&mut self, delta: i32) {
        let pages = self.map.scene.as_ref().map(|s| s.pages).unwrap_or(1);
        let next = (self.map.page as i32 + delta).clamp(0, pages as i32 - 1) as usize;
        if next != self.map.page {
            self.map.page = next;
            self.map.cursor = Cursor::default();
            self.map.scroll = (0, 0);
            self.map.dirty = true;
        }
    }

    /// Jump the map to a symbol or file (search, rail lists).
    pub fn reveal(&mut self, file: &str, sym: Option<usize>) {
        match sym {
            Some(i) => self.zoom_to(
                Level::File(file.to_string()),
                Some(Select::Symbol(file.to_string(), i)),
            ),
            None => self.zoom_to(
                Level::Dir(parent_dir(file).to_string()),
                Some(Select::Path(file.to_string())),
            ),
        }
    }

    // ---- flow ---------------------------------------------------------------

    pub fn open_flow(&mut self, target: SymId) {
        if let Some(cur) = self.flow.target.take() {
            if cur != target {
                self.flow.back.push(cur);
            }
        }
        self.flow.target = Some(target);
        self.flow.selected = 0;
        self.flow.scroll = (0, 0);
        self.flow.dirty = true;
        self.view = View::Flow;
    }

    pub fn flow_back(&mut self) {
        if let Some(prev) = self.flow.back.pop() {
            self.flow.target = Some(prev);
            self.flow.selected = 0;
            self.flow.scroll = (0, 0);
            self.flow.dirty = true;
        }
    }

    pub fn ensure_flow(&mut self) {
        if !self.flow.dirty && (self.flow.layout.is_some() || self.flow.error.is_some()) {
            return;
        }
        if self.flow.target.is_none() {
            self.flow.target = self
                .current_function()
                .or_else(|| self.first_changed_function());
        }
        self.flow.dirty = false;
        self.flow.error = None;
        let Some(target) = self.flow.target.clone() else {
            self.flow.chart = None;
            self.flow.layout = None;
            return;
        };
        let Some(lang) = Lang::from_path(&target.file) else {
            self.flow.error = Some(format!(
                "no flow for {} · codeMorph has grammars for {}",
                target.file,
                lang::supported_extensions()
            ));
            return;
        };
        let Some(fs) = self.index.symbols(&target.file).cloned() else {
            self.request_parse(vec![target.file.clone()]);
            self.drain_if_sync();
            self.flow.dirty = true;
            return;
        };
        let Some(sym) = fs.symbols.get(target.idx) else {
            self.flow.error = Some("that function is gone; the file changed".into());
            return;
        };
        let src = match std::fs::read(self.root.join(&target.file)) {
            Ok(b) => String::from_utf8_lossy(&b).into_owned(),
            Err(e) => {
                self.flow.error = Some(format!("cannot read {}: {e}", target.file));
                return;
            }
        };
        match flow::build(lang, &src, sym) {
            Some(chart) => {
                let l = flow::layout(&chart);
                if self.flow.selected >= l.boxes.len() {
                    self.flow.selected = 0;
                }
                self.flow.layout = Some(l);
                self.flow.chart = Some(chart);
            }
            None => self.flow.error = Some(format!("could not chart {}", sym.name)),
        }
    }

    fn first_changed_function(&self) -> Option<SymId> {
        self.marks
            .changed_symbols(&self.index)
            .into_iter()
            .next()
            .map(|(file, idx, _)| SymId { file, idx })
    }

    pub fn flow_symbol(&self) -> Option<&lang::Symbol> {
        let t = self.flow.target.as_ref()?;
        self.index.symbols(&t.file)?.symbols.get(t.idx)
    }

    pub fn flow_line_marks(&self) -> BTreeMap<u32, char> {
        self.flow
            .target
            .as_ref()
            .and_then(|t| self.marks.lines.get(&t.file).cloned())
            .unwrap_or_default()
    }

    pub fn flow_move(&mut self, delta: i32) {
        let Some(l) = &self.flow.layout else { return };
        if l.boxes.is_empty() {
            return;
        }
        let n = l.boxes.len() as i32;
        self.flow.selected = (self.flow.selected as i32 + delta).clamp(0, n - 1) as usize;
    }

    /// Move to the nearest box sideways (branches).
    pub fn flow_side(&mut self, dir: i32) {
        let Some(l) = &self.flow.layout else { return };
        let Some(cur) = l.boxes.get(self.flow.selected) else {
            return;
        };
        let cy = cur.y + cur.h / 2;
        let cx = cur.x + cur.w / 2;
        let best = l
            .boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                let bx = b.x + b.w / 2;
                if dir > 0 {
                    bx > cx + cur.w / 2
                } else {
                    bx + cur.w / 2 < cx
                }
            })
            .min_by_key(|(_, b)| {
                let by = b.y + b.h / 2;
                let bx = b.x + b.w / 2;
                (by as i64 - cy as i64).abs() * 4 + (bx as i64 - cx as i64).abs()
            })
            .map(|(i, _)| i);
        if let Some(i) = best {
            self.flow.selected = i;
        }
    }

    /// Calls made from the selected box (resolved targets).
    pub fn flow_calls_in_box(&self) -> Vec<SymId> {
        let (Some(t), Some(l)) = (&self.flow.target, &self.flow.layout) else {
            return Vec::new();
        };
        let Some(b) = l.boxes.get(self.flow.selected) else {
            return Vec::new();
        };
        let lines: BTreeSet<u32> = b.lines.iter().map(|x| x.line).collect();
        let mut out = Vec::new();
        for c in self.index.calls_from(&t.file, t.idx) {
            if lines.contains(&c.line) && !out.contains(&c.to) {
                out.push(c.to.clone());
            }
        }
        out
    }

    pub fn flow_enter(&mut self) {
        let calls = self.flow_calls_in_box();
        match calls.into_iter().find(|c| {
            self.index
                .symbols(&c.file)
                .and_then(|f| f.symbols.get(c.idx))
                .is_some_and(|s| s.kind.is_callable())
        }) {
            Some(c) => self.open_flow(c),
            None => self.message = Some("no call to a known function in this box".into()),
        }
    }

    /// Functions listed in the Flow rail: those of the target's file.
    pub fn flow_rail_functions(&self) -> Vec<(usize, String)> {
        let Some(t) = &self.flow.target else {
            return Vec::new();
        };
        let Some(fs) = self.index.symbols(&t.file) else {
            return Vec::new();
        };
        fs.symbols
            .iter()
            .enumerate()
            .filter(|(_, s)| s.kind.is_callable())
            .map(|(i, s)| (i, s.qual()))
            .collect()
    }

    // ---- changes ------------------------------------------------------------

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
                // across files
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
            // advance to the next hunk, like a checklist
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

    /// New-file line range covered by rows a..=b of the selected file.
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
                let n = dl.new_no.or_else(|| {
                    // a deleted line anchors to the next new line
                    f.hunks[*h].lines[*l..].iter().find_map(|x| x.new_no)
                });
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

    /// The agent pane comments go to.
    pub fn target_pane(&self) -> Option<String> {
        self.agent.as_ref().and_then(|a| a.pane_id.clone())
    }

    /// P (paste, no Enter) or S (submit): send the draft to the agent.
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
                    // Close the popup: the agent's pane is right underneath.
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

    /// path and line under the cursor in the current view (for e and y).
    pub fn location(&self) -> Option<(String, u32)> {
        match self.view {
            View::Map => {
                let t = self.current_function();
                if let Some(t) = t {
                    let s = self.index.symbols(&t.file)?.symbols.get(t.idx)?;
                    return Some((t.file, s.start_line));
                }
                let scene = self.map.scene.as_ref()?;
                match &scene.nodes.get(self.map.cursor.node)?.kind {
                    NodeKind::File(f) | NodeKind::External(f) => Some((f.clone(), 1)),
                    _ => None,
                }
            }
            View::Flow => {
                let t = self.flow.target.as_ref()?;
                let line = self
                    .flow
                    .layout
                    .as_ref()
                    .and_then(|l| l.boxes.get(self.flow.selected))
                    .map(|b| b.first_line())
                    .filter(|l| *l > 0)
                    .or_else(|| self.flow_symbol().map(|s| s.start_line))?;
                Some((t.file.clone(), line))
            }
            View::Changes => {
                let f = self.diffs.get(self.changes.file)?;
                let (start, _, _) = self
                    .line_range(self.changes.row, self.changes.row)
                    .unwrap_or((1, 1, String::new()));
                Some((f.path.clone(), start))
            }
            View::History => None,
        }
    }

    pub fn start_search(&mut self) {
        self.input = Some(Input {
            kind: InputKind::Search,
            text: String::new(),
        });
        self.search_hits.clear();
        self.search_sel = 0;
    }

    pub fn update_search(&mut self) {
        let q = match &self.input {
            Some(Input {
                kind: InputKind::Search,
                text,
            }) => text.clone(),
            _ => return,
        };
        self.search_hits = if q.is_empty() {
            Vec::new()
        } else {
            self.index.search(&q, 40)
        };
        self.search_sel = 0;
    }

    pub fn accept_search(&mut self) {
        if let Some(hit) = self.search_hits.get(self.search_sel).cloned() {
            let callable = hit.sym.and_then(|i| {
                self.index
                    .symbols(&hit.file)
                    .and_then(|f| f.symbols.get(i))
                    .map(|s| s.kind.is_callable())
            });
            if self.view == View::Flow && callable == Some(true) {
                self.open_flow(SymId {
                    file: hit.file.clone(),
                    idx: hit.sym.unwrap(),
                });
            } else {
                self.view = View::Map;
                self.reveal(&hit.file, hit.sym);
            }
        }
        self.input = None;
        self.search_hits.clear();
    }

    /// History ⏎: show the selected commit's diff.
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

    /// History `1`: the map, with this commit's changes marked.
    pub fn map_of_commit(&mut self) {
        if let Some(c) = self.history.commits.get(self.history.sel) {
            let sha = c.sha.clone();
            self.set_scope(Scope::Commit(sha));
            self.view = View::Map;
        }
    }
}

pub fn scope_name(s: &Scope) -> String {
    match s {
        Scope::Turn => "last turn".into(),
        Scope::Session => "this agent's session".into(),
        Scope::Head => "since HEAD".into(),
        Scope::Commit(sha) => format!("commit {}", &sha[..sha.len().min(7)]),
    }
}

fn selection_of(scene: &Scene, cursor: Cursor) -> Option<Select> {
    let node = scene.nodes.get(cursor.node)?;
    if let Some(r) = cursor.row {
        if let Some((f, i)) = node.rows.get(r).and_then(|r| r.sym.clone()) {
            return Some(Select::Symbol(f, i));
        }
    }
    match &node.kind {
        NodeKind::Dir(d) => Some(Select::Path(d.clone())),
        NodeKind::File(f) | NodeKind::External(f) => Some(Select::Path(f.clone())),
        NodeKind::Symbol { file, idx } => Some(Select::Symbol(file.clone(), *idx)),
        NodeKind::More => None,
    }
}

fn parse_file(root: &std::path::Path, f: &str) -> Option<(String, FileSymbols, (SystemTime, u64))> {
    let lang = Lang::from_path(f)?;
    let path = root.join(f);
    let meta = std::fs::metadata(&path).ok()?;
    let stamp = (
        meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        meta.len(),
    );
    let syms = if meta.len() > crate::index::MAX_PARSE_BYTES {
        FileSymbols {
            lang,
            symbols: Vec::new(),
            calls: Vec::new(),
            imports: Vec::new(),
            lines: 0,
            has_errors: false,
        }
    } else {
        let src = String::from_utf8_lossy(&std::fs::read(&path).ok()?).into_owned();
        lang::extract(lang, &src)
    };
    Some((f.to_string(), syms, stamp))
}

/// Resolve the directory and list files. Inside herdr, ask for the focused
/// pane's foreground cwd first: the agent may have moved.
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
    let (repo, root, files, truncated) = match git::discover(&root) {
        Ok(info) => {
            let mut files = git::list_files(&info.root).unwrap_or_default();
            let truncated = files.len() > MAX_FILES;
            files.truncate(MAX_FILES);
            let r = info.root.clone();
            (Ok(info), r, files, truncated)
        }
        Err(e) => {
            let root = root.canonicalize().unwrap_or(root);
            let files = git::walk_files(&root, MAX_FILES + 1);
            let truncated = files.len() > MAX_FILES;
            let mut files = files;
            files.truncate(MAX_FILES);
            (Err(e.map(|e| e.0)), root, files, truncated)
        }
    };
    let _ = tx.send(Msg::Repo(Box::new(RepoLoad {
        root,
        repo,
        files,
        truncated,
        agent,
    })));
}

/// Diff for a scope, plus the base versions' symbol names for A/M marks.
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
            // Standalone: the agent that worked here most recently.
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
            now_tree = shadow.snapshot("codeMorph: now").ok().map(|c| c.tree);
        }
        now_tree.clone()
    };

    // Agents strip: each agent's last turn in this repo.
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
    let result: Result<(Vec<FileDiff>, Option<String>), String> = match scope {
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
                        Some(to) => shadow
                            .diff(&start.tree, &to)
                            .map(|d| (d, Some(start.commit.clone())))
                            .map_err(|e| e.0),
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
        Scope::Commit(sha) => {
            info.noun = format!("commit {}", &sha[..sha.len().min(7)]);
            info.rail = format!("commit {}", &sha[..sha.len().min(7)]);
            info.header = info.rail.clone();
            git::show_commit(&repo.root, sha)
                .map(|d| (d, Some(format!("{sha}^"))))
                .map_err(|e| e.0)
        }
    };
    let (diffs, base) = match result {
        Ok((d, base)) => (Ok(d), base),
        Err(e) => (Err(e), None),
    };
    // Symbol names of the base version of each changed source file.
    let mut old_quals = BTreeMap::new();
    if let (Ok(d), Some(base)) = (&diffs, &base) {
        for f in d
            .iter()
            .filter(|f| matches!(f.status, FileStatus::Modified | FileStatus::Renamed))
            .take(200)
        {
            let path = f.old_path.as_deref().unwrap_or(&f.path);
            if Lang::from_path(path).is_none() {
                continue;
            }
            let src = if base.len() == 40 && !base.ends_with('^') {
                // a shadow checkpoint commit
                shadow.read_file(base, path)
            } else {
                git::run(&repo.root, &["show", &format!("{base}:{path}")]).ok()
            };
            if let Some(q) = src.and_then(|s| map::quals_of(path, &s)) {
                old_quals.insert(path.to_string(), q);
            }
        }
    }
    ChangeSet {
        scope: scope.clone(),
        pane_key: chosen.map(|p| p.pane_key.clone()),
        diffs,
        old_quals,
        info,
        agents,
    }
}

fn head_changes(
    store: &Store,
    repo: &RepoInfo,
    info: &mut ScopeInfo,
) -> Result<(Vec<FileDiff>, Option<String>), String> {
    info.noun = "since HEAD".into();
    info.rail = match &repo.head {
        Some(h) => format!("since HEAD · {}", &h[..h.len().min(7)]),
        None => "no commits yet".into(),
    };
    if info.header.is_empty() {
        info.header = String::new();
    }
    let scratch = store.shadow(repo).dir;
    git::diff_head(repo, &scratch)
        .map(|d| (d, repo.head.as_ref().map(|_| "HEAD".to_string())))
        .map_err(|e| e.0)
}

/// Worktrees with the herdr agents working in them.
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
                // the most specific worktree wins: skip if another worktree
                // inside this one holds the agent
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
