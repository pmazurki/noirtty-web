# NoirTTY Web Terminal

NoirTTY is a modern web-based terminal emulator built with Rust and WebGPU, tuned for mobile and low-latency interaction over WebSockets.

## Tech Stack

### Backend
- **axum + tokio** — async web server
- **portable-pty** — PTY management and shell spawning
- **WebSocket** — terminal I/O transport

### Frontend (WASM)
- **wgpu** — GPU-accelerated rendering (WebGPU + WebGL fallback)
- **vte** — ANSI escape parsing
- **glyphon / cosmic-text** — GPU text rendering pipeline

## Architecture

```
┌─────────────────────────────────────────────┐
│  Frontend (WASM + WebGPU)                   │
│  - VTE parser (ANSI escape sequences)       │
│  - Terminal grid state                      │
│  - WebGPU renderer (Canvas2D fallback)      │
│  - Keyboard/IME input handling              │
├─────────────────────────────────────────────┤
│  WebSocket                                  │
│  - Low-latency keyboard input               │
│  - Binary/JSON terminal output              │
├─────────────────────────────────────────────┤
│  Backend (Rust + Axum)                      │
│  - PTY management                           │
│  - Shell spawning (bash/zsh)                │
│  - Session handling                         │
└─────────────────────────────────────────────┘
```

## Features

- **WebGPU rendering** for hardware-accelerated terminal display
- **Fallbacks** for browsers without WebGPU (Canvas2D / WebGL)
- **Mobile-first UI** (safe-area aware layout, soft keyboard, toolbars)
- **Passkey auth** (WebAuthn; disabled on IP/.local)
- **PWA support**

## Prerequisites

- Rust **1.92+**
- `wasm-bindgen-cli`
- A modern browser with WebGPU (Safari/Chrome/Edge)

## Build

```bash
# Install tools
make setup

# Build everything (release)
make build

# Build separately
make build-wasm
make build-server

# Development build
make dev
```

## Run

```bash
# Development mode
make run

# Release mode
make run-release
```

Server starts on:
- **HTTPS**: https://localhost:3000

Debug UI:
```bash
NOIRTTY_DEBUG=1 ./target/debug/noirtty-web-server
```

Generated files (not committed by default):
- `certs/` (self-signed TLS cert + passkey storage)

Embedded assets:
NoirTTY embeds `static/` into the server binary. If you want to force file-based assets during dev:
```bash
NOIRTTY_EMBED_STATIC=0 ./dist/noirtty-web-server
```

Data directory:
By default certs/passkeys are stored in `./certs`. To override:
```bash
NOIRTTY_DATA_DIR=/path/to/data ./dist/noirtty-web-server
```

## Project Structure

```
noirtty-web/
├── server/                 # Rust backend
│   └── src/main.rs
├── client/                 # WASM frontend
│   ├── src/lib.rs
│   ├── src/terminal.rs
│   ├── src/renderer/
│   └── src/transport.rs
├── static/                 # Web assets
│   ├── index.html
│   └── manifest.json
├── dist/                   # Release artifacts (optional)
├── Cargo.toml
└── Makefile
```

## Notes

- WebAuthn does **not** work on IP or `.local`. Use a real domain for passkeys.
- For release artifacts, tag a version to trigger GitHub Releases:
  ```bash
  git tag v0.1.0
  git push --tags
  ```

## License

MIT
