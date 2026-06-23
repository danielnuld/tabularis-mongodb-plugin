//! Builds the MongoDB connection URI from connection params + plugin settings.

use crate::config::Config;
use crate::error::PluginError;
use crate::models::ConnectionParams;

/// Percent-encodes a userinfo component (RFC 3986 unreserved set is kept).
/// MongoDB requires the username and password in the URI to be encoded.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let keep = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~');
        if keep {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{b:02X}"));
        }
    }
    out
}

/// Builds the connection URI. If `cfg.uri` is set it is used verbatim;
/// otherwise a URI is assembled from host/port/credentials + settings.
pub fn build_uri(params: &ConnectionParams, cfg: &Config) -> Result<String, PluginError> {
    if !cfg.uri.trim().is_empty() {
        return Ok(cfg.uri.trim().to_string());
    }

    let host = params
        .host
        .as_deref()
        .filter(|h| !h.is_empty())
        .ok_or_else(|| PluginError::invalid_params("a host is required for MongoDB"))?;

    let scheme = if cfg.srv { "mongodb+srv" } else { "mongodb" };

    let mut uri = format!("{scheme}://");
    if let Some(user) = params.username.as_deref().filter(|u| !u.is_empty()) {
        uri.push_str(&percent_encode(user));
        if let Some(pass) = params.password.as_deref().filter(|p| !p.is_empty()) {
            uri.push(':');
            uri.push_str(&percent_encode(pass));
        }
        uri.push('@');
    }
    uri.push_str(host);
    // SRV URIs must not carry a port (the port comes from DNS SRV records).
    if !cfg.srv {
        if let Some(port) = params.port {
            uri.push_str(&format!(":{port}"));
        }
    }
    uri.push('/');

    let mut qs: Vec<String> = Vec::new();
    if !cfg.auth_source.trim().is_empty() {
        qs.push(format!("authSource={}", cfg.auth_source.trim()));
    }
    if !cfg.replica_set.trim().is_empty() {
        qs.push(format!("replicaSet={}", cfg.replica_set.trim()));
    }
    if cfg.tls {
        qs.push("tls=true".to_string());
    }
    let extra = cfg.extra.trim().trim_start_matches(['?', '&']);
    if !extra.is_empty() {
        qs.push(extra.to_string());
    }
    if !qs.is_empty() {
        uri.push('?');
        uri.push_str(&qs.join("&"));
    }

    Ok(uri)
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)] // clearer than struct-init in these tests
mod tests {
    use super::*;

    fn params() -> ConnectionParams {
        ConnectionParams {
            host: Some("db.local".to_string()),
            port: Some(27017),
            database: Some("app".to_string()),
            username: Some("user".to_string()),
            password: Some("p@ss:w/rd".to_string()),
            ssl_mode: None,
        }
    }

    #[test]
    fn encodes_credentials() {
        assert_eq!(percent_encode("p@ss:w/rd"), "p%40ss%3Aw%2Frd");
        assert_eq!(percent_encode("plain-1.0_x"), "plain-1.0_x");
    }

    #[test]
    fn builds_basic_uri() {
        let uri = build_uri(&params(), &Config::default()).unwrap();
        assert_eq!(uri, "mongodb://user:p%40ss%3Aw%2Frd@db.local:27017/");
    }

    #[test]
    fn adds_options() {
        let mut cfg = Config::default();
        cfg.auth_source = "admin".into();
        cfg.replica_set = "rs0".into();
        cfg.tls = true;
        let uri = build_uri(&params(), &cfg).unwrap();
        assert!(
            uri.ends_with("/?authSource=admin&replicaSet=rs0&tls=true"),
            "{uri}"
        );
    }

    #[test]
    fn srv_drops_port() {
        let mut cfg = Config::default();
        cfg.srv = true;
        let uri = build_uri(&params(), &cfg).unwrap();
        assert!(uri.starts_with("mongodb+srv://user:"), "{uri}");
        assert!(!uri.contains(":27017"), "{uri}");
    }

    #[test]
    fn explicit_uri_used_verbatim() {
        let mut cfg = Config::default();
        cfg.uri = "mongodb://h1,h2/?replicaSet=rs".into();
        let uri = build_uri(&params(), &cfg).unwrap();
        assert_eq!(uri, "mongodb://h1,h2/?replicaSet=rs");
    }

    #[test]
    fn missing_host_errors() {
        let mut p = params();
        p.host = None;
        assert_eq!(build_uri(&p, &Config::default()).unwrap_err().code, -32602);
    }
}
