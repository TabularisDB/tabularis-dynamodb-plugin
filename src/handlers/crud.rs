//! CRUD handlers: insert, update, delete records.
//!
//! All writes go through the native DynamoDB item APIs (PutItem, UpdateItem,
//! DeleteItem) rather than PartiQL. This avoids the 8KB `ExecuteStatement`
//! limit, correctly preserves typed values (numbers stay numbers, nested
//! maps/lists round-trip), and lets us validate key columns up front so
//! callers get a clear error instead of an opaque service failure.

use std::collections::HashMap;

use aws_sdk_dynamodb::types::AttributeValue;
use serde_json::{json, Value};

use crate::dynamodb::models::json_to_attribute_value;
use crate::error::{ErrorCode, PluginError};
use crate::handlers::connection;
use crate::rpc::{error_response, ok_response};

/// Build a DynamoDB client from params, validating connection config (#29).
async fn build_client(params: &Value) -> Result<crate::dynamodb::client::Client, PluginError> {
    connection::build_client(params).await
}

/// Insert a record via the native PutItem API.
pub async fn insert_record(id: Value, params: &Value) -> Value {
    let table = params.get("table").and_then(|t| t.as_str()).unwrap_or("");

    if table.is_empty() {
        return error_response(
            id,
            ErrorCode::InvalidParams,
            "table must be a non-empty string",
        );
    }

    let data = match params.get("data").and_then(|d| d.as_object()) {
        Some(obj) if !obj.is_empty() => obj,
        _ => {
            return error_response(
                id,
                ErrorCode::InvalidParams,
                "data must be a non-empty object",
            );
        }
    };

    // Build the item map with correct DynamoDB types (numbers, nested maps/lists).
    let mut item: HashMap<String, AttributeValue> = HashMap::new();
    for (k, v) in data.iter() {
        item.insert(k.clone(), json_to_attribute_value(v));
    }

    // Optional condition expression for conditional / idempotent inserts (#11).
    // An explicit `condition_expression` takes precedence. When `idempotent` is
    // true and no condition is supplied, we default to `attribute_not_exists(pk)`
    // so an existing item is never silently overwritten.
    let condition_expression = params
        .get("condition_expression")
        .and_then(|c| c.as_str())
        .map(|s| s.to_string());
    let idempotent = params
        .get("idempotent")
        .and_then(|b| b.as_bool())
        .unwrap_or(false);

    let client = match build_client(params).await {
        Ok(c) => c,
        Err(err) => return error_response(id, err.code, &err.message),
    };

    // Resolve the default idempotent condition (needs the real partition key).
    let pk_lookup = if condition_expression.is_none() && idempotent {
        match client.table_partition_key(table).await {
            Ok(pk) => Some(Ok(pk)),
            Err(err) => Some(Err(err.message)),
        }
    } else {
        None
    };
    match resolve_insert_condition(condition_expression.as_deref(), idempotent, pk_lookup) {
        Ok(condition) => match client.put_item(table, item, condition).await {
            Ok(()) => ok_response(id, json!({"affected_rows": 1})),
            Err(err) => error_response(id, ErrorCode::InternalError, &err.message),
        },
        Err(msg) => error_response(id, ErrorCode::InternalError, &msg),
    }
}

/// Resolve the condition expression for an insert (#11).
///
/// Precedence: an explicit `condition_expression` always wins. Otherwise, when
/// `idempotent` is set, build `attribute_not_exists(pk)` from the table's
/// partition key. Without either, no condition is applied.
fn resolve_insert_condition(
    explicit: Option<&str>,
    idempotent: bool,
    pk_lookup: Option<Result<String, String>>,
) -> Result<Option<String>, String> {
    match (explicit, idempotent, pk_lookup) {
        (Some(c), _, _) => Ok(Some(c.to_string())),
        (None, true, Some(Ok(pk))) => Ok(Some(format!("attribute_not_exists({pk})"))),
        (None, true, Some(Err(err))) => Err(err),
        _ => Ok(None),
    }
}

