//! The Flow builder: if/elif/else, loops, try/except, match, returns, for
//! Python, Rust, TypeScript and Go, and the layout of every paneMorph
//! function without overlapping boxes.

mod common;

use std::collections::BTreeMap;

use codemap::flow::{self, EndKind, Flow};
use codemap::git;
use codemap::index::Index;
use codemap::lang::{self, Lang};
use common::*;

fn chart(lang: Lang, src: &str, name: &str) -> flow::FlowChart {
    let fs = lang::extract(lang, src);
    let sym = fs
        .symbols
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no {name}"));
    flow::build(lang, src, sym).unwrap()
}

fn text_of(c: &flow::FlowChart) -> String {
    flow::layout(c).draw(&BTreeMap::new(), None).to_text()
}

#[test]
fn python_constructs() {
    let src = r#"def f(items):
    total = 0
    if not items:
        return 0
    elif len(items) > 10:
        raise ValueError("too many")
    for item in items:
        if item is None:
            continue
        total += item
    while total > 100:
        total -= 1
    try:
        save(total)
    except IOError as err:
        log(err)
    finally:
        close()
    match total:
        case 0:
            pass
        case _:
            print(total)
    return total
"#;
    let c = chart(Lang::Python, src, "f");
    let s = c.stats();
    assert_eq!(s["if"], 3, "{s:?}");
    assert_eq!(s["loop"], 2);
    assert_eq!(s["try"], 1);
    assert_eq!(s["handler"], 1);
    assert_eq!(s["switch"], 1);
    assert_eq!(s["return"], 2);
    assert_eq!(s["raise"], 1);
    assert_eq!(s["continue"], 1);
    // The first statement group, then the decision.
    match &c.body[..2] {
        [Flow::Stmts(lines), Flow::If { cond, .. }] => {
            assert_eq!(lines[0].text, "total = 0");
            assert_eq!(lines[0].line, 2);
            assert_eq!(cond.text, "not items");
        }
        other => panic!("unexpected {other:?}"),
    }
    let l = flow::layout(&c);
    assert!(l.overlaps().is_empty(), "{:?}", l.overlaps());
    let t = text_of(&c);
    for needle in [
        "▶ f()",
        "◇ not items",
        "◉ return 0",
        "✕ raise ValueError(\"too many\")",
        "↻ for item in items",
        "↺ continue",
        "↻ while total > 100",
        " try ",
        "except IOError as err",
        " finally ",
        "◇ match total",
        "case 0",
        "◉ return total",
        "yes",
        "each",
        "◀",
    ] {
        assert!(t.contains(needle), "missing {needle:?} in\n{t}");
    }
}

#[test]
fn rust_constructs() {
    let src = r#"fn run(xs: &[i32]) -> i32 {
    let mut n = 0;
    for x in xs {
        if *x < 0 {
            continue;
        }
        n += x;
    }
    loop {
        if n > 3 { break; }
        n += 1;
    }
    match n {
        0 => return -1,
        1 | 2 => { n += 10; }
        _ => {}
    }
    if n > 100 { n } else { n * 2 }
}
"#;
    let c = chart(Lang::Rust, src, "run");
    let s = c.stats();
    assert_eq!(s["loop"], 2, "{s:?}");
    assert_eq!(s["switch"], 1);
    assert_eq!(s["continue"], 1);
    assert_eq!(s["break"], 1);
    // `return -1` in an arm, and the tail `if` returns from both branches.
    assert_eq!(s["return"], 3, "{s:?}");
    assert!(flow::layout(&c).overlaps().is_empty());
    let t = text_of(&c);
    assert!(t.contains("◇ match n"), "{t}");
    assert!(t.contains("◉ n * 2"), "{t}");
}

