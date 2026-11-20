//! The lines of a placed picture: each line the cheapest way at right angles, given all the others
//! (viewer.js `gridLines`, docair's own router, SPEC §3.5).
//!
//! Lines run on tracks: beside every part (at its clearance), through the middle of every line's end,
//! and on a lattice 8 px apart in between. A line may leave a part anywhere on any side. Each line
//! takes the cheapest way given the others (`Prices` says what costs what: length, bends, crossings,
//! running on or close beside another line, the borders of boxes that hold neither end, a box's
//! name); then every line is routed again, given the others as they now are, until no line finds a
//! cheaper way (after McMurchie and Ebeling's PathFinder). In a big picture a line is first routed
//! roughly, from cell to cell, and then exactly within the cells around that way.

use crate::{Input, Placement, Rect};

/// What a line's way costs, in px of line (viewer.js `ROUTER`, tuned on docair's own layers).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Prices {
    /// Between two tracks in open space.
    pub pitch: f64,
    /// Between a line and a part, times the scale the part is drawn at.
    pub clear: f64,
    pub bend: f64,
    /// Crossing another line.
    pub cross: f64,
    /// A piece of line on top of another line, or on a track less than 3 px beside it: per piece …
    pub over: f64,
    /// … and per px.
    pub over_px: f64,
    /// Per px closer than `apart` beside another line.
    pub near: f64,
    pub apart: f64,
    /// Crossing the border of a box that holds neither end.
    pub thru: f64,
    /// Per px along a box's border.
    pub hug_box: f64,
    /// Per px on the track right beside a part.
    pub hug_part: f64,
    /// Per px across a box's name.
    pub name: f64,
    /// Ending where another line ends.
    pub share_end: f64,
    /// Per px an end is off the middle of its side.
    pub off_centre: f64,
    /// A line keeps to ways at most this much longer than the direct one: px, plus a share of it.
    pub detour: (f64, f64),
    /// Every line is routed again, given the others, at most this often.
    pub rounds: usize,
    /// Big pictures: a line is first routed from cell to cell of this size …
    pub cell: f64,
    /// … then exactly in this many cells around that way.
    pub around: i64,
    /// "Big": from this many grid nodes on.
    pub cells_from: usize,
    /// Up to this many lines, every placement is routed and the cheapest picture wins.
    pub exact_up_to: usize,
    /// More grid nodes than this: wider tracks.
    pub most: usize,
    /// Tracks closer than this are one.
    pub merge: f64,
}

pub(crate) const PRICES: Prices = Prices {
    pitch: 8.0,
    clear: 8.0,
    bend: 60.0,
    cross: 400.0,
    over: 1200.0,
    over_px: 6.0,
    near: 1.5,
    apart: 6.0,
    thru: 150.0,
    hug_box: 1.0,
    hug_part: 0.25,
    name: 3.0,
    share_end: 800.0,
    off_centre: 0.08,
    detour: (200.0, 0.3),
    rounds: 5,
    cell: 32.0,
    around: 2,
    cells_from: 60_000,
    exact_up_to: 60,
    most: 1_500_000,
    merge: 3.0,
};

/// The lines of a picture, routed.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Routes {
    /// Each line's points, from where it touches its source to where it touches its target, in the
    /// order of `Input::lines`.
    pub lines: Vec<Vec<(f64, f64)>>,
    /// What the lines cost: length, bends, crossings, lines on lines, borders of other boxes.
    pub price: f64,
    /// How many lines found no way and are drawn with one corner.
    pub none: usize,
}

/// Where a line may leave or enter a part: a node on the track beside one of its sides.
#[derive(Debug, Clone, Copy)]
struct TrackEnd {
    n: usize,
    /// 0: the line leaves along a row; 1: along a column.
    o: usize,
    stub: f64,
    off: f64,
    bx: f64,
    by: f64,
}

/// A line's way on the grid: the nodes it passes, and how it leaves its source and enters its target.
#[derive(Debug, Clone)]
struct GridWay {
    nodes: Vec<usize>,
    st: TrackEnd,
    tt: TrackEnd,
}

struct PartRoom {
    l: f64,
    r: f64,
    t: f64,
    b: f64,
    c: f64,
}

struct BoxRoom {
    part: usize,
    l: f64,
    r: f64,
    t: f64,
    b: f64,
    x: f64,
    sc: f64,
    name_w: f64,
}

/// The first index of `arr` at or after `v`.
fn lower(arr: &[f64], v: f64) -> isize {
    arr.partition_point(|&a| a < v) as isize
}

/// The index of the track at `v`, or -1.
fn track_at(arr: &[f64], v: f64) -> isize {
    let i = lower(arr, v - 0.25);
    if (i as usize) < arr.len() && (arr[i as usize] - v).abs() < 0.6 {
        i
    } else {
        -1
    }
}

/// The tracks along one axis: beside every part, through the middle of every end, a lattice between.
fn tracks(hard: &[f64], soft: &[f64], lo: f64, hi: f64, pitch: f64, merge: f64) -> Vec<f64> {
    const MARGIN: f64 = 40.0;
    let mut acc: Vec<f64> = Vec::new();
    let add = |vals: &[f64], d: f64, acc: &mut Vec<f64>| {
        let mut sorted = acc.clone();
        sorted.sort_by(f64::total_cmp);
        for &v in vals {
            let k = sorted.partition_point(|&u| u < v);
            let near = (k < sorted.len() && (sorted[k] - v).abs() < d)
                || (k > 0 && (sorted[k - 1] - v).abs() < d);
            if !near {
                acc.push(v);
                sorted.insert(k, v);
            }
        }
    };
    add(hard, 0.5, &mut acc);
    add(soft, merge, &mut acc);
    acc.sort_by(f64::total_cmp);
    let mut all = acc.clone();
    let mut k = 0;
    let mut v = ((lo - MARGIN) / pitch).floor() * pitch;
    while v <= hi + MARGIN {
        while k < acc.len() && acc[k] < v - merge {
            k += 1;
        }
        let close = (k < acc.len() && (acc[k] - v).abs() < merge)
            || (k > 0 && (acc[k - 1] - v).abs() < merge);
        if !close {
            all.push(v);
        }
        v += pitch;
    }
    all.sort_by(f64::total_cmp);
    all
}

