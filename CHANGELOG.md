# Changelog

## [0.1.8] — 2026-10-10

### Fixed

- **Profile connections honour the connection's own region, and an unknown
  profile name fails fast (#82).** Two defects in the profile path added with
  v0.1.7's extra fields. First, the profile exemption in `normalized_params`
  skipped the *entire* region chain, including the region parsed from an AWS
  endpoint hostname, so a connection pointing at e.g.
  `dynamodb.us-west-2.amazonaws.com` whose profile config named another region
  was signed with the wrong one and failed with
  `InvalidSignatureException: Credential should be scoped to a valid region`.
  Connection-level choices — explicit `extra["region"]` and the
  endpoint-hostname region — now apply with or without a profile; only the
  plugin-setting and `us-east-1` fallbacks stay out of a profile connection's
  way, so the profile config still decides when the connection says nothing.
  Second, a typo'd profile surfaced as an opaque `DynamoDB ping failed:
  dispatch failure`; the name is now checked against the shared credentials and
  config files and rejected with `AWS profile '<name>' not found in
  ~/.aws/credentials or ~/.aws/config`. Both files are resolved the way the SDK
  resolves them (`AWS_SHARED_CREDENTIALS_FILE` / `AWS_CONFIG_FILE`, `[name]` in
  credentials vs `[profile name]` in config, and `HOME` → Windows
  `USERPROFILE` / `HOMEDRIVE` + `HOMEPATH` — a GUI-launched plugin has no
  `HOME`).
- **The endpoint URL the AWS SDK resolves now reaches the DynamoDB service
  config (#77).** `pool::get_config` built the service config with
  `Config::builder()` and copied across only the region and the credentials
  provider, dropping the endpoint URL that `aws_config::defaults().load()`
  resolves from `AWS_ENDPOINT_URL` / `AWS_ENDPOINT_URL_DYNAMODB`. A connection
  relying on the default chain (HOST/PORT empty) could therefore not be pointed
  at DynamoDB Local — the request silently went to real AWS. The service config
  is now derived from the loaded `SdkConfig` (endpoint, retry policy, timeouts,
  HTTP client, identity cache); an explicit `endpoint` param is applied last, so
  HOST/PORT still wins over the environment.
- **Native-mode request bodies fail closed on unknown fields (#89).** `#!scan`,
  `#!query` and `#!get` parse a YAML mapping, and `parse_native_body` picked the
  known keys out of it while silently ignoring everything else, so a body
  written against the wire API could return every row (`FilterExpression`
  dropped), fail confusingly (`KeyConditionExpression`), or match nothing (`Key`
  as a wire-API mapping). Unknown fields are now rejected with `-32602`,
  naming the offending field(s) (deterministically sorted) and the supported
  set. `FilterExpression` / `KeyConditionExpression` support is deliberately
  not implemented — that is an expression-parsing feature, not a validation
  fix.

### Changed

- **The README and `.tabularium` now state the region chain the plugin actually
  has (#81).** An ambient `AWS_REGION` / `AWS_DEFAULT_REGION` does not
  participate in the chain for connections the plugin signs itself. Profile
  connections are the exception: they inherit the ambient region, because the
  SDK's own chain puts the environment ahead of the profile's `region` — the
  same precedence the AWS CLI has. Both statements are backed by SigV4
  capture-proxy measurements (the signed request is the only place the region is
  observable) and are written down instead of being implied.
- **`dev-install` / `uninstall` target the plugin dirs current Tabularis reads
  (#78).** The `[windows]` and `[macos]` recipes copied into the pre-#258
  project dirs, while Tabularis ≥ 0.24.0 reads the unified `tabularis` data dir
  and migrates the old tree away on first start — the build landed where the app
  never looks, with no error and no warning. They now install into
  `%APPDATA%\tabularis\plugins\<driver>` /
  `~/Library/Application Support/tabularis/plugins/<driver>` /
  `~/.local/share/tabularis/plugins/<driver>`, falling back to the legacy trees
  only when those are the ones that exist. The Windows `uninstall` recipe also
  gained the `#!pwsh` shebang it was missing: without it the body ran one line
  per shell, `$dest` was unset by the time `Test-Path` ran, and the recipe
  errored while deleting nothing.
- The README documents the development loop and the registry release flow
  (#83): how to build, install, seed fixtures and smoke-test the plugin, and how
  a tag becomes a registry entry.
- Dependency updates: the aws-sdk group (#94), `aws-sdk-dynamodb` 1.126.0 →
  1.127.0 (#95), `tokio` 1.53.1 → 1.53.2 (#96).

### Tests

- **New `tests/plugin_harness.py` (#79).** The seven Python suites hard-coded
  `../target/release/dynamodb-plugin.exe`, so no debug build, branch build or
  non-Windows host could run them, and they assumed a composite-key `test_users`
  fixture that nothing created — a fresh DynamoDB Local produced a wall of
  `ResourceNotFoundException`, and `test_issue_8.py` dropped the shared fixture
  on its way through, failing every suite after it in the same batch. The
  harness resolves the binary from `PLUGIN_BINARY`, exposes the shared
  connection params and the `test_users` definition, and creates and seeds
  `test_users` / `edge_cases` idempotently; `just seed-fixtures` runs it on
  demand.
- **The stale expectations are gone (#80, #90, #91).** `test_deep.py` sent
  PartiQL statements to the native modes, which parse YAML, and asserted the
  wrong shapes throughout; `test_issue_8.py` asserted a `DROP TABLE` guard that
  was deliberately removed in 2026-07. Both now pin the behaviour that exists —
  real mappings with row-count assertions, and the standard
  `ExecuteQueryResponse` envelope from a DROP against the suite's own throwaway
  table. The two `note_bug` texts narrating behaviour that no longer exists were
  replaced by assertions on the current contract, so a regression fails the
  suite instead of being reported as a bug. All seven suites exit 0 on a fresh
  DynamoDB Local (a second run is identical and leaves no throwaway tables
  behind).

## [0.1.7] — 2026-09-21

### Added

- Plugin-owned connection fields, contributed through the host's
  `connection-modal.extra_fields` slot (`tabularis#596`, requires Tabularis
  ≥ 0.23.0): an **AWS region** select (the 34 regions from the plugin's region
  setting, plus `Default (plugin setting)`), an optional **AWS profile**, and an
  optional **Session token** for temporary (STS) credentials. Every value is
  written to the connection's opaque `extra` map, which the host persists and
  forwards untouched; the plugin promotes `extra["profile"]` and
  `extra["session_token"]` onto the SDK parameters and consumes
  `extra["region"]` in the region precedence chain described below. The `ui/`
  bundle is built to `ui/dist/index.js` and ships inside each release archive.
- The generic USERNAME/PASSWORD fields keep their labels — the manifest has no
  per-field label override — so the slot carries an inline hint mapping them to
  Access Key ID / Secret Access Key and noting both may be left empty when a
  profile is used.

### Fixed

- Credentials-only connections no longer fail validation (#71). A connection
  supplying an access-key/secret pair but no HOST/PORT was rejected with
  `connection params required`, because the region fallback chain only ran when
  an endpoint was present while `build_client` requires
  `region + access_key_id + secret_access_key`. The chain now runs whenever the
  connection has something to sign with — an endpoint **or** a complete
  credential pair — so the request is signed with a resolved region and the AWS
  SDK derives the endpoint, which is what makes the HOST field optional.
- Blank (`""`) and `null` values for `endpoint`, `region` and `profile` are now
  treated as "not supplied" instead of shadowing a fallback. A present-but-blank
  key — which is what the GUI sends for untouched fields — previously suppressed
  region defaulting entirely, and also made the plugin-level **Default AWS
  region** setting a no-op for those connections.

### Notes

- Behaviour change: for a credentials-only connection with no region, the
  documented chain decides the signing region (explicit `region` →
  `extra["region"]` → endpoint hostname → plugin setting → `us-east-1`), and an
  ambient `AWS_DEFAULT_REGION`/`AWS_REGION` no longer leaks in. Verified against
  a SigV4 capture proxy: before this release such a connection signed
  `eu-north-1` in an environment that set `AWS_DEFAULT_REGION=eu-north-1`, and
  now signs `us-east-1`.
- Connections that already supplied an endpoint behave identically; the #29
  validation guard is unchanged (a connection with neither an endpoint nor
  credentials is still rejected).

## [0.1.6] — 2026-08-24

### Fixed

- `ORDER BY` clauses in PartiQL `SELECT` statements no longer fail with
  `ValidationException: Must have WHERE clause in the statement when using
  ORDER BY clause.` DynamoDB's `ExecuteStatement` only accepts `ORDER BY`
  when a `WHERE` pins the partition key and the ordered column is the sort
  key, so any other form (e.g. sorting a table without a sort key, or
  ordering by a non-key column — exactly what the GUI emits when a column
  header is clicked) was rejected outright. The clause is now detected,
  stripped from the statement, and re-applied client-side over a bounded
  paged read of the result set (`ORDER BY ... LIMIT n` chains correctly:
  `LIMIT` is stripped first, then `ORDER BY`). A 1000-row cap keeps the
  client-side sort bounded; when the cap is hit a warning is returned
  explaining how to get server-side ordering instead.
- Filtered browses with a LIMIT no longer fail with `ValidationException:
  Statement wasn't well formed, can't be processed: Expected RIGHT_PAREN`.
  The GUI wraps such browses in a derived table —
  `SELECT * FROM (<base> <where> <order_by> <limit>) AS limited_subset` —
  which DynamoDB PartiQL does not support. The wrapper is now unwrapped
  before the LIMIT/ORDER BY strippers run, so they operate on the inner
  statement instead of slicing into the subquery and dropping its closing
  paren. A wrapper is only unwrapped when it is a single derived table with
  nothing else at the top level (a following JOIN or outer WHERE is left
  untouched).

## [0.1.5] — 2026-08-07

### Added

- Official Amazon DynamoDB brand icon (SVG) added to the plugin repository
  and referenced via the `icon` field in `.tabularium`, following the same
  pattern as the DuckDB plugin.

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
