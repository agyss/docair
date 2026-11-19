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
 * Move a part by hand: it and what it holds shift, every box around it grows to hold it again
 * (with the room a box keeps around what it holds), and every line with an end in what moved is
 * routed again around all the parts, and its words placed again. `width(line)` is how wide each
 * line is drawn, for its arrow.
 *
 * The lines that stay are not obstacles for the moved ones: a moved line may run along a line
 * that stayed (docair-702 decision, see the issue).
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
    // Each box around it grows to hold what it holds, keeping its room around them.
    let room: Padding = crate::Options::default().padding;
    let mut cur = input.parts[m.part].parent.filter(|&q| q < n);
    while let Some(b) = cur {
        let s = placement.scale.get(b).copied().unwrap_or(1.0);
        let kids: Vec<usize> = (0..n)
            .filter(|&i| input.parts[i].parent == Some(b))
            .collect();
        let mut r = placement.parts[b];
        let (mut x1, mut y1, mut x2, mut y2) = (
            r.x - r.w / 2.0,
            r.y - r.h / 2.0,
            r.x + r.w / 2.0,
            r.y + r.h / 2.0,
        );
        let before = (x1, y1, x2, y2);
        for &k in &kids {
            let c = placement.parts[k];
            x1 = x1.min(c.x - c.w / 2.0 - room.left * s);
            y1 = y1.min(c.y - c.h / 2.0 - room.top * s);
            x2 = x2.max(c.x + c.w / 2.0 + room.right * s);
            y2 = y2.max(c.y + c.h / 2.0 + room.bottom * s);
        }
        if (x1, y1, x2, y2) == before {
            break;
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
    // The lines with an end in what moved, routed again around every part.
    let touched: Vec<usize> = (0..input.lines.len())
        .filter(|&j| moved.contains(&input.lines[j].from) || moved.contains(&input.lines[j].to))
        .collect();
    let sub = Input {
        parts: input.parts.clone(),
        lines: touched.iter().map(|&j| input.lines[j].clone()).collect(),
    };
    let routes = router::grid_lines(&sub, &placement, router::PRICES);
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
}
