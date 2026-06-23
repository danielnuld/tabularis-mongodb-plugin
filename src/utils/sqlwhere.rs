//! Translates the SQL `WHERE` / `ORDER BY` clauses that Tabularis' grid filter
//! bar generates into MongoDB filter / sort documents.
//!
//! The grid emits a limited, well-defined grammar (see Tabularis'
//! `buildSingleFilterClause`): conditions of the form `"col" OP value`, joined
//! by `AND`, where OP is one of `= != < > <= >= LIKE` / `NOT LIKE` /
//! `IS [NOT] NULL` / `IN (...)` / `NOT IN (...)` / `BETWEEN a AND b`. Identifiers
//! are double-quoted; string values single-quoted; numbers bare. This is not a
//! general SQL parser — only that grammar is supported.

use std::str::FromStr;

use bson::{doc, Bson, Document};

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Num(String),
    /// A bare word: either a keyword/operator (compared case-insensitively) or
    /// an unquoted column name. Tabularis only quotes identifiers for Postgres,
    /// so for this driver column names arrive here — original case is preserved.
    Word(String),
    Sym(String), // = != < > <= >= ( ) ,
}

/// Case-insensitive keyword check.
fn is_kw(tok: Option<&Tok>, kw: &str) -> bool {
    matches!(tok, Some(Tok::Word(w)) if w.eq_ignore_ascii_case(kw))
}

fn tokenize(input: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        match c {
            '"' | '\'' => {
                let quote = c;
                i += 1;
                let mut s = String::new();
                loop {
                    if i >= chars.len() {
                        return Err("unterminated quoted literal".to_string());
                    }
                    let ch = chars[i];
                    if ch == quote {
                        // Doubled quote is an escaped quote.
                        if i + 1 < chars.len() && chars[i + 1] == quote {
                            s.push(quote);
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    s.push(ch);
                    i += 1;
                }
                if quote == '"' {
                    out.push(Tok::Ident(s));
                } else {
                    out.push(Tok::Str(s));
                }
            }
            '(' | ')' | ',' => {
                out.push(Tok::Sym(c.to_string()));
                i += 1;
            }
            '=' => {
                out.push(Tok::Sym("=".to_string()));
                i += 1;
            }
            '!' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    out.push(Tok::Sym("!=".to_string()));
                    i += 2;
                } else {
                    return Err("unexpected '!'".to_string());
                }
            }
            '<' | '>' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    out.push(Tok::Sym(format!("{c}=")));
                    i += 2;
                } else {
                    out.push(Tok::Sym(c.to_string()));
                    i += 1;
                }
            }
            _ if c == '-' || c.is_ascii_digit() => {
                let start = i;
                i += 1;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                out.push(Tok::Num(chars[start..i].iter().collect()));
            }
            _ if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let w: String = chars[start..i].iter().collect();
                out.push(Tok::Word(w)); // preserve case; keywords compared case-insensitively
            }
            other => return Err(format!("unexpected character '{other}'")),
        }
    }
    Ok(out)
}

/// Converts a value token into BSON. `_id` values that look like a 24-char hex
/// ObjectId are converted so they match stored ObjectIds.
fn value_bson(tok: &Tok, column: &str) -> Result<Bson, String> {
    match tok {
        Tok::Num(n) => {
            if let Ok(i) = n.parse::<i64>() {
                Ok(i32::try_from(i).map(Bson::Int32).unwrap_or(Bson::Int64(i)))
            } else if let Ok(f) = n.parse::<f64>() {
                Ok(Bson::Double(f))
            } else {
                Err(format!("invalid number '{n}'"))
            }
        }
        Tok::Str(s) => {
            if column == "_id" && s.len() == 24 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
                if let Ok(oid) = bson::oid::ObjectId::from_str(s) {
                    return Ok(Bson::ObjectId(oid));
                }
            }
            Ok(Bson::String(s.clone()))
        }
        _ => Err("expected a value".to_string()),
    }
}

/// Builds a `$regex` condition from a SQL LIKE pattern (`%` -> `.*`, `_` -> `.`).
fn like_regex(pattern: &str) -> Bson {
    let mut re = String::from("^");
    for ch in pattern.chars() {
        match ch {
            '%' => re.push_str(".*"),
            '_' => re.push('.'),
            // Escape regex metacharacters.
            '.' | '+' | '*' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\' => {
                re.push('\\');
                re.push(ch);
            }
            _ => re.push(ch),
        }
    }
    re.push('$');
    Bson::Document(doc! { "$regex": re, "$options": "i" })
}

