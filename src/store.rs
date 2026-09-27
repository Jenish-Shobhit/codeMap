//! codeMap's side store. Everything codeMap remembers lives here, in the
//! plugin state directory, never in the user's repository:
//!
//! - `repos/<repo-key>/shadow.git`: a private git dir whose work tree is the
//!   user's checkout. Turn checkpoints are `git add -A` + `write-tree` +
//!   `commit-tree` with that private dir and its own index, so objects, refs
//!   and the index all stay here. Commands that write objects never see the
//!   user's object store (git would otherwise "freshen", i.e. touch, objects
//!   it finds there). Read-only commands such as `diff` borrow it for the
//!   process only, through `GIT_ALTERNATE_OBJECT_DIRECTORIES`.
//! - `panes/<pane-key>.json`: last agent status and the turn list per pane.
//! - `reviews/<repo-key>.json`: reviewed hunk hashes.
//! - `drafts/<repo-key>.json`: unsent review comments.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::git::{self, FileDiff, GitError, RepoInfo};
use crate::util;

/// How many turns to keep per pane before old checkpoints are dropped.
pub const MAX_TURNS: usize = 50;

#[derive(Debug, Clone)]
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root: PathBuf = root.into();
        // git runs with the work tree as its cwd, so the store must not be
        // relative to ours.
        let root = if root.is_relative() {
            std::env::current_dir()
                .map(|d| d.join(&root))
                .unwrap_or(root)
        } else {
            root
        };
        Store { root }
    }

    /// Resolve the state directory: an explicit override, herdr's plugin
    /// state dir, or the path herdr would give `dev.codemap` so that the
    /// standalone binary sees the checkpoints the hook wrote.
    pub fn from_env() -> Self {
        if let Some(dir) = env_path("CODEMAP_STATE_DIR") {
            return Store::new(dir);
        }
        if let Some(dir) = env_path("HERDR_PLUGIN_STATE_DIR") {
            return Store::new(dir);
        }
        let base = env_path("XDG_STATE_HOME")
            .map(|p| p.join("herdr"))
            .or_else(|| env_path("HOME").map(|h| h.join(".local/state/herdr")))
            .unwrap_or_else(|| std::env::temp_dir().join("herdr-state"));
        Store::new(base.join("plugins").join(crate::herdr::PLUGIN_ID))
    }

    pub fn repo_key(root: &Path) -> String {
        let name = root
            .file_name()
            .map(|n| sanitize(&n.to_string_lossy()))
            .unwrap_or_else(|| "repo".to_string());
        let hash = util::fnv_hex(root.to_string_lossy().as_bytes());
        format!("{name}-{}", &hash[..10])
    }

    pub fn shadow(&self, repo: &RepoInfo) -> Shadow {
        let dir = self.root.join("repos").join(Self::repo_key(&repo.root));
        Shadow {
            git_dir: dir.join("shadow.git"),
            dir,
            work_tree: repo.root.clone(),
            repo_objects: repo.objects_dir(),
            repo_common_dir: repo.common_dir.clone(),
        }
    }

    fn json_path(&self, kind: &str, key: &str) -> PathBuf {
        self.root.join(kind).join(format!("{}.json", sanitize(key)))
    }

    fn read_json<T: for<'de> Deserialize<'de> + Default>(&self, path: &Path) -> T {
        fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn write_json<T: Serialize>(&self, path: &Path, value: &T) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension(format!("json.tmp{}", std::process::id()));
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(serde_json::to_string_pretty(value)?.as_bytes())?;
        }
        fs::rename(tmp, path)
    }

    // ---- panes and turns ----------------------------------------------

    pub fn load_pane(&self, pane_key: &str) -> PaneState {
        let mut state: PaneState = self.read_json(&self.json_path("panes", pane_key));
        if state.pane_key.is_empty() {
            state.pane_key = pane_key.to_string();
        }
        state
    }

    pub fn save_pane(&self, state: &PaneState) -> std::io::Result<()> {
        self.write_json(&self.json_path("panes", &state.pane_key), state)
    }

    /// Every pane state that has turns in this repository, newest first.
    pub fn panes_for_repo(&self, root: &Path) -> Vec<PaneState> {
        let dir = self.root.join("panes");
        let Ok(entries) = fs::read_dir(&dir) else {
            return Vec::new();
        };
        let root_s = root.to_string_lossy();
        let mut out: Vec<PaneState> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .map(|e| self.read_json::<PaneState>(&e.path()))
            .filter(|p| p.turns.iter().any(|t| t.repo_root == root_s))
            .collect();
        out.sort_by_key(|p| std::cmp::Reverse(p.updated));
        out
    }

    // ---- reviews and drafts -------------------------------------------

    pub fn load_reviews(&self, repo_key: &str) -> Reviews {
        self.read_json(&self.json_path("reviews", repo_key))
    }

    pub fn save_reviews(&self, repo_key: &str, reviews: &Reviews) -> std::io::Result<()> {
        self.write_json(&self.json_path("reviews", repo_key), reviews)
    }

    pub fn load_drafts(&self, repo_key: &str) -> Vec<Comment> {
        self.read_json(&self.json_path("drafts", repo_key))
    }

    pub fn save_drafts(&self, repo_key: &str, drafts: &Vec<Comment>) -> std::io::Result<()> {
        self.write_json(&self.json_path("drafts", repo_key), drafts)
    }
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Keep file names portable: letters, digits, dot, dash, underscore.
pub fn sanitize(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let out = out.trim_matches('.').to_string();
    if out.is_empty() {
        "_".to_string()
    } else {
        out
    }
}

