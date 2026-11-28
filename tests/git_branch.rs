mod common;

use std::path::{Path, PathBuf};

use codemap::git::branch::{self, Branch, Deletion, Merged, Remote, Unique, Upstream};
use common::*;

fn named(root: &Path, name: &str) -> Branch {
    branch::list(root)
        .unwrap()
        .local
        .into_iter()
        .find(|b| b.name == name)
        .unwrap_or_else(|| panic!("{name} is not listed"))
}

fn names(root: &Path) -> Vec<String> {
    git(root, &["branch", "--format=%(refname:short)"])
        .lines()
        .map(str::to_string)
        .collect()
}

fn current(root: &Path) -> String {
    git(root, &["branch", "--show-current"]).trim().to_string()
}

fn tip(root: &Path, revision: &str) -> String {
    git(root, &["rev-parse", revision]).trim().to_string()
}

fn everything(root: &Path) -> impl PartialEq + std::fmt::Debug {
    (fingerprint(root), tree_bytes(root))
}

fn worktree_on(remoted: &Remoted, branch: &str) -> PathBuf {
    let path = remoted.tmp.path().join("elsewhere");
    git(
        &remoted.root,
        &["worktree", "add", "-q", path.to_str().unwrap(), branch],
    );
    path
}

fn tracking(name: &str, ahead: usize, behind: usize) -> Option<Upstream> {
    Some(Upstream {
        name: name.into(),
        ahead,
        behind,
        gone: false,
    })
}

#[test]
fn the_list_gives_each_branch_its_upstream_and_how_far_apart_they_are() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    let read = branch::list(p).unwrap();
    let listed: Vec<&str> = read.local.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(
        listed,
        ["feat/ahead", "feat/both", "local-only", "main", "merged"]
    );
    let upstream = |name: &str| named(p, name).upstream;
    assert_eq!(upstream("feat/ahead"), tracking("origin/feat/ahead", 2, 0));
    assert_eq!(upstream("feat/both"), tracking("origin/feat/both", 2, 1));
    assert_eq!(upstream("main"), tracking("origin/main", 0, 0));
    assert_eq!(upstream("local-only"), None);
    assert_eq!(upstream("merged"), None);
    let on: Vec<&str> = read
        .local
        .iter()
        .filter(|b| b.current)
        .map(|b| b.name.as_str())
        .collect();
    assert_eq!(on, ["main"]);
    assert!(read.local.iter().all(|b| b.elsewhere.is_none()));
    assert_eq!(named(p, "main").tip, tip(p, "refs/heads/main"));
    assert_eq!(
        read.remotes,
        [Remote {
            name: "origin".into(),
            branches: 3
        }]
    );
    assert_eq!(read.default.as_deref(), Some("refs/heads/main"));
}

#[test]
fn a_branch_behind_its_upstream_and_one_whose_upstream_is_gone() {
    let remoted = repo_with_origin();
    let p = remoted.root.as_path();
    git(p, &["switch", "-q", "-c", "feat/behind"]);
    commit_file(p, "b.py", "b = 1\n", "feat: one");
    commit_file(p, "b.py", "b = 2\n", "feat: two");
    git(p, &["push", "-q", "-u", "origin", "feat/behind"]);
    git(p, &["reset", "-q", "--hard", "HEAD~2"]);
    assert_eq!(
        named(p, "feat/behind").upstream,
        tracking("origin/feat/behind", 0, 2)
    );
    git(p, &["update-ref", "-d", "refs/remotes/origin/feat/behind"]);
    let gone = named(p, "feat/behind").upstream.unwrap();
    assert!(gone.gone);
    assert_eq!((gone.ahead, gone.behind), (0, 0));
    git(p, &["branch", "-q", "--set-upstream-to=main"]);
    assert_eq!(named(p, "feat/behind").upstream, tracking("main", 0, 0));
}

