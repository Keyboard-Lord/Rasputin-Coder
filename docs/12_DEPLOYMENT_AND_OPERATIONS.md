# Rasputin Deployment and Operations

## Deployment Model

Rasputin is a **local desktop application**, not a service. Deployment means:

1. Repository clone/download
2. Dependency installation (Rust, Ollama)
3. Model pulling
4. Build (or use pre-built if available)

## System Requirements

### Minimum
- **OS**: macOS 12+, Linux (Ubuntu 20.04+), Windows 10+
- **RAM**: 8 GB (16 GB recommended for 14B models)
- **Disk**: 2 GB for code + models
- **CPU**: x86_64 or ARM64

### Recommended
- **RAM**: 32 GB
- **GPU**: NVIDIA/AMD with CUDA/ROCm (for Ollama GPU acceleration)
- **SSD**: For model loading speed

## Installation Steps

### 1. Install Dependencies

**Rust**:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

**Ollama**:
```bash
# macOS
brew install ollama

# Linux
curl -fsSL https://ollama.com/install.sh | sh
```

### 2. Clone Repository
```bash
git clone <repository-url>
cd Rasputin-1
```

### 3. Build
```bash
cargo build --release
```

### 4. Pull Models
```bash
ollama pull qwen2.5-coder:14b
```

### 5. Start Ollama
```bash
ollama serve
```

### 6. Run Rasputin
```bash
./rasputin /path/to/workspace
```

## CI/CD Pipeline

GitHub Actions (`.github/workflows/ci.yml`):

```yaml
name: ci

on:
  push:
  pull_request:

jobs:
  rust:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Test Forge runtime
        run: cargo test -p forge_bootstrap --quiet
      - name: Test Rasputin TUI
        run: cargo test -p rasputin-tui --quiet
```

## Operations

### Startup
```bash
./rasputin [workspace-path]
```

If no workspace provided, restores last active repo from persistence.

### Daily Use
1. Launch with workspace
2. Use plain chat for discussion
3. Type task-like goals directly or use `/goal <description>` for code changes
4. Review inspector during execution
5. Check diff after completion

### Model Management

**Switch models**:
```bash
# In TUI
/model qwen2.5-coder:7b

# Or environment
FORGE_PLANNER_MODEL=qwen2.5-coder:7b ./rasputin ./workspace
```

**List available**:
```bash
ollama list
```

**Pull new model**:
```bash
ollama pull qwen2.5-coder:14b-q4KM
```

### Troubleshooting Operations

| Symptom | Diagnosis | Fix |
|---------|-----------|-----|
| "No repo attached" | Footer block reason | `/open <path>` |
| "Ollama disconnected" | Health check failed | `ollama serve` |
| Model not found | Model not in `ollama list` | `ollama pull <model>` |
| Task fails quickly | Planner/tool/validation error | Check Logs tab |
| Build fails | Syntax/type error | Review file, rerun |
| UI unresponsive | Terminal issue | `Ctrl+C`, restart |

### Log Monitoring

**View logs**:
```bash
tail -f ~/.local/share/rasputin/rasputin.log
```

**Log levels** (via `tracing`):
- ERROR: Failures
- WARN: Warnings, repair loops
- INFO: Normal operation
- DEBUG: Detailed execution (if enabled)

### State Management

**Reset all state**:
```bash
rm -rf ~/.local/share/rasputin/
```

**Reset just conversations**:
```bash
# Edit state.json, remove conversations array
```

**Archive old chats**: Use `/archive` or sidebar controls

## Scaling Considerations

### Single-User Design
- Rasputin is designed for single-developer use
- No multi-user support
- No concurrent task execution (one Forge worker at a time)

### Performance Tuning

**For faster startup**:
- Keep `forge_bootstrap` built: `cargo build -p forge_bootstrap`
- Use smaller models: `qwen2.5-coder:7b`
- Enable release builds

**For better quality**:
- Use larger models: `qwen2.5-coder:14b`
- Increase iteration limit (via fork)
- Provide more context in tasks

### Resource Limits

| Resource | Limit | Configurable |
|----------|-------|--------------|
| Iterations | 10 per task | Via code change |
| Repair retries | 3 | Hardcoded |
| Timeout | 30s per planner call | Via env var |
| Output | JSONL or human | `FORGE_OUTPUT_MODE` |

## Backup and Recovery

### Persistent State
Backup `~/.local/share/rasputin/state.json`:
```bash
cp ~/.local/share/rasputin/state.json ~/rasputin-backup.json
```

### Repository Safety
- Forge operates within attached repo only
- Validation gates prevent bad code persistence
- Auto-revert on validation failure (configurable)

### Recovery Procedures

**TUI crash**:
```bash
# State is persisted, just restart
./rasputin
```

**Interrupted chain**:
```bash
# After crash or /stop, resume where left off
./rasputin
/chain status              # Check which step failed
/chain resume <id>         # Continue from next step
```

**Corrupted state**:
```bash
rm ~/.local/share/rasputin/state.json
# Restart fresh
```

**Bad task output**:
- Review diff tab
- Check files on disk
- Revert manually if needed
- Rewrite the task-like goal or rerun `/goal <description>`

**Risk preview blocks execution**:
- Review risk summary displayed
- Address critical risks (GitConflict, etc.)
- Use `--force` flag if appropriate (proceeds despite risks)
- Or resolve underlying issue and retry

**Interrupt handling**:
- `/stop` or Ctrl+C interrupts current execution
- Step marked Failed, chain state preserved
- Use `/chain resume` to continue from next step

**Git grounding warnings**:
- Commit or stash changes before task execution
- Or proceed with warning acknowledgment

## Monitoring

### Health Checks

**Ollama**:
```bash
curl http://localhost:11434/api/tags
```

**Model availability**:
```bash
ollama list | grep <model>
```

### Metrics

Available in logs:
- Task duration
- Iteration count
- Validation pass/fail rates
- Tool usage counts

No built-in metrics export (Prometheus, etc.)—local-only design.

## Security Operations

### File System
- Sandbox: Repository boundary enforcement
- No arbitrary path traversal
- Read-before-write gate
- Validation-gated persistence

### Network
- Ollama only: localhost:11434
- No external API calls
- No telemetry or analytics

### Process
- Worker spawned per task
- Process isolation
- Clean termination on exit

## Update Procedures

### Code Updates
```bash
git pull
cargo build --release
```

### Model Updates
```bash
ollama pull qwen2.5-coder:14b  # Updates to latest
```

### Dependency Updates
```bash
cargo update
cargo test  # Verify after update
```
