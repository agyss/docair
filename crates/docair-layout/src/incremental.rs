//! A picture's last layout brought to its new input (docs/REGENERATE.md §5), so a picture changes
//! only where its parts did, and quickly: what stays keeps its place and its lines their ways.
//!
//! The parts are matched by id and box. What is gone is left out, and the boxes it was in fit
//! what they still hold. What is new goes below the other parts of its box, in a band that makes
//! room for it: everything below the band's line moves down by the band's height, so the boxes
//! across the line grow and nothing overlaps. A new box is laid out on its own first. The lines
//! that stay keep their ways, moved with the bands; the new ones, and the ones at a part that is
//! new or changed, are routed around everything else, as after a move by hand.

use std::collections::HashMap;

use crate::{Input, Layout, Options, Placement, Rect, Routes, place, router, word_spots};

/// The room between a new part and the parts above it, in px at the scale it is drawn at.
const GAP: f64 = 24.0;

/// Values closer than this, in px, are one.
const NEAR: f64 = 1.0;

/// How often the lines an update routes are routed again, given each other.
const UPDATE_ROUNDS: usize = 1;

/***
 * Bring `old`, the layout of `old_input`, to `input`: what stays keeps its place, what is gone is left out, what is new is placed below the other parts of its box, and the lines that need it are routed again (docs/REGENERATE.md §5). None when the picture changed too much to update, so it is laid out from scratch: more than a quarter of its parts are new or gone (and more than 8), a part gets its first part or loses its last, or `old` is not a layout of `old_input`.
 *
 * @calls docair-layout.place "lays a new box out on its own"
 ***/
pub fn update_layout(
    old_input: &Input,
    old: &Layout,
    input: &Input,
    options: &Options,
) -> Option<Layout> {
    try_update_layout(old_input, old, input, options).ok()
}

