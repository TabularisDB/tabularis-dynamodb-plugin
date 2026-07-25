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

/// Build the standard tabular response from a set of DynamoDB items.
fn items_to_response(
    items: Vec<std::collections::HashMap<String, Value>>,
    next_token: Option<String>,
    limit: Option<usize>,
    execution_time_ms: usize,
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

    ExecuteQueryResponse {
        affected_rows: rows.len(),
        execution_time_ms,
        truncated: truncated_by_limit,
        has_more,
        pagination: next_token.map(|t| json!({"next_token": t})),
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

    let client = match connection::build_client(params).await {
        Ok(c) => c,
        Err(err) => return error_response(id, err.code, &err.message),
    };

    let started = std::time::Instant::now();

    match query.mode {
        QueryMode::Partiql => {
            // #20: strip an unsupported LIMIT clause and apply it client-side.
            let (statement, inline_limit) = strip_partiql_limit(&query.body);
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
                        started.elapsed().as_millis() as usize,
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
                        started.elapsed().as_millis() as usize,
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
                        started.elapsed().as_millis() as usize,
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
                        started.elapsed().as_millis() as usize,
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
}
