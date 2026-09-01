//! Core engine for scry: repo identity, file walking, hashing, config.

pub mod config;
pub mod error;
pub mod hashing;
pub mod repo;
pub mod walk;

pub use error::{Error, Result};
