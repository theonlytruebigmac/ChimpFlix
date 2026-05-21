//! Smart collection rule DSL → parameterised SQL translator.
//!
//! The wire shape is a small JSON tree:
//!
//! ```json
//! {
//!   "operator": "and",
//!   "conditions": [
//!     {"field": "kind", "op": "eq", "value": "movie"},
//!     {"field": "genre", "op": "contains", "value": "Action"},
//!     {"field": "year", "op": "between", "value": [2020, 2029]}
//!   ]
//! }
//! ```
//!
//! `compile_to_sql` returns `(where_clause, joins, bindings)`. The
//! caller stitches that into the surrounding query template; bindings
//! flow through `sqlx`'s parameter binding, so user input never lands
//! in the SQL string itself. Field + op tokens are whitelisted here
//! and ignored if unknown — a typo in a rule that's already in the
//! DB degrades to "match nothing" instead of crashing playback.

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SmartRule {
    /// `and` / `or`. Anything else is rejected at parse time.
    pub operator: String,
    pub conditions: Vec<Condition>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Condition {
    pub field: String,
    pub op: String,
    /// JSON value — string, number, bool, or two-element array for
    /// `between`. Validated against the field's expected type when
    /// compiling.
    pub value: serde_json::Value,
}

/// Bound value that the caller will feed to `sqlx::query::bind`. We
/// keep the type spread minimal: SQLite happily coerces, and our
/// schema only stores text + integer + real here.
#[derive(Debug, Clone)]
pub enum Bind {
    Text(String),
    Int(i64),
    Real(f64),
}

/// Output of `compile_to_sql`. The where clause is a `(...)` block
/// that the caller can ANDfold into its surrounding query; joins is a
/// vec of SQL fragments to inject after the `items i` table; bindings
/// are the values in the order the `?` placeholders appear.
pub struct CompiledRule {
    pub where_clause: String,
    pub joins: Vec<&'static str>,
    pub bindings: Vec<Bind>,
}

const MAX_CONDITIONS: usize = 50;

/// Translate a parsed rule into a parameterised SQL fragment. Caller
/// is expected to wrap the resulting `where_clause` with its own
/// `WHERE` + any per-user library filter.
pub fn compile_to_sql(rule: &SmartRule) -> Result<CompiledRule> {
    let combinator = match rule.operator.to_ascii_lowercase().as_str() {
        "and" => "AND",
        "or" => "OR",
        other => return Err(anyhow!("unknown rule operator `{other}` (use and/or)")),
    };
    if rule.conditions.is_empty() {
        return Err(anyhow!("rule must have at least one condition"));
    }
    if rule.conditions.len() > MAX_CONDITIONS {
        return Err(anyhow!(
            "rule exceeds {MAX_CONDITIONS} condition cap; split into nested rules"
        ));
    }

    let mut clauses: Vec<String> = Vec::new();
    let mut bindings: Vec<Bind> = Vec::new();
    let mut joins: Vec<&'static str> = Vec::new();
    let mut needs_tags_join = false;
    let mut needs_genres_join = false;

    for cond in &rule.conditions {
        let clause = compile_condition(
            cond,
            &mut bindings,
            &mut needs_tags_join,
            &mut needs_genres_join,
        )?;
        clauses.push(clause);
    }
    if needs_tags_join {
        joins.push(
            "LEFT JOIN item_tags _it ON _it.item_id = i.id \
             LEFT JOIN tags _t ON _t.id = _it.tag_id",
        );
    }
    if needs_genres_join {
        // genre is stored as a JSON array on items.genres — no join
        // required; the LIKE-on-JSON in compile_condition handles it.
    }

    let where_clause = format!("({})", clauses.join(&format!(" {combinator} ")));
    Ok(CompiledRule {
        where_clause,
        joins,
        bindings,
    })
}

fn compile_condition(
    cond: &Condition,
    bindings: &mut Vec<Bind>,
    needs_tags_join: &mut bool,
    _needs_genres_join: &mut bool,
) -> Result<String> {
    let field = cond.field.as_str();
    let op = cond.op.as_str();
    match field {
        "kind" => {
            // Always eq: kind is a closed set (movie/show/episode).
            require_op(op, &["eq", "ne"])?;
            let v = expect_string(&cond.value)?;
            if !matches!(v.as_str(), "movie" | "show" | "episode") {
                return Err(anyhow!("kind must be one of movie / show / episode"));
            }
            bindings.push(Bind::Text(v));
            Ok(format!("i.kind {} ?", sql_op(op)))
        }
        "year" => {
            require_op(op, &["eq", "ne", "lt", "le", "gt", "ge", "between"])?;
            if op == "between" {
                let (lo, hi) = expect_range_int(&cond.value)?;
                bindings.push(Bind::Int(lo));
                bindings.push(Bind::Int(hi));
                Ok("i.year BETWEEN ? AND ?".to_string())
            } else {
                let n = expect_int(&cond.value)?;
                bindings.push(Bind::Int(n));
                Ok(format!("i.year {} ?", sql_op(op)))
            }
        }
        "rating_audience" => {
            require_op(op, &["eq", "ne", "lt", "le", "gt", "ge", "between"])?;
            if op == "between" {
                let (lo, hi) = expect_range_real(&cond.value)?;
                bindings.push(Bind::Real(lo));
                bindings.push(Bind::Real(hi));
                Ok("i.rating_audience BETWEEN ? AND ?".to_string())
            } else {
                let n = expect_real(&cond.value)?;
                bindings.push(Bind::Real(n));
                Ok(format!("i.rating_audience {} ?", sql_op(op)))
            }
        }
        "library_id" => {
            require_op(op, &["eq", "ne", "in"])?;
            if op == "in" {
                let list = expect_int_list(&cond.value)?;
                if list.is_empty() {
                    return Err(anyhow!("library_id `in` set must be non-empty"));
                }
                let placeholders = std::iter::repeat_n("?", list.len())
                    .collect::<Vec<_>>()
                    .join(",");
                for n in list {
                    bindings.push(Bind::Int(n));
                }
                Ok(format!("i.library_id IN ({placeholders})"))
            } else {
                let n = expect_int(&cond.value)?;
                bindings.push(Bind::Int(n));
                Ok(format!("i.library_id {} ?", sql_op(op)))
            }
        }
        "title" => {
            require_op(op, &["eq", "contains", "starts_with"])?;
            let v = expect_string(&cond.value)?;
            match op {
                "contains" => {
                    bindings.push(Bind::Text(format!("%{}%", escape_like(&v))));
                    Ok("i.title LIKE ? ESCAPE '\\' COLLATE NOCASE".to_string())
                }
                "starts_with" => {
                    bindings.push(Bind::Text(format!("{}%", escape_like(&v))));
                    Ok("i.title LIKE ? ESCAPE '\\' COLLATE NOCASE".to_string())
                }
                _ => {
                    bindings.push(Bind::Text(v));
                    Ok("i.title = ? COLLATE NOCASE".to_string())
                }
            }
        }
        "genre" => {
            // Genres are denormalised as a JSON array on items.genres.
            // We're after substring match against any element; a LIKE
            // on the serialised JSON (`["Action","Drama"]`) wrapped in
            // quotes catches the array form without needing a join.
            require_op(op, &["contains"])?;
            let v = expect_string(&cond.value)?;
            bindings.push(Bind::Text(format!("%\"{}\"%", escape_like(&v))));
            Ok("i.genres LIKE ? ESCAPE '\\' COLLATE NOCASE".to_string())
        }
        "tag" => {
            require_op(op, &["contains"])?;
            *needs_tags_join = true;
            let v = expect_string(&cond.value)?;
            bindings.push(Bind::Text(v));
            Ok("_t.name = ? COLLATE NOCASE".to_string())
        }
        "added_at" => {
            require_op(op, &["lt", "le", "gt", "ge", "between"])?;
            if op == "between" {
                let (lo, hi) = expect_range_int(&cond.value)?;
                bindings.push(Bind::Int(lo));
                bindings.push(Bind::Int(hi));
                Ok("i.added_at BETWEEN ? AND ?".to_string())
            } else {
                let n = expect_int(&cond.value)?;
                bindings.push(Bind::Int(n));
                Ok(format!("i.added_at {} ?", sql_op(op)))
            }
        }
        _ => Err(anyhow!("unknown field `{field}`")),
    }
}

fn sql_op(op: &str) -> &'static str {
    match op {
        "eq" => "=",
        "ne" => "!=",
        "lt" => "<",
        "le" => "<=",
        "gt" => ">",
        "ge" => ">=",
        _ => "=",
    }
}

