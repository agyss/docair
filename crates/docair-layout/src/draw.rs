//! How a routed line is drawn: its path, rounded where it turns, and its arrow. The SVG docair
//! writes and the page's moved lines (the WASM build) both use this, so a line is drawn the same
//! way in both.

use std::fmt::Write;

/// How far a line's corners are rounded, in px.
pub const CORNER: f64 = 6.0;

/// A number as the SVG writes it: one decimal at most, no `-0`.
pub fn num(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    if r == 0.0 {
        "0".to_string()
    } else if r.fract() == 0.0 {
        format!("{}", r as i64)
    } else {
        format!("{r:.1}")
    }
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
}

/// How long a line's arrow is, for its width (Cytoscape's triangle, at `arrow-scale` 0.9).
pub fn arrow_length(width: f64) -> f64 {
    0.3 * 0.9 * (width * 13.37).powf(0.9).max(29.0)
}

/// A line's points as a path: straight pieces, rounded where they turn.
pub fn rounded(pts: &[(f64, f64)]) -> String {
    let mut d = format!("M{},{}", num(pts[0].0), num(pts[0].1));
    for i in 1..pts.len() {
        let b = pts[i];
        if i + 1 == pts.len() {
            let _ = write!(d, " L{},{}", num(b.0), num(b.1));
            break;
        }
        let (a, c) = (pts[i - 1], pts[i + 1]);
        let (l1, l2) = (dist(a, b), dist(b, c));
        let u1 = ((b.0 - a.0) / l1, (b.1 - a.1) / l1);
        let u2 = ((c.0 - b.0) / l2, (c.1 - b.1) / l2);
        let r = CORNER.min(l1 / 2.0).min(l2 / 2.0);
        if (u1.0 * u2.1 - u1.1 * u2.0).abs() < 1e-6 || r < 0.5 {
            let _ = write!(d, " L{},{}", num(b.0), num(b.1));
        } else {
            let _ = write!(
                d,
                " L{},{} Q{},{} {},{}",
                num(b.0 - u1.0 * r),
                num(b.1 - u1.1 * r),
                num(b.0),
                num(b.1),
                num(b.0 + u2.0 * r),
                num(b.1 + u2.1 * r)
            );
        }
    }
    d
}

/// A line ready to draw: its path (`d` of the `ln` path) and its arrow (`d` of the `ah` path).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DrawnLine {
    pub d: String,
    pub arrow: String,
}

/// A routed line drawn at a width: repeated points dropped, the line stopping where its arrow
/// starts, the arrow a triangle on its last piece. `None` for fewer than two points.
pub fn line(route: &[(f64, f64)], width: f64) -> Option<DrawnLine> {
    let mut pts: Vec<(f64, f64)> = Vec::new();
    for &q in route {
        if pts.last().is_none_or(|p| dist(*p, q) > 0.01) {
            pts.push(q);
        }
    }
    if pts.len() < 2 {
        return None;
    }
    let a = arrow_length(width);
    let tip = pts[pts.len() - 1];
    let prev = pts[pts.len() - 2];
    let len = dist(prev, tip);
    let u = ((tip.0 - prev.0) / len, (tip.1 - prev.1) / len);
    let base = (tip.0 - u.0 * a, tip.1 - u.1 * a);
    if len > a {
        // The line stops where its arrow starts.
        let last = pts.len() - 1;
        pts[last] = base;
    }
    let (vx, vy) = (-u.1 * a / 2.0, u.0 * a / 2.0);
    Some(DrawnLine {
        d: rounded(&pts),
        arrow: format!(
            "M{},{} L{},{} L{},{} Z",
            num(tip.0),
            num(tip.1),
            num(base.0 + vx),
            num(base.1 + vy),
            num(base.0 - vx),
            num(base.1 - vy)
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_turns_round_and_stops_at_its_arrow() {
        assert_eq!(
            rounded(&[(0.0, 0.0), (0.0, 50.0), (40.0, 50.0)]),
            "M0,0 L0,44 Q0,50 6,50 L40,50"
        );
        assert_eq!(
            rounded(&[(0.0, 0.0), (0.0, 20.0), (0.0, 50.0)]),
            "M0,0 L0,20 L0,50"
        );
        let l = line(&[(0.0, 0.0), (0.0, 0.0), (0.0, 100.0)], 1.4).unwrap();
        let a = arrow_length(1.4);
        assert_eq!(l.d, format!("M0,0 L0,{}", num(100.0 - a)));
        assert!(l.arrow.starts_with("M0,100 L"), "{}", l.arrow);
        assert!(line(&[(1.0, 1.0)], 1.4).is_none());
    }
}