/// Parses a `WHERE` clause body into a MongoDB filter document.
pub fn parse_where(input: &str) -> Result<Document, String> {
    let toks = tokenize(input)?;
    let mut pos = 0;
    let mut conditions: Vec<Document> = Vec::new();

    while pos < toks.len() {
        let column = match &toks[pos] {
            Tok::Ident(c) => c.clone(),
            // Tolerate a bare word as a column name.
            Tok::Word(c) => c.clone(),
            t => return Err(format!("expected a column, found {t:?}")),
        };
        pos += 1;

        let cond = parse_condition(&toks, &mut pos, &column)?;
        conditions.push(cond);

        match toks.get(pos) {
            None => break,
            Some(Tok::Word(w)) if w.eq_ignore_ascii_case("AND") => {
                pos += 1;
                continue;
            }
            Some(t) => return Err(format!("expected AND or end, found {t:?}")),
        }
    }

    match conditions.len() {
        0 => Ok(Document::new()),
        1 => Ok(conditions.into_iter().next().unwrap()),
        _ => Ok(doc! { "$and": conditions.into_iter().map(Bson::Document).collect::<Vec<_>>() }),
    }
}

fn parse_condition(toks: &[Tok], pos: &mut usize, column: &str) -> Result<Document, String> {
    let op = toks
        .get(*pos)
        .ok_or_else(|| "expected an operator".to_string())?
        .clone();
    *pos += 1;

    match op {
        Tok::Sym(s) => {
            let val = value_bson(toks.get(*pos).ok_or("expected value")?, column)?;
            *pos += 1;
            let cond = match s.as_str() {
                "=" => Bson::Document(doc! { "$eq": val }),
                "!=" => Bson::Document(doc! { "$ne": val }),
                ">" => Bson::Document(doc! { "$gt": val }),
                "<" => Bson::Document(doc! { "$lt": val }),
                ">=" => Bson::Document(doc! { "$gte": val }),
                "<=" => Bson::Document(doc! { "$lte": val }),
                other => return Err(format!("unsupported operator '{other}'")),
            };
            Ok(doc! { column: cond })
        }
        Tok::Word(w) => match w.to_uppercase().as_str() {
            "LIKE" => {
                let s = expect_str(toks, pos)?;
                Ok(doc! { column: like_regex(&s) })
            }
            "IS" => {
                // IS NULL | IS NOT NULL
                if is_kw(toks.get(*pos), "NOT") {
                    *pos += 1;
                    expect_word(toks, pos, "NULL")?;
                    Ok(doc! { column: { "$ne": Bson::Null } })
                } else {
                    expect_word(toks, pos, "NULL")?;
                    Ok(doc! { column: Bson::Null })
                }
            }
            "IN" => {
                let vals = parse_list(toks, pos, column)?;
                Ok(doc! { column: { "$in": vals } })
            }
            "BETWEEN" => {
                let v1 = value_bson(toks.get(*pos).ok_or("expected value")?, column)?;
                *pos += 1;
                expect_word(toks, pos, "AND")?;
                let v2 = value_bson(toks.get(*pos).ok_or("expected value")?, column)?;
                *pos += 1;
                Ok(doc! { column: { "$gte": v1, "$lte": v2 } })
            }
            "NOT" => {
                // NOT LIKE | NOT IN
                if is_kw(toks.get(*pos), "LIKE") {
                    *pos += 1;
                    let s = expect_str(toks, pos)?;
                    Ok(doc! { column: { "$not": like_regex(&s) } })
                } else if is_kw(toks.get(*pos), "IN") {
                    *pos += 1;
                    let vals = parse_list(toks, pos, column)?;
                    Ok(doc! { column: { "$nin": vals } })
                } else {
                    Err(format!(
                        "expected LIKE or IN after NOT, found {:?}",
                        toks.get(*pos)
                    ))
                }
            }
            other => Err(format!("unsupported operator '{other}'")),
        },
        t => Err(format!("expected an operator, found {t:?}")),
    }
}

fn parse_list(toks: &[Tok], pos: &mut usize, column: &str) -> Result<Vec<Bson>, String> {
    expect_sym(toks, pos, "(")?;
    let mut vals = Vec::new();
    loop {
        match toks.get(*pos) {
            Some(Tok::Sym(s)) if s == ")" => {
                *pos += 1;
                break;
            }
            Some(Tok::Sym(s)) if s == "," => {
                *pos += 1;
            }
            Some(t) => {
                vals.push(value_bson(t, column)?);
                *pos += 1;
            }
            None => return Err("unterminated IN list".to_string()),
        }
    }
    Ok(vals)
}

