//! Languages and symbol extraction with tree-sitter.
//!
//! Definitions and calls come from tags-style queries (`@def.*` for
//! definitions, `@ref.call` for calls, as in tree-sitter's code navigation
//! tags). Imports are read by walking the tree, because their shapes differ
//! too much between languages for one capture scheme.

mod imports;
mod queries;

use std::sync::OnceLock;

use tree_sitter::{Language, Node, Parser, Query, QueryCursor, StreamingIterator, Tree};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Lang {
    Python,
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Go,
}

impl Lang {
    pub const ALL: [Lang; 6] = [
        Lang::Python,
        Lang::Rust,
        Lang::TypeScript,
        Lang::Tsx,
        Lang::JavaScript,
        Lang::Go,
    ];

    pub fn from_path(path: &str) -> Option<Lang> {
        let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
        match ext.as_str() {
            "py" | "pyi" => Some(Lang::Python),
            "rs" => Some(Lang::Rust),
            "ts" | "mts" | "cts" => Some(Lang::TypeScript),
            "tsx" => Some(Lang::Tsx),
            "js" | "jsx" | "mjs" | "cjs" => Some(Lang::JavaScript),
            "go" => Some(Lang::Go),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Lang::Python => "python",
            Lang::Rust => "rust",
            Lang::TypeScript => "typescript",
            Lang::Tsx => "tsx",
            Lang::JavaScript => "javascript",
            Lang::Go => "go",
        }
    }

    pub fn language(self) -> Language {
        match self {
            Lang::Python => tree_sitter_python::LANGUAGE.into(),
            Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
            Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Lang::Go => tree_sitter_go::LANGUAGE.into(),
        }
    }

    /// Whether the Flow view can chart functions of this language.
    pub fn has_flow(self) -> bool {
        true
    }

    fn query(self) -> &'static Query {
        static QUERIES: [OnceLock<Query>; 6] = [
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
        ];
        let idx = Lang::ALL.iter().position(|l| *l == self).unwrap_or(0);
        QUERIES[idx].get_or_init(|| {
            Query::new(&self.language(), queries::source(self))
                .unwrap_or_else(|e| panic!("bad {} tags query: {e}", self.name()))
        })
    }
}

/// Extensions codeMorph can parse, for "grammar missing" messages.
pub fn supported_extensions() -> &'static str {
    ".py .rs .ts .tsx .js .jsx .go"
}

