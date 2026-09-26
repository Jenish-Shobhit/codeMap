//! Flow: the control flow of one function as a flowchart.
//!
//! The syntax tree becomes a small structured tree (statement groups,
//! decisions, loops, try/except, match, returns), which is laid out
//! recursively: a sequence runs down one spine; a branch puts its head on
//! the spine, the fall-through path below it, and the other paths in columns
//! to the right that merge back (or loop back) below.

use std::collections::BTreeMap;

use tree_sitter::Node;

use crate::canvas::{Band, Canvas, Tone, DOWN, RIGHT};
use crate::lang::{self, Lang, Symbol};
use crate::util;

const MAX_LINE: usize = 64;
const MAX_STMT_LINES: usize = 3;
const MAX_BOX_LINES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowLine {
    pub text: String,
    /// 1-based source line.
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndKind {
    Return,
    Raise,
    Break,
    Continue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Flow {
    Stmts(Vec<FlowLine>),
    If {
        cond: FlowLine,
        yes: Vec<Flow>,
        no: Vec<Flow>,
    },
    Loop {
        head: FlowLine,
        body: Vec<Flow>,
        orelse: Vec<Flow>,
    },
    Try {
        body: Vec<Flow>,
        handlers: Vec<(FlowLine, Vec<Flow>)>,
        orelse: Vec<Flow>,
        finally: Vec<Flow>,
    },
    Switch {
        head: FlowLine,
        arms: Vec<(FlowLine, Vec<Flow>)>,
    },
    End {
        kind: EndKind,
        line: FlowLine,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowChart {
    pub name: String,
    pub lang: Lang,
    pub start_line: u32,
    pub end_line: u32,
    pub body: Vec<Flow>,
}

impl FlowChart {
    /// Counts of each construct, for tests and the header.
    pub fn stats(&self) -> BTreeMap<&'static str, usize> {
        fn walk(items: &[Flow], m: &mut BTreeMap<&'static str, usize>) {
            for f in items {
                match f {
                    Flow::Stmts(_) => *m.entry("stmts").or_default() += 1,
                    Flow::If { yes, no, .. } => {
                        *m.entry("if").or_default() += 1;
                        walk(yes, m);
                        walk(no, m);
                    }
                    Flow::Loop { body, orelse, .. } => {
                        *m.entry("loop").or_default() += 1;
                        walk(body, m);
                        walk(orelse, m);
                    }
                    Flow::Try {
                        body,
                        handlers,
                        orelse,
                        finally,
                    } => {
                        *m.entry("try").or_default() += 1;
                        walk(body, m);
                        for (_, h) in handlers {
                            *m.entry("handler").or_default() += 1;
                            walk(h, m);
                        }
                        walk(orelse, m);
                        walk(finally, m);
                    }
                    Flow::Switch { arms, .. } => {
                        *m.entry("switch").or_default() += 1;
                        for (_, a) in arms {
                            walk(a, m);
                        }
                    }
                    Flow::End { kind, .. } => {
                        let k = match kind {
                            EndKind::Return => "return",
                            EndKind::Raise => "raise",
                            EndKind::Break => "break",
                            EndKind::Continue => "continue",
                        };
                        *m.entry(k).or_default() += 1;
                    }
                }
            }
        }
        let mut m = BTreeMap::new();
        walk(&self.body, &mut m);
        m
    }
}

fn text<'a>(n: Node, src: &'a str) -> &'a str {
    src.get(n.byte_range()).unwrap_or("")
}

fn line_of(n: Node) -> u32 {
    n.start_position().row as u32 + 1
}

/// One-line text of a node: whitespace collapsed, cut to MAX_LINE.
fn one_line(n: Node, src: &str) -> String {
    let t: String = text(n, src)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    util::truncate(&t, MAX_LINE)
}

fn fl(text: String, line: u32) -> FlowLine {
    FlowLine { text, line }
}

/// A statement as up to three display lines, dedented.
fn stmt_lines(n: Node, src: &str) -> Vec<FlowLine> {
    let raw = text(n, src);
    let first = line_of(n);
    let indent = n.start_position().column;
    let lines: Vec<&str> = raw.lines().collect();
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if out.len() == MAX_STMT_LINES - 1 && lines.len() > MAX_STMT_LINES {
            let last = out.last_mut().map(|f: &mut FlowLine| {
                f.text.push_str(" …");
            });
            let _ = last;
            break;
        }
        let l = if i == 0 {
            l.to_string()
        } else {
            let lead = l.len() - l.trim_start().len();
            let cut = lead.min(indent);
            l[cut..].to_string()
        };
        let l = util::sanitize_line(l.trim_end());
        if l.trim().is_empty() {
            continue;
        }
        out.push(fl(util::truncate(&l, MAX_LINE), first + i as u32));
    }
    if out.is_empty() {
        out.push(fl(String::new(), first));
    }
    out
}

/// Find the function's syntax node and build its flow.
pub fn build(lang: Lang, src: &str, sym: &Symbol) -> Option<FlowChart> {
    let tree = lang::parse(lang, src)?;
    let root = tree.root_node();
    let mut node = root.descendant_for_byte_range(sym.start_byte, sym.end_byte)?;
    // Walk up until the node covers the whole definition.
    while node.start_byte() > sym.start_byte || node.end_byte() < sym.end_byte {
        node = node.parent()?;
    }
    let body = function_body(lang, node)?;
    let items = match lang {
        Lang::Python => py_block(body, src),
        Lang::Rust => {
            if body.kind() == "block" {
                rs_block(body, src, true)
            } else {
                vec![Flow::End {
                    kind: EndKind::Return,
                    line: fl(one_line(body, src), line_of(body)),
                }]
            }
        }
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript => {
            if body.kind() == "statement_block" {
                js_block(body, src)
            } else {
                vec![Flow::End {
                    kind: EndKind::Return,
                    line: fl(format!("return {}", one_line(body, src)), line_of(body)),
                }]
            }
        }
        Lang::Go => go_block(body, src),
    };
    Some(FlowChart {
        name: sym.name.clone(),
        lang,
        start_line: sym.start_line,
        end_line: sym.end_line,
        body: merge_stmts(items),
    })
}

fn function_body(lang: Lang, node: Node) -> Option<Node> {
    if let Some(b) = node.child_by_field_name("body") {
        return Some(b);
    }
    // JS `const f = () => ...`: the value holds the function.
    if matches!(lang, Lang::TypeScript | Lang::Tsx | Lang::JavaScript) {
        if let Some(v) = node.child_by_field_name("value") {
            return v.child_by_field_name("body");
        }
    }
    None
}

