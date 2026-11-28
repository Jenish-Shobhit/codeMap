use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::{bold, put, style, Areas, Line, Rail};
use crate::app::App;
use crate::branches::{Ahead, BranchesState, Pulls};
use crate::gh::{Ci, PullRequest, State};
use crate::git::branch::{Branch, Unique, Upstream};
use crate::theme::Theme;
use crate::util;

const LABEL: usize = 14;
const MOST_REMOTES: usize = 4;
const GAP: usize = 2;

type Spans = Vec<(String, Color)>;

fn spans_width(spans: &Spans) -> usize {
    spans.iter().map(|(text, _)| util::width(text)).sum()
}

pub fn render(buf: &mut Buffer, a: &Areas, app: &mut App) {
    let t = app.theme;
    let body = a.body;
    if let Some((text, color)) = notice(app) {
        put(buf, body.x, body.y, &text, style(color, t.body), body.width);
        return;
    }
    app.branches.scroll = list(buf, a.rail, app);
    let lines = match app.branches.selected() {
        Some(branch) => detail(app, branch, body),
        None => vec![Line::new().push(nothing(app), style(t.muted, t.body))],
    };
    for (row, line) in lines.iter().enumerate().take(body.height as usize) {
        line.draw(buf, body.x, body.y + row as u16, body.width);
    }
}

pub(super) fn hints(app: &App) -> Vec<(&'static str, String)> {
    let keys = ("?", "keys".to_string());
    if app.repo.is_none() || !app.branches.loaded {
        return vec![keys];
    }
    let Some(selected) = app.branches.selected() else {
        return vec![("n", "new branch".into()), keys];
    };
    let page = match app.branches.pull(&selected.name) {
        Some(_) => "pull request",
        None => "GitHub",
    };
    let mut hints = Vec::new();
    if !selected.current {
        hints.push(("⏎", "switch".to_string()));
    }
    hints.push(("n", "new".into()));
    if !selected.current {
        hints.extend([("m", "merge".to_string()), ("D", "delete".into())]);
    }
    hints.extend([("B", page.to_string()), ("y", "copy name".into()), keys]);
    hints
}

fn notice(app: &App) -> Option<(String, Color)> {
    let t = app.theme;
    if app.not_a_repo() {
        let text = format!("not a git repository · {}", util::tilde(&app.root));
        return Some((text, t.modified));
    }
    if !app.repo_known || !app.branches.loaded {
        return Some(("reading branches…".into(), t.muted));
    }
    app.branches
        .error
        .as_ref()
        .map(|e| (format!("git: {e}"), t.del))
}

fn nothing(app: &App) -> &'static str {
    if app.repo.as_ref().is_some_and(|repo| repo.is_empty()) {
        "no commits yet"
    } else {
        "no local branches"
    }
}

fn track(upstream: &Option<Upstream>, t: &Theme) -> Spans {
    let Some(upstream) = upstream else {
        return vec![("no upstream".into(), t.muted)];
    };
    if upstream.gone {
        return vec![("upstream gone".into(), t.modified)];
    }
    let mut spans = Spans::new();
    if upstream.ahead > 0 {
        spans.push((format!("↑{}", upstream.ahead), t.add));
    }
    if upstream.ahead > 0 && upstream.behind > 0 {
        spans.push((" ".into(), t.muted));
    }
    if upstream.behind > 0 {
        spans.push((format!("↓{}", upstream.behind), t.modified));
    }
    spans
}

fn ci_color(ci: Ci, t: &Theme) -> Color {
    match ci {
        Ci::Passed => t.add,
        Ci::Failed => t.del,
        Ci::Running => t.modified,
    }
}

fn pull_mark(pull: Option<&PullRequest>, t: &Theme) -> Spans {
    let Some(pull) = pull else {
        return Spans::new();
    };
    let mut spans = vec![(format!("#{}", pull.number), t.text2)];
    if let Some(ci) = pull.checks.ci() {
        spans.push((format!(" {}", ci.mark()), ci_color(ci, t)));
    }
    spans
}

struct Row {
    track: Spans,
    pull: Spans,
}

struct Columns {
    name: usize,
    track: usize,
}

