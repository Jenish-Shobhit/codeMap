use std::path::Path;

use super::{run, GitError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub parents: Vec<String>,
    pub refs: Vec<String>,
    pub author: String,
    pub time: i64,
    pub subject: String,
}

impl Commit {
    pub fn short(&self) -> &str {
        &self.sha[..self.sha.len().min(7)]
    }

    pub fn is_head(&self) -> bool {
        self.refs
            .iter()
            .any(|r| r == "HEAD" || r.starts_with("HEAD -> "))
    }

    pub fn labels(&self) -> Vec<String> {
        self.refs
            .iter()
            .map(|r| r.strip_prefix("HEAD -> ").unwrap_or(r).to_string())
            .filter(|r| r != "HEAD")
            .collect()
    }
}

const FIELD: char = '\u{1f}';
const RECORD: char = '\u{1e}';

pub fn log(root: &Path, limit: usize) -> Result<Vec<Commit>, GitError> {
    let n = format!("-n{limit}");
    let out = run(
        root,
        &[
            "log",
            "--all",
            "--topo-order",
            "--decorate=short",
            &n,
            "--format=%H%x1f%P%x1f%D%x1f%an%x1f%at%x1f%s%x1e",
        ],
    )?;
    Ok(parse_log(&out))
}

pub fn parse_log(out: &str) -> Vec<Commit> {
    out.split(RECORD).filter_map(parse_commit).collect()
}

