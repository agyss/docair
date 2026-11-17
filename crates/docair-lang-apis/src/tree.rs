//! A YAML or JSON document as a tree whose nodes know their line.
//!
//! API descriptions are loosely typed: status codes are often unquoted keys
//! (`200:`), versions are numbers (`swagger: 2.0`), and schemas nest freely.
//! A tree of plain values reads all of that, and every node keeps the line it
//! starts on, so facts can point at the exact place in the file.

use std::fmt;

use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_saphyr::{Options, Spanned};

/// One value of a document, and the line it starts on (1-based).
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub line: usize,
    pub value: Value,
}

/// A YAML value. Mapping keys keep their own type: `200:` is a number.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Seq(Vec<Node>),
    Map(Vec<(Node, Node)>),
}

/// Why a description could not be read, or what in it was not read, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub line: Option<usize>,
    pub message: String,
}

impl Problem {
    pub fn at(line: usize, message: impl Into<String>) -> Self {
        Problem {
            line: Some(line),
            message: message.into(),
        }
    }
}

/// Parse YAML 1.2, which includes JSON.
pub fn parse(text: &str) -> Result<Node, Problem> {
    let mut options = Options::default();
    options.with_snippet = false;
    serde_saphyr::from_str_with_options::<Node>(text, options).map_err(|e| Problem {
        line: e.location().map(|l| l.line() as usize),
        message: clean(&e.to_string()),
    })
}

/// `error: line 3 column 7: unclosed bracket '['` → `unclosed bracket '['`: the line is reported on its own.
fn clean(message: &str) -> String {
    let first = message.lines().next().unwrap_or("").trim();
    let rest = first.strip_prefix("error:").unwrap_or(first).trim();
    rest.strip_prefix("line ")
        .and_then(|r| r.split_once(": "))
        .map_or(rest, |(_, m)| m)
        .to_string()
}

impl Node {
    /// The value under `key` in a mapping.
    pub fn get(&self, key: &str) -> Option<&Node> {
        self.entries().find(|(k, _, _)| k == key).map(|(_, _, v)| v)
    }

    /// The entries of a mapping, in the file's order: key text, key node, value. Keys that are not scalars are skipped.
    pub fn entries(&self) -> impl Iterator<Item = (String, &Node, &Node)> {
        let pairs: &[(Node, Node)] = match &self.value {
            Value::Map(m) => m,
            _ => &[],
        };
        pairs.iter().filter_map(|(k, v)| Some((k.text()?, k, v)))
    }

    /// The items of a sequence; none for anything else.
    pub fn items(&self) -> &[Node] {
        match &self.value {
            Value::Seq(items) => items,
            _ => &[],
        }
    }

    /// A scalar as text: strings as they are, numbers and booleans as written (`2.0` stays `2.0`).
    pub fn text(&self) -> Option<String> {
        match &self.value {
            Value::Str(s) => Some(s.clone()),
            Value::Int(i) => Some(i.to_string()),
            Value::Float(x) if x.is_finite() && x.fract() == 0.0 => Some(format!("{x:.1}")),
            Value::Float(x) => Some(x.to_string()),
            Value::Bool(b) => Some(b.to_string()),
            _ => None,
        }
    }

