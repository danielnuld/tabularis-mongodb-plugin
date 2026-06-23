//! Plugin-wide settings, populated by the `initialize` RPC call.

use std::sync::{OnceLock, RwLock};

use serde_json::Value;

#[derive(Debug, Clone)]
pub struct Config {
    /// Optional full connection string. When set, it is used verbatim and the
    /// host/port/user/password fields only fill in credentials if absent.
    pub uri: String,
    /// Use the `mongodb+srv://` scheme (DNS seedlist) instead of `mongodb://`.
    pub srv: bool,
    /// Authentication database (authSource).
    pub auth_source: String,
    /// Replica set name.
    pub replica_set: String,
    /// Enable TLS.
    pub tls: bool,
    /// Number of documents sampled to infer a collection's columns.
    pub sample_size: i64,
    /// Extra connection-string query parameters, appended verbatim.
    pub extra: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            uri: String::new(),
            srv: false,
            auth_source: String::new(),
            replica_set: String::new(),
            tls: false,
            sample_size: 100,
            extra: String::new(),
        }
    }
}

impl Config {
    pub fn from_settings(settings: &Value) -> Self {
        let mut cfg = Config::default();
        let get_str = |k: &str| settings.get(k).and_then(Value::as_str).map(str::to_string);
        let get_bool = |k: &str| settings.get(k).and_then(Value::as_bool);
        if let Some(v) = get_str("uri") {
            cfg.uri = v;
        }
        if let Some(v) = get_bool("srv") {
            cfg.srv = v;
        }
        if let Some(v) = get_str("auth_source") {
            cfg.auth_source = v;
        }
        if let Some(v) = get_str("replica_set") {
            cfg.replica_set = v;
        }
        if let Some(v) = get_bool("tls") {
            cfg.tls = v;
        }
        if let Some(v) = settings.get("sample_size").and_then(Value::as_i64) {
            if v > 0 {
                cfg.sample_size = v;
            }
        }
        if let Some(v) = get_str("extra") {
            cfg.extra = v;
        }
        cfg
    }
}

fn store() -> &'static RwLock<Config> {
    static STORE: OnceLock<RwLock<Config>> = OnceLock::new();
    STORE.get_or_init(|| RwLock::new(Config::default()))
}

pub fn set(cfg: Config) {
    if let Ok(mut guard) = store().write() {
        *guard = cfg;
    }
}

pub fn get() -> Config {
    store().read().map(|g| g.clone()).unwrap_or_default()
}