#[test]
fn a_branch_checked_out_in_another_worktree_names_that_worktree() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    let elsewhere = worktree_on(&remoted, "local-only");
    assert_eq!(named(p, "local-only").elsewhere, Some(elsewhere.clone()));
    assert!(!named(p, "local-only").current);
    assert!(named(p, "main").current && named(p, "main").elsewhere.is_none());
    let seen_from_there = named(&elsewhere, "main");
    assert_eq!(seen_from_there.elsewhere.as_deref(), Some(p));
    assert!(named(&elsewhere, "local-only").current);
}

#[test]
fn a_detached_head_has_no_current_branch() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    git(p, &["switch", "-q", "--detach"]);
    assert!(branch::list(p).unwrap().local.iter().all(|b| !b.current));
}

#[test]
fn a_repository_with_no_commits_has_no_branches_and_no_default() {
    let unborn = TempDir::new("branch-unborn");
    git(unborn.path(), &["init", "-q"]);
    assert_eq!(
        branch::list(unborn.path()).unwrap(),
        branch::Branches::default()
    );
}

#[test]
fn the_default_branch_is_where_origin_head_points_then_main_then_master() {
    let remoted = repo_with_origin();
    let p = remoted.root.as_path();
    assert_eq!(
        branch::default_branch(p).as_deref(),
        Some("refs/heads/main")
    );
    git(p, &["switch", "-q", "-c", "trunk"]);
    git(p, &["push", "-q", "origin", "trunk"]);
    git(p, &["remote", "set-head", "origin", "trunk"]);
    assert_eq!(
        branch::default_branch(p).as_deref(),
        Some("refs/heads/trunk"),
        "origin/HEAD outranks a branch called main"
    );
    git(p, &["switch", "-q", "main"]);
    git(p, &["branch", "-q", "-D", "trunk"]);
    assert_eq!(
        branch::default_branch(p).as_deref(),
        Some("refs/remotes/origin/trunk"),
        "the remote-tracking branch when there is no local one"
    );
    git(p, &["remote", "set-head", "origin", "--delete"]);
    assert_eq!(
        branch::default_branch(p).as_deref(),
        Some("refs/heads/main")
    );

    let local = repo_with(&[("a.py", "x = 1\n")]);
    let p = local.path();
    assert_eq!(
        branch::default_branch(p).as_deref(),
        Some("refs/heads/main")
    );
    git(p, &["branch", "-q", "-m", "master"]);
    assert_eq!(
        branch::default_branch(p).as_deref(),
        Some("refs/heads/master")
    );
    git(p, &["branch", "-q", "main"]);
    assert_eq!(
        branch::default_branch(p).as_deref(),
        Some("refs/heads/main"),
        "main outranks master"
    );
    git(p, &["branch", "-q", "-D", "main"]);
    git(p, &["branch", "-q", "-m", "develop"]);
    assert_eq!(branch::default_branch(p), None);
}

#[test]
fn the_commits_a_branch_has_that_the_default_branch_does_not() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    let unique =
        |name: &str, shown| branch::unique_commits(p, name, "refs/heads/main", shown).unwrap();
    let subjects =
        |found: &Unique| -> Vec<String> { found.commits.iter().map(|(_, s)| s.clone()).collect() };
    let notes = unique("local-only", 30);
    assert_eq!(notes.total, 3);
    assert_eq!(
        subjects(&notes),
        ["docs: note 3", "docs: note 2", "docs: note 1"],
        "newest first"
    );
    let newest = &notes.commits[0].0;
    assert_eq!(newest.len(), 7);
    assert!(tip(p, "local-only").starts_with(newest.as_str()));
    let capped = unique("local-only", 2);
    assert_eq!(
        (capped.total, subjects(&capped)),
        (
            3,
            vec!["docs: note 3".to_string(), "docs: note 2".to_string()]
        )
    );
    assert_eq!(unique("merged", 30), Unique::default());
    assert_eq!(unique("feat/both", 30).total, 3);
    let against_the_remote =
        branch::unique_commits(p, "feat/ahead", "refs/remotes/origin/feat/ahead", 30).unwrap();
    assert_eq!(against_the_remote.total, 2);
    assert!(branch::unique_commits(p, "no-such-branch", "refs/heads/main", 30).is_err());
}

