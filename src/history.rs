use crate::app::{App, HistoryState, Location, Msg, View, HISTORY_LIMIT};
use crate::branches::told;
use crate::git::{self, branch, graph, Commit, FileCommit, FileDiff};
use crate::open;

#[derive(Debug)]
pub struct FileHistory {
    pub path: String,
    pub back: View,
    pub log: HistoryState,
    pub paths: Vec<Vec<String>>,
    pub loaded: bool,
    pub ask: u64,
}

impl FileHistory {
    fn new(path: String, back: View, ask: u64) -> Self {
        FileHistory {
            path,
            back,
            log: HistoryState::default(),
            paths: Vec::new(),
            loaded: false,
            ask,
        }
    }

    pub fn renamed_from(&self, commit: usize) -> Option<&str> {
        graph::renamed_from(self.paths.get(commit)?)
    }

    fn paths_at(&self, sha: &str) -> Vec<String> {
        self.log
            .commits
            .iter()
            .position(|c| c.sha == sha)
            .and_then(|i| self.paths.get(i).cloned())
            .unwrap_or_else(|| vec![self.path.clone()])
    }
}

fn not_a_file(dir: bool) -> Result<(), String> {
    if dir {
        Err("L shows the history of a file · this is a folder".into())
    } else {
        Ok(())
    }
}

impl App {
    pub fn page(&self) -> View {
        if self.file_history.is_some() {
            View::History
        } else {
            self.view
        }
    }

    pub fn shown_history(&self) -> &HistoryState {
        match &self.file_history {
            Some(file) => &file.log,
            None => &self.history,
        }
    }

    pub fn shown_history_mut(&mut self) -> &mut HistoryState {
        match &mut self.file_history {
            Some(file) => &mut file.log,
            None => &mut self.history,
        }
    }

    pub fn shown_history_loaded(&self) -> bool {
        match &self.file_history {
            Some(file) => file.loaded,
            None => self.history_loaded,
        }
    }

    pub fn selected_commit(&self) -> Option<&Commit> {
        let shown = self.shown_history();
        shown.commits.get(shown.sel)
    }

    pub fn file_for_history(&self) -> Result<String, String> {
        let repo = self
            .repo
            .as_ref()
            .ok_or("not a git repository · no history to show")?;
        let nothing = || "no file is selected".to_string();
        match self.view {
            View::Files => {
                let row = self.selected_file_row().ok_or_else(nothing)?;
                not_a_file(row.dir)?;
                Ok(row.path)
            }
            View::Status => self
                .status
                .selected()
                .map(|file| file.path.clone())
                .ok_or_else(nothing),
            View::Changes => self
                .diffs
                .get(self.changes.file)
                .map(|file| file.path.clone())
                .ok_or_else(nothing),
            View::Custom => {
                let item = self.custom.selected().ok_or_else(nothing)?;
                not_a_file(item.dir)?;
                let inside = item
                    .path
                    .strip_prefix(&repo.root)
                    .map_err(|_| "this file is not in the repository · no history to show")?;
                Ok(inside.to_string_lossy().into_owned())
            }
            View::History | View::Branches | View::Worktrees => Err(nothing()),
        }
    }

    pub fn open_file_history(&mut self) {
        let path = match self.file_for_history() {
            Ok(path) => path,
            Err(why) => {
                self.message = Some(why);
                return;
            }
        };
        self.file_history_asks += 1;
        let back = self.view;
        if View::GIT_TABS.contains(&back) {
            self.view = View::History;
        }
        self.file_history = Some(FileHistory::new(path, back, self.file_history_asks));
        self.request_file_history();
        self.drain_if_sync();
    }

    pub fn leave_file_history(&mut self) {
        if let Some(file) = self.file_history.take() {
            self.view = file.back;
        }
    }

    pub fn file_history_location(&self) -> Option<Location> {
        self.file_history.as_ref().map(|file| Location {
            path: file.path.clone(),
            line: None,
            dir: false,
        })
    }

    fn request_file_history(&mut self) {
        let (Some(repo), Some(file)) = (self.repo.clone(), self.file_history.as_ref()) else {
            return;
        };
        let (ask, path) = (file.ask, file.path.clone());
        self.spawn(move |tx| {
            let read = if repo.head.is_none() {
                Ok(Vec::new())
            } else {
                graph::file_log(&repo.root, &path, HISTORY_LIMIT).map_err(|e| e.0)
            };
            let _ = tx.send(Msg::FileHistory { ask, read });
        });
    }

    pub(crate) fn file_history_read(&mut self, ask: u64, read: Result<Vec<FileCommit>, String>) {
        let Some(file) = self.file_history.as_mut().filter(|f| f.ask == ask) else {
            return;
        };
        file.loaded = true;
        match read {
            Ok(touched) => {
                let (commits, paths) = touched.into_iter().map(|t| (t.commit, t.paths)).unzip();
                file.log.commits = commits;
                file.paths = paths;
                file.log.total = file.log.commits.len();
                file.log.error = None;
            }
            Err(e) => file.log.error = Some(e),
        }
    }

    pub(crate) fn request_file_commit_diff(&mut self, sha: String) {
        let (Some(repo), Some(file)) = (self.repo.clone(), self.file_history.as_mut()) else {
            return;
        };
        file.log.loading_detail = true;
        let (ask, paths) = (file.ask, file.paths_at(&sha));
        self.spawn(move |tx| {
            let read = git::show_commit_paths(&repo.root, &sha, &paths).map_err(|e| e.0);
            let _ = tx.send(Msg::FileCommitDiff { ask, sha, read });
        });
    }

    pub(crate) fn file_commit_diff_read(
        &mut self,
        ask: u64,
        sha: String,
        read: Result<Vec<FileDiff>, String>,
    ) {
        let Some(file) = self.file_history.as_mut().filter(|f| f.ask == ask) else {
            return;
        };
        file.log.loading_detail = false;
        match read {
            Ok(diff) => file.log.detail = Some((sha, diff)),
            Err(e) => self.message = Some(e),
        }
    }

    pub fn commit_link(&self) -> Result<String, String> {
        let commit = self.selected_commit().ok_or("no commit is selected")?;
        let origin = git::run(&self.root, &["remote", "get-url", "origin"])
            .map_err(|_| "this repository has no origin remote · nothing to open")?;
        let page = open::github_commit_url(&origin, &commit.sha)
            .ok_or("origin is not on GitHub · nothing to open")?;
        match branch::on_origin(&self.root, &commit.sha) {
            Ok(true) => Ok(page),
            Ok(false) => Err("this commit is not on origin yet · nothing to open".into()),
            Err(e) => Err(format!(
                "could not tell whether origin has this commit · {}",
                told(&e.0)
            )),
        }
    }
}
