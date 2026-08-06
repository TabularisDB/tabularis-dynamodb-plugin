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

/// Normalise connection params so that TabularisDB's generic connection form
/// (HOST / PORT / USERNAME / PASSWORD) maps onto the AWS-shaped fields the
/// DynamoDB driver actually consumes.
///
/// TabularisDB renders a standard HOST/PORT/USERNAME/PASSWORD form for any
/// driver that does not ship a custom `ui_extensions` connection UI. Those
/// generic values are semantically valid for DynamoDB Local:
///   - `host` + `port`        → `endpoint`  (http://host:port)
///   - `username` / `password` → `access_key_id` / `secret_access_key`
///
/// Explicit AWS keys always win; the generic fields are only consulted as
/// fallbacks when the corresponding AWS field is absent. This keeps the #29
/// validation meaningful while letting the out-of-the-box GUI form connect.
fn normalized_params(params: &Value) -> Value {
    let mut out = params.clone();
    let obj = match out.as_object_mut() {
        Some(o) => o,
        None => return out,
    };
    let inner_val = obj
        .entry("params")
        .or_insert_with(|| Value::Object(Default::default()));
    let inner = match inner_val.as_object_mut() {
        Some(m) => m,
        None => return out,
    };

    // host + port -> endpoint (only if no endpoint already supplied).
    if !inner.contains_key("endpoint") {
        let host = inner
            .get("host")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string());
        let port = inner.get("port").and_then(|v| match v {
            Value::String(s) => Some(s.trim().to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        });
        if let (Some(host), Some(port)) = (host, port) {
            if !host.is_empty() && !port.is_empty() {
                let host = host
                    .trim_start_matches("http://")
                    .trim_start_matches("https://");
                // AWS endpoints only speak TLS; http://host:443 fails at the
                // transport level. Use https for 443 and AWS hostnames.
                let scheme = if port == "443" || host.ends_with(".amazonaws.com") {
                    "https://"
                } else {
                    "http://"
                };
                inner.insert(
                    "endpoint".to_string(),
                    Value::String(format!("{scheme}{host}:{port}")),
                );
            }
        }
    }

    // username -> access_key_id (fallback).
    if !inner.contains_key("access_key_id") {
        if let Some(u) = inner
            .get("username")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        {
            inner.insert(
                "access_key_id".to_string(),
                Value::String(u.trim().to_string()),
            );
        }
    }
    // password -> secret_access_key (fallback).
    if !inner.contains_key("secret_access_key") {
        if let Some(p) = inner
            .get("password")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        {
            inner.insert(
                "secret_access_key".to_string(),
                Value::String(p.trim().to_string()),
            );
        }
    }

    // The AWS SDK requires a region for request signing even when talking to a
    // local endpoint (e.g. DynamoDB Local). Default it when an endpoint is set
    // but no region was supplied — the generic GUI form has no region field.
    //
    // Precedence: explicit top-level `region` > opaque `extra["region"]`
    // (connection-level extra fields, persisted and forwarded by the host
    // unchanged — the DynamoDB connection UI's region selector lands here)
    // > region parsed from an AWS endpoint hostname
    // (`dynamodb.us-west-2.amazonaws.com` -> `us-west-2`) > plugin-level
    // default-region setting (Settings → Plugins → DynamoDB) > us-east-1.
    // Profile connections are exempt: they take their region from the AWS
    // profile's own config.
    if inner.contains_key("endpoint")
        && !inner.contains_key("region")
        && !inner.contains_key("profile")
    {
        let region = inner
            .get("extra")
            .and_then(|v| v.get("region"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                inner
                    .get("endpoint")
                    .and_then(|v| v.as_str())
                    .and_then(region_from_endpoint)
                    .map(str::to_string)
            })
            .or_else(crate::settings::default_region)
            .unwrap_or_else(|| "us-east-1".to_string());
        inner.insert("region".to_string(), Value::String(region));
    }

    out
}

