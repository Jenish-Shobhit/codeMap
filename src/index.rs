//! The symbol index: every file of the worktree, their parsed symbols (parsed
//! lazily and cached by mtime and size), and the call and import edges
//! resolved between them.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use crate::lang::{self, FileSymbols, Lang, SymKind};

/// Files larger than this are listed but not parsed.
pub const MAX_PARSE_BYTES: u64 = 1_500_000;

/// A symbol somewhere in the repo.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymId {
    pub file: String,
    pub idx: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallEdge {
    pub from_file: String,
    /// Calling function, None for module-level code.
    pub from_sym: Option<usize>,
    pub to: SymId,
    pub line: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Index {
    pub root: PathBuf,
    /// Every file (relative, '/'-separated, sorted).
    pub files: Vec<String>,
    /// True when the listing was capped (huge repositories).
    pub truncated: bool,
    parsed: HashMap<String, Arc<FileSymbols>>,
    stamps: HashMap<String, (SystemTime, u64)>,
    pub calls: Vec<CallEdge>,
    /// file -> files it imports (resolved inside the repo).
    pub imports: BTreeMap<String, BTreeSet<String>>,
    resolved_generation: u64,
    generation: u64,
}

/// Methods too generic to link by name alone when the receiver is unknown.
const GENERIC_METHODS: &[&str] = &[
    "get",
    "set",
    "append",
    "extend",
    "pop",
    "push",
    "keys",
    "values",
    "items",
    "join",
    "split",
    "strip",
    "format",
    "replace",
    "update",
    "copy",
    "clear",
    "insert",
    "remove",
    "read",
    "write",
    "open",
    "close",
    "len",
    "map",
    "filter",
    "to_string",
    "clone",
    "unwrap",
    "iter",
    "into",
    "from",
    "as_str",
    "collect",
    "add",
    "sort",
    "find",
    "next",
    "send",
    "run",
    "call",
    "load",
    "save",
    "parse",
    "new",
    "default",
    "fmt",
    "eq",
    "hash",
    "log",
    "error",
    "info",
    "debug",
    "exists",
    "is_file",
    "is_dir",
    "lower",
    "upper",
    "startswith",
    "endswith",
    "encode",
    "decode",
    "then",
    "catch",
    "forEach",
    "toString",
    "Println",
    "Printf",
    "Sprintf",
    "Errorf",
];

impl Index {
    pub fn new(root: impl Into<PathBuf>, files: Vec<String>) -> Self {
        Index {
            root: root.into(),
            files,
            ..Default::default()
        }
    }

    pub fn is_parsed(&self, file: &str) -> bool {
        self.parsed.contains_key(file)
    }

    pub fn symbols(&self, file: &str) -> Option<&Arc<FileSymbols>> {
        self.parsed.get(file)
    }

    pub fn parsed_count(&self) -> usize {
        self.parsed.len()
    }

    pub fn source_files(&self) -> impl Iterator<Item = &String> {
        self.files.iter().filter(|f| Lang::from_path(f).is_some())
    }

    /// Parse one file if it changed since the cached parse. Returns true when
    /// the index changed.
    pub fn ensure_parsed(&mut self, file: &str) -> bool {
        let Some(lang) = Lang::from_path(file) else {
            return false;
        };
        let path = self.root.join(file);
        let Ok(meta) = std::fs::metadata(&path) else {
            return false;
        };
        let stamp = (
            meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            meta.len(),
        );
        if self.stamps.get(file) == Some(&stamp) && self.parsed.contains_key(file) {
            return false;
        }
        let symbols = if meta.len() > MAX_PARSE_BYTES {
            FileSymbols {
                lang,
                symbols: Vec::new(),
                calls: Vec::new(),
                imports: Vec::new(),
                lines: 0,
                has_errors: false,
            }
        } else {
            let src = std::fs::read(&path)
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();
            lang::extract(lang, &src)
        };
        self.insert_parsed(file, symbols, stamp);
        true
    }

    /// Insert symbols parsed elsewhere (a background thread).
    pub fn insert_parsed(&mut self, file: &str, symbols: FileSymbols, stamp: (SystemTime, u64)) {
        self.parsed.insert(file.to_string(), Arc::new(symbols));
        self.stamps.insert(file.to_string(), stamp);
        self.generation += 1;
    }

    /// Parse every source file under `dir` ("" for the whole repo), up to
    /// `limit` files. Returns how many were (re)parsed.
    pub fn ensure_parsed_under(&mut self, dir: &str, limit: usize) -> usize {
        let targets: Vec<String> = self
            .files
            .iter()
            .filter(|f| in_dir(f, dir) && Lang::from_path(f).is_some())
            .take(limit)
            .cloned()
            .collect();
        let mut n = 0;
        for f in targets {
            if self.ensure_parsed(&f) {
                n += 1;
            }
        }
        n
    }

    /// Resolve calls and imports across all parsed files (only when something
    /// changed since the last resolution).
    pub fn resolve(&mut self) {
        if self.resolved_generation == self.generation && self.generation != 0 {
            return;
        }
        self.resolved_generation = self.generation;
        let resolver = Resolver::new(self);
        let (calls, imports) = resolver.run();
        self.calls = calls;
        self.imports = imports;
    }

    /// Files directly inside `dir`.
    pub fn files_in(&self, dir: &str) -> Vec<&String> {
        self.files.iter().filter(|f| parent_dir(f) == dir).collect()
    }

    /// Immediate subdirectories of `dir`, with how many files each holds.
    pub fn subdirs(&self, dir: &str) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for f in &self.files {
            if !in_dir(f, dir) {
                continue;
            }
            let rest = if dir.is_empty() {
                f.as_str()
            } else {
                &f[dir.len() + 1..]
            };
            if let Some((first, _)) = rest.split_once('/') {
                let sub = if dir.is_empty() {
                    first.to_string()
                } else {
                    format!("{dir}/{first}")
                };
                *counts.entry(sub).or_default() += 1;
            }
        }
        counts.into_iter().collect()
    }

    /// Callable and container symbols under a directory, for counts.
    pub fn function_count_under(&self, dir: &str) -> usize {
        self.parsed
            .iter()
            .filter(|(f, _)| in_dir(f, dir))
            .map(|(_, s)| s.symbols.iter().filter(|x| x.kind.is_callable()).count())
            .sum()
    }

    /// Callers of a symbol (for "called from" in Flow).
    pub fn callers_of(&self, target: &SymId) -> Vec<&CallEdge> {
        self.calls.iter().filter(|c| &c.to == target).collect()
    }

    /// Calls made by a symbol.
    pub fn calls_from(&self, file: &str, sym: usize) -> Vec<&CallEdge> {
        self.calls
            .iter()
            .filter(|c| c.from_file == file && c.from_sym == Some(sym))
            .collect()
    }

    /// Search files and symbols by name.
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        let mut hits: Vec<SearchHit> = Vec::new();
        for f in &self.files {
            let name = f.rsplit('/').next().unwrap_or(f);
            if let Some(score) = crate::util::fuzzy_score(query, name) {
                hits.push(SearchHit {
                    file: f.clone(),
                    sym: None,
                    label: f.clone(),
                    score: score - 5,
                });
            }
        }
        for (f, syms) in &self.parsed {
            for (i, s) in syms.symbols.iter().enumerate() {
                if s.kind == SymKind::Impl {
                    continue;
                }
                if let Some(score) = crate::util::fuzzy_score(query, &s.qual()) {
                    hits.push(SearchHit {
                        file: f.clone(),
                        sym: Some(i),
                        label: s.qual(),
                        score,
                    });
                }
            }
        }
        hits.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then(a.label.cmp(&b.label))
                .then(a.file.cmp(&b.file))
        });
        hits.truncate(limit);
        hits
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub file: String,
    pub sym: Option<usize>,
    pub label: String,
    pub score: i64,
}

