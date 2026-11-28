use std::path::Path;
use std::process::{Command, Stdio};

use crate::git::RepoInfo;
use crate::herdr::{self, Client, PaneOpen};

pub const EDIT_PATH_ENV: &str = "CODEMAP_EDIT_PATH";
pub const EDIT_LINE_ENV: &str = "CODEMAP_EDIT_LINE";
pub const BROWSER_ENV: &str = "CODEMAP_BROWSER";
const MAC_BROWSERS: [&str; 2] = ["Google Chrome", "Safari"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opener {
    VsCode,
    Browser,
    GitHub,
}

pub fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

pub fn editor() -> String {
    std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .ok()
        .filter(|e| !e.trim().is_empty())
        .unwrap_or_else(|| if on_path("nvim") { "nvim" } else { "vi" }.to_string())
}

pub fn editor_argv(editor: &str, path: &str, line: Option<u32>) -> Vec<String> {
    let mut argv: Vec<String> = editor.split_whitespace().map(str::to_string).collect();
    let name = argv
        .first()
        .and_then(|p| Path::new(p).file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let at = |sep: &str| match line {
        Some(n) => format!("{path}{sep}{n}"),
        None => path.to_string(),
    };
    match name.as_str() {
        "code" | "cursor" => {
            argv.push("-g".into());
            argv.push(at(":"));
        }
        "zed" | "subl" | "hx" | "helix" => argv.push(at(":")),
        _ => {
            if let Some(n) = line {
                argv.push(format!("+{n}"));
            }
            argv.push(path.to_string());
        }
    }
    argv
}

pub fn editor_command(path: &Path, line: Option<u32>) -> Option<Command> {
    let argv = editor_argv(&editor(), &path.display().to_string(), line);
    let (program, args) = argv.split_first()?;
    let mut cmd = Command::new(program);
    cmd.args(args);
    Some(cmd)
}

pub fn edit_in_split(
    client: &Client,
    target_pane: &str,
    root: &Path,
    path: &Path,
    line: Option<u32>,
) -> Result<(), String> {
    let mut env = vec![(EDIT_PATH_ENV.to_string(), path.display().to_string())];
    if let Some(n) = line {
        env.push((EDIT_LINE_ENV.to_string(), n.to_string()));
    }
    let request = PaneOpen {
        plugin_id: herdr::PLUGIN_ID.to_string(),
        entrypoint: "edit".to_string(),
        placement: "split".to_string(),
        width: None,
        height: None,
        target_pane_id: Some(target_pane.to_string()),
        cwd: Some(root.display().to_string()),
        env,
        focus: true,
    };
    client
        .plugin_pane_open(&request)
        .map(|_| ())
        .map_err(|e| format!("could not open the editor in a split: {e}"))
}

fn detached(program: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("{program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} exited with {status}"))
    }
}

pub fn vscode(path: &Path, line: Option<u32>) -> Result<String, String> {
    let argv = editor_argv("code", &path.display().to_string(), line);
    let args: Vec<&str> = argv.iter().skip(1).map(String::as_str).collect();
    detached("code", &args).map(|()| "opened in VS Code".to_string())
}

fn open_with_default(target: &str) -> Result<(), String> {
    if cfg!(target_os = "macos") {
        detached("open", &[target])
    } else {
        detached("xdg-open", &[target])
    }
}

pub fn browser(path: &Path) -> Result<String, String> {
    let file = path.display().to_string();
    let chosen = std::env::var(BROWSER_ENV)
        .ok()
        .filter(|b| !b.trim().is_empty());
    if cfg!(target_os = "macos") {
        let apps: Vec<String> = match chosen {
            Some(app) => vec![app],
            None => MAC_BROWSERS.map(String::from).to_vec(),
        };
        for app in &apps {
            if detached("open", &["-a", app, &file]).is_ok() {
                return Ok(format!("opened in {app}"));
            }
        }
        return Err(format!("could not open {}", apps.join(" or ")));
    }
    match chosen.or_else(|| std::env::var("BROWSER").ok()) {
        Some(program) => detached(&program, &[&file]).map(|()| format!("opened in {program}")),
        None => open_with_default(&file).map(|()| "opened in the browser".to_string()),
    }
}

pub fn github_base(remote: &str) -> Option<String> {
    let remote = remote.trim();
    let rest = remote
        .strip_prefix("git@github.com:")
        .or_else(|| remote.strip_prefix("ssh://git@github.com/"))
        .or_else(|| remote.strip_prefix("https://github.com/"))
        .or_else(|| remote.strip_prefix("http://github.com/"))?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, name) = rest.split_once('/')?;
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return None;
    }
    Some(format!("https://github.com/{owner}/{name}"))
}

