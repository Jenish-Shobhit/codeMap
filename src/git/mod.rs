pub mod branch;
pub mod diff;
pub mod graph;
pub mod worktree;
pub mod write;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

pub use diff::{DiffLine, FileDiff, FileStatus, Hunk, LineKind};
pub use graph::{Commit, FileCommit, GraphRow};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitError(pub String);

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GitError {}

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
    stdout_of(output)
}

fn stdout_of(output: Output) -> Result<Vec<u8>, GitError> {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInfo {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
    pub head: Option<String>,
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
    files.retain(|f| root.join(f).is_file());
    Ok(files)
}

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

const DIFF_FORMAT: [&str; 7] = [
    "--no-color",
    "--no-ext-diff",
    "--no-textconv",
    "-M",
    "-U3",
    "--src-prefix=a/",
    "--dst-prefix=b/",
];

const STAGED_DIFF: &[&str] = &["--cached"];
const UNSTAGED_DIFF: &[&str] = &["--ours"];

static INDEX_COPIES: AtomicU64 = AtomicU64::new(0);

fn is_running(process: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(process) else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    let signalled = unsafe { libc::kill(pid, 0) };
    signalled == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

fn process_of_copy(file_name: &str) -> Option<u32> {
    let mut parts = file_name.split('.');
    let (index, kind) = (parts.next()?, parts.next()?);
    let (process, serial) = (parts.next()?, parts.next()?);
    if index != "index" || kind.is_empty() || parts.next().is_some() {
        return None;
    }
    digits(serial)?.parse::<u64>().ok()?;
    digits(process)?.parse().ok()
}

fn digits(part: &str) -> Option<&str> {
    (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())).then_some(part)
}

fn remove_copies_of_ended_processes(scratch: &Path) {
    let Ok(entries) = std::fs::read_dir(scratch) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let owner = name.to_str().and_then(process_of_copy);
        if owner.is_some_and(|process| !is_running(process)) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

struct IndexCopy<'a> {
    info: &'a RepoInfo,
    path: PathBuf,
}

impl<'a> IndexCopy<'a> {
    fn new(info: &'a RepoInfo, scratch: &Path, kind: &str) -> Result<Self, GitError> {
        std::fs::create_dir_all(scratch).map_err(|e| GitError(e.to_string()))?;
        remove_copies_of_ended_processes(scratch);
        let serial = INDEX_COPIES.fetch_add(1, Ordering::Relaxed);
        let name = format!("index.{kind}.{}.{serial}", std::process::id());
        let copy = IndexCopy {
            info,
            path: scratch.join(name),
        };
        let theirs = info.git_dir.join("index");
        let _ = std::fs::remove_file(&copy.path);
        if theirs.is_file() {
            std::fs::copy(&theirs, &copy.path).map_err(|e| GitError(e.to_string()))?;
            let mtime = std::fs::metadata(&theirs)
                .and_then(|m| m.modified())
                .map_err(|e| GitError(e.to_string()))?;
            std::fs::OpenOptions::new()
                .write(true)
                .open(&copy.path)
                .and_then(|f| f.set_modified(mtime))
                .map_err(|e| GitError(e.to_string()))?;
        }
        Ok(copy)
    }

    fn git(&self, args: &[&str]) -> Result<Vec<u8>, GitError> {
        let mut cmd = git_cmd(&self.info.root);
        cmd.env("GIT_INDEX_FILE", &self.path).args(args);
        run_cmd(cmd)
    }

    fn diff(&self, against: &[&str]) -> Result<Vec<FileDiff>, GitError> {
        let args = [&["diff"], &DIFF_FORMAT[..], against, &["--"]].concat();
        let out = self.git(&args)?;
        Ok(diff::parse(&String::from_utf8_lossy(&out)))
    }

    fn untracked(&self) -> Result<Vec<String>, GitError> {
        let out = self.git(&["ls-files", "-z", "--others", "--exclude-standard"])?;
        Ok(out
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect())
    }
}

impl Drop for IndexCopy<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn diff_head(info: &RepoInfo, scratch: &Path) -> Result<Vec<FileDiff>, GitError> {
    let index = IndexCopy::new(info, scratch, "head")?;
    let mut files = Vec::new();
    if info.head.is_some() {
        files = index.diff(&["HEAD"])?;
        for rel in index.untracked()? {
            files.push(diff::added_file(&info.root, &rel));
        }
    } else {
        for rel in list_files(&info.root)? {
            files.push(diff::added_file(&info.root, &rel));
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub staged: Vec<FileDiff>,
    pub unstaged: Vec<FileDiff>,
}

pub fn status(info: &RepoInfo, scratch: &Path) -> Result<Status, GitError> {
    let index = IndexCopy::new(info, scratch, "status")?;
    let mut staged = index.diff(STAGED_DIFF)?;
    let mut unstaged = index.diff(UNSTAGED_DIFF)?;
    for rel in index.untracked()? {
        unstaged.push(FileDiff {
            status: FileStatus::Untracked,
            ..diff::added_file(&info.root, &rel)
        });
    }
    staged.sort_by(|a, b| a.path.cmp(&b.path));
    unstaged.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Status { staged, unstaged })
}

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
            "--end-of-options",
            sha,
        ],
    )?;
    Ok(diff::parse(&out))
}

