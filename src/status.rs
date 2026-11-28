use crate::app::{App, Focus, Location};
use crate::ask::Ask;
use crate::commit::EMPTY_SUBJECT;
use crate::git::write::{self, restores_from_index, takes_hunks};
use crate::git::{self, FileDiff, FileStatus, GitError, Hunk, LineKind};
use crate::util::{self, parent_dir};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffRow {
    Hunk(usize),
    Line(usize, usize),
}

pub fn diff_rows(file: &FileDiff) -> Vec<DiffRow> {
    let mut rows = Vec::new();
    for (h, hunk) in file.hunks.iter().enumerate() {
        rows.push(DiffRow::Hunk(h));
        rows.extend((0..hunk.lines.len()).map(|l| DiffRow::Line(h, l)));
    }
    rows
}

fn line_after_change(hunk: &Hunk, at: usize) -> u32 {
    hunk.lines[at..]
        .iter()
        .find_map(|l| l.new_no)
        .unwrap_or(hunk.new_end())
        .max(1)
}

pub fn first_changed_line(file: &FileDiff, from_row: usize) -> Option<u32> {
    let rows = diff_rows(file);
    let changed = |row: &DiffRow| match *row {
        DiffRow::Line(h, l) if file.hunks[h].lines[l].kind != LineKind::Context => {
            Some(line_after_change(&file.hunks[h], l))
        }
        _ => None,
    };
    let from_row = from_row.min(rows.len());
    rows[from_row..]
        .iter()
        .find_map(changed)
        .or_else(|| rows[..from_row].iter().rev().find_map(changed))
}

