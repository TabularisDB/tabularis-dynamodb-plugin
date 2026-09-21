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

/// True when `key` is present and holds a non-blank string.
///
/// Absent, `null` and `""` are all "not supplied" as far as the SDK is
/// concerned (they normalise away in [`read_param`]), so the normalisation
/// below treats them identically instead of letting `null`/`""` shadow a
/// fallback.
fn has_non_blank(inner: &serde_json::Map<String, Value>, key: &str) -> bool {
    inner
        .get(key)
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty())
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

    // Opaque `extra` connection fields (the connection-modal.extra_fields slot
    // in the plugin's `ui/` bundle) -> AWS-shaped params. Only the fields the
    // generic form has no home for live here: `region` is consumed further
    // down, profile and session token are promoted here. Explicit top-level
    // values always win, and a blank value means "cleared" (the slot's
    // `setExtraField(key, "")` semantics), so it must not shadow a real one.
    for key in ["profile", "session_token"] {
        // An explicit value wins — but only if it isn't blank, since blanks
        // are dropped by `read_param` further down and would otherwise leave
        // the connection with neither the explicit nor the extra value.
        let explicit = inner
            .get(key)
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.trim().is_empty());
        if explicit {
            continue;
        }
        if let Some(v) = inner
            .get("extra")
            .and_then(|e| e.get(key))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            inner.insert(key.to_string(), Value::String(v.to_string()));
        }
    }

    // The AWS SDK requires a region for request signing even when talking to a
    // local endpoint (e.g. DynamoDB Local). Default it whenever the connection
    // has something to sign with — an endpoint (DynamoDB Local / custom
    // endpoint) or an explicit access-key/secret pair — since the generic GUI
    // form has no region field and a keys-only connection has no endpoint to
    // parse a region out of (#71).
    //
    // Precedence: explicit top-level `region` > opaque `extra["region"]`
    // (connection-level extra fields, persisted and forwarded by the host
    // unchanged — the DynamoDB connection UI's region selector lands here)
    // > region parsed from an AWS endpoint hostname
    // (`dynamodb.us-west-2.amazonaws.com` -> `us-west-2`) > plugin-level
    // default-region setting (Settings → Plugins → DynamoDB) > us-east-1.
    //
    // Profile connections take their region from the AWS profile's own config
    // (`extra["profile"]` included, which is why the promotion above runs
    // first), so the plugin-setting and us-east-1 fallbacks stay out of the
    // way — but connection-level choices (`extra["region"]` and the endpoint
    // hostname) still apply, because the signing region must match the
    // endpoint the request is sent to. A profile whose config disagrees with
    // the endpoint region otherwise fails with
    // `InvalidSignatureException: Credential should be scoped to a valid
    // region`.
    let has_endpoint = has_non_blank(inner, "endpoint");
    let has_creds =
        has_non_blank(inner, "access_key_id") && has_non_blank(inner, "secret_access_key");
    let has_profile = has_non_blank(inner, "profile");

    if (has_endpoint || has_creds) && !has_non_blank(inner, "region") {
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
            .or_else(|| {
                if has_profile {
                    None
                } else {
                    crate::settings::default_region()
                }
            })
            .or_else(|| {
                if has_profile {
                    None
                } else {
                    Some("us-east-1".to_string())
                }
            });
        if let Some(region) = region {
            inner.insert("region".to_string(), Value::String(region));
        }
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

/// Operating system whose home-directory conventions apply.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HomeOs {
    Windows,
    Unix,
}

impl HomeOs {
    fn real() -> Self {
        if std::env::consts::OS == "windows" {
            Self::Windows
        } else {
            Self::Unix
        }
    }
}