pub async fn update_record(id: Value, params: &Value) -> Value {
    let table = params.get("table").and_then(|t| t.as_str()).unwrap_or("");

    if table.is_empty() {
        return error_response(
            id,
            ErrorCode::InvalidParams,
            "table must be a non-empty string",
        );
    }

    // Support both composite keys (key: {col: val, ...}) and simple keys (pk_col + pk_val)
    let key_conditions = extract_key_conditions(params);

    let col_name = params
        .get("col_name")
        .and_then(|c| c.as_str())
        .unwrap_or("");

    let new_val = params.get("new_val");

    if key_conditions.is_empty() || col_name.is_empty() || new_val.is_none() {
        return error_response(
            id,
            ErrorCode::InvalidParams,
            "key (or pk_col + pk_val), col_name, and new_val are required",
        );
    }

    let client = match build_client(params).await {
        Ok(c) => c,
        Err(err) => return error_response(id, err.code, &err.message),
    };

    // Validate that the supplied key columns are actual table key attributes.
    match client.table_key_columns(table).await {
        Ok(valid_keys) => {
            if let Some(msg) = validate_key_columns(&key_conditions, &valid_keys) {
                return error_response(id, ErrorCode::InvalidParams, &msg);
            }
            if let Some(msg) = ensure_full_key(&key_conditions, &valid_keys) {
                return error_response(id, ErrorCode::InvalidParams, &msg);
            }
        }
        Err(err) => return error_response(id, ErrorCode::InternalError, &err.message),
    }

    let key = key_conditions
        .iter()
        .map(|(k, v)| (k.clone(), json_to_attribute_value(v)))
        .collect();

    let new_val_attr = json_to_attribute_value(new_val.unwrap());

    match client.update_item(table, key, col_name, new_val_attr).await {
        Ok(()) => ok_response(id, json!({"affected_rows": 1})),
        Err(err) => error_response(id, ErrorCode::InternalError, &err.message),
    }
}

pub async fn delete_record(id: Value, params: &Value) -> Value {
    let table = params.get("table").and_then(|t| t.as_str()).unwrap_or("");

    if table.is_empty() {
        return error_response(
            id,
            ErrorCode::InvalidParams,
            "table must be a non-empty string",
        );
    }

    // Support both composite keys (key: {col: val, ...}) and simple keys (pk_col + pk_val)
    let key_conditions = extract_key_conditions(params);

    if key_conditions.is_empty() {
        return error_response(
            id,
            ErrorCode::InvalidParams,
            "key (or pk_col + pk_val) is required",
        );
    }

    let client = match build_client(params).await {
        Ok(c) => c,
        Err(err) => return error_response(id, err.code, &err.message),
    };

    // Validate that the supplied key columns are actual table key attributes.
    match client.table_key_columns(table).await {
        Ok(valid_keys) => {
            if let Some(msg) = validate_key_columns(&key_conditions, &valid_keys) {
                return error_response(id, ErrorCode::InvalidParams, &msg);
            }
            if let Some(msg) = ensure_full_key(&key_conditions, &valid_keys) {
                return error_response(id, ErrorCode::InvalidParams, &msg);
            }
        }
        Err(err) => return error_response(id, ErrorCode::InternalError, &err.message),
    }

    let key = key_conditions
        .iter()
        .map(|(k, v)| (k.clone(), json_to_attribute_value(v)))
        .collect();

    match client.delete_item(table, key).await {
        Ok(()) => ok_response(id, json!({"affected_rows": 1})),
        Err(err) => error_response(id, ErrorCode::InternalError, &err.message),
    }
}

/// Extract key conditions from params. Supports:
/// - `key`: object mapping column names to values (for composite keys)
/// - `pk_col` + `pk_val`: simple single-column key (backward compat)
fn extract_key_conditions(params: &Value) -> Vec<(String, Value)> {
    // Prefer the `key` object for composite keys
    if let Some(key_obj) = params.get("key").and_then(|k| k.as_object()) {
        if !key_obj.is_empty() {
            return key_obj
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
        }
    }

    // Fall back to pk_col + pk_val for simple keys
    let pk_col = params.get("pk_col").and_then(|p| p.as_str()).unwrap_or("");
    let pk_val = params.get("pk_val");

    if !pk_col.is_empty() {
        if let Some(val) = pk_val {
            return vec![(pk_col.to_string(), val.clone())];
        }
    }

    Vec::new()
}

