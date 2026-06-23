//! MongoDB connection and operation layer.
//!
//! Clients are cached per connection URI (the `mongodb::Client` is cheap to
//! clone and pools connections internally). Every public function is
//! synchronous and drives the async driver on the shared Tokio runtime.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use bson::{Bson, Document};
use mongodb::results::{CollectionSpecification, CollectionType};
use mongodb::{Client, IndexModel};

use crate::config;
use crate::error::PluginError;
use crate::models::ConnectionParams;
use crate::runtime::runtime;
use crate::utils::connstr::build_uri;

/// Collection (or view) metadata from `listCollections`.
pub struct CollectionMeta {
    pub name: String,
    pub is_view: bool,
    pub view_on: Option<String>,
    pub pipeline: Option<Vec<Document>>,
}

fn clients() -> &'static Mutex<HashMap<String, Client>> {
    static C: OnceLock<Mutex<HashMap<String, Client>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

async fn client_for(params: &ConnectionParams) -> Result<Client, PluginError> {
    let uri = build_uri(params, &config::get())?;
    if let Some(c) = clients().lock().unwrap().get(&uri).cloned() {
        return Ok(c);
    }
    let client = Client::with_uri_str(&uri).await?;
    clients().lock().unwrap().insert(uri, client.clone());
    Ok(client)
}

/// Lightweight connectivity check via the `ping` admin command.
pub fn ping(params: &ConnectionParams) -> Result<(), PluginError> {
    runtime().block_on(async move {
        let client = client_for(params).await?;
        client
            .database("admin")
            .run_command(bson::doc! { "ping": 1 })
            .await?;
        Ok(())
    })
}

pub fn get_databases(params: &ConnectionParams) -> Result<Vec<String>, PluginError> {
    runtime().block_on(async move {
        let client = client_for(params).await?;
        Ok(client.list_database_names().await?)
    })
}

pub fn get_collections(
    params: &ConnectionParams,
    db: &str,
) -> Result<Vec<CollectionMeta>, PluginError> {
    runtime().block_on(async move {
        let client = client_for(params).await?;
        let mut cursor = client.database(db).list_collections().await?;
        let mut out = Vec::new();
        while cursor.advance().await? {
            let spec: CollectionSpecification = cursor.deserialize_current()?;
            // Skip MongoDB's internal collections (system.views, system.profile, ...).
            if spec.name.starts_with("system.") {
                continue;
            }
            out.push(CollectionMeta {
                name: spec.name,
                is_view: spec.collection_type == CollectionType::View,
                view_on: spec.options.view_on,
                pipeline: spec.options.pipeline,
            });
        }
        Ok(out)
    })
}

#[allow(clippy::too_many_arguments)]
pub fn find(
    params: &ConnectionParams,
    db: &str,
    coll: &str,
    filter: Document,
    skip: Option<u64>,
    limit: Option<i64>,
    sort: Option<Document>,
    projection: Option<Document>,
) -> Result<Vec<Document>, PluginError> {
    runtime().block_on(async move {
        let c = client_for(params)
            .await?
            .database(db)
            .collection::<Document>(coll);
        let mut action = c.find(filter);
        if let Some(s) = skip {
            action = action.skip(s);
        }
        if let Some(l) = limit {
            action = action.limit(l);
        }
        if let Some(s) = sort {
            action = action.sort(s);
        }
        if let Some(p) = projection {
            action = action.projection(p);
        }
        let mut cursor = action.await?;
        let mut out = Vec::new();
        while cursor.advance().await? {
            out.push(cursor.deserialize_current()?);
        }
        Ok(out)
    })
}

pub fn count(
    params: &ConnectionParams,
    db: &str,
    coll: &str,
    filter: Document,
) -> Result<u64, PluginError> {
    runtime().block_on(async move {
        let c = client_for(params)
            .await?
            .database(db)
            .collection::<Document>(coll);
        Ok(c.count_documents(filter).await?)
    })
}

