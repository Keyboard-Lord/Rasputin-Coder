# Rasputin Technology Stack

## Programming Languages

| Language | Purpose | Version |
|----------|---------|---------|
| **Rust** | Primary implementation | Edition 2024 (2021 for interface crate) |
| **Bash** | Launcher script, build orchestration | POSIX-compliant |

## Core Dependencies

### Rasputin TUI (`apps/rasputin-tui/Cargo.toml`)

| Crate | Version | Purpose |
|-------|---------|---------|
| `ratatui` | 0.29 | Terminal UI framework |
| `crossterm` | 0.28 | Cross-platform terminal control |
| `tokio` | 1.x | Async runtime (full features) |
| `serde` / `serde_json` | 1.x | Serialization |
| `toml` | 0.8 | TOML config parsing |
| `reqwest` | 0.12 | HTTP client for Ollama API |
| `chrono` | 0.4 | Date/time handling |
| `uuid` | 1.x | UUID generation |
| `rand` | 0.8 | Random generation |
| `tracing` / `tracing-subscriber` | 0.1/0.3 | Structured logging |
| `open` | 5.x | Open URLs in browser |
| `strip-ansi-escapes` | 0.2 | ANSI stripping |
| `syntect` | 5.x | Syntax highlighting |
| `clipboard` | 0.5 | Clipboard operations |
| `similar` | 2.7 | Diff generation |
| `md5` | 0.7 | Hashing |
| `anyhow` | 1.x | Error handling |

### Forge Runtime (`crates/forge-runtime/Cargo.toml`)

| Crate | Version | Purpose |
|-------|---------|---------|
| `serde` / `serde_json` | 1.0 | Serialization |
| `regex` | 1.x | Pattern matching |
| `sha2` | 0.10 | Cryptographic hashing |
| `sha3` | 0.10 | Additional hashing |
| `hex` | 0.4 | Hex encoding |
| `ctrlc` | 3.4 | Signal handling |
| `tracing` | 0.1 | Logging |

### Interface Layer (`crates/rasputin-interface/Cargo.toml`)

| Crate | Version | Purpose |
|-------|---------|---------|
| `chrono` | 0.4 | Date/time with serde |
| `serde` / `serde_json` | 1.0 | Serialization |
| `thiserror` | 1.0 | Error definitions |
| `uuid` | 1.x | UUID generation |
| `tokio` | 1.0 | Async (sync features only) |

## External Dependencies

| Tool | Purpose | Required |
|------|---------|----------|
| **Ollama** | Local LLM inference (loopback-only, no remote endpoints) | Yes |
| **Cargo/Rustc** | Build system | Yes |
| **Git** | Repository detection | No (optional) |
| **AppleScript** | Native folder picker (macOS host actions) | No (macOS only) |

## Supported Language Toolchains

Validation and build detection support:

| Language | Syntax Check | Build | Test |
|----------|--------------|-------|------|
| Rust | Built-in | `cargo build` | `cargo test` |
| Python | Built-in | N/A | `pytest` |
| JavaScript/TypeScript | Built-in | `tsc --noEmit`, `npm run build` | `npm test` |

## Build Configuration

### Workspace Structure
```toml
[workspace]
members = [
    "apps/rasputin-tui",
    "crates/forge-runtime",
    "crates/rasputin-interface",
]
resolver = "2"

[profile.release]
opt-level = 3
lto = true
strip = true
```

### Binary Names
- `rasputin-tui` — User-facing TUI application
- `forge_bootstrap` — Internal worker runtime (launched by TUI)

## CI/CD

GitHub Actions workflow (`.github/workflows/ci.yml`):
- Runs on: `push`, `pull_request`
- OS: `ubuntu-latest`
- Rust toolchain: `dtolnay/rust-toolchain@stable`
- Commands:
  - `cargo test -p forge_bootstrap --quiet`
  - `cargo test -p rasputin-tui --quiet`

## Development Tools

| Tool | Purpose |
|------|---------|
| `cargo` | Build, test, package management |
| `check_warnings.sh` | Script to check for compiler warnings |
| `rasputin` | Canonical launcher script |

## Version Constraints

- **Rust Edition**: 2024 (primary), 2021 (interface crate for compatibility)
- **Minimum Rust version**: Latest stable (uses Edition 2024 features)
- **Ollama API**: Compatible with standard Ollama REST API at `http://127.0.0.1:11434`

## Rationale for Choices

### Why Rust?
- Systems programming requirements (process management, terminal control)
- Strong type safety for complex state machines
- Excellent async/await support via Tokio
- Cross-platform compilation

### Why ratatui + crossterm?
- Pure Rust terminal UI solution
- Cross-platform (macOS, Linux, Windows)
- Immediate-mode rendering suitable for live event streams

### Why Ollama-only?
- **Privacy**: Code never leaves the machine (architecturally enforced via loopback-only HTTP client)
- **Security**: HTTP client assert-rejects any non-loopback endpoint (`127.0.0.1`, `localhost`, `[::1]` only)
- **Cost**: Zero per-token API fees
- **Control**: User manages model selection and updates
- **Offline operation**: Full functionality without internet connectivity

### Why Separate Crates?
- `rasputin-tui`: Product layer with heavy UI dependencies
- `forge-runtime`: Worker with minimal dependencies for fast spawn
- `rasputin-interface`: Shared types (partially integrated)
