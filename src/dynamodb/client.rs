use aws_sdk_dynamodb::Client as DynamoDbClient;

use crate::dynamodb::models::{ColumnInfo, DescribeTableOutput, ExecuteStatementOutput, IndexInfo};
use crate::dynamodb::pool;
use crate::error::PluginError;

/// Wraps an AWS DynamoDB SDK client with convenience methods.
#[derive(Debug, Clone)]
pub struct Client {
    inner: DynamoDbClient,
}

impl Client {
    /// Create a new DynamoDB client from connection parameters.
    pub async fn new(
        region: Option<&str>,
        access_key_id: Option<&str>,
        secret_access_key: Option<&str>,
        session_token: Option<&str>,
        profile: Option<&str>,
        endpoint: Option<&str>,
    ) -> Result<Self, PluginError> {
        let config = pool::get_config(
            region,
            access_key_id,
            secret_access_key,
            session_token,
            profile,
            endpoint,
        )
        .await?;

        Ok(Self {
            inner: DynamoDbClient::from_conf(config),
        })
    }

    /// Ping the DynamoDB service by listing tables (limit 1).
    pub async fn ping(&self) -> Result<(), PluginError> {
        self.inner
            .list_tables()
            .limit(1)
            .send()
            .await
            .map_err(|e| PluginError::internal(format!("DynamoDB ping failed: {e}")))?;
        Ok(())
    }

    /// List all table names.
    pub async fn list_tables(&self) -> Result<Vec<String>, PluginError> {
        let mut table_names = Vec::new();
        let mut last_evaluated_table_name: Option<String> = None;

        loop {
            let mut request = self.inner.list_tables();
            if let Some(ref last) = last_evaluated_table_name {
                request = request.exclusive_start_table_name(last.clone());
            }

            let response = request
                .send()
                .await
                .map_err(|e| PluginError::internal(format!("ListTables failed: {e}")))?;

            // table_names() returns &[String]
            table_names.extend(response.table_names().iter().cloned());

            last_evaluated_table_name = response.last_evaluated_table_name().map(|s| s.to_string());

            if last_evaluated_table_name.is_none() {
                break;
            }
        }

        Ok(table_names)
    }

    /// Describe a table (schema, indexes, status).
    pub async fn describe_table(
        &self,
        table_name: &str,
    ) -> Result<DescribeTableOutput, PluginError> {
        let response = self
            .inner
            .describe_table()
            .table_name(table_name)
            .send()
            .await
            .map_err(|e| PluginError::internal(format!("DescribeTable failed: {e}")))?;

        let table = response
            .table()
            .ok_or_else(|| PluginError::internal(format!("Table '{table_name}' not found")))?;

        // attribute_definitions() returns &[AttributeDefinition]
        let attribute_definitions: Vec<ColumnInfo> = table
            .attribute_definitions()
            .iter()
            .map(|d| {
                let attr_name = d.attribute_name().to_string();
                let attr_type = d.attribute_type().as_str().to_string();
                ColumnInfo::new(attr_name, &attr_type)
            })
            .collect();

        // key_schema() returns &[KeySchemaElement]
        let key_schema: Vec<(String, String)> = table
            .key_schema()
            .iter()
            .map(|k| {
                let attr_name = k.attribute_name().to_string();
                let key_type = k.key_type().as_str().to_string();
                (attr_name, key_type)
            })
            .collect();

        // Mark PK and SK columns
        let mut columns: Vec<ColumnInfo> = attribute_definitions
            .into_iter()
            .map(|mut col| {
                for (kname, ktype) in &key_schema {
                    if col.name == *kname {
                        if ktype == "HASH" {
                            col.is_pk = true;
                        } else if ktype == "RANGE" {
                            col.is_sort_key = true;
                        }
                    }
                }
                col
            })
            .collect();

        // Add any key-only columns not in attribute_definitions
        for (kname, _) in &key_schema {
            if !columns.iter().any(|c| c.name == *kname) {
                let mut col = ColumnInfo::new(kname.clone(), "S");
                for (kn, kt) in &key_schema {
                    if col.name == *kn {
                        if kt == "HASH" {
                            col.is_pk = true;
                        } else if kt == "RANGE" {
                            col.is_sort_key = true;
                        }
                    }
                }
                columns.push(col);
            }
        }

        let indexes = Self::extract_indexes(table);

        Ok(DescribeTableOutput {
            table_name: table.table_name().map(|s| s.to_string()),
            columns,
            indexes,
            table_status: table.table_status().map(|s| s.as_str().to_string()),
            item_count: table.item_count(),
            table_size_bytes: table.table_size_bytes(),
        })
    }

