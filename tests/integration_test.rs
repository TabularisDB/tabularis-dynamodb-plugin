//! Integration tests for the JSON-RPC plugin pipeline.
//!
//! These tests verify the full request/response cycle by calling `rpc::handle_line`
//! directly (which is what the worker pool does). They do NOT require a real
//! DynamoDB instance — they test parsing, dispatch, error handling, and response
//! formatting.

use dynamodb_plugin::rpc;
use serde_json::json;

#[tokio::test]
async fn full_initialize_flow() {
    let req = r#"{"jsonrpc":"2.0","method":"initialize","id":1}"#;
    let resp = rpc::handle_line(req).await;

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["result"]["success"], true);
    assert_eq!(resp["id"], 1);
}

#[tokio::test]
async fn full_get_databases_flow() {
    let req = r#"{"jsonrpc":"2.0","method":"get_databases","id":2}"#;
    let resp = rpc::handle_line(req).await;

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["result"], json!([]));
    assert_eq!(resp["id"], 2);
}

#[tokio::test]
async fn full_get_schemas_flow() {
    let req = r#"{"jsonrpc":"2.0","method":"get_schemas","id":3}"#;
    let resp = rpc::handle_line(req).await;

    assert_eq!(resp["result"], json!([]));
}

#[tokio::test]
async fn full_get_routines_flow() {
    let req = r#"{"jsonrpc":"2.0","method":"get_routines","id":4}"#;
    let resp = rpc::handle_line(req).await;

    assert_eq!(resp["result"], json!([]));
}

#[tokio::test]
async fn full_get_views_flow() {
    let req = r#"{"jsonrpc":"2.0","method":"get_views","id":5}"#;
    let resp = rpc::handle_line(req).await;

    assert_eq!(resp["result"], json!([]));
}

#[tokio::test]
async fn full_get_foreign_keys_flow() {
    let req = r#"{"jsonrpc":"2.0","method":"get_foreign_keys","id":6,"params":{"params":{}}}"#;
    let resp = rpc::handle_line(req).await;

    assert_eq!(resp["result"], json!([]));
}

#[tokio::test]
async fn full_unknown_method_flow() {
    let req = r#"{"jsonrpc":"2.0","method":"nonexistent","id":7}"#;
    let resp = rpc::handle_line(req).await;

    assert!(resp.get("error").is_some());
    assert_eq!(resp["error"]["code"], -32601);
    assert!(resp["error"]["message"]
        .as_str()
        .unwrap()
        .contains("nonexistent"));
}

#[tokio::test]
async fn full_malformed_json_flow() {
    let req = r#"not valid json"#;
    let resp = rpc::handle_line(req).await;

    assert!(resp.get("error").is_some());
    assert_eq!(resp["error"]["code"], -32700); // ParseError
}

#[tokio::test]
async fn full_execute_query_empty_flow() {
    let req =
        r#"{"jsonrpc":"2.0","method":"execute_query","id":8,"params":{"params":{},"query":""}}"#;
    let resp = rpc::handle_line(req).await;

    assert!(resp.get("error").is_some());
    assert_eq!(resp["error"]["message"], "query must be a non-empty string");
}

#[tokio::test]
async fn full_get_columns_empty_table_flow() {
    let req =
        r#"{"jsonrpc":"2.0","method":"get_columns","id":9,"params":{"params":{},"table":""}}"#;
    let resp = rpc::handle_line(req).await;

    assert!(resp.get("error").is_some());
    assert_eq!(resp["error"]["message"], "table must be a non-empty string");
}

#[tokio::test]
async fn full_get_indexes_empty_table_flow() {
    let req =
        r#"{"jsonrpc":"2.0","method":"get_indexes","id":10,"params":{"params":{},"table":""}}"#;
    let resp = rpc::handle_line(req).await;

    assert!(resp.get("error").is_some());
}

#[tokio::test]
async fn full_insert_record_empty_table_flow() {
    let req = r#"{"jsonrpc":"2.0","method":"insert_record","id":11,"params":{"params":{},"table":"","data":{}}}"#;
    let resp = rpc::handle_line(req).await;

    assert!(resp.get("error").is_some());
}

#[tokio::test]
async fn full_update_record_missing_params_flow() {
    let req = r#"{"jsonrpc":"2.0","method":"update_record","id":12,"params":{"params":{},"table":"users"}}"#;
    let resp = rpc::handle_line(req).await;

    assert!(resp.get("error").is_some());
}

