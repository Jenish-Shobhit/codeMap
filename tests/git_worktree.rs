mod common;

use std::path::Path;

use codemap::git::branch::Upstream;
use codemap::git::worktree::{self, Change, Changes, Checkout, Uncommitted};
use codemap::git::{self, FileStatus};
use codemap::store::Store;
use codemap::worktrees::{self, Entry};
use common::*;
use serde_json::json;

fn read(root: &Path, state: &TempDir) -> Vec<Checkout> {
    let store = Store::new(state.path());
    worktree::read(root, &|info| store.shadow(info).dir).unwrap()
}

fn at<'a>(checkouts: &'a [Checkout], folder: &Path) -> &'a Checkout {
    checkouts
        .iter()
        .find(|checkout| checkout.path == folder)
        .unwrap_or_else(|| panic!("{} is not listed", folder.display()))
}

fn uncommitted(checkout: &Checkout) -> &Uncommitted {
    match &checkout.changes {
        Changes::Read(uncommitted) => uncommitted,
        other => panic!("{} was not read: {other:?}", checkout.path.display()),
    }
}

fn shown(changes: &[Change]) -> Vec<String> {
    changes
        .iter()
        .map(|c| format!("{} {} +{} -{}", c.status.letter(), c.path, c.adds, c.dels))
        .collect()
}

fn summary(checkout: &Checkout) -> (usize, usize, usize) {
    let all = uncommitted(checkout);
    (all.files(), all.adds(), all.dels())
}

fn scratch_copies(state: &TempDir) -> Vec<String> {
    let repos = state.path().join("repos");
    let Ok(folders) = std::fs::read_dir(repos) else {
        return Vec::new();
    };
    folders
        .flatten()
        .flat_map(|folder| std::fs::read_dir(folder.path()).unwrap().flatten())
        .map(|file| file.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("index"))
        .collect()
}

#[test]
fn every_worktree_is_listed_with_its_branch_or_detached_head() {
    let repo = worktrees_repo();
    let state = TempDir::new("worktree-list-state");
    let checkouts = read(&repo.main, &state);
    let paths: Vec<&Path> = checkouts.iter().map(|c| c.path.as_path()).collect();
    assert_eq!(
        paths,
        [&repo.main, &repo.docs, &repo.gone, &repo.pinned],
        "the main worktree, then the others by path"
    );
    let head = |revision: &str| git(&repo.main, &["rev-parse", revision]).trim().to_string();
    let main = at(&checkouts, &repo.main);
    assert_eq!(main.branch.as_deref(), Some("main"));
    assert_eq!(main.head, Some(head("refs/heads/main")));
    assert_eq!(main.head_label(), "main");
    let docs = at(&checkouts, &repo.docs);
    assert_eq!(docs.branch.as_deref(), Some("docs"));
    assert_eq!(docs.head, Some(head("refs/heads/docs")));
    let pinned = at(&checkouts, &repo.pinned);
    assert_eq!(pinned.branch, None, "a detached head has no branch");
    assert_eq!(pinned.head, Some(head("refs/heads/main")));
    assert_eq!(pinned.head_label(), head("refs/heads/main")[..7]);
    assert_eq!(at(&checkouts, &repo.gone).branch.as_deref(), Some("gone"));
}

#[test]
fn a_worktree_tells_its_upstream_and_how_far_apart_they_are() {
    let repo = worktrees_repo();
    let state = TempDir::new("worktree-upstream-state");
    let checkouts = read(&repo.main, &state);
    let tracking = |name: &str, ahead: usize| {
        Some(Upstream {
            name: name.into(),
            ahead,
            behind: 0,
            gone: false,
        })
    };
    assert_eq!(
        at(&checkouts, &repo.main).upstream,
        tracking("origin/main", 0)
    );
    assert_eq!(
        at(&checkouts, &repo.docs).upstream,
        tracking("origin/docs", 1)
    );
    assert_eq!(at(&checkouts, &repo.pinned).upstream, None);
    assert_eq!(at(&checkouts, &repo.gone).upstream, None);
    let seen_from_docs = read(&repo.docs, &state);
    assert_eq!(seen_from_docs, checkouts, "the same list from any worktree");
}

#[test]
fn a_locked_worktree_and_one_whose_folder_is_missing_say_so() {
    let repo = worktrees_repo();
    let state = TempDir::new("worktree-states-state");
    let checkouts = read(&repo.main, &state);
    let pinned = at(&checkouts, &repo.pinned);
    assert_eq!(pinned.locked.as_deref(), Some("on a removable disk"));
    assert_eq!(pinned.prunable, None);
    assert!(!pinned.is_missing());
    let gone = at(&checkouts, &repo.gone);
    assert!(gone.is_missing());
    assert!(gone.prunable.is_some(), "git calls it prunable");
    assert_eq!(gone.locked, None);
    for present in [&repo.main, &repo.docs] {
        let checkout = at(&checkouts, present);
        assert_eq!((&checkout.locked, &checkout.prunable), (&None, &None));
        assert!(!checkout.is_missing());
    }
    git(
        &repo.main,
        &["worktree", "lock", repo.docs.to_str().unwrap()],
    );
    let checkouts = read(&repo.main, &state);
    assert_eq!(at(&checkouts, &repo.docs).locked.as_deref(), Some(""));
}

