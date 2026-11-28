use std::io::Write;
use std::path::{Component, Path};
use std::process::{Command, Output, Stdio};

use super::{
    diff, git_cmd, run, run_cmd, stdout_of, FileDiff, FileStatus, GitError, Hunk, DIFF_FORMAT,
    STAGED_DIFF, UNSTAGED_DIFF,
};

fn on_paths(root: &Path, args: &[&str], paths: &[&str]) -> Command {
    let mut cmd = git_cmd(root);
    cmd.arg("--literal-pathspecs")
        .args(args)
        .arg("--")
        .args(paths);
    cmd
}

fn both_names(file: &FileDiff) -> Vec<&str> {
    std::iter::once(file.path.as_str())
        .chain(file.old_path.as_deref())
        .collect()
}

fn has_commits(root: &Path) -> bool {
    run(root, &["rev-parse", "-q", "--verify", "HEAD^{commit}"]).is_ok()
}

pub fn stage_file(root: &Path, file: &FileDiff) -> Result<(), GitError> {
    run_cmd(on_paths(root, &["add", "-A"], &both_names(file))).map(drop)
}

pub fn unstage_file(root: &Path, file: &FileDiff) -> Result<(), GitError> {
    let cmd = if has_commits(root) {
        on_paths(root, &["restore", "--staged"], &both_names(file))
    } else {
        on_paths(root, &["rm", "--cached", "-q"], &[&file.path])
    };
    run_cmd(cmd).map(drop)
}

pub fn takes_hunks(file: &FileDiff) -> bool {
    file.status == FileStatus::Modified && !file.binary && !file.hunks.is_empty()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Onto {
    OldSide,
    NewSide,
}

fn starts_with_any(line: &[u8], prefixes: &[&[u8]]) -> bool {
    prefixes.iter().any(|p| line.starts_with(p))
}

fn after_ranges(hunk_header: &[u8]) -> &[u8] {
    let close = b" @@";
    hunk_header
        .windows(close.len())
        .position(|w| w == close)
        .map(|at| &hunk_header[at + close.len()..])
        .unwrap_or(b"\n")
}

fn lone_hunk_patch(raw: &[u8], shown: &Hunk, onto: Onto) -> Option<Vec<u8>> {
    let files = diff::parse(&String::from_utf8_lossy(raw));
    let [file] = files.as_slice() else {
        return None;
    };
    let at = file.hunks.iter().position(|h| h == shown)?;
    let mut patch = Vec::new();
    let mut hunks: Vec<Vec<&[u8]>> = Vec::new();
    for line in raw.split_inclusive(|b| *b == b'\n') {
        if line.starts_with(b"@@ ") {
            hunks.push(vec![line]);
        } else if let Some(hunk) = hunks.last_mut() {
            hunk.push(line);
        } else if starts_with_any(line, &[b"diff --git ", b"--- ", b"+++ "]) {
            patch.extend_from_slice(line);
        }
    }
    if hunks.len() != file.hunks.len() {
        return None;
    }
    let grown_above: i64 = file.hunks[..at]
        .iter()
        .map(|h| i64::from(h.new_lines) - i64::from(h.old_lines))
        .sum();
    let (old_start, new_start) = match onto {
        Onto::OldSide => (
            i64::from(shown.old_start),
            i64::from(shown.new_start) - grown_above,
        ),
        Onto::NewSide => (
            i64::from(shown.old_start) + grown_above,
            i64::from(shown.new_start),
        ),
    };
    if old_start < 0 || new_start < 0 {
        return None;
    }
    let (header, body) = hunks[at].split_first()?;
    patch.extend_from_slice(
        format!(
            "@@ -{old_start},{} +{new_start},{} @@",
            shown.old_lines, shown.new_lines
        )
        .as_bytes(),
    );
    patch.extend_from_slice(after_ranges(header));
    for line in body {
        patch.extend_from_slice(line);
    }
    if !patch.ends_with(b"\n") {
        patch.push(b'\n');
    }
    Some(patch)
}

fn feed(mut cmd: Command, input: &[u8]) -> Result<(), GitError> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| GitError(format!("git: {e}")))?;
    let sent = match child.stdin.take() {
        Some(mut stdin) => stdin.write_all(input),
        None => Ok(()),
    };
    let output = child
        .wait_with_output()
        .map_err(|e| GitError(format!("git: {e}")))?;
    stdout_of(output)?;
    sent.map_err(|e| GitError(format!("git: {e}")))
}

