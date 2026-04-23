# Rasputin

Rasputin is a local terminal-first coding agent that runs in your terminal, connects to local Ollama models, and can execute code changes through a bounded runtime engine called Forge.

**Current Product Reality**: The system works and is usable, with a 5-layer truth hierarchy and bounded autonomous goal loop:
- **Layer 1 (Outcome)**: Single authoritative `ExecutionOutcome` per chain (Success, Failed, Blocked, SuccessWithWarnings)
- **Layer 2 (Progress)**: Canonical `ExecutionState` via reducer transitions (Idle → Planning → Executing → Done)
- **Layer 3 (Audit)**: Immutable append-only `AuditLog` recording every transition and outcome
- **Layer 4 (Replay)**: Deterministic reconstruction from audit via `replay_audit_log()`
- **Layer 5 (Checkpoint)**: Validated snapshots with workspace integrity for crash recovery

**What You Get**:
- Terminal chat interface for local LLMs ✓
- Task-like natural-language input that becomes a bounded autonomous goal ✓
- Qwen-Coder-first goal planning with deterministic fallback ✓
- Bounded, validated code execution through Forge ✓
- **Multi-step chain execution** with validation gating ✓
- **5-layer truth hierarchy**: Outcome → Progress → Audit → Replay → Checkpoint ✓
- **Deterministic replay** from audit log for debugging and forensics ✓
- **Validated checkpoint resume** with workspace hash verification ✓
- **Fail-closed validation** across format → lint → build → test stages ✓
- **Audit-grounded execution timeline** in inspector with full traceability ✓
- Stage-oriented runtime surfaces in the inspector ✓

**What You Don't Get**:
- Unbounded background autonomy
- Question-like chat follow-up automatically mutating the previous Forge execution context
- Mid-task approval checkpoints within a chain step
- Faithful execution transcript in the main chat pane (task detail lives in inspector and logs)

---

## Start Here (Choose Your Path)

| I want to... | Read this |
|--------------|-----------|
| **Understand what this product actually does** | [01_PROJECT_OVERVIEW.md](docs/01_PROJECT_OVERVIEW.md) — elevator pitch, core purpose, design philosophy |
| **Understand the system architecture** | [02_ARCHITECTURE.md](docs/02_ARCHITECTURE.md) — layers, data flow, key decisions |
| **Understand the technology stack** | [03_TECHNOLOGY_STACK.md](docs/03_TECHNOLOGY_STACK.md) — languages, dependencies, build config |
| **Understand core concepts** | [04_CORE_CONCEPTS.md](docs/04_CORE_CONCEPTS.md) — domain model, abstractions, terminology |
| **Navigate the codebase** | [05_FOLDER_STRUCTURE.md](docs/05_FOLDER_STRUCTURE.md) — repository layout, key files, data paths |
| **Understand how it works** | [06_MAIN_WORKFLOWS.md](docs/06_MAIN_WORKFLOWS.md) — startup, chat, tasks, chains, validation |
| **Use the API** | [07_API_REFERENCE.md](docs/07_API_REFERENCE.md) — commands, shortcuts, types, tools |
| **Understand the data model** | [08_DATA_MODEL.md](docs/08_DATA_MODEL.md) — persistence, state, chains, validation data |
| **Configure the system** | [09_CONFIGURATION.md](docs/09_CONFIGURATION.md) — config files, env vars, Ollama setup |
| **Develop or contribute** | [10_DEVELOPMENT_GUIDE.md](docs/10_DEVELOPMENT_GUIDE.md) — setup, building, testing, debugging |
| **Understand testing** | [11_TESTING_STRATEGY.md](docs/11_TESTING_STRATEGY.md) — test organization, coverage, validation |
| **Deploy and operate** | [12_DEPLOYMENT_AND_OPERATIONS.md](docs/12_DEPLOYMENT_AND_OPERATIONS.md) — installation, ops, troubleshooting |
| **Understand security** | [13_SECURITY_AND_COMPLIANCE.md](docs/13_SECURITY_AND_COMPLIANCE.md) — architecture, controls, compliance |
| **Know what's resolved/not** | [14_KNOWN_LIMITATIONS_AND_TRADEOFFS.md](docs/14_KNOWN_LIMITATIONS_AND_TRADEOFFS.md) — boundaries, invariants, tradeoffs |
| **See the future roadmap** | [15_FUTURE_ROADMAP_AND_EXTENSIBILITY.md](docs/15_FUTURE_ROADMAP_AND_EXTENSIBILITY.md) — phases, priorities, metrics |

