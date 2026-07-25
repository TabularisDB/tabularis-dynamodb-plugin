//! JSON-RPC dispatch and response helpers.

use crate::{error::ErrorCode, handlers};
use serde_json::{json, Value};

/// Parse one JSON-RPC line and return the response value.
/// Never panics — parse errors and method failures are surfaced as JSON-RPC error responses.
pub async fn handle_line(line: &str) -> Value {
    let request: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(err) => {
            return error_response(
                Value::Null,
                ErrorCode::ParseError,
                &format!("parse error: {err}"),
            )
        }
    };

    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    match method.as_str() {
        "initialize" => ok_response(id, json!({"success": true})),
        "ping" => handlers::query::ping(id, &params).await,
        "test_connection" => handlers::query::test_connection(id, &params).await,

        // Metadata
        "get_tables" => handlers::metadata::get_tables(id, &params).await,
        "get_columns" => handlers::metadata::get_columns(id, &params).await,
        // DynamoDB has no database/schema concept — tables are the top-level
        // namespace. Return an empty list rather than a fabricated "default"
        // entry so clients don't try to switch to a nonexistent database (#25).
        "get_databases" => ok_response(id, json!([])),
        "get_schemas" => ok_response(id, json!([])),
        "get_routines" => ok_response(id, json!([])),
        "get_views" => ok_response(id, json!([])),
        "get_foreign_keys" => handlers::metadata::get_foreign_keys(id, &params).await,
        "get_indexes" => handlers::metadata::get_indexes(id, &params).await,

        // Query execution
        "execute_query" => handlers::query::execute_query(id, &params).await,

        // CRUD
        "insert_record" => handlers::crud::insert_record(id, &params).await,
        "update_record" => handlers::crud::update_record(id, &params).await,
        "delete_record" => handlers::crud::delete_record(id, &params).await,

        // DDL
        "get_create_table_sql" => handlers::ddl::get_create_table_sql(id, &params).await,
        "get_add_column_sql" => handlers::ddl::get_add_column_sql(id, &params).await,
        "get_alter_column_sql" => handlers::ddl::get_alter_column_sql(id, &params).await,
        "get_create_index_sql" => handlers::ddl::get_create_index_sql(id, &params).await,
        "get_create_foreign_key_sql" => {
            handlers::ddl::get_create_foreign_key_sql(id, &params).await
        }
        "drop_index" => handlers::ddl::drop_index(id, &params).await,
        "drop_foreign_key" => handlers::ddl::drop_foreign_key(id, &params).await,

        other => not_implemented(id, other),
    }
}

pub fn ok_response(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "result": result,
        "id": id,
    })
}

pub fn error_response(id: Value, code: ErrorCode, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "error": { "code": code.value(), "message": message },
        "id": id,
    })
}

pub fn not_implemented(id: Value, method: &str) -> Value {
    error_response(
        id,
        ErrorCode::MethodNotFound,
        &format!("method '{method}' is not implemented by this plugin yet"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn handle_initialize_returns_success() {
        let line = r#"{"jsonrpc":"2.0","method":"initialize","id":1}"#;
        let response = handle_line(line).await;
        assert_eq!(response["result"]["success"], json!(true));
        assert_eq!(response["id"], json!(1));
    }

    #[tokio::test]
    async fn handle_invalid_json_returns_parse_error() {
        let line = r#"not json"#;
        let response = handle_line(line).await;
        assert!(response["error"]["code"].as_i64().unwrap() < 0);
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("parse error"));
    }

    #[tokio::test]
    async fn handle_unknown_method_returns_method_not_found() {
        let line = r#"{"jsonrpc":"2.0","method":"unknown_method","id":1}"#;
        let response = handle_line(line).await;
        assert_eq!(
            response["error"]["code"],
            json!(ErrorCode::MethodNotFound.value())
        );
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unknown_method"));
    }

    #[tokio::test]
    async fn handle_get_databases_returns_empty() {
        let line = r#"{"jsonrpc":"2.0","method":"get_databases","id":1}"#;
        let response = handle_line(line).await;
        assert_eq!(response["result"], json!([]));
    }

    #[tokio::test]
    async fn handle_get_schemas_returns_empty() {
        let line = r#"{"jsonrpc":"2.0","method":"get_schemas","id":1}"#;
        let response = handle_line(line).await;
        assert_eq!(response["result"], json!([]));
    }

    #[tokio::test]
    async fn handle_get_routines_returns_empty() {
        let line = r#"{"jsonrpc":"2.0","method":"get_routines","id":1}"#;
        let response = handle_line(line).await;
        assert_eq!(response["result"], json!([]));
    }

    #[tokio::test]
    async fn handle_get_views_returns_empty() {
        let line = r#"{"jsonrpc":"2.0","method":"get_views","id":1}"#;
        let response = handle_line(line).await;
        assert_eq!(response["result"], json!([]));
    }

    #[tokio::test]
    async fn ok_response_format() {
        let resp = ok_response(json!(1), json!({"key": "value"}));
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["result"]["key"], "value");
        assert_eq!(resp["id"], 1);
    }

    #[tokio::test]
    async fn error_response_format() {
        let resp = error_response(json!(1), ErrorCode::InvalidParams, "bad input");
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["error"]["code"], ErrorCode::InvalidParams.value());
        assert_eq!(resp["error"]["message"], "bad input");
        assert_eq!(resp["id"], 1);
    }
}
