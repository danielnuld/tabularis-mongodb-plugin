//! Parser for the queries Tabularis sends to `execute_query`.
//!
//! Two shapes are accepted:
//!   * the host's auto-generated collection browse — `SELECT * FROM "coll"` —
//!     emitted when the user clicks a collection in the explorer; and
//!   * native MongoDB shell calls — `db.<collection>.<op>(<args>)` and the
//!     database-level `db.createCollection(...)` / `db.runCommand(...)`.
//!
//! This module only tokenises; argument bodies are parsed as Extended JSON by
//! `utils::bsonjson`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Statement {
    /// Host-generated `SELECT * FROM <collection> [WHERE ...] [ORDER BY ...]`
    /// browse query. `filter` / `sort` carry the raw clause bodies (translated
    /// to MongoDB by `utils::sqlwhere`); any `LIMIT` is ignored (the host
    /// paginates via the RPC `limit`/`page` fields).
    Browse {
        collection: String,
        filter: Option<String>,
        sort: Option<String>,
    },
    /// `db.<target>.<op>(<args>)`. For database-level calls (e.g.
    /// `db.createCollection(...)`) `target` is empty.
    Shell {
        target: String,
        op: String,
        args: Vec<String>,
    },
}

fn strip_trailing_semicolon(q: &str) -> &str {
    let q = q.trim();
    q.strip_suffix(';').map(str::trim_end).unwrap_or(q)
}

/// True when the query is the host's `SELECT ...` browse form.
pub fn is_browse(query: &str) -> bool {
    let q = strip_trailing_semicolon(query).trim_start();
    q.len() >= 6 && q[..6].eq_ignore_ascii_case("select")
}

/// Unwraps a `"x"`, `` `x` `` or `[x]`/bare identifier into its bare name.
fn unquote_identifier(raw: &str) -> String {
    let t = raw.trim().trim_end_matches(';').trim();
    let t = t
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(t);
    let bytes = t.as_bytes();
    if t.len() >= 2 {
        let (f, l) = (bytes[0], bytes[t.len() - 1]);
        if (f == b'"' && l == b'"') || (f == b'`' && l == b'`') || (f == b'\'' && l == b'\'') {
            return t[1..t.len() - 1].to_string();
        }
    }
    t.to_string()
}

/// Finds a whole-word keyword position in an uppercased string.
fn kw_pos(upper: &str, kw: &str) -> Option<usize> {
    let bytes = upper.as_bytes();
    upper.match_indices(kw).find_map(|(idx, _)| {
        let before = idx == 0 || bytes[idx - 1] == b' ';
        let after = idx + kw.len();
        let after_ok = after >= bytes.len() || bytes[after] == b' ';
        if before && after_ok {
            Some(idx)
        } else {
            None
        }
    })
}

/// Parses `SELECT * FROM <coll> [WHERE <w>] [ORDER BY <o>] [LIMIT n]` into the
/// collection name and the raw WHERE / ORDER BY clause bodies.
fn parse_browse(query: &str) -> Result<(String, Option<String>, Option<String>), String> {
    let q = strip_trailing_semicolon(query);
    let upper = q.to_ascii_uppercase();
    let from = upper
        .find(" FROM ")
        .ok_or_else(|| "could not find FROM in browse query".to_string())?;
    let after = &q[from + 6..];
    let lead_ws = after.len() - after.trim_start().len();
    let after = after.trim_start();

    let coll_end = after.find(char::is_whitespace).unwrap_or(after.len());
    let collection = unquote_identifier(&after[..coll_end]);
    if collection.is_empty() {
        return Err("empty collection name in browse query".to_string());
    }

    let rest = &after[coll_end..];
    let rest_upper = &upper[from + 6 + lead_ws + coll_end..];

    let where_p = kw_pos(rest_upper, "WHERE");
    let order_p = kw_pos(rest_upper, "ORDER BY");
    let limit_p = kw_pos(rest_upper, "LIMIT");

    let clause = |start: usize, ends: &[Option<usize>]| -> Option<String> {
        let end = ends
            .iter()
            .filter_map(|e| *e)
            .filter(|e| *e > start)
            .min()
            .unwrap_or(rest.len());
        let s = rest[start..end].trim();
        if s.is_empty() {
            None
        } else {
            Some(s.to_string())
        }
    };

    let filter = where_p.and_then(|p| clause(p + "WHERE".len(), &[order_p, limit_p]));
    let sort = order_p.and_then(|p| clause(p + "ORDER BY".len(), &[limit_p]));

    Ok((collection, filter, sort))
}