pub fn parent_dir(file: &str) -> &str {
    file.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

pub fn in_dir(file: &str, dir: &str) -> bool {
    dir.is_empty()
        || (file.len() > dir.len() && file.starts_with(dir) && file.as_bytes()[dir.len()] == b'/')
}

/// Name resolution by rules, most specific first:
/// same file, then explicit imports, then `self`/type receivers, then a
/// unique match across the repo.
struct Resolver<'a> {
    index: &'a Index,
    /// name -> symbols with that name (callables and containers)
    by_name: HashMap<&'a str, Vec<SymId>>,
    /// (container, method) -> symbols
    methods: HashMap<(&'a str, &'a str), Vec<SymId>>,
    files: BTreeSet<&'a str>,
}

impl<'a> Resolver<'a> {
    fn new(index: &'a Index) -> Self {
        let mut by_name: HashMap<&str, Vec<SymId>> = HashMap::new();
        let mut methods: HashMap<(&str, &str), Vec<SymId>> = HashMap::new();
        for (file, syms) in &index.parsed {
            for (i, s) in syms.symbols.iter().enumerate() {
                if matches!(s.kind, SymKind::Impl | SymKind::Module) {
                    continue;
                }
                let id = SymId {
                    file: file.clone(),
                    idx: i,
                };
                by_name.entry(s.name.as_str()).or_default().push(id.clone());
                if s.kind == SymKind::Method {
                    if let Some(c) = &s.container {
                        methods
                            .entry((c.as_str(), s.name.as_str()))
                            .or_default()
                            .push(id);
                    }
                }
            }
        }
        for v in by_name.values_mut() {
            v.sort();
        }
        for v in methods.values_mut() {
            v.sort();
        }
        Resolver {
            index,
            by_name,
            methods,
            files: index.files.iter().map(String::as_str).collect(),
        }
    }