    /// A text that says something: trimmed, not empty.
    pub fn words(&self) -> Option<String> {
        self.text()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// The node a local reference names, e.g. `#/components/schemas/Order`.
    /// None for a reference into another file, or one that names nothing.
    pub fn resolve(&self, reference: &str) -> Option<&Node> {
        let pointer = reference.strip_prefix('#')?;
        if pointer.is_empty() {
            return Some(self);
        }
        let mut cur = self;
        for seg in pointer.strip_prefix('/')?.split('/') {
            let seg = seg.replace("~1", "/").replace("~0", "~");
            cur = match &cur.value {
                Value::Map(_) => cur.get(&seg)?,
                Value::Seq(items) => items.get(seg.parse::<usize>().ok()?)?,
                _ => return None,
            };
        }
        Some(cur)
    }

    /// Follow local `$ref`s from this node, a few at most (they can form cycles).
    pub fn follow<'a>(&'a self, root: &'a Node) -> &'a Node {
        let mut cur = self;
        for _ in 0..8 {
            match cur
                .get("$ref")
                .and_then(Node::text)
                .and_then(|r| root.resolve(&r))
            {
                Some(next) => cur = next,
                None => break,
            }
        }
        cur
    }
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Node, D::Error> {
        let s = Spanned::<Value>::deserialize(d)?;
        Ok(Node {
            line: s.referenced.line() as usize,
            value: s.value,
        })
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Value, D::Error> {
        d.deserialize_any(AnyValue)
    }
}

struct AnyValue;

impl<'de> Visitor<'de> for AnyValue {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any YAML value")
    }
    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        Value::deserialize(d)
    }
    fn visit_bool<E>(self, b: bool) -> Result<Value, E> {
        Ok(Value::Bool(b))
    }
    fn visit_i64<E>(self, i: i64) -> Result<Value, E> {
        Ok(Value::Int(i))
    }
    fn visit_u64<E>(self, i: u64) -> Result<Value, E> {
        Ok(i64::try_from(i).map_or(Value::Float(i as f64), Value::Int))
    }
    fn visit_f64<E>(self, x: f64) -> Result<Value, E> {
        Ok(Value::Float(x))
    }
    fn visit_str<E>(self, s: &str) -> Result<Value, E> {
        Ok(Value::Str(s.to_string()))
    }
    fn visit_string<E>(self, s: String) -> Result<Value, E> {
        Ok(Value::Str(s))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Value, A::Error> {
        let mut items = Vec::new();
        while let Some(n) = a.next_element::<Node>()? {
            items.push(n);
        }
        Ok(Value::Seq(items))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Value, A::Error> {
        let mut pairs = Vec::new();
        while let Some(pair) = a.next_entry::<Node, Node>()? {
            pairs.push(pair);
        }
        Ok(Value::Map(pairs))
    }
}

/// The name a reference gives: `#/components/schemas/Order` → `Order`, `./schemas/order.yaml` → `order`.
pub fn ref_name(reference: &str) -> Option<String> {
    let (file, pointer) = reference.split_once('#').unwrap_or((reference, ""));
    if let Some(last) = pointer.rsplit('/').next().filter(|s| !s.is_empty()) {
        return Some(last.replace("~1", "/").replace("~0", "~"));
    }
    let stem = file.rsplit('/').next()?.split('.').next()?;
    (!stem.is_empty()).then(|| stem.to_string())
}

/// What a schema is called, in the words a line can carry: `Order`, `Order[]`, `A | B`, `string`.
/// None for an object without a name.
pub fn type_name(schema: &Node) -> Option<String> {
    type_name_at(schema, 0)
}

fn type_name_at(s: &Node, depth: usize) -> Option<String> {
    if depth > 8 {
        return None;
    }
    if let Some(r) = s.get("$ref").and_then(Node::text) {
        return ref_name(&r);
    }
    if let Some(title) = s.get("title").and_then(Node::words) {
        return Some(title);
    }
    let types: Vec<String> = match s.get("type") {
        Some(t) if !t.items().is_empty() => t.items().iter().filter_map(Node::text).collect(),
        Some(t) => t.text().into_iter().collect(),
        None => Vec::new(),
    };
    if types.iter().any(|t| t == "array") {
        return s
            .get("items")
            .and_then(|i| type_name_at(i, depth + 1))
            .map(|n| format!("{n}[]"));
    }
    for (key, sep) in [("oneOf", " | "), ("anyOf", " | "), ("allOf", " & ")] {
        if let Some(list) = s.get(key) {
            let names: Vec<String> = list
                .items()
                .iter()
                .filter_map(|i| type_name_at(i, depth + 1))
                .collect();
            if !names.is_empty() {
                return Some(names.join(sep));
            }
        }
    }
    types.into_iter().find(|t| t != "null" && t != "object")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_and_json_give_lines_numbers_and_unquoted_keys() {
        let yaml = parse("swagger: 2.0\npaths:\n  /orders:\n    post:\n      responses:\n        200:\n          description: ok\n").unwrap();
        assert_eq!(yaml.get("swagger").unwrap().text().as_deref(), Some("2.0"));
        let (method, key, _) = yaml
            .get("paths")
            .unwrap()
            .get("/orders")
            .unwrap()
            .entries()
            .next()
            .unwrap();
        assert_eq!((method.as_str(), key.line), ("post", 4));
        let responses = yaml.resolve("#/paths/~1orders/post/responses").unwrap();
        assert_eq!(responses.entries().next().unwrap().0, "200");

        let json = parse("{\n  \"openapi\": \"3.0.3\",\n  \"paths\": {\"/x\": {\"get\": {}}}\n}\n")
            .unwrap();
        assert_eq!(json.get("openapi").unwrap().line, 2);
        assert!(json.resolve("#/paths/~1x/get").is_some());
        assert!(json.resolve("other.yaml#/x").is_none());
    }

    #[test]
    fn a_broken_file_says_where() {
        let p = parse("openapi: 3.0.0\npaths:\n  /x: [unclosed\n").unwrap_err();
        assert_eq!(p.line, Some(3));
        assert!(!p.message.starts_with("error"), "{}", p.message);
    }

    #[test]
    fn schemas_are_named_like_a_line_carries_them() {
        let s = |y: &str| type_name(&parse(y).unwrap());
        assert_eq!(
            s("$ref: '#/components/schemas/Order'").as_deref(),
            Some("Order")
        );
        assert_eq!(s("$ref: './schemas/order.yaml'").as_deref(), Some("order"));
        assert_eq!(
            s("type: array\nitems: {$ref: '#/definitions/Charge'}").as_deref(),
            Some("Charge[]")
        );
        assert_eq!(
            s("oneOf: [{$ref: '#/a/Card'}, {$ref: '#/a/Bank'}]").as_deref(),
            Some("Card | Bank")
        );
        assert_eq!(s("type: [string, 'null']").as_deref(), Some("string"));
        assert_eq!(
            s("title: NewOrder\ntype: object").as_deref(),
            Some("NewOrder")
        );
        assert_eq!(s("type: object\nproperties: {id: {type: string}}"), None);
    }
}
