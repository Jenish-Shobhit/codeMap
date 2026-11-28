use crate::app::{App, Location, Msg};
use crate::branches::told;
use crate::git::blame::{self, BlameCommit, BlameLine, BlameRead, Blamed};
use crate::git::{self, FileDiff};
use crate::history::{HistoryFile, NoFile};
use crate::util;

pub const NOT_COMMITTED: &str = "this line is not committed yet";

#[derive(Debug)]
pub struct LineDiff {
    pub sha: String,
    pub files: Option<Vec<FileDiff>>,
    pub scroll: usize,
}

#[derive(Debug)]
pub struct BlamePage {
    pub path: String,
    pub was: Option<String>,
    pub read: Option<Result<BlameRead, String>>,
    pub sel: usize,
    pub scroll: usize,
    pub wanted: u32,
    pub centre: bool,
    pub author_width: usize,
    pub diff: Option<LineDiff>,
    pub ask: u64,
}

impl BlamePage {
    fn new(file: HistoryFile, wanted: u32, ask: u64) -> Self {
        BlamePage {
            path: file.path,
            was: file.was,
            read: None,
            sel: 0,
            scroll: 0,
            wanted: wanted.max(1),
            centre: true,
            author_width: 0,
            diff: None,
            ask,
        }
    }

    pub fn blamed(&self) -> Option<&Blamed> {
        match &self.read {
            Some(Ok(BlameRead::Lines(blamed))) => Some(blamed),
            _ => None,
        }
    }

    pub fn line_count(&self) -> usize {
        self.blamed().map_or(0, |b| b.lines.len())
    }

    pub fn selected_line(&self) -> Option<&BlameLine> {
        self.blamed()?.lines.get(self.sel)
    }

    pub fn selected_commit(&self) -> Option<&BlameCommit> {
        let blamed = self.blamed()?;
        blamed.commit_of(blamed.lines.get(self.sel)?)
    }

    pub fn on_a_committed_line(&self) -> bool {
        self.selected_commit().is_some()
    }

    pub fn selected_number(&self) -> u32 {
        self.selected_line().map_or(self.wanted, |l| l.number)
    }

    pub fn move_by(&mut self, delta: i64) {
        let count = self.line_count();
        if count == 0 {
            return;
        }
        self.sel = (self.sel as i64)
            .saturating_add(delta)
            .clamp(0, count as i64 - 1) as usize;
    }

    fn land_on_wanted_line(&mut self) {
        let Some(blamed) = self.blamed() else {
            return;
        };
        let last = blamed.lines.len().saturating_sub(1);
        let at = blamed.lines.iter().position(|l| l.number >= self.wanted);
        let author_width = blamed
            .commits
            .iter()
            .map(|c| util::width(&util::sanitize_line(&c.author)))
            .max()
            .unwrap_or(0);
        self.sel = at.unwrap_or(last);
        self.author_width = author_width;
    }
}

fn not_a_file_to_blame(why: NoFile) -> String {
    match why {
        NoFile::NotARepository => "not a git repository · nothing to blame",
        NoFile::NothingSelected => "no file is selected",
        NoFile::Folder => "A blames a file · this is a folder",
        NoFile::OutsideTheRepository => "this file is not in the repository · nothing to blame",
    }
    .to_string()
}

impl App {
    pub fn file_for_blame(&self) -> Result<HistoryFile, String> {
        self.selected_repo_file().map_err(not_a_file_to_blame)
    }

    pub fn open_blame(&mut self) {
        let file = match self.file_for_blame() {
            Ok(file) => file,
            Err(why) => {
                self.message = Some(why);
                return;
            }
        };
        let wanted = self.location().and_then(|l| l.line).unwrap_or(1);
        self.blame_asks += 1;
        self.blame = Some(BlamePage::new(file, wanted, self.blame_asks));
        self.request_blame();
        self.drain_if_sync();
    }

    pub fn leave_blame(&mut self) {
        let Some(page) = self.blame.as_mut() else {
            return;
        };
        if page.diff.take().is_none() {
            self.blame = None;
        }
    }

    pub fn blame_to_history(&mut self) {
        let Some(page) = self.blame.take() else {
            return;
        };
        if self.file_history.is_none() {
            self.show_file_history(HistoryFile {
                path: page.path,
                was: page.was,
            });
        }
    }

    pub fn blame_location(&self) -> Option<Location> {
        let page = self.blame.as_ref()?;
        Some(Location {
            path: page.path.clone(),
            line: Some(page.selected_number()),
            dir: false,
        })
    }

    fn request_blame(&mut self) {
        let (Some(repo), Some(page)) = (self.repo.clone(), self.blame.as_ref()) else {
            return;
        };
        let (ask, path, was) = (page.ask, page.path.clone(), page.was.clone());
        let scratch = self.store.shadow(&repo).dir;
        self.spawn(move |tx| {
            let read = blame::blame(&repo, &scratch, &path, was.as_deref()).map_err(|e| e.0);
            let _ = tx.send(Msg::Blame { ask, read });
        });
    }

    pub(crate) fn blame_read(&mut self, ask: u64, read: Result<BlameRead, String>) {
        let Some(page) = self.blame.as_mut().filter(|p| p.ask == ask) else {
            return;
        };
        page.read = Some(read);
        page.land_on_wanted_line();
    }

    pub fn blame_commit(&self) -> Result<&BlameCommit, String> {
        let page = self.blame.as_ref().ok_or("no file is blamed")?;
        match (page.selected_line(), page.selected_commit()) {
            (_, Some(commit)) => Ok(commit),
            (Some(_), None) => Err(NOT_COMMITTED.to_string()),
            (None, None) => Err("no line is selected".to_string()),
        }
    }

    pub fn blame_commit_link(&self) -> Result<String, String> {
        let sha = self.blame_commit()?.sha.clone();
        self.commit_link_for(&sha)
    }

    pub fn open_blame_diff(&mut self) {
        let commit = match self.blame_commit() {
            Ok(commit) => commit,
            Err(why) => {
                self.message = Some(why);
                return;
            }
        };
        let (sha, paths) = (commit.sha.clone(), commit.paths());
        let (Some(repo), Some(page)) = (self.repo.clone(), self.blame.as_mut()) else {
            return;
        };
        page.diff = Some(LineDiff {
            sha: sha.clone(),
            files: None,
            scroll: 0,
        });
        let ask = page.ask;
        self.spawn(move |tx| {
            let read = git::show_commit_paths(&repo.root, &sha, &paths).map_err(|e| e.0);
            let _ = tx.send(Msg::BlameDiff { ask, sha, read });
        });
        self.drain_if_sync();
    }

    pub(crate) fn blame_diff_read(
        &mut self,
        ask: u64,
        sha: String,
        read: Result<Vec<FileDiff>, String>,
    ) {
        let Some(page) = self.blame.as_mut().filter(|p| p.ask == ask) else {
            return;
        };
        let Some(diff) = page.diff.as_mut().filter(|d| d.sha == sha) else {
            return;
        };
        match read {
            Ok(files) => diff.files = Some(files),
            Err(e) => {
                page.diff = None;
                self.message = Some(format!("could not read that commit · {}", told(&e)));
            }
        }
    }
}