    fn run(&self) -> (Vec<CallEdge>, BTreeMap<String, BTreeSet<String>>) {
        let mut calls = Vec::new();
        let mut imports: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut files: Vec<&String> = self.index.parsed.keys().collect();
        files.sort();
        for file in files {
            let syms = &self.index.parsed[file];
            // Module imports: alias -> files; imported name -> (files, original name)
            let mut module_alias: HashMap<String, Vec<String>> = HashMap::new();
            let mut imported_names: HashMap<String, (Vec<String>, String)> = HashMap::new();
            for imp in &syms.imports {
                let targets = self.resolve_module(file, syms.lang, imp);
                for t in &targets {
                    if t != file {
                        imports.entry(file.clone()).or_default().insert(t.clone());
                    }
                }
                // Python `from pkg import mod` where mod is a module file.
                for (name, alias) in &imp.names {
                    let local = alias.clone().unwrap_or_else(|| name.clone());
                    let sub = self.resolve_submodule(file, syms.lang, imp, name);
                    if !sub.is_empty() {
                        for t in &sub {
                            if t != file {
                                imports.entry(file.clone()).or_default().insert(t.clone());
                            }
                        }
                        module_alias.insert(local, sub);
                    } else if !targets.is_empty() {
                        imported_names.insert(local, (targets.clone(), name.clone()));
                    }
                }
                if !targets.is_empty() {
                    let alias = imp.alias.clone().or_else(|| match syms.lang {
                        Lang::Python if imp.names.is_empty() => Some(
                            imp.module
                                .rsplit('.')
                                .next()
                                .unwrap_or(&imp.module)
                                .to_string(),
                        ),
                        Lang::Rust if imp.names.len() == 1 => Some(imp.names[0].0.clone()),
                        Lang::Rust => imp.module.strip_prefix("mod::").map(str::to_string),
                        _ => None,
                    });
                    if let Some(a) = alias {
                        module_alias.insert(a, targets.clone());
                    }
                }
            }
            for call in &syms.calls {
                if let Some(to) =
                    self.resolve_call(file, syms, call, &module_alias, &imported_names)
                {
                    // Skip a function "calling" its own definition line.
                    if to.file == *file
                        && Some(to.idx) == call.caller
                        && self.index.parsed[file].symbols[to.idx].start_line == call.line
                    {
                        continue;
                    }
                    calls.push(CallEdge {
                        from_file: file.clone(),
                        from_sym: call.caller,
                        to,
                        line: call.line,
                    });
                }
            }
        }
        (calls, imports)
    }

    fn sym(&self, id: &SymId) -> &lang::Symbol {
        &self.index.parsed[&id.file].symbols[id.idx]
    }

