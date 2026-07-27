use serde_json::Value;

/// Extracts the URL/connection string from the params object.
/// Tabularis stores the connection URI in `params.params.database` for
/// URI-passthrough drivers, or in `params.params.connection_string`.
pub fn extract_url(params: &Value) -> Option<String> {
    // Try connection_string first (explicit URI passthrough)
    if let Some(url) = params
        .get("params")
        .and_then(|p| p.get("connection_string"))
        .and_then(|d| d.as_str())
    {
        if !url.is_empty() {
            return Some(url.to_string());
        }
    }

    // Try database field (used by URI-passthrough drivers)
    if let Some(url) = params
        .get("params")
        .and_then(|p| p.get("database"))
        .and_then(|d| d.as_str())
    {
        if !url.is_empty() {
            return Some(url.to_string());
        }
    }

    // Try connection_uri
    if let Some(url) = params
        .get("params")
        .and_then(|p| p.get("connection_uri"))
        .and_then(|d| d.as_str())
    {
        if !url.is_empty() {
            return Some(url.to_string());
        }
    }

    None
}

/// Extracts the table name from the params object.
///
/// The TabularisDB GUI surfaces the selected table name in one of several
/// locations depending on the workflow context. Check the top-level keys first,
/// then fall back to the nested `params.params` object (the same shape the
/// connection handler reads for credentials / endpoint). Accept both `table`
/// and `table_name` (the `drop_table` handler supports both).
pub fn extract_table(params: &Value) -> Option<String> {
    // Top-level: "table" or "table_name"
    let top = params
        .get("table")
        .or_else(|| params.get("table_name"))
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty());
    if let Some(t) = top {
        return Some(t.to_string());
    }

    // Nested inside params.params (the GUI's output shape for tab-targeted
    // metadata queries like get_columns / get_indexes).
    let nested = params
        .get("params")
        .and_then(|p| p.get("table"))
        .or_else(|| params.get("params").and_then(|p| p.get("table_name")))
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty());
    if let Some(t) = nested {
        return Some(t.to_string());
    }

    None
}

/// Extracts the query string from the params object.
pub fn extract_query(params: &Value) -> Option<String> {
    params
        .get("query")
        .and_then(|d| d.as_str())
        .or_else(|| {
            params
                .get("params")
                .and_then(|p| p.get("query"))
                .and_then(|d| d.as_str())
        })
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Extracts the schema from the params object.
pub fn extract_schema(params: &Value) -> Option<String> {
    params
        .get("schema")
        .and_then(|s| s.as_str())
        .or_else(|| {
            params
                .get("params")
                .and_then(|p| p.get("schema"))
                .and_then(|s| s.as_str())
        })
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Extracts the limit from the params object.
pub fn extract_limit(params: &Value) -> Option<u32> {
    params
        .get("limit")
        .and_then(|l| l.as_u64())
        .map(|l| l as u32)
}

/// Extracts the page from the params object.
pub fn extract_page(params: &Value) -> u32 {
    params
        .get("page")
        .and_then(|p| p.as_u64())
        .map(|p| p as u32)
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_url_from_connection_string() {
        let params = json!({
            "params": {
                "connection_string": "http://localhost:8000"
            }
        });
        assert_eq!(extract_url(&params), Some("http://localhost:8000".into()));
    }

    #[test]
    fn extract_url_from_database_field() {
        let params = json!({
            "params": {
                "database": "http://localhost:8000"
            }
        });
        assert_eq!(extract_url(&params), Some("http://localhost:8000".into()));
    }

    #[test]
    fn extract_url_from_connection_uri() {
        let params = json!({
            "params": {
                "connection_uri": "http://localhost:8000"
            }
        });
        assert_eq!(extract_url(&params), Some("http://localhost:8000".into()));
    }

    #[test]
    fn extract_url_returns_none_when_missing() {
        let params = json!({"params": {}});
        assert_eq!(extract_url(&params), None);
    }

    #[test]
    fn extract_url_returns_none_when_empty() {
        let params = json!({
            "params": {
                "database": ""
            }
        });
        assert_eq!(extract_url(&params), None);
    }

    #[test]
    fn extract_table_returns_table_name() {
        let params = json!({"table": "users"});
        assert_eq!(extract_table(&params), Some("users".into()));
    }

    #[test]
    fn extract_table_accepts_table_name_key() {
        let params = json!({"table_name": "orders"});
        assert_eq!(extract_table(&params), Some("orders".into()));
    }

    #[test]
    fn extract_table_looks_inside_nested_params() {
        let params = json!({"params": {"table": "users"}});
        assert_eq!(extract_table(&params), Some("users".into()));
    }

    #[test]
    fn extract_table_looks_inside_nested_params_table_name() {
        let params = json!({"params": {"table_name": "orders"}});
        assert_eq!(extract_table(&params), Some("orders".into()));
    }

    #[test]
    fn extract_table_returns_none_when_missing() {
        let params = json!({});
        assert_eq!(extract_table(&params), None);
    }

    #[test]
    fn extract_query_returns_query_string() {
        let params = json!({"query": "SELECT * FROM users"});
        assert_eq!(extract_query(&params), Some("SELECT * FROM users".into()));
    }

    #[test]
    fn extract_schema_returns_schema() {
        let params = json!({"schema": "us-east-1"});
        assert_eq!(extract_schema(&params), Some("us-east-1".into()));
    }

    #[test]
    fn extract_limit_returns_limit() {
        let params = json!({"limit": 100});
        assert_eq!(extract_limit(&params), Some(100));
    }

    #[test]
    fn extract_limit_returns_none_when_missing() {
        let params = json!({});
        assert_eq!(extract_limit(&params), None);
    }

    #[test]
    fn extract_page_defaults_to_one() {
        let params = json!({});
        assert_eq!(extract_page(&params), 1);
    }

    #[test]
    fn extract_page_returns_specified_page() {
        let params = json!({"page": 3});
        assert_eq!(extract_page(&params), 3);
    }
}
