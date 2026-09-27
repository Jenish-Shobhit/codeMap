//! Git operations on temp repos: turn diffs from checkpoints, the shadow
//! store writing nothing into the repo, the commit graph and worktrees.

mod common;

use codemap::git::{self, FileStatus};
use codemap::store::{PaneState, Store, Turn};
use common::*;

fn store_in(t: &TempDir) -> Store {
    Store::new(t.path().join("state"))
}

#[test]
fn discover_reports_root_head_and_branch() {
    let repo = repo_with(&[("src/app.py", "def main():\n    return 1\n")]);
    let info = git::discover(&repo.path().join("src")).unwrap();
    assert_eq!(info.root, repo.path());
    assert!(info.head.is_some());
    assert_eq!(info.branch.as_deref(), Some("main"));
    assert!(!info.is_empty());
    let files = git::list_files(repo.path()).unwrap();
    assert_eq!(files, vec!["src/app.py"]);
}

#[test]
fn not_a_repo_is_reported_as_none() {
    let t = TempDir::new("plain");
    write(t.path(), "a.py", "x = 1\n");
    match git::discover(t.path()) {
        Err(None) => {}
        other => panic!("expected not-a-repo, got {other:?}"),
    }
    assert_eq!(git::walk_files(t.path(), 100), vec!["a.py"]);
}

#[test]
fn turn_diff_shows_only_the_turn() {
    let repo = repo_with(&[
        ("app.py", "def main():\n    return 1\n"),
        ("util.py", "def helper():\n    pass\n"),
    ]);
    let state = TempDir::new("state");
    let store = store_in(&state);
    // The user edits before the agent's turn: not part of the turn.
    write(repo.path(), "util.py", "def helper():\n    return 2\n");
    let info = git::discover(repo.path()).unwrap();
    let shadow = store.shadow(&info);

    let start = shadow.snapshot("turn 1 start").unwrap();
    // The agent's turn: edit app.py, add new.py.
    write(
        repo.path(),
        "app.py",
        "def main():\n    return run()\n\ndef run():\n    return 1\n",
    );
    write(repo.path(), "pkg/new.py", "def added():\n    pass\n");
    let end = shadow.snapshot("turn 1 end").unwrap();

    let turn = shadow.diff(&start.commit, &end.commit).unwrap();
    let paths: Vec<&str> = turn.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["app.py", "pkg/new.py"],
        "util.py was the user's edit"
    );
    assert_eq!(turn[1].status, FileStatus::Added);
    // Line-level marks: new lines 2-4 were added; main() as a whole changed.
    assert_eq!(turn[0].mark_for_range(2, 4), Some('A'));
    assert_eq!(turn[0].mark_for_range(1, 5), Some('M'));

    // Since HEAD includes the user's edit too.
    let head = info.head.clone().unwrap();
    let since_head = shadow.diff(&head, &end.tree).unwrap();
    let paths: Vec<&str> = since_head.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, vec!["app.py", "pkg/new.py", "util.py"]);

    // Refs live in the shadow only.
    shadow
        .update_ref("refs/codemap/p1/1-start", &start.commit)
        .unwrap();
    assert!(shadow
        .refs()
        .contains(&"refs/codemap/p1/1-start".to_string()));
    let user_refs = common::git(repo.path(), &["for-each-ref", "--format=%(refname)"]);
    assert!(!user_refs.contains("codemap"));
}

#[test]
fn shadow_store_writes_nothing_into_the_repo() {
    let repo = repo_with(&[("a.py", "x = 1\n"), ("b/c.py", "def f():\n    pass\n")]);
    write(repo.path(), "a.py", "x = 2\n");
    write(repo.path(), "untracked.py", "y = 3\n");
    // A stale stat cache tempts `git status` into refreshing the index.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let before = fingerprint(repo.path());

    let state = TempDir::new("state");
    let store = store_in(&state);
    let info = git::discover(repo.path()).unwrap();
    let shadow = store.shadow(&info);
    let a = shadow.snapshot("start").unwrap();
    write(repo.path(), "b/c.py", "def f():\n    return 1\n");
    let b = shadow.snapshot("end").unwrap();
    shadow
        .update_ref("refs/codemap/x/1-end", &b.commit)
        .unwrap();
    let _ = shadow.diff(&a.commit, &b.commit).unwrap();
    let _ = shadow.diff(info.head.as_deref().unwrap(), &b.tree).unwrap();
    let head_diff = git::diff_head(&info, &shadow.dir).unwrap();
    let paths: Vec<&str> = head_diff.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, vec!["a.py", "b/c.py", "untracked.py"]);
    assert_eq!(head_diff[2].status, FileStatus::Added);
    let _ = git::list_files(repo.path()).unwrap();
    let _ = git::graph::log(repo.path(), 100).unwrap();
    let _ = git::worktrees(repo.path()).unwrap();
    let _ = git::discover(repo.path()).unwrap();

    // Undo the one work-tree edit the test itself made, then compare.
    write(repo.path(), "b/c.py", "def f():\n    pass\n");
    let after = fingerprint(repo.path());
    assert_eq!(
        before.0, after.0,
        ".git changed: codeMap wrote into the repo"
    );
    assert_eq!(before.1, after.1, "git status changed");
    // The shadow really has its own objects and index.
    assert!(shadow.git_dir.join("index").is_file());
    // No persistent alternates: writes never reach the user's object store.
    assert!(!shadow.git_dir.join("objects/info/alternates").exists());
    let objects = std::fs::read_dir(shadow.git_dir.join("objects"))
        .unwrap()
        .count();
    assert!(objects > 2, "the shadow holds its own objects");
}

