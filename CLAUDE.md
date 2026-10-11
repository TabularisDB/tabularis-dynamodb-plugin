# CLAUDE.md

## Build & Test

```bash
cargo build              # Debug build
cargo build --release    # Release build
cargo test               # Run all tests
cargo clippy --all-targets -- -D warnings  # Lint
cargo fmt --all          # Format
```

## Project Structure

```text
src/
├── main.rs               # Stdio transport, worker pool
├── lib.rs                # Public module exports
├── rpc.rs                # JSON-RPC dispatch
├── models.rs             # ConnectionParams
├── error.rs              # PluginError, ErrorCode
├── dynamodb/
│   ├── mod.rs
│   ├── client.rs         # DynamoDB client wrapper
│   ├── pool.rs           # Config caching pool
│   └── models.rs         # Internal data types
├── handlers/
│   ├── mod.rs
│   ├── models.rs         # Query, ExecuteQueryResponse, ColumnResponse
│   ├── query.rs          # test_connection, ping, execute_query
│   ├── metadata.rs       # get_tables, get_columns, get_indexes
│   ├── crud.rs           # insert_record, update_record, delete_record
│   └── ddl.rs            # Schema change SQL generation
├── utils/
│   ├── mod.rs
│   └── extractor.rs      # Parameter extraction
└── bin/
    └── test_plugin.rs    # Local REPL
```

## Architecture Rules

- All RPC handlers are async and return `serde_json::Value`
- Handlers live in `handlers/`, organized by domain
- DynamoDB SDK calls go through `dynamodb::client::Client`
- Config is cached in `dynamodb::pool::CONFIG_POOL` (30-min TTL)
- AWS SDK uses `behavior-version-latest` feature
- Tests use real types, not mocks (integration tests need DynamoDB Local)

## Key Patterns

- **Adding a new RPC method**: Add to `rpc.rs` dispatch, create handler in `handlers/`
- **Query modes**: Shebang-prefixed (`#!partiql`, `#!scan`, `#!query`, `#!get`)
- **Connection params**: Extracted via `utils::extractor` helpers
- **Error handling**: Use `PluginError` with JSON-RPC error codes
- **Testing**: `cargo test` runs all unit tests; integration tests need DynamoDB Local

## Development

```bash
# Start DynamoDB Local
docker run -d --name dynamodb-local -p 8000:8000 amazon/dynamodb-local

# Seed test data
just seed-dynamodb

# Run REPL
cargo run --bin test_plugin

# Install to Tabularis
just dev-install
```
