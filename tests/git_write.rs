mod common;

use std::path::Path;

use codemap::git::write::{self, takes_hunks};
use codemap::git::{self, FileDiff, FileStatus, Status};
use codemap::store::Store;
use common::*;

fn status_of(repo: &Path) -> Status {
    let state = TempDir::new("write-state");
    let info = git::discover(repo).unwrap();
    let scratch = Store::new(state.path()).shadow(&info).dir;
    git::status(&info, &scratch).unwrap()
}

fn named<'a>(files: &'a [FileDiff], path: &str) -> &'a FileDiff {
    files
        .iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("{path} is not in {:?}", shown(files)))
}

fn staged(repo: &Path, path: &str) -> FileDiff {
    named(&status_of(repo).staged, path).clone()
}

fn unstaged(repo: &Path, path: &str) -> FileDiff {
    named(&status_of(repo).unstaged, path).clone()
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

fn none() -> Vec<String> {
    Vec::new()
}

fn porcelain(repo: &Path) -> Vec<String> {
    fingerprint(repo).1.lines().map(str::to_string).collect()
}

fn in_index(repo: &Path, path: &str) -> Vec<u8> {
    let out = git_output(repo, &["cat-file", "blob", &format!(":{path}")]);
    assert!(out.status.success(), "{path} is not in the index");
    out.stdout
}

fn on_disk(repo: &Path, path: &str) -> Vec<u8> {
    std::fs::read(repo.join(path)).unwrap()
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap()
}

fn numbered(count: usize) -> Vec<String> {
    (1..=count).map(|n| format!("line {n}")).collect()
}

fn joined(lines: &[String]) -> String {
    lines.iter().map(|l| format!("{l}\n")).collect()
}

fn with_changed(lines: &[String], changed: &[usize]) -> String {
    let mut lines = lines.to_vec();
    for n in changed {
        lines[n - 1] = format!("line {n} changed");
    }
    joined(&lines)
}

fn three_hunks() -> (TempDir, Vec<String>) {
    let lines = numbered(30);
    let repo = repo_with(&[("f.txt", &joined(&lines))]);
    write(repo.path(), "f.txt", &with_changed(&lines, &[1, 15, 30]));
    (repo, lines)
}

fn stanzas(changed: Option<usize>) -> String {
    (1..=5)
        .map(|n| {
            let three = if changed == Some(n) { "THREE" } else { "three" };
            format!("begin\n  one\n  two\n  {three}\n  four\n  five\n  six\nend\n")
        })
        .collect()
}

const EIGHT_NEW: &str = "n1\nn2\nn3\nn4\nn5\nn6\nn7\nn8\n";

#[test]
fn staging_a_file_takes_a_modification_a_new_file_and_a_deletion() {
    let repo = repo_with(&[("a.py", "x = 1\n"), ("gone.py", "y = 1\n")]);
    let p = repo.path();
    write(p, "a.py", "x = 2\n");
    write(p, "pkg/new.py", "z = 3\n");
    std::fs::remove_file(p.join("gone.py")).unwrap();
    for path in ["a.py", "gone.py", "pkg/new.py"] {
        write::stage_file(p, &unstaged(p, path)).unwrap();
    }
    assert_eq!(porcelain(p), ["M  a.py", "D  gone.py", "A  pkg/new.py"]);
    assert_eq!(text(on_disk(p, "a.py")), "x = 2\n", "the file is untouched");
}

#[test]
fn staging_one_file_leaves_the_others_alone() {
    let repo = repo_with(&[("a.py", "x = 1\n"), ("b.py", "y = 1\n")]);
    let p = repo.path();
    write(p, "a.py", "x = 2\n");
    write(p, "b.py", "y = 2\n");
    write(p, "c.py", "z = 2\n");
    write::stage_file(p, &unstaged(p, "b.py")).unwrap();
    assert_eq!(porcelain(p), [" M a.py", "M  b.py", "?? c.py"]);
}

#[test]
fn a_name_that_reads_as_a_pattern_is_taken_as_that_one_file() {
    let repo = repo_with(&[("a[1].txt", "one\n"), ("a1.txt", "one\n")]);
    let p = repo.path();
    write(p, "a[1].txt", "two\n");
    write(p, "a1.txt", "two\n");
    write::stage_file(p, &unstaged(p, "a[1].txt")).unwrap();
    assert_eq!(porcelain(p), [" M a1.txt", "M  a[1].txt"]);
    write::unstage_file(p, &staged(p, "a[1].txt")).unwrap();
    assert_eq!(porcelain(p), [" M a1.txt", " M a[1].txt"]);
    write::discard_file(p, &unstaged(p, "a[1].txt")).unwrap();
    assert_eq!(porcelain(p), [" M a1.txt"]);
    assert_eq!(text(on_disk(p, "a1.txt")), "two\n");
}

#[test]
fn staging_a_file_that_is_gone_from_git_and_disk_is_an_error() {
    let repo = repo_with(&[("a.py", "x = 1\n")]);
    let p = repo.path();
    write(p, "loose.py", "y = 1\n");
    let loose = unstaged(p, "loose.py");
    std::fs::remove_file(p.join("loose.py")).unwrap();
    let before = fingerprint(p);
    let err = write::stage_file(p, &loose).unwrap_err();
    assert!(err.0.contains("loose.py"), "{err}");
    assert_eq!(before, fingerprint(p));
}

#[test]
fn unstaging_a_file_puts_the_index_back_to_head() {
    let repo = repo_with(&[
        ("a.py", "x = 1\n"),
        ("gone.py", "y = 1\n"),
        ("old_name.py", "def keep():\n    return 1\n"),
    ]);
    let p = repo.path();
    write(p, "a.py", "x = 2\n");
    write(p, "new.py", "z = 3\n");
    git(p, &["add", "a.py", "new.py"]);
    git(p, &["rm", "-q", "gone.py"]);
    git(p, &["mv", "old_name.py", "new_name.py"]);
    assert_eq!(
        porcelain(p),
        [
            "M  a.py",
            "D  gone.py",
            "A  new.py",
            "R  old_name.py -> new_name.py"
        ]
    );
    write::unstage_file(p, &staged(p, "a.py")).unwrap();
    assert_eq!(text(on_disk(p, "a.py")), "x = 2\n", "the edit is kept");
    write::unstage_file(p, &staged(p, "new.py")).unwrap();
    write::unstage_file(p, &staged(p, "gone.py")).unwrap();
    let renamed = staged(p, "new_name.py");
    assert_eq!(renamed.status, FileStatus::Renamed);
    write::unstage_file(p, &renamed).unwrap();
    assert_eq!(
        porcelain(p),
        [
            " M a.py",
            " D gone.py",
            " D old_name.py",
            "?? new.py",
            "?? new_name.py"
        ]
    );
    assert_eq!(shown(&status_of(p).staged), none());
}

#[test]
fn unstaging_before_the_first_commit_takes_the_file_out_of_the_index() {
    let t = TempDir::new("write-unborn");
    let p = t.path();
    git(p, &["init", "-q"]);
    write(p, "first.py", "x = 1\n");
    write(p, "second.py", "y = 1\n");
    git(p, &["add", "-A"]);
    write::unstage_file(p, &staged(p, "first.py")).unwrap();
    assert_eq!(porcelain(p), ["A  second.py", "?? first.py"]);
    assert_eq!(text(on_disk(p, "first.py")), "x = 1\n");
}

#[test]
fn unstaging_before_the_first_commit_keeps_content_that_is_nowhere_else() {
    let t = TempDir::new("write-unborn-edited");
    let p = t.path();
    git(p, &["init", "-q"]);
    write(p, "first.py", "x = 1\n");
    git(p, &["add", "-A"]);
    write(p, "first.py", "x = 2\n");
    let err = write::unstage_file(p, &staged(p, "first.py")).unwrap_err();
    assert!(err.0.contains("staged content different"), "{err}");
    assert_eq!(porcelain(p), ["AM first.py"]);
    assert_eq!(text(in_index(p, "first.py")), "x = 1\n");
}

#[test]
fn staging_the_middle_hunk_stages_exactly_that_hunk() {
    let (repo, lines) = three_hunks();
    let p = repo.path();
    let file = unstaged(p, "f.txt");
    assert_eq!(file.hunks.len(), 3);
    write::stage_hunk(p, &file, 1).unwrap();
    assert_eq!(text(in_index(p, "f.txt")), with_changed(&lines, &[15]));
    assert_eq!(
        text(on_disk(p, "f.txt")),
        with_changed(&lines, &[1, 15, 30])
    );
    let status = status_of(p);
    let (staged, left) = (
        named(&status.staged, "f.txt"),
        named(&status.unstaged, "f.txt"),
    );
    assert_eq!(staged.hunks, [file.hunks[1].clone()]);
    assert_eq!(left.hunks, [file.hunks[0].clone(), file.hunks[2].clone()]);
}

#[test]
fn the_hunks_at_the_start_and_the_end_of_a_file_stage_on_their_own() {
    let (repo, lines) = three_hunks();
    let p = repo.path();
    write::stage_hunk(p, &unstaged(p, "f.txt"), 0).unwrap();
    assert_eq!(text(in_index(p, "f.txt")), with_changed(&lines, &[1]));
    let left = unstaged(p, "f.txt");
    assert_eq!(left.hunks.len(), 2);
    write::stage_hunk(p, &left, 1).unwrap();
    assert_eq!(text(in_index(p, "f.txt")), with_changed(&lines, &[1, 30]));
    write::stage_hunk(p, &unstaged(p, "f.txt"), 0).unwrap();
    assert_eq!(in_index(p, "f.txt"), on_disk(p, "f.txt"));
    assert_eq!(porcelain(p), ["M  f.txt"]);
}

#[test]
fn neighbouring_hunks_stage_one_at_a_time() {
    let lines = numbered(20);
    let repo = repo_with(&[("f.txt", &joined(&lines))]);
    let p = repo.path();
    write(p, "f.txt", &with_changed(&lines, &[5, 13]));
    let file = unstaged(p, "f.txt");
    assert_eq!(
        (
            file.hunks.len(),
            file.hunks[0].new_end(),
            file.hunks[1].new_start
        ),
        (2, 8, 10),
        "one unchanged line keeps the two hunks apart"
    );
    write::stage_hunk(p, &file, 1).unwrap();
    assert_eq!(text(in_index(p, "f.txt")), with_changed(&lines, &[13]));
    write::stage_hunk(p, &unstaged(p, "f.txt"), 0).unwrap();
    assert_eq!(text(in_index(p, "f.txt")), with_changed(&lines, &[5, 13]));
}

#[test]
fn a_hunk_that_grows_the_file_shifts_the_ones_below_it() {
    let lines = numbered(30);
    let repo = repo_with(&[("f.txt", &joined(&lines))]);
    let p = repo.path();
    let mut grown = lines.clone();
    grown[29] = "line 30 changed".into();
    grown.splice(2..2, ["new a".to_string(), "new b".into(), "new c".into()]);
    grown.remove(17);
    write(p, "f.txt", &joined(&grown));
    let file = unstaged(p, "f.txt");
    assert_eq!(file.hunks.len(), 3);
    write::stage_hunk(p, &file, 2).unwrap();
    assert_eq!(text(in_index(p, "f.txt")), with_changed(&lines, &[30]));
    write::stage_hunk(p, &unstaged(p, "f.txt"), 1).unwrap();
    let mut expected = lines.clone();
    expected[29] = "line 30 changed".into();
    expected.remove(14);
    assert_eq!(text(in_index(p, "f.txt")), joined(&expected));
    write::stage_hunk(p, &unstaged(p, "f.txt"), 0).unwrap();
    assert_eq!(in_index(p, "f.txt"), on_disk(p, "f.txt"));
}

#[test]
fn a_hunk_lands_on_its_own_lines_when_the_same_lines_repeat() {
    let repo = repo_with(&[("f.txt", &stanzas(None))]);
    let p = repo.path();
    let edited = format!("{EIGHT_NEW}{}", stanzas(Some(3)));
    write(p, "f.txt", &edited);
    let file = unstaged(p, "f.txt");
    assert_eq!(file.hunks.len(), 2);
    assert_eq!((file.hunks[1].old_start, file.hunks[1].new_start), (17, 25));

    write::stage_hunk(p, &file, 1).unwrap();
    assert_eq!(text(in_index(p, "f.txt")), stanzas(Some(3)));

    git(p, &["add", "f.txt"]);
    write::unstage_hunk(p, &staged(p, "f.txt"), 1).unwrap();
    assert_eq!(
        text(in_index(p, "f.txt")),
        format!("{EIGHT_NEW}{}", stanzas(None))
    );
    assert_eq!(
        text(on_disk(p, "f.txt")),
        edited,
        "unstaging keeps the file"
    );

    git(p, &["reset", "-q"]);
    write::discard_hunk(p, &unstaged(p, "f.txt"), 1).unwrap();
    assert_eq!(
        text(on_disk(p, "f.txt")),
        format!("{EIGHT_NEW}{}", stanzas(None))
    );
    assert_eq!(text(in_index(p, "f.txt")), stanzas(None));
}

#[test]
fn a_file_without_a_final_newline_stages_byte_for_byte() {
    for (before, after) in [
        ("one\ntwo\nlast", "one\ntwo\nlast changed"),
        ("one\ntwo\nlast\n", "one\ntwo\nlast"),
        ("one\ntwo\nlast", "one\ntwo\nlast\n"),
        ("one\ntwo\nlast", "one\nTWO\nlast"),
        ("one\ntwo\nlast", "one\ntwo\nlast\nmore"),
    ] {
        let repo = repo_with(&[("f.txt", before)]);
        let p = repo.path();
        write(p, "f.txt", after);
        write::stage_hunk(p, &unstaged(p, "f.txt"), 0).unwrap();
        assert_eq!(text(in_index(p, "f.txt")), after, "{before:?} to {after:?}");
        write::unstage_hunk(p, &staged(p, "f.txt"), 0).unwrap();
        assert_eq!(
            text(in_index(p, "f.txt")),
            before,
            "{after:?} to {before:?}"
        );
        write::discard_hunk(p, &unstaged(p, "f.txt"), 0).unwrap();
        assert_eq!(text(on_disk(p, "f.txt")), before, "{after:?} discarded");
    }
}

#[test]
fn a_file_that_was_empty_or_becomes_empty_stages_as_a_hunk() {
    for (before, after) in [("", "one\ntwo\n"), ("one\ntwo\n", ""), ("", "no newline")] {
        let repo = repo_with(&[("f.txt", before)]);
        let p = repo.path();
        write(p, "f.txt", after);
        let file = unstaged(p, "f.txt");
        assert_eq!((file.status, file.hunks.len()), (FileStatus::Modified, 1));
        write::stage_hunk(p, &file, 0).unwrap();
        assert_eq!(text(in_index(p, "f.txt")), after, "{before:?} to {after:?}");
        write::unstage_hunk(p, &staged(p, "f.txt"), 0).unwrap();
        assert_eq!(
            text(in_index(p, "f.txt")),
            before,
            "{after:?} to {before:?}"
        );
        write::discard_hunk(p, &unstaged(p, "f.txt"), 0).unwrap();
        assert_eq!(text(on_disk(p, "f.txt")), before, "{after:?} discarded");
    }
}

#[test]
fn a_hunk_stages_without_the_change_to_the_files_mode() {
    use std::os::unix::fs::PermissionsExt;
    let repo = repo_with(&[("run.sh", "echo one\n")]);
    let p = repo.path();
    write(p, "run.sh", "echo two\n");
    std::fs::set_permissions(p.join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    write::stage_hunk(p, &unstaged(p, "run.sh"), 0).unwrap();
    assert_eq!(text(in_index(p, "run.sh")), "echo two\n");
    let entry = git(p, &["ls-files", "--stage", "run.sh"]);
    assert!(
        entry.starts_with("100644 "),
        "the mode is not staged: {entry}"
    );
    let left = unstaged(p, "run.sh");
    assert!(left.hunks.is_empty() && !takes_hunks(&left));
    write::stage_file(p, &left).unwrap();
    let entry = git(p, &["ls-files", "--stage", "run.sh"]);
    assert!(entry.starts_with("100755 "), "{entry}");
}

#[test]
fn one_hunk_of_a_file_without_a_final_newline_leaves_the_end_alone() {
    let mut lines = numbered(20);
    let repo = repo_with(&[("f.txt", joined(&lines).trim_end())]);
    let p = repo.path();
    lines[1] = "line 2 changed".into();
    let first_only = joined(&lines);
    lines[19] = "line 20 changed".into();
    write(p, "f.txt", joined(&lines).trim_end());
    let file = unstaged(p, "f.txt");
    assert_eq!(file.hunks.len(), 2);
    write::stage_hunk(p, &file, 0).unwrap();
    assert_eq!(text(in_index(p, "f.txt")), first_only.trim_end());
    write::stage_hunk(p, &unstaged(p, "f.txt"), 0).unwrap();
    assert_eq!(in_index(p, "f.txt"), on_disk(p, "f.txt"));
}

#[test]
fn bytes_that_are_not_utf8_and_carriage_returns_stage_as_they_are() {
    let latin = b"caf\xe9 one\r\nplain two\r\nthree\r\n".to_vec();
    let edited = b"caf\xe9 one\r\nna\xefve two\r\nthree\r\n".to_vec();
    let repo = repo_with(&[("a.py", "x = 1\n")]);
    let p = repo.path();
    std::fs::write(p.join("latin.txt"), &latin).unwrap();
    git(p, &["add", "latin.txt"]);
    git(p, &["commit", "-q", "-m", "latin"]);
    std::fs::write(p.join("latin.txt"), &edited).unwrap();
    write::stage_hunk(p, &unstaged(p, "latin.txt"), 0).unwrap();
    assert_eq!(in_index(p, "latin.txt"), edited);
    write::unstage_hunk(p, &staged(p, "latin.txt"), 0).unwrap();
    assert_eq!(in_index(p, "latin.txt"), latin);
    write::discard_hunk(p, &unstaged(p, "latin.txt"), 0).unwrap();
    assert_eq!(on_disk(p, "latin.txt"), latin);
}

#[test]
fn a_hunk_of_a_file_with_an_awkward_name_stages() {
    let name = "my docs/na\u{ef}ve \"draft\".txt";
    let repo = repo_with(&[(name, "one\ntwo\n")]);
    let p = repo.path();
    write(p, name, "one\nTWO\n");
    let file = unstaged(p, name);
    write::stage_hunk(p, &file, 0).unwrap();
    assert_eq!(text(in_index(p, name)), "one\nTWO\n");
    write::unstage_file(p, &staged(p, name)).unwrap();
    write::discard_hunk(p, &unstaged(p, name), 0).unwrap();
    assert_eq!(text(on_disk(p, name)), "one\ntwo\n");
}

#[test]
fn unstaging_one_hunk_keeps_the_other_staged_and_the_file_as_it_is() {
    let (repo, lines) = three_hunks();
    let p = repo.path();
    git(p, &["add", "f.txt"]);
    let all = with_changed(&lines, &[1, 15, 30]);
    write::unstage_hunk(p, &staged(p, "f.txt"), 0).unwrap();
    assert_eq!(text(in_index(p, "f.txt")), with_changed(&lines, &[15, 30]));
    write::unstage_hunk(p, &staged(p, "f.txt"), 1).unwrap();
    assert_eq!(text(in_index(p, "f.txt")), with_changed(&lines, &[15]));
    assert_eq!(text(on_disk(p, "f.txt")), all);
    assert_eq!(porcelain(p), ["MM f.txt"]);
}

#[test]
fn a_hunk_stages_on_top_of_what_is_already_staged() {
    let lines = numbered(30);
    let repo = repo_with(&[("f.txt", &joined(&lines))]);
    let p = repo.path();
    write(p, "f.txt", &with_changed(&lines, &[15]));
    git(p, &["add", "f.txt"]);
    write(p, "f.txt", &with_changed(&lines, &[1, 15, 30]));
    let file = unstaged(p, "f.txt");
    assert_eq!(file.hunks.len(), 2);
    write::stage_hunk(p, &file, 1).unwrap();
    assert_eq!(text(in_index(p, "f.txt")), with_changed(&lines, &[15, 30]));
}

#[test]
fn a_hunk_that_changed_since_it_was_read_is_refused() {
    let (repo, lines) = three_hunks();
    let p = repo.path();
    let read = unstaged(p, "f.txt");
    let mut later = lines.clone();
    later[0] = "line 1 changed".into();
    later[14] = "line 15 changed again".into();
    later[29] = "line 30 changed".into();
    write(p, "f.txt", &joined(&later));
    let before = fingerprint(p);
    for hunk in [
        write::stage_hunk(p, &read, 1),
        write::discard_hunk(p, &read, 1),
    ] {
        let err = hunk.unwrap_err();
        assert_eq!(err.0, "f.txt changed since this diff was read");
    }
    assert_eq!(before, fingerprint(p));
    assert_eq!(text(on_disk(p, "f.txt")), joined(&later));
    write::stage_hunk(p, &read, 0).unwrap();
    assert_eq!(text(in_index(p, "f.txt")), with_changed(&lines, &[1]));
}

#[test]
fn a_hunk_whose_final_newline_went_missing_since_it_was_read_is_refused() {
    let repo = repo_with(&[("f.txt", "one\ntwo\nlast\n")]);
    let p = repo.path();
    write(p, "f.txt", "one\ntwo\nlast changed\n");
    let read = unstaged(p, "f.txt");
    write(p, "f.txt", "one\ntwo\nlast changed");
    let err = write::stage_hunk(p, &read, 0).unwrap_err();
    assert_eq!(err.0, "f.txt changed since this diff was read");
    assert_eq!(text(in_index(p, "f.txt")), "one\ntwo\nlast\n");
    write::stage_hunk(p, &unstaged(p, "f.txt"), 0).unwrap();
    assert_eq!(text(in_index(p, "f.txt")), "one\ntwo\nlast changed");
}

#[test]
fn a_hunk_is_refused_where_only_the_whole_file_makes_sense() {
    let repo = repo_with(&[
        ("gone.py", "y = 1\n"),
        ("logo.bin", "\u{0}\u{1}one"),
        ("old_name.py", "def keep():\n    return 1\n"),
    ]);
    let p = repo.path();
    write(p, "loose.py", "z = 1\n");
    write(p, "logo.bin", "\u{0}\u{1}two");
    write(p, "new.py", "n = 1\n");
    git(p, &["add", "new.py"]);
    git(p, &["mv", "old_name.py", "new_name.py"]);
    std::fs::remove_file(p.join("gone.py")).unwrap();
    let status = status_of(p);
    let before = (fingerprint(p), tree_bytes(p));
    for file in status.unstaged.iter().chain(&status.staged) {
        assert!(!takes_hunks(file), "{}", file.path);
        for refused in [
            write::stage_hunk(p, file, 0),
            write::unstage_hunk(p, file, 0),
            write::discard_hunk(p, file, 0),
        ] {
            let err = refused.unwrap_err();
            assert!(err.0.contains("no hunk to take on its own"), "{err}");
        }
    }
    assert_eq!(status.unstaged.len() + status.staged.len(), 5);
    assert_eq!(before, (fingerprint(p), tree_bytes(p)));
}

#[test]
fn a_hunk_number_past_the_end_is_refused() {
    let (repo, _) = three_hunks();
    let p = repo.path();
    let before = fingerprint(p);
    assert!(write::stage_hunk(p, &unstaged(p, "f.txt"), 3).is_err());
    assert_eq!(before, fingerprint(p));
}

#[test]
fn a_hunk_of_a_conflicted_file_is_refused_by_git() {
    let repo = repo_with(&[("f.py", "one\ntwo\nthree\n")]);
    let p = repo.path();
    git(p, &["checkout", "-q", "-b", "side"]);
    write(p, "f.py", "one\nTWO side\nthree\n");
    git(p, &["commit", "-q", "-am", "side"]);
    git(p, &["checkout", "-q", "main"]);
    write(p, "f.py", "one\nTWO main\nthree\n");
    git(p, &["commit", "-q", "-am", "main"]);
    let merge = git_output(p, &["merge", "-q", "side"]);
    assert_eq!(merge.status.code(), Some(1));
    let conflicted = unstaged(p, "f.py");
    let marked = on_disk(p, "f.py");
    let err = write::stage_hunk(p, &conflicted, 0).unwrap_err();
    assert!(err.0.contains("f.py"), "{err}");
    let err = write::discard_file(p, &conflicted).unwrap_err();
    assert!(err.0.contains("unmerged"), "{err}");
    assert_eq!(on_disk(p, "f.py"), marked);
    assert_eq!(porcelain(p), ["UU f.py"]);
    write::stage_file(p, &conflicted).unwrap();
    assert_eq!(porcelain(p), ["M  f.py"]);
}

#[test]
fn discarding_a_file_restores_it_from_the_index() {
    let repo = repo_with(&[("a.py", "one = 1\n"), ("gone.py", "y = 1\n")]);
    let p = repo.path();
    write(p, "a.py", "one = 1\nstaged = 2\n");
    git(p, &["add", "a.py"]);
    write(p, "a.py", "one = 1\nstaged = 2\nlater = 3\n");
    std::fs::remove_file(p.join("gone.py")).unwrap();
    write::discard_file(p, &unstaged(p, "a.py")).unwrap();
    assert_eq!(text(on_disk(p, "a.py")), "one = 1\nstaged = 2\n");
    assert_eq!(text(in_index(p, "a.py")), "one = 1\nstaged = 2\n");
    write::discard_file(p, &unstaged(p, "gone.py")).unwrap();
    assert_eq!(text(on_disk(p, "gone.py")), "y = 1\n");
    assert_eq!(porcelain(p), ["M  a.py"]);
}

#[test]
fn discarding_an_untracked_file_deletes_that_file_only() {
    let repo = repo_with(&[("a.py", "x = 1\n")]);
    let p = repo.path();
    write(p, "a.py", "x = 2\n");
    write(p, "notes/one.md", "one\n");
    write(p, "notes/two.md", "two\n");
    let before = fingerprint(p).0;
    write::discard_file(p, &unstaged(p, "notes/one.md")).unwrap();
    assert_eq!(porcelain(p), [" M a.py", "?? notes/two.md"]);
    assert!(!p.join("notes/one.md").exists());
    assert_eq!(text(on_disk(p, "a.py")), "x = 2\n");
    assert_eq!(
        before,
        fingerprint(p).0,
        "deleting a file leaves .git alone"
    );
}

#[test]
fn a_file_that_became_tracked_is_not_deleted() {
    let repo = repo_with(&[("a.py", "x = 1\n")]);
    let p = repo.path();
    write(p, "notes.md", "one\n");
    let seen = unstaged(p, "notes.md");
    assert_eq!(seen.status, FileStatus::Untracked);
    git(p, &["add", "notes.md"]);
    let err = write::discard_file(p, &seen).unwrap_err();
    assert_eq!(err.0, "notes.md is tracked now: nothing deleted");
    assert_eq!(text(on_disk(p, "notes.md")), "one\n");
}

#[test]
fn a_folder_or_a_path_outside_the_repository_is_never_deleted() {
    let outer = TempDir::new("write-outer");
    let p = outer.path().join("repo");
    std::fs::create_dir_all(&p).unwrap();
    git(&p, &["init", "-q"]);
    write(outer.path(), "outside.md", "keep\n");
    write(&p, "nested/inner.md", "keep\n");
    let inner_repo = p.join("nested");
    git(&inner_repo, &["init", "-q"]);
    git(&inner_repo, &["add", "-A"]);
    git(&inner_repo, &["commit", "-q", "-m", "inner"]);
    let nested = unstaged(&p, "nested/");
    assert_eq!(nested.status, FileStatus::Untracked);
    let err = write::discard_file(&p, &nested).unwrap_err();
    assert!(err.0.contains("is a folder"), "{err}");
    let outside = outer.path().join("outside.md");
    for path in [
        "../outside.md",
        outside.to_str().unwrap(),
        "",
        "nested/../../outside.md",
    ] {
        let err = write::delete_untracked(&p, path).unwrap_err();
        assert!(err.0.contains("is not a file in this repository"), "{err}");
    }
    assert!(write::delete_untracked(&p, "missing.md").is_err());
    assert_eq!(text(on_disk(outer.path(), "outside.md")), "keep\n");
    assert_eq!(text(on_disk(&p, "nested/inner.md")), "keep\n");
}

#[test]
fn a_file_added_with_intent_is_not_emptied_by_a_discard() {
    let repo = repo_with(&[("a.py", "x = 1\n")]);
    let p = repo.path();
    write(p, "later.py", "y = 2\n");
    git(p, &["add", "--intent-to-add", "later.py"]);
    let later = unstaged(p, "later.py");
    assert_eq!(later.status, FileStatus::Added);
    let err = write::discard_file(p, &later).unwrap_err();
    assert!(err.0.contains("nothing to restore it from"), "{err}");
    assert_eq!(text(on_disk(p, "later.py")), "y = 2\n");
}

#[test]
fn discarding_one_hunk_leaves_the_other_hunks_and_the_index() {
    let (repo, lines) = three_hunks();
    let p = repo.path();
    let before = fingerprint(p).0;
    write::discard_hunk(p, &unstaged(p, "f.txt"), 1).unwrap();
    assert_eq!(text(on_disk(p, "f.txt")), with_changed(&lines, &[1, 30]));
    assert_eq!(
        before,
        fingerprint(p).0,
        "a discarded hunk leaves .git alone"
    );
    write::discard_hunk(p, &unstaged(p, "f.txt"), 1).unwrap();
    assert_eq!(text(on_disk(p, "f.txt")), with_changed(&lines, &[1]));
    write::discard_hunk(p, &unstaged(p, "f.txt"), 0).unwrap();
    assert_eq!(text(on_disk(p, "f.txt")), joined(&lines));
    assert_eq!(porcelain(p), none());
}

#[test]
fn discarding_a_hunk_keeps_what_is_staged() {
    let lines = numbered(30);
    let repo = repo_with(&[("f.txt", &joined(&lines))]);
    let p = repo.path();
    write(p, "f.txt", &with_changed(&lines, &[15]));
    git(p, &["add", "f.txt"]);
    write(p, "f.txt", &with_changed(&lines, &[1, 15, 30]));
    write::discard_hunk(p, &unstaged(p, "f.txt"), 0).unwrap();
    assert_eq!(text(on_disk(p, "f.txt")), with_changed(&lines, &[15, 30]));
    assert_eq!(text(in_index(p, "f.txt")), with_changed(&lines, &[15]));
}

#[test]
fn a_commit_takes_the_subject_and_the_body() {
    let repo = repo_with(&[("a.py", "x = 1\n")]);
    let p = repo.path();
    commits_as_test(p);
    write(p, "a.py", "x = 2\n");
    write(p, "b.py", "y = 2\n");
    git(p, &["add", "a.py"]);
    let short = write::commit(p, "fix: count from two", "").unwrap();
    assert_eq!(short, git(p, &["rev-parse", "--short", "HEAD"]).trim());
    assert_eq!(
        git(p, &["log", "-1", "--format=%an <%ae>%n%B"]),
        "Test <test@example.com>\nfix: count from two\n\n"
    );
    assert_eq!(porcelain(p), ["?? b.py"], "only what was staged");

    git(p, &["add", "b.py"]);
    write::commit(p, "feat: add b", "It holds y.\n\nSecond paragraph.\n\n").unwrap();
    assert_eq!(
        git(p, &["log", "-1", "--format=%B"]),
        "feat: add b\n\nIt holds y.\n\nSecond paragraph.\n\n"
    );
    assert_eq!(git(p, &["rev-list", "--count", "HEAD"]).trim(), "3");
}

#[test]
fn the_first_commit_of_a_repository() {
    let t = TempDir::new("write-first-commit");
    let p = t.path();
    git(p, &["init", "-q"]);
    commits_as_test(p);
    write(p, "first.py", "x = 1\n");
    git(p, &["add", "first.py"]);
    let short = write::commit(p, "-m is a fine subject", "").unwrap();
    assert_eq!(
        git(p, &["log", "--format=%h %s"]).trim(),
        format!("{short} -m is a fine subject")
    );
}

#[test]
fn a_commit_the_hook_rejects_changes_nothing_and_says_why() {
    let repo = repo_with(&[("a.py", "x = 1\n")]);
    let p = repo.path();
    commits_as_test(p);
    hook(
        p,
        "pre-commit",
        "#!/bin/sh\necho 'lint: a.py is not formatted' >&2\nexit 1\n",
    );
    write(p, "a.py", "x = 2\n");
    git(p, &["add", "a.py"]);
    let head = git(p, &["rev-parse", "HEAD"]);
    let err = write::commit(p, "fix: count from two", "").unwrap_err();
    assert_eq!(err.0, "lint: a.py is not formatted");
    assert_eq!(git(p, &["rev-parse", "HEAD"]), head);
    assert_eq!(porcelain(p), ["M  a.py"]);
}

#[test]
fn a_commit_without_an_author_fails_with_gits_words() {
    let repo = repo_with(&[("a.py", "x = 1\n")]);
    let p = repo.path();
    commits_as_test(p);
    git(p, &["config", "user.name", ""]);
    git(p, &["config", "user.useConfigOnly", "true"]);
    write(p, "a.py", "x = 2\n");
    git(p, &["add", "a.py"]);
    let head = git(p, &["rev-parse", "HEAD"]);
    let err = write::commit(p, "fix: count from two", "").unwrap_err();
    assert!(!err.0.trim().is_empty(), "git says why in its own words");
    assert_eq!(git(p, &["rev-parse", "HEAD"]), head);
    assert_eq!(porcelain(p), ["M  a.py"]);
}

#[test]
fn a_commit_with_nothing_staged_fails_with_gits_words() {
    let repo = repo_with(&[("a.py", "x = 1\n")]);
    let p = repo.path();
    commits_as_test(p);
    write(p, "a.py", "x = 2\n");
    let err = write::commit(p, "fix: nothing", "").unwrap_err();
    assert!(
        err.0.contains("a.py"),
        "git lists what is not staged: {err}"
    );
    assert_eq!(git(p, &["rev-list", "--count", "HEAD"]).trim(), "1");
}
