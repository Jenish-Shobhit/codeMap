//! Layered layout: boxes never overlap and edges never cross a box, on
//! hand-made and pseudo-random graphs.

use codemorph::canvas::{Canvas, Tone};
use codemorph::layout::{layout, LEdge, LNode, Layout, Options};

/// Tiny deterministic PRNG (xorshift) so the test needs no dependencies.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + (self.next() as usize) % (hi - lo)
    }
}

pub fn draw(nodes: &[LNode], l: &Layout, names: &[String]) -> String {
    let mut c = Canvas::new(l.width + 2, l.height + 2);
    for r in &l.routes {
        c.polyline(&r.points, Tone::Rule);
        if r.reversed {
            let (x, y) = r.points[0];
            c.arrow(x, y, '▲', Tone::Rule);
        } else {
            let (x, y) = *r.points.last().unwrap();
            c.arrow(x, y, '▼', Tone::Rule);
        }
    }
    for (i, n) in l.nodes.iter().enumerate() {
        c.rect(n.x, n.y, n.w, n.h, Tone::Rule, false);
        c.text(n.x + 2, n.y + 1, &names[i], Tone::Text, false);
        let _ = nodes;
    }
    // Ports: an edge leaving a bottom border joins it with ┬.
    for r in &l.routes {
        let (x, y) = r.points[0];
        if y > 0 {
            c.bits(x, y - 1, codemorph::canvas::DOWN, Tone::Rule);
        }
        let (x, y) = r.points[r.points.len() - 1];
        let _ = (x, y);
    }
    c.to_text()
}

#[test]
fn random_graphs_have_no_overlaps_or_collisions() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for case in 0..200 {
        let n = rng.range(1, 18);
        let nodes: Vec<LNode> = (0..n)
            .map(|_| LNode {
                w: rng.range(6, 36),
                h: rng.range(3, 9),
            })
            .collect();
        let m = rng.range(0, n * 2 + 1);
        let edges: Vec<LEdge> = (0..m)
            .map(|_| LEdge {
                from: rng.range(0, n),
                to: rng.range(0, n),
            })
            .collect();
        let l = layout(&nodes, &edges, &Options::default());
        assert!(
            l.overlaps().is_empty(),
            "case {case}: overlapping boxes {:?}",
            l.overlaps()
        );
        assert!(
            l.route_collisions().is_empty(),
            "case {case}: edge through a box {:?}\n{:?}",
            l.route_collisions(),
            edges
        );
        for (i, r) in l.nodes.iter().enumerate() {
            assert_eq!(
                (r.w, r.h),
                (nodes[i].w.max(3), nodes[i].h),
                "case {case}: size kept"
            );
        }
        // Every non-self edge got a route.
        let distinct: std::collections::BTreeSet<(usize, usize)> = edges
            .iter()
            .filter(|e| e.from != e.to)
            .map(|e| (e.from, e.to))
            .collect();
        assert_eq!(l.routes.len(), distinct.len(), "case {case}");
    }
}

#[test]
fn orthogonal_routes_start_below_and_end_above() {
    let nodes = vec![
        LNode { w: 20, h: 5 },
        LNode { w: 14, h: 3 },
        LNode { w: 14, h: 3 },
        LNode { w: 30, h: 4 },
    ];
    let edges = vec![
        LEdge { from: 0, to: 1 },
        LEdge { from: 0, to: 2 },
        LEdge { from: 1, to: 3 },
        LEdge { from: 2, to: 3 },
    ];
    let l = layout(&nodes, &edges, &Options::default());
    for r in &l.routes {
        let src = l.nodes[r.edge.from];
        let dst = l.nodes[r.edge.to];
        let first = r.points[0];
        let last = *r.points.last().unwrap();
        assert_eq!(first.1, src.bottom(), "starts right below the source");
        assert!(
            first.0 > src.x && first.0 < src.right() - 1,
            "leaves through the bottom border"
        );
        assert_eq!(last.1 + 1, dst.y, "ends right above the target");
        assert!(last.0 > dst.x && last.0 < dst.right() - 1);
        for pair in r.points.windows(2) {
            assert!(
                pair[0].0 == pair[1].0 || pair[0].1 == pair[1].1,
                "orthogonal"
            );
        }
    }
    let names: Vec<String> = ["service.py", "api.py", "model.py", "topology.py"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let text = draw(&nodes, &l, &names);
    assert!(text.contains("▼"));
    assert!(text.contains("┬"));
    // Diamond without crossings.
    assert!(!text.contains('┼'), "\n{text}");
}

#[test]
#[ignore]
fn print_sample() {
    let nodes = vec![
        LNode { w: 24, h: 3 },
        LNode { w: 34, h: 5 },
        LNode { w: 30, h: 5 },
        LNode { w: 90, h: 5 },
        LNode { w: 30, h: 7 },
        LNode { w: 26, h: 4 },
        LNode { w: 38, h: 7 },
    ];
    let edges = vec![
        LEdge { from: 0, to: 3 },
        LEdge { from: 1, to: 3 },
        LEdge { from: 2, to: 3 },
        LEdge { from: 3, to: 4 },
        LEdge { from: 3, to: 5 },
        LEdge { from: 3, to: 6 },
        LEdge { from: 2, to: 6 },
        LEdge { from: 4, to: 5 },
        LEdge { from: 1, to: 6 },
    ];
    let l = layout(&nodes, &edges, &Options::default());
    let names: Vec<String> = [
        "extract.py",
        "open_selector.py",
        "selector.py",
        "service.py",
        "topology.py",
        "model.py",
        "api.py",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    println!("{}", draw(&nodes, &l, &names));
}
