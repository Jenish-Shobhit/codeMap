mod common;

use std::path::Path;

use codemap::git::blame::{self, BlameRead, Blamed, BLAME_BYTES};
use codemap::git::graph::is_object_id;
use codemap::git::{self, branch, FileDiff};
use common::*;

fn read(root: &Path, path: &str, was: Option<&str>) -> BlameRead {
    let info = git::discover(root).unwrap();
    let scratch = TempDir::new("blame-scratch");
    blame::blame(&info, scratch.path(), path, was).unwrap()
}

fn blamed(root: &Path, path: &str) -> Blamed {
    match read(root, path, None) {
        BlameRead::Lines(blamed) => blamed,
        other => panic!("{path}: {other:?}"),
    }
}

fn rows(blamed: &Blamed) -> Vec<(u32, String, String, String)> {
    blamed
        .lines
        .iter()
        .map(|line| match blamed.commit_of(line) {
            Some(c) => (
                line.number,
                c.author.clone(),
                c.summary.clone(),
                line.text.clone(),
            ),
            None => (
                line.number,
                "-".to_string(),
                "not committed".to_string(),
                line.text.clone(),
            ),
        })
        .collect()
}

fn row(number: u32, author: &str, summary: &str, text: &str) -> (u32, String, String, String) {
    (number, author.into(), summary.into(), text.into())
}

fn shown(diff: &[FileDiff]) -> Vec<String> {
    diff.iter()
        .map(|f| match &f.old_path {
            Some(old) => format!("{} {old} -> {}", f.status.letter(), f.path),
            None => format!(
                "{} {} +{} -{}",
                f.status.letter(),
                f.path,
                f.adds(),
                f.dels()
            ),
        })
        .collect()
}

fn commit_as(repo: &Path, author: &str, subject: &str) {
    git(repo, &["add", "-A"]);
    let by = format!("--author={author}");
    git(repo, &["commit", "-q", "-m", subject, &by]);
}

#[test]
fn each_line_names_the_commit_and_author_that_last_changed_it() {
    let repo = repo_with(&[("a.rs", "one\ntwo\nthree\nfour\n")]);
    let p = repo.path();
    write(p, "a.rs", "one\nTWO\nthree\nfour\n");
    commit_as(p, "Ada Lovelace <ada@example.com>", "feat: shout two");
    write(p, "a.rs", "one\nTWO\nthree\nFOUR\n");
    commit_as(p, "Grace Hopper <grace@example.com>", "feat: shout four");
    write(p, "a.rs", "one\nTWO\n3\nFOUR\nfive\n");

    let blamed = blamed(p, "a.rs");
    assert_eq!(
        rows(&blamed),
        [
            row(1, "Test", "initial", "one"),
            row(2, "Ada Lovelace", "feat: shout two", "TWO"),
            row(3, "-", "not committed", "3"),
            row(4, "Grace Hopper", "feat: shout four", "FOUR"),
            row(5, "-", "not committed", "five"),
        ]
    );
    assert_eq!(blamed.commits.len(), 3, "the zero id is not a commit");
    for c in &blamed.commits {
        assert!(is_object_id(&c.sha), "{}", c.sha);
        assert_eq!(c.short(), &c.sha[..7]);
        assert_eq!(c.time, 1_790_027_040);
    }
    let head = git(p, &["rev-parse", "HEAD"]).trim().to_string();
    let four = blamed.commit_of(&blamed.lines[3]).unwrap();
    assert_eq!(four.sha, head);
    assert_eq!(four.previous.as_deref(), Some("a.rs"));
    assert_eq!(four.filename, "a.rs");
    let first = blamed.commit_of(&blamed.lines[0]).unwrap();
    assert_eq!(first.previous, None, "the root commit has no parent");
}

