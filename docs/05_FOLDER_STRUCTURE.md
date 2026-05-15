# Rasputin Folder Structure

## Repository Layout

```
Rasputin-1/
├── apps/                          # User-facing applications
│   └── rasputin-tui/              # Terminal UI (the product)
│       ├── src/
│       │   ├── app.rs             # App state, repo attach, Forge handoff
│       │   ├── bootstrap.rs       # Launch intent parsing
│       │   ├── clipboard/         # Clipboard operations
│       │   ├── commands.rs        # Slash command parsing
│       │   ├── diff.rs            # Diff generation and display
│       │   ├── events.rs          # Input event handling
│       │   ├── forge_runtime.rs   # Worker spawning + event bridge
│       │   ├── host_actions.rs    # External tool integrations
│       │   ├── interface_integration.rs  # Partial orchestration layer
│       │   ├── main.rs            # Entry point
│       │   ├── ollama.rs          # Ollama HTTP client
│       │   ├── persistence.rs     # Product state persistence
│       │   ├── repo.rs            # Repository attachment
│       │   ├── state.rs           # UI state types
│       │   ├── syntax.rs          # Syntax highlighting
│       │   ├── ui/                # Ratatui rendering
│       │   │   ├── layout.rs
│       │   │   ├── input_box.rs
│       │   │   ├── preview_pane.rs
│       │   │   ├── widgets/
│       │   │   └── mod.rs
│       │   └── validation.rs      # TUI-local validation pipeline
│       └── Cargo.toml
├── crates/                        # Library crates
│   ├── forge-runtime/             # Bounded execution engine
│   │   ├── src/
│   │   │   ├── conformance_tests.rs
│   │   │   ├── context_assembly.rs
│   │   │   ├── crypto_hash.rs
│   │   │   ├── execution/
│   │   │   │   └── validation_engine.rs
│   │   │   ├── governance.rs
│   │   │   ├── main.rs            # Worker entry point
│   │   │   ├── planner/           # LLM planner implementations
│   │   │   ├── runtime.rs         # Bounded runtime loop
│   │   │   ├── runtime_gates.rs   # Read-before-write, mode gates
│   │   │   ├── state.rs           # AgentState
│   │   │   ├── tool_registry.rs   # Tool registration
│   │   │   ├── tools/             # Tool implementations
│   │   │   │   ├── browser_preview_tool.rs
│   │   │   │   ├── execute_command_tool.rs
│   │   │   │   ├── file_tools.rs
│   │   │   │   └── search_tools.rs
│   │   │   ├── types.rs           # Core domain types
│   │   │   └── validator.rs       # Output validation
│   │   └── Cargo.toml
│   ├── rasputin-interface/        # Shared/transparency types (partial integration)
│   └── rasputin-forge/            # Legacy Deep Forge CLI mode
│       ├── src/
│       └── Cargo.toml
├── docs/                          # Documentation
│   ├── 01_PROJECT_OVERVIEW.md     # (This canonical set)
│   ├── 02_ARCHITECTURE.md
│   ├── 03_TECHNOLOGY_STACK.md
│   ├── 04_CORE_CONCEPTS.md
│   ├── 05_FOLDER_STRUCTURE.md
│   ├── 06_MAIN_WORKFLOWS.md
│   ├── 07_API_REFERENCE.md
│   ├── 08_DATA_MODEL.md
│   ├── 09_CONFIGURATION.md
│   ├── 10_DEVELOPMENT_GUIDE.md
│   ├── 11_TESTING_STRATEGY.md
│   ├── 12_DEPLOYMENT_AND_OPERATIONS.md
│   ├── 13_SECURITY_AND_COMPLIANCE.md
│   ├── 14_KNOWN_LIMITATIONS_AND_TRADEOFFS.md
│   └── 15_FUTURE_ROADMAP_AND_EXTENSIBILITY.md
├── support/                       # Shared utilities
│   ├── install_terminal_profile.py
│   └── workspace_config.rs        # Workspace model discovery
├── config/                        # Configuration templates
│   └── ollama/                    # Ollama modelfiles
├── assets/                        # Static assets
│   └── fonts/                     # OCR terminal fonts
├── examples/                      # Example traces and code
│   └── end_to_end_trace/
├── research/                      # Non-shipping explorations
│   └── mockups/
├── .github/workflows/             # CI/CD
│   └── ci.yml
├── rasputin                       # Canonical launcher script
├── rasputin.json                  # Repo-local workspace config
├── Cargo.toml                     # Root workspace manifest
└── README.md                      # Product overview
```

## Directory Purposes

### `apps/rasputin-tui/`
The product. This is what users interact with. Contains:
- Terminal UI code (ratatui)
- Ollama chat integration
- Persistence for product state
- Forge worker launching

### `crates/forge-runtime/`
The bounded execution engine. Spawned per-task as `forge_bootstrap`. Contains:
- Runtime loop with iteration limits
- Tool registry and implementations
- Validation engine
- State management (AgentState)

### `crates/rasputin-interface/`
Partial orchestration layer. Contains useful code but is **not** the canonical hot path. Currently contains:
- Shared serialization types
- Intent specifications
- Approval-oriented structures (not wired into live path)

### `crates/rasputin-forge/`
Legacy Deep Forge CLI mode. It can still be launched explicitly through the launcher’s forge mode, but it is not the active TUI/Forge worker hot path. Treat it as a compatibility surface unless the task explicitly targets legacy Deep Forge behavior.

### `docs/`
Authoritative documentation lives directly in the 15 numbered markdown files listed above. There is no `docs/canonical/` subtree in the current repository.

### `support/`
Shared implementation code:
- `workspace_config.rs` — Workspace model resolution from `.forge/config.yaml` or `rasputin.json`

### `config/ollama/`
Ollama modelfiles for recommended models (qwen2.5-coder variants).

## Key File Paths

| File | Purpose |
|------|---------|
| `apps/rasputin-tui/src/main.rs` | Product entry point |
| `apps/rasputin-tui/src/app.rs` | App state and Forge handoff (`start_execution_task`) |
| `apps/rasputin-tui/src/forge_runtime.rs` | Worker bridge |
| `crates/forge-runtime/src/main.rs` | Worker entry point |
| `crates/forge-runtime/src/runtime.rs` | Bounded runtime loop (`run_bootstrap`) |
| `support/workspace_config.rs` | Config resolution |
| `rasputin` | Launcher script |

## Data Paths

| Path | Purpose |
|------|---------|
| `~/.local/share/rasputin/state.json` | Product persistence |
| `~/.local/share/forge/session.json` | Engine persistence (API exists but not auto-wired) |
| `.forge/config.yaml` | Workspace model config (preferred) |
| `rasputin.json` | Workspace model config (fallback) |

## Top-Level Files Reference

| File | Purpose |
|------|---------|
| `README.md` | Product overview and quick start |
| `Cargo.toml` | Root workspace manifest |
| `rasputin` | Canonical launcher script |
| `rasputin.json` | Repo-local workspace model config |

## Historical/Archive Directories

### `.historical_docs_backup/`
Contains archived documentation from previous phases. It is not authoritative for current runtime behavior.

### `research/`
Non-shipping exploration material:
- `research/mockups/index.html` — mockup/prototype UI artifact