fn escape_segment(segment: &str) -> String {
    let mut out = String::new();
    for b in segment.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn github_url(
    remote: &str,
    reference: &str,
    rel: &str,
    dir: bool,
    line: Option<u32>,
) -> Option<String> {
    let base = github_base(remote)?;
    let kind = if dir { "tree" } else { "blob" };
    let reference: Vec<String> = reference.split('/').map(escape_segment).collect();
    let path: Vec<String> = rel.split('/').map(escape_segment).collect();
    let anchor = match line {
        Some(n) if !dir => format!("#L{n}"),
        _ => String::new(),
    };
    Some(format!(
        "{base}/{kind}/{}/{}{anchor}",
        reference.join("/"),
        path.join("/")
    ))
}

pub fn github_branch_url(remote: &str, branch: &str) -> Option<String> {
    let base = github_base(remote)?;
    let branch: Vec<String> = branch.split('/').map(escape_segment).collect();
    Some(format!("{base}/tree/{}", branch.join("/")))
}

pub fn github_commit_url(remote: &str, sha: &str) -> Option<String> {
    let base = github_base(remote)?;
    let full = matches!(sha.len(), 40 | 64) && sha.bytes().all(|b| b.is_ascii_hexdigit());
    full.then(|| format!("{base}/commit/{sha}"))
}

pub fn web_page(address: &str) -> Result<String, String> {
    open_with_default(address).map(|()| format!("opened {address}"))
}

pub fn github(repo: &RepoInfo, rel: &str, dir: bool, line: Option<u32>) -> Result<String, String> {
    let remote = crate::git::run(&repo.root, &["remote", "get-url", "origin"])
        .map_err(|_| "this repository has no origin remote".to_string())?;
    let reference = repo
        .branch
        .clone()
        .or_else(|| repo.head.clone())
        .ok_or("this repository has no commits yet")?;
    let url = github_url(&remote, &reference, rel, dir, line)
        .ok_or_else(|| format!("origin is not on GitHub: {}", remote.trim()))?;
    open_with_default(&url).map(|()| format!("opened {url}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editors_take_the_line_in_their_own_form() {
        assert_eq!(
            editor_argv("nvim", "src/app.rs", Some(42)),
            ["nvim", "+42", "src/app.rs"]
        );
        assert_eq!(editor_argv("nvim", "src", None), ["nvim", "src"]);
        assert_eq!(
            editor_argv("/usr/local/bin/code --wait", "a.rs", Some(7)),
            ["/usr/local/bin/code", "--wait", "-g", "a.rs:7"]
        );
        assert_eq!(editor_argv("hx", "a.rs", Some(7)), ["hx", "a.rs:7"]);
        assert_eq!(editor_argv("zed", "a.rs", None), ["zed", "a.rs"]);
    }

    #[test]
    fn github_remotes_in_every_spelling_give_one_base() {
        for remote in [
            "git@github.com:acme/widgets.git",
            "https://github.com/acme/widgets.git\n",
            "https://github.com/acme/widgets/",
            "ssh://git@github.com/acme/widgets",
        ] {
            assert_eq!(
                github_base(remote).as_deref(),
                Some("https://github.com/acme/widgets"),
                "{remote}"
            );
        }
        assert_eq!(github_base("git@gitlab.com:acme/widgets.git"), None);
        assert_eq!(github_base("https://github.com/acme"), None);
    }

    #[test]
    fn a_branch_page_is_the_tree_of_that_branch() {
        assert_eq!(
            github_branch_url("git@github.com:acme/widgets.git", "feat/git branches#2").as_deref(),
            Some("https://github.com/acme/widgets/tree/feat/git%20branches%232")
        );
        assert_eq!(
            github_branch_url("https://github.com/acme/widgets", "main").as_deref(),
            Some("https://github.com/acme/widgets/tree/main")
        );
        assert_eq!(github_branch_url("/srv/git/widgets.git", "main"), None);
    }

    #[test]
    fn a_commit_page_is_named_by_the_full_sha() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            github_commit_url("git@github.com:acme/widgets.git", sha).as_deref(),
            Some("https://github.com/acme/widgets/commit/0123456789abcdef0123456789abcdef01234567")
        );
        assert_eq!(github_commit_url("/srv/git/widgets.git", sha), None);
        assert_eq!(
            github_commit_url("https://github.com/acme/widgets", "0123456"),
            None
        );
        assert_eq!(
            github_commit_url("https://github.com/acme/widgets", &sha.replace('a', "/")),
            None
        );
    }

    #[test]
    fn github_links_point_at_the_file_line_or_folder() {
        let remote = "git@github.com:acme/widgets.git";
        assert_eq!(
            github_url(remote, "main", "src/app.rs", false, Some(12)).as_deref(),
            Some("https://github.com/acme/widgets/blob/main/src/app.rs#L12")
        );
        assert_eq!(
            github_url(
                remote,
                "feat/files popup",
                "docs/my notes#1.md",
                false,
                None
            )
            .as_deref(),
            Some("https://github.com/acme/widgets/blob/feat/files%20popup/docs/my%20notes%231.md")
        );
        assert_eq!(
            github_url(remote, "main", "src/ui", true, Some(3)).as_deref(),
            Some("https://github.com/acme/widgets/tree/main/src/ui")
        );
    }
}