/// Resolve the home directory the AWS SDK itself would use, mirroring
/// `aws_runtime::fs_util::home_dir`: `HOME` first on every platform, then the
/// Windows variables (`USERPROFILE`, then `HOMEDRIVE` + `HOMEPATH`).
///
/// A GUI-launched plugin on Windows has no `HOME` — it is not a Windows
/// variable, only shells like git-bash set it — so resolving the profile files
/// from `HOME` alone rejects profiles the SDK resolves without complaint.
fn home_dir_from(
    os: HomeOs,
    home: Option<std::ffi::OsString>,
    userprofile: Option<std::ffi::OsString>,
    homedrive: Option<std::ffi::OsString>,
    homepath: Option<std::ffi::OsString>,
) -> Option<std::path::PathBuf> {
    if let Some(home) = home {
        return Some(std::path::PathBuf::from(home));
    }
    if os == HomeOs::Windows {
        if let Some(userprofile) = userprofile {
            return Some(std::path::PathBuf::from(userprofile));
        }
        if let (Some(mut drive), Some(path)) = (homedrive, homepath) {
            drive.push(path);
            return Some(std::path::PathBuf::from(drive));
        }
    }
    None
}

/// [`home_dir_from`] fed from this process's environment.
fn aws_home_dir() -> Option<std::path::PathBuf> {
    home_dir_from(
        HomeOs::real(),
        std::env::var_os("HOME"),
        std::env::var_os("USERPROFILE"),
        std::env::var_os("HOMEDRIVE"),
        std::env::var_os("HOMEPATH"),
    )
}

/// True when `name` exists as a section in the AWS shared credentials file
/// (`[name]`) or the AWS config file (`[profile name]`, or plain `[name]`
/// for `default`). Honours `AWS_SHARED_CREDENTIALS_FILE` / `AWS_CONFIG_FILE`.
fn profile_exists(name: &str) -> bool {
    let home = aws_home_dir();
    let creds_path = std::env::var_os("AWS_SHARED_CREDENTIALS_FILE")
        .map(std::path::PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".aws/credentials")));
    let config_path = std::env::var_os("AWS_CONFIG_FILE")
        .map(std::path::PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".aws/config")));

    if let Some(contents) = creds_path.and_then(|p| std::fs::read_to_string(p).ok()) {
        if ini_has_section(&contents, &[name.to_string()]) {
            return true;
        }
    }
    if let Some(contents) = config_path.and_then(|p| std::fs::read_to_string(p).ok()) {
        if ini_has_section(&contents, &[format!("profile {name}"), name.to_string()]) {
            return true;
        }
    }
    false
}