#[test]
fn a_renamed_file_keeps_the_names_it_had_and_each_diff_is_that_file() {
    let repo = repo_with(&[("old.rs", "a\nb\nc\nd\ne\nf\n"), ("other.rs", "x\n")]);
    let p = repo.path();
    git(p, &["mv", "old.rs", "mid.rs"]);
    write(p, "mid.rs", "a\nb\nc\nd\ne\nF\n");
    write(p, "other.rs", "y\n");
    commit_as(p, "Test <test@example.com>", "refactor: rename and edit");
    git(p, &["mv", "mid.rs", "new.rs"]);
    commit_as(p, "Test <test@example.com>", "refactor: rename only");
    write(p, "new.rs", "A\nb\nc\nd\ne\nF\n");
    write(p, "other.rs", "z\n");
    commit_as(p, "Test <test@example.com>", "feat: capital a");

    let blamed = blamed(p, "new.rs");
    let named: Vec<(&str, &str, Vec<String>)> = blamed
        .lines
        .iter()
        .map(|line| {
            let c = blamed.commit_of(line).unwrap();
            (c.summary.as_str(), c.filename.as_str(), c.paths())
        })
        .collect();
    assert_eq!(
        named[0],
        ("feat: capital a", "new.rs", vec!["new.rs".into()])
    );
    assert_eq!(named[1], ("initial", "old.rs", vec!["old.rs".into()]));
    assert_eq!(
        named[5],
        (
            "refactor: rename and edit",
            "mid.rs",
            vec!["old.rs".into(), "mid.rs".into()]
        )
    );
    let diffs: Vec<Vec<String>> = [0, 1, 5]
        .iter()
        .map(|i| {
            let c = blamed.commit_of(&blamed.lines[*i]).unwrap();
            shown(&git::show_commit_paths(p, &c.sha, &c.paths()).unwrap())
        })
        .collect();
    assert_eq!(
        diffs,
        [
            vec!["M new.rs +1 -1".to_string()],
            vec!["A old.rs +6 -0".to_string()],
            vec!["R old.rs -> mid.rs".to_string()],
        ],
        "only this file, under the names it had then"
    );
}

#[test]
fn tabs_empty_lines_crlf_wide_characters_and_no_final_newline_survive() {
    let text =
        "\tindented\n\n\t\ttwice\r\nwindows\r\n日本語 は 幅\nnaïve café\nno newline at the end";
    let repo = repo_with(&[("odd.txt", text)]);
    let blamed = blamed(repo.path(), "odd.txt");
    let lines: Vec<(u32, &str)> = blamed
        .lines
        .iter()
        .map(|l| (l.number, l.text.as_str()))
        .collect();
    assert_eq!(
        lines,
        [
            (1, "\tindented"),
            (2, ""),
            (3, "\t\ttwice"),
            (4, "windows"),
            (5, "日本語 は 幅"),
            (6, "naïve café"),
            (7, "no newline at the end"),
        ]
    );
    assert!(blamed.lines.iter().all(|l| l.commit == Some(0)));
}

#[test]
fn control_characters_in_an_author_or_a_subject_stay_inside_their_field() {
    let repo = repo_with(&[("a.txt", "1\n2\n")]);
    let p = repo.path();
    let target = p.join("written-by-git.txt");
    write(p, "a.txt", "1\ntwo\n");
    let author = "Ev\u{1f}il\u{1e}\tName <evil@example.com>";
    let subject = format!("s\u{1e}--output={}\u{1f}\tq\rr\u{7}", target.display());
    commit_as(p, author, &subject);

    let blamed = blamed(p, "a.txt");
    assert_eq!(blamed.lines.len(), 2);
    let evil = blamed.commit_of(&blamed.lines[1]).unwrap();
    assert_eq!(evil.author, "Ev\u{1f}il\u{1e}\tName");
    assert_eq!(evil.summary, subject);
    assert_eq!(blamed.lines[1].text, "two");
    assert_eq!(
        blamed.commit_of(&blamed.lines[0]).unwrap().summary,
        "initial"
    );
    git::show_commit_paths(p, &evil.sha, &evil.paths()).unwrap();
    assert!(!target.exists());
}

#[test]
fn awkward_names_are_blamed_literally() {
    let names = [
        "with space.rs",
        "[a]*.rs",
        "ab.rs",
        "-dash.rs",
        "résumé.md",
        "docs/naïve café.txt",
        ":(glob)magic.rs",
    ];
    let files: Vec<(&str, &str)> = names.iter().map(|n| (*n, "first\n")).collect();
    let repo = repo_with(&files);
    let p = repo.path();
    for name in names {
        commit_file(p, name, "first\nsecond\n", &format!("edit {name}"));
    }
    for name in names {
        let blamed = blamed(p, name);
        let subjects: Vec<String> = rows(&blamed).into_iter().map(|r| r.2).collect();
        assert_eq!(subjects, ["initial".to_string(), format!("edit {name}")]);
        let edit = blamed.commit_of(&blamed.lines[1]).unwrap();
        assert_eq!(edit.filename, name);
        assert_eq!(
            shown(&git::show_commit_paths(p, &edit.sha, &edit.paths()).unwrap()),
            [format!("M {name} +1 -0")]
        );
    }
}

