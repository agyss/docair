//! docair-ai: ask an AI model how the system works, get a walkthrough back (SPEC §10).
//!
//! [`AiConfig::resolve`] reads the settings, [`ask`] sends the question with a
//! digest of the graph and returns the walkthrough, bound to the graph.

pub mod ask;
pub mod client;
pub mod config;
pub mod glossary;
pub mod word;

pub use ask::{Reply, Turn, ViewChange, ask, check, digest};
pub use client::Usage;
pub use config::{AiConfig, Env};
pub use word::{Worded, word};