/// True when any of `names` appears as an INI section header.
fn ini_has_section(contents: &str, names: &[String]) -> bool {
    contents.lines().map(str::trim).any(|line| {
        line.starts_with('[')
            && line.ends_with(']')
            && names
                .iter()
                .any(|n| line[1..line.len() - 1].trim() == n.trim())
    })
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

    // Fail fast with an actionable message when the named profile does not
    // exist; the SDK otherwise surfaces a generic credential/dispatch error
    // that gives no hint the profile name itself is wrong.
    if let Some(profile_name) = profile {
        if !profile_exists(profile_name) {
            return Err(PluginError::invalid_params(format!(
                "AWS profile '{profile_name}' not found in ~/.aws/credentials or \
                 ~/.aws/config"
            )));
        }
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

    // ── Opaque `extra` connection fields: profile / session token ──────
    // (written by the plugin's `ui/` bundle through the host's
    // `connection-modal.extra_fields` slot)

    #[test]
    fn extra_profile_is_promoted() {
        let params = json!({"params": {
            "endpoint": "http://localhost:8000",
            "extra": {"profile": "staging"},
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["profile"], "staging");
    }

    #[test]
    fn extra_session_token_is_promoted() {
        let params = json!({"params": {
            "endpoint": "http://localhost:8000",
            "extra": {"session_token": "FQoGZXIvYXdzEXAMPLE"},
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["session_token"], "FQoGZXIvYXdzEXAMPLE");
    }

    #[test]
    fn extra_profile_and_session_token_together() {
        let params = json!({"params": {
            "username": "AKIA",
            "password": "secret",
            "extra": {"profile": "staging", "session_token": "tok"},
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["profile"], "staging");
        assert_eq!(n["params"]["session_token"], "tok");
        assert_eq!(n["params"]["access_key_id"], "AKIA");
    }

    #[test]
    fn extra_values_are_trimmed() {
        let params = json!({"params": {
            "endpoint": "http://localhost:8000",
            "extra": {"profile": "  staging  "},
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["profile"], "staging");
    }

    #[test]
    fn explicit_profile_wins_over_extra() {
        let params = json!({"params": {
            "endpoint": "http://localhost:8000",
            "profile": "default",
            "extra": {"profile": "staging"},
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["profile"], "default");
    }

    #[test]
    fn explicit_session_token_wins_over_extra() {
        let params = json!({"params": {
            "endpoint": "http://localhost:8000",
            "session_token": "explicit",
            "extra": {"session_token": "from-ui"},
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["session_token"], "explicit");
    }

    #[test]
    fn blank_explicit_profile_falls_back_to_extra() {
        // A blank top-level value is "not supplied" — read_param drops it, so
        // it must not mask the value the connection UI wrote.
        let params = json!({"params": {
            "endpoint": "http://localhost:8000",
            "profile": "   ",
            "extra": {"profile": "staging"},
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["profile"], "staging");
    }

    #[test]
    fn blank_extra_profile_is_ignored() {
        // setExtraField(key, "") clears a field — it must not become a
        // profile named "".
        let params = json!({"params": {
            "endpoint": "http://localhost:8000",
            "extra": {"profile": "  ", "session_token": ""},
        }});
        let n = normalized_params(&params);
        assert!(n["params"].get("profile").is_none());
        assert!(n["params"].get("session_token").is_none());
    }

    #[test]
    fn non_string_extra_profile_ignored() {
        let params = json!({"params": {
            "endpoint": "http://localhost:8000",
            "extra": {"profile": 42},
        }});
        let n = normalized_params(&params);
        assert!(n["params"].get("profile").is_none());
    }

    #[test]
    fn extra_profile_suppresses_region_default() {
        // A profile connection takes its region from ~/.aws/config, so a
        // profile chosen in the connection modal must keep the plugin-setting
        // and us-east-1 fallbacks out of the way.
        let params = json!({"params": {
            "endpoint": "http://localhost:8000",
            "extra": {"profile": "staging"},
        }});
        let n = normalized_params(&params);
        assert!(n["params"].get("region").is_none());
    }

    #[test]
    fn profile_with_aws_endpoint_still_gets_endpoint_region() {
        // The signing region must match the endpoint the request is sent to.
        // A profile only suppresses the *fallback* chain; a region parsed
        // from an AWS endpoint hostname still applies, otherwise the request
        // fails with "Credential should be scoped to a valid region".
        let params = json!({"params": {
            "endpoint": "https://dynamodb.us-west-2.amazonaws.com:443",
            "extra": {"profile": "staging"},
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["region"], "us-west-2");
    }

    #[test]
    fn profile_with_extra_region_keeps_explicit_region() {
        let params = json!({"params": {
            "endpoint": "http://localhost:8000",
            "extra": {"profile": "staging", "region": "eu-central-1"},
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["region"], "eu-central-1");
    }

    #[test]
    fn ini_has_section_matches_exact_names() {
        let contents =
            "[default]\naws_access_key_id = x\n\n[profile staging]\nregion = us-east-1\n";
        assert!(ini_has_section(contents, &["default".to_string()]));
        assert!(ini_has_section(
            contents,
            &["profile staging".to_string(), "staging".to_string()]
        ));
        assert!(!ini_has_section(contents, &["prod".to_string()]));
        // Substring of an existing section must not match.
        assert!(!ini_has_section(contents, &["stagin".to_string()]));
    }

    #[test]
    fn home_dir_matches_the_sdk_resolution() {
        let os = |s: &str| Some(std::ffi::OsString::from(s));
        // HOME wins on every platform, even when the Windows variables are
        // set too (they are only consulted when HOME is absent).
        assert_eq!(
            home_dir_from(
                HomeOs::Windows,
                os("/home/x"),
                os("C:\\Users\\x"),
                os("C:"),
                os("\\Users\\x")
            ),
            Some(std::path::PathBuf::from("/home/x"))
        );
        // Windows without HOME: USERPROFILE, which is what a GUI-launched
        // plugin process actually has.
        assert_eq!(
            home_dir_from(
                HomeOs::Windows,
                None,
                os("C:\\Users\\x"),
                os("C:"),
                os("\\Users\\x")
            ),
            Some(std::path::PathBuf::from("C:\\Users\\x"))
        );
        // Windows without HOME or USERPROFILE: HOMEDRIVE + HOMEPATH.
        assert_eq!(
            home_dir_from(HomeOs::Windows, None, None, os("C:"), os("\\Users\\x")),
            Some(std::path::PathBuf::from("C:\\Users\\x"))
        );
        // Unix never consults the Windows variables.
        assert_eq!(
            home_dir_from(
                HomeOs::Unix,
                None,
                os("C:\\Users\\x"),
                os("C:"),
                os("\\Users\\x")
            ),
            None
        );
    }

    #[test]
    fn profile_exists_reads_credentials_and_config() {
        let _guard = crate::settings::TEST_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("ddb-prof-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let creds = dir.join("credentials");
        let config = dir.join("config");
        std::fs::write(&creds, "[default]\naws_access_key_id = x\n").unwrap();
        std::fs::write(&config, "[profile staging]\nregion = us-east-1\n").unwrap();

        std::env::set_var("AWS_SHARED_CREDENTIALS_FILE", &creds);
        std::env::set_var("AWS_CONFIG_FILE", &config);

        assert!(profile_exists("default"));
        assert!(profile_exists("staging"));
        assert!(!profile_exists("nonexistent-profile"));

        std::env::remove_var("AWS_SHARED_CREDENTIALS_FILE");
        std::env::remove_var("AWS_CONFIG_FILE");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn unknown_profile_rejected_with_clear_message() {
        let params = json!({"params": {"extra": {"profile": "definitely-not-a-real-profile-xyz"}}});
        let err = build_client(&params).await.unwrap_err();
        assert!(
            err.message.contains("not found in ~/.aws/credentials"),
            "expected profile-not-found error, got: {}",
            err.message
        );
    }

    #[test]
    fn extra_region_still_applies_when_profile_is_unset() {
        // Regression guard for the promotion above: it must not disturb the
        // existing `extra["region"]` handling.
        let _guard = crate::settings::TEST_LOCK.lock().unwrap();
        let params = json!({"params": {
            "endpoint": "http://localhost:8000",
            "extra": {"region": "ap-southeast-2"},
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["region"], "ap-southeast-2");
        assert!(n["params"].get("profile").is_none());
    }

    #[tokio::test]
    async fn extra_profile_connection_is_accepted() {
        // A profile picked in the connection modal, with nothing else filled
        // in, is a complete connection as far as validation is concerned.
        let params = json!({"params": {"extra": {"profile": "staging"}}});
        match build_client(&params).await {
            Ok(_) => {}
            Err(e) => assert!(
                !e.message.contains("connection params required"),
                "extra profile should satisfy validation, got: {}",
                e.message
            ),
        }
    }

    #[tokio::test]
    async fn extra_session_token_alone_is_not_a_connection() {
        // A session token is only meaningful alongside credentials.
        let params = json!({"params": {"extra": {"session_token": "tok"}}});
        let err = build_client(&params).await.unwrap_err();
        assert!(err.message.contains("connection params required"));
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
    fn extra_region_applies_for_profile_connections() {
        // An explicit region picked in the connection modal is a
        // connection-level choice and wins over the profile's own config;
        // only the plugin-setting / us-east-1 fallbacks stay suppressed.
        let params = json!({"params": {
            "endpoint": "http://localhost:8000",
            "profile": "default",
            "extra": {"region": "ap-southeast-2"},
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["region"], "ap-southeast-2");
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

    // ── Keys-only connections (no host/port) — issue #71 ───────────────

    #[test]
    fn keys_only_defaults_region() {
        // Credentials but no endpoint: the only thing the request needs beyond
        // the keys is a signing region, and there is no endpoint hostname to
        // parse one from — so the fallback chain must run anyway.
        let _guard = crate::settings::TEST_LOCK.lock().unwrap();
        let params = json!({"params": {"username": "AKIA", "password": "secret"}});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["region"], "us-east-1");
        assert!(n["params"].get("endpoint").is_none());
    }

    #[test]
    fn keys_only_uses_plugin_default_region() {
        let _guard = crate::settings::TEST_LOCK.lock().unwrap();
        crate::settings::apply_initialize(&json!({"settings": {"region": "ap-southeast-2"}}));
        let params = json!({"params": {"username": "AKIA", "password": "secret"}});
        let n = normalized_params(&params);
        crate::settings::apply_initialize(&json!({}));
        assert_eq!(n["params"]["region"], "ap-southeast-2");
    }

    #[test]
    fn keys_only_uses_extra_region() {
        let _guard = crate::settings::TEST_LOCK.lock().unwrap();
        let params = json!({"params": {
            "username": "AKIA",
            "password": "secret",
            "extra": {"region": "eu-west-1"},
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["region"], "eu-west-1");
    }

    #[test]
    fn keys_only_without_secret_gets_no_region() {
        // Half a credential pair is not a signing configuration, so there is
        // nothing to default a region for — the request is rejected anyway.
        let params = json!({"params": {"username": "AKIA"}});
        let n = normalized_params(&params);
        assert!(n["params"].get("region").is_none());
    }

    #[test]
    fn null_region_does_not_shadow_the_default() {
        let _guard = crate::settings::TEST_LOCK.lock().unwrap();
        let params = json!({"params": {
            "username": "AKIA",
            "password": "secret",
            "region": null,
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["region"], "us-east-1");
    }

    #[test]
    fn blank_region_does_not_shadow_the_default() {
        let _guard = crate::settings::TEST_LOCK.lock().unwrap();
        let params = json!({"params": {
            "endpoint": "http://localhost:8000",
            "region": "   ",
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["region"], "us-east-1");
    }

    #[test]
    fn blank_endpoint_counts_as_absent() {
        // The GUI sends empty host/port as blank strings rather than omitting
        // them; an empty endpoint must not suppress the region fallback.
        let _guard = crate::settings::TEST_LOCK.lock().unwrap();
        let params = json!({"params": {
            "endpoint": "",
            "username": "AKIA",
            "password": "secret",
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["region"], "us-east-1");
    }

    #[test]
    fn blank_profile_does_not_suppress_region_default() {
        // A blank profile is "not supplied" (read_param drops it), so it must
        // not exempt the connection from region defaulting.
        let _guard = crate::settings::TEST_LOCK.lock().unwrap();
        let params = json!({"params": {
            "username": "AKIA",
            "password": "secret",
            "profile": "",
        }});
        let n = normalized_params(&params);
        assert_eq!(n["params"]["region"], "us-east-1");
    }

    #[test]
    fn no_endpoint_and_no_creds_leaves_region_unset() {
        // Nothing to sign with -> no region invented; validation rejects it.
        let params = json!({"params": {"host": "localhost"}});
        let n = normalized_params(&params);
        assert!(n["params"].get("region").is_none());
    }

    #[tokio::test]
    async fn credentials_only_is_accepted() {
        // Access key + secret with no host/port/endpoint — the plugin resolves
        // the endpoint from the region (AWS default endpoint).
        let params = json!({"params": {"username": "AKIA", "password": "secret"}});
        match build_client(&params).await {
            Ok(_) => {}
            Err(e) => assert!(
                !e.message.contains("connection params required"),
                "credentials-only connection should satisfy validation, got: {}",
                e.message
            ),
        }
    }
}
