set shell := ["bash", "-cu"]
set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

# Run DynamoDB Local via Docker
run-dynamodb:
	docker run -d --name dynamodb-local -p 8000:8000 amazon/dynamodb-local:latest -jar DynamoDBLocal.jar -sharedDb

# Seed test data into DynamoDB Local
seed-dynamodb:
	# Create test tables
	aws dynamodb create-table \
		--endpoint-url http://localhost:8000 \
		--table-name users \
		--attribute-definitions AttributeName=id,AttributeType=S \
		--key-schema AttributeName=id,KeyType=HASH \
		--billing-mode PAY_PER_REQUEST
	aws dynamodb create-table \
		--endpoint-url http://localhost:8000 \
		--table-name orders \
		--attribute-definitions AttributeName=id,AttributeType=S AttributeName=user_id,AttributeType=S \
		--key-schema AttributeName=id,KeyType=HASH AttributeName=user_id,KeyType=RANGE \
		--global-secondary-indexes IndexName=user_id-index,KeySchema=[{"AttributeName=user_id,KeyType=HASH"}],Projection={ProjectionType=ALL} \
		--billing-mode PAY_PER_REQUEST
	# Seed data
	aws dynamodb put-item --endpoint-url http://localhost:8000 --table-name users --item '{"id": {"S": "user1"}, "name": {"S": "Alice"}, "email": {"S": "alice@example.com"}, "age": {"N": "30"}}'
	aws dynamodb put-item --endpoint-url http://localhost:8000 --table-name users --item '{"id": {"S": "user2"}, "name": {"S": "Bob"}, "email": {"S": "bob@example.com"}, "age": {"N": "25"}}'
	aws dynamodb put-item --endpoint-url http://localhost:8000 --table-name orders --item '{"id": {"S": "order1"}, "user_id": {"S": "user1"}, "total": {"N": "99.99"}, "status": {"S": "shipped"}}'

# Create + seed the fixtures the Python suites assume: the composite-key
# `test_users` table and `edge_cases`. Idempotent, so it is safe to re-run after
# a suite has dropped or emptied a table (#79).
[unix]
seed-fixtures:
	python3 tests/plugin_harness.py

[windows]
seed-fixtures:
	python tests/plugin_harness.py

# Build the plugin binary in debug mode (plus UI if present)
build: build-ui
	cargo build

# Build for release (what the GitHub Actions workflow ships)
release: build-ui
	cargo build --release

# Run unit tests
test:
	cargo test

# Run tests with output
test-verbose:
	cargo test -- --nocapture

