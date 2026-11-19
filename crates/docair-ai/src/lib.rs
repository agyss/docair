//! docair-ai: ask an AI model how the system works, get a walkthrough back (SPEC §10).
//!
//! [`AiConfig::resolve`] reads the settings, [`ask`] sends the question with a
//! digest of the graph and returns the walkthrough, bound to the graph.

pub mod ask;
pub mod client;
pub mod config;
/***
 * The AI explains what two commits change: links, explanations and workflows, bound against both graphs (docs/EXPLAIN.md).
 ***/
pub mod explain;
/***
 * What a question points at: parts, lines, data and terms the reader picked, turned into words for the model.
 ***/
pub mod ask_context;
pub mod glossary;
pub mod word;

pub use ask::{Reply, Turn, ViewChange, ask, check, digest};
pub use client::Usage;
pub use config::{AiConfig, Env};
pub use word::{Worded, word};