#[tokio::test]
async fn full_delete_record_missing_params_flow() {
    let req = r#"{"jsonrpc":"2.0","method":"delete_record","id":13,"params":{"params":{},"table":"users"}}"#;
    let resp = rpc::handle_line(req).await;

    assert!(resp.get("error").is_some());
}

#[tokio::test]
async fn full_get_create_table_sql_flow() {
    let req = r#"{"jsonrpc":"2.0","method":"get_create_table_sql","id":14,"params":{"params":{},"table_name":"users"}}"#;
    let resp = rpc::handle_line(req).await;

    let statements = resp["result"].as_array().unwrap();
    assert!(statements[0].as_str().unwrap().contains("CREATE TABLE"));
    assert!(statements[0].as_str().unwrap().contains("users"));
}

#[tokio::test]
async fn full_get_add_column_sql_flow() {
    let req = r#"{"jsonrpc":"2.0","method":"get_add_column_sql","id":15,"params":{"params":{},"table":"users","column":{"name":"email","type":"STRING"}}}"#;
    let resp = rpc::handle_line(req).await;

    let statements = resp["result"].as_array().unwrap();
    assert!(statements[0].as_str().unwrap().contains("ALTER TABLE"));
    assert!(statements[0].as_str().unwrap().contains("email"));
}

#[tokio::test]
async fn full_drop_index_flow() {
    let req = r#"{"jsonrpc":"2.0","method":"drop_index","id":16,"params":{"params":{},"table":"users","index_name":"email-index"}}"#;
    let resp = rpc::handle_line(req).await;

    let statements = resp["result"].as_array().unwrap();
    assert!(statements[0].as_str().unwrap().contains("DROP INDEX"));
}

#[tokio::test]
async fn full_get_create_foreign_key_sql_flow() {
    let req =
        r#"{"jsonrpc":"2.0","method":"get_create_foreign_key_sql","id":17,"params":{"params":{}}}"#;
    let resp = rpc::handle_line(req).await;

    let statements = resp["result"].as_array().unwrap();
    assert!(statements[0].as_str().unwrap().contains("does not support"));
}

#[tokio::test]
async fn full_response_has_correct_jsonrpc_field() {
    let req = r#"{"jsonrpc":"2.0","method":"initialize","id":1}"#;
    let resp = rpc::handle_line(req).await;

    let serialized = serde_json::to_string(&resp).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&serialized).unwrap();

    assert_eq!(parsed["jsonrpc"], "2.0");
    assert_eq!(parsed["id"], 1);
}

#[tokio::test]
async fn full_null_id_handling() {
    // Some requests may have null id (notifications)
    let req = r#"{"jsonrpc":"2.0","method":"initialize","id":null}"#;
    let resp = rpc::handle_line(req).await;

    assert_eq!(resp["id"], serde_json::Value::Null);
    assert_eq!(resp["result"]["success"], true);
}

#[tokio::test]
async fn full_partiql_query_mode_detection() {
    use dynamodb_plugin::handlers::models::{Query, QueryMode};

    let q = Query::from("SELECT * FROM users".to_string());
    assert_eq!(q.mode, QueryMode::Partiql);
    assert_eq!(q.body, "SELECT * FROM users");

    let q = Query::from("#!partiql\nSELECT * FROM users".to_string());
    assert_eq!(q.mode, QueryMode::Partiql);
    assert_eq!(q.body, "SELECT * FROM users");

    let q = Query::from("#!scan\nTableName: users".to_string());
    assert_eq!(q.mode, QueryMode::Scan);
    assert_eq!(q.body, "TableName: users");
}

#[tokio::test]
async fn full_execute_query_response_serialization() {
    use dynamodb_plugin::handlers::models::ExecuteQueryResponse;
    use serde_json::json;

    let resp = ExecuteQueryResponse {
        columns: vec!["id".into(), "name".into()],
        rows: vec![vec![json!("1"), json!("Alice")]],
        affected_rows: 1,
        execution_time_ms: 42,
        truncated: false,
        has_more: false,
        pagination: None,
    };

    let serialized = serde_json::to_value(&resp).unwrap();
    assert_eq!(serialized["columns"][0], "id");
    assert_eq!(serialized["rows"][0][0], "1");
    assert_eq!(serialized["affected_rows"], 1);
    assert_eq!(serialized["execution_time_ms"], 42);
}