/// Tracks closer than `apart` beside each track: where each one's list starts, the tracks, and
/// whether they are less than `merge` apart (a line there is as good as on the other).
fn near_of(arr: &[f64], apart: f64, merge: f64) -> (Vec<usize>, Vec<usize>, Vec<bool>) {
    let mut start = vec![0usize; arr.len() + 1];
    let mut list = Vec::new();
    let mut tight = Vec::new();
    for i in 0..arr.len() {
        start[i] = list.len();
        let mut k = i;
        while k > 0 && arr[i] - arr[k - 1] < apart {
            k -= 1;
            list.push(k);
            tight.push(arr[i] - arr[k] < merge);
        }
        let mut k = i + 1;
        while k < arr.len() && arr[k] - arr[i] < apart {
            list.push(k);
            tight.push(arr[k] - arr[i] < merge);
            k += 1;
        }
    }
    start[arr.len()] = list.len();
    (start, list, tight)
}

/// A binary heap of (price, state), the smallest price first.
struct Heap {
    f: Vec<f64>,
    s: Vec<usize>,
}

impl Heap {
    fn push(&mut self, f: f64, s: usize) {
        let mut i = self.f.len();
        self.f.push(f);
        self.s.push(s);
        while i > 0 {
            let p = (i - 1) >> 1;
            if self.f[p] <= f {
                break;
            }
            self.f[i] = self.f[p];
            self.s[i] = self.s[p];
            i = p;
        }
        self.f[i] = f;
        self.s[i] = s;
    }

    fn pop(&mut self) -> Option<usize> {
        if self.f.is_empty() {
            return None;
        }
        let top = self.s[0];
        let f = self.f.pop().unwrap();
        let s = self.s.pop().unwrap();
        let n = self.f.len();
        if n == 0 {
            return Some(top);
        }
        let mut i = 0;
        loop {
            let mut c = 2 * i + 1;
            if c >= n {
                break;
            }
            if c + 1 < n && self.f[c + 1] < self.f[c] {
                c += 1;
            }
            if self.f[c] >= f {
                break;
            }
            self.f[i] = self.f[c];
            self.s[i] = self.s[c];
            i = c;
        }
        self.f[i] = f;
        self.s[i] = s;
        Some(top)
    }
}

/// The router's state for one picture.
struct Grid<'a> {
    placed: &'a Placement,
    p: Prices,
    x: Vec<f64>,
    y: Vec<f64>,
    nx: usize,
    ny: usize,
    h_c: Vec<f32>,
    v_c: Vec<f32>,
    blocked: Vec<bool>,
    h_box: Vec<i32>,
    v_box: Vec<i32>,
    near_x: (Vec<usize>, Vec<usize>, Vec<bool>),
    near_y: (Vec<usize>, Vec<usize>, Vec<bool>),
    terms: Vec<Option<Vec<TrackEnd>>>,
    // the rough picture
    rough: bool,
    cx: Vec<usize>,
    cy: Vec<usize>,
    ccx: usize,
    ccy: usize,
    cap_r: Vec<u32>,
    cap_d: Vec<u32>,
    use_r: Vec<i32>,
    use_d: Vec<i32>,
    flow_h: Vec<f32>,
    flow_v: Vec<f32>,
    cell_box: Vec<i32>,
    box_parent: Vec<i32>,
    box_depth: Vec<usize>,
    // who uses what
    h_use: Vec<i32>,
    v_use: Vec<i32>,
    h_thru: Vec<i32>,
    v_thru: Vec<i32>,
    corner: Vec<i32>,
    term_use: Vec<i32>,
    // the rough search
    c_g: Vec<f32>,
    c_stamp: Vec<u32>,
    c_done: Vec<u32>,
    c_from: Vec<i64>,
    in_way: Vec<u32>,
    c_gen: u32,
    // the exact search
    g_cost: Vec<f32>,
    stamp: Vec<u32>,
    done: Vec<u32>,
    from: Vec<i64>,
    goal_at: Vec<usize>,
    goal_gen: Vec<u32>,
    hx: Vec<f32>,
    hy: Vec<f32>,
    sx: Vec<f32>,
    sy: Vec<f32>,
    nb: usize,
    b_head: Vec<i64>,
    q_next: Vec<i64>,
    q_state: Vec<usize>,
    q_cur: usize,
    q_max: i64,
    gen_: u32,
}

impl<'a> Grid<'a> {
    fn rect(&self, i: usize) -> (f64, f64, f64, f64, f64, f64) {
        let r: Rect = self.placed.parts[i];
        (
            r.x - r.w / 2.0,
            r.x + r.w / 2.0,
            r.y - r.h / 2.0,
            r.y + r.h / 2.0,
            r.x,
            r.y,
        )
    }

    fn scale(&self, i: usize) -> f64 {
        self.placed.scale.get(i).copied().unwrap_or(1.0)
    }

    fn terminals(&mut self, id: usize) -> Vec<TrackEnd> {
        if let Some(t) = &self.terms[id] {
            return t.clone();
        }
        let (l, r, t, b, x, y) = self.rect(id);
        let c = self.p.clear * self.scale(id);
        let mut out = Vec::new();
        let m = (4.0 * self.scale(id)).min((r - l) / 4.0).min((b - t) / 4.0);
        let (jt, jb, il, ir) = (
            track_at(&self.y, t - c),
            track_at(&self.y, b + c),
            track_at(&self.x, l - c),
            track_at(&self.x, r + c),
        );
        let nx = self.nx;
        let mut i = lower(&self.x, l + m) as usize;
        while i < nx && self.x[i] <= r - m {
            if jt >= 0 && !self.blocked[jt as usize * nx + i] {
                out.push(TrackEnd {
                    n: jt as usize * nx + i,
                    o: 1,
                    stub: c,
                    off: (self.x[i] - x).abs(),
                    bx: self.x[i],
                    by: t,
                });
            }
            if jb >= 0 && !self.blocked[jb as usize * nx + i] {
                out.push(TrackEnd {
                    n: jb as usize * nx + i,
                    o: 1,
                    stub: c,
                    off: (self.x[i] - x).abs(),
                    bx: self.x[i],
                    by: b,
                });
            }
            i += 1;
        }
        let mut j = lower(&self.y, t + m) as usize;
        while j < self.ny && self.y[j] <= b - m {
            if il >= 0 && !self.blocked[j * nx + il as usize] {
                out.push(TrackEnd {
                    n: j * nx + il as usize,
                    o: 0,
                    stub: c,
                    off: (self.y[j] - y).abs(),
                    bx: l,
                    by: self.y[j],
                });
            }
            if ir >= 0 && !self.blocked[j * nx + ir as usize] {
                out.push(TrackEnd {
                    n: j * nx + ir as usize,
                    o: 0,
                    stub: c,
                    off: (self.y[j] - y).abs(),
                    bx: r,
                    by: self.y[j],
                });
            }
            j += 1;
        }
        self.terms[id] = Some(out.clone());
        out
    }