fn expect_str(toks: &[Tok], pos: &mut usize) -> Result<String, String> {
    match toks.get(*pos) {
        Some(Tok::Str(s)) => {
            *pos += 1;
            Ok(s.clone())
        }
        t => Err(format!("expected a string literal, found {t:?}")),
    }
}

fn expect_word(toks: &[Tok], pos: &mut usize, word: &str) -> Result<(), String> {
    match toks.get(*pos) {
        Some(Tok::Word(w)) if w.eq_ignore_ascii_case(word) => {
            *pos += 1;
            Ok(())
        }
        t => Err(format!("expected '{word}', found {t:?}")),
    }
}

fn expect_sym(toks: &[Tok], pos: &mut usize, sym: &str) -> Result<(), String> {
    match toks.get(*pos) {
        Some(Tok::Sym(s)) if s == sym => {
            *pos += 1;
            Ok(())
        }
        t => Err(format!("expected '{sym}', found {t:?}")),
    }
}

/// Parses an `ORDER BY` clause body (`"col" ASC, "col2" DESC`) into a sort doc.
pub fn parse_order_by(input: &str) -> Document {
    let toks = match tokenize(input) {
        Ok(t) => t,
        Err(_) => return Document::new(),
    };
    let mut sort = Document::new();
    let mut i = 0;
    while i < toks.len() {
        let col = match &toks[i] {
            Tok::Ident(c) | Tok::Word(c) => c.clone(),
            Tok::Sym(s) if s == "," => {
                i += 1;
                continue;
            }
            _ => {
                i += 1;
                continue;
            }
        };
        i += 1;
        let mut dir = 1i32;
        if let Some(Tok::Word(w)) = toks.get(i) {
            if w.eq_ignore_ascii_case("DESC") {
                dir = -1;
                i += 1;
            } else if w.eq_ignore_ascii_case("ASC") {
                i += 1;
            }
        }
        sort.insert(col, dir);
    }
    sort
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equality_string() {
        let f = parse_where(r#""country" = 'MX'"#).unwrap();
        assert_eq!(f, doc! { "country": { "$eq": "MX" } });
    }

    #[test]
    fn numeric_comparison() {
        let f = parse_where(r#""price" >= 100"#).unwrap();
        assert_eq!(f, doc! { "price": { "$gte": 100i32 } });
    }

    #[test]
    fn and_of_two_conditions() {
        let f = parse_where(r#""active" = 'true' AND "price" < 50"#).unwrap();
        assert_eq!(
            f,
            doc! { "$and": [ { "active": { "$eq": "true" } }, { "price": { "$lt": 50i32 } } ] }
        );
    }

    #[test]
    fn like_to_regex() {
        let f = parse_where(r#""name" LIKE 'La%'"#).unwrap();
        assert_eq!(f, doc! { "name": { "$regex": "^La.*$", "$options": "i" } });
    }

    #[test]
    fn between_and_in() {
        assert_eq!(
            parse_where(r#""age" BETWEEN 10 AND 20"#).unwrap(),
            doc! { "age": { "$gte": 10i32, "$lte": 20i32 } }
        );
        assert_eq!(
            parse_where(r#""country" IN ('MX', 'US')"#).unwrap(),
            doc! { "country": { "$in": ["MX", "US"] } }
        );
    }

    #[test]
    fn is_null_variants() {
        assert_eq!(
            parse_where(r#""x" IS NULL"#).unwrap(),
            doc! { "x": Bson::Null }
        );
        assert_eq!(
            parse_where(r#""x" IS NOT NULL"#).unwrap(),
            doc! { "x": { "$ne": Bson::Null } }
        );
    }

    #[test]
    fn id_hex_becomes_objectid() {
        let f = parse_where(r#""_id" = '507f1f77bcf86cd799439011'"#).unwrap();
        let inner = f.get_document("_id").unwrap();
        assert!(matches!(inner.get("$eq"), Some(Bson::ObjectId(_))));
    }

    #[test]
    fn order_by() {
        assert_eq!(
            parse_order_by(r#""price" DESC, "name" ASC"#),
            doc! { "price": -1i32, "name": 1i32 }
        );
    }

    #[test]
    fn unquoted_columns_preserve_case() {
        // Tabularis sends UNQUOTED identifiers for non-Postgres drivers, so the
        // column name must keep its original case (not be uppercased).
        let f = parse_where("price > 100 AND name LIKE 'M%'").unwrap();
        assert_eq!(
            f,
            doc! { "$and": [
                { "price": { "$gt": 100i32 } },
                { "name": { "$regex": "^M.*$", "$options": "i" } },
            ] }
        );
        assert_eq!(parse_order_by("name ASC"), doc! { "name": 1i32 });
    }
}