fn columns(state: &BranchesState, rows: &[Row], width: usize) -> Columns {
    let widest =
        |of: fn(&Row) -> &Spans| rows.iter().map(|r| spans_width(of(r))).max().unwrap_or(0);
    let track = widest(|row| &row.track);
    let pull = widest(|row| &row.pull);
    let beside = [track, pull]
        .into_iter()
        .filter(|w| *w > 0)
        .map(|w| w + GAP)
        .sum::<usize>();
    let longest = state
        .local
        .iter()
        .map(|b| util::width(&b.name))
        .max()
        .unwrap_or(0);
    Columns {
        name: longest.min(width.saturating_sub(GAP + beside).max(6)),
        track,
    }
}

fn branch_line(
    branch: &Branch,
    row: &Row,
    at: &Columns,
    selected: bool,
    t: &Theme,
) -> Line<'static> {
    let bg = if selected { t.active_row } else { t.rail };
    let name = if selected {
        bold(style(t.text, bg))
    } else {
        style(t.text, bg)
    };
    let mark = if branch.current { "● " } else { "  " };
    let mut line = Line::new()
        .bg(bg)
        .push(mark, style(t.add, bg))
        .push(util::pad(&branch.name, at.name), name);
    if at.track > 0 {
        line = line.push(" ".repeat(GAP), style(t.muted, bg));
        for (text, color) in &row.track {
            line = line.push(text.clone(), style(*color, bg));
        }
        let rest = at.track.saturating_sub(spans_width(&row.track));
        line = line.push(" ".repeat(rest), style(t.muted, bg));
    }
    if !row.pull.is_empty() {
        line = line.push(" ".repeat(GAP), style(t.muted, bg));
    }
    for (text, color) in &row.pull {
        line = line.push(text.clone(), style(*color, bg));
    }
    line
}

fn top_showing(chosen: usize, top: usize, height: usize) -> usize {
    if chosen <= top {
        chosen.saturating_sub(1)
    } else if chosen >= top + height {
        chosen + 1 - height
    } else {
        top
    }
}

fn list(buf: &mut Buffer, area: Rect, app: &App) -> usize {
    let state = &app.branches;
    if area.width == 0 || area.height == 0 {
        return state.scroll;
    }
    let t = app.theme;
    let remotes = state.remotes.len().min(MOST_REMOTES);
    let below = if remotes > 0 { remotes + 1 } else { 0 };
    let height = (area.height as usize).saturating_sub(below).max(1);
    let top = top_showing(state.sel + 1, state.scroll.min(state.local.len()), height);
    let rows: Vec<Row> = state
        .local
        .iter()
        .map(|branch| Row {
            track: track(&branch.upstream, &t),
            pull: pull_mark(state.pull(&branch.name), &t),
        })
        .collect();
    let at = columns(state, &rows, (area.width as usize).saturating_sub(1));
    let mut rail = Rail::new(buf, area, t);
    if top == 0 {
        rail.section(&format!("local · {}", state.local.len()));
    }
    let first = top.saturating_sub(1);
    let room = height - usize::from(top == 0);
    for (i, branch) in state.local.iter().enumerate().skip(first).take(room) {
        rail.line(branch_line(branch, &rows[i], &at, i == state.sel, &t));
    }
    if remotes > 0 {
        rail.blank();
    }
    for remote in state.remotes.iter().take(remotes) {
        rail.line(
            Line::new()
                .push(remote.name.clone(), bold(style(t.text2, t.rail)))
                .push(
                    format!(" · {}", util::plural(remote.branches, "branch", "branches")),
                    style(t.muted, t.rail),
                ),
        );
    }
    top
}

fn labelled(label: &str, t: &Theme) -> Line<'static> {
    Line::new().push(util::pad(label, LABEL), style(t.text2, t.body))
}

fn muted(text: impl Into<String>, t: &Theme) -> Line<'static> {
    Line::new().push(text, style(t.muted, t.body))
}

fn heading(branch: &Branch, width: usize, t: &Theme) -> Line<'static> {
    let name = Line::new().push(branch.name.clone(), bold(style(t.text, t.body)));
    if branch.current {
        return name.push("    current branch", style(t.branch, t.body));
    }
    let Some(path) = &branch.elsewhere else {
        return name;
    };
    let said = "    checked out in ";
    let room = width.saturating_sub(name.width() + util::width(said));
    name.push(said, style(t.modified, t.body)).push(
        util::truncate_left(&util::tilde(path), room),
        style(t.modified, t.body),
    )
}