    fn extract_indexes(table: &aws_sdk_dynamodb::types::TableDescription) -> Vec<IndexInfo> {
        let mut indexes = Vec::new();

        // Primary key as an index
        let pk_columns: Vec<String> = table
            .key_schema()
            .iter()
            .map(|k| k.attribute_name().to_string())
            .collect();
        if !pk_columns.is_empty() {
            indexes.push(IndexInfo {
                name: "primary".to_string(),
                columns: pk_columns,
                is_unique: true,
                is_primary: true,
            });
        }

        // Global Secondary Indexes
        for gsi in table.global_secondary_indexes() {
            let columns: Vec<String> = gsi
                .key_schema()
                .iter()
                .map(|k| k.attribute_name().to_string())
                .collect();
            indexes.push(IndexInfo {
                name: gsi.index_name().unwrap_or("unknown").to_string(),
                columns,
                is_unique: false,
                is_primary: false,
            });
        }

        // Local Secondary Indexes
        for lsi in table.local_secondary_indexes() {
            let columns: Vec<String> = lsi
                .key_schema()
                .iter()
                .map(|k| k.attribute_name().to_string())
                .collect();
            indexes.push(IndexInfo {
                name: lsi.index_name().unwrap_or("unknown").to_string(),
                columns,
                is_unique: false,
                is_primary: false,
            });
        }

        indexes
    }

    /// Execute a PartiQL statement.
    pub async fn execute_statement(
        &self,
        statement: &str,
    ) -> Result<ExecuteStatementOutput, PluginError> {
        let response = self
            .inner
            .execute_statement()
            .statement(statement)
            .send()
            .await
            .map_err(|e| PluginError::internal(format!("ExecuteStatement failed: {e}")))?;

        Ok(ExecuteStatementOutput::from_sdk(response))
    }

    /// Execute a PartiQL statement with pagination token.
    pub async fn execute_statement_with_token(
        &self,
        statement: &str,
        next_token: Option<&str>,
    ) -> Result<ExecuteStatementOutput, PluginError> {
        let mut request = self.inner.execute_statement().statement(statement);
        if let Some(token) = next_token {
            request = request.next_token(token);
        }

        let response = request
            .send()
            .await
            .map_err(|e| PluginError::internal(format!("ExecuteStatement failed: {e}")))?;

        Ok(ExecuteStatementOutput::from_sdk(response))
    }

    /// Return the set of key attribute names for a table (HASH + RANGE).
    /// Used to validate that a caller-supplied `key` object only contains
    /// real key columns before building a PartiQL WHERE clause.
    pub async fn table_key_columns(&self, table_name: &str) -> Result<Vec<String>, PluginError> {
        let desc = self.describe_table(table_name).await?;
        Ok(desc
            .columns
            .iter()
            .filter(|c| c.is_pk || c.is_sort_key)
            .map(|c| c.name.clone())
            .collect())
    }

    /// Insert a full item via the native PutItem API (no 8KB statement limit,
    /// supports nested maps/lists and binary values).
    pub async fn put_item(
        &self,
        table_name: &str,
        item: std::collections::HashMap<String, aws_sdk_dynamodb::types::AttributeValue>,
    ) -> Result<(), PluginError> {
        self.inner
            .put_item()
            .table_name(table_name)
            .set_item(Some(item))
            .send()
            .await
            .map_err(|e| PluginError::internal(format!("PutItem failed: {e}")))?;
        Ok(())
    }

    /// Update a single attribute on an item identified by its key, via the
    /// native UpdateItem API (avoids PartiQL escaping/size limits).
    pub async fn update_item(
        &self,
        table_name: &str,
        key: std::collections::HashMap<String, aws_sdk_dynamodb::types::AttributeValue>,
        col_name: &str,
        new_val: aws_sdk_dynamodb::types::AttributeValue,
    ) -> Result<(), PluginError> {
        self.inner
            .update_item()
            .table_name(table_name)
            .set_key(Some(key))
            .update_expression("SET #attr = :val")
            .expression_attribute_names("#attr".to_string(), col_name.to_string())
            .expression_attribute_values(":val".to_string(), new_val)
            .send()
            .await
            .map_err(|e| PluginError::internal(format!("UpdateItem failed: {e}")))?;
        Ok(())
    }

    /// Delete an item identified by its key via the native DeleteItem API.
    pub async fn delete_item(
        &self,
        table_name: &str,
        key: std::collections::HashMap<String, aws_sdk_dynamodb::types::AttributeValue>,
    ) -> Result<(), PluginError> {
        self.inner
            .delete_item()
            .table_name(table_name)
            .set_key(Some(key))
            .send()
            .await
            .map_err(|e| PluginError::internal(format!("DeleteItem failed: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn client_new_returns_ok_with_valid_params() {
        let client = Client::new(
            Some("us-east-1"),
            Some("AKID"),
            Some("SAK"),
            None,
            None,
            Some("http://localhost:8000"),
        )
        .await;
        assert!(client.is_ok(), "should create client: {:?}", client.err());
    }

    #[tokio::test]
    async fn client_new_returns_ok_with_minimal_params() {
        let client = Client::new(Some("us-east-1"), None, None, None, None, None).await;
        assert!(client.is_ok(), "should create client: {:?}", client.err());
    }
}
