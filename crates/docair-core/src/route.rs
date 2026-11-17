//! Routes and requested paths: the path algebra HTTP rendezvous points meet by (docs/ADAPTERS.md §7).
//!
//! Adapters hand over strings as the code writes them: `/api/orders/{id}`
//! (OpenAPI, axum), `/api/orders/:id` (express), `` `/api/orders/${id}` ``
//! (a TypeScript template), `${environment.apiUrl}/orders` (a prefix the code
//! does not show). This module splits them into segments, compares routes,
//! and says how well a request fits a route.

use std::fmt;

/// One segment of a route or a requested path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// Written as is: `orders`.
    Literal(String),
    /// A value: `{id}`, `:id`, `<id>`, `${id}`, or a segment partly made of one (`v{n}`). The name as written.
    Param(String),
    /// The rest of the path, one segment or more: `{*rest}`, `*rest`.
    Rest(String),
}

/// A route or a requested path, split into segments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    /// Whether the path starts where it says. `${base}/orders` does not say what comes before `/orders`.
    pub anchored: bool,
    pub segments: Vec<Segment>,
}

/// How well a request fits a route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fit {
    No,
    /// It may: a value the code computes meets a fixed segment, or the start of the request is unknown.
    Could,
    /// It does.
    Certain,
}

impl Route {
    /// Split a route or a requested path. A query string and a fragment are dropped, and so are
    /// empty segments (`//`, a trailing `/`); a scheme and host are dropped too.
    pub fn parse(text: &str) -> Route {
        let text = text.trim();
        let text = match text.find("://") {
            Some(i) if !text[..i].contains(['/', '$', '{']) => {
                let rest = &text[i + 3..];
                rest.find('/').map_or("/", |j| &rest[j..])
            }
            _ => text,
        };
        let parts = split(text);
        let unknown_start =
            parts.first().is_some_and(|p| is_expression(p)) && !text.starts_with('/');
        let segments = parts
            .iter()
            .skip(usize::from(unknown_start))
            .filter(|p| !p.is_empty())
            .map(|p| segment(p))
            .collect();
        Route {
            anchored: !unknown_start,
            segments,
        }
    }

    /// Whether two routes are the same up to the names of their parameters:
    /// `/orders/{id}` and `/orders/:orderId` are.
    pub fn same(&self, other: &Route) -> bool {
        self.anchored == other.anchored
            && self.segments.len() == other.segments.len()
            && self
                .segments
                .iter()
                .zip(&other.segments)
                .all(|(a, b)| match (a, b) {
                    (Segment::Literal(x), Segment::Literal(y)) => x == y,
                    (Segment::Param(_), Segment::Param(_))
                    | (Segment::Rest(_), Segment::Rest(_)) => true,
                    _ => false,
                })
    }

    /// How well this request fits `route`. A request with an unknown start fits by its known tail, and at most `Could`.
    pub fn fit(&self, route: &Route) -> Fit {
        if !self.anchored {
            let n = self.segments.len();
            if n == 0 {
                return Fit::Could;
            }
            // Try the tail of the route at every position a Rest could leave it.
            return (0..=route.segments.len().saturating_sub(n))
                .rev()
                .map(|start| fit_segments(&self.segments, &route.segments[start..]))
                .max()
                .unwrap_or(Fit::No)
                .min(Fit::Could);
        }
        fit_segments(&self.segments, &route.segments)
    }

    /// This route mounted under `prefix`: `/api` + `/orders` = `/api/orders`.
    pub fn under(&self, prefix: &Route) -> Route {
        Route {
            anchored: prefix.anchored,
            segments: prefix
                .segments
                .iter()
                .chain(&self.segments)
                .cloned()
                .collect(),
        }
    }

    /// The route with its parameters unnamed, for comparing and grouping: `/orders/{}`.
    pub fn key(&self) -> String {
        let mut out = if self.anchored {
            String::new()
        } else {
            "…".to_string()
        };
        for s in &self.segments {
            out.push('/');
            match s {
                Segment::Literal(l) => out.push_str(l),
                Segment::Param(_) => out.push_str("{}"),
                Segment::Rest(_) => out.push_str("{*}"),
            }
        }
        if out.is_empty() || out == "…" {
            out.push('/');
        }
        out
    }
}

impl fmt::Display for Route {
    /// Parameters written the OpenAPI way: `/orders/{id}`, `{*rest}`; an unknown start as `…`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.anchored {
            f.write_str("…")?;
        }
        if self.segments.is_empty() {
            return f.write_str("/");
        }
        for s in &self.segments {
            match s {
                Segment::Literal(l) => write!(f, "/{l}")?,
                Segment::Param(p) if p.starts_with('{') || is_expression(p) => write!(f, "/{p}")?,
                Segment::Param(p) => write!(f, "/{{{p}}}")?,
                Segment::Rest(r) => write!(f, "/{{*{r}}}")?,
            }
        }
        Ok(())
    }
}

/// A base path and a path, as one route string: `/api` + `/orders` = `/api/orders`.
pub fn join(base: &str, path: &str) -> String {
    Route::parse(path).under(&Route::parse(base)).to_string()
}