/// Merge consecutive statement groups into boxes of up to MAX_BOX_LINES.
fn merge_stmts(items: Vec<Flow>) -> Vec<Flow> {
    let mut out: Vec<Flow> = Vec::new();
    for item in items {
        match (out.last_mut(), item) {
            (Some(Flow::Stmts(prev)), Flow::Stmts(lines))
                if prev.len() + lines.len() <= MAX_BOX_LINES =>
            {
                prev.extend(lines);
            }
            (_, item) => out.push(item),
        }
    }
    out
}

fn named_children<'t>(n: Node<'t>) -> Vec<Node<'t>> {
    let mut c = n.walk();
    n.named_children(&mut c).collect()
}

// ---- Python ------------------------------------------------------------

fn py_block(block: Node, src: &str) -> Vec<Flow> {
    let mut out = Vec::new();
    for st in named_children(block) {
        py_stmt(st, src, &mut out);
    }
    merge_stmts(out)
}

fn py_stmt(st: Node, src: &str, out: &mut Vec<Flow>) {
    match st.kind() {
        "comment" => {}
        "if_statement" => {
            let cond = st
                .child_by_field_name("condition")
                .map(|c| one_line(c, src))
                .unwrap_or_default();
            let yes = st
                .child_by_field_name("consequence")
                .map(|b| py_block(b, src))
                .unwrap_or_default();
            let mut cur = st.walk();
            let alts: Vec<Node> = st.children_by_field_name("alternative", &mut cur).collect();
            let no = py_alternatives(&alts, src);
            out.push(Flow::If {
                cond: fl(cond, line_of(st)),
                yes,
                no,
            });
        }
        "for_statement" | "while_statement" => {
            let head = if st.kind() == "for_statement" {
                let left = st
                    .child_by_field_name("left")
                    .map(|n| one_line(n, src))
                    .unwrap_or_default();
                let right = st
                    .child_by_field_name("right")
                    .map(|n| one_line(n, src))
                    .unwrap_or_default();
                format!("for {left} in {right}")
            } else {
                format!(
                    "while {}",
                    st.child_by_field_name("condition")
                        .map(|n| one_line(n, src))
                        .unwrap_or_default()
                )
            };
            let body = st
                .child_by_field_name("body")
                .map(|b| py_block(b, src))
                .unwrap_or_default();
            let orelse = st
                .child_by_field_name("alternative")
                .and_then(|e| e.child_by_field_name("body"))
                .map(|b| py_block(b, src))
                .unwrap_or_default();
            out.push(Flow::Loop {
                head: fl(util::truncate(&head, MAX_LINE), line_of(st)),
                body,
                orelse,
            });
        }
        "try_statement" => {
            let body = st
                .child_by_field_name("body")
                .map(|b| py_block(b, src))
                .unwrap_or_default();
            let mut handlers = Vec::new();
            let mut orelse = Vec::new();
            let mut finally = Vec::new();
            for ch in named_children(st) {
                match ch.kind() {
                    "except_clause" | "except_group_clause" => {
                        let head = match ch.child_by_field_name("value") {
                            Some(v) => format!("except {}", one_line(v, src)),
                            None => {
                                // older grammars: the first non-block child
                                let first =
                                    named_children(ch).into_iter().find(|c| c.kind() != "block");
                                match first {
                                    Some(v) => format!("except {}", one_line(v, src)),
                                    None => "except".to_string(),
                                }
                            }
                        };
                        let block = named_children(ch).into_iter().find(|c| c.kind() == "block");
                        handlers.push((
                            fl(util::truncate(&head, MAX_LINE), line_of(ch)),
                            block.map(|b| py_block(b, src)).unwrap_or_default(),
                        ));
                    }
                    "else_clause" => {
                        orelse = ch
                            .child_by_field_name("body")
                            .map(|b| py_block(b, src))
                            .unwrap_or_default();
                    }
                    "finally_clause" => {
                        finally = named_children(ch)
                            .into_iter()
                            .find(|c| c.kind() == "block")
                            .map(|b| py_block(b, src))
                            .unwrap_or_default();
                    }
                    _ => {}
                }
            }
            out.push(Flow::Try {
                body,
                handlers,
                orelse,
                finally,
            });
        }
        "with_statement" => {
            let clause = named_children(st)
                .into_iter()
                .find(|c| c.kind() == "with_clause")
                .map(|c| one_line(c, src))
                .unwrap_or_default();
            out.push(Flow::Stmts(vec![fl(
                util::truncate(&format!("with {clause}"), MAX_LINE),
                line_of(st),
            )]));
            if let Some(b) = st.child_by_field_name("body") {
                out.extend(py_block(b, src));
            }
        }
        "match_statement" => {
            let subject = st
                .child_by_field_name("subject")
                .map(|n| one_line(n, src))
                .unwrap_or_default();
            let mut arms = Vec::new();
            if let Some(body) = st.child_by_field_name("body") {
                for case in named_children(body) {
                    if case.kind() != "case_clause" {
                        continue;
                    }
                    let pats: Vec<String> = named_children(case)
                        .into_iter()
                        .filter(|c| c.kind() == "case_pattern")
                        .map(|c| {
                            let t = one_line(c, src);
                            if t.is_empty() {
                                "_".to_string()
                            } else {
                                t
                            }
                        })
                        .collect();
                    let guard = case
                        .child_by_field_name("guard")
                        .map(|g| format!(" {}", one_line(g, src)))
                        .unwrap_or_default();
                    let body = case
                        .child_by_field_name("consequence")
                        .map(|b| py_block(b, src))
                        .unwrap_or_default();
                    arms.push((
                        fl(format!("case {}{guard}", pats.join(", ")), line_of(case)),
                        body,
                    ));
                }
            }
            out.push(Flow::Switch {
                head: fl(format!("match {subject}"), line_of(st)),
                arms,
            });
        }
        "return_statement" | "raise_statement" | "break_statement" | "continue_statement" => {
            let kind = match st.kind() {
                "return_statement" => EndKind::Return,
                "raise_statement" => EndKind::Raise,
                "break_statement" => EndKind::Break,
                _ => EndKind::Continue,
            };
            out.push(Flow::End {
                kind,
                line: fl(one_line(st, src), line_of(st)),
            });
        }
        "function_definition" | "class_definition" | "decorated_definition" => {
            let def = if st.kind() == "decorated_definition" {
                st.child_by_field_name("definition").unwrap_or(st)
            } else {
                st
            };
            let name = def
                .child_by_field_name("name")
                .map(|n| text(n, src))
                .unwrap_or("");
            let kw = if def.kind() == "class_definition" {
                "class"
            } else {
                "def"
            };
            let tail = if kw == "def" { "(…)" } else { "" };
            out.push(Flow::Stmts(vec![fl(
                format!("{kw} {name}{tail}"),
                line_of(st),
            )]));
        }
        _ => out.push(Flow::Stmts(stmt_lines(st, src))),
    }
}

