//! Node id rules (SPEC §3.1): lowercase `[a-z0-9_-]` segments joined with `.`.

/// Turn a symbol or file name into one id segment: `RustAdapter` → `rust-adapter`.
pub fn slug(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            if c.is_ascii_uppercase() {
                let prev = i.checked_sub(1).map(|j| chars[j]);
                let next_lower = chars.get(i + 1).is_some_and(|n| n.is_ascii_lowercase());
                let boundary = match prev {
                    Some(p) if p.is_ascii_lowercase() || p.is_ascii_digit() => true,
                    Some(p) if p.is_ascii_uppercase() => next_lower,
                    _ => false,
                };
                if boundary && !out.is_empty() && !out.ends_with('-') {
                    out.push('-');
                }
                out.push(c.to_ascii_lowercase());
            } else {
                out.push(c);
            }
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Whether an authored `@id` follows the id rules.
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.split('.').all(|seg| {
            !seg.is_empty()
                && seg
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("RustAdapter"), "rust-adapter");
        assert_eq!(slug("create_order"), "create_order");
        assert_eq!(slug("LANGUAGES"), "languages");
        assert_eq!(slug("HTMLParser"), "html-parser");
        assert_eq!(slug("orders.service"), "orders-service");
        assert_eq!(slug("docair-core"), "docair-core");
    }

    #[test]
    fn ids() {
        assert!(is_valid_id("orders.create"));
        assert!(is_valid_id("docair-core.extract.parse_body"));
        assert!(!is_valid_id("Orders.create"));
        assert!(!is_valid_id("orders..create"));
        assert!(!is_valid_id("topic:order.created"));
    }
}
