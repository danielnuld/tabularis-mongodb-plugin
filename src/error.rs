//! Plugin-local error type mapped onto JSON-RPC error codes.

use std::fmt;

#[derive(Debug)]
pub struct PluginError {
    pub code: i64,
    pub message: String,
}

impl PluginError {
    pub fn internal(msg: impl Into<String>) -> Self {
        Self {
            code: -32603,
            message: msg.into(),
        }
    }

    pub fn invalid_params(msg: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: msg.into(),
        }
    }
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for PluginError {}

impl From<mongodb::error::Error> for PluginError {
    fn from(err: mongodb::error::Error) -> Self {
        PluginError::internal(format!("MongoDB error: {err}"))
    }
}