fn py_alternatives(alts: &[Node], src: &str) -> Vec<Flow> {
    let Some((first, rest)) = alts.split_first() else {
        return Vec::new();
    };
    match first.kind() {
        "elif_clause" => {
            let cond = first
                .child_by_field_name("condition")
                .map(|c| one_line(c, src))
                .unwrap_or_default();
            let yes = first
                .child_by_field_name("consequence")
                .map(|b| py_block(b, src))
                .unwrap_or_default();
            vec![Flow::If {
                cond: fl(cond, line_of(*first)),
                yes,
                no: py_alternatives(rest, src),
            }]
        }
        "else_clause" => first
            .child_by_field_name("body")
            .map(|b| py_block(b, src))
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

// ---- Rust --------------------------------------------------------------

fn rs_is_control(kind: &str) -> bool {
    matches!(
        kind,
        "if_expression"
            | "match_expression"
            | "loop_expression"
            | "while_expression"
            | "for_expression"
            | "return_expression"
            | "break_expression"
            | "continue_expression"
    )
}

fn rs_block(block: Node, src: &str, tail_returns: bool) -> Vec<Flow> {
    let mut out = Vec::new();
    let children: Vec<Node> = named_children(block)
        .into_iter()
        .filter(|c| !matches!(c.kind(), "line_comment" | "block_comment"))
        .collect();
    let n = children.len();
    for (i, st) in children.into_iter().enumerate() {
        let is_tail = i + 1 == n
            && !matches!(st.kind(), "expression_statement" | "let_declaration")
            && !st.kind().ends_with("_item");
        match st.kind() {
            "expression_statement" => {
                let inner = st.named_child(0);
                // A block-like tail expression (`if`/`match` without `;`) is
                // still the block's value: it returns from the function.
                let tail_value = i + 1 == n && !text(st, src).trim_end().ends_with(';');
                match inner {
                    Some(e) if rs_is_control(e.kind()) => {
                        rs_expr(e, src, tail_value && tail_returns, &mut out)
                    }
                    _ => out.push(Flow::Stmts(stmt_lines(st, src))),
                }
            }
            k if is_tail && rs_is_control(k) => rs_expr(st, src, tail_returns, &mut out),
            _ if is_tail && tail_returns => out.push(Flow::End {
                kind: EndKind::Return,
                line: fl(one_line(st, src), line_of(st)),
            }),
            k if k.ends_with("_item") => {
                let name = st
                    .child_by_field_name("name")
                    .map(|n| text(n, src))
                    .unwrap_or("");
                let kw = k.trim_end_matches("_item");
                out.push(Flow::Stmts(vec![fl(format!("{kw} {name}"), line_of(st))]));
            }
            _ => out.push(Flow::Stmts(stmt_lines(st, src))),
        }
    }
    merge_stmts(out)
}

fn rs_branch_body(n: Node, src: &str, tail_returns: bool) -> Vec<Flow> {
    if n.kind() == "block" {
        return rs_block(n, src, tail_returns);
    }
    let mut out = Vec::new();
    if rs_is_control(n.kind()) {
        rs_expr(n, src, tail_returns, &mut out);
    } else if tail_returns {
        out.push(Flow::End {
            kind: EndKind::Return,
            line: fl(one_line(n, src), line_of(n)),
        });
    } else {
        out.push(Flow::Stmts(vec![fl(one_line(n, src), line_of(n))]));
    }
    out
}

fn rs_expr(e: Node, src: &str, tail_returns: bool, out: &mut Vec<Flow>) {
    match e.kind() {
        "if_expression" => {
            let cond = e
                .child_by_field_name("condition")
                .map(|c| one_line(c, src))
                .unwrap_or_default();
            let yes = e
                .child_by_field_name("consequence")
                .map(|b| rs_block(b, src, tail_returns))
                .unwrap_or_default();
            let no = e
                .child_by_field_name("alternative")
                .and_then(|alt| alt.named_child(0))
                .map(|b| rs_branch_body(b, src, tail_returns))
                .unwrap_or_default();
            out.push(Flow::If {
                cond: fl(cond, line_of(e)),
                yes,
                no,
            });
        }
        "match_expression" => {
            let value = e
                .child_by_field_name("value")
                .map(|c| one_line(c, src))
                .unwrap_or_default();
            let mut arms = Vec::new();
            if let Some(body) = e.child_by_field_name("body") {
                for arm in named_children(body) {
                    if arm.kind() != "match_arm" {
                        continue;
                    }
                    let pat = arm
                        .child_by_field_name("pattern")
                        .map(|p| one_line(p, src))
                        .unwrap_or_default();
                    let body = arm
                        .child_by_field_name("value")
                        .map(|v| rs_branch_body(v, src, tail_returns))
                        .unwrap_or_default();
                    arms.push((fl(pat, line_of(arm)), body));
                }
            }
            out.push(Flow::Switch {
                head: fl(format!("match {value}"), line_of(e)),
                arms,
            });
        }
        "loop_expression" | "while_expression" | "for_expression" => {
            let head = match e.kind() {
                "loop_expression" => "loop".to_string(),
                "while_expression" => format!(
                    "while {}",
                    e.child_by_field_name("condition")
                        .map(|c| one_line(c, src))
                        .unwrap_or_default()
                ),
                _ => format!(
                    "for {} in {}",
                    e.child_by_field_name("pattern")
                        .map(|c| one_line(c, src))
                        .unwrap_or_default(),
                    e.child_by_field_name("value")
                        .map(|c| one_line(c, src))
                        .unwrap_or_default()
                ),
            };
            let body = e
                .child_by_field_name("body")
                .map(|b| rs_block(b, src, false))
                .unwrap_or_default();
            out.push(Flow::Loop {
                head: fl(util::truncate(&head, MAX_LINE), line_of(e)),
                body,
                orelse: Vec::new(),
            });
        }
        "return_expression" | "break_expression" | "continue_expression" => {
            let kind = match e.kind() {
                "return_expression" => EndKind::Return,
                "break_expression" => EndKind::Break,
                _ => EndKind::Continue,
            };
            out.push(Flow::End {
                kind,
                line: fl(one_line(e, src), line_of(e)),
            });
        }
        _ => out.push(Flow::Stmts(stmt_lines(e, src))),
    }
}

// ---- JavaScript / TypeScript ------------------------------------------

fn js_block(block: Node, src: &str) -> Vec<Flow> {
    let mut out = Vec::new();
    if block.kind() == "statement_block" {
        for st in named_children(block) {
            js_stmt(st, src, &mut out);
        }
    } else {
        js_stmt(block, src, &mut out);
    }
    merge_stmts(out)
}

fn strip_parens(s: String) -> String {
    let t = s.trim();
    if t.starts_with('(') && t.ends_with(')') {
        t[1..t.len() - 1].trim().to_string()
    } else {
        t.to_string()
    }
}

/// Text of a statement up to its body (a loop header).
fn head_text(st: Node, body: Option<Node>, src: &str) -> String {
    let end = body.map(|b| b.start_byte()).unwrap_or(st.end_byte());
    let t = src.get(st.start_byte()..end).unwrap_or("");
    let t: String = t.split_whitespace().collect::<Vec<_>>().join(" ");
    util::truncate(t.trim_end_matches('{').trim(), MAX_LINE)
}

fn js_stmt(st: Node, src: &str, out: &mut Vec<Flow>) {
    match st.kind() {
        "comment" | "empty_statement" => {}
        "if_statement" => {
            let cond = st
                .child_by_field_name("condition")
                .map(|c| strip_parens(one_line(c, src)))
                .unwrap_or_default();
            let yes = st
                .child_by_field_name("consequence")
                .map(|b| js_block(b, src))
                .unwrap_or_default();
            let no = st
                .child_by_field_name("alternative")
                .and_then(|alt| alt.named_child(0))
                .map(|b| js_block(b, src))
                .unwrap_or_default();
            out.push(Flow::If {
                cond: fl(cond, line_of(st)),
                yes,
                no,
            });
        }
        "for_statement" | "for_in_statement" | "while_statement" | "do_statement" => {
            let body = st.child_by_field_name("body");
            let head = if st.kind() == "do_statement" {
                format!(
                    "do … while {}",
                    st.child_by_field_name("condition")
                        .map(|c| strip_parens(one_line(c, src)))
                        .unwrap_or_default()
                )
            } else {
                head_text(st, body, src)
            };
            out.push(Flow::Loop {
                head: fl(head, line_of(st)),
                body: body.map(|b| js_block(b, src)).unwrap_or_default(),
                orelse: Vec::new(),
            });
        }
        "try_statement" => {
            let body = st
                .child_by_field_name("body")
                .map(|b| js_block(b, src))
                .unwrap_or_default();
            let mut handlers = Vec::new();
            if let Some(h) = st.child_by_field_name("handler") {
                let param = h
                    .child_by_field_name("parameter")
                    .map(|p| format!(" ({})", one_line(p, src)))
                    .unwrap_or_default();
                handlers.push((
                    fl(format!("catch{param}"), line_of(h)),
                    h.child_by_field_name("body")
                        .map(|b| js_block(b, src))
                        .unwrap_or_default(),
                ));
            }
            let finally = st
                .child_by_field_name("finalizer")
                .and_then(|f| f.child_by_field_name("body"))
                .map(|b| js_block(b, src))
                .unwrap_or_default();
            out.push(Flow::Try {
                body,
                handlers,
                orelse: Vec::new(),
                finally,
            });
        }
        "switch_statement" => {
            let value = st
                .child_by_field_name("value")
                .map(|v| strip_parens(one_line(v, src)))
                .unwrap_or_default();
            let mut arms = Vec::new();
            if let Some(body) = st.child_by_field_name("body") {
                for case in named_children(body) {
                    let label = match case.kind() {
                        "switch_case" => format!(
                            "case {}",
                            case.child_by_field_name("value")
                                .map(|v| one_line(v, src))
                                .unwrap_or_default()
                        ),
                        "switch_default" => "default".to_string(),
                        _ => continue,
                    };
                    let mut items = Vec::new();
                    let mut c = case.walk();
                    for b in case.children_by_field_name("body", &mut c) {
                        js_stmt(b, src, &mut items);
                    }
                    arms.push((fl(label, line_of(case)), merge_stmts(items)));
                }
            }
            out.push(Flow::Switch {
                head: fl(format!("switch {value}"), line_of(st)),
                arms,
            });
        }
        "return_statement" | "throw_statement" | "break_statement" | "continue_statement" => {
            let kind = match st.kind() {
                "return_statement" => EndKind::Return,
                "throw_statement" => EndKind::Raise,
                "break_statement" => EndKind::Break,
                _ => EndKind::Continue,
            };
            out.push(Flow::End {
                kind,
                line: fl(
                    one_line(st, src).trim_end_matches(';').to_string(),
                    line_of(st),
                ),
            });
        }
        "statement_block" => out.extend(js_block(st, src)),
        "function_declaration" | "class_declaration" | "generator_function_declaration" => {
            let name = st
                .child_by_field_name("name")
                .map(|n| text(n, src))
                .unwrap_or("");
            let kw = if st.kind() == "class_declaration" {
                "class"
            } else {
                "function"
            };
            out.push(Flow::Stmts(vec![fl(format!("{kw} {name}"), line_of(st))]));
        }
        _ => out.push(Flow::Stmts(stmt_lines(st, src))),
    }
}

// ---- Go ----------------------------------------------------------------

fn go_block(block: Node, src: &str) -> Vec<Flow> {
    let mut out = Vec::new();
    let mut stmts = Vec::new();
    for c in named_children(block) {
        if c.kind() == "statement_list" {
            stmts.extend(named_children(c));
        } else {
            stmts.push(c);
        }
    }
    for st in stmts {
        go_stmt(st, src, &mut out);
    }
    merge_stmts(out)
}

fn go_stmt(st: Node, src: &str, out: &mut Vec<Flow>) {
    match st.kind() {
        "comment" | "empty_statement" => {}
        "if_statement" => {
            let init = st
                .child_by_field_name("initializer")
                .map(|i| format!("{}; ", one_line(i, src)))
                .unwrap_or_default();
            let cond = st
                .child_by_field_name("condition")
                .map(|c| one_line(c, src))
                .unwrap_or_default();
            let yes = st
                .child_by_field_name("consequence")
                .map(|b| go_block(b, src))
                .unwrap_or_default();
            let no = st
                .child_by_field_name("alternative")
                .map(|a| {
                    if a.kind() == "block" {
                        go_block(a, src)
                    } else {
                        let mut v = Vec::new();
                        go_stmt(a, src, &mut v);
                        v
                    }
                })
                .unwrap_or_default();
            out.push(Flow::If {
                cond: fl(
                    util::truncate(&format!("{init}{cond}"), MAX_LINE),
                    line_of(st),
                ),
                yes,
                no,
            });
        }
        "for_statement" => {
            let body = st.child_by_field_name("body");
            out.push(Flow::Loop {
                head: fl(head_text(st, body, src), line_of(st)),
                body: body.map(|b| go_block(b, src)).unwrap_or_default(),
                orelse: Vec::new(),
            });
        }
        "expression_switch_statement" | "type_switch_statement" | "select_statement" => {
            let cases: Vec<Node> = named_children(st)
                .into_iter()
                .filter(|c| c.kind().ends_with("_case"))
                .collect();
            let head = head_text(st, cases.first().copied(), src);
            let mut arms = Vec::new();
            for case in cases {
                let label = match case.kind() {
                    "default_case" => "default".to_string(),
                    _ => {
                        let v = case
                            .child_by_field_name("value")
                            .or_else(|| case.child_by_field_name("type"))
                            .or_else(|| case.child_by_field_name("communication"))
                            .map(|v| one_line(v, src))
                            .unwrap_or_default();
                        format!("case {v}")
                    }
                };
                let mut items = Vec::new();
                for c in named_children(case) {
                    if c.kind() == "statement_list" {
                        for s in named_children(c) {
                            go_stmt(s, src, &mut items);
                        }
                    }
                }
                arms.push((
                    fl(util::truncate(&label, MAX_LINE), line_of(case)),
                    merge_stmts(items),
                ));
            }
            out.push(Flow::Switch {
                head: fl(head, line_of(st)),
                arms,
            });
        }
        "return_statement" | "break_statement" | "continue_statement" | "goto_statement" => {
            let kind = match st.kind() {
                "return_statement" => EndKind::Return,
                "break_statement" => EndKind::Break,
                "continue_statement" => EndKind::Continue,
                _ => EndKind::Break,
            };
            out.push(Flow::End {
                kind,
                line: fl(one_line(st, src), line_of(st)),
            });
        }
        "block" => out.extend(go_block(st, src)),
        _ => {
            // panic(...) ends the path like a raise
            let t = one_line(st, src);
            if t.starts_with("panic(") {
                out.push(Flow::End {
                    kind: EndKind::Raise,
                    line: fl(t, line_of(st)),
                });
            } else {
                out.push(Flow::Stmts(stmt_lines(st, src)));
            }
        }
    }
}

// ---- Layout ------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxKind {
    Stmt,
    Decision,
    Loop,
    Try,
    Handler,
    Switch,
    End(EndKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowBox {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    pub title: Option<String>,
    pub lines: Vec<FlowLine>,
    pub kind: BoxKind,
    /// Row of the box used by an outgoing bus (branches).
    pub bus_row: usize,
}

impl FlowBox {
    pub fn first_line(&self) -> u32 {
        self.lines.first().map(|l| l.line).unwrap_or(0)
    }
    pub fn last_line(&self) -> u32 {
        self.lines.iter().map(|l| l.line).max().unwrap_or(0)
    }
}

#[derive(Debug, Clone)]
enum Op {
    Poly(Vec<(usize, usize)>),
    Arrow(usize, usize, char),
    /// A text label; `true` when it may sit on a horizontal line (a bus).
    Label(usize, usize, String, bool),
}

#[derive(Debug, Clone)]
struct Blk {
    w: usize,
    h: usize,
    spine: usize,
    exit: bool,
    boxes: Vec<FlowBox>,
    ops: Vec<Op>,
}

impl Blk {
    fn shift(mut self, dx: usize, dy: usize) -> Blk {
        for b in &mut self.boxes {
            b.x += dx;
            b.y += dy;
        }
        for op in &mut self.ops {
            match op {
                Op::Poly(pts) => {
                    for p in pts {
                        p.0 += dx;
                        p.1 += dy;
                    }
                }
                Op::Arrow(x, y, _) | Op::Label(x, y, _, _) => {
                    *x += dx;
                    *y += dy;
                }
            }
        }
        self
    }

    fn absorb(&mut self, other: Blk) {
        self.boxes.extend(other.boxes);
        self.ops.extend(other.ops);
    }

    /// A single box with nothing else in it (for inline side placement).
    fn is_single_box(&self) -> bool {
        self.boxes.len() == 1 && self.ops.is_empty()
    }
}

fn end_prefix(kind: EndKind) -> &'static str {
    match kind {
        EndKind::Return => "◉ ",
        EndKind::Raise => "✕ ",
        EndKind::Break => "↓ ",
        EndKind::Continue => "↺ ",
    }
}

fn box_blk(title: Option<String>, lines: Vec<FlowLine>, kind: BoxKind, extra_rows: usize) -> Blk {
    let mut w = lines
        .iter()
        .map(|l| util::width(&l.text))
        .max()
        .unwrap_or(0)
        + 4;
    if let Some(t) = &title {
        w = w.max(util::width(t) + 6);
    }
    let w = w.max(9);
    let h = lines.len().max(1) + 2 + extra_rows;
    // Loops take the bus on their first row (the second is the loop-back
    // entry); terminal boxes are entered on their first line.
    let bus_row = if matches!(kind, BoxKind::Loop | BoxKind::End(_)) {
        1
    } else {
        h / 2
    };
    let exit = !matches!(kind, BoxKind::End(_));
    Blk {
        w,
        h,
        spine: w / 2,
        exit,
        boxes: vec![FlowBox {
            x: 0,
            y: 0,
            w,
            h,
            title,
            lines,
            kind,
            bus_row,
        }],
        ops: Vec::new(),
    }
}

fn empty_blk() -> Blk {
    Blk {
        w: 1,
        h: 0,
        spine: 0,
        exit: true,
        boxes: Vec::new(),
        ops: Vec::new(),
    }
}

/// Stack blocks on one spine with ▼ connectors.
fn seq(blocks: Vec<Blk>) -> Blk {
    let blocks: Vec<Blk> = blocks.into_iter().filter(|b| b.h > 0).collect();
    if blocks.is_empty() {
        return empty_blk();
    }
    let spine = blocks.iter().map(|b| b.spine).max().unwrap_or(0);
    let mut out = Blk {
        w: 0,
        h: 0,
        spine,
        exit: true,
        boxes: Vec::new(),
        ops: Vec::new(),
    };
    let mut y = 0usize;
    let mut prev_exit: Option<bool> = None;
    for b in blocks {
        if let Some(pe) = prev_exit {
            if pe {
                // from the previous block's bottom row into an arrow
                out.ops.push(Op::Poly(vec![(spine, y - 1), (spine, y)]));
                out.ops.push(Op::Arrow(spine, y, '▼'));
                y += 1;
            } else {
                // unreachable code after a return: leave a gap, no arrow
                y += 1;
            }
        }
        let x = spine - b.spine;
        let bw = x + b.w;
        let bh = b.h;
        prev_exit = Some(b.exit);
        let shifted = b.shift(x, y);
        out.w = out.w.max(bw);
        out.absorb(shifted);
        y += bh;
    }
    out.h = y;
    out.exit = prev_exit.unwrap_or(true);
    out
}

struct Side {
    label: String,
    blk: Blk,
    back_edge: bool,
}

/// A head box with a fall-through path below and side paths to the right.
fn branch(head: Blk, down: Option<(String, Blk)>, sides: Vec<Side>) -> Blk {
    let head_box = head.boxes[0].clone();
    let bus_row = head_box.bus_row;
    let down = down.filter(|(_, b)| b.h > 0);
    let spine = head
        .spine
        .max(down.as_ref().map(|(_, b)| b.spine).unwrap_or(0));
    let head_x = spine - head.spine;
    let mut out = Blk {
        w: 0,
        h: 0,
        spine,
        exit: false,
        boxes: Vec::new(),
        ops: Vec::new(),
    };
    let head_h = head.h;
    let head_right = head_x + head.w - 1;
    out.absorb(head.shift(head_x, 0));
    let mut right_edge = head_right + 1;
    let mut bottom = head_h - 1; // last used row
                                 // The fall-through path.
    let mut down_end: Option<usize> = Some(head_h - 1); // row where the spine path ends
    if let Some((label, d)) = down {
        let dx = spine - d.spine;
        let dy = head_h + 1;
        out.ops
            .push(Op::Poly(vec![(spine, head_h - 1), (spine, head_h)]));
        out.ops.push(Op::Arrow(spine, head_h, '▼'));
        if !label.is_empty() {
            out.ops.push(Op::Label(spine + 2, head_h, label, false));
        }
        right_edge = right_edge.max(dx + d.w);
        bottom = bottom.max(dy + d.h - 1);
        down_end = if d.exit { Some(dy + d.h - 1) } else { None };
        out.absorb(d.shift(dx, dy));
    }
    let has_back = sides.iter().any(|s| s.back_edge);
    let inline = sides.len() == 1 && !has_back && sides[0].blk.is_single_box();
    let mut merges: Vec<(usize, usize)> = Vec::new(); // (x, bottom row) of side exits
    if inline {
        // One small side box on the same rows as the head.
        let s = sides.into_iter().next().unwrap();
        let sb = &s.blk.boxes[0];
        let label = format!(" {} ", s.label);
        let gap = util::width(&label) + 6;
        let sy = bus_row.saturating_sub(sb.bus_row);
        // A side that ends (return, raise...) needs no merge line, so it can
        // sit right beside the head when it stays above the fall-through.
        let beside = !s.blk.exit && sy + s.blk.h <= head_h + 1;
        let sx = if beside {
            head_right + 1 + gap
        } else {
            right_edge.max(head_right + 1) + gap
        };
        let s_bus = sy + sb.bus_row;
        out.ops
            .push(Op::Poly(vec![(head_right, bus_row), (sx - 1, s_bus)]));
        out.ops.push(Op::Arrow(sx - 1, s_bus, '▶'));
        let mid = head_right + 1 + (sx - 1 - head_right - util::width(&label)) / 2;
        out.ops.push(Op::Label(mid, bus_row, label, true));
        let sh = s.blk.h;
        let sw = s.blk.w;
        let spine_i = sx + s.blk.spine;
        let exit = s.blk.exit;
        out.absorb(s.blk.shift(sx, sy));
        right_edge = right_edge.max(sx + sw);
        bottom = bottom.max(sy + sh - 1);
        if exit {
            merges.push((spine_i, sy + sh - 1));
        }
    } else if !sides.is_empty() {
        let back_col = right_edge + 2;
        let mut sx = right_edge + if has_back { 5 } else { 3 };
        let side_top = head_h + 1;
        let n = sides.len();
        let mut spines = Vec::new();
        for s in sides {
            let spine_i = sx + s.blk.spine;
            spines.push(spine_i);
            // drop from the bus into the side
            out.ops
                .push(Op::Poly(vec![(spine_i, bus_row), (spine_i, side_top - 1)]));
            out.ops.push(Op::Arrow(spine_i, side_top - 1, '▼'));
            if !s.label.is_empty() {
                out.ops
                    .push(Op::Label(spine_i + 2, bus_row + 1, s.label.clone(), false));
            }
            let sh = s.blk.h;
            let sw = s.blk.w;
            let exit = s.blk.exit;
            let back = s.back_edge;
            out.absorb(s.blk.shift(sx, side_top));
            let sbottom = side_top + sh - 1;
            bottom = bottom.max(sbottom);
            if back && exit {
                // loop back into the head's second row
                let turn = sbottom + 1;
                out.ops.push(Op::Poly(vec![
                    (spine_i, sbottom),
                    (spine_i, turn),
                    (back_col, turn),
                    (back_col, 2),
                    (head_right + 1, 2),
                ]));
                out.ops.push(Op::Arrow(head_right + 1, 2, '◀'));
                bottom = bottom.max(turn);
            } else if exit {
                merges.push((spine_i, sbottom));
            }
            right_edge = right_edge.max(sx + sw);
            sx += sw + 3;
        }
        // the bus: from the head's right border to the last drop
        let last = *spines.last().unwrap();
        out.ops
            .push(Op::Poly(vec![(head_right, bus_row), (last, bus_row)]));
        let _ = n;
    }
    // Merge surviving paths below everything.
    if merges.is_empty() {
        if let Some(end) = down_end {
            if end < bottom {
                out.ops.push(Op::Poly(vec![(spine, end), (spine, bottom)]));
            }
            out.exit = true;
            out.h = bottom + 1;
        } else {
            out.exit = false;
            out.h = bottom + 1;
        }
    } else {
        let m = bottom + 1;
        for (x, b) in &merges {
            out.ops.push(Op::Poly(vec![(*x, *b), (*x, m), (spine, m)]));
        }
        if let Some(end) = down_end {
            out.ops.push(Op::Poly(vec![(spine, end), (spine, m)]));
        }
        out.exit = true;
        out.h = m + 1;
    }
    out.w = right_edge.max(out.w).max(head_right + 1);
    out
}

fn is_simple(items: &[Flow]) -> Option<Vec<FlowLine>> {
    match items {
        [Flow::Stmts(lines)] => Some(lines.clone()),
        [] => Some(Vec::new()),
        _ => None,
    }
}

/// A side branch that runs a few statements and then ends (a guard
/// clause) becomes one terminal box, so it can sit beside its decision.
fn lay_branch(items: &[Flow]) -> Blk {
    if let [Flow::Stmts(lines), Flow::End { kind, line }] = items {
        if lines.len() <= 3 {
            let mut all: Vec<FlowLine> = lines
                .iter()
                .map(|l| fl(util::truncate(&l.text, 48), l.line))
                .collect();
            let t = format!("{}{}", end_prefix(*kind), line.text);
            all.push(fl(util::truncate(&t, 48), line.line));
            return box_blk(None, all, BoxKind::End(*kind), 0);
        }
    }
    lay(items)
}

fn lay(items: &[Flow]) -> Blk {
    let blocks: Vec<Blk> = items.iter().map(lay_one).collect();
    seq(blocks)
}

fn lay_one(f: &Flow) -> Blk {
    match f {
        Flow::Stmts(lines) => box_blk(None, lines.clone(), BoxKind::Stmt, 0),
        Flow::End { kind, line } => {
            let t = format!("{}{}", end_prefix(*kind), line.text);
            box_blk(
                None,
                vec![fl(util::truncate(&t, MAX_LINE), line.line)],
                BoxKind::End(*kind),
                0,
            )
        }
        Flow::If { cond, yes, no } => {
            let head = box_blk(
                None,
                vec![fl(
                    util::truncate(&format!("◇ {}", cond.text), MAX_LINE),
                    cond.line,
                )],
                BoxKind::Decision,
                0,
            );
            let yes_blk = if yes.is_empty() {
                box_blk(None, vec![fl("pass".into(), cond.line)], BoxKind::Stmt, 0)
            } else {
                lay_branch(yes)
            };
            let down = if no.is_empty() {
                None
            } else {
                Some(("no".to_string(), lay(no)))
            };
            let mut b = branch(
                head,
                down,
                vec![Side {
                    label: "yes".into(),
                    blk: yes_blk,
                    back_edge: false,
                }],
            );
            if no.is_empty() && b.exit {
                // label the fall-through when there is no else
                b.ops.push(Op::Label(b.spine + 2, 3, "no".into(), false));
            }
            b
        }
        Flow::Loop { head, body, orelse } => {
            let h = box_blk(
                None,
                vec![fl(
                    util::truncate(&format!("↻ {}", head.text), MAX_LINE),
                    head.line,
                )],
                BoxKind::Loop,
                1,
            );
            let body_blk = if body.is_empty() {
                box_blk(None, vec![fl("pass".into(), head.line)], BoxKind::Stmt, 0)
            } else {
                lay(body)
            };
            let down = if orelse.is_empty() {
                None
            } else {
                Some(("done".to_string(), titled(Some("else".into()), orelse)))
            };
            let mut b = branch(
                h,
                down,
                vec![Side {
                    label: "each".into(),
                    blk: body_blk,
                    back_edge: true,
                }],
            );
            if orelse.is_empty() && b.exit {
                b.ops.push(Op::Label(b.spine + 2, 4, "done".into(), false));
            }
            b
        }
        Flow::Try {
            body,
            handlers,
            orelse,
            finally,
        } => {
            let (head, down_items): (Blk, Vec<Blk>) = match is_simple(body) {
                Some(lines) if !lines.is_empty() => {
                    (box_blk(Some("try".into()), lines, BoxKind::Try, 0), vec![])
                }
                _ => (
                    box_blk(None, vec![fl("try".into(), 0)], BoxKind::Try, 0),
                    vec![lay(body)],
                ),
            };
            let mut down_blocks = down_items;
            if !orelse.is_empty() {
                down_blocks.push(titled(Some("else".into()), orelse));
            }
            let down = if down_blocks.is_empty() {
                None
            } else {
                Some((String::new(), seq(down_blocks)))
            };
            let sides = handlers
                .iter()
                .map(|(h, items)| Side {
                    label: "raises".into(),
                    blk: titled_handler(h, items),
                    back_edge: false,
                })
                .collect::<Vec<_>>();
            let sides = if sides.len() > 1 {
                // several handlers: label each drop with its except clause
                sides
                    .into_iter()
                    .map(|mut s| {
                        s.label = String::new();
                        s
                    })
                    .collect()
            } else {
                sides
            };
            let b = branch(head, down, sides);
            if finally.is_empty() {
                b
            } else {
                seq(vec![b, titled(Some("finally".into()), finally)])
            }
        }
        Flow::Switch { head, arms } => {
            let h = box_blk(
                None,
                vec![fl(
                    util::truncate(&format!("◇ {}", head.text), MAX_LINE),
                    head.line,
                )],
                BoxKind::Switch,
                0,
            );
            let sides = arms
                .iter()
                .map(|(pat, items)| Side {
                    label: String::new(),
                    blk: titled_arm(pat, items),
                    back_edge: false,
                })
                .collect();
            branch(h, None, sides)
        }
    }
}

/// A block whose first box carries a title (else, finally).
fn titled(title: Option<String>, items: &[Flow]) -> Blk {
    match is_simple(items) {
        Some(lines) if !lines.is_empty() => box_blk(title, lines, BoxKind::Stmt, 0),
        _ => {
            let head = box_blk(
                None,
                vec![fl(title.unwrap_or_default(), 0)],
                BoxKind::Stmt,
                0,
            );
            seq(vec![head, lay(items)])
        }
    }
}

fn titled_handler(h: &FlowLine, items: &[Flow]) -> Blk {
    match is_simple(items) {
        Some(lines) if !lines.is_empty() => {
            box_blk(Some(h.text.clone()), lines, BoxKind::Handler, 0)
        }
        _ => {
            let head = box_blk(None, vec![h.clone()], BoxKind::Handler, 0);
            seq(vec![head, lay(items)])
        }
    }
}

fn titled_arm(pat: &FlowLine, items: &[Flow]) -> Blk {
    match items {
        [Flow::Stmts(lines)] => box_blk(Some(pat.text.clone()), lines.clone(), BoxKind::Stmt, 0),
        [Flow::End { kind, line }] => {
            let t = format!("{}{}", end_prefix(*kind), line.text);
            box_blk(
                Some(pat.text.clone()),
                vec![fl(util::truncate(&t, MAX_LINE), line.line)],
                BoxKind::End(*kind),
                0,
            )
        }
        _ => {
            let head = box_blk(None, vec![pat.clone()], BoxKind::Stmt, 0);
            seq(vec![head, lay(items)])
        }
    }
}

/// A laid-out flowchart: boxes (in reading order) and a canvas-ready plan.
#[derive(Debug, Clone)]
pub struct FlowLayout {
    pub boxes: Vec<FlowBox>,
    ops: Vec<Op>,
    pub width: usize,
    pub height: usize,
}

/// Columns kept free on the left for change marks.
pub const GUTTER: usize = 2;

pub fn layout(chart: &FlowChart) -> FlowLayout {
    let start = box_blk(
        None,
        vec![fl(format!("▶ {}()", chart.name), chart.start_line)],
        BoxKind::Stmt,
        0,
    );
    let mut parts = vec![start];
    if !chart.body.is_empty() {
        parts.push(lay(&chart.body));
    }
    let b = seq(parts).shift(GUTTER, 0);
    let mut boxes = b.boxes;
    // The entry box is drawn round, like a terminal node.
    if let Some(first) = boxes.first_mut() {
        first.kind = BoxKind::End(EndKind::Return);
    }
    boxes.sort_by_key(|x| (x.y, x.x));
    FlowLayout {
        width: b.w + GUTTER + 2,
        height: b.h + 1,
        boxes,
        ops: b.ops,
    }
}

impl FlowLayout {
    pub fn overlaps(&self) -> Vec<(usize, usize)> {
        let mut bad = Vec::new();
        for i in 0..self.boxes.len() {
            for j in i + 1..self.boxes.len() {
                let (a, b) = (&self.boxes[i], &self.boxes[j]);
                if a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h {
                    bad.push((i, j));
                }
            }
        }
        bad
    }

    /// Index of the box holding a source line.
    pub fn box_for_line(&self, line: u32) -> Option<usize> {
        self.boxes
            .iter()
            .position(|b| b.lines.iter().any(|l| l.line == line))
    }

    /// Draw with change marks (line -> 'A'/'M') and a selected box.
    pub fn draw(&self, marks: &BTreeMap<u32, char>, selected: Option<usize>) -> Canvas {
        let mut c = Canvas::new(self.width + 1, self.height + 1);
        for op in &self.ops {
            if let Op::Poly(pts) = op {
                c.polyline(pts, Tone::Rule);
            }
        }
        for op in &self.ops {
            if let Op::Arrow(x, y, ch) = op {
                c.arrow(*x, *y, *ch, Tone::Rule);
            }
        }
        for (i, b) in self.boxes.iter().enumerate() {
            let sel = selected == Some(i);
            let line_marks: Vec<Option<char>> = b
                .lines
                .iter()
                .map(|l| marks.get(&l.line).copied())
                .collect();
            let all_added = !line_marks.is_empty() && line_marks.iter().all(|m| *m == Some('A'));
            let tone = if sel {
                Tone::Text
            } else if all_added {
                Tone::Add
            } else {
                match b.kind {
                    BoxKind::End(EndKind::Raise) => Tone::Warn,
                    _ => Tone::Rule,
                }
            };
            // keep junction bits on the borders
            let mut keep = Vec::new();
            for x in b.x..b.x + b.w {
                for y in [b.y, b.y + b.h - 1] {
                    if let Some(cell) = c.get(x, y) {
                        keep.push((x, y, cell.mask));
                    }
                }
            }
            for y in b.y..b.y + b.h {
                for x in [b.x, b.x + b.w - 1] {
                    if let Some(cell) = c.get(x, y) {
                        keep.push((x, y, cell.mask));
                    }
                }
            }
            c.rect(b.x, b.y, b.w, b.h, tone, matches!(b.kind, BoxKind::End(_)));
            for (x, y, mask) in keep {
                let on_bottom = y == b.y + b.h - 1 && mask & DOWN != 0;
                let on_right = x == b.x + b.w - 1 && mask & RIGHT != 0;
                if on_bottom {
                    c.bits(x, y, DOWN, tone);
                }
                if on_right {
                    c.bits(x, y, RIGHT, tone);
                }
            }
            if let Some(t) = &b.title {
                c.text(
                    b.x + 2,
                    b.y,
                    &format!(" {} ", util::truncate(t, b.w.saturating_sub(6))),
                    if sel { Tone::Text } else { Tone::Dim },
                    true,
                );
            }
            for (li, l) in b.lines.iter().enumerate() {
                let y = b.y + 1 + li;
                let m = line_marks[li];
                let text_tone = match (m, b.kind) {
                    (Some('A'), _) => Tone::Add,
                    (Some(_), _) => Tone::Mod,
                    (None, BoxKind::Decision | BoxKind::Switch | BoxKind::Loop) => Tone::Text,
                    (None, BoxKind::End(_)) => Tone::Text,
                    (None, _) => Tone::Dim,
                };
                c.text_max(
                    b.x + 2,
                    y,
                    &l.text,
                    b.w.saturating_sub(4),
                    if sel { Tone::Text } else { text_tone },
                    sel,
                );
                if let Some(mk) = m {
                    c.text(
                        0,
                        y,
                        &mk.to_string(),
                        if mk == 'A' { Tone::Add } else { Tone::Mod },
                        true,
                    );
                }
                if sel {
                    c.band(b.x + 1, b.x + b.w - 2, y, Band::Active);
                }
            }
        }
        for op in &self.ops {
            if let Op::Label(x, y, s, on_line) = op {
                let w = util::width(s);
                // bus labels may sit on a horizontal line; others need space
                let clear = (0..w).all(|i| {
                    c.get(x + i, *y).is_some_and(|cell| {
                        !cell.txt
                            && cell.arrow.is_none()
                            && (cell.mask == 0
                                || (*on_line && cell.mask & (DOWN | crate::canvas::UP) == 0))
                    })
                });
                if clear {
                    c.text(*x, *y, s, Tone::Muted, false);
                }
            }
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chart(lang: Lang, src: &str, name: &str) -> FlowChart {
        let fs = lang::extract(lang, src);
        let sym = fs.symbols.iter().find(|s| s.name == name).unwrap();
        build(lang, src, sym).unwrap()
    }

    #[test]
    fn python_if_else_chain() {
        let src = "def f(x):\n    if x > 1:\n        return 1\n    elif x < 0:\n        y = 2\n    else:\n        y = 3\n    return y\n";
        let c = chart(Lang::Python, src, "f");
        assert_eq!(c.stats()["if"], 2);
        assert_eq!(c.stats()["return"], 2);
        let l = layout(&c);
        assert!(l.overlaps().is_empty());
        let text = l.draw(&BTreeMap::new(), None).to_text();
        assert!(text.contains("◇ x > 1"), "{text}");
        assert!(text.contains("◉ return y"));
    }
}
