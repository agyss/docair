//! A box's parts placed by the lines between them (docs/PLACEMENT.md §2): one at a time, the most
//! connected first, each into the free place beside the parts already placed where its lines to them
//! are shortest, with a price on making the box bigger; then each part is taken out and put back
//! where it is best. A line's length is measured between the parts it really ends at, deep inside
//! the box's parts. The placement is one candidate among those in layers (`place`, `lay_out`).

/// Between two parts, in px: room for the lines that run between them (docair-lj0.5).
pub(crate) const GAP: f64 = 96.0;

/// What a px of the box's width plus height costs, for each line of the part placed (docair-lj0.5).
const GROW: f64 = 1.0;

/// How often every part is taken out and put back where it is best (docair-lj0.5).
const ROUNDS: usize = 3;

/// How hard a line that leaves the box pulls its part its way: px of price per px the part goes
/// that way (docs/PLACEMENT.md §2, docair-lj0.7).
const PULL: f64 = 3.0;

/// A line that leaves the box: the part it leaves from (its index among the box's parts), where it
/// ends inside it (from its corner), and the way it went in the placement before, as a unit vector.
pub(crate) type Outward = (usize, (f64, f64), (f64, f64));

/// A line between two of a box's parts: the first part (its index among the box's parts), where
/// the line ends inside it (from its corner), the second part, and where it ends inside that one.
pub(crate) type LineEnds = (usize, (f64, f64), usize, (f64, f64));

#[derive(Clone, Copy)]
struct Tile {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// Whether two parts are closer than `GAP`.
fn too_close(a: &Tile, b: &Tile) -> bool {
    a.x < b.x + b.w + GAP - 1e-6
        && b.x < a.x + a.w + GAP - 1e-6
        && a.y < b.y + b.h + GAP - 1e-6
        && b.y < a.y + a.h + GAP - 1e-6
}

/// The width plus the height of what the placed parts cover.
fn extent<'a>(tiles: impl Iterator<Item = &'a Tile>) -> f64 {
    let (mut x1, mut y1) = (f64::INFINITY, f64::INFINITY);
    let (mut x2, mut y2) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for s in tiles {
        x1 = x1.min(s.x);
        y1 = y1.min(s.y);
        x2 = x2.max(s.x + s.w);
        y2 = y2.max(s.y + s.h);
    }
    if x1.is_finite() {
        (x2 - x1) + (y2 - y1)
    } else {
        0.0
    }
}

/// How long part `i`'s lines to the parts placed are, with `i` at `at`.
fn length(
    i: usize,
    at: &Tile,
    placed: &[Option<Tile>],
    lines_of: &[Vec<usize>],
    ends: &[LineEnds],
    pulls: &[Outward],
) -> f64 {
    let mut sum = 0.0;
    for &(c, inside, way) in pulls {
        if c == i {
            sum -= PULL * (way.0 * (at.x + inside.0) + way.1 * (at.y + inside.1));
        }
    }
    for &e in &lines_of[i] {
        let (a, in_a, b, in_b) = ends[e];
        let (mine, other, theirs) = if a == i {
            (in_a, b, in_b)
        } else {
            (in_b, a, in_a)
        };
        if let Some(o) = &placed[other] {
            sum += (at.x + mine.0 - o.x - theirs.0).abs() + (at.y + mine.1 - o.y - theirs.1).abs();
        }
    }
    sum
}

/// The best free place for part `i` beside the parts placed (`placed` without `i`), and its price.
fn best_free_place(
    i: usize,
    sizes: &[(f64, f64)],
    placed: &[Option<Tile>],
    lines_of: &[Vec<usize>],
    ends: &[LineEnds],
    pulls: &[Outward],
) -> (Tile, f64) {
    let (w, h) = sizes[i];
    let others: Vec<Tile> = placed.iter().flatten().copied().collect();
    if others.is_empty() {
        return (
            Tile {
                x: 0.0,
                y: 0.0,
                w,
                h,
            },
            0.0,
        );
    }
    let base = extent(others.iter());
    let grow = GROW * (1.0 + lines_of[i].len() as f64);
    let mut best: Option<(Tile, f64)> = None;
    for p in &others {
        let g = GAP;
        // Beside each placed part: right, below, left and above, at its start, middle or end.
        for (x, y) in [
            (p.x + p.w + g, p.y),
            (p.x + p.w + g, p.y + p.h - h),
            (p.x + p.w + g, p.y + (p.h - h) / 2.0),
            (p.x, p.y + p.h + g),
            (p.x + p.w - w, p.y + p.h + g),
            (p.x + (p.w - w) / 2.0, p.y + p.h + g),
            (p.x - g - w, p.y),
            (p.x - g - w, p.y + p.h - h),
            (p.x, p.y - g - h),
            (p.x + p.w - w, p.y - g - h),
        ] {
            let s = Tile { x, y, w, h };
            if others.iter().any(|o| too_close(&s, o)) {
                continue;
            }
            let price = length(i, &s, placed, lines_of, ends, pulls)
                + grow * (extent(others.iter().chain(std::iter::once(&s))) - base);
            if best.as_ref().is_none_or(|b| price < b.1) {
                best = Some((s, price));
            }
        }
    }
    // Some side of the part furthest right is always free.
    best.unwrap_or_else(|| {
        let right = others
            .iter()
            .map(|o| o.x + o.w)
            .fold(f64::NEG_INFINITY, f64::max);
        (
            Tile {
                x: right + GAP,
                y: 0.0,
                w,
                h,
            },
            f64::INFINITY,
        )
    })
}

