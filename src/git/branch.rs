use std::path::{Path, PathBuf};

use super::write::{as_typed, said};
use super::{run, GitError};
use crate::util;

const FIELDS: &str = "--format=%(refname)%00%(objectname)%00%(upstream)%00%(upstream:track,nobracket)%00%(HEAD)%00%(worktreepath)";
const LOCAL: &str = "refs/heads/";
const REMOTE: &str = "refs/remotes/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upstream {
    pub name: String,
    pub ahead: usize,
    pub behind: usize,
    pub gone: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub tip: String,
    pub current: bool,
    pub elsewhere: Option<PathBuf>,
    pub upstream: Option<Upstream>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub branches: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Branches {
    pub local: Vec<Branch>,
    pub remotes: Vec<Remote>,
    pub default: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Unique {
    pub total: usize,
    pub commits: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deletion {
    pub branch: String,
    pub tip: String,
    pub only_here: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Merged {
    AlreadyUpToDate,
    FastForward,
    Commit,
    Conflicts(usize),
    Unfinished(String),
}

pub fn short_name(refname: &str) -> &str {
    refname
        .strip_prefix(LOCAL)
        .or_else(|| refname.strip_prefix(REMOTE))
        .unwrap_or(refname)
}

pub fn short_sha(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

fn count_after(track: &str, word: &str) -> usize {
    track
        .split(", ")
        .find_map(|part| part.strip_prefix(word))
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or(0)
}

fn parse_branch(line: &str) -> Option<Branch> {
    let mut fields = line.split('\0');
    let name = fields.next()?.strip_prefix(LOCAL)?.to_string();
    let tip = fields.next()?.to_string();
    let upstream = fields.next()?;
    let track = fields.next()?;
    let current = fields.next()? == "*";
    let worktree = fields.next().unwrap_or_default();
    Some(Branch {
        name,
        tip,
        current,
        elsewhere: (!current && !worktree.is_empty()).then(|| PathBuf::from(worktree)),
        upstream: (!upstream.is_empty()).then(|| Upstream {
            name: short_name(upstream).to_string(),
            ahead: count_after(track, "ahead "),
            behind: count_after(track, "behind "),
            gone: track == "gone",
        }),
    })
}

pub fn parse_branches(out: &str) -> Vec<Branch> {
    out.lines().filter_map(parse_branch).collect()
}

fn exists(root: &Path, refname: &str) -> bool {
    run(root, &["show-ref", "--verify", "--quiet", refname]).is_ok()
}

fn origin_head(root: &Path) -> Option<String> {
    let target = run(root, &["symbolic-ref", "-q", "refs/remotes/origin/HEAD"]).ok()?;
    let name = target.trim().strip_prefix("refs/remotes/origin/")?;
    Some(name.to_string())
}

pub fn default_branch(root: &Path) -> Option<String> {
    let mut names: Vec<String> = origin_head(root).into_iter().collect();
    names.extend(["main".to_string(), "master".to_string()]);
    names
        .iter()
        .flat_map(|name| [format!("{LOCAL}{name}"), format!("{REMOTE}origin/{name}")])
        .find(|refname| exists(root, refname))
}

fn remotes(root: &Path) -> Result<Vec<Remote>, GitError> {
    let names = run(root, &["remote"])?;
    let refs = run(
        root,
        &["for-each-ref", "--format=%(refname)", "refs/remotes"],
    )?;
    Ok(names
        .lines()
        .filter(|name| !name.is_empty())
        .map(|name| {
            let prefix = format!("{REMOTE}{name}/");
            let branches = refs
                .lines()
                .filter_map(|refname| refname.strip_prefix(&prefix))
                .filter(|branch| *branch != "HEAD")
                .count();
            Remote {
                name: name.to_string(),
                branches,
            }
        })
        .collect())
}

pub fn list(root: &Path) -> Result<Branches, GitError> {
    let out = run(root, &["for-each-ref", FIELDS, "refs/heads"])?;
    Ok(Branches {
        local: parse_branches(&out),
        remotes: remotes(root)?,
        default: default_branch(root),
    })
}

pub fn find(root: &Path, name: &str) -> Result<Branch, GitError> {
    let out = run(root, &["for-each-ref", FIELDS, &format!("{LOCAL}{name}")])?;
    parse_branches(&out)
        .into_iter()
        .find(|branch| branch.name == name)
        .ok_or_else(|| GitError(format!("there is no branch named {name}")))
}

fn count(root: &Path, revisions: &[&str]) -> Result<usize, GitError> {
    let args = [&["rev-list", "--count"], revisions, &["--"]].concat();
    run(root, &args)?
        .trim()
        .parse()
        .map_err(|_| GitError("git rev-list printed no count".into()))
}

pub fn unique_commits(
    root: &Path,
    branch: &str,
    base: &str,
    shown: usize,
) -> Result<Unique, GitError> {
    let range = format!("{base}..{LOCAL}{branch}");
    let total = count(root, &[&range])?;
    let most = format!("-n{shown}");
    let out = run(root, &["log", &most, "--format=%H%x1f%s", &range, "--"])?;
    let commits = out
        .lines()
        .filter_map(|line| line.split_once('\u{1f}'))
        .map(|(sha, subject)| (short_sha(sha).to_string(), subject.to_string()))
        .collect();
    Ok(Unique { total, commits })
}

pub fn only_here(root: &Path, branch: &str) -> Result<usize, GitError> {
    count(
        root,
        &[
            &format!("{LOCAL}{branch}"),
            "--not",
            &format!("--exclude={branch}"),
            "--branches",
            "--remotes",
        ],
    )
}

fn acted_on(name: &str) -> Result<(), GitError> {
    if name.starts_with('-') {
        return Err(GitError(format!(
            "{name} starts with a dash: codeMap does not act on it"
        )));
    }
    Ok(())
}

fn typed(root: &Path, args: &[&str]) -> Result<(), GitError> {
    let output = as_typed(root)
        .args(args)
        .output()
        .map_err(|e| GitError(format!("git: {e}")))?;
    said(args[0], &output)
}

pub fn switch(root: &Path, name: &str) -> Result<(), GitError> {
    acted_on(name)?;
    typed(root, &["switch", "--no-guess", "--", name])
}

pub fn create(root: &Path, name: &str) -> Result<(), GitError> {
    let valid = run(root, &["check-ref-format", "--branch", name])
        .is_ok_and(|normal| normal.trim_end_matches('\n') == name);
    if !valid {
        return Err(GitError(format!(
            "{name:?} is not a valid branch name: nothing created"
        )));
    }
    if exists(root, &format!("{LOCAL}{name}")) {
        return Err(GitError(format!(
            "a branch named {name} already exists: nothing created"
        )));
    }
    typed(root, &["switch", "-c", name])
}

fn head(root: &Path) -> Option<String> {
    run(root, &["rev-parse", "-q", "--verify", "HEAD^{commit}"])
        .ok()
        .map(|sha| sha.trim().to_string())
}

fn merging(root: &Path) -> bool {
    run(root, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).is_ok()
}

fn conflicted(root: &Path) -> usize {
    run(root, &["diff", "--name-only", "--diff-filter=U", "-z"])
        .map(|out| out.split('\0').filter(|path| !path.is_empty()).count())
        .unwrap_or(0)
}

fn names_only_the_branch(root: &Path, branch: &Branch) -> Result<(), GitError> {
    let spelled = format!("{}^{{commit}}", branch.name);
    let resolved = run(root, &["rev-parse", "-q", "--verify", &spelled]).ok();
    if resolved.as_deref().map(str::trim) == Some(branch.tip.as_str()) {
        return Ok(());
    }
    Err(GitError(format!(
        "{} also names another ref: codeMap will not guess which one to merge",
        branch.name
    )))
}

pub fn merge(root: &Path, name: &str) -> Result<Merged, GitError> {
    acted_on(name)?;
    let branch = find(root, name)?;
    if branch.current {
        return Err(GitError(format!(
            "{name} is the current branch: a branch does not merge into itself"
        )));
    }
    names_only_the_branch(root, &branch)?;
    let before = head(root);
    let was_merging = merging(root);
    let output = as_typed(root)
        .env("GIT_MERGE_AUTOEDIT", "no")
        .args(["merge", "--no-edit", "--", name])
        .output()
        .map_err(|e| GitError(format!("git: {e}")))?;
    let refused = said("merge", &output);
    if let (Err(why), false, true) = (&refused, was_merging, merging(root)) {
        return Ok(match conflicted(root) {
            0 => Merged::Unfinished(why.0.clone()),
            conflicts => Merged::Conflicts(conflicts),
        });
    }
    refused?;
    let after = head(root);
    Ok(if after == before {
        Merged::AlreadyUpToDate
    } else if after.as_deref() == Some(branch.tip.as_str()) {
        Merged::FastForward
    } else {
        Merged::Commit
    })
}

pub fn deletion(root: &Path, name: &str) -> Result<Deletion, GitError> {
    acted_on(name)?;
    let branch = find(root, name)?;
    if branch.current {
        return Err(GitError(format!(
            "{name} is the current branch: switch to another one before deleting it"
        )));
    }
    if let Some(path) = &branch.elsewhere {
        return Err(GitError(format!(
            "{name} is checked out in {}: nothing deleted",
            util::tilde(path)
        )));
    }
    Ok(Deletion {
        branch: branch.name,
        tip: branch.tip,
        only_here: only_here(root, name)?,
    })
}

pub fn delete(root: &Path, asked: &Deletion) -> Result<(), GitError> {
    if deletion(root, &asked.branch)? != *asked {
        return Err(GitError(format!(
            "{} changed since the question was asked: nothing deleted",
            asked.branch
        )));
    }
    typed(root, &["branch", "-D", "--", &asked.branch])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(fields: [&str; 6]) -> String {
        fields.join("\0")
    }

    #[test]
    fn a_branch_line_gives_its_name_tip_upstream_and_where_it_is_checked_out() {
        let out = [
            line([
                "refs/heads/feat/x",
                "a384f27ac8b1",
                "refs/remotes/origin/feat/x",
                "ahead 2, behind 1",
                " ",
                "",
            ]),
            line([
                "refs/heads/main",
                "6c4772177b2b",
                "refs/remotes/origin/main",
                "",
                "*",
                "/code/repo",
            ]),
            line(["refs/heads/topic", "6c4772177b2b", "", "", " ", "/code/wt"]),
            line([
                "refs/heads/old",
                "1111111",
                "refs/remotes/origin/old",
                "gone",
                " ",
                "",
            ]),
            line([
                "refs/heads/local",
                "2222222",
                "refs/heads/main",
                "behind 12",
                " ",
                "",
            ]),
            "refs/tags/v1\0abc\0\0\0 \0".to_string(),
            "garbage".to_string(),
        ]
        .join("\n");
        let branches = parse_branches(&out);
        let names: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["feat/x", "main", "topic", "old", "local"]);
        let upstream = |at: usize| branches[at].upstream.clone().unwrap();
        assert_eq!(
            upstream(0),
            Upstream {
                name: "origin/feat/x".into(),
                ahead: 2,
                behind: 1,
                gone: false
            }
        );
        assert_eq!((upstream(1).ahead, upstream(1).behind), (0, 0));
        assert!(branches[1].current && branches[1].elsewhere.is_none());
        assert_eq!(branches[2].upstream, None);
        assert_eq!(branches[2].elsewhere, Some(PathBuf::from("/code/wt")));
        assert!(upstream(3).gone);
        assert_eq!(
            (upstream(4).name.as_str(), upstream(4).behind),
            ("main", 12)
        );
    }

    #[test]
    fn short_names_drop_the_ref_namespace() {
        assert_eq!(short_name("refs/heads/feat/x"), "feat/x");
        assert_eq!(short_name("refs/remotes/origin/main"), "origin/main");
        assert_eq!(short_name("main"), "main");
        assert_eq!(short_sha("a384f27ac8b1"), "a384f27");
        assert_eq!(short_sha("a38"), "a38");
    }

    #[test]
    fn a_name_that_starts_with_a_dash_is_never_passed_to_git() {
        let nowhere = Path::new("/codemap-test/not-a-repository");
        for refused in [
            switch(nowhere, "--detach").map(drop),
            merge(nowhere, "-s").map(drop),
            deletion(nowhere, "-D").map(drop),
        ] {
            assert!(refused.unwrap_err().0.contains("starts with a dash"));
        }
    }
}
