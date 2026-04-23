# Rasputin Project Overview

## Elevator Pitch

Rasputin is a **local terminal-first coding agent** that runs entirely on your machine, connects to local Ollama LLMs, and executes code changes through a bounded, validated runtime engine called Forge. It provides deterministic, privacy-preserving AI-assisted coding without external API dependencies.

## Core Purpose

Rasputin bridges the gap between conversational AI assistants and deterministic code execution systems. It enables developers to:

- Chat with local LLMs for advice and explanation
- State task-like goals in natural language and have them planned/executed through a bounded autonomous loop
- Execute bounded code modification tasks with validation-gated persistence
- Maintain privacy by keeping all code and inference local
- Review changes through technical inspection surfaces before acceptance

## Product Reality

The system is intentionally **split** between two distinct runtime layers:

| Layer | Purpose | Lifetime |
|-------|---------|----------|
| **Rasputin TUI** | Terminal UX, repo attachment, chat persistence, chain orchestration, command routing | Long-running product session |
| **Forge Worker** | Task planning, tool execution, mutation validation, bounded execution | One task = one process |

**Current State**: The trust loop is hardened with:
- Task-like plain text or `/goal` → Qwen-Coder plan → bounded chain → validate → auto-continue → complete/replay
- `/plan → /preview → execute → interrupt → resume → complete → replay`
- Any interrupt at any point has a clear recovery path
- No silent failures, no stale state confusion

## What You Get

### Core System
- Terminal-native UI with sidebar, chat panel, composer, and optional inspector
- Local Ollama integration (no cloud APIs) — **loopback-only HTTP, no server exposure**
- Core file tools: `read_file`, `write_file`, `apply_patch`
- Discovery tools: `list_dir`, `grep_search`
- Syntax, build, and test validation gates
- Per-task bounded execution (max 10 iterations by default)
- Deterministic settings (temperature 0.0-0.1, seed 42)
- Qwen-Coder-first goal planning with deterministic heuristic fallback
- **Persistent chains** with step tracking and resume capability
- **Auto-resume** for autonomous continuation within policy bounds
- **Risk forecasting** with preview of upcoming steps and detected risks

### UI/UX Model: Normal vs Operator Mode

Rasputin operates in two experience modes:

| Mode | Audience | Inspector | Status Bar | Composer |
|------|----------|-----------|------------|----------|
| **Normal** | Daily users | Manual toggle only | Human-readable: "Working...", "Step 2 of 5" | Conversational hints |
| **Operator** | Debug/audit | Auto-shows on execution | Technical: Chain IDs, Git SHAs | Full mode toggle [CHAT][EDIT][TASK] |

**Design Philosophy**: Normal mode hides debug machinery. Operator mode exposes all audit surfaces. Toggle via sidebar "View" section.

### Security Posture

- **Local-only Ollama**: HTTP client restricted to `127.0.0.1:11434`, `localhost`, or `[::1]` — remote endpoints are rejected at client construction
- **No cloud API paths**: System is architecturally incapable of calling OpenAI, Anthropic, or other cloud APIs
- **Repository boundary enforcement**: All file operations validated against repo root (path traversal blocked)
- **Command allowlisting**: Shell execution restricted to safe commands (cargo, npm, git, etc.) with destructive operations requiring confirmation

## What You Don't Get

- Question-like chat automatically mutating prior Forge worker state
- Mid-task approval checkpoints (chain-level only)
- Automatic *unbounded* background continuation
- Cloud API integration (OpenAI, Anthropic, etc.)
- Codebase search within planner context (user must provide paths)

## Target Users

Rasputin is designed for developers who:
- Prioritize privacy and local control
- Accept bounded, deterministic autonomy over unbounded background agents
- Prefer technical precision over conversational polish
- Are willing to provide explicit file paths and context
- Value validation-gated code quality over speed

## Quick Start

```bash
# Start Rasputin with a workspace
./rasputin /path/to/workspace

# Inside the TUI
create a Rust CLI that prints hello world
# or explicitly:
/goal create a Rust CLI that prints hello world
/goal confirm                # Explicit acceptance; task-like plain text queues this automatically
/plan                        # Show multi-step plan
/preview                     # Forecast risks and preview execution
/stop                        # Interrupt if needed
/chain resume                # Resume from interruption
```

