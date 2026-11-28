mod common;

use std::collections::BTreeSet;
use std::path::Path;

use codemap::git::{self, FileDiff, FileStatus, LineKind, Status};
use codemap::store::Store;
use common::*;

fn status_of(repo: &Path) -> Status {
    let state = TempDir::new("status-state");
    let info = git::discover(repo).unwrap();
    let scratch = Store::new(state.path()).shadow(&info).dir;
    git::status(&info, &scratch).unwrap()
}

fn shown(files: &[FileDiff]) -> Vec<String> {
    files
        .iter()
        .map(|f| {
            format!(
                "{} {} +{} -{}",
                f.status.letter(),
                f.path,
                f.adds(),
                f.dels()
            )
        })
        .collect()
}

fn added_text(file: &FileDiff) -> Vec<&str> {
    file.hunks
        .iter()
        .flat_map(|h| &h.lines)
        .filter(|l| l.kind == LineKind::Add)
        .map(|l| l.text.as_str())
        .collect()
}

fn none() -> Vec<String> {
    Vec::new()
}

#[test]
fn a_staged_modification_is_staged_only() {
    let repo = repo_with(&[("a.py", "x = 1\n")]);
    write(repo.path(), "a.py", "x = 2\n");
    git(repo.path(), &["add", "a.py"]);
    let status = status_of(repo.path());
    assert_eq!(shown(&status.staged), ["M a.py +1 -1"]);
    assert_eq!(shown(&status.unstaged), none());
}

#[test]
fn an_unstaged_modification_and_deletion_are_not_staged() {
    let repo = repo_with(&[("a.py", "x = 1\n"), ("gone.py", "y = 1\n")]);
    write(repo.path(), "a.py", "x = 2\n");
    std::fs::remove_file(repo.path().join("gone.py")).unwrap();
    let status = status_of(repo.path());
    assert_eq!(shown(&status.staged), none());
    assert_eq!(shown(&status.unstaged), ["M a.py +1 -1", "D gone.py +0 -1"]);
}

#[test]
fn a_partly_staged_file_is_in_both_lists_with_its_own_diff() {
    let repo = repo_with(&[("a.py", "one = 1\n")]);
    write(repo.path(), "a.py", "one = 1\nstaged = 2\n");
    git(repo.path(), &["add", "a.py"]);
    write(repo.path(), "a.py", "one = 1\nstaged = 2\nlater = 3\n");
    let status = status_of(repo.path());
    assert_eq!(shown(&status.staged), ["M a.py +1 -0"]);
    assert_eq!(shown(&status.unstaged), ["M a.py +1 -0"]);
    assert_eq!(added_text(&status.staged[0]), ["staged = 2"]);
    assert_eq!(added_text(&status.unstaged[0]), ["later = 3"]);
}

#[test]
fn an_untracked_file_is_told_apart_from_a_staged_new_file() {
    let repo = repo_with(&[("a.py", "x = 1\n")]);
    write(repo.path(), "staged.py", "def new():\n    pass\n");
    git(repo.path(), &["add", "staged.py"]);
    write(repo.path(), "pkg/loose.py", "one\ntwo\nthree\n");
    let status = status_of(repo.path());
    assert_eq!(shown(&status.staged), ["A staged.py +2 -0"]);
    assert_eq!(shown(&status.unstaged), ["? pkg/loose.py +3 -0"]);
    assert_eq!(status.staged[0].status, FileStatus::Added);
    assert_eq!(status.unstaged[0].status, FileStatus::Untracked);
    assert_eq!(added_text(&status.unstaged[0]), ["one", "two", "three"]);
}

#[test]
fn an_ignored_file_is_not_listed() {
    let repo = repo_with(&[(".gitignore", "*.log\n")]);
    write(repo.path(), "debug.log", "noise\n");
    let status = status_of(repo.path());
    assert_eq!(status, Status::default());
}

#[test]
fn a_file_added_with_intent_is_new_but_not_untracked() {
    let repo = repo_with(&[("a.py", "x = 1\n")]);
    write(repo.path(), "later.py", "y = 2\n");
    git(repo.path(), &["add", "--intent-to-add", "later.py"]);
    let status = status_of(repo.path());
    assert_eq!(shown(&status.staged), none());
    assert_eq!(shown(&status.unstaged), ["A later.py +1 -0"]);
}