struct HunkWrite<'a> {
    read: &'a [&'a str],
    onto: Onto,
    apply: &'a [&'a str],
}

fn write_hunk(root: &Path, file: &FileDiff, hunk: usize, how: HunkWrite) -> Result<(), GitError> {
    let shown = file
        .hunks
        .get(hunk)
        .filter(|_| takes_hunks(file))
        .ok_or_else(|| GitError(format!("{} has no hunk to take on its own", file.path)))?;
    let read = [&["diff"], &DIFF_FORMAT[..], how.read].concat();
    let raw = run_cmd(on_paths(root, &read, &[&file.path]))?;
    let patch = lone_hunk_patch(&raw, shown, how.onto)
        .ok_or_else(|| GitError(format!("{} changed since this diff was read", file.path)))?;
    let mut apply = git_cmd(root);
    apply
        .arg("apply")
        .args(how.apply)
        .args(["--whitespace=nowarn", "-"]);
    feed(apply, &patch)
}

pub fn stage_hunk(root: &Path, file: &FileDiff, hunk: usize) -> Result<(), GitError> {
    let how = HunkWrite {
        read: UNSTAGED_DIFF,
        onto: Onto::OldSide,
        apply: &["--cached"],
    };
    write_hunk(root, file, hunk, how)
}

pub fn unstage_hunk(root: &Path, file: &FileDiff, hunk: usize) -> Result<(), GitError> {
    let how = HunkWrite {
        read: STAGED_DIFF,
        onto: Onto::NewSide,
        apply: &["--cached", "--reverse"],
    };
    write_hunk(root, file, hunk, how)
}

pub fn discard_hunk(root: &Path, file: &FileDiff, hunk: usize) -> Result<(), GitError> {
    let how = HunkWrite {
        read: UNSTAGED_DIFF,
        onto: Onto::NewSide,
        apply: &["--reverse"],
    };
    write_hunk(root, file, hunk, how)
}

pub fn restores_from_index(file: &FileDiff) -> bool {
    matches!(file.status, FileStatus::Modified | FileStatus::Deleted)
}

pub fn discard_file(root: &Path, file: &FileDiff) -> Result<(), GitError> {
    if file.status == FileStatus::Untracked {
        return delete_untracked(root, &file.path);
    }
    if !restores_from_index(file) {
        return Err(GitError(format!(
            "{} is new to the index: there is nothing to restore it from",
            file.path
        )));
    }
    run_cmd(on_paths(root, &["restore", "--worktree"], &[&file.path])).map(drop)
}

pub fn delete_untracked(root: &Path, rel: &str) -> Result<(), GitError> {
    let inside = Path::new(rel);
    let stays_inside = inside
        .components()
        .all(|part| matches!(part, Component::Normal(_)));
    if rel.is_empty() || !stays_inside {
        return Err(GitError(format!("{rel} is not a file in this repository")));
    }
    if !run_cmd(on_paths(root, &["ls-files", "-z"], &[rel]))?.is_empty() {
        return Err(GitError(format!("{rel} is tracked now: nothing deleted")));
    }
    let path = root.join(inside);
    let told = |e: std::io::Error| GitError(format!("{rel}: {e}"));
    if std::fs::symlink_metadata(&path).map_err(told)?.is_dir() {
        return Err(GitError(format!(
            "{rel} is a folder: codeMap deletes one file at a time"
        )));
    }
    std::fs::remove_file(&path).map_err(told)
}