# Run integration tests against DynamoDB Local (requires `just run-dynamodb` +
# `just seed-dynamodb` first). Skipped automatically if DYNAMODB_ENDPOINT unset.
test-integration:
	DYNAMODB_ENDPOINT=${DYNAMODB_ENDPOINT:-http://localhost:8000} cargo test --test dynamodb_local_test -- --test-threads=1

# Launch the local REPL
repl:
	cargo run --bin test_plugin

# Run clippy
lint:
	cargo clippy --all-targets -- -D warnings

# Format code
fmt:
	cargo fmt --all

# Build the UI extension if present (no-op otherwise)
[unix]
build-ui:
	@if [ -f ui/package.json ]; then \
		echo "Building UI extension..."; \
		(cd ui && npm install --no-audit --no-fund && npm run build); \
	fi

[windows]
build-ui:
	#!pwsh

	if (Test-Path ui/package.json) {
		Write-Host "Building UI extension..."
		Push-Location ui
		try {
			npm install --no-audit --no-fund
			if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
			npm run build
			if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
		} finally {
			Pop-Location
		}
	}

# Typecheck + test the UI extension (builds the bundle first)
[unix]
test-ui:
	@if [ -f ui/package.json ]; then \
		echo "Testing UI extension..."; \
		(cd ui && npm install --no-audit --no-fund && npm run typecheck && npm test); \
	else \
		echo "No ui/package.json — nothing to test."; \
	fi

[windows]
test-ui:
	#!pwsh

	if (Test-Path ui/package.json) {
		Write-Host "Testing UI extension..."
		Push-Location ui
		try {
			npm install --no-audit --no-fund
			if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
			npm run typecheck
			if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
			npm test
			if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
		} finally {
			Pop-Location
		}
	} else {
		Write-Host "No ui/package.json — nothing to test."
	}

# Build + copy binary and manifest into the Tabularis plugin folder
[linux]
dev-install: build
	mkdir -p ~/.local/share/tabularis/plugins/dynamodb
	cp target/debug/dynamodb-plugin ~/.local/share/tabularis/plugins/dynamodb/
	cp .tabularium ~/.local/share/tabularis/plugins/dynamodb/
	@if [ -f ui/dist/index.js ]; then \
		mkdir -p ~/.local/share/tabularis/plugins/dynamodb/ui/dist; \
		cp ui/dist/index.js ~/.local/share/tabularis/plugins/dynamodb/ui/dist/; \
	fi
	@echo "Installed to ~/.local/share/tabularis/plugins/dynamodb"
	@echo "Restart Tabularis (or toggle the plugin in Settings) to pick up changes."

[macos]
dev-install: build
	# Tabularis >= 0.24.0 reads plugins from the unified `tabularis` project dir.
	# Releases before that used the `com.debba.tabularis` identifier and migrate
	# an existing tree into the new one on first start, so install into whichever
	# path already exists and default to the current one on a clean machine.
	dest="$HOME/Library/Application Support/tabularis/plugins/dynamodb"; \
	legacy="$HOME/Library/Application Support/com.debba.tabularis/plugins/dynamodb"; \
	if [ ! -d "$dest" ] && [ -d "$legacy" ]; then dest="$legacy"; fi; \
	mkdir -p "$dest"; \
	cp target/debug/dynamodb-plugin "$dest/"; \
	cp .tabularium "$dest/"; \
	if [ -f ui/dist/index.js ]; then mkdir -p "$dest/ui/dist"; cp ui/dist/index.js "$dest/ui/dist/"; fi; \
	echo "Installed to $dest"; \
	echo "Restart Tabularis (or toggle the plugin in Settings) to pick up changes."

[windows]
dev-install: build
	#!pwsh

	# Tabularis >= 0.24.0 reads plugins from the unified `tabularis` project dir.
	# Releases before that used the `debba\tabularis\data` tree and migrate an
	# existing install into the new one on first start, so target whichever path
	# already exists and default to the current one on a clean machine.
	$dest = Join-Path $env:APPDATA "tabularis\plugins\dynamodb"
	$legacy = Join-Path $env:APPDATA "debba\tabularis\data\plugins\dynamodb"
	if (-not (Test-Path $dest) -and (Test-Path $legacy)) { $dest = $legacy }
	New-Item -ItemType Directory -Force -Path $dest | Out-Null
	Copy-Item "target\debug\dynamodb-plugin.exe" $dest
	Copy-Item ".tabularium" $dest
	if (Test-Path "ui\dist\index.js") {
		New-Item -ItemType Directory -Force -Path "$dest\ui\dist" | Out-Null
		Copy-Item "ui\dist\index.js" "$dest\ui\dist"
	}
	Write-Host "Installed to $dest"
	Write-Host "Restart Tabularis (or toggle the plugin in Settings) to pick up changes."

[linux]
uninstall:
	rm -rf ~/.local/share/tabularis/plugins/dynamodb

[macos]
uninstall:
	# Both trees: an install can live in either, depending on the app version.
	rm -rf "$HOME/Library/Application Support/tabularis/plugins/dynamodb"
	rm -rf "$HOME/Library/Application Support/com.debba.tabularis/plugins/dynamodb"

[windows]
uninstall:
	#!pwsh

	# Both trees: an install can live in either, depending on the app version.
	# (This recipe ran line-by-line without a shebang before, so `$dest` was
	# unset by the time `Test-Path` ran and nothing was removed.)
	$current = Join-Path $env:APPDATA "tabularis\plugins\dynamodb"
	$legacy = Join-Path $env:APPDATA "debba\tabularis\data\plugins\dynamodb"
	if (Test-Path $current) { Remove-Item -Recurse -Force $current }
	if (Test-Path $legacy) { Remove-Item -Recurse -Force $legacy }
