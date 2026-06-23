//! Column type inference for schemaless collections.
//!
//! MongoDB documents have no fixed schema, so columns are inferred by sampling
//! documents. Nested documents and arrays are reported as `JSON` (which makes
//! Tabularis render the cell with JSON highlighting and the JSON editor);
//! scalars get a friendly BSON type name. A field with conflicting scalar types
//! across the sample is reported as `MIXED`.

use std::collections::HashSet;

use bson::{Bson, Document};

/// Friendly type name for a single BSON value.
pub fn bson_type_name(b: &Bson) -> &'static str {
    match b {
        Bson::Document(_) | Bson::Array(_) => "JSON",
        Bson::String(_) => "STRING",
        Bson::Int32(_) => "INT32",
        Bson::Int64(_) => "INT64",
        Bson::Double(_) => "DOUBLE",
        Bson::Decimal128(_) => "DECIMAL",
        Bson::Boolean(_) => "BOOLEAN",
        Bson::DateTime(_) => "DATE",
        Bson::Timestamp(_) => "TIMESTAMP",
        Bson::ObjectId(_) => "OBJECTID",
        Bson::Binary(_) => "BINARY",
        Bson::RegularExpression(_) => "REGEX",
        Bson::Null | Bson::Undefined => "NULL",
        _ => "MIXED",
    }
}

/// True for the numeric BSON family (including the collapsed `NUMBER`), which
/// is treated as compatible so a field holding both ints and doubles — common
/// in MongoDB — is reported as `NUMBER` rather than `MIXED`.
fn is_numeric(t: &str) -> bool {
    matches!(t, "INT32" | "INT64" | "DOUBLE" | "DECIMAL" | "NUMBER")
}

/// Resolves a single column type from the values observed for a field across
/// the sampled documents.
pub fn column_type<'a, I: IntoIterator<Item = &'a Bson>>(values: I) -> String {
    let mut seen: Option<&'static str> = None;
    for v in values {
        let t = bson_type_name(v);
        if t == "NULL" {
            continue;
        }
        if t == "JSON" {
            return "JSON".to_string();
        }
        match seen {
            None => seen = Some(t),
            Some(prev) if prev != t => {
                if is_numeric(prev) && is_numeric(t) {
                    seen = Some("NUMBER");
                } else {
                    return "MIXED".to_string();
                }
            }
            _ => {}
        }
    }
    seen.unwrap_or("NULL").to_string()
}

/// Infers `(column_name, type)` pairs from a sample of documents. Column order
/// follows first-seen order across the sample, with `_id` pinned first.
pub fn infer_columns(docs: &[Document]) -> Vec<(String, String)> {
    let mut order: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for d in docs {
        for (k, _) in d {
            if seen.insert(k.to_string()) {
                order.push(k.to_string());
            }
        }
    }
    // Stable sort keeps first-seen order; `_id` is pinned to the front.
    order.sort_by_key(|k| if k == "_id" { 0 } else { 1 });

    order
        .into_iter()
        .map(|k| {
            let values = docs.iter().filter_map(|d| d.get(&k));
            let ty = column_type(values);
            (k, ty)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;

    #[test]
    fn nested_is_json() {
        assert_eq!(bson_type_name(&Bson::Document(Default::default())), "JSON");
        assert_eq!(bson_type_name(&Bson::Array(vec![])), "JSON");
    }

    #[test]
    fn single_scalar_type() {
        let vals = [Bson::String("a".into()), Bson::String("b".into())];
        assert_eq!(column_type(vals.iter()), "STRING");
    }

    #[test]
    fn nulls_are_ignored_until_a_type_appears() {
        let vals = [Bson::Null, Bson::Int32(1), Bson::Null];
        assert_eq!(column_type(vals.iter()), "INT32");
    }

    #[test]
    fn conflicting_scalars_are_mixed() {
        let vals = [Bson::Int32(1), Bson::String("x".into())];
        assert_eq!(column_type(vals.iter()), "MIXED");
    }

    #[test]
    fn mixed_numerics_collapse_to_number() {
        let vals = [Bson::Int32(300), Bson::Double(19.99), Bson::Int64(5)];
        assert_eq!(column_type(vals.iter()), "NUMBER");
    }

    #[test]
    fn any_nested_wins_json() {
        let vals = [Bson::Int32(1), Bson::Array(vec![Bson::Int32(2)])];
        assert_eq!(column_type(vals.iter()), "JSON");
    }

    #[test]
    fn all_null_is_null() {
        let vals = [Bson::Null];
        assert_eq!(column_type(vals.iter()), "NULL");
    }

    #[test]
    fn infer_columns_pins_id_first_and_types() {
        let docs = vec![
            doc! { "name": "a", "age": 30i32, "_id": 1i32 },
            doc! { "_id": 2i32, "name": "b", "tags": ["x", "y"] },
        ];
        let cols = infer_columns(&docs);
        assert_eq!(cols[0].0, "_id");
        let map: std::collections::HashMap<_, _> = cols.iter().cloned().collect();
        assert_eq!(map["name"], "STRING");
        assert_eq!(map["age"], "INT32");
        assert_eq!(map["tags"], "JSON");
    }
}
