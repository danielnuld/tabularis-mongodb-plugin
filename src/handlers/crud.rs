//! Row-level CRUD on documents, keyed by the primary field (`_id`).
//!
//! Tabularis sends `_id` back as the hex string we emit for ObjectIds, so when
//! the primary key is `_id` and the value looks like a 24-char hex string it is
//! converted back to an ObjectId for matching.

use std::str::FromStr;

use bson::{doc, Bson, Document};
use serde_json::{json, Value};

use crate::client;
use crate::error::PluginError;
use crate::handlers::{database_of, require_str};
use crate::models::connection_params;
use crate::utils::bsonjson::json_to_bson;

pub fn insert_record(top: &Value) -> Result<Value, PluginError> {
    let params = connection_params(top);
    let db = database_of(top)?;
    let table = require_str(top, "table")?;
    let data = top
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| PluginError::invalid_params("insert_record requires a 'data' object"))?;

    let mut document = Document::new();
    for (k, v) in data {
        document.insert(k.clone(), json_to_bson(v));
    }

    let n = client::insert_many(&params, &db, table, vec![document])?;
    Ok(json!(n))
}

pub fn update_record(top: &Value) -> Result<Value, PluginError> {
    let params = connection_params(top);
    let db = database_of(top)?;
    let table = require_str(top, "table")?;
    let pk_col = require_str(top, "pk_col")?;
    let col_name = require_str(top, "col_name")?;
    let pk_val = top.get("pk_val").cloned().unwrap_or(Value::Null);
    let new_val = top.get("new_val").cloned().unwrap_or(Value::Null);

    let filter = doc! { pk_col: pk_to_bson(pk_col, &pk_val) };
    let update = doc! { "$set": { col_name: json_to_bson(&new_val) } };
    let n = client::update(&params, &db, table, filter, update, false)?;
    Ok(json!(n))
}

pub fn delete_record(top: &Value) -> Result<Value, PluginError> {
    let params = connection_params(top);
    let db = database_of(top)?;
    let table = require_str(top, "table")?;
    let pk_col = require_str(top, "pk_col")?;
    let pk_val = top.get("pk_val").cloned().unwrap_or(Value::Null);

    let filter = doc! { pk_col: pk_to_bson(pk_col, &pk_val) };
    let n = client::delete(&params, &db, table, filter, false)?;
    Ok(json!(n))
}

/// Converts a primary-key value to BSON. For `_id` values that look like a
/// 24-character hex ObjectId, rebuilds the ObjectId so it matches the stored
/// document.
fn pk_to_bson(pk_col: &str, pk_val: &Value) -> Bson {
    if pk_col == "_id" {
        if let Value::String(s) = pk_val {
            if is_object_id_hex(s) {
                if let Ok(oid) = bson::oid::ObjectId::from_str(s) {
                    return Bson::ObjectId(oid);
                }
            }
        }
    }
    json_to_bson(pk_val)
}

fn is_object_id_hex(s: &str) -> bool {
    s.len() == 24 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_hex_string_becomes_objectid() {
        let v = pk_to_bson("_id", &json!("507f1f77bcf86cd799439011"));
        assert!(matches!(v, Bson::ObjectId(_)));
    }

    #[test]
    fn non_hex_id_stays_as_is() {
        assert!(matches!(
            pk_to_bson("_id", &json!("alice")),
            Bson::String(_)
        ));
        assert!(matches!(pk_to_bson("_id", &json!(42)), Bson::Int32(42)));
    }

    #[test]
    fn other_columns_are_literal() {
        // A 24-hex value on a non-_id column must stay a string.
        let v = pk_to_bson("code", &json!("507f1f77bcf86cd799439011"));
        assert!(matches!(v, Bson::String(_)));
    }
}