pub fn parse(lang: Lang, src: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&lang.language()).ok()?;
    parser.parse(src, None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymKind {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Trait,
    Interface,
    Type,
    Impl,
    Module,
}

impl SymKind {
    /// Functions and methods have bodies worth charting and can make calls.
    pub fn is_callable(self) -> bool {
        matches!(self, SymKind::Function | SymKind::Method)
    }

    /// Containers group methods in the Map.
    pub fn is_container(self) -> bool {
        matches!(
            self,
            SymKind::Class
                | SymKind::Struct
                | SymKind::Enum
                | SymKind::Trait
                | SymKind::Interface
                | SymKind::Impl
        )
    }

    pub fn keyword(self, lang: Lang) -> &'static str {
        match (self, lang) {
            (SymKind::Function | SymKind::Method, Lang::Python) => "def",
            (SymKind::Function | SymKind::Method, Lang::Rust) => "fn",
            (SymKind::Function | SymKind::Method, Lang::Go) => "func",
            (SymKind::Function | SymKind::Method, _) => "function",
            (SymKind::Class, _) => "class",
            (SymKind::Struct, Lang::Go) => "type",
            (SymKind::Struct, _) => "struct",
            (SymKind::Enum, _) => "enum",
            (SymKind::Trait, _) => "trait",
            (SymKind::Interface, _) => "interface",
            (SymKind::Type, _) => "type",
            (SymKind::Impl, _) => "impl",
            (SymKind::Module, _) => "mod",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymKind,
    /// 1-based, inclusive.
    pub start_line: u32,
    pub end_line: u32,
    pub start_byte: usize,
    pub end_byte: usize,
    /// Enclosing symbol (class, impl, function) by index.
    pub parent: Option<usize>,
    /// Type a method belongs to (class name, impl type, Go receiver).
    pub container: Option<String>,
}

impl Symbol {
    /// "Class.method" or "name".
    pub fn qual(&self) -> String {
        match &self.container {
            Some(c) if self.kind == SymKind::Method => format!("{c}.{}", self.name),
            _ => self.name.clone(),
        }
    }

    pub fn lines(&self) -> u32 {
        self.end_line.saturating_sub(self.start_line) + 1
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRef {
    /// Innermost enclosing function or method, None at module level.
    pub caller: Option<usize>,
    pub name: String,
    /// `self` in `self.foo()`, `api` in `api.request()`, `Foo` in `Foo::new()`.
    pub receiver: Option<String>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportRef {
    /// Module path as written: "panemorph.api", "crate::git::diff", "./util", "fmt".
    pub module: String,
    /// Imported names with optional alias: `from m import a as b`.
    pub names: Vec<(String, Option<String>)>,
    /// Alias for the module itself: `import numpy as np`, Go package names.
    pub alias: Option<String>,
    /// Python relative import depth (number of leading dots).
    pub level: u32,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSymbols {
    pub lang: Lang,
    pub symbols: Vec<Symbol>,
    pub calls: Vec<CallRef>,
    pub imports: Vec<ImportRef>,
    pub lines: u32,
    /// True when tree-sitter reported syntax errors (results are best effort).
    pub has_errors: bool,
}

impl FileSymbols {
    /// Top-level definitions (no parent, or whose parent is a module).
    pub fn top_level(&self) -> Vec<usize> {
        (0..self.symbols.len())
            .filter(|&i| match self.symbols[i].parent {
                None => true,
                Some(p) => self.symbols[p].kind == SymKind::Module,
            })
            .collect()
    }

    pub fn children(&self, parent: usize) -> Vec<usize> {
        (0..self.symbols.len())
            .filter(|&i| self.symbols[i].parent == Some(parent))
            .collect()
    }

    /// The innermost callable containing a 1-based line.
    pub fn callable_at_line(&self, line: u32) -> Option<usize> {
        self.symbols
            .iter()
            .enumerate()
            .filter(|(_, s)| s.kind.is_callable() && s.start_line <= line && line <= s.end_line)
            .min_by_key(|(_, s)| s.end_line - s.start_line)
            .map(|(i, _)| i)
    }

    pub fn find(&self, qual: &str) -> Option<usize> {
        self.symbols.iter().position(|s| s.qual() == qual)
    }
}

fn text<'a>(node: Node, src: &'a str) -> &'a str {
    src.get(node.byte_range()).unwrap_or("")
}

/// Clean a type expression down to its base name: `Foo<T>` -> `Foo`,
/// `&mut crate::x::Foo` -> `Foo`.
fn base_type_name(s: &str) -> String {
    let s = s.trim().trim_start_matches('&').trim_start_matches("mut ").trim();
    let s = s.split('<').next().unwrap_or(s);
    let s = s.rsplit("::").next().unwrap_or(s);
    let s = s.trim_start_matches('*');
    s.trim().to_string()
}

/// Parse `src` and pull out definitions, calls and imports.
pub fn extract(lang: Lang, src: &str) -> FileSymbols {
    let lines = src.lines().count() as u32;
    let Some(tree) = parse(lang, src) else {
        return FileSymbols {
            lang,
            symbols: Vec::new(),
            calls: Vec::new(),
            imports: Vec::new(),
            lines,
            has_errors: true,
        };
    };
    extract_tree(lang, src, &tree)
}

pub fn extract_tree(lang: Lang, src: &str, tree: &Tree) -> FileSymbols {
    let root = tree.root_node();
    let query = lang.query();
    let names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, root, src.as_bytes());

    struct RawDef {
        kind: SymKind,
        name: String,
        start_byte: usize,
        end_byte: usize,
        start_line: u32,
        end_line: u32,
        recv: Option<String>,
        is_go_type: Option<String>,
    }
    struct RawCall {
        name: String,
        recv: Option<String>,
        byte: usize,
        line: u32,
    }
    let mut defs: Vec<RawDef> = Vec::new();
    let mut calls: Vec<RawCall> = Vec::new();

    while let Some(m) = matches.next() {
        let mut def_node: Option<(Node, &str)> = None;
        let mut name_node: Option<Node> = None;
        let mut recv_node: Option<Node> = None;
        let mut call_node: Option<Node> = None;
        for cap in m.captures {
            let cname = names[cap.index as usize];
            match cname {
                "name" => name_node = Some(cap.node),
                "recv" => recv_node = Some(cap.node),
                "ref.call" => call_node = Some(cap.node),
                c if c.starts_with("def.") => def_node = Some((cap.node, &c[4..])),
                _ => {}
            }
        }
        if let (Some((node, kind)), Some(name)) = (def_node, name_node) {
            let kind = match kind {
                "function" => SymKind::Function,
                "method" => SymKind::Method,
                "class" => SymKind::Class,
                "struct" => SymKind::Struct,
                "enum" => SymKind::Enum,
                "trait" => SymKind::Trait,
                "interface" => SymKind::Interface,
                "type" => SymKind::Type,
                "impl" => SymKind::Impl,
                "module" => SymKind::Module,
                _ => SymKind::Function,
            };
            let mut name_text = text(name, src).to_string();
            if kind == SymKind::Impl {
                name_text = base_type_name(&name_text);
            }
            // Go `type X struct/interface` decides the kind from the type node.
            let go_type = if lang == Lang::Go && kind == SymKind::Struct {
                node.child_by_field_name("type").map(|t| t.kind().to_string())
            } else {
                None
            };
            defs.push(RawDef {
                kind,
                name: name_text,
                start_byte: node.start_byte(),
                end_byte: node.end_byte(),
                start_line: node.start_position().row as u32 + 1,
                end_line: node.end_position().row as u32 + 1,
                recv: recv_node.map(|r| base_type_name(text(r, src))),
                is_go_type: go_type,
            });
        } else if let (Some(call), Some(name)) = (call_node, name_node) {
            let recv = recv_node.map(|r| {
                let t = text(r, src);
                // Keep receivers short: `self.client` -> "self.client",
                // long expressions -> their last identifier.
                let t = t.trim();
                if t.len() > 60 || t.contains('\n') || t.contains('(') {
                    t.rsplit(['.', ':', ')']).find(|p| !p.is_empty()).unwrap_or("").trim().to_string()
                } else {
                    t.to_string()
                }
            });
            calls.push(RawCall {
                name: text(name, src).to_string(),
                recv,
                byte: call.start_byte(),
                line: call.start_position().row as u32 + 1,
            });
        }
    }

    // Deduplicate definitions (a node can match two patterns) and sort by
    // position so parents precede children.
    defs.sort_by(|a, b| {
        a.start_byte
            .cmp(&b.start_byte)
            .then(b.end_byte.cmp(&a.end_byte))
    });
    defs.dedup_by(|a, b| a.start_byte == b.start_byte && a.end_byte == b.end_byte);

    let mut symbols: Vec<Symbol> = Vec::with_capacity(defs.len());
    let mut stack: Vec<usize> = Vec::new();
    for d in defs {
        while let Some(&top) = stack.last() {
            if symbols[top].end_byte <= d.start_byte {
                stack.pop();
            } else {
                break;
            }
        }
        let parent = stack.last().copied();
        let mut kind = d.kind;
        let mut container = None;
        if let Some(p) = parent {
            let pk = symbols[p].kind;
            if kind == SymKind::Function && pk.is_container() {
                kind = SymKind::Method;
                container = Some(symbols[p].name.clone());
            }
        }
        if kind == SymKind::Method && container.is_none() {
            container = d.recv.clone().or_else(|| parent.map(|p| symbols[p].name.clone()));
        }
        if let Some(t) = &d.is_go_type {
            kind = match t.as_str() {
                "interface_type" => SymKind::Interface,
                "struct_type" => SymKind::Struct,
                _ => SymKind::Type,
            };
        }
        symbols.push(Symbol {
            name: d.name,
            kind,
            start_line: d.start_line,
            end_line: d.end_line,
            start_byte: d.start_byte,
            end_byte: d.end_byte,
            parent,
            container,
        });
        stack.push(symbols.len() - 1);
    }

    // Attach each call to the innermost enclosing callable.
    let callables: Vec<usize> = (0..symbols.len())
        .filter(|&i| symbols[i].kind.is_callable())
        .collect();
    let mut call_refs: Vec<CallRef> = calls
        .into_iter()
        .map(|c| {
            let caller = callables
                .iter()
                .copied()
                .filter(|&i| symbols[i].start_byte <= c.byte && c.byte < symbols[i].end_byte)
                .min_by_key(|&i| symbols[i].end_byte - symbols[i].start_byte);
            CallRef {
                caller,
                name: c.name,
                receiver: c.recv,
                line: c.line,
            }
        })
        .collect();
    call_refs.sort_by_key(|c| c.line);

    let imports = imports::collect(lang, root, src);

    FileSymbols {
        lang,
        symbols,
        calls: call_refs,
        imports,
        lines: src.lines().count() as u32,
        has_errors: root.has_error(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_query_compiles() {
        for lang in Lang::ALL {
            let _ = lang.query();
        }
    }

    #[test]
    fn detects_languages() {
        assert_eq!(Lang::from_path("a/b.py"), Some(Lang::Python));
        assert_eq!(Lang::from_path("main.rs"), Some(Lang::Rust));
        assert_eq!(Lang::from_path("x.tsx"), Some(Lang::Tsx));
        assert_eq!(Lang::from_path("README.md"), None);
        assert_eq!(Lang::from_path("Makefile"), None);
    }

    #[test]
    fn python_defs_calls_imports() {
        let src = r#"import os
from .api import HerdrClient, HerdrError as Err
from . import model

class Service:
    def __init__(self, client):
        self.client = client

    def current(self):
        return self.client.request("pane.current")

def main():
    svc = Service(HerdrClient())
    svc.current()
    helper()

def helper():
    os.getcwd()
"#;
        let fs = extract(Lang::Python, src);
        let quals: Vec<String> = fs.symbols.iter().map(Symbol::qual).collect();
        assert_eq!(quals, vec!["Service", "Service.__init__", "Service.current", "main", "helper"]);
        assert_eq!(fs.symbols[1].kind, SymKind::Method);
        assert_eq!(fs.symbols[3].start_line, 12);
        assert_eq!(fs.symbols[3].end_line, 15);
        let main_calls: Vec<(&str, Option<&str>)> = fs
            .calls
            .iter()
            .filter(|c| c.caller == Some(3))
            .map(|c| (c.name.as_str(), c.receiver.as_deref()))
            .collect();
        assert_eq!(
            main_calls,
            vec![("Service", None), ("HerdrClient", None), ("current", Some("svc")), ("helper", None)]
        );
        assert_eq!(fs.imports.len(), 3);
        assert_eq!(fs.imports[1].module, "api");
        assert_eq!(fs.imports[1].level, 1);
        assert_eq!(
            fs.imports[1].names,
            vec![("HerdrClient".to_string(), None), ("HerdrError".to_string(), Some("Err".to_string()))]
        );
        assert_eq!(fs.imports[2].names[0].0, "model");
        assert_eq!(fs.top_level(), vec![0, 3, 4]);
        assert_eq!(fs.callable_at_line(14), Some(3));
    }

    #[test]
    fn rust_impls_and_paths() {
        let src = r#"use crate::git::{self, diff::parse};
use std::path::Path;

pub struct Store { root: String }

impl Store {
    pub fn new(root: &str) -> Self { Store { root: root.into() } }
    pub fn load(&self) -> usize { self.count() + helper() }
    fn count(&self) -> usize { 1 }
}

fn helper() -> usize {
    let s = Store::new("x");
    parse("");
    s.load()
}

trait Show { fn show(&self); }
"#;
        let fs = extract(Lang::Rust, src);
        let quals: Vec<String> = fs.symbols.iter().map(Symbol::qual).collect();
        assert_eq!(
            quals,
            vec!["Store", "Store", "Store.new", "Store.load", "Store.count", "helper", "Show", "Show.show"]
        );
        assert_eq!(fs.symbols[1].kind, SymKind::Impl);
        let helper_calls: Vec<(&str, Option<&str>)> = fs
            .calls
            .iter()
            .filter(|c| c.caller == Some(5))
            .map(|c| (c.name.as_str(), c.receiver.as_deref()))
            .collect();
        assert_eq!(helper_calls, vec![("new", Some("Store")), ("parse", None), ("load", Some("s"))]);
        let modules: Vec<&str> = fs.imports.iter().map(|i| i.module.as_str()).collect();
        assert!(modules.contains(&"crate::git"));
        assert!(modules.contains(&"crate::git::diff::parse"));
        assert!(modules.contains(&"std::path::Path"));
    }

    #[test]
    fn typescript_and_go() {
        let ts = r#"import { request } from "./api";
import * as util from "../util";
export class Board {
  render() { return this.draw(util.fmt(1)); }
  draw(x: number) { return request(x); }
}
export const make = () => new Board();
function top() { make(); }
interface Shape { area(): number }
"#;
        let fs = extract(Lang::TypeScript, ts);
        let quals: Vec<String> = fs.symbols.iter().map(Symbol::qual).collect();
        assert_eq!(quals, vec!["Board", "Board.render", "Board.draw", "make", "top", "Shape"]);
        assert_eq!(fs.imports[0].module, "./api");
        assert_eq!(fs.imports[1].alias.as_deref(), Some("util"));
        assert!(fs.calls.iter().any(|c| c.name == "Board" && c.caller == Some(3)));

        let go = r#"package main

import (
    "fmt"
    h "example.com/app/helpers"
)

type Server struct{ n int }
type Runner interface{ Run() }

func (s *Server) Start() { s.loop(); h.Log("x") }
func (s Server) loop() {}
func main() { s := &Server{}; s.Start(); fmt.Println("hi") }
"#;
        let fs = extract(Lang::Go, go);
        let quals: Vec<String> = fs.symbols.iter().map(Symbol::qual).collect();
        assert_eq!(quals, vec!["Server", "Runner", "Server.Start", "Server.loop", "main"]);
        assert_eq!(fs.symbols[1].kind, SymKind::Interface);
        assert_eq!(fs.imports.len(), 2);
        assert_eq!(fs.imports[1].alias.as_deref(), Some("h"));
        assert_eq!(fs.imports[1].module, "example.com/app/helpers");
    }
}
