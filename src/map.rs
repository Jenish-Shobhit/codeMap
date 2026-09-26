//! The Map: folders, files and symbols as boxes, calls and imports as lines.
//!
//! Levels zoom from a directory (its subfolders and files) into a file (its
//! functions and classes, plus the files it calls into and is called from).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::canvas::{Band, Canvas, Tone, DOWN};
use crate::git::{FileDiff, FileStatus};
use crate::index::{in_dir, parent_dir, Index};
use crate::lang::{self, Lang, SymKind};
use crate::layout::{self, LEdge, LNode, Layout, Options};
use crate::util;

/// Boxes per page before a level paginates.
pub const PAGE_SIZE: usize = 36;
const MAX_ROWS: usize = 8;
const MAX_BOX_W: usize = 46;
const MIN_BOX_W: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Level {
    Dir(String),
    File(String),
}

impl Level {
    pub fn parent(&self) -> Option<Level> {
        match self {
            Level::File(f) => Some(Level::Dir(parent_dir(f).to_string())),
            Level::Dir(d) if d.is_empty() => None,
            Level::Dir(d) => Some(Level::Dir(parent_dir(d).to_string())),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Level::Dir(d) if d.is_empty() => "./".to_string(),
            Level::Dir(d) => format!("{d}/"),
            Level::File(f) => f.clone(),
        }
    }
}

/// What changed, per file and per symbol, in the active scope.
#[derive(Debug, Clone, Default)]
pub struct Marks {
    pub files: HashMap<String, char>,
    /// file -> symbol index -> 'A' | 'M'
    pub symbols: HashMap<String, BTreeMap<usize, char>>,
    /// file -> set of changed new-side lines (for Flow)
    pub lines: HashMap<String, BTreeMap<u32, char>>,
}

impl Marks {
    pub fn file(&self, f: &str) -> Option<char> {
        self.files.get(f).copied()
    }

    pub fn symbol(&self, f: &str, idx: usize) -> Option<char> {
        self.symbols.get(f).and_then(|m| m.get(&idx).copied())
    }

