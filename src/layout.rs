//! Layered graph layout after Sugiyama: break cycles, assign layers, insert
//! dummy nodes for long edges, reduce crossings with barycenter sweeps, place
//! nodes horizontally, then route edges orthogonally through the channels
//! between layers on non-overlapping tracks. Coordinates are terminal cells.

use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LNode {
    pub w: usize,
    pub h: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LEdge {
    pub from: usize,
    pub to: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl Rect {
    pub fn right(&self) -> usize {
        self.x + self.w
    }
    pub fn bottom(&self) -> usize {
        self.y + self.h
    }
    pub fn center_x(&self) -> usize {
        self.x + self.w / 2
    }
    pub fn contains(&self, x: usize, y: usize) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }
}

/// An edge drawn as an orthogonal polyline. The first point is the cell just
/// below the source box, the last is the cell just above the target box
/// (where the arrowhead goes). For edges that point up the layers (cycles),
/// `reversed` is set and the arrowhead belongs at the first point instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub edge: LEdge,
    pub points: Vec<(usize, usize)>,
    pub reversed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub nodes: Vec<Rect>,
    pub routes: Vec<Route>,
    pub width: usize,
    pub height: usize,
    /// Layer index per node (isolated nodes get usize::MAX).
    pub layer: Vec<usize>,
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Columns between neighbouring boxes in a layer.
    pub node_gap: usize,
    /// Wrap width for the grid of unconnected boxes.
    pub max_width: usize,
    /// Sweeps of barycenter crossing reduction.
    pub sweeps: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            node_gap: 4,
            max_width: 120,
            sweeps: 12,
        }
    }
}

/// An item in a layer: a real node or a dummy on a long edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Node(usize),
    Dummy(usize),
}

