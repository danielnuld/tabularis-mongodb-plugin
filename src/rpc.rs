//! JSON-RPC dispatch and response helpers.

use serde_json::{json, Value};

use crate::config::{self, Config};
use crate::error::PluginError;
use crate::handlers;

/// Parses one JSON-RPC line and returns the response value. Never panics.
pub fn handle_line(line: &str) -> Value {
    let request: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(err) => return error_response(Value::Null, -32700, &format!("parse error: {err}")),
    };

    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    match method.as_str() {
        "initialize" => {
            let settings = params.get("settings").cloned().unwrap_or(Value::Null);
            config::set(Config::from_settings(&settings));
            ok_response(id, Value::Null)
        }
        "ping" => respond(id, handlers::query::ping(&params)),
        "test_connection" => respond(id, handlers::query::test_connection(&params)),

        // Schema discovery.
        "get_databases" => respond(id, handlers::metadata::get_databases(&params)),
        "get_schemas" => ok_response(id, json!([])),
        "get_tables" => respond(id, handlers::metadata::get_tables(&params)),
        "get_columns" => respond(id, handlers::metadata::get_columns(&params)),
        "get_foreign_keys" => respond(id, handlers::metadata::get_foreign_keys(&params)),
        "get_indexes" => respond(id, handlers::metadata::get_indexes(&params)),

        // Views.
        "get_views" => respond(id, handlers::metadata::get_views(&params)),
        "get_view_definition" => respond(id, handlers::metadata::get_view_definition(&params)),
        "get_view_columns" => respond(id, handlers::metadata::get_view_columns(&params)),

        // Routines (not applicable to MongoDB).
        "get_routines" => respond(id, handlers::metadata::get_routines(&params)),
        "get_routine_parameters" => ok_response(id, json!([])),
        "get_routine_definition" => {
            respond(id, handlers::metadata::get_routine_definition(&params))
        }

        // Batch / ER diagram.
        "get_schema_snapshot" => respond(id, handlers::metadata::get_schema_snapshot(&params)),
        "get_all_columns_batch" => respond(id, handlers::metadata::get_all_columns_batch(&params)),
        "get_all_foreign_keys_batch" => {
            respond(id, handlers::metadata::get_all_foreign_keys_batch(&params))
        }

        // Query execution.
        "execute_query" => respond(id, handlers::query::execute_query(&params)),

        // CRUD.
        "insert_record" => respond(id, handlers::crud::insert_record(&params)),
        "update_record" => respond(id, handlers::crud::update_record(&params)),
        "delete_record" => respond(id, handlers::crud::delete_record(&params)),

        // DDL.
        "get_create_table_sql" => respond(id, handlers::ddl::get_create_table_sql(&params)),
        "get_add_column_sql" => respond(id, handlers::ddl::get_add_column_sql(&params)),
        "get_alter_column_sql" => respond(id, handlers::ddl::get_alter_column_sql(&params)),
        "get_create_index_sql" => respond(id, handlers::ddl::get_create_index_sql(&params)),
        "get_create_foreign_key_sql" => {
            respond(id, handlers::ddl::get_create_foreign_key_sql(&params))
        }
        "drop_index" => respond(id, handlers::ddl::drop_index(&params)),
        "drop_foreign_key" => respond(id, handlers::ddl::drop_foreign_key(&params)),

        other => not_implemented(id, other),
    }
}

pub fn respond(id: Value, result: Result<Value, PluginError>) -> Value {
    match result {
        Ok(value) => ok_response(id, value),
        Err(err) => error_response(id, err.code, &err.message),
    }
}

pub fn ok_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "result": result, "id": id })
}

pub fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "error": { "code": code, "message": message }, "id": id })
}

pub fn not_implemented(id: Value, method: &str) -> Value {
    error_response(
        id,
        -32601,
        &format!("method '{method}' is not implemented by this plugin"),
    )
}
