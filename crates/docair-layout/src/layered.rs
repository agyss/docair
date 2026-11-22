//! One box laid out in layers (after Sugiyama): the parts of a box placed so that its lines point one
//! way, cross little and run straight.
//!
//! 1. **Cycles** are broken: the parts get an order in which most of the weight of the lines points
//!    ahead (Eades, Lin and Smyth's greedy order), a line's weight being 1 plus how much it should
//!    point ahead. Lines against that order are laid out turned around.
//! 2. **Layers:** the longest path first, then each part moved, within what its lines allow, to the
//!    layer that keeps its lines shortest.
//! 3. A line across several layers gets a **bend point** in each layer between.
//! 4. **Fewer crossings:** sweeps down and up, each layer ordered by where its lines go
//!    (barycentres), starting from the order the parts were given in; the order with the fewest
//!    crossings is kept.
//! 5. **Positions** across the layers: each part as near the parts its lines go to as the space
//!    between parts allows, found layer by layer as a weighted isotonic regression.
//!
//! Parts with no lines between them are laid out apart and packed in rows.

/// Which way the layers run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum Direction {
    /// Layers from top to bottom: the lines point down.
    Down,
    /// Layers from left to right: the lines point right.
    Right,
}

/// The room a layout keeps, in px (ELK's spacing, times 1.6, as viewer.js gave it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Spacing {
    /// Between two layers.
    pub between_layers: f64,
    /// Between a line and a part, along the layers.
    pub line_part_between_layers: f64,
    /// Between two parts in a layer.
    pub part_part: f64,
    /// Between a line and a part in a layer.
    pub line_part: f64,
    /// Between two lines in a layer.
    pub line_line: f64,
    /// Between two groups of parts with no lines between them.
    pub apart: f64,
}

impl Default for Spacing {
    fn default() -> Self {
        Spacing {
            between_layers: 48.0,
            line_part_between_layers: 19.0,
            part_part: 22.0,
            line_part: 16.0,
            line_line: 10.0,
            apart: 48.0,
        }
    }
}

/// A line inside a box, between two of its parts.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BoxLine {
    pub a: usize,
    pub b: usize,
    /// How much the line should point ahead (higher: more).
    pub ahead: u32,
    /// The size of its words, kept beside its arrow.
    pub words: Option<(f64, f64)>,
}

/// One box's parts and lines.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BoxGraph {
    /// Width and height of each part.
    pub sizes: Vec<(f64, f64)>,
    pub lines: Vec<BoxLine>,
    pub direction: Direction,
    /// Where lines leave the box: a part pulled toward a place across the layers, from 0 (the left
    /// or top end) to 1, with a weight (viewer.js gave ELK ports for this).
    pub pulls: Vec<(usize, f64, f64)>,
    /// Lines that leave the box before its first layer or after its last: the part, whether after,
    /// and where along that side (0 to 1). Each becomes a node of its own in a layer before or after
    /// the box's layers, so the box's layers keep room for the line and its crossings count.
    pub exits: Vec<(usize, bool, f64)>,
}