pub fn upstream_words(upstream: &Upstream) -> String {
    let commits = |n: usize| util::plural(n, "commit", "commits");
    let how_far = match (upstream.ahead, upstream.behind) {
        _ if upstream.gone => "gone".to_string(),
        (0, 0) => "in step".to_string(),
        (ahead, 0) => format!("{} ahead", commits(ahead)),
        (0, behind) => format!("{} behind", commits(behind)),
        (ahead, behind) => format!("{} ahead, {behind} behind", commits(ahead)),
    };
    format!("{} · {how_far}", upstream.name)
}

fn upstream_line(branch: &Branch, t: &Theme) -> Line<'static> {
    let line = labelled("upstream", t);
    match &branch.upstream {
        Some(upstream) => line.push(upstream_words(upstream), style(t.text, t.body)),
        None => line.push("none", style(t.muted, t.body)),
    }
}

fn state_color(state: State, t: &Theme) -> Color {
    match state {
        State::Open => t.add,
        State::Merged => t.accent,
        State::Closed => t.del,
    }
}

pub fn checks_words(pull: &PullRequest) -> String {
    match pull.checks.total() {
        0 => "no checks".into(),
        total => format!("{} of {total} checks passed", pull.checks.passed),
    }
}

fn pull_lines(state: &BranchesState, branch: &Branch, t: &Theme) -> Vec<Line<'static>> {
    let label = || labelled("pull request", t);
    let pulls = match &state.pulls {
        Pulls::NotAsked | Pulls::Reading => {
            return vec![label().push("reading…", style(t.muted, t.body))]
        }
        Pulls::Unknown(why) => {
            return vec![muted(
                format!("no pull request information · {}", util::sanitize_line(why)),
                t,
            )]
        }
        Pulls::Known(pulls) => pulls,
    };
    let Some(pull) = pulls.get(&branch.name) else {
        return vec![label().push("none", style(t.muted, t.body))];
    };
    let title = label()
        .push(format!("#{} ", pull.number), style(t.text2, t.body))
        .push(util::sanitize_line(&pull.title), style(t.text, t.body));
    let mut facts = labelled("", t)
        .push(pull.state.word(), style(state_color(pull.state, t), t.body))
        .push(
            format!(" · into {} · ", util::sanitize_line(&pull.base)),
            style(t.muted, t.body),
        )
        .push(checks_words(pull), style(t.text2, t.body));
    if let Some(ci) = pull.checks.ci() {
        facts = facts.push(format!(" {}", ci.mark()), style(ci_color(ci, t), t.body));
    }
    vec![title, facts]
}

fn commit_lines(unique: &Unique, default: &str, room: usize, t: &Theme) -> Vec<Line<'static>> {
    if unique.total == 0 {
        return vec![muted(format!("no commits that {default} does not have"), t)];
    }
    let title = format!(
        "{} not on {default}",
        util::plural(unique.total, "commit", "commits")
    );
    let mut lines = vec![Line::new().push(title, bold(style(t.text2, t.body)))];
    let room = room.saturating_sub(1);
    let fits = if unique.total > room {
        room.saturating_sub(1)
    } else {
        room
    };
    let shown = unique.commits.len().min(fits);
    for (sha, subject) in unique.commits.iter().take(shown) {
        lines.push(
            Line::new()
                .push(format!("{sha} "), style(t.muted, t.body))
                .push(util::sanitize_line(subject), style(t.text, t.body)),
        );
    }
    if unique.total > shown {
        lines.push(muted(format!("and {} more", unique.total - shown), t));
    }
    lines
}

fn ahead_lines(
    state: &BranchesState,
    branch: &Branch,
    room: usize,
    t: &Theme,
) -> Vec<Line<'static>> {
    let Some(default) = state.default_name() else {
        return vec![muted("no default branch to compare with", t)];
    };
    if state.is_default(branch) {
        return vec![muted(format!("{default} is the default branch"), t)];
    }
    match state.ahead.get(&branch.name) {
        None | Some(Ahead::Reading) => vec![muted("reading commits…", t)],
        Some(Ahead::Failed(e)) => {
            vec![Line::new().push(format!("git: {e}"), style(t.del, t.body))]
        }
        Some(Ahead::Known(unique)) => commit_lines(unique, default, room, t),
    }
}