fn parse_commit(rec: &str) -> Option<Commit> {
    let rec = rec.trim_start_matches('\n');
    if rec.trim().is_empty() {
        return None;
    }
    let mut f = rec.split(FIELD);
    let sha = f.next()?.trim().to_string();
    if sha.len() < 7 {
        return None;
    }
    let parents = f
        .next()
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let refs = f
        .next()
        .unwrap_or("")
        .split(", ")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    let author = f.next().unwrap_or("").to_string();
    let time = f.next().unwrap_or("0").trim().parse().unwrap_or(0);
    let subject = f.next().unwrap_or("").trim_end().to_string();
    Some(Commit {
        sha,
        parents,
        refs,
        author,
        time,
        subject,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCommit {
    pub commit: Commit,
    pub paths: Vec<String>,
}

pub fn renamed_from(paths: &[String]) -> Option<&str> {
    match paths {
        [from, to] if from != to => Some(from),
        _ => None,
    }
}

pub fn file_log(root: &Path, path: &str, limit: usize) -> Result<Vec<FileCommit>, GitError> {
    let n = format!("-n{limit}");
    let out = run(
        root,
        &[
            "--literal-pathspecs",
            "log",
            "--follow",
            "--decorate=short",
            "--name-status",
            "-z",
            &n,
            "--format=%x1e%H%x1f%P%x1f%D%x1f%an%x1f%at%x1f%s",
            "--",
            path,
        ],
    )?;
    Ok(parse_file_log(&out, path))
}

pub fn parse_file_log(out: &str, path: &str) -> Vec<FileCommit> {
    let mut known_as = path.to_string();
    let mut commits = Vec::new();
    for rec in out.split(RECORD) {
        let (head, changes) = rec.split_once('\0').unwrap_or((rec, ""));
        let Some(commit) = parse_commit(head) else {
            continue;
        };
        let mut paths = changed_paths(changes.strip_prefix('\n').unwrap_or(changes));
        match paths.first() {
            Some(older_name) => known_as = older_name.clone(),
            None => paths.push(known_as.clone()),
        }
        commits.push(FileCommit { commit, paths });
    }
    commits
}

fn changed_paths(name_status: &str) -> Vec<String> {
    let mut fields = name_status.split('\0').filter(|f| !f.is_empty());
    let mut paths = Vec::new();
    while let Some(status) = fields.next() {
        let named = if status.starts_with(['R', 'C']) { 2 } else { 1 };
        paths.extend(fields.by_ref().take(named).map(str::to_string));
    }
    paths
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRow {
    pub cells: Vec<char>,
    pub colors: Vec<usize>,
    pub commit: Option<usize>,
}

impl GraphRow {
    pub fn text(&self) -> String {
        self.cells.iter().collect::<String>().trim_end().to_string()
    }
}

pub fn layout(commits: &[Commit]) -> Vec<GraphRow> {
    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut lane_color: Vec<usize> = Vec::new();
    let mut next_color = 0usize;
    let mut rows = Vec::new();

    for (idx, commit) in commits.iter().enumerate() {
        let expecting: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(_, l)| l.as_deref() == Some(commit.sha.as_str()))
            .map(|(i, _)| i)
            .collect();
        let col = match expecting.first() {
            Some(&c) => c,
            None => {
                let free = lanes.iter().position(Option::is_none);
                match free {
                    Some(f) => {
                        lanes[f] = Some(commit.sha.clone());
                        lane_color[f] = next_color;
                        next_color += 1;
                        f
                    }
                    None => {
                        lanes.push(Some(commit.sha.clone()));
                        lane_color.push(next_color);
                        next_color += 1;
                        lanes.len() - 1
                    }
                }
            }
        };

        if expecting.len() > 1 {
            let last = *expecting.last().unwrap();
            let mut row = blank_row(&lanes, &lane_color);
            for (j, lane) in lanes.iter().enumerate().take(last + 1).skip(col) {
                let cell = j * 2;
                let glyph = if j == col {
                    '├'
                } else if j == last {
                    '╯'
                } else if expecting.contains(&j) {
                    '┴'
                } else if lane.is_some() {
                    '┼'
                } else {
                    '─'
                };
                row.cells[cell] = glyph;
                if j < last {
                    row.cells[cell + 1] = '─';
                    row.colors[cell + 1] = lane_color[expecting[1]];
                }
                if j != col {
                    row.colors[cell] = lane_color[expecting[1]];
                }
            }
            rows.push(row);
            for &j in &expecting[1..] {
                lanes[j] = None;
            }
        }

        let mut row = blank_row(&lanes, &lane_color);
        row.cells[col * 2] = '●';
        row.commit = Some(idx);
        rows.push(row);

        let mut parents = commit.parents.iter();
        lanes[col] = parents.next().cloned();
        let extra: Vec<String> = parents.cloned().collect();
        if !extra.is_empty() {
            let mut targets = Vec::new();
            for p in &extra {
                if let Some(existing) = lanes.iter().position(|l| l.as_deref() == Some(p.as_str()))
                {
                    targets.push((existing, false));
                } else {
                    let free = lanes
                        .iter()
                        .enumerate()
                        .skip(col + 1)
                        .find(|(_, l)| l.is_none())
                        .map(|(i, _)| i);
                    let slot = match free {
                        Some(f) => f,
                        None => {
                            lanes.push(None);
                            lane_color.push(0);
                            lanes.len() - 1
                        }
                    };
                    lanes[slot] = Some(p.clone());
                    lane_color[slot] = next_color;
                    next_color += 1;
                    targets.push((slot, true));
                }
            }
            let mut row = blank_row(&lanes, &lane_color);
            for (t, opened) in targets {
                let (lo, hi) = if t > col { (col, t) } else { (t, col) };
                for (j, lane) in lanes.iter().enumerate().take(hi + 1).skip(lo) {
                    let cell = j * 2;
                    let glyph = if j == col {
                        if t > col {
                            '├'
                        } else {
                            '┤'
                        }
                    } else if j == t {
                        if opened {
                            if t > col {
                                '╮'
                            } else {
                                '╭'
                            }
                        } else if t > col {
                            '┤'
                        } else {
                            '├'
                        }
                    } else if lane.is_some() {
                        '┼'
                    } else {
                        '─'
                    };
                    row.cells[cell] = glyph;
                    if j < hi {
                        row.cells[cell + 1] = '─';
                        row.colors[cell + 1] = lane_color[t];
                    }
                    if j != col {
                        row.colors[cell] = lane_color[t];
                    }
                }
            }
            rows.push(row);
        }
        while lanes.last().is_some_and(Option::is_none) {
            lanes.pop();
            lane_color.pop();
        }
    }
    rows
}

fn blank_row(lanes: &[Option<String>], lane_color: &[usize]) -> GraphRow {
    let width = lanes.len().max(1) * 2;
    let mut cells = vec![' '; width];
    let mut colors = vec![0; width];
    for (j, lane) in lanes.iter().enumerate() {
        if lane.is_some() {
            cells[j * 2] = '│';
        }
        colors[j * 2] = lane_color.get(j).copied().unwrap_or(0);
        colors[j * 2 + 1] = colors[j * 2];
    }
    GraphRow {
        cells,
        colors,
        commit: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(sha: &str, parents: &[&str]) -> Commit {
        Commit {
            sha: format!("{sha:0<40}"),
            parents: parents.iter().map(|p| format!("{p:0<40}")).collect(),
            refs: vec![],
            author: "a".into(),
            time: 0,
            subject: sha.into(),
        }
    }

    #[test]
    fn linear_history_is_one_lane() {
        let commits = vec![c("c3", &["c2"]), c("c2", &["c1"]), c("c1", &[])];
        let rows = layout(&commits);
        let text: Vec<String> = rows.iter().map(GraphRow::text).collect();
        assert_eq!(text, vec!["●", "●", "●"]);
    }

    #[test]
    fn branches_join_on_their_base() {
        let commits = vec![
            c("a1", &["b0"]),
            c("a2", &["b0"]),
            c("a3", &["b0"]),
            c("b0", &["z"]),
            c("z", &[]),
        ];
        let rows = layout(&commits);
        let text: Vec<String> = rows.iter().map(GraphRow::text).collect();
        assert_eq!(text[0], "●");
        assert_eq!(text[1], "│ ●");
        assert_eq!(text[2], "│ │ ●");
        assert_eq!(text[3], "├─┴─╯");
        assert_eq!(text[4], "●");
        assert_eq!(rows.iter().filter(|r| r.commit.is_some()).count(), 5);
    }

    #[test]
    fn merge_opens_a_lane() {
        let commits = vec![
            c("m", &["a", "b"]),
            c("b", &["base"]),
            c("a", &["base"]),
            c("base", &[]),
        ];
        let rows = layout(&commits);
        let text: Vec<String> = rows.iter().map(GraphRow::text).collect();
        assert_eq!(text[0], "●");
        assert_eq!(text[1], "├─╮");
        assert_eq!(text[2], "│ ●");
        assert_eq!(text[3], "● │");
        assert_eq!(text[4], "├─╯");
        assert_eq!(text[5], "●");
    }

    #[test]
    fn a_file_log_keeps_the_name_the_file_had_in_each_commit() {
        let sha = |c: char| c.to_string().repeat(40);
        let record = |c: char, subject: &str, changes: &str| {
            format!(
                "\u{1e}{}\u{1f}\u{1f}\u{1f}Test\u{1f}1789000000\u{1f}{subject}\0\n{changes}",
                sha(c)
            )
        };
        let out = [
            record('4', "edit", "M\0new name.rs\0"),
            record('3', "merge", ""),
            record('2', "rename", "R100\0[a]*.rs\0new name.rs\0"),
            record('1', "add", "A\0[a]*.rs\0"),
        ]
        .concat();
        let commits = parse_file_log(&out, "new name.rs");
        let shown: Vec<(&str, Vec<&str>, Option<&str>)> = commits
            .iter()
            .map(|c| {
                (
                    c.commit.subject.as_str(),
                    c.paths.iter().map(String::as_str).collect(),
                    renamed_from(&c.paths),
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                ("edit", vec!["new name.rs"], None),
                ("merge", vec!["new name.rs"], None),
                ("rename", vec!["[a]*.rs", "new name.rs"], Some("[a]*.rs")),
                ("add", vec!["[a]*.rs"], None),
            ]
        );
        assert_eq!(commits[0].commit.sha, sha('4'));
        assert!(parse_file_log("", "a.rs").is_empty());
    }

    #[test]
    fn parses_log_records() {
        let out = "1111111111111111111111111111111111111111\u{1f}2222222222222222222222222222222222222222\u{1f}HEAD -> main, tag: v1\u{1f}Jenish\u{1f}1789000000\u{1f}fix: things\u{1e}\n";
        let commits = parse_log(out);
        assert_eq!(commits.len(), 1);
        assert!(commits[0].is_head());
        assert_eq!(commits[0].labels(), vec!["main", "tag: v1"]);
        assert_eq!(commits[0].short(), "1111111");
    }
}
