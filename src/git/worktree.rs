use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::branch::{self, short_sha, Upstream};
use super::{discover, status, worktrees, FileDiff, FileStatus, GitError, RepoInfo, Worktree};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    pub status: FileStatus,
    pub adds: usize,
    pub dels: usize,
    pub binary: bool,
}

impl Change {
    fn of(file: &FileDiff) -> Change {
        Change {
            path: file.path.clone(),
            status: file.status,
            adds: file.adds(),
            dels: file.dels(),
            binary: file.binary,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Uncommitted {
    pub staged: Vec<Change>,
    pub unstaged: Vec<Change>,
}

impl Uncommitted {
    fn both(&self) -> impl Iterator<Item = &Change> {
        self.staged.iter().chain(&self.unstaged)
    }

    pub fn is_clean(&self) -> bool {
        self.staged.is_empty() && self.unstaged.is_empty()
    }

    pub fn files(&self) -> usize {
        let paths: BTreeSet<&str> = self.both().map(|change| change.path.as_str()).collect();
        paths.len()
    }

    pub fn adds(&self) -> usize {
        self.both().map(|change| change.adds).sum()
    }

    pub fn dels(&self) -> usize {
        self.both().map(|change| change.dels).sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Changes {
    Read(Uncommitted),
    Missing,
    Unreadable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub upstream: Option<Upstream>,
    pub locked: Option<String>,
    pub prunable: Option<String>,
    pub changes: Changes,
}

impl Checkout {
    pub fn is_missing(&self) -> bool {
        self.changes == Changes::Missing
    }

    pub fn head_label(&self) -> String {
        match (&self.branch, &self.head) {
            (Some(branch), _) => branch.clone(),
            (None, Some(head)) => short_sha(head).to_string(),
            (None, None) => "no commits".into(),
        }
    }
}

pub type Scratch<'a> = &'a dyn Fn(&RepoInfo) -> PathBuf;

fn changes_in(folder: &Path, scratch: Scratch) -> Changes {
    if !folder.is_dir() {
        return Changes::Missing;
    }
    let info = match discover(folder) {
        Ok(info) if info.root == folder => info,
        Ok(_) | Err(None) => return Changes::Missing,
        Err(Some(e)) => return Changes::Unreadable(e.0),
    };
    match status(&info, &scratch(&info)) {
        Ok(read) => Changes::Read(Uncommitted {
            staged: read.staged.iter().map(Change::of).collect(),
            unstaged: read.unstaged.iter().map(Change::of).collect(),
        }),
        Err(e) => Changes::Unreadable(e.0),
    }
}

fn born(head: Option<String>) -> Option<String> {
    head.filter(|sha| sha.bytes().any(|digit| digit != b'0'))
}

fn checkout(
    listed: Worktree,
    upstreams: &BTreeMap<String, Option<Upstream>>,
    scratch: Scratch,
) -> Checkout {
    let path = listed.path.canonicalize().unwrap_or(listed.path);
    let upstream = listed
        .branch
        .as_ref()
        .and_then(|name| upstreams.get(name).cloned().flatten());
    Checkout {
        changes: changes_in(&path, scratch),
        path,
        head: born(listed.head),
        branch: listed.branch,
        upstream,
        locked: listed.locked,
        prunable: listed.prunable,
    }
}

pub fn read(root: &Path, scratch: Scratch) -> Result<Vec<Checkout>, GitError> {
    let listed = worktrees(root)?;
    let upstreams: BTreeMap<String, Option<Upstream>> = branch::local(root)
        .unwrap_or_default()
        .into_iter()
        .map(|branch| (branch.name, branch.upstream))
        .collect();
    Ok(listed
        .into_iter()
        .filter(|worktree| !worktree.bare)
        .map(|worktree| checkout(worktree, &upstreams, scratch))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(path: &str, adds: usize, dels: usize) -> Change {
        Change {
            path: path.into(),
            status: FileStatus::Modified,
            adds,
            dels,
            binary: false,
        }
    }

    #[test]
    fn a_file_in_both_lists_is_one_file_and_its_lines_add_up() {
        let partly_staged = Uncommitted {
            staged: vec![change("a.py", 1, 2), change("b.py", 3, 0)],
            unstaged: vec![change("a.py", 2, 0), change("c.py", 0, 4)],
        };
        assert_eq!(
            (
                partly_staged.files(),
                partly_staged.adds(),
                partly_staged.dels()
            ),
            (3, 6, 6)
        );
        assert!(!partly_staged.is_clean());
        let clean = Uncommitted::default();
        assert!(clean.is_clean());
        assert_eq!((clean.files(), clean.adds(), clean.dels()), (0, 0, 0));
    }

    #[test]
    fn a_checkout_is_named_by_its_branch_or_its_short_hash() {
        let on = |branch: Option<&str>, head: Option<&str>| Checkout {
            path: PathBuf::from("/code/repo"),
            head: born(head.map(str::to_string)),
            branch: branch.map(str::to_string),
            upstream: None,
            locked: None,
            prunable: None,
            changes: Changes::Missing,
        };
        assert_eq!(
            on(Some("feat/x"), Some("a384f27ac8b1")).head_label(),
            "feat/x"
        );
        assert_eq!(on(None, Some("a384f27ac8b1")).head_label(), "a384f27");
        let unborn = on(None, Some("0000000000000000000000000000000000000000"));
        assert_eq!(
            (unborn.head.as_ref(), unborn.head_label().as_str()),
            (None, "no commits")
        );
        assert!(unborn.is_missing());
    }
}
