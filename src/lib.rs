pub mod api;
pub mod auth;
pub mod cli;
pub mod comments;
pub mod cookie;
pub mod crawler;
mod fs_utils;
pub mod insights;
pub mod mcp;
mod net;
pub mod obscura;
pub mod openapi;
pub mod settings;
pub mod stats;

/// Shorthand for the ubiquitous `map_err(|error| error.to_string())`.
pub(crate) fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}