/// The top left corner of each part, the content starting at (0, 0).
pub(crate) fn lay_out(g: &BoxGraph, sp: &Spacing) -> Vec<(f64, f64)> {
    let n = g.sizes.len();
    if n == 0 {
        return Vec::new();
    }
    // Parts joined by lines lie together; each group is laid out on its own, then the groups are packed.
    let mut uf: Vec<usize> = (0..n).collect();
    fn root(uf: &mut [usize], x: usize) -> usize {
        let mut r = x;
        while uf[r] != r {
            r = uf[r];
        }
        let mut c = x;
        while uf[c] != r {
            let next = uf[c];
            uf[c] = r;
            c = next;
        }
        r
    }
    for l in &g.lines {
        if l.a < n && l.b < n && l.a != l.b {
            let (ra, rb) = (root(&mut uf, l.a), root(&mut uf, l.b));
            if ra != rb {
                uf[ra.max(rb)] = ra.min(rb);
            }
        }
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut group_of: Vec<usize> = vec![usize::MAX; n];
    for v in 0..n {
        let r = root(&mut uf, v);
        if group_of[r] == usize::MAX {
            group_of[r] = groups.len();
            groups.push(Vec::new());
        }
        let k = group_of[r];
        group_of[v] = k;
        groups[k].push(v);
    }
    let mut placed: Vec<LaidGroup> = Vec::new();
    for members in &groups {
        let (pts, w, h) = lay_out_group(g, members, sp);
        placed.push((pts, w, h));
    }
    // Pack the groups in rows, in the order they were given, about 1.6 times as wide as high.
    let area: f64 = placed
        .iter()
        .map(|(_, w, h)| (w + sp.apart) * (h + sp.apart))
        .sum();
    let widest = placed.iter().map(|(_, w, _)| *w).fold(0.0, f64::max);
    let target = widest.max((area * 1.6).sqrt());
    let mut out = vec![(0.0, 0.0); n];
    let (mut x, mut y, mut row_h) = (0.0f64, 0.0f64, 0.0f64);
    for (pts, w, h) in placed {
        if x > 0.0 && x + w > target {
            x = 0.0;
            y += row_h + sp.apart;
            row_h = 0.0;
        }
        for (v, px, py) in pts {
            out[v] = (x + px, y + py);
        }
        x += w + sp.apart;
        row_h = row_h.max(h);
    }
    out
}

/// One group laid out: each part with its top left corner, and the group's width and height.
type LaidGroup = (Vec<(usize, f64, f64)>, f64, f64);

/// A line in one group: its ends, its weight and its words.
type WeightedLine = (usize, usize, f64, Option<(f64, f64)>);

/// A node of the layered layout: a part, or a bend point of a line across several layers.
#[derive(Debug, Clone)]
struct Node {
    /// The part, or none for a bend point.
    part: Option<usize>,
    /// Where a line leaves the box: fixed at this place along the side, from 0 to 1.
    fixed: Option<f64>,
    /// Size along the layers and across them.
    along: f64,
    across: f64,
    layer: usize,
}

/// A piece of a line between two neighbouring layers, from a node in the upper one to one in the lower.
#[derive(Debug, Clone)]
struct LinePiece {
    up: usize,
    down: usize,
    weight: f64,
    /// The words of the line, on its last piece.
    words: Option<(f64, f64)>,
}

/// One group of parts joined by lines: positions relative to the group's top left, and its size.
fn lay_out_group(
    g: &BoxGraph,
    members: &[usize],
    sp: &Spacing,
) -> (Vec<(usize, f64, f64)>, f64, f64) {
    let down = g.direction == Direction::Down;
    let local: std::collections::HashMap<usize, usize> =
        members.iter().enumerate().map(|(i, &v)| (v, i)).collect();
    let m = members.len();
    // The lines between two parts, once each way that is left after cycles are broken.
    let mut lines: Vec<WeightedLine> = Vec::new();
    for l in &g.lines {
        let (Some(&a), Some(&b)) = (local.get(&l.a), local.get(&l.b)) else {
            continue;
        };
        if a == b {
            continue;
        }
        lines.push((a, b, 1.0 + l.ahead as f64, l.words));
    }
    // 1. An order in which most of the lines' weight points ahead.
    let rank = greedy_order(m, &lines);
    let forward: Vec<WeightedLine> = lines
        .iter()
        .map(|&(a, b, w, words)| {
            if rank[a] < rank[b] {
                (a, b, w, words)
            } else {
                (b, a, w, words)
            }
        })
        .collect();
    // 2. Layers.
    let layer = layers(m, &rank, &forward);
    let exits: Vec<(usize, bool, f64)> = g
        .exits
        .iter()
        .filter_map(|&(c, out, at)| local.get(&c).map(|&i| (i, out, at.clamp(0.0, 1.0))))
        .collect();
    // A layer before the first for lines that leave the box that way.
    let before = exits.iter().any(|e| !e.1);
    let layer: Vec<usize> = layer.iter().map(|&l| l + usize::from(before)).collect();
    let mut owner: Vec<usize> = (0..m).collect();
    // 3. Nodes: the parts, and a bend point in each layer a line passes.
    let mut nodes: Vec<Node> = (0..m)
        .map(|i| {
            let (w, h) = g.sizes[members[i]];
            Node {
                part: Some(i),
                fixed: None,
                along: if down { h } else { w },
                across: if down { w } else { h },
                layer: layer[i],
            }
        })
        .collect();
    let mut pieces: Vec<LinePiece> = Vec::new();
    for &(a, b, w, words) in &forward {
        let (la, lb) = (layer[a], layer[b]);
        let mut prev = a;
        for l in la + 1..lb {
            nodes.push(Node {
                part: None,
                fixed: None,
                along: 0.0,
                across: sp.line_line,
                layer: l,
            });
            let d = nodes.len() - 1;
            owner.push(a);
            pieces.push(LinePiece {
                up: prev,
                down: d,
                weight: w * 2.0,
                words: None,
            });
            prev = d;
        }
        pieces.push(LinePiece {
            up: prev,
            down: b,
            weight: if prev == a { w } else { w * 2.0 },
            words,
        });
    }
    // Lines that leave the box: a node of their own before the first layer or after the last, and a
    // bend point in every layer between, so the box keeps a way open for them.
    let after = exits.iter().any(|e| e.1);
    let real_depth = layer.iter().copied().max().unwrap_or(0) + 1;
    let after_layer = real_depth;
    for &(i, out, at) in &exits {
        let port_layer = if out { after_layer } else { 0 };
        nodes.push(Node {
            part: None,
            fixed: Some(at),
            along: 0.0,
            across: 0.0,
            layer: port_layer,
        });
        let port = nodes.len() - 1;
        owner.push(i);
        let (from_l, to_l, first) = if out {
            (layer[i], port_layer, i)
        } else {
            (0, layer[i], port)
        };
        let mut prev = first;
        for l in from_l + 1..to_l {
            nodes.push(Node {
                part: None,
                fixed: None,
                along: 0.0,
                across: sp.line_line,
                layer: l,
            });
            let d = nodes.len() - 1;
            owner.push(i);
            pieces.push(LinePiece {
                up: prev,
                down: d,
                weight: 2.0,
                words: None,
            });
            prev = d;
        }
        pieces.push(LinePiece {
            up: prev,
            down: if out { port } else { i },
            weight: 1.0,
            words: None,
        });
    }
    let depth = real_depth + usize::from(after);
    let fixed_layer: Vec<bool> = (0..depth)
        .map(|l| (before && l == 0) || (after && l == after_layer))
        .collect();
    // 4. The order in each layer: the parts as given (a bend point after the part its line comes from), then sweeps.
    let mut order: Vec<Vec<usize>> = vec![Vec::new(); depth];
    let key: Vec<f64> = nodes
        .iter()
        .enumerate()
        .map(|(v, nd)| match (nd.part, nd.fixed) {
            (Some(i), _) => i as f64,
            (None, Some(at)) => at,
            (None, None) => owner[v] as f64 + 0.5,
        })
        .collect();
    for (v, nd) in nodes.iter().enumerate() {
        order[nd.layer].push(v);
    }
    for row in &mut order {
        row.sort_by(|&x, &y| key[x].total_cmp(&key[y]).then(x.cmp(&y)));
    }
    let ups: Vec<Vec<(usize, f64)>> = neighbours(&nodes, &pieces, true);
    let downs: Vec<Vec<(usize, f64)>> = neighbours(&nodes, &pieces, false);
    let mut pull: Vec<Vec<(f64, f64)>> = vec![Vec::new(); nodes.len()];
    for &(child, at, w) in &g.pulls {
        if let Some(&i) = local.get(&child) {
            pull[i].push((at.clamp(0.0, 1.0), w));
        }
    }
    order = fewer_crossings(order, &ups, &downs, &pull, &nodes, &fixed_layer);
    // 5. Positions across the layers.
    let across = positions(&order, &nodes, &ups, &downs, &pull, sp, &fixed_layer);
    // Along the layers: each layer as thick as its thickest part, the room between layers for the words.
    let mut thick = vec![0.0f64; depth];
    for nd in &nodes {
        thick[nd.layer] = thick[nd.layer].max(nd.along);
    }
    let mut gap = vec![sp.between_layers; depth];
    if before {
        gap[0] = 0.0;
    }
    for p in &pieces {
        if let Some((ww, wh)) = p.words {
            let need = if down { wh } else { ww } + 2.0 * sp.line_part_between_layers;
            let l = nodes[p.up].layer;
            gap[l] = gap[l].max(need);
        }
    }
    let mut start = vec![0.0f64; depth];
    for l in 1..depth {
        start[l] = start[l - 1] + thick[l - 1] + gap[l - 1];
    }
    let mut out = Vec::with_capacity(m);
    let (mut max_a, mut max_b) = (0.0f64, 0.0f64);
    let inside = |v: &&usize| nodes[**v].fixed.is_none();
    let min_across = order
        .iter()
        .flatten()
        .filter(inside)
        .map(|&v| across[v] - nodes[v].across / 2.0)
        .fold(f64::INFINITY, f64::min);
    let min_along = nodes
        .iter()
        .filter(|nd| nd.part.is_some())
        .map(|nd| start[nd.layer] + (thick[nd.layer] - nd.along) / 2.0)
        .fold(f64::INFINITY, f64::min);
    for v in order.iter().flatten().filter(inside) {
        if nodes[*v].part.is_none() {
            // A bend point keeps its room inside the box.
            max_a = max_a.max(across[*v] + nodes[*v].across / 2.0 - min_across);
        }
    }
    for (v, nd) in nodes.iter().enumerate() {
        let Some(i) = nd.part else { continue };
        let a0 = across[v] - nd.across / 2.0 - min_across;
        let b0 = start[nd.layer] + (thick[nd.layer] - nd.along) / 2.0 - min_along;
        max_a = max_a.max(a0 + nd.across);
        max_b = max_b.max(b0 + nd.along);
        let (x, y) = if down { (a0, b0) } else { (b0, a0) };
        out.push((members[i], x, y));
    }
    let (w, h) = if down { (max_a, max_b) } else { (max_b, max_a) };
    (out, w, h)
}

/// For each node, its neighbours in the layer above (`up`) or below, with the weight of the piece.
fn neighbours(nodes: &[Node], pieces: &[LinePiece], up: bool) -> Vec<Vec<(usize, f64)>> {
    let mut out = vec![Vec::new(); nodes.len()];
    for p in pieces {
        if up {
            out[p.down].push((p.up, p.weight));
        } else {
            out[p.up].push((p.down, p.weight));
        }
    }
    out
}

/// Eades, Lin and Smyth's greedy order: sinks to the end, sources to the front, else the part whose
/// lines point out most. Returns each part's place in the order.
fn greedy_order(m: usize, lines: &[WeightedLine]) -> Vec<usize> {
    let mut outw = vec![0.0f64; m];
    let mut inw = vec![0.0f64; m];
    let mut outs: Vec<Vec<(usize, f64)>> = vec![Vec::new(); m];
    let mut ins: Vec<Vec<(usize, f64)>> = vec![Vec::new(); m];
    for &(a, b, w, _) in lines {
        outw[a] += w;
        inw[b] += w;
        outs[a].push((b, w));
        ins[b].push((a, w));
    }
    let mut left: Vec<usize> = Vec::new();
    let mut right: Vec<usize> = Vec::new();
    let mut gone = vec![false; m];
    let mut remaining = m;
    let remove = |v: usize, gone: &mut [bool], outw: &mut [f64], inw: &mut [f64]| {
        gone[v] = true;
        for &(b, w) in &outs[v] {
            inw[b] -= w;
        }
        for &(a, w) in &ins[v] {
            outw[a] -= w;
        }
    };
    let eps = 1e-9;
    while remaining > 0 {
        let mut progress = true;
        while progress {
            progress = false;
            if let Some(v) = (0..m).find(|&v| !gone[v] && outw[v] <= eps) {
                remove(v, &mut gone, &mut outw, &mut inw);
                right.push(v);
                remaining -= 1;
                progress = true;
                continue;
            }
            if let Some(v) = (0..m).find(|&v| !gone[v] && inw[v] <= eps) {
                remove(v, &mut gone, &mut outw, &mut inw);
                left.push(v);
                remaining -= 1;
                progress = true;
            }
        }
        if remaining == 0 {
            break;
        }
        let v = (0..m)
            .filter(|&v| !gone[v])
            .max_by(|&x, &y| {
                (outw[x] - inw[x])
                    .total_cmp(&(outw[y] - inw[y]))
                    .then(y.cmp(&x))
            })
            .expect("a part is left");
        remove(v, &mut gone, &mut outw, &mut inw);
        left.push(v);
        remaining -= 1;
    }
    right.reverse();
    let mut rank = vec![0; m];
    for (k, &v) in left.iter().chain(&right).enumerate() {
        rank[v] = k;
    }
    rank
}

/// Each part's layer: the longest path from the parts no line points to, then each part moved, as far
/// as its lines allow, to where they are shortest (a weighted median).
fn layers(m: usize, rank: &[usize], forward: &[WeightedLine]) -> Vec<usize> {
    let mut by_rank: Vec<usize> = (0..m).collect();
    by_rank.sort_by_key(|&v| rank[v]);
    let mut preds: Vec<Vec<(usize, f64)>> = vec![Vec::new(); m];
    let mut succs: Vec<Vec<(usize, f64)>> = vec![Vec::new(); m];
    for &(a, b, w, _) in forward {
        preds[b].push((a, w));
        succs[a].push((b, w));
    }
    let mut layer = vec![0i64; m];
    for &v in &by_rank {
        for &(a, _) in &preds[v] {
            layer[v] = layer[v].max(layer[a] + 1);
        }
    }
    for _ in 0..6 {
        let mut moved = false;
        for &v in by_rank.iter().rev().chain(by_rank.iter()) {
            if preds[v].is_empty() && succs[v].is_empty() {
                continue;
            }
            let lo = preds[v]
                .iter()
                .map(|&(a, _)| layer[a] + 1)
                .max()
                .unwrap_or(i64::MIN);
            let hi = succs[v]
                .iter()
                .map(|&(b, _)| layer[b] - 1)
                .min()
                .unwrap_or(i64::MAX);
            let mut want: Vec<(i64, f64)> = preds[v]
                .iter()
                .map(|&(a, w)| (layer[a] + 1, w))
                .chain(succs[v].iter().map(|&(b, w)| (layer[b] - 1, w)))
                .collect();
            want.sort_by_key(|&(l, _)| l);
            let total: f64 = want.iter().map(|&(_, w)| w).sum();
            let mut acc = 0.0;
            let mut median = want[0].0;
            for &(l, w) in &want {
                acc += w;
                if acc * 2.0 >= total {
                    median = l;
                    break;
                }
            }
            let to = median.clamp(lo.min(hi), hi.max(lo));
            if to != layer[v] && to >= lo && to <= hi {
                layer[v] = to;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    let low = layer.iter().copied().min().unwrap_or(0);
    layer.iter().map(|&l| (l - low) as usize).collect()
}

/// How many pieces between two neighbouring layers cross, for the given orders (Barth, Jünger and
/// Mutzel's count with a Fenwick tree).
fn crossings_between(
    upper: &[usize],
    lower: &[usize],
    downs: &[Vec<(usize, f64)>],
    at: &mut [usize],
) -> f64 {
    for (k, &v) in lower.iter().enumerate() {
        at[v] = k;
    }
    let mut ends: Vec<(usize, usize, f64)> = Vec::new();
    for (k, &u) in upper.iter().enumerate() {
        for &(d, _) in &downs[u] {
            if at[d] != usize::MAX {
                ends.push((k, at[d], 1.0));
            }
        }
    }
    for &v in lower {
        at[v] = usize::MAX;
    }
    ends.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let size = lower.len() + 1;
    let mut tree = vec![0.0f64; size + 1];
    let (mut added, mut count) = (0.0f64, 0.0f64);
    let mut g = 0;
    while g < ends.len() {
        let mut h = g;
        while h < ends.len() && ends[h].0 == ends[g].0 {
            h += 1;
        }
        // Pieces from one node cross none of each other: count the whole group first, then add it.
        for &(_, j, w) in &ends[g..h] {
            let mut at_most = 0.0;
            let mut i = j + 1;
            while i > 0 {
                at_most += tree[i];
                i &= i - 1;
            }
            count += w * (added - at_most);
        }
        for &(_, j, w) in &ends[g..h] {
            let mut i = j + 1;
            while i <= size {
                tree[i] += w;
                i += i & i.wrapping_neg();
            }
            added += w;
        }
        g = h;
    }
    count
}

fn crossings(order: &[Vec<usize>], downs: &[Vec<(usize, f64)>]) -> f64 {
    let mut at = vec![usize::MAX; downs.len()];
    order
        .windows(2)
        .map(|w| crossings_between(&w[0], &w[1], downs, &mut at))
        .sum()
}

/// Sweeps down and up, each layer sorted by the mean place of its neighbours; the order with the
/// fewest crossings is kept.
fn fewer_crossings(
    mut order: Vec<Vec<usize>>,
    ups: &[Vec<(usize, f64)>],
    downs: &[Vec<(usize, f64)>],
    pull: &[Vec<(f64, f64)>],
    nodes: &[Node],
    fixed_layer: &[bool],
) -> Vec<Vec<usize>> {
    let pulled = pull.iter().any(|p| !p.is_empty());
    let cost = |order: &[Vec<usize>]| {
        crossings(order, downs) + if pulled { misplaced(order, pull) } else { 0.0 }
    };
    let mut best = order.clone();
    let mut least = cost(&order);
    if least == 0.0 || (order.len() < 2 && !pulled) {
        return best;
    }
    let mut place = vec![0.0f64; nodes.len()];
    let mut stale = 0;
    for sweep in 0..24 {
        let going_down = sweep % 2 == 0;
        let range: Vec<usize> = match (pulled, going_down) {
            (true, true) => (0..order.len()).collect(),
            (true, false) => (0..order.len()).rev().collect(),
            (false, true) => (1..order.len()).collect(),
            (false, false) => (0..order.len() - 1).rev().collect(),
        };
        for row in &order {
            for (k, &v) in row.iter().enumerate() {
                place[v] = k as f64;
            }
        }
        for l in range {
            if fixed_layer[l] {
                continue;
            }
            let near = if going_down { ups } else { downs };
            let last = order[l].len().saturating_sub(1) as f64;
            let mut keyed: Vec<(f64, usize, usize)> = order[l]
                .iter()
                .enumerate()
                .map(|(k, &v)| {
                    let (mut sum, mut w) = near[v]
                        .iter()
                        .fold((0.0, 0.0), |(s, t), &(u, wt)| (s + place[u] * wt, t + wt));
                    // A line out of the box pulls its part toward the side it leaves by.
                    for &(at, wt) in &pull[v] {
                        sum += at * last * wt;
                        w += wt;
                    }
                    (if w > 0.0 { sum / w } else { k as f64 }, k, v)
                })
                .collect();
            // Ties go left first, and right first every other two sweeps (dagre's biasRight).
            let right = sweep % 4 >= 2;
            keyed.sort_by(|a, b| {
                a.0.total_cmp(&b.0)
                    .then(if right { b.1.cmp(&a.1) } else { a.1.cmp(&b.1) })
            });
            order[l] = keyed.into_iter().map(|(_, _, v)| v).collect();
            for (k, &v) in order[l].iter().enumerate() {
                place[v] = k as f64;
            }
        }
        transpose(&mut order, ups, downs, &mut place, fixed_layer);
        let c = cost(&order);
        if c < least {
            least = c;
            best = order.clone();
            stale = 0;
            if least == 0.0 {
                break;
            }
        } else {
            stale += 1;
            if stale >= 4 {
                break;
            }
        }
    }
    best
}

/// How far the parts stand from where the lines that leave the box pull them, in places of their layer.
fn misplaced(order: &[Vec<usize>], pull: &[Vec<(f64, f64)>]) -> f64 {
    let mut sum = 0.0;
    for row in order {
        let last = row.len().saturating_sub(1) as f64;
        for (k, &v) in row.iter().enumerate() {
            for &(at, w) in &pull[v] {
                sum += (k as f64 - at * last).abs() * w * 0.5;
            }
        }
    }
    sum
}

/// Swap two neighbours in a layer while that crosses fewer lines to the layers above and below
/// (the transpose step of Gansner, Koutsofios, North and Vo).
fn transpose(
    order: &mut [Vec<usize>],
    ups: &[Vec<(usize, f64)>],
    downs: &[Vec<(usize, f64)>],
    place: &mut [f64],
    fixed_layer: &[bool],
) {
    for row in order.iter() {
        for (k, &v) in row.iter().enumerate() {
            place[v] = k as f64;
        }
    }
    // Crossings between the lines of u and of v when u stands left of v.
    let pair = |u: usize, v: usize, place: &[f64]| -> usize {
        let mut c = 0;
        for near in [ups, downs] {
            for &(a, _) in &near[u] {
                for &(b, _) in &near[v] {
                    if place[a] > place[b] {
                        c += 1;
                    }
                }
            }
        }
        c
    };
    for _ in 0..4 {
        let mut better = false;
        for (l, row) in order.iter_mut().enumerate() {
            if fixed_layer[l] {
                continue;
            }
            for k in 0..row.len().saturating_sub(1) {
                let (u, v) = (row[k], row[k + 1]);
                if pair(u, v, place) > pair(v, u, place) {
                    row.swap(k, k + 1);
                    place[u] = (k + 1) as f64;
                    place[v] = k as f64;
                    better = true;
                }
            }
        }
        if !better {
            break;
        }
    }
}

/// The centre of each node across the layers.
fn positions(
    order: &[Vec<usize>],
    nodes: &[Node],
    ups: &[Vec<(usize, f64)>],
    downs: &[Vec<(usize, f64)>],
    pull: &[Vec<(f64, f64)>],
    sp: &Spacing,
    fixed_layer: &[bool],
) -> Vec<f64> {
    let gap = |a: usize, b: usize| match (nodes[a].part.is_some(), nodes[b].part.is_some()) {
        (true, true) => sp.part_part,
        (false, false) => sp.line_line,
        _ => sp.line_part,
    };
    let mut x = vec![0.0f64; nodes.len()];
    // Packed, from the left.
    for row in order {
        let mut at = 0.0;
        for (k, &v) in row.iter().enumerate() {
            if k > 0 {
                at += gap(row[k - 1], v);
            }
            x[v] = at + nodes[v].across / 2.0;
            at += nodes[v].across;
        }
    }
    let rows: Vec<&Vec<usize>> = order.iter().collect();
    // How wide the widest layer is, packed: where a pull from 0 to 1 points.
    let extent = order
        .iter()
        .enumerate()
        .filter(|(l, _)| !fixed_layer[*l])
        .map(|(_, row)| row.last().map_or(0.0, |&v| x[v] + nodes[v].across / 2.0))
        .fold(0.0, f64::max);
    // Where lines leave the box: fixed at their place along its side.
    for row in order {
        for &v in row {
            if let Some(at) = nodes[v].fixed {
                x[v] = at * extent;
            }
        }
    }
    for pass in 0..10 {
        let use_up = pass % 2 == 0;
        let both = pass >= 8;
        let range: Vec<usize> = if use_up {
            (0..rows.len()).collect()
        } else {
            (0..rows.len()).rev().collect()
        };
        for l in range {
            if fixed_layer[l] {
                continue;
            }
            let row = rows[l];
            let want: Vec<(f64, f64)> = row
                .iter()
                .map(|&v| {
                    let near: Vec<&(usize, f64)> = if both {
                        ups[v].iter().chain(downs[v].iter()).collect()
                    } else if use_up {
                        ups[v].iter().collect()
                    } else {
                        downs[v].iter().collect()
                    };
                    let (mut s, mut w) = near
                        .iter()
                        .fold((0.0, 0.0), |(s, t), &&(u, wt)| (s + x[u] * wt, t + wt));
                    if !near.is_empty() {
                        // The weighted median of the neighbours: in line with one of them.
                        let mut at: Vec<(f64, f64)> =
                            near.iter().map(|&&(u, wt)| (x[u], wt)).collect();
                        at.sort_by(|a, b| a.0.total_cmp(&b.0));
                        let half = w / 2.0;
                        let mut acc = 0.0;
                        let mut med = at[0].0;
                        for &(xv, wt) in &at {
                            acc += wt;
                            if acc >= half {
                                med = xv;
                                break;
                            }
                        }
                        s = med * w;
                    }
                    for &(at, wt) in &pull[v] {
                        s += at * extent * wt;
                        w += wt;
                    }
                    if w > 0.0 { (s / w, w) } else { (x[v], 0.05) }
                })
                .collect();
            let placed = isotonic(row, &want, nodes, &gap);
            for (k, &v) in row.iter().enumerate() {
                x[v] = placed[k];
            }
        }
    }
    x
}

/// The centres closest to what each node wants (weighted least squares) that keep the order and the gaps.
fn isotonic(
    row: &[usize],
    want: &[(f64, f64)],
    nodes: &[Node],
    gap: &dyn Fn(usize, usize) -> f64,
) -> Vec<f64> {
    // With c[k] the least distance from the first centre, y = x - c must not fall: pool adjacent violators.
    let mut c = vec![0.0f64; row.len()];
    for k in 1..row.len() {
        c[k] = c[k - 1]
            + nodes[row[k - 1]].across / 2.0
            + gap(row[k - 1], row[k])
            + nodes[row[k]].across / 2.0;
    }
    // Blocks: (sum of weight·value, sum of weight, count).
    let mut blocks: Vec<(f64, f64, usize)> = Vec::new();
    for k in 0..row.len() {
        let (d, w) = want[k];
        blocks.push(((d - c[k]) * w, w, 1));
        while blocks.len() >= 2 {
            let n = blocks.len();
            let (s1, w1, _) = blocks[n - 2];
            let (s2, w2, _) = blocks[n - 1];
            if s1 / w1 > s2 / w2 {
                let (_, _, k2) = blocks.pop().unwrap();
                let last = blocks.last_mut().unwrap();
                last.0 += s2;
                last.1 += w2;
                last.2 += k2;
            } else {
                break;
            }
        }
    }
    let mut out = Vec::with_capacity(row.len());
    let mut k = 0;
    for (s, w, count) in blocks {
        let y = s / w;
        for _ in 0..count {
            out.push(y + c[k]);
            k += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(a: usize, b: usize) -> BoxLine {
        BoxLine {
            a,
            b,
            ahead: 1,
            words: None,
        }
    }

    #[test]
    fn a_chain_runs_down_in_layers() {
        let g = BoxGraph {
            sizes: vec![(60.0, 30.0); 3],
            lines: vec![line(0, 1), line(1, 2)],
            direction: Direction::Down,
            pulls: Vec::new(),
            exits: Vec::new(),
        };
        let p = lay_out(&g, &Spacing::default());
        assert!(p[0].1 < p[1].1 && p[1].1 < p[2].1, "{p:?}");
        assert!(
            (p[0].0 - p[1].0).abs() < 1e-6 && (p[1].0 - p[2].0).abs() < 1e-6,
            "a chain runs straight: {p:?}"
        );
    }

    #[test]
    fn a_cycle_is_broken_by_priority() {
        let g = BoxGraph {
            sizes: vec![(60.0, 30.0); 2],
            lines: vec![
                BoxLine {
                    a: 0,
                    b: 1,
                    ahead: 10,
                    words: None,
                },
                BoxLine {
                    a: 1,
                    b: 0,
                    ahead: 0,
                    words: None,
                },
            ],
            direction: Direction::Right,
            pulls: Vec::new(),
            exits: Vec::new(),
        };
        let p = lay_out(&g, &Spacing::default());
        assert!(
            p[0].0 < p[1].0,
            "the line that points ahead most points right: {p:?}"
        );
    }

    #[test]
    fn parts_without_lines_are_packed_apart() {
        let g = BoxGraph {
            sizes: vec![(60.0, 30.0); 4],
            lines: Vec::new(),
            direction: Direction::Down,
            pulls: Vec::new(),
            exits: Vec::new(),
        };
        let p = lay_out(&g, &Spacing::default());
        for i in 0..4 {
            for j in i + 1..4 {
                let apart = (p[i].0 - p[j].0).abs() >= 60.0 || (p[i].1 - p[j].1).abs() >= 30.0;
                assert!(apart, "{p:?}");
            }
        }
    }

    #[test]
    fn crossings_are_undone() {
        // 0 → 3 and 1 → 2 cross in the order given; the sweeps undo it.
        let g = BoxGraph {
            sizes: vec![(60.0, 30.0); 4],
            lines: vec![line(0, 3), line(1, 2), line(0, 2)],
            direction: Direction::Down,
            pulls: Vec::new(),
            exits: Vec::new(),
        };
        let p = lay_out(&g, &Spacing::default());
        let left_first = p[0].0 < p[1].0;
        assert_eq!(left_first, p[3].0 < p[2].0, "{p:?}");
        let order = vec![vec![0, 1], vec![2, 3]];
        let downs = vec![vec![(3, 1.0), (2, 1.0)], vec![(2, 1.0)], vec![], vec![]];
        assert_eq!(crossings(&order, &downs), 1.0);
    }
}
