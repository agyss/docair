//! A part moved by hand (docs/SLICES.md §7): the part and what it holds go where it was put down,
//! the boxes around it grow to hold it, and the lines that touch it are routed again around
//! everything else, by the same router. Nothing else moves: the other parts keep their places,
//! the other lines their ways and their words.

use crate::{Input, Layout, Padding, Rect, draw, router, word_spots};

/// A part put down somewhere else: by how much it moved.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PartMove {
    /// By index into `Input::parts`.
    pub part: usize,
    pub dx: f64,
    pub dy: f64,
}

/// A part or box at its new place.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ShiftedPart {
    pub part: usize,
    pub id: String,
    pub at: Rect,
}

/// A line routed again: by index into `Input::lines`, its id, its points, how to draw it, and its words.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ReroutedLine {
    pub line: usize,
    pub id: String,
    pub points: Vec<(f64, f64)>,
    /// None when the router found fewer than two points.
    pub drawn: Option<draw::DrawnLine>,
    pub words: Option<Rect>,
}

/// What a move changed: the parts and boxes now elsewhere or bigger, and the lines routed again.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MoveOutcome {
    pub parts: Vec<ShiftedPart>,
    pub lines: Vec<ReroutedLine>,
    /// The layout with the move in, for the next move.
    pub layout: Layout,
}

/***
 * Move a part by hand: it and what it holds shift; every box around it grows to hold it again
 * (with the room a box keeps around what it holds), and shrinks where the part was what held its
 * side out (docair-702.41); every line with an end in what moved, or inside a box that shrank, is
 * routed again around all the parts and around the lines that stay (docair-702.40), and its words
 * placed again. `width(line)` is how wide each line is drawn, for its arrow.
 *
 * @calls docair-layout.route "routes the moved lines around every part"
 ***/
