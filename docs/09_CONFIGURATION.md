# Rasputin Configuration

## Quick Reference

| Config File | Purpose | Priority |
|-------------|---------|----------|
| `.forge/config.yaml` | Workspace model config | **1 (highest)** |
| `.forge/config.yml` | Alternative YAML name | 2 |
| `rasputin.json` | Legacy JSON config | 3 |
| Environment variables | Runtime overrides | Variable-specific |

## Workspace Configuration

### `.forge/config.yaml` (Preferred)

Location: `<workspace>/.forge/config.yaml`

Format:
```yaml
planner:
  model: qwen2.5-coder:14b

ollama:
  model: qwen2.5-coder:14b
```

Both `planner.model` and `ollama.model` are recognized. The `planner:` section is preferred.

### `rasputin.json` (Fallback)

Location: `<workspace>/rasputin.json`

Format:
```json
{
  "ollama_model": "qwen2.5-coder:14b"
}
```

## Environment Variables

| Variable | Description | Default | Example |
|----------|-------------|---------|---------|
| `FORGE_PLANNER_MODEL` | Model tag for planner | (from config) | `qwen2.5-coder:14b` |
| `FORGE_PLANNER_ENDPOINT` | Ollama API URL (loopback-only) | `http://127.0.0.1:11434` | `http://localhost:11434` |
| `FORGE_PLANNER_TEMPERATURE` | Sampling temperature | `0.0` | `0.1` |
| `FORGE_PLANNER_SEED` | Random seed | `42` | `12345` |
| `FORGE_CSS_COMPRESSION` | Enable prompt compression | `false` | `true` |
| `FORGE_OUTPUT_MODE` | Output format | `human` | `jsonl` |
| `FORGE_RUNTIME_BIN` | Worker binary path | auto-detected | `/path/to/forge_bootstrap` |
| `OLLAMA_HOST` | Ollama base URL | `http://localhost:11434` | `http://127.0.0.1:11434` |

**Note**: Temperature is clamped to `0.0..=0.1` regardless of environment setting.

**Security Constraint**: The Ollama endpoint is **architecturally restricted to loopback addresses only** (`127.0.0.1`, `localhost`, `[::1]`). Non-loopback endpoints are rejected at client construction. This is a non-negotiable security invariant — the system cannot be configured to call remote Ollama instances or cloud APIs.

## Runtime Configuration (Internal)

### RuntimeConfig Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `max_iterations` | `u32` | `10` | Maximum planner iterations |
| `task` | `String` | (required) | Task description |
| `auto_revert` | `bool` | `true` | Auto-revert on validation failure |
| `mode` | `ExecutionMode` | `Edit` | Execution mode |
| `planner_type` | `String` | `"http"` | Planner implementation |
| `planner_endpoint` | `String` | `http://127.0.0.1:11434` | Ollama API endpoint |
| `planner_model` | `String` | `qwen2.5-coder:14b` | Model tag |
| `planner_timeout_seconds` | `u32` | `30` | Planner request timeout |
| `planner_temperature` | `f32` | `0.0` | Temperature (clamped 0.0-0.1) |
| `planner_seed` | `u64` | `42` | Random seed |
| `css_compression` | `bool` | `false` | CSS prompt compression |

## Model Selection

### Model Priority

1. `FORGE_PLANNER_MODEL` environment variable
2. `.forge/config.yaml` → `planner.model`
3. `.forge/config.yaml` → `ollama.model`
4. `rasputin.json` → `ollama_model`
5. Default: `qwen2.5-coder:14b`

### Model Normalization

Model tags are normalized against installed Ollama models. If the requested model is not found, the system falls back through a preference chain:
- Requested model → Preferred candidates → Default → Any available

### CSS Compression

Auto-enabled for large models (14B+) to reduce prompt size. Can be forced via `FORGE_CSS_COMPRESSION=true`.

## Ollama Setup

### Prerequisites

1. Install Ollama: https://ollama.com
2. Pull recommended model:
   ```bash
   ollama pull qwen2.5-coder:14b
   ```
3. Start Ollama server:
   ```bash
   ollama serve
   ```

### Recommended Models

| Model | Size | Use Case |
|-------|------|----------|
| `qwen2.5-coder:14b` | 14B parameters | Balanced quality and speed |
| `qwen2.5-coder:14b-q4KM` | Quantized | Reduced memory usage |
| `qwen2.5-coder:7b` | 7B parameters | Faster, lower quality |

Smaller models (3B) may hallucinate frequently and are not recommended for serious use.

## Terminal Configuration

### macOS Terminal Profile

The launcher (`rasputin`) can install an OCR-optimized terminal profile:

```bash
# Automatic on macOS Terminal
./rasputin /path/to/workspace
```

Profile features:
- OCR-A BT font for better character recognition
- Optimized colors for terminal UI

Manual installation:
```bash
python3 support/install_terminal_profile.py
```

## Data Directories

| Purpose | Path |
|---------|------|
| Product state | `~/.local/share/rasputin/state.json` |
| Engine state (API) | `~/.local/share/forge/session.json` |
| Logs | `~/.local/share/rasputin/rasputin.log` |

## Validation Configuration

### Runtime Validation Stages

Current policy (not user-configurable):
- **Syntax**: Enabled for Python, JS/TS, Rust
- **Lint**: Skipped (emits "not configured" message)
- **Build**: Enabled when project type detected
- **Test**: Enabled when project type detected

### Project Type Detection

| Language | Build Command | Test Command |
|----------|-------------|--------------|
| Rust | `cargo build --quiet` | `cargo test --quiet` |
| TypeScript/Node | `tsc --noEmit` or `npm run build` | `npm test` |
| Python | (none) | `python -m pytest -q` |

## Configuration Examples

### Full `.forge/config.yaml`
```yaml
planner:
  model: qwen2.5-coder:14b-q4KM
  temperature: 0.0
  seed: 42

ollama:
  model: qwen2.5-coder:14b-q4KM
  endpoint: http://127.0.0.1:11434
```

### Development Override
```bash
# Use smaller model for testing
FORGE_PLANNER_MODEL=qwen2.5-coder:7b ./rasputin ./my-project

# JSONL output for scripting
FORGE_OUTPUT_MODE=jsonl ./rasputin ./my-project
```

## Troubleshooting

### Model Not Found
```
Check: ollama list
Fix: ollama pull <model>
Or: Use /model command to switch to available model
```

### Ollama Connection Failed
```
Check: curl http://localhost:11434/api/tags
Fix: ollama serve
Or: Set OLLAMA_HOST to correct address
```

### Worker Binary Missing
```
Auto-fix: TUI runs `cargo build --quiet -p forge_bootstrap`
Manual: cargo build -p forge_bootstrap
```