    /// What it costs a line to go between two cells with different boxes around them: the borders between.
    fn borders(&self, mut a: i32, mut b: i32, allowed: &[bool]) -> f64 {
        let mut cost = 0.0;
        let mut guard = 0;
        while a != b && guard < 4096 {
            guard += 1;
            if a >= 0 && (b < 0 || self.box_depth[a as usize] >= self.box_depth[b as usize]) {
                if !allowed[a as usize] {
                    cost += self.p.thru;
                }
                a = self.box_parent[a as usize];
            } else {
                if !allowed[b as usize] {
                    cost += self.p.thru;
                }
                b = self.box_parent[b as usize];
            }
        }
        cost
    }

    /// A line's rough way, cell by cell: marks the cells around it in `in_way`. False: none found.
    fn rough_way(&mut self, from: usize, to: usize, allowed: &[bool]) -> bool {
        let cs = self.p.cell;
        let (x0, y0) = (self.x[0], self.y[0]);
        let (ccx, ccy) = (self.ccx as i64, self.ccy as i64);
        let cells = |r: (f64, f64, f64, f64, f64, f64), c: f64| {
            (
                (((r.0 - c - x0) / cs).floor() as i64).max(0),
                (((r.1 + c - x0) / cs).floor() as i64).min(ccx - 1),
                (((r.2 - c - y0) / cs).floor() as i64).max(0),
                (((r.3 + c - y0) / cs).floor() as i64).min(ccy - 1),
            )
        };
        let (sp1, sp2, sq1, sq2) = cells(self.rect(from), self.p.clear * self.scale(from));
        let (tp1, tp2, tq1, tq2) = cells(self.rect(to), self.p.clear * self.scale(to));
        let direct =
            ((tp1 - sp2).max(sp1 - tp2).max(0) + (tq1 - sq2).max(sq1 - tq2).max(0)) as f64 * cs;
        let budget = direct * (1.0 + self.p.detour.1) + self.p.detour.0 + 2.0 * cs;
        self.c_gen += 1;
        let g = self.c_gen;
        let bend = self.p.bend;
        let left = |p: i64, q: i64, o: usize| {
            let a = if p < tp1 {
                tp1 - p
            } else if p > tp2 {
                p - tp2
            } else {
                0
            };
            let b = if q < tq1 {
                tq1 - q
            } else if q > tq2 {
                q - tq2
            } else {
                0
            };
            (a + b) as f64 * cs
                + if a > 0 && b > 0 {
                    bend
                } else if a > 0 {
                    if o == 1 { bend } else { 0.0 }
                } else if b > 0 {
                    if o == 1 { 0.0 } else { bend }
                } else {
                    0.0
                }
        };
        let mut heap = Heap {
            f: Vec::new(),
            s: Vec::new(),
        };
        for q in sq1..=sq2 {
            for p in sp1..=sp2 {
                for o in 0..2 {
                    let s = ((q * ccx + p) * 2) as usize + o;
                    self.c_stamp[s] = g;
                    self.c_g[s] = 0.0;
                    self.c_from[s] = -1;
                    heap.push(left(p, q, o), s);
                }
            }
        }
        let mut end: i64 = -1;
        while let Some(s) = heap.pop() {
            if self.c_done[s] == g {
                continue;
            }
            self.c_done[s] = g;
            let c = (s >> 1) as i64;
            let o = s & 1;
            let p = c % ccx;
            let q = c / ccx;
            if p >= tp1 && p <= tp2 && q >= tq1 && q <= tq2 {
                end = s as i64;
                break;
            }
            for d in 0..4 {
                let (p2, q2) = match d {
                    0 => (p + 1, q),
                    1 => (p - 1, q),
                    2 => (p, q + 1),
                    _ => (p, q - 1),
                };
                if p2 < 0 || p2 >= ccx || q2 < 0 || q2 >= ccy {
                    continue;
                }
                let c2 = q2 * ccx + p2;
                // the border between the two cells is kept at the left or upper one
                let brd = c.min(c2) as usize;
                let (cap, used) = if d < 2 {
                    (self.cap_r[brd], self.use_r[brd])
                } else {
                    (self.cap_d[brd], self.use_d[brd])
                };
                if cap == 0 {
                    continue;
                }
                let o2 = if d < 2 { 0 } else { 1 };
                let dist = |v: i64, lo: i64, hi: i64| {
                    if v < lo {
                        lo - v
                    } else if v > hi {
                        v - hi
                    } else {
                        0
                    }
                };
                let far = (dist(p2, sp1, sp2)
                    + dist(q2, sq1, sq2)
                    + dist(p2, tp1, tp2)
                    + dist(q2, tq1, tq2)) as f64
                    * cs;
                if far > budget {
                    continue;
                }
                let (cu, cu2) = (c as usize, c2 as usize);
                // The lines that run the other way through the two cells are crossed; a full border costs like a line run on.
                let mut cost = self.c_g[s] as f64
                    + cs
                    + self.p.cross
                        * 0.5
                        * if o2 == 1 {
                            self.flow_h[cu] as f64 + self.flow_h[cu2] as f64
                        } else {
                            self.flow_v[cu] as f64 + self.flow_v[cu2] as f64
                        };
                if used >= cap as i32 {
                    cost += self.p.over + self.p.over_px * cs;
                } else if used > 0 {
                    cost += self.p.near * cs * (used as f64 / cap as f64);
                }
                if o2 != o {
                    cost += bend;
                }
                if self.cell_box[cu] != self.cell_box[cu2] {
                    cost += self.borders(self.cell_box[cu], self.cell_box[cu2], allowed);
                }
                let s2 = cu2 * 2 + o2;
                if self.c_stamp[s2] == g && self.c_g[s2] as f64 <= cost {
                    continue;
                }
                self.c_stamp[s2] = g;
                self.c_g[s2] = cost as f32;
                self.c_from[s2] = s as i64;
                heap.push(cost + left(p2, q2, o2), s2);
            }
        }
        if end < 0 {
            return false;
        }
        let mark = |grid: &mut Self, p: i64, q: i64, a: i64| {
            for v in (q - a).max(0)..=(q + a).min(ccy - 1) {
                for u in (p - a).max(0)..=(p + a).min(ccx - 1) {
                    grid.in_way[(v * ccx + u) as usize] = g;
                }
            }
        };
        let mut s = end;
        while s >= 0 {
            let c = s >> 1;
            mark(self, c % ccx, c / ccx, self.p.around);
            s = self.c_from[s as usize];
        }
        for q in sq1..=sq2 {
            for p in sp1..=sp2 {
                mark(self, p, q, 1);
            }
        }
        for q in tq1..=tq2 {
            for p in tp1..=tp2 {
                mark(self, p, q, 1);
            }
        }
        true
    }

