mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use codemap::gh::{self, Checks, Ci, Gh, PullRequest, State, Unavailable};
use common::*;

fn pull_requests() -> String {
    fixture("gh/pull_requests.json")
}

fn checks(passed: usize, failed: usize, running: usize) -> Checks {
    Checks {
        passed,
        failed,
        running,
    }
}

#[test]
fn the_list_from_gh_becomes_one_pull_request_per_branch() {
    let pulls = gh::parse(&pull_requests()).unwrap();
    let branches: Vec<&str> = pulls.keys().map(String::as_str).collect();
    assert_eq!(
        branches,
        ["feat/ahead", "feat/both", "local-only", "merged"],
        "a state this version does not know is left out"
    );
    assert_eq!(
        pulls["feat/ahead"],
        PullRequest {
            number: 14,
            title: "feat: count ahead of the remote".into(),
            state: State::Open,
            base: "main".into(),
            url: "https://github.com/acme/widgets/pull/14".into(),
            checks: checks(3, 0, 0),
        }
    );
    assert_eq!(pulls["feat/ahead"].checks.ci(), Some(Ci::Passed));
}

#[test]
fn checks_pass_fail_or_are_still_running() {
    let pulls = gh::parse(&pull_requests()).unwrap();
    let failing = &pulls["feat/both"];
    assert_eq!(failing.checks, checks(1, 1, 1));
    assert_eq!(
        failing.checks.ci(),
        Some(Ci::Failed),
        "a failure shows while others run"
    );
    let pending = &pulls["local-only"];
    assert_eq!(pending.checks, checks(0, 0, 2));
    assert_eq!(pending.checks.ci(), Some(Ci::Running));
    assert_eq!(pending.base, "release/1.x");
    let merged = &pulls["merged"];
    assert_eq!(
        (merged.checks, merged.checks.ci()),
        (checks(1, 0, 0), Some(Ci::Passed))
    );
}

#[test]
fn of_several_pull_requests_an_open_one_wins_and_otherwise_the_newest() {
    let pulls = gh::parse(&pull_requests()).unwrap();
    assert_eq!(
        (pulls["feat/both"].number, pulls["feat/both"].state),
        (12, State::Open),
        "the open one, though a closed one is newer"
    );
    assert_eq!(
        (pulls["merged"].number, pulls["merged"].state),
        (8, State::Merged),
        "the newest of a closed and two merged ones"
    );
    let two_open = r#"[
        {"number": 3, "state": "OPEN", "headRefName": "x"},
        {"number": 7, "state": "OPEN", "headRefName": "x"},
        {"number": 5, "state": "OPEN", "headRefName": "x"}
    ]"#;
    assert_eq!(gh::parse(two_open).unwrap()["x"].number, 7);
    let closed =
        r#"[{"number": 4, "state": "CLOSED", "headRefName": "x", "statusCheckRollup": null}]"#;
    let only = &gh::parse(closed).unwrap()["x"];
    assert_eq!((only.state, only.checks.ci()), (State::Closed, None));
    assert_eq!((only.title.as_str(), only.link()), ("", None));
}

#[test]
fn output_that_is_not_the_list_is_unreadable() {
    assert_eq!(gh::parse("[]").unwrap().len(), 0);
    for garbage in [
        "",
        "not json",
        "{}",
        "[1, 2]",
        r#"{"message": "Bad credentials"}"#,
        r#"[{"number": "seven", "state": "OPEN", "headRefName": "x"}]"#,
        r#"[{"state": "OPEN", "headRefName": "x"}]"#,
        r#"[{"number": 1, "state": "OPEN", "headRefName": "x"}"#,
    ] {
        assert_eq!(
            gh::parse(garbage),
            Err(Unavailable::Unreadable),
            "{garbage:?}"
        );
    }
}

fn github_repo() -> Remoted {
    repo_with_origin()
}

#[test]
fn gh_is_run_in_the_repository_with_its_prompts_off_and_no_input() {
    let remoted = github_repo();
    let bin = TempDir::new("gh-bin");
    write(bin.path(), "pull-requests.json", &pull_requests());
    let gh = fake_gh(
        bin.path(),
        r#"here=$(dirname "$0")
{
  echo "args=$*"
  echo "cwd=$(pwd -P)"
  echo "prompt=$GH_PROMPT_DISABLED notifier=$GH_NO_UPDATE_NOTIFIER"
  if read -r line; then echo "stdin=open"; else echo "stdin=closed"; fi
} > "$here/seen"
exec cat "$here/pull-requests.json""#,
    );
    let pulls = gh.pull_requests(&remoted.root).unwrap();
    assert_eq!(pulls.len(), 4);
    assert_eq!(pulls["feat/ahead"].number, 14);
    let seen = std::fs::read_to_string(bin.path().join("seen")).unwrap();
    assert_eq!(
        seen,
        format!(
            "args=pr list --state all --limit 100 --json number,title,state,headRefName,baseRefName,url,statusCheckRollup\ncwd={}\nprompt=1 notifier=1\nstdin=closed\n",
            remoted.root.display()
        )
    );
}

#[test]
fn a_gh_that_is_not_installed_is_a_reason_not_an_error() {
    let remoted = github_repo();
    let nowhere = TempDir::new("gh-missing");
    for program in [
        nowhere.path().join("gh"),
        Path::new("codemap-test-no-such-gh-on-the-path").to_path_buf(),
    ] {
        assert_eq!(
            Gh::at(program).pull_requests(&remoted.root),
            Err(Unavailable::NotInstalled)
        );
    }
    assert_eq!(Unavailable::NotInstalled.why(), "gh is not installed");
}