/// Extract the AWS region from a standard DynamoDB endpoint hostname, e.g.
/// `https://dynamodb.us-west-2.amazonaws.com:443` -> `us-west-2`.
/// Returns None for non-AWS hosts (localhost, IP addresses, custom domains).
fn region_from_endpoint(endpoint: &str) -> Option<&str> {
    let host = endpoint
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split([':', '/'])
        .next()?;
    let rest = host.strip_prefix("dynamodb.")?;
    let region = rest.strip_suffix(".amazonaws.com")?;
    if region.is_empty() {
        None
    } else {
        Some(region)
    }
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
    // Map generic GUI fields (host/port/username/password) onto AWS-shaped
    // keys before reading, so TabularisDB's default connection form works.
    let params = normalized_params(params);

    let region = read_param(&params, "region");
    let access_key_id = read_param(&params, "access_key_id");
    let secret_access_key = read_param(&params, "secret_access_key");
    let session_token = read_param(&params, "session_token");
    let profile = read_param(&params, "profile");
    let endpoint = read_param(&params, "endpoint");

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

    // ── Generic GUI form normalisation ──────────────────────────────────

    #[test]
    fn normalizes_host_port_to_endpoint() {
        let params = json!({"params": {"host": "localhost", "port": "8000"}});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["endpoint"], "http://localhost:8000");
    }

    #[test]
    fn normalizes_numeric_port() {
        let params = json!({"params": {"host": "localhost", "port": 8000}});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["endpoint"], "http://localhost:8000");
    }

    #[test]
    fn normalizes_credentials() {
        let params = json!({"params": {"username": "local", "password": "local"}});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["access_key_id"], "local");
        assert_eq!(n["params"]["secret_access_key"], "local");
    }

    #[test]
    fn explicit_aws_keys_win_over_generic() {
        let params = json!({"params": {
            "endpoint": "https://dynamodb.us-east-1.amazonaws.com",
            "host": "localhost",
            "port": "8000",
            "access_key_id": "AKIA",
            "username": "local",
        }});
        let n = normalized_params(&params);
        assert_eq!(
            n["params"]["endpoint"],
            "https://dynamodb.us-east-1.amazonaws.com"
        );
        assert_eq!(n["params"]["access_key_id"], "AKIA");
    }

    #[test]
    fn defaults_region_when_endpoint_present() {
        let _guard = crate::settings::TEST_LOCK.lock().unwrap();
        let params = json!({"params": {"host": "localhost", "port": "8000"}});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["region"], "us-east-1");
    }

    #[test]
    fn plugin_setting_region_used_for_non_aws_endpoint() {
        let _guard = crate::settings::TEST_LOCK.lock().unwrap();
        crate::settings::apply_initialize(&json!({"settings": {"region": "ap-southeast-2"}}));
        let params = json!({"params": {"endpoint": "http://localhost:8000"}});
        let n = normalized_params(&params);
        crate::settings::apply_initialize(&json!({}));
        assert_eq!(n["params"]["region"], "ap-southeast-2");
    }

    #[test]
    fn aws_host_on_443_uses_https() {
        let params = json!({"params": {
            "host": "dynamodb.us-west-2.amazonaws.com",
            "port": 443,
        }});
        let n = normalized_params(&params);
        assert_eq!(
            n["params"]["endpoint"],
            "https://dynamodb.us-west-2.amazonaws.com:443"
        );
    }

    #[test]
    fn aws_hostname_implies_https_without_443() {
        let params = json!({"params": {
            "host": "dynamodb.eu-west-1.amazonaws.com",
            "port": "8443",
        }});
        let n = normalized_params(&params);
        assert_eq!(
            n["params"]["endpoint"],
            "https://dynamodb.eu-west-1.amazonaws.com:8443"
        );
    }

    #[test]
    fn region_parsed_from_aws_endpoint() {
        // SigV4 signs with this region; it must match the endpoint's region
        // or AWS rejects every request with InvalidSignatureException.
        let params = json!({"params": {
            "host": "dynamodb.us-west-2.amazonaws.com",
            "port": 443,
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["region"], "us-west-2");
    }

    #[test]
    fn region_defaults_for_non_aws_endpoint() {
        let _guard = crate::settings::TEST_LOCK.lock().unwrap();
        let params = json!({"params": {"endpoint": "http://localhost:8000"}});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["region"], "us-east-1");
    }

    #[test]
    fn explicit_region_wins_over_endpoint_parsing() {
        let params = json!({"params": {
            "endpoint": "https://dynamodb.us-west-2.amazonaws.com",
            "region": "eu-west-1",
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["region"], "eu-west-1");
    }

    #[test]
    fn does_not_override_explicit_region() {
        let params =
            json!({"params": {"endpoint": "http://localhost:8000", "region": "eu-west-1"}});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["region"], "eu-west-1");
    }

    // ── Opaque `extra` map (connection-level custom fields) ────────────

    #[test]
    fn extra_region_used_when_no_explicit_region() {
        let _guard = crate::settings::TEST_LOCK.lock().unwrap();
        let params = json!({"params": {
            "endpoint": "http://localhost:8000",
            "extra": {"region": "ap-southeast-2"},
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["region"], "ap-southeast-2");
    }

    #[test]
    fn explicit_region_wins_over_extra() {
        let params = json!({"params": {
            "endpoint": "http://localhost:8000",
            "region": "eu-west-1",
            "extra": {"region": "ap-southeast-2"},
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["region"], "eu-west-1");
    }

    #[test]
    fn extra_region_wins_over_endpoint_parsing() {
        // A user-chosen region beats the one inferred from the hostname.
        let params = json!({"params": {
            "endpoint": "https://dynamodb.us-west-2.amazonaws.com",
            "extra": {"region": "eu-west-1"},
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["region"], "eu-west-1");
    }

    #[test]
    fn blank_extra_region_falls_back_to_endpoint_parsing() {
        let params = json!({"params": {
            "endpoint": "https://dynamodb.us-west-2.amazonaws.com",
            "extra": {"region": "  "},
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["region"], "us-west-2");
    }

    #[test]
    fn non_string_extra_region_ignored() {
        let params = json!({"params": {
            "endpoint": "https://dynamodb.us-west-2.amazonaws.com",
            "extra": {"region": 42},
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["region"], "us-west-2");
    }

    #[test]
    fn extra_region_ignored_for_profile_connections() {
        // Profile connections inherit their region from ~/.aws/config.
        let params = json!({"params": {
            "endpoint": "http://localhost:8000",
            "profile": "default",
            "extra": {"region": "ap-southeast-2"},
        }});
        let n = normalized_params(&params);
        assert!(n["params"].get("region").is_none());
    }

    #[tokio::test]
    async fn generic_gui_form_is_accepted() {
        // Exactly what TabularisDB's default connection form sends.
        let params = json!({"params": {
            "host": "localhost",
            "port": "8000",
            "username": "local",
            "password": "local",
        }});
        match build_client(&params).await {
            Ok(_) => {}
            Err(e) => assert!(
                !e.message.contains("connection params required"),
                "generic form should satisfy validation, got: {}",
                e.message
            ),
        }
    }
}
