//! Connection and query execution handlers.
//!
//! `execute_query` supports four modes selected by a leading `#!<mode>` line:
//!   - `#!partiql` (default) — pass a PartiQL statement to ExecuteStatement.
//!   - `#!scan`  — native Scan API (full table read, limit + pagination).
//!   - `#!query` — native Query API (single partition-key lookup, limit + pagination).
//!   - `#!get`   — native GetItem API (fetch one item by full key).
//!
//! PartiQL `LIMIT n` is not supported by DynamoDB's ExecuteStatement, so it is
//! detected, stripped from the statement, and applied as a client-side row cap.
//! Pagination is threaded through both PartiQL (`next_token`) and the native
//! APIs (opaque token derived from `LastEvaluatedKey`).

use serde_json::{json, Value};

use crate::dynamodb::models::decode_pagination_token;
use crate::error::ErrorCode;
use crate::error::PluginError;
use crate::handlers::connection;
use crate::handlers::models::{ExecuteQueryResponse, Query, QueryMode};
use crate::rpc::{error_response, ok_response};
use crate::utils::extractor;

pub async fn test_connection(id: Value, params: &Value) -> Value {
    let client = match connection::build_client(params).await {
        Ok(c) => c,
        Err(err) => return error_response(id, err.code, &err.message),
    };

    match client.ping().await {
        Ok(_) => ok_response(id, json!({"success": true})),
        Err(err) => error_response(id, ErrorCode::InternalError, &err.message),
    }
}

pub async fn ping(id: Value, params: &Value) -> Value {
    test_connection(id, params).await
}

/// Extract an optional pagination token passed by the caller.
fn extract_next_token(params: &Value) -> Option<String> {
    params
        .get("next_token")
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Extract an optional row limit passed by the caller.
fn extract_limit(params: &Value) -> Option<i32> {
    params
        .get("limit")
        .and_then(|l| l.as_i64())
        .and_then(|l| i32::try_from(l).ok())
}

/// Extract the 1-based page number passed by the caller (defaults to 1).
fn extract_page(params: &Value) -> u32 {
    params
        .get("page")
        .and_then(|p| p.as_u64())
        .and_then(|p| u32::try_from(p).ok())
        .filter(|p| *p > 0)
        .unwrap_or(1)
}

/// Build the pagination object the Tabularis app deserializes into its
/// `Pagination` struct, which requires `page`, `page_size`, `total_rows`
/// (optional) and `has_more`. The opaque DynamoDB resume token rides along
/// as `next_token` when present.
fn build_pagination(
    page: u32,
    page_size: usize,
    has_more: bool,
    next_token: Option<String>,
) -> Value {
    let mut pg = json!({
        "page": page,
        "page_size": page_size,
        "total_rows": Value::Null,
        "has_more": has_more,
    });
    if let Some(token) = next_token {
        pg["next_token"] = Value::String(token);
    }
    pg
}

/// Split a PartiQL body into individual statements on semicolons that sit
/// outside of single-quoted string literals (#17). Trailing/empty segments are
/// dropped, so `INSERT ...; UPDATE ...;` yields two statements.
///
/// Pure helper so the splitting rules can be unit-tested without a live table.
fn split_statements(body: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut in_string = false;

    for c in body.chars() {
        match c {
            '\'' => {
                in_string = !in_string;
                current.push(c);
            }
            ';' if !in_string => {
                let trimmed = current.trim();
                if !trimmed.is_empty() {
                    statements.push(trimmed.to_owned());
                }
                current.clear();
            }
            _ => current.push(c),
        }
    }
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        statements.push(trimmed.to_owned());
    }
    statements
}

/// Classify a PartiQL statement as destructive for the safety guard (#8).
/// Returns a human-readable reason when the statement is an unguarded
/// `DELETE FROM <table>` (no WHERE clause).
///
/// DROP TABLE is intentionally NOT blocked here — the GUI shows its own
/// confirmation dialog before sending the statement, and DynamoDB routes
/// it through the native DeleteTable API regardless. Blocking it would
/// cause the GUI to silently "succeed" while the table remains.
///
/// Pure helper so the classification can be unit-tested without a live table.
fn destructive_warning(statement: &str) -> Option<String> {
    // Normalize: collapse whitespace and uppercase for keyword matching, but
    // keep the original for quoting back to the caller.
    let norm: String = statement.split_whitespace().collect::<Vec<_>>().join(" ");
    let upper = norm.to_uppercase();

    // An unguarded DELETE (DELETE FROM <t> with no WHERE) wipes the table.
    if upper.starts_with("DELETE FROM") && !upper.contains(" WHERE ") {
        return Some(format!(
            "Refusing to execute a WHERE-less DELETE without confirmation: `{norm}`. \
             This would remove all items from the table. \
             Re-run with `allow_destructive: true` to proceed, or add a WHERE clause."
        ));
    }

    None
}

/// A DDL statement translated from PartiQL into a native DynamoDB control-plane
/// call. DynamoDB PartiQL is DML-only — `CREATE TABLE` / `DROP TABLE` are
/// rejected by `ExecuteStatement` with a cryptic "service error", so they must
/// be routed through the native `CreateTable` / `DeleteTable` APIs instead.
enum DdlStatement {
    CreateTable {
        table_name: String,
        partition_key: String,
        sort_key: Option<String>,
        key_types: std::collections::HashMap<String, String>,
    },
    DropTable {
        table_name: String,
    },
}