#[test]
fn each_worktree_has_its_own_uncommitted_changes() {
    let repo = worktrees_repo();
    let state = TempDir::new("worktree-changes-state");
    let checkouts = read(&repo.main, &state);
    let main = uncommitted(at(&checkouts, &repo.main));
    assert_eq!(shown(&main.staged), ["M app.py +1 -1"]);
    assert_eq!(
        shown(&main.unstaged),
        ["M app.py +1 -1", "? notes.md +3 -0"]
    );
    assert_eq!(main.unstaged[1].status, FileStatus::Untracked);
    assert_eq!(
        summary(at(&checkouts, &repo.main)),
        (2, 5, 2),
        "a partly staged file is one file"
    );
    let docs = uncommitted(at(&checkouts, &repo.docs));
    assert_eq!(shown(&docs.staged), ["A examples/first.md +2 -0"]);
    assert_eq!(shown(&docs.unstaged), ["M guide.md +2 -0"]);
    assert_eq!(summary(at(&checkouts, &repo.docs)), (2, 4, 0));
    let pinned = at(&checkouts, &repo.pinned);
    assert!(uncommitted(pinned).is_clean());
    assert_eq!(summary(pinned), (0, 0, 0));
    assert_eq!(at(&checkouts, &repo.gone).changes, Changes::Missing);
}

#[test]
fn each_summary_is_what_the_status_of_that_worktree_reads() {
    let repo = worktrees_repo();
    let state = TempDir::new("worktree-same-state");
    let store = Store::new(state.path());
    let checkouts = read(&repo.main, &state);
    for folder in repo.present() {
        let info = git::discover(folder).unwrap();
        let status = git::status(&info, &store.shadow(&info).dir).unwrap();
        let paths = |files: &[git::FileDiff]| -> Vec<String> {
            files.iter().map(|f| f.path.clone()).collect()
        };
        let listed = |changes: &[Change]| -> Vec<String> {
            changes.iter().map(|c| c.path.clone()).collect()
        };
        let read = uncommitted(at(&checkouts, folder));
        assert_eq!(listed(&read.staged), paths(&status.staged), "{folder:?}");
        assert_eq!(
            listed(&read.unstaged),
            paths(&status.unstaged),
            "{folder:?}"
        );
    }
}

#[test]
fn a_folder_that_is_no_longer_a_worktree_counts_as_missing() {
    let repo = worktrees_repo();
    let state = TempDir::new("worktree-replaced-state");
    write(&repo.gone, "plain.txt", "not a checkout\n");
    let checkouts = read(&repo.main, &state);
    assert!(at(&checkouts, &repo.gone).is_missing());
    let inside = repo.main.join("nested");
    git(
        &repo.main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "nested",
            inside.to_str().unwrap(),
        ],
    );
    std::fs::remove_dir_all(&inside).unwrap();
    write(&inside, "plain.txt", "a folder of the main worktree now\n");
    let checkouts = read(&repo.main, &state);
    assert!(
        at(&checkouts, &inside).is_missing(),
        "the main worktree around it is not read in its place"
    );
}

#[test]
fn a_bare_repository_lists_only_its_worktrees() {
    let tmp = TempDir::new("worktree-bare");
    let state = TempDir::new("worktree-bare-state");
    let seed = tmp.path().join("seed");
    std::fs::create_dir_all(&seed).unwrap();
    git(&seed, &["init", "-q"]);
    commit_file(&seed, "a.py", "x = 1\n", "initial");
    let bare = tmp.path().join("bare.git");
    git(
        tmp.path(),
        &["clone", "-q", "--bare", "seed", bare.to_str().unwrap()],
    );
    let checked_out = tmp.path().join("checked-out");
    git(
        &bare,
        &[
            "worktree",
            "add",
            "-q",
            checked_out.to_str().unwrap(),
            "main",
        ],
    );
    write(&checked_out, "a.py", "x = 2\n");
    assert_eq!(git::worktrees(&checked_out).unwrap().len(), 2);
    let checkouts = read(&checked_out, &state);
    assert_eq!(
        checkouts.len(),
        1,
        "the bare repository has no files to show"
    );
    assert_eq!(checkouts[0].path, checked_out);
    assert_eq!(
        shown(&uncommitted(&checkouts[0]).unstaged),
        ["M a.py +1 -1"]
    );
}

#[test]
fn a_repository_with_no_commits_is_one_worktree_without_a_head() {
    let unborn = TempDir::new("worktree-unborn");
    let state = TempDir::new("worktree-unborn-state");
    git(unborn.path(), &["init", "-q"]);
    write(unborn.path(), "first.py", "x = 1\n");
    let checkouts = read(unborn.path(), &state);
    assert_eq!(checkouts.len(), 1);
    assert_eq!(checkouts[0].head, None);
    assert_eq!(checkouts[0].branch.as_deref(), Some("main"));
    assert_eq!(
        shown(&uncommitted(&checkouts[0]).unstaged),
        ["? first.py +1 -0"]
    );
}