pub fn layout(nodes: &[LNode], edges: &[LEdge], opts: &Options) -> Layout {
    let n = nodes.len();
    let mut out = Layout {
        nodes: vec![Rect::default(); n],
        layer: vec![usize::MAX; n],
        ..Default::default()
    };
    if n == 0 {
        return out;
    }
    // 1. Clean edges: no self loops, no duplicates.
    let mut edge_set: BTreeSet<LEdge> = BTreeSet::new();
    for e in edges {
        if e.from != e.to && e.from < n && e.to < n {
            edge_set.insert(*e);
        }
    }
    let edges: Vec<LEdge> = edge_set.into_iter().collect();
    let mut connected = vec![false; n];
    for e in &edges {
        connected[e.from] = true;
        connected[e.to] = true;
    }

    // 2. Break cycles: reverse DFS back edges.
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in &edges {
        adj[e.from].push(e.to);
    }
    let mut state = vec![0u8; n]; // 0 new, 1 on stack, 2 done
    let mut back: BTreeSet<LEdge> = BTreeSet::new();
    for start in 0..n {
        if state[start] != 0 || !connected[start] {
            continue;
        }
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        state[start] = 1;
        while let Some(&mut (v, ref mut i)) = stack.last_mut() {
            if *i < adj[v].len() {
                let w = adj[v][*i];
                *i += 1;
                match state[w] {
                    0 => {
                        state[w] = 1;
                        stack.push((w, 0));
                    }
                    1 => {
                        back.insert(LEdge { from: v, to: w });
                    }
                    _ => {}
                }
            } else {
                state[v] = 2;
                stack.pop();
            }
        }
    }
    // Directed edges used for layering (u above v).
    let dag: Vec<(usize, usize, LEdge, bool)> = edges
        .iter()
        .map(|e| {
            if back.contains(e) {
                (e.to, e.from, *e, true)
            } else {
                (e.from, e.to, *e, false)
            }
        })
        .collect();

    // 3. Longest-path layering.
    let mut indeg = vec![0usize; n];
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(u, v, _, _) in &dag {
        succ[u].push(v);
        indeg[v] += 1;
    }
    let mut layer = vec![0usize; n];
    let mut queue: Vec<usize> = (0..n).filter(|&v| connected[v] && indeg[v] == 0).collect();
    let mut head = 0;
    while head < queue.len() {
        let u = queue[head];
        head += 1;
        for &v in &succ[u] {
            layer[v] = layer[v].max(layer[u] + 1);
            indeg[v] -= 1;
            if indeg[v] == 0 {
                queue.push(v);
            }
        }
    }
    // Pull sources down next to their first successor to shorten edges.
    for &u in queue.iter().rev() {
        let has_pred = dag.iter().any(|&(_, v, _, _)| v == u);
        if !has_pred && !succ[u].is_empty() {
            let min_succ = succ[u].iter().map(|&v| layer[v]).min().unwrap_or(1);
            layer[u] = min_succ.saturating_sub(1);
        }
    }
    let n_layers = (0..n).filter(|&v| connected[v]).map(|v| layer[v] + 1).max().unwrap_or(0);

    // 4. Dummy chains for long edges.
    let mut layers: Vec<Vec<Item>> = vec![Vec::new(); n_layers];
    // Initial order: BFS discovery order for stability.
    for &v in &queue {
        layers[layer[v]].push(Item::Node(v));
    }
    // chains[i] = items from upper to lower for dag edge i
    let mut dummies = 0usize;
    let mut chains: Vec<Vec<Item>> = Vec::with_capacity(dag.len());
    for &(u, v, _, _) in &dag {
        let mut chain = vec![Item::Node(u)];
        for l in layer[u] + 1..layer[v] {
            let d = Item::Dummy(dummies);
            dummies += 1;
            layers[l].push(d);
            chain.push(d);
        }
        chain.push(Item::Node(v));
        chains.push(chain);
    }
    let mut dummy_layer = vec![0usize; dummies];
    for (l, items) in layers.iter().enumerate() {
        for it in items {
            if let Item::Dummy(d) = it {
                dummy_layer[*d] = l;
            }
        }
    }
    let _ = dummy_layer;

    // Adjacency between consecutive-layer items.
    let key = |it: Item| -> usize {
        match it {
            Item::Node(v) => v,
            Item::Dummy(d) => n + d,
        }
    };
    let total = n + dummies;
    let mut up: Vec<Vec<usize>> = vec![Vec::new(); total];
    let mut down: Vec<Vec<usize>> = vec![Vec::new(); total];
    for chain in &chains {
        for pair in chain.windows(2) {
            let (a, b) = (key(pair[0]), key(pair[1]));
            down[a].push(b);
            up[b].push(a);
        }
    }

    // 5. Crossing reduction by barycenters.
    let mut pos = vec![0f64; total];
    let set_pos = |layers: &Vec<Vec<Item>>, pos: &mut Vec<f64>| {
        for items in layers {
            for (i, it) in items.iter().enumerate() {
                pos[key(*it)] = i as f64;
            }
        }
    };
    set_pos(&layers, &mut pos);
    let count_crossings = |layers: &Vec<Vec<Item>>, pos: &Vec<f64>| -> usize {
        let mut c = 0;
        for l in 0..layers.len().saturating_sub(1) {
            let mut segs: Vec<(f64, f64)> = Vec::new();
            for it in &layers[l] {
                let a = key(*it);
                for &b in &down[a] {
                    segs.push((pos[a], pos[b]));
                }
            }
            for i in 0..segs.len() {
                for j in i + 1..segs.len() {
                    let (a1, b1) = segs[i];
                    let (a2, b2) = segs[j];
                    if (a1 < a2 && b1 > b2) || (a1 > a2 && b1 < b2) {
                        c += 1;
                    }
                }
            }
        }
        c
    };
    let mut best = layers.clone();
    let mut best_c = count_crossings(&layers, &pos);
    for sweep in 0..opts.sweeps {
        if best_c == 0 {
            break;
        }
        let downward = sweep % 2 == 0;
        let order: Vec<usize> = if downward {
            (1..layers.len()).collect()
        } else {
            (0..layers.len().saturating_sub(1)).rev().collect()
        };
        for l in order {
            let mut scored: Vec<(f64, usize, Item)> = layers[l]
                .iter()
                .enumerate()
                .map(|(i, it)| {
                    let k = key(*it);
                    let nb = if downward { &up[k] } else { &down[k] };
                    let bc = if nb.is_empty() {
                        pos[k]
                    } else {
                        nb.iter().map(|&x| pos[x]).sum::<f64>() / nb.len() as f64
                    };
                    (bc, i, *it)
                })
                .collect();
            scored.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap().then(a.1.cmp(&b.1)));
            layers[l] = scored.into_iter().map(|s| s.2).collect();
            for (i, it) in layers[l].iter().enumerate() {
                pos[key(*it)] = i as f64;
            }
        }
        let c = count_crossings(&layers, &pos);
        if c < best_c {
            best_c = c;
            best = layers.clone();
        }
    }
    let layers = best;
    set_pos(&layers, &mut pos);

    // 6. Horizontal placement: pack, then pull towards neighbour centres.
    let width_of = |it: Item| -> usize {
        match it {
            Item::Node(v) => nodes[v].w.max(3),
            Item::Dummy(_) => 1,
        }
    };
    let gap = opts.node_gap.max(1);
    let mut x = vec![0f64; total];
    for items in &layers {
        let mut cx = 0f64;
        for it in items {
            x[key(*it)] = cx;
            cx += (width_of(*it) + gap) as f64;
        }
    }
    let center = |x: &Vec<f64>, it: Item| -> f64 { x[key(it)] + width_of(it) as f64 / 2.0 };
    for iter in 0..8 {
        let order: Vec<usize> = if iter % 2 == 0 {
            (0..layers.len()).collect()
        } else {
            (0..layers.len()).rev().collect()
        };
        for l in order {
            let items = &layers[l];
            if items.is_empty() {
                continue;
            }
            let desired: Vec<f64> = items
                .iter()
                .map(|it| {
                    let k = key(*it);
                    let mut nb: Vec<usize> = Vec::new();
                    nb.extend(&up[k]);
                    nb.extend(&down[k]);
                    if nb.is_empty() {
                        x[k]
                    } else {
                        let c: f64 = nb
                            .iter()
                            .map(|&m| {
                                let it2 = if m < n { Item::Node(m) } else { Item::Dummy(m - n) };
                                center(&x, it2)
                            })
                            .sum::<f64>()
                            / nb.len() as f64;
                        c - width_of(*it) as f64 / 2.0
                    }
                })
                .collect();
            let widths: Vec<f64> = items.iter().map(|it| width_of(*it) as f64).collect();
            let m = items.len();
            let mut fwd = vec![0f64; m];
            for i in 0..m {
                fwd[i] = if i == 0 {
                    desired[0]
                } else {
                    desired[i].max(fwd[i - 1] + widths[i - 1] + gap as f64)
                };
            }
            let mut bwd = vec![0f64; m];
            for i in (0..m).rev() {
                bwd[i] = if i == m - 1 {
                    desired[i]
                } else {
                    desired[i].min(bwd[i + 1] - widths[i] - gap as f64)
                };
            }
            for i in 0..m {
                x[key(items[i])] = ((fwd[i] + bwd[i]) / 2.0).round();
            }
            // Averaging keeps order but can still violate gaps after rounding.
            for i in 1..m {
                let min_x = x[key(items[i - 1])] + widths[i - 1] + gap as f64;
                if x[key(items[i])] < min_x {
                    x[key(items[i])] = min_x;
                }
            }
        }
    }
    let min_x = (0..total).filter(|&k| k >= n || connected[k]).map(|k| x[k]).fold(f64::INFINITY, f64::min);
    let shift = if min_x.is_finite() { -min_x } else { 0.0 };
    let xi: Vec<usize> = x.iter().map(|v| (v + shift).max(0.0) as usize).collect();

    // 7. Ports and channel tracks.
    // Segments per channel l (between layer l and l+1): (edge idx, from item, to item)
    let mut seg_by_channel: Vec<Vec<(usize, Item, Item)>> = vec![Vec::new(); layers.len()];
    for (ei, chain) in chains.iter().enumerate() {
        for pair in chain.windows(2) {
            let l = match pair[0] {
                Item::Node(v) => layer[v],
                Item::Dummy(d) => layers.iter().position(|ls| ls.contains(&Item::Dummy(d))).unwrap_or(0),
            };
            seg_by_channel[l].push((ei, pair[0], pair[1]));
        }
    }
    let item_x = |it: Item| -> usize { xi[key(it)] };
    // Port positions: bottom ports for sources, top ports for targets.
    let mut bottom_port: HashMap<(usize, usize), usize> = HashMap::new(); // (edge, node) -> x
    let mut top_port: HashMap<(usize, usize), usize> = HashMap::new();
    for v in 0..n {
        if !connected[v] {
            continue;
        }
        let w = nodes[v].w.max(3);
        let place = |mut list: Vec<(usize, usize)>| -> Vec<(usize, usize)> {
            // list of (edge, other x) -> (edge, port x)
            list.sort_by_key(|&(e, ox)| (ox, e));
            let k = list.len();
            let interior = w.saturating_sub(2).max(1);
            list.iter()
                .enumerate()
                .map(|(i, &(e, _))| {
                    let off = if k >= interior {
                        (i * interior) / k.max(1)
                    } else {
                        ((i + 1) * interior) / (k + 1)
                    };
                    (e, xi[v] + 1 + off.min(interior - 1))
                })
                .collect()
        };
        let mut outs = Vec::new();
        let mut ins = Vec::new();
        for segs in &seg_by_channel {
            for &(ei, a, b) in segs {
                if a == Item::Node(v) {
                    outs.push((ei, item_x(b) + if let Item::Node(t) = b { nodes[t].w / 2 } else { 0 }));
                }
                if b == Item::Node(v) {
                    ins.push((ei, item_x(a) + if let Item::Node(s) = a { nodes[s].w / 2 } else { 0 }));
                }
            }
        }
        for (e, px) in place(outs) {
            bottom_port.insert((e, v), px);
        }
        for (e, px) in place(ins) {
            top_port.insert((e, v), px);
        }
    }
    let seg_x = |ei: usize, a: Item, b: Item| -> (usize, usize) {
        let x1 = match a {
            Item::Node(v) => bottom_port[&(ei, v)],
            Item::Dummy(_) => item_x(a),
        };
        let x2 = match b {
            Item::Node(v) => top_port[&(ei, v)],
            Item::Dummy(_) => item_x(b),
        };
        (x1, x2)
    };
    // Track assignment per channel.
    let mut track_of: HashMap<(usize, usize), usize> = HashMap::new(); // (channel, seg idx) -> track
    let mut tracks_in: Vec<usize> = vec![0; layers.len()];
    for (l, segs) in seg_by_channel.iter().enumerate() {
        let mut spans: Vec<(usize, usize, usize)> = segs
            .iter()
            .enumerate()
            .filter_map(|(si, &(ei, a, b))| {
                let (x1, x2) = seg_x(ei, a, b);
                (x1 != x2).then_some((x1.min(x2), x1.max(x2), si))
            })
            .collect();
        spans.sort();
        let mut track_end: Vec<usize> = Vec::new();
        for (lo, hi, si) in spans {
            let t = match track_end.iter().position(|&end| end + 1 < lo) {
                Some(t) => t,
                None => {
                    track_end.push(0);
                    track_end.len() - 1
                }
            };
            track_end[t] = hi;
            track_of.insert((l, si), t);
        }
        tracks_in[l] = track_end.len();
    }

    // 8. Vertical placement.
    let layer_h: Vec<usize> = layers
        .iter()
        .map(|items| {
            items
                .iter()
                .filter_map(|it| match it {
                    Item::Node(v) => Some(nodes[*v].h),
                    Item::Dummy(_) => None,
                })
                .max()
                .unwrap_or(1)
        })
        .collect();
    let mut layer_y = vec![0usize; layers.len()];
    let mut y = 0usize;
    for l in 0..layers.len() {
        layer_y[l] = y;
        y += layer_h[l];
        if l + 1 < layers.len() {
            y += tracks_in[l] + 2;
        }
    }
    let layered_height = y;

    for (l, items) in layers.iter().enumerate() {
        for it in items {
            if let Item::Node(v) = it {
                out.nodes[*v] = Rect {
                    x: xi[*v],
                    y: layer_y[l],
                    w: nodes[*v].w.max(3),
                    h: nodes[*v].h,
                };
                out.layer[*v] = l;
            }
        }
    }

    // 9. Routes.
    for (ei, chain) in chains.iter().enumerate() {
        let (_, _, orig, reversed) = dag[ei];
        let mut points: Vec<(usize, usize)> = Vec::new();
        for pair in chain.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let l = match a {
                Item::Node(v) => layer[v],
                Item::Dummy(_) => layers.iter().position(|ls| ls.contains(&a)).unwrap_or(0),
            };
            let si = seg_by_channel[l]
                .iter()
                .position(|&(e2, a2, b2)| e2 == ei && a2 == a && b2 == b)
                .unwrap_or(0);
            let (x1, x2) = seg_x(ei, a, b);
            let y_start = match a {
                Item::Node(v) => layer_y[l] + nodes[v].h,
                Item::Dummy(_) => layer_y[l],
            };
            let channel_top = layer_y[l] + layer_h[l];
            let y_end = layer_y[l + 1] - 1;
            let y_end = match b {
                Item::Node(_) => y_end,
                // dummies pass straight through their layer band
                Item::Dummy(_) => layer_y[l + 1] + layer_h[l + 1] - 1,
            };
            if points.is_empty() || points.last() != Some(&(x1, y_start)) {
                points.push((x1, y_start));
            }
            if x1 != x2 {
                let t = track_of.get(&(l, si)).copied().unwrap_or(0);
                let ty = channel_top + 1 + t;
                points.push((x1, ty));
                points.push((x2, ty));
            }
            points.push((x2, y_end));
        }
        // Remove collinear duplicates.
        let mut clean: Vec<(usize, usize)> = Vec::new();
        for p in points {
            if clean.last() == Some(&p) {
                continue;
            }
            if clean.len() >= 2 {
                let a = clean[clean.len() - 2];
                let b = clean[clean.len() - 1];
                if (a.0 == b.0 && b.0 == p.0) || (a.1 == b.1 && b.1 == p.1) {
                    clean.pop();
                }
            }
            clean.push(p);
        }
        out.routes.push(Route {
            edge: orig,
            points: clean,
            reversed,
        });
    }

    // 10. Unconnected nodes in a grid below.
    let mut max_right = (0..n).filter(|&v| connected[v]).map(|v| out.nodes[v].right()).max().unwrap_or(0);
    for r in &out.routes {
        for p in &r.points {
            max_right = max_right.max(p.0 + 1);
        }
    }
    let wrap = opts.max_width.max(max_right).max(20);
    let mut gx = 0usize;
    let mut gy = if layered_height > 0 { layered_height + 2 } else { 0 };
    let mut row_h = 0usize;
    for v in 0..n {
        if connected[v] {
            continue;
        }
        let w = nodes[v].w.max(3);
        if gx > 0 && gx + w > wrap {
            gx = 0;
            gy += row_h + 1;
            row_h = 0;
        }
        out.nodes[v] = Rect {
            x: gx,
            y: gy,
            w,
            h: nodes[v].h,
        };
        gx += w + gap;
        row_h = row_h.max(nodes[v].h);
    }
    out.width = out.nodes.iter().map(Rect::right).max().unwrap_or(0).max(max_right);
    out.height = out.nodes.iter().map(Rect::bottom).max().unwrap_or(0);
    for r in &out.routes {
        for p in &r.points {
            out.height = out.height.max(p.1 + 1);
        }
    }
    out
}

