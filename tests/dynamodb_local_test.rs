//! Integration tests against DynamoDB Local (#13).
//!
//! These exercise the full plugin lifecycle through `rpc::handle_line` with a
//! REAL DynamoDB client, unlike `integration_test.rs` which is mock-only.
//!
//! Gated behind the `DYNAMODB_ENDPOINT` env var so CI skips them when Docker /
//! DynamoDB Local is unavailable. To run locally:
//!
//! ```bash
//! just run-dynamodb        # start DynamoDB Local (Docker)
//! just seed-dynamodb       # create + seed `users` and `orders`
//! DYNAMODB_ENDPOINT=http://localhost:8000 cargo test --test dynamodb_local_test
//! ```
//!
//! The tests operate on the seeded `users` table (HASH `id: S`) using unique
//! `itest-*` ids so they don't clobber seed data, and clean up after themselves.

use dynamodb_plugin::rpc;
use serde_json::{json, Value};

/// Returns the DynamoDB endpoint if the env gate is set, else `None`.
fn endpoint() -> Option<String> {
    std::env::var("DYNAMODB_ENDPOINT")
        .ok()
        .filter(|e| !e.is_empty())
}

/// Build the connection params block for a given endpoint.
fn conn_params(ep: &str) -> Value {
    json!({
        "params": {
            "region": "us-east-1",
            "access_key_id": "local",
            "secret_access_key": "local",
            "endpoint": ep
        }
    })
}

/// Fire a JSON-RPC request and return the parsed response.
async fn call(method: &str, params: Value) -> Value {
    let req = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    rpc::handle_line(&req.to_string()).await
}

fn assert_ok(resp: &Value, what: &str) {
    assert!(
        resp.get("error").is_none(),
        "{what} unexpectedly errored: {}",
        serde_json::to_string(resp).unwrap_or_default()
    );
}

