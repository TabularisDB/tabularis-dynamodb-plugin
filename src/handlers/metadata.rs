//! Schema metadata: tables, columns, indexes, foreign keys.

use serde_json::{json, Value};

use crate::dynamodb::client::Client;
use crate::error::ErrorCode;
use crate::handlers::models::ColumnResponse;
use crate::rpc::{error_response, ok_response};
use crate::utils::extractor;

/// Build a DynamoDB client from params (shared helper).
async fn build_client(params: &Value) -> Result<Client, Value> {
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
        json!({
            "code": ErrorCode::InternalError.value(),
            "message": e.to_string()
        })
    })
}

/// Returns the list of tables in DynamoDB.
pub async fn get_tables(id: Value, params: &Value) -> Value {
    let client = match build_client(params).await {
        Ok(c) => c,
        Err(err) => {
            return error_response(
                id,
                ErrorCode::InternalError,
                err["message"].as_str().unwrap_or("unknown error"),
            );
        }
    };

    match client.list_tables().await {
        Ok(tables) => {
            let result: Vec<Value> = tables
                .into_iter()
                .map(|name| {
                    json!({
                        "name": name,
                        "schema": null,
                        "comment": null,
                    })
                })
                .collect();
            ok_response(id, json!(result))
        }
        Err(err) => error_response(id, ErrorCode::InternalError, &err.message),
    }
}

/// Returns the columns (attribute definitions) for a given table.
pub async fn get_columns(id: Value, params: &Value) -> Value {
    let table_name = match extractor::extract_table(params) {
        Some(tb) if !tb.is_empty() => tb,
        _ => {
            return error_response(
                id,
                ErrorCode::InvalidParams,
                "table must be a non-empty string",
            )
        }
    };

    let client = match build_client(params).await {
        Ok(c) => c,
        Err(err) => {
            return error_response(
                id,
                ErrorCode::InternalError,
                err["message"].as_str().unwrap_or("unknown error"),
            );
        }
    };

    match client.describe_table(&table_name).await {
        Ok(desc) => {
            let columns: Vec<ColumnResponse> = desc
                .columns
                .iter()
                .map(|col| ColumnResponse {
                    name: col.name.clone(),
                    data_type: col.data_type.clone(),
                    is_pk: col.is_pk,
                    // In DynamoDB, only key attributes (HASH + RANGE) are required.
                    // All non-key attributes are optional/nullable.
                    is_nullable: !(col.is_pk || col.is_sort_key),
                    is_auto_increment: false,
                })
                .collect();

            ok_response(id, json!(columns))
        }
        Err(err) => error_response(id, ErrorCode::InternalError, &err.message),
    }
}

/// Returns indexes (GSI + LSI) for a given table.
pub async fn get_indexes(id: Value, params: &Value) -> Value {
    let table_name = match extractor::extract_table(params) {
        Some(tb) if !tb.is_empty() => tb,
        _ => {
            return error_response(
                id,
                ErrorCode::InvalidParams,
                "table must be a non-empty string",
            )
        }
    };

    let client = match build_client(params).await {
        Ok(c) => c,
        Err(err) => {
            return error_response(
                id,
                ErrorCode::InternalError,
                err["message"].as_str().unwrap_or("unknown error"),
            );
        }
    };

    match client.describe_table(&table_name).await {
        Ok(desc) => {
            let indexes: Vec<Value> = desc
                .indexes
                .iter()
                .map(|idx| {
                    json!({
                        "name": idx.name,
                        "columns": idx.columns,
                        "is_unique": idx.is_unique,
                        "is_primary": idx.is_primary,
                    })
                })
                .collect();

            ok_response(id, json!(indexes))
        }
        Err(err) => error_response(id, ErrorCode::InternalError, &err.message),
    }
}

/// Returns empty foreign keys (DynamoDB has no FK constraints).
pub async fn get_foreign_keys(id: Value, _params: &Value) -> Value {
    ok_response(id, json!([]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn get_tables_with_missing_params_returns_error_or_empty() {
        let params = json!({"params": {}});
        let result = get_tables(json!(1), &params).await;
        // Should not panic — will try to connect and fail gracefully or return empty
        assert!(result.get("error").is_some() || result.get("result").is_some());
    }

    #[tokio::test]
    async fn get_columns_with_empty_table_returns_error() {
        let params = json!({"params": {}, "table": ""});
        let result = get_columns(json!(1), &params).await;
        assert!(result.get("error").is_some());
        assert_eq!(
            result["error"]["message"],
            "table must be a non-empty string"
        );
    }

    #[tokio::test]
    async fn get_foreign_keys_returns_empty_array() {
        let params = json!({"params": {}});
        let result = get_foreign_keys(json!(1), &params).await;
        assert_eq!(result["result"], json!([]));
    }

    #[tokio::test]
    async fn get_indexes_with_empty_table_returns_error() {
        let params = json!({"params": {}, "table": ""});
        let result = get_indexes(json!(1), &params).await;
        assert!(result.get("error").is_some());
    }
}
