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

fn git_command(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir)
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
            "-c",
            "maintenance.auto=false",
            "-c",
            "gc.auto=0",
        ])
        .args(args);
    cmd
}

pub fn git_output(dir: &Path, args: &[&str]) -> std::process::Output {
    git_command(dir, args).output().expect("git runs")
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = git_output(dir, args);
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

pub fn commits_as_test(repo: &Path) {
    let hooks = repo.join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    for (key, value) in [
        ("user.name", "Test"),
        ("user.email", "test@example.com"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", hooks.to_str().unwrap()),
    ] {
        git(repo, &["config", key, value]);
    }
}

pub fn hook(repo: &Path, name: &str, script: &str) {
    executable(&repo.join(".git/hooks").join(name), script);
}

pub fn tree_bytes(repo: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![repo.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                if path != repo.join(".git") {
                    stack.push(path);
                }
            } else {
                let name = path.strip_prefix(repo).unwrap().display().to_string();
                files.insert(name, std::fs::read(&path).unwrap_or_default());
            }
        }
    }
    files
}

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
    (map, status_without_refreshing(repo))
}

pub fn status_without_refreshing(folder: &Path) -> String {
    let status = Command::new("git")
        .current_dir(folder)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(["status", "--porcelain=v1", "--untracked-files=all"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&status.stdout).into_owned()
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

pub const NOW: i64 = 1_790_027_160;

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

pub fn no_gh() -> codemap::gh::Gh {
    codemap::gh::Gh::at("/codemap-test/no-such-gh")
}

pub fn executable(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

pub fn fake_gh(dir: &Path, body: &str) -> codemap::gh::Gh {
    let program = dir.join("gh");
    executable(&program, &format!("#!/bin/sh\n{body}\n"));
    codemap::gh::Gh::at(program)
}

pub fn gh_printing(dir: &Path, json: &str) -> codemap::gh::Gh {
    write(dir, "pull-requests.json", json);
    fake_gh(dir, "exec cat \"$(dirname \"$0\")/pull-requests.json\"")
}

pub fn commit_file(repo: &Path, rel: &str, content: &str, subject: &str) {
    write(repo, rel, content);
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-q", "-m", subject]);
}

pub struct Remoted {
    pub tmp: TempDir,
    pub root: PathBuf,
    pub origin: PathBuf,
}

pub fn repo_with_origin() -> Remoted {
    let tmp = TempDir::new("remoted");
    let root = tmp.path().join("work");
    let origin = tmp.path().join("origin.git");
    std::fs::create_dir_all(&root).unwrap();
    git(tmp.path(), &["init", "-q", "--bare", "origin.git"]);
    git(&root, &["init", "-q"]);
    commits_as_test(&root);
    commit_file(&root, "app.py", "x = 1\n", "initial");
    git(
        &root,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&root, &["push", "-q", "-u", "origin", "main"]);
    git(&root, &["remote", "set-head", "origin", "main"]);
    Remoted { tmp, root, origin }
}

pub fn branches_repo() -> Remoted {
    let remoted = repo_with_origin();
    let p = remoted.root.as_path();
    git(p, &["switch", "-q", "-c", "feat/ahead"]);
    commit_file(p, "ahead.py", "a = 1\n", "feat: start counting");
    git(p, &["push", "-q", "-u", "origin", "feat/ahead"]);
    commit_file(p, "ahead.py", "a = 2\n", "feat: count to two");
    commit_file(p, "ahead.py", "a = 3\n", "feat: count to three");
    git(p, &["switch", "-q", "-c", "feat/both", "main"]);
    commit_file(p, "both.py", "b = 1\n", "fix: the shared start");
    commit_file(p, "both.py", "b = 2\n", "fix: only on the remote");
    git(p, &["push", "-q", "-u", "origin", "feat/both"]);
    git(p, &["reset", "-q", "--hard", "HEAD~1"]);
    commit_file(p, "both.py", "b = 3\n", "fix: the first local step");
    commit_file(p, "both.py", "b = 4\n", "fix: the second local step");
    git(p, &["switch", "-q", "-c", "local-only", "main"]);
    for n in 1..=3 {
        let note = format!("note {n}\n");
        commit_file(p, "notes.md", &note, &format!("docs: note {n}"));
    }
    git(p, &["branch", "-q", "merged", "main"]);
    git(p, &["switch", "-q", "main"]);
    remoted
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
        gh: no_gh(),
    });
    app.load_blocking();
    app
}

pub fn custom_roots() -> (TempDir, codemap::custom::Roots) {
    let tmp = TempDir::new("custom");
    let p = tmp.path();
    write(
        p,
        "demo/CLAUDE.md",
        "# demo\n\nA small service with a review workflow.\n\n## Rules\n\nRun the tests first.\n\n## Releases\n\nTag from main.\n",
    );
    write(
        p,
        "demo/.claude/skills/review/SKILL.md",
        "---\nname: review\ndescription: Review a diff for bugs before it is committed.\n---\n\n# Review\n\n1. Read the diff.\n2. Run the tests.\n",
    );
    write(
        p,
        "demo/.claude/agents/planner.md",
        "---\nname: planner\ndescription: Plans the work before any edit.\nmodel: opus\ntools: Read, Grep\n---\n\nPlan first.\n",
    );
    write(p, "demo/.claude/commands/ship.md", "Ship the branch.\n");
    write(
        p,
        "demo/.claude/settings.json",
        "{\"model\":\"sonnet\",\"permissions\":{\"defaultMode\":\"plan\",\"allow\":[\"Read\"]},\"hooks\":{\"PreToolUse\":[{\"matcher\":\"Bash\",\"hooks\":[{\"type\":\"command\",\"command\":\"./scripts/check.sh\"}]}]}}",
    );
    write(
        p,
        "demo/.mcp.json",
        "{\"mcpServers\":{\"docs\":{\"type\":\"http\",\"url\":\"https://mcp.example.com/v1\"}}}",
    );
    write(p, "demo/AGENTS.md", "# Agents\n\nRun the tests first.\n");
    write(p, "home/.claude/CLAUDE.md", "");
    write(
        p,
        "home/.claude/settings.json",
        "{\"model\":\"opus\",\"enabledPlugins\":{\"frontend-design@official\":true}}",
    );
    write(
        p,
        "home/.config/herdr/config.toml",
        "[theme]\nname = \"gruvbox\"\n[keys]\nprefix = \"ctrl+b\"\n[[keys.command]]\nkey = \"ctrl+e\"\n",
    );
    write(p, "home/.config/herdr/config.toml.bak", "");
    write(
        p,
        "home/.config/yazi/yazi.toml",
        "[opener]\nnvim = []\nvscode = []\n",
    );
    write(
        p,
        "home/.config/nvim/init.lua",
        "-- tuned for watching an agent\nvim.g.mapleader = \" \"\n",
    );
    write(p, "home/.codex/AGENTS.md", "Be brief.\n");
    let roots = codemap::custom::Roots {
        repo: p.join("demo"),
        home: p.join("home"),
        config: p.join("home/.config"),
    };
    (tmp, roots)
}

pub fn custom_app(roots: &codemap::custom::Roots, state: &Path) -> codemap::app::App {
    let mut app = codemap::app::App::new(codemap::app::Options {
        path: Some(roots.repo.clone()),
        view: codemap::app::View::Custom,
        context: None,
        client: None,
        store: codemap::store::Store::new(state),
        theme: dracula(),
        gh: no_gh(),
    });
    app.custom.roots = Some(roots.clone());
    app.load_blocking();
    app
}

pub struct Worktrees {
    pub remoted: Remoted,
    pub main: PathBuf,
    pub docs: PathBuf,
    pub gone: PathBuf,
    pub pinned: PathBuf,
}

impl Worktrees {
    pub fn base(&self) -> &Path {
        self.remoted.tmp.path()
    }

    pub fn present(&self) -> [&Path; 3] {
        [&self.main, &self.docs, &self.pinned]
    }
}

pub fn worktrees_repo() -> Worktrees {
    let remoted = repo_with_origin();
    let main = remoted.root.clone();
    let p = main.as_path();
    let beside = |name: &str| remoted.tmp.path().join(name);
    let (docs, gone, pinned) = (beside("docs"), beside("gone"), beside("pinned"));
    let path = |folder: &Path| folder.to_str().unwrap().to_string();
    git(p, &["worktree", "add", "-q", "-b", "docs", &path(&docs)]);
    git(&docs, &["push", "-q", "-u", "origin", "docs"]);
    commit_file(&docs, "guide.md", "# Guide\n", "docs: start the guide");
    git(p, &["worktree", "add", "-q", "--detach", &path(&pinned)]);
    git(
        p,
        &[
            "worktree",
            "lock",
            "--reason",
            "on a removable disk",
            &path(&pinned),
        ],
    );
    git(p, &["worktree", "add", "-q", "-b", "gone", &path(&gone)]);
    std::fs::remove_dir_all(&gone).unwrap();
    write(p, "app.py", "x = 2\n");
    git(p, &["add", "app.py"]);
    write(p, "app.py", "x = 3\n");
    write(p, "notes.md", "# Notes\n\nKeep the timeout.\n");
    write(&docs, "guide.md", "# Guide\n\nRun it.\n");
    write(&docs, "examples/first.md", "one\ntwo\n");
    git(&docs, &["add", "examples/first.md"]);
    Worktrees {
        remoted,
        main,
        docs,
        gone,
        pinned,
    }
}

pub fn untouched(worktrees: &Worktrees) -> impl PartialEq + std::fmt::Debug {
    let each: Vec<_> = worktrees
        .present()
        .iter()
        .map(|folder| (tree_bytes(folder), status_without_refreshing(folder)))
        .collect();
    (fingerprint(&worktrees.main), each, worktrees.gone.exists())
}

pub struct FakeHerdr {
    pub socket: PathBuf,
}

impl FakeHerdr {
    pub fn client(&self) -> codemap::herdr::Client {
        codemap::herdr::Client::new(&self.socket)
    }
}

impl Drop for FakeHerdr {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket);
    }
}