#[test]
fn switching_checks_out_the_branch() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    branch::switch(p, "local-only").unwrap();
    assert_eq!(current(p), "local-only");
    assert_eq!(
        std::fs::read_to_string(p.join("notes.md")).unwrap(),
        "note 3\n"
    );
    branch::switch(p, "feat/ahead").unwrap();
    assert_eq!(current(p), "feat/ahead");
    assert!(!p.join("notes.md").exists());
}

#[test]
fn switching_is_refused_when_local_changes_would_be_overwritten() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    write(p, "notes.md", "mine, not committed\n");
    let before = everything(p);
    let err = branch::switch(p, "local-only").unwrap_err();
    assert!(err.0.contains("notes.md"), "git names the file: {err}");
    assert_eq!(current(p), "main");
    assert!(before == everything(p), "nothing changed");

    std::fs::remove_file(p.join("notes.md")).unwrap();
    git(p, &["switch", "-q", "-c", "edits-app"]);
    commit_file(p, "app.py", "x = 5\n", "fix: five");
    git(p, &["switch", "-q", "main"]);
    write(p, "app.py", "x = 9\n");
    let before = everything(p);
    let err = branch::switch(p, "edits-app").unwrap_err();
    assert!(err.0.contains("app.py"), "git names the file: {err}");
    assert_eq!(current(p), "main");
    assert_eq!(
        std::fs::read_to_string(p.join("app.py")).unwrap(),
        "x = 9\n",
        "the edit is kept"
    );
    assert!(before == everything(p), "nothing changed");
}

#[test]
fn switching_to_a_branch_another_worktree_has_is_refused_by_git() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    let elsewhere = worktree_on(&remoted, "local-only");
    let head = std::fs::read(p.join(".git/HEAD")).unwrap();
    let err = branch::switch(p, "local-only").unwrap_err();
    assert!(
        err.0.contains("local-only") && err.0.contains(elsewhere.to_str().unwrap()),
        "git names the branch and the worktree: {err}"
    );
    assert_eq!(current(p), "main");
    assert_eq!(std::fs::read(p.join(".git/HEAD")).unwrap(), head);
}

#[test]
fn switching_never_guesses_a_branch_from_a_remote_one() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    git(p, &["push", "-q", "origin", "main:only-on-the-remote"]);
    let before = names(p);
    assert!(branch::switch(p, "only-on-the-remote").is_err());
    assert!(branch::switch(p, "no-such-branch").is_err());
    assert_eq!((names(p), current(p)), (before, "main".to_string()));
}

#[test]
fn creating_a_branch_starts_it_at_head_and_switches_to_it() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    git(p, &["switch", "-q", "local-only"]);
    write(p, "app.py", "x = 2\n");
    branch::create(p, "feat/new-one").unwrap();
    assert_eq!(current(p), "feat/new-one");
    assert_eq!(tip(p, "feat/new-one"), tip(p, "local-only"));
    assert_eq!(
        std::fs::read_to_string(p.join("app.py")).unwrap(),
        "x = 2\n",
        "what is not committed comes along"
    );
    assert_eq!(named(p, "feat/new-one").upstream, None);
}

#[test]
fn a_name_git_would_not_take_creates_nothing() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    let before = everything(p);
    for name in [
        "two words",
        "-dash",
        "--detach",
        "a..b",
        "HEAD",
        "x.lock",
        "ends/",
        "wild*card",
        "@{-1}",
        "tilde~1",
        "",
    ] {
        let err = branch::create(p, name).unwrap_err();
        assert!(
            err.0
                .ends_with("is not a valid branch name: nothing created"),
            "{name:?}: {err}"
        );
    }
    assert_eq!(
        branch::create(p, "two words").unwrap_err().0,
        "\"two words\" is not a valid branch name: nothing created"
    );
    assert!(before == everything(p));
    assert_eq!(current(p), "main");
}