/// A checkpoint: a commit in the shadow repository.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub commit: String,
    pub tree: String,
    pub at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub n: u32,
    pub repo_root: String,
    #[serde(default)]
    pub start: Option<Checkpoint>,
    #[serde(default)]
    pub end: Option<Checkpoint>,
    /// The status that ended the turn (idle, done or blocked).
    #[serde(default)]
    pub end_status: Option<String>,
    #[serde(default)]
    pub open: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneState {
    pub pane_key: String,
    #[serde(default)]
    pub pane_id: String,
    #[serde(default)]
    pub terminal_id: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub last_status: Option<String>,
    #[serde(default)]
    pub updated: i64,
    #[serde(default)]
    pub turns: Vec<Turn>,
}

impl PaneState {
    pub fn key_for(terminal_id: Option<&str>, pane_id: &str, agent: Option<&str>) -> String {
        let who = terminal_id.filter(|t| !t.is_empty()).unwrap_or(pane_id);
        format!("{}-{}", sanitize(who), sanitize(agent.unwrap_or("agent")))
    }

    pub fn last_turn(&self, root: &Path) -> Option<&Turn> {
        let root = root.to_string_lossy();
        self.turns.iter().rev().find(|t| t.repo_root == root)
    }

    pub fn first_turn(&self, root: &Path) -> Option<&Turn> {
        let root = root.to_string_lossy();
        self.turns.iter().find(|t| t.repo_root == root)
    }

    pub fn matches_pane(&self, pane_id: Option<&str>, terminal_id: Option<&str>) -> bool {
        if let (Some(a), Some(b)) = (terminal_id, self.terminal_id.as_deref()) {
            if a == b {
                return true;
            }
        }
        pane_id.is_some_and(|p| p == self.pane_id)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reviews {
    /// Reviewed hunks, each keyed `"<path>\u{0}<hunk hash>"`.
    #[serde(default)]
    pub hunks: std::collections::BTreeSet<String>,
}

impl Reviews {
    pub fn key(path: &str, hash: &str) -> String {
        format!("{path}\u{0}{hash}")
    }

    pub fn is_reviewed(&self, path: &str, hash: &str) -> bool {
        self.hunks.contains(&Self::key(path, hash))
    }

    pub fn toggle(&mut self, path: &str, hash: &str) -> bool {
        let key = Self::key(path, hash);
        if self.hunks.remove(&key) {
            false
        } else {
            self.hunks.insert(key);
            true
        }
    }
}

/// A review comment anchored to lines of the new file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub path: String,
    pub start: u32,
    pub end: u32,
    pub text: String,
    #[serde(default)]
    pub hunk: String,
}

impl Comment {
    pub fn anchor(&self) -> String {
        if self.end > self.start {
            format!("{}:{}-{}", self.path, self.start, self.end)
        } else {
            format!("{}:{}", self.path, self.start)
        }
    }
}

/// The text P and S send to the agent: one `path:lines — note` per comment.
pub fn format_comments(comments: &[Comment]) -> String {
    let mut out = String::from("Review comments from codeMap:\n");
    for c in comments {
        out.push('\n');
        out.push_str(&c.anchor());
        out.push_str(" — ");
        out.push_str(c.text.trim());
    }
    out
}

/// The shadow repository for one checkout.
#[derive(Debug, Clone)]
pub struct Shadow {
    pub dir: PathBuf,
    pub git_dir: PathBuf,
    pub work_tree: PathBuf,
    repo_objects: PathBuf,
    repo_common_dir: PathBuf,
}

impl Shadow {
    fn index_file(&self) -> PathBuf {
        self.git_dir.join("index")
    }

