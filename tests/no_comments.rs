use std::fs;
use std::path::{Path, PathBuf};

use tree_sitter::{Language, Node, Parser};

fn sources(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name != "fixtures") {
                sources(&path, ext, out);
            }
        } else if path.extension().is_some_and(|e| e == ext) {
            out.push(path);
        }
    }
}

fn first_statement(node: Node) -> bool {
    let mut prev = node.prev_named_sibling();
    while let Some(p) = prev {
        if p.kind() != "comment" {
            return false;
        }
        prev = p.prev_named_sibling();
    }
    true
}

fn is_docstring(node: Node) -> bool {
    node.kind() == "expression_statement"
        && node.named_child_count() == 1
        && node.named_child(0).is_some_and(|c| c.kind() == "string")
        && first_statement(node)
        && node
            .parent()
            .is_some_and(|p| matches!(p.kind(), "module" | "block"))
}

fn comment_rows(language: &Language, src: &str, python: bool) -> Vec<usize> {
    let mut parser = Parser::new();
    parser.set_language(language).expect("grammar loads");
    let tree = parser.parse(src, None).expect("source parses");
    let mut rows = Vec::new();
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        let comment = matches!(node.kind(), "line_comment" | "block_comment" | "comment");
        let shebang = python && node.start_byte() == 0 && src.starts_with("#!");
        if (comment && !shebang) || (python && is_docstring(node)) {
            rows.push(node.start_position().row + 1);
        } else if cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return rows;
            }
        }
    }
}

#[test]
fn code_has_no_comments() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut rust = Vec::new();
    for dir in ["src", "tests", "examples"] {
        sources(&root.join(dir), "rs", &mut rust);
    }
    let mut python = Vec::new();
    sources(&root.join("scripts"), "py", &mut python);
    assert!(rust.len() > 25, "sources not found");

    let rust_grammar: Language = tree_sitter_rust::LANGUAGE.into();
    let python_grammar: Language = tree_sitter_python::LANGUAGE.into();
    let mut found = Vec::new();
    for (paths, grammar, is_python) in [
        (&rust, &rust_grammar, false),
        (&python, &python_grammar, true),
    ] {
        for path in paths {
            let src = fs::read_to_string(path).expect("source is UTF-8");
            for row in comment_rows(grammar, &src, is_python) {
                let rel = path.strip_prefix(root).unwrap_or(path);
                found.push(format!("{}:{row}", rel.display()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "Comments are banned in code (CLAUDE.md). Remove them:\n{}",
        found.join("\n")
    );
}
