use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::Path;

use super::diff::unquote;
use super::graph::is_object_id;
use super::{run, GitError, IndexCopy, RepoInfo};

pub const BLAME_BYTES: u64 = 1024 * 1024;
const BINARY_PROBE: usize = 8_000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlameCommit {
    pub sha: String,
    pub author: String,
    pub time: i64,
    pub summary: String,
    pub filename: String,
    pub previous: Option<String>,
}

impl BlameCommit {
    pub fn short(&self) -> &str {
        &self.sha[..self.sha.len().min(7)]
    }

    pub fn paths(&self) -> Vec<String> {
        let mut paths: Vec<String> = self.previous.iter().cloned().collect();
        if !paths.contains(&self.filename) {
            paths.push(self.filename.clone());
        }
        paths
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameLine {
    pub number: u32,
    pub commit: Option<usize>,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Blamed {
    pub commits: Vec<BlameCommit>,
    pub lines: Vec<BlameLine>,
}

impl Blamed {
    pub fn commit_of(&self, line: &BlameLine) -> Option<&BlameCommit> {
        line.commit.and_then(|c| self.commits.get(c))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlameRead {
    Lines(Blamed),
    NotCommitted,
    NotRegular,
    Deleted,
    Binary,
    TooLarge(u64),
    Empty,
}

fn is_not_committed(sha: &str) -> bool {
    sha.bytes().all(|b| b == b'0')
}

fn header(line: &str) -> Option<(&str, u32)> {
    let mut fields = line.split(' ');
    let sha = fields.next()?;
    let _original: u32 = fields.next()?.parse().ok()?;
    let number: u32 = fields.next()?.parse().ok()?;
    let group = fields.next();
    if fields.next().is_some() || group.is_some_and(|n| n.parse::<u32>().is_err()) {
        return None;
    }
    (is_object_id(sha) && number > 0).then_some((sha, number))
}

fn previous_name(value: &str) -> Option<String> {
    let (sha, name) = value.split_once(' ')?;
    is_object_id(sha).then(|| unquote(name))
}

pub fn parse_porcelain(out: &str) -> Blamed {
    let mut blamed = Blamed::default();
    let mut known: HashMap<String, usize> = HashMap::new();
    let mut current: Option<(Option<usize>, u32)> = None;
    for line in out.split('\n') {
        if let Some(text) = line.strip_prefix('\t') {
            if let Some((commit, number)) = current.take() {
                let text = text.strip_suffix('\r').unwrap_or(text).to_string();
                blamed.lines.push(BlameLine {
                    number,
                    commit,
                    text,
                });
            }
            continue;
        }
        if let Some((sha, number)) = header(line) {
            let commit = (!is_not_committed(sha)).then(|| {
                *known.entry(sha.to_string()).or_insert_with(|| {
                    blamed.commits.push(BlameCommit {
                        sha: sha.to_string(),
                        ..Default::default()
                    });
                    blamed.commits.len() - 1
                })
            });
            current = Some((commit, number));
            continue;
        }
        let Some((Some(at), _)) = current else {
            continue;
        };
        let commit = &mut blamed.commits[at];
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "author" => commit.author = value.to_string(),
            "author-time" => commit.time = value.trim().parse().unwrap_or(0),
            "summary" => commit.summary = value.to_string(),
            "filename" => commit.filename = unquote(value),
            "previous" => commit.previous = previous_name(value),
            _ => {}
        }
    }
    blamed
}

fn in_head(root: &Path, path: &str) -> Result<bool, GitError> {
    let listed = run(
        root,
        &[
            "--literal-pathspecs",
            "ls-tree",
            "-z",
            "--name-only",
            "HEAD",
            "--",
            path,
        ],
    )?;
    Ok(listed.split('\0').any(|name| name == path))
}

fn committed_name<'a>(
    root: &Path,
    path: &'a str,
    was: Option<&'a str>,
) -> Result<Option<&'a str>, GitError> {
    if in_head(root, path)? {
        return Ok(Some(path));
    }
    match was {
        Some(old) if in_head(root, old)? => Ok(Some(old)),
        _ => Ok(None),
    }
}

fn worth_blaming(full: &Path, size: u64) -> Result<Option<BlameRead>, GitError> {
    if size > BLAME_BYTES {
        return Ok(Some(BlameRead::TooLarge(size)));
    }
    let bytes = std::fs::read(full).map_err(|e| GitError(e.to_string()))?;
    if bytes.is_empty() {
        return Ok(Some(BlameRead::Empty));
    }
    if bytes.iter().take(BINARY_PROBE).any(|b| *b == 0) {
        return Ok(Some(BlameRead::Binary));
    }
    Ok(None)
}

pub fn blame(
    info: &RepoInfo,
    scratch: &Path,
    path: &str,
    was: Option<&str>,
) -> Result<BlameRead, GitError> {
    if info.head.is_none() {
        return Ok(BlameRead::NotCommitted);
    }
    let full = info.root.join(path);
    let meta = match std::fs::symlink_metadata(&full) {
        Ok(meta) => meta,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(BlameRead::Deleted),
        Err(e) => return Err(GitError(e.to_string())),
    };
    if meta.is_dir() {
        return Err(GitError(format!("{path} is a folder")));
    }
    let is_link = meta.file_type().is_symlink();
    if !is_link && !meta.file_type().is_file() {
        return Ok(BlameRead::NotRegular);
    }
    let Some(name) = committed_name(&info.root, path, was)? else {
        return Ok(BlameRead::NotCommitted);
    };
    if is_link && name != path {
        return Ok(BlameRead::NotCommitted);
    }
    if !is_link {
        if let Some(refused) = worth_blaming(&full, meta.len())? {
            return Ok(refused);
        }
    }
    let contents = format!("--contents={}", full.display());
    let mut args = vec![
        "--literal-pathspecs",
        "blame",
        "--porcelain",
        "--no-textconv",
    ];
    if name != path {
        args.push(&contents);
    }
    args.extend(["--", name]);
    let index = IndexCopy::new(info, scratch, "blame")?;
    let out = index.git(&args)?;
    let blamed = parse_porcelain(&String::from_utf8_lossy(&out));
    if blamed.lines.is_empty() {
        return Ok(BlameRead::Empty);
    }
    Ok(BlameRead::Lines(blamed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_header_needs_a_full_object_id_and_line_numbers() {
        let sha = "a".repeat(40);
        assert_eq!(header(&format!("{sha} 3 4 2")), Some((sha.as_str(), 4)));
        assert_eq!(header(&format!("{sha} 3 4")), Some((sha.as_str(), 4)));
        let long = "b".repeat(64);
        assert_eq!(header(&format!("{long} 1 1 1")), Some((long.as_str(), 1)));
        for bad in [
            format!("{} 1 1 1", "a".repeat(39)),
            format!("--output=x{} 1 1 1", "a".repeat(30)),
            format!("{sha} 1 0 1"),
            format!("{sha} x 1 1"),
            format!("{sha} 1 1 x"),
            format!("{sha} 1 1 1 1"),
            sha.clone(),
            "author 1 1 1".to_string(),
            String::new(),
        ] {
            assert_eq!(header(&bad), None, "{bad}");
        }
    }

    #[test]
    fn porcelain_without_a_valid_header_keeps_no_line() {
        let sha = "c".repeat(40);
        let out = format!(
            "not-an-id 1 1 1\nauthor Mallory\n\tdropped\n{sha} 1 2 1\nauthor A\nsummary s\nfilename \"x\\ty.rs\"\nprevious --output=z x.rs\n\tkept\n"
        );
        let blamed = parse_porcelain(&out);
        assert_eq!(blamed.lines.len(), 1);
        assert_eq!(blamed.lines[0].number, 2);
        assert_eq!(blamed.lines[0].text, "kept");
        assert_eq!(blamed.commits.len(), 1);
        assert_eq!(blamed.commits[0].author, "A");
        assert_eq!(blamed.commits[0].filename, "x\ty.rs");
        assert_eq!(blamed.commits[0].previous, None);
        assert_eq!(blamed.commits[0].paths(), ["x\ty.rs"]);
    }

    #[test]
    fn a_commit_names_the_file_before_and_after() {
        let commit = BlameCommit {
            filename: "new.rs".into(),
            previous: Some("old.rs".into()),
            ..Default::default()
        };
        assert_eq!(commit.paths(), ["old.rs", "new.rs"]);
        let same = BlameCommit {
            filename: "a.rs".into(),
            previous: Some("a.rs".into()),
            ..Default::default()
        };
        assert_eq!(same.paths(), ["a.rs"]);
    }
}