#[test]
fn a_name_that_exists_creates_nothing() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    let before = everything(p);
    for name in ["local-only", "main", "feat/both"] {
        assert_eq!(
            branch::create(p, name).unwrap_err().0,
            format!("a branch named {name} already exists: nothing created")
        );
    }
    assert!(before == everything(p));
}

#[test]
fn the_first_branch_of_a_repository_with_no_commits_can_be_renamed_by_creating() {
    let unborn = TempDir::new("branch-create-unborn");
    let p = unborn.path();
    git(p, &["init", "-q"]);
    branch::create(p, "trunk").unwrap();
    assert_eq!(git(p, &["symbolic-ref", "HEAD"]).trim(), "refs/heads/trunk");
}

#[test]
fn merging_a_branch_that_is_ahead_fast_forwards() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    assert_eq!(branch::merge(p, "local-only").unwrap(), Merged::FastForward);
    assert_eq!(tip(p, "main"), tip(p, "local-only"));
    assert_eq!(current(p), "main");
    assert_eq!(
        branch::merge(p, "local-only").unwrap(),
        Merged::AlreadyUpToDate
    );
    assert_eq!(branch::merge(p, "merged").unwrap(), Merged::AlreadyUpToDate);
}

#[test]
fn merging_a_branch_that_diverged_makes_a_merge_commit_without_an_editor() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    commit_file(p, "app.py", "x = 2\n", "fix: main moves on");
    let ours = tip(p, "main");
    assert_eq!(branch::merge(p, "local-only").unwrap(), Merged::Commit);
    assert_eq!(
        git(p, &["log", "-1", "--format=%P"]).trim(),
        format!("{ours} {}", tip(p, "local-only"))
    );
    assert_eq!(
        git(p, &["log", "-1", "--format=%s"]).trim(),
        "Merge branch 'local-only'"
    );
    assert_eq!(fingerprint(p).1, "", "nothing is left to commit");
    assert_eq!(
        std::fs::read_to_string(p.join("notes.md")).unwrap(),
        "note 3\n"
    );
}

#[test]
fn a_merge_that_conflicts_is_left_as_git_left_it() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    commit_file(p, "notes.md", "the note on main\n", "docs: a note on main");
    let ours = tip(p, "main");
    assert_eq!(
        branch::merge(p, "local-only").unwrap(),
        Merged::Conflicts(1)
    );
    assert_eq!(tip(p, "main"), ours, "no commit was made");
    assert_eq!(tip(p, "MERGE_HEAD"), tip(p, "local-only"));
    assert_eq!(fingerprint(p).1, "AA notes.md\n");
    let marked = std::fs::read_to_string(p.join("notes.md")).unwrap();
    assert!(
        marked.contains("<<<<<<<") && marked.contains("the note on main"),
        "{marked}"
    );

    let stopped = everything(p);
    let err = branch::merge(p, "feat/ahead").unwrap_err();
    assert!(
        !err.0.trim().is_empty(),
        "a merge during a merge fails in git's own words"
    );
    assert!(stopped == everything(p), "and is not taken for a conflict");
}

#[test]
fn a_merge_a_hook_refuses_to_commit_is_left_unfinished_as_git_left_it() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    commit_file(p, "app.py", "x = 2\n", "fix: main moves on");
    hook(
        p,
        "pre-merge-commit",
        "#!/bin/sh\necho 'lint: merges need a ticket' >&2\nexit 1\n",
    );
    let ours = tip(p, "main");
    let Merged::Unfinished(why) = branch::merge(p, "local-only").unwrap() else {
        panic!("the hook lets no merge commit through");
    };
    assert!(why.contains("lint: merges need a ticket"), "{why}");
    assert_eq!(tip(p, "main"), ours, "no commit was made");
    assert_eq!(tip(p, "MERGE_HEAD"), tip(p, "local-only"));
    assert_eq!(
        fingerprint(p).1,
        "A  notes.md\n",
        "what the merge brought is staged, for the user to commit or abort"
    );
}