impl Layout {
    /// Pairs of boxes that overlap (should always be empty).
    pub fn overlaps(&self) -> Vec<(usize, usize)> {
        let mut bad = Vec::new();
        for i in 0..self.nodes.len() {
            for j in i + 1..self.nodes.len() {
                if self.nodes[i].intersects(&self.nodes[j]) {
                    bad.push((i, j));
                }
            }
        }
        bad
    }

    /// Route cells that pass through a box (should always be empty).
    pub fn route_collisions(&self) -> Vec<(usize, (usize, usize))> {
        let mut bad = Vec::new();
        for (ri, r) in self.routes.iter().enumerate() {
            for pair in r.points.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let cells: Vec<(usize, usize)> = if a.1 == b.1 {
                    (a.0.min(b.0)..=a.0.max(b.0)).map(|x| (x, a.1)).collect()
                } else {
                    (a.1.min(b.1)..=a.1.max(b.1)).map(|y| (a.0, y)).collect()
                };
                for c in cells {
                    if self.nodes.iter().any(|n| n.contains(c.0, c.1)) {
                        bad.push((ri, c));
                    }
                }
            }
        }
        bad
    }

    /// Number of layers used by connected nodes.
    pub fn layer_count(&self) -> usize {
        self.layer
            .iter()
            .filter(|&&l| l != usize::MAX)
            .map(|l| l + 1)
            .max()
            .unwrap_or(0)
    }

    /// Nodes grouped by their row (for keyboard navigation).
    pub fn rows(&self) -> BTreeMap<usize, Vec<usize>> {
        let mut rows: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (i, r) in self.nodes.iter().enumerate() {
            rows.entry(r.y).or_default().push(i);
        }
        for v in rows.values_mut() {
            v.sort_by_key(|&i| self.nodes[i].x);
        }
        rows
    }
}

