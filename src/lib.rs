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

#[cfg(test)]
pub(crate) mod test_support {
    pub fn must<T, E: std::fmt::Debug>(result: Result<T, E>) -> T {
        match result {
            Ok(value) => value,
            Err(error) => panic!("expected Ok, got Err({error:?})"),
        }
    }

    pub fn present<T>(value: Option<T>) -> T {
        match value {
            Some(value) => value,
            None => panic!("expected Some, got None"),
        }
    }
}