    fn q_push(&mut self, f: f64, s: usize) {
        let b = (f.max(0.0) as usize).min(self.nb - 1);
        self.q_state.push(s);
        self.q_next.push(self.b_head[b]);
        self.b_head[b] = self.q_state.len() as i64 - 1;
        if b < self.q_cur {
            self.q_cur = b;
        }
        if b as i64 > self.q_max {
            self.q_max = b as i64;
        }
    }

    /// The cheapest way for a line, given the others. `within`: 0 the cells its rough way marked,
    /// 1 no further around than the detour allows, 2 anywhere.
    fn search(&mut self, from: usize, to: usize, allowed: &[bool], within: u8) -> Option<GridWay> {
        let src = self.terminals(from);
        let dst = self.terminals(to);
        if src.is_empty() || dst.is_empty() {
            return None;
        }
        let tr = self.rect(to);
        let sr = self.rect(from);
        let tc = self.p.clear * self.scale(to);
        let sc = self.p.clear * self.scale(from);
        let (gx1, gx2, gy1, gy2) = (tr.0 - tc, tr.1 + tc, tr.2 - tc, tr.3 + tc);
        let (ax1, ax2, ay1, ay2) = (sr.0 - sc, sr.1 + sc, sr.2 - sc, sr.3 + sc);
        let dist = |v: f64, lo: f64, hi: f64| {
            if v < lo {
                lo - v
            } else if v > hi {
                v - hi
            } else {
                0.0
            }
        };
        for i in 0..self.nx {
            let x = self.x[i];
            self.hx[i] = dist(x, gx1, gx2) as f32;
            self.sx[i] = dist(x, ax1, ax2) as f32;
        }
        for j in 0..self.ny {
            let y = self.y[j];
            self.hy[j] = dist(y, gy1, gy2) as f32;
            self.sy[j] = dist(y, ay1, ay2) as f32;
        }
        let direct = (gx1 - ax2).max(ax1 - gx2).max(0.0) + (gy1 - ay2).max(ay1 - gy2).max(0.0);
        let budget = if within == 1 {
            direct * (1.0 + self.p.detour.1) + self.p.detour.0
        } else {
            f64::INFINITY
        };
        let cells = within == 0;
        self.gen_ += 1;
        let g = self.gen_;
        if self.q_max >= 0 {
            for b in self.q_cur..=self.q_max as usize {
                self.b_head[b] = -1;
            }
        }
        self.q_state.clear();
        self.q_next.clear();
        self.q_cur = self.nb;
        self.q_max = -1;
        for (k, t) in dst.iter().enumerate() {
            self.goal_at[t.n] = k;
            self.goal_gen[t.n] = g;
        }
        let bend = self.p.bend;
        let nx = self.nx;
        for t in &src {
            let s = t.n * 2 + t.o;
            let c =
                t.stub + self.p.off_centre * t.off + self.p.share_end * self.term_use[t.n] as f64;
            if self.stamp[s] == g && c >= self.g_cost[s] as f64 {
                continue;
            }
            self.stamp[s] = g;
            self.g_cost[s] = c as f32;
            self.from[s] = -1;
            let left = self.left(t.n % nx, t.n / nx, t.o, bend);
            self.q_push(c + left, s);
        }
        let mut best = f64::INFINITY;
        let mut best_s: i64 = -1;
        let mut best_t: Option<TrackEnd> = None;
        loop {
            while self.q_max >= 0 && self.q_cur as i64 <= self.q_max && self.b_head[self.q_cur] < 0
            {
                self.q_cur += 1;
            }
            if self.q_max < 0 || self.q_cur as i64 > self.q_max || self.q_cur as f64 >= best {
                break;
            }
            let q = self.b_head[self.q_cur] as usize;
            self.b_head[self.q_cur] = self.q_next[q];
            let s = self.q_state[q];
            if self.done[s] == g {
                continue;
            }
            self.done[s] = g;
            let n = s >> 1;
            let o = s & 1;
            let g_here = self.g_cost[s] as f64;
            if self.goal_gen[n] == g {
                // Into the part: straight in if the line already lies that way.
                let t = dst[self.goal_at[n]];
                let c = g_here
                    + if o == t.o { 0.0 } else { bend }
                    + t.stub
                    + self.p.off_centre * t.off
                    + self.p.share_end * self.term_use[n] as f64;
                if c < best {
                    best = c;
                    best_s = s as i64;
                    best_t = Some(t);
                }
            }
            let i = n % nx;
            let j = n / nx;
            // Four ways on: right, left (along the row), down, up (along the column).
            for d in 0..4 {
                let (e, n2, i2, j2) = match d {
                    0 => (n, n + 1, i + 1, j),
                    1 => {
                        if i == 0 {
                            continue;
                        }
                        (n - 1, n - 1, i - 1, j)
                    }
                    2 => (n, n + nx, i, j + 1),
                    _ => {
                        if j == 0 {
                            continue;
                        }
                        (n - nx, n - nx, i, j - 1)
                    }
                };
                let o2 = if d < 2 { 0 } else { 1 };
                let mut c = if o2 == 1 { self.v_c[e] } else { self.h_c[e] } as f64;
                if c < 0.0 {
                    continue;
                }
                if cells {
                    if self.in_way[self.cy[j2] * self.ccx + self.cx[i2]] != self.c_gen {
                        continue;
                    }
                } else if self.sx[i2] as f64
                    + self.sy[j2] as f64
                    + self.hx[i2] as f64
                    + self.hy[j2] as f64
                    > budget
                {
                    continue;
                }
                let len = if o2 == 1 {
                    (self.y[j2] - self.y[j]).abs()
                } else {
                    (self.x[i2] - self.x[i]).abs()
                };
                c += g_here;
                let bx = if o2 == 1 {
                    self.v_box[e]
                } else {
                    self.h_box[e]
                };
                if bx >= 0 && !allowed[bx as usize] {
                    c += self.p.thru;
                }
                let used = if o2 == 1 {
                    self.v_use[e]
                } else {
                    self.h_use[e]
                };
                if used != 0 {
                    c += used as f64 * (self.p.over + self.p.over_px * len);
                }
                if o2 == 1 {
                    let (st, list, tight) = &self.near_x;
                    for k in st[i]..st[i + 1] {
                        let beside = self.v_use[e - i + list[k]];
                        if beside != 0 {
                            c += beside as f64
                                * if tight[k] {
                                    self.p.over + self.p.over_px * len
                                } else {
                                    self.p.near * len
                                };
                        }
                    }
                } else {
                    let (st, list, tight) = &self.near_y;
                    for k in st[j]..st[j + 1] {
                        let beside = self.h_use[list[k] * nx + (e % nx)];
                        if beside != 0 {
                            c += beside as f64
                                * if tight[k] {
                                    self.p.over + self.p.over_px * len
                                } else {
                                    self.p.near * len
                                };
                        }
                    }
                }
                // A bend where another line bends would touch it; straight on crosses the lines that go the other way here.
                if o2 != o {
                    c += bend + self.p.cross * self.corner[n] as f64;
                } else {
                    c += self.p.cross
                        * if o2 == 1 {
                            self.h_thru[n] as f64
                        } else {
                            self.v_thru[n] as f64
                        };
                }
                let s2 = n2 * 2 + o2;
                if self.stamp[s2] == g && self.g_cost[s2] as f64 <= c {
                    continue;
                }
                self.stamp[s2] = g;
                self.g_cost[s2] = c as f32;
                self.from[s2] = s as i64;
                let left = self.left(i2, j2, o2, bend);
                self.q_push(c + left, s2);
            }
        }
        if best_s < 0 {
            return None;
        }
        let mut way: Vec<usize> = Vec::new();
        let mut s0 = best_s;
        let mut s = best_s;
        while s >= 0 {
            // a node reached along its row and left along its column is listed twice: once is enough
            if way.last() != Some(&((s >> 1) as usize)) {
                way.push((s >> 1) as usize);
            }
            s0 = s;
            s = self.from[s as usize];
        }
        way.reverse();
        let st = *src
            .iter()
            .find(|t| t.n == (s0 >> 1) as usize && t.o == (s0 & 1) as usize)?;
        Some(GridWay {
            nodes: way,
            st,
            tt: best_t?,
        })
    }