/// Detect and parse a DDL statement, returning `None` for DML (which flows on
/// to `ExecuteStatement`). Supports an optional sort key declared as the second
/// `PRIMARY KEY` column, e.g. `PRIMARY KEY ("id", "ts")`.
///
/// Handles quoted identifiers (`"items"`, `` `items` ``, `[items]`) and the
/// `IF NOT EXISTS` / `IF EXISTS` suffixes (parsed but not enforced — DynamoDB
/// has no native idempotent create/drop, so a duplicate create surfaces as a
/// "table already exists" error from the API).
fn parse_ddl(statement: &str) -> Option<DdlStatement> {
    let norm: String = statement.split_whitespace().collect::<Vec<_>>().join(" ");
    let upper = norm.to_uppercase();

    if upper.starts_with("DROP TABLE") {
        let after = norm["DROP TABLE".len()..].trim();
        let table = take_identifier(after)?;
        return Some(DdlStatement::DropTable { table_name: table });
    }

    if upper.starts_with("CREATE TABLE") {
        let rest = norm["CREATE TABLE".len()..].trim();
        // Split into the table name and the parenthesized column list. The
        // first '(' opens the columns.
        let paren = rest.find('(')?;
        let table = take_identifier(rest[..paren].trim())?;
        let close = rest.rfind(')')?;
        if close <= paren {
            return None;
        }
        let cols_src = &rest[paren + 1..close];

        // Walk comma-separated column definitions that sit at the top level
        // (not nested inside the PRIMARY KEY parentheses).
        let mut columns: Vec<(String, String)> = Vec::new();
        let mut key_cols: Vec<String> = Vec::new();
        for part in split_top_level_commas(cols_src) {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let p_upper = part.to_uppercase();
            if p_upper.starts_with("PRIMARY KEY") {
                let kp = part.find('(')?;
                let kc = part.rfind(')')?;
                if kc > kp {
                    key_cols = part[kp + 1..kc]
                        .split(',')
                        .filter_map(|k| take_identifier(k.trim()))
                        .collect();
                }
            } else if let Some((name, ty)) = parse_column_def(part) {
                columns.push((name, ty));
            }
        }

        let partition_key = key_cols.first().cloned()?;
        let sort_key = key_cols.get(1).cloned();

        let mut key_types = std::collections::HashMap::new();
        for (name, ty) in columns {
            key_types.insert(name, ty);
        }

        return Some(DdlStatement::CreateTable {
            table_name: table,
            partition_key,
            sort_key,
            key_types,
        });
    }

    None
}