#[test]
fn typescript_and_go_constructs() {
    let ts = "function f(x: number) {\n  if (x > 1) { return 1; } else { g(); }\n  for (const i of xs) { if (!i) continue; h(i); }\n  try { k(); } catch (e) { throw e; }\n  switch (x) { case 1: a(); break; default: b(); }\n  return 0;\n}\n";
    let c = chart(Lang::TypeScript, ts, "f");
    let s = c.stats();
    assert_eq!(
        (s["if"], s["loop"], s["try"], s["switch"]),
        (2, 1, 1, 1),
        "{s:?}"
    );
    assert_eq!(s["raise"], 1);
    assert!(flow::layout(&c).overlaps().is_empty());

    let go = "package m\nfunc f(xs []int) int {\n  total := 0\n  for _, x := range xs {\n    if x < 0 { continue }\n    total += x\n  }\n  switch total { case 0: return -1; default: total++ }\n  if total > 9 { panic(\"big\") }\n  return total\n}\n";
    let c = chart(Lang::Go, go, "f");
    let s = c.stats();
    assert_eq!((s["loop"], s["switch"], s["if"]), (1, 1, 2), "{s:?}");
    assert_eq!(s["raise"], 1);
    assert_eq!(s["return"], 2);
    assert!(flow::layout(&c).overlaps().is_empty());
}

#[test]
fn early_return_ends_the_path() {
    let src = "def g(x):\n    if x:\n        return 1\n    return 2\n";
    let c = chart(Lang::Python, src, "g");
    match &c.body[0] {
        Flow::If { yes, no, .. } => {
            assert!(matches!(
                yes[0],
                Flow::End {
                    kind: EndKind::Return,
                    ..
                }
            ));
            assert!(no.is_empty());
        }
        other => panic!("{other:?}"),
    }
    let t = text_of(&c);
    // The guard's return sits beside the decision on the same row.
    let row = t.lines().find(|l| l.contains("◇ x")).unwrap();
    assert!(row.contains("yes") && row.contains("◉ return 1"), "{t}");
}

#[test]
fn changed_lines_are_marked() {
    let src = "def h(a):\n    b = a + 1\n    c = b * 2\n    return c\n";
    let c = chart(Lang::Python, src, "h");
    let l = flow::layout(&c);
    let mut marks = BTreeMap::new();
    marks.insert(3u32, 'M');
    marks.insert(4u32, 'A');
    let canvas = l.draw(&marks, None);
    let t = canvas.to_text();
    let m_row = t.lines().find(|r| r.contains("c = b * 2")).unwrap();
    assert!(m_row.starts_with('M'), "{t}");
    let a_row = t.lines().find(|r| r.contains("◉ return c")).unwrap();
    assert!(a_row.starts_with('A'), "{t}");
    assert_eq!(l.box_for_line(2), l.box_for_line(3));
}

/// M2's exit criterion: every function in paneMorph draws with no
/// overlapping boxes.
#[test]
fn every_panemorph_function_lays_out_cleanly() {
    let root = fixtures_dir().join("panemorph");
    let files = git::walk_files(&root, 10_000);
    let mut index = Index::new(&root, files.clone());
    index.ensure_parsed_under("", 10_000);
    let mut charted = 0;
    for f in files.iter().filter(|f| f.ends_with(".py")) {
        let src = std::fs::read_to_string(root.join(f)).unwrap();
        let fs = index.symbols(f).unwrap();
        for s in fs.symbols.iter().filter(|s| s.kind.is_callable()) {
            let c = flow::build(Lang::Python, &src, s).unwrap_or_else(|| panic!("{f}:{}", s.name));
            let l = flow::layout(&c);
            assert!(
                l.overlaps().is_empty(),
                "{f}:{} overlaps {:?}",
                s.name,
                l.overlaps()
            );
            let canvas = l.draw(&BTreeMap::new(), None);
            assert!(canvas.w > 0 && canvas.h > 0);
            charted += 1;
        }
    }
    assert!(charted >= 38, "charted {charted} functions");
}
