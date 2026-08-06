//! Shared request/response shapes.
//!
//! These mirror the `ConnectionParams` struct the host sends. Keep fields
//! optional — different database types leave different fields blank.

use std::collections::HashMap;

use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ConnectionParams {
    pub driver: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub database: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub region: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
    pub profile: Option<String>,
    pub endpoint: Option<String>,
    /// Opaque driver-specific connection fields, persisted and forwarded by
    /// the host unchanged — it never inspects what a plugin stores here.
    /// The DynamoDB region selector writes `region` into this map (see
    /// `handlers::connection::normalized_params`). Non-string values are
    /// dropped: the map contract is `String -> String`.
    pub extra: HashMap<String, String>,
}

impl ConnectionParams {
    pub fn from_value(value: &Value) -> Self {
        let obj = value.as_object();
        let get_str = |k: &str| {
            obj.and_then(|o| o.get(k))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        let port = obj
            .and_then(|o| o.get("port"))
            .and_then(Value::as_u64)
            .and_then(|p| u16::try_from(p).ok());
        let extra = obj
            .and_then(|o| o.get("extra"))
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        Self {
            driver: get_str("driver"),
            host: get_str("host"),
            port,
            database: get_str("database"),
            username: get_str("username"),
            password: get_str("password"),
            region: get_str("region"),
            access_key_id: get_str("access_key_id"),
            secret_access_key: get_str("secret_access_key"),
            session_token: get_str("session_token"),
            profile: get_str("profile"),
            endpoint: get_str("endpoint"),
            extra,
        }
    }
}

/// Extract the nested `params` object every RPC method receives.
/// Tabularis wraps connection params in `params.params`.
pub fn inner_params(value: &Value) -> &Value {
    value.get("params").unwrap_or(&Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn connection_params_from_value_parses_all_fields() {
        let value = json!({
            "driver": "dynamodb",
            "host": "localhost",
            "port": 8000,
            "database": "default",
            "username": "user",
            "password": "pass",
            "region": "us-east-1",
            "access_key_id": "AKID",
            "secret_access_key": "SAK",
            "session_token": "token",
            "profile": "default",
            "endpoint": "http://localhost:8000",
            "extra": {"region": "us-east-1", "note": "prod", "flag": true}
        });

        let params = ConnectionParams::from_value(&value);

        assert_eq!(params.driver.as_deref(), Some("dynamodb"));
        assert_eq!(params.host.as_deref(), Some("localhost"));
        assert_eq!(params.port, Some(8000));
        assert_eq!(params.database.as_deref(), Some("default"));
        assert_eq!(params.username.as_deref(), Some("user"));
        assert_eq!(params.password.as_deref(), Some("pass"));
        assert_eq!(params.region.as_deref(), Some("us-east-1"));
        assert_eq!(params.access_key_id.as_deref(), Some("AKID"));
        assert_eq!(params.secret_access_key.as_deref(), Some("SAK"));
        assert_eq!(params.session_token.as_deref(), Some("token"));
        assert_eq!(params.profile.as_deref(), Some("default"));
        assert_eq!(params.endpoint.as_deref(), Some("http://localhost:8000"));
        assert_eq!(
            params.extra.get("region").map(String::as_str),
            Some("us-east-1")
        );
        assert_eq!(params.extra.get("note").map(String::as_str), Some("prod"));
        // Non-string entries are dropped — the contract is String -> String.
        assert!(!params.extra.contains_key("flag"));
    }

    #[test]
    fn connection_params_from_value_handles_missing_fields() {
        let value = json!({});
        let params = ConnectionParams::from_value(&value);

        assert!(params.driver.is_none());
        assert!(params.host.is_none());
        assert!(params.port.is_none());
        assert!(params.database.is_none());
        assert!(params.profile.is_none());
        assert!(params.region.is_none());
        assert!(params.access_key_id.is_none());
        assert!(params.extra.is_empty());
    }

    #[test]
    fn connection_params_from_value_ignores_non_object_extra() {
        let value = json!({"extra": "not-an-object"});
        let params = ConnectionParams::from_value(&value);
        assert!(params.extra.is_empty());
    }

    #[test]
    fn inner_params_extracts_nested_params() {
        let value = json!({
            "params": {
                "database": "test"
            },
            "query": "SELECT * FROM users"
        });
        let inner = inner_params(&value);
        assert_eq!(inner["database"], json!("test"));
    }

    #[test]
    fn inner_params_returns_null_when_missing() {
        let value = json!({"query": "test"});
        assert_eq!(inner_params(&value), &Value::Null);
    }
}