#[test]
fn outside_a_repository_there_is_nothing_to_list() {
    let plain = TempDir::new("worktree-plain");
    let state = TempDir::new("worktree-plain-state");
    let store = Store::new(state.path());
    let read = worktree::read(plain.path(), &|info| store.shadow(info).dir);
    assert!(read.unwrap_err().0.contains("not a git repository"));
}

fn names(entry: &Entry) -> Vec<&str> {
    entry.agents.iter().map(|a| a.name.as_str()).collect()
}

#[test]
fn agents_are_placed_in_the_worktree_they_work_in() {
    let repo = worktrees_repo();
    let state = TempDir::new("worktree-agents-state");
    let checkouts = read(&repo.main, &state);
    let inside_main = repo.main.join("nested");
    git(
        &repo.main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "nested",
            inside_main.to_str().unwrap(),
        ],
    );
    let with_nested = read(&repo.main, &state);
    let listed = [
        listed_agent("w1:p1", "claude", "idle", &repo.main),
        listed_agent("w2:p1", "codex", "working", &repo.docs.join("examples")),
        listed_agent("w2:p2", "pi", "idle", &repo.docs),
        listed_agent("w3:p1", "amp", "working", repo.base()),
        listed_agent("w3:p2", "droid", "idle", &repo.gone),
        listed_agent("w4:p1", "opencode", "blocked", &inside_main),
        json!({"pane_id": "w5:p1", "agent": "gemini", "agent_status": "idle",
               "cwd": "/", "foreground_cwd": repo.pinned.display().to_string()}),
    ];
    let entries = worktrees::with_agents(checkouts, &listed);
    let entry = |folder: &Path| entries.iter().find(|e| e.checkout.path == folder).unwrap();
    assert_eq!(names(entry(&repo.main)), ["claude", "opencode"]);
    assert_eq!(
        names(entry(&repo.docs)),
        ["codex", "pi"],
        "one in a subfolder, one at the root"
    );
    assert_eq!(
        names(entry(&repo.pinned)),
        ["gemini"],
        "the foreground folder"
    );
    assert_eq!(
        names(entry(&repo.gone)),
        ["droid"],
        "still where herdr says it is"
    );
    let placed: usize = entries.iter().map(|e| e.agents.len()).sum();
    assert_eq!(placed, 6, "the one outside every worktree is in none");
    assert_eq!(entry(&repo.docs).adopted().unwrap().name, "codex");
    assert_eq!(entry(&repo.main).adopted().unwrap().name, "claude");

    let entries = worktrees::with_agents(with_nested, &listed);
    let entry = |folder: &Path| entries.iter().find(|e| e.checkout.path == folder).unwrap();
    assert_eq!(names(entry(&repo.main)), ["claude"]);
    assert_eq!(
        names(entry(&inside_main)),
        ["opencode"],
        "the innermost worktree, not the one around it"
    );
}

#[test]
fn reading_the_worktrees_writes_nothing_in_any_of_them() {
    let repo = worktrees_repo();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    write(&repo.main, "app.py", "x = 3\n");
    write(&repo.docs, "guide.md", "# Guide\n\nRun it.\n");
    write(&repo.pinned, "app.py", "x = 1\n");
    let before = untouched(&repo);
    let state = TempDir::new("worktree-quiet-state");
    let store = Store::new(state.path());
    for from in repo.present() {
        let listing = worktrees::read(from, &store, None).unwrap();
        assert_eq!(listing.entries.len(), 4);
        assert!(!listing.agents_known, "no herdr was asked");
        assert!(listing.entries.iter().all(|e| e.agents.is_empty()));
    }
    assert_eq!(before, untouched(&repo), "a worktree changed while reading");
    assert_eq!(scratch_copies(&state), Vec::<String>::new());
    assert!(!repo.gone.exists(), "a missing folder is not made again");
}

#[test]
fn the_summaries_and_the_status_tab_can_read_one_worktree_at_once() {
    let repo = worktrees_repo();
    let state = TempDir::new("worktree-crowd-state");
    let store = Store::new(state.path());
    let before = untouched(&repo);
    let alone = read(&repo.main, &state);
    let info = git::discover(&repo.docs).unwrap();
    let scratch = store.shadow(&info).dir;
    let status_alone = git::status(&info, &scratch).unwrap();
    std::thread::scope(|readers| {
        for reader in 0..8 {
            let (repo, state, alone) = (&repo, &state, &alone);
            let (info, scratch, status_alone) = (&info, &scratch, &status_alone);
            readers.spawn(move || {
                for _ in 0..3 {
                    if reader % 2 == 0 {
                        assert_eq!(&read(&repo.main, state), alone);
                    } else {
                        assert_eq!(&git::status(info, scratch).unwrap(), status_alone);
                    }
                }
            });
        }
    });
    assert_eq!(scratch_copies(&state), Vec::<String>::new());
    assert_eq!(before, untouched(&repo));
}
