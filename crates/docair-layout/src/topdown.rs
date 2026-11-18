//! Top-down (after Kasperowski and von Hanxleden): what is inside a box is laid out on its own, then
//! scaled to the box's size budget, so a box with a lot inside does not blow the picture up; the
//! reader zooms in to read it. Inside out, each box shrinks what it holds to at most its budget, but
//! never below its floor. A line between parts in different boxes is laid out as a line between the
//! two parts side by side in the box that holds both ends, so the order things happen in is kept.

use std::collections::HashMap;

use crate::layered::{BoxGraph, BoxLine, Direction, Spacing, lay_out};
use crate::{Input, Options, Placement, Rect};

/// A line as one box lays it out: between two of its parts.
struct Lifted {
    a: usize,
    b: usize,
    /// The line itself, when it runs between the two parts directly: its words get room.
    own: Option<usize>,
    ahead: u32,
}

/// A box's content after its layout: its scale, where it starts and its size.
#[derive(Clone, Copy, Default)]
struct ContentFit {
    k: f64,
    min_x: f64,
    min_y: f64,
    w: f64,
    h: f64,
}

/// A way a box learns where its lines go, from the placement before (viewer.js `TOPDOWN_WAYS`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct BoxWay {
    pub key: &'static str,
    /// A pull for every line that leaves a box; else one for all lines of one part that leave by one side.
    pub per_line: bool,
    /// Only boxes whose parts have lines among them are pulled (the others are packed).
    pub linked: bool,
    /// The parts of every box are given in the order of where their lines go.
    pub sorted: bool,
}

pub(crate) const WAYS: [BoxWay; 4] = [
    BoxWay {
        key: "a port per line",
        per_line: true,
        linked: false,
        sorted: false,
    },
    BoxWay {
        key: "ports in order",
        per_line: false,
        linked: false,
        sorted: false,
    },
    BoxWay {
        key: "a port per line, parts in order",
        per_line: true,
        linked: true,
        sorted: true,
    },
    BoxWay {
        key: "ports and parts in order",
        per_line: false,
        linked: true,
        sorted: true,
    },
];

/// A way with a pull per line is left out when a box would get more pulls than this.
const LINE_PORTS: usize = 40;

/// The side of `a` that faces `b`: 0 top, 1 right, 2 bottom, 3 left.
fn facing(a: &Rect, b: &Rect) -> u8 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    if dx.abs() / (a.w + b.w).max(1.0) > dy.abs() / (a.h + b.h).max(1.0) {
        if dx > 0.0 { 1 } else { 3 }
    } else if dy > 0.0 {
        2
    } else {
        0
    }
}