#[test]
fn a_staged_deletion_and_a_staged_rename() {
    let repo = repo_with(&[
        ("gone.py", "x = 1\n"),
        ("old_name.py", "def keep():\n    return 1\n"),
    ]);
    git(repo.path(), &["rm", "-q", "gone.py"]);
    git(repo.path(), &["mv", "old_name.py", "new_name.py"]);
    let status = status_of(repo.path());
    assert_eq!(
        shown(&status.staged),
        ["D gone.py +0 -1", "R new_name.py +0 -0"]
    );
    assert_eq!(status.staged[0].status, FileStatus::Deleted);
    assert_eq!(status.staged[1].status, FileStatus::Renamed);
    assert_eq!(status.staged[1].old_path.as_deref(), Some("old_name.py"));
    assert_eq!(shown(&status.unstaged), none());
}

#[test]
fn a_clean_tree_has_nothing_in_either_list() {
    let repo = repo_with(&[("a.py", "x = 1\n")]);
    assert_eq!(status_of(repo.path()), Status::default());
}

#[test]
fn a_repository_with_no_commits_counts_added_files_as_staged() {
    let t = TempDir::new("status-unborn");
    git(t.path(), &["init", "-q"]);
    write(t.path(), "first.py", "def hello():\n    print('hi')\n");
    write(t.path(), "second.py", "x = 1\n");
    git(t.path(), &["add", "first.py"]);
    assert!(git::discover(t.path()).unwrap().is_empty());
    let status = status_of(t.path());
    assert_eq!(shown(&status.staged), ["A first.py +2 -0"]);
    assert_eq!(shown(&status.unstaged), ["? second.py +1 -0"]);
}

#[test]
fn a_repository_with_no_commits_and_no_index_lists_untracked_files() {
    let t = TempDir::new("status-bare-start");
    git(t.path(), &["init", "-q"]);
    write(t.path(), "first.py", "x = 1\n");
    assert!(!t.path().join(".git/index").exists());
    let status = status_of(t.path());
    assert_eq!(shown(&status.staged), none());
    assert_eq!(shown(&status.unstaged), ["? first.py +1 -0"]);
}

#[test]
fn a_conflicted_file_is_not_staged_and_shows_its_markers() {
    let repo = repo_with(&[("a.py", "a = 1\n"), ("f.py", "one\ntwo\nthree\n")]);
    let p = repo.path();
    git(p, &["checkout", "-q", "-b", "side"]);
    write(p, "f.py", "one\nTWO side\nthree\n");
    git(p, &["commit", "-q", "-am", "side"]);
    git(p, &["checkout", "-q", "main"]);
    write(p, "f.py", "one\nTWO main\nthree\n");
    git(p, &["commit", "-q", "-am", "main"]);
    write(p, "a.py", "a = 2\n");
    let merge = git_output(
        p,
        &["-c", "merge.conflictStyle=merge", "merge", "-q", "side"],
    );
    let unmerged = git(p, &["ls-files", "--unmerged"]);
    let conflicted: BTreeSet<&str> = unmerged
        .lines()
        .filter_map(|entry| entry.split('\t').nth(1))
        .collect();
    assert_eq!(
        (merge.status.code(), conflicted),
        (Some(1), BTreeSet::from(["f.py"])),
        "the merge stops on a conflict in f.py: {}",
        String::from_utf8_lossy(&merge.stderr)
    );
    let status = status_of(p);
    assert_eq!(shown(&status.staged), none());
    assert_eq!(shown(&status.unstaged), ["M a.py +1 -1", "M f.py +4 -0"]);
    assert_eq!(added_text(&status.unstaged[0]), ["a = 2"]);
    assert!(added_text(&status.unstaged[1]).contains(&"TWO side"));
}

#[test]
fn reading_the_status_writes_nothing_into_the_repository() {
    let repo = repo_with(&[
        ("touched.py", "x = 1\n"),
        ("staged.py", "y = 1\n"),
        ("edited.py", "z = 1\n"),
    ]);
    let p = repo.path();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    write(p, "touched.py", "x = 1\n");
    write(p, "staged.py", "y = 2\n");
    git(p, &["add", "staged.py"]);
    write(p, "edited.py", "z = 2\n");
    write(p, "untracked.py", "w = 3\n");
    let before = fingerprint(p);

    let state = TempDir::new("status-state");
    let info = git::discover(p).unwrap();
    let scratch = Store::new(state.path()).shadow(&info).dir;
    let status = git::status(&info, &scratch).unwrap();
    assert_eq!(shown(&status.staged), ["M staged.py +1 -1"]);
    assert_eq!(
        shown(&status.unstaged),
        ["M edited.py +1 -1", "? untracked.py +1 -0"]
    );

    let after = fingerprint(p);
    assert_eq!(
        before.0, after.0,
        ".git changed: codeMap wrote into the repo"
    );
    assert_eq!(before.1, after.1, "git status changed");
    let left_behind: Vec<_> = std::fs::read_dir(&scratch).unwrap().flatten().collect();
    assert!(left_behind.is_empty(), "{left_behind:?}");
}

