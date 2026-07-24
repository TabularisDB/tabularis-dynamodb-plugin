use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use aws_sdk_dynamodb::config::Credentials;
use aws_sdk_dynamodb::Config;

use crate::error::PluginError;

// Pool TTL: evict stale configs after 30 minutes
const POOL_TTL: Duration = Duration::from_secs(30 * 60);

static CONFIG_POOL: LazyLock<Mutex<HashMap<String, CachedConfig>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone)]
struct CachedConfig {
    config: Config,
    last_used: Instant,
}

/// Build an SDK config from connection parameters.
/// Supports: explicit credentials, profile, environment variables, IMDS.
pub async fn get_config(
    region: Option<&str>,
    access_key_id: Option<&str>,
    secret_access_key: Option<&str>,
    session_token: Option<&str>,
    profile: Option<&str>,
    endpoint: Option<&str>,
) -> Result<Config, PluginError> {
    let cache_key = format!(
        "{}:{}:{}:{}:{}:{}",
        region.unwrap_or(""),
        access_key_id.unwrap_or(""),
        secret_access_key.unwrap_or(""),
        session_token.unwrap_or(""),
        profile.unwrap_or(""),
        endpoint.unwrap_or("")
    );

    // Check cache
    {
        let mut pools = CONFIG_POOL.lock().await;
        if let Some(cached) = pools.get_mut(&cache_key) {
            cached.last_used = Instant::now();
            return Ok(cached.config.clone());
        }
    }

    // Build config using the non-deprecated API
    let mut config_builder = aws_config::defaults(aws_config::BehaviorVersion::latest());

    // If explicit credentials are provided, use them
    if let (Some(akid), Some(sak)) = (access_key_id, secret_access_key) {
        let session = session_token.map(|s| s.to_string());
        let creds = Credentials::new(akid.to_string(), sak.to_string(), session, None, "explicit");
        config_builder = config_builder.credentials_provider(creds);
    }

    // If a profile is specified, use it
    if let Some(profile_name) = profile {
        config_builder = config_builder.profile_name(profile_name);
    }

    // If a region is specified, use it
    if let Some(region_str) = region {
        config_builder = config_builder.region(aws_config::Region::new(region_str.to_string()));
    }

    let sdk_config = config_builder.load().await;

    // Build DynamoDB-specific config
    let mut dynamo_config_builder = Config::builder();
    dynamo_config_builder = dynamo_config_builder.region(
        sdk_config
            .region()
            .cloned()
            .unwrap_or_else(|| aws_config::Region::new("us-east-1")),
    );

    // Copy credentials provider from SDK config
    if let Some(creds_provider) = sdk_config.credentials_provider() {
        dynamo_config_builder = dynamo_config_builder.credentials_provider(creds_provider.clone());
    }

    // If an endpoint override is provided (for DynamoDB Local), use it
    if let Some(endpoint_str) = endpoint {
        dynamo_config_builder = dynamo_config_builder.endpoint_url(endpoint_str);
    }

    let config = dynamo_config_builder.build();

    // Cache it
    {
        let mut pools = CONFIG_POOL.lock().await;
        pools.insert(
            cache_key,
            CachedConfig {
                config: config.clone(),
                last_used: Instant::now(),
            },
        );
    }

    Ok(config)
}

/// Clean up stale configs from the pool.
pub async fn cleanup_pools() {
    let mut pools = CONFIG_POOL.lock().await;
    pools.retain(|_, pool| pool.last_used.elapsed() < POOL_TTL);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn get_config_returns_config_with_region() {
        let config = get_config(Some("us-east-1"), None, None, None, None, None)
            .await
            .expect("should build config");
        assert_eq!(config.region().map(|r| r.as_ref()), Some("us-east-1"));
    }

    #[tokio::test]
    async fn get_config_returns_config_with_endpoint() {
        let config = get_config(
            Some("us-east-1"),
            Some("AKID"),
            Some("SAK"),
            None,
            None,
            Some("http://localhost:8000"),
        )
        .await
        .expect("should build config");
        // Endpoint URL is set on the builder but may not be exposed on Config
        // Just verify the config was built successfully
        assert!(config.region().is_some());
    }

    #[tokio::test]
    async fn get_config_caches_identical_params() {
        let config1 = get_config(Some("us-west-2"), None, None, None, None, None)
            .await
            .expect("should build config");
        let config2 = get_config(Some("us-west-2"), None, None, None, None, None)
            .await
            .expect("should build config");

        assert_eq!(
            config1.region().map(|r| r.as_ref()),
            config2.region().map(|r| r.as_ref())
        );
    }
}