#[test]
fn a_merge_git_refuses_leaves_the_branch_the_index_and_the_files_alone() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    write(p, "notes.md", "mine, not committed\n");
    let (mut git_dir, status) = fingerprint(p);
    let tree = tree_bytes(p);
    let err = branch::merge(p, "local-only").unwrap_err();
    assert!(err.0.contains("notes.md"), "git names the file: {err}");
    let (mut after, status_after) = fingerprint(p);
    assert_ne!(
        git_dir.remove("ORIG_HEAD"),
        after.remove("ORIG_HEAD"),
        "git itself notes where HEAD was before it tries"
    );
    assert_eq!(tip(p, "ORIG_HEAD"), tip(p, "main"));
    assert!(git_dir == after, "nothing else in .git changed");
    assert_eq!(status, status_after);
    assert_eq!(tree, tree_bytes(p));
}

#[test]
fn a_branch_does_not_merge_into_itself() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    let before = everything(p);
    assert_eq!(
        branch::merge(p, "main").unwrap_err().0,
        "main is the current branch: a branch does not merge into itself"
    );
    assert_eq!(
        branch::merge(p, "no-such-branch").unwrap_err().0,
        "there is no branch named no-such-branch"
    );
    assert!(before == everything(p));
}

#[test]
fn a_name_that_is_also_a_tag_is_not_merged_by_guessing() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    git(p, &["tag", "local-only", "feat/ahead"]);
    let before = everything(p);
    let err = branch::merge(p, "local-only").unwrap_err();
    assert!(err.0.contains("also names another ref"), "{err}");
    assert!(before == everything(p));
}

fn asked(root: &Path, name: &str) -> Deletion {
    branch::deletion(root, name).unwrap()
}

#[test]
fn deleting_a_merged_branch_loses_nothing() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    let merged = asked(p, "merged");
    assert_eq!(
        merged,
        Deletion {
            branch: "merged".into(),
            tip: tip(p, "main"),
            only_here: 0
        }
    );
    branch::delete(p, &merged).unwrap();
    assert_eq!(names(p), ["feat/ahead", "feat/both", "local-only", "main"]);
    assert_eq!(current(p), "main");
}

#[test]
fn a_squash_merged_branch_whose_tip_is_still_on_the_remote_loses_nothing() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    git(p, &["switch", "-q", "-c", "feat/squashed"]);
    commit_file(p, "s.py", "s = 1\n", "feat: one half");
    commit_file(p, "s.py", "s = 2\n", "feat: the other half");
    git(p, &["push", "-q", "-u", "origin", "feat/squashed"]);
    git(p, &["switch", "-q", "main"]);
    git(p, &["merge", "-q", "--squash", "feat/squashed"]);
    git(p, &["commit", "-q", "-m", "feat: both halves (#7)"]);
    assert_eq!(
        git(
            p,
            &["branch", "--merged", "main", "--format=%(refname:short)"]
        )
        .lines()
        .filter(|b| *b == "feat/squashed")
        .count(),
        0,
        "git does not see a squash as a merge"
    );
    let squashed = asked(p, "feat/squashed");
    assert_eq!(squashed.only_here, 0, "origin/feat/squashed has the tip");
    branch::delete(p, &squashed).unwrap();
    assert!(!names(p).contains(&"feat/squashed".to_string()));
    assert_eq!(
        tip(p, "refs/remotes/origin/feat/squashed"),
        squashed.tip,
        "the remote-tracking branch is not touched"
    );
}

