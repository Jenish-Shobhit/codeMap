//! The demo workspace both captures show: paneMorph's code in a git
//! repository with a short history, and the agent turn applied to it.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The repository and its second worktree.
pub struct Repo {
    pub root: PathBuf,
    pub worktree: PathBuf,
}

fn git(dir: &Path, date: &str, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Example Dev")
        .env("GIT_AUTHOR_EMAIL", "dev@example.com")
        .env("GIT_COMMITTER_NAME", "Example Dev")
        .env("GIT_COMMITTER_EMAIL", "dev@example.com")
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
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
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

pub fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let p = entry.path();
        let dest = to.join(entry.file_name());
        if p.is_dir() {
            copy_tree(&p, &dest);
        } else {
            std::fs::copy(&p, &dest).unwrap();
        }
    }
}

/// Copy one fixture file as it was before the agent's turn.
fn copy(fx: &Path, root: &Path, rel: &str) {
    let dest = root.join(rel);
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    let before = fx.join("panemorph_before").join(rel);
    let src = if before.exists() {
        before
    } else {
        fx.join("panemorph").join(rel)
    };
    std::fs::copy(src, dest).unwrap();
}

fn write(root: &Path, rel: &str, text: &str) {
    std::fs::write(root.join(rel), text).unwrap();
}

/// Build `code/paneMorph` (and the `code/paneMorph-docs` worktree) under
/// `home`, with the code as it was before the agent's turn.
pub fn build_repo(fx: &Path, home: &Path) -> Repo {
    let code = home.join("code");
    let root = code.join("paneMorph");
    std::fs::create_dir_all(&root).unwrap();
    let r = root.as_path();
    let day = |d: u32, h: u32| format!("2026-09-{d:02}T{h:02}:12:00Z");
    let commit = |date: &str, msg: &str| {
        git(r, date, &["add", "-A"]);
        git(r, date, &["commit", "-q", "-m", msg]);
    };
    let merge = |date: &str, branch: &str| {
        let msg = format!("Merge branch '{branch}'");
        git(r, date, &["merge", "-q", "--no-ff", branch, "-m", &msg]);
    };

    git(r, &day(12, 10), &["init", "-q"]);
    write(r, "README.md", "# paneMorph\n\nMove live herdr panes.\n");
    write(r, ".gitignore", "__pycache__/\n*.pyc\n");
    copy(fx, r, "panemorph/__init__.py");
    commit(&day(12, 10), "chore: scaffold the package");
    write(r, "LICENSE", "MIT License\n");
    commit(&day(12, 11), "chore: add the MIT license");
    for f in ["api.py", "model.py"] {
        copy(fx, r, &format!("panemorph/{f}"));
    }
    commit(&day(13, 9), "feat: add the herdr API client");
    copy(fx, r, "panemorph/topology.py");
    commit(&day(14, 15), "feat: plan tab merges from the layout tree");
    for f in ["service.py", "doctor.py"] {
        copy(fx, r, &format!("panemorph/{f}"));
    }
    commit(&day(15, 14), "feat: add the pane service and doctor");

    git(r, &day(16, 9), &["checkout", "-q", "-b", "workflows"]);
    for f in [
        "__init__.py",
        "extract.py",
        "open_selector.py",
        "selector.py",
    ] {
        copy(fx, r, &format!("panemorph/actions/{f}"));
    }
    commit(&day(16, 10), "feat: implement pane and tab workflows");
    let manifest = "id = \"dev.panemorph\"\nname = \"paneMorph\"\n";
    write(r, "herdr-plugin.toml", manifest);
    commit(&day(16, 16), "feat: add the herdr plugin manifest");
    git(r, &day(16, 17), &["checkout", "-q", "main"]);
    let readme = "# paneMorph\n\nMove live herdr panes between tabs without restarting them.\n";
    write(r, "README.md", readme);
    commit(&day(16, 18), "docs: describe the selector");
    merge(&day(17, 10), "workflows");
    git(r, &day(17, 10), &["tag", "v0.1.0"]);

    git(
        r,
        &day(18, 9),
        &["checkout", "-q", "-b", "overlay-selector"],
    );
    write(
        r,
        "herdr-plugin.toml",
        &format!("{manifest}placement = \"overlay\"\n"),
    );
    commit(&day(18, 11), "fix: open the selector as an overlay");
    git(r, &day(18, 12), &["checkout", "-q", "main"]);
    let changelog = "# Changelog\n\n## 0.1.0\n\n- Send and bring panes.\n";
    write(r, "CHANGELOG.md", changelog);
    commit(&day(19, 15), "chore: add a changelog");
    merge(&day(20, 10), "overlay-selector");
    git(r, &day(20, 10), &["branch", "-q", "-D", "overlay-selector"]);

    // A second worktree, where another agent writes docs.
    let worktree = code.join("paneMorph-docs");
    let wt = worktree.to_str().unwrap();
    git(
        r,
        &day(21, 9),
        &["worktree", "add", "-q", "-b", "docs-examples", wt],
    );
    write(&worktree, "README.md", &format!("{readme}\n## Examples\n"));
    git(
        &worktree,
        &day(21, 20),
        &["commit", "-q", "-am", "docs: add usage examples"],
    );
    Repo {
        root: root.canonicalize().unwrap(),
        worktree: worktree.canonicalize().unwrap(),
    }
}

/// What the agent's turn wrote: the real change of paneMorph's b8b645e.
pub fn apply_turn(fx: &Path, root: &Path) {
    copy_tree(&fx.join("panemorph"), root);
}