fn fit_segments(request: &[Segment], route: &[Segment]) -> Fit {
    let mut fit = Fit::Certain;
    let mut i = 0;
    for (k, r) in route.iter().enumerate() {
        if let Segment::Rest(_) = r {
            // The rest of the route takes one segment or more.
            return if request.len() > i { fit } else { Fit::No };
        }
        let Some(q) = request.get(i) else {
            return Fit::No;
        };
        let step = match (q, r) {
            (Segment::Literal(a), Segment::Literal(b)) if a == b => Fit::Certain,
            (Segment::Literal(_), Segment::Literal(_)) => return Fit::No,
            (_, Segment::Param(_)) => Fit::Certain,
            (Segment::Param(_), Segment::Literal(_)) => Fit::Could,
            // A request whose rest is unknown could go on to any route that starts the same.
            (Segment::Rest(_), _) => return fit.min(Fit::Could),
            (_, Segment::Rest(_)) => unreachable!("a Rest in the route returns above, at {k}"),
        };
        fit = fit.min(step);
        i += 1;
    }
    if i == request.len() { fit } else { Fit::No }
}

/// Split at `/`, and stop at `?` or `#`, outside `${…}` and `{…}`.
fn split(text: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut depth = 0usize;
    for c in text.chars() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            '/' if depth == 0 => {
                parts.push(String::new());
                continue;
            }
            '?' | '#' if depth == 0 => break,
            _ => {}
        }
        if let Some(last) = parts.last_mut() {
            last.push(c);
        }
    }
    if parts.first().is_some_and(String::is_empty) && text.starts_with('/') {
        parts.remove(0);
    }
    parts
}

fn is_expression(part: &str) -> bool {
    part.starts_with("${") && part.ends_with('}')
}

fn segment(part: &str) -> Segment {
    if let Some(inner) = part.strip_prefix('{').and_then(|p| p.strip_suffix('}')) {
        if let Some(rest) = inner.strip_prefix('*') {
            return Segment::Rest(rest.to_string());
        }
        // `{id:[0-9]+}`: the name before the pattern.
        let name = inner.split(':').next().unwrap_or(inner);
        return Segment::Param(name.to_string());
    }
    if let Some(name) = part.strip_prefix(':') {
        return Segment::Param(name.to_string());
    }
    if let Some(rest) = part.strip_prefix('*') {
        return Segment::Rest(rest.to_string());
    }
    if let Some(inner) = part.strip_prefix('<').and_then(|p| p.strip_suffix('>')) {
        let name = inner.rsplit(':').next().unwrap_or(inner);
        return Segment::Param(name.to_string());
    }
    if is_expression(part) {
        return Segment::Param(part.to_string());
    }
    if part.contains("${") || (part.contains('{') && part.contains('}')) {
        // Partly a value, such as `v{n}` or `${id}.json`: it matches like a parameter.
        return Segment::Param(part.to_string());
    }
    Segment::Literal(part.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(s: &str) -> Route {
        Route::parse(s)
    }

    #[test]
    fn parameters_in_every_spelling() {
        let want = r("/api/orders/{id}");
        for s in [
            "/api/orders/:id",
            "/api/orders/${id}",
            "/api/orders/<int:id>",
            "/api/orders/{id:[0-9]+}",
            "api/orders/{orderId}/",
            "/api//orders/{id}?expand=items",
            "https://shop.example.com/api/orders/{id}#top",
        ] {
            assert!(r(s).same(&want), "{s} → {:?}", r(s));
        }
        assert!(!r("/api/orders").same(&want));
        assert!(!r("/api/order/{id}").same(&want));
        assert_eq!(
            r("/files/{*path}").segments[1],
            Segment::Rest("path".into())
        );
        assert_eq!(r("/").segments, vec![]);
    }

    #[test]
    fn expressions_with_slashes_or_question_marks_stay_one_segment() {
        let x = r("/api/${a ? 'x/y' : b}/items?q=1");
        assert_eq!(x.segments.len(), 3);
        assert!(matches!(&x.segments[1], Segment::Param(p) if p.contains('?')));
    }

    #[test]
    fn requests_fit_routes() {
        let route = r("/api/orders/{id}");
        assert_eq!(r("/api/orders/42").fit(&route), Fit::Certain);
        assert_eq!(r("/api/orders/${order.id}").fit(&route), Fit::Certain);
        assert_eq!(r("/api/orders").fit(&route), Fit::No);
        assert_eq!(r("/api/orders/42/items").fit(&route), Fit::No);
        // A computed value meets a fixed segment: it could be that one.
        assert_eq!(r("/api/${kind}/42").fit(&route), Fit::Could);
        assert_eq!(
            r("/api/orders/new").fit(&r("/api/orders/new")),
            Fit::Certain
        );
        assert_eq!(r("/files/a/b/c").fit(&r("/files/{*path}")), Fit::Certain);
        assert_eq!(r("/files").fit(&r("/files/{*path}")), Fit::No);
    }

    #[test]
    fn an_unknown_start_fits_by_the_known_tail() {
        let req = r("${environment.apiUrl}/orders/${id}");
        assert!(!req.anchored);
        assert_eq!(req.fit(&r("/api/orders/{id}")), Fit::Could);
        assert_eq!(req.fit(&r("/orders/{id}")), Fit::Could);
        assert_eq!(req.fit(&r("/api/carts/{id}")), Fit::No);
        assert_eq!(req.to_string(), "…/orders/${id}");
    }

    #[test]
    fn prefixes_compose() {
        assert_eq!(join("/api", "/orders"), "/api/orders");
        assert_eq!(join("/api/", "orders/{id}"), "/api/orders/{id}");
        assert_eq!(join("", "/orders"), "/orders");
        assert_eq!(join("/api", "/"), "/api");
        assert_eq!(
            r("/orders/:id").under(&r("/api")).to_string(),
            "/api/orders/{id}"
        );
        assert_eq!(r("/orders/{id}").key(), "/orders/{}");
        assert_eq!(r("/").key(), "/");
    }
}