#[test]
fn many_reads_of_one_repository_at_once_all_see_the_same_index() {
    let repo = repo_with(&[
        ("staged.py", "y = 1\n"),
        ("edited.py", "z = 1\n"),
        ("kept.py", "k = 1\n"),
    ]);
    let p = repo.path();
    write(p, "staged.py", "y = 2\n");
    git(p, &["add", "staged.py"]);
    write(p, "edited.py", "z = 2\n");
    write(p, "untracked.py", "w = 3\n");
    let before = fingerprint(p);

    let state = TempDir::new("status-crowd-state");
    let info = git::discover(p).unwrap();
    let scratch = Store::new(state.path()).shadow(&info).dir;
    let status =
        || git::status(&info, &scratch).map(|read| (shown(&read.staged), shown(&read.unstaged)));
    let since_head = || git::diff_head(&info, &scratch).map(|files| shown(&files));
    let alone = (status(), since_head());
    assert_eq!(
        alone.0,
        Ok((
            vec!["M staged.py +1 -1".to_string()],
            vec![
                "M edited.py +1 -1".to_string(),
                "? untracked.py +1 -0".to_string()
            ]
        ))
    );
    assert_eq!(
        alone.1,
        Ok(vec![
            "M edited.py +1 -1".to_string(),
            "M staged.py +1 -1".to_string(),
            "A untracked.py +1 -0".to_string()
        ])
    );

    const READERS: usize = 12;
    const READS_EACH: usize = 6;
    let start = std::sync::Barrier::new(READERS);
    let wrong: Vec<String> = std::thread::scope(|readers| {
        let reading: Vec<_> = (0..READERS)
            .map(|reader| {
                let (status, since_head, alone, start) = (&status, &since_head, &alone, &start);
                readers.spawn(move || {
                    start.wait();
                    let mut wrong = Vec::new();
                    for read in 0..READS_EACH {
                        if reader % 3 == 0 {
                            let seen = since_head();
                            if seen != alone.1 {
                                wrong.push(format!("reader {reader} read {read}: {seen:?}"));
                            }
                        } else {
                            let seen = status();
                            if seen != alone.0 {
                                wrong.push(format!("reader {reader} read {read}: {seen:?}"));
                            }
                        }
                    }
                    wrong
                })
            })
            .collect();
        reading
            .into_iter()
            .flat_map(|reader| reader.join().unwrap())
            .collect()
    });
    assert!(
        wrong.is_empty(),
        "{} of {} reads saw another index:\n{}",
        wrong.len(),
        READERS * READS_EACH,
        wrong.join("\n")
    );

    let left_behind: Vec<_> = std::fs::read_dir(&scratch).unwrap().flatten().collect();
    assert!(left_behind.is_empty(), "{left_behind:?}");
    let after = fingerprint(p);
    assert_eq!(before.0, after.0, ".git changed while reading");
    assert_eq!(before.1, after.1, "git status changed while reading");
}

#[test]
fn a_copy_left_by_a_process_that_ended_is_removed_and_no_other_file_is() {
    let repo = repo_with(&[("a.py", "x = 1\n")]);
    let state = TempDir::new("status-leftover-state");
    let info = git::discover(repo.path()).unwrap();
    let scratch = Store::new(state.path()).shadow(&info).dir;
    let mut ended = std::process::Command::new("git")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let ended_id = ended.id();
    ended.wait().unwrap();
    let left_behind = format!("index.status.{ended_id}.3");
    let being_read = format!("index.head.{}.999999", std::process::id());
    for name in [
        left_behind.as_str(),
        being_read.as_str(),
        "index.status",
        "notes.txt",
    ] {
        write(&scratch, name, "not an index\n");
    }
    assert_eq!(git::status(&info, &scratch).unwrap(), Status::default());
    let mut kept: Vec<String> = std::fs::read_dir(&scratch)
        .unwrap()
        .flatten()
        .map(|file| file.file_name().to_string_lossy().into_owned())
        .collect();
    kept.sort();
    assert_eq!(kept, [being_read.as_str(), "index.status", "notes.txt"]);
}