pub(super) fn as_typed(root: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

pub(super) fn said(subcommand: &str, output: &Output) -> Result<(), GitError> {
    if output.status.success() {
        return Ok(());
    }
    let words = [&output.stderr, &output.stdout]
        .into_iter()
        .map(|bytes| String::from_utf8_lossy(bytes).trim().to_string())
        .find(|text| !text.is_empty());
    Err(GitError(words.unwrap_or_else(|| {
        format!("git {subcommand} exited with {}", output.status)
    })))
}

pub fn commit(root: &Path, subject: &str, body: &str) -> Result<String, GitError> {
    let mut cmd = as_typed(root);
    cmd.args(["commit", "-m", subject]);
    if !body.trim().is_empty() {
        cmd.args(["-m", body.trim_end()]);
    }
    let output = cmd.output().map_err(|e| GitError(format!("git: {e}")))?;
    said("commit", &output)?;
    run(root, &["rev-parse", "--short", "HEAD"]).map(|sha| sha.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_HUNKS: &str = "diff --git a/src/api.py b/src/api.py
index 1111111..2222222 100755
old mode 100644
new mode 100755
--- a/src/api.py
+++ b/src/api.py
@@ -1,4 +1,6 @@ import json
 import json
-import os
+import sys
+import time
+import zlib
 from pathlib import Path
 def main():
@@ -10,3 +12,2 @@ def main():
     x = 1
-    return x
     y = 2
\\ No newline at end of file
";

    fn shown(at: usize) -> Hunk {
        diff::parse(TWO_HUNKS).remove(0).hunks.remove(at)
    }

    fn patch(at: usize, onto: Onto) -> String {
        String::from_utf8(lone_hunk_patch(TWO_HUNKS.as_bytes(), &shown(at), onto).unwrap()).unwrap()
    }

    const NAMES: &str =
        "diff --git a/src/api.py b/src/api.py\n--- a/src/api.py\n+++ b/src/api.py\n";
    const LAST: &str = "     x = 1\n-    return x\n     y = 2\n\\ No newline at end of file\n";

    #[test]
    fn the_patch_names_the_file_and_leaves_its_mode_out() {
        let first = patch(0, Onto::OldSide);
        assert!(first.starts_with(&format!("{NAMES}@@ -1,4 +1,6 @@ import json\n")));
        assert!(first.ends_with("+import zlib\n from pathlib import Path\n def main():\n"));
        assert!(
            !first.contains("mode") && !first.contains("index"),
            "{first}"
        );
        assert_eq!(
            first,
            patch(0, Onto::NewSide),
            "nothing above the first hunk"
        );
    }

    #[test]
    fn a_hunk_on_its_own_is_numbered_as_if_the_ones_above_were_not_applied() {
        assert_eq!(
            patch(1, Onto::OldSide),
            format!("{NAMES}@@ -10,3 +10,2 @@ def main():\n{LAST}")
        );
        assert_eq!(
            patch(1, Onto::NewSide),
            format!("{NAMES}@@ -12,3 +12,2 @@ def main():\n{LAST}")
        );
    }

    #[test]
    fn a_hunk_that_is_not_in_the_diff_has_no_patch() {
        let mut other = shown(1);
        other.lines[1].text.push('!');
        assert_eq!(
            lone_hunk_patch(TWO_HUNKS.as_bytes(), &other, Onto::OldSide),
            None
        );
        let mut ends_with_newline = shown(1);
        ends_with_newline.lines[2].no_newline = false;
        assert_eq!(
            lone_hunk_patch(TWO_HUNKS.as_bytes(), &ends_with_newline, Onto::OldSide),
            None
        );
        let twice = format!("{TWO_HUNKS}{TWO_HUNKS}");
        assert_eq!(
            lone_hunk_patch(twice.as_bytes(), &shown(0), Onto::OldSide),
            None
        );
        assert_eq!(lone_hunk_patch(b"", &shown(0), Onto::OldSide), None);
    }

    #[test]
    fn bytes_the_text_cannot_hold_come_from_git_unchanged() {
        let raw = b"diff --git a/l.txt b/l.txt\n--- a/l.txt\n+++ b/l.txt\n@@ -1 +1 @@\n-caf\xe9\r\n+na\xefve\r\n";
        let hunk = diff::parse(&String::from_utf8_lossy(raw))
            .remove(0)
            .hunks
            .remove(0);
        assert_eq!(hunk.lines[0].text, "caf\u{fffd}\r");
        let patch = lone_hunk_patch(raw, &hunk, Onto::OldSide).unwrap();
        assert!(patch.ends_with(b"@@ -1,1 +1,1 @@\n-caf\xe9\r\n+na\xefve\r\n"));
    }
}
