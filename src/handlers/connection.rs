//! Shared DynamoDB client construction and connection-config validation.
//!
//! Historically each handler module (query, metadata, crud) carried its own
//! copy of `build_client`, and none of them validated the connection
//! configuration — `test_connection` with `{"params": {}}` would silently
//! succeed by falling through to the AWS SDK default credential chain. This
//! module centralises client construction and enforces that callers supply a
//! meaningful connection target (issue #29).

use serde_json::Value;

use crate::dynamodb::client::Client;
use crate::error::PluginError;

/// Read an optional string field from the nested `params` object.
fn read_param<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params
        .get("params")
        .and_then(|p| p.get(key))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
}

/// Build a DynamoDB client from a JSON-RPC params object, validating the
/// connection configuration first.
///
/// Validation rule (#29): at least one of
///   - an explicit `endpoint` (e.g. DynamoDB Local), or
///   - a `region` together with credentials (`access_key_id` +
///     `secret_access_key`) or a `profile`
///
/// must be present. Otherwise we reject the request rather than silently
/// resolving credentials from the ambient environment, which would make the
/// connection "test" meaningless.
pub async fn build_client(params: &Value) -> Result<Client, PluginError> {
    let region = read_param(params, "region");
    let access_key_id = read_param(params, "access_key_id");
    let secret_access_key = read_param(params, "secret_access_key");
    let session_token = read_param(params, "session_token");
    let profile = read_param(params, "profile");
    let endpoint = read_param(params, "endpoint");

    let has_endpoint = endpoint.is_some();
    let has_explicit_creds =
        region.is_some() && access_key_id.is_some() && secret_access_key.is_some();
    let has_profile = profile.is_some();

    if !(has_endpoint || has_explicit_creds || has_profile) {
        return Err(PluginError::invalid_params(
            "connection params required: provide at least an endpoint, or \
             region + access_key_id + secret_access_key, or a profile",
        ));
    }

    Client::new(
        region,
        access_key_id,
        secret_access_key,
        session_token,
        profile,
        endpoint,
    )
    .await
    .map_err(|e| PluginError::internal(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn read_param_ignores_blank_and_missing() {
        let params = json!({"params": {"region": "  ", "endpoint": "http://localhost:8000"}});
        assert_eq!(read_param(&params, "region"), None);
        assert_eq!(
            read_param(&params, "endpoint"),
            Some("http://localhost:8000")
        );
        assert_eq!(read_param(&params, "profile"), None);
    }

    #[tokio::test]
    async fn empty_params_rejected() {
        let params = json!({"params": {}});
        let err = build_client(&params).await.unwrap_err();
        assert!(err.message.contains("connection params required"));
    }

    #[tokio::test]
    async fn missing_params_object_rejected() {
        let params = json!({});
        let err = build_client(&params).await.unwrap_err();
        assert!(err.message.contains("connection params required"));
    }

    #[tokio::test]
    async fn endpoint_only_is_accepted() {
        // Endpoint present (DynamoDB Local) — client construction should not be
        // rejected by validation. It may still fail to build for other reasons,
        // but NOT with the "connection params required" message.
        let params =
            json!({"params": {"endpoint": "http://localhost:8000", "region": "us-east-1"}});
        match build_client(&params).await {
            Ok(_) => {}
            Err(e) => assert!(!e.message.contains("connection params required")),
        }
    }

    #[tokio::test]
    async fn region_without_creds_rejected() {
        let params = json!({"params": {"region": "us-east-1"}});
        let err = build_client(&params).await.unwrap_err();
        assert!(err.message.contains("connection params required"));
    }
}
