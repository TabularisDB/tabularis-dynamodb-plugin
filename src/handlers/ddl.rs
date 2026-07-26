//! DDL handlers: generate SQL/PartiQL statements for schema changes.

use serde_json::{json, Value};

use crate::error::ErrorCode;
use crate::handlers::connection;
use crate::rpc::{error_response, ok_response};

/// Generate a PartiQL CREATE TABLE statement.
///
/// Builds the statement from optional `columns` and `key_schema` params:
///   - `columns`: array of `{name, type}` objects (type is a DynamoDB type code
///     or a SQL type name).
///   - `key_schema`: `{partition_key, sort_key}` (sort_key optional).
///
/// When no columns are supplied, falls back to a single `id STRING` primary key
/// (backward-compatible with the original stub). Composite HASH + RANGE keys are
/// emitted as `PRIMARY KEY (pk, sk)`.
pub async fn get_create_table_sql(id: Value, params: &Value) -> Value {
    let table_name = params
        .get("table_name")
        .and_then(|t| t.as_str())
        .unwrap_or("");

    if table_name.is_empty() {
        return error_response(
            id,
            ErrorCode::InvalidParams,
            "table_name must be a non-empty string",
        );
    }

    let columns = params
        .get("columns")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    let name = v.get("name").and_then(|n| n.as_str())?;
                    let raw_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("STRING");
                    Some((name.to_string(), dynamodb_type_to_sql(raw_type)))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let partition_key = params
        .get("key_schema")
        .and_then(|k| k.get("partition_key"))
        .and_then(|p| p.as_str())
        .map(|s| s.to_string());

    let sort_key = params
        .get("key_schema")
        .and_then(|k| k.get("sort_key"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());

    let sql = if columns.is_empty() && partition_key.is_none() {
        // Backward-compatible default when no schema information is provided.
        format!(
            "CREATE TABLE \"{}\" (id STRING, PRIMARY KEY (id))",
            table_name
        )
    } else {
        let mut col_defs: Vec<String> = Vec::new();
        let mut key_cols: Vec<String> = Vec::new();

        if let Some(pk) = &partition_key {
            let pk_type = columns
                .iter()
                .find(|(n, _)| n == pk)
                .map(|(_, t)| t.clone())
                .unwrap_or_else(|| "STRING".to_string());
            col_defs.push(format!("\"{}\" {}", pk, pk_type));
            key_cols.push(format!("\"{}\"", pk));
        }
        if let Some(sk) = &sort_key {
            let sk_type = columns
                .iter()
                .find(|(n, _)| n == sk)
                .map(|(_, t)| t.clone())
                .unwrap_or_else(|| "STRING".to_string());
            col_defs.push(format!("\"{}\" {}", sk, sk_type));
            key_cols.push(format!("\"{}\"", sk));
        }
        for (name, sql_type) in &columns {
            if Some(name) == partition_key.as_ref() || Some(name) == sort_key.as_ref() {
                continue;
            }
            col_defs.push(format!("\"{}\" {}", name, sql_type));
        }

        let pk = if key_cols.is_empty() {
            "PRIMARY KEY (\"id\")".to_string()
        } else {
            format!("PRIMARY KEY ({})", key_cols.join(", "))
        };

        format!(
            "CREATE TABLE \"{}\" ({}, {})",
            table_name,
            col_defs.join(", "),
            pk
        )
    };

    ok_response(id, json!([sql]))
}

/// Map a DynamoDB attribute type code to a PartiQL/SQL column type.
/// SQL type names (already uppercase keywords) are passed through unchanged.
fn dynamodb_type_to_sql(raw: &str) -> String {
    match raw.to_uppercase().as_str() {
        "S" | "STRING" => "STRING".to_string(),
        "N" | "NUMBER" => "NUMBER".to_string(),
        "B" | "BINARY" => "BINARY".to_string(),
        "BOOL" | "BOOLEAN" => "BOOLEAN".to_string(),
        "L" | "M" | "JSON" => "JSON".to_string(),
        other => other.to_string(),
    }
}

/// Generate a PartiQL ALTER TABLE ADD COLUMN statement.
pub async fn get_add_column_sql(id: Value, params: &Value) -> Value {
    let table = params.get("table").and_then(|t| t.as_str()).unwrap_or("");

    let column_name = params
        .get("column")
        .and_then(|c| c.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("");

    let column_type = params
        .get("column")
        .and_then(|c| c.get("type"))
        .and_then(|t| t.as_str())
        .unwrap_or("STRING");

    if table.is_empty() || column_name.is_empty() {
        return error_response(
            id,
            ErrorCode::InvalidParams,
            "table and column name are required",
        );
    }

    let sql = format!(
        "ALTER TABLE \"{}\" ADD COLUMN \"{}\" {}",
        table, column_name, column_type
    );

    ok_response(id, json!([sql]))
}

/// Generate a PartiQL ALTER TABLE MODIFY statement.
pub async fn get_alter_column_sql(id: Value, params: &Value) -> Value {
    let table = params.get("table").and_then(|t| t.as_str()).unwrap_or("");

    let old_name = params
        .get("old_column")
        .and_then(|c| c.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("");

    let new_name = params
        .get("new_column")
        .and_then(|c| c.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("");

    if table.is_empty() || old_name.is_empty() {
        return error_response(
            id,
            ErrorCode::InvalidParams,
            "table and old column name are required",
        );
    }

    if old_name != new_name && !old_name.is_empty() && !new_name.is_empty() {
        let sql = format!(
            "ALTER TABLE \"{}\" MODIFY \"{}\" NAME \"{}\"",
            table, old_name, new_name
        );
        ok_response(id, json!([sql]))
    } else {
        ok_response(id, json!(["// No rename needed"]))
    }
}

/// Generate a PartiQL CREATE INDEX statement.
pub async fn get_create_index_sql(id: Value, params: &Value) -> Value {
    let table = params.get("table").and_then(|t| t.as_str()).unwrap_or("");

    let columns: Vec<String> = params
        .get("columns")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    if table.is_empty() || columns.is_empty() {
        return error_response(
            id,
            ErrorCode::InvalidParams,
            "table and columns are required",
        );
    }

    let _index_name = format!("{}_{}_index", table, columns.join("_"));
    let cols: Vec<String> = columns.iter().map(|c| format!("\"{}\"", c)).collect();

    let sql = format!(
        "CREATE INDEX ON \"{}\" ({}) INCLUDE ALL",
        table,
        cols.join(", ")
    );

    ok_response(id, json!([sql]))
}

/// Generate a PartiQL DROP INDEX statement.
pub async fn drop_index(id: Value, params: &Value) -> Value {
    let table = params.get("table").and_then(|t| t.as_str()).unwrap_or("");

    let index_name = params
        .get("index_name")
        .and_then(|n| n.as_str())
        .unwrap_or("");

    if table.is_empty() || index_name.is_empty() {
        return error_response(
            id,
            ErrorCode::InvalidParams,
            "table and index_name are required",
        );
    }

    let sql = format!("DROP INDEX \"{}\" ON \"{}\"", index_name, table);

    ok_response(id, json!([sql]))
}

/// Generate a PartiQL CREATE FOREIGN KEY statement (not supported by DynamoDB).
pub async fn get_create_foreign_key_sql(id: Value, _params: &Value) -> Value {
    ok_response(
        id,
        json!(["// DynamoDB does not support foreign key constraints"]),
    )
}

/// Drop a foreign key (not supported by DynamoDB).
pub async fn drop_foreign_key(id: Value, _params: &Value) -> Value {
    ok_response(
        id,
        json!(["// DynamoDB does not support foreign key constraints"]),
    )
}

/// Drop a table via the native DynamoDB DeleteTable control-plane API.
///
/// The TabularisDB GUI invokes `drop_table` directly when the user deletes a
/// table from the sidebar. Unlike the `execute_query` DDL interception path
/// (which requires `allow_destructive: true`), this handler is an explicit
/// user-initiated action from the GUI's delete confirmation dialog, so no
/// additional destructive guard is applied here.
pub async fn drop_table(id: Value, params: &Value) -> Value {
    let table_name = params
        .get("table")
        .and_then(|t| t.as_str())
        .or_else(|| params.get("table_name").and_then(|t| t.as_str()))
        .unwrap_or("");

    if table_name.is_empty() {
        return error_response(
            id,
            ErrorCode::InvalidParams,
            "table must be a non-empty string",
        );
    }

    let client = match connection::build_client(params).await {
        Ok(c) => c,
        Err(err) => return error_response(id, err.code, &err.message),
    };

    match client.delete_table(table_name).await {
        Ok(()) => ok_response(
            id,
            json!({
                "success": true,
                "message": format!("Table \"{table_name}\" deleted")
            }),
        ),
        Err(err) => error_response(id, err.code, &err.message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn get_create_table_sql_with_empty_name_returns_error() {
        let params = json!({"params": {}, "table_name": ""});
        let result = get_create_table_sql(json!(1), &params).await;
        assert!(result.get("error").is_some());
    }

    #[tokio::test]
    async fn get_create_table_sql_generates_statement() {
        let params = json!({"params": {}, "table_name": "users"});
        let result = get_create_table_sql(json!(1), &params).await;
        let statements = result["result"].as_array().unwrap();
        assert!(statements[0].as_str().unwrap().contains("CREATE TABLE"));
        assert!(statements[0].as_str().unwrap().contains("users"));
    }

    #[tokio::test]
    async fn get_create_table_sql_uses_key_schema_and_columns() {
        let params = json!({
            "params": {},
            "table_name": "orders",
            "columns": [
                {"name": "id", "type": "S"},
                {"name": "created_at", "type": "S"},
                {"name": "total", "type": "N"},
                {"name": "meta", "type": "M"}
            ],
            "key_schema": {"partition_key": "id", "sort_key": "created_at"}
        });
        let result = get_create_table_sql(json!(1), &params).await;
        let sql = result["result"].as_array().unwrap()[0]
            .as_str()
            .unwrap()
            .to_string();
        // Composite key in PRIMARY KEY clause
        assert!(
            sql.contains("PRIMARY KEY (\"id\", \"created_at\")"),
            "sql was: {sql}"
        );
        // Key columns typed from columns list
        assert!(sql.contains("\"id\" STRING"), "sql was: {sql}");
        assert!(sql.contains("\"created_at\" STRING"), "sql was: {sql}");
        // Non-key columns typed from DynamoDB codes
        assert!(sql.contains("\"total\" NUMBER"), "sql was: {sql}");
        assert!(sql.contains("\"meta\" JSON"), "sql was: {sql}");
    }

    #[tokio::test]
    async fn get_create_table_sql_partition_key_only() {
        let params = json!({
            "params": {},
            "table_name": "users",
            "columns": [{"name": "email", "type": "S"}],
            "key_schema": {"partition_key": "email"}
        });
        let result = get_create_table_sql(json!(1), &params).await;
        let sql = result["result"].as_array().unwrap()[0]
            .as_str()
            .unwrap()
            .to_string();
        assert!(sql.contains("PRIMARY KEY (\"email\")"), "sql was: {sql}");
        assert!(sql.contains("\"email\" STRING"), "sql was: {sql}");
    }

    #[tokio::test]
    async fn get_create_table_sql_falls_back_to_default_without_schema() {
        let params = json!({"params": {}, "table_name": "legacy"});
        let result = get_create_table_sql(json!(1), &params).await;
        let sql = result["result"].as_array().unwrap()[0]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(
            sql, "CREATE TABLE \"legacy\" (id STRING, PRIMARY KEY (id))",
            "backward-compatible stub changed"
        );
    }

    #[tokio::test]
    async fn get_add_column_sql_generates_statement() {
        let params = json!({
            "params": {},
            "table": "users",
            "column": {"name": "email", "type": "STRING"}
        });
        let result = get_add_column_sql(json!(1), &params).await;
        let statements = result["result"].as_array().unwrap();
        assert!(statements[0].as_str().unwrap().contains("ALTER TABLE"));
        assert!(statements[0].as_str().unwrap().contains("email"));
    }

    #[tokio::test]
    async fn get_create_index_sql_generates_statement() {
        let params = json!({
            "params": {},
            "table": "users",
            "columns": ["email"]
        });
        let result = get_create_index_sql(json!(1), &params).await;
        let statements = result["result"].as_array().unwrap();
        assert!(statements[0].as_str().unwrap().contains("CREATE INDEX"));
    }

    #[tokio::test]
    async fn drop_index_generates_statement() {
        let params = json!({
            "params": {},
            "table": "users",
            "index_name": "email-index"
        });
        let result = drop_index(json!(1), &params).await;
        let statements = result["result"].as_array().unwrap();
        assert!(statements[0].as_str().unwrap().contains("DROP INDEX"));
    }

    #[tokio::test]
    async fn get_create_foreign_key_sql_returns_not_supported() {
        let params = json!({"params": {}});
        let result = get_create_foreign_key_sql(json!(1), &params).await;
        let statements = result["result"].as_array().unwrap();
        assert!(statements[0].as_str().unwrap().contains("does not support"));
    }

    #[tokio::test]
    async fn drop_table_with_missing_table_returns_error() {
        let params = json!({"params": {"host": "localhost", "port": "8000"}});
        let result = drop_table(json!(1), &params).await;
        assert!(result.get("error").is_some());
        assert_eq!(
            result["error"]["message"],
            "table must be a non-empty string"
        );
    }

    #[tokio::test]
    async fn drop_table_with_empty_table_returns_error() {
        let params = json!({"params": {"host": "localhost", "port": "8000"}, "table": ""});
        let result = drop_table(json!(1), &params).await;
        assert!(result.get("error").is_some());
        assert_eq!(
            result["error"]["message"],
            "table must be a non-empty string"
        );
    }

    #[tokio::test]
    async fn drop_table_accepts_table_name_param() {
        // Both "table" and "table_name" should be accepted as the table
        // identifier. Without a live endpoint the client construction will
        // fail, but it must NOT fail on "table must be a non-empty string".
        let params = json!({
            "params": {"host": "localhost", "port": "8000", "username": "x", "password": "x"},
            "table_name": "my-table"
        });
        let result = drop_table(json!(1), &params).await;
        // If there's an error it should be a connection error, not a param error.
        if let Some(err) = result.get("error") {
            assert!(
                !err["message"].as_str().unwrap().contains("table must be"),
                "table_name param was not recognised"
            );
        }
    }
}
