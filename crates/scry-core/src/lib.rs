//! Core engine for scry: repo identity, file walking, hashing, config.

pub mod chat;
pub mod chunker;
pub mod config;
pub mod embed;
pub mod error;
pub mod hashing;
pub mod index;
pub mod memory;
pub mod repo;
pub mod search;
pub mod store;
pub mod walk;

pub use error::{Error, Result};
