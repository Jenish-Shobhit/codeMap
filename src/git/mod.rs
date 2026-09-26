//! Git through the `git` CLI. Every command that touches the user's repository
//! is read-only: `GIT_OPTIONAL_LOCKS=0` stops `git status`-style commands from
//! refreshing (writing) the index, and codeMorph never runs a command that
//! creates refs, stashes or index entries there. Checkpoints live in a shadow
//! repository in codeMorph's own state directory (see `store`).

pub mod diff;
pub mod graph;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub use diff::{DiffLine, FileDiff, FileStatus, Hunk, LineKind};
pub use graph::{Commit, GraphRow};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitError(pub String);

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GitError {}

/// A `git` command with a predictable, side-effect-free environment.
pub fn git_cmd(cwd: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat")
        .env("LC_ALL", "C")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .args(["-c", "core.quotePath=false", "-c", "color.ui=false"])
        .stdin(Stdio::null());
    cmd
}

pub fn run_cmd(mut cmd: Command) -> Result<Vec<u8>, GitError> {
    let output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| GitError(format!("git: {e}")))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(GitError(if err.is_empty() {
            format!("git exited with {}", output.status)
        } else {
            err
        }));
    }
    Ok(output.stdout)
}

pub fn run(cwd: &Path, args: &[&str]) -> Result<String, GitError> {
    let mut cmd = git_cmd(cwd);
    cmd.args(args);
    run_cmd(cmd).map(|out| String::from_utf8_lossy(&out).into_owned())
}

/// Facts about the repository around a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInfo {
    /// The worktree root (`--show-toplevel`).
    pub root: PathBuf,
    /// This worktree's git dir (`.git` or `.git/worktrees/<name>`).
    pub git_dir: PathBuf,
    /// The shared git dir holding objects and refs.
    pub common_dir: PathBuf,
    /// HEAD commit, None in an empty repository.
    pub head: Option<String>,
    /// Current branch name, None when detached or empty.
    pub branch: Option<String>,
}

impl RepoInfo {
    pub fn is_empty(&self) -> bool {
        self.head.is_none()
    }

    pub fn objects_dir(&self) -> PathBuf {
        self.common_dir.join("objects")
    }

    pub fn name(&self) -> String {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string())
    }
}

/// Find the repository containing `path`. Err(None) means "not a git repo".
pub fn discover(path: &Path) -> Result<RepoInfo, Option<GitError>> {
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().map(Path::to_path_buf).unwrap_or_default()
    };
    if !dir.is_dir() {
        return Err(Some(GitError(format!("{} does not exist", dir.display()))));
    }
    let out = match run(
        &dir,
        &[
            "rev-parse",
            "--show-toplevel",
            "--absolute-git-dir",
            "--git-common-dir",
        ],
    ) {
        Ok(out) => out,
        Err(err) => {
            if err.0.contains("not a git repository") {
                return Err(None);
            }
            return Err(Some(err));
        }
    };
    let mut lines = out.lines();
    let root = PathBuf::from(lines.next().unwrap_or_default());
    let git_dir = PathBuf::from(lines.next().unwrap_or_default());
    let common = lines.next().unwrap_or_default();
    if root.as_os_str().is_empty() {
        // A bare repository or inside .git: nothing to map.
        return Err(None);
    }
    let common_dir = {
        let p = PathBuf::from(common);
        if p.is_absolute() {
            p
        } else {
            dir.join(p)
        }
    };
    let common_dir = common_dir.canonicalize().unwrap_or(common_dir);
    let head = run(&root, &["rev-parse", "-q", "--verify", "HEAD^{commit}"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let branch = run(&root, &["symbolic-ref", "-q", "--short", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    Ok(RepoInfo {
        root,
        git_dir,
        common_dir,
        head,
        branch,
    })
}

/// Tracked plus untracked-but-not-ignored files, relative to the root.
pub fn list_files(root: &Path) -> Result<Vec<String>, GitError> {
    let mut cmd = git_cmd(root);
    cmd.args([
        "ls-files",
        "-z",
        "--cached",
        "--others",
        "--exclude-standard",
        "--deduplicate",
    ]);
    let out = run_cmd(cmd)?;
    let mut files: Vec<String> = out
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    files.sort();
    files.dedup();
    // Deleted-but-tracked files still show up in --cached; keep only real files.
    files.retain(|f| root.join(f).is_file());
    Ok(files)
}

/// Directories never worth walking outside a git repository.
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "__pycache__",
    ".venv",
    "venv",
    "dist",
    "build",
    ".mypy_cache",
    ".pytest_cache",
    ".tox",
    ".idea",
    ".next",
];

/// Walk a plain directory (not a git repo), capped at `limit` files.
pub fn walk_files(root: &Path, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries.into_iter().rev() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                if name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) {
                    continue;
                }
                stack.push(entry.path());
            } else if ft.is_file() {
                if let Ok(rel) = entry.path().strip_prefix(root) {
                    out.push(rel.to_string_lossy().into_owned());
                }
                if out.len() >= limit {
                    out.sort();
                    return out;
                }
            }
        }
    }
    out.sort();
    out
}

