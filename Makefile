# NoirTTY Web Terminal - Build System
# Modern web terminal with WebGPU + WebTransport

.PHONY: all build build-wasm build-server run run-https test clean help dev

# Enable WebGPU on wasm builds (Safari/Chrome)
WASM_RUSTFLAGS ?= --cfg=web_sys_unstable_apis --cfg=web

# Verbose logging for server (make run V=1)
RUN_FLAGS :=
WATCH_RUN_FLAGS :=
RUN_ENV :=
# INSECURE=1 disables HTTPS (for local dev only)
ifneq ($(INSECURE),)
RUN_ENV += NOIRTTY_INSECURE=1
endif
ifneq ($(CERT_HOSTS),)
RUN_ENV += NOIRTTY_CERT_HOSTS=$(CERT_HOSTS)
endif
ifneq ($(V),)
RUN_FLAGS += -v
WATCH_RUN_FLAGS += -- -v
endif

# Default target
all: build

# Build everything
build: build-wasm build-server

# Build WASM client
build-wasm:
	@echo "Building WASM client..."
	cd client && RUSTFLAGS="$(WASM_RUSTFLAGS) $(RUSTFLAGS)" cargo build --target wasm32-unknown-unknown --release
	@echo "Running wasm-bindgen..."
	wasm-bindgen --target web --out-dir static \
		target/wasm32-unknown-unknown/release/noirtty_web_client.wasm
	@echo "WASM client built successfully"

# Build server
build-server:
	@echo "Building server..."
	cd server && cargo build --release
	@echo "Server built successfully"

# Development build (debug mode)
dev: dev-wasm dev-server

dev-wasm:
	@echo "Building WASM client (debug)..."
	cd client && RUSTFLAGS="$(WASM_RUSTFLAGS) $(RUSTFLAGS)" cargo build --target wasm32-unknown-unknown
	wasm-bindgen --target web --out-dir static --debug \
		target/wasm32-unknown-unknown/debug/noirtty_web_client.wasm

dev-server:
	@echo "Building server (debug)..."
	cd server && cargo build

# Run development server (HTTPS by default)
run: dev
	@echo "Starting NoirTTY Web Server (HTTPS)..."
	@echo "  HTTPS: https://localhost:3000"
	$(RUN_ENV) ./target/debug/noirtty-web-server $(RUN_FLAGS)

# Run development server WITHOUT TLS (insecure, local dev only)
run-insecure: dev
	@echo "⚠️  Starting NoirTTY Web Server (INSECURE MODE)..."
	@echo "  HTTP: http://localhost:3000"
	$(RUN_ENV) NOIRTTY_INSECURE=1 ./target/debug/noirtty-web-server $(RUN_FLAGS)

# Run with auto-restart (requires cargo-watch)
watch:
	@echo "Checking for cargo-watch..."
	@which cargo-watch > /dev/null || (echo "Installing cargo-watch..." && cargo install cargo-watch)
	@echo "Starting Server in Watch Mode (HTTPS)..."
	@echo "  HTTPS: https://localhost:3000"
	cargo watch -w server -x "run -p noirtty-web-server $(WATCH_RUN_FLAGS)"

# Kill lingering server processes (ports 3000 & 4433)
kill:
	@echo "Killing processes on ports 3000 and 4433..."
	@lsof -ti:3000 | xargs kill -9 2>/dev/null || true
	@lsof -ti:4433 | xargs kill -9 2>/dev/null || true
	@echo "Done."

# Run release server (HTTPS by default)
run-release: build
	@echo "Starting NoirTTY Web Server (HTTPS, release)..."
	@echo "  HTTPS: https://localhost:3000"
	$(RUN_ENV) ./target/release/noirtty-web-server $(RUN_FLAGS)

# Run release server WITHOUT TLS (insecure, NOT recommended)
run-release-insecure: build
	@echo "⚠️  Starting NoirTTY Web Server (INSECURE MODE, release)..."
	@echo "  HTTP: http://localhost:3000"
	$(RUN_ENV) NOIRTTY_INSECURE=1 ./target/release/noirtty-web-server $(RUN_FLAGS)

# Run tests
test:
	cargo test --workspace

# Check compilation
check:
	cargo check --workspace

# Format code
fmt:
	cargo fmt --all

# Lint code
lint:
	cargo clippy --workspace -- -D warnings

# Clean build artifacts
clean:
	cargo clean
	rm -f static/noirtty_web_client.js
	rm -f static/noirtty_web_client.d.ts
	rm -f static/noirtty_web_client_bg.wasm
	rm -f static/noirtty_web_client_bg.wasm.d.ts

# Install required tools
setup:
	@echo "Installing required tools..."
	rustup target add wasm32-unknown-unknown
	cargo install wasm-bindgen-cli
	@echo "Setup complete"

# Reset authentication (delete passkey and generate new setup token)
reset-auth: dev
	@echo "Resetting authentication..."
	$(RUN_ENV) NOIRTTY_RESET_AUTH=1 ./target/debug/noirtty-web-server $(RUN_FLAGS)

# Show help
help:
	@echo "NoirTTY Web Terminal - Build System"
	@echo ""
	@echo "Usage: make [target]"
	@echo ""
	@echo "Targets:"
	@echo "  build            Build everything (release)"
	@echo "  build-wasm       Build WASM client only"
	@echo "  build-server     Build server only"
	@echo "  dev              Build in debug mode"
	@echo "  run              Build and run with HTTPS (debug)"
	@echo "  run-insecure     Build and run WITHOUT TLS (debug, local dev only)"
	@echo "  run-release      Build and run with HTTPS (release)"
	@echo "  run-release-insecure  Run without TLS (release, NOT recommended)"
	@echo "  reset-auth       Reset passkey and generate new setup token"
	@echo "  test             Run tests"
	@echo "  check            Check compilation"
	@echo "  fmt              Format code"
	@echo "  lint             Run clippy"
	@echo "  clean            Clean build artifacts"
	@echo "  setup            Install required tools"
	@echo "  help             Show this help"
	@echo ""
	@echo "Flags:"
	@echo "  V=1              Enable verbose server logs (pass -v)"
	@echo "  INSECURE=1       Disable HTTPS (use only for local development)"
	@echo "  CERT_HOSTS=      Comma-separated SANs for the cert (e.g. 192.168.8.177)"
	@echo "  NOIRTTY_DEBUG=1  Enable debug UI in the web client"