fn detail(app: &App, branch: &Branch, body: Rect) -> Vec<Line<'static>> {
    let t = app.theme;
    let state = &app.branches;
    let mut lines = vec![
        heading(branch, body.width as usize, &t),
        Line::new(),
        upstream_line(branch, &t),
    ];
    lines.extend(pull_lines(state, branch, &t));
    lines.push(Line::new());
    let room = (body.height as usize).saturating_sub(lines.len());
    lines.extend(ahead_lines(state, branch, room, &t));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gh::Checks;

    fn upstream(ahead: usize, behind: usize) -> Upstream {
        Upstream {
            name: "origin/feat/x".into(),
            ahead,
            behind,
            gone: false,
        }
    }

    #[test]
    fn the_upstream_is_told_in_words() {
        assert_eq!(upstream_words(&upstream(0, 0)), "origin/feat/x · in step");
        assert_eq!(
            upstream_words(&upstream(1, 0)),
            "origin/feat/x · 1 commit ahead"
        );
        assert_eq!(
            upstream_words(&upstream(0, 3)),
            "origin/feat/x · 3 commits behind"
        );
        assert_eq!(
            upstream_words(&upstream(2, 1)),
            "origin/feat/x · 2 commits ahead, 1 behind"
        );
        let gone = Upstream {
            gone: true,
            ..upstream(0, 0)
        };
        assert_eq!(upstream_words(&gone), "origin/feat/x · gone");
    }

    #[test]
    fn checks_are_counted_as_passed_out_of_all() {
        let with = |passed, failed, running| PullRequest {
            number: 7,
            title: String::new(),
            state: State::Open,
            base: "main".into(),
            url: String::new(),
            checks: Checks {
                passed,
                failed,
                running,
            },
        };
        assert_eq!(checks_words(&with(0, 0, 0)), "no checks");
        assert_eq!(checks_words(&with(3, 1, 2)), "3 of 6 checks passed");
    }

    #[test]
    fn the_list_scrolls_to_keep_the_selection_and_its_title_in_view() {
        assert_eq!(top_showing(1, 0, 5), 0);
        assert_eq!(top_showing(5, 0, 5), 1, "down past the last row");
        assert_eq!(top_showing(4, 1, 5), 1, "already in view");
        assert_eq!(top_showing(2, 4, 5), 1, "up");
        assert_eq!(top_showing(1, 4, 5), 0, "the first branch brings the title");
    }

    #[test]
    fn commits_past_the_room_are_counted_not_listed() {
        let t = Theme::from_config_text("");
        let unique = |total: usize, listed: usize| Unique {
            total,
            commits: (0..listed)
                .map(|n| (format!("{n:07}"), format!("commit {n}")))
                .collect(),
        };
        let text = |lines: Vec<Line>| -> Vec<String> {
            lines
                .iter()
                .map(|l| l.spans.iter().map(|(s, _)| s.as_str()).collect())
                .collect()
        };
        assert_eq!(
            text(commit_lines(&unique(0, 0), "main", 10, &t)),
            ["no commits that main does not have"]
        );
        assert_eq!(
            text(commit_lines(&unique(2, 2), "main", 10, &t)),
            [
                "2 commits not on main",
                "0000000 commit 0",
                "0000001 commit 1"
            ]
        );
        assert_eq!(
            text(commit_lines(&unique(5, 5), "origin/main", 4, &t)),
            [
                "5 commits not on origin/main",
                "0000000 commit 0",
                "0000001 commit 1",
                "and 3 more"
            ]
        );
        assert_eq!(
            text(commit_lines(&unique(40, 3), "main", 10, &t))
                .last()
                .unwrap(),
            "and 37 more"
        );
        assert_eq!(
            text(commit_lines(&unique(1, 1), "main", 2, &t)),
            ["1 commit not on main", "0000000 commit 0"]
        );
    }
}