fn by_folder_then_path(files: &mut [FileDiff]) {
    files.sort_by(|a, b| (parent_dir(&a.path), &a.path).cmp(&(parent_dir(&b.path), &b.path)));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    File,
    Hunk(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    staged: bool,
    at: usize,
    path: String,
    hunk: Option<usize>,
    scroll: usize,
}

fn changed_lines(hunks: &[Hunk]) -> String {
    let lines = hunks.iter().map(|h| h.adds() + h.dels()).sum();
    util::plural(lines, "changed line", "changed lines")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Discard {
    File(FileDiff),
    Hunk(FileDiff, usize),
}

impl Discard {
    pub fn question(&self) -> String {
        match self {
            Discard::Hunk(file, at) => format!(
                "discard {} in {} (hunk {} of {})?",
                changed_lines(&file.hunks[*at..=*at]),
                file.path,
                at + 1,
                file.hunks.len()
            ),
            Discard::File(file) => match file.status {
                FileStatus::Untracked => format!("delete the untracked file {}?", file.path),
                FileStatus::Deleted => format!("discard the deletion of {}?", file.path),
                _ if file.hunks.is_empty() => format!("discard the changes to {}?", file.path),
                _ => format!("discard {} in {}?", changed_lines(&file.hunks), file.path),
            },
        }
    }

    pub fn verb(&self) -> &'static str {
        match self {
            Discard::File(file) if file.status == FileStatus::Untracked => "deletes",
            _ => "discards",
        }
    }

    fn done(&self) -> String {
        match self {
            Discard::Hunk(file, at) => format!(
                "discarded hunk {} of {} in {}",
                at + 1,
                file.hunks.len(),
                file.path
            ),
            Discard::File(file) => match file.status {
                FileStatus::Untracked => format!("deleted {}", file.path),
                FileStatus::Deleted => format!("restored {}", file.path),
                _ => format!("discarded the changes to {}", file.path),
            },
        }
    }

    fn run(&self, root: &std::path::Path) -> Result<(), GitError> {
        match self {
            Discard::File(file) => write::discard_file(root, file),
            Discard::Hunk(file, at) => write::discard_hunk(root, file, *at),
        }
    }
}

#[derive(Debug, Default)]
pub struct StatusState {
    pub staged: Vec<FileDiff>,
    pub unstaged: Vec<FileDiff>,
    pub loaded: bool,
    pub error: Option<String>,
    pub sel: usize,
    pub list_scroll: usize,
    pub scroll: usize,
    pub focus: Focus,
}

impl StatusState {
    pub fn load(&mut self, result: Result<git::Status, String>) {
        self.loaded = true;
        match result {
            Ok(status) => {
                self.staged = status.staged;
                self.unstaged = status.unstaged;
                self.error = None;
            }
            Err(e) => {
                self.staged.clear();
                self.unstaged.clear();
                self.error = Some(e);
            }
        }
        by_folder_then_path(&mut self.staged);
        by_folder_then_path(&mut self.unstaged);
        self.sel = self.sel.min(self.len().saturating_sub(1));
        self.scroll = 0;
    }

    pub fn len(&self) -> usize {
        self.staged.len() + self.unstaged.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn selected(&self) -> Option<&FileDiff> {
        self.staged.iter().chain(&self.unstaged).nth(self.sel)
    }

    pub fn selected_is_staged(&self) -> bool {
        self.sel < self.staged.len()
    }

    pub fn move_by(&mut self, delta: i32) {
        if self.is_empty() {
            return;
        }
        let last = self.len() as i64 - 1;
        let next = (self.sel as i64 + delta as i64).clamp(0, last) as usize;
        if next != self.sel {
            self.sel = next;
            self.scroll = 0;
        }
    }

    pub fn scroll_by(&mut self, delta: i32) {
        let rows = self.selected().map(|f| diff_rows(f).len()).unwrap_or(0);
        let last = rows.saturating_sub(1) as i64;
        self.scroll = (self.scroll as i64 + delta as i64).clamp(0, last) as usize;
    }

    pub fn location(&self) -> Option<Location> {
        let file = self.selected()?;
        Some(Location {
            path: file.path.clone(),
            line: first_changed_line(file, self.scroll),
            dir: false,
        })
    }

    pub fn hunk_at_cursor(&self) -> Option<usize> {
        match *diff_rows(self.selected()?).get(self.scroll)? {
            DiffRow::Hunk(h) | DiffRow::Line(h, _) => Some(h),
        }
    }

    pub fn target(&self) -> Option<Target> {
        let file = self.selected()?;
        if self.focus == Focus::Body && takes_hunks(file) {
            self.hunk_at_cursor().map(Target::Hunk)
        } else {
            Some(Target::File)
        }
    }

    pub fn next_hunk(&mut self, delta: i32) {
        let Some(file) = self.selected() else {
            return;
        };
        let rows = diff_rows(file);
        let mut headings = (0..rows.len()).filter(|i| matches!(rows[*i], DiffRow::Hunk(_)));
        let to = if delta > 0 {
            headings.find(|row| *row > self.scroll)
        } else {
            headings.rfind(|row| *row < self.scroll)
        };
        self.scroll = to.unwrap_or(self.scroll);
    }

    pub fn place(&self) -> Place {
        let staged = self.selected_is_staged();
        Place {
            staged,
            at: self.sel - if staged { 0 } else { self.staged.len() },
            path: self.selected().map(|f| f.path.clone()).unwrap_or_default(),
            hunk: match self.target() {
                Some(Target::Hunk(h)) => Some(h),
                _ => None,
            },
            scroll: self.scroll,
        }
    }

    pub fn restore(&mut self, place: &Place) {
        let first_unstaged = self.staged.len();
        let (group, first, other, other_first) = if place.staged {
            (&self.staged, 0, &self.unstaged, first_unstaged)
        } else {
            (&self.unstaged, first_unstaged, &self.staged, 0)
        };
        let find = |files: &[FileDiff]| files.iter().position(|f| f.path == place.path);
        let same_entry = find(group);
        self.sel = match same_entry {
            Some(at) => first + at,
            None if !group.is_empty() => first + place.at.min(group.len() - 1),
            None => match find(other) {
                Some(at) => other_first + at,
                None => first.min(self.len().saturating_sub(1)),
            },
        };
        if group.is_empty() {
            self.focus = Focus::Rail;
        }
        self.scroll = 0;
        let (Some(_), Some(file)) = (same_entry, self.selected()) else {
            return;
        };
        let rows = diff_rows(file);
        self.scroll = match place.hunk {
            Some(h) => {
                let near = h.min(file.hunks.len().saturating_sub(1));
                rows.iter()
                    .position(|row| *row == DiffRow::Hunk(near))
                    .unwrap_or(0)
            }
            None => place.scroll.min(rows.len().saturating_sub(1)),
        };
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl App {
    fn after_write(&mut self, place: &Place, result: Result<String, GitError>) {
        self.reload_status();
        self.status.restore(place);
        self.message = Some(match result {
            Ok(done) => done,
            Err(e) => one_line(&e.0),
        });
    }

    pub fn toggle_staged(&mut self) {
        let (Some(file), Some(target)) = (self.status.selected().cloned(), self.status.target())
        else {
            return;
        };
        let root = self.root.clone();
        let staged = self.status.selected_is_staged();
        let result = match (staged, target) {
            (false, Target::File) => write::stage_file(&root, &file),
            (false, Target::Hunk(h)) => write::stage_hunk(&root, &file, h),
            (true, Target::File) => write::unstage_file(&root, &file),
            (true, Target::Hunk(h)) => write::unstage_hunk(&root, &file, h),
        };
        let verb = if staged { "unstaged" } else { "staged" };
        let done = match target {
            Target::File => format!("{verb} {}", file.path),
            Target::Hunk(h) => format!(
                "{verb} hunk {} of {} in {}",
                h + 1,
                file.hunks.len(),
                file.path
            ),
        };
        let place = self.status.place();
        self.after_write(&place, result.map(|()| done));
    }

    pub fn ask_discard(&mut self) {
        let (Some(file), Some(target)) = (self.status.selected().cloned(), self.status.target())
        else {
            return;
        };
        if self.status.selected_is_staged() {
            self.message =
                Some("x discards only what is not staged · space unstages this first".into());
            return;
        }
        let asked = match target {
            Target::Hunk(h) => Discard::Hunk(file, h),
            Target::File if file.status == FileStatus::Untracked || restores_from_index(&file) => {
                Discard::File(file)
            }
            Target::File => {
                self.message = Some(format!(
                    "{} is new to the index · there is nothing to restore it from",
                    file.path
                ));
                return;
            }
        };
        self.confirm = Some(Ask::Discard(asked));
    }

    pub(crate) fn discard(&mut self, asked: &Discard) {
        let result = asked.run(&self.root);
        let place = self.status.place();
        self.after_write(&place, result.map(|()| asked.done()));
        self.reload_review();
    }

    pub fn commit_title(&self) -> String {
        match self.repo.as_ref().and_then(|r| r.branch.as_deref()) {
            Some(branch) => format!("commit to {branch}"),
            None => "commit on a detached HEAD".into(),
        }
    }

    pub fn open_commit_box(&mut self) {
        if !self.status.loaded || self.repo.is_none() {
            return;
        }
        if self.status.staged.is_empty() {
            self.message = Some("nothing staged to commit · space stages the selected file".into());
            return;
        }
        self.commit.open();
    }

    pub fn submit_commit(&mut self) {
        match self.commit.message() {
            Some((subject, body)) => {
                self.commit.busy = true;
                self.request_commit(subject, body);
            }
            None => self.commit.refuse(EMPTY_SUBJECT),
        }
    }

    pub fn commit_finished(&mut self, result: Result<(String, String), String>) {
        self.repaint = true;
        match result {
            Ok((short, subject)) => {
                self.commit.committed();
                let place = self.status.place();
                self.reload_after_commit();
                self.status.restore(&place);
                self.message = Some(format!("committed {short} {subject}"));
            }
            Err(said) => self.commit.refuse(&said),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::diff;

    const TWO_HUNKS: &str = "diff --git a/src/api.py b/src/api.py
--- a/src/api.py
+++ b/src/api.py
@@ -1,4 +1,5 @@ import json
 import json
-import os
+import sys
+import time
 from pathlib import Path
 def main():
@@ -10,3 +11,2 @@ def main():
     x = 1
-    return x
     y = 2
";

    fn file(text: &str) -> FileDiff {
        diff::parse(text).remove(0)
    }

    fn named(path: &str) -> FileDiff {
        FileDiff {
            path: path.into(),
            ..file(TWO_HUNKS)
        }
    }

    fn state(staged: &[&str], unstaged: &[&str]) -> StatusState {
        let mut state = StatusState::default();
        state.load(Ok(git::Status {
            staged: staged.iter().map(|p| named(p)).collect(),
            unstaged: unstaged.iter().map(|p| named(p)).collect(),
        }));
        state
    }

    fn paths(files: &[FileDiff]) -> Vec<&str> {
        files.iter().map(|f| f.path.as_str()).collect()
    }

    #[test]
    fn rows_are_each_hunk_header_then_its_lines() {
        let rows = diff_rows(&file(TWO_HUNKS));
        assert_eq!(rows.len(), 11);
        assert_eq!(rows[0], DiffRow::Hunk(0));
        assert_eq!(rows[1], DiffRow::Line(0, 0));
        assert_eq!(rows[7], DiffRow::Hunk(1));
        assert_eq!(rows[10], DiffRow::Line(1, 2));
    }

    #[test]
    fn the_first_changed_line_follows_the_scroll() {
        let f = file(TWO_HUNKS);
        assert_eq!(first_changed_line(&f, 0), Some(2));
        assert_eq!(first_changed_line(&f, 3), Some(2));
        assert_eq!(first_changed_line(&f, 4), Some(3));
        assert_eq!(first_changed_line(&f, 5), Some(12), "a removed line");
        assert_eq!(first_changed_line(&f, 10), Some(12), "the change above");
        assert_eq!(first_changed_line(&f, 99), Some(12));
    }

    #[test]
    fn a_removed_file_and_a_file_without_text_changes() {
        let gone = file("diff --git a/g.py b/g.py\ndeleted file mode 100644\n--- a/g.py\n+++ /dev/null\n@@ -1 +0,0 @@\n-x = 1\n");
        assert_eq!(first_changed_line(&gone, 0), Some(1));
        let renamed = file("diff --git a/a.py b/b.py\nrename from a.py\nrename to b.py\n");
        assert_eq!(first_changed_line(&renamed, 0), None);
    }

    #[test]
    fn files_are_ordered_by_folder_so_the_list_reads_top_to_bottom() {
        let s = state(&["src/ui/mod.rs", "src/app.rs", "README.md"], &["src/a.rs"]);
        assert_eq!(
            paths(&s.staged),
            ["README.md", "src/app.rs", "src/ui/mod.rs"]
        );
        assert_eq!(s.len(), 4);
    }

    #[test]
    fn one_selection_moves_through_both_lists() {
        let mut s = state(&["a.py"], &["a.py", "b.py"]);
        assert!(s.selected_is_staged());
        s.scroll_by(4);
        assert_eq!(s.scroll, 4);
        s.move_by(1);
        assert_eq!((s.sel, s.scroll), (1, 0), "a new file starts at its top");
        assert!(!s.selected_is_staged());
        assert_eq!(s.selected().unwrap().path, "a.py");
        s.move_by(15);
        assert_eq!(s.selected().unwrap().path, "b.py");
        s.move_by(-(i32::MAX / 2));
        assert_eq!(s.sel, 0);
    }

    #[test]
    fn the_body_scroll_stops_at_the_last_row() {
        let mut s = state(&["a.py"], &[]);
        s.scroll_by(i32::MAX / 2);
        assert_eq!(s.scroll, 10);
        s.scroll_by(-3);
        assert_eq!(s.scroll, 7);
        s.scroll_by(-99);
        assert_eq!(s.scroll, 0);
    }

    #[test]
    fn the_location_is_the_selected_file_at_its_first_change() {
        let mut s = state(&["a.py"], &["b.py"]);
        s.move_by(1);
        let at = s.location().unwrap();
        assert_eq!(
            (at.path.as_str(), at.line, at.dir),
            ("b.py", Some(2), false)
        );
        s.scroll_by(8);
        assert_eq!(s.location().unwrap().line, Some(12));
        assert_eq!(StatusState::default().location(), None);
    }

    fn in_diff(mut s: StatusState) -> StatusState {
        s.focus = Focus::Body;
        s
    }

    #[test]
    fn the_list_acts_on_the_file_and_the_diff_on_the_hunk_at_its_top_row() {
        let mut s = state(&["a.py"], &["b.py"]);
        s.focus = Focus::Rail;
        s.scroll_by(8);
        assert_eq!(s.target(), Some(Target::File));
        let mut s = in_diff(s);
        assert_eq!(s.target(), Some(Target::Hunk(1)));
        s.scroll_by(-1);
        assert_eq!(s.target(), Some(Target::Hunk(1)), "its heading");
        s.scroll_by(-1);
        assert_eq!(s.target(), Some(Target::Hunk(0)), "the row above");
        assert_eq!(in_diff(StatusState::default()).target(), None);
    }

    #[test]
    fn the_diff_acts_on_the_file_when_a_hunk_alone_means_nothing() {
        let whole_file_only = [
            FileDiff {
                status: FileStatus::Untracked,
                ..named("new.py")
            },
            FileDiff {
                status: FileStatus::Added,
                ..named("added.py")
            },
            FileDiff {
                status: FileStatus::Deleted,
                ..named("gone.py")
            },
            FileDiff {
                status: FileStatus::Renamed,
                old_path: Some("before.py".into()),
                ..named("after.py")
            },
            FileDiff {
                binary: true,
                hunks: Vec::new(),
                ..named("logo.png")
            },
            FileDiff {
                hunks: Vec::new(),
                ..named("mode.sh")
            },
        ];
        for file in whole_file_only {
            let mut s = in_diff(StatusState::default());
            s.load(Ok(git::Status {
                staged: Vec::new(),
                unstaged: vec![file.clone()],
            }));
            assert_eq!(s.target(), Some(Target::File), "{}", file.path);
        }
    }

    #[test]
    fn the_hunk_keys_step_between_hunk_headings() {
        let mut s = in_diff(state(&["a.py"], &[]));
        s.next_hunk(1);
        assert_eq!(s.scroll, 7);
        s.next_hunk(1);
        assert_eq!(s.scroll, 7, "the last hunk");
        s.scroll_by(2);
        s.next_hunk(-1);
        assert_eq!(s.scroll, 7, "first to the heading of this hunk");
        s.next_hunk(-1);
        assert_eq!(s.scroll, 0);
        s.next_hunk(-1);
        assert_eq!(s.scroll, 0);
        let mut empty = in_diff(StatusState::default());
        empty.next_hunk(1);
        assert_eq!(empty.scroll, 0);
    }

    fn reloaded(mut s: StatusState, staged: &[&str], unstaged: &[&str]) -> StatusState {
        let place = s.place();
        s.load(Ok(git::Status {
            staged: staged.iter().map(|p| named(p)).collect(),
            unstaged: unstaged.iter().map(|p| named(p)).collect(),
        }));
        s.restore(&place);
        s
    }

    fn at(s: &StatusState) -> (&str, bool) {
        (s.selected().unwrap().path.as_str(), s.selected_is_staged())
    }

    #[test]
    fn after_a_write_the_selection_stays_in_its_list() {
        let mut s = state(&["x.py"], &["a.py", "b.py", "c.py"]);
        s.focus = Focus::Rail;
        s.move_by(2);
        s.scroll_by(3);
        let kept = reloaded(s, &["x.py"], &["a.py", "b.py", "c.py"]);
        assert_eq!((at(&kept), kept.scroll), (("b.py", false), 3));

        let mut s = state(&["x.py"], &["a.py", "b.py", "c.py"]);
        s.move_by(2);
        let next = reloaded(s, &["b.py", "x.py"], &["a.py", "c.py"]);
        assert_eq!((at(&next), next.scroll), (("c.py", false), 0));

        let mut s = state(&[], &["a.py", "b.py"]);
        s.move_by(1);
        let above = reloaded(s, &["b.py"], &["a.py"]);
        assert_eq!(
            at(&above),
            ("a.py", false),
            "the last one gives way upwards"
        );

        let s = state(&["a.py", "b.py"], &["c.py"]);
        let unstaged = reloaded(s, &["b.py"], &["a.py", "c.py"]);
        assert_eq!(at(&unstaged), ("b.py", true));
    }

    #[test]
    fn when_its_list_empties_the_selection_follows_the_file_or_stays_in_range() {
        let mut s = in_diff(state(&["x.py"], &["a.py"]));
        s.move_by(1);
        let followed = reloaded(s, &["a.py", "x.py"], &[]);
        assert_eq!(at(&followed), ("a.py", true));
        assert_eq!(followed.focus, Focus::Rail, "back to the list");

        let mut s = state(&["x.py", "y.py"], &["a.py"]);
        s.move_by(2);
        let discarded = reloaded(s, &["x.py", "y.py"], &[]);
        assert_eq!(at(&discarded), ("y.py", true));

        let s = state(&["x.py"], &["a.py"]);
        let committed = reloaded(s, &[], &["a.py"]);
        assert_eq!(at(&committed), ("a.py", false));

        let s = in_diff(state(&["x.py"], &[]));
        let clean = reloaded(s, &[], &[]);
        assert_eq!(
            (clean.sel, clean.selected(), clean.focus),
            (0, None, Focus::Rail)
        );
    }

    #[test]
    fn after_a_hunk_is_written_the_cursor_is_on_the_hunk_that_took_its_place() {
        let one_hunk = || FileDiff {
            hunks: file(TWO_HUNKS).hunks[1..].to_vec(),
            ..named("a.py")
        };
        let mut s = in_diff(state(&[], &["a.py"]));
        let first = s.place();
        s.load(Ok(git::Status {
            staged: vec![one_hunk()],
            unstaged: vec![one_hunk()],
        }));
        s.restore(&first);
        assert_eq!(
            (at(&s), s.scroll, s.focus),
            (("a.py", false), 0, Focus::Body)
        );

        let mut s = in_diff(state(&[], &["a.py"]));
        s.next_hunk(1);
        let last = s.place();
        s.load(Ok(git::Status {
            staged: vec![one_hunk()],
            unstaged: vec![one_hunk()],
        }));
        s.restore(&last);
        assert_eq!(s.target(), Some(Target::Hunk(0)), "the one above it");
    }

    #[test]
    fn a_discard_question_names_what_is_lost() {
        let modified = named("src/app.py");
        assert_eq!(
            Discard::File(modified.clone()).question(),
            "discard 4 changed lines in src/app.py?"
        );
        assert_eq!(
            Discard::Hunk(modified.clone(), 1).question(),
            "discard 1 changed line in src/app.py (hunk 2 of 2)?"
        );
        let untracked = Discard::File(FileDiff {
            status: FileStatus::Untracked,
            ..named("notes.md")
        });
        assert_eq!(untracked.question(), "delete the untracked file notes.md?");
        assert_eq!(untracked.verb(), "deletes");
        let deleted = Discard::File(FileDiff {
            status: FileStatus::Deleted,
            ..named("gone.py")
        });
        assert_eq!(deleted.question(), "discard the deletion of gone.py?");
        let binary = Discard::File(FileDiff {
            binary: true,
            hunks: Vec::new(),
            ..named("logo.png")
        });
        assert_eq!(binary.question(), "discard the changes to logo.png?");
        assert_eq!(binary.verb(), "discards");
    }

    #[test]
    fn a_message_from_git_fits_one_line() {
        assert_eq!(
            one_line("error: the following file has staged content\nfile and the HEAD:\n    f\n"),
            "error: the following file has staged content file and the HEAD: f"
        );
    }

    #[test]
    fn a_failed_read_keeps_the_error_and_an_empty_list() {
        let mut s = state(&["a.py"], &["b.py"]);
        s.move_by(1);
        s.load(Err("index file corrupt".into()));
        assert!(s.loaded && s.is_empty());
        assert_eq!(s.sel, 0);
        assert_eq!(s.error.as_deref(), Some("index file corrupt"));
    }
}