/// Return an error message if any supplied key column is not a real key
/// attribute on the table; otherwise `None`.
fn validate_key_columns(
    key_conditions: &[(String, Value)],
    valid_keys: &[String],
) -> Option<String> {
    for (col, _) in key_conditions {
        if !valid_keys.iter().any(|k| k == col) {
            return Some(format!(
                "column '{col}' is not a key attribute of this table (valid key columns: {})",
                valid_keys.join(", ")
            ));
        }
    }
    None
}

/// Return an error message if the supplied key does not cover all of the
/// table's key attributes (HASH + RANGE); otherwise `None`. DynamoDB's
/// item APIs require the full key to address an item.
fn ensure_full_key(key_conditions: &[(String, Value)], valid_keys: &[String]) -> Option<String> {
    let missing: Vec<String> = valid_keys
        .iter()
        .filter(|k| !key_conditions.iter().any(|(col, _)| col == *k))
        .cloned()
        .collect();
    if missing.is_empty() {
        None
    } else {
        Some(format!(
            "key is missing required key attribute(s): {} (must provide all key columns)",
            missing.join(", ")
        ))
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
    async fn insert_record_with_missing_data_returns_error() {
        let params = json!({"params": {}, "table": "users"});
        let result = insert_record(json!(1), &params).await;
        assert!(result.get("error").is_some());
        assert_eq!(
            result["error"]["message"],
            "data must be a non-empty object"
        );
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
    fn validate_key_columns_accepts_valid() {
        let valid = vec!["id".to_string(), "created_at".to_string()];
        let conds = vec![
            ("id".to_string(), json!("a")),
            ("created_at".to_string(), json!("b")),
        ];
        assert!(validate_key_columns(&conds, &valid).is_none());
    }

    #[test]
    fn validate_key_columns_rejects_non_key() {
        let valid = vec!["id".to_string()];
        let conds = vec![("email".to_string(), json!("x@y.z"))];
        let msg = validate_key_columns(&conds, &valid).unwrap();
        assert!(msg.contains("email"));
        assert!(msg.contains("not a key attribute"));
    }

    #[test]
    fn ensure_full_key_detects_missing_sort_key() {
        let valid = vec!["id".to_string(), "created_at".to_string()];
        let conds = vec![("id".to_string(), json!("a"))];
        let msg = ensure_full_key(&conds, &valid).unwrap();
        assert!(msg.contains("created_at"));
    }

    #[test]
    fn ensure_full_key_ok_when_complete() {
        let valid = vec!["id".to_string(), "created_at".to_string()];
        let conds = vec![
            ("id".to_string(), json!("a")),
            ("created_at".to_string(), json!("b")),
        ];
        assert!(ensure_full_key(&conds, &valid).is_none());
    }

    #[test]
    fn extract_key_conditions_prefers_key_object() {
        let params = json!({
            "key": {"id": "a", "created_at": "b"},
            "pk_col": "id",
            "pk_val": "ignored"
        });
        let conds = extract_key_conditions(&params);
        assert_eq!(conds.len(), 2);
    }

    #[test]
    fn extract_key_conditions_falls_back_to_pk() {
        let params = json!({"pk_col": "id", "pk_val": "a"});
        let conds = extract_key_conditions(&params);
        assert_eq!(conds.len(), 1);
        assert_eq!(conds[0].0, "id");
    }

    #[test]
    fn resolve_condition_none_when_not_idempotent() {
        assert_eq!(resolve_insert_condition(None, false, None).unwrap(), None);
    }

    #[test]
    fn resolve_condition_explicit_wins() {
        let pk = Some(Ok("id".to_string()));
        assert_eq!(
            resolve_insert_condition(Some("attribute_not_exists(x)"), true, pk).unwrap(),
            Some("attribute_not_exists(x)".to_string()),
            "explicit condition should take precedence over idempotent default"
        );
    }

    #[test]
    fn resolve_condition_idempotent_builds_attribute_not_exists() {
        let pk = Some(Ok("id".to_string()));
        assert_eq!(
            resolve_insert_condition(None, true, pk).unwrap(),
            Some("attribute_not_exists(id)".to_string())
        );
    }

    #[test]
    fn resolve_condition_idempotent_propagates_pk_error() {
        let pk = Some(Err("describe failed".to_string()));
        assert!(resolve_insert_condition(None, true, pk).is_err());
    }
}
