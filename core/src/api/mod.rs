//! Shared API types for CLI ↔ GUI communication.
//!
//! These types define the JSON schema for CLI output and Tauri parsing.
//! Both sides use the same types for serialization/deserialization,
//! eliminating stringly-typed manual JSON parsing.

mod responses;
mod errors;

pub use responses::*;
pub use errors::*;