---

## Product Reality Summary

## What Is Implemented
- One canonical startup path: `./rasputin [workspace-path]`
- One canonical product entrypoint: `apps/rasputin-tui/src/main.rs`
- One canonical Forge handoff path: `apps/rasputin-tui/src/app.rs::start_execution_task`
- One internal Forge worker runtime: `crates/forge-runtime/src/main.rs` -> `run_bootstrap()`
- Canonical docs are the 15 numbered files under `docs/`

## Repository Structure

```
Rasputin-1/
├── apps/
│   └── rasputin-tui/          # User-facing terminal UI (the product)
├── crates/
│   ├── forge-runtime/         # Bounded execution engine (worker process)
│   └── rasputin-interface/    # Partial orchestration layer (NOT the hot path)
├── docs/                      # 15 canonical docs (01-15)
│   ├── 01_PROJECT_OVERVIEW.md
│   ├── 02_ARCHITECTURE.md
│   ├── ... (through 15)
│   └── 15_FUTURE_ROADMAP_AND_EXTENSIBILITY.md
├── support/                   # Shared workspace config
├── config/                    # Ollama model files
└── research/                  # Mockups and explorations (not product)
```

**Canonical docs are the 15 numbered files in `docs/`**. They are the single source of truth. Historical material is archived in `.historical_docs_backup/`.

## Quick Start

```bash
# Run the product
./rasputin /path/to/workspace

# Or via Cargo
cargo run -p rasputin-tui -- /path/to/workspace
```

**First time using Rasputin?** Read [01_PROJECT_OVERVIEW.md](docs/01_PROJECT_OVERVIEW.md) for the elevator pitch and quick start.

---

## Documentation Structure

**Authoritative** (15 canonical files):
All documentation is consolidated into the 15 numbered files in `docs/`.

**Historical** (archived in `.historical_docs_backup/`):
- Previous V1.5/V2.x implementation summaries
- Consolidated supplementary docs (APPROVAL_CHECKPOINTS, GIT_GROUNDING, etc.)
- Audit reports and sprint progress docs

**Non-authoritative**:
- `research/` — mockups and future explorations

---

## The Runtime Reality (Important)

The product has two distinct runtime layers that share a UI shell:

1. **Rasputin Product State** — long-running TUI state (chat history, repos, preferences)
2. **Forge Worker State** — per-task execution state (files read, mutations, validation)

**User consequence**: Task-like plain text is treated as a goal, planned with Qwen-Coder, confirmed automatically, and executed through a bounded chain. Question-like chat remains normal Ollama chat and does not mutate the previous Forge task's worker context.

This is **intentional**: autonomy is bounded by step limits, validation gates, approval checkpoints, and worker isolation. See [04_CORE_CONCEPTS.md](docs/04_CORE_CONCEPTS.md) and [14_KNOWN_LIMITATIONS_AND_TRADEOFFS.md](docs/14_KNOWN_LIMITATIONS_AND_TRADEOFFS.md) for the full rationale.

---

## Non-Goals (What This Is NOT)

**Rasputin is explicitly NOT:**

| Category | What It Is NOT | Why |
|----------|----------------|-----|
| **Architecture** | A single shared in-memory model for chat and Forge | Forge worker state is process-isolated by design |
| **Execution** | An unbounded autonomous daemon | Execution has hard iteration limits and validation gates |
| **Control** | An approval-driven orchestration system | No mid-task pause/resume within a chain step; resume requires explicit approval |
| **Tooling** | A general-purpose shell replacement | Planner sees bounded tool surface with mode-gated access |
| **Infrastructure** | A daemon/service | Process-per-task, no background worker |
| **Connectivity** | A cloud-connected product | Local Ollama only—no GPT-4, Claude, or APIs |
| **UX** | A narrative-driven interface | Backend-shaped event stream by design |

**Correct mental model**: A **bounded, validated, local autonomous SWE loop**—not an unbounded background agent.

**Chain execution exists**: Multi-step chains with validation gating, checkpoints at validated boundaries, and guarded resume with explicit approval.

If you need continuous background execution, per-action approval inside every worker step, or cloud frontier models, Rasputin **will not meet your needs** in its current form.