/// Rows come back positionally: `result.columns` names the fields and each
/// entry in `result.rows` is an array of values in that order. Look up a
/// column's value from the first row by name.
fn first_row_field(resp: &Value, column: &str) -> Option<String> {
    let result = &resp["result"];
    let columns = result["columns"].as_array()?;
    let idx = columns.iter().position(|c| c.as_str() == Some(column))?;
    let rows = result["rows"].as_array()?;
    rows.first()?.as_array()?.get(idx).map(|v| match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

fn first_row_count(resp: &Value) -> usize {
    resp["result"]["rows"]
        .as_array()
        .map(|r| r.len())
        .unwrap_or(0)
}

fn unique_id(prefix: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{prefix}-{nanos}")
}

// ── Connection ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_connection_against_local() {
    let Some(ep) = endpoint() else { return };
    let mut params = conn_params(&ep);
    params["params"]["table"] = json!("users");

    let resp = call("test_connection", params).await;
    assert_ok(&resp, "test_connection");
}

// ── Metadata ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn get_tables_lists_seeded_tables() {
    let Some(ep) = endpoint() else { return };
    let resp = call("get_tables", conn_params(&ep)).await;
    assert_ok(&resp, "get_tables");

    let tables: Vec<String> = resp["result"]
        .as_array()
        .expect("get_tables result is an array")
        .iter()
        .filter_map(|t| t.get("name").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
        .collect();

    assert!(
        tables.iter().any(|t| t == "users"),
        "users table present: {tables:?}"
    );
    assert!(
        tables.iter().any(|t| t == "orders"),
        "orders table present: {tables:?}"
    );
}

#[tokio::test]
async fn get_columns_returns_user_attributes() {
    let Some(ep) = endpoint() else { return };
    let mut params = conn_params(&ep);
    params["table"] = json!("users");

    let resp = call("get_columns", params).await;
    assert_ok(&resp, "get_columns");

    let columns = resp["result"]
        .as_array()
        .expect("get_columns result is an array");
    assert!(!columns.is_empty(), "users should have columns");
}

// ── execute_query: all 4 modes ──────────────────────────────────────────────

#[tokio::test]
async fn execute_query_partiql_select() {
    let Some(ep) = endpoint() else { return };
    let mut params = conn_params(&ep);
    params["query"] = json!("SELECT id FROM users LIMIT 5");

    let resp = call("execute_query", params).await;
    assert_ok(&resp, "partiql select");
    assert!(resp["result"]["rows"].is_array(), "rows present");
}

#[tokio::test]
async fn execute_query_scan_mode() {
    let Some(ep) = endpoint() else { return };
    let mut params = conn_params(&ep);
    // Native modes carry their config in the query body as YAML.
    params["query"] = json!("#!scan\nTableName: users\nLimit: 5");

    let resp = call("execute_query", params).await;
    assert_ok(&resp, "#!scan");
    assert!(resp["result"]["rows"].is_array());
}

#[tokio::test]
async fn execute_query_query_mode() {
    let Some(ep) = endpoint() else { return };
    let mut params = conn_params(&ep);
    // Query on the users HASH key (id) for a specific partition value.
    params["query"] = json!("#!query\nTableName: users\nPartitionKey: id\nPartitionValue: user1");

    let resp = call("execute_query", params).await;
    assert_ok(&resp, "#!query");
    assert!(resp["result"]["rows"].is_array());
}

#[tokio::test]
async fn execute_query_get_mode() {
    let Some(ep) = endpoint() else { return };
    let mut params = conn_params(&ep);
    params["query"] = json!("#!get\nTableName: users\nKey:\n  id: user1");

    let resp = call("execute_query", params).await;
    assert_ok(&resp, "#!get");
    let rows = resp["result"]["rows"].as_array().expect("rows array");
    assert_eq!(
        rows.len(),
        1,
        "get for user1 returns exactly one row: {rows:?}"
    );
}

// ── CRUD lifecycle ──────────────────────────────────────────────────────────

#[tokio::test]
async fn crud_lifecycle_insert_update_delete() {
    let Some(ep) = endpoint() else { return };
    let uid = unique_id("itest");

    // INSERT via native API.
    let mut ins = conn_params(&ep);
    ins["table"] = json!("users");
    ins["data"] = json!({ "id": uid, "name": "Integration Test", "age": 42 });
    let resp = call("insert_record", ins).await;
    assert_ok(&resp, "insert_record");

    // Read it back via PartiQL.
    let mut sel = conn_params(&ep);
    sel["query"] = json!(format!("SELECT id, name FROM users WHERE id = '{uid}'"));
    let resp = call("execute_query", sel).await;
    assert_ok(&resp, "select after insert");
    assert_eq!(first_row_count(&resp), 1, "inserted row readable");

    // UPDATE via native API.
    let mut upd = conn_params(&ep);
    upd["table"] = json!("users");
    upd["key"] = json!({ "id": uid });
    upd["col_name"] = json!("name");
    upd["new_val"] = json!("Updated Name");
    let resp = call("update_record", upd).await;
    assert_ok(&resp, "update_record");

    // Confirm update applied.
    let mut sel2 = conn_params(&ep);
    sel2["query"] = json!(format!("SELECT name FROM users WHERE id = '{uid}'"));
    let resp = call("execute_query", sel2).await;
    assert_ok(&resp, "select after update");
    let name = first_row_field(&resp, "name").unwrap_or_default();
    assert_eq!(name, "Updated Name", "update persisted");

    // DELETE via native API.
    let mut del = conn_params(&ep);
    del["table"] = json!("users");
    del["key"] = json!({ "id": uid });
    let resp = call("delete_record", del).await;
    assert_ok(&resp, "delete_record");

    // Confirm gone.
    let mut sel3 = conn_params(&ep);
    sel3["query"] = json!(format!("SELECT id FROM users WHERE id = '{uid}'"));
    let resp = call("execute_query", sel3).await;
    assert_ok(&resp, "select after delete");
    assert_eq!(first_row_count(&resp), 0, "deleted row no longer present");
}

// ── PartiQL write lifecycle (#17 transactions) ──────────────────────────────

#[tokio::test]
async fn partiql_transaction_multi_insert_atomic() {
    let Some(ep) = endpoint() else { return };
    let a = unique_id("txa");
    let b = unique_id("txb");

    // Two-statement transaction commits both.
    let mut txn = conn_params(&ep);
    txn["query"] = json!(format!(
        "INSERT INTO users VALUE {{'id': '{a}', 'name': 'A'}}; \
         INSERT INTO users VALUE {{'id': '{b}', 'name': 'B'}}"
    ));
    let resp = call("execute_query", txn).await;
    assert_ok(&resp, "transaction");
    assert_eq!(
        resp["result"]["affected_rows"].as_i64(),
        Some(2),
        "both stmts applied"
    );

    // Both rows present.
    for uid in [&a, &b] {
        let mut sel = conn_params(&ep);
        sel["query"] = json!(format!("SELECT id FROM users WHERE id = '{uid}'"));
        let resp = call("execute_query", sel).await;
        assert_eq!(first_row_count(&resp), 1, "tx row {uid} committed");
    }

    // Atomic rollback: one valid + one invalid table rolls everything back.
    let c = unique_id("txc");
    let mut bad = conn_params(&ep);
    bad["query"] = json!(format!(
        "INSERT INTO users VALUE {{'id': '{c}', 'name': 'C'}}; \
         INSERT INTO no_such_table_xyz VALUE {{'id': 'x'}}"
    ));
    let resp = call("execute_query", bad).await;
    assert!(resp.get("error").is_some(), "mixed transaction rejected");

    let mut sel = conn_params(&ep);
    sel["query"] = json!(format!("SELECT id FROM users WHERE id = '{c}'"));
    let resp = call("execute_query", sel).await;
    assert_eq!(first_row_count(&resp), 0, "txc rolled back, not committed");

    // Cleanup committed rows.
    for uid in [a, b] {
        let mut del = conn_params(&ep);
        del["table"] = json!("users");
        del["key"] = json!({ "id": uid });
        let _ = call("delete_record", del).await;
    }
}

// ── DDL: CREATE / DROP TABLE through the native control plane ───────────────

#[tokio::test]
async fn ddl_create_and_drop_table() {
    let Some(ep) = endpoint() else { return };
    // Unique table name so parallel runs never collide.
    let table = format!("itest_{}", unique_id("ddl"));

    // The exact shape the TabularisDB GUI sends for "Create New Table":
    // quoted identifiers, STRING columns, a single-column PRIMARY KEY.
    let mut create = conn_params(&ep);
    create["query"] = json!(format!(
        "CREATE TABLE \"{table}\" (\"id\" STRING, \"name\" STRING, PRIMARY KEY (\"id\"));"
    ));
    let resp = call("execute_query", create.clone()).await;
    assert_ok(&resp, "CREATE TABLE via GUI-shaped DDL");
    assert_eq!(resp["result"]["affected_rows"].as_i64(), Some(1));

    // Table now appears in get_tables.
    let resp = call("get_tables", conn_params(&ep)).await;
    assert_ok(&resp, "get_tables after create");
    let names: Vec<String> = resp["result"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| t["name"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    assert!(names.contains(&table), "created table listed: {names:?}");

    // Re-create must fail cleanly with an "already exists" error, not a
    // cryptic service error or a panic.
    let resp = call("execute_query", create).await;
    let msg = resp["error"]["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains("already exists"),
        "expected already-exists error, got: {msg}"
    );

    // Column schema: id is the partition key.
    let mut cols = conn_params(&ep);
    cols["table"] = json!(&table);
    let resp = call("get_columns", cols).await;
    assert_ok(&resp, "get_columns on created table");
    let pk_is_id = resp["result"]
        .as_array()
        .map(|a| {
            a.iter()
                .any(|c| c["name"].as_str() == Some("id") && c["is_pk"].as_bool() == Some(true))
        })
        .unwrap_or(false);
    assert!(pk_is_id, "id flagged as partition key");

    // DROP without allow_destructive must be refused (safety guard).
    let mut drop_guarded = conn_params(&ep);
    drop_guarded["query"] = json!(format!("DROP TABLE \"{table}\""));
    let resp = call("execute_query", drop_guarded).await;
    assert!(
        resp["result"]["warning"].as_str().is_some(),
        "DROP TABLE refused without allow_destructive"
    );

    // DROP with allow_destructive succeeds.
    let mut drop = conn_params(&ep);
    drop["query"] = json!(format!("DROP TABLE \"{table}\""));
    drop["allow_destructive"] = json!(true);
    let resp = call("execute_query", drop).await;
    assert_ok(&resp, "DROP TABLE with allow_destructive");
    assert_eq!(resp["result"]["affected_rows"].as_i64(), Some(1));

    // Table gone.
    let resp = call("get_tables", conn_params(&ep)).await;
    let names: Vec<String> = resp["result"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| t["name"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    assert!(!names.contains(&table), "dropped table removed: {names:?}");
}