    /// What is left at least from a node: the way to the target's room, and the bend that takes.
    fn left(&self, i: usize, j: usize, o: usize, bend: f64) -> f64 {
        let a = self.hx[i] as f64;
        let b = self.hy[j] as f64;
        a + b
            + if a > 0.0 && b > 0.0 {
                bend
            } else if a > 0.0 {
                if o == 1 { bend } else { 0.0 }
            } else if b > 0.0 {
                if o == 1 { 0.0 } else { bend }
            } else {
                0.0
            }
    }

    /// A line already drawn, as its corner points, counted as using the tracks it runs along.
    fn occupy_points(&mut self, pts: &[(f64, f64)]) {
        let nx = self.nx;
        for w in pts.windows(2) {
            let (ia, ja) = (nearest(&self.x, w[0].0), nearest(&self.y, w[0].1));
            let (ib, jb) = (nearest(&self.x, w[1].0), nearest(&self.y, w[1].1));
            if ja == jb {
                for i in ia.min(ib)..ia.max(ib) {
                    self.h_use[ja * nx + i] += 1;
                }
            } else if ia == ib {
                for j in ja.min(jb)..ja.max(jb) {
                    self.v_use[j * nx + ia] += 1;
                }
            }
        }
    }

    fn occupy(&mut self, way: &GridWay, sign: i32) {
        let nd = &way.nodes;
        let nx = self.nx;
        self.term_use[nd[0]] += sign;
        self.term_use[nd[nd.len() - 1]] += sign;
        for k in 0..nd.len() {
            let n = nd[k];
            if k + 1 < nd.len() {
                let flat = nd[k + 1].abs_diff(n) == 1;
                let e = n.min(nd[k + 1]); // an edge is kept at its left or upper node
                if flat {
                    self.h_use[e] += sign;
                } else {
                    self.v_use[e] += sign;
                }
                if self.rough {
                    let i = e % nx;
                    let j = e / nx;
                    let c = self.cy[j] * self.ccx + self.cx[i];
                    if flat {
                        let delta = sign as f64 * (self.x[i + 1] - self.x[i]) / self.p.cell;
                        self.flow_h[c] = (self.flow_h[c] as f64 + delta) as f32;
                        if self.cx[i + 1] != self.cx[i] {
                            self.use_r[c] += sign;
                        }
                    } else {
                        let delta = sign as f64 * (self.y[j + 1] - self.y[j]) / self.p.cell;
                        self.flow_v[c] = (self.flow_v[c] as f64 + delta) as f32;
                        if self.cy[j + 1] != self.cy[j] {
                            self.use_d[c] += sign;
                        }
                    }
                }
            }
            let prev_flat = if k > 0 {
                nd[k - 1].abs_diff(n) == 1
            } else {
                way.st.o == 0
            };
            let next_flat = if k + 1 < nd.len() {
                nd[k + 1].abs_diff(n) == 1
            } else {
                way.tt.o == 0
            };
            if prev_flat && next_flat {
                self.h_thru[n] += sign;
            } else if !prev_flat && !next_flat {
                self.v_thru[n] += sign;
            } else {
                self.corner[n] += sign;
            }
        }
    }
}

/// A right-angled line with one corner, from the side of `a` that faces `b` to the side of `b` that faces `a`.
fn corner(a: &Rect, b: &Rect) -> Vec<(f64, f64)> {
    let facing = |a: &Rect, b: &Rect| {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        if dx.abs() / (a.w + b.w).max(1.0) > dy.abs() / (a.h + b.h).max(1.0) {
            if dx > 0.0 { 1 } else { 3 }
        } else if dy > 0.0 {
            2
        } else {
            0
        }
    };
    let at = |p: &Rect, side: u8| match side {
        0 => (p.x, p.y - p.h / 2.0),
        2 => (p.x, p.y + p.h / 2.0),
        3 => (p.x - p.w / 2.0, p.y),
        _ => (p.x + p.w / 2.0, p.y),
    };
    let s = at(a, facing(a, b));
    let t = at(b, facing(b, a));
    let vertical = matches!(facing(a, b), 0 | 2);
    vec![s, if vertical { (s.0, t.1) } else { (t.0, s.1) }, t]
}

/// Draw the lines of a placed picture: each line the cheapest way, given all the others.
pub(crate) fn grid_lines(input: &Input, placed: &Placement, p: Prices) -> Routes {
    grid_lines_around(input, placed, p, &[])
}

/// The track nearest `v`.
fn nearest(arr: &[f64], v: f64) -> usize {
    let i = arr.partition_point(|&a| a < v);
    match (i.checked_sub(1), arr.get(i)) {
        (Some(k), Some(&a)) if (v - arr[k]).abs() <= (a - v).abs() => k,
        (Some(k), None) => k,
        _ => i.min(arr.len().saturating_sub(1)),
    }
}

/***
 * As `grid_lines`, with lines already drawn that stay where they are (docair-702.40): their
 * tracks are among the router's tracks and count as used before the new lines are routed, so the new lines
 * keep off them as they keep off each other; the staying lines themselves are not moved.
 ***/