/// Parse `git diff` output between two tree-ish objects of the user's repo.
pub fn diff_trees(root: &Path, from: &str, to: &str) -> Result<Vec<FileDiff>, GitError> {
    let out = run(
        root,
        &[
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "-M",
            "-U3",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            from,
            to,
        ],
    )?;
    Ok(diff::parse(&out))
}

/// Working tree against HEAD (staged, unstaged and untracked), without
/// touching the user's index: git reads a private copy of it, so any stat
/// refresh lands in `scratch`, not in the repository.
pub fn diff_head(info: &RepoInfo, scratch: &Path) -> Result<Vec<FileDiff>, GitError> {
    std::fs::create_dir_all(scratch).map_err(|e| GitError(e.to_string()))?;
    let index_copy = scratch.join("index.head");
    let theirs = info.git_dir.join("index");
    let _ = std::fs::remove_file(&index_copy);
    if theirs.is_file() {
        std::fs::copy(&theirs, &index_copy).map_err(|e| GitError(e.to_string()))?;
    }
    let with_index = |args: &[&str]| -> Result<Vec<u8>, GitError> {
        let mut cmd = git_cmd(&info.root);
        cmd.env("GIT_INDEX_FILE", &index_copy).args(args);
        run_cmd(cmd)
    };
    let mut files = Vec::new();
    if info.head.is_some() {
        let out = with_index(&[
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "-M",
            "-U3",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "HEAD",
            "--",
        ])?;
        files = diff::parse(&String::from_utf8_lossy(&out));
        let untracked = with_index(&["ls-files", "-z", "--others", "--exclude-standard"])?;
        for rel in untracked.split(|b| *b == 0).filter(|s| !s.is_empty()) {
            let rel = String::from_utf8_lossy(rel).into_owned();
            files.push(diff::added_file(&info.root, &rel));
        }
    } else {
        // Empty repository: everything is new.
        for rel in list_files(&info.root)? {
            files.push(diff::added_file(&info.root, &rel));
        }
    }
    let _ = std::fs::remove_file(&index_copy);
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// Files and diff of one commit.
pub fn show_commit(root: &Path, sha: &str) -> Result<Vec<FileDiff>, GitError> {
    let out = run(
        root,
        &[
            "show",
            "--format=",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "-M",
            "-U3",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--first-parent",
            sha,
        ],
    )?;
    Ok(diff::parse(&out))
}

/// One `git worktree list --porcelain` record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub bare: bool,
}

pub fn worktrees(root: &Path) -> Result<Vec<Worktree>, GitError> {
    let out = run(root, &["worktree", "list", "--porcelain"])?;
    Ok(parse_worktrees(&out))
}

pub fn parse_worktrees(out: &str) -> Vec<Worktree> {
    let mut list = Vec::new();
    let mut cur: Option<Worktree> = None;
    for line in out.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            if let Some(w) = cur.take() {
                list.push(w);
            }
            cur = Some(Worktree {
                path: PathBuf::from(path),
                head: None,
                branch: None,
                detached: false,
                bare: false,
            });
        } else if let Some(w) = cur.as_mut() {
            if let Some(head) = line.strip_prefix("HEAD ") {
                w.head = Some(head.to_string());
            } else if let Some(branch) = line.strip_prefix("branch ") {
                w.branch = Some(branch.trim_start_matches("refs/heads/").to_string());
            } else if line == "detached" {
                w.detached = true;
            } else if line == "bare" {
                w.bare = true;
            }
        }
    }
    if let Some(w) = cur.take() {
        list.push(w);
    }
    list
}

/// Number of commits reachable from all refs (for the History header).
pub fn count_commits(root: &Path) -> usize {
    run(root, &["rev-list", "--all", "--count"])
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_porcelain() {
        let out = "worktree /a/repo\nHEAD 1111\nbranch refs/heads/main\n\nworktree /a/wt\nHEAD 2222\ndetached\n\n";
        let w = parse_worktrees(out);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].branch.as_deref(), Some("main"));
        assert!(w[1].detached);
        assert_eq!(w[1].head.as_deref(), Some("2222"));
    }
}
