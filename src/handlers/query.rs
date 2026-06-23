//! Connection checks and query execution.
//!
//! Queries are either the host's collection-browse `SELECT * FROM coll` (auto
//! translated to a paginated `find`) or native MongoDB shell calls
//! (`db.coll.op(args)`), parsed by `utils::query_parse`.

use bson::{doc, Bson, Document};
use serde_json::{json, Value};

use crate::client;
use crate::error::PluginError;
use crate::handlers::{database_of, require_str};
use crate::models::connection_params;
use crate::utils::bsonjson::{bson_to_json, parse_bson, parse_document};
use crate::utils::pagination::offset;
use crate::utils::query_parse::{parse, Statement};
use crate::utils::sqlwhere;
use crate::utils::typeinfer;

pub fn ping(top: &Value) -> Result<Value, PluginError> {
    client::ping(&connection_params(top))?;
    Ok(Value::Null)
}

pub fn test_connection(top: &Value) -> Result<Value, PluginError> {
    client::ping(&connection_params(top))?;
    Ok(json!({ "success": true }))
}

pub fn execute_query(top: &Value) -> Result<Value, PluginError> {
    let query = require_str(top, "query")?.to_string();
    let page = top.get("page").and_then(Value::as_u64).unwrap_or(1).max(1);
    let page_size = top.get("limit").and_then(Value::as_u64);
    let params = connection_params(top);
    let db = database_of(top)?;

    let stmt = parse(&query).map_err(PluginError::invalid_params)?;

    match stmt {
        Statement::Browse {
            collection,
            filter,
            sort,
        } => {
            let ps = page_size.unwrap_or(100);
            let mongo_filter = match filter {
                Some(w) => sqlwhere::parse_where(&w).map_err(PluginError::invalid_params)?,
                None => Document::new(),
            };
            let mongo_sort = sort
                .map(|o| sqlwhere::parse_order_by(&o))
                .filter(|d| !d.is_empty());
            let total = client::count(&params, &db, &collection, mongo_filter.clone())?;
            let docs = client::find(
                &params,
                &db,
                &collection,
                mongo_filter,
                Some(offset(page, ps)),
                Some(ps as i64),
                mongo_sort,
                None,
            )?;
            Ok(rows_result(&docs, page, ps, Some(total)))
        }
        Statement::Shell { target, op, args } => {
            dispatch_shell(&params, &db, &target, &op, &args, page, page_size)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn dispatch_shell(
    params: &crate::models::ConnectionParams,
    db: &str,
    target: &str,
    op: &str,
    args: &[String],
    page: u64,
    page_size: Option<u64>,
) -> Result<Value, PluginError> {
    let need_coll = || -> Result<&str, PluginError> {
        if target.is_empty() {
            Err(PluginError::invalid_params(format!(
                "`{op}` requires a collection: db.<collection>.{op}(...)"
            )))
        } else {
            Ok(target)
        }
    };

    match op {
        "find" | "findOne" => {
            let coll = need_coll()?;
            let filter = arg_doc(args, 0)?;
            let projection = arg_doc_opt(args, 1)?;
            let ps = page_size.unwrap_or(100);
            let (skip, limit) = if op == "findOne" {
                (None, Some(1))
            } else {
                (Some(offset(page, ps)), Some(ps as i64))
            };
            let total = if op == "findOne" {
                None
            } else {
                Some(client::count(params, db, coll, filter.clone())?)
            };
            let docs = client::find(params, db, coll, filter, skip, limit, None, projection)?;
            Ok(rows_result(&docs, page, ps, total))
        }
        "aggregate" => {
            let coll = need_coll()?;
            let mut pipeline = arg_pipeline(args, 0)?;
            // Paginate after the user's stages when the host requests a page.
            if let Some(ps) = page_size {
                pipeline.push(doc! { "$skip": offset(page, ps) as i64 });
                pipeline.push(doc! { "$limit": ps as i64 });
            }
            let docs = client::aggregate(params, db, coll, pipeline)?;
            let ps = page_size.unwrap_or(docs.len() as u64);
            Ok(rows_result(&docs, page, ps, None))
        }
        "countDocuments" | "count" => {
            let coll = need_coll()?;
            let filter = arg_doc(args, 0)?;
            let n = client::count(params, db, coll, filter)?;
            Ok(scalar_result("count", vec![json!(n)]))
        }
        "distinct" => {
            let coll = need_coll()?;
            let field = arg_string(args, 0)?;
            let filter = arg_doc_opt(args, 1)?.unwrap_or_default();
            let values = client::distinct(params, db, coll, &field, filter)?;
            let json_vals = values.iter().map(bson_to_json).collect();
            Ok(scalar_result(&field, json_vals))
        }
        "insertOne" => {
            let coll = need_coll()?;
            let d = arg_doc(args, 0)?;
            Ok(affected_result(client::insert_many(
                params,
                db,
                coll,
                vec![d],
            )?))
        }
        "insertMany" => {
            let coll = need_coll()?;
            let docs = arg_doc_array(args, 0)?;
            Ok(affected_result(client::insert_many(
                params, db, coll, docs,
            )?))
        }
        "updateOne" | "updateMany" => {
            let coll = need_coll()?;
            let filter = arg_doc(args, 0)?;
            let update = arg_doc(args, 1)?;
            let n = client::update(params, db, coll, filter, update, op == "updateMany")?;
            Ok(affected_result(n))
        }
        "replaceOne" => {
            let coll = need_coll()?;
            let filter = arg_doc(args, 0)?;
            let replacement = arg_doc(args, 1)?;
            Ok(affected_result(client::replace_one(
                params,
                db,
                coll,
                filter,
                replacement,
            )?))
        }
        "deleteOne" | "deleteMany" => {
            let coll = need_coll()?;
            let filter = arg_doc(args, 0)?;
            let n = client::delete(params, db, coll, filter, op == "deleteMany")?;
            Ok(affected_result(n))
        }
        "drop" => {
            let coll = need_coll()?;
            client::drop_collection(params, db, coll)?;
            Ok(affected_result(0))
        }
        "createCollection" => {
            let name = arg_string(args, 0)?;
            client::create_collection(params, db, &name)?;
            Ok(affected_result(0))
        }
        "createIndex" => {
            let coll = need_coll()?;
            let keys = arg_doc(args, 0)?;
            let opts = arg_doc_opt(args, 1)?.unwrap_or_default();
            let n = create_index(params, db, coll, keys, opts)?;
            Ok(affected_result(n))
        }
        "runCommand" => {
            let cmd = arg_doc(args, 0)?;
            let result = client::run_command(params, db, cmd)?;
            Ok(rows_result(&[result], 1, 1, None))
        }
        other => Err(PluginError::invalid_params(format!(
            "unsupported MongoDB operation: '{other}'"
        ))),
    }
}

/// Builds and runs a `createIndexes` command; returns 1 on success.
fn create_index(
    params: &crate::models::ConnectionParams,
    db: &str,
    coll: &str,
    keys: Document,
    opts: Document,
) -> Result<u64, PluginError> {
    let name = opts
        .get_str("name")
        .map(str::to_string)
        .unwrap_or_else(|_| index_name_from_keys(&keys));
    let mut spec = doc! { "key": keys, "name": name };
    for (k, v) in opts {
        if k != "name" {
            spec.insert(k, v);
        }
    }
    client::run_command(
        params,
        db,
        doc! { "createIndexes": coll, "indexes": vec![spec] },
    )?;
    Ok(1)
}

fn index_name_from_keys(keys: &Document) -> String {
    keys.iter()
        .map(|(k, v)| {
            let dir = v.as_i32().unwrap_or(1);
            format!("{k}_{dir}")
        })
        .collect::<Vec<_>>()
        .join("_")
}

// --- argument helpers -----------------------------------------------------

fn arg_doc(args: &[String], idx: usize) -> Result<Document, PluginError> {
    match args.get(idx) {
        Some(s) => parse_document(s).map_err(PluginError::invalid_params),
        None if idx == 0 => Ok(Document::new()),
        None => Err(PluginError::invalid_params(format!(
            "missing argument #{}",
            idx + 1
        ))),
    }
}

fn arg_doc_opt(args: &[String], idx: usize) -> Result<Option<Document>, PluginError> {
    match args.get(idx) {
        Some(s) => parse_document(s)
            .map(Some)
            .map_err(PluginError::invalid_params),
        None => Ok(None),
    }
}

fn arg_doc_array(args: &[String], idx: usize) -> Result<Vec<Document>, PluginError> {
    let s = args
        .get(idx)
        .ok_or_else(|| PluginError::invalid_params("expected an array of documents"))?;
    match parse_bson(s).map_err(PluginError::invalid_params)? {
        Bson::Array(items) => Ok(items
            .into_iter()
            .filter_map(|b| match b {
                Bson::Document(d) => Some(d),
                _ => None,
            })
            .collect()),
        _ => Err(PluginError::invalid_params("expected a JSON array")),
    }
}

fn arg_pipeline(args: &[String], idx: usize) -> Result<Vec<Document>, PluginError> {
    arg_doc_array(args, idx)
}

fn arg_string(args: &[String], idx: usize) -> Result<String, PluginError> {
    let s = args
        .get(idx)
        .ok_or_else(|| PluginError::invalid_params(format!("missing argument #{}", idx + 1)))?;
    match parse_bson(s).map_err(PluginError::invalid_params)? {
        Bson::String(v) => Ok(v),
        other => Ok(bson_to_json(&other)
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| s.trim_matches(['"', '\'']).to_string())),
    }
}

// --- result builders ------------------------------------------------------

fn ordered_columns(docs: &[Document]) -> Vec<String> {
    typeinfer::infer_columns(docs)
        .into_iter()
        .map(|(n, _)| n)
        .collect()
}

fn rows_result(docs: &[Document], page: u64, page_size: u64, total: Option<u64>) -> Value {
    let cols = ordered_columns(docs);
    let rows: Vec<Vec<Value>> = docs
        .iter()
        .map(|d| {
            cols.iter()
                .map(|c| d.get(c).map(bson_to_json).unwrap_or(Value::Null))
                .collect()
        })
        .collect();
    let returned = rows.len() as u64;
    let has_more = match total {
        Some(t) => page.saturating_mul(page_size) < t,
        None => page_size > 0 && returned == page_size,
    };
    json!({
        "columns": cols,
        "rows": rows,
        "affected_rows": returned,
        "truncated": false,
        "pagination": {
            "page": page,
            "page_size": page_size,
            "total_rows": total,
            "has_more": has_more,
        }
    })
}

fn affected_result(n: u64) -> Value {
    json!({
        "columns": [],
        "rows": [],
        "affected_rows": n,
        "truncated": false,
        "pagination": Value::Null,
    })
}

fn scalar_result(column: &str, values: Vec<Value>) -> Value {
    let rows: Vec<Vec<Value>> = values.into_iter().map(|v| vec![v]).collect();
    let n = rows.len() as u64;
    json!({
        "columns": [column],
        "rows": rows,
        "affected_rows": n,
        "truncated": false,
        "pagination": Value::Null,
    })
}