    pub fn changed_under(&self, dir: &str) -> usize {
        self.files.keys().filter(|f| in_dir(f, dir)).count()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Changed callables, for the rail: (file, symbol index, mark).
    pub fn changed_symbols(&self, index: &Index) -> Vec<(String, usize, char)> {
        let mut out = Vec::new();
        for (file, syms) in &self.symbols {
            let Some(fs) = index.symbols(file) else {
                continue;
            };
            for (&i, &m) in syms {
                if fs.symbols.get(i).is_some_and(|s| s.kind.is_callable()) {
                    out.push((file.clone(), i, m));
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        out
    }
}

/// Symbol names of a source text, for `compute_marks`.
pub fn quals_of(path: &str, src: &str) -> Option<BTreeSet<String>> {
    let lang = Lang::from_path(path)?;
    Some(
        lang::extract(lang, src)
            .symbols
            .iter()
            .map(|s| s.qual())
            .collect(),
    )
}

/// Compute marks from a diff. `old_quals` gives the symbol names of the
/// base version of a file, so a symbol whose name is new is "A" even when
/// git aligned some of its lines with old ones.
pub fn compute_marks(
    index: &Index,
    diffs: &[FileDiff],
    old_quals: &dyn Fn(&FileDiff) -> Option<BTreeSet<String>>,
) -> Marks {
    let mut marks = Marks::default();
    for d in diffs {
        if d.status == FileStatus::Deleted {
            continue;
        }
        marks.files.insert(d.path.clone(), d.status.letter());
        let added = d.added_lines();
        let touched = d.touched_lines();
        let mut line_marks = BTreeMap::new();
        for n in &touched {
            line_marks.insert(*n, if added.contains(n) { 'A' } else { 'M' });
        }
        marks.lines.insert(d.path.clone(), line_marks);
        let Some(fs) = index.symbols(&d.path) else {
            continue;
        };
        let old_quals: Option<BTreeSet<String>> = if d.status == FileStatus::Added {
            Some(BTreeSet::new())
        } else {
            old_quals(d)
        };
        let mut sm = BTreeMap::new();
        for (i, s) in fs.symbols.iter().enumerate() {
            if s.kind == SymKind::Impl {
                continue;
            }
            let is_new = old_quals.as_ref().is_some_and(|q| !q.contains(&s.qual()));
            if is_new {
                sm.insert(i, 'A');
            } else if let Some(m) = d.mark_for_range(s.start_line, s.end_line) {
                // Containers are "M" when anything inside them changed.
                let m = if old_quals.is_some() && m == 'A' {
                    'M'
                } else {
                    m
                };
                if !s.kind.is_container()
                    || touched.range(s.start_line..=s.end_line).next().is_some()
                {
                    sm.insert(i, m);
                }
            }
        }
        marks.symbols.insert(d.path.clone(), sm);
    }
    marks
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Dir(String),
    File(String),
    /// A function, method or class of the file level.
    Symbol {
        file: String,
        idx: usize,
    },
    /// Another file, at the file level (callers and callees).
    External(String),
    /// "N more" when a level is paginated.
    More,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub text: String,
    pub mark: Option<char>,
    /// Symbol in `file` this row stands for (selectable rows).
    pub sym: Option<(String, usize)>,
    pub dim: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub kind: NodeKind,
    pub title: String,
    pub rows: Vec<Row>,
    pub mark: Option<char>,
    pub muted: bool,
}

impl Node {
    fn size(&self) -> (usize, usize) {
        let mut w = util::width(&self.title) + 6;
        for r in &self.rows {
            w = w.max(util::width(&r.text) + if r.mark.is_some() { 6 } else { 4 });
        }
        (w.clamp(MIN_BOX_W, MAX_BOX_W), self.rows.len().max(1) + 2)
    }

    /// The symbol this node stands for, if any.
    pub fn symbol(&self) -> Option<(String, usize)> {
        match &self.kind {
            NodeKind::Symbol { file, idx } => Some((file.clone(), *idx)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub calls: usize,
    pub names: Vec<String>,
    pub import_only: bool,
}

#[derive(Debug, Clone)]
pub struct Scene {
    pub level: Level,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub layout: Layout,
    pub page: usize,
    pub pages: usize,
    /// "7 modules · 38 functions · 4 changed"
    pub summary: String,
}

fn file_name(f: &str) -> &str {
    f.rsplit('/').next().unwrap_or(f)
}

fn dir_name(d: &str) -> String {
    format!("{}/", file_name(d))
}

fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Rows describing a file's top-level symbols.
fn file_rows(index: &Index, marks: &Marks, file: &str) -> (Vec<Row>, Option<String>) {
    let Some(fs) = index.symbols(file) else {
        let size = std::fs::metadata(index.root.join(file))
            .map(|m| m.len())
            .unwrap_or(0);
        let note = if Lang::from_path(file).is_some() {
            "not parsed yet".to_string()
        } else {
            human_size(size)
        };
        return (
            vec![Row {
                text: note,
                mark: None,
                sym: None,
                dim: true,
            }],
            None,
        );
    };
    let top = fs.top_level();
    let top: Vec<usize> = top
        .into_iter()
        .filter(|&i| fs.symbols[i].kind != SymKind::Impl)
        .collect();
    let mut title_extra = None;
    let mut rows = Vec::new();
    let only_container = top.len() == 1 && fs.symbols[top[0]].kind.is_container();
    let listed: Vec<usize> = if only_container {
        title_extra = Some(fs.symbols[top[0]].name.clone());
        methods_of(fs, top[0])
    } else {
        top.clone()
    };
    for &i in &listed {
        let s = &fs.symbols[i];
        let text = if s.kind.is_container() && !only_container {
            let n = methods_of(fs, i).len();
            if n > 0 {
                format!("{} · {}", s.name, util::plural(n, "method", "methods"))
            } else {
                s.name.clone()
            }
        } else {
            s.name.clone()
        };
        rows.push(Row {
            text,
            mark: marks.symbol(file, i),
            sym: Some((file.to_string(), i)),
            dim: s.name.starts_with('_') && s.kind.is_callable(),
        });
    }
    if rows.is_empty() {
        rows.push(Row {
            text: "no functions".into(),
            mark: None,
            sym: None,
            dim: true,
        });
    }
    // Changed rows first when the box must be cut.
    if rows.len() > MAX_ROWS {
        let extra = rows.len() - (MAX_ROWS - 1);
        let mut keep: Vec<Row> = rows.iter().filter(|r| r.mark.is_some()).cloned().collect();
        for r in &rows {
            if keep.len() >= MAX_ROWS - 1 {
                break;
            }
            if r.mark.is_none() {
                keep.push(r.clone());
            }
        }
        keep.truncate(MAX_ROWS - 1);
        // keep source order
        keep.sort_by_key(|r| r.sym.as_ref().map(|s| s.1).unwrap_or(usize::MAX));
        keep.push(Row {
            text: format!("… {extra} more"),
            mark: None,
            sym: None,
            dim: true,
        });
        rows = keep;
    }
    (rows, title_extra)
}

/// Methods of a container, including those in `impl` blocks of the same
/// type elsewhere in the file (Rust).
pub fn methods_of(fs: &lang::FileSymbols, container: usize) -> Vec<usize> {
    let name = &fs.symbols[container].name;
    let mut out: Vec<usize> = fs.children(container);
    for (i, s) in fs.symbols.iter().enumerate() {
        if s.kind == SymKind::Impl && &s.name == name && i != container {
            out.extend(fs.children(i));
        }
    }
    out.retain(|&i| fs.symbols[i].kind.is_callable());
    out.sort();
    out.dedup();
    out
}

/// Build the scene for a level.
pub fn build(index: &Index, marks: &Marks, level: &Level, page: usize, max_width: usize) -> Scene {
    let (nodes, edges, pages, summary) = match level {
        Level::Dir(dir) => build_dir(index, marks, dir, page),
        Level::File(file) => build_file(index, marks, file),
    };
    let lnodes: Vec<LNode> = nodes
        .iter()
        .map(|n| {
            let (w, h) = n.size();
            LNode { w, h }
        })
        .collect();
    let ledges: Vec<LEdge> = edges
        .iter()
        .map(|e| LEdge {
            from: e.from,
            to: e.to,
        })
        .collect();
    let layout = layout::layout(
        &lnodes,
        &ledges,
        &Options {
            max_width: max_width.max(40),
            ..Default::default()
        },
    );
    Scene {
        level: level.clone(),
        nodes,
        edges,
        layout,
        page,
        pages,
        summary,
    }
}

type Built = (Vec<Node>, Vec<Edge>, usize, String);

fn build_dir(index: &Index, marks: &Marks, dir: &str, page: usize) -> Built {
    let subdirs = index.subdirs(dir);
    let files = index.files_in(dir);
    let mut nodes: Vec<Node> = Vec::new();
    for (sub, count) in &subdirs {
        let changed = marks.changed_under(sub);
        let fns = index.function_count_under(sub);
        let mut rows = vec![Row {
            text: if fns > 0 {
                format!(
                    "{} · {}",
                    util::plural(*count, "file", "files"),
                    util::plural(fns, "function", "functions")
                )
            } else {
                util::plural(*count, "file", "files")
            },
            mark: None,
            sym: None,
            dim: true,
        }];
        if changed > 0 {
            rows.push(Row {
                text: format!("{changed} changed"),
                mark: Some('M'),
                sym: None,
                dim: false,
            });
        }
        nodes.push(Node {
            kind: NodeKind::Dir(sub.clone()),
            title: dir_name(sub),
            rows,
            mark: (changed > 0).then_some('M'),
            muted: false,
        });
    }
    for f in &files {
        let (rows, extra) = file_rows(index, marks, f);
        let title = match extra {
            Some(c) => format!("{} · {c}", file_name(f)),
            None => file_name(f).to_string(),
        };
        nodes.push(Node {
            kind: NodeKind::File((*f).clone()),
            title,
            rows,
            mark: marks.file(f).map(|m| if m == 'A' { 'A' } else { 'M' }),
            muted: Lang::from_path(f).is_none(),
        });
    }
    // Pagination: code first when cutting (dirs, source files, then others).
    let total = nodes.len();
    let pages = total.div_ceil(PAGE_SIZE).max(1);
    let page = page.min(pages - 1);
    if total > PAGE_SIZE {
        nodes.sort_by_key(|n| match &n.kind {
            NodeKind::Dir(_) => 0,
            NodeKind::File(f) if Lang::from_path(f).is_some() => 1,
            _ => 2,
        });
        let start = page * PAGE_SIZE;
        let rest = total.saturating_sub(start + PAGE_SIZE);
        nodes = nodes.into_iter().skip(start).take(PAGE_SIZE).collect();
        if rest > 0 {
            nodes.push(Node {
                kind: NodeKind::More,
                title: format!("{rest} more"),
                rows: vec![Row {
                    text: "] next page".into(),
                    mark: None,
                    sym: None,
                    dim: true,
                }],
                mark: None,
                muted: true,
            });
        }
    }
    // Which node holds a file?
    let owner = |file: &str| -> Option<usize> {
        nodes.iter().position(|n| match &n.kind {
            NodeKind::File(f) => f == file,
            NodeKind::Dir(d) => in_dir(file, d),
            _ => false,
        })
    };
    let mut agg: BTreeMap<(usize, usize), (usize, BTreeSet<String>, bool)> = BTreeMap::new();
    for c in &index.calls {
        if !in_dir(&c.from_file, dir) || !in_dir(&c.to.file, dir) {
            continue;
        }
        let (Some(a), Some(b)) = (owner(&c.from_file), owner(&c.to.file)) else {
            continue;
        };
        if a == b {
            continue;
        }
        let name = index
            .symbols(&c.to.file)
            .map(|s| s.symbols[c.to.idx].name.clone())
            .unwrap_or_default();
        let e = agg.entry((a, b)).or_default();
        e.0 += 1;
        e.1.insert(name);
    }
    for (from, tos) in &index.imports {
        if !in_dir(from, dir) {
            continue;
        }
        for to in tos {
            if !in_dir(to, dir) {
                continue;
            }
            let (Some(a), Some(b)) = (owner(from), owner(to)) else {
                continue;
            };
            if a == b {
                continue;
            }
            agg.entry((a, b)).or_insert((0, BTreeSet::new(), true));
        }
    }
    let edges: Vec<Edge> = agg
        .into_iter()
        .map(|((from, to), (calls, names, _))| Edge {
            from,
            to,
            calls,
            names: names.into_iter().collect(),
            import_only: calls == 0,
        })
        .collect();
    let modules = files
        .iter()
        .filter(|f| Lang::from_path(f).is_some())
        .count();
    let fns = index.function_count_under(dir);
    let changed = marks.changed_under(dir);
    let mut summary = vec![];
    if !subdirs.is_empty() {
        summary.push(util::plural(subdirs.len(), "folder", "folders"));
    }
    summary.push(util::plural(modules, "module", "modules"));
    if fns > 0 {
        summary.push(util::plural(fns, "function", "functions"));
    }
    if changed > 0 {
        summary.push(format!("{changed} changed"));
    }
    (nodes, edges, pages, summary.join(" · "))
}

fn build_file(index: &Index, marks: &Marks, file: &str) -> Built {
    let mut nodes: Vec<Node> = Vec::new();
    let Some(fs) = index.symbols(file) else {
        let (rows, _) = file_rows(index, marks, file);
        nodes.push(Node {
            kind: NodeKind::File(file.to_string()),
            title: file_name(file).to_string(),
            rows,
            mark: marks.file(file),
            muted: true,
        });
        return (nodes, Vec::new(), 1, "no grammar for this file".to_string());
    };
    let lang = fs.lang;
    // One node per top-level function and container.
    let top: Vec<usize> = fs
        .top_level()
        .into_iter()
        .filter(|&i| {
            fs.symbols[i].kind != SymKind::Impl
                || !fs.top_level().iter().any(|&j| {
                    j != i
                        && fs.symbols[j].name == fs.symbols[i].name
                        && fs.symbols[j].kind != SymKind::Impl
                })
        })
        .collect();
    let mut node_of_sym: HashMap<usize, usize> = HashMap::new();
    for &i in &top {
        let s = &fs.symbols[i];
        let mut rows = Vec::new();
        if s.kind.is_container() {
            for m in methods_of(fs, i) {
                let ms = &fs.symbols[m];
                rows.push(Row {
                    text: ms.name.clone(),
                    mark: marks.symbol(file, m),
                    sym: Some((file.to_string(), m)),
                    dim: ms.name.starts_with('_'),
                });
                node_of_sym.insert(m, nodes.len());
            }
            // Impl blocks of the same type map to this node too.
            for (j, other) in fs.symbols.iter().enumerate() {
                if other.kind == SymKind::Impl && other.name == s.name {
                    node_of_sym.insert(j, nodes.len());
                }
            }
            if rows.is_empty() {
                rows.push(Row {
                    text: format!(
                        "{} · lines {}–{}",
                        s.kind.keyword(lang),
                        s.start_line,
                        s.end_line
                    ),
                    mark: None,
                    sym: None,
                    dim: true,
                });
            }
        } else {
            let calls = index.calls_from(file, i).len();
            rows.push(Row {
                text: format!("lines {}–{}", s.start_line, s.end_line),
                mark: None,
                sym: None,
                dim: true,
            });
            if calls > 0 {
                rows.push(Row {
                    text: util::plural(calls, "call", "calls"),
                    mark: None,
                    sym: None,
                    dim: true,
                });
            }
        }
        node_of_sym.insert(i, nodes.len());
        let title = if s.kind.is_container() {
            format!("{} {}", s.kind.keyword(lang), s.name)
        } else {
            s.name.clone()
        };
        nodes.push(Node {
            kind: NodeKind::Symbol {
                file: file.to_string(),
                idx: i,
            },
            title,
            rows,
            mark: marks.symbol(file, i),
            muted: false,
        });
    }
    // Map nested functions to their outermost top-level node.
    let owner_node = |mut i: usize| -> Option<usize> {
        loop {
            if let Some(&n) = node_of_sym.get(&i) {
                return Some(n);
            }
            i = fs.symbols[i].parent?;
        }
    };
    let mut agg: BTreeMap<(usize, usize), (usize, BTreeSet<String>)> = BTreeMap::new();
    // External files: callees and callers.
    let mut ext_out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut ext_in: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut ext_edges: Vec<(Option<usize>, String, bool, String)> = Vec::new(); // (local node, ext file, outgoing, name)
    for c in &index.calls {
        if c.from_file == file && c.to.file == file {
            let (Some(a), Some(b)) = (c.from_sym.and_then(owner_node), owner_node(c.to.idx)) else {
                continue;
            };
            if a != b {
                let name = fs.symbols[c.to.idx].name.clone();
                let e = agg.entry((a, b)).or_default();
                e.0 += 1;
                e.1.insert(name);
            }
        } else if c.from_file == file {
            let name = index
                .symbols(&c.to.file)
                .map(|s| s.symbols[c.to.idx].qual())
                .unwrap_or_default();
            ext_out
                .entry(c.to.file.clone())
                .or_default()
                .insert(name.clone());
            ext_edges.push((
                c.from_sym.and_then(owner_node),
                c.to.file.clone(),
                true,
                name,
            ));
        } else if c.to.file == file {
            let name = c
                .from_sym
                .and_then(|s| index.symbols(&c.from_file).map(|f| f.symbols[s].qual()))
                .unwrap_or_else(|| "module".to_string());
            ext_in.entry(c.from_file.clone()).or_default().insert(name);
            ext_edges.push((
                owner_node(c.to.idx),
                c.from_file.clone(),
                false,
                String::new(),
            ));
        }
    }
    // Keep the busiest external files.
    let mut ext_files: Vec<(String, usize)> = ext_out
        .iter()
        .map(|(f, s)| (f.clone(), s.len()))
        .chain(ext_in.iter().map(|(f, s)| (f.clone(), s.len())))
        .collect();
    ext_files.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut seen = BTreeSet::new();
    ext_files.retain(|(f, _)| seen.insert(f.clone()));
    ext_files.truncate(10);
    let mut ext_node: HashMap<String, usize> = HashMap::new();
    for (f, _) in &ext_files {
        let mut rows: Vec<Row> = Vec::new();
        if let Some(names) = ext_out.get(f) {
            for n in names.iter().take(6) {
                rows.push(Row {
                    text: n.clone(),
                    mark: None,
                    sym: None,
                    dim: false,
                });
            }
        }
        if let Some(names) = ext_in.get(f) {
            for n in names.iter().take(4) {
                rows.push(Row {
                    text: format!("← {n}"),
                    mark: None,
                    sym: None,
                    dim: true,
                });
            }
        }
        ext_node.insert(f.clone(), nodes.len());
        nodes.push(Node {
            kind: NodeKind::External(f.clone()),
            title: f.clone(),
            rows,
            mark: marks.file(f),
            muted: true,
        });
    }
    for (local, f, outgoing, name) in ext_edges {
        let (Some(local), Some(&ext)) = (local, ext_node.get(&f)) else {
            continue;
        };
        let key = if outgoing { (local, ext) } else { (ext, local) };
        let e = agg.entry(key).or_default();
        e.0 += 1;
        if !name.is_empty() {
            e.1.insert(name);
        }
    }
    let edges = agg
        .into_iter()
        .map(|((from, to), (calls, names))| Edge {
            from,
            to,
            calls,
            names: names.into_iter().collect(),
            import_only: false,
        })
        .collect();
    let fns = fs.symbols.iter().filter(|s| s.kind.is_callable()).count();
    let changed = marks.symbols.get(file).map(|m| m.len()).unwrap_or(0);
    let mut summary = vec![
        lang.name().to_string(),
        util::plural(fns, "function", "functions"),
        util::plural(fs.lines as usize, "line", "lines"),
    ];
    if changed > 0 {
        summary.push(format!("{changed} changed"));
    }
    (nodes, edges, 1, summary.join(" · "))
}

/// Selection inside a scene: a node and optionally one of its rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cursor {
    pub node: usize,
    pub row: Option<usize>,
}

/// Draw a scene. The selected node gets a bright border and an active band
/// on the selected row; edges touching it are highlighted.
pub fn draw(scene: &Scene, cursor: Option<Cursor>) -> Canvas {
    let l = &scene.layout;
    let mut c = Canvas::new(l.width + 2, l.height + 2);
    let selected = cursor.map(|c| c.node);
    let mut order: Vec<usize> = (0..l.routes.len()).collect();
    // Highlighted edges last so they win shared cells.
    order.sort_by_key(|&i| {
        let e = l.routes[i].edge;
        selected.is_some_and(|s| e.from == s || e.to == s)
    });
    for i in order {
        let r = &l.routes[i];
        let e = r.edge;
        let hot = selected.is_some_and(|s| e.from == s || e.to == s);
        let import_only = scene
            .edges
            .iter()
            .any(|x| x.from == e.from && x.to == e.to && x.import_only);
        let tone = if hot {
            Tone::Blue
        } else if import_only {
            Tone::Muted
        } else {
            Tone::Rule
        };
        c.polyline(&r.points, tone);
        let (sx, sy) = r.points[0];
        // join the source's bottom border
        if sy > 0 {
            c.bits(sx, sy - 1, DOWN, tone);
        }
        if r.reversed {
            c.arrow(sx, sy, '▲', tone);
        } else {
            let (tx, ty) = *r.points.last().unwrap();
            c.arrow(tx, ty, '▼', tone);
        }
    }
    for (i, n) in scene.nodes.iter().enumerate() {
        let r = l.nodes[i];
        let is_sel = selected == Some(i);
        let tone = if is_sel {
            Tone::Text
        } else {
            match n.mark {
                Some('A') => Tone::Add,
                Some(_) => Tone::Mod,
                None if n.muted => Tone::Muted,
                None => Tone::Rule,
            }
        };
        // Keep port junctions drawn by edges on the borders.
        let mut saved = Vec::new();
        for x in r.x..r.right() {
            for y in [r.y, r.bottom() - 1] {
                if let Some(cell) = c.get(x, y) {
                    saved.push((x, y, cell.mask, cell.tone));
                }
            }
        }
        c.rect(
            r.x,
            r.y,
            r.w,
            r.h,
            tone,
            matches!(n.kind, NodeKind::Dir(_) | NodeKind::More),
        );
        for (x, y, mask, t) in saved {
            if mask & DOWN != 0 && y == r.bottom() - 1 {
                c.bits(x, y, DOWN, t);
            }
        }
        let title_tone = if is_sel {
            Tone::Text
        } else if n.muted {
            Tone::Dim
        } else {
            Tone::Text
        };
        let max_title = r.w.saturating_sub(4);
        let title = format!(
            " {} ",
            util::truncate(&n.title, max_title.saturating_sub(2))
        );
        c.text(r.x + 2, r.y, &title, title_tone, is_sel || n.mark.is_some());
        for (ri, row) in n.rows.iter().enumerate() {
            let y = r.y + 1 + ri;
            if y >= r.bottom() - 1 {
                break;
            }
            let row_sel = is_sel && cursor.and_then(|c| c.row) == Some(ri);
            let tone = match row.mark {
                Some('A') => Tone::Add,
                Some(_) => Tone::Mod,
                None if row.dim => Tone::Muted,
                None => Tone::Dim,
            };
            let avail = r.w.saturating_sub(if row.mark.is_some() { 6 } else { 4 });
            c.text_max(
                r.x + 2,
                y,
                &row.text,
                avail,
                if row_sel { Tone::Text } else { tone },
                row_sel,
            );
            if let Some(m) = row.mark {
                c.text(
                    r.right() - 3,
                    y,
                    &m.to_string(),
                    if m == 'A' { Tone::Add } else { Tone::Mod },
                    true,
                );
            }
            if row_sel {
                c.band(r.x + 1, r.right() - 2, y, Band::Active);
            }
        }
        if is_sel && cursor.and_then(|c| c.row).is_none() {
            for y in r.y + 1..r.bottom() - 1 {
                c.band(r.x + 1, r.right() - 2, y, Band::Active);
            }
        }
    }
    // Edge labels: callee names beside the arrowhead when there is room.
    for (ei, e) in scene.edges.iter().enumerate() {
        let Some(route) = l
            .routes
            .iter()
            .find(|r| r.edge.from == e.from && r.edge.to == e.to)
        else {
            continue;
        };
        let _ = ei;
        if e.names.is_empty() || route.reversed {
            continue;
        }
        let (tx, ty) = *route.points.last().unwrap();
        let label = if e.names.len() <= 2 {
            e.names.join(" · ")
        } else {
            format!("{} +{}", e.names[0], e.names.len() - 1)
        };
        let label = util::truncate(&label, 28);
        let w = util::width(&label);
        if c.is_free(tx + 1, ty, w + 2, 1) {
            c.text(tx + 2, ty, &label, Tone::Muted, false);
        }
    }
    c
}

/// Index of the node at a scene position (for mouse-free tests and search).
pub fn node_for_symbol(scene: &Scene, file: &str, idx: usize) -> Option<Cursor> {
    for (i, n) in scene.nodes.iter().enumerate() {
        if let NodeKind::Symbol { file: f, idx: s } = &n.kind {
            if f == file && *s == idx {
                return Some(Cursor { node: i, row: None });
            }
        }
        for (ri, r) in n.rows.iter().enumerate() {
            if r.sym.as_ref().is_some_and(|(f, s)| f == file && *s == idx) {
                return Some(Cursor {
                    node: i,
                    row: Some(ri),
                });
            }
        }
    }
    None
}

/// The node standing for a file or directory at a dir level.
pub fn node_for_path(scene: &Scene, path: &str) -> Option<usize> {
    scene.nodes.iter().position(|n| match &n.kind {
        NodeKind::File(f) | NodeKind::External(f) => f == path,
        NodeKind::Dir(d) => d == path || in_dir(path, d),
        _ => false,
    })
}
