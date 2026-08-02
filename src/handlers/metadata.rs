//! Schema metadata: tables, columns, indexes, foreign keys.

use futures::stream::{self, StreamExt};
use serde_json::{json, Value};

use crate::error::ErrorCode;
use crate::handlers::connection;
use crate::handlers::models::ColumnResponse;
use crate::rpc::{error_response, ok_response};
use crate::utils::extractor;

/// Max in-flight DescribeTable calls while building the table list. On
/// accounts with hundreds of tables a serial loop takes over a minute
/// (~300ms per describe), which trips the GUI's connection timeout. Bounding
/// concurrency keeps the full metadata (item count, size, status) without
/// overwhelming the account's DescribeTable rate limit.
const DESCRIBE_TABLE_CONCURRENCY: usize = 16;

/// Returns the list of tables in DynamoDB with metadata.
pub async fn get_tables(id: Value, params: &Value) -> Value {
    let client = match connection::build_client(params).await {
        Ok(c) => c,
        Err(err) => return error_response(id, err.code, &err.message),
    };

    match client.list_tables().await {
        Ok(table_names) => {
            let mut results: Vec<Value> = stream::iter(table_names.into_iter().map(|name| {
                let client = client.clone();
                async move {
                    match client.describe_table(&name).await {
                        Ok(desc) => json!({
                            "name": name,
                            "comment": null,
                            "item_count": desc.item_count.unwrap_or(0),
                            "table_size_bytes": desc.table_size_bytes.unwrap_or(0),
                            "table_status": desc.table_status.unwrap_or_else(|| "ACTIVE".to_string()),
                        }),
                        Err(_) => json!({
                            "name": name,
                            "comment": null,
                            "item_count": 0,
                            "table_size_bytes": 0,
                            "table_status": "UNKNOWN",
                        }),
                    }
                }
            }))
            .buffer_unordered(DESCRIBE_TABLE_CONCURRENCY)
            .collect()
            .await;
            // buffer_unordered completes out of order; restore alphabetical
            // ordering so the sidebar matches ListTables order.
            results.sort_by(|a, b| {
                a["name"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["name"].as_str().unwrap_or(""))
            });
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
            // The GUI (src/utils/indexes.ts groupIndexes) expects one row per
            // indexed column with `column_name` (singular), then collapses
            // rows client-side into grouped indexes. Returning `columns`
            // (array) leaves the GUI showing 0 indexes because it can't
            // match the shape it expects.
            let indexes: Vec<Value> = desc
                .indexes
                .iter()
                .flat_map(|idx| {
                    idx.columns.iter().enumerate().map(move |(seq, col)| {
                        json!({
                            "name": &idx.name,
                            "column_name": col,
                            "is_unique": idx.is_unique,
                            "is_primary": idx.is_primary,
                            "seq_in_index": seq + 1,
                        })
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
