# Changelog

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

### Fixed

- `get_tables` no longer issues DescribeTable calls serially. On AWS accounts
  with hundreds of tables the serial loop took over a minute (~300ms per
  table), exceeding the GUI's connection timeout and failing the initial
  connection. Describes now run with bounded concurrency (16 in flight) and
  results are re-sorted alphabetically to preserve ListTables ordering.