#[test]
fn a_squash_merged_branch_that_is_gone_from_the_remote_would_lose_its_commits() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    git(p, &["switch", "-q", "-c", "feat/squashed"]);
    commit_file(p, "s.py", "s = 1\n", "feat: one half");
    commit_file(p, "s.py", "s = 2\n", "feat: the other half");
    git(p, &["push", "-q", "-u", "origin", "feat/squashed"]);
    git(p, &["switch", "-q", "main"]);
    git(p, &["merge", "-q", "--squash", "feat/squashed"]);
    git(p, &["commit", "-q", "-m", "feat: both halves (#7)"]);
    git(
        p,
        &["update-ref", "-d", "refs/remotes/origin/feat/squashed"],
    );
    assert_eq!(asked(p, "feat/squashed").only_here, 2);
}

#[test]
fn the_commits_only_on_a_branch_are_counted_against_every_other_branch() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    assert_eq!(asked(p, "local-only").only_here, 3, "never pushed");
    assert_eq!(
        asked(p, "feat/ahead").only_here,
        2,
        "the first of its three commits is on origin/feat/ahead"
    );
    assert_eq!(
        asked(p, "feat/both").only_here,
        2,
        "what origin/feat/both has beyond it does not count"
    );
    assert_eq!(asked(p, "merged").only_here, 0);
    assert_eq!(branch::only_here(p, "main").unwrap(), 0);

    git(p, &["tag", "kept", "local-only"]);
    assert_eq!(
        asked(p, "local-only").only_here,
        3,
        "a tag is not a branch: the count is of commits on no other branch"
    );
    git(p, &["branch", "-q", "local-only-2", "local-only~1"]);
    assert_eq!(
        asked(p, "local-only").only_here,
        1,
        "another local branch holds the first two"
    );
    git(p, &["branch", "-q", "-f", "local-only-2", "local-only"]);
    assert_eq!(asked(p, "local-only").only_here, 0);
    assert_eq!(asked(p, "local-only-2").only_here, 0);
    git(p, &["branch", "-q", "-D", "local-only-2"]);
    git(p, &["push", "-q", "origin", "local-only:refs/heads/backup"]);
    assert_eq!(
        asked(p, "local-only").only_here,
        0,
        "a remote-tracking branch of another name holds them"
    );
}

#[test]
fn deleting_an_unpushed_branch_removes_it_and_only_it() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    let unpushed = asked(p, "local-only");
    assert_eq!(
        (unpushed.only_here, unpushed.tip.as_str()),
        (3, tip(p, "local-only").as_str())
    );
    let tree = tree_bytes(p);
    branch::delete(p, &unpushed).unwrap();
    assert_eq!(names(p), ["feat/ahead", "feat/both", "main", "merged"]);
    assert_eq!(tree, tree_bytes(p), "the working tree is not touched");
    assert_eq!(
        git(&remoted.origin, &["branch", "--format=%(refname:short)"]),
        "feat/ahead\nfeat/both\nmain\n",
        "nothing is deleted on the remote"
    );
}

#[test]
fn the_current_branch_is_never_deleted() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    let before = everything(p);
    assert_eq!(
        branch::deletion(p, "main").unwrap_err().0,
        "main is the current branch: switch to another one before deleting it"
    );
    let forged = Deletion {
        branch: "main".into(),
        tip: tip(p, "main"),
        only_here: 0,
    };
    assert!(branch::delete(p, &forged)
        .unwrap_err()
        .0
        .contains("is the current branch"));
    assert!(before == everything(p));
}

#[test]
fn a_branch_checked_out_in_another_worktree_is_never_deleted() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    let elsewhere = worktree_on(&remoted, "local-only");
    let before = everything(p);
    let err = branch::deletion(p, "local-only").unwrap_err();
    assert!(
        err.0.starts_with("local-only is checked out in ")
            && err.0.ends_with("elsewhere: nothing deleted"),
        "{err}"
    );
    let forged = Deletion {
        branch: "local-only".into(),
        tip: tip(p, "local-only"),
        only_here: 3,
    };
    assert!(branch::delete(p, &forged).is_err());
    assert!(before == everything(p));
    assert_eq!(current(&elsewhere), "local-only");
}