pub fn listed_agent(pane: &str, name: &str, status: &str, cwd: &Path) -> serde_json::Value {
    serde_json::json!({
        "pane_id": pane,
        "terminal_id": format!("term_{pane}"),
        "workspace_id": "w1",
        "tab_id": "w1:t1",
        "agent": name,
        "agent_status": status,
        "cwd": cwd.display().to_string(),
    })
}

pub fn fake_herdr(agents: Vec<serde_json::Value>) -> FakeHerdr {
    fake_herdr_where_panes_are(agents.clone(), agents)
}

pub fn fake_herdr_where_panes_are(
    agents: Vec<serde_json::Value>,
    panes: Vec<serde_json::Value>,
) -> FakeHerdr {
    use std::io::{BufRead, BufReader, Write};

    let socket = PathBuf::from(format!(
        "/tmp/cmw-{}-{}.sock",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&socket);
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut asked = String::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            if reader.read_line(&mut asked).unwrap_or(0) == 0 {
                continue;
            }
            let request: serde_json::Value = serde_json::from_str(asked.trim()).unwrap();
            let pane = panes
                .iter()
                .find(|pane| pane["pane_id"] == request["params"]["pane_id"]);
            let reply = match (request["method"].as_str(), pane) {
                (Some("agent.list"), _) => serde_json::json!({"result": {"agents": agents}}),
                (Some("pane.get"), Some(pane)) => serde_json::json!({"result": {"pane": pane}}),
                _ => serde_json::json!({"error": {"code": "unsupported", "message": "fake"}}),
            };
            let mut reply = reply;
            reply["id"] = request["id"].clone();
            let mut stream = stream;
            let _ = writeln!(stream, "{reply}");
        }
    });
    FakeHerdr { socket }
}
