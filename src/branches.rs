use std::collections::BTreeMap;

use crate::app::{App, Input, InputKind, Msg, View};
use crate::ask::Ask;
use crate::gh::{PullRequest, PullRequests};
use crate::git::branch::{self, short_name, short_sha, Branch, Branches, Deletion, Merged, Remote};
use crate::git::{self, GitError};
use crate::open;
use crate::util;

pub const SHOWN_COMMITS: usize = 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ahead {
    Reading,
    Known(branch::Unique),
    Failed(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Pulls {
    #[default]
    NotAsked,
    Reading,
    Known(PullRequests),
    Unknown(String),
}

#[derive(Debug, Default)]
pub struct BranchesState {
    pub local: Vec<Branch>,
    pub remotes: Vec<Remote>,
    pub default: Option<String>,
    pub loaded: bool,
    pub error: Option<String>,
    pub sel: usize,
    pub scroll: usize,
    pub ahead: BTreeMap<String, Ahead>,
    pub pulls: Pulls,
    pub reads: u64,
}

impl BranchesState {
    fn at(&self, name: &str) -> Option<usize> {
        self.local.iter().position(|b| b.name == name)
    }

    pub fn load(&mut self, result: Result<Branches, String>, select: Option<&str>) {
        let kept = self.selected().map(|b| b.name.clone());
        let first_read = !self.loaded;
        match result {
            Ok(read) => {
                self.local = read.local;
                self.remotes = read.remotes;
                self.default = read.default;
                self.error = None;
            }
            Err(e) => {
                self.local.clear();
                self.remotes.clear();
                self.default = None;
                self.error = Some(e);
            }
        }
        self.loaded = true;
        self.reads += 1;
        self.ahead.clear();
        let current = || self.local.iter().position(|b| b.current);
        let wanted = select
            .and_then(|name| self.at(name))
            .or_else(|| kept.and_then(|name| self.at(&name)))
            .or_else(|| first_read.then(current).flatten());
        self.sel = wanted
            .unwrap_or(self.sel)
            .min(self.local.len().saturating_sub(1));
    }

    pub fn selected(&self) -> Option<&Branch> {
        self.local.get(self.sel)
    }

    pub fn move_by(&mut self, delta: i32) {
        let last = self.local.len().saturating_sub(1) as i64;
        self.sel = (self.sel as i64 + delta as i64).clamp(0, last) as usize;
    }

    pub fn pull(&self, branch: &str) -> Option<&PullRequest> {
        match &self.pulls {
            Pulls::Known(pulls) => pulls.get(branch),
            _ => None,
        }
    }

    pub fn default_name(&self) -> Option<&str> {
        self.default.as_deref().map(short_name)
    }

    pub fn is_default(&self, branch: &Branch) -> bool {
        self.default.as_deref() == Some(format!("refs/heads/{}", branch.name).as_str())
    }
}

pub(crate) fn told(said: &str) -> String {
    let one_line = said.split_whitespace().collect::<Vec<_>>().join(" ");
    util::sanitize_line(&one_line)
}

fn name_on_origin(branch: &Branch) -> &str {
    branch
        .upstream
        .as_ref()
        .and_then(|upstream| upstream.name.strip_prefix("origin/"))
        .unwrap_or(&branch.name)
}

impl App {
    pub fn request_branches(&mut self) {
        if !View::GIT_TABS.contains(&self.view) {
            return;
        }
        let Some(repo) = self.repo.clone() else {
            self.branches.loaded = true;
            return;
        };
        let asked_after = self.branches.reads;
        self.spawn(move |tx| {
            let read = branch::list(&repo.root).map_err(|e| e.0);
            let _ = tx.send(Msg::Branches { asked_after, read });
        });
    }

    pub fn reload_branches(&mut self, select: Option<&str>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let read = branch::list(&repo.root).map_err(|e| e.0);
        self.branches.load(read, select);
        self.request_ahead();
    }

    pub fn branches_shown(&mut self) {
        if self.view != View::Branches {
            return;
        }
        self.request_pulls();
        self.request_ahead();
    }

    fn request_pulls(&mut self) {
        if self.branches.pulls != Pulls::NotAsked {
            return;
        }
        let Some(repo) = self.repo.clone() else {
            return;
        };
        self.branches.pulls = Pulls::Reading;
        let gh = self.gh.clone();
        self.spawn(move |tx| {
            let read = gh.pull_requests(&repo.root).map_err(|e| e.why());
            let _ = tx.send(Msg::Pulls(read));
        });
    }

    pub fn request_ahead(&mut self) {
        if self.view != View::Branches {
            return;
        }
        let (Some(repo), Some(selected), Some(base)) = (
            self.repo.clone(),
            self.branches.selected(),
            self.branches.default.clone(),
        ) else {
            return;
        };
        let name = selected.name.clone();
        if self.branches.is_default(selected) || self.branches.ahead.contains_key(&name) {
            return;
        }
        self.branches.ahead.insert(name.clone(), Ahead::Reading);
        let read = self.branches.reads;
        self.spawn(move |tx| {
            let found = branch::unique_commits(&repo.root, &name, &base, SHOWN_COMMITS);
            let _ = tx.send(Msg::Ahead {
                read,
                branch: name,
                found: found.map_err(|e| e.0),
            });
        });
    }

    pub fn ahead_read(&mut self, read: u64, branch: String, found: Result<branch::Unique, String>) {
        if read != self.branches.reads {
            return;
        }
        let ahead = match found {
            Ok(unique) => Ahead::Known(unique),
            Err(e) => Ahead::Failed(e),
        };
        self.branches.ahead.insert(branch, ahead);
    }

    pub fn branches_move(&mut self, delta: i32) {
        self.branches.move_by(delta);
        self.request_ahead();
        self.drain_if_sync();
    }

    pub fn head_label(&self) -> String {
        match self.repo.as_ref() {
            Some(repo) => match (&repo.branch, &repo.head) {
                (Some(branch), _) => branch.clone(),
                (None, Some(head)) => format!("the detached HEAD at {}", short_sha(head)),
                (None, None) => "HEAD".into(),
            },
            None => "HEAD".into(),
        }
    }

    fn agent_is_working(&self) -> bool {
        self.agent.as_ref().is_some_and(|agent| agent.is_working())
    }

    fn after_branch_write(&mut self, select: Option<&str>, result: Result<String, GitError>) {
        self.reload_git();
        self.reload_branches(select.filter(|_| result.is_ok()));
        self.drain_if_sync();
        self.message = Some(match result {
            Ok(done) => done,
            Err(e) => told(&e.0),
        });
    }

    pub fn switch_selected(&mut self) {
        let Some(selected) = self.branches.selected() else {
            return;
        };
        let name = selected.name.clone();
        if selected.current {
            self.message = Some(format!("already on {name}"));
        } else if self.agent_is_working() {
            self.confirm = Some(Ask::Switch { branch: name });
        } else {
            self.switch_to(&name);
        }
    }

    pub(crate) fn switch_to(&mut self, name: &str) {
        let result = branch::switch(&self.root, name).map(|()| format!("switched to {name}"));
        self.after_branch_write(Some(name), result);
    }

    pub fn start_new_branch(&mut self) {
        if self.repo.is_none() || !self.branches.loaded {
            return;
        }
        self.input = Some(Input {
            kind: InputKind::BranchName,
            text: String::new(),
        });
    }

    pub fn create_branch(&mut self, typed: &str) {
        let name = typed.trim();
        if name.is_empty() {
            self.message = Some("a branch needs a name · nothing created".into());
            return;
        }
        let from = self.head_label();
        let result = branch::create(&self.root, name)
            .map(|()| format!("created {name} from {from} and switched to it"));
        self.after_branch_write(Some(name), result);
    }

    pub fn ask_merge(&mut self) {
        let Some(selected) = self.branches.selected() else {
            return;
        };
        let name = selected.name.clone();
        if selected.current {
            self.message = Some(format!(
                "{name} is the current branch · a branch does not merge into itself"
            ));
            return;
        }
        self.confirm = Some(Ask::Merge {
            branch: name,
            into: self.head_label(),
            agent_working: self.agent_is_working(),
        });
    }

    pub(crate) fn merge(&mut self, name: &str) {
        let into = self.head_label();
        let result = branch::merge(&self.root, name).map(|merged| match merged {
            Merged::AlreadyUpToDate => format!("{into} already has everything on {name}"),
            Merged::FastForward => format!("fast-forwarded {into} to {name}"),
            Merged::Commit => format!("merged {name} into {into}"),
            Merged::Conflicts(files) => format!(
                "the merge of {name} stopped on conflicts in {} · the Status tab (1) lists them",
                util::plural(files, "file", "files")
            ),
            Merged::Unfinished(why) => format!(
                "the merge of {name} is not committed · the Status tab (1) shows it · {}",
                told(&why)
            ),
        });
        self.after_branch_write(Some(name), result);
    }

    pub fn ask_delete(&mut self) {
        let Some(selected) = self.branches.selected() else {
            return;
        };
        match branch::deletion(&self.root, &selected.name) {
            Ok(deletion) => self.confirm = Some(Ask::Delete(deletion)),
            Err(e) => self.message = Some(told(&e.0)),
        }
    }

    pub(crate) fn delete(&mut self, asked: &Deletion) {
        let done = format!("deleted {} (was {})", asked.branch, short_sha(&asked.tip));
        let result = branch::delete(&self.root, asked).map(|()| done);
        self.after_branch_write(None, result);
    }

    pub fn branch_link(&self) -> Result<String, String> {
        let selected = self.branches.selected().ok_or("no branch is selected")?;
        let pull_request = self.branches.pull(&selected.name).and_then(|p| p.link());
        if let Some(page) = pull_request {
            return Ok(page.to_string());
        }
        let origin = git::run(&self.root, &["remote", "get-url", "origin"])
            .map_err(|_| "this repository has no origin remote · nothing to open")?;
        open::github_branch_url(&origin, name_on_origin(selected))
            .ok_or_else(|| "origin is not on GitHub · nothing to open".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str, current: bool) -> Branch {
        Branch {
            name: name.into(),
            tip: format!("{name}-tip"),
            current,
            elsewhere: None,
            upstream: None,
        }
    }

    fn read(names: &[&str], current: &str) -> Result<Branches, String> {
        Ok(Branches {
            local: names.iter().map(|n| named(n, *n == current)).collect(),
            remotes: Vec::new(),
            default: Some("refs/heads/main".into()),
        })
    }

    fn on(state: &BranchesState) -> &str {
        state.selected().map(|b| b.name.as_str()).unwrap_or("")
    }

    #[test]
    fn the_list_opens_on_the_current_branch() {
        let mut state = BranchesState::default();
        state.load(read(&["docs", "feat/x", "main"], "main"), None);
        assert_eq!((on(&state), state.sel), ("main", 2));
        let mut detached = BranchesState::default();
        detached.load(read(&["docs", "main"], ""), None);
        assert_eq!(on(&detached), "docs");
    }

    #[test]
    fn a_later_read_keeps_the_selected_branch_or_takes_the_one_asked_for() {
        let mut state = BranchesState::default();
        state.load(read(&["docs", "feat/x", "main"], "main"), None);
        state.move_by(-1);
        state.load(read(&["a-new-one", "docs", "feat/x", "main"], "main"), None);
        assert_eq!(on(&state), "feat/x", "by name, not by row");
        state.load(
            read(&["docs", "feat/x", "feat/y", "main"], "feat/y"),
            Some("feat/y"),
        );
        assert_eq!(on(&state), "feat/y");
        state.load(read(&["docs", "feat/x", "main"], "main"), None);
        assert_eq!(on(&state), "main", "the row a deleted branch was on");
        state.move_by(1);
        state.load(read(&["docs"], "docs"), None);
        assert_eq!(on(&state), "docs");
        state.load(read(&[], ""), None);
        assert_eq!((state.sel, state.selected()), (0, None));
    }

    #[test]
    fn the_selection_stops_at_both_ends() {
        let mut state = BranchesState::default();
        state.move_by(3);
        assert_eq!(state.sel, 0);
        state.load(read(&["docs", "feat/x", "main"], "docs"), None);
        state.move_by(i32::MAX / 2);
        assert_eq!(on(&state), "main");
        state.move_by(-(i32::MAX / 2));
        assert_eq!(on(&state), "docs");
    }

    #[test]
    fn a_read_that_fails_keeps_the_error_and_no_branches() {
        let mut state = BranchesState::default();
        state.load(read(&["docs", "main"], "main"), None);
        state.ahead.insert("docs".into(), Ahead::Reading);
        let before = state.reads;
        state.load(Err("unknown field name: worktreepath".into()), None);
        assert!(state.loaded && state.local.is_empty() && state.ahead.is_empty());
        assert_eq!(
            state.error.as_deref(),
            Some("unknown field name: worktreepath")
        );
        assert_eq!(state.reads, before + 1);
    }

    #[test]
    fn the_default_branch_is_named_without_its_namespace() {
        let mut state = BranchesState::default();
        state.load(read(&["docs", "main"], "main"), None);
        assert_eq!(state.default_name(), Some("main"));
        assert!(state.is_default(&named("main", true)));
        assert!(!state.is_default(&named("docs", false)));
        state.default = Some("refs/remotes/origin/trunk".into());
        assert_eq!(state.default_name(), Some("origin/trunk"));
        assert!(!state.is_default(&named("trunk", false)));
    }

    #[test]
    fn the_page_on_github_is_named_by_the_upstream_branch() {
        let mut pushed = named("local-name", false);
        assert_eq!(name_on_origin(&pushed), "local-name");
        pushed.upstream = Some(branch::Upstream {
            name: "origin/feat/x".into(),
            ahead: 0,
            behind: 0,
            gone: false,
        });
        assert_eq!(name_on_origin(&pushed), "feat/x");
        pushed.upstream.as_mut().unwrap().name = "fork/feat/y".into();
        assert_eq!(name_on_origin(&pushed), "local-name");
    }

    #[test]
    fn what_git_said_fits_one_line_without_control_characters() {
        assert_eq!(
            told("error: Your local changes\n\tf.txt\u{1b}[0m\nAborting\n"),
            "error: Your local changes f.txt·[0m Aborting"
        );
    }
}