/// One top-down placement in one direction: as it is, or in a way from the placement `prev`.
/// None when the way does not suit the picture (`LINE_PORTS`).
pub(crate) fn top_down(
    input: &Input,
    direction: Direction,
    opts: &Options,
    way: Option<&BoxWay>,
    prev: Option<&Placement>,
) -> Option<Placement> {
    let n = input.parts.len();
    let root = n;
    let parent_of = |i: usize| {
        input.parts[i]
            .parent
            .filter(|&p| p < n && p != i)
            .unwrap_or(root)
    };
    let mut kids: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
    for i in 0..n {
        kids[parent_of(i)].push(i);
    }
    let is_box = |i: usize| i == root || !kids[i].is_empty();
    let chain = |i: usize| -> Vec<usize> {
        let mut out = Vec::new();
        let mut cur = i;
        while cur != root && out.len() <= n {
            out.push(cur);
            cur = parent_of(cur);
        }
        out.reverse();
        out
    };
    // Each line, lifted to the two parts side by side under the box that holds both ends.
    let mut lifted: Vec<Vec<Lifted>> = (0..=n).map(|_| Vec::new()).collect();
    let mut at: HashMap<(usize, usize, usize), usize> = HashMap::new();
    for (li, l) in input.lines.iter().enumerate() {
        if l.from >= n || l.to >= n {
            continue;
        }
        let (a, b) = if l.flip {
            (chain(l.to), chain(l.from))
        } else {
            (chain(l.from), chain(l.to))
        };
        let mut k = 0;
        while k < a.len() && k < b.len() && a[k] == b[k] {
            k += 1;
        }
        if k >= a.len() || k >= b.len() {
            continue;
        }
        let holder = if k == 0 { root } else { a[k - 1] };
        let own = !l.flip && a[k] == l.from && b[k] == l.to;
        match at.get(&(holder, a[k], b[k])) {
            Some(&j) => {
                let had = &mut lifted[holder][j];
                let ahead = had.ahead.max(l.ahead);
                if own {
                    *had = Lifted {
                        a: a[k],
                        b: b[k],
                        own: Some(li),
                        ahead,
                    };
                } else {
                    had.ahead = ahead;
                }
            }
            None => {
                at.insert((holder, a[k], b[k]), lifted[holder].len());
                lifted[holder].push(Lifted {
                    a: a[k],
                    b: b[k],
                    own: own.then_some(li),
                    ahead: l.ahead,
                });
            }
        }
    }
    // Where each box's lines go outside it, in the placement before: a pull toward that place.
    let mut pulls: Vec<Vec<(usize, f64, f64)>> = vec![Vec::new(); n + 1];
    let mut exits: Vec<Vec<(usize, bool, f64)>> = vec![Vec::new(); n + 1];
    let mut order: Vec<Option<Vec<usize>>> = vec![None; n + 1];
    if let (Some(way), Some(prev)) = (way, prev) {
        let mut seen: std::collections::HashSet<(usize, usize, u8, bool)> =
            std::collections::HashSet::new();
        for l in &input.lines {
            if l.from >= n || l.to >= n {
                continue;
            }
            let (a, b) = (chain(l.from), chain(l.to));
            for (mine, theirs, other, out) in [(&a, &b, l.to, true), (&b, &a, l.from, false)] {
                for k in 0..mine.len().saturating_sub(1) {
                    let bx = mine[k];
                    if theirs.contains(&bx) || (way.linked && lifted[bx].is_empty()) {
                        continue;
                    }
                    let (bp, at) = (&prev.parts[bx], &prev.parts[other]);
                    let side = facing(bp, at);
                    let child = mine[k + 1];
                    if !way.per_line && !seen.insert((bx, child, side, out)) {
                        continue;
                    }
                    // Across the layers, from 0 at the left (or top) end of the box to 1 at the other.
                    let along_x = ((at.x - (bp.x - bp.w / 2.0)) / bp.w.max(1.0)).clamp(0.0, 1.0);
                    let along_y = ((at.y - (bp.y - bp.h / 2.0)) / bp.h.max(1.0)).clamp(0.0, 1.0);
                    // A line leaving before the box's first layer or after its last gets a node of
                    // its own there; one leaving by a side across the layers pulls its part that way.
                    let exit = match (direction, side) {
                        (Direction::Down, 0) => Some((false, along_x)),
                        (Direction::Down, 2) => Some((true, along_x)),
                        (Direction::Right, 3) => Some((false, along_y)),
                        (Direction::Right, 1) => Some((true, along_y)),
                        _ => None,
                    };
                    let place = match (direction, side) {
                        (Direction::Down, 0 | 2) => along_x,
                        (Direction::Down, 1) => 1.0,
                        (Direction::Down, _) => 0.0,
                        (Direction::Right, 1 | 3) => along_y,
                        (Direction::Right, 2) => 1.0,
                        (Direction::Right, _) => 0.0,
                    };
                    match exit {
                        Some((out, at)) => {
                            exits[bx].push((child, out, at));
                            pulls[bx].push((child, place, 1.0));
                        }
                        _ => pulls[bx].push((child, place, 1.0)),
                    }
                }
            }
        }
        // A pull per line for a box with very many lines: the way never wins, and costs the most.
        if way.per_line && (0..=n).any(|b| pulls[b].len() + exits[b].len() > LINE_PORTS) {
            return None;
        }
        if way.sorted {
            // The parts of each box in the order of where their lines go (across the direction of the layout).
            let axis = |r: &Rect| {
                if direction == Direction::Right {
                    r.y
                } else {
                    r.x
                }
            };
            let mut sum = vec![(0.0f64, 0usize); n];
            for l in &input.lines {
                if l.from >= n || l.to >= n {
                    continue;
                }
                let (a, b) = (chain(l.from), chain(l.to));
                let (pa, pb) = (axis(&prev.parts[l.from]), axis(&prev.parts[l.to]));
                let mut k = 0;
                while k < a.len() && k < b.len() && a[k] == b[k] {
                    k += 1;
                }
                for &v in &a[k..] {
                    sum[v].0 += pb;
                    sum[v].1 += 1;
                }
                for &v in &b[k..] {
                    sum[v].0 += pa;
                    sum[v].1 += 1;
                }
            }
            let key = |v: usize| {
                if sum[v].1 > 0 {
                    sum[v].0 / sum[v].1 as f64
                } else {
                    axis(&prev.parts[v])
                }
            };
            for bx in 0..=n {
                let mut ids = kids[bx].clone();
                ids.sort_by(|&u, &v| key(u).total_cmp(&key(v)));
                order[bx] = Some(ids);
            }
        }
    }
    // Inside out: the boxes that hold no boxes first, then the boxes around them.
    let mut height = vec![0usize; n + 1];
    fn height_of(b: usize, kids: &[Vec<usize>], height: &mut [usize], depth: usize) -> usize {
        if height[b] > 0 || depth > kids.len() {
            return height[b];
        }
        let h = 1 + kids[b]
            .iter()
            .filter(|&&c| !kids[c].is_empty())
            .map(|&c| height_of(c, kids, height, depth + 1))
            .max()
            .unwrap_or(0);
        height[b] = h;
        h
    }
    height_of(root, &kids, &mut height, 0);
    let mut boxes: Vec<usize> = (0..=n).filter(|&b| is_box(b) && height[b] > 0).collect();
    boxes.sort_by_key(|&b| (height[b], b));
    let mut size: Vec<(f64, f64)> = input.parts.iter().map(|p| (p.width, p.height)).collect();
    {
        // A part with many lines is as wide as they need to meet it side by side: a track each, half on either long side.
        let mut lines_at = vec![0usize; n];
        for l in &input.lines {
            if l.from < n && l.to < n {
                lines_at[l.from] += 1;
                lines_at[l.to] += 1;
            }
        }
        for i in 0..n {
            if kids[i].is_empty() {
                let need = (lines_at[i] as f64 / 2.0).ceil() * 8.0 * 1.5 + 16.0;
                if direction == Direction::Down {
                    size[i].0 = size[i].0.max(need);
                } else {
                    size[i].1 = size[i].1.max(need);
                }
            }
        }
    }
    let mut local: Vec<(f64, f64)> = vec![(0.0, 0.0); n];
    let mut inner: Vec<ContentFit> = vec![ContentFit::default(); n + 1];
    let sp = Spacing::default();
    let pad = opts.padding;
    for &bx in &boxes {
        let ids = order[bx].as_ref().unwrap_or(&kids[bx]);
        let index: HashMap<usize, usize> = ids.iter().enumerate().map(|(k, &v)| (v, k)).collect();
        let lines = lifted[bx]
            .iter()
            .filter_map(|l| {
                Some(BoxLine {
                    a: *index.get(&l.a)?,
                    b: *index.get(&l.b)?,
                    ahead: l.ahead,
                    words: l.own.and_then(|li| input.lines[li].words),
                })
            })
            .collect();
        let g = BoxGraph {
            sizes: ids.iter().map(|&v| size[v]).collect(),
            lines,
            direction,
            pulls: pulls[bx]
                .iter()
                .filter_map(|&(c, at, w)| Some((*index.get(&c)?, at, w)))
                .collect(),
            exits: exits[bx]
                .iter()
                .filter_map(|&(c, out, at)| Some((*index.get(&c)?, out, at)))
                .collect(),
        };
        let pts = lay_out(&g, &sp);
        let (mut x1, mut y1, mut x2, mut y2) = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for (k, &v) in ids.iter().enumerate() {
            let (x, y) = pts[k];
            local[v] = (x, y);
            x1 = x1.min(x);
            y1 = y1.min(y);
            x2 = x2.max(x + size[v].0);
            y2 = y2.max(y + size[v].1);
        }
        let w = (x2 - x1).max(1.0);
        let h = (y2 - y1).max(1.0);
        // The top of the picture is not scaled; a box shrinks what it holds to its budget, not below its floor.
        let k = if bx == root {
            1.0
        } else {
            opts.floor.max((opts.budget / w.max(h)).min(1.0))
        };
        inner[bx] = ContentFit {
            k,
            min_x: x1,
            min_y: y1,
            w,
            h,
        };
        if bx != root {
            size[bx] = (
                input.parts[bx].min_width.max(w * k + pad.left + pad.right),
                h * k + pad.top + pad.bottom,
            );
        }
    }
    // Place everything: inside a box, its content scaled by the box's k, below its name, centred.
    let mut pos = vec![
        Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0
        };
        n
    ];
    let mut scale = vec![1.0f64; n];
    let mut todo: Vec<(usize, f64, f64, f64)> = vec![(root, 0.0, 0.0, 1.0)];
    while let Some((bx, ox, oy, s)) = todo.pop() {
        let ContentFit {
            k, min_x, min_y, ..
        } = inner[bx];
        let cs = s * k;
        for &id in &kids[bx] {
            let (lx, ly) = local[id];
            let (sw, sh) = size[id];
            let x = ox + (lx - min_x) * cs;
            let y = oy + (ly - min_y) * cs;
            pos[id] = Rect {
                x: x + sw * cs / 2.0,
                y: y + sh * cs / 2.0,
                w: sw * cs,
                h: sh * cs,
            };
            scale[id] = cs;
            if !kids[id].is_empty() {
                let c = inner[id];
                let free = sw - pad.left - pad.right - c.w * c.k;
                todo.push((
                    id,
                    x + (pad.left + free.max(0.0) / 2.0) * cs,
                    y + pad.top * cs,
                    cs,
                ));
            }
        }
    }
    let r = inner[root];
    let fit = 72.0;
    let zoom = ((opts.canvas.0 - fit) / r.w).min((opts.canvas.1 - fit) / r.h);
    let mut placed = Placement {
        parts: pos,
        scale,
        width: r.w,
        height: r.h,
        direction,
        zoom,
        way: way.map_or("as it is", |w| w.key).to_string(),
        price: 0.0,
    };
    placed.price = straight_price(input, &placed);
    Some(placed)
}

