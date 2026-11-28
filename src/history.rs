use crate::app::{App, HistoryState, Location, Msg, View, HISTORY_LIMIT};
use crate::branches::told;
use crate::git::{self, branch, graph, Commit, FileCommit, FileDiff};
use crate::open;

#[derive(Debug)]
pub struct FileHistory {
    pub path: String,
    pub renamed_in_the_tree: Option<String>,
    pub back: View,
    pub log: HistoryState,
    pub paths: Vec<Vec<String>>,
    pub copied: Vec<bool>,
    pub loaded: bool,
    pub ask: u64,
}

impl FileHistory {
    fn new(file: HistoryFile, back: View, ask: u64) -> Self {
        FileHistory {
            path: file.path,
            renamed_in_the_tree: file.was,
            back,
            log: HistoryState::default(),
            paths: Vec::new(),
            copied: Vec::new(),
            loaded: false,
            ask,
        }
    }

    pub fn renamed_from(&self, commit: usize) -> Option<&str> {
        graph::renamed_from(self.paths.get(commit)?)
    }

    pub fn copied_from(&self, commit: usize) -> bool {
        self.copied.get(commit).copied().unwrap_or(false)
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

pub struct HistoryFile {
    pub path: String,
    pub was: Option<String>,
}

impl HistoryFile {
    fn named(path: String) -> Self {
        HistoryFile { path, was: None }
    }

    fn of(file: &FileDiff) -> Self {
        HistoryFile {
            path: file.path.clone(),
            was: file.old_path.clone(),
        }
    }
}

pub enum NoFile {
    NotARepository,
    NothingSelected,
    Folder,
    OutsideTheRepository,
}

fn not_a_file(dir: bool) -> Result<(), NoFile> {
    if dir {
        Err(NoFile::Folder)
    } else {
        Ok(())
    }
}

fn read_file_log(
    root: &std::path::Path,
    path: &str,
    was: Option<&str>,
) -> Result<Vec<FileCommit>, String> {
    let log = graph::file_log(root, path, HISTORY_LIMIT).map_err(|e| e.0)?;
    match was {
        Some(old) if log.is_empty() => graph::file_log(root, old, HISTORY_LIMIT).map_err(|e| e.0),
        _ => Ok(log),
    }
}

impl App {
    pub fn page(&self) -> View {
        if self.blame.is_some() {
            View::Blame
        } else if self.file_history.is_some() {
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

    pub fn selected_repo_file(&self) -> Result<HistoryFile, NoFile> {
        let repo = self.repo.as_ref().ok_or(NoFile::NotARepository)?;
        if let Some(page) = &self.blame {
            return Ok(HistoryFile {
                path: page.path.clone(),
                was: page.was.clone(),
            });
        }
        if let Some(file) = &self.file_history {
            return Ok(HistoryFile {
                path: file.path.clone(),
                was: file.renamed_in_the_tree.clone(),
            });
        }
        let nothing = || NoFile::NothingSelected;
        match self.view {
            View::Files => {
                let row = self.selected_file_row().ok_or_else(nothing)?;
                not_a_file(row.dir)?;
                let was = self.file_diff(&row.path).and_then(|d| d.old_path.clone());
                Ok(HistoryFile {
                    path: row.path,
                    was,
                })
            }
            View::Status => self
                .status
                .selected()
                .map(HistoryFile::of)
                .ok_or_else(nothing),
            View::Changes => self
                .diffs
                .get(self.changes.file)
                .map(HistoryFile::of)
                .ok_or_else(nothing),
            View::Custom => {
                let item = self.custom.selected().ok_or_else(nothing)?;
                not_a_file(item.dir)?;
                let inside = item
                    .path
                    .strip_prefix(&repo.root)
                    .map_err(|_| NoFile::OutsideTheRepository)?;
                Ok(HistoryFile::named(inside.to_string_lossy().into_owned()))
            }
            View::History | View::Branches | View::Worktrees | View::Blame => Err(nothing()),
        }
    }

    pub fn file_for_history(&self) -> Result<HistoryFile, String> {
        self.selected_repo_file().map_err(|why| {
            match why {
                NoFile::NotARepository => "not a git repository · no history to show",
                NoFile::NothingSelected => "no file is selected",
                NoFile::Folder => "L shows the history of a file · this is a folder",
                NoFile::OutsideTheRepository => {
                    "this file is not in the repository · no history to show"
                }
            }
            .to_string()
        })
    }

    pub fn open_file_history(&mut self) {
        match self.file_for_history() {
            Ok(file) => self.show_file_history(file),
            Err(why) => self.message = Some(why),
        }
    }

    pub(crate) fn show_file_history(&mut self, file: HistoryFile) {
        self.file_history_asks += 1;
        let back = self.view;
        if View::GIT_TABS.contains(&back) {
            self.view = View::History;
        }
        self.file_history = Some(FileHistory::new(file, back, self.file_history_asks));
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
        let (ask, path, was) = (
            file.ask,
            file.path.clone(),
            file.renamed_in_the_tree.clone(),
        );
        self.spawn(move |tx| {
            let read = if repo.head.is_none() {
                Ok(Vec::new())
            } else {
                read_file_log(&repo.root, &path, was.as_deref())
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
                file.copied = touched.iter().map(|t| t.copied).collect();
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
        self.commit_link_for(&commit.sha)
    }

    pub fn commit_link_for(&self, sha: &str) -> Result<String, String> {
        let origin = git::run(&self.root, &["remote", "get-url", "origin"])
            .map_err(|_| "this repository has no origin remote · nothing to open")?;
        let page = open::github_commit_url(&origin, sha)
            .ok_or("origin is not on GitHub · nothing to open")?;
        match branch::on_origin(&self.root, sha) {
            Ok(true) => Ok(page),
            Ok(false) => Err("this commit is not on origin yet · nothing to open".into()),
            Err(e) => Err(format!(
                "could not tell whether origin has this commit · {}",
                told(&e.0)
            )),
        }
    }
}
