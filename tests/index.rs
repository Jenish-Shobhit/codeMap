//! Tree-sitter extraction and call-edge resolution on real code: paneMorph's
//! Python package (copied as fixtures) and small Rust/TypeScript/Go samples.

mod common;

use codemorph::git;
use codemorph::index::{Index, SymId};
use codemorph::lang::{self, Lang, SymKind};
use common::*;

fn panemorph_index() -> Index {
    let root = fixtures_dir().join("panemorph");
    let files = git::walk_files(&root, 10_000);
    let mut index = Index::new(&root, files);
    index.ensure_parsed_under("", 10_000);
    index.resolve();
    index
}

fn edge_names(index: &Index, from_file: &str, from_qual: &str) -> Vec<String> {
    let syms = index.symbols(from_file).unwrap();
    let from = syms.find(from_qual).unwrap_or_else(|| panic!("no {from_qual} in {from_file}"));
    let mut out: Vec<String> = index
        .calls_from(from_file, from)
        .iter()
        .map(|e| {
            let to = &index.symbols(&e.to.file).unwrap().symbols[e.to.idx];
            format!("{}:{}", e.to.file.rsplit('/').next().unwrap(), to.qual())
        })
        .collect();
    out.dedup();
    out
}

#[test]
fn extracts_panemorph_symbols() {
    let index = panemorph_index();
    let service = index.symbols("panemorph/service.py").unwrap();
    assert_eq!(service.lang, Lang::Python);
    let quals: Vec<String> = service.symbols.iter().map(|s| s.qual()).collect();
    for q in [
        "PaneMorphService",
        "PaneMorphService.__init__",
        "PaneMorphService.current",
        "PaneMorphService.snapshot_for",
        "PaneMorphService.send",
        "PaneMorphService.bring",
        "PaneMorphService._rollback",
    ] {
        assert!(quals.contains(&q.to_string()), "missing {q}: {quals:?}");
    }
    let cls = &service.symbols[service.find("PaneMorphService").unwrap()];
    assert_eq!(cls.kind, SymKind::Class);
    assert_eq!(cls.start_line, 10);
    assert!(service.children(service.find("PaneMorphService").unwrap()).len() >= 15);

    let api = index.symbols("panemorph/api.py").unwrap();
    let quals: Vec<String> = api.symbols.iter().map(|s| s.qual()).collect();
    assert_eq!(
        quals,
        vec![
            "HerdrError",
            "HerdrClient",
            "HerdrClient.__init__",
            "HerdrClient.request",
            "HerdrClient.snapshot",
            "HerdrClient.current_pane",
            "HerdrClient.notify",
            "caller_pane_id"
        ]
    );
    let selector = index.symbols("panemorph/actions/selector.py").unwrap();
    let main = &selector.symbols[selector.find("main").unwrap()];
    assert_eq!((main.start_line, main.end_line), (75, 98));
}

#[test]
fn resolves_calls_across_files() {
    let index = panemorph_index();
    let main = edge_names(&index, "panemorph/actions/selector.py", "main");
    assert_eq!(
        main,
        vec![
            "api.py:HerdrClient",
            "service.py:PaneMorphService",
            "service.py:PaneMorphService.current",
            "api.py:caller_pane_id",
            "service.py:PaneMorphService.snapshot_for",
            "service.py:PaneMorphService.send_choices",
            "service.py:PaneMorphService.bring_choices",
        ]
    );
    // self.method() inside a class stays inside the class.
    let extract = edge_names(&index, "panemorph/service.py", "PaneMorphService.extract");
    assert!(extract.contains(&"service.py:PaneMorphService.snapshot_for".to_string()));
    assert!(extract.contains(&"service.py:PaneMorphService._id".to_string()));
    assert!(extract.contains(&"service.py:PaneMorphService._ensure_unzoomed".to_string()));
    // self.client.request(): a unique method name across the repo.
    assert!(extract.contains(&"api.py:HerdrClient.request".to_string()));
    // Generic names through unknown receivers are not guessed.
    let all_targets: Vec<String> = index
        .calls
        .iter()
        .map(|c| index.symbols(&c.to.file).unwrap().symbols[c.to.idx].name.clone())
        .collect();
    assert!(!all_targets.contains(&"get".to_string()));
}

#[test]
fn resolves_imports_to_files() {
    let index = panemorph_index();
    let service_imports: Vec<&String> = index.imports["panemorph/service.py"].iter().collect();
    assert_eq!(
        service_imports,
        vec!["panemorph/api.py", "panemorph/model.py", "panemorph/topology.py"]
    );
    let selector_imports: Vec<&String> = index.imports["panemorph/actions/selector.py"].iter().collect();
    assert_eq!(
        selector_imports,
        vec!["panemorph/api.py", "panemorph/model.py", "panemorph/service.py"]
    );
}