## Canonical Entry Points

- **User bootstrap**: `./rasputin [workspace-path]`
- **Product entrypoint**: `apps/rasputin-tui/src/main.rs`
- **Forge worker**: `crates/forge-runtime/src/main.rs` (spawned per task)

## Design Philosophy

> **Local, bounded, deterministic execution of AI-generated code changes with validation-gated persistence.**

Every architectural decision flows from this statement: bounded iterations prevent runaway execution, strict contracts ensure runtime stability, validation gates prevent bad code persistence, and local-first design protects privacy.

## Competitive Positioning

### Market Category

Rasputin is a **bounded autonomous execution system** that uses local language models as a planning layer, not as a source of truth.

| Category | Description | Examples |
|----------|------------|----------|
| Conversational Coding Assistants | AI helps write/edit code interactively | Codex CLI, Windsurf |
| Autonomous Agents | AI explores and modifies codebases with minimal input | AutoGPT-style tools |
| **Bounded Autonomous Execution Systems** | AI generates plans, execution continues only within policy and validation bounds | **Rasputin** |

### vs. Codex CLI / Windsurf

| Aspect | Competitors | Rasputin |
|--------|-------------|----------|
| Codebase awareness | Strong, automatic | Bounded repo snapshot plus planner-visible tools |
| Conversational UX | Smooth, continuous | Task-like text enters goal loop; questions remain chat |
| Autonomous discovery | Yes | Bounded by repo snapshot and planner-visible tools |
| Validation gating | Weak/absent | Strong (syntax → build → test) |
| Determinism | Low | High (bounded, seeded, temperature-clamped) |
| Privacy | Cloud-dependent | Fully local |

### Core Differentiator

> **Rasputin does not try to be the smartest system. It tries to be the most trustworthy system.**

- Others answer: "What should we do?"
- Rasputin answers: "What can we prove is safe to do?"

### Product Identity

Rasputin is best understood as:
> **"Codex-at-home: A local, bounded autonomous SWE loop for AI-generated code changes."**

This positions Rasputin as the self-hosted, privacy-preserving alternative to cloud coding assistants. You bring your own compute (Ollama), your own repos, your own policies. The system executes deterministically within bounds you control.

## Implementation Status (Consolidated)

### Implemented
- `./rasputin` is the single user-facing startup path
- `apps/rasputin-tui` is the active product application
- `crates/forge-runtime` is the active bounded execution engine
- Chain execution with multi-step task chains and validation gating
- Natural-language task-like input routes into goal planning and bounded execution
- Qwen-Coder-first goal planning with fallback to the deterministic heuristic planner
- Checkpoints with validated state saved after each successful chain step
- Guarded resume plus auto-resume for accepted goal chains
- Fail-closed validation across format → build → test stages
- Revert integrity with file contents restored on validation failure
- Risk forecasting with GitConflict detection and blocking
- Interrupt handling with context preservation
- Git grounding with dirty worktree detection
- **Normal/Operator experience modes** with debug surface gating
- **Dual-mode status bar** (human-readable vs technical)
- **Conversational composer UI** in Normal mode
- **Native folder picker** for project creation (macOS AppleScript)

### Partial
- `crates/rasputin-interface` contains real code but is not the canonical live control loop
- Question-like chat and Forge worker state remain separate session models
- TUI-local `/validate` exists but is separate from Forge runtime validation

### Planned or Research-Only
- Unified conversation-plus-execution session continuity
- Live approval checkpoints in the normal task hot path
- Stronger multi-turn follow-up semantics

## Documentation Authority

### Canonical Documents
Only these locations define current system behavior:
- `docs/01_PROJECT_OVERVIEW.md` through `docs/15_FUTURE_ROADMAP_AND_EXTENSIBILITY.md` (this canonical set)
- `README.md`

### Non-Canonical
- `.historical_docs_backup/` — historical context only
- `research/` — exploratory, not implementation status

### Runtime Overrides
If documentation disagrees with the active implementation, **the active code wins**.
