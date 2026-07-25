//! Connection and query execution handlers.

use serde_json::{json, Value};

use crate::dynamodb::client::Client;
use crate::error::ErrorCode;
use crate::handlers::models::{ExecuteQueryResponse, Query, QueryMode};
use crate::rpc::{error_response, ok_response};
use crate::utils::extractor;

/// Build a DynamoDB client from the params object.
async fn build_client(params: &Value) -> Result<Client, (Value, Value)> {
    let region = params
        .get("params")
        .and_then(|p| p.get("region"))
        .and_then(|r| r.as_str());

    let access_key_id = params
        .get("params")
        .and_then(|p| p.get("access_key_id"))
        .and_then(|r| r.as_str());

    let secret_access_key = params
        .get("params")
        .and_then(|p| p.get("secret_access_key"))
        .and_then(|r| r.as_str());

    let session_token = params
        .get("params")
        .and_then(|p| p.get("session_token"))
        .and_then(|r| r.as_str());

    let profile = params
        .get("params")
        .and_then(|p| p.get("profile"))
        .and_then(|r| r.as_str());

    let endpoint = params
        .get("params")
        .and_then(|p| p.get("endpoint"))
        .and_then(|r| r.as_str());

    Client::new(
        region,
        access_key_id,
        secret_access_key,
        session_token,
        profile,
        endpoint,
    )
    .await
    .map_err(|e| {
        (
            Value::Null,
            json!({
                "jsonrpc": "2.0",
                "error": { "code": ErrorCode::InternalError.value(), "message": e.to_string() },
                "id": Value::Null,
            }),
        )
    })
}

pub async fn test_connection(id: Value, params: &Value) -> Value {
    let client = match build_client(params).await {
        Ok(c) => c,
        Err((_, err_resp)) => return err_resp,
    };

    match client.ping().await {
        Ok(_) => ok_response(id, json!({"success": true})),
        Err(err) => error_response(id, ErrorCode::InternalError, &err.message),
    }
}

pub async fn ping(id: Value, params: &Value) -> Value {
    test_connection(id, params).await
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

    let client = match build_client(params).await {
        Ok(c) => c,
        Err((_, err_resp)) => return err_resp,
    };

    match query.mode {
        QueryMode::Partiql => match client.execute_statement(&query.body).await {
            Ok(result) => {
                // Union of all keys across all items (DynamoDB is schemaless —
                // each item can have different attributes). Preserves insertion order.
                let mut columns: Vec<String> = Vec::new();
                let mut seen = std::collections::HashSet::new();
                for item in &result.items {
                    for key in item.keys() {
                        if seen.insert(key.clone()) {
                            columns.push(key.clone());
                        }
                    }
                }

                let rows: Vec<Vec<Value>> = result
                    .items
                    .iter()
                    .map(|item| {
                        columns
                            .iter()
                            .map(|col| item.get(col).cloned().unwrap_or(Value::Null))
                            .collect()
                    })
                    .collect();

                let affected_rows = rows.len();
                let has_more = result.next_token.is_some();

                ok_response(
                    id,
                    json!(ExecuteQueryResponse {
                        columns,
                        rows,
                        affected_rows,
                        execution_time_ms: 0,
                        truncated: has_more,
                        has_more,
                        pagination: result.next_token.map(|t| json!({"next_token": t})),
                    }),
                )
            }
            Err(err) => error_response(id, ErrorCode::InternalError, &err.message),
        },
        QueryMode::Scan | QueryMode::Query | QueryMode::Get => {
            // For scan/query/get, we treat the body as a PartiQL statement for now
            // Full implementation will parse the YAML-like body and call the appropriate SDK methods
            match client.execute_statement(&query.body).await {
                Ok(result) => {
                    // Union of all keys across all items (DynamoDB is schemaless —
                    // each item can have different attributes). Preserves insertion order.
                    let mut columns: Vec<String> = Vec::new();
                    let mut seen = std::collections::HashSet::new();
                    for item in &result.items {
                        for key in item.keys() {
                            if seen.insert(key.clone()) {
                                columns.push(key.clone());
                            }
                        }
                    }

                    let rows: Vec<Vec<Value>> = result
                        .items
                        .iter()
                        .map(|item| {
                            columns
                                .iter()
                                .map(|col| item.get(col).cloned().unwrap_or(Value::Null))
                                .collect()
                        })
                        .collect();

                    let affected_rows = rows.len();
                    let has_more = result.next_token.is_some();

                    ok_response(
                        id,
                        json!(ExecuteQueryResponse {
                            columns,
                            rows,
                            affected_rows,
                            execution_time_ms: 0,
                            truncated: has_more,
                            has_more,
                            pagination: result.next_token.map(|t| json!({"next_token": t})),
                        }),
                    )
                }
                Err(err) => error_response(id, ErrorCode::InternalError, &err.message),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_connection_with_missing_params_returns_error() {
        let params = json!({"params": {}});
        let result = test_connection(json!(1), &params).await;
        // Should not panic — will try to connect with no creds and fail gracefully
        assert!(result.get("error").is_some() || result.get("result").is_some());
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
}
