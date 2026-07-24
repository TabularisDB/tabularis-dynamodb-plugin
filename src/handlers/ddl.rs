//! DDL handlers: generate SQL/PartiQL statements for schema changes.

use serde_json::{json, Value};

use crate::error::ErrorCode;
use crate::rpc::{error_response, ok_response};

/// Generate a PartiQL CREATE TABLE statement.
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

    // Generate a basic CREATE TABLE statement
    // The full implementation would inspect columns and key schema from params
    let sql = format!(
        "CREATE TABLE \"{}\" (id STRING, PRIMARY KEY (id))",
        table_name
    );

    ok_response(id, json!([sql]))
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
}