#[test]
fn a_branch_that_changed_after_the_question_is_not_deleted() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    let seen = asked(p, "merged");
    assert_eq!(seen.only_here, 0);
    git(p, &["branch", "-q", "-f", "merged", "local-only"]);
    git(p, &["branch", "-q", "-D", "local-only"]);
    assert_eq!(
        branch::delete(p, &seen).unwrap_err().0,
        "merged changed since the question was asked: nothing deleted"
    );
    assert!(names(p).contains(&"merged".to_string()));

    let seen = asked(p, "merged");
    assert_eq!(seen.only_here, 3);
    git(p, &["branch", "-q", "holds-them-too", "merged"]);
    assert!(
        branch::delete(p, &seen).is_err(),
        "the same tip with another count is another question"
    );
    let safe = asked(p, "merged");
    assert_eq!(safe.only_here, 0);
    branch::delete(p, &safe).unwrap();
    assert!(!names(p).contains(&"merged".to_string()));
    assert!(branch::deletion(p, "merged").is_err(), "it is gone");
}

#[test]
fn a_name_with_unusual_characters_is_taken_as_that_one_branch() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    for name in ["feat/{x}+y@z#1!", "fïx/ünï-côdé", "feat/ahead-2", "x"] {
        branch::create(p, name).unwrap();
        assert_eq!(current(p), name);
        commit_file(p, "odd.py", &format!("{name}\n"), "feat: an odd name");
        assert_eq!(named(p, name).tip, tip(p, "HEAD"));
        branch::switch(p, "main").unwrap();
        assert_eq!(asked(p, name).only_here, 1, "{name}");
        let unique = branch::unique_commits(p, name, "refs/heads/main", 30).unwrap();
        assert_eq!(unique.total, 1, "{name}");
        assert_eq!(
            branch::merge(p, name).unwrap(),
            Merged::FastForward,
            "{name}"
        );
        let merged = asked(p, name);
        assert_eq!(merged.only_here, 0, "{name}");
        branch::delete(p, &merged).unwrap();
        assert!(!names(p).contains(&name.to_string()), "{name}");
    }
    assert_eq!(
        names(p),
        ["feat/ahead", "feat/both", "local-only", "main", "merged"]
    );
    assert_eq!(asked(p, "feat/ahead").only_here, 2);
}

#[test]
fn the_only_branch_of_a_detached_checkout_would_lose_every_commit() {
    let repo = repo_with(&[("a.py", "x = 1\n")]);
    let p = repo.path();
    commit_file(p, "a.py", "x = 2\n", "second");
    git(p, &["switch", "-q", "--detach"]);
    assert_eq!(
        asked(p, "main"),
        Deletion {
            branch: "main".into(),
            tip: tip(p, "HEAD"),
            only_here: 2
        }
    );
}

#[test]
fn reading_the_branches_writes_nothing_into_the_repository() {
    let remoted = branches_repo();
    let p = remoted.root.as_path();
    worktree_on(&remoted, "merged");
    write(p, "app.py", "x = 2\n");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    write(p, "app.py", "x = 3\n");
    let before = everything(p);
    let read = branch::list(p).unwrap();
    for b in &read.local {
        branch::unique_commits(p, &b.name, "refs/heads/main", 30).unwrap();
        branch::only_here(p, &b.name).unwrap();
        let _ = branch::deletion(p, &b.name);
        branch::find(p, &b.name).unwrap();
    }
    branch::default_branch(p);
    for refused in ["two words", "main", "@{-1}"] {
        branch::create(p, refused).unwrap_err();
    }
    branch::merge(p, "main").unwrap_err();
    assert!(before == everything(p), "looking changed the repository");
}
