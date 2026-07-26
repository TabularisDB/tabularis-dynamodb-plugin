use aws_sdk_dynamodb::Client as DynamoDbClient;

use crate::dynamodb::models::{
    ColumnInfo, DescribeTableOutput, ExecuteStatementOutput, IndexInfo, TransactionOutput,
};
use crate::dynamodb::pool;
use crate::error::PluginError;

/// Arguments for a native DynamoDB Query (single partition-key lookup).
#[derive(Debug, Clone)]
pub struct QueryArgs {
    pub table_name: String,
    pub pk_name: String,
    pub pk_val: aws_sdk_dynamodb::types::AttributeValue,
    pub sk_name: Option<String>,
    pub sk_val: Option<aws_sdk_dynamodb::types::AttributeValue>,
    pub limit: Option<i32>,
    pub exclusive_start_key:
        Option<std::collections::HashMap<String, aws_sdk_dynamodb::types::AttributeValue>>,
}

/// Wraps an AWS DynamoDB SDK client with convenience methods.
#[derive(Debug, Clone)]
pub struct Client {
    inner: DynamoDbClient,
}

impl Client {
    /// Hard cap on items returned by a single Scan request (#8). Prevents
    /// runaway full-table scans on large tables; callers page past it with
    /// the returned `next_token`.
    pub const MAX_SCAN_ITEMS: i32 = 1000;

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
            .return_consumed_capacity(aws_sdk_dynamodb::types::ReturnConsumedCapacity::Total)
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
        let mut request = self
            .inner
            .execute_statement()
            .statement(statement)
            .return_consumed_capacity(aws_sdk_dynamodb::types::ReturnConsumedCapacity::Total);
        if let Some(token) = next_token {
            request = request.next_token(token);
        }

        let response = request
            .send()
            .await
            .map_err(|e| PluginError::internal(format!("ExecuteStatement failed: {e}")))?;