#[test]
fn callers_are_known() {
    let index = panemorph_index();
    let api = index.symbols("panemorph/api.py").unwrap();
    let target = SymId {
        file: "panemorph/api.py".into(),
        idx: api.find("caller_pane_id").unwrap(),
    };
    let mut callers: Vec<String> = index
        .callers_of(&target)
        .iter()
        .map(|c| c.from_file.clone())
        .collect();
    callers.sort();
    callers.dedup();
    assert!(callers.contains(&"panemorph/actions/selector.py".to_string()));
    assert!(callers.contains(&"panemorph/actions/extract.py".to_string()));
}

#[test]
fn rust_crate_paths_resolve() {
    let t = TempDir::new("rustcrate");
    write(t.path(), "src/lib.rs", "pub mod store;\npub mod git;\n");
    write(
        t.path(),
        "src/git/mod.rs",
        "pub mod diff;\npub fn run() -> String { diff::parse(\"\") }\n",
    );
    write(t.path(), "src/git/diff.rs", "pub fn parse(s: &str) -> String { s.to_string() }\n");
    write(
        t.path(),
        "src/store.rs",
        "use crate::git::diff::parse;\nuse crate::git;\npub struct Store;\nimpl Store {\n    pub fn open() -> Self { git::run(); parse(\"x\"); Store }\n    pub fn close(&self) { Self::open(); }\n}\n",
    );
    let files = git::walk_files(t.path(), 100);
    let mut index = Index::new(t.path(), files);
    index.ensure_parsed_under("", 100);
    index.resolve();
    let store_imports: Vec<&String> = index.imports["src/store.rs"].iter().collect();
    assert_eq!(store_imports, vec!["src/git/diff.rs", "src/git/mod.rs"]);
    let lib_imports: Vec<&String> = index.imports["src/lib.rs"].iter().collect();
    assert_eq!(lib_imports, vec!["src/git/mod.rs", "src/store.rs"]);
    assert_eq!(
        edge_names(&index, "src/store.rs", "Store.open"),
        vec!["mod.rs:run", "diff.rs:parse"]
    );
    assert_eq!(edge_names(&index, "src/store.rs", "Store.close"), vec!["store.rs:Store.open"]);
    assert_eq!(edge_names(&index, "src/git/mod.rs", "run"), vec!["diff.rs:parse"]);
}

#[test]
fn typescript_relative_imports_resolve() {
    let t = TempDir::new("ts");
    write(t.path(), "src/api.ts", "export function request(x: number) { return x; }\n");
    write(
        t.path(),
        "src/board.ts",
        "import { request } from \"./api\";\nexport class Board { draw() { return request(1); } }\n",
    );
    write(t.path(), "src/index.ts", "import { Board } from \"./board\";\nnew Board().draw();\n");
    let files = git::walk_files(t.path(), 100);
    let mut index = Index::new(t.path(), files);
    index.ensure_parsed_under("", 100);
    index.resolve();
    assert_eq!(edge_names(&index, "src/board.ts", "Board.draw"), vec!["api.ts:request"]);
    assert!(index.imports["src/index.ts"].contains("src/board.ts"));
    // module-level calls have no caller symbol but still resolve
    assert!(index.calls.iter().any(|c| c.from_file == "src/index.ts" && c.from_sym.is_none()));
}

#[test]
fn unsupported_files_are_listed_not_parsed() {
    let root = fixtures_dir().join("panemorph");
    let mut files = git::walk_files(&root, 10_000);
    files.push("README.md".into());
    let mut index = Index::new(&root, files);
    assert!(!index.ensure_parsed("README.md"));
    assert!(index.symbols("README.md").is_none());
    assert!(index.files.contains(&"README.md".to_string()));
}

#[test]
fn parse_cache_is_reused() {
    let t = TempDir::new("cache");
    write(t.path(), "a.py", "def f():\n    pass\n");
    let mut index = Index::new(t.path(), vec!["a.py".into()]);
    assert!(index.ensure_parsed("a.py"));
    assert!(!index.ensure_parsed("a.py"), "unchanged file must not be re-parsed");
    std::thread::sleep(std::time::Duration::from_millis(20));
    write(t.path(), "a.py", "def f():\n    pass\n\ndef g():\n    f()\n");
    assert!(index.ensure_parsed("a.py"));
    assert_eq!(index.symbols("a.py").unwrap().symbols.len(), 2);
}

#[test]
fn search_finds_symbols_and_files() {
    let index = panemorph_index();
    let hits = index.search("snapshot_for", 5);
    assert_eq!(hits[0].label, "PaneMorphService.snapshot_for");
    let hits = index.search("topology", 5);
    assert_eq!(hits[0].file, "panemorph/topology.py");
}

#[test]
fn real_rust_source_parses() {
    // codeMorph's own source doubles as a Rust fixture.
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/store.rs")).unwrap();
    let fs = lang::extract(Lang::Rust, &src);
    assert!(!fs.has_errors);
    let quals: Vec<String> = fs.symbols.iter().map(|s| s.qual()).collect();
    assert!(quals.contains(&"Shadow.snapshot".to_string()), "{quals:?}");
    assert!(quals.contains(&"Store.from_env".to_string()));
    assert!(quals.contains(&"format_comments".to_string()));
}