pub(crate) fn grid_lines_around(
    input: &Input,
    placed: &Placement,
    p: Prices,
    stay: &[Vec<(f64, f64)>],
) -> Routes {
    let n_parts = input.parts.len();
    let mut is_box = vec![false; n_parts];
    for part in &input.parts {
        if let Some(q) = part.parent.filter(|&q| q < n_parts) {
            is_box[q] = true;
        }
    }
    let valid: Vec<usize> = (0..input.lines.len())
        .filter(|&k| input.lines[k].from < n_parts && input.lines[k].to < n_parts)
        .collect();
    let mut out: Vec<Vec<(f64, f64)>> = vec![Vec::new(); input.lines.len()];
    if valid.is_empty() {
        return Routes {
            lines: out,
            price: 0.0,
            none: 0,
        };
    }
    let mut ends = vec![false; n_parts];
    for &k in &valid {
        ends[input.lines[k].from] = true;
        ends[input.lines[k].to] = true;
    }
    let rect = |i: usize| {
        let r = placed.parts[i];
        (
            r.x - r.w / 2.0,
            r.x + r.w / 2.0,
            r.y - r.h / 2.0,
            r.y + r.h / 2.0,
            r.x,
            r.y,
        )
    };
    let scale = |i: usize| placed.scale.get(i).copied().unwrap_or(1.0);
    let parts: Vec<(usize, PartRoom)> = (0..n_parts)
        .filter(|&i| !is_box[i])
        .map(|i| {
            let (l, r, t, b, _, _) = rect(i);
            (
                i,
                PartRoom {
                    l,
                    r,
                    t,
                    b,
                    c: p.clear * scale(i),
                },
            )
        })
        .collect();
    let boxes: Vec<BoxRoom> = (0..n_parts)
        .filter(|&i| is_box[i])
        .map(|i| {
            let (l, r, t, b, x, _) = rect(i);
            BoxRoom {
                part: i,
                l,
                r,
                t,
                b,
                x,
                sc: scale(i),
                name_w: input.parts[i].name_width,
            }
        })
        .collect();
    let mut box_index = vec![-1i32; n_parts];
    for (k, b) in boxes.iter().enumerate() {
        box_index[b.part] = k as i32;
    }
    // ── Tracks ──
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for r in &placed.parts {
        min_x = min_x.min(r.x - r.w / 2.0);
        min_y = min_y.min(r.y - r.h / 2.0);
        max_x = max_x.max(r.x + r.w / 2.0);
        max_y = max_y.max(r.y + r.h / 2.0);
    }
    let (mut hard_x, mut hard_y, mut soft_x, mut soft_y) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (i, pr) in &parts {
        hard_x.extend([pr.l - pr.c, pr.r + pr.c]);
        hard_y.extend([pr.t - pr.c, pr.b + pr.c]);
        if ends[*i] {
            soft_x.push(placed.parts[*i].x);
            soft_y.push(placed.parts[*i].y);
        }
    }
    for b in &boxes {
        if !ends[b.part] {
            continue;
        }
        let c = p.clear * b.sc;
        hard_x.extend([b.l - c, b.r + c]);
        hard_y.extend([b.t - c, b.b + c]);
        soft_x.push(placed.parts[b.part].x);
        soft_y.push(placed.parts[b.part].y);
    }
    // The staying lines' tracks are tracks too, so their use can be counted (docair-702.40).
    for &(px, py) in stay.iter().flatten() {
        soft_x.push(px);
        soft_y.push(py);
    }
    let mut pitch = p.pitch;
    let mut x = tracks(&hard_x, &soft_x, min_x, max_x, pitch, p.merge);
    let mut y = tracks(&hard_y, &soft_y, min_y, max_y, pitch, p.merge);
    // A picture too big for 8 px tracks gets wider ones.
    while x.len() * y.len() > p.most {
        pitch *= 1.5;
        x = tracks(&hard_x, &soft_x, min_x, max_x, pitch, p.merge);
        y = tracks(&hard_y, &soft_y, min_y, max_y, pitch, p.merge);
    }
    let (nx, ny) = (x.len(), y.len());
    let n = nx * ny;
    // ── An edge's own price ──
    let mut h_c = vec![0.0f32; n];
    let mut v_c = vec![0.0f32; n];
    for j in 0..ny {
        for i in 0..nx {
            h_c[j * nx + i] = if i + 1 < nx {
                (x[i + 1] - x[i]) as f32
            } else {
                -1.0
            };
            v_c[j * nx + i] = if j + 1 < ny {
                (y[j + 1] - y[j]) as f32
            } else {
                -1.0
            };
        }
    }
    let mut h_soft = vec![0.0f32; n];
    let mut v_soft = vec![0.0f32; n];
    let mut blocked = vec![false; n];
    let rows = |i1: isize, i2: isize| ((i1 - 1).max(0), i2.min(nx as isize - 2));
    let cols = |j1: isize, j2: isize| ((j1 - 1).max(0), j2.min(ny as isize - 2));
    let at = |j: isize, i: isize| (j as usize) * nx + i as usize;
    for (_, pr) in &parts {
        let (i1, i2) = (
            lower(&x, pr.l - pr.c + 0.5),
            lower(&x, pr.r + pr.c - 0.5) - 1,
        );
        let (j1, j2) = (
            lower(&y, pr.t - pr.c + 0.5),
            lower(&y, pr.b + pr.c - 0.5) - 1,
        );
        let (ia, ib) = rows(i1, i2);
        let (ja, jb) = cols(j1, j2);
        for j in j1..=j2 {
            for i in i1..=i2 {
                blocked[at(j, i)] = true;
            }
            for i in ia..=ib {
                h_c[at(j, i)] = -1.0;
            }
        }
        for i in i1..=i2 {
            for j in ja..=jb {
                v_c[at(j, i)] = -1.0;
            }
        }
        // The tracks right beside the part cost a little more: a line that only passes keeps its distance.
        for j in [track_at(&y, pr.t - pr.c), track_at(&y, pr.b + pr.c)] {
            if j >= 0 {
                for i in ia..=ib {
                    h_soft[at(j, i)] = h_soft[at(j, i)].max(p.hug_part as f32);
                }
            }
        }
        for i in [track_at(&x, pr.l - pr.c), track_at(&x, pr.r + pr.c)] {
            if i >= 0 {
                for j in ja..=jb {
                    v_soft[at(j, i)] = v_soft[at(j, i)].max(p.hug_part as f32);
                }
            }
        }
    }
    // ── Boxes: whose border an edge crosses; the tracks along a border; a box's name ──
    let mut h_box = vec![-1i32; n];
    let mut v_box = vec![-1i32; n];
    for (bi, b) in boxes.iter().enumerate() {
        let (i1, i2) = (lower(&x, b.l), lower(&x, b.r) - 1);
        let (j1, j2) = (lower(&y, b.t), lower(&y, b.b) - 1);
        let (ia, ib) = rows(i1, i2);
        let (ja, jb) = cols(j1, j2);
        for j in j1..=j2 {
            if i1 > 0 {
                h_box[at(j, i1 - 1)] = bi as i32;
            }
            if i2 >= 0 && i2 < nx as isize - 1 {
                h_box[at(j, i2)] = bi as i32;
            }
        }
        for i in i1..=i2 {
            if j1 > 0 {
                v_box[at(j1 - 1, i)] = bi as i32;
            }
            if j2 >= 0 && j2 < ny as isize - 1 {
                v_box[at(j2, i)] = bi as i32;
            }
        }
        for bx in [b.l, b.r] {
            let mut i = lower(&x, bx - 3.0);
            while (i as usize) < nx && x[i as usize] <= bx + 3.0 {
                for j in ja..=jb {
                    v_soft[at(j, i)] = v_soft[at(j, i)].max(p.hug_box as f32);
                }
                i += 1;
            }
        }
        for by in [b.t, b.b] {
            let mut j = lower(&y, by - 3.0);
            while (j as usize) < ny && y[j as usize] <= by + 3.0 {
                for i in ia..=ib {
                    h_soft[at(j, i)] = h_soft[at(j, i)].max(p.hug_box as f32);
                }
                j += 1;
            }
        }
        if b.name_w > 0.0 {
            // The name stands in the band at the box's top, in the middle.
            let a1 = lower(&x, b.x - b.name_w * b.sc / 2.0 - 2.0);
            let a2 = lower(&x, b.x + b.name_w * b.sc / 2.0 + 2.0) - 1;
            let c1 = lower(&y, b.t + 3.0 * b.sc);
            let c2 = lower(&y, b.t + 21.0 * b.sc) - 1;
            for j in c1..=c2 {
                for i in (a1 - 1).max(0)..=a2.min(nx as isize - 2) {
                    h_soft[at(j, i)] += p.name as f32;
                }
            }
            for i in a1..=a2 {
                for j in (c1 - 1).max(0)..=c2.min(ny as isize - 2) {
                    v_soft[at(j, i)] += p.name as f32;
                }
            }
        }
    }
    for k in 0..n {
        if h_c[k] > 0.0 {
            h_c[k] = (h_c[k] as f64 * (1.0 + h_soft[k] as f64)) as f32;
        }
        if v_c[k] > 0.0 {
            v_c[k] = (v_c[k] as f64 * (1.0 + v_soft[k] as f64)) as f32;
        }
    }
    let near_x = near_of(&x, p.apart, p.merge);
    let near_y = near_of(&y, p.apart, p.merge);
    // ── The rough picture: cells of `cell` px ──
    let cs = p.cell;
    let rough = n >= p.cells_from;
    let cx: Vec<usize> = x
        .iter()
        .map(|&v| ((v - x[0]) / cs).floor() as usize)
        .collect();
    let cy: Vec<usize> = y
        .iter()
        .map(|&v| ((v - y[0]) / cs).floor() as usize)
        .collect();
    let (ccx, ccy) = (cx[nx - 1] + 1, cy[ny - 1] + 1);
    let cn = ccx * ccy;
    let mut cap_r = vec![0u32; cn];
    let mut cap_d = vec![0u32; cn];
    let mut cell_box = vec![-1i32; cn];
    if rough {
        for j in 0..ny {
            for i in 0..nx {
                let k = j * nx + i;
                let c = cy[j] * ccx + cx[i];
                if i + 1 < nx && cx[i + 1] != cx[i] && h_c[k] >= 0.0 {
                    cap_r[c] += 1;
                }
                if j + 1 < ny && cy[j + 1] != cy[j] && v_c[k] >= 0.0 {
                    cap_d[c] += 1;
                }
            }
        }
        // big boxes first: the smaller ones inside them overwrite
        let mut by_area: Vec<usize> = (0..boxes.len()).collect();
        by_area.sort_by(|&u, &v| {
            let au = (boxes[u].r - boxes[u].l) * (boxes[u].b - boxes[u].t);
            let av = (boxes[v].r - boxes[v].l) * (boxes[v].b - boxes[v].t);
            av.total_cmp(&au)
        });
        for bi in by_area {
            let b = &boxes[bi];
            let a1 = ((b.l - x[0]) / cs - 0.5).ceil() as i64;
            let a2 = ((b.r - x[0]) / cs - 0.5).floor() as i64;
            let c1 = ((b.t - y[0]) / cs - 0.5).ceil() as i64;
            let c2 = ((b.b - y[0]) / cs - 0.5).floor() as i64;
            for q in c1.max(0)..=c2.min(ccy as i64 - 1) {
                for pp in a1.max(0)..=a2.min(ccx as i64 - 1) {
                    cell_box[q as usize * ccx + pp as usize] = bi as i32;
                }
            }
        }
    }
    let box_parent: Vec<i32> = boxes
        .iter()
        .map(|b| {
            input.parts[b.part]
                .parent
                .filter(|&q| q < n_parts)
                .map_or(-1, |q| box_index[q])
        })
        .collect();
    let box_depth: Vec<usize> = (0..boxes.len())
        .map(|i| {
            let mut d = 0;
            let mut c = box_parent[i];
            while c >= 0 && d <= boxes.len() {
                d += 1;
                c = box_parent[c as usize];
            }
            d
        })
        .collect();
    let nb = ((max_x - min_x + max_y - min_y) * 4.0 + 40000.0).ceil() as usize;
    let mut grid = Grid {
        placed,
        p,
        nx,
        ny,
        h_c,
        v_c,
        blocked,
        h_box,
        v_box,
        near_x,
        near_y,
        terms: vec![None; n_parts],
        rough,
        cx,
        cy,
        ccx,
        ccy,
        cap_r,
        cap_d,
        use_r: vec![0; cn],
        use_d: vec![0; cn],
        flow_h: vec![0.0; cn],
        flow_v: vec![0.0; cn],
        cell_box,
        box_parent,
        box_depth,
        h_use: vec![0; n],
        v_use: vec![0; n],
        h_thru: vec![0; n],
        v_thru: vec![0; n],
        corner: vec![0; n],
        term_use: vec![0; n],
        c_g: vec![0.0; cn * 2],
        c_stamp: vec![0; cn * 2],
        c_done: vec![0; cn * 2],
        c_from: vec![-1; cn * 2],
        in_way: vec![0; cn],
        c_gen: 0,
        g_cost: vec![0.0; n * 2],
        stamp: vec![0; n * 2],
        done: vec![0; n * 2],
        from: vec![-1; n * 2],
        goal_at: vec![0; n],
        goal_gen: vec![0; n],
        hx: vec![0.0; nx],
        hy: vec![0.0; ny],
        sx: vec![0.0; nx],
        sy: vec![0.0; ny],
        nb,
        b_head: vec![-1; nb],
        q_next: Vec::new(),
        q_state: Vec::new(),
        q_cur: nb,
        q_max: -1,
        gen_: 0,
        x,
        y,
    };
    // The boxes around either end: their borders are the line's to cross.
    let allowed: Vec<Vec<bool>> = valid
        .iter()
        .map(|&k| {
            let l = &input.lines[k];
            let mut a = vec![false; boxes.len()];
            for id in [l.from, l.to] {
                let mut c = Some(id);
                let mut guard = 0;
                while let Some(v) = c {
                    if guard > n_parts {
                        break;
                    }
                    guard += 1;
                    if box_index[v] >= 0 {
                        a[box_index[v] as usize] = true;
                    }
                    c = input.parts[v].parent.filter(|&q| q < n_parts);
                }
            }
            a
        })
        .collect();
    // The lines that stay use their tracks: the new ones pay to run along them (docair-702.40).
    for pts in stay {
        grid.occupy_points(pts);
    }
    // Short lines first: they have the fewest ways to choose from.
    let span = |k: usize| {
        let l = &input.lines[k];
        let (a, b) = (placed.parts[l.from], placed.parts[l.to]);
        (a.x - b.x).abs() + (a.y - b.y).abs()
    };
    let mut order: Vec<usize> = (0..valid.len()).collect();
    order.sort_by(|&u, &v| span(valid[u]).total_cmp(&span(valid[v])));
    let mut ways: Vec<Option<GridWay>> = vec![None; valid.len()];
    for _ in 0..p.rounds {
        let mut changed = 0;
        for &li in &order {
            let l = &input.lines[valid[li]];
            let old = ways[li].take();
            if let Some(w) = &old {
                grid.occupy(w, -1);
            }
            let found = (if grid.rough && grid.rough_way(l.from, l.to, &allowed[li]) {
                grid.search(l.from, l.to, &allowed[li], 0)
            } else {
                None
            })
            .or_else(|| grid.search(l.from, l.to, &allowed[li], 1))
            .or_else(|| grid.search(l.from, l.to, &allowed[li], 2));
            match found {
                None => {
                    if let Some(w) = &old {
                        grid.occupy(w, 1);
                    }
                    ways[li] = old;
                }
                Some(w) => {
                    if old.as_ref().is_none_or(|o| o.nodes != w.nodes) {
                        changed += 1;
                    }
                    grid.occupy(&w, 1);
                    ways[li] = Some(w);
                }
            }
        }
        if changed == 0 {
            break;
        }
    }
    // The routes, and the picture's price.
    let mut price = 0.0;
    let mut none = 0;
    for (li, &k) in valid.iter().enumerate() {
        let l = &input.lines[k];
        let Some(w) = &ways[li] else {
            none += 1;
            out[k] = corner(&placed.parts[l.from], &placed.parts[l.to]);
            price += 5000.0;
            continue;
        };
        let mut pts: Vec<(f64, f64)> = vec![(w.st.bx, w.st.by)];
        let mut add = |q: (f64, f64)| {
            let k = pts.len();
            // points on a straight run are left out
            if k >= 2
                && (((pts[k - 2].1 - pts[k - 1].1).abs() < 0.01
                    && (pts[k - 1].1 - q.1).abs() < 0.01)
                    || ((pts[k - 2].0 - pts[k - 1].0).abs() < 0.01
                        && (pts[k - 1].0 - q.0).abs() < 0.01))
            {
                pts[k - 1] = q;
            } else {
                pts.push(q);
            }
        };
        for &nd in &w.nodes {
            add((grid.x[nd % nx], grid.y[nd / nx]));
        }
        add((w.tt.bx, w.tt.by));
        let len: f64 = pts
            .windows(2)
            .map(|s| (s[1].0 - s[0].0).abs() + (s[1].1 - s[0].1).abs())
            .sum();
        price += len + p.bend * (pts.len() as f64 - 2.0);
        for s in w.nodes.windows(2) {
            let e = s[0].min(s[1]);
            let bx = if s[1].abs_diff(s[0]) == 1 {
                grid.h_box[e]
            } else {
                grid.v_box[e]
            };
            if bx >= 0 && !allowed[li][bx as usize] {
                price += p.thru;
            }
        }
        out[k] = pts;
    }
    for k in 0..n {
        price += p.cross * grid.h_thru[k] as f64 * grid.v_thru[k] as f64;
        if grid.h_use[k] > 1 {
            price +=
                (grid.h_use[k] - 1) as f64 * (p.over + p.over_px * (grid.h_c[k].max(0.0) as f64));
        }
        if grid.v_use[k] > 1 {
            price +=
                (grid.v_use[k] - 1) as f64 * (p.over + p.over_px * (grid.v_c[k].max(0.0) as f64));
        }
    }
    Routes {
        lines: out,
        price,
        none,
    }
}