#[test]
fn files_with_nothing_to_blame_say_why() {
    let repo = repo_with(&[
        ("kept.rs", "x\n"),
        ("gone.rs", "y\n"),
        ("empty.txt", ""),
        ("picture.bin", "\0\u{1}\u{2}"),
        ("big.txt", "small\n"),
    ]);
    let p = repo.path();
    write(p, "untracked.rs", "u\n");
    write(p, "staged.rs", "s\n");
    git(p, &["add", "staged.rs"]);
    std::fs::remove_file(p.join("gone.rs")).unwrap();
    let line = "a line that is long enough to add up quickly\n";
    let big = line.repeat(BLAME_BYTES as usize / line.len() + 1);
    write(p, "big.txt", &big);

    assert_eq!(read(p, "untracked.rs", None), BlameRead::NotCommitted);
    assert_eq!(read(p, "staged.rs", None), BlameRead::NotCommitted);
    assert_eq!(read(p, "gone.rs", None), BlameRead::Deleted);
    assert_eq!(read(p, "never-existed.rs", None), BlameRead::Deleted);
    assert_eq!(read(p, "empty.txt", None), BlameRead::Empty);
    assert_eq!(read(p, "picture.bin", None), BlameRead::Binary);
    assert_eq!(
        read(p, "big.txt", None),
        BlameRead::TooLarge(big.len() as u64)
    );
    assert!(matches!(read(p, "kept.rs", None), BlameRead::Lines(_)));
    write(p, "kept.rs", "");
    assert_eq!(read(p, "kept.rs", None), BlameRead::Empty);

    std::fs::create_dir_all(p.join("folder")).unwrap();
    let info = git::discover(p).unwrap();
    let scratch = TempDir::new("blame-scratch");
    let folder = blame::blame(&info, scratch.path(), "folder", None).unwrap_err();
    assert_eq!(folder.0, "folder is a folder");

    let fresh = TempDir::new("no-commits");
    git(fresh.path(), &["init", "-q"]);
    write(fresh.path(), "a.txt", "a\n");
    assert_eq!(read(fresh.path(), "a.txt", None), BlameRead::NotCommitted);
}

#[test]
fn an_uncommitted_rename_is_blamed_under_its_old_name() {
    let repo = repo_with(&[("old.rs", "one\ntwo\n")]);
    let p = repo.path();
    git(p, &["mv", "old.rs", "new.rs"]);
    write(p, "new.rs", "one\ntwo\nthree\n");
    assert_eq!(read(p, "new.rs", None), BlameRead::NotCommitted);
    let BlameRead::Lines(blamed) = read(p, "new.rs", Some("old.rs")) else {
        panic!("blamed under the old name");
    };
    assert_eq!(
        rows(&blamed),
        [
            row(1, "Test", "initial", "one"),
            row(2, "Test", "initial", "two"),
            row(3, "-", "not committed", "three"),
        ]
    );
}

#[test]
fn nothing_in_blame_output_becomes_an_option_of_a_later_git_call() {
    let repo = repo_with(&[("seed.txt", "seed\n")]);
    let p = repo.path();
    let target = p.join("written-by-git.txt");
    let option = format!("--output={}", target.display());
    let fake_header = format!("{} 1 1 1", "0123456789".repeat(4));
    let relative = "--output=written-by-git.txt";
    let hostile_name = format!("a\n{fake_header}\n{relative}");
    write(p, &hostile_name, &format!("{fake_header}\n{relative}\n"));
    let subject = format!("s\u{1e}{option}\nprevious {} {option}", "f".repeat(40));
    commit_as(p, &format!("{option} <x@example.com>"), &subject);
    git(p, &["mv", &hostile_name, "plain.txt"]);
    commit_as(p, "Test <test@example.com>", "refactor: a plain name");
    write(p, "--output=named.txt", "a file named like an option\n");
    commit_as(p, "Test <test@example.com>", &option);

    for path in ["plain.txt", "--output=named.txt"] {
        let blamed = blamed(p, path);
        assert!(!blamed.lines.is_empty(), "{path}");
        for c in &blamed.commits {
            assert!(is_object_id(&c.sha), "{path}: {}", c.sha);
            git::show_commit_paths(p, &c.sha, &c.paths()).unwrap();
            git::show_commit(p, &c.sha).unwrap();
            branch::on_origin(p, &c.sha).unwrap();
        }
    }
    let plain = blamed(p, "plain.txt");
    assert_eq!(
        plain
            .lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>(),
        [fake_header.as_str(), relative]
    );
    let first = plain.commit_of(&plain.lines[0]).unwrap();
    assert_eq!(
        first.filename, hostile_name,
        "a quoted name is read back whole"
    );
    assert_eq!(first.summary, format!("s\u{1e}{option}"));
    assert!(!target.exists(), "git wrote {}", target.display());
    assert!(git::show_commit_paths(p, &option, &["plain.txt".to_string()]).is_err());
    assert!(!target.exists());
}