/// Splits a comma-separated argument list at top level, respecting nested
/// `()[]{}` and string literals.
pub fn split_top_level_commas(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut in_str: Option<char> = None;
    let mut escaped = false;
    let mut start = 0usize;
    let bytes = s.char_indices().collect::<Vec<_>>();
    for &(i, c) in &bytes {
        if let Some(q) = in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                in_str = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => in_str = Some(c),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(s[start..i].trim().to_string());
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    let tail = s[start..].trim();
    if !tail.is_empty() || !out.is_empty() {
        out.push(tail.to_string());
    }
    out.retain(|a| !a.is_empty());
    out
}

/// Parses a query into a [`Statement`].
pub fn parse(query: &str) -> Result<Statement, String> {
    let q = strip_trailing_semicolon(query);

    if is_browse(q) {
        let (collection, filter, sort) = parse_browse(q)?;
        return Ok(Statement::Browse {
            collection,
            filter,
            sort,
        });
    }

    let trimmed = q.trim();
    let rest = trimmed.strip_prefix("db.").ok_or_else(|| {
        "query must be `db.<collection>.<op>(...)` or a SELECT browse".to_string()
    })?;

    let open = rest
        .find('(')
        .ok_or_else(|| "missing '(' in shell call".to_string())?;
    if !rest.trim_end().ends_with(')') {
        return Err("missing closing ')' in shell call".to_string());
    }
    let path = rest[..open].trim();
    let close = rest.rfind(')').unwrap();
    let args_str = &rest[open + 1..close];

    let (target, op) = match path.rfind('.') {
        Some(idx) => (
            unquote_identifier(&path[..idx]),
            path[idx + 1..].trim().to_string(),
        ),
        None => (String::new(), path.to_string()),
    };
    if op.is_empty() {
        return Err("missing operation name".to_string());
    }

    Ok(Statement::Shell {
        target,
        op,
        args: split_top_level_commas(args_str),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browse_select() {
        assert_eq!(
            parse("SELECT * FROM \"orders\"").unwrap(),
            Statement::Browse {
                collection: "orders".into(),
                filter: None,
                sort: None
            }
        );
    }

    #[test]
    fn browse_with_where_order_limit() {
        let s =
            parse(r#"SELECT * FROM "products" WHERE "price" > 100 ORDER BY "name" ASC LIMIT 50"#)
                .unwrap();
        assert_eq!(
            s,
            Statement::Browse {
                collection: "products".into(),
                filter: Some(r#""price" > 100"#.into()),
                sort: Some(r#""name" ASC"#.into()),
            }
        );
    }

    #[test]
    fn browse_where_only() {
        let s = parse(r#"SELECT * FROM "u" WHERE "a" = 'x'"#).unwrap();
        assert_eq!(
            s,
            Statement::Browse {
                collection: "u".into(),
                filter: Some(r#""a" = 'x'"#.into()),
                sort: None,
            }
        );
    }

    #[test]
    fn find_with_filter_and_projection() {
        let s = parse("db.users.find({ \"age\": { \"$gt\": 30 } }, { \"name\": 1 })").unwrap();
        assert_eq!(
            s,
            Statement::Shell {
                target: "users".into(),
                op: "find".into(),
                args: vec![
                    "{ \"age\": { \"$gt\": 30 } }".into(),
                    "{ \"name\": 1 }".into()
                ],
            }
        );
    }

    #[test]
    fn aggregate_pipeline_is_single_arg() {
        let s = parse("db.sales.aggregate([{\"$match\":{\"a\":1}},{\"$group\":{\"_id\":\"$k\"}}])")
            .unwrap();
        if let Statement::Shell { target, op, args } = s {
            assert_eq!(target, "sales");
            assert_eq!(op, "aggregate");
            assert_eq!(
                args.len(),
                1,
                "pipeline array must stay one argument: {args:?}"
            );
        } else {
            panic!("expected shell");
        }
    }

    #[test]
    fn commas_inside_strings_and_braces_are_safe() {
        let parts = split_top_level_commas(r#"{"a":"x,y"}, {"b":[1,2,3]}"#);
        assert_eq!(parts, vec![r#"{"a":"x,y"}"#, r#"{"b":[1,2,3]}"#]);
    }

    #[test]
    fn database_level_call_has_empty_target() {
        let s = parse("db.createCollection(\"logs\")").unwrap();
        assert_eq!(
            s,
            Statement::Shell {
                target: String::new(),
                op: "createCollection".into(),
                args: vec!["\"logs\"".into()]
            }
        );
    }

    #[test]
    fn delete_op() {
        let s = parse("db.t.deleteOne({\"_id\": 1})").unwrap();
        assert_eq!(
            s,
            Statement::Shell {
                target: "t".into(),
                op: "deleteOne".into(),
                args: vec!["{\"_id\": 1}".into()]
            }
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("hello world").is_err());
        assert!(parse("db.users.find(").is_err());
    }
}
