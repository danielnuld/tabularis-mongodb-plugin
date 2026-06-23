//! Request shapes the host sends to the plugin.
//!
//! The host serialises `ConnectionParams` with the `database` field as an
//! untagged enum, so on the wire it is *either* a plain string (`"mydb"`) or an
//! array of strings. We accept both.

use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct ConnectionParams {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub database: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    #[allow(dead_code)]
    pub ssl_mode: Option<String>,
}

impl ConnectionParams {
    pub fn from_value(value: &Value) -> Self {
        let obj = value.as_object();
        let get_str = |k: &str| {
            obj.and_then(|o| o.get(k))
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|s| !s.is_empty())
        };
        let port = obj
            .and_then(|o| o.get("port"))
            .and_then(Value::as_u64)
            .and_then(|p| u16::try_from(p).ok());
        let database = obj
            .and_then(|o| o.get("database"))
            .and_then(database_as_str);

        Self {
            host: get_str("host"),
            port,
            database,
            username: get_str("username"),
            password: get_str("password"),
            ssl_mode: get_str("ssl_mode"),
        }
    }
}

/// Accepts a bare string or `[strings]` (untagged DatabaseSelection).
fn database_as_str(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Array(arr) => arr
            .first()
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        _ => None,
    }
}

/// The host wraps connection params in `params.params`.
pub fn connection_params(top: &Value) -> ConnectionParams {
    ConnectionParams::from_value(top.get("params").unwrap_or(&Value::Null))
}

/// Reads a string field from the top-level params object.
pub fn str_field<'a>(top: &'a Value, key: &str) -> Option<&'a str> {
    top.get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn database_string_or_array() {
        assert_eq!(
            ConnectionParams::from_value(&json!({"database":"app"}))
                .database
                .as_deref(),
            Some("app")
        );
        assert_eq!(
            ConnectionParams::from_value(&json!({"database":["a","b"]}))
                .database
                .as_deref(),
            Some("a")
        );
    }

    #[test]
    fn empty_strings_are_none() {
        let p = ConnectionParams::from_value(&json!({"host":"","username":"u"}));
        assert_eq!(p.host, None);
        assert_eq!(p.username.as_deref(), Some("u"));
    }
}