/// What a placement's lines cost drawn straight from part to part: their length, and 400 px for every
/// crossing (viewer.js `straightPrice`, with the router's price of a crossing).
pub(crate) fn straight_price(input: &Input, placed: &Placement) -> f64 {
    let n = placed.parts.len();
    let segs: Vec<(usize, usize)> = input
        .lines
        .iter()
        .filter(|l| l.from < n && l.to < n)
        .map(|l| (l.from, l.to))
        .collect();
    let at = |i: usize| (placed.parts[i].x, placed.parts[i].y);
    let ccw = |a: (f64, f64), b: (f64, f64), c: (f64, f64)| {
        (c.1 - a.1) * (b.0 - a.0) > (b.1 - a.1) * (c.0 - a.0)
    };
    let mut price = 0.0;
    for (i, &(a, b)) in segs.iter().enumerate() {
        let (pa, pb) = (at(a), at(b));
        price += (pa.0 - pb.0).abs() + (pa.1 - pb.1).abs();
        for &(c, d) in &segs[i + 1..] {
            if a == c || a == d || b == c || b == d {
                continue;
            }
            let (pc, pd) = (at(c), at(d));
            if ccw(pa, pc, pd) != ccw(pb, pc, pd) && ccw(pa, pb, pc) != ccw(pa, pb, pd) {
                price += CROSSING;
            }
        }
    }
    price
}

