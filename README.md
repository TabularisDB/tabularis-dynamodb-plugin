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
- [Releasing](#releasing)
- [Changelog](#changelog)
- [License](#license)

## Features

- **Connection** — Connect using explicit AWS credentials (Access Key/Secret Key), an AWS region selector, AWS profiles, environment variables, or IAM roles. Supports AWS regions and custom endpoints for DynamoDB Local. See [Connection Configuration](#connection-configuration) for which value goes in which field.
- **Table Browsing** — List all tables in the connected region and inspect their schemas.
- **Schema Inspection** — View table attribute definitions, key schema (partition key and sort key), and data types.
- **Index Inspection** — List global secondary indexes (GSI) and local secondary indexes (LSI) for each table.
- **Query Execution** — Run PartiQL queries using four modes: `#!partiql`, `#!scan`, `#!query`, and `#!get`.
- **Inline Editing** — Insert, update, and delete items directly from the Tabularis data grid.
- **DDL-equivalent Generation** — Generates PartiQL statements for `CREATE TABLE`, `ADD COLUMN`, and `CREATE INDEX`.
- **Cross-platform** — Pre-built binaries for Linux (x86_64), macOS (aarch64), and Windows (x86_64).
- **DynamoDB Local Support** — Full compatibility with DynamoDB Local for offline development and testing.

## Connection Configuration

Tabularis renders its generic connection form (HOST / PORT / USERNAME / PASSWORD) for this plugin, and the plugin maps those fields onto AWS ones. There is no AWS-specific field in the host form yet, so this is what to type:

| Tabularis field | Value for AWS | Value for DynamoDB Local |
|---|---|---|
| **HOST** | `dynamodb.<region>.amazonaws.com`, e.g. `dynamodb.us-east-1.amazonaws.com` | `localhost` |
| **PORT** | `443` | `8000` |
| **USERNAME** | your AWS Access Key ID | any non-empty string |
| **PASSWORD** | your AWS Secret Access Key | any non-empty string |

Host and port are only used to derive the endpoint: the region is parsed out of the hostname, port `443` (or any `*.amazonaws.com` host) switches it to `https://`, and any other host is treated as a plain `http://` endpoint. HOST and PORT may be left empty when an access key and secret are supplied — the request is then signed against the default endpoint for the resolved region.

### Region resolution

The signing region is resolved in this order (first match wins):

| # | Source | How to set it |
|---|---|---|
| 1 | explicit `region` parameter | raw JSON-RPC / `test_plugin` REPL |
| 2 | per-connection region | region selector in the connection modal (stored in the connection's `extra` map) |
| 3 | endpoint hostname | `dynamodb.us-west-2.amazonaws.com` → `us-west-2` |
| 4 | plugin default region | **Settings → Plugins → DynamoDB → Default AWS region** |
| 5 | fallback | `us-east-1` |

SigV4 signing requires a region and AWS rejects a request whose signing region does not match the endpoint's region (`InvalidSignatureException`), so let the region be derived from the hostname, or set it explicitly. Profile-based connections are exempt: they take their region from `~/.aws/config`.

### Authentication methods

Credentials are resolved in the following order:

1. **Explicit credentials** — the Access Key ID / Secret Access Key entered in USERNAME / PASSWORD (mapped to `access_key_id` / `secret_access_key`).
2. **Session token** — `session_token` for temporary credentials (e.g. from AWS STS).
3. **AWS profile** — `profile`, pointing at a named profile in `~/.aws/credentials`.
4. **Environment variables** — the plugin reads `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and `AWS_REGION` from the environment.
5. **IMDS** — automatically uses EC2 instance metadata or ECS task roles when running on AWS infrastructure.

For DynamoDB Local, point HOST/PORT at it (`localhost` / `8000`) and give USERNAME/PASSWORD any non-empty value — the plugin passes the endpoint through as-is instead of signing against real AWS:

```bash
docker run -d --name dynamodb-local -p 8000:8000 amazon/dynamodb-local -jar DynamoDBLocal.jar -sharedDb
```

### Raw JSON-RPC parameters

These are the internal parameter names the plugin reads. They are what the fields above map onto, and are only ever typed directly when driving the plugin over JSON-RPC (see [Manual JSON-RPC test via shell](#manual-json-rpc-test-via-shell)).

| Parameter | Description | Required |
|-----------|-------------|----------|
| `region` | AWS region (e.g., `us-east-1`, `ap-southeast-2`) | Yes |
| `access_key_id` | AWS access key ID | If not using profile/env |
| `secret_access_key` | AWS secret access key | If not using profile/env |
| `session_token` | Temporary session token (for STS) | No |
| `profile` | AWS profile name | No |
| `endpoint` | Custom endpoint URL (for DynamoDB Local) | No |

`region` can also be supplied through the connection-level `extra` map (`{"extra": {"region": "us-west-2"}}`) — that is where the connection modal's region selector stores it. Profile-based connections are exempt from the region fallbacks above.

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
3. Copy `dynamodb-plugin` (or `dynamodb-plugin.exe` on Windows) and `.tabularium` into the Tabularis plugins directory:

| OS | Plugins Directory |
|---|---|
| **Linux** | `~/.local/share/tabularis/plugins/dynamodb/` |
| **macOS** | `~/Library/Application Support/tabularis/plugins/dynamodb/` |
| **Windows** | `%APPDATA%\tabularis\plugins\dynamodb\` |

4. Restart Tabularis.

Since core PR [#258](https://github.com/TabularisDB/tabularis/pull/258) the plugin folder lives under the unified `tabularis` directory on Windows and macOS; older builds read it from the legacy `com.debba.tabularis` location (on Windows `%APPDATA%\debba\tabularis\data\plugins\dynamodb\`), and the app moves plugins out of there into the new folder on first startup.

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

The binary will be located at `target/release/dynamodb-plugin`.

### Install Locally

Use the provided justfile recipes:

```bash
# Install to Tabularis plugins directory
just dev-install

# Or manually copy
cp target/release/dynamodb-plugin ~/.local/share/tabularis/plugins/dynamodb/
cp .tabularium ~/.local/share/tabularis/plugins/dynamodb/
```

## Development

Working on the plugin needs Rust (edition 2021, toolchain pinned by `rust-toolchain.toml`), Docker for DynamoDB Local, and Node.js only if you touch `ui/`.

### Common just recipes

The `justfile` wraps the same commands CI runs on every PR (the `Test` job plus the `UI extension` job), so a local green run is the same check CI performs:

| Recipe | What it does |
|--------|--------------|
| `just run-dynamodb` | Start DynamoDB Local in Docker |
| `just seed-dynamodb` | Create and seed the `users` and `orders` tables |
| `just seed-fixtures` | Create and seed the Python suites' fixtures (`test_users` with its composite key, plus `edge_cases`) |
| `just build` | Debug build; builds the `ui/` bundle first when `ui/package.json` exists |
| `just test` | `cargo test` |
| `just test-integration` | Integration tests against DynamoDB Local (`DYNAMODB_ENDPOINT`, default `http://localhost:8000`) |
| `just test-ui` | `npm run typecheck` plus the UI bundle tests |
| `just lint` | `cargo clippy --all-targets -- -D warnings` |
| `just fmt` | `cargo fmt --all` |
| `just repl` | Launch the RPC REPL (`cargo run --bin test_plugin`) |
| `just dev-install` | Build, then copy the binary, `.tabularium` and the UI bundle into the plugin folder |

The integration suite is gated on `DYNAMODB_ENDPOINT`: `just test-integration` defaults it to `http://localhost:8000`, so start and seed DynamoDB Local first (`just run-dynamodb`, `just seed-dynamodb`). With the variable unset the whole suite skips — which is how CI stays green without Docker.

**Installing a local build:** `just dev-install` writes into the plugin folder from [Manual Installation](#manual-installation) — the unified `tabularis` directory on Windows and macOS. On a machine where only the pre-#258 tree exists it installs there instead, so an older Tabularis that has not migrated yet still picks the build up. Close Tabularis first: it locks the running binary.

### Project layout

- `src/main.rs` — stdio transport and worker pool
- `src/rpc.rs` — JSON-RPC method dispatch
- `src/handlers/` — `connection`, `query`, `metadata`, `crud` and `ddl` handlers
- `src/dynamodb/` — AWS SDK client wrapper and the 30-minute config pool
- `src/utils/extractor.rs` — parameter extraction helpers
- `src/bin/test_plugin.rs` — the RPC REPL
- `ui/` — UI extension bundle contributing the connection-modal fields

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

#### Python suites (`tests/*.py`)

`tests/` also holds end-to-end suites that drive the built binary over JSON-RPC against DynamoDB Local. They cover RPC surface `cargo test` does not — the native `#!scan`/`#!query`/`#!get` modes, pagination, protocol abuse, the destructive-SQL guards.

```bash
# 1. DynamoDB Local, plus the fixtures the suites assume
just run-dynamodb
just seed-fixtures

# 2. A build to drive, then run a suite from tests/
cargo build --release
cd tests && python test_local.py        # also test_deep.py, test_group_a.py, ...
```

| Env var | Default | What it does |
|---|---|---|
| `PLUGIN_BINARY` | `target/release/dynamodb-plugin(.exe)` | Binary under test. Point it at a branch or debug build to exercise it without overwriting `target/release` |
| `DYNAMODB_ENDPOINT` | `http://localhost:8000` | Where DynamoDB Local listens |

Each suite provisions what it needs through `tests/plugin_harness.py` before it runs — it creates and seeds the composite-key `test_users` table (and `edge_cases` where used) rather than expecting a previous run to have left it behind, and it restores `test_users` after its own destructive checks. `just seed-fixtures` runs the same helper on demand.

### Manual JSON-RPC test via shell

```bash
echo '{"jsonrpc":"2.0","method":"test_connection","params":{"params":{"region":"us-east-1","access_key_id":"fake","secret_access_key":"fake","endpoint":"http://localhost:8000"}},"id":1}' \
  | ./target/release/dynamodb-plugin
```

### Tech Stack

- **Language:** Rust (edition 2021)
- **Database driver:** [aws-sdk-dynamodb](https://crates.io/crates/aws-sdk-dynamodb) (official AWS SDK for Rust)
- **Serialization:** serde + serde_json
- **Async runtime:** tokio
- **Protocol:** JSON-RPC 2.0 over stdio

## Releasing

Releases are cut from a version tag: CI builds the archives, and the hosted registry ingests the result on its own. Nothing else needs to be updated.

1. Bump `version` in **both** `Cargo.toml` and `.tabularium` to the same value — the tag and the manifest `version` must agree, and the registry rejects an ingest whose tag (minus the `v`) disagrees with the manifest.
2. Move the unreleased entries in `CHANGELOG.md` into a new `## [X.Y.Z] — YYYY-MM-DD` section at the top.
3. Commit (`chore(release): bump version to vX.Y.Z`) and push to `main`.
4. Tag that commit and push the tag: `git tag vX.Y.Z && git push origin main --tags`.
5. `.github/workflows/release.yml` runs on the tag. It builds five targets — `linux-x64`, `linux-arm64`, `darwin-x64`, `darwin-arm64`, `win-x64` — stages each binary with `.tabularium` (plus `ui/dist/index.js` when the UI extension is present), publishes a GitHub Release with the five `dynamodb-plugin-<platform>.zip` archives, and attaches `.tabularium` as a standalone asset under the name `default.tabularium`. That standalone copy is the manifest the registry reads.

Practically, the `.tabularium` version bump is what marks a release — the tag only triggers the build.

### Distribution: the hosted registry

The plugin is published through [registry.tabularis.dev/plugins/dynamodb](https://registry.tabularis.dev/plugins/dynamodb), which reads the manifest from the release assets and refreshes its version table automatically when a new tag is published. The plugin is already submitted and approved there, so a release needs **no** manual re-submission — v0.1.6 and v0.1.7 both appeared on that page without one.

Bumping the plugin's entry in the core repo's `plugins/registry.json` is the legacy path, kept only for plugins that were never added to the hosted registry — see the maintainer's note on [tabularis#796](https://github.com/TabularisDB/tabularis/pull/796#issuecomment-5756546338). For this plugin that PR is unnecessary.

The registry renders this README on the plugin page, so README edits reach users with the next release.

### Post-release checks

- The GitHub Release lists all five zips plus `default.tabularium`.
- `curl -s https://registry.tabularis.dev/api/plugins/dynamodb | jq '.latestVersion'` reports the new version, with the release and its per-platform assets under `releases`.
- Installing or updating the plugin from inside Tabularis pulls the new build.

## [Changelog](./CHANGELOG.md)

## Maintainers

* @fuleinist

## License

Apache-2.0.
