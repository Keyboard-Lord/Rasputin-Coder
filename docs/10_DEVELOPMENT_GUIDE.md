# Rasputin Development Guide

## Prerequisites

- **Rust**: Latest stable toolchain (uses Edition 2024)
- **Cargo**: Included with Rust
- **Ollama**: Local LLM runtime (https://ollama.com)
- **Git**: For repository operations

## Setup

### 1. Clone Repository
```bash
git clone <repository-url>
cd Rasputin-1
```

### 2. Install Rust (if needed)
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
```

### 3. Install Ollama
```bash
# macOS
brew install ollama

# Linux
curl -fsSL https://ollama.com/install.sh | sh
```

### 4. Pull Recommended Model
```bash
ollama pull qwen2.5-coder:14b
```

### 5. Start Ollama
```bash
ollama serve
```

## Building

### Build Entire Workspace
```bash
cargo build
```

### Build Release
```bash
cargo build --release
```

### Build Specific Package
```bash
# TUI application
cargo build -p rasputin-tui

# Forge worker
cargo build -p forge_bootstrap
```

### Check Without Building
```bash
cargo check
```

### Check Warnings
```bash
./scripts/check_warnings.sh
```

## Running

### Canonical Launcher (Recommended)
```bash
./rasputin /path/to/workspace
```

### Direct Cargo Run
```bash
cargo run -p rasputin-tui -- /path/to/workspace
```

### Run Forge Worker Directly (Testing)
```bash
cargo run -p forge_bootstrap -- "Create hello.txt" 10 http
```

## Testing

### Run All Tests
```bash
cargo test
```

### Run Package Tests
```bash
cargo test -p rasputin-tui
cargo test -p forge_bootstrap
```

### Run Specific Test Categories
```bash
# Validation tests
cargo test -p forge_bootstrap validation

# Chain tests
cargo test -p forge_bootstrap chain

# State hash tests
cargo test -p forge_bootstrap state_hash
```

### Run Quiet Tests (CI Mode)
```bash
cargo test -p forge_bootstrap --quiet
cargo test -p rasputin-tui --quiet
```

## Project Structure

### Key Modules

| Path | Purpose |
|------|---------|
| `apps/rasputin-tui/src/app.rs` | Main app state and command handling |
| `apps/rasputin-tui/src/main.rs` | Entry point |
| `apps/rasputin-tui/src/state.rs` | UI state types |
| `apps/rasputin-tui/src/ollama.rs` | HTTP client for Ollama chat API |
| `observability.rs` | Inspector views, debug bundles, event viewing |
| `persistence.rs` | Product state persistence, chain orchestration |
| `repo.rs` | Repository attachment |
| `apps/rasputin-tui/src/forge_runtime.rs` | Worker bridge |
| `crates/forge-runtime/src/runtime.rs` | Bounded execution loop |
| `crates/forge-runtime/src/validation_engine.rs` | Post-mutation validation |
| `crates/forge-runtime/src/state.rs` | `AgentState` management |
| `crates/forge-runtime/src/governance.rs` | Runtime governance |
| `crates/forge-runtime/src/task_intake.rs` | Task classification and risk assessment |
| `crates/forge-runtime/src/git_grounding.rs` | Git status capture |
| `crates/forge-runtime/src/approval_checkpoint.rs` | Checkpoint management |
| `crates/forge-runtime/src/tool_registry.rs` | Tool system |
| `support/workspace_config.rs` | Config resolution |

## Debugging

### Enable Logging
Logs are written to `~/.local/share/rasputin/rasputin.log` automatically.

### View Logs
```bash
tail -f ~/.local/share/rasputin/rasputin.log
```

### JSONL Output Mode
```bash
FORGE_OUTPUT_MODE=jsonl cargo run -p forge_bootstrap -- "task description"
```

### Verbose Build
```bash
cargo build -p rasputin-tui -v
```

## Code Organization

### Adding a New Command

1. Define in `apps/rasputin-tui/src/commands.rs`:
   ```rust
   pub enum Command {
       MyNewCommand { arg: String },
   }
   ```

2. Parse in `parse_command()`:
   ```rust
   if lower.starts_with("/mycommand ") {
       return Command::MyNewCommand { arg: ... };
   }
   ```

3. Handle in `apps/rasputin-tui/src/app.rs::handle_command()`:
   ```rust
   Command::MyNewCommand { arg } => {
       // Implementation
   }
   ```

### Adding a New Tool

1. Implement `Tool` trait in `crates/forge-runtime/src/tools/`:
   ```rust
   impl Tool for MyTool {
       fn name(&self) -> ToolName { ToolName::new("my_tool").unwrap() }
       fn execute(&self, args: &ToolArguments, ctx: &ExecutionContext) -> Result<ToolResult, ForgeError> {
           // Implementation
       }
   }
   ```

2. Register in `crates/forge-runtime/src/tool_registry.rs`:
   ```rust
   Self::register_tool(&mut tools, MyTool::new());
   ```

## Common Development Tasks

### Reset Persistent State
```bash
rm ~/.local/share/rasputin/state.json
```

### Test With Different Model
```bash
FORGE_PLANNER_MODEL=qwen2.5-coder:7b ./rasputin ./test-workspace
```

### Check Ollama Health
```bash
curl http://localhost:11434/api/tags
```

### Build Worker for Testing
```bash
cargo build -p forge_bootstrap
```

## CI/CD

GitHub Actions workflow (`.github/workflows/ci.yml`):

```yaml
jobs:
  rust:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test -p forge_bootstrap --quiet
      - run: cargo test -p rasputin-tui --quiet
```

## Development Tips

### TUI Testing
The TUI uses `ratatui` with `crossterm`. To test without UI:
- Use `FORGE_OUTPUT_MODE=jsonl` for worker testing
- Run unit tests on individual modules

### Forge Testing
The Forge runtime can run standalone:
```bash
cargo run -p forge_bootstrap -- "task" 10 http
```

### Chain Development Testing

Test chain persistence and commands:
```bash
# Build and run TUI
cargo run -p rasputin-tui

# In TUI, test chain commands:
/chains                              # List chains
/chain status                        # Show active chain
/chain switch <id>                   # Switch chains
/chain resume <id>                   # Resume chain execution
/plan                                # Show chain plan

# After /quit and restart, verify:
/chains                              # Chains should persist
```

### Chain Unit Testing

Test chain persistence code:
```bash
cargo test -p rasputin-tui persistence
```

### Model Testing
Test with different models to verify behavior:
```bash
FORGE_PLANNER_MODEL=qwen2.5-coder:7b cargo run -p forge_bootstrap -- "task"
FORGE_PLANNER_MODEL=qwen2.5-coder:14b cargo run -p forge_bootstrap -- "task"
```

### Validation Testing
Create test projects with different languages to validate syntax/build/test stages.

## Code Style

- Use strongly-typed domain types (`SessionId`, `ToolName`)
- Fail-closed on errors (return errors rather than panicking)
- Document public APIs with doc comments
- Prefer `tracing` for logging over `println!`

## Code Audit Summary (Consolidated)

### Executive Verdict
**Rasputin is a real terminal-first local app.** Every major user-facing feature traces through to genuine host-side effects — filesystem writes, shell command execution, process spawning, and JSON persistence to disk.

### Feature Truth Table

| Feature | Real Host Effect? | Code Path |
|---------|-------------------|-----------|
| Project creation | ✅ REAL | `Command::NewProject` → `host_actions::create_project` → `fs::create_dir_all` |
| Project attach/connect | ✅ REAL | `Command::OpenRepo` → `host_actions::attach_project` → `Repo::attach` |
| Project delete | ✅ REAL | `Command::DeleteProject` → `fs::remove_dir_all` |
| File read | ✅ REAL | `Command::ReadFile` → `fs::read_to_string` with path boundary enforcement |
| File write | ✅ REAL | `Command::WriteFile` → `fs::write` + diff computation |
| File patch (TUI) | ⚠️ PARTIAL | `str::replacen` — no hash binding (Forge runtime version is hardened) |
| Shell command | ✅ REAL | `Command::RunShellCommand` → `Command::new($SHELL).arg("-lc").arg(cmd)` |
| Forge task execution | ✅ REAL | Spawns `forge_bootstrap` child process, streams JSONL events |
| Persistence (save/load) | ✅ REAL | Writes to `~/.local/share/rasputin/` |
| Search (rg) | ✅ REAL | `std::process::Command::new("rg")` |
| Browser preview | ✅ REAL | `open::that(url)` |

### Critical Findings (from Audit)

**CF-1: TUI ApplyPatch is unhardened**
- TUI's `apply_patch` uses `str::replacen` with no hash binding
- Forge runtime's `ApplyPatchTool` requires `expected_hash` and enforces cardinality
- **Impact**: User issuing `/replace` in EDIT mode gets weaker patch than through TASK mode

**CF-2: Boot does not restore last active conversation**
- `App::new()` always creates a new `conversation_id`
- User sees blank chat on every restart
- **Impact**: Poor UX — context loss on restart

## Troubleshooting Development Issues

### Build Fails
```bash
# Clean and rebuild
cargo clean
cargo build
```

### Ollama Not Found
```bash
# Check Ollama is running
curl http://localhost:11434/api/tags

# Start Ollama
ollama serve
```

### Worker Spawn Fails
```bash
# Build worker manually
cargo build -p forge_bootstrap

# Check binary exists
ls target/debug/forge_bootstrap
```

### Terminal UI Issues
```bash
# Check terminal capabilities
echo $TERM

# Try different terminal emulator if issues persist
```
