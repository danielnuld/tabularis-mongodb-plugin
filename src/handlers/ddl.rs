//! DDL for MongoDB.
//!
//! MongoDB collections are schemaless, so SQL-style table/column DDL does not
//! apply (`manage_tables` is off). Index management does map: index creation is
//! exposed as a native `createIndex` statement the host runs back through
//! `execute_query`, and `drop_index` runs `dropIndexes` directly.

use bson::doc;
use serde_json::{json, Value};

use crate::client;
use crate::error::PluginError;
use crate::handlers::{database_of, require_str};
use crate::models::connection_params;

fn unsupported(method: &str) -> Result<Value, PluginError> {
    Err(PluginError {
        code: -32601,
        message: format!(
            "'{method}' does not apply to MongoDB (schemaless collections); \
             use native commands via the query editor"
        ),
    })
}

pub fn get_create_table_sql(_top: &Value) -> Result<Value, PluginError> {
    unsupported("get_create_table_sql")
}

pub fn get_add_column_sql(_top: &Value) -> Result<Value, PluginError> {
    unsupported("get_add_column_sql")
}

pub fn get_alter_column_sql(_top: &Value) -> Result<Value, PluginError> {
    unsupported("get_alter_column_sql")
}

pub fn get_create_foreign_key_sql(_top: &Value) -> Result<Value, PluginError> {
    unsupported("get_create_foreign_key_sql")
}

pub fn drop_foreign_key(_top: &Value) -> Result<Value, PluginError> {
    unsupported("drop_foreign_key")
}

/// Emits a native `db.<coll>.createIndex(keys, options)` statement, which the
/// host then runs through `execute_query`.
pub fn get_create_index_sql(top: &Value) -> Result<Value, PluginError> {
    let table = require_str(top, "table")?;
    let index_name = require_str(top, "index_name")?;
    let is_unique = top
        .get("is_unique")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let columns: Vec<String> = top
        .get("columns")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| format!("\"{s}\": 1")))
                .collect()
        })
        .unwrap_or_default();
    let keys = columns.join(", ");
    let sql = format!(
        "db.{table}.createIndex({{ {keys} }}, {{ \"name\": \"{index_name}\", \"unique\": {is_unique} }})"
    );
    Ok(json!([sql]))
}

pub fn drop_index(top: &Value) -> Result<Value, PluginError> {
    let params = connection_params(top);
    let db = database_of(top)?;
    let table = require_str(top, "table")?;
    let index_name = require_str(top, "index_name")?;
    client::run_command(
        &params,
        &db,
        doc! { "dropIndexes": table, "index": index_name },
    )?;
    Ok(Value::Null)
}