/// What a crossing costs, in px of line.
pub(crate) const CROSSING: f64 = 400.0;

#[cfg(test)]
mod tests {
    use crate::{Input, Line, Options, Part, place};

    fn part(id: &str, parent: Option<usize>) -> Part {
        Part {
            id: id.into(),
            parent,
            width: 80.0,
            height: 30.0,
            min_width: 0.0,
            name_width: 0.0,
        }
    }

    #[test]
    fn a_box_holds_its_parts_below_its_name() {
        let input = Input {
            parts: vec![
                part("box", None),
                part("a", Some(0)),
                part("b", Some(0)),
                part("out", None),
            ],
            lines: vec![
                Line {
                    id: "a-b".into(),
                    from: 1,
                    to: 2,
                    flip: false,
                    ahead: 1,
                    words: None,
                },
                Line {
                    id: "b-out".into(),
                    from: 2,
                    to: 3,
                    flip: false,
                    ahead: 1,
                    words: None,
                },
            ],
        };
        let p = place(&input, &Options::default());
        let (bx, a, b) = (p.parts[0], p.parts[1], p.parts[2]);
        for r in [a, b] {
            assert!(
                r.x - r.w / 2.0 >= bx.x - bx.w / 2.0 - 1e-6
                    && r.x + r.w / 2.0 <= bx.x + bx.w / 2.0 + 1e-6
            );
            assert!(
                r.y - r.h / 2.0 >= bx.y - bx.h / 2.0 + 28.0 - 1e-6,
                "below the name: {p:?}"
            );
            assert!(r.y + r.h / 2.0 <= bx.y + bx.h / 2.0 + 1e-6);
        }
        assert_eq!(
            p,
            place(&input, &Options::default()),
            "the same input gives the same picture"
        );
    }

    #[test]
    fn a_layout_is_kept_by_what_it_reads() {
        use crate::key;
        let input = Input {
            parts: vec![part("a", None), part("b", None)],
            lines: vec![Line {
                id: "a-b".into(),
                from: 0,
                to: 1,
                flip: false,
                ahead: 1,
                words: None,
            }],
        };
        let k = key(&input, &Options::default());
        assert_eq!(k, key(&input.clone(), &Options::default()));
        let mut renamed = input.clone();
        renamed.parts[0].id = "z".into();
        renamed.lines[0].id = "z-b".into();
        assert_eq!(
            k,
            key(&renamed, &Options::default()),
            "ids are not what a layout reads"
        );
        let mut wider = input.clone();
        wider.parts[1].width += 6.0;
        assert_ne!(k, key(&wider, &Options::default()), "a size is");
    }

    #[test]
    fn a_big_box_shrinks_what_it_holds_but_not_below_its_floor() {
        let mut parts = vec![part("box", None)];
        for i in 0..200 {
            parts.push(part(&format!("p{i}"), Some(0)));
        }
        let p = place(
            &Input {
                parts,
                lines: Vec::new(),
            },
            &Options::default(),
        );
        assert!(
            p.scale[1] < 1.0 && p.scale[1] >= 0.7 - 1e-9,
            "{}",
            p.scale[1]
        );
    }
}