    fn resolve_call(
        &self,
        file: &'a str,
        syms: &FileSymbols,
        call: &lang::CallRef,
        module_alias: &HashMap<String, Vec<String>>,
        imported_names: &HashMap<String, (Vec<String>, String)>,
    ) -> Option<SymId> {
        let name = call.name.as_str();
        let candidates = self.by_name.get(name)?;
        let in_file = |f: &'a str| candidates.iter().filter(move |c| c.file == f);
        let caller_container = call.caller.and_then(|c| syms.symbols[c].container.clone());
        match call.receiver.as_deref() {
            None => {
                // 1. Same file: prefer non-methods (functions, classes).
                if let Some(c) = in_file(file).find(|c| self.sym(c).kind != SymKind::Method) {
                    return Some(c.clone());
                }
                // 2. Imported by name.
                if let Some((targets, original)) = imported_names.get(name) {
                    for t in targets {
                        if let Some(c) = self.by_name.get(original.as_str()).and_then(|v| {
                            v.iter()
                                .find(|c| &c.file == t && self.sym(c).kind != SymKind::Method)
                        }) {
                            return Some(c.clone());
                        }
                    }
                }
                // 3. Unique non-method definition in the repo.
                let globals: Vec<&SymId> = candidates
                    .iter()
                    .filter(|c| self.sym(c).kind != SymKind::Method)
                    .collect();
                if globals.len() == 1 && !GENERIC_METHODS.contains(&name) {
                    return Some(globals[0].clone());
                }
                None
            }
            Some(recv) => {
                let recv_head = recv.split(['.', ':']).next().unwrap_or(recv);
                // self.foo() / this.foo() / Self::foo()
                if matches!(recv, "self" | "this" | "cls" | "Self") {
                    if let Some(container) = &caller_container {
                        if let Some(v) = self.methods.get(&(container.as_str(), name)) {
                            if let Some(c) = v.iter().find(|c| c.file == file).or(v.first()) {
                                return Some(c.clone());
                            }
                        }
                    }
                    if let Some(c) = in_file(file).find(|c| self.sym(c).kind == SymKind::Method) {
                        return Some(c.clone());
                    }
                    return None;
                }
                // module.func() through an import alias
                if let Some(targets) = module_alias
                    .get(recv)
                    .or_else(|| module_alias.get(recv_head))
                {
                    for t in targets {
                        if let Some(c) = candidates.iter().find(|c| &c.file == t) {
                            return Some(c.clone());
                        }
                    }
                }
                // Type::method() / Type.method()
                let recv_last = recv
                    .rsplit(['.', ':'])
                    .find(|s| !s.is_empty())
                    .unwrap_or(recv);
                if let Some(v) = self.methods.get(&(recv_last, name)) {
                    if let Some(c) = v.iter().find(|c| c.file == file).or(v.first()) {
                        return Some(c.clone());
                    }
                }
                if GENERIC_METHODS.contains(&name) {
                    return None;
                }
                // obj.method(): a unique method of that name, preferring the
                // same file, then the same directory.
                let methods: Vec<&SymId> = candidates
                    .iter()
                    .filter(|c| self.sym(c).kind == SymKind::Method)
                    .collect();
                if methods.len() == 1 {
                    return Some(methods[0].clone());
                }
                if let Some(c) = methods.iter().find(|c| c.file == file) {
                    return Some((*c).clone());
                }
                let dir = parent_dir(file);
                let same_dir: Vec<&&SymId> = methods
                    .iter()
                    .filter(|c| parent_dir(&c.file) == dir)
                    .collect();
                if same_dir.len() == 1 {
                    return Some((*same_dir[0]).clone());
                }
                None
            }
        }
    }

    fn exists(&self, rel: &str) -> bool {
        self.files.contains(rel)
    }

    fn first_existing(&self, candidates: &[String]) -> Vec<String> {
        candidates
            .iter()
            .find(|c| self.exists(c))
            .map(|c| vec![c.clone()])
            .unwrap_or_default()
    }

    /// Files a module path refers to.
    fn resolve_module(&self, file: &str, lang: Lang, imp: &lang::ImportRef) -> Vec<String> {
        let dir = parent_dir(file);
        match lang {
            Lang::Python => {
                let base = if imp.level > 0 {
                    let mut d = dir.to_string();
                    for _ in 1..imp.level {
                        d = parent_dir(&d).to_string();
                    }
                    d
                } else {
                    String::new()
                };
                if imp.module.is_empty() {
                    // `from . import x`: the package itself
                    return self.first_existing(&[join_path(&base, "__init__.py")]);
                }
                let rel = imp.module.replace('.', "/");
                let mut cands = vec![
                    join_path(&base, &format!("{rel}.py")),
                    join_path(&base, &format!("{rel}/__init__.py")),
                ];
                if imp.level == 0 {
                    cands.push(format!("src/{rel}.py"));
                    cands.push(format!("src/{rel}/__init__.py"));
                }
                self.first_existing(&cands)
            }
            Lang::Rust => self.resolve_rust_path(file, &imp.module),
            Lang::TypeScript | Lang::Tsx | Lang::JavaScript => {
                if !imp.module.starts_with('.') {
                    return Vec::new();
                }
                let base = normalize(&join_path(dir, &imp.module));
                let mut cands = vec![base.clone()];
                for ext in ["ts", "tsx", "js", "jsx", "mjs", "cjs"] {
                    cands.push(format!("{base}.{ext}"));
                }
                for ext in ["ts", "tsx", "js", "jsx"] {
                    cands.push(format!("{base}/index.{ext}"));
                }
                self.first_existing(&cands)
            }
            Lang::Go => {
                // Match the import path's tail against directories in the repo.
                let tail = imp.module.as_str();
                let mut best: Option<&str> = None;
                for f in &self.files {
                    if !f.ends_with(".go") {
                        continue;
                    }
                    let d = parent_dir(f);
                    if !d.is_empty() && (tail == d || tail.ends_with(&format!("/{d}"))) {
                        best = Some(d);
                        break;
                    }
                }
                match best {
                    Some(d) => self
                        .files
                        .iter()
                        .filter(|f| {
                            parent_dir(f) == d && f.ends_with(".go") && !f.ends_with("_test.go")
                        })
                        .map(|f| f.to_string())
                        .collect(),
                    None => Vec::new(),
                }
            }
        }
    }