fn require_op(op: &str, allowed: &[&str]) -> Result<()> {
    if !allowed.contains(&op) {
        return Err(anyhow!(
            "operator `{op}` not allowed; valid: {}",
            allowed.join(", ")
        ));
    }
    Ok(())
}

fn expect_string(v: &serde_json::Value) -> Result<String> {
    v.as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("expected string value"))
}

fn expect_int(v: &serde_json::Value) -> Result<i64> {
    v.as_i64().ok_or_else(|| anyhow!("expected integer value"))
}

fn expect_real(v: &serde_json::Value) -> Result<f64> {
    v.as_f64().ok_or_else(|| anyhow!("expected numeric value"))
}

fn expect_range_int(v: &serde_json::Value) -> Result<(i64, i64)> {
    let arr = v
        .as_array()
        .ok_or_else(|| anyhow!("between expects [lo, hi]"))?;
    if arr.len() != 2 {
        return Err(anyhow!("between value must be exactly two elements"));
    }
    Ok((expect_int(&arr[0])?, expect_int(&arr[1])?))
}

fn expect_range_real(v: &serde_json::Value) -> Result<(f64, f64)> {
    let arr = v
        .as_array()
        .ok_or_else(|| anyhow!("between expects [lo, hi]"))?;
    if arr.len() != 2 {
        return Err(anyhow!("between value must be exactly two elements"));
    }
    Ok((expect_real(&arr[0])?, expect_real(&arr[1])?))
}

