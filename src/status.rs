use crate::app::{Focus, Location};
use crate::git::{self, FileDiff, Hunk, LineKind};
use crate::util::parent_dir;

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
