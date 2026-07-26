use serde_json::Value;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Plugin-internal types (not serde-deserialized from SDK — we extract manually)
// ---------------------------------------------------------------------------

/// Represents a column in a DynamoDB table (attribute definition).
#[derive(Debug, Clone)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub is_pk: bool,
    pub is_sort_key: bool,
}

impl ColumnInfo {
    pub fn new(name: String, data_type: &str) -> Self {
        Self {
            name,
            data_type: attribute_type_name(data_type).to_string(),
            is_pk: false,
            is_sort_key: false,
        }
    }
}

/// Represents an index (GSI or LSI).
#[derive(Debug, Clone)]
pub struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub is_unique: bool,
    pub is_primary: bool,
}

/// Output of describe_table.
#[derive(Debug, Clone)]
pub struct DescribeTableOutput {
    pub table_name: Option<String>,
    pub columns: Vec<ColumnInfo>,
    pub indexes: Vec<IndexInfo>,
    pub table_status: Option<String>,
    pub item_count: Option<i64>,
    pub table_size_bytes: Option<i64>,
}

/// Extract read/write capacity units from a DynamoDB `ConsumedCapacity`.
/// Returns `None` when the API didn't report capacity (e.g. capacity reporting
/// was not requested).
pub(crate) fn consumed_capacity_units(cc: &aws_sdk_dynamodb::types::ConsumedCapacity) -> f64 {
    cc.capacity_units().unwrap_or(0.0)
}

/// Output of execute_statement.
#[derive(Debug, Clone)]
pub struct ExecuteStatementOutput {
    pub items: Vec<HashMap<String, Value>>,
    pub next_token: Option<String>,
    pub consumed_capacity: Option<f64>,
}
impl ExecuteStatementOutput {
    pub fn from_sdk(
        response: aws_sdk_dynamodb::operation::execute_statement::ExecuteStatementOutput,
    ) -> Self {
        // items() returns &[HashMap<String, AttributeValue>]
        let items = response
            .items()
            .iter()
            .map(|item| {
                let mut map = HashMap::new();
                for (key, val) in item.iter() {
                    map.insert(key.clone(), attribute_value_to_json(val));
                }
                map
            })
            .collect();

        let next_token = response.next_token().map(|s| s.to_string());
        let consumed_capacity = response.consumed_capacity().map(consumed_capacity_units);

        Self {
            items,
            next_token,
            consumed_capacity,
        }
    }
}

/// Generic output of a native SDK read operation (Scan / Query / GetItem),
/// using an opaque, base64-encoded pagination token derived from
/// `LastEvaluatedKey` so callers can resume reads.
#[derive(Debug, Clone)]
pub struct ItemOutput {
    pub items: Vec<HashMap<String, Value>>,
    pub next_token: Option<String>,
    pub consumed_capacity: Option<f64>,
}

impl ItemOutput {
    pub fn new(
        raw_items: &[HashMap<String, aws_sdk_dynamodb::types::AttributeValue>],
        last_key: Option<&HashMap<String, aws_sdk_dynamodb::types::AttributeValue>>,
        consumed_capacity: Option<f64>,
    ) -> Self {
        let items = raw_items
            .iter()
            .map(|item| {
                let mut map = HashMap::new();
                for (k, v) in item.iter() {
                    map.insert(k.clone(), attribute_value_to_json(v));
                }
                map
            })
            .collect();

        let next_token = last_key.and_then(encode_last_key);

        Self {
            items,
            next_token,
            consumed_capacity,
        }
    }
}

use base64::Engine as _;

/// Encode a DynamoDB `LastEvaluatedKey` into an opaque base64 token.
fn encode_last_key(
    key: &HashMap<String, aws_sdk_dynamodb::types::AttributeValue>,
) -> Option<String> {
    // Serialize to a plain JSON map of {attr: json-value}, then base64 it.
    let mut obj = serde_json::Map::new();
    for (k, v) in key.iter() {
        obj.insert(k.clone(), attribute_value_to_json(v));
    }
    let json = serde_json::Value::Object(obj);
    let raw = serde_json::to_vec(&json).ok()?;
    Some(base64::engine::general_purpose::STANDARD.encode(raw))
}

/// Decode an opaque base64 token back into a DynamoDB key map (AttributeValue form).
pub fn decode_pagination_token(
    token: &str,
) -> Option<HashMap<String, aws_sdk_dynamodb::types::AttributeValue>> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(token.trim())
        .ok()?;
    let value: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    let obj = value.as_object()?;
    let mut key = HashMap::new();
    for (k, v) in obj.iter() {
        key.insert(k.clone(), json_to_attribute_value(v));
    }
    Some(key)
}

/// Convert an SDK AttributeValue to a serde_json Value.
pub fn attribute_value_to_json(av: &aws_sdk_dynamodb::types::AttributeValue) -> Value {
    use aws_sdk_dynamodb::types::AttributeValue;

    match av {
        AttributeValue::S(s) => Value::String(s.clone()),
        AttributeValue::N(n) => Value::String(n.clone()),
        AttributeValue::B(_) => Value::String("[Binary]".to_string()),
        AttributeValue::Bool(b) => Value::Bool(*b),
        AttributeValue::Null(_) => Value::Null,
        AttributeValue::Ss(ss) => {
            Value::Array(ss.iter().map(|s| Value::String(s.clone())).collect())
        }
        AttributeValue::Ns(ns) => {
            Value::Array(ns.iter().map(|n| Value::String(n.clone())).collect())
        }
        AttributeValue::Bs(_) => Value::Array(vec![]),
        AttributeValue::L(list) => Value::Array(list.iter().map(attribute_value_to_json).collect()),
        AttributeValue::M(map) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in map.iter() {
                obj.insert(k.clone(), attribute_value_to_json(v));
            }
            Value::Object(obj)
        }
        _ => Value::Null,
    }
}

