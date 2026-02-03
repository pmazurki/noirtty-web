# NoirTTY Web Terminal

A modern web-based terminal emulator built with Rust, WebGPU, and WebTransport.

## 🚀 Tech Stack (2025 Edition)

### Backend
- **axum + tokio** - Async web framework
- **web-transport-quinn** - WebTransport/HTTP3 for low-latency I/O
- **portable-pty** - PTY management for shell spawning

### Frontend (WASM)
- **wgpu** - WebGPU rendering (Metal/Vulkan/WebGPU)
- **vte** - VTE parser for ANSI escape sequences
- **cosmic-text** - SDF font rendering (planned)

## Architecture

```
┌─────────────────────────────────────────────┐
│  Frontend (WASM + WebGPU)                   │
│  - VTE parser (ANSI escape sequences)       │
│  - Terminal grid state                      │
│  - WebGPU instanced renderer                │
│  - Keyboard/IME input handling              │
├─────────────────────────────────────────────┤
│  WebTransport (HTTP/3 / QUIC)               │
│  - Bidirectional streams                    │
│  - Low-latency keyboard input               │
│  - Binary terminal output                   │
├─────────────────────────────────────────────┤
│  Backend (Rust + Axum)                      │
│  - PTY management                           │
│  - Shell spawning (bash/zsh)                │
│  - Session handling                         │
└─────────────────────────────────────────────┘
```

## Features

- **WebGPU rendering** - Hardware-accelerated terminal display
- **Instanced rendering** - One draw call for entire terminal grid
- **Full keyboard support** - Including function keys, modifiers, IME
- **Touch gestures** - Optimized for iPad and mobile (planned)
- **PWA support** - Install as standalone app

## Prerequisites

- Rust 1.92+
- wasm-bindgen-cli
- Modern browser with WebGPU support:
  - Safari 17.4+ (macOS/iPadOS)
  - Chrome 113+
  - Firefox 118+ (with flag)

## Building

```bash
# Install tools
make setup

# Build everything
make build

# Or build separately
make build-wasm    # WASM client
make build-server  # Server

# Development build
make dev
```

## Running

```bash
# Development mode
make run

# Release mode
make run-release
```

Server starts on:
- HTTPS: https://localhost:3000

Debug UI:
```bash
NOIRTTY_DEBUG=1 ./target/debug/noirtty-web-server
```

Generated files (not committed):
- `certs/` (self‑signed TLS cert + passkey storage)
- `static/noirtty_web_client*` (WASM + JS from wasm-bindgen)

## Project Structure

```
noirtty-web/
├── server/           # Rust backend
│   └── src/
│       └── main.rs   # WebTransport server + PTY
├── client/           # WASM frontend
│   └── src/
│       ├── lib.rs        # Main WASM module
│       ├── terminal.rs   # VTE parser + grid state
│       ├── renderer.rs   # WebGPU renderer
│       ├── transport.rs  # WebTransport client
│       ├── input.rs      # Keyboard handling
│       └── shaders/
│           └── terminal.wgsl
├── static/           # Web assets
│   ├── index.html
│   └── manifest.json
├── Cargo.toml        # Workspace config
└── Makefile
```

## Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `Cmd/Ctrl+C` | Copy selection |
| `Cmd/Ctrl+V` | Paste from clipboard |
| Arrow keys | Navigate |
| F1-F12 | Function keys |

## TODO

- [ ] SDF font rendering with cosmic-text
- [ ] WebTransport client (currently stub)
- [ ] Touch gestures (scroll, pinch zoom)
- [ ] Selection and copy
- [ ] 120Hz ProMotion support
- [ ] Terminal bell audio

## License

MIT License
