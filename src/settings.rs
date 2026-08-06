//! Plugin-level settings delivered by the Tabularis host.
//!
//! The host renders the `settings` array from `.tabularium` on its plugin
//! settings page and sends the saved values to this process in the
//! `initialize` RPC (`params.settings`). Values are process-global — the
//! host spawns one plugin process per driver, not per connection.

use std::sync::OnceLock;
use std::sync::RwLock;

use serde_json::Value;

#[derive(Debug, Default)]
struct PluginSettings {
    /// Default AWS region used when a connection supplies neither an explicit
    /// `region` nor an AWS endpoint hostname to parse one from.
    default_region: Option<String>,
}

static SETTINGS: OnceLock<RwLock<PluginSettings>> = OnceLock::new();

fn cell() -> &'static RwLock<PluginSettings> {
    SETTINGS.get_or_init(|| RwLock::new(PluginSettings::default()))
}

/// Store settings from an `initialize` request. Unknown/missing keys are
/// ignored; blank strings are treated as unset.
pub fn apply_initialize(params: &Value) {
    let region = params
        .get("settings")
        .and_then(|s| s.get("region"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if let Ok(mut guard) = cell().write() {
        guard.default_region = region;
    }
}

/// The configured default region, if any.
pub fn default_region() -> Option<String> {
    cell().read().ok().and_then(|g| g.default_region.clone())
}

#[cfg(test)]
pub static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Single test function: the settings cell is process-global, so parallel
    /// tests mutating it would race. Hold TEST_LOCK so connection-handler
    /// tests exercising the region fallback don't observe a half-set value.
    #[test]
    fn initialize_settings_roundtrip() {
        let _guard = TEST_LOCK.lock().unwrap();

        apply_initialize(&json!({"settings": {"region": "eu-west-1"}}));
        assert_eq!(default_region().as_deref(), Some("eu-west-1"));

        apply_initialize(&json!({"settings": {"region": "  "}}));
        assert_eq!(default_region(), None);

        apply_initialize(&json!({}));
        assert_eq!(default_region(), None);
    }
}
