use std::collections::BTreeMap;
use std::io::{ErrorKind, Read};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::util;

pub const PROGRAM_ENV: &str = "CODEMAP_GH";
pub const TIMEOUT: Duration = Duration::from_secs(5);
pub const LIST_PULL_REQUESTS: [&str; 8] = [
    "pr",
    "list",
    "--state",
    "all",
    "--limit",
    "100",
    "--json",
    "number,title,state,headRefName,baseRefName,url,statusCheckRollup",
];
const MOST_OUTPUT: u64 = 8 << 20;
const BUSY_RETRIES: usize = 5;
const PIPE_GRACE: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Open,
    Merged,
    Closed,
}

impl State {
    pub fn word(self) -> &'static str {
        match self {
            State::Open => "open",
            State::Merged => "merged",
            State::Closed => "closed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ci {
    Passed,
    Failed,
    Running,
}

impl Ci {
    pub fn mark(self) -> &'static str {
        match self {
            Ci::Passed => "✓",
            Ci::Failed => "✗",
            Ci::Running => "…",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Checks {
    pub passed: usize,
    pub failed: usize,
    pub running: usize,
}

impl Checks {
    pub fn total(&self) -> usize {
        self.passed + self.failed + self.running
    }

    pub fn ci(&self) -> Option<Ci> {
        if self.failed > 0 {
            Some(Ci::Failed)
        } else if self.running > 0 {
            Some(Ci::Running)
        } else if self.passed > 0 {
            Some(Ci::Passed)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub state: State,
    pub base: String,
    pub url: String,
    pub checks: Checks,
}

impl PullRequest {
    pub fn link(&self) -> Option<&str> {
        let plain = !self
            .url
            .chars()
            .any(|c| c.is_control() || c.is_whitespace());
        (self.url.starts_with("https://") && plain).then_some(self.url.as_str())
    }
}

pub type PullRequests = BTreeMap<String, PullRequest>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unavailable {
    NoRemote,
    NotInstalled,
    NotStarted(String),
    TimedOut(Duration),
    NotSignedIn,
    NotGitHub,
    Offline,
    Failed(String),
    Unreadable,
}

impl Unavailable {
    pub fn why(&self) -> String {
        match self {
            Unavailable::NoRemote => "this repository has no remote".into(),
            Unavailable::NotInstalled => "gh is not installed".into(),
            Unavailable::NotStarted(why) => format!("gh could not be run: {why}"),
            Unavailable::TimedOut(limit) => {
                let seconds = limit.as_secs_f32();
                let unit = if seconds == 1.0 { "second" } else { "seconds" };
                format!("gh did not answer within {seconds} {unit}")
            }
            Unavailable::NotSignedIn => "gh is not signed in to GitHub".into(),
            Unavailable::NotGitHub => "this repository is not on GitHub".into(),
            Unavailable::Offline => "gh could not reach GitHub".into(),
            Unavailable::Failed(said) => format!("gh said: {said}"),
            Unavailable::Unreadable => "gh printed something codeMap could not read".into(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawCheck {
    status: Option<String>,
    conclusion: Option<String>,
    state: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPullRequest {
    number: u64,
    #[serde(default)]
    title: String,
    state: String,
    head_ref_name: String,
    #[serde(default)]
    base_ref_name: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    status_check_rollup: Option<Vec<RawCheck>>,
}

fn upper(field: &Option<String>) -> String {
    field.as_deref().unwrap_or_default().to_ascii_uppercase()
}

fn outcome(check: &RawCheck) -> Ci {
    let state = upper(&check.state);
    if !state.is_empty() {
        return match state.as_str() {
            "SUCCESS" => Ci::Passed,
            "PENDING" | "EXPECTED" => Ci::Running,
            _ => Ci::Failed,
        };
    }
    if upper(&check.status) != "COMPLETED" {
        return Ci::Running;
    }
    match upper(&check.conclusion).as_str() {
        "SUCCESS" | "NEUTRAL" | "SKIPPED" => Ci::Passed,
        _ => Ci::Failed,
    }
}

fn checks(rollup: &[RawCheck]) -> Checks {
    let mut checks = Checks::default();
    for check in rollup {
        match outcome(check) {
            Ci::Passed => checks.passed += 1,
            Ci::Failed => checks.failed += 1,
            Ci::Running => checks.running += 1,
        }
    }
    checks
}

fn state(word: &str) -> Option<State> {
    match word.to_ascii_uppercase().as_str() {
        "OPEN" => Some(State::Open),
        "MERGED" => Some(State::Merged),
        "CLOSED" => Some(State::Closed),
        _ => None,
    }
}

fn preferred(candidate: &PullRequest, over: &PullRequest) -> bool {
    let rank = |pull: &PullRequest| (pull.state == State::Open, pull.number);
    rank(candidate) > rank(over)
}

pub fn parse(json: &str) -> Result<PullRequests, Unavailable> {
    let raw: Vec<RawPullRequest> =
        serde_json::from_str(json).map_err(|_| Unavailable::Unreadable)?;
    let mut by_branch = PullRequests::new();
    for pull in raw {
        let Some(state) = state(&pull.state) else {
            continue;
        };
        let found = PullRequest {
            number: pull.number,
            title: pull.title,
            state,
            base: pull.base_ref_name,
            url: pull.url,
            checks: checks(&pull.status_check_rollup.unwrap_or_default()),
        };
        let known = by_branch.get(&pull.head_ref_name);
        if known.is_none_or(|known| preferred(&found, known)) {
            by_branch.insert(pull.head_ref_name, found);
        }
    }
    Ok(by_branch)
}

pub fn refusal(stderr: &str) -> Unavailable {
    let said = stderr.to_ascii_lowercase();
    let says = |phrases: &[&str]| phrases.iter().any(|phrase| said.contains(phrase));
    if says(&[
        "known github host",
        "no git remotes",
        "not a git repository",
    ]) {
        return Unavailable::NotGitHub;
    }
    if says(&[
        "gh auth login",
        "not logged in",
        "bad credentials",
        "http 401",
    ]) {
        return Unavailable::NotSignedIn;
    }
    if says(&[
        "error connecting",
        "dial tcp",
        "no such host",
        "could not resolve host",
        "network is unreachable",
        "connection refused",
        "i/o timeout",
    ]) {
        return Unavailable::Offline;
    }
    let first = stderr.lines().map(str::trim).find(|line| !line.is_empty());
    Unavailable::Failed(match first {
        Some(line) => util::truncate(&util::sanitize_line(line), 80),
        None => "nothing, and failed".into(),
    })
}

struct Answer {
    success: bool,
    stdout: String,
    stderr: String,
}

fn collected(pipe: Option<impl Read + Send + 'static>) -> Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(pipe) = pipe {
            let _ = pipe.take(MOST_OUTPUT).read_to_end(&mut bytes);
        }
        let _ = tx.send(bytes);
    });
    rx
}

fn text(pipe: &Receiver<Vec<u8>>) -> String {
    let bytes = pipe.recv_timeout(PIPE_GRACE).unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn stop(child: &mut Child) {
    if let Ok(group) = libc::pid_t::try_from(child.id()) {
        if group > 1 {
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gh {
    pub program: PathBuf,
    pub timeout: Duration,
}

impl Gh {
    pub fn at(program: impl Into<PathBuf>) -> Gh {
        Gh {
            program: program.into(),
            timeout: TIMEOUT,
        }
    }

    pub fn from_env() -> Gh {
        let named = std::env::var_os(PROGRAM_ENV).filter(|program| !program.is_empty());
        Gh::at(named.unwrap_or_else(|| "gh".into()))
    }

    fn started(&self, root: &Path) -> Result<Child, Unavailable> {
        let mut busy = 0;
        loop {
            let spawned = Command::new(&self.program)
                .current_dir(root)
                .args(LIST_PULL_REQUESTS)
                .env("GH_PROMPT_DISABLED", "1")
                .env("GH_NO_UPDATE_NOTIFIER", "1")
                .env_remove("GH_FORCE_TTY")
                .env_remove("CLICOLOR_FORCE")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .process_group(0)
                .spawn();
            match spawned {
                Ok(child) => return Ok(child),
                Err(e) if e.kind() == ErrorKind::ExecutableFileBusy && busy < BUSY_RETRIES => {
                    busy += 1;
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) if e.kind() == ErrorKind::NotFound => return Err(Unavailable::NotInstalled),
                Err(e) => return Err(Unavailable::NotStarted(e.to_string())),
            }
        }
    }

    fn answer(&self, root: &Path) -> Result<Answer, Unavailable> {
        let mut child = self.started(root)?;
        let stdout = collected(child.stdout.take());
        let stderr = collected(child.stderr.take());
        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() >= deadline => {
                    stop(&mut child);
                    return Err(Unavailable::TimedOut(self.timeout));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => {
                    stop(&mut child);
                    return Err(Unavailable::NotStarted(e.to_string()));
                }
            }
        };
        Ok(Answer {
            success: status.success(),
            stdout: text(&stdout),
            stderr: text(&stderr),
        })
    }

    pub fn pull_requests(&self, root: &Path) -> Result<PullRequests, Unavailable> {
        let remotes = crate::git::run(root, &["remote"]);
        if remotes.is_ok_and(|names| names.trim().is_empty()) {
            return Err(Unavailable::NoRemote);
        }
        let answer = self.answer(root)?;
        if !answer.success {
            return Err(refusal(&answer.stderr));
        }
        parse(&answer.stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(status: &str, conclusion: &str) -> RawCheck {
        RawCheck {
            status: Some(status.into()),
            conclusion: Some(conclusion.into()),
            state: None,
        }
    }

    fn context(state: &str) -> RawCheck {
        RawCheck {
            state: Some(state.into()),
            ..Default::default()
        }
    }

    #[test]
    fn a_check_run_and_a_commit_status_each_pass_fail_or_run() {
        for (raw, expected) in [
            (check("COMPLETED", "SUCCESS"), Ci::Passed),
            (check("COMPLETED", "NEUTRAL"), Ci::Passed),
            (check("COMPLETED", "SKIPPED"), Ci::Passed),
            (check("COMPLETED", "FAILURE"), Ci::Failed),
            (check("COMPLETED", "CANCELLED"), Ci::Failed),
            (check("COMPLETED", "TIMED_OUT"), Ci::Failed),
            (check("COMPLETED", "ACTION_REQUIRED"), Ci::Failed),
            (check("COMPLETED", ""), Ci::Failed),
            (check("IN_PROGRESS", ""), Ci::Running),
            (check("QUEUED", ""), Ci::Running),
            (check("", ""), Ci::Running),
            (context("SUCCESS"), Ci::Passed),
            (context("PENDING"), Ci::Running),
            (context("EXPECTED"), Ci::Running),
            (context("FAILURE"), Ci::Failed),
            (context("ERROR"), Ci::Failed),
        ] {
            assert_eq!(outcome(&raw), expected, "{raw:?}");
        }
    }

    #[test]
    fn one_failure_outweighs_what_runs_and_what_runs_outweighs_what_passed() {
        let of = |passed, failed, running| Checks {
            passed,
            failed,
            running,
        };
        assert_eq!(of(0, 0, 0).ci(), None);
        assert_eq!(of(3, 0, 0).ci(), Some(Ci::Passed));
        assert_eq!(of(3, 0, 1).ci(), Some(Ci::Running));
        assert_eq!(of(3, 1, 1).ci(), Some(Ci::Failed));
        assert_eq!(of(3, 1, 1).total(), 5);
        assert_eq!(
            [Ci::Passed.mark(), Ci::Failed.mark(), Ci::Running.mark()],
            ["✓", "✗", "…"]
        );
    }

    #[test]
    fn only_an_https_address_without_blanks_is_opened() {
        let with = |url: &str| PullRequest {
            number: 1,
            title: String::new(),
            state: State::Open,
            base: "main".into(),
            url: url.into(),
            checks: Checks::default(),
        };
        let page = "https://github.com/acme/widgets/pull/1";
        assert_eq!(with(page).link(), Some(page));
        for refused in [
            "",
            "-a Calculator",
            "file:///etc/passwd",
            "http://github.com/acme/widgets/pull/1",
            "https://github.com/acme/widgets/pull/1 -a Calculator",
            "https://github.com/acme\n/widgets",
        ] {
            assert_eq!(with(refused).link(), None, "{refused:?}");
        }
    }

    #[test]
    fn what_gh_says_when_it_fails_becomes_a_plain_reason() {
        for (stderr, expected) in [
            (
                "To get started with GitHub CLI, please run:  gh auth login\n",
                Unavailable::NotSignedIn,
            ),
            ("HTTP 401: Bad credentials (https://api.github.com/graphql)", Unavailable::NotSignedIn),
            (
                "none of the git remotes configured for this repository point to a known GitHub host. To tell gh about a new GitHub host, please use `gh auth login`",
                Unavailable::NotGitHub,
            ),
            ("no git remotes found\n", Unavailable::NotGitHub),
            (
                "error connecting to api.github.com\ncheck your internet connection or https://githubstatus.com\n",
                Unavailable::Offline,
            ),
            (
                "Post \"https://api.github.com/graphql\": dial tcp: lookup api.github.com: no such host",
                Unavailable::Offline,
            ),
            (
                "\n  GraphQL: Resource not accessible\u{1b}[0m\nmore\n",
                Unavailable::Failed("GraphQL: Resource not accessible·[0m".into()),
            ),
            ("", Unavailable::Failed("nothing, and failed".into())),
        ] {
            assert_eq!(refusal(stderr), expected, "{stderr:?}");
        }
        let timed_out = |millis| Unavailable::TimedOut(Duration::from_millis(millis)).why();
        assert_eq!(timed_out(5000), "gh did not answer within 5 seconds");
        assert_eq!(timed_out(1000), "gh did not answer within 1 second");
        assert_eq!(timed_out(200), "gh did not answer within 0.2 seconds");
        assert_eq!(Unavailable::Failed("boom".into()).why(), "gh said: boom");
    }

    #[test]
    fn the_program_is_gh_at_the_default_timeout() {
        let gh = Gh::at("gh");
        assert_eq!(
            (gh.program.as_os_str(), gh.timeout),
            ("gh".as_ref(), TIMEOUT)
        );
        assert_eq!(
            LIST_PULL_REQUESTS.join(" "),
            "pr list --state all --limit 100 --json number,title,state,headRefName,baseRefName,url,statusCheckRollup"
        );
    }
}