/***
 * Place a box's parts by the lines between them (docs/PLACEMENT.md §2): the corner of each part,
 * in the box's own px, given each part's size, each line between two of them with where it ends
 * inside both, and each line that leaves the box with the way it went before (none in a first
 * placement). The same input gives the same placement.
 ***/
pub(crate) fn place_by_lines(
    sizes: &[(f64, f64)],
    ends: &[LineEnds],
    pulls: &[Outward],
) -> Vec<(f64, f64)> {
    let n = sizes.len();
    let mut lines_of: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut weight = vec![vec![0.0f64; n]; n];
    for (k, &(a, _, b, _)) in ends.iter().enumerate() {
        lines_of[a].push(k);
        lines_of[b].push(k);
        weight[a][b] += 1.0;
        weight[b][a] += 1.0;
    }
    let area = |i: usize| sizes[i].0 * sizes[i].1;
    let mut placed: Vec<Option<Tile>> = vec![None; n];
    let mut order: Vec<usize> = Vec::with_capacity(n);
    // The most connected first, then always the part with the most lines to those placed; ties go
    // to the bigger part, then to the one given first.
    while order.len() < n {
        let next = (0..n)
            .filter(|&i| placed[i].is_none())
            .max_by(|&a, &b| {
                let to = |i: usize| -> f64 {
                    if order.is_empty() {
                        weight[i].iter().sum()
                    } else {
                        order.iter().map(|&j| weight[i][j]).sum()
                    }
                };
                to(a)
                    .total_cmp(&to(b))
                    .then(area(a).total_cmp(&area(b)))
                    .then(b.cmp(&a))
            })
            .expect("a part is left");
        let (s, _) = best_free_place(next, sizes, &placed, &lines_of, ends, pulls);
        placed[next] = Some(s);
        order.push(next);
    }
    // Each part taken out and put back where it is best, while that helps.
    for _ in 0..ROUNDS {
        let mut moved = false;
        for &i in &order {
            let now = placed[i].expect("every part is placed");
            placed[i] = None;
            let others = placed.iter().flatten();
            let stay = length(i, &now, &placed, &lines_of, ends, pulls)
                + GROW
                    * (1.0 + lines_of[i].len() as f64)
                    * (extent(others.clone().chain(std::iter::once(&now))) - extent(others));
            let (s, price) = best_free_place(i, sizes, &placed, &lines_of, ends, pulls);
            if price + 1e-6 < stay {
                placed[i] = Some(s);
                moved = true;
            } else {
                placed[i] = Some(now);
            }
        }
        if !moved {
            break;
        }
    }
    placed
        .iter()
        .map(|s| s.map_or((0.0, 0.0), |s| (s.x, s.y)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiles(sizes: &[(f64, f64)], at: &[(f64, f64)]) -> Vec<Tile> {
        sizes
            .iter()
            .zip(at)
            .map(|(&(w, h), &(x, y))| Tile { x, y, w, h })
            .collect()
    }

    #[test]
    fn parts_with_many_lines_between_them_end_up_side_by_side() {
        // Four parts: 0 and 3 share six lines, 1 and 2 one each with 0.
        let sizes = [(100.0, 40.0); 4];
        let mid = (50.0, 20.0);
        let mut ends = vec![(0, mid, 3, mid); 6];
        ends.push((0, mid, 1, mid));
        ends.push((0, mid, 2, mid));
        let at = place_by_lines(&sizes, &ends, &[]);
        let s = tiles(&sizes, &at);
        let d = |a: usize, b: usize| (s[a].x - s[b].x).abs() + (s[a].y - s[b].y).abs();
        assert!(d(0, 3) <= d(0, 1) && d(0, 3) <= d(0, 2), "{at:?}");
        for a in 0..4 {
            for b in a + 1..4 {
                assert!(
                    !too_close(&s[a], &s[b]),
                    "parts {a} and {b} keep {GAP} px apart: {at:?}"
                );
            }
        }
        assert_eq!(
            at,
            place_by_lines(&sizes, &ends, &[]),
            "the same input, the same placement"
        );
    }

    #[test]
    fn a_line_that_leaves_the_box_pulls_its_part_its_way() {
        // Two parts without lines between them; part 1's line leaves to the left.
        let sizes = [(100.0, 40.0); 2];
        let free = place_by_lines(&sizes, &[], &[]);
        let pulled = place_by_lines(&sizes, &[], &[(1, (50.0, 20.0), (-1.0, 0.0))]);
        assert!(
            pulled[1].0 < pulled[0].0,
            "part 1 goes left of part 0: {pulled:?} (free: {free:?})"
        );
    }

    #[test]
    fn parts_without_lines_keep_the_box_small() {
        let sizes = [(100.0, 40.0), (100.0, 40.0), (100.0, 40.0)];
        let at = place_by_lines(&sizes, &[], &[]);
        let s = tiles(&sizes, &at);
        let e = extent(s.iter());
        // Three in a row or a column, or two and one: never further apart than that.
        assert!(e <= 3.0 * 100.0 + 2.0 * GAP + 40.0 + 1e-6, "{at:?}");
    }
}
