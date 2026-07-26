use serde::Serialize;
use serde_json::Value;

use crate::dynamodb::models::attribute_type_name;

/// Represents a parsed query with mode detection.
#[derive(Debug, Clone, PartialEq)]
pub enum QueryMode {
    Partiql,
    Scan,
    Query,
    Get,
}

#[derive(Debug, Clone)]
pub struct Query {
    pub mode: QueryMode,
    pub body: String,
}

impl From<String> for Query {
    fn from(query: String) -> Self {
        let query = query.trim_start().to_string();

        let (header, body) = query.split_once('\n').unwrap_or((&query, ""));

        let (mode, body) = match header.trim() {
            "#!partiql" => (QueryMode::Partiql, body.trim_start()),
            "#!scan" => (QueryMode::Scan, body.trim_start()),
            "#!query" => (QueryMode::Query, body.trim_start()),
            "#!get" => (QueryMode::Get, body.trim_start()),
            _ => (QueryMode::Partiql, query.as_str()),
        };

        Self {
            mode,
            body: body.to_owned(),
        }
    }
}

/// Response shape for execute_query.
#[derive(Debug, Serialize)]
pub struct ExecuteQueryResponse {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub affected_rows: usize,
    pub execution_time_ms: usize,
    pub truncated: bool,
    pub has_more: bool,
    pub pagination: Option<Value>,
    /// Consumed read/write capacity units reported by DynamoDB (#8). Only
    /// present when the underlying API returned capacity information.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumed_capacity: Option<f64>,
    /// Warning for destructive operations that were blocked pending
    /// confirmation (#8). Present when execution was refused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

impl ExecuteQueryResponse {
    pub fn empty() -> Self {
        Self {
            columns: vec![],
            rows: vec![],
            affected_rows: 0,
            execution_time_ms: 0,
            truncated: false,
            has_more: false,
            pagination: None,
            consumed_capacity: None,
            warning: None,
        }
    }
}

/// Response shape for get_columns.
#[derive(Debug, Serialize)]
pub struct ColumnResponse {
    pub name: String,
    pub data_type: String,
    pub is_pk: bool,
    pub is_nullable: bool,
    pub is_auto_increment: bool,
}

impl ColumnResponse {
    pub fn new(name: String, attr_type: &str, is_pk: bool, _is_sort_key: bool) -> Self {
        Self {
            name,
            data_type: attribute_type_name(attr_type).to_string(),
            is_pk,
            is_nullable: false,
            is_auto_increment: false,
        }
    }
}

/// Response shape for get_indexes.
#[derive(Debug, Serialize)]
pub struct IndexResponse {
    pub name: String,
    pub columns: Vec<String>,
    pub is_unique: bool,
    pub is_primary: bool,
}

/// Converts a DynamoDB item (HashMap<String, Value>) to a flat row of values.
pub fn dynamo_item_to_row(
    item: &std::collections::HashMap<String, Value>,
    columns: &[String],
) -> Vec<Value> {
    columns
        .iter()
        .map(|col| {
            item.get(col)
                .and_then(|v| {
                    // DynamoDB returns typed values like {"S": "hello"}, {"N": "42"}
                    // Extract the actual value
                    v.as_object().and_then(|obj| obj.values().next().cloned())
                })
                .unwrap_or(Value::Null)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_partiql_query_default() {
        let q = Query::from("SELECT * FROM users".to_string());
        assert_eq!(q.mode, QueryMode::Partiql);
        assert_eq!(q.body, "SELECT * FROM users");
    }

    #[test]
    fn parse_partiql_query_explicit() {
        let q = Query::from("#!partiql\nSELECT * FROM users".to_string());
        assert_eq!(q.mode, QueryMode::Partiql);
        assert_eq!(q.body, "SELECT * FROM users");
    }

    #[test]
    fn parse_scan_query() {
        let q = Query::from("#!scan\nTableName: users\nLimit: 100".to_string());
        assert_eq!(q.mode, QueryMode::Scan);
        assert!(q.body.contains("TableName: users"));
    }

    #[test]
    fn parse_query_mode() {
        let q =
            Query::from("#!query\nTableName: users\nKeyConditionExpression: id = :id".to_string());
        assert_eq!(q.mode, QueryMode::Query);
        assert!(q.body.contains("KeyConditionExpression"));
    }

    #[test]
    fn parse_get_mode() {
        let q = Query::from("#!get\nTableName: users\nKey: {\"id\": {\"S\": \"abc\"}}".to_string());
        assert_eq!(q.mode, QueryMode::Get);
        assert!(q.body.contains("TableName: users"));
    }

    #[test]
    fn column_response_creates_correctly() {
        let col = ColumnResponse::new("id".into(), "S", true, false);
        assert_eq!(col.name, "id");
        assert_eq!(col.data_type, "STRING");
        assert!(col.is_pk);
        assert!(!col.is_nullable);
        assert!(!col.is_auto_increment);
    }

    #[test]
    fn column_response_handles_number_type() {
        let col = ColumnResponse::new("age".into(), "N", false, false);
        assert_eq!(col.name, "age");
        assert_eq!(col.data_type, "NUMBER");
        assert!(!col.is_pk);
    }

    #[test]
    fn dynamo_item_to_row_extracts_values() {
        let mut item = std::collections::HashMap::new();
        item.insert("id".into(), json!({"S": "user1"}));
        item.insert("name".into(), json!({"S": "Alice"}));
        item.insert("age".into(), json!({"N": "30"}));

        let columns = vec!["id".into(), "name".into(), "age".into()];
        let row = dynamo_item_to_row(&item, &columns);

        assert_eq!(row.len(), 3);
        assert_eq!(row[0], json!("user1"));
        assert_eq!(row[1], json!("Alice"));
        assert_eq!(row[2], json!("30"));
    }

    #[test]
    fn dynamo_item_to_row_handles_missing_columns() {
        let item = std::collections::HashMap::new();
        let columns = vec!["missing".into()];
        let row = dynamo_item_to_row(&item, &columns);
        assert_eq!(row[0], Value::Null);
    }

    #[test]
    fn execute_query_response_empty() {
        let resp = ExecuteQueryResponse::empty();
        assert!(resp.columns.is_empty());
        assert!(resp.rows.is_empty());
        assert_eq!(resp.affected_rows, 0);
    }
}