    /// A git command bound to the shadow dir, its index and the user's
    /// checkout as the work tree. It can write objects, so it must not see
    /// the user's object store.
    pub fn git(&self) -> Command {
        let mut cmd = git::git_cmd(&self.work_tree);
        cmd.env("GIT_DIR", &self.git_dir)
            .env("GIT_WORK_TREE", &self.work_tree)
            .env("GIT_INDEX_FILE", self.index_file())
            .env("GIT_AUTHOR_NAME", "codeMap")
            .env("GIT_AUTHOR_EMAIL", "codemap@localhost")
            .env("GIT_COMMITTER_NAME", "codeMap")
            .env("GIT_COMMITTER_EMAIL", "codemap@localhost")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES");
        cmd
    }

    /// A read-only git command that can also resolve the user's commits
    /// (HEAD, history) through a per-process alternate object directory.
    pub fn git_read(&self) -> Command {
        let mut cmd = self.git();
        cmd.env("GIT_ALTERNATE_OBJECT_DIRECTORIES", &self.repo_objects);
        cmd
    }

    fn run_read(&self, args: &[&str]) -> Result<String, GitError> {
        let mut cmd = self.git_read();
        cmd.args(args);
        git::run_cmd(cmd).map(|out| String::from_utf8_lossy(&out).into_owned())
    }

    fn run(&self, args: &[&str]) -> Result<String, GitError> {
        let mut cmd = self.git();
        cmd.args(args);
        git::run_cmd(cmd).map(|out| String::from_utf8_lossy(&out).into_owned())
    }

    pub fn exists(&self) -> bool {
        self.git_dir.join("HEAD").is_file()
    }

    /// Create the shadow dir on first use.
    pub fn ensure(&self) -> Result<(), GitError> {
        fs::create_dir_all(&self.dir).map_err(|e| GitError(e.to_string()))?;
        if !self.exists() {
            let mut cmd = git::git_cmd(&self.dir);
            cmd.args(["init", "-q", "--bare"]).arg(&self.git_dir);
            git::run_cmd(cmd)?;
            for (k, v) in [
                ("core.bare", "false"),
                ("core.autocrlf", "false"),
                ("core.fsmonitor", "false"),
                ("core.untrackedCache", "false"),
                ("gc.autoDetach", "false"),
                ("maintenance.auto", "false"),
                ("advice.addEmbeddedRepo", "false"),
            ] {
                let mut cmd = git::git_cmd(&self.dir);
                cmd.env("GIT_DIR", &self.git_dir).args(["config", k, v]);
                git::run_cmd(cmd)?;
            }
        }
        // Never keep a persistent alternates file: writes would freshen
        // (touch) the user's objects through it.
        let _ = fs::remove_file(self.git_dir.join("objects").join("info").join("alternates"));
        // Mirror the repo's own excludes so snapshots match what git shows.
        let info = self.git_dir.join("info");
        fs::create_dir_all(&info).map_err(|e| GitError(e.to_string()))?;
        let mut exclude = fs::read_to_string(self.repo_common_dir.join("info").join("exclude"))
            .unwrap_or_default();
        exclude.push_str("\n.git\n");
        let _ = fs::write(info.join("exclude"), exclude);
        Ok(())
    }