#[cfg(test)]
mod tests {
    use crate::{Input, Line, Options, Part, lay_out};

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

    fn line(from: usize, to: usize) -> Line {
        Line {
            id: format!("{from}-{to}"),
            from,
            to,
            flip: false,
            ahead: 1,
            words: None,
        }
    }

    #[test]
    fn a_line_runs_at_right_angles_from_part_to_part() {
        let input = Input {
            parts: vec![
                part("box", None),
                part("a", Some(0)),
                part("b", Some(0)),
                part("c", None),
            ],
            lines: vec![line(1, 2), line(2, 3), line(1, 3)],
        };
        let laid = lay_out(&input, &Options::default());
        assert_eq!(laid.routes.none, 0);
        for (k, pts) in laid.routes.lines.iter().enumerate() {
            assert!(pts.len() >= 2, "line {k} has points: {pts:?}");
            for s in pts.windows(2) {
                let straight = (s[0].0 - s[1].0).abs() < 1e-6 || (s[0].1 - s[1].1).abs() < 1e-6;
                assert!(straight, "line {k} runs at right angles: {pts:?}");
            }
            let l = &input.lines[k];
            for (end, at) in [(l.from, pts[0]), (l.to, pts[pts.len() - 1])] {
                let r = laid.placement.parts[end];
                let on_border = ((at.0 - (r.x - r.w / 2.0)).abs() < 1e-6
                    || (at.0 - (r.x + r.w / 2.0)).abs() < 1e-6)
                    || ((at.1 - (r.y - r.h / 2.0)).abs() < 1e-6
                        || (at.1 - (r.y + r.h / 2.0)).abs() < 1e-6);
                assert!(
                    on_border,
                    "line {k} touches part {end} on its border: {at:?} {r:?}"
                );
            }
        }
        assert_eq!(
            laid,
            lay_out(&input, &Options::default()),
            "the same input, the same lines"
        );
    }
}
