pub mod crud;
pub mod ddl;
pub mod metadata;
pub mod query;

use serde_json::Value;

use crate::error::PluginError;
use crate::models::{connection_params, str_field};

/// Reads a required string field from the top-level params.
pub fn require_str<'a>(top: &'a Value, key: &str) -> Result<&'a str, PluginError> {
    str_field(top, key)
        .ok_or_else(|| PluginError::invalid_params(format!("missing required field '{key}'")))
}

/// The selected database (from connection params). Required for any
/// collection-scoped operation in multi-database mode.
pub fn database_of(top: &Value) -> Result<String, PluginError> {
    connection_params(top)
        .database
        .ok_or_else(|| PluginError::invalid_params("no database selected for this connection"))
}