    /// Serialise snapshot work between concurrent hook runs.
    fn lock(&self) -> Result<LockGuard, GitError> {
        let path = self.dir.join("snapshot.lock");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    let _ = writeln!(f, "{}", std::process::id());
                    return Ok(LockGuard { path });
                }
                Err(_) => {
                    let stale = fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| SystemTime::now().duration_since(t).ok())
                        .is_some_and(|age| age > Duration::from_secs(60));
                    if stale {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    if Instant::now() > deadline {
                        return Err(GitError("snapshot lock busy".to_string()));
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
        }
    }

    /// Record the work tree as it is now. Returns (commit, tree).
    pub fn snapshot(&self, message: &str) -> Result<Checkpoint, GitError> {
        self.ensure()?;
        let _guard = self.lock()?;
        // Our own index: its stat cache makes later snapshots cheap. It is
        // never seeded from the user's index, whose entries point at objects
        // this store does not have.
        let add = self.run(&["add", "-A", "--ignore-errors", "--", "."]);
        if add.is_err() {
            let _ = fs::remove_file(self.index_file());
            self.run(&["add", "-A", "--ignore-errors", "--", "."])?;
        }
        let tree = self.run(&["write-tree"])?.trim().to_string();
        let commit = self
            .run(&["commit-tree", &tree, "-m", message])?
            .trim()
            .to_string();
        Ok(Checkpoint {
            commit,
            tree,
            at: util::now_unix(),
        })
    }

    pub fn update_ref(&self, name: &str, commit: &str) -> Result<(), GitError> {
        self.run(&["update-ref", name, commit]).map(|_| ())
    }

    pub fn delete_ref(&self, name: &str) -> Result<(), GitError> {
        self.run(&["update-ref", "-d", name]).map(|_| ())
    }

    pub fn refs(&self) -> Vec<String> {
        self.run(&["for-each-ref", "--format=%(refname)"])
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// Diff two tree-ish objects (checkpoint commits, trees, or commits of the
    /// user's repo, which are reachable through alternates).
    pub fn diff(&self, from: &str, to: &str) -> Result<Vec<FileDiff>, GitError> {
        let out = self.run_read(&[
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
        ])?;
        Ok(git::diff::parse(&out))
    }

    /// The empty tree, for diffs against "nothing" (an empty repository).
    pub fn empty_tree(&self) -> Result<String, GitError> {
        let mut cmd = self.git();
        cmd.args(["hash-object", "-t", "tree", "--stdin"]);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let child = cmd.spawn().map_err(|e| GitError(e.to_string()))?;
        let out = child
            .wait_with_output()
            .map_err(|e| GitError(e.to_string()))?;
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Contents of a file at a checkpoint.
    pub fn read_file(&self, commit: &str, path: &str) -> Option<String> {
        self.run_read(&["show", &format!("{commit}:{path}")]).ok()
    }
}

struct LockGuard {
    path: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_portable() {
        assert_eq!(sanitize("w1:p2"), "w1_p2");
        assert_eq!(
            PaneState::key_for(Some("term_abc"), "w1:p1", Some("claude")),
            "term_abc-claude"
        );
        assert_eq!(PaneState::key_for(None, "w1:p1", None), "w1_p1-agent");
        let k = Store::repo_key(Path::new("/Users/x/Desktop/paneMorph"));
        assert!(k.starts_with("paneMorph-"));
    }

    #[test]
    fn relative_roots_become_absolute() {
        let s = Store::new("some/state");
        assert!(s.root.is_absolute());
        assert!(s.root.ends_with("some/state"));
    }

    #[test]
    fn comment_format() {
        let comments = vec![
            Comment {
                path: "src/api.rs".into(),
                start: 42,
                end: 48,
                text: "use the existing helper".into(),
                hunk: String::new(),
            },
            Comment {
                path: "a.py".into(),
                start: 3,
                end: 3,
                text: " no timeout ".into(),
                hunk: String::new(),
            },
        ];
        let text = format_comments(&comments);
        assert!(text.contains("src/api.rs:42-48 — use the existing helper"));
        assert!(text.contains("a.py:3 — no timeout"));
    }

    #[test]
    fn reviews_toggle() {
        let mut r = Reviews::default();
        assert!(r.toggle("a.py", "h1"));
        assert!(r.is_reviewed("a.py", "h1"));
        assert!(!r.toggle("a.py", "h1"));
        assert!(!r.is_reviewed("a.py", "h1"));
    }
}
