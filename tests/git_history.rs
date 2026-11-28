mod common;

use std::path::Path;

use codemap::app::HISTORY_LIMIT;
use codemap::git::graph::{self, FileCommit};
use codemap::git::{self, branch, FileDiff, FileStatus};
use common::*;

fn history_of(root: &Path, path: &str) -> Vec<FileCommit> {
    graph::file_log(root, path, HISTORY_LIMIT).unwrap()
}

fn subjects(history: &[FileCommit]) -> Vec<&str> {
    history.iter().map(|c| c.commit.subject.as_str()).collect()
}

fn diff_of(root: &Path, touched: &FileCommit) -> Vec<FileDiff> {
    git::show_commit_paths(root, &touched.commit.sha, &touched.paths).unwrap()
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

#[test]
fn a_file_history_follows_a_rename_and_each_diff_holds_only_that_file() {
    let repo = repo_with(&[("a.rs", "one\ntwo\nthree\n"), ("other.rs", "x\n")]);
    let p = repo.path();
    write(p, "a.rs", "one\ntwo\nthree\nfour\n");
    write(p, "other.rs", "y\n");
    git(p, &["commit", "-qam", "feat: count to four"]);
    commit_file(p, "other.rs", "z\n", "chore: only the other file");
    git(p, &["mv", "a.rs", "b.rs"]);
    git(p, &["commit", "-qm", "refactor: rename a.rs to b.rs"]);
    write(p, "b.rs", "one\ntwo\nthree\nfour\nfive\n");
    write(p, "other.rs", "w\n");
    git(p, &["commit", "-qam", "feat: count to five"]);

    let history = history_of(p, "b.rs");
    assert_eq!(
        subjects(&history),
        [
            "feat: count to five",
            "refactor: rename a.rs to b.rs",
            "feat: count to four",
            "initial"
        ]
    );
    let names: Vec<&[String]> = history.iter().map(|c| c.paths.as_slice()).collect();
    assert_eq!(
        names,
        [
            &["b.rs".to_string()][..],
            &["a.rs".to_string(), "b.rs".to_string()][..],
            &["a.rs".to_string()][..],
            &["a.rs".to_string()][..],
        ]
    );
    let diffs: Vec<Vec<String>> = history.iter().map(|c| shown(&diff_of(p, c))).collect();
    assert_eq!(
        diffs,
        [
            vec!["M b.rs +1 -0".to_string()],
            vec!["R a.rs -> b.rs".to_string()],
            vec!["M a.rs +1 -0".to_string()],
            vec!["A a.rs +3 -0".to_string()],
        ],
        "the other file never shows, and the old name is read before the rename"
    );
    let rename = diff_of(p, &history[1]);
    assert_eq!(rename[0].status, FileStatus::Renamed);
}

#[test]
fn awkward_names_are_passed_to_git_literally() {
    let names = [
        "with space.rs",
        "[a]*.rs",
        "ab.rs",
        "-dash.rs",
        "résumé.md",
        "docs/naïve café.txt",
    ];
    let files: Vec<(&str, &str)> = names.iter().map(|n| (*n, "first\n")).collect();
    let repo = repo_with(&files);
    let p = repo.path();
    for name in names {
        commit_file(p, name, "first\nsecond\n", &format!("edit {name}"));
    }
    for name in names {
        let history = history_of(p, name);
        assert_eq!(
            subjects(&history),
            [format!("edit {name}").as_str(), "initial"],
            "{name}"
        );
        assert_eq!(
            shown(&diff_of(p, &history[0])),
            [format!("M {name} +1 -0")],
            "{name}"
        );
        assert_eq!(
            shown(&diff_of(p, &history[1])),
            [format!("A {name} +1 -0")],
            "{name}"
        );
    }
}

#[test]
fn a_new_or_untracked_file_has_no_history_and_no_error() {
    let repo = repo_with(&[("kept.rs", "x\n")]);
    let p = repo.path();
    write(p, "untracked.rs", "y\n");
    write(p, "staged.rs", "z\n");
    git(p, &["add", "staged.rs"]);
    for name in ["untracked.rs", "staged.rs", "never-existed.rs"] {
        assert!(history_of(p, name).is_empty(), "{name}");
    }
}

#[test]
fn the_history_is_capped_like_the_full_one() {
    let repo = repo_with(&[("count.txt", "0\n")]);
    let p = repo.path();
    for n in 1..=4 {
        commit_file(p, "count.txt", &format!("{n}\n"), &format!("count {n}"));
    }
    let newest = graph::file_log(p, "count.txt", 2).unwrap();
    assert_eq!(subjects(&newest), ["count 4", "count 3"]);
}

#[test]
fn a_commit_is_on_origin_only_when_a_remote_tracking_branch_holds_it() {
    let remoted = repo_with_origin();
    let p = remoted.root.as_path();
    let pushed = git(p, &["rev-parse", "HEAD"]).trim().to_string();
    commit_file(p, "app.py", "x = 2\n", "fix: not pushed yet");
    let local = git(p, &["rev-parse", "HEAD"]).trim().to_string();
    assert_eq!(branch::on_origin(p, &pushed), Ok(true));
    assert_eq!(branch::on_origin(p, &local), Ok(false));
    git(p, &["branch", "-q", "only-here"]);
    assert_eq!(
        branch::on_origin(p, &local),
        Ok(false),
        "a local branch is not origin"
    );
    let lone = repo_with(&[("a.txt", "a\n")]);
    let head = git(lone.path(), &["rev-parse", "HEAD"]).trim().to_string();
    assert_eq!(branch::on_origin(lone.path(), &head), Ok(false));
}

fn repo_with_option_in_a_subject() -> (common::TempDir, std::path::PathBuf) {
    let repo = repo_with(&[("a.txt", "1\n")]);
    let p = repo.path();
    let target = p.join("written-by-git.txt");
    commit_file(
        p,
        "a.txt",
        "2\n",
        &format!("s\u{1e}--output={}", target.display()),
    );
    commit_file(p, "a.txt", "3\n", "plain");
    (repo, target)
}

#[test]
fn a_record_separator_in_a_subject_never_makes_a_commit_that_git_reads_as_an_option() {
    let (repo, target) = repo_with_option_in_a_subject();
    let p = repo.path();

    let full = graph::log(p, 100).unwrap();
    let file = history_of(p, "a.txt");
    let listed: Vec<&str> = full.iter().map(|c| c.sha.as_str()).collect();
    assert_eq!(full.len(), 3, "{listed:?}");
    assert_eq!(file.len(), 3);
    for sha in full
        .iter()
        .map(|c| &c.sha)
        .chain(file.iter().map(|c| &c.commit.sha))
    {
        assert!(
            sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()),
            "{sha}"
        );
    }
    let subjects_of_full: Vec<&str> = full.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects_of_full, ["plain", "s", "initial"]);

    for c in &full {
        git::show_commit(p, &c.sha).unwrap();
    }
    for c in &file {
        diff_of(p, c);
    }
    assert!(!target.exists(), "git wrote {}", target.display());
}

#[test]
fn a_sha_that_looks_like_an_option_is_a_revision_not_an_option() {
    let repo = repo_with(&[("a.txt", "1\n")]);
    let p = repo.path();
    let target = p.join("written-by-git.txt");
    let option = format!("--output={}", target.display());
    assert!(git::show_commit(p, &option).is_err());
    assert!(git::show_commit_paths(p, &option, &["a.txt".to_string()]).is_err());
    assert!(!target.exists());
}

#[test]
fn a_copied_file_is_marked_as_copied() {
    let repo = repo_with(&[("src.rs", "one\ntwo\nthree\n")]);
    let p = repo.path();
    commit_file(p, "copy.rs", "one\ntwo\nthree\n", "feat: copy it");
    let copied = history_of(p, "copy.rs");
    assert_eq!(copied[0].paths, ["src.rs", "copy.rs"]);
    assert!(copied[0].copied);
    assert!(!history_of(p, "src.rs")[0].copied);
}