/// The nearest node from `from` in a direction (dx, dy), for arrow keys.
pub fn neighbor(layout: &Layout, from: usize, dx: i32, dy: i32) -> Option<usize> {
    let a = layout.nodes.get(from)?;
    let (ax, ay) = (a.x as i64 + a.w as i64 / 2, a.y as i64 + a.h as i64 / 2);
    let mut best: Option<(i64, usize)> = None;
    for (i, b) in layout.nodes.iter().enumerate() {
        if i == from {
            continue;
        }
        let (bx, by) = (b.x as i64 + b.w as i64 / 2, b.y as i64 + b.h as i64 / 2);
        let (ddx, ddy) = (bx - ax, by - ay);
        let primary = ddx * dx as i64 + ddy * dy as i64;
        if dy != 0 {
            // must be in another row in that direction
            let beyond = if dy > 0 { b.y as i64 >= a.bottom() as i64 } else { (b.bottom() as i64) <= a.y as i64 };
            if !beyond {
                continue;
            }
        } else {
            let overlap_rows = (b.y as i64) < a.bottom() as i64 && (a.y as i64) < b.bottom() as i64;
            let beyond = if dx > 0 { b.x as i64 >= a.right() as i64 } else { (b.right() as i64) <= a.x as i64 };
            if !beyond || !overlap_rows {
                continue;
            }
        }
        if primary <= 0 {
            continue;
        }
        let secondary = (ddx * dy as i64).abs() + (ddy * dx as i64).abs();
        let score = primary + secondary * 3;
        if best.is_none_or(|(s, _)| score < s) {
            best = Some((score, i));
        }
    }
    best.map(|(_, i)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nodes(sizes: &[(usize, usize)]) -> Vec<LNode> {
        sizes.iter().map(|&(w, h)| LNode { w, h }).collect()
    }

    #[test]
    fn chain_is_vertical() {
        let ns = nodes(&[(10, 3), (10, 3), (10, 3)]);
        let es = vec![LEdge { from: 0, to: 1 }, LEdge { from: 1, to: 2 }];
        let l = layout(&ns, &es, &Options::default());
        assert_eq!(l.layer, vec![0, 1, 2]);
        assert!(l.nodes[0].y < l.nodes[1].y && l.nodes[1].y < l.nodes[2].y);
        assert!(l.overlaps().is_empty());
        assert!(l.route_collisions().is_empty());
        // straight edges
        assert_eq!(l.routes[0].points.len(), 2);
    }

    #[test]
    fn cycles_are_broken() {
        let ns = nodes(&[(8, 3), (8, 3), (8, 3)]);
        let es = vec![
            LEdge { from: 0, to: 1 },
            LEdge { from: 1, to: 2 },
            LEdge { from: 2, to: 0 },
        ];
        let l = layout(&ns, &es, &Options::default());
        assert_eq!(l.routes.iter().filter(|r| r.reversed).count(), 1);
        assert!(l.overlaps().is_empty());
        assert!(l.route_collisions().is_empty());
    }

    #[test]
    fn long_edges_get_dummies_and_avoid_boxes() {
        // 0 -> 1 -> 2 -> 3 and a long edge 0 -> 3 that must go around 1 and 2
        let ns = nodes(&[(20, 4), (30, 5), (30, 5), (20, 3), (12, 3)]);
        let es = vec![
            LEdge { from: 0, to: 1 },
            LEdge { from: 1, to: 2 },
            LEdge { from: 2, to: 3 },
            LEdge { from: 0, to: 3 },
            LEdge { from: 0, to: 4 },
        ];
        let l = layout(&ns, &es, &Options::default());
        assert!(l.overlaps().is_empty(), "{:?}", l.overlaps());
        assert!(l.route_collisions().is_empty(), "{:?}", l.route_collisions());
    }

    #[test]
    fn isolated_nodes_wrap_in_a_grid() {
        let ns = nodes(&[(30, 3); 6]);
        let l = layout(&ns, &[], &Options { max_width: 70, ..Default::default() });
        assert!(l.overlaps().is_empty());
        assert!(l.width <= 70);
        assert!(l.height >= 9);
    }

    #[test]
    fn arrow_neighbors() {
        let ns = nodes(&[(10, 3), (10, 3), (10, 3)]);
        let es = vec![LEdge { from: 0, to: 1 }, LEdge { from: 0, to: 2 }];
        let l = layout(&ns, &es, &Options::default());
        let below = neighbor(&l, 0, 0, 1).unwrap();
        assert!(below == 1 || below == 2);
        let right = neighbor(&l, below, 1, 0).or_else(|| neighbor(&l, below, -1, 0));
        assert!(right.is_some());
        assert_eq!(neighbor(&l, 0, 0, -1), None);
    }
}