pub fn show_commit_paths(
    root: &Path,
    sha: &str,
    paths: &[String],
) -> Result<Vec<FileDiff>, GitError> {
    let mut args = vec!["--literal-pathspecs", "show", "--format="];
    args.extend(DIFF_FORMAT);
    args.extend(["--first-parent", "--end-of-options", sha, "--"]);
    args.extend(paths.iter().map(String::as_str));
    Ok(diff::parse(&run(root, &args)?))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub bare: bool,
    pub locked: Option<String>,
    pub prunable: Option<String>,
}

pub fn worktrees(root: &Path) -> Result<Vec<Worktree>, GitError> {
    let out = run(root, &["worktree", "list", "--porcelain"])?;
    Ok(parse_worktrees(&out))
}

fn reason_after(line: &str, word: &str) -> Option<String> {
    let rest = line.strip_prefix(word)?;
    (rest.is_empty() || rest.starts_with(' ')).then(|| rest.trim().to_string())
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
                ..Default::default()
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
            } else if let Some(why) = reason_after(line, "locked") {
                w.locked = Some(why);
            } else if let Some(why) = reason_after(line, "prunable") {
                w.prunable = Some(why);
            }
        }
    }
    if let Some(w) = cur.take() {
        list.push(w);
    }
    list
}

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
        assert!(w.iter().all(|w| w.locked.is_none() && w.prunable.is_none()));
    }

    #[test]
    fn a_locked_and_a_prunable_worktree_keep_what_git_said_about_them() {
        let out = "worktree /a/repo\nHEAD 1111\nbranch refs/heads/main\n\nworktree /a/usb\nHEAD 2222\nbranch refs/heads/usb\nlocked on a removable disk\n\nworktree /a/kept\nHEAD 3333\ndetached\nlocked\n\nworktree /a/gone\nHEAD 4444\nbranch refs/heads/gone\nprunable gitdir file points to non-existent location\n\nworktree /a/bare.git\nbare\n";
        let w = parse_worktrees(out);
        let paths: Vec<_> = w.iter().map(|w| w.path.display().to_string()).collect();
        assert_eq!(
            paths,
            ["/a/repo", "/a/usb", "/a/kept", "/a/gone", "/a/bare.git"]
        );
        assert_eq!(w[1].locked.as_deref(), Some("on a removable disk"));
        assert_eq!(w[2].locked.as_deref(), Some(""));
        assert_eq!(
            w[3].prunable.as_deref(),
            Some("gitdir file points to non-existent location")
        );
        assert_eq!((w[0].locked.as_ref(), w[0].prunable.as_ref()), (None, None));
        assert!(w[4].bare && !w[3].bare);
    }

    #[test]
    fn an_index_copy_is_named_after_the_process_that_made_it() {
        assert_eq!(process_of_copy("index.status.4242.7"), Some(4242));
        assert_eq!(process_of_copy("index.head.1.0"), Some(1));
        for other in [
            "index.status",
            "index.head",
            "index",
            "shadow.git",
            "notes.4242.7",
            "index.4242.7",
            "index.status.4242.7.lock",
            "old.index.status.4242.7",
            "index..4242.7",
            "index.status.x.7",
            "index.status.4242.x",
            "index.status.+4242.7",
            "index.status.4242.+7",
            "index.status.-1.7",
            "index.status.4242.-1",
            "index.status..7",
            "index.status.4242.",
            "index.status.42 42.7",
            "index.status.4242.7 ",
            "index.status.٤٢٤٢.7",
            "index.status.4242.٧",
            "index.status.4294967296.7",
            "index.status.4242.18446744073709551616",
        ] {
            assert_eq!(process_of_copy(other), None, "{other}");
        }
        assert!(is_running(std::process::id()));
        assert!(!is_running(0));
        assert!(!is_running(u32::MAX));
    }
}
