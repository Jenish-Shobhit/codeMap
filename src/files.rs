use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;
use std::time::SystemTime;

use crate::app::{App, Focus, Location};
use crate::git::{FileDiff, LineKind};
use crate::util;

pub const MAX_FILES: usize = 20_000;
pub const PREVIEW_BYTES: u64 = 512 * 1024;
pub const MATCH_LIMIT: usize = 200;
pub const REVEAL_LIMIT: usize = 40;
const BINARY_PROBE: usize = 8_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub path: String,
    pub depth: usize,
    pub dir: bool,
}

impl Row {
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

#[derive(Debug, Clone, Default)]
struct Children {
    dirs: BTreeSet<String>,
    files: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Tree {
    files: Vec<String>,
    children: BTreeMap<String, Children>,
}

impl Tree {
    pub fn new(mut files: Vec<String>) -> Tree {
        files.sort();
        files.dedup();
        let mut children: BTreeMap<String, Children> = BTreeMap::new();
        for file in &files {
            let mut child = file.as_str();
            let mut is_file = true;
            loop {
                let parent = util::parent_dir(child);
                let entry = children.entry(parent.to_string()).or_default();
                if is_file {
                    entry.files.push(child.to_string());
                } else if !entry.dirs.insert(child.to_string()) {
                    break;
                }
                if parent.is_empty() {
                    break;
                }
                child = parent;
                is_file = false;
            }
        }
        Tree { files, children }
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn has_file(&self, file: &str) -> bool {
        self.files
            .binary_search_by(|f| f.as_str().cmp(file))
            .is_ok()
    }

    pub fn has_dir(&self, dir: &str) -> bool {
        !dir.is_empty() && self.children.contains_key(dir)
    }

    pub fn count_under(&self, dir: &str) -> usize {
        self.files.iter().filter(|f| in_dir(f, dir)).count()
    }

    pub fn rows(&self, open: &BTreeSet<String>) -> Vec<Row> {
        let mut rows = Vec::new();
        self.push_children("", 0, open, &mut rows);
        rows
    }

    fn push_children(&self, dir: &str, depth: usize, open: &BTreeSet<String>, rows: &mut Vec<Row>) {
        let Some(children) = self.children.get(dir) else {
            return;
        };
        for sub in &children.dirs {
            rows.push(Row {
                path: sub.clone(),
                depth,
                dir: true,
            });
            if open.contains(sub) {
                self.push_children(sub, depth + 1, open, rows);
            }
        }
        for file in &children.files {
            rows.push(Row {
                path: file.clone(),
                depth,
                dir: false,
            });
        }
    }

    pub fn matches(&self, query: &str) -> Vec<Row> {
        let mut scored: Vec<(i64, &String)> = self
            .files
            .iter()
            .filter_map(|f| {
                let name = f.rsplit('/').next().unwrap_or(f);
                let by_name = util::fuzzy_score(query, name).map(|s| s + 100);
                let by_path = util::fuzzy_score(query, f);
                by_name.max(by_path).map(|s| (s, f))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
        scored
            .into_iter()
            .take(MATCH_LIMIT)
            .map(|(_, f)| Row {
                path: f.clone(),
                depth: 0,
                dir: false,
            })
            .collect()
    }
}

pub fn in_dir(file: &str, dir: &str) -> bool {
    dir.is_empty()
        || (file.len() > dir.len() && file.starts_with(dir) && file.as_bytes()[dir.len()] == b'/')
}

pub fn ancestors(path: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut cur = util::parent_dir(path);
    while !cur.is_empty() {
        out.push(cur);
        cur = util::parent_dir(cur);
    }
    out.reverse();
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Text(Vec<String>),
    Binary,
    Empty,
    Unreadable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub path: String,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub truncated: bool,
    pub body: Body,
}

impl Preview {
    pub fn lines(&self) -> usize {
        match &self.body {
            Body::Text(lines) => lines.len(),
            _ => 0,
        }
    }
}

pub fn preview(root: &Path, rel: &str) -> Preview {
    let full = root.join(rel);
    let meta = std::fs::metadata(&full).ok();
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let modified = modified_at(root, rel);
    let mut bytes = Vec::new();
    let read =
        std::fs::File::open(&full).and_then(|f| f.take(PREVIEW_BYTES).read_to_end(&mut bytes));
    let truncated = size > PREVIEW_BYTES;
    let body = match read {
        Err(e) => Body::Unreadable(e.to_string()),
        Ok(_) if bytes.is_empty() => Body::Empty,
        Ok(_) if bytes.iter().take(BINARY_PROBE).any(|b| *b == 0) => Body::Binary,
        Ok(_) => {
            let text = String::from_utf8_lossy(&bytes);
            let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
            if truncated {
                lines.pop();
            }
            Body::Text(lines)
        }
    };
    Preview {
        path: rel.to_string(),
        size,
        modified,
        truncated,
        body,
    }
}

fn modified_at(root: &Path, rel: &str) -> Option<SystemTime> {
    std::fs::metadata(root.join(rel))
        .and_then(|m| m.modified())
        .ok()
}

pub fn human_size(n: u64) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{} KB", n / 1024)
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}

#[derive(Debug, Default)]
pub struct FilesState {
    pub tree: Tree,
    pub loaded: bool,
    pub truncated: bool,
    pub placed: bool,
    pub open: BTreeSet<String>,
    pub sel: usize,
    pub scroll: usize,
    pub focus: Focus,
    pub filter: Option<String>,
    pub before_filter: Option<String>,
    pub preview: Option<Preview>,
    pub preview_scroll: usize,
}

impl App {
    pub fn file_rows(&self) -> Vec<Row> {
        match self.files.filter.as_deref().filter(|q| !q.is_empty()) {
            Some(query) => self.files.tree.matches(query),
            None => self.files.tree.rows(&self.files.open),
        }
    }

    pub fn selected_file_row(&self) -> Option<Row> {
        self.file_rows().into_iter().nth(self.files.sel)
    }

    pub fn file_diff(&self, path: &str) -> Option<&FileDiff> {
        self.diffs.iter().find(|d| d.path == path)
    }

    pub fn changed_under(&self, dir: &str) -> Vec<&FileDiff> {
        self.diffs.iter().filter(|d| in_dir(&d.path, dir)).collect()
    }

    pub fn added_lines(&self, path: &str) -> BTreeSet<u32> {
        self.file_diff(path)
            .into_iter()
            .flat_map(|d| &d.hunks)
            .flat_map(|h| &h.lines)
            .filter(|l| l.kind == LineKind::Add)
            .filter_map(|l| l.new_no)
            .collect()
    }

    pub fn place_files(&mut self) {
        let waiting_for_diffs = self.repo.is_some() && !self.diffs_loaded;
        if self.files.placed || !self.files.loaded || waiting_for_diffs {
            return;
        }
        self.files.placed = true;
        let changed: Vec<String> = self
            .diffs
            .iter()
            .map(|d| d.path.clone())
            .filter(|p| self.files.tree.has_file(p))
            .take(REVEAL_LIMIT)
            .collect();
        for path in &changed {
            for dir in ancestors(path) {
                self.files.open.insert(dir.to_string());
            }
        }
        if let Some(first) = changed.first() {
            self.select_path(first);
        }
    }

    pub fn select_path(&mut self, path: &str) {
        for dir in ancestors(path) {
            self.files.open.insert(dir.to_string());
        }
        let at = self.file_rows().iter().position(|r| r.path == path);
        self.files.sel = at.unwrap_or(0);
        self.files.preview_scroll = 0;
    }

    pub fn files_move(&mut self, delta: i32) {
        let count = self.file_rows().len();
        if count == 0 {
            return;
        }
        let next = (self.files.sel as i64 + delta as i64).clamp(0, count as i64 - 1) as usize;
        if next != self.files.sel {
            self.files.sel = next;
            self.files.preview_scroll = 0;
        }
    }

    pub fn files_toggle(&mut self) {
        let Some(row) = self.selected_file_row().filter(|r| r.dir) else {
            return;
        };
        if !self.files.open.remove(&row.path) {
            self.files.open.insert(row.path);
        }
    }

    pub fn files_right(&mut self) {
        let Some(row) = self.selected_file_row() else {
            return;
        };
        if !row.dir {
            self.files.focus = Focus::Body;
        } else if !self.files.open.insert(row.path) {
            self.files_move(1);
        }
    }

    pub fn files_left(&mut self) {
        let Some(row) = self.selected_file_row() else {
            return;
        };
        if row.dir && self.files.open.remove(&row.path) {
            return;
        }
        let parent = util::parent_dir(&row.path).to_string();
        if !parent.is_empty() && self.files.filter.is_none() {
            self.select_path(&parent);
        }
    }

    pub fn start_filter(&mut self) {
        self.files.before_filter = self.selected_file_row().map(|r| r.path);
        self.files.filter = Some(String::new());
        self.files.focus = Focus::Rail;
    }

    pub fn update_filter(&mut self, text: &str) {
        self.files.filter = Some(text.to_string());
        self.files.sel = 0;
        self.files.scroll = 0;
        self.files.preview_scroll = 0;
    }

    pub fn finish_filter(&mut self, accept: bool) {
        let picked = self
            .selected_file_row()
            .filter(|_| accept)
            .map(|r| r.path)
            .or_else(|| self.files.before_filter.take());
        self.files.filter = None;
        self.files.before_filter = None;
        self.files.scroll = 0;
        match picked {
            Some(path) => self.select_path(&path),
            None => self.files.sel = 0,
        }
    }

    pub fn ensure_preview(&mut self) {
        let Some(path) = self.selected_file_row().filter(|r| !r.dir).map(|r| r.path) else {
            self.files.preview = None;
            return;
        };
        let current = self
            .files
            .preview
            .as_ref()
            .is_some_and(|p| p.path == path && p.modified == modified_at(&self.root, &path));
        if !current {
            self.files.preview = Some(preview(&self.root, &path));
        }
    }

    pub fn preview_scroll_by(&mut self, delta: i32) {
        let lines = self.files.preview.as_ref().map(Preview::lines).unwrap_or(0);
        let last = lines.saturating_sub(1) as i64;
        self.files.preview_scroll =
            (self.files.preview_scroll as i64 + delta as i64).clamp(0, last) as usize;
    }

    pub fn file_location(&self) -> Option<Location> {
        let row = self.selected_file_row()?;
        let line = if row.dir {
            None
        } else if self.files.focus == Focus::Body {
            Some(self.files.preview_scroll as u32 + 1)
        } else {
            self.added_lines(&row.path).into_iter().next()
        };
        Some(Location {
            path: row.path,
            line,
            dir: row.dir,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> Tree {
        Tree::new(
            [
                "src/ui/mod.rs",
                "README.md",
                "src/app.rs",
                "src/ui/files.rs",
                "Cargo.toml",
                "docs/assets/files.svg",
            ]
            .map(String::from)
            .to_vec(),
        )
    }

    fn shown(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| {
                format!(
                    "{}{}{}",
                    "  ".repeat(r.depth),
                    r.name(),
                    if r.dir { "/" } else { "" }
                )
            })
            .collect()
    }

    #[test]
    fn closed_tree_lists_folders_before_files() {
        let rows = tree().rows(&BTreeSet::new());
        assert_eq!(shown(&rows), ["docs/", "src/", "Cargo.toml", "README.md"]);
    }

    #[test]
    fn open_folders_show_their_children_indented() {
        let open: BTreeSet<String> = ["src", "src/ui"].map(String::from).into();
        let rows = tree().rows(&open);
        assert_eq!(
            shown(&rows),
            [
                "docs/",
                "src/",
                "  ui/",
                "    files.rs",
                "    mod.rs",
                "  app.rs",
                "Cargo.toml",
                "README.md"
            ]
        );
        assert_eq!(rows[3].path, "src/ui/files.rs");
    }

    #[test]
    fn a_folder_open_inside_a_closed_one_stays_hidden() {
        let open: BTreeSet<String> = ["src/ui"].map(String::from).into();
        assert_eq!(tree().rows(&open).len(), 4);
    }

    #[test]
    fn counts_and_folder_lookup() {
        let t = tree();
        assert_eq!(t.len(), 6);
        assert_eq!(t.count_under("src"), 3);
        assert_eq!(t.count_under(""), 6);
        assert!(t.has_file("src/app.rs"));
        assert!(!t.has_file("src"));
        assert!(t.has_dir("src/ui"));
        assert!(!t.has_dir("src/app.rs"));
        assert!(!t.has_dir(""));
    }

    #[test]
    fn matches_rank_file_names_above_folder_names() {
        let t = Tree::new(
            ["files/readme.md", "src/files.rs", "src/app.rs"]
                .map(String::from)
                .to_vec(),
        );
        let hits: Vec<String> = t.matches("files").into_iter().map(|r| r.path).collect();
        assert_eq!(hits, ["src/files.rs", "files/readme.md"]);
        assert!(t.matches("zzz").is_empty());
    }

    #[test]
    fn ancestors_run_from_the_top_down() {
        assert_eq!(ancestors("a/b/c.rs"), ["a", "a/b"]);
        assert!(ancestors("c.rs").is_empty());
        assert!(in_dir("a/b/c.rs", "a"));
        assert!(!in_dir("ab/c.rs", "a"));
    }

    #[test]
    fn previews_tell_text_from_binary_and_empty() {
        let dir = std::env::temp_dir().join(format!("codemap-preview-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(dir.join("b.bin"), [1u8, 0, 2]).unwrap();
        std::fs::write(dir.join("c.txt"), "").unwrap();
        let text = preview(&dir, "a.txt");
        assert_eq!(text.body, Body::Text(vec!["one".into(), "two".into()]));
        assert_eq!((text.size, text.truncated, text.lines()), (8, false, 2));
        assert_eq!(preview(&dir, "b.bin").body, Body::Binary);
        assert_eq!(preview(&dir, "c.txt").body, Body::Empty);
        assert!(matches!(preview(&dir, "gone").body, Body::Unreadable(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sizes_read_in_bytes_kilobytes_and_megabytes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(24 * 1024), "24 KB");
        assert_eq!(human_size(3 * 1024 * 1024 / 2), "1.5 MB");
    }
}