#[test]
fn a_gh_that_fails_says_why_in_plain_words() {
    let remoted = github_repo();
    let bin = TempDir::new("gh-fails");
    let failing = |stderr: &str, code: u8| {
        let gh = fake_gh(
            bin.path(),
            &format!("echo '[]'\necho '{stderr}' >&2\nexit {code}"),
        );
        gh.pull_requests(&remoted.root).unwrap_err()
    };
    let not_signed_in = failing(
        "To get started with GitHub CLI, please run:  gh auth login",
        4,
    );
    assert_eq!(not_signed_in, Unavailable::NotSignedIn);
    assert_eq!(not_signed_in.why(), "gh is not signed in to GitHub");
    let not_github = failing(
        "none of the git remotes configured for this repository point to a known GitHub host",
        1,
    );
    assert_eq!(not_github.why(), "this repository is not on GitHub");
    let offline = failing("error connecting to api.github.com", 1);
    assert_eq!(offline.why(), "gh could not reach GitHub");
    let other = failing("GraphQL: Could not resolve to a Repository", 1);
    assert_eq!(
        other.why(),
        "gh said: GraphQL: Could not resolve to a Repository"
    );
    assert_eq!(failing("", 1).why(), "gh said: nothing, and failed");
}

#[test]
fn a_gh_that_prints_something_else_is_unreadable() {
    let remoted = github_repo();
    let bin = TempDir::new("gh-garbage");
    for printed in ["", "<html>503</html>", "{\"data\": null}"] {
        let gh = gh_printing(bin.path(), printed);
        let unreadable = gh.pull_requests(&remoted.root).unwrap_err();
        assert_eq!(unreadable, Unavailable::Unreadable, "{printed:?}");
        assert_eq!(
            unreadable.why(),
            "gh printed something codeMap could not read"
        );
    }
    let cannot_run = TempDir::new("gh-not-a-program");
    write(cannot_run.path(), "gh", "not a program\n");
    let refused = Gh::at(cannot_run.path().join("gh"))
        .pull_requests(&remoted.root)
        .unwrap_err();
    assert!(matches!(refused, Unavailable::NotStarted(_)), "{refused:?}");
    assert!(
        refused.why().starts_with("gh could not be run: "),
        "{refused:?}"
    );
}

fn anyone_in_process_group(group: i32) -> bool {
    unsafe { libc::kill(-group, 0) == 0 }
}

#[test]
fn a_gh_that_does_not_answer_is_killed_at_the_timeout() {
    let remoted = github_repo();
    let bin = TempDir::new("gh-slow");
    let quick_until_told = fake_gh(
        bin.path(),
        r#"here=$(dirname "$0")
[ -f "$here/be-slow" ] || { echo '[]'; exit 0; }
echo $$ > "$here/pid"
sleep 30
echo '[]'"#,
    );
    assert_eq!(
        quick_until_told
            .pull_requests(&remoted.root)
            .map(|p| p.len()),
        Ok(0),
        "the same program answers when it is quick"
    );
    write(bin.path(), "be-slow", "");
    let slow = Gh {
        timeout: Duration::from_millis(300),
        ..quick_until_told
    };
    let started = Instant::now();
    let timed_out = slow.pull_requests(&remoted.root).unwrap_err();
    let took = started.elapsed();
    assert_eq!(timed_out, Unavailable::TimedOut(Duration::from_millis(300)));
    assert!(
        took >= Duration::from_millis(300) && took < Duration::from_secs(10),
        "gave up after {took:?}"
    );
    assert_eq!(
        Unavailable::TimedOut(gh::TIMEOUT).why(),
        "gh did not answer within 5 seconds"
    );
    let script: i32 = std::fs::read_to_string(bin.path().join("pid"))
        .expect("the fake was running when it was stopped")
        .trim()
        .parse()
        .unwrap();
    let gone_by = Instant::now() + Duration::from_secs(5);
    while anyone_in_process_group(script) && Instant::now() < gone_by {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !anyone_in_process_group(script),
        "the script and the sleep it started were killed"
    );
}

#[test]
fn a_repository_without_a_remote_never_runs_gh() {
    let local = repo_with(&[("a.py", "x = 1\n")]);
    let bin = TempDir::new("gh-unused");
    let gh = fake_gh(bin.path(), "touch \"$(dirname \"$0\")/ran\"\necho '[]'");
    let no_remote = gh.pull_requests(local.path()).unwrap_err();
    assert_eq!(no_remote, Unavailable::NoRemote);
    assert_eq!(no_remote.why(), "this repository has no remote");
    assert!(!bin.path().join("ran").exists());
}

#[test]
fn asking_gh_writes_nothing_into_the_repository() {
    let remoted = github_repo();
    let bin = TempDir::new("gh-reads");
    let before = (fingerprint(&remoted.root), tree_bytes(&remoted.root));
    let gh = gh_printing(bin.path(), &pull_requests());
    gh.pull_requests(&remoted.root).unwrap();
    Gh::at(bin.path().join("missing"))
        .pull_requests(&remoted.root)
        .unwrap_err();
    assert_eq!(
        before,
        (fingerprint(&remoted.root), tree_bytes(&remoted.root))
    );
}
