//! Helpers shared by the integration tests: temp dirs and throwaway repos.
#![allow(dead_code)]

pub mod pty;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct TempDir(pub PathBuf);

static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl TempDir {
    pub fn new(name: &str) -> Self {
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!(
            "codemap-test-{name}-{}-{n}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();
        // Resolve /var -> /private/var on macOS so paths compare equal to git's.
        TempDir(base.canonicalize().unwrap())
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_AUTHOR_DATE", "2026-09-21T21:44:00Z")
        .env("GIT_COMMITTER_DATE", "2026-09-21T21:44:00Z")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .args([
            "-c",
            "init.defaultBranch=main",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "tag.gpgsign=false",
        ])
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, content).unwrap();
}

/// A repo with one commit containing `files`.
pub fn repo_with(files: &[(&str, &str)]) -> TempDir {
    let t = TempDir::new("repo");
    git(t.path(), &["init", "-q"]);
    for (rel, content) in files {
        write(t.path(), rel, content);
    }
    git(t.path(), &["add", "-A"]);
    git(t.path(), &["commit", "-q", "-m", "initial"]);
    t
}

/// Fingerprint of everything under `.git` (path -> size, mtime, content hash)
/// plus `git status` output. Any write codeMap made to the repo shows up.
pub fn fingerprint(repo: &Path) -> (BTreeMap<String, (u64, u128, u64)>, String) {
    let mut map = BTreeMap::new();
    let git_dir = repo.join(".git");
    let mut stack = vec![git_dir.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            let meta = entry.metadata().unwrap();
            if meta.is_dir() {
                stack.push(path);
            } else {
                let bytes = std::fs::read(&path).unwrap_or_default();
                let mtime = meta
                    .modified()
                    .unwrap()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                map.insert(
                    path.strip_prefix(&git_dir).unwrap().display().to_string(),
                    (meta.len(), mtime, codemap::util::fnv64(&bytes)),
                );
            }
        }
    }
    let status = Command::new("git")
        .current_dir(repo)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(["status", "--porcelain=v1", "--untracked-files=all"])
        .output()
        .unwrap();
    (map, String::from_utf8_lossy(&status.stdout).into_owned())
}

pub fn fixture(name: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("fixture {}: {e}", p.display()))
}

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Fixed "now" for snapshots: 21 Sep 2026 21:46:00 UTC.
pub const NOW: i64 = 1_790_027_160;

/// paneMorph's package as a git repo with history, plus one recorded agent
/// turn (the real change of commit b8b645e) in codeMap's shadow store.
pub struct PmRepo {
    pub tmp: TempDir,
    pub root: PathBuf,
    pub state: TempDir,
    pub store: codemap::store::Store,
}

pub fn copy_tree(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let p = entry.path();
        let dest = to.join(entry.file_name());
        if p.is_dir() {
            std::fs::create_dir_all(&dest).unwrap();
            copy_tree(&p, &dest);
        } else {
            std::fs::copy(&p, &dest).unwrap();
        }
    }
}

pub fn panemorph_repo() -> PmRepo {
    let tmp = TempDir::new("pm");
    let root = tmp.path().join("paneMorph");
    std::fs::create_dir_all(&root).unwrap();
    let p = root.as_path();
    git(p, &["init", "-q"]);
    let fx = fixtures_dir();
    copy_tree(&fx.join("panemorph"), p);
    copy_tree(&fx.join("panemorph_before"), p);
    // History: two feature commits, a tag and a docs branch.
    let actions = p.join("panemorph/actions");
    let stash = TempDir::new("pm-actions");
    copy_tree(&actions, stash.path());
    std::fs::remove_dir_all(&actions).unwrap();
    write(p, "README.md", "# paneMorph\n\nMove live herdr panes.\n");
    git(p, &["add", "-A"]);
    git(
        p,
        &[
            "commit",
            "-q",
            "-m",
            "feat: add Herdr API client and topology planner",
        ],
    );
    std::fs::create_dir_all(&actions).unwrap();
    copy_tree(stash.path(), &actions);
    git(p, &["add", "-A"]);
    git(
        p,
        &[
            "commit",
            "-q",
            "-m",
            "feat: implement pane and tab workflows",
        ],
    );
    git(p, &["tag", "v0.1.0"]);
    git(p, &["checkout", "-q", "-b", "docs"]);
    write(
        p,
        "README.md",
        "# paneMorph\n\nMove live herdr panes between tabs.\n",
    );
    git(p, &["commit", "-q", "-am", "docs: describe workflows"]);
    git(p, &["checkout", "-q", "main"]);
    git(
        p,
        &[
            "merge",
            "-q",
            "--no-ff",
            "docs",
            "-m",
            "Merge branch 'docs'",
        ],
    );

    // The agent's turn, recorded the way the hook records it.
    let state = TempDir::new("pm-state");
    let store = codemap::store::Store::new(state.path());
    let info = codemap::git::discover(p).unwrap();
    let shadow = store.shadow(&info);
    let mut start = shadow.snapshot("turn 1 start").unwrap();
    copy_tree(&fx.join("panemorph"), p);
    let mut end = shadow.snapshot("turn 1 end").unwrap();
    start.at = NOW - 420;
    end.at = NOW - 120;
    let mut pane = codemap::store::PaneState {
        pane_key: "term_1-claude".into(),
        pane_id: "w1:p2".into(),
        terminal_id: Some("term_1".into()),
        agent: Some("claude".into()),
        last_status: Some("idle".into()),
        updated: NOW - 120,
        turns: Vec::new(),
    };
    pane.turns.push(codemap::store::Turn {
        n: 3,
        repo_root: info.root.to_string_lossy().into_owned(),
        start: Some(start),
        end: Some(end),
        end_status: Some("idle".into()),
        open: false,
    });
    store.save_pane(&pane).unwrap();
    PmRepo {
        tmp,
        root,
        state,
        store,
    }
}

pub fn dracula() -> codemap::theme::Theme {
    codemap::theme::Theme::from_config_text(
        "[theme]\nname = \"dracula\"\n[theme.custom]\npanel_bg = \"#000000\"\nsidebar_bg = \"#000000\"\nactive_row_bg = \"#141414\"\nselection_bg = \"#1a1a1a\"\n",
    )
}

pub fn agent_context() -> codemap::herdr::PluginContext {
    codemap::herdr::PluginContext {
        workspace_label: Some("paneMorph".into()),
        tab_label: Some("selector-fix".into()),
        focused_pane_id: Some("w1:p2".into()),
        focused_pane_agent: Some("claude".into()),
        focused_pane_status: Some("idle".into()),
        ..Default::default()
    }
}

pub fn app_for(pm: &PmRepo, view: codemap::app::View, with_agent: bool) -> codemap::app::App {
    codemap::util::freeze_time(NOW);
    let mut app = codemap::app::App::new(codemap::app::Options {
        path: Some(pm.root.clone()),
        view,
        context: with_agent.then(agent_context),
        client: None,
        store: pm.store.clone(),
        theme: dracula(),
    });
    app.load_blocking();
    app
}
