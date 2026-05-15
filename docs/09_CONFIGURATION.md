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
  model: huihui_ai/deepseek-r1-abliterated:14b

ollama:
  model: huihui_ai/deepseek-r1-abliterated:14b
```

Both `planner.model` and `ollama.model` are recognized. The `planner:` section is preferred.

### `rasputin.json` (Fallback)

Location: `<workspace>/rasputin.json`

Format:
```json
{
  "ollama_model": "huihui_ai/deepseek-r1-abliterated:14b"
}
```

## Environment Variables

| Variable | Description | Default | Example |
|----------|-------------|---------|---------|
| `FORGE_PLANNER_MODEL` | Model tag for planner | (from config) | `huihui_ai/deepseek-r1-abliterated:14b` |
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
| `planner_model` | `String` | `huihui_ai/deepseek-r1-abliterated:14b` | Model tag |
| `sandbox_mode` | `SandboxMode` | `repo_boundary_only` | Current execution containment level; not a true sandbox |
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
5. Default: `huihui_ai/deepseek-r1-abliterated:14b`

**Backward Compatibility**: Legacy Qwen models (`qwen2.5-coder:14b` and variants) remain fully supported. Existing configurations will continue to work.

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
   ollama pull huihui_ai/deepseek-r1-abliterated:14b
   ```
3. Start Ollama server:
   ```bash
   ollama serve
   ```

### Recommended Models

| Model | Size | Use Case |
|-------|------|----------|
| `huihui_ai/deepseek-r1-abliterated:14b` | 14B parameters | Primary recommended model |
| `qwen2.5-coder:14b` | 14B parameters | Legacy backward compatibility |
| `qwen2.5-coder:14b-q4KM` | Quantized | Reduced memory usage (legacy) |
| `qwen2.5-coder:7b` | 7B parameters | Faster, lower quality (legacy) |

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
  model: huihui_ai/deepseek-r1-abliterated:14b
  temperature: 0.0
  seed: 42

ollama:
  model: huihui_ai/deepseek-r1-abliterated:14b
  endpoint: http://127.0.0.1:11434

execution:
  sandbox_mode: repo_boundary_only
```

Supported `execution.sandbox_mode` values:
- `none`: no repository boundary mode label; still subject to runtime tool policy
- `repo_boundary_only`: current default; repo path checks plus bounded worker execution
- `disposable_workspace`: TUI-managed git worktree execution; source workspace is not mutated and promotion is report-only
- `external_container`: roadmap placeholder; rejected until implemented

Optional disposable workspace settings:
```yaml
execution:
  sandbox_mode: disposable_workspace
  disposable_workspace:
    backend: git_worktree
    retain_on_failure: false
    retain_on_success: false
    require_explicit_promotion: true
```

Current implementation scope:
- `git_worktree` backend only
- non-git recursive-copy backend is not implemented
- direct `forge_bootstrap` runs still reject `disposable_workspace`; the TUI wrapper creates the worktree and runs the worker with `repo_boundary_only` inside it
- `retain_on_failure: true` preserves a failed disposable worktree and reports the retained path; default `false` removes it
- `retain_on_success: true` preserves a successful disposable worktree and reports the retained path; default `false` removes it
- promotion is an explicit report containing changed files and diff; automatic source workspace mutation is not implemented
- promotion reports include `execution_environment`, `changes_made`, `validation_results`, `source_head_before`, `source_head_after`, and `promotion_status`
- source HEAD lookup, diff generation, promotion report generation, and cleanup failures fail closed instead of silently succeeding
- `disposable_workspace` is workspace isolation only. It is not OS-level containment, and commands still run with the user's permissions inside the disposable worktree.

Disposable workspace runtime events:
- `disposable_workspace_created`
- `disposable_workspace_execution_started`
- `disposable_workspace_validation_passed`
- `disposable_workspace_validation_failed`
- `disposable_workspace_promotion_report_created`
- `disposable_workspace_promotion_pending`
- `disposable_workspace_cleaned`
- `disposable_workspace_retained`
- `disposable_workspace_cleanup_failed`

### Development Override
```bash
# Use alternative model for testing
FORGE_PLANNER_MODEL=huihui_ai/deepseek-r1-abliterated:14b ./rasputin ./my-project

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