#[test]
fn reading_a_blame_leaves_the_repository_and_its_index_alone() {
    let repo = repo_with(&[("a.txt", "1\n2\n"), ("b.txt", "b\n")]);
    let p = repo.path();
    write(p, "a.txt", "1\n2\n3\n");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    write(p, "b.txt", "b\n");
    let before = (fingerprint(p), tree_bytes(p));
    for path in ["a.txt", "b.txt"] {
        assert!(matches!(read(p, path, None), BlameRead::Lines(_)));
    }
    assert_eq!((fingerprint(p), tree_bytes(p)), before);
}

#[cfg(unix)]
#[test]
fn anything_but_a_regular_file_or_a_link_is_refused_without_being_opened() {
    let repo = repo_with(&[("pipe.txt", "x\n")]);
    let p = repo.path();
    std::fs::remove_file(p.join("pipe.txt")).unwrap();
    let made = std::process::Command::new("mkfifo")
        .arg(p.join("pipe.txt"))
        .status()
        .unwrap();
    assert!(made.success());
    let (tx, rx) = std::sync::mpsc::channel();
    let root = p.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(read(&root, "pipe.txt", None));
    });
    let read = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("a pipe must not block the blame");
    assert_eq!(read, BlameRead::NotRegular);
}

#[cfg(unix)]
fn repo_with_a_link_to_outside() -> (TempDir, TempDir) {
    let outside = TempDir::new("outside");
    write(outside.path(), "secret.txt", "OUTSIDE-MARKER\nsecond\n");
    let repo = TempDir::new("repo");
    let p = repo.path();
    git(p, &["init", "-q"]);
    write(p, "seed.txt", "seed\n");
    std::os::unix::fs::symlink(outside.path().join("secret.txt"), p.join("link")).unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "initial"]);
    (repo, outside)
}

#[cfg(unix)]
#[test]
fn a_committed_link_is_blamed_as_its_own_text_never_as_its_target() {
    let (repo, outside) = repo_with_a_link_to_outside();
    let blamed = blamed(repo.path(), "link");
    let target = outside.path().join("secret.txt");
    assert_eq!(
        rows(&blamed),
        [row(1, "Test", "initial", &target.display().to_string())]
    );
    assert!(!format!("{blamed:?}").contains("OUTSIDE-MARKER"));
}

#[cfg(unix)]
#[test]
fn a_link_renamed_but_not_committed_is_never_read_through() {
    let (repo, _outside) = repo_with_a_link_to_outside();
    let p = repo.path();
    git(p, &["mv", "link", "moved"]);
    let result = read(p, "moved", Some("link"));
    assert_eq!(result, BlameRead::NotCommitted);
    assert!(!format!("{result:?}").contains("OUTSIDE-MARKER"));
    assert_eq!(read(p, "moved", None), BlameRead::NotCommitted);
}

#[test]
fn a_date_that_cannot_be_converted_is_a_question_mark_and_none_panics() {
    codemap::util::freeze_time(1_790_027_160);
    assert_eq!(codemap::util::date(i64::MIN), "?");
    assert_eq!(codemap::util::date(i64::MAX), "?");
    assert_eq!(codemap::util::date(0), "1 Jan 1970");
    assert_eq!(codemap::util::date(253_402_300_800), "1 Jan 10000");
    assert_eq!(codemap::util::date(99_999_999_999_999), "7 Nov 3170843");
    assert_eq!(codemap::util::date(-124_271_193_600), "2 Jan -1968");
    for unix in [i64::MIN, i64::MAX, 0, 253_402_300_800, -124_271_193_600] {
        let shown = codemap::util::pad(&codemap::util::date(unix), 11);
        assert_eq!(codemap::util::width(&shown), 11, "{unix}");
    }
}