/// Split on commas that are not nested inside parentheses.
fn split_top_level_commas(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut cur = String::new();
    for c in src.chars() {
        match c {
            '(' => {
                depth += 1;
                cur.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                cur.push(c);
            }
            ',' if depth == 0 => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// Parse a single `<name> <TYPE>` column definition, stripping inline
/// constraints like `NOT NULL`. Returns the identifier and the type token.
fn parse_column_def(def: &str) -> Option<(String, String)> {
    let mut tokens = def.split_whitespace();
    let name = take_identifier(tokens.next()?)?;
    let ty = tokens.next()?.to_string();
    Some((name, ty))
}

/// Extract a leading SQL identifier, stripping a leading `IF NOT EXISTS` /
/// `IF EXISTS` clause and surrounding quotes/brackets.
fn take_identifier(raw: &str) -> Option<String> {
    let raw = raw.trim();
    // Drop a leading `IF NOT EXISTS` / `IF EXISTS` guard (case-insensitive),
    // keeping the original casing of the identifier that follows.
    let lower = raw.to_lowercase();
    let raw = if let Some(rest) = lower.strip_prefix("if not exists ") {
        &raw[raw.len() - rest.len()..]
    } else if let Some(rest) = lower.strip_prefix("if exists ") {
        &raw[raw.len() - rest.len()..]
    } else {
        raw
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let bytes = raw.as_bytes();
    let ident: String = match bytes[0] {
        b'"' => raw.chars().skip(1).take_while(|&c| c != '"').collect(),
        b'`' => raw.chars().skip(1).take_while(|&c| c != '`').collect(),
        b'[' => raw.chars().skip(1).take_while(|&c| c != ']').collect(),
        _ => raw
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
            .collect(),
    };
    if ident.is_empty() {
        None
    } else {
        Some(ident)
    }
}

/// Execute a parsed DDL statement against the native DynamoDB control plane.
async fn execute_ddl(
    client: &crate::dynamodb::client::Client,
    ddl: DdlStatement,
    started: std::time::Instant,
) -> Result<ExecuteQueryResponse, crate::error::PluginError> {
    match ddl {
        DdlStatement::CreateTable {
            table_name,
            partition_key,
            sort_key,
            key_types,
        } => {
            client
                .create_table(&table_name, &partition_key, sort_key.as_deref(), &key_types)
                .await?;
            let mut resp = ExecuteQueryResponse::empty();
            resp.affected_rows = 1;
            resp.execution_time_ms = started.elapsed().as_millis() as usize;
            resp.warning = Some(format!(
                "Created table \"{table_name}\" (partition key: \"{partition_key}\"{})",
                sort_key
                    .map(|s| format!(", sort key: \"{s}\""))
                    .unwrap_or_default()
            ));
            Ok(resp)
        }
        DdlStatement::DropTable { table_name } => {
            client.delete_table(&table_name).await?;
            let mut resp = ExecuteQueryResponse::empty();
            resp.affected_rows = 1;
            resp.execution_time_ms = started.elapsed().as_millis() as usize;
            resp.warning = Some(format!("Dropped table \"{table_name}\""));
            Ok(resp)
        }
    }
}

// ── Aggregate interception ──────────────────────────────────────────────────

/// Supported aggregate functions.
#[derive(Debug, Clone, PartialEq)]
enum AggregateFunc {
    /// COUNT(*) or COUNT("col")
    Count { column: Option<String> },
    /// SUM("col")
    Sum { column: String },
    /// AVG("col")
    Avg { column: String },
    /// MIN("col")
    Min { column: String },
    /// MAX("col")
    Max { column: String },
}

/// A parsed single-aggregate SELECT.
#[derive(Debug, Clone, PartialEq)]
struct AggregateQuery {
    table: String,
    func: AggregateFunc,
    alias: String,
}

/// Detect `SELECT AGG(...) [AS alias] FROM "table"`.
///
/// Returns `None` for anything that isn't a simple single-aggregate SELECT
/// (plain column selects, WHERE clauses, JOINs, multiple aggregates, etc.).
fn parse_aggregate(sql: &str) -> Option<AggregateQuery> {
    let trimmed = sql.trim();
    let upper = trimmed.to_uppercase();
    if !upper.starts_with("SELECT ") {
        return None;
    }

    let rest = &trimmed[7..]; // after "SELECT "
    let upper_rest = rest.to_uppercase();
    let from_pos = find_keyword(&upper_rest, "FROM")?;

    let select_list = rest[..from_pos].trim();
    let after_from = rest[from_pos + 4..].trim();
    let table = take_identifier(after_from)?;

    // Reject WHERE / GROUP BY / HAVING / ORDER BY / LIMIT / JOIN.
    let upper_after = after_from.to_uppercase();
    for kw in ["WHERE", "GROUP", "HAVING", "ORDER", "LIMIT", "JOIN"] {
        if find_keyword(&upper_after, kw).is_some() {
            return None;
        }
    }

    // Parse the aggregate function.
    let (func_call, alias_part) = split_agg_alias(select_list)?;
    // Reject multi-expression select lists (e.g. "COUNT(*), SUM(age)"): a
    // top-level comma means more than one aggregate, which we don't support.
    if func_call.contains(',') {
        return None;
    }
    let func_upper = func_call.to_uppercase();

    let paren_open = func_upper.find('(')?;
    let paren_close = func_upper.rfind(')')?;
    if paren_close < paren_open {
        return None;
    }

    let func_name = func_upper[..paren_open].trim();
    let arg = func_call[paren_open + 1..paren_close].trim();

    let func = match func_name {
        "COUNT" => {
            let column = if arg == "*" {
                None
            } else {
                Some(strip_agg_quotes(arg)?)
            };
            AggregateFunc::Count { column }
        }
        "SUM" => AggregateFunc::Sum {
            column: strip_agg_quotes(arg)?,
        },
        "AVG" => AggregateFunc::Avg {
            column: strip_agg_quotes(arg)?,
        },
        "MIN" => AggregateFunc::Min {
            column: strip_agg_quotes(arg)?,
        },
        "MAX" => AggregateFunc::Max {
            column: strip_agg_quotes(arg)?,
        },
        _ => return None,
    };

    let alias = alias_part.unwrap_or_else(|| func_name.to_lowercase());

    Some(AggregateQuery { table, func, alias })
}

/// Split `"FUNC(...) AS alias"` into `("FUNC(...)", Some("alias"))`.
fn split_agg_alias(select_list: &str) -> Option<(&str, Option<String>)> {
    let upper = select_list.to_uppercase();
    if let Some(as_pos) = find_keyword(&upper, "AS") {
        let func_part = select_list[..as_pos].trim();
        let alias = take_identifier(select_list[as_pos + 2..].trim())?;
        Some((func_part, Some(alias)))
    } else {
        let s = select_list.trim();
        if s.contains('(') && s.contains(')') {
            Some((s, None))
        } else {
            None
        }
    }
}

/// Strip surrounding quotes from an aggregate argument. Returns None if empty.
fn strip_agg_quotes(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
        || (s.starts_with('`') && s.ends_with('`') && s.len() >= 2)
    {
        Some(s[1..s.len() - 1].to_owned())
    } else {
        Some(s.to_owned())
    }
}

/// Execute a single-aggregate query by scanning all items and reducing.
async fn execute_aggregate(
    client: &crate::dynamodb::client::Client,
    agg: AggregateQuery,
    started: std::time::Instant,
) -> Result<ExecuteQueryResponse, PluginError> {
    let mut all_items: Vec<std::collections::HashMap<String, Value>> = Vec::new();
    let mut next_token: Option<String> = None;
    let mut total_capacity: f64 = 0.0;

    for _ in 0..10_000 {
        // scan() takes a raw exclusive-start-key but returns a base64 string
        // token; decode between pages.
        let esk = next_token
            .as_deref()
            .and_then(crate::dynamodb::models::decode_pagination_token);
        let res = client.scan(&agg.table, None, esk).await?;
        total_capacity += res.consumed_capacity.unwrap_or(0.0);
        all_items.extend(res.items);
        match res.next_token {
            Some(t) => next_token = Some(t),
            None => break,
        }
    }

    let result_value: Value = match &agg.func {
        AggregateFunc::Count { column: None } => {
            Value::Number(serde_json::Number::from(all_items.len() as i64))
        }
        AggregateFunc::Count { column: Some(col) } => {
            let count = all_items
                .iter()
                .filter(|item| item.get(col).is_some_and(|v| !v.is_null()))
                .count();
            Value::Number(serde_json::Number::from(count as i64))
        }
        AggregateFunc::Sum { column } => {
            let sum: f64 = all_items
                .iter()
                .filter_map(|item| item.get(column))
                .filter_map(as_numeric)
                .sum();
            json_number(sum)
        }
        AggregateFunc::Avg { column } => {
            let vals: Vec<f64> = all_items
                .iter()
                .filter_map(|item| item.get(column))
                .filter_map(as_numeric)
                .collect();
            if vals.is_empty() {
                Value::Null
            } else {
                json_number(vals.iter().sum::<f64>() / vals.len() as f64)
            }
        }
        AggregateFunc::Min { column } => all_items
            .iter()
            .filter_map(|item| item.get(column))
            .filter_map(as_numeric)
            .fold(None, |acc: Option<f64>, v| {
                Some(acc.map_or(v, |a| a.min(v)))
            })
            .map(json_number)
            .unwrap_or(Value::Null),
        AggregateFunc::Max { column } => all_items
            .iter()
            .filter_map(|item| item.get(column))
            .filter_map(as_numeric)
            .fold(None, |acc: Option<f64>, v| {
                Some(acc.map_or(v, |a| a.max(v)))
            })
            .map(json_number)
            .unwrap_or(Value::Null),
    };

    let mut resp = ExecuteQueryResponse::empty();
    resp.columns = vec![agg.alias];
    resp.rows = vec![vec![result_value]];
    resp.affected_rows = 1;
    resp.execution_time_ms = started.elapsed().as_millis() as usize;
    resp.consumed_capacity = if total_capacity > 0.0 {
        Some(total_capacity)
    } else {
        None
    };

    Ok(resp)
}

/// Extract a numeric value from a JSON value.
///
/// DynamoDB `N` attributes are serialised as JSON *strings* (e.g. `"30"`),
/// so plain `as_f64()` misses them. Accept both JSON numbers and numeric
/// strings so aggregates work over either representation.
fn as_numeric(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// Format a float as a JSON number: integers stay integer.
fn json_number(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 9_007_199_254_740_992.0 {
        Value::Number(serde_json::Number::from(v as i64))
    } else {
        serde_json::Number::from_f64(v)
            .map(Value::Number)
            .unwrap_or(Value::Null)
    }
}

/// Find a keyword (e.g. "FROM", "AS") at a word boundary in an uppercased string.
fn find_keyword(haystack: &str, keyword: &str) -> Option<usize> {
    let mut search_from = 0;
    while let Some(pos) = haystack[search_from..].find(keyword) {
        let abs = search_from + pos;
        let before_ok = abs == 0 || !haystack.as_bytes()[abs - 1].is_ascii_alphanumeric();
        let end = abs + keyword.len();
        let after_ok = end >= haystack.len() || !haystack.as_bytes()[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return Some(abs);
        }
        search_from = abs + 1;
    }
    None
}

/// Build the standard tabular response from a set of DynamoDB items.
fn items_to_response(
    items: Vec<std::collections::HashMap<String, Value>>,
    next_token: Option<String>,
    limit: Option<usize>,
    page: u32,
    execution_time_ms: usize,
    consumed_capacity: Option<f64>,
) -> ExecuteQueryResponse {
    // Union of all keys across all items (DynamoDB is schemaless — each item
    // can have different attributes). Preserves first-seen order.
    let mut columns: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for item in &items {
        for key in item.keys() {
            if seen.insert(key.clone()) {
                columns.push(key.clone());
            }
        }
    }

    let mut rows: Vec<Vec<Value>> = items
        .iter()
        .map(|item| {
            columns
                .iter()
                .map(|col| item.get(col).cloned().unwrap_or(Value::Null))
                .collect()
        })
        .collect();

    // Apply a client-side row cap when a LIMIT was requested.
    let truncated_by_limit = match limit {
        Some(l) if rows.len() > l => {
            rows.truncate(l);
            true
        }
        _ => false,
    };

    let has_more = next_token.is_some() || truncated_by_limit;
    let page_size = limit.unwrap_or(rows.len());

    ExecuteQueryResponse {
        affected_rows: rows.len(),
        execution_time_ms,
        truncated: truncated_by_limit,
        has_more,
        pagination: Some(build_pagination(page, page_size, has_more, next_token)),
        consumed_capacity,
        warning: None,
        columns,
        rows,
    }
}

/// Strip a trailing PartiQL `LIMIT <n>` clause (unsupported by
/// ExecuteStatement) and return the cleaned statement plus the cap.
///
/// Returns `(statement_without_limit, Some(n))` when a LIMIT was found, or
/// `(original, None)` otherwise. Only a terminal LIMIT (optionally followed by
/// whitespace/semicolon) is treated as a row cap.
fn strip_partiql_limit(body: &str) -> (String, Option<usize>) {
    let trimmed = body.trim_end().trim_end_matches(';').trim_end();
    let lower = trimmed.to_lowercase();

    // Find the last standalone occurrence of " limit ".
    let needle = " limit ";
    let Some(pos) = lower.rfind(needle) else {
        return (body.to_string(), None);
    };

    let after = &trimmed[pos + needle.len()..];
    let after_trimmed = after.trim();
    // The remainder must be purely a positive integer to be treated as LIMIT n.
    if !after_trimmed.is_empty() && after_trimmed.chars().all(|c| c.is_ascii_digit()) {
        let n: usize = after_trimmed.parse().unwrap_or(0);
        let stmt = trimmed[..pos].trim_end().to_string();
        (stmt, Some(n))
    } else {
        (body.to_string(), None)
    }
}

/// Parsed request body for the native scan/query/get modes.
struct NativeRequest {
    table_name: String,
    limit: Option<i32>,
    // query-mode fields
    partition_key: Option<String>,
    partition_value: Option<Value>,
    sort_key_name: Option<String>,
    sort_key_value: Option<Value>,
    // get-mode field
    key: Option<std::collections::HashMap<String, Value>>,
}

/// Parse the YAML-ish body of a `#!scan`/`#!query`/`#!get` request.
fn parse_native_body(body: &str) -> Result<NativeRequest, String> {
    let value: Value = serde_yaml::from_str(body)
        .map_err(|e| format!("could not parse request body as YAML: {e}"))?;

    let obj = value
        .as_object()
        .ok_or_else(|| "request body must be a YAML mapping".to_string())?;

    let get_str = |key: &str| obj.get(key).and_then(|v| v.as_str()).map(|s| s.to_string());

    let table_name = obj
        .get("TableName")
        .or_else(|| obj.get("table_name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "TableName is required".to_string())?;

    let limit = obj
        .get("Limit")
        .or_else(|| obj.get("limit"))
        .and_then(|v| v.as_i64())
        .and_then(|l| i32::try_from(l).ok());

    let partition_key = get_str("PartitionKey")
        .or_else(|| get_str("partition_key"))
        .or_else(|| get_str("pk"));
    let partition_value = obj
        .get("PartitionValue")
        .or_else(|| obj.get("partition_value"))
        .or_else(|| obj.get("pk_value"))
        .cloned();

    let sort_key_name = get_str("SortKeyName")
        .or_else(|| get_str("sort_key_name"))
        .or_else(|| get_str("sk"));
    let sort_key_value = obj
        .get("SortKeyValue")
        .or_else(|| obj.get("sort_key_value"))
        .or_else(|| obj.get("sk_value"))
        .cloned();

    // `Key` for get-mode: a plain mapping of {column: value}.
    let key = obj.get("Key").or_else(|| obj.get("key")).and_then(|k| {
        k.as_object().map(|m| {
            m.iter()
                .map(|(col, val)| (col.clone(), val.clone()))
                .collect()
        })
    });

    Ok(NativeRequest {
        table_name,
        limit,
        partition_key,
        partition_value,
        sort_key_name,
        sort_key_value,
        key,
    })
}

pub async fn execute_query(id: Value, params: &Value) -> Value {
    let query_str = match extractor::extract_query(params) {
        Some(q) if !q.is_empty() => q,
        _ => {
            return error_response(
                id,
                ErrorCode::InvalidParams,
                "query must be a non-empty string",
            )
        }
    };

    let query = Query::from(query_str);
    let next_token = extract_next_token(params);
    let param_limit = extract_limit(params);
    let page = extract_page(params);
    let allow_destructive = params
        .get("allow_destructive")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // #17: optional idempotency token for multi-statement transactions.
    let client_request_token = params
        .get("client_request_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_owned());

    let client = match connection::build_client(params).await {
        Ok(c) => c,
        Err(err) => return error_response(id, err.code, &err.message),
    };

    let started = std::time::Instant::now();

    match query.mode {
        QueryMode::Partiql => {
            // #17: multi-statement input routes through ExecuteTransaction.
            // Pagination (next_token) and LIMIT don't apply to transactions, so
            // only treat it as a transaction when there are 2+ statements and
            // no pagination token in play.
            let statements = split_statements(&query.body);
            if statements.len() > 1 && next_token.is_none() {
                // #8: apply the destructive guard to every statement.
                if !allow_destructive {
                    for stmt in &statements {
                        if let Some(warning) = destructive_warning(stmt) {
                            let mut resp = ExecuteQueryResponse::empty();
                            resp.warning = Some(warning);
                            return ok_response(id, json!(resp));
                        }
                    }
                }

                let result = client
                    .execute_transaction(&statements, client_request_token.as_deref())
                    .await;

                return match result {
                    Ok(txn) => {
                        let mut resp = ExecuteQueryResponse::empty();
                        resp.affected_rows = txn.affected_rows;
                        resp.execution_time_ms = started.elapsed().as_millis() as usize;
                        resp.consumed_capacity = txn.consumed_capacity;
                        ok_response(id, json!(resp))
                    }
                    Err(err) => error_response(id, ErrorCode::InternalError, &err.message),
                };
            }

            // #20: strip an unsupported LIMIT clause and apply it client-side.
            let (statement, inline_limit) = strip_partiql_limit(&query.body);

            // #8: refuse destructive statements without explicit confirmation.
            if !allow_destructive {
                if let Some(warning) = destructive_warning(&statement) {
                    let mut resp = ExecuteQueryResponse::empty();
                    resp.warning = Some(warning);
                    return ok_response(id, json!(resp));
                }
            }

            // DynamoDB PartiQL is DML-only: CREATE TABLE / DROP TABLE are
            // rejected by ExecuteStatement with a cryptic "service error".
            // Intercept DDL and route it through the native control-plane API.
            if let Some(ddl) = parse_ddl(&statement) {
                return match execute_ddl(&client, ddl, started).await {
                    Ok(resp) => ok_response(id, json!(resp)),
                    Err(err) => error_response(id, err.code, &err.message),
                };
            }

            // DynamoDB PartiQL has no aggregate functions. Intercept simple
            // single-aggregate SELECTs and compute them client-side via Scan.
            // Only when no pagination token is in play (aggregates always
            // return exactly 1 row).
            if next_token.is_none() {
                if let Some(agg) = parse_aggregate(&statement) {
                    return match execute_aggregate(&client, agg, started).await {
                        Ok(resp) => ok_response(id, json!(resp)),
                        Err(err) => error_response(id, err.code, &err.message),
                    };
                }
            }

            let effective_limit = inline_limit.or(param_limit.map(|l| l as usize));

            let result = match next_token.as_deref() {
                Some(token) => {
                    client
                        .execute_statement_with_token(&statement, Some(token))
                        .await
                }
                None => client.execute_statement(&statement).await,
            };

            match result {
                Ok(res) => ok_response(
                    id,
                    json!(items_to_response(
                        res.items,
                        res.next_token,
                        effective_limit,
                        page,
                        started.elapsed().as_millis() as usize,
                        res.consumed_capacity,
                    )),
                ),
                Err(err) => error_response(id, ErrorCode::InternalError, &err.message),
            }
        }
        QueryMode::Scan => {
            let req = match parse_native_body(&query.body) {
                Ok(r) => r,
                Err(msg) => return error_response(id, ErrorCode::InvalidParams, &msg),
            };
            let limit = req.limit.or(param_limit);
            let esk = next_token.as_deref().and_then(decode_pagination_token);
            match client.scan(&req.table_name, limit, esk).await {
                Ok(res) => ok_response(
                    id,
                    json!(items_to_response(
                        res.items,
                        res.next_token,
                        None,
                        page,
                        started.elapsed().as_millis() as usize,
                        res.consumed_capacity,
                    )),
                ),
                Err(err) => error_response(id, ErrorCode::InternalError, &err.message),
            }
        }
        QueryMode::Query => {
            let req = match parse_native_body(&query.body) {
                Ok(r) => r,
                Err(msg) => return error_response(id, ErrorCode::InvalidParams, &msg),
            };
            let pk_name = match req.partition_key {
                Some(p) => p,
                None => {
                    return error_response(
                        id,
                        ErrorCode::InvalidParams,
                        "#!query requires PartitionKey",
                    )
                }
            };
            let pk_val = match req.partition_value {
                Some(v) => v,
                None => {
                    return error_response(
                        id,
                        ErrorCode::InvalidParams,
                        "#!query requires PartitionValue",
                    )
                }
            };
            let pk_av = crate::dynamodb::models::json_to_attribute_value(&pk_val);
            let sk_name = req.sort_key_name.clone();
            let sk_av = req.sort_key_value.as_ref().map(json_to_av);
            let limit = req.limit.or(param_limit);
            let esk = next_token.as_deref().and_then(decode_pagination_token);
            let args = crate::dynamodb::client::QueryArgs {
                table_name: req.table_name.clone(),
                pk_name,
                pk_val: pk_av,
                sk_name,
                sk_val: sk_av,
                limit,
                exclusive_start_key: esk,
            };
            match client.query(args).await {
                Ok(res) => ok_response(
                    id,
                    json!(items_to_response(
                        res.items,
                        res.next_token,
                        None,
                        page,
                        started.elapsed().as_millis() as usize,
                        res.consumed_capacity,
                    )),
                ),
                Err(err) => error_response(id, ErrorCode::InternalError, &err.message),
            }
        }
        QueryMode::Get => {
            let req = match parse_native_body(&query.body) {
                Ok(r) => r,
                Err(msg) => return error_response(id, ErrorCode::InvalidParams, &msg),
            };
            let key_map = match req.key {
                Some(k) if !k.is_empty() => k,
                _ => {
                    return error_response(
                        id,
                        ErrorCode::InvalidParams,
                        "#!get requires a non-empty Key mapping",
                    )
                }
            };
            let key = key_map
                .iter()
                .map(|(col, val)| (col.clone(), json_to_av(val)))
                .collect();
            match client.get_item(&req.table_name, key).await {
                Ok(res) => ok_response(
                    id,
                    json!(items_to_response(
                        res.items,
                        res.next_token,
                        None,
                        page,
                        started.elapsed().as_millis() as usize,
                        res.consumed_capacity,
                    )),
                ),
                Err(err) => error_response(id, ErrorCode::InternalError, &err.message),
            }
        }
    }
}

/// Shorthand for converting a JSON value to a DynamoDB AttributeValue.
fn json_to_av(v: &Value) -> aws_sdk_dynamodb::types::AttributeValue {
    crate::dynamodb::models::json_to_attribute_value(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_connection_with_empty_params_returns_error() {
        let params = json!({"params": {}});
        let result = test_connection(json!(1), &params).await;
        assert!(result.get("error").is_some());
        assert!(result["error"]["message"]
            .as_str()
            .unwrap()
            .contains("connection params required"));
    }

    #[tokio::test]
    async fn execute_query_with_empty_query_returns_error() {
        let params = json!({"params": {}, "query": ""});
        let result = execute_query(json!(1), &params).await;
        assert!(result.get("error").is_some());
        assert_eq!(
            result["error"]["message"],
            "query must be a non-empty string"
        );
    }

    #[tokio::test]
    async fn execute_query_with_missing_query_returns_error() {
        let params = json!({"params": {}});
        let result = execute_query(json!(1), &params).await;
        assert!(result.get("error").is_some());
    }

    #[test]
    fn strip_limit_removes_trailing_limit() {
        let (stmt, lim) = strip_partiql_limit("SELECT * FROM users LIMIT 10");
        assert_eq!(stmt, "SELECT * FROM users");
        assert_eq!(lim, Some(10));
    }

    #[test]
    fn pagination_contains_app_required_fields() {
        // The Tabularis app deserializes `pagination` into a struct requiring
        // `page`, `page_size`, `total_rows` and `has_more`; omitting any of
        // them fails the whole response with "missing field ...".
        let pg = build_pagination(3, 50, true, Some("tok".to_string()));
        assert_eq!(pg["page"], 3);
        assert_eq!(pg["page_size"], 50);
        assert!(pg["total_rows"].is_null());
        assert_eq!(pg["has_more"], true);
        assert_eq!(pg["next_token"], "tok");

        let pg = build_pagination(1, 100, false, None);
        assert_eq!(pg["page"], 1);
        assert_eq!(pg["has_more"], false);
        assert!(pg.get("next_token").is_none());
    }

    #[test]
    fn items_response_pagination_matches_app_contract() {
        let mut item = std::collections::HashMap::new();
        item.insert("id".to_string(), json!("abc"));
        let resp = items_to_response(vec![item], None, None, 2, 5, None);
        let pg = resp.pagination.expect("pagination must be present");
        assert_eq!(pg["page"], 2);
        assert_eq!(pg["page_size"], 1); // falls back to row count
        assert_eq!(pg["has_more"], false);
        assert!(pg["total_rows"].is_null());
    }

    #[test]
    fn extract_page_defaults_to_one() {
        assert_eq!(extract_page(&json!({})), 1);
        assert_eq!(extract_page(&json!({"page": 0})), 1);
        assert_eq!(extract_page(&json!({"page": 4})), 4);
    }

    #[test]
    fn strip_limit_handles_semicolon_and_case() {
        let (stmt, lim) = strip_partiql_limit("select * from users limit 5;");
        assert_eq!(stmt, "select * from users");
        assert_eq!(lim, Some(5));
    }

    #[test]
    fn strip_limit_no_limit_present() {
        let (stmt, lim) = strip_partiql_limit("SELECT * FROM users");
        assert_eq!(stmt, "SELECT * FROM users");
        assert_eq!(lim, None);
    }

    #[test]
    fn strip_limit_ignores_non_numeric_suffix() {
        // "limited" should not be mistaken for a LIMIT clause.
        let (stmt, lim) = strip_partiql_limit("SELECT limited_col FROM users");
        assert_eq!(stmt, "SELECT limited_col FROM users");
        assert_eq!(lim, None);
    }

    #[test]
    fn parse_scan_body() {
        let req = parse_native_body("TableName: users\nLimit: 100").unwrap();
        assert_eq!(req.table_name, "users");
        assert_eq!(req.limit, Some(100));
    }

    #[test]
    fn parse_query_body() {
        let req =
            parse_native_body("TableName: users\nPartitionKey: id\nPartitionValue: abc").unwrap();
        assert_eq!(req.table_name, "users");
        assert_eq!(req.partition_key.as_deref(), Some("id"));
        assert_eq!(req.partition_value, Some(json!("abc")));
    }

    #[test]
    fn parse_get_body() {
        let req = parse_native_body("TableName: users\nKey:\n  id: abc").unwrap();
        let key = req.key.unwrap();
        assert_eq!(key.get("id"), Some(&json!("abc")));
    }

    #[test]
    fn parse_body_requires_table_name() {
        assert!(parse_native_body("Limit: 5").is_err());
    }

    #[test]
    fn destructive_guard_does_not_block_drop_table() {
        // DROP TABLE is intentionally not blocked: the GUI shows its own
        // confirmation dialog before sending it, and blocking would cause
        // the GUI to silently "succeed" while the table remains.
        assert!(destructive_warning("DROP TABLE users").is_none());
        assert!(destructive_warning("  drop   table  users ").is_none());
    }

    #[test]
    fn destructive_guard_flags_whereless_delete() {
        assert!(destructive_warning("DELETE FROM users").is_some());
        assert!(destructive_warning("delete from users").is_some());
    }

    #[test]
    fn destructive_guard_allows_guarded_delete() {
        assert!(destructive_warning("DELETE FROM users WHERE id = 'x'").is_none());
    }

    #[test]
    fn destructive_guard_allows_select_and_insert() {
        assert!(destructive_warning("SELECT * FROM users").is_none());
        assert!(destructive_warning("INSERT INTO users VALUE {'id': '1'}").is_none());
    }

    #[test]
    fn split_statements_single() {
        assert_eq!(
            split_statements("SELECT * FROM users"),
            vec!["SELECT * FROM users".to_string()]
        );
        assert!(split_statements("").is_empty());
        assert!(split_statements("   ;  ").is_empty());
    }

    #[test]
    fn split_statements_multi() {
        let got = split_statements(
            "INSERT INTO users VALUE {'id': 'u1', 'name': 'Alice'};\n\
             UPDATE orders SET 'status'='shipped' WHERE 'id'='o1';",
        );
        assert_eq!(got.len(), 2);
        assert!(got[0].starts_with("INSERT INTO users"));
        assert!(got[1].starts_with("UPDATE orders"));
    }

    #[test]
    fn split_statements_ignores_semicolon_in_string() {
        let got = split_statements("INSERT INTO users VALUE {'id': 'a;b', 'note': 'x;y;z'}");
        assert_eq!(
            got.len(),
            1,
            "semicolon inside quotes must not split: {got:?}"
        );
        assert!(got[0].contains("a;b"));
    }

    #[test]
    fn split_statements_drops_trailing_empty() {
        let got = split_statements("DELETE FROM a WHERE x='1'; DELETE FROM b WHERE y='2';");
        assert_eq!(got.len(), 2);
    }

    // ---- DDL parsing tests ----

    fn unwrap_create(ddl: Option<DdlStatement>) -> (String, String, Option<String>) {
        match ddl.expect("expected DDL") {
            DdlStatement::CreateTable {
                table_name,
                partition_key,
                sort_key,
                ..
            } => (table_name, partition_key, sort_key),
            DdlStatement::DropTable { .. } => panic!("expected CREATE, got DROP"),
        }
    }

    #[test]
    fn ddl_parses_the_reported_gui_query() {
        // The exact statement the TabularisDB GUI sends for "Create New Table".
        let sql = "CREATE TABLE \"items\" (\"id\" STRING, \"name\" STRING, PRIMARY KEY (\"id\"))";
        let (table, pk, sk) = unwrap_create(parse_ddl(sql));
        assert_eq!(table, "items");
        assert_eq!(pk, "id");
        assert_eq!(sk, None);
    }

    #[test]
    fn ddl_create_captures_key_types() {
        let sql = "CREATE TABLE t (id NUMBER, name STRING, PRIMARY KEY (id))";
        match parse_ddl(sql).expect("ddl") {
            DdlStatement::CreateTable { key_types, .. } => {
                assert_eq!(key_types.get("id").map(|s| s.as_str()), Some("NUMBER"));
                assert_eq!(key_types.get("name").map(|s| s.as_str()), Some("STRING"));
            }
            _ => panic!("expected create"),
        }
    }

    #[test]
    fn ddl_create_with_sort_key() {
        let sql = "CREATE TABLE events (pk STRING, sk STRING, PRIMARY KEY (pk, sk))";
        let (table, pk, sk) = unwrap_create(parse_ddl(sql));
        assert_eq!(table, "events");
        assert_eq!(pk, "pk");
        assert_eq!(sk.as_deref(), Some("sk"));
    }

    #[test]
    fn ddl_create_unquoted_identifiers() {
        let sql = "CREATE TABLE users (id STRING, PRIMARY KEY (id))";
        let (table, pk, _) = unwrap_create(parse_ddl(sql));
        assert_eq!(table, "users");
        assert_eq!(pk, "id");
    }

    #[test]
    fn ddl_create_if_not_exists() {
        let sql = "CREATE TABLE IF NOT EXISTS \"items\" (\"id\" STRING, PRIMARY KEY (\"id\"))";
        let (table, pk, _) = unwrap_create(parse_ddl(sql));
        assert_eq!(table, "items");
        assert_eq!(pk, "id");
    }

    #[test]
    fn ddl_create_is_none_without_primary_key() {
        // No PRIMARY KEY clause -> cannot build a key schema -> treat as
        // non-DDL (falls through to ExecuteStatement, which will error, but
        // we don't guess a key).
        assert!(parse_ddl("CREATE TABLE t (id STRING)").is_none());
    }

    #[test]
    fn ddl_drop_table_quoted() {
        match parse_ddl("DROP TABLE \"items\"").expect("ddl") {
            DdlStatement::DropTable { table_name } => assert_eq!(table_name, "items"),
            _ => panic!("expected drop"),
        }
    }

    #[test]
    fn ddl_drop_table_if_exists() {
        match parse_ddl("DROP TABLE IF EXISTS items").expect("ddl") {
            DdlStatement::DropTable { table_name } => assert_eq!(table_name, "items"),
            _ => panic!("expected drop"),
        }
    }

    #[test]
    fn ddl_is_none_for_dml() {
        assert!(parse_ddl("SELECT * FROM users").is_none());
        assert!(parse_ddl("INSERT INTO users VALUE {'id': '1'}").is_none());
        assert!(parse_ddl("DELETE FROM users WHERE id = '1'").is_none());
    }

    // ── Aggregate parser tests ──────────────────────────────────────────────

    fn agg(sql: &str) -> AggregateQuery {
        parse_aggregate(sql).expect("expected aggregate")
    }

    #[test]
    fn aggregate_count_star_with_alias() {
        // The exact user-reported query.
        let q = agg("SELECT COUNT(*) as count FROM \"orders\"");
        assert_eq!(q.table, "orders");
        assert_eq!(q.func, AggregateFunc::Count { column: None });
        assert_eq!(q.alias, "count");
    }

    #[test]
    fn aggregate_count_star_no_alias_defaults_lowercase() {
        let q = agg("SELECT COUNT(*) FROM users");
        assert_eq!(q.func, AggregateFunc::Count { column: None });
        assert_eq!(q.alias, "count");
    }

    #[test]
    fn aggregate_count_column() {
        let q = agg("SELECT COUNT(\"name\") as named FROM users");
        assert_eq!(
            q.func,
            AggregateFunc::Count {
                column: Some("name".to_owned())
            }
        );
        assert_eq!(q.alias, "named");
    }

    #[test]
    fn aggregate_sum_avg_min_max() {
        assert_eq!(
            agg("SELECT SUM(\"age\") as t FROM users").func,
            AggregateFunc::Sum {
                column: "age".to_owned()
            }
        );
        assert_eq!(
            agg("SELECT AVG(age) FROM users").func,
            AggregateFunc::Avg {
                column: "age".to_owned()
            }
        );
        assert_eq!(
            agg("SELECT MIN(\"age\") as youngest FROM users").func,
            AggregateFunc::Min {
                column: "age".to_owned()
            }
        );
        assert_eq!(
            agg("SELECT MAX(`age`) as oldest FROM users").func,
            AggregateFunc::Max {
                column: "age".to_owned()
            }
        );
    }

    #[test]
    fn aggregate_case_insensitive_keywords() {
        let q = agg("select sum(\"age\") as total from users");
        assert_eq!(
            q.func,
            AggregateFunc::Sum {
                column: "age".to_owned()
            }
        );
        assert_eq!(q.alias, "total");
    }

    #[test]
    fn aggregate_rejects_unsupported_clauses() {
        assert!(parse_aggregate("SELECT COUNT(*) FROM users WHERE id = '1'").is_none());
        assert!(parse_aggregate("SELECT COUNT(*) FROM users GROUP BY name").is_none());
        assert!(parse_aggregate("SELECT COUNT(*) FROM users HAVING name = 'x'").is_none());
        assert!(parse_aggregate("SELECT COUNT(*) FROM users ORDER BY id").is_none());
        assert!(parse_aggregate("SELECT COUNT(*) FROM users LIMIT 10").is_none());
        assert!(parse_aggregate("SELECT COUNT(*) FROM a JOIN b ON a.id = b.id").is_none());
    }

    #[test]
    fn aggregate_rejects_multiple_aggregates() {
        // Two aggregates in one select list -> not a simple single-aggregate.
        assert!(parse_aggregate("SELECT COUNT(*), SUM(age) FROM users").is_none());
    }

    #[test]
    fn aggregate_is_none_for_plain_select() {
        assert!(parse_aggregate("SELECT * FROM users").is_none());
        assert!(parse_aggregate("SELECT id, name FROM users").is_none());
    }

    #[test]
    fn aggregate_rejects_unknown_function() {
        assert!(parse_aggregate("SELECT MEDIAN(\"age\") FROM users").is_none());
    }

    // ── Numeric helpers (DynamoDB N attrs are JSON strings) ─────────────────

    #[test]
    fn as_numeric_accepts_json_number_and_numeric_string() {
        assert_eq!(as_numeric(&json!(42)), Some(42.0));
        assert_eq!(as_numeric(&json!("42")), Some(42.0));
        assert_eq!(as_numeric(&json!("34.5")), Some(34.5));
        assert_eq!(as_numeric(&json!(" 7 ")), Some(7.0));
    }

    #[test]
    fn as_numeric_rejects_non_numeric() {
        assert_eq!(as_numeric(&json!("hello")), None);
        assert_eq!(as_numeric(&json!(null)), None);
        assert_eq!(as_numeric(&json!(true)), None);
        assert_eq!(as_numeric(&json!(["1"])), None);
    }

    #[test]
    fn json_number_keeps_integers_integral() {
        assert_eq!(json_number(5.0), json!(5));
        assert_eq!(json_number(34.75), json!(34.75));
    }
}
