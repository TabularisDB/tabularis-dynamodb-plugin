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
		(cd ui && pnpm install && pnpm run build); \
	fi

[windows]
build-ui:
	#!pwsh

	if (Test-Path ui/package.json) {
		Write-Host "Building UI extension..."
		Push-Location ui
		try {
			pnpm i
			if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
			pnpm run build
			if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
		} finally {
			Pop-Location
		}
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
	mkdir -p "$HOME/Library/Application Support/com.debba.tabularis/plugins/dynamodb"
	cp target/debug/dynamodb-plugin "$HOME/Library/Application Support/com.debba.tabularis/plugins/dynamodb/"
	cp .tabularium "$HOME/Library/Application Support/com.debba.tabularis/plugins/dynamodb/"
	@if [ -f ui/dist/index.js ]; then \
		mkdir -p "$HOME/Library/Application Support/com.debba.tabularis/plugins/dynamodb/ui/dist"; \
		cp ui/dist/index.js "$HOME/Library/Application Support/com.debba.tabularis/plugins/dynamodb/ui/dist/"; \
	fi
	@echo "Installed to ~/Library/Application Support/com.debba.tabularis/plugins/dynamodb"
	@echo "Restart Tabularis (or toggle the plugin in Settings) to pick up changes."

[windows]
dev-install: build
	#!pwsh
	$dest = Join-Path $env:APPDATA "debba\tabularis\data\plugins\dynamodb"
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
	rm -rf "$HOME/Library/Application Support/com.debba.tabularis/plugins/dynamodb"

[windows]
uninstall:
	$dest = Join-Path $env:APPDATA "debba\tabularis\data\plugins\dynamodb"
	if (Test-Path $dest) { Remove-Item -Recurse -Force $dest }
