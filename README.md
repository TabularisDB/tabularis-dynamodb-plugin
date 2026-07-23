<div align="center">
  <img src="https://raw.githubusercontent.com/debba/tabularis/main/public/logo-sm.png" width="120" height="120" />
</div>

# tabularis-dynamodb-plugin

<p align="center">

![](https://img.shields.io/github/release/tabularisDB/tabularis-dynamodb-plugin.svg?style=flat)
![](https://img.shields.io/github/downloads/tabularisDB/tabularis-dynamodb-plugin/total.svg?style=flat)
![Build & Release](https://github.com/tabularisDB/tabularis-dynamodb-plugin/workflows/Release/badge.svg)
[![Discord](https://img.shields.io/discord/1502944695808950282?color=5865F2&logo=discord&logoColor=white)](https://discord.com/invite/K2hmhfHRSt)

</p>

A [DynamoDB](https://aws.amazon.com/dynamodb/) plugin for [Tabularis](https://github.com/TabularisDB/tabularis), the lightweight database management tool.

This plugin enables Tabularis to connect to AWS DynamoDB and DynamoDB Local instances, providing table browsing, schema inspection, PartiQL query execution, and full CRUD operations through a JSON-RPC 2.0 over stdio interface.

**Discord** - [Join our discord server](https://discord.com/invite/K2hmhfHRSt) and chat with the maintainers.

## Table of Contents

- [Features](#features)
- [Connection Configuration](#connection-configuration)
- [Supported DynamoDB Data Types](#supported-dynamodb-data-types)
- [Installation](#installation)
  - [Automatic (via Tabularis)](#automatic-via-tabularis)
  - [Manual Installation](#manual-installation)
- [How It Works](#how-it-works)
- [Query Syntax](#query-syntax)
- [Supported Operations](#supported-operations)
- [Building from Source](#building-from-source)
- [Development](#development)
- [Changelog](#changelog)
- [License](#license)

## Features

- **Connection** — Connect using explicit AWS credentials (Access Key/Secret Key), AWS profiles, environment variables, or IAM roles. Supports AWS regions and custom endpoints for DynamoDB Local.
- **Table Browsing** — List all tables in the connected region and inspect their schemas.
- **Schema Inspection** — View table attribute definitions, key schema (partition key and sort key), and data types.
- **Index Inspection** — List global secondary indexes (GSI) and local secondary indexes (LSI) for each table.
- **Query Execution** — Run PartiQL queries using four modes: `#!partiql`, `#!scan`, `#!query`, and `#!get`.
- **Inline Editing** — Insert, update, and delete items directly from the Tabularis data grid.
- **DDL-equivalent Generation** — Generates PartiQL statements for `CREATE TABLE`, `ADD COLUMN`, and `CREATE INDEX`.
- **Cross-platform** — Pre-built binaries for Linux (x86_64, aarch64), macOS (x86_64, aarch64), and Windows (x86_64).
- **DynamoDB Local Support** — Full compatibility with DynamoDB Local for offline development and testing.

## Connection Configuration

The plugin supports multiple authentication methods, resolved in the following order:

1. **Explicit credentials** — Pass `access_key_id`, `secret_access_key`, and `region` directly in the connection parameters.
2. **Session token** — Add `session_token` for temporary credentials (e.g., from AWS STS).
3. **AWS profile** — Specify `profile` to use a profile from `~/.aws/credentials`.
4. **Environment variables** — The plugin reads `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and `AWS_REGION` from the environment.
5. **IMDS** — Automatically uses EC2 instance metadata or ECS task roles when running on AWS infrastructure.

For DynamoDB Local, set `endpoint` to `http://localhost:8000` to override the default AWS endpoint.

### Connection Parameters

| Parameter | Description | Required |
|-----------|-------------|----------|
| `region` | AWS region (e.g., `us-east-1`, `ap-southeast-2`) | Yes |
| `access_key_id` | AWS access key ID | If not using profile/env |
| `secret_access_key` | AWS secret access key | If not using profile/env |
| `session_token` | Temporary session token (for STS) | No |
| `profile` | AWS profile name | No |
| `endpoint` | Custom endpoint URL (for DynamoDB Local) | No |

## Supported DynamoDB Data Types

| Category | Types |
|---|---|
| **Scalar** | STRING, NUMBER, BINARY, BOOLEAN, NULL |
| **Set** | STRING_SET, NUMBER_SET, BINARY_SET |
| **Structured** | LIST, MAP |

## Installation

### Automatic (via Tabularis)

If your version of Tabularis supports plugin management, the DynamoDB plugin can be installed directly from the application.

### Manual Installation

1. Download the latest release for your platform from the [Releases page](https://github.com/TabularisDB/tabularis-dynamodb-plugin/releases).
2. Extract the archive.
3. Copy `tabularis-dynamodb-plugin` (or `tabularis-dynamodb-plugin.exe` on Windows) and `manifest.json` into the Tabularis plugins directory:

| OS | Plugins Directory |
|---|---|
| **Linux** | `~/.local/share/tabularis/plugins/dynamodb/` |
| **macOS** | `~/Library/Application Support/com.debba.tabularis/plugins/dynamodb/` |
| **Windows** | `%APPDATA%\debba\tabularis\data\plugins\dynamodb\` |

4. Restart Tabularis.

## How It Works

The plugin is a standalone Rust binary that communicates with Tabularis through **JSON-RPC 2.0 over stdio**:

1. Tabularis spawns the plugin as a child process.
2. Requests are sent as newline-delimited JSON-RPC messages to the plugin's `stdin`.
3. The plugin connects to DynamoDB using the official AWS SDK for Rust and writes responses to `stdout`.

Connection state is pooled — AWS SDK configs are cached with a 30-minute TTL to avoid re-creating credentials for every request.

## Query Syntax

The `execute_query` method supports four query modes, determined by an optional shebang (`#!`) at the beginning of the query:

### PartiQL Mode (default)

```sql
SELECT * FROM users WHERE id = 'user123'
SELECT * FROM orders WHERE user_id = 'user123' AND order_date > '2024-01-01'
```

You can also explicitly specify the mode:

```sql
#!partiql
SELECT * FROM users WHERE id = 'user123'
```

### Scan Mode

```sql
#!scan
TableName: users
FilterExpression: age > :val
ExpressionAttributeValues: {":val": {"N": "25"}}
```

### Query Mode (requires key condition)

```sql
#!query
TableName: users
KeyConditionExpression: id = :id
ExpressionAttributeValues: {":id": {"S": "user123"}}
```

### Get Mode

```sql
#!get
TableName: users
Key: {"id": {"S": "user123"}}
```

## Supported Operations

| Method | Description |
|---|---|
| `test_connection` | Ping the DynamoDB service by listing tables (limit 1) |
| `get_tables` | List all tables in the connected region |
| `get_columns` | Get attribute definitions and key schema for a table |
| `get_foreign_keys` | Returns `[]` (DynamoDB has no foreign key constraints) |
| `get_indexes` | List global secondary indexes (GSI) and local secondary indexes (LSI) for a table |
| `execute_query` | Run PartiQL queries with four modes (partiql, scan, query, get) |
| `insert_record` | Insert a new item (generates PartiQL INSERT statement) |
| `update_record` | Update a single attribute by primary key (generates PartiQL UPDATE statement) |
| `delete_record` | Delete an item by primary key (generates PartiQL DELETE statement) |
| `get_create_table_sql` | Generates PartiQL `CREATE TABLE` statement |
| `get_add_column_sql` | Generates PartiQL `ALTER TABLE ADD COLUMN` statement |
| `get_alter_column_sql` | Generates PartiQL `ALTER TABLE MODIFY` statement |
| `get_create_index_sql` | Generates PartiQL `CREATE INDEX` statement for GSI creation |
| `get_create_foreign_key_sql` | Returns a not-supported note |
| `drop_index` | Generates PartiQL `DROP INDEX` statement |
| `drop_foreign_key` | Returns a not-supported note |

## Building from Source

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (edition 2021)
- A running DynamoDB Local instance (for integration tests)

### Build

```bash
cargo build --release
```

The binary will be located at `target/release/tabularis-dynamodb-plugin`.

### Install Locally

Use the provided justfile recipes:

```bash
# Install to Tabularis plugins directory
just dev-install

# Or manually copy
cp target/release/tabularis-dynamodb-plugin ~/.local/share/tabularis/plugins/dynamodb/
cp manifest.json ~/.local/share/tabularis/plugins/dynamodb/
```

## Development

### Testing the Plugin

**Simulated Tabularis integration test** (interactive REPL for testing RPC handlers):

```bash
cargo run --bin test_plugin
```

The REPL provides shortcuts:

| Command | Description |
|---------|-------------|
| `:init` | Send initialize request |
| `:ping` | Send ping request |
| `:tables` | Send get_tables request |
| `:help` | Show available commands |
| `:exit` / `:q` | Exit the REPL |
| `<json>` | Send any raw JSON-RPC request |

### Setting Up DynamoDB Local

Start DynamoDB Local via Docker:

```bash
docker run -d --name dynamodb-local -p 8000:8000 amazon/dynamodb-local -jar DynamoDBLocal.jar -sharedDb
```

Seed test data:

```bash
just seed-dynamodb
```

### Running Tests

```bash
# Unit tests
cargo test

# Integration tests (requires DynamoDB Local)
cargo test --test integration_test
```

### Manual JSON-RPC test via shell

```bash
echo '{"jsonrpc":"2.0","method":"test_connection","params":{"params":{"region":"us-east-1","access_key_id":"fake","secret_access_key":"fake","endpoint":"http://localhost:8000"}},"id":1}' \
  | ./target/release/tabularis-dynamodb-plugin
```

### Tech Stack

- **Language:** Rust (edition 2021)
- **Database driver:** [aws-sdk-dynamodb](https://crates.io/crates/aws-sdk-dynamodb) (official AWS SDK for Rust)
- **Serialization:** serde + serde_json
- **Async runtime:** tokio
- **Protocol:** JSON-RPC 2.0 over stdio

## [Changelog](./CHANGELOG.md)

## License

Apache License 2.0