    /// `from pkg import mod` where `mod` is itself a module file.
    fn resolve_submodule(
        &self,
        file: &str,
        lang: Lang,
        imp: &lang::ImportRef,
        name: &str,
    ) -> Vec<String> {
        if lang != Lang::Python || name == "*" {
            return Vec::new();
        }
        let mut sub = imp.clone();
        sub.module = if imp.module.is_empty() {
            name.to_string()
        } else {
            format!("{}.{name}", imp.module)
        };
        let found = self.resolve_module(file, lang, &sub);
        found
            .into_iter()
            .filter(|f| !f.ends_with("__init__.py") || f.contains(&format!("{name}/")))
            .collect()
    }

    fn resolve_rust_path(&self, file: &str, path: &str) -> Vec<String> {
        let dir = parent_dir(file);
        if let Some(name) = path.strip_prefix("mod::") {
            // `mod foo;` next to this file (or in its directory for mod.rs/lib.rs/main.rs)
            let fname = file.rsplit('/').next().unwrap_or(file);
            let base = if matches!(fname, "mod.rs" | "lib.rs" | "main.rs") {
                dir.to_string()
            } else {
                join_path(dir, fname.trim_end_matches(".rs"))
            };
            return self.first_existing(&[
                join_path(&base, &format!("{name}.rs")),
                join_path(&base, &format!("{name}/mod.rs")),
            ]);
        }
        let segs: Vec<&str> = path.split("::").filter(|s| *s != "*").collect();
        let (base, rest): (String, &[&str]) = match segs.first() {
            Some(&"crate") => {
                // The crate root: the directory holding lib.rs or main.rs.
                let mut root = String::from("src");
                let mut d = dir;
                loop {
                    if self.exists(&join_path(d, "lib.rs")) || self.exists(&join_path(d, "main.rs"))
                    {
                        root = d.to_string();
                        break;
                    }
                    if d.is_empty() {
                        break;
                    }
                    d = parent_dir(d);
                }
                (root, &segs[1..])
            }
            Some(&"super") => (parent_dir(dir).to_string(), &segs[1..]),
            Some(&"self") => (dir.to_string(), &segs[1..]),
            _ => return Vec::new(),
        };
        // Longest prefix of segments that names a file.
        for n in (1..=rest.len()).rev() {
            let rel = rest[..n].join("/");
            let found = self.first_existing(&[
                join_path(&base, &format!("{rel}.rs")),
                join_path(&base, &format!("{rel}/mod.rs")),
            ]);
            if !found.is_empty() {
                return found;
            }
        }
        Vec::new()
    }
}

fn join_path(dir: &str, rel: &str) -> String {
    if dir.is_empty() {
        rel.to_string()
    } else {
        format!("{dir}/{rel}")
    }
}

/// Collapse `.` and `..` segments.
fn normalize(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    out.join("/")
}

/// Resolve a path relative to the index root into an absolute path.
pub fn abs(root: &Path, rel: &str) -> PathBuf {
    root.join(rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_helpers() {
        assert_eq!(parent_dir("a/b/c.py"), "a/b");
        assert_eq!(parent_dir("c.py"), "");
        assert!(in_dir("a/b/c.py", "a"));
        assert!(!in_dir("ab/c.py", "a"));
        assert!(in_dir("x.py", ""));
        assert_eq!(normalize("src/app/../util/./x"), "src/util/x");
    }
}