/// As `update`, with why the picture is laid out from scratch instead.
pub fn try_update_layout(
    old_input: &Input,
    old: &Layout,
    input: &Input,
    options: &Options,
) -> Result<Layout, &'static str> {
    let (on, n) = (old_input.parts.len(), input.parts.len());
    if old.placement.parts.len() != on
        || old.placement.scale.len() != on
        || old.routes.lines.len() != old_input.lines.len()
        || old.words.len() != old_input.lines.len()
        || n == 0
    {
        return Err("the old layout is not one of the old input");
    }
    // Each part matched by its id and the id of its box.
    fn box_id(inp: &Input, i: usize) -> Option<&str> {
        inp.parts[i].parent.map(|p| inp.parts[p].id.as_str())
    }
    let old_at: HashMap<(&str, Option<&str>), usize> = (0..on)
        .map(|i| ((old_input.parts[i].id.as_str(), box_id(old_input, i)), i))
        .collect();
    let was: Vec<Option<usize>> = (0..n)
        .map(|i| {
            old_at
                .get(&(input.parts[i].id.as_str(), box_id(input, i)))
                .copied()
        })
        .collect();
    let kept: Vec<bool> = {
        let mut k = vec![false; on];
        for &o in was.iter().flatten() {
            k[o] = true;
        }
        k
    };
    let new_count = was.iter().filter(|w| w.is_none()).count();
    let gone_count = kept.iter().filter(|k| !**k).count();
    let changed = new_count + gone_count;
    if changed > 8 && changed * 4 > n.max(on) {
        return Err("more than a quarter of the parts are new or gone");
    }
    let is_box = |inp: &Input| {
        let mut b = vec![false; inp.parts.len()];
        for p in &inp.parts {
            if let Some(q) = p.parent.filter(|&q| q < b.len()) {
                b[q] = true;
            }
        }
        b
    };
    let (old_box, new_box) = (is_box(old_input), is_box(input));
    // A part that gets its first part or loses its last is a box, or no longer one: laid out again.
    for (i, w) in was.iter().enumerate() {
        if let Some(o) = *w
            && old_box[o] != new_box[i]
        {
            return Err("a part got its first part or lost its last");
        }
    }

    // The kept parts where they were.
    let mut at: Vec<Rect> = vec![
        Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0
        };
        n
    ];
    let mut scale: Vec<f64> = vec![1.0; n];
    let mut placed = vec![false; n];
    for i in 0..n {
        if let Some(o) = was[i] {
            at[i] = old.placement.parts[o];
            scale[i] = old.placement.scale[o];
            placed[i] = true;
        }
    }
    let mut routes: Vec<Option<Vec<(f64, f64)>>> = vec![None; input.lines.len()];
    let mut words: Vec<Option<Rect>> = vec![None; input.lines.len()];
    // The lines that stay: matched by their ends' ids and which of the lines between them they are.
    let line_key = |inp: &Input, j: usize, seen: &mut HashMap<(String, String), usize>| {
        let l = &inp.lines[j];
        let ends = (inp.parts[l.from].id.clone(), inp.parts[l.to].id.clone());
        let k = seen.entry(ends.clone()).or_insert(0);
        *k += 1;
        (ends.0, ends.1, *k)
    };
    let mut seen = HashMap::new();
    let old_lines: HashMap<(String, String, usize), usize> = (0..old_input.lines.len())
        .map(|j| (line_key(old_input, j, &mut seen), j))
        .collect();
    let mut seen = HashMap::new();
    let mut again: Vec<bool> = vec![false; input.lines.len()];
    for j in 0..input.lines.len() {
        let key = line_key(input, j, &mut seen);
        let l = &input.lines[j];
        match old_lines.get(&key) {
            Some(&o)
                if was[l.from].is_some()
                    && was[l.to].is_some()
                    && old_input.lines[o].words == l.words
                    && old.routes.lines[o].len() > 1 =>
            {
                routes[j] = Some(old.routes.lines[o].clone());
                words[j] = old.words[o];
            }
            _ => again[j] = true,
        }
    }
    let mut lay = UpdateWork {
        at,
        scale,
        placed,
        routes,
        words,
        is_box: new_box,
    };

    // Gone parts: each box that lost one fits what it still holds.
    let mut shrunk: Vec<usize> = Vec::new();
    for (o, k) in kept.iter().enumerate() {
        if *k {
            continue;
        }
        let Some(b) = old_input.parts[o].parent else {
            continue;
        };
        let Some(nb) = was.iter().position(|w| *w == Some(b)) else {
            continue;
        };
        if !shrunk.contains(&nb) {
            shrunk.push(nb);
        }
    }
    for &b in &shrunk {
        lay.fit_up(input, b, options);
    }

    // Kept parts whose size changed: they keep their top left, and make room when they grew. The size
    // compared is the input's: a placed part can be wider than its input says, as a part with many
    // lines is made wider (topdown); it grows by as much as its input did.
    let mut resized: Vec<usize> = Vec::new();
    for i in 0..n {
        let Some(o) = was[i] else {
            continue;
        };
        if lay.is_box[i] {
            continue;
        }
        let s = lay.scale[i];
        let (dw, dh) = (
            (input.parts[i].width - old_input.parts[o].width) * s,
            (input.parts[i].height - old_input.parts[o].height) * s,
        );
        if dw.abs() <= NEAR && dh.abs() <= NEAR {
            continue;
        }
        let r = lay.at[i];
        let (w, h) = ((r.w + dw).max(1.0), (r.h + dh).max(1.0));
        let (left, top) = (r.x - r.w / 2.0, r.y - r.h / 2.0);
        if w > r.w + NEAR {
            lay.band_x(left + r.w, w - r.w);
        }
        if h > r.h + NEAR {
            lay.band_y(top + r.h, h - r.h);
        }
        lay.at[i] = Rect {
            x: left + w / 2.0,
            y: top + h / 2.0,
            w,
            h,
        };
        resized.push(i);
    }

    // New parts: each new part whose box is placed, with what it holds, goes below its box's parts.
    let pad = options.padding;
    for i in 0..n {
        if lay.placed[i] || input.parts[i].parent.is_some_and(|p| !lay.placed[p]) {
            continue;
        }
        let parent = input.parts[i].parent;
        // The new part and the new parts inside it.
        let inside = |k: usize| {
            let mut c = Some(k);
            while let Some(x) = c {
                if x == i {
                    return true;
                }
                c = input.parts[x].parent;
            }
            false
        };
        let group: Vec<usize> = (0..n).filter(|&k| !lay.placed[k] && inside(k)).collect();
        let sub = sub_input(input, &group);
        let sub_placed = place(&sub, options);
        // The scale it is drawn at: its box's other parts', else the top's.
        let siblings: Vec<usize> = (0..n)
            .filter(|&k| lay.placed[k] && input.parts[k].parent == parent)
            .collect();
        let s = siblings
            .first()
            .map_or_else(|| parent.map_or(1.0, |p| lay.scale[p]), |&k| lay.scale[k]);
        let (w, h) = (sub_placed.width * s, sub_placed.height * s);
        let (left, cut) = match parent {
            Some(p) => {
                let r = lay.at[p];
                let sp = lay.scale[p];
                let inner_left = r.x - r.w / 2.0 + pad.left * sp;
                let bottom_of_kids = siblings
                    .iter()
                    .map(|&k| lay.at[k].y + lay.at[k].h / 2.0)
                    .fold(f64::NEG_INFINITY, f64::max);
                let cut = if bottom_of_kids.is_finite() {
                    bottom_of_kids + NEAR / 2.0
                } else {
                    r.y - r.h / 2.0 + pad.top * sp
                };
                // Too wide for its box: room to the right first.
                let inner_right = r.x + r.w / 2.0 - pad.right * sp;
                if inner_left + w > inner_right {
                    lay.band_x(inner_right - NEAR / 2.0, inner_left + w - inner_right);
                }
                (inner_left, cut)
            }
            None => {
                let (l, b) = (0..n).filter(|&k| lay.placed[k]).fold(
                    (f64::INFINITY, f64::NEG_INFINITY),
                    |(l, b), k| {
                        let r = lay.at[k];
                        (l.min(r.x - r.w / 2.0), b.max(r.y + r.h / 2.0))
                    },
                );
                (if l.is_finite() { l } else { 0.0 }, b + NEAR / 2.0)
            }
        };
        let gap = GAP * s;
        lay.band_y(cut, gap + h);
        let top = cut + gap;
        for (k, &g) in group.iter().enumerate() {
            let r = sub_placed.parts[k];
            lay.at[g] = Rect {
                x: left + r.x * s,
                y: top + r.y * s,
                w: r.w * s,
                h: r.h * s,
            };
            lay.scale[g] = sub_placed.scale[k] * s;
            lay.placed[g] = true;
        }
    }
    if lay.placed.iter().any(|p| !p) {
        return Err("a new part found no box to go into");
    }

    // Lines routed again: new ones, ones at a new or resized part, ones whose ends no longer sit on
    // their parts, ones that now run through a new part.
    let new_part: Vec<bool> = was.iter().map(Option::is_none).collect();
    for j in 0..input.lines.len() {
        if again[j] {
            continue;
        }
        let l = &input.lines[j];
        let Some(pts) = &lay.routes[j] else {
            again[j] = true;
            continue;
        };
        let off = |p: (f64, f64), i: usize| !on_border(&lay.at[i], p);
        if new_part[l.from]
            || new_part[l.to]
            || resized.contains(&l.from)
            || resized.contains(&l.to)
            || pts.first().is_none_or(|&p| off(p, l.from))
            || pts.last().is_none_or(|&p| off(p, l.to))
            || (0..n).any(|i| new_part[i] && !lay.is_box[i] && crosses(pts, &lay.at[i]))
        {
            again[j] = true;
        }
    }
    let placement = {
        let (r, b) = lay.at.iter().fold((0.0f64, 0.0f64), |(r, b), p| {
            (r.max(p.x + p.w / 2.0), b.max(p.y + p.h / 2.0))
        });
        let (width, height) = (r.max(old.placement.width), b.max(old.placement.height));
        let fit = 72.0;
        Placement {
            parts: lay.at.clone(),
            scale: lay.scale.clone(),
            width,
            height,
            direction: old.placement.direction,
            zoom: ((options.canvas.0 - fit) / width).min((options.canvas.1 - fit) / height),
            way: "updated".to_string(),
            price: old.placement.price,
        }
    };
    let todo: Vec<usize> = (0..input.lines.len()).filter(|&j| again[j]).collect();
    let timed = std::env::var("DOCAIR_TIMING").is_ok_and(|v| !v.is_empty() && v != "0");
    let started = std::time::Instant::now();
    if timed && todo.len() > 50 {
        eprintln!(
            "docair timing: update: routing {} of {} lines again…",
            todo.len(),
            input.lines.len()
        );
    }
    if !todo.is_empty() {
        let stay: Vec<Vec<(f64, f64)>> = (0..input.lines.len())
            .filter(|&j| !again[j])
            .filter_map(|j| lay.routes[j].clone())
            .collect();
        let sub = Input {
            parts: input.parts.clone(),
            lines: todo.iter().map(|&j| input.lines[j].clone()).collect(),
        };
        // One round: each line routed once, given the lines that stay and the ones routed before it.
        // The later rounds of a full layout settle every line against every other; here most lines
        // stay, and on docair's Code layer a second round doubled the time (2.1 s to 4.2 s for 24
        // lines) for the same lines kept.
        let prices = router::Prices {
            rounds: UPDATE_ROUNDS,
            ..router::PRICES
        };
        let routed = router::grid_lines_around(&sub, &placement, prices, &stay);
        if timed {
            eprintln!(
                "docair timing: update: routed in {:.1} ms",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        let placed_words = word_spots::place_words(&sub, &placement, &routed);
        for (k, &j) in todo.iter().enumerate() {
            lay.routes[j] = routed.lines.get(k).cloned();
            lay.words[j] = placed_words.get(k).copied().flatten();
        }
    }
    if timed {
        eprintln!(
            "docair timing: update: {new_count} parts new, {gone_count} gone, {} of {} lines routed again in {:.1} ms",
            todo.len(),
            input.lines.len(),
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    let lines: Vec<Vec<(f64, f64)>> = lay
        .routes
        .into_iter()
        .map(Option::unwrap_or_default)
        .collect();
    let none = lines.iter().filter(|l| l.len() < 2).count();
    Ok(Layout {
        placement,
        routes: Routes {
            lines,
            price: old.routes.price,
            none,
        },
        words: lay.words,
    })
}

/// The layout being updated.
struct UpdateWork {
    at: Vec<Rect>,
    scale: Vec<f64>,
    placed: Vec<bool>,
    routes: Vec<Option<Vec<(f64, f64)>>>,
    words: Vec<Option<Rect>>,
    is_box: Vec<bool>,
}

impl UpdateWork {
    /// Room across the picture below `cut`: every placed thing below it moves down by `dh`; a box
    /// across it grows; a part moves when its middle is below it.
    fn band_y(&mut self, cut: f64, dh: f64) {
        if dh <= 0.0 {
            return;
        }
        for i in 0..self.at.len() {
            if !self.placed[i] {
                continue;
            }
            let r = &mut self.at[i];
            if self.is_box[i] {
                let (mut t, mut b) = (r.y - r.h / 2.0, r.y + r.h / 2.0);
                if t >= cut {
                    t += dh;
                }
                if b >= cut {
                    b += dh;
                }
                r.y = (t + b) / 2.0;
                r.h = b - t;
            } else if r.y >= cut {
                r.y += dh;
            }
        }
        for pts in self.routes.iter_mut().flatten() {
            for p in pts.iter_mut() {
                if p.1 >= cut {
                    p.1 += dh;
                }
            }
        }
        for w in self.words.iter_mut().flatten() {
            if w.y >= cut {
                w.y += dh;
            }
        }
    }

    /// As `band_y`, to the right of `cut`.
    fn band_x(&mut self, cut: f64, dw: f64) {
        if dw <= 0.0 {
            return;
        }
        for i in 0..self.at.len() {
            if !self.placed[i] {
                continue;
            }
            let r = &mut self.at[i];
            if self.is_box[i] {
                let (mut l, mut rr) = (r.x - r.w / 2.0, r.x + r.w / 2.0);
                if l >= cut {
                    l += dw;
                }
                if rr >= cut {
                    rr += dw;
                }
                r.x = (l + rr) / 2.0;
                r.w = rr - l;
            } else if r.x >= cut {
                r.x += dw;
            }
        }
        for pts in self.routes.iter_mut().flatten() {
            for p in pts.iter_mut() {
                if p.0 >= cut {
                    p.0 += dw;
                }
            }
        }
        for w in self.words.iter_mut().flatten() {
            if w.x >= cut {
                w.x += dw;
            }
        }
    }

    /// The box `b` and each box around it fit what they hold, with their room around it and wide
    /// enough for their names, as after a part is moved out of them (docair-702.41, 702.45).
    fn fit_up(&mut self, input: &Input, b: usize, options: &Options) {
        let room = options.padding;
        let mut cur = Some(b);
        while let Some(bx) = cur {
            let kids: Vec<usize> = (0..input.parts.len())
                .filter(|&k| self.placed[k] && input.parts[k].parent == Some(bx))
                .collect();
            if kids.is_empty() {
                break;
            }
            let s = self.scale[bx];
            let (l, t, r, bt) = kids.iter().fold(
                (
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ),
                |(l, t, r, b), &k| {
                    let c = self.at[k];
                    (
                        l.min(c.x - c.w / 2.0),
                        t.min(c.y - c.h / 2.0),
                        r.max(c.x + c.w / 2.0),
                        b.max(c.y + c.h / 2.0),
                    )
                },
            );
            let x1 = l - room.left * s;
            let y1 = t - room.top * s;
            let x2 = (r + room.right * s).max(x1 + input.parts[bx].min_width * s);
            let y2 = bt + room.bottom * s;
            self.at[bx] = Rect {
                x: (x1 + x2) / 2.0,
                y: (y1 + y2) / 2.0,
                w: x2 - x1,
                h: y2 - y1,
            };
            cur = input.parts[bx].parent;
        }
    }
}

/// The new parts of a group as an input of their own, their boxes and lines among them.
fn sub_input(input: &Input, group: &[usize]) -> Input {
    let index: HashMap<usize, usize> = group.iter().enumerate().map(|(k, &g)| (g, k)).collect();
    let parts = group
        .iter()
        .map(|&g| {
            let mut p = input.parts[g].clone();
            p.parent = p.parent.and_then(|q| index.get(&q).copied());
            p
        })
        .collect();
    let lines = input
        .lines
        .iter()
        .filter_map(|l| {
            let (f, t) = (index.get(&l.from)?, index.get(&l.to)?);
            let mut l = l.clone();
            l.from = *f;
            l.to = *t;
            Some(l)
        })
        .collect();
    Input { parts, lines }
}

/// Whether a point lies on a part's border.
fn on_border(r: &Rect, (x, y): (f64, f64)) -> bool {
    let (l, rr, t, b) = (
        r.x - r.w / 2.0,
        r.x + r.w / 2.0,
        r.y - r.h / 2.0,
        r.y + r.h / 2.0,
    );
    let in_x = x >= l - NEAR && x <= rr + NEAR;
    let in_y = y >= t - NEAR && y <= b + NEAR;
    (in_y && ((x - l).abs() <= NEAR || (x - rr).abs() <= NEAR))
        || (in_x && ((y - t).abs() <= NEAR || (y - b).abs() <= NEAR))
}

/// Whether a line at right angles runs through a part (not just along its border).
fn crosses(pts: &[(f64, f64)], r: &Rect) -> bool {
    let (l, rr, t, b) = (
        r.x - r.w / 2.0 + NEAR,
        r.x + r.w / 2.0 - NEAR,
        r.y - r.h / 2.0 + NEAR,
        r.y + r.h / 2.0 - NEAR,
    );
    pts.windows(2).any(|s| {
        let (x0, x1) = (s[0].0.min(s[1].0), s[0].0.max(s[1].0));
        let (y0, y1) = (s[0].1.min(s[1].1), s[0].1.max(s[1].1));
        x1 > l && x0 < rr && y1 > t && y0 < b
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Line, Part, lay_out};

    fn part(id: &str, parent: Option<usize>) -> Part {
        Part {
            id: id.to_string(),
            parent,
            width: 80.0,
            height: 30.0,
            min_width: 0.0,
            name_width: 0.0,
        }
    }

    fn line(id: &str, from: usize, to: usize) -> Line {
        Line {
            id: id.to_string(),
            from,
            to,
            flip: false,
            ahead: 0,
            words: Some((40.0, 14.0)),
        }
    }

    /// A box with three parts in a row of calls, and a part outside it.
    fn shop() -> Input {
        Input {
            parts: vec![
                part("api", None),
                part("a", Some(0)),
                part("b", Some(0)),
                part("c", Some(0)),
                part("db", None),
            ],
            lines: vec![line("0", 1, 2), line("1", 2, 3), line("2", 3, 4)],
        }
    }

    fn options() -> Options {
        Options {
            budget: f64::INFINITY,
            floor: 1.0,
            ..Options::default()
        }
    }

    /// No two parts that are not inside one another overlap.
    fn no_overlaps(input: &Input, l: &Layout) {
        let inside = |a: usize, b: usize| {
            let mut c = input.parts[a].parent;
            while let Some(x) = c {
                if x == b {
                    return true;
                }
                c = input.parts[x].parent;
            }
            false
        };
        for a in 0..input.parts.len() {
            for b in a + 1..input.parts.len() {
                if inside(a, b) || inside(b, a) {
                    continue;
                }
                let (p, q) = (l.placement.parts[a], l.placement.parts[b]);
                let apart = (p.x - q.x).abs() * 2.0 >= p.w + q.w - 0.01
                    || (p.y - q.y).abs() * 2.0 >= p.h + q.h - 0.01;
                assert!(
                    apart,
                    "{} and {} overlap: {p:?} {q:?}",
                    input.parts[a].id, input.parts[b].id
                );
            }
        }
        // A part lies inside its box.
        for (i, p) in input.parts.iter().enumerate() {
            if let Some(b) = p.parent {
                let (r, o) = (l.placement.parts[i], l.placement.parts[b]);
                assert!(
                    r.x - r.w / 2.0 >= o.x - o.w / 2.0 - 0.01
                        && r.x + r.w / 2.0 <= o.x + o.w / 2.0 + 0.01
                        && r.y - r.h / 2.0 >= o.y - o.h / 2.0 - 0.01
                        && r.y + r.h / 2.0 <= o.y + o.h / 2.0 + 0.01,
                    "{} lies outside its box",
                    p.id
                );
            }
        }
    }

    #[test]
    fn the_same_input_keeps_the_layout() {
        let (inp, opts) = (shop(), options());
        let old = lay_out(&inp, &opts);
        let new = update_layout(&inp, &old, &inp, &opts).unwrap();
        assert_eq!(new.placement.parts, old.placement.parts);
        assert_eq!(new.routes.lines, old.routes.lines);
        assert_eq!(new.words, old.words);
    }

    #[test]
    fn a_new_part_goes_below_its_box_s_parts_and_what_stays_keeps_its_place() {
        let (old_in, opts) = (shop(), options());
        let old = lay_out(&old_in, &opts);
        let mut inp = old_in.clone();
        inp.parts.push(part("audit", Some(0)));
        inp.lines.push(line("3", 3, 5));
        let new = update_layout(&old_in, &old, &inp, &opts).unwrap();
        no_overlaps(&inp, &new);
        // a, b and c keep their places; the box grew down.
        for i in 1..4 {
            assert_eq!(
                new.placement.parts[i], old.placement.parts[i],
                "{}",
                inp.parts[i].id
            );
        }
        let (bx, audit) = (new.placement.parts[0], new.placement.parts[5]);
        assert!(bx.h > old.placement.parts[0].h);
        let lowest = (1..4)
            .map(|i| old.placement.parts[i].y + old.placement.parts[i].h / 2.0)
            .fold(f64::MIN, f64::max);
        assert!(
            audit.y - audit.h / 2.0 > lowest,
            "the new part is below the others"
        );
        // Its line is routed, the old ones kept.
        assert!(new.routes.lines[3].len() >= 2);
        assert_eq!(new.routes.lines[0], old.routes.lines[0]);
    }

    #[test]
    fn a_gone_part_takes_its_lines_and_its_box_fits_what_is_left() {
        let (old_in, opts) = (shop(), options());
        let old = lay_out(&old_in, &opts);
        // c and its lines are gone.
        let inp = Input {
            parts: vec![
                part("api", None),
                part("a", Some(0)),
                part("b", Some(0)),
                part("db", None),
            ],
            lines: vec![line("0", 1, 2)],
        };
        let new = update_layout(&old_in, &old, &inp, &opts).unwrap();
        no_overlaps(&inp, &new);
        assert_eq!(new.routes.lines.len(), 1);
        assert_eq!(new.routes.lines[0], old.routes.lines[0]);
        assert_eq!(new.placement.parts[1], old.placement.parts[1]);
    }

    #[test]
    fn a_part_that_grew_makes_room_and_its_lines_are_routed_again() {
        let (old_in, opts) = (shop(), options());
        let old = lay_out(&old_in, &opts);
        let mut inp = old_in.clone();
        inp.parts[2].width = 200.0;
        let new = update_layout(&old_in, &old, &inp, &opts).unwrap();
        no_overlaps(&inp, &new);
        assert!((new.placement.parts[2].w - 200.0).abs() < 0.01);
        for j in [0, 1] {
            let pts = &new.routes.lines[j];
            assert!(pts.len() >= 2);
        }
    }

    #[test]
    fn too_many_changes_are_laid_out_from_scratch() {
        let opts = options();
        let parts = (0..20)
            .map(|k| part(&format!("p{k}"), None))
            .collect::<Vec<_>>();
        let old_in = Input {
            parts: parts.clone(),
            lines: vec![],
        };
        let old = lay_out(&old_in, &opts);
        let mut renamed = parts;
        for p in renamed.iter_mut().take(12) {
            p.id.push('x');
        }
        let inp = Input {
            parts: renamed,
            lines: vec![],
        };
        assert!(update_layout(&old_in, &old, &inp, &opts).is_none());
        // A part that gets its first part is a box now.
        let mut boxed = old_in.clone();
        boxed.parts.push(part("inner", Some(0)));
        assert!(update_layout(&old_in, &old, &boxed, &opts).is_none());
    }

    #[test]
    fn an_update_is_the_same_every_time() {
        let (old_in, opts) = (shop(), options());
        let old = lay_out(&old_in, &opts);
        let mut inp = old_in.clone();
        inp.parts.push(part("audit", Some(0)));
        inp.parts.push(part("log", None));
        inp.lines.push(line("3", 3, 5));
        inp.lines.push(line("4", 5, 6));
        let a = update_layout(&old_in, &old, &inp, &opts).unwrap();
        let b = update_layout(&old_in, &old, &inp, &opts).unwrap();
        assert_eq!(a, b);
        no_overlaps(&inp, &a);
    }
}
