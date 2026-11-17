//! What an API description says, as read from its file.

use crate::tree::Problem;

/// The kind of description.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// OpenAPI 3.x, or Swagger 2.0.
    OpenApi,
    AsyncApi,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::OpenApi => "OpenAPI",
            Format::AsyncApi => "AsyncAPI",
        }
    }
}

/// One API description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Api {
    pub format: Format,
    /// As the file writes it: `3.1.0`, `2.0`, `2.6.0`.
    pub version: String,
    pub title: String,
    pub description: Option<String>,
    /// `x-docair-part-of`: the node the description belongs to.
    pub part_of: Option<String>,
    /// The line of the version key.
    pub line: usize,
    /// OpenAPI: the routes the described service answers.
    pub operations: Vec<Operation>,
    /// AsyncAPI: what the described application sends and receives.
    pub events: Vec<Event>,
    /// AsyncAPI: the protocols of the servers, e.g. `kafka`.
    pub protocols: Vec<String>,
    /// Parts of the description docair does not read, and where they are.
    pub skipped: Vec<Problem>,
}

impl Api {
    pub fn new(format: Format, version: String, line: usize) -> Self {
        Api {
            format,
            version,
            title: String::new(),
            description: None,
            part_of: None,
            line,
            operations: Vec::new(),
            events: Vec::new(),
            protocols: Vec::new(),
            skipped: Vec::new(),
        }
    }
}

/// An OpenAPI operation: a route the described service answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    /// Upper case: `POST`.
    pub method: String,
    /// The path as the description writes it, e.g. `/orders/{id}`.
    pub path: String,
    /// The path of the servers in front of it, e.g. `/api`; empty when there is none.
    pub base: String,
    /// The `operationId`.
    pub id: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    /// What the request sends: the body's type, else the names of its path and query parameters.
    pub payload: Option<String>,
    /// What a successful answer carries.
    pub reply: Option<String>,
    /// The line of the method key.
    pub line: usize,
}

impl Operation {
    /// The route a client calls: base and path.
    pub fn route(&self) -> String {
        join(&self.base, &self.path)
    }
}

/// Whether the described application sends to a channel or receives from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Send,
    Receive,
}

/// AsyncAPI: the described application sends to, or receives from, a channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub action: Action,
    /// Where messages travel: the channel's address, e.g. `order.created`.
    pub address: String,
    /// What travels: the message's name, e.g. `OrderCreated`, or `A | B`.
    pub message: Option<String>,
    /// The `operationId`.
    pub id: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub line: usize,
}

/// A base path and a path, as one route: `/api` + `/orders` = `/api/orders`.
pub fn join(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    if path.is_empty() || path == "/" {
        return if base.is_empty() {
            "/".to_string()
        } else {
            base.to_string()
        };
    }
    if path.starts_with('/') {
        format!("{base}{path}")
    } else {
        format!("{base}/{path}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_join_base_and_path() {
        assert_eq!(join("/api", "/orders"), "/api/orders");
        assert_eq!(join("/api/", "/orders"), "/api/orders");
        assert_eq!(join("", "/orders"), "/orders");
        assert_eq!(join("/api", "/"), "/api");
        assert_eq!(join("", ""), "/");
    }
}