/// Convert a serde_json Value to an SDK AttributeValue (for PutItem / typed writes).
pub fn json_to_attribute_value(v: &Value) -> aws_sdk_dynamodb::types::AttributeValue {
    use aws_sdk_dynamodb::types::AttributeValue;

    match v {
        Value::String(s) => AttributeValue::S(s.clone()),
        // JSON numbers map to DynamoDB's numeric type (stored as decimal strings).
        Value::Number(n) => AttributeValue::N(n.to_string()),
        Value::Bool(b) => AttributeValue::Bool(*b),
        Value::Null => AttributeValue::Null(true),
        Value::Array(arr) => AttributeValue::L(arr.iter().map(json_to_attribute_value).collect()),
        Value::Object(obj) => {
            let mut map = std::collections::HashMap::new();
            for (k, val) in obj.iter() {
                map.insert(k.clone(), json_to_attribute_value(val));
            }
            AttributeValue::M(map)
        }
    }
}

/// Maps DynamoDB attribute types to human-readable type names.
pub fn attribute_type_name(attr_type: &str) -> &'static str {
    match attr_type {
        "S" => "STRING",
        "N" => "NUMBER",
        "B" => "BINARY",
        "BOOL" => "BOOLEAN",
        "SS" => "STRING_SET",
        "NS" => "NUMBER_SET",
        "BS" => "BINARY_SET",
        "L" => "JSON",
        "M" => "JSON",
        "NULL" => "NULL",
        _ => "STRING",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attribute_type_name_maps_all_types() {
        assert_eq!(attribute_type_name("S"), "STRING");
        assert_eq!(attribute_type_name("N"), "NUMBER");
        assert_eq!(attribute_type_name("B"), "BINARY");
        assert_eq!(attribute_type_name("BOOL"), "BOOLEAN");
        assert_eq!(attribute_type_name("SS"), "STRING_SET");
        assert_eq!(attribute_type_name("NS"), "NUMBER_SET");
        assert_eq!(attribute_type_name("BS"), "BINARY_SET");
        assert_eq!(attribute_type_name("L"), "JSON");
        assert_eq!(attribute_type_name("M"), "JSON");
        assert_eq!(attribute_type_name("NULL"), "NULL");
    }

    #[test]
    fn attribute_type_name_defaults_to_string() {
        assert_eq!(attribute_type_name("UNKNOWN"), "STRING");
    }

    #[test]
    fn column_info_new_sets_correct_type() {
        let col = ColumnInfo::new("id".to_string(), "S");
        assert_eq!(col.name, "id");
        assert_eq!(col.data_type, "STRING");
        assert!(!col.is_pk);
        assert!(!col.is_sort_key);
    }

    #[test]
    fn attribute_value_string_to_json() {
        let av = aws_sdk_dynamodb::types::AttributeValue::S("hello".to_string());
        assert_eq!(
            attribute_value_to_json(&av),
            Value::String("hello".to_string())
        );
    }

    #[test]
    fn attribute_value_number_to_json() {
        let av = aws_sdk_dynamodb::types::AttributeValue::N("42".to_string());
        assert_eq!(
            attribute_value_to_json(&av),
            Value::String("42".to_string())
        );
    }

    #[test]
    fn attribute_value_bool_to_json() {
        let av = aws_sdk_dynamodb::types::AttributeValue::Bool(true);
        assert_eq!(attribute_value_to_json(&av), Value::Bool(true));
    }

    #[test]
    fn attribute_value_null_to_json() {
        let av = aws_sdk_dynamodb::types::AttributeValue::Null(true);
        assert_eq!(attribute_value_to_json(&av), Value::Null);
    }

    #[test]
    fn attribute_value_string_set_to_json() {
        let av = aws_sdk_dynamodb::types::AttributeValue::Ss(vec!["a".into(), "b".into()]);
        let json = attribute_value_to_json(&av);
        assert_eq!(json.as_array().unwrap().len(), 2);
    }

    #[test]
    fn attribute_value_list_to_json() {
        let av = aws_sdk_dynamodb::types::AttributeValue::L(vec![
            aws_sdk_dynamodb::types::AttributeValue::S("a".into()),
            aws_sdk_dynamodb::types::AttributeValue::N("1".into()),
        ]);
        let json = attribute_value_to_json(&av);
        assert_eq!(json.as_array().unwrap().len(), 2);
    }

    #[test]
    fn attribute_value_map_to_json() {
        let mut map = HashMap::new();
        map.insert(
            "name".to_string(),
            aws_sdk_dynamodb::types::AttributeValue::S("Alice".into()),
        );
        let av = aws_sdk_dynamodb::types::AttributeValue::M(map);
        let json = attribute_value_to_json(&av);
        assert_eq!(json["name"], Value::String("Alice".to_string()));
    }
}