fn expect_int_list(v: &serde_json::Value) -> Result<Vec<i64>> {
    let arr = v
        .as_array()
        .ok_or_else(|| anyhow!("in expects an array of integers"))?;
    arr.iter().map(expect_int).collect()
}

/// Escape % and _ so a user-typed substring with wildcards in it
/// (e.g. searching for a "50_") doesn't act as a wildcard.
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule_from(json: &str) -> SmartRule {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn compiles_simple_kind_eq() {
        let r = rule_from(
            r#"{"operator":"and","conditions":[{"field":"kind","op":"eq","value":"movie"}]}"#,
        );
        let c = compile_to_sql(&r).unwrap();
        assert_eq!(c.where_clause, "(i.kind = ?)");
        assert_eq!(c.bindings.len(), 1);
        assert!(matches!(c.bindings[0], Bind::Text(ref s) if s == "movie"));
    }

    #[test]
    fn compiles_year_between() {
        let r = rule_from(
            r#"{"operator":"and","conditions":[{"field":"year","op":"between","value":[2020,2029]}]}"#,
        );
        let c = compile_to_sql(&r).unwrap();
        assert_eq!(c.where_clause, "(i.year BETWEEN ? AND ?)");
        assert_eq!(c.bindings.len(), 2);
    }

    #[test]
    fn compiles_and_combinator() {
        let r = rule_from(
            r#"{"operator":"and","conditions":[
                {"field":"kind","op":"eq","value":"movie"},
                {"field":"year","op":"ge","value":2010}
            ]}"#,
        );
        let c = compile_to_sql(&r).unwrap();
        assert!(c.where_clause.contains(" AND "));
        assert!(c.where_clause.contains("i.kind = ?"));
        assert!(c.where_clause.contains("i.year >= ?"));
    }

    #[test]
    fn compiles_or_combinator() {
        let r = rule_from(
            r#"{"operator":"or","conditions":[
                {"field":"kind","op":"eq","value":"movie"},
                {"field":"kind","op":"eq","value":"show"}
            ]}"#,
        );
        let c = compile_to_sql(&r).unwrap();
        assert!(c.where_clause.contains(" OR "));
    }

    #[test]
    fn rejects_unknown_field() {
        let r = rule_from(
            r#"{"operator":"and","conditions":[{"field":"haxx","op":"eq","value":"x"}]}"#,
        );
        assert!(compile_to_sql(&r).is_err());
    }

    #[test]
    fn rejects_unknown_op() {
        let r = rule_from(
            r#"{"operator":"and","conditions":[{"field":"kind","op":"matches","value":"x"}]}"#,
        );
        assert!(compile_to_sql(&r).is_err());
    }

    #[test]
    fn rejects_invalid_kind_value() {
        let r = rule_from(
            r#"{"operator":"and","conditions":[{"field":"kind","op":"eq","value":"video"}]}"#,
        );
        assert!(compile_to_sql(&r).is_err());
    }

    #[test]
    fn rejects_empty_conditions() {
        let r = rule_from(r#"{"operator":"and","conditions":[]}"#);
        assert!(compile_to_sql(&r).is_err());
    }

    #[test]
    fn rejects_overlong_rule() {
        let mut conds = String::from("[");
        for i in 0..MAX_CONDITIONS + 1 {
            if i > 0 {
                conds.push(',');
            }
            conds.push_str(r#"{"field":"kind","op":"eq","value":"movie"}"#);
        }
        conds.push(']');
        let r = rule_from(&format!(r#"{{"operator":"and","conditions":{conds}}}"#));
        assert!(compile_to_sql(&r).is_err());
    }

    #[test]
    fn genre_contains_uses_quoted_substring() {
        let r = rule_from(
            r#"{"operator":"and","conditions":[{"field":"genre","op":"contains","value":"Action"}]}"#,
        );
        let c = compile_to_sql(&r).unwrap();
        match &c.bindings[0] {
            Bind::Text(s) => assert_eq!(s, r#"%"Action"%"#),
            _ => panic!("expected text binding"),
        }
    }

    #[test]
    fn library_id_in_emits_placeholders() {
        let r = rule_from(
            r#"{"operator":"and","conditions":[{"field":"library_id","op":"in","value":[1,3,5]}]}"#,
        );
        let c = compile_to_sql(&r).unwrap();
        assert!(c.where_clause.contains("IN (?,?,?)"));
        assert_eq!(c.bindings.len(), 3);
    }

    #[test]
    fn escape_like_escapes_wildcards() {
        assert_eq!(escape_like("50%off"), "50\\%off");
        assert_eq!(escape_like("foo_bar"), "foo\\_bar");
        assert_eq!(escape_like("a\\b"), "a\\\\b");
    }
}
