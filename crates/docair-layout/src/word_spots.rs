//! Where each line's words go (viewer.js `placeLabels`): on the line, where they cover no part, no
//! other words, no box's name and no other line; of such spots, the one nearest the arrow. Where
//! every spot on and beside the line covers something, the one that covers least (a part counts
//! most, then words and names, then lines).

use crate::text::{columns, wrap};
use crate::{Input, Placement, Rect, Routes};

#[derive(Debug, Clone, Copy)]
struct Area {
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

/// What words on another line's piece cost: as much as covering 20 px² of a part.
const LINE_PRICE: f64 = 60.0;

fn overlap(b: &Area, t: &Area) -> f64 {
    (b.x2.min(t.x2) - b.x1.max(t.x1)).max(0.0) * (b.y2.min(t.y2) - b.y1.max(t.y1)).max(0.0)
}

/// The place of each line's words, in the order of `Input::lines`; none for a line without words.
pub(crate) fn place_words(input: &Input, placed: &Placement, routes: &Routes) -> Vec<Option<Rect>> {
    let n = input.parts.len();
    let mut is_box = vec![false; n];
    for p in &input.parts {
        if let Some(q) = p.parent.filter(|&q| q < n) {
            is_box[q] = true;
        }
    }
    let scale = |i: usize| placed.scale.get(i).copied().unwrap_or(1.0);
    let mut parts: Vec<Area> = Vec::new();
    let mut words: Vec<Area> = Vec::new();
    for (i, p) in placed.parts.iter().enumerate() {
        if !is_box[i] {
            parts.push(Area {
                x1: p.x - p.w / 2.0 - 2.0,
                x2: p.x + p.w / 2.0 + 2.0,
                y1: p.y - p.h / 2.0 - 2.0,
                y2: p.y + p.h / 2.0 + 2.0,
            });
        } else if input.parts[i].name_width > 0.0 {
            // A box's name stands in the band at its top, in the middle.
            let sc = scale(i);
            let half = input.parts[i].name_width * sc / 2.0 + 2.0;
            let top = p.y - p.h / 2.0;
            words.push(Area {
                x1: p.x - half,
                x2: p.x + half,
                y1: top + 3.0 * sc,
                y2: top + 21.0 * sc,
            });
        }
    }
    // Every line's pieces, as thin boxes: words placed on another line's would hide it.
    let mut pieces: Vec<(usize, Area)> = Vec::new();
    for (k, r) in routes.lines.iter().enumerate() {
        for s in r.windows(2) {
            pieces.push((
                k,
                Area {
                    x1: s[0].0.min(s[1].0) - 1.0,
                    x2: s[0].0.max(s[1].0) + 1.0,
                    y1: s[0].1.min(s[1].1) - 1.0,
                    y2: s[0].1.max(s[1].1) + 1.0,
                },
            ));
        }
    }
    let mut out = vec![None; input.lines.len()];
    for (k, l) in input.lines.iter().enumerate() {
        let r = &routes.lines[k];
        let Some((w0, h0)) = l.words else { continue };
        if r.len() < 2 || l.from >= n || l.to >= n {
            continue;
        }
        // The words are as small as the smaller of the line's two ends is drawn.
        let f = scale(l.from).min(scale(l.to));
        let (w, h) = (w0 * f, h0 * f);
        // Spots along every piece, from the arrow back and from each piece's middle outwards: on the line first, then beside it.
        let mut on: Vec<(f64, f64)> = Vec::new();
        let mut beside: Vec<(f64, f64)> = Vec::new();
        for i in (1..r.len()).rev() {
            let (a, b) = (r[i - 1], r[i]);
            let flat = (b.1 - a.1).abs() < 0.5;
            let len = (b.0 - a.0).abs() + (b.1 - a.1).abs();
            let half = if flat { w } else { h } / 2.0 + 4.0;
            // the words stay on their piece where it is long enough for them
            let reach = (len / 2.0 - half).max(0.0);
            let mut d = 0.0;
            let mut count = 0;
            while d <= reach && count < 30 {
                let ts: Vec<f64> = if d > 0.0 {
                    vec![len / 2.0 + d, len / 2.0 - d]
                } else {
                    vec![len / 2.0]
                };
                for t in ts {
                    let c = (
                        a.0 + (b.0 - a.0) * t / len.max(1.0),
                        a.1 + (b.1 - a.1) * t / len.max(1.0),
                    );
                    on.push(c);
                    if flat {
                        beside.push((c.0, c.1 - h / 2.0 - 3.0));
                        beside.push((c.0, c.1 + h / 2.0 + 3.0));
                    } else {
                        beside.push((c.0 - w / 2.0 - 3.0, c.1));
                        beside.push((c.0 + w / 2.0 + 3.0, c.1));
                    }
                }
                d += 10.0;
                count += 1;
            }
        }
        // Only what lies near the line can be covered.
        let (mut rx1, mut rx2, mut ry1, mut ry2) = (
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        );
        for q in r {
            rx1 = rx1.min(q.0);
            rx2 = rx2.max(q.0);
            ry1 = ry1.min(q.1);
            ry2 = ry2.max(q.1);
        }
        let reach = Area {
            x1: rx1 - w - 6.0,
            x2: rx2 + w + 6.0,
            y1: ry1 - h - 6.0,
            y2: ry2 + h + 6.0,
        };
        let close =
            |t: &Area| t.x1 < reach.x2 && t.x2 > reach.x1 && t.y1 < reach.y2 && t.y2 > reach.y1;
        let near_parts: Vec<Area> = parts.iter().filter(|t| close(t)).copied().collect();
        let near_words: Vec<Area> = words.iter().filter(|t| close(t)).copied().collect();
        let near_pieces: Vec<Area> = pieces
            .iter()
            .filter(|(m, t)| *m != k && close(t))
            .map(|(_, t)| *t)
            .collect();
        let area_at = |c: (f64, f64)| Area {
            x1: c.0 - w / 2.0,
            x2: c.0 + w / 2.0,
            y1: c.1 - h / 2.0,
            y2: c.1 + h / 2.0,
        };
        let cost = |c: (f64, f64)| {
            let bx = area_at(c);
            near_parts
                .iter()
                .map(|t| 3.0 * overlap(&bx, t))
                .sum::<f64>()
                + near_words
                    .iter()
                    .map(|t| 2.0 * overlap(&bx, t))
                    .sum::<f64>()
                // Each line the words would sit on costs the same, however it crosses them:
                // fewer lines covered beats a smaller overlap (docair-irq).
                + near_pieces
                    .iter()
                    .filter(|t| overlap(&bx, t) > 0.0)
                    .count() as f64
                    * LINE_PRICE
        };
        let mut at: Option<(f64, f64)> = None;
        let mut least = f64::INFINITY;
        for c in on.iter().chain(&beside) {
            let price = cost(*c);
            if price < least {
                at = Some(*c);
                least = price;
            }
            if price == 0.0 {
                break;
            }
        }
        let Some(at) = at else { continue };
        words.push(area_at(at));
        out[k] = Some(Rect {
            x: at.0,
            y: at.1,
            w,
            h,
        });
    }
    out
}

/// The size of a text as a line's words: in lines of about 28 columns, 11 px, 14 px a line.
pub fn words_size(font: &crate::Monospace, text: &str) -> Option<(f64, f64)> {
    if columns(text) == 0 {
        return None;
    }
    let lines = wrap(text, crate::text::WORDS_COLUMNS);
    let w = lines
        .iter()
        .map(|l| font.width(l, 11.0).ceil())
        .fold(0.0, f64::max)
        + 22.0;
    Some((w, 14.0 * lines.len() as f64 + 3.0))
}

#[cfg(test)]
mod tests {
    use crate::{Input, Line, Options, Part, lay_out};

    fn part(id: &str) -> Part {
        Part {
            id: id.into(),
            parent: None,
            width: 80.0,
            height: 30.0,
            min_width: 0.0,
            name_width: 0.0,
        }
    }

    #[test]
    fn words_stand_on_their_line_and_cover_no_part() {
        let input = Input {
            parts: vec![part("a"), part("b"), part("c")],
            lines: vec![
                Line {
                    id: "a-b".into(),
                    from: 0,
                    to: 1,
                    flip: false,
                    ahead: 1,
                    words: Some((90.0, 17.0)),
                },
                Line {
                    id: "b-c".into(),
                    from: 1,
                    to: 2,
                    flip: false,
                    ahead: 1,
                    words: None,
                },
            ],
        };
        let laid = lay_out(&input, &Options::default());
        assert!(
            laid.words[1].is_none(),
            "a line without words gets no place for them"
        );
        let w = laid.words[0].expect("the words have a place");
        for p in &laid.placement.parts {
            let apart =
                (w.x - p.x).abs() * 2.0 >= w.w + p.w || (w.y - p.y).abs() * 2.0 >= w.h + p.h;
            assert!(apart, "the words {w:?} cover no part {p:?}");
        }
    }
}
