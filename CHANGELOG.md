# Changelog

## [0.1.4] — 2026-08-06

### Fixed

- `normalized_params` built `http://host:443` for AWS endpoints, which fail at
  the transport level (DynamoDB endpoints only speak TLS). HTTPS is now used
  when the port is 443 or the host ends with `.amazonaws.com`.
- `normalized_params` defaulted the signing region to `us-east-1` even when
  the endpoint was another region's AWS endpoint, so every request failed
  with `InvalidSignatureException`. The signing region is now resolved in
  order: explicit `region` param, `extra["region"]` connection field, region
  parsed from an AWS endpoint hostname
  (`dynamodb.us-west-2.amazonaws.com` → `us-west-2`), the plugin-level
  default-region setting, and only then `us-east-1`. Together these restore
  connecting to real AWS DynamoDB via the generic GUI connection form
  (host/port/username/password).

### Added

- Plugin-level **Default AWS region** setting (Settings → Plugins →
  DynamoDB), declared in `.tabularium` and delivered via the `initialize`
  RPC. Used when a connection supplies neither an explicit region nor an
  AWS endpoint hostname to parse one from.
- `ConnectionParams` now parses the opaque `extra: HashMap<String, String>`
  connection fields the host persists and forwards to drivers unchanged.
  `extra["region"]` acts as the per-connection signing region when no
  explicit `region` param is present — once the host ships the generic
  connection-UI support ([TabularisDB/tabularis#596](https://github.com/TabularisDB/tabularis/pull/596)),
  a region selector lives entirely in this plugin instead of the core app.

## [0.1.3] — 2026-08-04

### Fixed

- `execute_query` responses now include a complete `pagination` object with
  the `page`, `page_size`, `total_rows` and `has_more` fields the Tabularis
  app's `Pagination` struct requires. Previously only `next_token` was sent,
  and the app rejected every query response with "missing field `page`".
  The page number is read from the request's `page` param (default 1) and
  `page_size` from `limit` (falling back to the returned row count).
- `get_tables` no longer issues DescribeTable calls serially. On AWS accounts
  with hundreds of tables the serial loop took over a minute (~300ms per
  table), exceeding the GUI's connection timeout and failing the initial
  connection. Describes now run with bounded concurrency (16 in flight) and
  results are re-sorted alphabetically to preserve ListTables ordering.

## [0.1.2] — 2026-08-03

### Changed

- Bumped `base64` from 0.22.1 to 0.23.0.

### Fixed

- `manifest.json` updated to satisfy the Tabularium driver-kind contract.

## [0.1.1] — 2026-08-02

### Added

- Migrated to the `.tabularium` manifest for the Tabularium registry.

### Fixed

- `get_indexes` now returns one row per indexed column to match the GUI
  contract.

## [0.1.0] — 2026-07-22

### Added

- Initial plugin scaffold with Rust project structure
- JSON-RPC 2.0 over stdio transport with async worker pool (4 workers, bounded queue)
- AWS DynamoDB client wrapper with connection pool caching (30-min TTL)
- AWS credential resolution: explicit keys, profile, environment variables, IMDS
- Endpoint override for DynamoDB Local testing
- RPC method dispatch: `initialize`, `ping`, `test_connection`
- Metadata handlers: `get_tables`, `get_columns`, `get_indexes`, `get_foreign_keys`
- Query execution via PartiQL (`execute_query`) with 4 query modes: `#!partiql`, `#!scan`, `#!query`, `#!get`
- CRUD handlers: `insert_record`, `update_record`, `delete_record` (via PartiQL)
- DDL handlers: `get_create_table_sql`, `get_add_column_sql`, `get_alter_column_sql`, `get_create_index_sql`, `drop_index`
- `manifest.json` with DynamoDB-specific data types and capabilities
- Local REPL (`cargo run --bin test_plugin`) for testing RPC handlers
- 78 unit tests covering all modules
- GitHub Actions release workflow (cross-platform builds)
- `justfile` with development recipes

### Changed

- Rename plugin binary and release assets from `tabularis-dynamodb-plugin` to
  `dynamodb-plugin` to match the org-wide plugin naming convention (e.g.
  `elasticsearch-plugin`). `Cargo.toml` package/`[[bin]]` name, `manifest.json`
  `executable`, and the release archive names are updated accordingly.
- `manifest.json` now references the plugin manifest `$schema` and uses the
  standard capability key set (`folder_based`, `no_connection_required`,
  `alter_column`, `create_foreign_keys`).
- Release workflow gains a forward-compatible UI-extension build step (no-op
  until a `ui/` folder is added) and ships `dynamodb-plugin-<platform>.zip`
  artifacts.
- `justfile` `dev-install`/`uninstall` are split per OS (linux/macos/windows),
  fixing the macOS plugin directory path, and `build`/`release` now chain a
  `build-ui` passthrough.

### Added

- `LICENSE` file (Apache-2.0, matching the license already declared in
  `Cargo.toml`).
- `CODEOWNERS`, `.editorconfig`, and Dependabot config (cargo + GitHub Actions,
  weekly).
- Expanded `.gitignore` with IDE and build directories.