        Ok(ExecuteStatementOutput::from_sdk(response))
    }

    /// Execute a multi-statement PartiQL transaction (#17).
    ///
    /// All statements run atomically via DynamoDB's `ExecuteTransaction` API —
    /// either every statement succeeds or none of them are applied. DynamoDB
    /// limits transactions to 100 statements and 25 unique items touched.
    ///
    /// `client_request_token` enables idempotent retries: if the same token is
    /// resubmitted within 10 minutes, DynamoDB returns the original result
    /// without re-executing.
    pub async fn execute_transaction(
        &self,
        statements: &[String],
        client_request_token: Option<&str>,
    ) -> Result<TransactionOutput, PluginError> {
        if statements.is_empty() {
            return Err(PluginError::invalid_params(
                "execute_transaction requires at least one statement".to_string(),
            ));
        }
        if statements.len() > 100 {
            return Err(PluginError::invalid_params(format!(
                "DynamoDB transactions support at most 100 statements, got {}",
                statements.len()
            )));
        }

        let param_statements: Vec<aws_sdk_dynamodb::types::ParameterizedStatement> = statements
            .iter()
            .map(|s| {
                aws_sdk_dynamodb::types::ParameterizedStatement::builder()
                    .statement(s)
                    .build()
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                PluginError::internal(format!("failed to build transaction statement: {e}"))
            })?;

        let mut req = self
            .inner
            .execute_transaction()
            .set_transact_statements(Some(param_statements));
        if let Some(token) = client_request_token {
            req = req.client_request_token(token);
        }

        let response = req
            .send()
            .await
            .map_err(|e| PluginError::internal(format!("ExecuteTransaction failed: {e}")))?;

        Ok(TransactionOutput::from_sdk(response, statements.len()))
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

    /// Return the partition-key (HASH) column name for a table, used to build
    /// the default idempotent-insert condition `attribute_not_exists(pk)`.
    pub async fn table_partition_key(&self, table_name: &str) -> Result<String, PluginError> {
        let desc = self.describe_table(table_name).await?;
        desc.columns
            .iter()
            .find(|c| c.is_pk)
            .map(|c| c.name.clone())
            .ok_or_else(|| {
                PluginError::internal(format!(
                    "could not determine partition key for {table_name}"
                ))
            })
    }

    /// Insert a full item via the native PutItem API (no 8KB statement limit,
    /// supports nested maps/lists and binary values).
    pub async fn put_item(
        &self,
        table_name: &str,
        item: std::collections::HashMap<String, aws_sdk_dynamodb::types::AttributeValue>,
        condition_expression: Option<String>,
    ) -> Result<(), PluginError> {
        let mut request = self
            .inner
            .put_item()
            .table_name(table_name)
            .set_item(Some(item));
        if let Some(cond) = condition_expression {
            request = request.condition_expression(cond);
        }
        request.send().await.map_err(|e| {
            let svc_err = e.into_service_error();
            if svc_err.is_conditional_check_failed_exception() {
                PluginError::internal(
                    "ConditionalCheckFailed: the item already exists (insert condition not met)"
                        .to_string(),
                )
            } else {
                PluginError::internal(format!("PutItem failed: {svc_err}"))
            }
        })?;
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

    /// Full table Scan via the native Scan API (supports limit + pagination).
    ///
    /// A hard cap (`MAX_SCAN_ITEMS`) bounds items per request to prevent
    /// runaway full-table scans on large tables (#8). Callers page past it
    /// with the returned `next_token`.
    pub async fn scan(
        &self,
        table_name: &str,
        limit: Option<i32>,
        exclusive_start_key: Option<
            std::collections::HashMap<String, aws_sdk_dynamodb::types::AttributeValue>,
        >,
    ) -> Result<crate::dynamodb::models::ItemOutput, PluginError> {
        // Enforce the safety cap: use the caller's limit if smaller, otherwise
        // default to MAX_SCAN_ITEMS (#8).
        let effective_limit = limit.map_or(Self::MAX_SCAN_ITEMS, |l| l.min(Self::MAX_SCAN_ITEMS));

        let mut req = self
            .inner
            .scan()
            .table_name(table_name)
            .limit(effective_limit)
            .return_consumed_capacity(aws_sdk_dynamodb::types::ReturnConsumedCapacity::Total);
        if let Some(k) = exclusive_start_key {
            req = req.set_exclusive_start_key(Some(k));
        }
        let resp = req
            .send()
            .await
            .map_err(|e| PluginError::internal(format!("Scan failed: {e}")))?;
        let consumed_capacity = resp
            .consumed_capacity()
            .map(super::models::consumed_capacity_units);
        Ok(crate::dynamodb::models::ItemOutput::new(
            resp.items(),
            resp.last_evaluated_key(),
            consumed_capacity,
        ))
    }

    /// Query a single partition key value via the native Query API.
    /// The HASH key (and optional equality sort-key condition) come from `args`.
    pub async fn query(
        &self,
        args: QueryArgs,
    ) -> Result<crate::dynamodb::models::ItemOutput, PluginError> {
        let mut names = std::collections::HashMap::new();
        let mut values = std::collections::HashMap::new();
        names.insert("#pk".to_string(), args.pk_name);
        values.insert(":pkv".to_string(), args.pk_val);

        let key_condition = if let (Some(sk_name), Some(sk_val)) = (args.sk_name, args.sk_val) {
            names.insert("#sk".to_string(), sk_name);
            values.insert(":skv".to_string(), sk_val);
            "#pk = :pkv AND #sk = :skv".to_string()
        } else {
            "#pk = :pkv".to_string()
        };

        let mut req = self
            .inner
            .query()
            .table_name(&args.table_name)
            .key_condition_expression(key_condition)
            .return_consumed_capacity(aws_sdk_dynamodb::types::ReturnConsumedCapacity::Total)
            .set_expression_attribute_names(Some(names))
            .set_expression_attribute_values(Some(values));
        if let Some(l) = args.limit {
            req = req.limit(l);
        }
        if let Some(k) = args.exclusive_start_key {
            req = req.set_exclusive_start_key(Some(k));
        }
        let resp = req
            .send()
            .await
            .map_err(|e| PluginError::internal(format!("Query failed: {e}")))?;
        let consumed_capacity = resp
            .consumed_capacity()
            .map(super::models::consumed_capacity_units);
        Ok(crate::dynamodb::models::ItemOutput::new(
            resp.items(),
            resp.last_evaluated_key(),
            consumed_capacity,
        ))
    }

    /// Fetch a single item by full key via the native GetItem API.
    pub async fn get_item(
        &self,
        table_name: &str,
        key: std::collections::HashMap<String, aws_sdk_dynamodb::types::AttributeValue>,
    ) -> Result<crate::dynamodb::models::ItemOutput, PluginError> {
        let resp = self
            .inner
            .get_item()
            .table_name(table_name)
            .return_consumed_capacity(aws_sdk_dynamodb::types::ReturnConsumedCapacity::Total)
            .set_key(Some(key))
            .send()
            .await
            .map_err(|e| PluginError::internal(format!("GetItem failed: {e}")))?;
        let items = match resp.item() {
            Some(item) => vec![item.clone()],
            None => vec![],
        };
        let consumed_capacity = resp
            .consumed_capacity()
            .map(super::models::consumed_capacity_units);
        Ok(crate::dynamodb::models::ItemOutput::new(
            &items,
            None,
            consumed_capacity,
        ))
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