#[test]
fn empty_repo_snapshots_against_the_empty_tree() {
    let t = TempDir::new("empty");
    common::git(t.path(), &["init", "-q"]);
    write(t.path(), "first.py", "def hello():\n    print('hi')\n");
    let info = git::discover(t.path()).unwrap();
    assert!(info.is_empty());
    let state = TempDir::new("state");
    let store = store_in(&state);
    let shadow = store.shadow(&info);
    let now = shadow.snapshot("now").unwrap();
    let empty = shadow.empty_tree().unwrap();
    let diff = shadow.diff(&empty, &now.tree).unwrap();
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0].status, FileStatus::Added);
    assert_eq!(git::graph::log(t.path(), 10).unwrap_or_default().len(), 0);
}

#[test]
fn graph_of_branches_and_merge() {
    let repo = repo_with(&[("a.txt", "1\n")]);
    let p = repo.path();
    common::git(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "b.txt", "2\n");
    common::git(p, &["add", "-A"]);
    common::git(p, &["commit", "-q", "-m", "feature work"]);
    common::git(p, &["checkout", "-q", "main"]);
    write(p, "c.txt", "3\n");
    common::git(p, &["add", "-A"]);
    common::git(p, &["commit", "-q", "-m", "main work"]);
    common::git(
        p,
        &["merge", "-q", "--no-ff", "feature", "-m", "merge feature"],
    );
    common::git(p, &["tag", "v0.1.0"]);

    let commits = git::graph::log(p, 100).unwrap();
    assert_eq!(commits.len(), 4);
    assert!(commits[0].is_head());
    assert!(commits[0].labels().iter().any(|l| l == "main"));
    assert!(commits[0].labels().iter().any(|l| l == "tag: v0.1.0"));
    assert_eq!(commits[0].parents.len(), 2);
    let rows = git::graph::layout(&commits);
    let text: Vec<String> = rows.iter().map(|r| r.text()).collect();
    assert_eq!(text[0], "●");
    assert_eq!(text[1], "├─╮");
    assert!(text.iter().any(|t| t == "├─╯"), "{text:?}");
    assert_eq!(rows.iter().filter(|r| r.commit.is_some()).count(), 4);

    // Commit diff for History's ⏎.
    let merge_files = git::show_commit(p, &commits[0].sha).unwrap();
    assert_eq!(merge_files.len(), 1);
    assert_eq!(merge_files[0].path, "b.txt");
}

#[test]
fn worktrees_are_listed() {
    let repo = repo_with(&[("a.txt", "1\n")]);
    let wt = TempDir::new("wt");
    let wt_path = wt.path().join("feature-wt");
    common::git(
        repo.path(),
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "agent-branch",
            wt_path.to_str().unwrap(),
        ],
    );
    let list = git::worktrees(repo.path()).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[1].branch.as_deref(), Some("agent-branch"));
    // A linked worktree resolves to its own root but the shared object dir.
    let info = git::discover(&wt_path).unwrap();
    assert_eq!(info.root, wt_path.canonicalize().unwrap());
    assert_eq!(info.common_dir, repo.path().join(".git"));
    // Snapshots of a linked worktree work and leave the main repo alone.
    let before = fingerprint(repo.path());
    let state = TempDir::new("state");
    let shadow = store_in(&state).shadow(&info);
    write(&wt_path, "agent.py", "x = 1\n");
    let snap = shadow.snapshot("wt").unwrap();
    assert!(!snap.tree.is_empty());
    std::fs::remove_file(wt_path.join("agent.py")).unwrap();
    let after = fingerprint(repo.path());
    assert_eq!(before.0, after.0);
}

#[test]
fn pane_state_round_trips() {
    let state = TempDir::new("state");
    let store = store_in(&state);
    let key = PaneState::key_for(Some("term_1"), "w1:p1", Some("claude"));
    let mut pane = store.load_pane(&key);
    assert!(pane.turns.is_empty());
    pane.pane_id = "w1:p1".into();
    pane.terminal_id = Some("term_1".into());
    pane.updated = 5;
    pane.turns.push(Turn {
        n: 1,
        repo_root: "/r".into(),
        open: true,
        ..Default::default()
    });
    store.save_pane(&pane).unwrap();
    let loaded = store.load_pane(&key);
    assert_eq!(loaded, pane);
    assert_eq!(store.panes_for_repo(std::path::Path::new("/r")).len(), 1);
    assert!(store
        .panes_for_repo(std::path::Path::new("/other"))
        .is_empty());
}
