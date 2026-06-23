//! Schema metadata for MongoDB.
//!
//! Databases map to databases, collections to "tables", and document fields to
//! "columns" inferred by sampling. MongoDB has no foreign keys or stored
//! routines, so those return empty.

use bson::Document;
use serde_json::{json, Value};

use crate::client::{self, CollectionMeta};
use crate::config;
use crate::error::PluginError;
use crate::handlers::{database_of, require_str};
use crate::models::{connection_params, ConnectionParams};
use crate::utils::bsonjson::document_to_json;
use crate::utils::typeinfer::infer_columns;

pub fn get_databases(top: &Value) -> Result<Value, PluginError> {
    let names = client::get_databases(&connection_params(top))?;
    Ok(json!(names))
}

pub fn get_tables(top: &Value) -> Result<Value, PluginError> {
    let params = connection_params(top);
    let db = database_of(top)?;
    let tables: Vec<Value> = client::get_collections(&params, &db)?
        .into_iter()
        .filter(|c| !c.is_view)
        .map(|c| json!({ "name": c.name }))
        .collect();
    Ok(json!(tables))
}

pub fn get_columns(top: &Value) -> Result<Value, PluginError> {
    let params = connection_params(top);
    let db = database_of(top)?;
    let table = require_str(top, "table")?;
    Ok(json!(columns_of(&params, &db, table)?))
}

pub fn get_foreign_keys(_top: &Value) -> Result<Value, PluginError> {
    Ok(json!([]))
}

pub fn get_indexes(top: &Value) -> Result<Value, PluginError> {
    let params = connection_params(top);
    let db = database_of(top)?;
    let table = require_str(top, "table")?;
    let indexes = client::list_indexes(&params, &db, table)?;

    let mut rows = Vec::new();
    for im in &indexes {
        let name = im
            .options
            .as_ref()
            .and_then(|o| o.name.clone())
            .unwrap_or_else(|| "index".to_string());
        let is_primary = name == "_id_";
        let unique = is_primary || im.options.as_ref().and_then(|o| o.unique).unwrap_or(false);
        let mut seq = 0i32;
        for (field, _dir) in &im.keys {
            seq += 1;
            rows.push(json!({
                "name": name,
                "column_name": field,
                "is_unique": unique,
                "is_primary": is_primary,
                "seq_in_index": seq,
            }));
        }
    }
    Ok(json!(rows))
}

pub fn get_views(top: &Value) -> Result<Value, PluginError> {
    let params = connection_params(top);
    let db = database_of(top)?;
    let views: Vec<Value> = client::get_collections(&params, &db)?
        .into_iter()
        .filter(|c| c.is_view)
        .map(|c| json!({ "name": c.name, "definition": view_definition(&c) }))
        .collect();
    Ok(json!(views))
}

pub fn get_view_definition(top: &Value) -> Result<Value, PluginError> {
    let params = connection_params(top);
    let db = database_of(top)?;
    let view = require_str(top, "view_name")?;
    let def = client::get_collections(&params, &db)?
        .into_iter()
        .find(|c| c.is_view && c.name == view)
        .map(|c| view_definition(&c))
        .unwrap_or_default();
    Ok(json!(def))
}

pub fn get_view_columns(top: &Value) -> Result<Value, PluginError> {
    let params = connection_params(top);
    let db = database_of(top)?;
    let view = require_str(top, "view_name")?;
    Ok(json!(columns_of(&params, &db, view)?))
}

pub fn get_routines(_top: &Value) -> Result<Value, PluginError> {
    Ok(json!([]))
}

pub fn get_routine_definition(_top: &Value) -> Result<Value, PluginError> {
    Ok(json!(""))
}

pub fn get_schema_snapshot(top: &Value) -> Result<Value, PluginError> {
    let params = connection_params(top);
    let db = database_of(top)?;
    let mut schemas = Vec::new();
    for c in client::get_collections(&params, &db)? {
        if c.is_view {
            continue;
        }
        let columns = columns_of(&params, &db, &c.name)?;
        schemas.push(json!({
            "name": c.name,
            "columns": columns,
            "foreign_keys": [],
        }));
    }
    Ok(json!(schemas))
}

pub fn get_all_columns_batch(top: &Value) -> Result<Value, PluginError> {
    let params = connection_params(top);
    let db = database_of(top)?;
    let mut map = serde_json::Map::new();
    for c in client::get_collections(&params, &db)? {
        if c.is_view {
            continue;
        }
        map.insert(c.name.clone(), json!(columns_of(&params, &db, &c.name)?));
    }
    Ok(Value::Object(map))
}

pub fn get_all_foreign_keys_batch(top: &Value) -> Result<Value, PluginError> {
    let params = connection_params(top);
    let db = database_of(top)?;
    let mut map = serde_json::Map::new();
    for c in client::get_collections(&params, &db)? {
        map.insert(c.name, json!([]));
    }
    Ok(Value::Object(map))
}

// --- helpers --------------------------------------------------------------

/// Samples a collection and returns `TableColumn` JSON objects.
fn columns_of(params: &ConnectionParams, db: &str, coll: &str) -> Result<Vec<Value>, PluginError> {
    let sample = config::get().sample_size;
    let docs = client::find(
        params,
        db,
        coll,
        Document::new(),
        None,
        Some(sample),
        None,
        None,
    )?;
    let inferred = infer_columns(&docs);

    if inferred.is_empty() {
        // Empty collection — still present the implicit primary key.
        return Ok(vec![column_json("_id", "OBJECTID", true)]);
    }

    Ok(inferred
        .into_iter()
        .map(|(name, ty)| {
            let is_id = name == "_id";
            column_json(&name, &ty, is_id)
        })
        .collect())
}

fn column_json(name: &str, data_type: &str, is_id: bool) -> Value {
    json!({
        "name": name,
        "data_type": data_type,
        "is_pk": is_id,
        "is_nullable": !is_id,
        "is_auto_increment": false,
    })
}

/// Renders a view's definition (its source collection + aggregation pipeline).
fn view_definition(c: &CollectionMeta) -> String {
    let on = c.view_on.clone().unwrap_or_default();
    match &c.pipeline {
        Some(p) => {
            let stages: Vec<Value> = p.iter().map(document_to_json).collect();
            let pretty = serde_json::to_string_pretty(&stages).unwrap_or_else(|_| "[]".to_string());
            format!("db.createView(\"{}\", \"{on}\", {pretty})", c.name)
        }
        None => format!("view on \"{on}\""),
    }
}
