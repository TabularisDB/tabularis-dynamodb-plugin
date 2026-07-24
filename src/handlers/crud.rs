//! CRUD handlers: insert, update, delete records.

use serde_json::{json, Value};

use crate::dynamodb::client::Client;
use crate::error::ErrorCode;
use crate::rpc::{error_response, ok_response};

/// Build a DynamoDB client from params.
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

    Client::new(region, access_key_id, secret_access_key, session_token, profile, endpoint)
        .await
        .map_err(|e| {
            json!({
                "code": ErrorCode::InternalError.value(),
                "message": e.to_string()
            })
        })
}

pub async fn insert_record(id: Value, params: &Value) -> Value {
    let table = params
        .get("table")
        .and_then(|t| t.as_str())
        .unwrap_or("");

    if table.is_empty() {
        return error_response(id, ErrorCode::InvalidParams, "table must be a non-empty string");
    }

    let data = params
        .get("data")
        .and_then(|d| d.as_object())
        .map(|obj| {
            // Convert JSON data map to a PartiQL-compatible INSERT statement
            let cols: Vec<String> = obj.keys().cloned().collect();
            let vals: Vec<String> = obj
                .values()
                .map(|v| match v {
                    Value::String(s) => format!("'{}'", s.replace('\'', "''")),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    Value::Null => "NULL".to_string(),
                    _ => format!("'{}'", v.to_string().replace('\'', "''")),
                })
                .collect();

            format!(
                "INSERT INTO \"{}\" VALUE {{ {} }}",
                table,
                cols.iter()
                    .zip(vals.iter())
                    .map(|(c, v)| format!("'{}': {}", c, v))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        });

    let Some(statement) = data else {
        return error_response(id, ErrorCode::InvalidParams, "data must be a non-empty object");
    };

    let client = match build_client(params).await {
        Ok(c) => c,
        Err(err) => {
            return error_response(id, ErrorCode::InternalError, err["message"].as_str().unwrap_or("unknown error"));
        }
    };

    match client.execute_statement(&statement).await {
        Ok(_) => ok_response(id, json!({"affected_rows": 1})),
        Err(err) => error_response(id, ErrorCode::InternalError, &err.message),
    }
}

pub async fn update_record(id: Value, params: &Value) -> Value {
    let table = params
        .get("table")
        .and_then(|t| t.as_str())
        .unwrap_or("");

    if table.is_empty() {
        return error_response(id, ErrorCode::InvalidParams, "table must be a non-empty string");
    }

    let pk_col = params
        .get("pk_col")
        .and_then(|p| p.as_str())
        .unwrap_or("");

    let pk_val = params.get("pk_val");
    let col_name = params
        .get("col_name")
        .and_then(|c| c.as_str())
        .unwrap_or("");

    let new_val = params.get("new_val");

    if pk_col.is_empty() || col_name.is_empty() || pk_val.is_none() || new_val.is_none() {
        return error_response(
            id,
            ErrorCode::InvalidParams,
            "pk_col, pk_val, col_name, and new_val are required",
        );
    }

    // Build PartiQL UPDATE statement
    let pk_val_str = value_to_partiql_literal(pk_val.unwrap());
    let new_val_str = value_to_partiql_literal(new_val.unwrap());

    let statement = format!(
        "UPDATE \"{}\" SET \"{}\" = {} WHERE \"{}\" = {}",
        table, col_name, new_val_str, pk_col, pk_val_str
    );

    let client = match build_client(params).await {
        Ok(c) => c,
        Err(err) => {
            return error_response(id, ErrorCode::InternalError, err["message"].as_str().unwrap_or("unknown error"));
        }
    };

    match client.execute_statement(&statement).await {
        Ok(_) => ok_response(id, json!({"affected_rows": 1})),
        Err(err) => error_response(id, ErrorCode::InternalError, &err.message),
    }
}

pub async fn delete_record(id: Value, params: &Value) -> Value {
    let table = params
        .get("table")
        .and_then(|t| t.as_str())
        .unwrap_or("");

    if table.is_empty() {
        return error_response(id, ErrorCode::InvalidParams, "table must be a non-empty string");
    }

    let pk_col = params
        .get("pk_col")
        .and_then(|p| p.as_str())
        .unwrap_or("");

    let pk_val = params.get("pk_val");

    if pk_col.is_empty() || pk_val.is_none() {
        return error_response(
            id,
            ErrorCode::InvalidParams,
            "pk_col and pk_val are required",
        );
    }

    let pk_val_str = value_to_partiql_literal(pk_val.unwrap());

    let statement = format!(
        "DELETE FROM \"{}\" WHERE \"{}\" = {}",
        table, pk_col, pk_val_str
    );

    let client = match build_client(params).await {
        Ok(c) => c,
        Err(err) => {
            return error_response(id, ErrorCode::InternalError, err["message"].as_str().unwrap_or("unknown error"));
        }
    };

    match client.execute_statement(&statement).await {
        Ok(_) => ok_response(id, json!({"affected_rows": 1})),
        Err(err) => error_response(id, ErrorCode::InternalError, &err.message),
    }
}

/// Convert a JSON value to a PartiQL literal string.
fn value_to_partiql_literal(v: &Value) -> String {
    match v {
        Value::String(s) => format!("'{}'", s.replace('\'', "''")),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "NULL".to_string(),
        Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(value_to_partiql_literal).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Object(obj) => {
            let entries: Vec<String> = obj
                .iter()
                .map(|(k, v)| format!("\"{}\": {}", k, value_to_partiql_literal(v)))
                .collect();
            format!("{{{}}}", entries.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn insert_record_with_empty_table_returns_error() {
        let params = json!({"params": {}, "table": "", "data": {}});
        let result = insert_record(json!(1), &params).await;
        assert!(result.get("error").is_some());
    }

    #[tokio::test]
    async fn update_record_with_missing_params_returns_error() {
        let params = json!({"params": {}, "table": "users"});
        let result = update_record(json!(1), &params).await;
        assert!(result.get("error").is_some());
    }

    #[tokio::test]
    async fn delete_record_with_missing_params_returns_error() {
        let params = json!({"params": {}, "table": "users"});
        let result = delete_record(json!(1), &params).await;
        assert!(result.get("error").is_some());
    }

    #[test]
    fn value_to_partiql_literal_string() {
        assert_eq!(value_to_partiql_literal(&json!("hello")), "'hello'");
    }

    #[test]
    fn value_to_partiql_literal_string_with_quote() {
        assert_eq!(
            value_to_partiql_literal(&json!("it's")),
            "'it''s'"
        );
    }

    #[test]
    fn value_to_partiql_literal_number() {
        assert_eq!(value_to_partiql_literal(&json!(42)), "42");
        assert_eq!(value_to_partiql_literal(&json!(2.71)), "2.71");
    }

    #[test]
    fn value_to_partiql_literal_bool() {
        assert_eq!(value_to_partiql_literal(&json!(true)), "true");
        assert_eq!(value_to_partiql_literal(&json!(false)), "false");
    }

    #[test]
    fn value_to_partiql_literal_null() {
        assert_eq!(value_to_partiql_literal(&Value::Null), "NULL");
    }

    #[test]
    fn value_to_partiql_literal_object() {
        let v = json!({"name": "Alice", "age": 30});
        let result = value_to_partiql_literal(&v);
        assert!(result.contains("\"name\""));
        assert!(result.contains("\"age\""));
        assert!(result.contains("'Alice'"));
        assert!(result.contains("30"));
    }
}
