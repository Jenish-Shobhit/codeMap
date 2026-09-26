//! Import statements, read by walking the syntax tree.

use tree_sitter::Node;

use super::{text, ImportRef, Lang};

pub fn collect(lang: Lang, root: Node, src: &str) -> Vec<ImportRef> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let handled = match lang {
            Lang::Python => python(node, src, &mut out),
            Lang::Rust => rust(node, src, &mut out),
            Lang::TypeScript | Lang::Tsx | Lang::JavaScript => js(node, src, &mut out),
            Lang::Go => go(node, src, &mut out),
        };
        if !handled {
            let mut cursor = node.walk();
            let children: Vec<Node> = node.named_children(&mut cursor).collect();
            for child in children.into_iter().rev() {
                stack.push(child);
            }
        }
    }
    out.sort_by_key(|i| i.line);
    out
}

fn line(node: Node) -> u32 {
    node.start_position().row as u32 + 1
}

fn python(node: Node, src: &str, out: &mut Vec<ImportRef>) -> bool {
    match node.kind() {
        "import_statement" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                match child.kind() {
                    "dotted_name" => out.push(ImportRef {
                        module: text(child, src).to_string(),
                        line: line(node),
                        ..Default::default()
                    }),
                    "aliased_import" => out.push(ImportRef {
                        module: child
                            .child_by_field_name("name")
                            .map(|n| text(n, src).to_string())
                            .unwrap_or_default(),
                        alias: child
                            .child_by_field_name("alias")
                            .map(|n| text(n, src).to_string()),
                        line: line(node),
                        ..Default::default()
                    }),
                    _ => {}
                }
            }
            true
        }
        "import_from_statement" => {
            let mut imp = ImportRef {
                line: line(node),
                ..Default::default()
            };
            if let Some(m) = node.child_by_field_name("module_name") {
                let t = text(m, src);
                let level = t.chars().take_while(|c| *c == '.').count() as u32;
                imp.level = level;
                imp.module = t[level as usize..].to_string();
            }
            let mut cursor = node.walk();
            for child in node.children_by_field_name("name", &mut cursor) {
                match child.kind() {
                    "dotted_name" => imp.names.push((text(child, src).to_string(), None)),
                    "aliased_import" => imp.names.push((
                        child
                            .child_by_field_name("name")
                            .map(|n| text(n, src).to_string())
                            .unwrap_or_default(),
                        child
                            .child_by_field_name("alias")
                            .map(|n| text(n, src).to_string()),
                    )),
                    _ => {}
                }
            }
            let mut cursor = node.walk();
            if node
                .named_children(&mut cursor)
                .any(|c| c.kind() == "wildcard_import")
            {
                imp.names.push(("*".to_string(), None));
            }
            out.push(imp);
            true
        }
        _ => false,
    }
}

fn rust(node: Node, src: &str, out: &mut Vec<ImportRef>) -> bool {
    match node.kind() {
        "use_declaration" => {
            if let Some(arg) = node.child_by_field_name("argument") {
                let mut paths = Vec::new();
                flatten_use(arg, src, "", &mut paths);
                for (path, alias) in paths {
                    let last = path.rsplit("::").next().unwrap_or(&path).to_string();
                    out.push(ImportRef {
                        module: path,
                        names: vec![(last, alias.clone())],
                        alias,
                        line: line(node),
                        ..Default::default()
                    });
                }
            }
            true
        }
        // `mod foo;` pulls in foo.rs / foo/mod.rs.
        "mod_item" if node.child_by_field_name("body").is_none() => {
            if let Some(name) = node.child_by_field_name("name") {
                out.push(ImportRef {
                    module: format!("mod::{}", text(name, src)),
                    line: line(node),
                    ..Default::default()
                });
            }
            true
        }
        _ => false,
    }
}

fn join(prefix: &str, s: &str) -> String {
    if prefix.is_empty() {
        s.to_string()
    } else if s == "self" {
        prefix.to_string()
    } else {
        format!("{prefix}::{s}")
    }
}

fn flatten_use(node: Node, src: &str, prefix: &str, out: &mut Vec<(String, Option<String>)>) {
    match node.kind() {
        "use_as_clause" => {
            let path = node
                .child_by_field_name("path")
                .map(|n| text(n, src))
                .unwrap_or("");
            let alias = node
                .child_by_field_name("alias")
                .map(|n| text(n, src).to_string());
            out.push((join(prefix, path), alias));
        }
        "scoped_use_list" => {
            let path = node
                .child_by_field_name("path")
                .map(|n| text(n, src))
                .unwrap_or("");
            let new_prefix = join(prefix, path);
            if let Some(list) = node.child_by_field_name("list") {
                flatten_use(list, src, &new_prefix, out);
            }
        }
        "use_list" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                flatten_use(child, src, prefix, out);
            }
        }
        "use_wildcard" => {
            let t = text(node, src)
                .trim_end_matches("::*")
                .trim_end_matches('*');
            let base = join(prefix, t.trim_end_matches("::"));
            out.push((format!("{base}::*"), None));
        }
        "identifier" | "scoped_identifier" | "self" | "crate" | "super" => {
            out.push((join(prefix, text(node, src)), None));
        }
        _ => {}
    }
}

fn unquote(s: &str) -> String {
    s.trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .to_string()
}

fn js(node: Node, src: &str, out: &mut Vec<ImportRef>) -> bool {
    match node.kind() {
        "import_statement" => {
            let mut imp = ImportRef {
                line: line(node),
                ..Default::default()
            };
            if let Some(source) = node.child_by_field_name("source") {
                imp.module = unquote(text(source, src));
            }
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() != "import_clause" {
                    continue;
                }
                let mut c2 = child.walk();
                for part in child.named_children(&mut c2) {
                    match part.kind() {
                        "identifier" => imp
                            .names
                            .push(("default".to_string(), Some(text(part, src).to_string()))),
                        "namespace_import" => {
                            let mut c3 = part.walk();
                            let id = part
                                .named_children(&mut c3)
                                .find(|n| n.kind() == "identifier");
                            if let Some(id) = id {
                                imp.alias = Some(text(id, src).to_string());
                            }
                        }
                        "named_imports" => {
                            let mut c3 = part.walk();
                            for spec in part.named_children(&mut c3) {
                                if spec.kind() != "import_specifier" {
                                    continue;
                                }
                                let name = spec
                                    .child_by_field_name("name")
                                    .map(|n| text(n, src).to_string())
                                    .unwrap_or_default();
                                let alias = spec
                                    .child_by_field_name("alias")
                                    .map(|n| text(n, src).to_string());
                                imp.names.push((name, alias));
                            }
                        }
                        _ => {}
                    }
                }
            }
            out.push(imp);
            true
        }
        "export_statement" => {
            if let Some(source) = node.child_by_field_name("source") {
                out.push(ImportRef {
                    module: unquote(text(source, src)),
                    line: line(node),
                    ..Default::default()
                });
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

fn go(node: Node, src: &str, out: &mut Vec<ImportRef>) -> bool {
    match node.kind() {
        "import_spec" => {
            let module = node
                .child_by_field_name("path")
                .map(|n| unquote(text(n, src)))
                .unwrap_or_default();
            let alias = node
                .child_by_field_name("name")
                .map(|n| text(n, src).to_string())
                .or_else(|| module.rsplit('/').next().map(str::to_string));
            out.push(ImportRef {
                module,
                alias,
                line: line(node),
                ..Default::default()
            });
            true
        }
        _ => false,
    }
}
