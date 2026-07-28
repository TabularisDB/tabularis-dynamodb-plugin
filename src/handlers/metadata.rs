//! Schema metadata: tables, columns, indexes, foreign keys.

use serde_json::{json, Value};

use crate::error::ErrorCode;
use crate::handlers::connection;
use crate::handlers::models::ColumnResponse;
use crate::rpc::{error_response, ok_response};
use crate::utils::extractor;

/// Returns the list of tables in DynamoDB with metadata.
pub async fn get_tables(id: Value, params: &Value) -> Value {
    let client = match connection::build_client(params).await {
        Ok(c) => c,
        Err(err) => return error_response(id, err.code, &err.message),
    };

    match client.list_tables().await {
        Ok(table_names) => {
            let mut results = Vec::new();
            for name in table_names {
                // Fetch full table metadata via describe_table
                match client.describe_table(&name).await {
                    Ok(desc) => {
                        results.push(json!({
                            "name": name,
                            "comment": null,
                            "item_count": desc.item_count.unwrap_or(0),
                            "table_size_bytes": desc.table_size_bytes.unwrap_or(0),
                            "table_status": desc.table_status.unwrap_or_else(|| "ACTIVE".to_string()),
                        }));
                    }
                    Err(_) => {
                        // Fallback if describe_table fails
                        results.push(json!({
                            "name": name,
                            "comment": null,
                            "item_count": 0,
                            "table_size_bytes": 0,
                            "table_status": "UNKNOWN",
                        }));
                    }
                }
            }
            ok_response(id, json!(results))
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

    let client = match connection::build_client(params).await {
        Ok(c) => c,
        Err(err) => return error_response(id, err.code, &err.message),
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

    let client = match connection::build_client(params).await {
        Ok(c) => c,
        Err(err) => return error_response(id, err.code, &err.message),
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
    async fn get_tables_with_missing_params_returns_error() {
        let params = json!({"params": {}});
        let result = get_tables(json!(1), &params).await;
        assert!(result.get("error").is_some());
        assert!(result["error"]["message"]
            .as_str()
            .unwrap()
            .contains("connection params required"));
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