pub fn aggregate(
    params: &ConnectionParams,
    db: &str,
    coll: &str,
    pipeline: Vec<Document>,
) -> Result<Vec<Document>, PluginError> {
    runtime().block_on(async move {
        let c = client_for(params)
            .await?
            .database(db)
            .collection::<Document>(coll);
        let mut cursor = c.aggregate(pipeline).await?;
        let mut out = Vec::new();
        while cursor.advance().await? {
            out.push(cursor.deserialize_current()?);
        }
        Ok(out)
    })
}

pub fn distinct(
    params: &ConnectionParams,
    db: &str,
    coll: &str,
    field: &str,
    filter: Document,
) -> Result<Vec<Bson>, PluginError> {
    runtime().block_on(async move {
        let c = client_for(params)
            .await?
            .database(db)
            .collection::<Document>(coll);
        Ok(c.distinct(field, filter).await?)
    })
}

/// Inserts one or more documents; returns the number inserted.
pub fn insert_many(
    params: &ConnectionParams,
    db: &str,
    coll: &str,
    docs: Vec<Document>,
) -> Result<u64, PluginError> {
    runtime().block_on(async move {
        let c = client_for(params)
            .await?
            .database(db)
            .collection::<Document>(coll);
        let res = c.insert_many(docs).await?;
        Ok(res.inserted_ids.len() as u64)
    })
}

/// Updates documents matching `filter` with the `update` document; returns the
/// number modified.
pub fn update(
    params: &ConnectionParams,
    db: &str,
    coll: &str,
    filter: Document,
    update: Document,
    many: bool,
) -> Result<u64, PluginError> {
    runtime().block_on(async move {
        let c = client_for(params)
            .await?
            .database(db)
            .collection::<Document>(coll);
        let modified = if many {
            c.update_many(filter, update).await?.modified_count
        } else {
            c.update_one(filter, update).await?.modified_count
        };
        Ok(modified)
    })
}

pub fn replace_one(
    params: &ConnectionParams,
    db: &str,
    coll: &str,
    filter: Document,
    replacement: Document,
) -> Result<u64, PluginError> {
    runtime().block_on(async move {
        let c = client_for(params)
            .await?
            .database(db)
            .collection::<Document>(coll);
        Ok(c.replace_one(filter, replacement).await?.modified_count)
    })
}

pub fn delete(
    params: &ConnectionParams,
    db: &str,
    coll: &str,
    filter: Document,
    many: bool,
) -> Result<u64, PluginError> {
    runtime().block_on(async move {
        let c = client_for(params)
            .await?
            .database(db)
            .collection::<Document>(coll);
        let deleted = if many {
            c.delete_many(filter).await?.deleted_count
        } else {
            c.delete_one(filter).await?.deleted_count
        };
        Ok(deleted)
    })
}

pub fn list_indexes(
    params: &ConnectionParams,
    db: &str,
    coll: &str,
) -> Result<Vec<IndexModel>, PluginError> {
    runtime().block_on(async move {
        let c = client_for(params)
            .await?
            .database(db)
            .collection::<Document>(coll);
        let mut cursor = c.list_indexes().await?;
        let mut out = Vec::new();
        while cursor.advance().await? {
            out.push(cursor.deserialize_current()?);
        }
        Ok(out)
    })
}

pub fn create_collection(
    params: &ConnectionParams,
    db: &str,
    name: &str,
) -> Result<(), PluginError> {
    runtime().block_on(async move {
        client_for(params)
            .await?
            .database(db)
            .create_collection(name)
            .await?;
        Ok(())
    })
}

pub fn drop_collection(params: &ConnectionParams, db: &str, coll: &str) -> Result<(), PluginError> {
    runtime().block_on(async move {
        client_for(params)
            .await?
            .database(db)
            .collection::<Document>(coll)
            .drop()
            .await?;
        Ok(())
    })
}

/// Runs a raw database command (used for native `db.runCommand(...)`).
pub fn run_command(
    params: &ConnectionParams,
    db: &str,
    command: Document,
) -> Result<Document, PluginError> {
    runtime().block_on(async move {
        Ok(client_for(params)
            .await?
            .database(db)
            .run_command(command)
            .await?)
    })
}
