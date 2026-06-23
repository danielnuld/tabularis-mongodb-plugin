//! Conversions between JSON (what the host and the query editor speak) and
//! BSON (what MongoDB speaks).
//!
//! `json_to_bson` is a hand-written converter rather than bson's Extended JSON
//! `TryFrom`, so that MongoDB query operators (`$gt`, `$match`, `$in`, ...) are
//! passed through as plain document keys while only the well-known Extended
//! JSON wrappers (`$oid`, `$date`, `$numberLong`, `$numberDecimal`,
//! `$numberInt`) are interpreted. `preprocess` first rewrites the common shell
//! helpers (`ObjectId("..")`, `ISODate("..")`, ...) into those wrappers.

use std::str::FromStr;
use std::sync::OnceLock;

use bson::{Bson, Decimal128, Document};
use regex::Regex;
use serde_json::Value;

/// Rewrites MongoDB shell constructors into Extended JSON so the body becomes
/// valid JSON that `json_to_bson` understands.
pub fn preprocess(input: &str) -> String {
    struct Rules {
        oid: Regex,
        date: Regex,
        long: Regex,
        decimal: Regex,
        int: Regex,
    }
    static RULES: OnceLock<Rules> = OnceLock::new();
    let r = RULES.get_or_init(|| Rules {
        oid: Regex::new(r#"ObjectId\(\s*["']([0-9a-fA-F]{24})["']\s*\)"#).unwrap(),
        date: Regex::new(r#"(?:ISODate|new\s+Date)\(\s*["']([^"']*)["']\s*\)"#).unwrap(),
        long: Regex::new(r#"NumberLong\(\s*["']?(-?\d+)["']?\s*\)"#).unwrap(),
        decimal: Regex::new(r#"NumberDecimal\(\s*["']([^"']*)["']\s*\)"#).unwrap(),
        int: Regex::new(r#"NumberInt\(\s*["']?(-?\d+)["']?\s*\)"#).unwrap(),
    });
    // `$$` is a literal `$` in regex replacements; `${1}` is capture group 1.
    let s = r.oid.replace_all(input, r#"{"$$oid":"${1}"}"#);
    let s = r.date.replace_all(&s, r#"{"$$date":"${1}"}"#);
    let s = r.long.replace_all(&s, r#"{"$$numberLong":"${1}"}"#);
    let s = r.decimal.replace_all(&s, r#"{"$$numberDecimal":"${1}"}"#);
    let s = r.int.replace_all(&s, r#"${1}"#);
    s.into_owned()
}

/// Parses a JSON/Extended-JSON string into a BSON value (after `preprocess`).
pub fn parse_bson(input: &str) -> Result<Bson, String> {
    let pre = preprocess(input);
    let value: Value = serde_json::from_str(&pre).map_err(|e| format!("invalid JSON: {e}"))?;
    Ok(json_to_bson(&value))
}

/// Parses a JSON object string into a BSON `Document`.
pub fn parse_document(input: &str) -> Result<Document, String> {
    match parse_bson(input)? {
        Bson::Document(d) => Ok(d),
        _ => Err("expected a JSON object".to_string()),
    }
}

/// Converts a `serde_json::Value` into BSON, interpreting only the well-known
/// Extended JSON wrappers and leaving query operators intact.
pub fn json_to_bson(value: &Value) -> Bson {
    match value {
        Value::Null => Bson::Null,
        Value::Bool(b) => Bson::Boolean(*b),
        Value::Number(n) => number_to_bson(n),
        Value::String(s) => Bson::String(s.clone()),
        Value::Array(a) => Bson::Array(a.iter().map(json_to_bson).collect()),
        Value::Object(map) => {
            if map.len() == 1 {
                let (k, v) = map.iter().next().unwrap();
                if let Some(b) = wrapper_to_bson(k, v) {
                    return b;
                }
            }
            let mut doc = Document::new();
            for (k, v) in map {
                doc.insert(k.clone(), json_to_bson(v));
            }
            Bson::Document(doc)
        }
    }
}

fn number_to_bson(n: &serde_json::Number) -> Bson {
    if let Some(i) = n.as_i64() {
        if let Ok(i32v) = i32::try_from(i) {
            return Bson::Int32(i32v);
        }
        return Bson::Int64(i);
    }
    Bson::Double(n.as_f64().unwrap_or(f64::NAN))
}

/// Recognises a single-key Extended JSON wrapper; returns `None` for anything
/// else (e.g. `$gt`), so it stays a regular document key.
fn wrapper_to_bson(key: &str, v: &Value) -> Option<Bson> {
    match key {
        "$oid" => v
            .as_str()
            .and_then(|s| bson::oid::ObjectId::from_str(s).ok())
            .map(Bson::ObjectId),
        "$date" => match v {
            Value::String(s) => bson::DateTime::parse_rfc3339_str(s)
                .ok()
                .map(Bson::DateTime),
            Value::Number(n) => n
                .as_i64()
                .map(|ms| Bson::DateTime(bson::DateTime::from_millis(ms))),
            _ => None,
        },
        "$numberLong" => v
            .as_str()
            .and_then(|s| s.parse::<i64>().ok())
            .map(Bson::Int64),
        "$numberInt" => v
            .as_str()
            .and_then(|s| s.parse::<i32>().ok())
            .map(Bson::Int32),
        "$numberDecimal" => v
            .as_str()
            .and_then(|s| Decimal128::from_str(s).ok())
            .map(Bson::Decimal128),
        _ => None,
    }
}

/// Converts BSON to a display-friendly `serde_json::Value`: ObjectId → hex
/// string, dates → RFC3339 string, nested docs/arrays preserved, etc.
pub fn bson_to_json(b: &Bson) -> Value {
    match b {
        Bson::Double(f) => serde_json::Number::from_f64(*f)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(f.to_string())),
        Bson::String(s) => Value::String(s.clone()),
        Bson::Boolean(b) => Value::Bool(*b),
        Bson::Null | Bson::Undefined => Value::Null,
        Bson::Int32(i) => Value::Number((*i).into()),
        Bson::Int64(i) => Value::Number((*i).into()),
        Bson::ObjectId(o) => Value::String(o.to_hex()),
        Bson::DateTime(d) => {
            Value::String(d.try_to_rfc3339_string().unwrap_or_else(|_| d.to_string()))
        }
        Bson::Decimal128(d) => Value::String(d.to_string()),
        Bson::Array(a) => Value::Array(a.iter().map(bson_to_json).collect()),
        Bson::Document(d) => document_to_json(d),
        Bson::Binary(bin) => Value::String(format!("<binary: {} bytes>", bin.bytes.len())),
        Bson::RegularExpression(r) => Value::String(format!("/{}/{}", r.pattern, r.options)),
        Bson::JavaScriptCode(c) => Value::String(c.clone()),
        Bson::Symbol(s) => Value::String(s.clone()),
        Bson::Timestamp(t) => Value::String(format!("Timestamp({}, {})", t.time, t.increment)),
        Bson::MaxKey => Value::String("MaxKey".to_string()),
        Bson::MinKey => Value::String("MinKey".to_string()),
        other => Value::String(format!("{other:?}")),
    }
}

/// Converts a BSON `Document` into a JSON object.
pub fn document_to_json(d: &Document) -> Value {
    Value::Object(
        d.iter()
            .map(|(k, v)| (k.clone(), bson_to_json(v)))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn query_operators_stay_as_keys() {
        let d = parse_document(r#"{"age": {"$gt": 30}, "tags": {"$in": ["a","b"]}}"#).unwrap();
        let age = d.get_document("age").unwrap();
        assert_eq!(age.get_i32("$gt").unwrap(), 30);
        assert!(d.get_document("tags").unwrap().get_array("$in").is_ok());
    }

    #[test]
    fn oid_wrapper_becomes_objectid() {
        let b = parse_bson(r#"{"$oid":"507f1f77bcf86cd799439011"}"#).unwrap();
        assert!(matches!(b, Bson::ObjectId(_)));
    }

    #[test]
    fn preprocess_rewrites_shell_helpers() {
        assert_eq!(
            preprocess(r#"{"_id": ObjectId("507f1f77bcf86cd799439011")}"#),
            r#"{"_id": {"$oid":"507f1f77bcf86cd799439011"}}"#
        );
        assert_eq!(
            preprocess(r#"{"d": ISODate("2024-01-02T03:04:05Z")}"#),
            r#"{"d": {"$date":"2024-01-02T03:04:05Z"}}"#
        );
        assert_eq!(preprocess(r#"{"n": NumberInt(5)}"#), r#"{"n": 5}"#);
    }

    #[test]
    fn objectid_roundtrips_to_hex() {
        let b = parse_bson(r#"ObjectId("507f1f77bcf86cd799439011")"#).unwrap();
        assert_eq!(bson_to_json(&b), json!("507f1f77bcf86cd799439011"));
    }

    #[test]
    fn numbers_map_to_int_or_double() {
        assert!(matches!(json_to_bson(&json!(7)), Bson::Int32(7)));
        assert!(matches!(
            json_to_bson(&json!(5000000000i64)),
            Bson::Int64(_)
        ));
        assert!(matches!(json_to_bson(&json!(1.5)), Bson::Double(_)));
    }

    #[test]
    fn nested_document_preserved_in_output() {
        let mut inner = Document::new();
        inner.insert("k", Bson::Int32(1));
        let v = bson_to_json(&Bson::Document(inner));
        assert_eq!(v, json!({"k": 1}));
    }
}