pub fn move_part(
    input: &Input,
    layout: &Layout,
    m: PartMove,
    width: &dyn Fn(usize) -> f64,
) -> MoveOutcome {
    let n = input.parts.len();
    let mut placement = layout.placement.clone();
    if m.part >= n || placement.parts.len() != n {
        return MoveOutcome {
            parts: Vec::new(),
            lines: Vec::new(),
            layout: layout.clone(),
        };
    }
    // The part and every part inside it.
    let inside = |i: usize| {
        let mut cur = Some(i);
        while let Some(c) = cur {
            if c == m.part {
                return true;
            }
            cur = input.parts[c].parent.filter(|&q| q < n);
        }
        false
    };
    let moved: Vec<usize> = (0..n).filter(|&i| inside(i)).collect();
    for &i in &moved {
        placement.parts[i].x += m.dx;
        placement.parts[i].y += m.dy;
    }
    let mut changed: Vec<usize> = moved.clone();
    let mut shrunk: Vec<usize> = Vec::new();
    // Each box around it grows to hold what it holds, keeping its room around them; a side its kids
    // drew back from follows them by as much, so it never ends tighter than the layout made it
    // (docair-702.41).
    let room: Padding = crate::Options::default().padding;
    let extent = |at: &[Rect], kids: &[usize]| {
        kids.iter().fold(
            (
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ),
            |(l, t, r, b), &k| {
                let c = at[k];
                (
                    l.min(c.x - c.w / 2.0),
                    t.min(c.y - c.h / 2.0),
                    r.max(c.x + c.w / 2.0),
                    b.max(c.y + c.h / 2.0),
                )
            },
        )
    };
    let mut cur = input.parts[m.part].parent.filter(|&q| q < n);
    while let Some(b) = cur {
        let s = placement.scale.get(b).copied().unwrap_or(1.0);
        let kids: Vec<usize> = (0..n)
            .filter(|&i| input.parts[i].parent == Some(b))
            .collect();
        let mut r = placement.parts[b];
        let before = (
            r.x - r.w / 2.0,
            r.y - r.h / 2.0,
            r.x + r.w / 2.0,
            r.y + r.h / 2.0,
        );
        let (l0, t0, r0, b0) = extent(&layout.placement.parts, &kids);
        let (l1, t1, r1, b1) = extent(&placement.parts, &kids);
        let x1 = (before.0 + (l1 - l0).max(0.0)).min(l1 - room.left * s);
        let y1 = (before.1 + (t1 - t0).max(0.0)).min(t1 - room.top * s);
        let x2 = (before.2 + (r1 - r0).min(0.0)).max(r1 + room.right * s);
        let y2 = (before.3 + (b1 - b0).min(0.0)).max(b1 + room.bottom * s);
        let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
        if near(x1, before.0) && near(y1, before.1) && near(x2, before.2) && near(y2, before.3) {
            break;
        }
        if x1 > before.0 || y1 > before.1 || x2 < before.2 || y2 < before.3 {
            shrunk.push(b);
        }
        r = Rect {
            x: (x1 + x2) / 2.0,
            y: (y1 + y2) / 2.0,
            w: x2 - x1,
            h: y2 - y1,
        };
        placement.parts[b] = r;
        changed.push(b);
        cur = input.parts[b].parent.filter(|&q| q < n);
    }
    // The lines with an end in what moved, or inside a box that shrank (its border may now cut
    // them), routed again around every part and around the lines that stay (docair-702.40).
    let in_shrunk = |i: usize| {
        let mut c = input.parts[i].parent.filter(|&q| q < n);
        while let Some(b) = c {
            if shrunk.contains(&b) {
                return true;
            }
            c = input.parts[b].parent.filter(|&q| q < n);
        }
        false
    };
    let touched: Vec<usize> = (0..input.lines.len())
        .filter(|&j| {
            let l = &input.lines[j];
            moved.contains(&l.from) || moved.contains(&l.to) || in_shrunk(l.from) || in_shrunk(l.to)
        })
        .collect();
    let stay: Vec<Vec<(f64, f64)>> = (0..input.lines.len())
        .filter(|j| !touched.contains(j))
        .filter_map(|j| layout.routes.lines.get(j).cloned())
        .filter(|pts| pts.len() > 1)
        .collect();
    let sub = Input {
        parts: input.parts.clone(),
        lines: touched.iter().map(|&j| input.lines[j].clone()).collect(),
    };
    let routes = router::grid_lines_around(&sub, &placement, router::PRICES, &stay);
    let words = word_spots::place_words(&sub, &placement, &routes);
    let mut next = Layout {
        placement,
        routes: layout.routes.clone(),
        words: layout.words.clone(),
    };
    let mut lines = Vec::new();
    for (k, &j) in touched.iter().enumerate() {
        let points = routes.lines.get(k).cloned().unwrap_or_default();
        let w = words.get(k).copied().flatten();
        if let Some(slot) = next.routes.lines.get_mut(j) {
            *slot = points.clone();
        }
        if let Some(slot) = next.words.get_mut(j) {
            *slot = w;
        }
        lines.push(ReroutedLine {
            line: j,
            id: input.lines[j].id.clone(),
            drawn: draw::line(&points, width(j)),
            points,
            words: w,
        });
    }
    changed.sort_unstable();
    changed.dedup();
    MoveOutcome {
        parts: changed
            .into_iter()
            .map(|i| ShiftedPart {
                part: i,
                id: input.parts[i].id.clone(),
                at: next.placement.parts[i],
            })
            .collect(),
        lines,
        layout: next,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Line, Options, Part, lay_out};

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

    #[test]
    fn a_moved_part_takes_its_lines_and_its_box_grows() {
        // A box holding a and b; c outside; a -> b, b -> c, a -> c.
        let input = Input {
            parts: vec![
                part("box", None),
                part("a", Some(0)),
                part("b", Some(0)),
                part("c", None),
            ],
            lines: vec![line("0", 1, 2), line("1", 2, 3), line("2", 1, 3)],
        };
        let laid = lay_out(&input, &Options::default());
        let m = move_part(
            &input,
            &laid,
            PartMove {
                part: 2,
                dx: 0.0,
                dy: 300.0,
            },
            &|_| 1.4,
        );
        let ids: Vec<&str> = m.parts.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"b") && ids.contains(&"box"), "{ids:?}");
        let b = m.layout.placement.parts[2];
        let bx = m.layout.placement.parts[0];
        assert!(
            b.y + b.h / 2.0 <= bx.y + bx.h / 2.0,
            "the box holds b again"
        );
        let moved: Vec<&str> = m.lines.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(moved, ["0", "1"], "only the lines of b are routed again");
        assert_eq!(
            m.layout.routes.lines[2], laid.routes.lines[2],
            "a -> c keeps its way"
        );
        for l in &m.lines {
            let (first, last) = (l.points[0], *l.points.last().unwrap());
            let on = |p: (f64, f64), r: Rect| {
                (p.0 - r.x).abs() <= r.w / 2.0 + 1.0 && (p.1 - r.y).abs() <= r.h / 2.0 + 1.0
            };
            let line = &input.lines[l.line];
            assert!(
                on(first, m.layout.placement.parts[line.from]),
                "{} starts on its part",
                l.id
            );
            assert!(
                on(last, m.layout.placement.parts[line.to]),
                "{} ends on its part",
                l.id
            );
            for w in l.points.windows(2) {
                assert!(
                    (w[0].0 - w[1].0).abs() < 0.01 || (w[0].1 - w[1].1).abs() < 0.01,
                    "right angles"
                );
            }
            assert!(l.drawn.is_some());
        }
    }

    /// docair-702.41: moved back, a part takes its box back to where it was; the lines inside the box
    /// that shrank are routed again.
    #[test]
    fn a_box_shrinks_back_when_its_part_comes_back() {
        let input = Input {
            parts: vec![
                part("box", None),
                part("a", Some(0)),
                part("b", Some(0)),
                part("c", None),
            ],
            lines: vec![line("0", 1, 2), line("1", 2, 3), line("2", 1, 3)],
        };
        let laid = lay_out(&input, &Options::default());
        let away = move_part(
            &input,
            &laid,
            PartMove {
                part: 2,
                dx: 0.0,
                dy: 300.0,
            },
            &|_| 1.4,
        );
        assert!(away.layout.placement.parts[0].h > laid.placement.parts[0].h + 100.0);
        let back = move_part(
            &input,
            &away.layout,
            PartMove {
                part: 2,
                dx: 0.0,
                dy: -300.0,
            },
            &|_| 1.4,
        );
        let (was, now) = (laid.placement.parts[0], back.layout.placement.parts[0]);
        for (x, y) in [
            (was.x, now.x),
            (was.y, now.y),
            (was.w, now.w),
            (was.h, now.h),
        ] {
            assert!(
                (x - y).abs() < 0.01,
                "the box is as laid out again: {was:?} {now:?}"
            );
        }
        let rerouted: Vec<&str> = back.lines.iter().map(|l| l.id.as_str()).collect();
        assert!(
            rerouted.contains(&"2"),
            "a -> c, inside the box that shrank: {rerouted:?}"
        );
    }

    /// docair-702.40: a line routed around lines that stay keeps off them: with a staying line on
    /// its straight way, it takes another track.
    #[test]
    fn a_line_keeps_off_the_lines_that_stay() {
        let input = Input {
            parts: vec![part("a", None), part("b", None)],
            lines: vec![line("0", 0, 1)],
        };
        let laid = lay_out(&input, &Options::default());
        let alone = laid.routes.lines[0].clone();
        assert!(alone.len() >= 2);
        let around = crate::router::grid_lines_around(
            &input,
            &laid.placement,
            crate::router::PRICES,
            std::slice::from_ref(&alone),
        );
        assert_ne!(around.lines[0], alone, "not along the line that stays");
    }
}
