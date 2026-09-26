//! Unified diff parsing (`git diff` / `git show` output) and the line sets the
//! Map and Flow views use to mark what changed.

use std::collections::BTreeSet;

use crate::util;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
}

impl FileStatus {
    pub fn letter(self) -> char {
        match self {
            FileStatus::Added => 'A',
            FileStatus::Modified => 'M',
            FileStatus::Deleted => 'D',
            FileStatus::Renamed => 'R',
            FileStatus::Copied => 'C',
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Add,
    Del,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    /// The text git prints after the second `@@` (often the enclosing function).
    pub section: String,
    pub lines: Vec<DiffLine>,
}

impl Hunk {
    /// Identity of a hunk for "reviewed" marks: a hash of its +/- lines, so the
    /// mark survives unrelated edits elsewhere and resets when the hunk changes.
    pub fn hash(&self) -> String {
        let mut bytes = Vec::new();
        for line in &self.lines {
            match line.kind {
                LineKind::Add => bytes.push(b'+'),
                LineKind::Del => bytes.push(b'-'),
                LineKind::Context => continue,
            }
            bytes.extend_from_slice(line.text.as_bytes());
            bytes.push(b'\n');
        }
        util::fnv_hex(&bytes)
    }

    pub fn adds(&self) -> usize {
        self.lines.iter().filter(|l| l.kind == LineKind::Add).count()
    }

    pub fn dels(&self) -> usize {
        self.lines.iter().filter(|l| l.kind == LineKind::Del).count()
    }

    /// Last new-side line number covered by the hunk.
    pub fn new_end(&self) -> u32 {
        if self.new_lines == 0 {
            self.new_start
        } else {
            self.new_start + self.new_lines - 1
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub binary: bool,
    pub hunks: Vec<Hunk>,
}

impl FileDiff {
    pub fn adds(&self) -> usize {
        self.hunks.iter().map(Hunk::adds).sum()
    }

    pub fn dels(&self) -> usize {
        self.hunks.iter().map(Hunk::dels).sum()
    }

    /// New-side line numbers of added lines.
    pub fn added_lines(&self) -> BTreeSet<u32> {
        self.hunks
            .iter()
            .flat_map(|h| h.lines.iter())
            .filter(|l| l.kind == LineKind::Add)
            .filter_map(|l| l.new_no)
            .collect()
    }

    /// New-side lines that were added, plus the line after each deletion, so a
    /// pure deletion still marks the code around it.
    pub fn touched_lines(&self) -> BTreeSet<u32> {
        let mut set = BTreeSet::new();
        for hunk in &self.hunks {
            let mut pending_del = false;
            let mut last_new = hunk.new_start.saturating_sub(1);
            for line in &hunk.lines {
                match line.kind {
                    LineKind::Add => {
                        if let Some(n) = line.new_no {
                            set.insert(n);
                            last_new = n;
                        }
                        pending_del = false;
                    }
                    LineKind::Del => pending_del = true,
                    LineKind::Context => {
                        if let Some(n) = line.new_no {
                            if pending_del {
                                set.insert(n);
                                if n > 1 {
                                    set.insert(n - 1);
                                }
                            }
                            last_new = n;
                        }
                        pending_del = false;
                    }
                }
            }
            if pending_del {
                set.insert(last_new.max(1));
                set.insert(last_new + 1);
            }
        }
        set
    }

    /// Mark for a line range [start, end] of the new file: A when every line
    /// in it was added, M when any line was touched, None otherwise.
    pub fn mark_for_range(&self, start: u32, end: u32) -> Option<char> {
        if self.status == FileStatus::Added {
            return Some('A');
        }
        let added = self.added_lines();
        let touched = self.touched_lines();
        let all_added = (start..=end).all(|n| added.contains(&n));
        if all_added && end >= start {
            return Some('A');
        }
        if touched.range(start..=end).next().is_some() {
            return Some('M');
        }
        None
    }
}

/// A whole file shown as added (untracked files, empty repositories).
pub fn added_file(root: &std::path::Path, rel: &str) -> FileDiff {
    let bytes = std::fs::read(root.join(rel)).unwrap_or_default();
    let probe = &bytes[..bytes.len().min(8000)];
    let binary = probe.contains(&0);
    let mut hunks = Vec::new();
    if !binary && !bytes.is_empty() {
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<DiffLine> = text
            .lines()
            .take(20_000)
            .enumerate()
            .map(|(i, l)| DiffLine {
                kind: LineKind::Add,
                old_no: None,
                new_no: Some(i as u32 + 1),
                text: l.to_string(),
            })
            .collect();
        let n = lines.len() as u32;
        hunks.push(Hunk {
            old_start: 0,
            old_lines: 0,
            new_start: 1,
            new_lines: n,
            section: String::new(),
            lines,
        });
    }
    FileDiff {
        path: rel.to_string(),
        old_path: None,
        status: FileStatus::Added,
        binary,
        hunks,
    }
}

/// Parse unified diff text produced with `--src-prefix=a/ --dst-prefix=b/`.
pub fn parse(text: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut cur: Option<FileDiff> = None;
    let mut hunk: Option<Hunk> = None;
    let mut old_no = 0u32;
    let mut new_no = 0u32;

    let flush_hunk = |cur: &mut Option<FileDiff>, hunk: &mut Option<Hunk>| {
        if let (Some(f), Some(h)) = (cur.as_mut(), hunk.take()) {
            f.hunks.push(h);
        }
    };

    for line in text.split('\n') {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            flush_hunk(&mut cur, &mut hunk);
            if let Some(f) = cur.take() {
                files.push(f);
            }
            let (a, b) = split_git_header(rest);
            cur = Some(FileDiff {
                path: b.clone(),
                old_path: if a != b { Some(a) } else { None },
                status: FileStatus::Modified,
                binary: false,
                hunks: Vec::new(),
            });
            continue;
        }
        let Some(file) = cur.as_mut() else { continue };
        if let Some(rest) = line.strip_prefix("@@ ") {
            flush_hunk(&mut cur, &mut hunk);
            if let Some(h) = parse_hunk_header(rest) {
                old_no = h.old_start;
                new_no = h.new_start;
                hunk = Some(h);
            }
            continue;
        }
        if hunk.is_none() {
            parse_header_line(file, line);
            continue;
        }
        let Some(h) = hunk.as_mut() else { continue };
        if let Some(t) = line.strip_prefix('+') {
            h.lines.push(DiffLine {
                kind: LineKind::Add,
                old_no: None,
                new_no: Some(new_no),
                text: t.to_string(),
            });
            new_no += 1;
        } else if let Some(t) = line.strip_prefix('-') {
            h.lines.push(DiffLine {
                kind: LineKind::Del,
                old_no: Some(old_no),
                new_no: None,
                text: t.to_string(),
            });
            old_no += 1;
        } else if let Some(t) = line.strip_prefix(' ') {
            h.lines.push(DiffLine {
                kind: LineKind::Context,
                old_no: Some(old_no),
                new_no: Some(new_no),
                text: t.to_string(),
            });
            old_no += 1;
            new_no += 1;
        }
        // "\ No newline at end of file" is ignored.
    }
    flush_hunk(&mut cur, &mut hunk);
    if let Some(f) = cur.take() {
        files.push(f);
    }
    for f in &mut files {
        if f.status == FileStatus::Deleted {
            if let Some(old) = &f.old_path {
                if f.path.is_empty() {
                    f.path = old.clone();
                }
            }
            f.old_path = None;
        }
        if f.status == FileStatus::Added {
            f.old_path = None;
        }
    }
    files
}

fn parse_header_line(file: &mut FileDiff, line: &str) {
    if line.starts_with("new file mode") {
        file.status = FileStatus::Added;
    } else if line.starts_with("deleted file mode") {
        file.status = FileStatus::Deleted;
    } else if let Some(p) = line.strip_prefix("rename from ") {
        file.status = FileStatus::Renamed;
        file.old_path = Some(unquote(p));
    } else if let Some(p) = line.strip_prefix("rename to ") {
        file.status = FileStatus::Renamed;
        file.path = unquote(p);
    } else if let Some(p) = line.strip_prefix("copy from ") {
        file.status = FileStatus::Copied;
        file.old_path = Some(unquote(p));
    } else if let Some(p) = line.strip_prefix("copy to ") {
        file.status = FileStatus::Copied;
        file.path = unquote(p);
    } else if line.starts_with("Binary files ") || line == "GIT binary patch" {
        file.binary = true;
    } else if let Some(p) = line.strip_prefix("--- ") {
        if p == "/dev/null" {
            file.status = FileStatus::Added;
        } else if matches!(file.status, FileStatus::Renamed | FileStatus::Copied) {
            let p = unquote(p);
            file.old_path = Some(p.strip_prefix("a/").unwrap_or(&p).to_string());
        }
    } else if let Some(p) = line.strip_prefix("+++ ") {
        if p == "/dev/null" {
            file.status = FileStatus::Deleted;
        } else {
            let p = unquote(p);
            file.path = p.strip_prefix("b/").unwrap_or(&p).to_string();
        }
    }
}

fn parse_hunk_header(rest: &str) -> Option<Hunk> {
    // "-a,b +c,d @@ section"
    let end = rest.find(" @@")?;
    let ranges = &rest[..end];
    let section = rest[end + 3..].trim().to_string();
    let mut parts = ranges.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let (old_start, old_lines) = parse_range(old)?;
    let (new_start, new_lines) = parse_range(new)?;
    Some(Hunk {
        old_start,
        old_lines,
        new_start,
        new_lines,
        section,
        lines: Vec::new(),
    })
}

fn parse_range(s: &str) -> Option<(u32, u32)> {
    match s.split_once(',') {
        Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
        None => Some((s.parse().ok()?, 1)),
    }
}

/// "a/x b/y" (possibly quoted) -> ("x", "y").
fn split_git_header(rest: &str) -> (String, String) {
    let rest = rest.trim();
    if rest.starts_with('"') {
        // quoted form: "a/x" "b/y" or mixed
        let mut parts = Vec::new();
        let mut buf = String::new();
        let mut in_q = false;
        let mut chars = rest.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '"' => {
                    in_q = !in_q;
                    buf.push(c);
                }
                ' ' if !in_q => {
                    if !buf.is_empty() {
                        parts.push(std::mem::take(&mut buf));
                    }
                }
                '\\' if in_q => {
                    buf.push(c);
                    if let Some(n) = chars.next() {
                        buf.push(n);
                    }
                }
                _ => buf.push(c),
            }
        }
        if !buf.is_empty() {
            parts.push(buf);
        }
        if parts.len() >= 2 {
            let a = unquote(&parts[0]);
            let b = unquote(&parts[1]);
            return (
                a.strip_prefix("a/").unwrap_or(&a).to_string(),
                b.strip_prefix("b/").unwrap_or(&b).to_string(),
            );
        }
    }
    // Unquoted: both halves are equal length when the path did not change.
    let body = rest.strip_prefix("a/").unwrap_or(rest);
    if let Some(idx) = find_middle_split(body) {
        let a = &body[..idx];
        let b = &body[idx + 3..];
        return (a.to_string(), b.to_string());
    }
    match body.split_once(" b/") {
        Some((a, b)) => (a.to_string(), b.to_string()),
        None => (body.to_string(), body.to_string()),
    }
}

fn find_middle_split(body: &str) -> Option<usize> {
    // body = "<p> b/<p>"; if the two paths are identical the split is exactly
    // in the middle.
    let len = body.len();
    if len < 4 || (len - 3) % 2 != 0 {
        return None;
    }
    let half = (len - 3) / 2;
    if body.is_char_boundary(half) && &body[half..half + 3] == " b/" && body[..half] == body[half + 3..] {
        Some(half)
    } else {
        None
    }
}

fn unquote(s: &str) -> String {
    let s = s.trim_end_matches('\t');
    if !(s.starts_with('"') && s.ends_with('"') && s.len() >= 2) {
        return s.to_string();
    }
    let inner = &s[1..s.len() - 1];
    let mut out: Vec<u8> = Vec::new();
    let bytes = inner.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            let n = bytes[i + 1];
            match n {
                b'n' => {
                    out.push(b'\n');
                    i += 2;
                }
                b't' => {
                    out.push(b'\t');
                    i += 2;
                }
                b'"' | b'\\' => {
                    out.push(n);
                    i += 2;
                }
                b'0'..=b'7' if i + 3 < bytes.len() + 1 => {
                    let oct = &inner[i + 1..(i + 4).min(inner.len())];
                    if let Ok(v) = u8::from_str_radix(oct, 8) {
                        out.push(v);
                        i += 4;
                    } else {
                        out.push(n);
                        i += 2;
                    }
                }
                _ => {
                    out.push(n);
                    i += 2;
                }
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "diff --git a/src/api.py b/src/api.py
index 1111111..2222222 100644
--- a/src/api.py
+++ b/src/api.py
@@ -1,4 +1,5 @@ import json
 import json
-import os
+import sys
+import time

 def main():
@@ -10,3 +11,3 @@ def main():
     x = 1
-    return x
+    return x + 1

diff --git a/new.py b/new.py
new file mode 100644
index 0000000..3333333
--- /dev/null
+++ b/new.py
@@ -0,0 +1,2 @@
+def added():
+    pass
diff --git a/gone.py b/gone.py
deleted file mode 100644
index 4444444..0000000
--- a/gone.py
+++ /dev/null
@@ -1 +0,0 @@
-x = 1
diff --git a/old name.py b/new name.py
similarity index 90%
rename from old name.py
rename to new name.py
diff --git a/logo.png b/logo.png
index 5555555..6666666 100644
Binary files a/logo.png and b/logo.png differ
";

    #[test]
    fn parses_statuses_and_hunks() {
        let files = parse(SAMPLE);
        assert_eq!(files.len(), 5);
        let api = &files[0];
        assert_eq!(api.path, "src/api.py");
        assert_eq!(api.status, FileStatus::Modified);
        assert_eq!(api.hunks.len(), 2);
        assert_eq!(api.adds(), 3);
        assert_eq!(api.dels(), 2);
        assert_eq!(api.hunks[0].section, "import json");
        assert_eq!(api.hunks[1].new_start, 11);
        let added: Vec<u32> = api.added_lines().into_iter().collect();
        assert_eq!(added, vec![2, 3, 12]);

        assert_eq!(files[1].status, FileStatus::Added);
        assert_eq!(files[1].path, "new.py");
        assert_eq!(files[2].status, FileStatus::Deleted);
        assert_eq!(files[2].path, "gone.py");
        assert_eq!(files[3].status, FileStatus::Renamed);
        assert_eq!(files[3].path, "new name.py");
        assert_eq!(files[3].old_path.as_deref(), Some("old name.py"));
        assert!(files[4].binary);
        assert_eq!(files[4].path, "logo.png");
    }

    #[test]
    fn marks_ranges() {
        let files = parse(SAMPLE);
        let api = &files[0];
        // main() spans 5..13 in the new file; line 12 changed
        assert_eq!(api.mark_for_range(5, 13), Some('M'));
        assert_eq!(api.mark_for_range(2, 3), Some('A'));
        assert_eq!(api.mark_for_range(20, 30), None);
        assert_eq!(files[1].mark_for_range(1, 2), Some('A'));
    }

    #[test]
    fn hunk_hash_ignores_context() {
        let files = parse(SAMPLE);
        let h = &files[0].hunks[1];
        let mut h2 = h.clone();
        h2.lines[0].text = "    y = 2".into(); // context line
        assert_eq!(h.hash(), h2.hash());
        h2.lines[1].text = "    return y".into();
        assert_ne!(h.hash(), h2.hash());
    }

    #[test]
    fn quoted_paths() {
        let text = "diff --git \"a/sp\\303\\244ce.py\" \"b/sp\\303\\244ce.py\"\nnew file mode 100644\n--- /dev/null\n+++ \"b/sp\\303\\244ce.py\"\n@@ -0,0 +1 @@\n+x\n";
        let files = parse(text);
        assert_eq!(files[0].path, "späce.py");
    }

    #[test]
    fn deletion_touches_neighbours() {
        let text = "diff --git a/f.py b/f.py\n--- a/f.py\n+++ b/f.py\n@@ -1,4 +1,3 @@\n a\n-b\n c\n d\n";
        let files = parse(text);
        let t: Vec<u32> = files[0].touched_lines().into_iter().collect();
        assert!(t.contains(&2));
    }
}
